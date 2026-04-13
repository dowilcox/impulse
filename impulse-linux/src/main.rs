mod editor_bridge;
mod file_tree_model;
mod helpers;
mod keybindings;
mod lsp_bridge;
mod search_model;
mod settings_model;
mod terminal_bridge;
mod theme_bridge;
mod window_model;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use std::process::ExitCode;

fn print_lsp_status() {
    let mut statuses = Vec::new();
    statuses.extend(
        impulse_core::lsp::managed_web_lsp_status()
            .into_iter()
            .map(|status| ("managed", status)),
    );
    statuses.extend(
        impulse_core::lsp::system_lsp_status()
            .into_iter()
            .map(|status| ("system", status)),
    );

    for (kind, status) in statuses {
        let state = if status.resolved_path.is_some() {
            "installed"
        } else {
            "not installed"
        };
        if let Some(path) = status.resolved_path {
            println!(
                "{} [{}]: {} ({})",
                status.command,
                kind,
                state,
                path.display()
            );
        } else {
            println!("{} [{}]: {}", status.command, kind, state);
        }
    }
}

fn main() -> ExitCode {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--install-lsp-servers") {
        match impulse_core::lsp::install_managed_web_lsp_servers() {
            Ok(path) => {
                println!("Installed managed LSP servers to {}", path.display());
                return ExitCode::SUCCESS;
            }
            Err(err) => {
                eprintln!("Error: {}", err);
                return ExitCode::from(1);
            }
        }
    }

    if args.iter().any(|arg| arg == "--check-lsp-servers") {
        print_lsp_status();
        return ExitCode::SUCCESS;
    }

    // Initialize QtWebEngine before creating the application
    helpers::init_webengine();

    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        let url = QUrl::from("qrc:/qt/qml/dev/impulse/app/qml/Main.qml");
        engine.load(&url);
    } else {
        log::error!("Failed to create QML engine");
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }

    ExitCode::SUCCESS
}
