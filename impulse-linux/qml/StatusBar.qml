// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore
import dev.impulse.app

ToolBar {
    id: statusBarRoot
    position: ToolBar.Footer

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 10
        anchors.rightMargin: 10
        spacing: 0

        // ── Left side ─────────────────────────────────────────────────────

        // Shell name
        Label {
            text: windowModel.shell_name
            font.pixelSize: 12
            visible: windowModel.shell_name.length > 0
        }

        ToolSeparator {
            visible: windowModel.shell_name.length > 0
            Layout.preferredHeight: 14
        }

        // Git branch
        RowLayout {
            spacing: 4
            visible: windowModel.git_branch.length > 0

            Label {
                text: "\u2387"  // branch symbol
                font.pixelSize: 12
                color: palette.highlight
            }
            Label {
                text: windowModel.git_branch
                font.pixelSize: 12
                color: palette.highlight
                elide: Text.ElideRight
                Layout.maximumWidth: 160
            }
        }

        ToolSeparator {
            visible: windowModel.git_branch.length > 0
            Layout.preferredHeight: 14
        }

        // Current directory (abbreviated)
        Label {
            text: {
                var dir = windowModel.current_directory
                var home = StandardPaths.writableLocation(StandardPaths.HomeLocation)
                if (home && dir.indexOf(home) === 0) {
                    dir = "~" + dir.substring(home.length)
                }
                return dir
            }
            font.pixelSize: 12
            elide: Text.ElideMiddle
            Layout.maximumWidth: 300
        }

        // ── Spacer ────────────────────────────────────────────────────────
        Item { Layout.fillWidth: true }

        // ── Right side ────────────────────────────────────────────────────

        // Blame info
        Label {
            text: windowModel.blame_info
            font.pixelSize: 11
            font.italic: true
            visible: windowModel.blame_info.length > 0
            elide: Text.ElideRight
            Layout.maximumWidth: 240
        }

        ToolSeparator {
            visible: windowModel.blame_info.length > 0
            Layout.preferredHeight: 14
        }

        // Preview toggle (for markdown/SVG editors)
        ToolButton {
            id: previewToggle
            visible: {
                var ca = contentArea
                return ca && ca.activeEditorPath.length > 0 && editorBridge.is_previewable_file(ca.activeEditorPath)
            }
            text: "Preview"
            font.pixelSize: 11
            onClicked: {
                var ca = contentArea
                if (ca) {
                    var view = ca.activeView()
                    if (view && view.togglePreview) {
                        view.togglePreview()
                    }
                }
            }
            ToolTip.visible: hovered
            ToolTip.text: "Toggle Preview"
            ToolTip.delay: 600
        }

        ToolSeparator {
            visible: previewToggle.visible
            Layout.preferredHeight: 14
        }

        // Cursor position
        Label {
            text: "Ln " + windowModel.cursor_line + ", Col " + windowModel.cursor_column
            font.pixelSize: 12
            visible: windowModel.cursor_line > 0
        }

        ToolSeparator {
            visible: windowModel.cursor_line > 0
            Layout.preferredHeight: 14
        }

        // Language
        Label {
            text: windowModel.language
            font.pixelSize: 12
            visible: windowModel.language.length > 0
        }

        ToolSeparator {
            visible: windowModel.language.length > 0
            Layout.preferredHeight: 14
        }

        // Encoding
        Label {
            text: windowModel.encoding
            font.pixelSize: 12
            visible: windowModel.encoding.length > 0
        }

        ToolSeparator {
            visible: windowModel.encoding.length > 0
            Layout.preferredHeight: 14
        }

        // Indent info
        Label {
            text: windowModel.indent_info
            font.pixelSize: 12
            visible: windowModel.indent_info.length > 0
        }
    }
}
