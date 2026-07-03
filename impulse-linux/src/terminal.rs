use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk4::cairo::{Context, FontSlant, FontWeight};
use gtk4::glib;
use gtk4::prelude::*;
use impulse_terminal::{
    CellFlags, CommandBlockFlags, CommandHistoryMatchKind, CommandHistoryQuery,
    CommandHistorySearchResult, CursorShape, RgbColor, SelectionKind, TerminalBackend,
    TerminalCommandBlock, TerminalConfig, TerminalEvent, TerminalMode, CELL_STRIDE,
    FIXED_HEADER_SIZE, RANGE_ENTRY_SIZE,
};

use crate::theme::ThemeColors;

const TERMINAL_DATA_KEY: &str = "impulse-terminal-state";
const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 30;
const DEFAULT_CELL_WIDTH: u16 = 9;
const DEFAULT_CELL_HEIGHT: u16 = 18;
const TERMINAL_PADDING: f64 = 8.0;
const FONT_POINT_TO_PIXEL_SCALE: f64 = 96.0 / 72.0;
const ACTIVE_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const IDLE_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(120);
const HIDDEN_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(500);

const FILTERED_LD_VARS: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "LD_DEBUG",
    "LD_PROFILE",
    "LD_DYNAMIC_WEAK",
    "LD_BIND_NOW",
];
const FILTERED_PARENT_COLOR_VARS: &[&str] =
    &["NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE", "FORCE_COLOR"];

/// GTK terminal widget backed by `impulse_terminal::TerminalBackend`.
///
/// The root widget stores its state in object data so surrounding split/tab code
/// can rediscover terminals while walking the GTK widget tree.
pub type Terminal = gtk4::Box;

type TerminalCallback = Box<dyn Fn(&Terminal) + 'static>;

/// Redirects a keystroke declined by the read-only grid to the input bar,
/// forwarding the typed printable character, if any.
type InputRedirect = Box<dyn Fn(Option<char>)>;

pub struct ShellSpawnCache {
    shell_name: String,
    launch: impulse_core::shell::ShellLaunchConfig,
    working_dir: String,
}

impl ShellSpawnCache {
    pub fn new() -> Self {
        let launch = impulse_core::shell::prepare_shell_launch_config().unwrap_or_else(|e| {
            log::error!("Failed to prepare shell launch: {}", e);
            let shell_path = impulse_core::shell::get_default_shell_path();
            impulse_core::shell::ShellLaunchConfig {
                shell_path,
                shell_args: Vec::new(),
                env_vars: HashMap::new(),
                temp_files: Vec::new(),
            }
        });

        let shell_name = std::path::Path::new(&launch.shell_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string();

        let working_dir =
            impulse_core::shell::get_home_directory().unwrap_or_else(|_| "/".to_string());

        Self {
            shell_name,
            launch,
            working_dir,
        }
    }

    pub fn shell_name(&self) -> &str {
        &self.shell_name
    }
}

impl Drop for ShellSpawnCache {
    fn drop(&mut self) {
        impulse_core::shell::cleanup_temp_files(&self.launch.temp_files);
    }
}

struct TerminalState {
    drawing: gtk4::DrawingArea,
    backend: RefCell<Option<TerminalBackend>>,
    grid_buffer: RefCell<Vec<u8>>,
    font_family: RefCell<String>,
    font_size: Cell<i32>,
    cell_width: Cell<u16>,
    cell_height: Cell<u16>,
    scrollback_lines: Cell<usize>,
    cursor_shape: Cell<CursorShape>,
    cursor_blink: Cell<bool>,
    minimum_contrast: Cell<f32>,
    cols: Cell<u16>,
    rows: Cell<u16>,
    current_directory: RefCell<Option<String>>,
    title: RefCell<String>,
    mode_bits: Cell<u16>,
    colors: RefCell<impulse_terminal::TerminalColors>,
    copy_on_select: Rc<Cell<bool>>,
    scroll_on_output: Cell<bool>,
    terminal_bell: Cell<bool>,
    selected_command_block_id: Cell<Option<u64>>,
    /// Warp model: while the context/input bar manages this terminal, the
    /// grid only takes keyboard input when a full-screen/raw TUI owns it.
    input_bar_managed: Cell<bool>,
    /// Called when the read-only grid declines a keystroke; the window wires
    /// this to focus the input bar (forwarding a printable char, if any).
    input_redirect: RefCell<Option<InputRedirect>>,
    /// Last observed grid-interactivity, for surfacing TUI-ownership flips
    /// (alt screen / raw mode) to the input bar as they happen.
    last_grid_interactive: Cell<bool>,
    /// Block currently under the pointer; washed like Warp's hover highlight.
    hovered_block_id: Cell<Option<u64>>,
    /// The hover-toolbar button under the pointer (for hover highlight).
    hovered_toolbar_button: Cell<Option<ToolbarButton>>,
    /// Hit targets for the hover toolbar buttons, rebuilt every frame.
    hover_toolbar_targets: RefCell<Vec<ToolbarTarget>>,
    /// The block overlay from the last frame, kept for pointer hit-testing.
    last_overlay: RefCell<Option<impulse_terminal::BlockOverlay>>,
    blocks_enabled: Cell<bool>,
    block_style: Cell<BlockStyle>,
    is_command_running: Cell<bool>,
    last_command_exit: Cell<Option<i32>>,
    last_command_duration_ms: Cell<Option<u64>>,
    cwd_callbacks: RefCell<Vec<TerminalCallback>>,
    command_block_callbacks: RefCell<Vec<TerminalCallback>>,
    title_callbacks: RefCell<Vec<TerminalCallback>>,
    child_exited_callbacks: RefCell<Vec<TerminalCallback>>,
}

impl TerminalState {
    fn new(drawing: &gtk4::DrawingArea, copy_on_select: Rc<Cell<bool>>) -> Self {
        Self {
            drawing: drawing.clone(),
            backend: RefCell::new(None),
            grid_buffer: RefCell::new(Vec::new()),
            font_family: RefCell::new("monospace".to_string()),
            font_size: Cell::new(14),
            cell_width: Cell::new(DEFAULT_CELL_WIDTH),
            cell_height: Cell::new(DEFAULT_CELL_HEIGHT),
            scrollback_lines: Cell::new(10_000),
            cursor_shape: Cell::new(CursorShape::Block),
            cursor_blink: Cell::new(true),
            minimum_contrast: Cell::new(3.0),
            cols: Cell::new(DEFAULT_COLS),
            rows: Cell::new(DEFAULT_ROWS),
            current_directory: RefCell::new(None),
            title: RefCell::new("Terminal".to_string()),
            mode_bits: Cell::new(0),
            colors: RefCell::new(impulse_terminal::TerminalColors::default()),
            copy_on_select,
            scroll_on_output: Cell::new(true),
            terminal_bell: Cell::new(false),
            selected_command_block_id: Cell::new(None),
            input_bar_managed: Cell::new(false),
            input_redirect: RefCell::new(None),
            last_grid_interactive: Cell::new(false),
            hovered_block_id: Cell::new(None),
            hovered_toolbar_button: Cell::new(None),
            hover_toolbar_targets: RefCell::new(Vec::new()),
            last_overlay: RefCell::new(None),
            blocks_enabled: Cell::new(true),
            block_style: Cell::new(block_style_from_theme(&crate::theme::KANAGAWA)),
            is_command_running: Cell::new(false),
            last_command_exit: Cell::new(None),
            last_command_duration_ms: Cell::new(None),
            cwd_callbacks: RefCell::new(Vec::new()),
            command_block_callbacks: RefCell::new(Vec::new()),
            title_callbacks: RefCell::new(Vec::new()),
            child_exited_callbacks: RefCell::new(Vec::new()),
        }
    }
}

/// Buttons in the per-block hover toolbar (Warp-style).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolbarButton {
    CopyOutput,
    Menu,
}

/// A hover-toolbar button's hit rectangle, rebuilt every frame while a block
/// is hovered.
#[derive(Clone, Copy)]
struct ToolbarTarget {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    button: ToolbarButton,
    block_id: u64,
}

/// Chrome colors for Warp-style command-block decorations, derived from the
/// active theme in `apply_settings`.
#[derive(Clone, Copy)]
struct BlockStyle {
    separator: RgbColor,
    muted_text: RgbColor,
    failed: RgbColor,
    accent: RgbColor,
    prompt_fill: RgbColor,
}

fn block_style_from_theme(theme: &ThemeColors) -> BlockStyle {
    BlockStyle {
        separator: hex_to_rgb(theme.bg_highlight),
        muted_text: hex_to_rgb(theme.comment),
        failed: hex_to_rgb(theme.red),
        accent: hex_to_rgb(theme.blue),
        prompt_fill: hex_to_rgb(theme.fg),
    }
}

pub fn create_terminal(
    settings: &crate::settings::Settings,
    theme: &ThemeColors,
    copy_on_select_flag: Rc<Cell<bool>>,
) -> Terminal {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.set_focusable(true);
    root.add_css_class("terminal-view");

    let drawing = gtk4::DrawingArea::new();
    drawing.set_hexpand(true);
    drawing.set_vexpand(true);
    drawing.set_focusable(false);
    root.append(&drawing);

    let state = Rc::new(TerminalState::new(&drawing, copy_on_select_flag));
    unsafe {
        root.set_data(TERMINAL_DATA_KEY, state.clone());
    }

    apply_settings(&root, settings, theme, &state.copy_on_select);
    install_draw_handler(&drawing, state.clone());
    install_input_handlers(&root);
    install_destroy_handler(&root);
    start_event_poll(&root);

    root
}

pub fn from_widget(widget: &gtk4::Widget) -> Option<Terminal> {
    let terminal = widget.clone().downcast::<gtk4::Box>().ok()?;
    state(&terminal).map(|_| terminal)
}

pub fn apply_settings(
    terminal: &Terminal,
    settings: &crate::settings::Settings,
    theme: &ThemeColors,
    copy_on_select_flag: &Cell<bool>,
) {
    let Some(state) = state(terminal) else {
        return;
    };

    let family = if settings.terminal_font_family.is_empty() {
        "monospace"
    } else {
        &settings.terminal_font_family
    };
    *state.font_family.borrow_mut() = family.to_string();
    state.font_size.set(settings.terminal_font_size.max(8));
    state
        .scrollback_lines
        .set(settings.terminal_scrollback.max(100) as usize);
    state
        .cursor_shape
        .set(parse_cursor_shape(&settings.terminal_cursor_shape));
    state.cursor_blink.set(settings.terminal_cursor_blink);
    state
        .minimum_contrast
        .set(settings.terminal_minimum_contrast.clamp(1.0, 21.0) as f32);
    state
        .scroll_on_output
        .set(settings.terminal_scroll_on_output);
    state.terminal_bell.set(settings.terminal_bell);
    copy_on_select_flag.set(settings.terminal_copy_on_select);
    state.blocks_enabled.set(settings.terminal_blocks);
    state.input_bar_managed.set(settings.terminal_context_bar);
    state.block_style.set(block_style_from_theme(theme));
    *state.colors.borrow_mut() = terminal_colors(theme);

    if let Some(backend) = state.backend.borrow_mut().as_mut() {
        backend.set_colors(&terminal_config(settings, theme, None, None));
    }
    state.drawing.queue_draw();
}

