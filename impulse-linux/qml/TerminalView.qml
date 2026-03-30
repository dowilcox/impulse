// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QMLTermWidget
import dev.impulse.app

Item {
    id: termViewRoot

    // Load bundled JetBrains Mono font
    FontLoader {
        id: jetbrainsFont
        source: "file://" + windowModel.project_root + "impulse-editor/vendor/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf"
    }

    // ── Terminal widget ───────────────────────────────────────────────────────
    QMLTermWidget {
        id: termWidget
        anchors.fill: parent
        focus: true

        font.family: jetbrainsFont.status === FontLoader.Ready ? jetbrainsFont.name : "Monospace"
        font.pixelSize: 14

        colorScheme: "Linux"

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
            termWidget.forceActiveFocus()
        }
    }
}
