use gtk4::glib;
use gtk4::prelude::*;
use std::path::PathBuf;
use vte4::prelude::*;

use crate::theme::ThemeColors;

/// Create a new VTE terminal widget configured with Impulse shell integration.
pub fn create_terminal(
    settings: &crate::settings::Settings,
    theme: &ThemeColors,
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
        &settings.font_family
    };
    let mut font_desc = gtk4::pango::FontDescription::from_string(font_family);
    font_desc.set_size(settings.font_size * 1024);
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
    terminal.set_scroll_on_output(false);
    terminal.set_scroll_on_keystroke(true);
    terminal.set_mouse_autohide(true);
    terminal.set_allow_hyperlink(true);
    terminal.set_audible_bell(settings.terminal_bell);

    // Size
    terminal.set_size(120, 30);
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);

    // Accept file path drops from sidebar drag source
    let drop_target = gtk4::DropTarget::new(glib::types::Type::STRING, gtk4::gdk::DragAction::COPY);
    {
        let term = terminal.clone();
        drop_target.connect_drop(move |_target, value, _x, _y| {
            if let Ok(path) = value.get::<String>() {
                // Shell-escape the path using single quotes
                let escaped = format!("'{}'", path.replace('\'', "'\\''"));
                term.feed_child(escaped.as_bytes());
                return true;
            }
            false
        });
    }
    terminal.add_controller(drop_target);

    terminal
}

/// Apply settings changes to an existing terminal (font, cursor, scrollback, etc.).
pub fn apply_settings(terminal: &vte4::Terminal, settings: &crate::settings::Settings, theme: &ThemeColors) {
    let palette = theme.terminal_palette_rgba();
    let palette_refs: Vec<&gtk4::gdk::RGBA> = palette.iter().collect();
    terminal.set_colors(Some(&theme.fg_rgba()), Some(&theme.bg_rgba()), &palette_refs);

    let font_family = if !settings.terminal_font_family.is_empty() {
        &settings.terminal_font_family
    } else {
        &settings.font_family
    };
    let mut font_desc = gtk4::pango::FontDescription::from_string(font_family);
    font_desc.set_size(settings.font_size * 1024);
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
    terminal.set_audible_bell(settings.terminal_bell);
}

/// Spawn the user's shell inside a VTE terminal with Impulse integration scripts.
pub fn spawn_shell(terminal: &vte4::Terminal) {
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

    // Add Impulse env vars
    envv.push("TERM_PROGRAM=Impulse".to_string());
    envv.push("TERM_PROGRAM_VERSION=0.1.0".to_string());
    envv.push("TERM=xterm-256color".to_string());

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
