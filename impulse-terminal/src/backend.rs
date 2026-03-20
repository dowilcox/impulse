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

use crate::config::TerminalConfig;
use crate::event::TerminalEvent;
use crate::grid::{
    CellFlags, CursorShape, CursorState, GridSnapshot, RgbColor, StyledCell, TerminalMode,
};

// ---------------------------------------------------------------------------
// Event proxy — bridges alacritty_terminal events to our channel
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FullEventProxy {
    event_sender: Sender<TerminalEvent>,
    pty_write_sender: Sender<String>,
}

impl EventListener for FullEventProxy {
    fn send_event(&self, event: AlacEvent) {
        match event {
            AlacEvent::PtyWrite(text) => {
                let _ = self.pty_write_sender.send(text);
            },
            AlacEvent::ColorRequest(_, _)
            | AlacEvent::TextAreaSizeRequest(_)
            | AlacEvent::MouseCursorDirty => {},
            AlacEvent::Wakeup => {
                let _ = self.event_sender.send(TerminalEvent::Wakeup);
            },
            AlacEvent::Title(title) => {
                let _ = self.event_sender.send(TerminalEvent::TitleChanged(title));
            },
            AlacEvent::ResetTitle => {
                let _ = self.event_sender.send(TerminalEvent::ResetTitle);
            },
            AlacEvent::Bell => {
                let _ = self.event_sender.send(TerminalEvent::Bell);
            },
            AlacEvent::Exit => {
                let _ = self.event_sender.send(TerminalEvent::Exit);
            },
            AlacEvent::ChildExit(code) => {
                let _ = self.event_sender.send(TerminalEvent::ChildExited(code));
            },
            AlacEvent::ClipboardStore(_, text) => {
                let _ = self.event_sender.send(TerminalEvent::ClipboardStore(text));
            },
            AlacEvent::ClipboardLoad(_, _) => {
                let _ = self.event_sender.send(TerminalEvent::ClipboardLoad);
            },
            AlacEvent::CursorBlinkingChange => {
                let _ = self.event_sender.send(TerminalEvent::CursorBlinkingChange);
            },
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
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

// ---------------------------------------------------------------------------
// TerminalBackend
// ---------------------------------------------------------------------------

/// The main terminal backend. One instance per terminal tab/split.
///
/// Owns the terminal state (`Term`), the PTY event loop, and provides
/// a high-level API for frontends.
pub struct TerminalBackend {
    /// The terminal state (grid, cursor, scrollback, etc.).
    term: Arc<FairMutex<Term<FullEventProxy>>>,
    /// Sender to push input/resize/shutdown messages to the PTY event loop.
    event_loop_sender: EventLoopSender,
    /// Receiver for terminal events (title change, bell, etc.).
    event_rx: Receiver<TerminalEvent>,
    /// Receiver for PtyWrite events that need to be forwarded to the PTY.
    pty_write_rx: Receiver<String>,
    /// Handle to the PTY reader thread.
    _pty_thread: Option<JoinHandle<(EventLoop<tty::Pty, FullEventProxy>, alacritty_terminal::event_loop::State)>>,
    /// Terminal dimensions.
    cols: u16,
    rows: u16,
    /// Our configured color palette for resolving named/indexed colors.
    /// This is separate from the terminal's colors, which can be modified by apps.
    configured_colors: ConfiguredColors,
    /// PID of the child shell process.
    child_pid: u32,
}

/// Our configured color palette, used as the base for color resolution.
struct ConfiguredColors {
    foreground: RgbColor,
    background: RgbColor,
    /// Full 256-color palette + named colors, pre-computed.
    palette: [RgbColor; 269],
}

impl ConfiguredColors {
    fn from_config(config: &TerminalConfig) -> Self {
        let mut palette = [RgbColor::new(0, 0, 0); 269];

        // Set 16 ANSI colors from config.
        for (i, c) in config.colors.palette.iter().enumerate() {
            palette[i] = *c;
        }

        // Compute 6x6x6 color cube (indices 16-231).
        for i in 16u16..232 {
            let idx = i - 16;
            let r = (idx / 36) as u8;
            let g = ((idx % 36) / 6) as u8;
            let b = (idx % 6) as u8;
            let to_val = |v: u8| if v == 0 { 0u8 } else { 55 + 40 * v };
            palette[i as usize] = RgbColor::new(to_val(r), to_val(g), to_val(b));
        }

        // Compute grayscale ramp (indices 232-255).
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

        // Dim colors (darken the base 8 colors).
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

    /// Resolve an alacritty Color to an RgbColor.
    fn resolve(
        &self,
        color: AlacColor,
        term_colors: &alacritty_terminal::term::color::Colors,
    ) -> RgbColor {
        match color {
            AlacColor::Spec(rgb) => RgbColor::new(rgb.r, rgb.g, rgb.b),
            AlacColor::Named(named) => {
                // Check if the terminal has overridden this color (apps can change colors).
                if let Some(rgb) = term_colors[named] {
                    RgbColor::new(rgb.r, rgb.g, rgb.b)
                } else {
                    self.palette[named as usize]
                }
            },
            AlacColor::Indexed(idx) => {
                if let Some(rgb) = term_colors[idx as usize] {
                    RgbColor::new(rgb.r, rgb.g, rgb.b)
                } else {
                    self.palette[idx as usize]
                }
            },
        }
    }
}

impl TerminalBackend {
    /// Create a new terminal backend and spawn a shell process.
    ///
    /// `cols` and `rows` are the initial terminal dimensions in cells.
    /// `cell_width` and `cell_height` are pixel dimensions of a single cell
    /// (used for TIOCSWINSZ).
    pub fn new(
        config: TerminalConfig,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<Self, String> {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (pty_write_tx, pty_write_rx) = crossbeam_channel::unbounded();

        let event_proxy = FullEventProxy {
            event_sender: event_tx,
            pty_write_sender: pty_write_tx,
        };

        let alac_config = config.to_alacritty_config();
        let pty_options = config.to_pty_options();
        let configured_colors = ConfiguredColors::from_config(&config);

        // Create the terminal.
        let size = TermSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        let term = Term::new(alac_config, &size, event_proxy.clone());
        let term = Arc::new(FairMutex::new(term));

        // Create the PTY.
        let window_size = WindowSize {
            num_lines: rows,
            num_cols: cols,
            cell_width,
            cell_height,
        };
        let pty = tty::new(&pty_options, window_size, 0)
            .map_err(|e| format!("Failed to create PTY: {}", e))?;

        // Grab the child PID before passing the PTY to the event loop.
        let child_pid = pty.child().id();

        // Create and spawn the event loop.
        let event_loop = EventLoop::new(
            Arc::clone(&term),
            event_proxy,
            pty,
            pty_options.drain_on_exit,
            false,
        )
        .map_err(|e| format!("Failed to create event loop: {}", e))?;

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
            configured_colors,
            child_pid,
        })
    }

    /// Send keyboard input bytes to the PTY.
    pub fn write(&self, data: &[u8]) {
        self.drain_pty_writes();

        if data.is_empty() {
            return;
        }
        let _ = self.event_loop_sender.send(Msg::Input(Cow::Owned(data.to_vec())));
    }

    /// Resize the terminal grid and PTY.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;

        let size = TermSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        self.term.lock().resize(size);

        let window_size = WindowSize {
            num_lines: rows,
            num_cols: cols,
            cell_width,
            cell_height,
        };
        let _ = self.event_loop_sender.send(Msg::Resize(window_size));
    }

    /// Get a snapshot of the visible grid for rendering.
    pub fn grid_snapshot(&self) -> GridSnapshot {
        let term = self.term.lock();
        let content = term.renderable_content();
        let term_colors = content.colors;
        let mode = content.mode;
        let cursor = content.cursor;
        let selection = content.selection;
        let num_cols = term.columns();
        let num_lines = term.screen_lines();

        // Initialize grid with default cells.
        let mut cells: Vec<Vec<StyledCell>> = (0..num_lines)
            .map(|_| {
                (0..num_cols)
                    .map(|_| StyledCell {
                        character: ' ',
                        fg: self.configured_colors.foreground,
                        bg: self.configured_colors.background,
                        flags: CellFlags::NONE,
                    })
                    .collect()
            })
            .collect();

        // Fill from display iterator.
        for indexed in content.display_iter {
            let row = indexed.point.line.0 as usize;
            let col = indexed.point.column.0;

            if row < num_lines && col < num_cols {
                cells[row][col] = StyledCell {
                    character: indexed.cell.c,
                    fg: self.configured_colors.resolve(indexed.cell.fg, term_colors),
                    bg: self.configured_colors.resolve(indexed.cell.bg, term_colors),
                    flags: convert_flags(indexed.cell.flags),
                };
            }
        }

        // Build selection ranges.
        let mut selection_ranges = Vec::new();
        let has_selection = selection.is_some();
        if let Some(sel) = &selection {
            let start_line = sel.start.line.0.max(0) as usize;
            let end_line = (sel.end.line.0.max(0) as usize).min(num_lines.saturating_sub(1));

            for row in start_line..=end_line {
                let start_col = if row == start_line { sel.start.column.0 } else { 0 };
                let end_col = if row == end_line {
                    sel.end.column.0
                } else {
                    num_cols.saturating_sub(1)
                };
                selection_ranges.push((row, start_col, end_col));
            }
        }

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

        let terminal_mode = TerminalMode {
            show_cursor: mode.contains(TermMode::SHOW_CURSOR),
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            mouse_report_click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
            mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
            mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
            mouse_sgr: mode.contains(TermMode::SGR_MOUSE),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            focus_in_out: mode.contains(TermMode::FOCUS_IN_OUT),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            line_wrap: mode.contains(TermMode::LINE_WRAP),
        };

        GridSnapshot {
            cells,
            cursor: cursor_state,
            has_selection,
            selection_ranges,
            cols: num_cols,
            lines: num_lines,
            mode: terminal_mode,
        }
    }

    /// Poll for terminal events (non-blocking).
    pub fn poll_events(&self) -> Vec<TerminalEvent> {
        self.drain_pty_writes();

        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Start a text selection at the given grid position.
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
        term.selection = Some(Selection::new(
            ty,
            point,
            alacritty_terminal::index::Side::Left,
        ));
    }

    /// Update the current selection to the given grid position.
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

    /// Get the selected text as a string.
    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string()
    }

    /// Scroll the viewport.
    ///
    /// Positive `delta` scrolls up (towards history), negative scrolls down.
    pub fn scroll(&self, delta: i32) {
        self.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
    }

    /// Scroll the viewport to the bottom (most recent output).
    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Bottom);
    }

    /// Get the current terminal mode flags.
    pub fn mode(&self) -> TerminalMode {
        let mode = *self.term.lock().mode();
        TerminalMode {
            show_cursor: mode.contains(TermMode::SHOW_CURSOR),
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            mouse_report_click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
            mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
            mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
            mouse_sgr: mode.contains(TermMode::SGR_MOUSE),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            focus_in_out: mode.contains(TermMode::FOCUS_IN_OUT),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            line_wrap: mode.contains(TermMode::LINE_WRAP),
        }
    }

    /// Get the current terminal dimensions.
    pub fn dimensions(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// Get the PID of the child shell process.
    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    /// Update the configured color palette.
    pub fn set_colors(&mut self, colors: &crate::TerminalColors) {
        self.configured_colors = ConfiguredColors::from_config_colors(colors);
    }

    /// Notify the terminal about focus change.
    pub fn set_focus(&self, focused: bool) {
        self.term.lock().is_focused = focused;
    }

    /// Shut down the terminal and kill the child process.
    pub fn shutdown(&self) {
        let _ = self.event_loop_sender.send(Msg::Shutdown);
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Drain pending PtyWrite events and forward them to the PTY.
    fn drain_pty_writes(&self) {
        while let Ok(text) = self.pty_write_rx.try_recv() {
            let _ = self
                .event_loop_sender
                .send(Msg::Input(Cow::Owned(text.into_bytes())));
        }
    }
}

impl ConfiguredColors {
    fn from_config_colors(colors: &crate::TerminalColors) -> Self {
        // Build a minimal TerminalConfig just for colors.
        let config = TerminalConfig {
            colors: crate::TerminalColors {
                foreground: colors.foreground,
                background: colors.background,
                palette: colors.palette,
            },
            ..Default::default()
        };
        Self::from_config(&config)
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Selection kind for `start_selection()`.
#[derive(Clone, Copy, Debug)]
pub enum SelectionKind {
    /// Click-and-drag character-level selection.
    Simple,
    /// Block/column selection.
    Block,
    /// Word-level (double-click) selection.
    Semantic,
    /// Line-level (triple-click) selection.
    Lines,
}

/// Convert alacritty cell flags to our CellFlags.
fn convert_flags(flags: AlacFlags) -> CellFlags {
    let mut result = CellFlags::NONE;
    if flags.contains(AlacFlags::BOLD) {
        result.insert(CellFlags::BOLD);
    }
    if flags.contains(AlacFlags::ITALIC) {
        result.insert(CellFlags::ITALIC);
    }
    if flags.contains(AlacFlags::UNDERLINE) {
        result.insert(CellFlags::UNDERLINE);
    }
    if flags.contains(AlacFlags::STRIKEOUT) {
        result.insert(CellFlags::STRIKETHROUGH);
    }
    if flags.contains(AlacFlags::DIM) {
        result.insert(CellFlags::DIM);
    }
    if flags.contains(AlacFlags::INVERSE) {
        result.insert(CellFlags::INVERSE);
    }
    if flags.contains(AlacFlags::HIDDEN) {
        result.insert(CellFlags::HIDDEN);
    }
    if flags.contains(AlacFlags::WIDE_CHAR) {
        result.insert(CellFlags::WIDE_CHAR);
    }
    if flags.contains(AlacFlags::WIDE_CHAR_SPACER) {
        result.insert(CellFlags::WIDE_CHAR_SPACER);
    }
    if flags.contains(AlacFlags::DOUBLE_UNDERLINE) {
        result.insert(CellFlags::DOUBLE_UNDERLINE);
    }
    if flags.contains(AlacFlags::UNDERCURL) {
        result.insert(CellFlags::UNDERCURL);
    }
    if flags.contains(AlacFlags::DOTTED_UNDERLINE) {
        result.insert(CellFlags::DOTTED_UNDERLINE);
    }
    if flags.contains(AlacFlags::DASHED_UNDERLINE) {
        result.insert(CellFlags::DASHED_UNDERLINE);
    }
    result
}
