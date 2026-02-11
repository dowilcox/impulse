use std::cell::Cell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use std::path::PathBuf;
use vte4::prelude::*;

use crate::theme::ThemeColors;

/// Cached shell spawn parameters, computed once and reused for every new tab.
pub struct ShellSpawnCache {
    shell_name: String,
    argv: Vec<String>,
    envv: Vec<String>,
    working_dir: String,
    /// Temp files that must outlive all terminal sessions.
    _temp_files: Vec<PathBuf>,
}

impl ShellSpawnCache {
    /// Build the cache once at startup.
    pub fn new() -> Self {
        let shell_path = impulse_core::shell::get_default_shell_path();
        let shell_type = impulse_core::shell::detect_shell_type(&shell_path);

        let shell_name = std::path::Path::new(&shell_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string();

        let working_dir =
            impulse_core::shell::get_home_directory().unwrap_or_else(|_| "/".to_string());

        // Build integration temp files (bash/zsh rc wrappers)
        let temp_files = match impulse_core::shell::build_shell_command(&shell_path, &shell_type) {
            Ok((_cmd, files)) => files,
            Err(e) => {
                log::error!("Failed to build shell command: {}", e);
                Vec::new()
            }
        };

        let (argv, envv) = build_spawn_params(&shell_path, &shell_type, &temp_files);

        ShellSpawnCache {
            shell_name,
            argv,
            envv,
            working_dir,
            _temp_files: temp_files,
        }
    }

    pub fn shell_name(&self) -> &str {
        &self.shell_name
    }
}

/// Create a new VTE terminal widget configured with Impulse shell integration.
pub fn create_terminal(
    settings: &crate::settings::Settings,
    theme: &ThemeColors,
    copy_on_select_flag: Rc<Cell<bool>>,
) -> vte4::Terminal {
    let terminal = vte4::Terminal::new();

    // Appearance
    let palette = theme.terminal_palette_rgba();
    let palette_refs: Vec<&gtk4::gdk::RGBA> = palette.iter().collect();
    terminal.set_colors(
        Some(&theme.fg_rgba()),
        Some(&theme.bg_rgba()),
        &palette_refs,
    );
    let font_family = if !settings.terminal_font_family.is_empty() {
        &settings.terminal_font_family
    } else {
        "monospace"
    };
    let mut font_desc = gtk4::pango::FontDescription::from_string(font_family);
    font_desc.set_size(settings.terminal_font_size * 1024);
    terminal.set_font_desc(Some(&font_desc));
    let cursor_blink = if settings.terminal_cursor_blink {
        vte4::CursorBlinkMode::On
    } else {
        vte4::CursorBlinkMode::Off
    };
    terminal.set_cursor_blink_mode(cursor_blink);
    let cursor_shape = match settings.terminal_cursor_shape.as_str() {
        "ibeam" => vte4::CursorShape::Ibeam,
        "underline" => vte4::CursorShape::Underline,
        _ => vte4::CursorShape::Block,
    };
    terminal.set_cursor_shape(cursor_shape);
    terminal.set_scrollback_lines(settings.terminal_scrollback);
    terminal.set_scroll_on_output(settings.terminal_scroll_on_output);
    terminal.set_scroll_on_keystroke(true);
    terminal.set_mouse_autohide(true);
    terminal.set_allow_hyperlink(settings.terminal_allow_hyperlink);
    terminal.set_bold_is_bright(settings.terminal_bold_is_bright);
    terminal.set_audible_bell(settings.terminal_bell);

    // Copy-on-select: always connect the signal, check the flag inside
    copy_on_select_flag.set(settings.terminal_copy_on_select);
    {
        let flag = copy_on_select_flag;
        terminal.connect_selection_changed(move |term| {
            if flag.get() && term.has_selection() {
                term.copy_clipboard_format(vte4::Format::Text);
            }
        });
    }

    // Size
    terminal.set_size(120, 30);
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);

