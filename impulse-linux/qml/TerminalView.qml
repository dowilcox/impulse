// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QMLTermWidget 1.0
import dev.impulse.app

Item {
    id: termViewRoot

    // Track splits. The root is always a single terminal or a SplitView with children.
    property var splitOrientation: Qt.Horizontal

    // Expose terminal session for external callers
    readonly property alias session: termSession
    readonly property alias terminal: termWidget

    // ── Color palette from theme ──────────────────────────────────────────────
    function applyColorScheme() {
        var palette
        try {
            palette = JSON.parse(theme.terminal_palette_json)
        } catch (e) {
            return
        }
        if (!palette || !palette.colors) return

        // QMLTermWidget uses colorScheme property or direct color table access.
        // We set the colors directly via the terminal color table.
        var colors = palette.colors
        if (colors.length >= 16) {
            for (var i = 0; i < 16; i++) {
                termWidget.setColorTableEntry(i, colors[i])
            }
        }
        if (palette.foreground) {
            termWidget.foregroundColor = palette.foreground
        }
        if (palette.background) {
            termWidget.backgroundColor = palette.background
        }
        if (palette.cursor) {
            termWidget.cursorColor = palette.cursor
        }
    }

    // ── Terminal widget ───────────────────────────────────────────────────────
    QMLTermWidget {
        id: termWidget
        anchors.fill: parent

        font.family: settings.terminal_font_family || "Monospace"
        font.pixelSize: settings.terminal_font_size > 0 ? settings.terminal_font_size : 14

        // Color configuration
        colorScheme: "Linux"  // fallback; overridden by applyColorScheme
        backgroundColor: theme.bg
        foregroundColor: theme.fg

        // Scrollback
        scrollbarVisible: true

        // Cursor
        blinkingCursor: settings.terminal_cursor_blink

        // Sessions
        session: QMLTermSession {
            id: termSession
            initialWorkingDirectory: windowModel.current_directory

            // Detect the user's shell or use a sensible default
            shellProgram: {
                var shell = windowModel.shell_name
                if (shell && shell.length > 0) {
                    // Map short names to paths
                    if (shell === "fish")  return "/usr/bin/fish"
                    if (shell === "zsh")   return "/usr/bin/zsh"
                    if (shell === "bash")  return "/usr/bin/bash"
                    return "/usr/bin/" + shell
                }
                return "/bin/sh"
            }
            shellProgramArgs: []

            // Forward CWD changes from the terminal
            onCurrentDirectoryChanged: {
                if (currentDirectory && currentDirectory.length > 0) {
                    windowModel.set_directory(currentDirectory)
                }
            }
        }

        // Copy-on-select
        property bool copyOnSelect: settings.terminal_copy_on_select
        onSelectionChanged: {
            if (copyOnSelect && hasSelection) {
                copyClipboard()
            }
        }

        Component.onCompleted: {
            termSession.startShellProgram()
            applyColorScheme()
        }

        // Reapply colors when theme changes
        Connections {
            target: theme
            function onTheme_id_changed() {
                termViewRoot.applyColorScheme()
            }
        }

        // Context menu
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.RightButton
            propagateComposedEvents: true

            onClicked: function(mouse) {
                if (mouse.button === Qt.RightButton) {
                    termContextMenu.popup()
                }
            }
        }
    }

    // ── Terminal context menu ─────────────────────────────────────────────────
    Menu {
        id: termContextMenu

        background: Rectangle {
            color: theme.bg_surface
            border.color: theme.border
            border.width: 1
            radius: 6
        }

        MenuItem {
            text: "Copy"
            enabled: termWidget.hasSelection
            onTriggered: termWidget.copyClipboard()
            contentItem: Text { text: parent.text; color: parent.enabled ? theme.fg : theme.fg_muted; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuItem {
            text: "Paste"
            onTriggered: termWidget.pasteClipboard()
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuSeparator {
            contentItem: Rectangle { implicitHeight: 1; color: theme.border }
        }
        MenuItem {
            text: "Clear"
            onTriggered: termWidget.clear()
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuSeparator {
            contentItem: Rectangle { implicitHeight: 1; color: theme.border }
        }
        MenuItem {
            text: "Split Horizontal"
            onTriggered: windowModel.create_tab("terminal")  // placeholder: actual split logic would create nested SplitView
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuItem {
            text: "Split Vertical"
            onTriggered: windowModel.create_tab("terminal")  // placeholder: actual split logic
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
    }

    // ── Keyboard shortcuts local to terminal ──────────────────────────────────
    Keys.onPressed: function(event) {
        // Ctrl+Shift+C — copy
        if (event.modifiers === (Qt.ControlModifier | Qt.ShiftModifier) && event.key === Qt.Key_C) {
            termWidget.copyClipboard()
            event.accepted = true
        }
        // Ctrl+Shift+V — paste
        else if (event.modifiers === (Qt.ControlModifier | Qt.ShiftModifier) && event.key === Qt.Key_V) {
            termWidget.pasteClipboard()
            event.accepted = true
        }
    }
}
