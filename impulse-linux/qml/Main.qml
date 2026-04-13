// SPDX-License-Identifier: GPL-3.0-only
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

ApplicationWindow {
    id: root
    property bool startupComplete: false

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

    Timer {
        id: uiStateSaveDebounce
        interval: 300
        repeat: false
        onTriggered: settings.save()
    }

    // ── Startup ──────────────────────────────────────────────────────────────
    Timer {
        interval: 100
        running: true
        repeat: false
        onTriggered: {
            // Load persisted settings first
            settings.load()

            if (settings.window_width > 0)
                root.width = settings.window_width
            if (settings.window_height > 0)
                root.height = settings.window_height

            // Apply theme from settings (fall back to "nord")
            var themeId = settings.color_scheme.length > 0 ? settings.color_scheme : "nord"
            theme.set_theme(themeId)

            // Determine initial directory: CLI arg > settings > CWD
            var dir = windowModel.get_initial_directory()
            if (!windowModel.has_startup_path() && settings.last_directory.length > 0) {
                dir = settings.last_directory
            }

            windowModel.set_directory(dir)
            fileTreeModel.show_hidden = settings.sidebar_show_hidden
            fileTreeModel.load_root(dir)
            searchModel.root_path = dir

            // Apply sidebar visibility from settings
            windowModel.sidebar_visible = settings.sidebar_visible
            sidebar.SplitView.preferredWidth = settings.sidebar_width > 0 ? settings.sidebar_width : 260

            // Open a terminal tab on launch
            windowModel.create_tab("terminal")
            startupComplete = true
        }
    }

    onClosing: function(close) {
        settings.set_setting("window_width", Math.round(root.width).toString())
        settings.set_setting("window_height", Math.round(root.height).toString())
        settings.set_setting("sidebar_visible", windowModel.sidebar_visible ? "true" : "false")
        settings.set_setting("sidebar_width", Math.round(sidebar.width).toString())
        settings.set_setting("last_directory", windowModel.current_directory)
        settings.set_setting("sidebar_show_hidden", fileTreeModel.show_hidden ? "true" : "false")
        settings.save()
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
    Shortcut {
        sequence: "Ctrl+,"
        onActivated: settingsWindow.show()
    }

    // ── Wire file tree clicks to content area ────────────────────────────────
    Connections {
        target: fileTreeModel
        function onFile_activated(path) {
            contentArea.openFile(path)
            fileTreeModel.set_active_path(path)
        }
    }

    // ── Wire search result clicks to content area ────────────────────────────
    Connections {
        target: searchModel
        function onResult_selected(path, line) {
            contentArea.openFile(path, line)
            fileTreeModel.set_active_path(path)
        }
    }

    // ── Sync active file path on tab switch ──────────────────────────────────
    Connections {
        target: windowModel
        function onTab_switched() {
            var info = contentArea.currentTabInfo()
            if (info && info.filePath) {
                fileTreeModel.set_active_path(info.filePath)
            } else {
                fileTreeModel.set_active_path("")
            }
        }
        function onDirectory_changed() {
            if (!root.startupComplete)
                return
            searchModel.root_path = windowModel.current_directory
            settings.set_setting("last_directory", windowModel.current_directory)
            uiStateSaveDebounce.restart()
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

    SettingsWindow {
        id: settingsWindow
        visible: false
    }
}
