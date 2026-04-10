// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QMLTermWidget
import dev.impulse.app

Item {
    id: termViewRoot
    clip: true

    // Load bundled JetBrains Mono font
    FontLoader {
        id: jetbrainsFont
        source: "file://" + windowModel.project_root + "impulse-editor/vendor/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf"
    }

    // TODO: image clipboard paste fallback. macOS saves clipboard images to a
    // temp PNG and pastes the path wrapped in bracketed-paste so Claude Code
    // and other TUI apps can attach screenshots — see
    // impulse-macos/.../Terminal/TerminalTab.swift pasteFromClipboard().
    // QMLTermWidget handles paste internally; implementing this on Linux
    // likely needs a custom paste action on Ctrl+Shift+V that inspects
    // QGuiApplication::clipboard() and feeds the path via sendText.

    // ── Terminal widget ───────────────────────────────────────────────────────
    QMLTermWidget {
        id: termWidget
        anchors.fill: parent

        font.family: jetbrainsFont.status === FontLoader.Ready ? jetbrainsFont.name : "Monospace"
        font.pixelSize: settings.terminal_font_size > 0 ? settings.terminal_font_size : 16

        colorScheme: theme.is_light ? "BlackOnWhite" : "DarkPastels"

        session: QMLTermSession {
            id: termSession
            initialWorkingDirectory: windowModel.current_directory

            shellProgram: {
                var shell = windowModel.shell_name
                if (shell && shell.length > 0) {
                    if (shell === "fish")  return "/usr/bin/fish"
                    if (shell === "zsh")   return "/usr/bin/zsh"
                    if (shell === "bash")  return "/usr/bin/bash"
                    return "/usr/bin/" + shell
                }
                return "/bin/sh"
            }
            shellProgramArgs: []
        }

        Component.onCompleted: {
            termSession.startShellProgram()
        }

        // Re-grab focus when this view becomes visible (tab switch)
        onVisibleChanged: {
            if (visible) {
                termWidget.forceActiveFocus()
            }
        }
    }

    // Give terminal focus when clicked
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.NoButton
        onPressed: function(mouse) { mouse.accepted = false }
        cursorShape: Qt.IBeamCursor
    }

    onVisibleChanged: {
        if (visible) {
            termWidget.forceActiveFocus()
        }
    }

    // Grab focus on activation
    onActiveFocusChanged: {
        if (activeFocus) {
            termWidget.forceActiveFocus()
        }
    }
}
