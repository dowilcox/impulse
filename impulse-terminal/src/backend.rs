//! Terminal backend — owns the alacritty_terminal::Term and PTY event loop.

use std::borrow::Cow;
use std::sync::Arc;
use std::thread::JoinHandle;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags as AlacFlags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AlacColor, CursorShape as AlacCursorShape, NamedColor};
use crossbeam_channel::{Receiver, Sender};

use crate::buffer::{self, HighlightRange};
use crate::config::TerminalConfig;
use crate::event::TerminalEvent;
use crate::grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};

// ---------------------------------------------------------------------------
// Event proxy — bridges alacritty events to our channel
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct EventProxy {
    event_tx: Sender<TerminalEvent>,
    pty_write_tx: Sender<String>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacEvent) {
        match event {
            AlacEvent::PtyWrite(text) => { let _ = self.pty_write_tx.send(text); }
            AlacEvent::Wakeup => { let _ = self.event_tx.send(TerminalEvent::Wakeup); }
            AlacEvent::Title(title) => { let _ = self.event_tx.send(TerminalEvent::TitleChanged(title)); }
            AlacEvent::ResetTitle => { let _ = self.event_tx.send(TerminalEvent::ResetTitle); }
            AlacEvent::Bell => { let _ = self.event_tx.send(TerminalEvent::Bell); }
            AlacEvent::Exit => { let _ = self.event_tx.send(TerminalEvent::Exit); }
            AlacEvent::ChildExit(status) => {
                // ExitStatus -> i32: use code() which returns Option<i32>, defaulting to -1.
                let code = status.code().unwrap_or(-1);
                let _ = self.event_tx.send(TerminalEvent::ChildExited(code));
            }
            AlacEvent::ClipboardStore(_, text) => { let _ = self.event_tx.send(TerminalEvent::ClipboardStore(text)); }
            AlacEvent::ClipboardLoad(_, _) => { let _ = self.event_tx.send(TerminalEvent::ClipboardLoad); }
            AlacEvent::CursorBlinkingChange => { let _ = self.event_tx.send(TerminalEvent::CursorBlinkingChange); }
            AlacEvent::ColorRequest(_, _)
            | AlacEvent::TextAreaSizeRequest(_)
            | AlacEvent::MouseCursorDirty => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal size helper
// ---------------------------------------------------------------------------

struct TermSize {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize { self.screen_lines }
    fn screen_lines(&self) -> usize { self.screen_lines }
    fn columns(&self) -> usize { self.columns }
}

// ---------------------------------------------------------------------------
// Color resolution
// ---------------------------------------------------------------------------

struct ConfiguredColors {
    foreground: RgbColor,
    background: RgbColor,
    palette: [RgbColor; 269],
}

impl ConfiguredColors {
    fn from_config(config: &TerminalConfig) -> Self {
        let mut palette = [RgbColor::new(0, 0, 0); 269];

        // 16 ANSI colors from config.
        for (i, c) in config.colors.palette.iter().enumerate() {
            palette[i] = *c;
        }

        // 6x6x6 color cube (indices 16-231).
        for i in 16u16..232 {
            let idx = i - 16;
            let r = (idx / 36) as u8;
            let g = ((idx % 36) / 6) as u8;
            let b = (idx % 6) as u8;
            let to_val = |v: u8| if v == 0 { 0u8 } else { 55 + 40 * v };
            palette[i as usize] = RgbColor::new(to_val(r), to_val(g), to_val(b));
        }

        // Grayscale ramp (indices 232-255).
        for i in 232u16..256 {
            let val = (8 + 10 * (i - 232)) as u8;
            palette[i as usize] = RgbColor::new(val, val, val);
        }

        // Named colors.
        palette[NamedColor::Foreground as usize] = config.colors.foreground;
        palette[NamedColor::Background as usize] = config.colors.background;
        palette[NamedColor::Cursor as usize] = config.colors.foreground;
        palette[NamedColor::BrightForeground as usize] = config.colors.foreground;
        palette[NamedColor::DimForeground as usize] = config.colors.foreground;

        // Dim colors.
        palette[NamedColor::DimBlack as usize] = RgbColor::new(0, 0, 0);
        for i in 1..8usize {
            let base = config.colors.palette[i];
            palette[NamedColor::DimBlack as usize + i] =
                RgbColor::new(base.r * 3 / 4, base.g * 3 / 4, base.b * 3 / 4);
        }

        Self {
            foreground: config.colors.foreground,
            background: config.colors.background,
            palette,
        }
    }

    fn resolve(&self, color: AlacColor, term_colors: &alacritty_terminal::term::color::Colors) -> RgbColor {
        match color {
            AlacColor::Spec(rgb) => RgbColor::new(rgb.r, rgb.g, rgb.b),
            AlacColor::Named(named) => {
                if let Some(rgb) = term_colors[named] {
                    RgbColor::new(rgb.r, rgb.g, rgb.b)
                } else {
                    self.palette[named as usize]
                }
            }
            AlacColor::Indexed(idx) => {
                if let Some(rgb) = term_colors[idx as usize] {
                    RgbColor::new(rgb.r, rgb.g, rgb.b)
                } else {
                    self.palette[idx as usize]
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Selection kind
// ---------------------------------------------------------------------------

/// Selection kind for `start_selection()`.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum SelectionKind {
    Simple = 0,
    Block = 1,
    Semantic = 2,
    Lines = 3,
}

impl SelectionKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Block,
            2 => Self::Semantic,
            3 => Self::Lines,
            _ => Self::Simple,
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalBackend
// ---------------------------------------------------------------------------

/// The main terminal backend. One instance per terminal tab/split.
pub struct TerminalBackend {
    term: Arc<FairMutex<Term<EventProxy>>>,
    event_loop_sender: EventLoopSender,
    event_rx: Receiver<TerminalEvent>,
    pty_write_rx: Receiver<String>,
    _pty_thread: Option<JoinHandle<(EventLoop<tty::Pty, EventProxy>, alacritty_terminal::event_loop::State)>>,
    cols: u16,
    rows: u16,
    colors: ConfiguredColors,
    child_pid: u32,
}

impl TerminalBackend {
    /// Create a new terminal backend and spawn a shell process.
    pub fn new(
        config: TerminalConfig,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<Self, String> {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (pty_write_tx, pty_write_rx) = crossbeam_channel::unbounded();
        let proxy = EventProxy { event_tx, pty_write_tx };

        let alac_config = config.to_alacritty_config();
        let pty_options = config.to_pty_options();
        let colors = ConfiguredColors::from_config(&config);

        let size = TermSize { columns: cols as usize, screen_lines: rows as usize };
        let term = Term::new(alac_config, &size, proxy.clone());
        let term = Arc::new(FairMutex::new(term));

        let window_size = WindowSize { num_lines: rows, num_cols: cols, cell_width, cell_height };
        let pty = tty::new(&pty_options, window_size, 0)
            .map_err(|e| format!("Failed to create PTY: {e}"))?;
        let child_pid = pty.child().id();

        let event_loop = EventLoop::new(Arc::clone(&term), proxy, pty, pty_options.drain_on_exit, false)
            .map_err(|e| format!("Failed to create event loop: {e}"))?;
        let event_loop_sender = event_loop.channel();
        let pty_thread = event_loop.spawn();

        Ok(Self {
            term,
            event_loop_sender,
            event_rx,
            pty_write_rx,
            _pty_thread: Some(pty_thread),
            cols,
            rows,
            colors,
            child_pid,
        })
    }

    /// Send input bytes to the PTY.
    pub fn write(&self, data: &[u8]) {
        self.drain_pty_writes();
        if !data.is_empty() {
            let _ = self.event_loop_sender.send(Msg::Input(Cow::Owned(data.to_vec())));
        }
    }

    /// Resize the terminal grid and PTY.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        if cols == self.cols && rows == self.rows { return; }
        self.cols = cols;
        self.rows = rows;
        let size = TermSize { columns: cols as usize, screen_lines: rows as usize };
        self.term.lock().resize(size);
        let ws = WindowSize { num_lines: rows, num_cols: cols, cell_width, cell_height };
        let _ = self.event_loop_sender.send(Msg::Resize(ws));
    }

    /// Poll for terminal events (non-blocking).
    pub fn poll_events(&self) -> Vec<TerminalEvent> {
        self.drain_pty_writes();
        let mut events = Vec::new();
        while let Ok(ev) = self.event_rx.try_recv() {
            events.push(ev);
        }
        events
    }

    /// Write the visible grid into a pre-allocated binary buffer.
    /// Returns the number of bytes written.
    pub fn write_grid_to_buffer(&self, buf: &mut [u8]) -> usize {
        let term = self.term.lock();
        let content = term.renderable_content();
        let term_colors = content.colors;
        let mode = content.mode;
        let cursor = content.cursor;
        let num_cols = term.columns();
        let num_lines = term.screen_lines();

        // Build mode flags.
        let mut mode_flags = TerminalMode::empty();
        if mode.contains(TermMode::SHOW_CURSOR) { mode_flags |= TerminalMode::SHOW_CURSOR; }
        if mode.contains(TermMode::APP_CURSOR) { mode_flags |= TerminalMode::APP_CURSOR; }
        if mode.contains(TermMode::APP_KEYPAD) { mode_flags |= TerminalMode::APP_KEYPAD; }
        if mode.contains(TermMode::MOUSE_REPORT_CLICK) { mode_flags |= TerminalMode::MOUSE_REPORT_CLICK; }
        if mode.contains(TermMode::MOUSE_MOTION) { mode_flags |= TerminalMode::MOUSE_MOTION; }
        if mode.contains(TermMode::MOUSE_DRAG) { mode_flags |= TerminalMode::MOUSE_DRAG; }
        if mode.contains(TermMode::SGR_MOUSE) { mode_flags |= TerminalMode::MOUSE_SGR; }
        if mode.contains(TermMode::BRACKETED_PASTE) { mode_flags |= TerminalMode::BRACKETED_PASTE; }
        if mode.contains(TermMode::FOCUS_IN_OUT) { mode_flags |= TerminalMode::FOCUS_IN_OUT; }
        if mode.contains(TermMode::ALT_SCREEN) { mode_flags |= TerminalMode::ALT_SCREEN; }
        if mode.contains(TermMode::LINE_WRAP) { mode_flags |= TerminalMode::LINE_WRAP; }

        // Cursor state.
        let cursor_visible = mode.contains(TermMode::SHOW_CURSOR)
            && cursor.point.line.0 >= 0
            && (cursor.point.line.0 as usize) < num_lines;
        let cursor_state = CursorState {
            row: cursor.point.line.0.max(0) as usize,
            col: cursor.point.column.0,
            shape: match cursor.shape {
                AlacCursorShape::Block => CursorShape::Block,
                AlacCursorShape::Beam => CursorShape::Beam,
                AlacCursorShape::Underline => CursorShape::Underline,
                AlacCursorShape::HollowBlock => CursorShape::HollowBlock,
                AlacCursorShape::Hidden => CursorShape::Hidden,
            },
            visible: cursor_visible,
        };

        // Selection ranges.
        let mut selection_ranges = Vec::new();
        if let Some(sel) = &content.selection {
            let start_line = sel.start.line.0.max(0) as usize;
            let end_line = (sel.end.line.0.max(0) as usize).min(num_lines.saturating_sub(1));
            for row in start_line..=end_line {
                let sc = if row == start_line { sel.start.column.0 } else { 0 };
                let ec = if row == end_line { sel.end.column.0 } else { num_cols.saturating_sub(1) };
                selection_ranges.push(HighlightRange {
                    row: row as u16,
                    start_col: sc as u16,
                    end_col: ec as u16,
                });
            }
        }

        // TODO: search match ranges will be added in Task 9.
        let search_ranges: Vec<HighlightRange> = Vec::new();

        let required = buffer::buffer_size(
            num_cols as u16, num_lines as u16,
            selection_ranges.len() as u16, search_ranges.len() as u16,
        );
        if buf.len() < required { return 0; }

        // Write header.
        let cell_offset = buffer::write_header(
            buf,
            num_cols as u16, num_lines as u16,
            &cursor_state, mode_flags,
            &selection_ranges, &search_ranges,
        );

        // Initialize all cells to space with default colors.
        let default_fg = self.colors.foreground;
        let default_bg = self.colors.background;
        for i in 0..(num_cols * num_lines) {
            buffer::write_cell(
                buf,
                cell_offset + i * buffer::CELL_STRIDE,
                ' ', default_fg, default_bg, CellFlags::empty(),
            );
        }

        // Fill from display iterator.
        for indexed in content.display_iter {
            let row = indexed.point.line.0 as usize;
            let col = indexed.point.column.0;
            if row < num_lines && col < num_cols {
                let offset = cell_offset + (row * num_cols + col) * buffer::CELL_STRIDE;
                let fg = self.colors.resolve(indexed.cell.fg, term_colors);
                let bg = self.colors.resolve(indexed.cell.bg, term_colors);
                let flags = convert_flags(indexed.cell.flags);
                buffer::write_cell(buf, offset, indexed.cell.c, fg, bg, flags);
            }
        }

        required
    }

    /// Calculate the buffer size needed for a grid snapshot.
    pub fn grid_buffer_size(&self) -> usize {
        let term = self.term.lock();
        let lines = term.screen_lines() as u16;
        let cols = term.columns() as u16;
        // Allow up to lines*2 ranges for selection + search.
        buffer::buffer_size(cols, lines, lines, lines)
    }

    /// Start a text selection.
    pub fn start_selection(&self, col: usize, row: usize, kind: SelectionKind) {
        let mut term = self.term.lock();
        let point = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(row as i32),
            alacritty_terminal::index::Column(col),
        );
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Block => SelectionType::Block,
            SelectionKind::Semantic => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
        };
        term.selection = Some(Selection::new(ty, point, alacritty_terminal::index::Side::Left));
    }

    /// Update the current selection endpoint.
    pub fn update_selection(&self, col: usize, row: usize) {
        let mut term = self.term.lock();
        if let Some(ref mut sel) = term.selection {
            let point = alacritty_terminal::index::Point::new(
                alacritty_terminal::index::Line(row as i32),
                alacritty_terminal::index::Column(col),
            );
            sel.update(point, alacritty_terminal::index::Side::Right);
        }
    }

    /// Clear the current selection.
    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    /// Get the selected text.
    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string()
    }

    /// Scroll the viewport. Positive = up (towards history), negative = down.
    pub fn scroll(&self, delta: i32) {
        self.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
    }

    /// Scroll the viewport to the bottom.
    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Bottom);
    }

    /// Get the current terminal mode flags.
    pub fn mode(&self) -> TerminalMode {
        let mode = *self.term.lock().mode();
        let mut flags = TerminalMode::empty();
        if mode.contains(TermMode::SHOW_CURSOR) { flags |= TerminalMode::SHOW_CURSOR; }
        if mode.contains(TermMode::APP_CURSOR) { flags |= TerminalMode::APP_CURSOR; }
        if mode.contains(TermMode::APP_KEYPAD) { flags |= TerminalMode::APP_KEYPAD; }
        if mode.contains(TermMode::MOUSE_REPORT_CLICK) { flags |= TerminalMode::MOUSE_REPORT_CLICK; }
        if mode.contains(TermMode::MOUSE_MOTION) { flags |= TerminalMode::MOUSE_MOTION; }
        if mode.contains(TermMode::MOUSE_DRAG) { flags |= TerminalMode::MOUSE_DRAG; }
        if mode.contains(TermMode::SGR_MOUSE) { flags |= TerminalMode::MOUSE_SGR; }
        if mode.contains(TermMode::BRACKETED_PASTE) { flags |= TerminalMode::BRACKETED_PASTE; }
        if mode.contains(TermMode::FOCUS_IN_OUT) { flags |= TerminalMode::FOCUS_IN_OUT; }
        if mode.contains(TermMode::ALT_SCREEN) { flags |= TerminalMode::ALT_SCREEN; }
        if mode.contains(TermMode::LINE_WRAP) { flags |= TerminalMode::LINE_WRAP; }
        flags
    }

    /// Get the current terminal dimensions.
    pub fn dimensions(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// Get the PID of the child shell process.
    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    /// Notify the terminal about focus change.
    pub fn set_focus(&self, focused: bool) {
        self.term.lock().is_focused = focused;
    }

    /// Shut down the terminal.
    pub fn shutdown(&self) {
        let _ = self.event_loop_sender.send(Msg::Shutdown);
    }

    /// Access the term lock (for search module in Task 9).
    pub(crate) fn term(&self) -> &Arc<FairMutex<Term<EventProxy>>> {
        &self.term
    }

    fn drain_pty_writes(&self) {
        while let Ok(text) = self.pty_write_rx.try_recv() {
            let _ = self.event_loop_sender.send(Msg::Input(Cow::Owned(text.into_bytes())));
        }
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Convert alacritty cell flags to our CellFlags.
fn convert_flags(flags: AlacFlags) -> CellFlags {
    let mut result = CellFlags::empty();
    if flags.contains(AlacFlags::BOLD) { result |= CellFlags::BOLD; }
    if flags.contains(AlacFlags::ITALIC) { result |= CellFlags::ITALIC; }
    if flags.contains(AlacFlags::UNDERLINE) { result |= CellFlags::UNDERLINE; }
    if flags.contains(AlacFlags::STRIKEOUT) { result |= CellFlags::STRIKETHROUGH; }
    if flags.contains(AlacFlags::DIM) { result |= CellFlags::DIM; }
    if flags.contains(AlacFlags::INVERSE) { result |= CellFlags::INVERSE; }
    if flags.contains(AlacFlags::HIDDEN) { result |= CellFlags::HIDDEN; }
    if flags.contains(AlacFlags::WIDE_CHAR) { result |= CellFlags::WIDE_CHAR; }
    if flags.contains(AlacFlags::WIDE_CHAR_SPACER) { result |= CellFlags::WIDE_CHAR_SPACER; }
    if flags.contains(AlacFlags::DOUBLE_UNDERLINE) { result |= CellFlags::DOUBLE_UNDERLINE; }
    if flags.contains(AlacFlags::UNDERCURL) { result |= CellFlags::UNDERCURL; }
    if flags.contains(AlacFlags::DOTTED_UNDERLINE) { result |= CellFlags::DOTTED_UNDERLINE; }
    if flags.contains(AlacFlags::DASHED_UNDERLINE) { result |= CellFlags::DASHED_UNDERLINE; }
    result
}
