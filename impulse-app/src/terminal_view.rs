//! A terminal pane: `impulse-terminal` drives the PTY and grid, and this
//! paints it the way `TerminalRenderer.swift` does: cell backgrounds in
//! runs, text on a fixed cell grid, the selection and search matches, the
//! cursor (hollow when the pane isn't focused), and command-block
//! decorations (a hairline above each block, a wash and left stripe for
//! failed or running ones, exit status and duration at the right of the
//! block's first row). Keys go through `impulse_kit::key_encoder`; typed
//! text and input methods through GPUI's text input; the mouse selects,
//! scrolls history, or reports to programs that ask for it.
//!
//! Around the grid: the input bar (`terminal_input_bar`), which stands in
//! for the shell's own prompt, so the prompt isn't drawn and the last
//! output sits against the bar; find (`terminal_find_bar`); and command
//! blocks, hints and links (`terminal_blocks`).

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::{
    actions, canvas, div, fill, point, prelude::*, px, size, AnyElement, App, Bounds,
    ClipboardItem, Context, ElementInputHandler, Entity, EntityId, EntityInputHandler, FocusHandle,
    Font, FontStyle, FontWeight, Hsla, KeyBinding, KeyDownEvent, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, ScrollDelta,
    ScrollWheelEvent, SharedString, StrikethroughStyle, TextAlign, TextRun, UTF16Selection,
    UnderlineStyle, Window,
};
use impulse_kit::chrome::{ChromePalette, Rgba as KitRgba};
use impulse_kit::key_encoder::{self, KeyPress};
use impulse_kit::shell::lookup::CommandLookup;
use impulse_kit::shell::ShellType;
use impulse_kit::terminal_find::TerminalFindQuery;
use impulse_kit::theme::ResolvedTheme;
use impulse_terminal::{
    BlockOverlay, CellFlags, RgbColor, SelectionKind, TerminalBackend, TerminalColors,
    TerminalConfig, TerminalEvent, TerminalMode, CELL_STRIDE, FIXED_HEADER_SIZE, RANGE_ENTRY_SIZE,
};

use crate::command_editor::{CommandEditor, EditorColors};
use crate::command_history::CommandHistory;
use crate::settings_store::SettingsStore;
use crate::terminal_blocks::{Blocks, FileChecks, Hints, HoverLink, RowText, ToolbarButton};
use crate::terminal_find_bar::FindBar;
use crate::terminal_input_bar::InputBar;

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
        // Tab is the shell's (the window would move focus with it).
        KeyBinding::new("tab", gpui_kit::NoAction, c),
        KeyBinding::new("shift-tab", gpui_kit::NoAction, c),
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
pub(crate) struct Cell {
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
pub(crate) struct Grid {
    pub cols: usize,
    pub rows: usize,
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

    /// No visible glyph on `row` (only spaces and NULs).
    fn is_blank_row(&self, row: usize) -> bool {
        (0..self.cols).all(|col| {
            let cell = self.cell(row, col);
            cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) || cell.ch == ' ' || cell.ch == '\0'
        })
    }

    /// The last row at or above `up_to` with a glyph on it.
    fn last_non_blank_row(&self, up_to: i32) -> i32 {
        let mut row = up_to.min(self.rows as i32 - 1);
        while row >= 0 && self.is_blank_row(row as usize) {
            row -= 1;
        }
        row
    }

    /// Row `row` as text (NUL reads as a space; wide-character spacers
    /// are skipped).
    pub fn row_text(&self, row: usize) -> RowText {
        let mut text = String::new();
        let mut col_for_byte = Vec::new();
        let mut byte_for_col = Vec::new();
        let mut hyperlink = Vec::new();
        for col in 0..self.cols {
            let cell = self.cell(row, col);
            hyperlink.push(cell.flags.contains(CellFlags::HYPERLINK));
            if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                byte_for_col.push(None);
                continue;
            }
            byte_for_col.push(Some(text.len()));
            let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
            text.push(ch);
            col_for_byte.extend(std::iter::repeat_n(col, ch.len_utf8()));
        }
        RowText {
            text,
            col_for_byte,
            byte_for_col,
            hyperlink,
        }
    }
}

/// Cell size and where text sits in a cell.
#[derive(Clone, Copy)]
struct Metrics {
    width: Pixels,
    height: Pixels,
    font_size: Pixels,
}

/// Hiding the shell's prompt below the last output once a command ends
/// (the Swift renderer's prompt tuck): with a screenful of output the grid
/// scrolls up until the last output meets the input bar, and scrolling
/// back down stops there.
#[derive(Default)]
struct Tuck {
    /// Armed when a command finishes; done once the prompt is out of view.
    armed: bool,
    pending: bool,
    scrolled: bool,
    /// The display offset scrolling down stops at, at an idle prompt.
    floor: Option<usize>,
}

