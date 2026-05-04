use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk4::cairo::{Context, FontSlant, FontWeight};
use gtk4::glib;
use gtk4::prelude::*;
use impulse_terminal::{
    CellFlags, CursorShape, RgbColor, SelectionKind, TerminalBackend, TerminalCommandBlock,
    TerminalConfig, TerminalEvent, TerminalMode, CELL_STRIDE, FIXED_HEADER_SIZE, RANGE_ENTRY_SIZE,
};

use crate::theme::ThemeColors;

const TERMINAL_DATA_KEY: &str = "impulse-terminal-state";
const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 30;
const DEFAULT_CELL_WIDTH: u16 = 9;
const DEFAULT_CELL_HEIGHT: u16 = 18;
const TERMINAL_PADDING: f64 = 8.0;
const FONT_POINT_TO_PIXEL_SCALE: f64 = 96.0 / 72.0;

const FILTERED_LD_VARS: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "LD_DEBUG",
    "LD_PROFILE",
    "LD_DYNAMIC_WEAK",
    "LD_BIND_NOW",
];

/// GTK terminal widget backed by `impulse_terminal::TerminalBackend`.
///
/// The root widget stores its state in object data so surrounding split/tab code
/// can rediscover terminals while walking the GTK widget tree.
pub type Terminal = gtk4::Box;

type TerminalCallback = Box<dyn Fn(&Terminal) + 'static>;

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
    cwd_callbacks: RefCell<Vec<TerminalCallback>>,
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
            cwd_callbacks: RefCell::new(Vec::new()),
            title_callbacks: RefCell::new(Vec::new()),
            child_exited_callbacks: RefCell::new(Vec::new()),
        }
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
        .scroll_on_output
        .set(settings.terminal_scroll_on_output);
    state.terminal_bell.set(settings.terminal_bell);
    copy_on_select_flag.set(settings.terminal_copy_on_select);
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
            if !command.trim().is_empty() {
                write_text(terminal, &(command + "\n"));
            }
        }
    }
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

    let click = gtk4::GestureClick::new();
    click.set_button(0);
    {
        let term = terminal.clone();
        click.connect_pressed(move |gesture, n_press, x, y| {
            term.grab_focus();
            if gesture.current_button() == 3 {
                show_context_menu(&term, x, y);
            } else if gesture.current_button() == 1 {
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

fn show_context_menu(terminal: &Terminal, x: f64, y: f64) {
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

    let blocks = command_blocks(terminal);
    let has_command = blocks.iter().any(has_command_text);
    let has_output = blocks.iter().any(|block| !block.output.is_empty());
    let has_block = has_command || has_output;
    let has_failed_block = blocks.iter().any(is_failed_command_block);
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
    let weak = terminal.downgrade();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(term) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        poll_events(&term);
        glib::ControlFlow::Continue
    });
}

fn poll_events(terminal: &Terminal) {
    let Some(state) = state(terminal) else {
        return;
    };
    let events = state
        .backend
        .borrow()
        .as_ref()
        .map(TerminalBackend::poll_events)
        .unwrap_or_default();

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
            | TerminalEvent::PromptStart
            | TerminalEvent::CommandStart
            | TerminalEvent::CommandEnd(_)
            | TerminalEvent::AttentionRequest(_)
            | TerminalEvent::Notification { .. }
            | TerminalEvent::PtyWrite(_) => {}
            TerminalEvent::CommandBlockStarted(_) | TerminalEvent::CommandBlockEnded(_) => {
                state.selected_command_block_id.set(None);
            }
        }
    }

    if needs_draw {
        refresh_grid(&state);
    }
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

    for row in 0..snapshot_rows {
        for col in 0..snapshot_cols {
            let offset = cell_offset + (row * snapshot_cols + col) * CELL_STRIDE;
            let ch = read_char(&buf, offset);
            let flags = CellFlags::from_bits_truncate(read_u16(&buf, offset + 10));
            if flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }

            let selected = range_contains(&buf, ranges_offset, selection_count, row, col);
            let searched = range_contains(&buf, search_offset, search_count, row, col);
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
            set_rgb(cr, bg);
            cr.rectangle(x, y, cell_width, cell_height);
            let _ = cr.fill();

            if ch != ' ' && !flags.contains(CellFlags::HIDDEN) {
                cr.select_font_face(
                    &font_family,
                    if flags.contains(CellFlags::ITALIC) {
                        FontSlant::Italic
                    } else {
                        FontSlant::Normal
                    },
                    if flags.contains(CellFlags::BOLD) {
                        FontWeight::Bold
                    } else {
                        FontWeight::Normal
                    },
                );
                cr.set_font_size(font_size);
                set_rgb(cr, fg);
                cr.move_to(x, y + ascent);
                let _ = cr.show_text(&ch.to_string());
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

    if cursor_visible && cursor_row < snapshot_rows && cursor_col < snapshot_cols {
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
        .filter(|(k, _)| !FILTERED_LD_VARS.contains(&k.as_str()))
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

fn range_contains(buf: &[u8], offset: usize, count: usize, row: usize, col: usize) -> bool {
    for idx in 0..count {
        let base = offset + idx * RANGE_ENTRY_SIZE;
        let range_row = read_u16(buf, base) as usize;
        if range_row != row {
            continue;
        }
        let start = read_u16(buf, base + 2) as usize;
        let end = read_u16(buf, base + 4) as usize;
        if col >= start && col <= end {
            return true;
        }
    }
    false
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
