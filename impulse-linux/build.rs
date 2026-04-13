use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    // In cxx-qt-build 0.8, QmlModule uses a builder pattern and Rust bridge
    // files are registered on CxxQtBuilder directly (not inside QmlModule).
    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("dev.impulse.app").qml_files([
        "qml/Main.qml",
        "qml/Sidebar.qml",
        "qml/FileTreeView.qml",
        "qml/FileNodeDelegate.qml",
        "qml/TabBar.qml",
        "qml/ContentArea.qml",
        "qml/TerminalView.qml",
        "qml/EditorView.qml",
        "qml/StatusBar.qml",
        "qml/SearchPanel.qml",
        "qml/QuickOpenDialog.qml",
        "qml/CommandPalette.qml",
        "qml/GoToLineDialog.qml",
        "qml/SettingsWindow.qml",
    ]))
    .files([
        "src/window_model.rs",
        "src/theme_bridge.rs",
        "src/file_tree_model.rs",
        "src/editor_bridge.rs",
        "src/lsp_bridge.rs",
        "src/search_model.rs",
        "src/settings_model.rs",
        "src/terminal_bridge.rs",
    ])
    .qt_module("QuickControls2")
    .qt_module("WebEngineQuick");

    // SAFETY: We only add our own C++ helper files and include paths;
    // no Qt internals or generated code is modified.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.file("cpp/helpers.cpp");
            cc.include("cpp");
        })
    };

    builder.build();
}