    // Ctrl+V / Ctrl+Shift+V paste directly on the terminal widget
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let term = terminal.clone();
        key_controller.connect_key_pressed(move |_, key, _, modifiers| {
            let ctrl = modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
            let is_v = key == gtk4::gdk::Key::v || key == gtk4::gdk::Key::V;
            if ctrl && is_v {
                paste_from_clipboard(&term);
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    terminal.add_controller(key_controller);

    // Accept text drops (e.g. paths from sidebar drag source)
    let drop_target_text =
        gtk4::DropTarget::new(glib::types::Type::STRING, gtk4::gdk::DragAction::COPY);
    {
        let term = terminal.clone();
        drop_target_text.connect_drop(move |_target, value, _x, _y| {
            if let Ok(text) = value.get::<String>() {
                let escaped = shell_escape(&text);
                term.feed_child(escaped.as_bytes());
                return true;
            }
            false
        });
    }
    terminal.add_controller(drop_target_text);

    // Accept file drops (images, media, documents from file managers)
    let drop_target_files = gtk4::DropTarget::new(
        gtk4::gdk::FileList::static_type(),
        gtk4::gdk::DragAction::COPY,
    );
    {
        let term = terminal.clone();
        drop_target_files.connect_drop(move |_target, value, _x, _y| {
            if let Ok(file_list) = value.get::<gtk4::gdk::FileList>() {
                let files = file_list.files();
                let paths: Vec<String> = files
                    .iter()
                    .filter_map(|f| f.path())
                    .map(|p| shell_escape(&p.to_string_lossy()))
                    .collect();
                if !paths.is_empty() {
                    let joined = paths.join(" ");
                    term.feed_child(joined.as_bytes());
                    return true;
                }
            }
            false
        });
    }
    terminal.add_controller(drop_target_files);

    terminal
}

/// Apply settings changes to an existing terminal (font, cursor, scrollback, etc.).
pub fn apply_settings(
    terminal: &vte4::Terminal,
    settings: &crate::settings::Settings,
    theme: &ThemeColors,
    copy_on_select_flag: &Cell<bool>,
) {
    let palette = theme.terminal_palette_rgba();
    let palette_refs: Vec<&gtk4::gdk::RGBA> = palette.iter().collect();
    terminal.set_colors(
        Some(&theme.fg_rgba()),
        Some(&theme.bg_rgba()),
        &palette_refs,
    );

    let font_family = if !settings.terminal_font_family.is_empty() {
        &settings.terminal_font_family
    } else {
        "monospace"
    };
    let mut font_desc = gtk4::pango::FontDescription::from_string(font_family);
    font_desc.set_size(settings.terminal_font_size * 1024);
    terminal.set_font_desc(Some(&font_desc));

    terminal.set_cursor_blink_mode(if settings.terminal_cursor_blink {
        vte4::CursorBlinkMode::On
    } else {
        vte4::CursorBlinkMode::Off
    });
    terminal.set_cursor_shape(match settings.terminal_cursor_shape.as_str() {
        "ibeam" => vte4::CursorShape::Ibeam,
        "underline" => vte4::CursorShape::Underline,
        _ => vte4::CursorShape::Block,
    });
    terminal.set_scrollback_lines(settings.terminal_scrollback);
    terminal.set_scroll_on_output(settings.terminal_scroll_on_output);
    terminal.set_allow_hyperlink(settings.terminal_allow_hyperlink);
    terminal.set_bold_is_bright(settings.terminal_bold_is_bright);
    terminal.set_audible_bell(settings.terminal_bell);

    // Update the copy-on-select flag (checked inside the already-connected signal)
    copy_on_select_flag.set(settings.terminal_copy_on_select);
}

/// Spawn the user's shell inside a VTE terminal using pre-cached spawn parameters.
pub fn spawn_shell(terminal: &vte4::Terminal, cache: &Rc<ShellSpawnCache>) {
    let argv_refs: Vec<&str> = cache.argv.iter().map(|s| s.as_str()).collect();
    let envv_refs: Vec<&str> = cache.envv.iter().map(|s| s.as_str()).collect();

    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        Some(&cache.working_dir),
        &argv_refs,
        &envv_refs,
        gtk4::glib::SpawnFlags::DEFAULT,
        || {},
        -1,
        gtk4::gio::Cancellable::NONE,
        |result| match result {
            Ok(pid) => log::info!("Shell spawned with PID: {:?}", pid),
            Err(e) => log::error!("Failed to spawn shell: {}", e),
        },
    );
}

/// Spawn a command (with arguments) in a VTE terminal instead of the default shell.
pub fn spawn_command(terminal: &vte4::Terminal, command: &str, args: &[String]) {
    let mut argv: Vec<&str> = vec![command];
    for arg in args {
        argv.push(arg.as_str());
    }

    // Inherit current environment
    let envv: Vec<String> = std::env::vars()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    let envv_refs: Vec<&str> = envv.iter().map(|s| s.as_str()).collect();

    let working_dir = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());

    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        Some(&working_dir),
        &argv,
        &envv_refs,
        gtk4::glib::SpawnFlags::DEFAULT,
        || {},
        -1,
        gtk4::gio::Cancellable::NONE,
        move |result| match result {
            Ok(pid) => log::info!("Command spawned with PID: {:?}", pid),
            Err(e) => log::error!("Failed to spawn command: {}", e),
        },
    );
}

