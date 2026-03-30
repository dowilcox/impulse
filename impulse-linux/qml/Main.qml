// SPDX-License-Identifier: GPL-3.0-only
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

ApplicationWindow {
    id: root

    // Resource paths — project root resolved from Rust via executable path
    readonly property string iconsDir: "file://" + windowModel.project_root + "assets/icons/"
    readonly property string fontsDir: "file://" + windowModel.project_root + "impulse-editor/vendor/fonts/"

    title: {
        var t = "Impulse"
        if (contentArea.activeEditorPath.length > 0) {
            var name = contentArea.activeEditorPath.split("/").pop()
            t = (contentArea.activeEditorModified ? "\u25CF " : "") + name + " \u2014 Impulse"
        }
        return t
    }
    width: 1280
    height: 800
    visible: true

    // ── Models ───────────────────────────────────────────────────────────────
    WindowModel { id: windowModel }
    ThemeBridge { id: theme }
    FileTreeModel { id: fileTreeModel }
    EditorBridge { id: editorBridge }
    LspBridge { id: lspBridge }
    SearchModel { id: searchModel }
    SettingsModel { id: settings }

    // ── Startup ──────────────────────────────────────────────────────────────
    Timer {
        interval: 100
        running: true
        repeat: false
        onTriggered: {
            theme.set_theme("nord")
            windowModel.set_directory("/home/dowilcox")
            fileTreeModel.load_root("/home/dowilcox")
            searchModel.root_path = "/home/dowilcox"
            // Open a terminal tab on launch
            windowModel.create_tab("terminal")
        }
    }

    // ── Keyboard shortcuts ───────────────────────────────────────────────────
    Shortcut { sequence: "Ctrl+B"; onActivated: windowModel.toggle_sidebar() }
    Shortcut { sequence: "Ctrl+T"; onActivated: windowModel.create_tab("terminal") }
    Shortcut {
        sequence: "Ctrl+W"
        onActivated: {
            if (windowModel.tab_count > 0) {
                windowModel.close_tab(windowModel.active_tab_index)
            }
        }
    }
    Shortcut {
        sequence: "Ctrl+S"
        onActivated: contentArea.saveActiveEditor()
    }
    Shortcut {
        sequence: "Ctrl+P"
        onActivated: quickOpenDialog.open()
    }
    Shortcut {
        sequence: "Ctrl+Shift+P"
        onActivated: commandPalette.open()
    }
    Shortcut {
        sequence: "Ctrl+G"
        onActivated: goToLineDialog.open()
    }
    Shortcut {
        sequence: "Ctrl+Tab"
        onActivated: {
            if (windowModel.tab_count > 1) {
                var next = (windowModel.active_tab_index + 1) % windowModel.tab_count
                windowModel.select_tab(next)
            }
        }
    }
    Shortcut {
        sequence: "Ctrl+Shift+Tab"
        onActivated: {
            if (windowModel.tab_count > 1) {
                var prev = (windowModel.active_tab_index - 1 + windowModel.tab_count) % windowModel.tab_count
                windowModel.select_tab(prev)
            }
        }
    }
    Shortcut {
        sequence: "Ctrl+Shift+F"
        onActivated: {
            windowModel.sidebar_visible = true
            sidebar.searchMode = true
        }
    }

    // ── Wire file tree clicks to content area ────────────────────────────────
    Connections {
        target: fileTreeModel
        function onFile_activated(path) {
            contentArea.openFile(path)
        }
    }

    // ── Wire search result clicks to content area ────────────────────────────
    Connections {
        target: searchModel
        function onResult_selected(path, line) {  // Fixed signal name from camelCase
            contentArea.openFile(path, line)
        }
    }

    // ── Main layout ──────────────────────────────────────────────────────────
    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal

            // ── Sidebar ──────────────────────────────────────────────────
            Sidebar {
                id: sidebar
                SplitView.preferredWidth: 260
                SplitView.minimumWidth: 180
                SplitView.maximumWidth: root.width * 0.5
                visible: windowModel.sidebar_visible
            }

            // ── Content column (tab bar + content + placeholder) ─────────
            ColumnLayout {
                SplitView.fillWidth: true
                spacing: 0

                TabBar {
                    id: tabBar
                    Layout.fillWidth: true
                }

                ContentArea {
                    id: contentArea
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                }
            }
        }

        StatusBar {
            id: statusBar
            Layout.fillWidth: true
        }
    }

    // ── Dialogs (hidden by default) ──────────────────────────────────────────
    QuickOpenDialog {
        id: quickOpenDialog
        x: Math.round((root.width - width) / 2)
        y: Math.round(root.height * 0.2)
        onFileSelected: function(path) {
            contentArea.openFile(path)
        }
    }

    CommandPalette {
        id: commandPalette
        x: Math.round((root.width - width) / 2)
        y: Math.round(root.height * 0.2)
    }

    GoToLineDialog {
        id: goToLineDialog
        x: Math.round((root.width - width) / 2)
        y: Math.round(root.height * 0.25)
    }
}