pub fn spawn_shell(terminal: &Terminal, cache: &Rc<ShellSpawnCache>, working_dir: Option<&str>) {
    let Some(state) = state(terminal) else {
        return;
    };
    let dir = working_dir.unwrap_or(&cache.working_dir).to_string();
    let config = backend_config_from_launch(&cache.launch, Some(dir), &state);
    start_backend(terminal, &state, config);
}

pub fn spawn_command(
    terminal: &Terminal,
    command: &str,
    args: &[String],
    working_dir: Option<&str>,
) {
    let Some(state) = state(terminal) else {
        return;
    };
    let fallback_dir = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let mut config = TerminalConfig {
        shell_path: command.to_string(),
        shell_args: args.to_vec(),
        working_directory: Some(working_dir.unwrap_or(&fallback_dir).to_string()),
        scrollback_lines: state.scrollback_lines.get(),
        cursor_shape: state.cursor_shape.get(),
        cursor_blink: state.cursor_blink.get(),
        env_vars: filtered_env_map(),
        colors: state_colors(&state),
        minimum_contrast: state.minimum_contrast.get(),
    };
    config
        .env_vars
        .insert("TERM_PROGRAM".into(), "Impulse".into());
    config.env_vars.insert(
        "TERM_PROGRAM_VERSION".into(),
        env!("CARGO_PKG_VERSION").into(),
    );
    config
        .env_vars
        .insert("TERM".into(), "xterm-256color".into());
    config
        .env_vars
        .insert("COLORTERM".into(), "truecolor".into());
    start_backend(terminal, &state, config);
}

pub fn write(terminal: &Terminal, bytes: &[u8]) {
    if let Some(state) = state(terminal) {
        if let Some(backend) = state.backend.borrow().as_ref() {
            backend.write(bytes);
        }
    }
}

pub fn write_text(terminal: &Terminal, text: &str) {
    write(terminal, text.as_bytes());
}

pub fn paste_from_clipboard(terminal: &Terminal) {
    use gtk4::gdk::prelude::TextureExt;

    let clipboard = terminal.clipboard();
    let formats = clipboard.formats();

    if formats.contains_type(glib::types::Type::STRING) {
        let term = terminal.clone();
        clipboard.read_text_async(None::<&gtk4::gio::Cancellable>, move |result| {
            if let Ok(Some(text)) = result {
                paste_text(&term, &text);
            }
        });
        return;
    }

    if formats.contains_type(gtk4::gdk::Texture::static_type()) {
        let term = terminal.clone();
        clipboard.read_texture_async(None::<&gtk4::gio::Cancellable>, move |result| {
            if let Ok(Some(texture)) = result {
                let tmp = tempfile::Builder::new()
                    .prefix("impulse-clipboard-")
                    .suffix(".png")
                    .tempfile();
                let Ok(tmp) = tmp else {
                    return;
                };
                let persist_path = match tmp.keep() {
                    Ok((_file, path)) => path,
                    Err(e) => {
                        log::error!("Failed to persist temp file: {}", e);
                        return;
                    }
                };
                let path = persist_path.to_string_lossy().to_string();
                match texture.save_to_png(&path) {
                    Ok(()) => {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            if let Err(e) = std::fs::set_permissions(
                                &path,
                                std::fs::Permissions::from_mode(0o600),
                            ) {
                                log::warn!("Failed to set permissions on {}: {}", path, e);
                            }
                        }
                        write_text(&term, &shell_escape(&path));
                    }
                    Err(e) => log::warn!("Failed to save clipboard image: {}", e),
                }
            }
        });
    }
}

pub fn copy_selection(terminal: &Terminal) {
    if let Some(text) = selected_text(terminal) {
        terminal.clipboard().set_text(&text);
    }
}

pub fn selected_text(terminal: &Terminal) -> Option<String> {
    state(terminal)?
        .backend
        .borrow()
        .as_ref()
        .and_then(TerminalBackend::selected_text)
}

pub fn current_directory(terminal: &Terminal) -> Option<String> {
    state(terminal)?.current_directory.borrow().clone()
}

pub fn title(terminal: &Terminal) -> String {
    state(terminal)
        .map(|s| s.title.borrow().clone())
        .unwrap_or_else(|| "Terminal".to_string())
}

/// Whether a command is currently executing (between OSC 133;C and ;D).
pub fn is_command_running(terminal: &Terminal) -> bool {
    state(terminal).is_some_and(|state| state.is_command_running.get())
}

/// Wire the callback invoked when the read-only grid declines a keystroke.
/// The callback receives the typed printable character, if any, so the first
/// keystroke isn't lost while focus moves to the input bar.
pub fn set_input_redirect(terminal: &Terminal, redirect: impl Fn(Option<char>) + 'static) {
    if let Some(state) = state(terminal) {
        *state.input_redirect.borrow_mut() = Some(Box::new(redirect));
    }
}

/// Whether a full-screen/raw TUI currently owns the grid: the alternate
/// screen (vim, htop), or a running command that turned on bracketed-paste
/// or mouse reporting (Claude Code, fzf). A bare prompt has bracketed paste
/// on too, so the running gate keeps the input bar active at the prompt.
/// The input bar hides while this is true.
pub fn tui_owns_grid(terminal: &Terminal) -> bool {
    state(terminal).is_some_and(|state| tui_owns_grid_state(&state))
}

fn tui_owns_grid_state(state: &Rc<TerminalState>) -> bool {
    let bits = state.mode_bits.get();
    let alt_screen = bits & TerminalMode::ALT_SCREEN.bits() != 0;
    let raw_mode = bits
        & (TerminalMode::BRACKETED_PASTE.bits()
            | TerminalMode::MOUSE_REPORT_CLICK.bits()
            | TerminalMode::MOUSE_MOTION.bits()
            | TerminalMode::MOUSE_DRAG.bits())
        != 0;
    alt_screen || (state.is_command_running.get() && raw_mode)
}

/// Whether the grid itself should take keyboard input: the input bar is not
/// managing this terminal, or a TUI owns the grid. Mirrors the macOS
/// renderer's `keyboardInteractive`.
fn grid_keyboard_interactive(state: &Rc<TerminalState>) -> bool {
    !state.input_bar_managed.get() || tui_owns_grid_state(state)
}

/// Exit code and duration of the most recently completed command.
pub fn last_command_status(terminal: &Terminal) -> Option<(i32, Option<u64>)> {
    let state = state(terminal)?;
    let exit = state.last_command_exit.get()?;
    Some((exit, state.last_command_duration_ms.get()))
}

/// "✓ · 1.2s" / "✗ 1 · 3.4s" text for the context bar's status chip.
/// Shares the formatting used by the in-terminal block chips.
pub fn command_status_chip_text(exit_code: i32, duration_ms: Option<u64>) -> String {
    block_chip_text(Some(exit_code), duration_ms).unwrap_or_default()
}

/// Ask the shell to clear the screen (context-bar Clear button).
pub fn clear_screen(terminal: &Terminal) {
    write(terminal, &[0x0C]);
}

/// Open the command-history picker (context-bar History button).
pub fn show_history(terminal: &Terminal) {
    show_command_history_picker(terminal);
}

/// Invoke `f` whenever a command block starts or ends (for the context bar).
pub fn connect_command_block_changed(terminal: &Terminal, f: impl Fn(&Terminal) + 'static) {
    if let Some(state) = state(terminal) {
        state.command_block_callbacks.borrow_mut().push(Box::new(f));
    }
}

pub fn connect_current_directory_changed(terminal: &Terminal, f: impl Fn(&Terminal) + 'static) {
    if let Some(state) = state(terminal) {
        state.cwd_callbacks.borrow_mut().push(Box::new(f));
    }
}

pub fn connect_title_changed(terminal: &Terminal, f: impl Fn(&Terminal) + 'static) {
    if let Some(state) = state(terminal) {
        state.title_callbacks.borrow_mut().push(Box::new(f));
    }
}

pub fn connect_child_exited(terminal: &Terminal, f: impl Fn(&Terminal) + 'static) {
    if let Some(state) = state(terminal) {
        state.child_exited_callbacks.borrow_mut().push(Box::new(f));
    }
}

pub fn search(terminal: &Terminal, pattern: &str) {
    if let Some(state) = state(terminal) {
        if let Some(backend) = state.backend.borrow().as_ref() {
            if pattern.is_empty() {
                backend.search_clear();
            } else {
                backend.search(&regex_escape(pattern));
                backend.search_next();
            }
            refresh_grid(&state);
        }
    }
}

pub fn search_next(terminal: &Terminal) {
    if let Some(state) = state(terminal) {
        if let Some(backend) = state.backend.borrow().as_ref() {
            backend.search_next();
            refresh_grid(&state);
        }
    }
}

pub fn search_previous(terminal: &Terminal) {
    if let Some(state) = state(terminal) {
        if let Some(backend) = state.backend.borrow().as_ref() {
            backend.search_prev();
            refresh_grid(&state);
        }
    }
}

pub fn search_clear(terminal: &Terminal) {
    if let Some(state) = state(terminal) {
        if let Some(backend) = state.backend.borrow().as_ref() {
            backend.search_clear();
            refresh_grid(&state);
        }
    }
}

pub fn command_blocks(terminal: &Terminal) -> Vec<TerminalCommandBlock> {
    state(terminal)
        .and_then(|state| {
            state
                .backend
                .borrow()
                .as_ref()
                .map(TerminalBackend::command_blocks)
        })
        .unwrap_or_default()
}

pub fn command_block_flags(terminal: &Terminal) -> CommandBlockFlags {
    state(terminal)
        .and_then(|state| {
            state
                .backend
                .borrow()
                .as_ref()
                .map(TerminalBackend::command_block_flags)
        })
        .unwrap_or_default()
}

pub fn running_close_risk_command(
    terminal: &Terminal,
) -> Option<impulse_core::close_risk::RunningCommandRisk> {
    let block = command_blocks(terminal)
        .into_iter()
        .rev()
        .find(|block| block.ended_at_ms.is_none())?;
    Some(impulse_core::close_risk::RunningCommandRisk {
        command: block.command,
        cwd: block.cwd.or_else(|| current_directory(terminal)),
        started_at_ms: block.started_at_ms,
    })
}

