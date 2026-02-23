mod editor;
mod editor_webview;
mod file_icons;
mod keybindings;
mod lsp_completion;
mod lsp_hover;
mod project_search;
mod settings;
mod settings_page;
mod sidebar;
mod status_bar;
mod terminal;
mod terminal_container;
mod theme;
mod window;

use libadwaita as adw;
use libadwaita::prelude::*;
use std::backtrace::Backtrace;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

const APP_ID: &str = "dev.impulse.Impulse";
const APP_ID_DEVEL: &str = "dev.impulse.Impulse.Devel";

enum StartupMode {
    RunGui,
    InstallLspServers,
    CheckLspServers,
}

fn is_devel_mode() -> bool {
    std::env::args().any(|a| a == "--dev")
}

fn parse_startup_mode() -> StartupMode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--install-lsp-servers") {
        StartupMode::InstallLspServers
    } else if args.iter().any(|a| a == "--check-lsp-servers") {
        StartupMode::CheckLspServers
    } else {
        StartupMode::RunGui
    }
}

fn run_lsp_install() -> i32 {
    match impulse_core::lsp::install_managed_web_lsp_servers() {
        Ok(bin_dir) => {
            println!("Installed managed web LSP servers to {}", bin_dir.display());
            let statuses = impulse_core::lsp::managed_web_lsp_status();
            let installed = statuses
                .iter()
                .filter(|s| s.resolved_path.is_some())
                .count();
            println!(
                "Resolved {}/{} recommended server commands.",
                installed,
                statuses.len()
            );
            0
        }
        Err(e) => {
            eprintln!("Failed to install managed web LSP servers: {}", e);
            1
        }
    }
}

fn run_lsp_check() -> i32 {
    let statuses = impulse_core::lsp::managed_web_lsp_status();
    for status in &statuses {
        match &status.resolved_path {
            Some(path) => println!("OK   {:32} {}", status.command, path.display()),
            None => println!("MISS {:32} not found", status.command),
        }
    }
    let missing = statuses
        .iter()
        .filter(|s| s.resolved_path.is_none())
        .count();
    if missing == 0 {
        println!("All managed web LSP commands are available.");
        0
    } else {
        println!(
            "{} managed web LSP command(s) are missing. Run --install-lsp-servers.",
            missing
        );
        1
    }
}

fn state_dir() -> Option<PathBuf> {
    if let Ok(xdg_state_home) = std::env::var("XDG_STATE_HOME") {
        if !xdg_state_home.is_empty() {
            return Some(PathBuf::from(xdg_state_home).join("impulse"));
        }
    }

    std::env::var("HOME").ok().map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("impulse")
    })
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let timestamp = chrono_like_now();
        let backtrace = Backtrace::force_capture();
        let msg = format!(
            "[{}] panic: {}\nbacktrace:\n{}\n\n",
            timestamp, panic_info, backtrace
        );

        if let Some(dir) = state_dir() {
            let _ = std::fs::create_dir_all(&dir);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(e) =
                    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                {
                    log::warn!("Failed to set permissions on {:?}: {}", dir, e);
                }
            }
            let path = dir.join("panic.log");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&path)
            {
                let _ = f.write_all(msg.as_bytes());
            }
        }

        eprintln!("{}", msg);
        default_hook(panic_info);
    }));
}

fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => format!("{}.{:03}", d.as_secs(), d.subsec_millis()),
        Err(_) => "unknown-time".to_string(),
    }
}

const APP_ICON_SVG: &[u8] = include_bytes!("../../assets/impulse-logo.svg");

/// Install the app icon into the user's icon theme directory so GTK can find it.
fn install_app_icon() {
    let icon_dir = match std::env::var("HOME").ok() {
        Some(home) => PathBuf::from(home).join(".local/share/icons/hicolor/scalable/apps"),
        None => return,
    };
    let icon_path = icon_dir.join("dev.impulse.Impulse.svg");
    // Always overwrite so the icon stays in sync with the embedded SVG.
    if std::fs::create_dir_all(&icon_dir).is_ok() {
        let _ = std::fs::write(&icon_path, APP_ICON_SVG);
    }
}

fn main() {
    env_logger::init();
    install_panic_hook();

    match parse_startup_mode() {
        StartupMode::InstallLspServers => {
            std::process::exit(run_lsp_install());
        }
        StartupMode::CheckLspServers => {
            std::process::exit(run_lsp_check());
        }
        StartupMode::RunGui => {}
    }

    let devel = is_devel_mode();
    let app_id = if devel { APP_ID_DEVEL } else { APP_ID };
    let app = adw::Application::builder().application_id(app_id).build();

    app.connect_startup(move |_app| {
        let style_manager = adw::StyleManager::default();
        style_manager.set_color_scheme(adw::ColorScheme::ForceDark);

        // Install application icon into user icon theme and set as default
        install_app_icon();
        gtk4::Window::set_default_icon_name("dev.impulse.Impulse");

        if devel {
            log::info!("Running in development mode (app-id: {})", APP_ID_DEVEL);
        }
    });

    app.connect_activate(move |app| {
        window::build_window(app);
    });

    // Filter out custom flags so GTK/GLib doesn't reject them.
    let gtk_args: Vec<String> = std::env::args()
        .filter(|a| {
            !matches!(
                a.as_str(),
                "--dev" | "--install-lsp-servers" | "--check-lsp-servers"
            )
        })
        .collect();
    app.run_with_args(&gtk_args);
}
