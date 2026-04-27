//! Terminal backend — owns the alacritty_terminal::Term and PTY event loop.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, OnResize, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags as AlacFlags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Processor;
use alacritty_terminal::vte::ansi::{
    Color as AlacColor, CursorShape as AlacCursorShape, NamedColor,
};
use crossbeam_channel::{Receiver, Sender};

use crate::buffer::{self, HighlightRange};
use crate::config::TerminalConfig;
use crate::event::TerminalEvent;
use crate::grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};
use crate::search::{SearchResult, TerminalSearch};

// ---------------------------------------------------------------------------
// Event proxy — bridges alacritty events to our channel
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct EventProxy {
    event_tx: Sender<TerminalEvent>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacEvent) {
        match event {
            AlacEvent::PtyWrite(text) => {
                let _ = self.event_tx.send(TerminalEvent::PtyWrite(text));
            }
            AlacEvent::Wakeup => {
                let _ = self.event_tx.send(TerminalEvent::Wakeup);
            }
            AlacEvent::Title(title) => {
                let _ = self
                    .event_tx
                    .send(TerminalEvent::TitleChanged(sanitize_title(&title)));
            }
            AlacEvent::ResetTitle => {
                let _ = self.event_tx.send(TerminalEvent::ResetTitle);
            }
            AlacEvent::Bell => {
                let _ = self.event_tx.send(TerminalEvent::Bell);
            }
            AlacEvent::Exit => {
                let _ = self.event_tx.send(TerminalEvent::Exit);
            }
            AlacEvent::ChildExit(status) => {
                // ExitStatus -> i32: use code() which returns Option<i32>, defaulting to -1.
                let code = status.code().unwrap_or(-1);
                let _ = self.event_tx.send(TerminalEvent::ChildExited(code));
            }
            AlacEvent::ClipboardStore(_, text) => {
                let _ = self.event_tx.send(TerminalEvent::ClipboardStore(text));
            }
            AlacEvent::ClipboardLoad(_, _) => {
                let _ = self.event_tx.send(TerminalEvent::ClipboardLoad);
            }
            AlacEvent::CursorBlinkingChange => {
                let _ = self.event_tx.send(TerminalEvent::CursorBlinkingChange);
            }
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

    fn resolve(
        &self,
        color: AlacColor,
        term_colors: &alacritty_terminal::term::color::Colors,
    ) -> RgbColor {
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
// BackendMsg — commands sent to the PTY read thread
// ---------------------------------------------------------------------------

/// Messages sent from the main thread to the PTY read thread.
enum BackendMsg {
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    },
    Shutdown,
}

fn flush_pending_input<W: Write>(writer: &mut W, pending: &mut VecDeque<u8>) -> io::Result<()> {
    while !pending.is_empty() {
        let (front, _) = pending.as_slices();
        if front.is_empty() {
            break;
        }

        match writer.write(front) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write PTY input",
                ));
            }
            Ok(written) => {
                pending.drain(..written);
            }
            Err(ref err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// TerminalBackend
// ---------------------------------------------------------------------------

/// The main terminal backend. One instance per terminal tab/split.
pub struct TerminalBackend {
    term: Arc<FairMutex<Term<EventProxy>>>,
    cmd_tx: Sender<BackendMsg>,
    event_rx: Receiver<TerminalEvent>,
    read_thread: Mutex<Option<JoinHandle<()>>>,
    cols: u16,
    rows: u16,
    colors: ConfiguredColors,
    child_pid: u32,
    search: Mutex<TerminalSearch>,
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
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<BackendMsg>();
        let proxy = EventProxy {
            event_tx: event_tx.clone(),
        };

        let alac_config = config.to_alacritty_config();
        let pty_options = config.to_pty_options();
        let colors = ConfiguredColors::from_config(&config);

        let size = TermSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        let term = Term::new(alac_config, &size, proxy);
        let term = Arc::new(FairMutex::new(term));

        let window_size = WindowSize {
            num_lines: rows,
            num_cols: cols,
            cell_width,
            cell_height,
        };
        let pty = tty::new(&pty_options, window_size, 0)
            .map_err(|e| format!("Failed to create PTY: {e}"))?;
        let child_pid = pty.child().id();

        // Clone the PTY fd up front so the reader thread can own its own handle
        // while the writer side keeps using the original. Doing it here lets us
        // surface clone failures as a Result instead of panicking on the
        // background thread.
        let reader_file = pty
            .file()
            .try_clone()
            .map_err(|e| format!("Failed to clone PTY fd: {e}"))?;

        let term_clone = Arc::clone(&term);
        let read_thread = std::thread::Builder::new()
            .name("impulse-pty-reader".into())
            .spawn(move || {
                Self::read_loop(pty, reader_file, term_clone, event_tx, cmd_rx);
            })
            .map_err(|e| format!("Failed to spawn read thread: {e}"))?;

        Ok(Self {
            term,
            cmd_tx,
            event_rx,
            read_thread: Mutex::new(Some(read_thread)),
            cols,
            rows,
            colors,
            child_pid,
            search: Mutex::new(TerminalSearch::new()),
        })
    }

    /// The PTY read loop — runs on a dedicated thread.
    ///
    /// Reads bytes from the PTY, scans for OSC sequences, feeds data to
    /// alacritty's terminal state machine, and processes commands from the
    /// main thread (input, resize, shutdown).
    fn read_loop(
        pty: tty::Pty,
        reader_file: std::fs::File,
        term: Arc<FairMutex<Term<EventProxy>>>,
        event_tx: Sender<TerminalEvent>,
        cmd_rx: Receiver<BackendMsg>,
    ) {
        let mut buf = [0u8; 0x10000]; // 64KB read buffer
        let mut processor: Processor = Processor::new();
        let mut scanner = crate::osc_scanner::OscScanner::new();

        let mut reader = std::io::BufReader::new(reader_file);

        // We need mutable access to the Pty for on_resize and writing.
        // Since we own it, we can use an Arc<Mutex> for the rare write/resize path.
        let pty = Arc::new(std::sync::Mutex::new(pty));
        let pty_for_loop = Arc::clone(&pty);
        let mut pending_input: VecDeque<u8> = VecDeque::new();

        // Helper closure: process a single BackendMsg. Returns false on Shutdown.
        let handle_cmd = |msg: BackendMsg, pending_input: &mut VecDeque<u8>| -> bool {
            match msg {
                BackendMsg::Input(data) => {
                    pending_input.extend(data);
                    true
                }
                BackendMsg::Resize {
                    cols,
                    rows,
                    cell_width,
                    cell_height,
                } => {
                    let ws = WindowSize {
                        num_lines: rows,
                        num_cols: cols,
                        cell_width,
                        cell_height,
                    };
                    if let Ok(mut p) = pty_for_loop.lock() {
                        p.on_resize(ws);
                    }
                    let size = TermSize {
                        columns: cols as usize,
                        screen_lines: rows as usize,
                    };
                    term.lock().resize(size);
                    true
                }
                BackendMsg::Shutdown => false,
            }
        };

        loop {
            // Drain all pending commands first (non-blocking).
            while let Ok(msg) = cmd_rx.try_recv() {
                if !handle_cmd(msg, &mut pending_input) {
                    return;
                }
            }

            if !pending_input.is_empty() {
                if let Ok(p) = pty_for_loop.lock() {
                    let mut file = p.file();
                    if let Err(err) = flush_pending_input(&mut file, &mut pending_input) {
                        log::warn!("failed to write PTY input: {err}");
                        pending_input.clear();
                    }
                }
            }

            // Try to read from PTY (non-blocking since fd is non-blocking).
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = event_tx.send(TerminalEvent::Exit);
                    return;
                }
                Ok(n) => {
                    // Scan for OSC sequences.
                    scanner.scan(&buf[..n]);
                    for osc_event in scanner.drain_events() {
                        match osc_event {
                            crate::osc_scanner::OscEvent::CwdChanged(path) => {
                                let _ = event_tx.send(TerminalEvent::CwdChanged(path));
                            }
                            crate::osc_scanner::OscEvent::PromptStart => {
                                let _ = event_tx.send(TerminalEvent::PromptStart);
                            }
                            crate::osc_scanner::OscEvent::CommandStart => {
                                let _ = event_tx.send(TerminalEvent::CommandStart);
                            }
                            crate::osc_scanner::OscEvent::CommandEnd(code) => {
                                let _ = event_tx.send(TerminalEvent::CommandEnd(code));
                            }
                        }
                    }

                    // Feed bytes to alacritty's terminal state machine.
                    {
                        let mut term_locked = term.lock();
                        processor.advance(&mut *term_locked, &buf[..n]);
                    }

                    // Notify frontend of content change.
                    let _ = event_tx.send(TerminalEvent::Wakeup);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // PTY has no data. Block on the command channel with a
                    // timeout so we wake instantly on input/resize/shutdown
                    // instead of busy-polling with a sleep.
                    crossbeam_channel::select! {
                        recv(cmd_rx) -> msg => {
                            match msg {
                                Ok(msg) => { if !handle_cmd(msg, &mut pending_input) { return; } }
                                Err(_) => return, // Channel closed
                            }
                        }
                        default(std::time::Duration::from_millis(5)) => {
                            // Timeout — retry PTY read.
                        }
                    }
                    continue;
                }
                Err(_) => {
                    let _ = event_tx.send(TerminalEvent::Exit);
                    return;
                }
            }
        }
    }

    /// Send input bytes to the PTY.
    pub fn write(&self, data: &[u8]) {
        if !data.is_empty() {
            let _ = self.cmd_tx.send(BackendMsg::Input(data.to_vec()));
        }
    }

    /// Resize the terminal grid and PTY.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        let _ = self.cmd_tx.send(BackendMsg::Resize {
            cols,
            rows,
            cell_width,
            cell_height,
        });
    }

    /// Poll for terminal events (non-blocking).
    pub fn poll_events(&self) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = self.event_rx.try_recv() {
            match &ev {
                TerminalEvent::PtyWrite(text) => {
                    // PtyWrite responses need to go back to the PTY.
                    let _ = self
                        .cmd_tx
                        .send(BackendMsg::Input(text.as_bytes().to_vec()));
                }
                _ => events.push(ev),
            }
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
        // When the viewport is scrolled into history, `display_iter` emits
        // points with negative line numbers. Shift by `display_offset` so
        // each cell maps to a 0-based viewport row.
        let display_offset = term.grid().display_offset() as i32;

        // Build mode flags.
        let mut mode_flags = TerminalMode::empty();
        if mode.contains(TermMode::SHOW_CURSOR) {
            mode_flags |= TerminalMode::SHOW_CURSOR;
        }
        if mode.contains(TermMode::APP_CURSOR) {
            mode_flags |= TerminalMode::APP_CURSOR;
        }
        if mode.contains(TermMode::APP_KEYPAD) {
            mode_flags |= TerminalMode::APP_KEYPAD;
        }
        if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            mode_flags |= TerminalMode::MOUSE_REPORT_CLICK;
        }
        if mode.contains(TermMode::MOUSE_MOTION) {
            mode_flags |= TerminalMode::MOUSE_MOTION;
        }
        if mode.contains(TermMode::MOUSE_DRAG) {
            mode_flags |= TerminalMode::MOUSE_DRAG;
        }
        if mode.contains(TermMode::SGR_MOUSE) {
            mode_flags |= TerminalMode::MOUSE_SGR;
        }
        if mode.contains(TermMode::BRACKETED_PASTE) {
            mode_flags |= TerminalMode::BRACKETED_PASTE;
        }
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            mode_flags |= TerminalMode::FOCUS_IN_OUT;
        }
        if mode.contains(TermMode::ALT_SCREEN) {
            mode_flags |= TerminalMode::ALT_SCREEN;
        }
        if mode.contains(TermMode::LINE_WRAP) {
            mode_flags |= TerminalMode::LINE_WRAP;
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

        // Selection ranges.
        let mut selection_ranges = Vec::new();
        if let Some(sel) = &content.selection {
            let start_line = sel.start.line.0.max(0) as usize;
            let end_line = (sel.end.line.0.max(0) as usize).min(num_lines.saturating_sub(1));
            for row in start_line..=end_line {
                let sc = if row == start_line {
                    sel.start.column.0
                } else {
                    0
                };
                let ec = if row == end_line {
                    sel.end.column.0
                } else {
                    num_cols.saturating_sub(1)
                };
                selection_ranges.push(HighlightRange {
                    row: row as u16,
                    start_col: sc as u16,
                    end_col: ec as u16,
                });
            }
        }

        let search_ranges = self
            .search
            .lock()
            .map(|mut s| s.visible_matches(&term))
            .unwrap_or_default();

        let required = buffer::buffer_size(
            num_cols as u16,
            num_lines as u16,
            selection_ranges.len() as u16,
            search_ranges.len() as u16,
        );
        if buf.len() < required {
            return 0;
        }

        // Write header.
        let cell_offset = buffer::write_header(
            buf,
            num_cols as u16,
            num_lines as u16,
            &cursor_state,
            mode_flags,
            &selection_ranges,
            &search_ranges,
        );

        // Initialize all cells to space with default colors.
        let default_fg = self.colors.foreground;
        let default_bg = self.colors.background;
        for i in 0..(num_cols * num_lines) {
            buffer::write_cell(
                buf,
                cell_offset + i * buffer::CELL_STRIDE,
                ' ',
                default_fg,
                default_bg,
                CellFlags::empty(),
            );
        }

        // Fill from display iterator.
        for indexed in content.display_iter {
            let row_i32 = indexed.point.line.0 + display_offset;
            let col = indexed.point.column.0;
            if row_i32 >= 0 && (row_i32 as usize) < num_lines && col < num_cols {
                let row = row_i32 as usize;
                let offset = cell_offset + (row * num_cols + col) * buffer::CELL_STRIDE;
                let fg = self.colors.resolve(indexed.cell.fg, term_colors);
                let bg = self.colors.resolve(indexed.cell.bg, term_colors);
                let mut flags = convert_flags(indexed.cell.flags);
                if indexed.cell.hyperlink().is_some() {
                    flags |= CellFlags::HYPERLINK;
                }
                buffer::write_cell(buf, offset, indexed.cell.c, fg, bg, flags);
            }
        }

        required
    }

    /// Calculate the buffer size needed for a grid snapshot.
    pub fn grid_buffer_size(&self) -> usize {
        // Use self.cols/rows (updated synchronously in resize()) rather than
        // querying the term (which is resized asynchronously on the read
        // thread). This prevents a race where the buffer is allocated for the
        // old grid size, then write_grid_to_buffer() sees the new (larger)
        // grid and returns 0 because the buffer is too small.
        let lines = self.rows;
        let cols = self.cols;
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
        term.selection = Some(Selection::new(
            ty,
            point,
            alacritty_terminal::index::Side::Left,
        ));
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

    /// Select the entire terminal buffer, including scrollback.
    pub fn select_all(&self) {
        let mut term = self.term.lock();
        let grid = term.grid();
        if grid.columns() == 0 || grid.total_lines() == 0 {
            return;
        }

        let start = alacritty_terminal::index::Point::new(
            grid.topmost_line(),
            alacritty_terminal::index::Column(0),
        );
        let end = alacritty_terminal::index::Point::new(grid.bottommost_line(), grid.last_column());
        let mut selection = Selection::new(
            SelectionType::Simple,
            start,
            alacritty_terminal::index::Side::Left,
        );
        selection.update(end, alacritty_terminal::index::Side::Right);
        term.selection = Some(selection);
    }

    /// Get the selected text.
    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string()
    }

    /// Scroll the viewport. Positive = up (towards history), negative = down.
    pub fn scroll(&self, delta: i32) {
        self.term
            .lock()
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
    }

    /// Scroll the viewport to the bottom.
    pub fn scroll_to_bottom(&self) {
        self.term
            .lock()
            .scroll_display(alacritty_terminal::grid::Scroll::Bottom);
    }

    /// Update the terminal's color palette at runtime (for live theme changes).
    pub fn set_colors(&mut self, config: &TerminalConfig) {
        self.colors = ConfiguredColors::from_config(config);
    }

    /// Return the hyperlink URI at the given grid cell, if any.
    /// Used by the frontend for hover/click handling on OSC 8 hyperlinks.
    pub fn hyperlink_at(&self, col: usize, row: usize) -> Option<String> {
        let term = self.term.lock();
        let point = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(row as i32),
            alacritty_terminal::index::Column(col),
        );
        let grid = term.grid();
        if row >= grid.screen_lines() || col >= grid.columns() {
            return None;
        }
        let cell = &grid[point];
        cell.hyperlink().map(|h| h.uri().to_string())
    }

    /// Get the current terminal mode flags.
    pub fn mode(&self) -> TerminalMode {
        let mode = *self.term.lock().mode();
        let mut flags = TerminalMode::empty();
        if mode.contains(TermMode::SHOW_CURSOR) {
            flags |= TerminalMode::SHOW_CURSOR;
        }
        if mode.contains(TermMode::APP_CURSOR) {
            flags |= TerminalMode::APP_CURSOR;
        }
        if mode.contains(TermMode::APP_KEYPAD) {
            flags |= TerminalMode::APP_KEYPAD;
        }
        if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            flags |= TerminalMode::MOUSE_REPORT_CLICK;
        }
        if mode.contains(TermMode::MOUSE_MOTION) {
            flags |= TerminalMode::MOUSE_MOTION;
        }
        if mode.contains(TermMode::MOUSE_DRAG) {
            flags |= TerminalMode::MOUSE_DRAG;
        }
        if mode.contains(TermMode::SGR_MOUSE) {
            flags |= TerminalMode::MOUSE_SGR;
        }
        if mode.contains(TermMode::BRACKETED_PASTE) {
            flags |= TerminalMode::BRACKETED_PASTE;
        }
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            flags |= TerminalMode::FOCUS_IN_OUT;
        }
        if mode.contains(TermMode::ALT_SCREEN) {
            flags |= TerminalMode::ALT_SCREEN;
        }
        if mode.contains(TermMode::LINE_WRAP) {
            flags |= TerminalMode::LINE_WRAP;
        }
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
    ///
    /// When the PTY has DECSET 1004 (FOCUS_IN_OUT) enabled, emit the
    /// corresponding `\x1b[I` / `\x1b[O` sequence so TUIs like neovim and tmux
    /// can react to the application gaining or losing focus.
    pub fn set_focus(&self, focused: bool) {
        let emit = {
            let mut term = self.term.lock();
            term.is_focused = focused;
            term.mode().contains(TermMode::FOCUS_IN_OUT)
        };
        if emit {
            let seq: &[u8] = if focused { b"\x1b[I" } else { b"\x1b[O" };
            let _ = self.cmd_tx.send(BackendMsg::Input(seq.to_vec()));
        }
    }

    /// Shut down the terminal and wait for the reader thread to exit so the
    /// `Arc<FairMutex<Term>>` has no outstanding borrows when this backend is
    /// dropped.
    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(BackendMsg::Shutdown);
        let handle = self.read_thread.lock().ok().and_then(|mut h| h.take());
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    /// Search for a regex pattern in the terminal. Returns the first match.
    pub fn search(&self, pattern: &str) -> SearchResult {
        let term = self.term.lock();
        let Ok(mut search) = self.search.lock() else {
            return SearchResult::no_match();
        };
        search.search(&term, pattern)
    }

    /// Find the next search match after the current one.
    pub fn search_next(&self) -> SearchResult {
        let term = self.term.lock();
        let Ok(mut search) = self.search.lock() else {
            return SearchResult::no_match();
        };
        search.search_next(&term)
    }

    /// Find the previous search match before the current one.
    pub fn search_prev(&self) -> SearchResult {
        let term = self.term.lock();
        let Ok(mut search) = self.search.lock() else {
            return SearchResult::no_match();
        };
        search.search_prev(&term)
    }

    /// Clear the current search state.
    pub fn search_clear(&self) {
        if let Ok(mut search) = self.search.lock() {
            search.clear();
        }
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Strip ASCII control characters (except tab and newline) from a shell-set
/// title so embedded NULs don't silently truncate strings on the Swift side.
fn sanitize_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| *c == '\t' || *c == '\n' || (*c >= ' ' && *c != '\x7f'))
        .collect()
}