/// Build argv and environment variables for spawning the shell with integration.
fn build_spawn_params(
    shell_path: &str,
    shell_type: &impulse_core::shell::ShellType,
    temp_files: &[PathBuf],
) -> (Vec<String>, Vec<String>) {
    let mut argv = vec![shell_path.to_string()];
    let mut envv = Vec::new();

    // Inherit current environment
    for (key, value) in std::env::vars() {
        envv.push(format!("{}={}", key, value));
    }

    // Add Impulse / VTE env vars
    envv.push("TERM_PROGRAM=Impulse".to_string());
    envv.push("TERM_PROGRAM_VERSION=0.1.0".to_string());
    envv.push("TERM=xterm-256color".to_string());
    envv.push("COLORTERM=truecolor".to_string());
    // VTE_VERSION tells shells (especially fish) this is a VTE terminal,
    // so they know DA1 queries will be answered and can adjust timing.
    envv.push("VTE_VERSION=8203".to_string());

    match shell_type {
        impulse_core::shell::ShellType::Bash => {
            if let Some(rc) = temp_files.iter().find(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("impulse-bash-rc"))
                    .unwrap_or(false)
            }) {
                argv.push("--rcfile".to_string());
                argv.push(rc.to_string_lossy().to_string());
            }
        }
        impulse_core::shell::ShellType::Zsh => {
            argv.push("--login".to_string());
            if let Some(zdotdir) = temp_files.iter().find_map(|p| {
                p.parent().filter(|parent| {
                    parent
                        .file_name()
                        .map(|n| n.to_string_lossy().starts_with("impulse-zsh"))
                        .unwrap_or(false)
                })
            }) {
                envv.retain(|e| !e.starts_with("ZDOTDIR="));
                envv.push(format!("ZDOTDIR={}", zdotdir.to_string_lossy()));
            }
        }
        impulse_core::shell::ShellType::Fish => {
            argv.push("--login".to_string());
            argv.push("--init-command".to_string());
            argv.push(impulse_core::shell::get_integration_script(shell_type).to_string());
        }
    }

    (argv, envv)
}

/// Shell-escape a string using single quotes.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Paste clipboard contents into a terminal.
///
/// If the clipboard contains text, it is pasted directly (respecting bracketed
/// paste mode).  If it contains only an image (e.g. a screenshot), the image is
/// saved to a temporary PNG file and its shell-escaped path is fed to the PTY so
/// that CLI tools like Claude Code can consume it.
pub fn paste_from_clipboard(terminal: &vte4::Terminal) {
    use gtk4::gdk::prelude::TextureExt;

    let clipboard = terminal.clipboard();
    let formats = clipboard.formats();

    // Prefer text when available — this is the normal paste path.
    if formats.contains_type(glib::types::Type::STRING) {
        let term = terminal.clone();
        clipboard.read_text_async(None::<&gtk4::gio::Cancellable>, move |result| {
            if let Ok(Some(text)) = result {
                term.paste_text(&text);
            }
        });
        return;
    }

    // Fall back to image: save as a temp PNG and paste the path.
    if formats.contains_type(gtk4::gdk::Texture::static_type()) {
        let term = terminal.clone();
        clipboard.read_texture_async(None::<&gtk4::gio::Cancellable>, move |result| {
            if let Ok(Some(texture)) = result {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let path = format!("/tmp/impulse-clipboard-{}.png", ts);
                match texture.save_to_png(&path) {
                    Ok(()) => {
                        let escaped = shell_escape(&path);
                        term.feed_child(escaped.as_bytes());
                    }
                    Err(e) => log::warn!("Failed to save clipboard image: {}", e),
                }
            }
        });
    }
}
