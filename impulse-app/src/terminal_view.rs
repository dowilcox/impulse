//! A terminal pane: `impulse-terminal` drives the PTY and grid, and this
//! paints it the way `TerminalRenderer.swift` does: cell backgrounds in
//! runs, text on a fixed cell grid, the selection and search matches, the
//! cursor (hollow when the pane isn't focused), and command-block
//! decorations (a hairline above each block, a wash and left stripe for
//! failed or running ones, exit status and duration at the right of the
//! block's first row). Keys go through `impulse_kit::key_encoder`; typed
//! text and input methods through GPUI's text input; the mouse selects,
//! scrolls history, or reports to programs that ask for it.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::{
    actions, canvas, div, fill, point, prelude::*, px, size, App, Bounds, ClipboardItem, Context,
    ElementInputHandler, EntityInputHandler, FocusHandle, Font, FontStyle, FontWeight, Hsla,
    KeyBinding, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Rgba, ScrollDelta, ScrollWheelEvent, SharedString, StrikethroughStyle,
    TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use impulse_kit::chrome::{ChromePalette, Rgba as KitRgba};
use impulse_kit::key_encoder::{self, KeyPress};
use impulse_kit::theme::ResolvedTheme;
use impulse_terminal::{
    BlockOverlay, CellFlags, RgbColor, SelectionKind, TerminalBackend, TerminalColors,
    TerminalConfig, TerminalEvent, TerminalMode, CELL_STRIDE, FIXED_HEADER_SIZE, RANGE_ENTRY_SIZE,
};

use crate::settings_store::SettingsStore;

/// Same inset as the Swift renderer's `padding`.
const PADDING: f32 = 12.0;

actions!(
    terminal,
    [
        Copy,
        Paste,
        SelectAll,
        Clear,
        ScrollPageUp,
        ScrollPageDown,
        ScrollToTop,
        ScrollToBottom
    ]
);

/// The key context of a terminal pane.
pub const CONTEXT: &str = "Terminal";

pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    let mut bindings = vec![
        KeyBinding::new("shift-pageup", ScrollPageUp, c),
        KeyBinding::new("shift-pagedown", ScrollPageDown, c),
        KeyBinding::new("shift-home", ScrollToTop, c),
        KeyBinding::new("shift-end", ScrollToBottom, c),
    ];
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("cmd-c", Copy, c),
            KeyBinding::new("cmd-v", Paste, c),
            KeyBinding::new("cmd-a", SelectAll, c),
            KeyBinding::new("cmd-k", Clear, c),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-shift-c", Copy, c),
            KeyBinding::new("ctrl-shift-v", Paste, c),
            KeyBinding::new("ctrl-shift-a", SelectAll, c),
            KeyBinding::new("ctrl-shift-k", Clear, c),
        ]);
    }
    cx.bind_keys(bindings);
}

pub fn rgba(c: KitRgba) -> Rgba {
    Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

fn cell_hsla(c: RgbColor) -> Hsla {
    Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

fn rgb_color(hex: &str) -> RgbColor {
    let c = KitRgba::from_hex(hex);
    RgbColor::new(
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
    )
}

/// One visible cell, decoded from the grid snapshot.
#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    fg: RgbColor,
    bg: RgbColor,
    flags: CellFlags,
}

/// A highlighted span of one row, columns inclusive.
#[derive(Clone, Copy)]
struct Span {
    row: usize,
    start: usize,
    end: usize,
}

/// The grid snapshot `write_grid_to_buffer` fills (the same bytes the Swift
/// `GridBufferReader` parses).
struct Grid {
    cols: usize,
    rows: usize,
    cursor: Option<(usize, usize, u8)>,
    selection: Vec<Span>,
    matches: Vec<Span>,
    cells: Vec<Cell>,
}

impl Grid {
    fn read(buf: &[u8]) -> Option<Grid> {
        if buf.len() < FIXED_HEADER_SIZE {
            return None;
        }
        let u16_at = |at: usize| u16::from_le_bytes([buf[at], buf[at + 1]]) as usize;
        let (cols, rows) = (u16_at(0), u16_at(2));
        let (cursor_row, cursor_col, shape, visible) = (u16_at(4), u16_at(6), buf[8], buf[9] != 0);
        let (selections, searches) = (u16_at(12), u16_at(14));
        let span = |i: usize| {
            let at = FIXED_HEADER_SIZE + i * RANGE_ENTRY_SIZE;
            Span {
                row: u16_at(at),
                start: u16_at(at + 2),
                end: u16_at(at + 4),
            }
        };
        let start = FIXED_HEADER_SIZE + (selections + searches) * RANGE_ENTRY_SIZE;
        if buf.len() < start + cols * rows * CELL_STRIDE {
            return None;
        }
        let cells = (0..cols * rows)
            .map(|i| {
                let at = start + i * CELL_STRIDE;
                let code = u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]);
                Cell {
                    ch: char::from_u32(code).unwrap_or(' '),
                    fg: RgbColor::new(buf[at + 4], buf[at + 5], buf[at + 6]),
                    bg: RgbColor::new(buf[at + 7], buf[at + 8], buf[at + 9]),
                    flags: CellFlags::from_bits_truncate(u16::from_le_bytes([
                        buf[at + 10],
                        buf[at + 11],
                    ])),
                }
            })
            .collect();
        Some(Grid {
            cols,
            rows,
            cursor: visible.then_some((cursor_row, cursor_col, shape)),
            selection: (0..selections).map(span).collect(),
            matches: (selections..selections + searches).map(span).collect(),
            cells,
        })
    }

    fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.cells[row * self.cols + col]
    }
}