pub fn copy_last_command(terminal: &Terminal) {
    if let Some(block) = latest_command_block(terminal) {
        if let Some(command) = block.command {
            if !command.trim().is_empty() {
                terminal.clipboard().set_text(&command);
            }
        }
    }
}

pub fn copy_last_command_output(terminal: &Terminal) {
    if let Some(block) = latest_output_block(terminal) {
        if !block.output.is_empty() {
            terminal.clipboard().set_text(&block.output);
        }
    }
}

pub fn rerun_last_command(terminal: &Terminal) {
    if let Some(block) = latest_command_block(terminal) {
        if let Some(command) = block.command {
            rerun_command_text(terminal, &command);
        }
    }
}

/// Recent command strings for the input bar (newest first), for history
/// cycling and ghost autosuggestions.
pub fn recent_commands(terminal: &Terminal, limit: usize) -> Vec<String> {
    let Some(state) = state(terminal) else {
        return Vec::new();
    };
    let commands = state
        .backend
        .borrow()
        .as_ref()
        .map(|backend| backend.recent_command_strings(limit))
        .unwrap_or_default();
    commands
}

fn command_history_search(
    terminal: &Terminal,
    text: &str,
    limit: usize,
) -> Vec<CommandHistorySearchResult> {
    let Some(state) = state(terminal) else {
        return Vec::new();
    };
    let query = CommandHistoryQuery {
        text: text.to_string(),
        cwd: state.current_directory.borrow().clone(),
        session_id: None,
        limit: Some(limit),
    };
    let results = state
        .backend
        .borrow()
        .as_ref()
        .map(|backend| backend.search_command_history(&query))
        .unwrap_or_default();
    results
}

fn has_command_history(terminal: &Terminal) -> bool {
    state(terminal)
        .and_then(|state| {
            state
                .backend
                .borrow()
                .as_ref()
                .map(TerminalBackend::has_command_history)
        })
        .unwrap_or(false)
}

fn rerun_command_text(terminal: &Terminal, command: &str) -> bool {
    state(terminal)
        .and_then(|state| {
            state
                .backend
                .borrow()
                .as_ref()
                .map(|backend| backend.rerun_command(command))
        })
        .unwrap_or(false)
}

pub fn jump_to_previous_command_block(terminal: &Terminal) -> bool {
    let Some(state) = state(terminal) else {
        return false;
    };
    let blocks = navigable_command_blocks(terminal);
    if blocks.is_empty() {
        return false;
    }

    let selected = state.selected_command_block_id.get();
    let target = selected
        .and_then(|selected| {
            blocks
                .iter()
                .position(|block| block.id.0 == selected)
                .and_then(|index| index.checked_sub(1))
        })
        .and_then(|index| blocks.get(index))
        .or_else(|| blocks.last());

    let Some(target) = target else {
        return false;
    };
    scroll_to_command_block(&state, target.id.0)
}

pub fn jump_to_next_command_block(terminal: &Terminal) -> bool {
    let Some(state) = state(terminal) else {
        return false;
    };
    let blocks = navigable_command_blocks(terminal);
    if blocks.is_empty() {
        return false;
    }

    let selected = state.selected_command_block_id.get();
    let target = selected
        .and_then(|selected| {
            blocks
                .iter()
                .position(|block| block.id.0 == selected)
                .map(|index| index + 1)
        })
        .and_then(|index| blocks.get(index));

    if let Some(target) = target {
        return scroll_to_command_block(&state, target.id.0);
    }

    if selected.is_some() {
        state.selected_command_block_id.set(None);
        if let Some(backend) = state.backend.borrow().as_ref() {
            backend.scroll_to_bottom();
            refresh_grid(&state);
            return true;
        }
        return false;
    }

    scroll_to_command_block(&state, blocks[0].id.0)
}

pub fn jump_to_last_failed_command_block(terminal: &Terminal) -> bool {
    let Some(state) = state(terminal) else {
        return false;
    };
    let Some(target) = latest_failed_command_block(terminal) else {
        return false;
    };
    scroll_to_command_block(&state, target.id.0)
}

fn latest_command_block(terminal: &Terminal) -> Option<TerminalCommandBlock> {
    command_blocks(terminal)
        .into_iter()
        .rev()
        .find(has_command_text)
}

fn latest_output_block(terminal: &Terminal) -> Option<TerminalCommandBlock> {
    command_blocks(terminal)
        .into_iter()
        .rev()
        .find(|block| !block.output.is_empty())
}

fn latest_failed_command_block(terminal: &Terminal) -> Option<TerminalCommandBlock> {
    command_blocks(terminal)
        .into_iter()
        .rev()
        .find(is_failed_command_block)
}

fn navigable_command_blocks(terminal: &Terminal) -> Vec<TerminalCommandBlock> {
    command_blocks(terminal)
        .into_iter()
        .filter(|block| has_command_text(block) || !block.output.is_empty())
        .collect()
}

fn has_command_text(block: &TerminalCommandBlock) -> bool {
    block
        .command
        .as_ref()
        .map(|command| !command.trim().is_empty())
        .unwrap_or(false)
}

fn is_failed_command_block(block: &TerminalCommandBlock) -> bool {
    block.exit_code.map(|code| code != 0).unwrap_or(false)
}

fn scroll_to_command_block(state: &Rc<TerminalState>, block_id: u64) -> bool {
    let scrolled = state
        .backend
        .borrow()
        .as_ref()
        .map(|backend| backend.scroll_to_command_block(impulse_terminal::TerminalBlockId(block_id)))
        .unwrap_or(false);
    if scrolled {
        state.selected_command_block_id.set(Some(block_id));
        refresh_grid(state);
    }
    scrolled
}

pub fn set_font(terminal: &Terminal, family: &str, size: i32) {
    if let Some(state) = state(terminal) {
        *state.font_family.borrow_mut() = family.to_string();
        state.font_size.set(size.max(8));
        state.drawing.queue_draw();
    }
}

fn state(terminal: &Terminal) -> Option<Rc<TerminalState>> {
    unsafe {
        terminal
            .data::<Rc<TerminalState>>(TERMINAL_DATA_KEY)
            .map(|ptr| ptr.as_ref().clone())
    }
}

fn state_colors(state: &TerminalState) -> impulse_terminal::TerminalColors {
    clone_terminal_colors(&state.colors.borrow())
}

fn clone_terminal_colors(
    colors: &impulse_terminal::TerminalColors,
) -> impulse_terminal::TerminalColors {
    impulse_terminal::TerminalColors {
        foreground: colors.foreground,
        background: colors.background,
        palette: colors.palette,
    }
}

fn terminal_config(
    settings: &crate::settings::Settings,
    theme: &ThemeColors,
    working_directory: Option<String>,
    launch: Option<&impulse_core::shell::ShellLaunchConfig>,
) -> TerminalConfig {
    let mut config = TerminalConfig {
        scrollback_lines: settings.terminal_scrollback.max(100) as usize,
        cursor_shape: parse_cursor_shape(&settings.terminal_cursor_shape),
        cursor_blink: settings.terminal_cursor_blink,
        working_directory,
        colors: terminal_colors(theme),
        minimum_contrast: settings.terminal_minimum_contrast.clamp(1.0, 21.0) as f32,
        ..TerminalConfig::default()
    };
    if let Some(launch) = launch {
        config.shell_path = launch.shell_path.clone();
        config.shell_args = launch.shell_args.clone();
        config.env_vars = launch.env_vars.clone();
        config
            .env_vars
            .insert("COLORTERM".into(), "truecolor".into());
    }
    config
}

fn backend_config_from_launch(
    launch: &impulse_core::shell::ShellLaunchConfig,
    working_directory: Option<String>,
    state: &TerminalState,
) -> TerminalConfig {
    let mut config = TerminalConfig {
        shell_path: launch.shell_path.clone(),
        shell_args: launch.shell_args.clone(),
        scrollback_lines: state.scrollback_lines.get(),
        cursor_shape: state.cursor_shape.get(),
        cursor_blink: state.cursor_blink.get(),
        env_vars: launch.env_vars.clone(),
        working_directory,
        colors: state_colors(state),
        minimum_contrast: state.minimum_contrast.get(),
    };
    config
        .env_vars
        .insert("COLORTERM".into(), "truecolor".into());
    config
}

fn start_backend(terminal: &Terminal, state: &Rc<TerminalState>, config: TerminalConfig) {
    if let Some(backend) = state.backend.borrow_mut().take() {
        backend.shutdown();
    }
    let cols = state.cols.get().max(2);
    let rows = state.rows.get().max(1);
    match TerminalBackend::new(
        config,
        cols,
        rows,
        state.cell_width.get().max(1),
        state.cell_height.get().max(1),
    ) {
        Ok(backend) => {
            *state.backend.borrow_mut() = Some(backend);
            refresh_grid(state);
            terminal.grab_focus();
        }
        Err(e) => {
            log::error!("Failed to start terminal backend: {}", e);
        }
    }
}

fn install_draw_handler(drawing: &gtk4::DrawingArea, state: Rc<TerminalState>) {
    drawing.set_draw_func(move |_area, cr, width, height| {
        draw_terminal(cr, width, height, &state);
    });
}