pub struct TerminalView {
    pub(crate) backend: TerminalBackend,
    /// The grid's keyboard focus.
    pub(crate) focus: FocusHandle,
    /// The whole pane's: the grid, the input bar and the find bar.
    root_focus: FocusHandle,
    pub(crate) entity_id: EntityId,
    pub(crate) theme: ResolvedTheme,
    pub(crate) palette: ChromePalette,
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
    /// `IMPULSE_PANE_TOKEN`: how `impulse` in this terminal names it.
    pub token: String,
    /// The shell's startup files, removed when the terminal closes.
    temp_paths: Vec<std::path::PathBuf>,

    // The input bar.
    pub(crate) editor: Entity<CommandEditor>,
    pub(crate) bar: InputBar,
    /// How the shell quotes (completions, inserted paths).
    pub(crate) shell: ShellType,
    pub(crate) shell_name: String,
    /// What the shell can run, from its integration.
    pub(crate) lookup: CommandLookup,
    pub(crate) branch: Option<String>,
    /// Between a command block's start and end (OSC 133).
    pub(crate) command_running: bool,
    /// The shell reports its prompts, so `command_running` false means it
    /// waits at one.
    tracks_prompts: bool,
    /// The program reads a password (echo off): the bar masks input.
    pub(crate) password: bool,
    pub(crate) last_exit: Option<i32>,
    /// A full-screen program, or a running command that asked for raw
    /// input (bracketed paste, the mouse), owns the grid: the bar hides and
    /// keys go straight to it.
    pub(crate) direct: bool,
    /// No input bar at all (the Quick Terminal): type at the shell's prompt.
    pub(crate) no_bar: bool,
    /// The command sent from the bar began with a space (kept out of the
    /// history).
    pub(crate) hidden_pending: bool,
    current_hidden: bool,
    current_command: Option<String>,
    /// Blocks whose command was typed with a leading space.
    pub(crate) hidden_blocks: HashSet<u64>,
    session_id: String,
    /// Give the bar the keyboard at the next frame (a command finished).
    focus_bar_pending: bool,

    // Blocks, hints and links.
    pub(crate) blocks: Blocks,
    pub(crate) hints: Option<Hints>,
    hover_link: Option<HoverLink>,
    hover_cell: Option<(usize, usize)>,
    pub(crate) file_checks: FileChecks,

    // Find.
    pub(crate) find: Option<FindBar>,
    pub(crate) last_find: Option<TerminalFindQuery>,

    // The prompt the bar stands in for.
    tuck: Tuck,
    /// The last row drawn (`None`: every row). Rows below it are the
    /// shell's prompt, which the bar replaces.
    pub(crate) last_content_row: Option<usize>,
    /// How far the rows are pushed down so the last one meets the bar.
    y_offset: Pixels,
    /// Rows drawn this frame.
    draw_rows: usize,
    /// The agent running in it, and its attention mark (`agent_tracker`).
    pub(crate) agent: crate::agent_tracker::AgentTracker,
    _wake: gpui_kit::Task<()>,
    _blink: gpui_kit::Task<()>,
}

/// What a terminal reports to whoever shows it.
pub enum TerminalViewEvent {
    TitleChanged,
    Exited(i32),
    Bell,
    /// Open a file in an editor tab (a link or hint), at a 1-based line and
    /// column.
    OpenFile {
        path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
    },
    /// Run a window command (Command History from the bar).
    RunCommand(String),
    /// A message for the status bar.
    Notice(String),
    /// A command finished, with its exit status when the shell said.
    CommandEnded(Option<i32>),
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
        let token = crate::control_server::new_token();
        let mut env = crate::control_server::terminal_env(
            &token,
            SettingsStore::get(cx).terminal_editor_integration,
        );
        // In a task: which one, its slot and the main checkout.
        env.extend(crate::tasks::identity(&cwd));
        let launch = crate::shell_launch::launch(&env);
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

        let shell_path = impulse_kit::shell::login::default_shell_path();
        let lookup = CommandLookup::new();
        let editor = cx.new(|cx| {
            let mut editor = CommandEditor::new(false, &theme, cx);
            editor.font = font.clone();
            editor.font_size = px((settings.terminal_font_size - 1).max(6) as f32);
            editor.placeholder = "Run a command…".into();
            editor.lookup = Some((lookup.clone(), Some(cwd.clone())));
            editor
        });
        cx.subscribe_in(&editor, window, Self::on_editor_event)
            .detach();
        let session_id = format!(
            "{:x}-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
            cx.entity_id().as_u64()
        );