/// Cell size and where text sits in a cell.
#[derive(Clone, Copy)]
struct Metrics {
    width: Pixels,
    height: Pixels,
    font_size: Pixels,
}

pub struct TerminalView {
    backend: TerminalBackend,
    focus: FocusHandle,
    theme: ResolvedTheme,
    palette: ChromePalette,
    font: Font,
    metrics: Metrics,
    size: (u16, u16),
    buffer: Vec<u8>,
    pub title: String,
    pub cwd: String,
    pub last_duration_ms: Option<u64>,
    /// The shell's exit status, once it has exited.
    pub exited: Option<i32>,
    /// A drag is selecting text (rather than reporting to the program).
    selecting: bool,
    /// Wheel movement not yet a whole line.
    scroll_remainder: f32,
    /// Text being composed by an input method, shown at the cursor.
    marked: Option<String>,
    blink_on: bool,
    last_input: Instant,
    /// The shell's startup files, removed when the terminal closes.
    temp_paths: Vec<std::path::PathBuf>,
    _wake: gpui_kit::Task<()>,
    _blink: gpui_kit::Task<()>,
}

/// What a terminal reports to whoever shows it.
pub enum TerminalViewEvent {
    TitleChanged,
    Exited(i32),
    Bell,
}

impl gpui_kit::EventEmitter<TerminalViewEvent> for TerminalView {}

impl TerminalView {
    pub fn new(
        theme: ResolvedTheme,
        palette: ChromePalette,
        cwd: String,
        commands: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::restoring(theme, palette, cwd, commands, None, window, cx)
    }