fn install_input_handlers(terminal: &Terminal) {
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let term = terminal.clone();
        key_controller.connect_key_pressed(move |_, key, _, modifiers| {
            handle_key_press(&term, key, modifiers)
        });
    }
    terminal.add_controller(key_controller);

    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    {
        let term = terminal.clone();
        scroll.connect_scroll(move |_, _dx, dy| {
            if let Some(state) = state(&term) {
                if let Some(backend) = state.backend.borrow().as_ref() {
                    let delta = if dy < 0.0 { 3 } else { -3 };
                    backend.scroll(delta);
                    refresh_grid(&state);
                    return gtk4::glib::Propagation::Stop;
                }
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    terminal.add_controller(scroll);

    // Pointer tracking for the Warp-style block hover highlight and toolbar.
    let motion = gtk4::EventControllerMotion::new();
    {
        let term = terminal.clone();
        motion.connect_motion(move |_, x, y| update_block_hover(&term, x, y));
    }
    {
        let term = terminal.clone();
        motion.connect_leave(move |_| {
            if let Some(state) = state(&term) {
                let changed = state.hovered_block_id.take().is_some()
                    || state.hovered_toolbar_button.take().is_some();
                if changed {
                    state.drawing.queue_draw();
                }
            }
        });
    }
    terminal.add_controller(motion);

    let click = gtk4::GestureClick::new();
    click.set_button(0);
    {
        let term = terminal.clone();
        click.connect_pressed(move |gesture, n_press, x, y| {
            term.grab_focus();
            if gesture.current_button() == 3 {
                show_context_menu(&term, x, y, block_id_at(&term, y));
            } else if gesture.current_button() == 1 {
                // Hover-toolbar buttons take precedence over selection.
                if let Some(target) = toolbar_target_at(&term, x, y) {
                    match target.button {
                        ToolbarButton::CopyOutput => {
                            copy_block(&term, target.block_id, false, true);
                        }
                        ToolbarButton::Menu => show_block_menu(&term, target.block_id, x, y),
                    }
                    return;
                }
                let kind = match n_press {
                    2 => SelectionKind::Semantic,
                    3 => SelectionKind::Lines,
                    _ => SelectionKind::Simple,
                };
                if let Some((col, row)) = coords_to_cell(&term, x, y) {
                    if let Some(state) = state(&term) {
                        if let Some(backend) = state.backend.borrow().as_ref() {
                            backend.start_selection(col, row, kind);
                            refresh_grid(&state);
                        }
                    }
                }
            }
        });
    }
    terminal.add_controller(click);

    let drag = gtk4::GestureDrag::new();
    {
        let term = terminal.clone();
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some((start_x, start_y)) = gesture.start_point() else {
                return;
            };
            if let Some((col, row)) = coords_to_cell(&term, start_x + dx, start_y + dy) {
                if let Some(state) = state(&term) {
                    if let Some(backend) = state.backend.borrow().as_ref() {
                        backend.update_selection(col, row);
                        refresh_grid(&state);
                    }
                }
            }
        });
    }
    {
        let term = terminal.clone();
        drag.connect_drag_end(move |_, _x, _y| {
            if let Some(state) = state(&term) {
                if state.copy_on_select.get() {
                    copy_selection(&term);
                }
            }
        });
    }
    terminal.add_controller(drag);

    let drop_target_text =
        gtk4::DropTarget::new(glib::types::Type::STRING, gtk4::gdk::DragAction::COPY);
    {
        let term = terminal.clone();
        drop_target_text.connect_drop(move |_target, value, _x, _y| {
            if let Ok(text) = value.get::<String>() {
                write_text(
                    &term,
                    &shell_escape(&text.replace('\n', " ").replace('\r', "")),
                );
                return true;
            }
            false
        });
    }
    terminal.add_controller(drop_target_text);

    let drop_target_files = gtk4::DropTarget::new(
        gtk4::gdk::FileList::static_type(),
        gtk4::gdk::DragAction::COPY,
    );
    {
        let term = terminal.clone();
        drop_target_files.connect_drop(move |_target, value, _x, _y| {
            if let Ok(file_list) = value.get::<gtk4::gdk::FileList>() {
                let paths: Vec<String> = file_list
                    .files()
                    .iter()
                    .filter_map(|f| f.path())
                    .map(|p| shell_escape(&p.to_string_lossy()))
                    .collect();
                if !paths.is_empty() {
                    write_text(&term, &paths.join(" "));
                    return true;
                }
            }
            false
        });
    }
    terminal.add_controller(drop_target_files);
}

/// Update the hovered block / toolbar button from a pointer position and
/// repaint when either changed.
fn update_block_hover(terminal: &Terminal, x: f64, y: f64) {
    let Some(state) = state(terminal) else {
        return;
    };

    // Toolbar buttons hit-test first (the toolbar floats over the block).
    let mut hovered_button = None;
    let mut toolbar_block = None;
    for target in state.hover_toolbar_targets.borrow().iter() {
        if x >= target.x
            && x < target.x + target.width
            && y >= target.y
            && y < target.y + target.height
        {
            hovered_button = Some(target.button);
            toolbar_block = Some(target.block_id);
            break;
        }
    }

    let hovered_block = toolbar_block.or_else(|| {
        let overlay = state.last_overlay.borrow();
        let overlay = overlay.as_ref()?;
        let cell_height = state.cell_height.get().max(1) as f64;
        let row = ((y - TERMINAL_PADDING) / cell_height).floor() as i32;
        overlay
            .blocks
            .iter()
            .find(|block| row >= block.start_row && row <= block.end_row)
            .map(|block| block.id)
    });

    let changed = state.hovered_block_id.get() != hovered_block
        || state.hovered_toolbar_button.get() != hovered_button;
    state.hovered_block_id.set(hovered_block);
    state.hovered_toolbar_button.set(hovered_button);
    if changed {
        state.drawing.queue_draw();
    }
}

/// The command block under viewport y-position `y`, from the last frame.
fn block_id_at(terminal: &Terminal, y: f64) -> Option<u64> {
    let state = state(terminal)?;
    let overlay = state.last_overlay.borrow();
    let overlay = overlay.as_ref()?;
    let cell_height = state.cell_height.get().max(1) as f64;
    let row = ((y - TERMINAL_PADDING) / cell_height).floor() as i32;
    overlay
        .blocks
        .iter()
        .find(|block| row >= block.start_row && row <= block.end_row)
        .map(|block| block.id)
}

fn toolbar_target_at(terminal: &Terminal, x: f64, y: f64) -> Option<ToolbarTarget> {
    let state = state(terminal)?;
    let targets = state.hover_toolbar_targets.borrow();
    targets
        .iter()
        .find(|t| x >= t.x && x < t.x + t.width && y >= t.y && y < t.y + t.height)
        .copied()
}

fn block_with_id(terminal: &Terminal, id: u64) -> Option<TerminalCommandBlock> {
    command_blocks(terminal)
        .into_iter()
        .find(|block| block.id.0 == id)
}

/// Copy a block's command and/or output to the clipboard (Warp-style block
/// actions; mirrors macOS `copyBlock`).
pub fn copy_block(terminal: &Terminal, id: u64, include_command: bool, include_output: bool) {
    let Some(block) = block_with_id(terminal, id) else {
        return;
    };
    let mut parts: Vec<String> = Vec::new();
    if include_command {
        if let Some(command) = block.command.as_deref() {
            let command = command.trim();
            if !command.is_empty() {
                parts.push(command.to_string());
            }
        }
    }
    if include_output {
        let output = block.output.trim_matches('\n');
        if !output.is_empty() {
            parts.push(output.to_string());
        }
    }
    if !parts.is_empty() {
        terminal.clipboard().set_text(&parts.join("\n\n"));
    }
}

/// Re-run a specific block's command in the terminal.
pub fn rerun_block(terminal: &Terminal, id: u64) {
    if let Some(command) = block_with_id(terminal, id).and_then(|block| block.command) {
        if !command.trim().is_empty() {
            rerun_command_text(terminal, &command);
        }
    }
}

/// Block-focused menu shown by the hover toolbar's "⋯" button: per-block
/// copy/re-run actions plus block navigation.
fn show_block_menu(terminal: &Terminal, block_id: u64, x: f64, y: f64) {
    let popover = gtk4::Popover::new();
    popover.set_has_arrow(false);
    popover.set_parent(terminal);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|popover| popover.unparent());

    let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    append_block_actions(&menu_box, &popover, terminal, block_id);
    append_context_separator(&menu_box);

    let block_flags = command_block_flags(terminal);
    let has_block = block_flags.has_command || block_flags.has_output;
    append_context_button(&menu_box, "Previous Command Block", has_block, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            jump_to_previous_command_block(&term);
            popover.popdown();
        }
    });
    append_context_button(&menu_box, "Next Command Block", has_block, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            jump_to_next_command_block(&term);
            popover.popdown();
        }
    });
    append_context_button(&menu_box, "Last Failed Command", block_flags.has_failed, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            jump_to_last_failed_command_block(&term);
            popover.popdown();
        }
    });

    popover.set_child(Some(&menu_box));
    popover.popup();
}

/// Per-block copy / re-run actions, shared by the right-click menu (when a
/// block is under the pointer) and the hover toolbar's block menu.
fn append_block_actions(
    menu_box: &gtk4::Box,
    popover: &gtk4::Popover,
    terminal: &Terminal,
    block_id: u64,
) {
    let block = block_with_id(terminal, block_id);
    let has_command = block
        .as_ref()
        .and_then(|b| b.command.as_deref())
        .is_some_and(|c| !c.trim().is_empty());
    let has_output = block.as_ref().is_some_and(|b| !b.output.is_empty());
    let is_running = block.as_ref().is_some_and(|b| b.ended_at_ms.is_none());

    append_context_button(menu_box, "Copy Command", has_command, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            copy_block(&term, block_id, true, false);
            popover.popdown();
        }
    });
    append_context_button(menu_box, "Copy Output", has_output, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            copy_block(&term, block_id, false, true);
            popover.popdown();
        }
    });
    append_context_button(
        menu_box,
        "Copy Command & Output",
        has_command || has_output,
        {
            let term = terminal.clone();
            let popover = popover.clone();
            move || {
                copy_block(&term, block_id, true, true);
                popover.popdown();
            }
        },
    );
    append_context_button(menu_box, "Re-run Command", has_command && !is_running, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            rerun_block(&term, block_id);
            popover.popdown();
        }
    });
}

fn show_context_menu(terminal: &Terminal, x: f64, y: f64, block_under_pointer: Option<u64>) {
    let popover = gtk4::Popover::new();
    popover.set_has_arrow(false);
    popover.set_parent(terminal);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|popover| popover.unparent());

    let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let block_flags = command_block_flags(terminal);
    let has_command = block_flags.has_command;
    let has_output = block_flags.has_output;
    let has_block = has_command || has_output;
    let has_failed_block = block_flags.has_failed;
    let has_history = has_command_history(terminal);
    let has_selection = selected_text(terminal)
        .map(|text| !text.is_empty())
        .unwrap_or(false);

    append_context_button(&menu_box, "Copy", has_selection, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            copy_selection(&term);
            popover.popdown();
        }
    });
    append_context_button(&menu_box, "Paste", true, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            paste_from_clipboard(&term);
            popover.popdown();
        }
    });

    append_context_separator(&menu_box);

    if let Some(block_id) = block_under_pointer {
        // Scope the copy/re-run actions to the block under the pointer.
        append_block_actions(&menu_box, &popover, terminal, block_id);
    } else {
        append_context_button(&menu_box, "Copy Last Command", has_command, {
            let term = terminal.clone();
            let popover = popover.clone();
            move || {
                copy_last_command(&term);
                popover.popdown();
            }
        });
        append_context_button(&menu_box, "Copy Last Command Output", has_output, {
            let term = terminal.clone();
            let popover = popover.clone();
            move || {
                copy_last_command_output(&term);
                popover.popdown();
            }
        });
        append_context_button(&menu_box, "Rerun Last Command", has_command, {
            let term = terminal.clone();
            let popover = popover.clone();
            move || {
                rerun_last_command(&term);
                popover.popdown();
            }
        });
    }
    append_context_button(&menu_box, "Command History...", has_history, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            popover.popdown();
            show_command_history_picker(&term);
        }
    });

    append_context_separator(&menu_box);

    append_context_button(&menu_box, "Previous Command Block", has_block, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            jump_to_previous_command_block(&term);
            popover.popdown();
        }
    });
    append_context_button(&menu_box, "Next Command Block", has_block, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            jump_to_next_command_block(&term);
            popover.popdown();
        }
    });
    append_context_button(&menu_box, "Last Failed Command", has_failed_block, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            jump_to_last_failed_command_block(&term);
            popover.popdown();
        }
    });

    append_context_separator(&menu_box);

    append_context_button(&menu_box, "Select All", true, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            if let Some(state) = state(&term) {
                if let Some(backend) = state.backend.borrow().as_ref() {
                    backend.select_all();
                    refresh_grid(&state);
                }
            }
            popover.popdown();
        }
    });
    append_context_button(&menu_box, "Clear", true, {
        let term = terminal.clone();
        let popover = popover.clone();
        move || {
            write(&term, b"\x0c");
            popover.popdown();
        }
    });

    popover.set_child(Some(&menu_box));
    popover.popup();
}