/// Convert alacritty cell flags to our CellFlags.
fn convert_flags(flags: AlacFlags) -> CellFlags {
    let mut result = CellFlags::empty();
    if flags.contains(AlacFlags::BOLD) {
        result |= CellFlags::BOLD;
    }
    if flags.contains(AlacFlags::ITALIC) {
        result |= CellFlags::ITALIC;
    }
    if flags.contains(AlacFlags::UNDERLINE) {
        result |= CellFlags::UNDERLINE;
    }
    if flags.contains(AlacFlags::STRIKEOUT) {
        result |= CellFlags::STRIKETHROUGH;
    }
    if flags.contains(AlacFlags::DIM) {
        result |= CellFlags::DIM;
    }
    if flags.contains(AlacFlags::INVERSE) {
        result |= CellFlags::INVERSE;
    }
    if flags.contains(AlacFlags::HIDDEN) {
        result |= CellFlags::HIDDEN;
    }
    if flags.contains(AlacFlags::WIDE_CHAR) {
        result |= CellFlags::WIDE_CHAR;
    }
    if flags.contains(AlacFlags::WIDE_CHAR_SPACER) {
        result |= CellFlags::WIDE_CHAR_SPACER;
    }
    if flags.contains(AlacFlags::DOUBLE_UNDERLINE) {
        result |= CellFlags::DOUBLE_UNDERLINE;
    }
    if flags.contains(AlacFlags::UNDERCURL) {
        result |= CellFlags::UNDERCURL;
    }
    if flags.contains(AlacFlags::DOTTED_UNDERLINE) {
        result |= CellFlags::DOTTED_UNDERLINE;
    }
    if flags.contains(AlacFlags::DASHED_UNDERLINE) {
        result |= CellFlags::DASHED_UNDERLINE;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::flush_pending_input;
    use std::collections::VecDeque;
    use std::io::{self, Write};

    struct FlakyWriter {
        output: Vec<u8>,
        calls: usize,
    }

    impl FlakyWriter {
        fn new() -> Self {
            Self {
                output: Vec::new(),
                calls: 0,
            }
        }
    }

    impl Write for FlakyWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            match self.calls {
                1 => {
                    let count = buf.len().min(3);
                    self.output.extend_from_slice(&buf[..count]);
                    Ok(count)
                }
                2 => Err(io::Error::from(io::ErrorKind::WouldBlock)),
                3 => Err(io::Error::from(io::ErrorKind::Interrupted)),
                _ => {
                    let count = buf.len().min(4);
                    self.output.extend_from_slice(&buf[..count]);
                    Ok(count)
                }
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct ZeroWriter;

    impl Write for ZeroWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Ok(0)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn flush_pending_input_preserves_bytes_across_would_block() {
        let mut writer = FlakyWriter::new();
        let mut pending = VecDeque::from(Vec::from(&b"large paste payload"[..]));

        flush_pending_input(&mut writer, &mut pending).unwrap();
        assert_eq!(writer.output, b"lar");
        assert_eq!(pending.len(), b"ge paste payload".len());

        flush_pending_input(&mut writer, &mut pending).unwrap();
        assert_eq!(writer.output, b"large paste payload");
        assert!(pending.is_empty());
    }

    #[test]
    fn flush_pending_input_reports_write_zero() {
        let mut writer = ZeroWriter;
        let mut pending = VecDeque::from(Vec::from(&b"payload"[..]));
        let err = flush_pending_input(&mut writer, &mut pending).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WriteZero);
    }
}
