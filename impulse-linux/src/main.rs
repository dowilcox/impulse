mod editor_bridge;
mod file_tree_model;
mod helpers;
mod keybindings;
mod lsp_bridge;
mod search_model;
mod settings_model;
mod theme_bridge;
mod window_model;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn main() {
    env_logger::init();
    eprintln!("[impulse] === STARTING IMPULSE ===");

    // Initialize QtWebEngine before creating the application
    helpers::init_webengine();
    eprintln!("[impulse] WebEngine initialized");

    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        let url = QUrl::from("qrc:/qt/qml/dev/impulse/app/qml/Main.qml");
        eprintln!("[impulse] Loading QML from: {}", url.to_string());
        engine.load(&url);
        eprintln!("[impulse] QML engine loaded, root objects: checking...");
    } else {
        eprintln!("[impulse] ERROR: QML engine is None!");
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
