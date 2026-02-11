use std::cell::Cell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use std::path::PathBuf;
use vte4::prelude::*;

use crate::theme::ThemeColors;

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

/// Spawn the user's shell inside a VTE terminal with Impulse integration scripts.
/// Defers spawning until the terminal widget is realized, ensuring the PTY is
/// fully connected before the shell starts (fixes fish DA1 query timeout).
pub fn spawn_shell(terminal: &vte4::Terminal) {
    if terminal.is_realized() {
        do_spawn_shell(terminal);
    } else {
        let term = terminal.clone();
        terminal.connect_realize(move |_| {
            do_spawn_shell(&term);
        });
    }
}

fn do_spawn_shell(terminal: &vte4::Terminal) {
    let shell_path = impulse_core::shell::get_default_shell_path();
    let shell_type = impulse_core::shell::detect_shell_type(&shell_path);

    // Build the shell command with integration scripts
    let (_cmd, _temp_files) =
        match impulse_core::shell::build_shell_command(&shell_path, &shell_type) {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to build shell command: {}", e);
                spawn_shell_fallback(terminal, &shell_path);
                return;
            }
        };

    let (argv, envv) = extract_spawn_params(&shell_path, &shell_type, &_temp_files);

    let working_dir = impulse_core::shell::get_home_directory().unwrap_or_else(|_| "/".to_string());

    let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let envv_refs: Vec<&str> = envv.iter().map(|s| s.as_str()).collect();

    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        Some(&working_dir),
        &argv_refs,
        &envv_refs,
        gtk4::glib::SpawnFlags::DEFAULT,
        || {}, // child_setup (no-op)
        -1,    // timeout
        gtk4::gio::Cancellable::NONE,
        |result| {
            // callback
            match result {
                Ok(pid) => log::info!("Shell spawned with PID: {:?}", pid),
                Err(e) => log::error!("Failed to spawn shell: {}", e),
            }
        },
    );

    // Leak temp files so they persist for the lifetime of the terminal session.
    std::mem::forget(_temp_files);
}

/// Extract argv and environment variables for spawning the shell with integration.
fn extract_spawn_params(
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

/// Paste clipboard contents into a terminal using the GTK4 clipboard API.
/// Uses `paste_text()` which respects bracketed paste mode.
pub fn paste_from_clipboard(terminal: &vte4::Terminal) {
    let clipboard = terminal.clipboard();
    let term = terminal.clone();
    clipboard.read_text_async(None::<&gtk4::gio::Cancellable>, move |result| {
        if let Ok(Some(text)) = result {
            term.paste_text(&text);
        }
    });
}

fn spawn_shell_fallback(terminal: &vte4::Terminal, shell_path: &str) {
    let working_dir = impulse_core::shell::get_home_directory().unwrap_or_else(|_| "/".to_string());

    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        Some(&working_dir),
        &[shell_path],
        &[] as &[&str],
        gtk4::glib::SpawnFlags::DEFAULT,
        || {},
        -1,
        gtk4::gio::Cancellable::NONE,
        |result| {
            if let Err(e) = result {
                log::error!("Failed to spawn fallback shell: {}", e);
            }
        },
    );
}