    /// A terminal that first shows `transcript` (a previous session's
    /// output) under a dim "restored" rule, as the Swift app does.
    pub fn restoring(
        theme: ResolvedTheme,
        palette: ChromePalette,
        cwd: String,
        commands: Vec<String>,
        transcript: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = SettingsStore::get(cx).clone();
        let font = gpui_kit::font(SharedString::from(settings.terminal_font_family.clone()));
        let metrics = measure(&font, settings.terminal_font_size as f32, window);
        let launch = crate::shell_launch::launch(&[]);
        let config = TerminalConfig {
            shell_path: launch.shell.clone(),
            shell_args: launch.args.clone(),
            working_directory: Some(cwd.clone()),
            env_vars: launch.env.clone(),
            colors: colors(&theme),
            scrollback_lines: settings.terminal_scrollback.max(0) as usize,
            minimum_contrast: settings.terminal_minimum_contrast as f32,
            allow_clipboard_write: settings.terminal_allow_osc52_write,
            allow_clipboard_read: settings.terminal_allow_osc52_read,
            restored_transcript: transcript.filter(|t| !t.is_empty()).map(|t| {
                format!("{t}\u{1b}[0;2m── restored from the previous session ──\u{1b}[0m\r\n")
            }),
            ..Default::default()
        };
        let size = (80, 24);
        let backend = TerminalBackend::new(
            config,
            size.0,
            size.1,
            f32::from(metrics.width) as u16,
            f32::from(metrics.height) as u16,
        )
        .expect("terminal backend");
        for command in commands {
            backend.write(format!("{command}\r").as_bytes());
        }

        // Sleep until the backend says there's output (the Swift renderer's
        // wake callback), then take events and damage on the main thread.
        let (wake_tx, wake_rx) = async_channel::bounded::<()>(1);
        backend.set_wakeup_callback(Some(Box::new(move || {
            let _ = wake_tx.try_send(());
        })));
        let wake = cx.spawn(async move |this, cx| {
            while wake_rx.recv().await.is_ok() {
                if this.update(cx, |view, cx| view.drain(cx)).is_err() {
                    break;
                }
            }
        });
        let blink = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(530))
                .await;
            let alive = this.update(cx, |view, cx| {
                // Solid while typing, as macOS text cursors are.
                let blinks = SettingsStore::get(cx).terminal_cursor_blink
                    && view.last_input.elapsed() > Duration::from_millis(600);
                let on = !blinks || !view.blink_on;
                if on != view.blink_on {
                    view.blink_on = on;
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        });

        Self {
            backend,
            focus: cx.focus_handle(),
            theme,
            palette,
            font,
            metrics,
            size,
            buffer: Vec::new(),
            title: String::new(),
            temp_paths: launch.temp_paths,
            cwd,
            last_duration_ms: None,
            exited: None,
            selecting: false,
            scroll_remainder: 0.0,
            marked: None,
            blink_on: true,
            last_input: Instant::now(),
            _wake: wake,
            _blink: blink,
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// About the last `max_rows` rows of output, with colors, for the
    /// session.
    pub fn transcript(&self, max_rows: usize) -> String {
        self.backend.transcript(max_rows, true)
    }

    pub fn write(&self, bytes: &[u8]) {
        self.backend.write(bytes);
    }

    /// The program in the foreground, if it isn't the shell.
    pub fn is_busy(&self) -> bool {
        self.backend
            .foreground_pid()
            .is_some_and(|pid| pid as u32 != self.backend.child_pid())
    }

    /// New theme or terminal settings.
    pub fn set_theme(
        &mut self,
        theme: ResolvedTheme,
        palette: ChromePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = SettingsStore::get(cx).clone();
        let mut config = TerminalConfig {
            colors: colors(&theme),
            minimum_contrast: settings.terminal_minimum_contrast as f32,
            ..Default::default()
        };
        config.scrollback_lines = settings.terminal_scrollback.max(0) as usize;
        self.backend.set_colors(&config);
        self.font = gpui_kit::font(SharedString::from(settings.terminal_font_family.clone()));
        self.metrics = measure(&self.font, settings.terminal_font_size as f32, window);
        self.theme = theme;
        self.palette = palette;
        cx.notify();
    }

    /// Events and damage since the last wake.
    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for event in self.backend.poll_events() {
            changed = true;
            match event {
                TerminalEvent::CwdChanged(path) => self.cwd = path,
                TerminalEvent::TitleChanged(title) => {
                    self.title = title;
                    cx.emit(TerminalViewEvent::TitleChanged);
                }
                TerminalEvent::ResetTitle => {
                    self.title.clear();
                    cx.emit(TerminalViewEvent::TitleChanged);
                }
                TerminalEvent::CommandBlockEnded(block) => {
                    self.last_duration_ms = block
                        .ended_at_ms
                        .map(|end| end.saturating_sub(block.started_at_ms));
                }
                TerminalEvent::ClipboardStore(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                TerminalEvent::ClipboardLoad => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        use std::io::Write as _;
                        let mut reply = Vec::new();
                        let _ = write!(reply, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
                        self.backend.write(&reply);
                    }
                }
                TerminalEvent::Bell => cx.emit(TerminalViewEvent::Bell),
                TerminalEvent::ChildExited(code) => {
                    self.exited = Some(code);
                    cx.emit(TerminalViewEvent::Exited(code));
                }
                _ => {}
            }
        }
        if !matches!(self.backend.take_damage(), Some(rows) if rows.is_empty()) {
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// Fit the grid to the pane.
    fn fit(&mut self, bounds: Bounds<Pixels>) {
        let m = self.metrics;
        let cols = ((f32::from(bounds.size.width) - PADDING * 2.0) / f32::from(m.width))
            .floor()
            .max(2.0) as u16;
        let rows = ((f32::from(bounds.size.height) - PADDING) / f32::from(m.height))
            .floor()
            .max(1.0) as u16;
        if (cols, rows) != self.size {
            self.size = (cols, rows);
            self.backend.resize(
                cols,
                rows,
                f32::from(m.width) as u16,
                f32::from(m.height) as u16,
            );
        }
    }

    fn snapshot(&mut self) -> (Option<Grid>, BlockOverlay, TerminalMode) {
        let needed = self.backend.grid_buffer_size();
        if self.buffer.len() < needed {
            self.buffer.resize(needed, 0);
        }
        let written = self.backend.write_grid_to_buffer(&mut self.buffer);
        (
            Grid::read(&self.buffer[..written]),
            self.backend.block_overlay(),
            self.backend.mode(),
        )
    }

    /// Typing goes to the bottom of the output, as in other terminals.
    fn typed(&mut self) {
        self.last_input = Instant::now();
        self.blink_on = true;
        if self.backend.display_offset() > 0 {
            self.backend.scroll_to_bottom();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = &k.modifiers;
        let settings = SettingsStore::get(cx);
        let meta =
            key_encoder::alt_is_meta(&settings.terminal_option_as_meta, cfg!(target_os = "macos"));
        let press = KeyPress {
            key: &k.key,
            text: k.key_char.as_deref(),
            ctrl: m.control,
            alt: m.alt,
            shift: m.shift,
            platform: m.platform,
        };
        let app_cursor = self.backend.mode().contains(TerminalMode::APP_CURSOR);
        if let Some(bytes) = key_encoder::encode(&press, app_cursor, meta) {
            self.backend.clear_selection();
            self.backend.write(&bytes);
            self.typed();
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// The cell under `position` (window coordinates), clamped to the grid.
    fn cell_at(&self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> (usize, usize) {
        let x = f32::from(position.x - bounds.origin.x) - PADDING;
        let y = f32::from(position.y - bounds.origin.y) - PADDING / 2.0;
        let col = (x / f32::from(self.metrics.width)).floor().max(0.0) as usize;
        let row = (y / f32::from(self.metrics.height)).floor().max(0.0) as usize;
        (
            col.min(self.size.0.saturating_sub(1) as usize),
            row.min(self.size.1.saturating_sub(1) as usize),
        )
    }

    /// The program wants the mouse (and Shift isn't held to select anyway).
    fn reports_mouse(&self, modifiers: &Modifiers) -> bool {
        !modifiers.shift
            && self.backend.mode().intersects(
                TerminalMode::MOUSE_REPORT_CLICK
                    | TerminalMode::MOUSE_DRAG
                    | TerminalMode::MOUSE_MOTION,
            )
    }

    fn report_mouse(&self, button: u8, col: usize, row: usize, pressed: bool, motion: bool) {
        let code = button + if motion { 32 } else { 0 };
        let bytes = if self.backend.mode().contains(TerminalMode::MOUSE_SGR) {
            format!(
                "\x1b[<{code};{};{}{}",
                col + 1,
                row + 1,
                if pressed { 'M' } else { 'm' }
            )
            .into_bytes()
        } else {
            let code = if pressed { code } else { 3 };
            vec![
                0x1b,
                b'[',
                b'M',
                32 + code,
                (33 + col).min(255) as u8,
                (33 + row).min(255) as u8,
            ]
        };
        self.backend.write(&bytes);
    }

    fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus(window, cx);
        let (col, row) = self.cell_at(event.position, bounds);
        if self.reports_mouse(&event.modifiers) {
            let button = match event.button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                _ => return,
            };
            self.report_mouse(button, col, row, true, false);
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        // ⌘-click (Ctrl-click on Linux) opens a link.
        let opens = if cfg!(target_os = "macos") {
            event.modifiers.platform
        } else {
            event.modifiers.control
        };
        if opens && SettingsStore::get(cx).terminal_allow_hyperlink {
            if let Some(url) = self.backend.hyperlink_at(col, row) {
                cx.open_url(&url);
                return;
            }
        }
        let kind = match event.click_count {
            2 => SelectionKind::Semantic,
            n if n >= 3 => SelectionKind::Lines,
            _ if event.modifiers.alt => SelectionKind::Block,
            _ => SelectionKind::Simple,
        };
        self.backend.start_selection(col, row, kind);
        if event.click_count >= 2 {
            self.backend.update_selection(col, row);
        }
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let (col, row) = self.cell_at(event.position, bounds);
        if event.pressed_button.is_some() && self.reports_mouse(&event.modifiers) {
            if self
                .backend
                .mode()
                .intersects(TerminalMode::MOUSE_DRAG | TerminalMode::MOUSE_MOTION)
            {
                self.report_mouse(0, col, row, true, true);
            }
            return;
        }
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        // Past the top or bottom: scroll history while selecting.
        let y = event.position.y - bounds.origin.y;
        if y < px(0.) {
            self.backend.scroll(1);
        } else if y > bounds.size.height {
            self.backend.scroll(-1);
        }
        self.backend.update_selection(col, row);
        cx.notify();
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.reports_mouse(&event.modifiers) {
            let (col, row) = self.cell_at(event.position, bounds);
            self.report_mouse(0, col, row, false, false);
            return;
        }
        if !self.selecting {
            return;
        }
        self.selecting = false;
        if SettingsStore::get(cx).terminal_copy_on_select {
            if let Some(text) = self.backend.selected_text().filter(|t| !t.is_empty()) {
                #[cfg(target_os = "linux")]
                cx.write_to_primary(ClipboardItem::new_string(text.clone()));
                #[cfg(target_os = "macos")]
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    fn scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / f32::from(self.metrics.height),
        } + self.scroll_remainder;
        let whole = lines.trunc();
        self.scroll_remainder = lines - whole;
        if whole == 0.0 {
            return;
        }
        let mode = self.backend.mode();
        if self.reports_mouse(&event.modifiers) {
            let (col, row) = self.cell_at(event.position, bounds);
            let button = if whole > 0.0 { 64 } else { 65 };
            for _ in 0..whole.abs() as usize {
                self.report_mouse(button, col, row, true, false);
            }
        } else if mode.contains(TerminalMode::ALT_SCREEN) {
            // Full-screen programs without mouse support scroll by arrows.
            let arrow: &[u8] = match (whole > 0.0, mode.contains(TerminalMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            for _ in 0..whole.abs() as usize {
                self.backend.write(arrow);
            }
        } else {
            self.backend.scroll(whole as i32);
        }
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        match self.backend.selected_text().filter(|t| !t.is_empty()) {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            // ⌘C with nothing selected interrupts, as Ctrl+C would.
            None if cfg!(target_os = "macos") => self.backend.write(&[0x03]),
            None => {}
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            let bracketed = self.backend.mode().contains(TerminalMode::BRACKETED_PASTE);
            self.backend
                .write(&key_encoder::paste_bytes(&text, bracketed));
            self.typed();
            cx.notify();
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.backend.select_all();
        cx.notify();
    }

    /// ⌘K: clear the screen and the scrollback, as Terminal.app does.
    fn clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        self.backend.write(b"\x0c");
        cx.notify();
    }

    fn page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.backend.scroll(self.size.1 as i32 - 1);
        cx.notify();
    }

    fn page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.backend.scroll(-(self.size.1 as i32 - 1));
        cx.notify();
    }

    fn scroll_top(&mut self, _: &ScrollToTop, _: &mut Window, cx: &mut Context<Self>) {
        self.backend.scroll(i32::MAX / 2);
        cx.notify();
    }

    fn scroll_bottom(&mut self, _: &ScrollToBottom, _: &mut Window, cx: &mut Context<Self>) {
        self.backend.scroll_to_bottom();
        cx.notify();
    }
}

fn colors(theme: &ResolvedTheme) -> TerminalColors {
    let mut palette16: [RgbColor; 16] = [RgbColor::new(0, 0, 0); 16];
    for (slot, hex) in palette16.iter_mut().zip(theme.terminal_palette.iter()) {
        *slot = rgb_color(hex);
    }
    TerminalColors {
        foreground: rgb_color(&theme.terminal_fg),
        background: rgb_color(&theme.terminal_bg),
        palette: palette16,
    }
}

/// `TerminalFontMetrics`: the cell is the width of "W" and the font's
/// ascent plus descent, rounded up.
fn measure(font: &Font, font_size: f32, window: &Window) -> Metrics {
    let size = px(font_size);
    let text = window.text_system();
    let id = text.resolve_font(font);
    let width = text
        .advance(id, size, 'W')
        .map(|s| s.width)
        .unwrap_or(px(font_size * 0.6));
    let height = text.ascent(id, size) + text.descent(id, size).abs();
    Metrics {
        width: px(f32::from(width).ceil()),
        height: px((f32::from(height) * 1.15).ceil()),
        font_size: size,
    }
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `TerminalRenderer.blockChipText`.
fn block_chip_text(exit_code: Option<i32>, duration_ms: Option<u64>) -> Option<String> {
    let exit_code = exit_code?;
    let mark = if exit_code == 0 {
        "✓".to_string()
    } else {
        format!("✗ {exit_code}")
    };
    Some(match duration_ms {
        Some(ms) => format!("{mark} · {}", format_block_duration(ms)),
        None => mark,
    })
}

/// `TerminalRenderer.formatBlockDuration`.
fn format_block_duration(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let seconds = ms as f64 / 1000.0;
    if seconds < 60.0 {
        return format!("{seconds:.1}s");
    }
    let minutes = seconds as u64 / 60;
    if minutes < 60 {
        return format!("{minutes}m {}s", seconds as u64 % 60);
    }
    format!("{}h {}m", minutes / 60, minutes % 60)
}

/// Everything a frame paints, worked out before painting.
struct Frame {
    grid: Option<Grid>,
    overlay: BlockOverlay,
    mode: TerminalMode,
    metrics: Metrics,
    font: Font,
    theme: ResolvedTheme,
    accent: KitRgba,
    selection: KitRgba,
    focused: bool,
    blink_on: bool,
    cursor_shape: String,
    marked: Option<String>,
}

impl Frame {
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let bg = rgb_color(&self.theme.terminal_bg);
        window.paint_quad(fill(bounds, cell_hsla(bg)));
        let Some(grid) = &self.grid else { return };
        let m = self.metrics;
        let origin = bounds.origin + point(px(PADDING), px(PADDING / 2.0));
        let cell_origin =
            |row: usize, col: usize| origin + point(m.width * col as f32, m.height * row as f32);
        let row_bounds = |row: usize| {
            Bounds::new(
                point(bounds.origin.x, origin.y + m.height * row as f32),
                size(bounds.size.width, m.height),
            )
        };
        let muted = KitRgba::from_hex(&self.theme.fg_muted);
        let red = KitRgba::from_hex(&self.theme.red);
        let decorate = !self.overlay.alt_screen && !self.mode.contains(TerminalMode::ALT_SCREEN);

        // Block washes under everything.
        if decorate {
            for block in &self.overlay.blocks {
                if !(block.failed) {
                    continue;
                }
                let start = block.start_row.max(0) as usize;
                let end = (block.end_row.max(0) as usize).min(grid.rows.saturating_sub(1));
                for row in start..=end {
                    window.paint_quad(fill(row_bounds(row), rgba(red.opacity(0.07))));
                }
            }
        }

        // Cell backgrounds, in runs.
        for row in 0..grid.rows {
            let mut col = 0;
            while col < grid.cols {
                let cell = grid.cell(row, col);
                let span = if cell.flags.contains(CellFlags::WIDE_CHAR) {
                    2
                } else {
                    1
                };
                let cell_bg = if cell.flags.contains(CellFlags::INVERSE) {
                    cell.fg
                } else {
                    cell.bg
                };
                if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) || cell_bg == bg {
                    col += 1;
                    continue;
                }
                let start = col;
                col += span;
                while col < grid.cols {
                    let next = grid.cell(row, col);
                    let next_bg = if next.flags.contains(CellFlags::INVERSE) {
                        next.fg
                    } else {
                        next.bg
                    };
                    if next_bg != cell_bg {
                        break;
                    }
                    col += 1;
                }
                window.paint_quad(fill(
                    Bounds::new(
                        cell_origin(row, start),
                        size(m.width * (col - start) as f32, m.height),
                    ),
                    cell_hsla(cell_bg),
                ));
            }
        }

        // Search matches, then the selection.
        let paint_spans = |spans: &[Span], color: Hsla, window: &mut Window| {
            for span in spans.iter().filter(|s| s.row < grid.rows) {
                let end = span.end.min(grid.cols.saturating_sub(1));
                if end < span.start {
                    continue;
                }
                window.paint_quad(fill(
                    Bounds::new(
                        cell_origin(span.row, span.start),
                        size(m.width * (end - span.start + 1) as f32, m.height),
                    ),
                    color,
                ));
            }
        };
        let yellow = KitRgba::from_hex(&self.theme.yellow);
        paint_spans(&grid.matches, rgba(yellow.opacity(0.35)).into(), window);
        paint_spans(&grid.selection, rgba(self.selection).into(), window);

        // Block separators and stripes.
        if decorate {
            for block in &self.overlay.blocks {
                if block.start_row > 0 && (block.start_row as usize) < grid.rows {
                    let y = origin.y + m.height * block.start_row as f32;
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, px(1.))),
                        rgba(muted.opacity(0.28)),
                    ));
                }
                if block.failed || block.is_running {
                    let color = if block.failed { red } else { self.accent };
                    let start = block.start_row.max(0) as usize;
                    let end = (block.end_row.max(0) as usize).min(grid.rows.saturating_sub(1));
                    if start < grid.rows {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(bounds.origin.x + px(3.), origin.y + m.height * start as f32),
                                size(px(3.), m.height * (end - start + 1) as f32),
                            ),
                            rgba(color),
                        ));
                    }
                }
            }
        }