fn append_context_button(
    menu_box: &gtk4::Box,
    label: &str,
    enabled: bool,
    action: impl Fn() + 'static,
) {
    let button = gtk4::Button::with_label(label);
    button.set_sensitive(enabled);
    button.set_halign(gtk4::Align::Fill);
    button.connect_clicked(move |_| action());
    menu_box.append(&button);
}

fn append_context_separator(menu_box: &gtk4::Box) {
    let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    menu_box.append(&separator);
}

fn show_command_history_picker(terminal: &Terminal) {
    let dialog = gtk4::Window::builder()
        .modal(true)
        .decorated(false)
        .default_width(680)
        .default_height(360)
        .build();
    dialog.add_css_class("quick-open");
    if let Some(window) = terminal
        .root()
        .and_then(|root| root.downcast::<gtk4::Window>().ok())
    {
        dialog.set_transient_for(Some(&window));
    }

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);

    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some("Search command history..."));
    vbox.append(&entry);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    scroll.set_child(Some(&list));
    vbox.append(&scroll);

    let button_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    button_box.set_halign(gtk4::Align::End);
    button_box.set_margin_top(8);
    let insert_button = gtk4::Button::with_label("Insert");
    let run_button = gtk4::Button::with_label("Run");
    button_box.append(&insert_button);
    button_box.append(&run_button);
    vbox.append(&button_box);

    dialog.set_child(Some(&vbox));

    let results: Rc<RefCell<Vec<CommandHistorySearchResult>>> = Rc::new(RefCell::new(Vec::new()));
    populate_command_history_list(terminal, &list, &results, "");
    update_history_picker_buttons(&list, &insert_button, &run_button);

    {
        let term = terminal.clone();
        let list = list.clone();
        let results = results.clone();
        let insert_button = insert_button.clone();
        let run_button = run_button.clone();
        entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            populate_command_history_list(&term, &list, &results, &query);
            update_history_picker_buttons(&list, &insert_button, &run_button);
        });
    }

    {
        let insert_button = insert_button.clone();
        let run_button = run_button.clone();
        list.connect_row_selected(move |list, _| {
            update_history_picker_buttons(list, &insert_button, &run_button);
        });
    }

    {
        let term = terminal.clone();
        let dialog = dialog.clone();
        let results = results.clone();
        list.connect_row_activated(move |list, _| {
            if activate_selected_history_command(&term, list, &results, true) {
                dialog.close();
            }
        });
    }

    {
        let term = terminal.clone();
        let dialog = dialog.clone();
        let list = list.clone();
        let results = results.clone();
        insert_button.connect_clicked(move |_| {
            if activate_selected_history_command(&term, &list, &results, false) {
                dialog.close();
            }
        });
    }

    {
        let term = terminal.clone();
        let dialog = dialog.clone();
        let list = list.clone();
        let results = results.clone();
        run_button.connect_clicked(move |_| {
            if activate_selected_history_command(&term, &list, &results, true) {
                dialog.close();
            }
        });
    }

    let key_controller = gtk4::EventControllerKey::new();
    {
        let term = terminal.clone();
        let dialog = dialog.clone();
        let list = list.clone();
        let results = results.clone();
        key_controller.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk4::gdk::Key::Escape {
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }
            if key == gtk4::gdk::Key::Return || key == gtk4::gdk::Key::KP_Enter {
                let insert = modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
                if activate_selected_history_command(&term, &list, &results, !insert) {
                    dialog.close();
                }
                return gtk4::glib::Propagation::Stop;
            }
            if key == gtk4::gdk::Key::Down {
                select_adjacent_history_row(&list, 1);
                return gtk4::glib::Propagation::Stop;
            }
            if key == gtk4::gdk::Key::Up {
                select_adjacent_history_row(&list, -1);
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    entry.add_controller(key_controller);

    dialog.present();
    entry.grab_focus();
}

fn populate_command_history_list(
    terminal: &Terminal,
    list: &gtk4::ListBox,
    results: &Rc<RefCell<Vec<CommandHistorySearchResult>>>,
    query: &str,
) {
    let matches = command_history_search(terminal, query, 30);
    *results.borrow_mut() = matches;
    clear_listbox(list);

    for result in results.borrow().iter() {
        list.append(&command_history_row(result));
    }
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
}

fn command_history_row(result: &CommandHistorySearchResult) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    box_.set_margin_top(6);
    box_.set_margin_bottom(6);
    box_.set_margin_start(8);
    box_.set_margin_end(8);

    let command = gtk4::Label::new(Some(&result.record.command));
    command.set_halign(gtk4::Align::Start);
    command.set_xalign(0.0);
    command.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    box_.append(&command);

    let detail = gtk4::Label::new(Some(&command_history_detail(result)));
    detail.set_halign(gtk4::Align::Start);
    detail.set_xalign(0.0);
    detail.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    detail.add_css_class("dim-label");
    box_.append(&detail);

    row.set_child(Some(&box_));
    row
}

fn command_history_detail(result: &CommandHistorySearchResult) -> String {
    let mut parts = Vec::new();
    parts.push(
        match result.kind {
            CommandHistoryMatchKind::Recent => "Recent",
            CommandHistoryMatchKind::Prefix => "Prefix",
            CommandHistoryMatchKind::Fuzzy => "Fuzzy",
        }
        .to_string(),
    );
    if let Some(exit_code) = result.record.exit_code {
        parts.push(format!("Exit {exit_code}"));
    }
    if let Some(cwd) = result.record.cwd.as_deref() {
        if !cwd.is_empty() {
            let name = std::path::Path::new(cwd)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(cwd);
            parts.push(name.to_string());
        }
    }
    parts.join(" - ")
}

fn activate_selected_history_command(
    terminal: &Terminal,
    list: &gtk4::ListBox,
    results: &Rc<RefCell<Vec<CommandHistorySearchResult>>>,
    run: bool,
) -> bool {
    let Some(row) = list.selected_row() else {
        return false;
    };
    let index = row.index();
    if index < 0 {
        return false;
    }
    let Some(result) = results.borrow().get(index as usize).cloned() else {
        return false;
    };
    if run {
        rerun_command_text(terminal, &result.record.command)
    } else {
        write_text(terminal, &result.record.command);
        true
    }
}

fn select_adjacent_history_row(list: &gtk4::ListBox, delta: i32) {
    let next = list
        .selected_row()
        .map(|row| (row.index() + delta).max(0))
        .unwrap_or(0);
    if let Some(row) = list.row_at_index(next) {
        list.select_row(Some(&row));
    }
}

fn update_history_picker_buttons(
    list: &gtk4::ListBox,
    insert_button: &gtk4::Button,
    run_button: &gtk4::Button,
) {
    let has_selection = list.selected_row().is_some();
    insert_button.set_sensitive(has_selection);
    run_button.set_sensitive(has_selection);
}

fn clear_listbox(list: &gtk4::ListBox) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
}

fn install_destroy_handler(terminal: &Terminal) {
    terminal.connect_destroy(|term| {
        if let Some(state) = state(term) {
            if let Some(backend) = state.backend.borrow_mut().take() {
                backend.shutdown();
            }
        }
    });
}

fn start_event_poll(terminal: &Terminal) {
    schedule_event_poll(terminal, ACTIVE_EVENT_POLL_INTERVAL);
}

fn schedule_event_poll(terminal: &Terminal, delay: Duration) {
    let weak = terminal.downgrade();
    glib::timeout_add_local_once(delay, move || {
        let Some(term) = weak.upgrade() else {
            return;
        };
        let had_events = poll_events(&term);
        let next_delay = if had_events {
            ACTIVE_EVENT_POLL_INTERVAL
        } else if term.is_mapped() {
            IDLE_EVENT_POLL_INTERVAL
        } else {
            HIDDEN_EVENT_POLL_INTERVAL
        };
        schedule_event_poll(&term, next_delay);
    });
}

