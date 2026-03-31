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
}