        // Text: runs of cells sharing a style, each at its first cell's x so
        // every glyph keeps to the grid.
        for row in 0..grid.rows {
            let mut col = 0;
            while col < grid.cols {
                let start = col;
                let first = *grid.cell(row, col);
                let style = text_style(&first);
                let mut text = String::new();
                while col < grid.cols {
                    let cell = grid.cell(row, col);
                    if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                        col += 1;
                        continue;
                    }
                    if text_style(cell) != style
                        || cell.flags.contains(CellFlags::WIDE_CHAR) && !text.is_empty()
                    {
                        break;
                    }
                    let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
                    text.push(ch);
                    col += 1;
                    // A wide character stands alone.
                    if cell.flags.contains(CellFlags::WIDE_CHAR) {
                        break;
                    }
                }
                if text.trim().is_empty() && !style.underline && !style.strike {
                    continue;
                }
                let run = TextRun {
                    len: text.len(),
                    font: Font {
                        weight: if style.bold {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        },
                        style: if style.italic {
                            FontStyle::Italic
                        } else {
                            FontStyle::Normal
                        },
                        ..self.font.clone()
                    },
                    color: style.color,
                    background_color: None,
                    underline: style.underline.then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(style.color),
                        wavy: style.wavy,
                    }),
                    strikethrough: style.strike.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(style.color),
                    }),
                };
                let line = window
                    .text_system()
                    .shape_line(text.into(), m.font_size, &[run], None);
                let _ = line.paint(
                    cell_origin(row, start),
                    m.height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }

        // The cursor: a block behind its character (drawn above as the
        // background color), a beam or underline, or a hollow box when the
        // pane isn't focused.
        if let Some((row, col, _)) = grid
            .cursor
            .filter(|(r, c, _)| *r < grid.rows && *c < grid.cols)
        {
            let cursor = cell_hsla(rgb_color(&self.theme.cursor));
            let at = cell_origin(row, col);
            let wide = grid.cell(row, col).flags.contains(CellFlags::WIDE_CHAR);
            let width = if wide { m.width * 2. } else { m.width };
            if !self.focused {
                window.paint_quad(gpui_kit::outline(
                    Bounds::new(at, size(width, m.height)),
                    cursor,
                    gpui_kit::BorderStyle::Solid,
                ));
            } else if self.blink_on {
                match self.cursor_shape.as_str() {
                    "beam" => {
                        window.paint_quad(fill(Bounds::new(at, size(px(2.), m.height)), cursor))
                    }
                    "underline" => window.paint_quad(fill(
                        Bounds::new(at + point(px(0.), m.height - px(2.)), size(width, px(2.))),
                        cursor,
                    )),
                    _ => {
                        window.paint_quad(fill(Bounds::new(at, size(width, m.height)), cursor));
                        // The character under it, in the background color.
                        let cell = grid.cell(row, col);
                        if cell.ch != ' ' && cell.ch != '\0' {
                            let text: SharedString = cell.ch.to_string().into();
                            let run = TextRun {
                                len: text.len(),
                                font: self.font.clone(),
                                color: cell_hsla(bg),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let line =
                                window
                                    .text_system()
                                    .shape_line(text, m.font_size, &[run], None);
                            let _ = line.paint(at, m.height, TextAlign::Left, None, window, cx);
                        }
                    }
                }
            }
            // Text an input method is composing, underlined at the cursor.
            if let Some(marked) = self.marked.as_ref().filter(|t| !t.is_empty()) {
                let text: SharedString = marked.clone().into();
                let color = cell_hsla(rgb_color(&self.theme.terminal_fg));
                let run = TextRun {
                    len: text.len(),
                    font: self.font.clone(),
                    color,
                    background_color: Some(cell_hsla(bg)),
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(color),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                let line = window
                    .text_system()
                    .shape_line(text, m.font_size, &[run], None);
                let _ = line.paint(at, m.height, TextAlign::Left, None, window, cx);
            }
        }

        // Exit status and duration at the right of each block's first row.
        if decorate {
            for block in &self.overlay.blocks {
                if block.start_row < 0 || block.start_row as usize >= grid.rows {
                    continue;
                }
                let Some(chip) = block_chip_text(block.exit_code, block.duration_ms) else {
                    continue;
                };
                let color: Hsla = if block.failed {
                    rgba(red).into()
                } else {
                    rgba(muted.opacity(0.75)).into()
                };
                let text: SharedString = chip.into();
                let run = TextRun {
                    len: text.len(),
                    font: self.font.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let chip_size = m.font_size * 0.85;
                let line = window
                    .text_system()
                    .shape_line(text, chip_size, &[run], None);
                let x = bounds.origin.x + bounds.size.width - px(PADDING) - line.width;
                let y = origin.y + m.height * block.start_row as f32;
                let _ = line.paint(point(x, y), m.height, TextAlign::Left, None, window, cx);
            }
        }
    }
}

#[derive(PartialEq)]
struct Style {
    color: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    wavy: bool,
    strike: bool,
}

/// A cell's text style.
fn text_style(cell: &Cell) -> Style {
    let f = cell.flags;
    let fg = if f.contains(CellFlags::INVERSE) {
        cell.bg
    } else {
        cell.fg
    };
    let mut color = cell_hsla(fg);
    if f.contains(CellFlags::DIM) {
        color.a *= 0.6;
    }
    if f.contains(CellFlags::HIDDEN) {
        color.a = 0.0;
    }
    Style {
        color,
        bold: f.contains(CellFlags::BOLD),
        italic: f.contains(CellFlags::ITALIC),
        underline: f.intersects(
            CellFlags::UNDERLINE
                | CellFlags::DOUBLE_UNDERLINE
                | CellFlags::UNDERCURL
                | CellFlags::DOTTED_UNDERLINE
                | CellFlags::DASHED_UNDERLINE
                | CellFlags::HYPERLINK,
        ),
        wavy: f.contains(CellFlags::UNDERCURL),
        strike: f.contains(CellFlags::STRIKETHROUGH),
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|t| 0..t.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        if text.is_empty() {
            return;
        }
        self.backend.clear_selection();
        self.backend.write(&key_encoder::paste_bytes(text, false));
        self.typed();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = Some(text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // At the cursor, so candidate windows open beside it.
        let (row, col) = self
            .buffer
            .get(4..8)
            .map(|b| {
                (
                    u16::from_le_bytes([b[0], b[1]]),
                    u16::from_le_bytes([b[2], b[3]]),
                )
            })
            .unwrap_or((0, 0));
        let m = self.metrics;
        Some(Bounds::new(
            element_bounds.origin
                + point(
                    px(PADDING) + m.width * col as f32,
                    px(PADDING / 2.0) + m.height * row as f32,
                ),
            size(m.width, m.height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let focused = self.focus.is_focused(window);
        let bounds_cell = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
        let (down_bounds, move_bounds, up_bounds, wheel_bounds) = (
            bounds_cell.clone(),
            bounds_cell.clone(),
            bounds_cell.clone(),
            bounds_cell.clone(),
        );
        div()
            .id("terminal")
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .size_full()
            .cursor(gpui_kit::CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::scroll_top))
            .on_action(cx.listener(Self::scroll_bottom))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, e: &MouseDownEvent, window, cx| {
                    view.mouse_down(e, down_bounds.get(), window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|view, e: &MouseDownEvent, window, cx| {
                    let bounds = Bounds::default();
                    if view.reports_mouse(&e.modifiers) {
                        view.mouse_down(e, bounds, window, cx);
                    }
                }),
            )
            .on_mouse_move(cx.listener(move |view, e: &MouseMoveEvent, _, cx| {
                view.mouse_move(e, move_bounds.get(), cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |view, e: &MouseUpEvent, _, cx| {
                    view.mouse_up(e, up_bounds.get(), cx)
                }),
            )
            .on_scroll_wheel(cx.listener(move |view, e: &ScrollWheelEvent, _, cx| {
                view.scroll_wheel(e, wheel_bounds.get(), cx)
            }))
            .child(
                canvas(
                    {
                        let view = view.clone();
                        move |bounds, _window, cx| {
                            bounds_cell.set(bounds);
                            view.update(cx, |view, cx| {
                                view.fit(bounds);
                                let (grid, overlay, mode) = view.snapshot();
                                let settings = SettingsStore::get(cx);
                                Frame {
                                    grid,
                                    overlay,
                                    mode,
                                    metrics: view.metrics,
                                    font: view.font.clone(),
                                    theme: view.theme.clone(),
                                    accent: view.palette.accent,
                                    selection: view.palette.selection_strong,
                                    focused,
                                    blink_on: view.blink_on,
                                    cursor_shape: settings.terminal_cursor_shape.clone(),
                                    marked: view.marked.clone(),
                                }
                            })
                        }
                    },
                    move |bounds, frame: Frame, window, cx| {
                        let focus = view.read(cx).focus.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, view.clone()),
                            cx,
                        );
                        frame.paint(bounds, window, cx);
                    },
                )
                .size_full(),
            )
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.backend.shutdown();
        for path in &self.temp_paths {
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
        }
    }
}

impl gpui_kit::Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