fn poll_events(terminal: &Terminal) -> bool {
    let Some(state) = state(terminal) else {
        return false;
    };
    let events = state
        .backend
        .borrow()
        .as_ref()
        .map(TerminalBackend::poll_events)
        .unwrap_or_default();

    let had_events = !events.is_empty();
    let mut needs_draw = false;
    for event in events {
        match event {
            TerminalEvent::Wakeup => needs_draw = true,
            TerminalEvent::TitleChanged(title) => {
                *state.title.borrow_mut() = title;
                for callback in state.title_callbacks.borrow().iter() {
                    callback(terminal);
                }
            }
            TerminalEvent::ResetTitle => {
                *state.title.borrow_mut() = "Terminal".to_string();
                for callback in state.title_callbacks.borrow().iter() {
                    callback(terminal);
                }
            }
            TerminalEvent::Bell => {
                if state.terminal_bell.get() {
                    log::info!("terminal bell");
                }
            }
            TerminalEvent::ChildExited(_) | TerminalEvent::Exit => {
                for callback in state.child_exited_callbacks.borrow().iter() {
                    callback(terminal);
                }
            }
            TerminalEvent::CwdChanged(path) => {
                *state.current_directory.borrow_mut() = Some(path);
                for callback in state.cwd_callbacks.borrow().iter() {
                    callback(terminal);
                }
            }
            TerminalEvent::ClipboardStore(text) => terminal.clipboard().set_text(&text),
            TerminalEvent::ClipboardLoad
            | TerminalEvent::CursorBlinkingChange
            | TerminalEvent::CommandStart
            | TerminalEvent::CommandEnd(_)
            | TerminalEvent::AttentionRequest(_)
            | TerminalEvent::Notification { .. }
            | TerminalEvent::PtyWrite(_) => {}
            TerminalEvent::PromptStart => {
                // The live prompt region moved; repaint block decorations.
                needs_draw = true;
            }
            TerminalEvent::CommandBlockStarted(_) => {
                state.selected_command_block_id.set(None);
                state.is_command_running.set(true);
                needs_draw = true;
                for callback in state.command_block_callbacks.borrow().iter() {
                    callback(terminal);
                }
            }
            TerminalEvent::CommandBlockEnded(block) => {
                state.selected_command_block_id.set(None);
                state.is_command_running.set(false);
                state.last_command_exit.set(block.exit_code);
                state.last_command_duration_ms.set(
                    block
                        .ended_at_ms
                        .map(|ended| ended.saturating_sub(block.started_at_ms)),
                );
                needs_draw = true;
                for callback in state.command_block_callbacks.borrow().iter() {
                    callback(terminal);
                }
            }
        }
    }

    if needs_draw {
        refresh_grid(&state);
    }

    // Surface TUI-ownership flips (alt screen / raw mode) to the input bar
    // through the command-block callbacks, so it can hide/reappear promptly.
    let interactive = grid_keyboard_interactive(&state);
    if interactive != state.last_grid_interactive.get() {
        state.last_grid_interactive.set(interactive);
        for callback in state.command_block_callbacks.borrow().iter() {
            callback(terminal);
        }
    }

    had_events
}

fn refresh_grid(state: &Rc<TerminalState>) {
    if let Some(backend) = state.backend.borrow().as_ref() {
        let required = backend.grid_buffer_size();
        let mut buf = state.grid_buffer.borrow_mut();
        if buf.len() < required {
            buf.resize(required, 0);
        }
        let written = backend.write_grid_to_buffer(&mut buf);
        if written > 0 {
            state.mode_bits.set(backend.mode().bits());
            state.drawing.queue_draw();
        }
    }
}

fn draw_terminal(cr: &Context, width: i32, height: i32, state: &Rc<TerminalState>) {
    let font_family = state.font_family.borrow().clone();
    let font_size = state.font_size.get().max(8) as f64 * FONT_POINT_TO_PIXEL_SCALE;
    cr.select_font_face(&font_family, FontSlant::Normal, FontWeight::Normal);
    cr.set_font_size(font_size);

    let font_extents = cr.font_extents().ok();
    let text_extents = cr.text_extents("M").ok();
    let cell_width = text_extents
        .map(|e| e.x_advance().ceil().max(1.0))
        .unwrap_or(DEFAULT_CELL_WIDTH as f64);
    let cell_height = font_extents
        .map(|e| e.height().ceil().max(1.0))
        .unwrap_or(DEFAULT_CELL_HEIGHT as f64);
    let ascent = font_extents
        .map(|e| e.ascent())
        .unwrap_or((DEFAULT_CELL_HEIGHT - 4) as f64);

    state.cell_width.set(cell_width as u16);
    state.cell_height.set(cell_height as u16);

    let cols = (((width as f64 - TERMINAL_PADDING * 2.0) / cell_width).floor() as u16).max(2);
    let rows = (((height as f64 - TERMINAL_PADDING * 2.0) / cell_height).floor() as u16).max(1);
    resize_backend_if_needed(state, cols, rows);

    let bg = current_background(state);
    set_rgb(cr, bg);
    cr.rectangle(0.0, 0.0, width as f64, height as f64);
    let _ = cr.fill();

    let buf = state.grid_buffer.borrow();
    if buf.len() < FIXED_HEADER_SIZE {
        return;
    }

    let snapshot_cols = read_u16(&buf, 0) as usize;
    let snapshot_rows = read_u16(&buf, 2) as usize;
    if snapshot_cols == 0 || snapshot_rows == 0 {
        return;
    }

    let cursor_row = read_u16(&buf, 4) as usize;
    let cursor_col = read_u16(&buf, 6) as usize;
    let cursor_shape = buf[8];
    let cursor_visible = buf[9] != 0;
    let selection_count = read_u16(&buf, 12) as usize;
    let search_count = read_u16(&buf, 14) as usize;
    let ranges_offset = FIXED_HEADER_SIZE;
    let search_offset = ranges_offset + selection_count * RANGE_ENTRY_SIZE;
    let cell_offset = search_offset + search_count * RANGE_ENTRY_SIZE;

    if buf.len() < cell_offset + snapshot_cols * snapshot_rows * CELL_STRIDE {
        return;
    }

    let cell_count = snapshot_cols * snapshot_rows;
    let mut selected_cells = vec![false; cell_count];
    let mut searched_cells = vec![false; cell_count];
    mark_ranges(
        &buf,
        ranges_offset,
        selection_count,
        snapshot_cols,
        snapshot_rows,
        &mut selected_cells,
    );
    mark_ranges(
        &buf,
        search_offset,
        search_count,
        snapshot_cols,
        snapshot_rows,
        &mut searched_cells,
    );

    // Command-block decorations: viewport-mapped regions from the backend,
    // skipped on the alternate screen (TUIs own the grid there).
    let block_overlay = if state.blocks_enabled.get() {
        state
            .backend
            .borrow()
            .as_ref()
            .map(|backend| backend.block_overlay())
            .filter(|overlay| !overlay.alt_screen)
    } else {
        None
    };
    // Keep the overlay for pointer hit-testing (hover highlight, toolbar,
    // right-click block menu), and rebuild the toolbar targets this frame.
    *state.last_overlay.borrow_mut() = block_overlay.clone();
    state.hover_toolbar_targets.borrow_mut().clear();

    if let Some(overlay) = &block_overlay {
        draw_block_washes(
            cr,
            overlay,
            &state.block_style.get(),
            state.selected_command_block_id.get(),
            state.hovered_block_id.get(),
            width as f64,
            cell_height,
            snapshot_rows as i32,
        );
    }

    let default_bg = current_background(state);
    let mut active_font: Option<(FontSlant, FontWeight)> =
        Some((FontSlant::Normal, FontWeight::Normal));

    for row in 0..snapshot_rows {
        for col in 0..snapshot_cols {
            let offset = cell_offset + (row * snapshot_cols + col) * CELL_STRIDE;
            let ch = read_char(&buf, offset);
            let flags = CellFlags::from_bits_truncate(read_u16(&buf, offset + 10));
            if flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }

            let cell_index = row * snapshot_cols + col;
            let selected = selected_cells[cell_index];
            let searched = searched_cells[cell_index];
            let mut fg = RgbColor::new(buf[offset + 4], buf[offset + 5], buf[offset + 6]);
            let mut bg = RgbColor::new(buf[offset + 7], buf[offset + 8], buf[offset + 9]);
            if flags.contains(CellFlags::INVERSE) || selected {
                std::mem::swap(&mut fg, &mut bg);
            }
            if searched {
                bg = RgbColor::new(184, 132, 28);
            }

            let x = TERMINAL_PADDING + col as f64 * cell_width;
            let y = TERMINAL_PADDING + row as f64 * cell_height;
            if bg != default_bg || selected || searched || flags.contains(CellFlags::INVERSE) {
                set_rgb(cr, bg);
                cr.rectangle(x, y, cell_width, cell_height);
                let _ = cr.fill();
            }

            if ch != ' ' && !flags.contains(CellFlags::HIDDEN) {
                let slant = if flags.contains(CellFlags::ITALIC) {
                    FontSlant::Italic
                } else {
                    FontSlant::Normal
                };
                let weight = if flags.contains(CellFlags::BOLD) {
                    FontWeight::Bold
                } else {
                    FontWeight::Normal
                };
                if active_font != Some((slant, weight)) {
                    cr.select_font_face(&font_family, slant, weight);
                    cr.set_font_size(font_size);
                    active_font = Some((slant, weight));
                }
                set_rgb(cr, fg);
                cr.move_to(x, y + ascent);
                let mut encoded = [0u8; 4];
                let _ = cr.show_text(ch.encode_utf8(&mut encoded));
                if flags.intersects(
                    CellFlags::UNDERLINE
                        | CellFlags::DOUBLE_UNDERLINE
                        | CellFlags::UNDERCURL
                        | CellFlags::DOTTED_UNDERLINE
                        | CellFlags::DASHED_UNDERLINE,
                ) {
                    cr.rectangle(x, y + cell_height - 2.0, cell_width, 1.0);
                    let _ = cr.fill();
                }
            }
        }
    }

    if let Some(overlay) = &block_overlay {
        draw_block_decorations(
            cr,
            overlay,
            &state.block_style.get(),
            state.selected_command_block_id.get(),
            state.hovered_block_id.get(),
            default_bg,
            width as f64,
            cell_width,
            cell_height,
            font_size,
            ascent,
            snapshot_rows as i32,
            &font_family,
        );
        // Warp-style hover toolbar at the hovered block's top-right.
        if let Some(hovered) = state.hovered_block_id.get() {
            if let Some(block) = overlay.blocks.iter().find(|b| b.id == hovered) {
                draw_block_toolbar(
                    cr,
                    state,
                    block,
                    &state.block_style.get(),
                    default_bg,
                    width as f64,
                    cell_height,
                    snapshot_rows as i32,
                );
            }
        }
        // Restore the cell font face after chip text rendering.
        cr.select_font_face(&font_family, FontSlant::Normal, FontWeight::Normal);
        cr.set_font_size(font_size);
    }

    // Warp model: the input bar owns the cursor at the prompt, so the in-grid
    // cursor renders only while the grid itself takes keyboard input (TUIs).
    if cursor_visible
        && grid_keyboard_interactive(state)
        && cursor_row < snapshot_rows
        && cursor_col < snapshot_cols
    {
        let x = TERMINAL_PADDING + cursor_col as f64 * cell_width;
        let y = TERMINAL_PADDING + cursor_row as f64 * cell_height;
        set_rgb(cr, RgbColor::new(220, 215, 186));
        match cursor_shape {
            1 => cr.rectangle(x, y, 1.5, cell_height),
            2 => cr.rectangle(x, y + cell_height - 2.0, cell_width, 2.0),
            3 => {
                cr.rectangle(x + 0.5, y + 0.5, cell_width - 1.0, cell_height - 1.0);
                let _ = cr.stroke();
                return;
            }
            _ => cr.rectangle(x, y, cell_width, cell_height),
        }
        let _ = cr.fill();
    }
}