        let mut view = Self {
            backend,
            focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            entity_id: cx.entity_id(),
            theme,
            palette,
            font,
            metrics,
            size,
            buffer: Vec::new(),
            title: String::new(),
            temp_paths: launch.temp_paths,
            token,
            cwd,
            last_duration_ms: None,
            exited: None,
            selecting: false,
            scroll_remainder: 0.0,
            marked: None,
            blink_on: true,
            last_input: Instant::now(),
            editor,
            bar: InputBar::default(),
            shell: ShellType::detect(&shell_path),
            shell_name: impulse_kit::shell::login::default_shell_name(),
            lookup,
            branch: None,
            command_running: false,
            tracks_prompts: false,
            password: false,
            last_exit: None,
            direct: false,
            no_bar: false,
            hidden_pending: false,
            current_hidden: false,
            current_command: None,
            hidden_blocks: HashSet::new(),
            session_id,
            focus_bar_pending: false,
            blocks: Blocks::default(),
            hints: None,
            hover_link: None,
            hover_cell: None,
            file_checks: FileChecks::default(),
            find: None,
            last_find: None,
            tuck: Tuck::default(),
            last_content_row: None,
            y_offset: px(0.),
            draw_rows: usize::MAX,
            agent: Default::default(),
            _wake: wake,
            _blink: blink,
        };
        view.refresh_branch(cx);
        CommandHistory::preload(cx);
        view
    }

    /// The pane's focus (it contains the grid's and the input bar's).
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.root_focus
    }

    /// Give the terminal the keyboard: the input bar when it's showing,
    /// otherwise the grid.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.note_active();
        let grid = self.hints.is_some() || !self.blocks.selection.is_empty() || !self.bar_shown(cx);
        if grid {
            window.focus(&self.focus, cx);
        } else {
            let editor = self.editor.read(cx).focus_handle().clone();
            window.focus(&editor, cx);
        }
    }

    /// At a prompt with the bar showing, typing in the grid belongs in the
    /// bar: in the shell's own line it would sit unseen and run with the
    /// next command.
    fn typing_goes_to_bar(&self, cx: &App) -> bool {
        self.bar_shown(cx) && !self.command_running && self.tracks_prompts
    }

    /// The git branch of the terminal's folder, looked up in the background.
    fn refresh_branch(&mut self, cx: &mut Context<Self>) {
        let dir = self.cwd.clone();
        let task = cx.background_spawn(async move {
            let branch = crate::workspace::git_branch(std::path::Path::new(&dir));
            (!branch.is_empty()).then_some(branch)
        });
        cx.spawn(async move |this, cx| {
            let branch = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.branch != branch {
                    view.branch = branch;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// A fresh grid snapshot (rows for links and hints).
    pub(crate) fn grid_now(&mut self) -> Option<Grid> {
        self.snapshot().0
    }

    /// About the last `max_rows` rows of output, with colors, for the
    /// session.
    pub fn transcript(&self, max_rows: usize) -> String {
        self.backend.transcript(max_rows, true)
    }

    pub fn write(&self, bytes: &[u8]) {
        self.backend.write(bytes);
    }

    /// The shell's process id.
    pub fn shell_pid(&self) -> i32 {
        self.backend.child_pid() as i32
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
        let colors = EditorColors::new(&theme);
        self.editor
            .update(cx, |editor, _| editor.colors = colors.clone());
        if let Some(find) = &self.find {
            find.set_colors(colors, cx);
        }
        self.theme = theme;
        self.palette = palette;
        cx.notify();
    }

    /// Events and damage since the last wake.
    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for event in self.backend.poll_events() {
            changed = true;
            self.agent_observe(&event, cx);
            match event {
                TerminalEvent::CwdChanged(path) => {
                    if path != self.cwd {
                        self.cwd = path;
                        self.refresh_branch(cx);
                    }
                }
                TerminalEvent::PromptStart => {
                    // A prompt means nothing runs, even if a command's end
                    // mark was lost (the bar would otherwise stay hidden at
                    // a bare prompt), and nothing reads a password.
                    self.command_running = false;
                    self.tracks_prompts = true;
                    self.password = false;
                }
                TerminalEvent::CommandBlockStarted(block) => {
                    self.command_running = true;
                    self.current_command = block.command.clone();
                    self.current_hidden = std::mem::take(&mut self.hidden_pending);
                    if self.current_hidden {
                        self.hidden_blocks.insert(block.id.0);
                    }
                    self.blocks.highlighted = None;
                    self.blocks.selection.clear();
                    self.close_menu(cx);
                }
                TerminalEvent::PasswordInputChanged(active) => self.password = active,
                TerminalEvent::ShellNames(names) => self.lookup.set_names(names),
                TerminalEvent::ShellPath(path) => self.lookup.set_path(&path),
                TerminalEvent::TitleChanged(title) => {
                    self.title = title;
                    cx.emit(TerminalViewEvent::TitleChanged);
                }
                TerminalEvent::ResetTitle => {
                    self.title.clear();
                    cx.emit(TerminalViewEvent::TitleChanged);
                }
                TerminalEvent::CommandBlockEnded(block) => {
                    self.command_running = false;
                    self.password = false;
                    self.bar.session_revision += 1;
                    self.last_exit = block.exit_code;
                    self.last_duration_ms = block
                        .ended_at_ms
                        .map(|end| end.saturating_sub(block.started_at_ms));
                    if let Some(command) = block.command.clone().or(self.current_command.clone()) {
                        if !self.current_hidden {
                            let cwd = block
                                .cwd
                                .clone()
                                .or_else(|| (!self.cwd.is_empty()).then(|| self.cwd.clone()));
                            CommandHistory::record(
                                &command,
                                cwd,
                                block.exit_code,
                                self.last_duration_ms.map(|d| d as i64),
                                Some(self.session_id.clone()),
                                cx,
                            );
                        }
                    }
                    self.blocks.highlighted = None;
                    // Tuck the prompt away once, when it appears after this
                    // command; the shell is back for the next command.
                    self.tuck = Tuck {
                        armed: true,
                        ..Tuck::default()
                    };
                    self.focus_bar_pending = true;
                    self.refresh_branch(cx);
                    cx.emit(TerminalViewEvent::CommandEnded(block.exit_code));
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
            self.agent_output(cx);
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

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = &k.modifiers;
        if self.hints.is_some() {
            self.hint_key(event, window, cx);
            return cx.stop_propagation();
        }
        if !self.blocks.selection.is_empty() && self.block_selection_key(event, window, cx) {
            return cx.stop_propagation();
        }
        let secondary = if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        };
        // Cmd+Up selects blocks from the grid too, as it does in the bar.
        if !self.direct && k.key == "up" && secondary && !m.shift && !m.alt {
            self.begin_block_selection(window, cx);
            return cx.stop_propagation();
        }
        if self.typing_goes_to_bar(cx) {
            // Return and Escape go back to the bar; text goes in through
            // the input handler (`replace_text_in_range`); other keys mean
            // nothing here and mustn't reach the shell's line.
            if matches!(k.key.as_str(), "enter" | "escape") {
                self.focus(window, cx);
                cx.stop_propagation();
            } else if m.control || m.platform || k.key_char.is_none() {
                cx.stop_propagation();
            }
            return;
        }
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
            self.agent_key(k.key == "enter", cx);
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// The cell under `position` (window coordinates), clamped to the grid.
    fn cell_at(&self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> (usize, usize) {
        let x = f32::from(position.x - bounds.origin.x) - PADDING;
        let y = f32::from(position.y - bounds.origin.y - self.y_offset) - PADDING / 2.0;
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
        if self.blocks.menu.take().is_some() {
            cx.notify();
        }
        if self.hints.is_some() {
            self.hints = None;
            cx.notify();
        }
        // A plain click leaves a block selection.
        let opens = if cfg!(target_os = "macos") {
            event.modifiers.platform
        } else {
            event.modifiers.control
        };
        if !opens && !self.blocks.selection.is_empty() {
            self.end_block_selection(false, window, cx);
        }
        // The grid at a prompt is read-only: the bar keeps the keyboard,
        // and the mouse still selects.
        self.focus(window, cx);
        let (col, row) = self.cell_at(event.position, bounds);
        if opens && event.button == MouseButton::Left {
            // Cmd-click (Ctrl-click on Linux) opens a link; elsewhere on a
            // block it adds the block to the selection or drops it.
            if SettingsStore::get(cx).terminal_allow_hyperlink {
                if let Some(link) = self.link_at(col, row) {
                    self.open_link(link.target, cx);
                    return;
                }
            }
            if let Some(id) = self.block_at_row(row, cx) {
                self.toggle_block_selection(id, cx);
                if !self.blocks.selection.is_empty() {
                    window.focus(&self.focus, cx);
                }
                return;
            }
        }
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
        if event.pressed_button.is_none() {
            self.update_hover(col, row, cx);
        }
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

    /// The command block on viewport row `row` (not the live prompt).
    pub(crate) fn block_at_row(&self, row: usize, cx: &App) -> Option<u64> {
        if self.direct || !SettingsStore::get(cx).terminal_blocks {
            return None;
        }
        let overlay = self.backend.block_overlay();
        if overlay.alt_screen {
            return None;
        }
        let row = row as i32;
        if overlay.prompt_row.is_some_and(|p| row >= p) {
            return None;
        }
        overlay
            .blocks
            .iter()
            .find(|b| row >= b.start_row && row <= b.end_row)
            .map(|b| b.id)
    }

    /// The pointer moved to a cell: the block under it (hover wash and
    /// toolbar) and the link under it (underline, Cmd-click).
    fn update_hover(&mut self, col: usize, row: usize, cx: &mut Context<Self>) {
        let hovered = self.block_at_row(row, cx);
        if hovered != self.blocks.hovered {
            self.blocks.hovered = hovered;
            cx.notify();
        }
        if self.hover_cell == Some((col, row)) {
            return;
        }
        self.hover_cell = Some((col, row));
        let link = if SettingsStore::get(cx).terminal_allow_hyperlink {
            self.link_at(col, row)
        } else {
            None
        };
        if link != self.hover_link {
            self.hover_link = link;
            cx.notify();
        }
    }

    fn mouse_left(&mut self, cx: &mut Context<Self>) {
        self.hover_cell = None;
        if self.blocks.hovered.take().is_some() | self.hover_link.take().is_some() {
            cx.notify();
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
            let mut delta = whole as i32;
            // Scrolling down stops where the tucked-away prompt begins.
            if let (true, Some(floor), false) = (delta < 0, self.tuck.floor, self.command_running) {
                let offset = self.backend.display_offset() as i32;
                delta = delta.max(floor as i32 - offset).min(0);
            }
            if delta != 0 {
                self.backend.scroll(delta);
            }
        }
        self.hover_cell = None;
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        // Selected blocks: each command and its output.
        if !self.blocks.selection.is_empty() {
            return self.copy_selected_blocks(cx);
        }
        match self.backend.selected_text().filter(|t| !t.is_empty()) {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            // ⌘C with nothing selected interrupts, as Ctrl+C would.
            None if cfg!(target_os = "macos") => self.backend.write(&[0x03]),
            None => {}
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            // The bar owns input: what's pasted belongs in it. In the
            // shell's line it would sit unseen and run with the next command.
            if self.bar_shown(cx) {
                return self.insert_into_bar(&text, window, cx);
            }
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
pub(crate) fn format_block_duration(ms: u64) -> String {
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
    /// Block decorations are on and no program owns the grid.
    decorate: bool,
    /// Rows drawn: those above the prompt the input bar stands in for.
    draw_rows: usize,
    y_offset: Pixels,
    /// Blocks moved to or selected (accent wash and stripe).
    emphasized: HashSet<u64>,
    bookmarks: HashSet<u64>,
    hovered: Option<u64>,
    hover_link: Option<HoverLink>,
    /// Hint targets: row, columns, label, and whether it fits what's typed.
    hints: Vec<(usize, usize, usize, String, bool)>,
}

impl Frame {
    /// The sticky header: "❯ command" (cut to fit) and the block's status
    /// on a bar over the top row, with the failed or running stripe.
    fn paint_sticky_header(
        &self,
        block: &impulse_terminal::BlockOverlayRegion,
        command: &str,
        bounds: Bounds<Pixels>,
        (muted, red): (KitRgba, KitRgba),
        window: &mut Window,
        cx: &mut App,
    ) {
        let m = self.metrics;
        let bar_height = m.height + px(6.);
        let top = bounds.origin.y;
        window.paint_quad(fill(
            Bounds::new(bounds.origin, size(bounds.size.width, bar_height)),
            cell_hsla(rgb_color(&self.theme.terminal_bg)),
        ));
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, top + bar_height - px(1.)),
                size(bounds.size.width, px(1.)),
            ),
            rgba(muted.opacity(0.28)),
        ));
        if block.failed || block.is_running {
            let color = if block.failed { red } else { self.accent };
            window.paint_quad(fill(
                Bounds::new(
                    point(bounds.origin.x + px(3.), top),
                    size(px(3.), bar_height),
                ),
                rgba(color),
            ));
        }
        let shape = |text: String, size_scale: f32, color: Hsla, window: &mut Window| {
            let text: SharedString = text.into();
            let run = TextRun {
                len: text.len(),
                font: self.font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text, m.font_size * size_scale, &[run], None)
        };
        let mut status_width = px(0.);
        if let Some(status) = block_chip_text(block.exit_code, block.duration_ms) {
            let color: Hsla = if block.failed {
                rgba(red).into()
            } else {
                rgba(muted).into()
            };
            let line = shape(status, 0.85, color, window);
            status_width = line.width;
            let x = bounds.origin.x + bounds.size.width - px(PADDING) - line.width;
            let _ = line.paint(
                point(x, top + px(3.)),
                m.height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        let room = bounds.size.width - px(PADDING * 2.) - status_width - px(16.);
        let full = format!("\u{276F} {command}");
        let mut chars: Vec<char> = full.chars().collect();
        let mut line = shape(full.clone(), 1.0, rgba(muted).into(), window);
        while line.width > room && chars.len() > 3 {
            chars.pop();
            let cut: String = chars.iter().collect::<String>() + "\u{2026}";
            line = shape(cut, 1.0, rgba(muted).into(), window);
        }
        let _ = line.paint(
            point(bounds.origin.x + px(PADDING), top + px(3.)),
            m.height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let bg = rgb_color(&self.theme.terminal_bg);
        window.paint_quad(fill(bounds, cell_hsla(bg)));
        let Some(grid) = &self.grid else { return };
        let rows = self.draw_rows.min(grid.rows);
        let m = self.metrics;
        let origin = bounds.origin + point(px(PADDING), px(PADDING / 2.0) + self.y_offset);
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
        let fg = KitRgba::from_hex(&self.theme.fg);
        let decorate = self.decorate
            && !self.overlay.alt_screen
            && !self.mode.contains(TerminalMode::ALT_SCREEN);
        let block_rows = |start: i32, end: i32| {
            let start = start.max(0) as usize;
            let end = (end.max(0) as usize).min(rows.saturating_sub(1));
            (start < rows).then_some(start..=end)
        };

        // Block washes under everything: the hovered block, the one moved
        // to or selected, failed ones.
        if decorate {
            for block in &self.overlay.blocks {
                let emphasized = self.emphasized.contains(&block.id);
                let color = if emphasized {
                    self.accent.opacity(0.09)
                } else if block.failed {
                    red.opacity(0.07)
                } else if self.hovered == Some(block.id) {
                    fg.opacity(0.05)
                } else {
                    continue;
                };
                if let Some(range) = block_rows(block.start_row, block.end_row) {
                    for row in range {
                        window.paint_quad(fill(row_bounds(row), rgba(color)));
                    }
                }
            }
        }

        // Cell backgrounds, in runs.
        for row in 0..rows {
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
            for span in spans.iter().filter(|s| s.row < rows) {
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

        // Block separators, stripes and bookmark ribbons.
        if decorate {
            for block in &self.overlay.blocks {
                if block.start_row > 0 && (block.start_row as usize) < rows {
                    let y = origin.y + m.height * block.start_row as f32;
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, px(1.))),
                        rgba(muted.opacity(0.28)),
                    ));
                }
                let emphasized = self.emphasized.contains(&block.id);
                if block.failed || block.is_running || emphasized {
                    let color = if block.failed { red } else { self.accent };
                    if let Some(range) = block_rows(block.start_row, block.end_row) {
                        let (start, end) = (*range.start(), *range.end());
                        window.paint_quad(fill(
                            Bounds::new(
                                point(bounds.origin.x + px(3.), origin.y + m.height * start as f32),
                                size(px(3.), m.height * (end - start + 1) as f32),
                            ),
                            rgba(color),
                        ));
                    }
                }
                if self.bookmarks.contains(&block.id)
                    && block.start_row >= 0
                    && (block.start_row as usize) < rows
                {
                    // A ribbon hanging from the block's top edge in the
                    // right margin.
                    let top = origin.y + m.height * block.start_row as f32;
                    let width = px(7.0f32.min((PADDING - 2.).max(4.)));
                    let left = bounds.origin.x + bounds.size.width - (px(PADDING) + width) / 2.;
                    let mut ribbon = gpui_kit::PathBuilder::fill();
                    ribbon.move_to(point(left, top));
                    ribbon.line_to(point(left + width, top));
                    ribbon.line_to(point(left + width, top + m.height));
                    ribbon.line_to(point(left + width / 2., top + m.height - px(4.)));
                    ribbon.line_to(point(left, top + m.height));
                    ribbon.close();
                    if let Ok(path) = ribbon.build() {
                        window.paint_path(path, rgba(self.accent));
                    }
                }
            }
        }

        // Text: runs of cells sharing a style, each at its first cell's x so
        // every glyph keeps to the grid.
        for row in 0..rows {
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
                let line = window.text_system().shape_line(
                    text.into(),
                    m.font_size,
                    &[run],
                    Some(m.width),
                );
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

        // The link under the pointer, underlined in the cursor color.
        if let Some(link) = self.hover_link.as_ref().filter(|l| l.row < rows) {
            let at = cell_origin(link.row, link.start_col) + point(px(0.), m.height - px(1.));
            window.paint_quad(fill(
                Bounds::new(
                    at,
                    size(m.width * (link.end_col - link.start_col) as f32, px(1.)),
                ),
                cell_hsla(rgb_color(&self.theme.cursor)),
            ));
        }

        // The cursor: a block behind its character (drawn above as the
        // background color), a beam or underline, or a hollow box when the
        // pane isn't focused.
        if let Some((row, col, _)) = grid.cursor.filter(|(r, c, _)| *r < rows && *c < grid.cols) {
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

        // Exit status and duration at the right of each block's first row
        // (the hovered block shows its toolbar there instead), unless
        // glyphs are already there.
        if decorate {
            for block in &self.overlay.blocks {
                if block.start_row < 0
                    || block.start_row as usize >= rows
                    || self.hovered == Some(block.id)
                {
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
                let span = (f32::from(line.width) / f32::from(m.width)).ceil() as usize + 2;
                let row = block.start_row as usize;
                if span >= grid.cols / 2
                    || (grid.cols - span..grid.cols).any(|col| {
                        let ch = grid.cell(row, col).ch;
                        ch != ' ' && ch != '\0'
                    })
                {
                    continue;
                }
                let x = bounds.origin.x + bounds.size.width - px(PADDING) - line.width;
                let y = origin.y + m.height * row as f32;
                let _ = line.paint(point(x, y), m.height, TextAlign::Left, None, window, cx);
            }
        }

        // Warp-style sticky header: when a block starts above the viewport,
        // its command stays along the top edge so you know whose output
        // you're reading.
        if decorate && rows > 1 {
            let pinned = self
                .overlay
                .blocks
                .iter()
                .find(|b| b.start_row < 0 && b.end_row >= 1);
            if let Some((block, command)) = pinned.and_then(|b| {
                let command = b.command.as_deref().filter(|c| !c.is_empty())?;
                Some((b, command))
            }) {
                self.paint_sticky_header(block, command, bounds, (muted, red), window, cx);
            }
        }

        // Hints: each target underlined, its label in a pill over its start.
        for (row, start, end, label, fits) in &self.hints {
            if *row >= rows {
                continue;
            }
            let top = origin.y + m.height * *row as f32;
            let x = origin.x + m.width * *start as f32;
            window.paint_quad(fill(
                Bounds::new(
                    point(x, top + m.height - px(1.5)),
                    size(m.width * (end - start) as f32, px(1.5)),
                ),
                rgba(self.accent.opacity(if *fits { 0.9 } else { 0.25 })),
            ));
            if !fits {
                continue;
            }
            let text: SharedString = label.to_uppercase().into();
            let run = TextRun {
                len: text.len(),
                font: Font {
                    weight: FontWeight::BOLD,
                    ..self.font.clone()
                },
                color: cell_hsla(bg),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_line(text, m.font_size * 0.85, &[run], None);
            let pill = Bounds::new(
                point(x - px(1.), top + px(1.)),
                size(line.width + px(8.), m.height - px(2.)),
            );
            window.paint_quad(fill(pill, rgba(self.accent)).corner_radii(px(3.)));
            let _ = line.paint(
                point(pill.origin.x + px(4.), top),
                m.height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        if text.is_empty() || self.hints.is_some() {
            return;
        }
        if self.typing_goes_to_bar(cx) || !self.blocks.selection.is_empty() && self.bar_shown(cx) {
            // An unbound Ctrl key can arrive as its control character.
            let typed: String = text
                .chars()
                .filter(|c| !impulse_kit::shell::quoting::is_control(*c))
                .collect();
            self.end_block_selection(false, window, cx);
            if !typed.is_empty() {
                self.insert_into_bar(&typed, window, cx);
            }
            return;
        }
        self.backend.clear_selection();
        self.backend.write(&key_encoder::paste_bytes(text, false));
        self.typed();
        self.agent_key(false, cx);
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

impl TerminalView {
    /// Which rows to draw and how far down: with the input bar standing in
    /// for the prompt, an idle shell's prompt isn't drawn and the last
    /// output meets the bar (pushed down while it's shorter than the pane;
    /// with a screenful, the grid scrolls up once to tuck the prompt away).
    /// While a command runs, its output meets the bar until the pane fills.
    fn layout_rows(
        &mut self,
        grid: &Grid,
        overlay: &BlockOverlay,
        suppress: bool,
        cx: &mut Context<Self>,
    ) {
        let rows = grid.rows as i32;
        let mut last = rows - 1;
        let mut anchor = false;
        if !overlay.alt_screen && !self.direct {
            let fills = overlay.blocks.iter().any(|b| b.start_row < 0);
            let at_prompt = overlay.prompt_row.is_some();
            if suppress && at_prompt {
                last = if let Some(block) = overlay.blocks.last() {
                    block.end_row
                } else if overlay.has_blocks {
                    overlay.prompt_row.map_or(rows - 1, |p| p - 1)
                } else {
                    -1
                };
                // Without the blank lines shells print before a prompt.
                last = grid.last_non_blank_row(last);
                anchor = !fills;
                if fills && self.tuck.armed {
                    let hidden = (rows - 1) - last;
                    if hidden <= 0 {
                        self.tuck.armed = false;
                    } else if !self.tuck.pending {
                        self.tuck.pending = true;
                        self.tuck.scrolled = true;
                        cx.spawn(async move |this, cx| {
                            let _ = this.update(cx, |view, cx| {
                                view.backend.scroll(hidden);
                                view.tuck.pending = false;
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                }
            } else if let Some(cursor) = overlay.cursor_row {
                last = cursor;
                anchor = !fills;
                self.tuck.floor = None;
            }
            // Tucked: the prompt is out of view. Scrolling down stops here.
            if self.tuck.armed && self.tuck.scrolled && !self.tuck.pending && !at_prompt {
                self.tuck.armed = false;
                self.tuck.floor = Some(overlay.display_offset.max(0) as usize);
            }
        }
        let last = last.clamp(-1, rows - 1);
        self.last_content_row = (last < rows - 1).then_some(last.max(0) as usize);
        let draw_rows = (last + 1) as usize;
        self.y_offset = if anchor && last >= 0 {
            self.metrics.height * (rows - 1 - last) as f32
        } else {
            px(0.)
        };
        self.draw_rows = draw_rows;
    }

    /// The toolbar over the hovered block: copy output, run again, more.
    fn block_toolbar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.blocks.hovered?;
        if self.direct || self.hints.is_some() {
            return None;
        }
        let overlay = self.backend.block_overlay();
        let block = overlay.blocks.iter().find(|b| b.id == id)?;
        let row = block.start_row.max(0) as usize;
        if row >= self.draw_rows {
            return None;
        }
        let ch = self.metrics.height;
        let muted = KitRgba::from_hex(&self.theme.fg_muted);
        let bg = KitRgba::from_hex(&self.theme.terminal_bg);
        let mut buttons = vec![(ToolbarButton::CopyOutput, "copy", "Copy output")];
        if block.command.as_ref().is_some_and(|c| !c.is_empty()) && !block.is_running {
            buttons.push((ToolbarButton::Rerun, "rotate-ccw", "Rerun command"));
        }
        buttons.push((ToolbarButton::Menu, "ellipsis", "More"));
        let top = px(PADDING / 2.) + self.y_offset + ch * row as f32 + ch * 0.55;
        Some(
            div()
                .absolute()
                .top(top)
                .right(px(PADDING))
                .h(ch)
                .px(px(4.))
                .flex()
                .rounded(ch.min(px(14.)) / 2.)
                .bg(rgba(bg.opacity(0.96)))
                .border_1()
                .border_color(rgba(muted.opacity(0.28)))
                .children(buttons.into_iter().map(|(button, icon, _help)| {
                    div()
                        .id(icon)
                        .w(px(26.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|d| d.bg(rgba(muted.opacity(0.15))))
                        .child(crate::workbench_chrome::icon(icon, 12., muted.opacity(0.8)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, e: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                match button {
                                    ToolbarButton::CopyOutput => {
                                        view.copy_block(id, false, true, cx)
                                    }
                                    ToolbarButton::Rerun => view.rerun_block(id, cx),
                                    ToolbarButton::Menu => {
                                        view.blocks.menu = Some((id, e.position));
                                        cx.notify();
                                    }
                                }
                            }),
                        )
                }))
                .into_any_element(),
        )
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let focused = self.focus.is_focused(window);
        let contains_focus = self.focus_handle().contains_focused(window, cx);
        if contains_focus {
            self.note_active();
        }
        self.agent_note_watched(window, cx);
        // A program taking the grid (or giving it back) moves the keyboard
        // between the grid and the bar.
        let mode = self.backend.mode();
        let raw = mode.intersects(
            TerminalMode::BRACKETED_PASTE
                | TerminalMode::MOUSE_REPORT_CLICK
                | TerminalMode::MOUSE_MOTION
                | TerminalMode::MOUSE_DRAG,
        );
        let direct = mode.contains(TerminalMode::ALT_SCREEN) || self.command_running && raw;
        if direct != self.direct {
            self.direct = direct;
            if direct {
                self.close_menu(cx);
            }
            if contains_focus {
                cx.defer_in(window, |view, window, cx| view.focus(window, cx));
            }
        }
        // Back at the prompt after a command: the bar takes the keyboard.
        if std::mem::take(&mut self.focus_bar_pending)
            && focused
            && self.find.is_none()
            && self.bar_shown(cx)
        {
            cx.defer_in(window, |view, window, cx| view.focus(window, cx));
        }
        let bottom = if self.bar_shown(cx) {
            Some(if self.has_live_bar() {
                self.input_bar(window, cx)
            } else {
                let height = self.bar_height(window, cx);
                self.bar_placeholder(height)
            })
        } else {
            None
        };
        let link_cursor = self.hover_link.is_some();
        let toolbar = self.block_toolbar(cx);
        let bounds_cell = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
        let (down_bounds, move_bounds, up_bounds, wheel_bounds) = (
            bounds_cell.clone(),
            bounds_cell.clone(),
            bounds_cell.clone(),
            bounds_cell.clone(),
        );
        let grid = div()
            .id("terminal")
            .track_focus(&self.focus)
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .when(link_cursor, |d| d.cursor_pointer())
            .when(!link_cursor, |d| d.cursor(gpui_kit::CursorStyle::IBeam))
            .on_key_down(cx.listener(Self::key_down))
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
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                if !hovered {
                    view.mouse_left(cx);
                }
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
                                let settings = SettingsStore::get(cx).clone();
                                let suppress = settings.terminal_context_bar && !view.no_bar;
                                if let Some(grid) = &grid {
                                    view.layout_rows(grid, &overlay, suppress, cx);
                                }
                                let mut emphasized: HashSet<u64> =
                                    view.blocks.selection.iter().copied().collect();
                                emphasized.extend(view.blocks.highlighted);
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
                                    decorate: settings.terminal_blocks && !view.direct,
                                    draw_rows: view.draw_rows,
                                    y_offset: view.y_offset,
                                    emphasized,
                                    bookmarks: view.blocks.bookmarks.clone(),
                                    hovered: view.blocks.hovered,
                                    hover_link: view.hover_link.clone(),
                                    hints: view
                                        .hints
                                        .as_ref()
                                        .map(|h| {
                                            h.targets
                                                .iter()
                                                .map(|t| {
                                                    (
                                                        t.row,
                                                        t.start_col,
                                                        t.end_col,
                                                        t.label.clone(),
                                                        t.label.starts_with(&h.typed),
                                                    )
                                                })
                                                .collect()
                                        })
                                        .unwrap_or_default(),
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
            .children(toolbar);
        div()
            .id("terminal-pane")
            .track_focus(&self.root_focus)
            .key_context(CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::scroll_top))
            .on_action(cx.listener(Self::scroll_bottom))
            .children(self.find_bar(cx))
            .child(grid)
            .children(bottom)
            .children(self.block_menu(cx))
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
        self.root_focus.clone()
    }
}