fn set_rgba(cr: &Context, color: RgbColor, alpha: f64) {
    cr.set_source_rgba(
        color.r as f64 / 255.0,
        color.g as f64 / 255.0,
        color.b as f64 / 255.0,
        alpha,
    );
}

/// Translucent row washes under the text: failure tint, navigation
/// highlight, and the live input-prompt region.
#[allow(clippy::too_many_arguments)]
fn draw_block_washes(
    cr: &Context,
    overlay: &impulse_terminal::BlockOverlay,
    style: &BlockStyle,
    highlighted_block: Option<u64>,
    hovered_block: Option<u64>,
    width: f64,
    cell_height: f64,
    rows: i32,
) {
    let fill_rows = |color: RgbColor, alpha: f64, start: i32, end: i32| {
        let start = start.max(0);
        let end = end.min(rows - 1);
        if end < start {
            return;
        }
        set_rgba(cr, color, alpha);
        cr.rectangle(
            0.0,
            TERMINAL_PADDING + start as f64 * cell_height,
            width,
            (end - start + 1) as f64 * cell_height,
        );
        let _ = cr.fill();
    };

    // Live prompt region: a quiet, distinct surface for the input area.
    if let Some(prompt_row) = overlay.prompt_row {
        let end = overlay.cursor_row.unwrap_or(rows - 1);
        fill_rows(style.prompt_fill, 0.035, prompt_row, end);
    }

    for block in &overlay.blocks {
        let is_highlighted = Some(block.id) == highlighted_block;
        let is_hovered = Some(block.id) == hovered_block;
        if block.failed {
            fill_rows(style.failed, 0.07, block.start_row, block.end_row);
        }
        if is_highlighted {
            fill_rows(style.accent, 0.09, block.start_row, block.end_row);
        } else if is_hovered {
            fill_rows(style.accent, 0.05, block.start_row, block.end_row);
        }
    }
}

/// Hairline separators between blocks, left-edge status stripes (Warp's
/// "flag pole"), and right-aligned exit/duration chips.
#[allow(clippy::too_many_arguments)]
fn draw_block_decorations(
    cr: &Context,
    overlay: &impulse_terminal::BlockOverlay,
    style: &BlockStyle,
    highlighted_block: Option<u64>,
    hovered_block: Option<u64>,
    default_bg: RgbColor,
    width: f64,
    _cell_width: f64,
    cell_height: f64,
    font_size: f64,
    ascent: f64,
    rows: i32,
    font_family: &str,
) {
    let draw_separator = |row: i32| {
        if row <= 0 || row >= rows {
            return;
        }
        let y = (TERMINAL_PADDING + row as f64 * cell_height).round() - 0.5;
        set_rgba(cr, style.separator, 0.6);
        cr.set_line_width(1.0);
        cr.move_to(TERMINAL_PADDING, y);
        cr.line_to(width - TERMINAL_PADDING, y);
        let _ = cr.stroke();
    };

    for block in &overlay.blocks {
        let start_row = block.start_row;
        let end_row = block.end_row.min(rows - 1);

        draw_separator(start_row);

        // Left-edge stripe for failed, running, and highlighted blocks,
        // drawn in the padding gutter so it never covers glyphs.
        let is_highlighted = Some(block.id) == highlighted_block;
        if block.failed || block.is_running || is_highlighted {
            let color = if block.failed {
                style.failed
            } else {
                style.accent
            };
            let visible_start = start_row.max(0);
            if end_row >= visible_start {
                set_rgba(cr, color, 1.0);
                cr.rectangle(
                    1.5,
                    TERMINAL_PADDING + visible_start as f64 * cell_height,
                    2.5,
                    (end_row - visible_start + 1) as f64 * cell_height,
                );
                let _ = cr.fill();
            }
        }

        // Exit/duration chip on the block's first line. Suppressed while the
        // block is hovered — the hover toolbar occupies that corner.
        if !block.is_running
            && Some(block.id) != hovered_block
            && start_row >= 0
            && start_row < rows
        {
            if let Some(text) = block_chip_text(block.exit_code, block.duration_ms) {
                draw_block_chip(
                    cr,
                    &text,
                    block.failed,
                    style,
                    default_bg,
                    width,
                    cell_height,
                    font_size,
                    ascent,
                    start_row,
                    font_family,
                );
            }
        }
    }

    // Hairline above the live prompt region.
    if let Some(prompt_row) = overlay.prompt_row {
        draw_separator(prompt_row);
    }
}

/// Append a rounded-rectangle path.
fn rounded_rect_path(cr: &Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        1.5 * std::f64::consts::PI,
    );
    cr.close_path();
}

/// Floating action toolbar at a hovered block's top-right: a quick
/// copy-output button and a "⋯" options menu (Warp-style, mirrors macOS
/// `drawBlockToolbar`). Rebuilds the pointer hit targets as it draws.
#[allow(clippy::too_many_arguments)]
fn draw_block_toolbar(
    cr: &Context,
    state: &Rc<TerminalState>,
    block: &impulse_terminal::BlockOverlayRegion,
    style: &BlockStyle,
    default_bg: RgbColor,
    width: f64,
    cell_height: f64,
    rows: i32,
) {
    // Pin to the block's first visible row (top edge when scrolled past).
    let row = block.start_row.max(0);
    if row >= rows {
        return;
    }

    let button_w = 26.0;
    let inset = 4.0;
    let buttons = [ToolbarButton::CopyOutput, ToolbarButton::Menu];
    let toolbar_h = cell_height;
    let toolbar_w = inset * 2.0 + button_w * buttons.len() as f64;
    let x = width - TERMINAL_PADDING - toolbar_w;
    // Nudge the toolbar down from the block's top edge so it floats clear of
    // the separator and reads as part of the command row.
    let y = TERMINAL_PADDING + row as f64 * cell_height + cell_height * 0.55;
    let radius = (toolbar_h / 2.0).min(7.0);

    // Pill background + border.
    rounded_rect_path(cr, x, y, toolbar_w, toolbar_h, radius);
    set_rgba(cr, default_bg, 0.96);
    let _ = cr.fill();
    rounded_rect_path(cr, x, y, toolbar_w, toolbar_h, radius);
    set_rgba(cr, style.separator, 0.6);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    let hovered_button = state.hovered_toolbar_button.get();
    let mut targets = state.hover_toolbar_targets.borrow_mut();
    for (index, button) in buttons.iter().enumerate() {
        let bx = x + inset + index as f64 * button_w;

        if hovered_button == Some(*button) {
            set_rgba(cr, style.muted_text, 0.15);
            rounded_rect_path(cr, bx + 1.0, y + 2.0, button_w - 2.0, toolbar_h - 4.0, 4.0);
            let _ = cr.fill();
        }

        let icon_alpha = if hovered_button == Some(*button) {
            1.0
        } else {
            0.8
        };
        // Icon box centered in the button.
        let icon = 12.0;
        let ix = bx + (button_w - icon) / 2.0;
        let iy = y + (toolbar_h - icon) / 2.0;
        match button {
            ToolbarButton::CopyOutput => {
                draw_copy_glyph(cr, ix, iy, icon, style.muted_text, icon_alpha, default_bg);
            }
            ToolbarButton::Menu => {
                draw_kebab_glyph(cr, ix, iy, icon, style.muted_text, icon_alpha);
            }
        }

        targets.push(ToolbarTarget {
            x: bx,
            y,
            width: button_w,
            height: toolbar_h,
            button: *button,
            block_id: block.id,
        });
    }
}

/// Two overlapping rounded rectangles — the universal "copy" glyph.
fn draw_copy_glyph(
    cr: &Context,
    x: f64,
    y: f64,
    size: f64,
    color: RgbColor,
    alpha: f64,
    background: RgbColor,
) {
    let w = size * 0.66;
    let h = size * 0.78;
    cr.set_line_width(1.3);
    // Back sheet.
    set_rgba(cr, color, alpha);
    rounded_rect_path(cr, x, y, w, h, 2.0);
    let _ = cr.stroke();
    // Front sheet, filled with the background so it visually overlaps.
    set_rgba(cr, background, 1.0);
    rounded_rect_path(cr, x + size - w, y + size - h, w, h, 2.0);
    let _ = cr.fill();
    set_rgba(cr, color, alpha);
    rounded_rect_path(cr, x + size - w, y + size - h, w, h, 2.0);
    let _ = cr.stroke();
}

/// Three vertical dots — the "more options" kebab glyph.
fn draw_kebab_glyph(cr: &Context, x: f64, y: f64, size: f64, color: RgbColor, alpha: f64) {
    let dot = 1.1;
    let cx = x + size / 2.0;
    let spacing = (size - dot * 2.0) / 2.0;
    set_rgba(cr, color, alpha);
    for i in 0..3 {
        cr.arc(
            cx,
            y + dot + i as f64 * spacing,
            dot,
            0.0,
            2.0 * std::f64::consts::PI,
        );
        let _ = cr.fill();
    }
}

/// "✓ · 1.2s" / "✗ 1 · 3.4s" summary for a completed block.
fn block_chip_text(exit_code: Option<i32>, duration_ms: Option<u64>) -> Option<String> {
    let exit_code = exit_code?;
    let mark = if exit_code == 0 {
        "✓".to_string()
    } else {
        format!("✗ {exit_code}")
    };
    match duration_ms {
        Some(ms) => Some(format!("{mark} · {}", format_block_duration(ms))),
        None => Some(mark),
    }
}

fn format_block_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 3_600_000 {
        format!("{}m {}s", ms / 60_000, (ms % 60_000) / 1000)
    } else {
        format!("{}h {}m", ms / 3_600_000, (ms % 3_600_000) / 60_000)
    }
}

/// Right-aligned rounded chip over the block's first line.
#[allow(clippy::too_many_arguments)]
fn draw_block_chip(
    cr: &Context,
    text: &str,
    failed: bool,
    style: &BlockStyle,
    default_bg: RgbColor,
    width: f64,
    cell_height: f64,
    font_size: f64,
    ascent: f64,
    row: i32,
    font_family: &str,
) {
    cr.select_font_face(font_family, FontSlant::Normal, FontWeight::Normal);
    cr.set_font_size((font_size * 0.85).round());
    let Ok(extents) = cr.text_extents(text) else {
        return;
    };

    let chip_padding = 6.0;
    let chip_height = cell_height - 2.0;
    let chip_width = extents.x_advance() + chip_padding * 2.0;
    let x = width - TERMINAL_PADDING - chip_width;
    let y = TERMINAL_PADDING + row as f64 * cell_height + 1.0;
    let radius = (chip_height / 2.0).min(6.0);

    // Rounded-rect pill.
    let path = |cr: &Context| {
        cr.new_sub_path();
        cr.arc(
            x + chip_width - radius,
            y + radius,
            radius,
            -std::f64::consts::FRAC_PI_2,
            0.0,
        );
        cr.arc(
            x + chip_width - radius,
            y + chip_height - radius,
            radius,
            0.0,
            std::f64::consts::FRAC_PI_2,
        );
        cr.arc(
            x + radius,
            y + chip_height - radius,
            radius,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        );
        cr.arc(
            x + radius,
            y + radius,
            radius,
            std::f64::consts::PI,
            1.5 * std::f64::consts::PI,
        );
        cr.close_path();
    };

    path(cr);
    set_rgba(cr, default_bg, 0.92);
    let _ = cr.fill();
    path(cr);
    set_rgba(cr, style.separator, 0.6);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    let text_color = if failed {
        style.failed
    } else {
        style.muted_text
    };
    set_rgba(cr, text_color, 1.0);
    // Baseline-align with the row's cell text.
    cr.move_to(
        x + chip_padding,
        TERMINAL_PADDING + row as f64 * cell_height + ascent,
    );
    let _ = cr.show_text(text);
}

fn resize_backend_if_needed(state: &Rc<TerminalState>, cols: u16, rows: u16) {
    if cols == state.cols.get() && rows == state.rows.get() {
        return;
    }
    state.cols.set(cols);
    state.rows.set(rows);
    if let Some(backend) = state.backend.borrow_mut().as_mut() {
        backend.resize(
            cols,
            rows,
            state.cell_width.get().max(1),
            state.cell_height.get().max(1),
        );
    }
}

fn handle_key_press(
    terminal: &Terminal,
    key: gtk4::gdk::Key,
    modifiers: gtk4::gdk::ModifierType,
) -> gtk4::glib::Propagation {
    let ctrl = modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
    let alt = modifiers.contains(gtk4::gdk::ModifierType::ALT_MASK);
    let super_ = modifiers.contains(gtk4::gdk::ModifierType::SUPER_MASK);

    if super_ {
        return gtk4::glib::Propagation::Proceed;
    }

    if ctrl && shift {
        if key == gtk4::gdk::Key::C || key == gtk4::gdk::Key::c {
            copy_selection(terminal);
            return gtk4::glib::Propagation::Stop;
        }
        if key == gtk4::gdk::Key::V || key == gtk4::gdk::Key::v {
            paste_from_clipboard(terminal);
            return gtk4::glib::Propagation::Stop;
        }
    }

    if ctrl && (key == gtk4::gdk::Key::V || key == gtk4::gdk::Key::v) {
        paste_from_clipboard(terminal);
        return gtk4::glib::Propagation::Stop;
    }

    // Warp model: at the prompt the grid is read-only — typing belongs to
    // the pinned input bar. Redirect the keystroke there, forwarding a
    // printable character so the first keypress isn't lost.
    if let Some(state) = state(terminal) {
        if !grid_keyboard_interactive(&state) {
            let redirect = state.input_redirect.borrow();
            if let Some(redirect) = redirect.as_ref() {
                let ch = if !ctrl && !alt {
                    key.to_unicode().filter(|c| !c.is_control())
                } else {
                    None
                };
                redirect(ch);
                return gtk4::glib::Propagation::Stop;
            }
        }
    }

    if let Some(seq) = special_key_sequence(terminal, key, modifiers) {
        write_text(terminal, &seq);
        return gtk4::glib::Propagation::Stop;
    }

    if ctrl {
        if let Some(ch) = key.to_unicode().map(|c| c.to_ascii_lowercase()) {
            if ch.is_ascii_lowercase() {
                write(terminal, &[(ch as u8) - b'a' + 1]);
                return gtk4::glib::Propagation::Stop;
            }
            match ch {
                '[' => {
                    write(terminal, b"\x1b");
                    return gtk4::glib::Propagation::Stop;
                }
                '\\' => {
                    write(terminal, &[0x1c]);
                    return gtk4::glib::Propagation::Stop;
                }
                ']' => {
                    write(terminal, &[0x1d]);
                    return gtk4::glib::Propagation::Stop;
                }
                _ => {}
            }
        }
    }

    if let Some(ch) = key.to_unicode() {
        if !ch.is_control() {
            let mut text = String::new();
            if alt {
                text.push('\x1b');
            }
            text.push(ch);
            write_text(terminal, &text);
            return gtk4::glib::Propagation::Stop;
        }
    }

    gtk4::glib::Propagation::Proceed
}

fn special_key_sequence(
    terminal: &Terminal,
    key: gtk4::gdk::Key,
    modifiers: gtk4::gdk::ModifierType,
) -> Option<String> {
    let ctrl = modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
    let app_cursor = state(terminal)
        .map(|s| s.mode_bits.get() & TerminalMode::APP_CURSOR.bits() != 0)
        .unwrap_or(false);

    match key {
        gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter => {
            Some(if shift && !ctrl { "\x1b[13;2u" } else { "\r" }.to_string())
        }
        gtk4::gdk::Key::BackSpace => Some("\x7f".to_string()),
        gtk4::gdk::Key::Tab => Some("\t".to_string()),
        gtk4::gdk::Key::ISO_Left_Tab => Some("\x1b[Z".to_string()),
        gtk4::gdk::Key::Escape => Some("\x1b".to_string()),
        gtk4::gdk::Key::Up => Some(if app_cursor { "\x1bOA" } else { "\x1b[A" }.to_string()),
        gtk4::gdk::Key::Down => Some(if app_cursor { "\x1bOB" } else { "\x1b[B" }.to_string()),
        gtk4::gdk::Key::Right => Some(if app_cursor { "\x1bOC" } else { "\x1b[C" }.to_string()),
        gtk4::gdk::Key::Left => Some(if app_cursor { "\x1bOD" } else { "\x1b[D" }.to_string()),
        gtk4::gdk::Key::Home => Some("\x1b[H".to_string()),
        gtk4::gdk::Key::End => Some("\x1b[F".to_string()),
        gtk4::gdk::Key::Page_Up => Some("\x1b[5~".to_string()),
        gtk4::gdk::Key::Page_Down => Some("\x1b[6~".to_string()),
        gtk4::gdk::Key::Delete => Some("\x1b[3~".to_string()),
        gtk4::gdk::Key::F1 => Some("\x1bOP".to_string()),
        gtk4::gdk::Key::F2 => Some("\x1bOQ".to_string()),
        gtk4::gdk::Key::F3 => Some("\x1bOR".to_string()),
        gtk4::gdk::Key::F4 => Some("\x1bOS".to_string()),
        gtk4::gdk::Key::F5 => Some("\x1b[15~".to_string()),
        gtk4::gdk::Key::F6 => Some("\x1b[17~".to_string()),
        gtk4::gdk::Key::F7 => Some("\x1b[18~".to_string()),
        gtk4::gdk::Key::F8 => Some("\x1b[19~".to_string()),
        gtk4::gdk::Key::F9 => Some("\x1b[20~".to_string()),
        gtk4::gdk::Key::F10 => Some("\x1b[21~".to_string()),
        gtk4::gdk::Key::F11 => Some("\x1b[23~".to_string()),
        gtk4::gdk::Key::F12 => Some("\x1b[24~".to_string()),
        _ => None,
    }
}

fn paste_text(terminal: &Terminal, text: &str) {
    if text.is_empty() {
        return;
    }
    let bracketed = state(terminal)
        .map(|s| s.mode_bits.get() & TerminalMode::BRACKETED_PASTE.bits() != 0)
        .unwrap_or(false);
    if bracketed {
        write(terminal, b"\x1b[200~");
    }
    write_text(terminal, text);
    if bracketed {
        write(terminal, b"\x1b[201~");
    }
}

fn coords_to_cell(terminal: &Terminal, x: f64, y: f64) -> Option<(usize, usize)> {
    let state = state(terminal)?;
    let col = ((x - TERMINAL_PADDING) / state.cell_width.get().max(1) as f64).floor();
    let row = ((y - TERMINAL_PADDING) / state.cell_height.get().max(1) as f64).floor();
    if col < 0.0 || row < 0.0 {
        return None;
    }
    Some((col as usize, row as usize))
}

fn terminal_colors(theme: &ThemeColors) -> impulse_terminal::TerminalColors {
    let mut palette = [RgbColor::new(0, 0, 0); 16];
    for (idx, hex) in theme.terminal_palette.iter().enumerate().take(16) {
        palette[idx] = hex_to_rgb(hex);
    }
    impulse_terminal::TerminalColors {
        foreground: hex_to_rgb(theme.fg),
        background: hex_to_rgb(theme.bg),
        palette,
    }
}

fn parse_cursor_shape(shape: &str) -> CursorShape {
    match shape {
        "ibeam" | "beam" | "line" => CursorShape::Beam,
        "underline" => CursorShape::Underline,
        _ => CursorShape::Block,
    }
}

fn current_background(state: &TerminalState) -> RgbColor {
    state.colors.borrow().background
}

fn filtered_env_map() -> HashMap<String, String> {
    std::env::vars()
        .filter(|(k, _)| {
            !FILTERED_LD_VARS.contains(&k.as_str())
                && !FILTERED_PARENT_COLOR_VARS.contains(&k.as_str())
        })
        .collect()
}

fn hex_to_rgb(hex: &str) -> RgbColor {
    let s = hex.trim_start_matches('#');
    if s.len() < 6 {
        return RgbColor::new(255, 0, 255);
    }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(255);
    RgbColor::new(r, g, b)
}

fn set_rgb(cr: &Context, color: RgbColor) {
    cr.set_source_rgb(
        color.r as f64 / 255.0,
        color.g as f64 / 255.0,
        color.b as f64 / 255.0,
    );
}

fn read_u16(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buf[offset], buf[offset + 1]])
}

fn read_char(buf: &[u8], offset: usize) -> char {
    let cp = u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ]);
    char::from_u32(cp).unwrap_or(' ')
}

fn mark_ranges(
    buf: &[u8],
    offset: usize,
    count: usize,
    cols: usize,
    rows: usize,
    cells: &mut [bool],
) {
    for idx in 0..count {
        let base = offset + idx * RANGE_ENTRY_SIZE;
        let range_row = read_u16(buf, base) as usize;
        if range_row >= rows {
            continue;
        }
        let start = read_u16(buf, base + 2) as usize;
        let end = read_u16(buf, base + 4) as usize;
        if start >= cols {
            continue;
        }
        let row_offset = range_row * cols;
        for col in start..=end.min(cols.saturating_sub(1)) {
            cells[row_offset + col] = true;
        }
    }
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn regex_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '.' | '+' | '*' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}
