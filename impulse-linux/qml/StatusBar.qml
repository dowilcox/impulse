// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore
import dev.impulse.app

Rectangle {
    id: statusBarRoot
    height: 24
    color: theme.bg_dark

    // Top border
    Rectangle {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 1
        color: theme.border
    }

    RowLayout {
        anchors.fill: parent
        anchors.topMargin: 1  // below border
        anchors.leftMargin: 10
        anchors.rightMargin: 10
        spacing: 0

        // ── Left side ─────────────────────────────────────────────────────

        // Shell name
        StatusLabel {
            text: windowModel.shell_name
            visible: windowModel.shell_name.length > 0
        }

        StatusDivider { visible: windowModel.shell_name.length > 0 }

        // Git branch
        RowLayout {
            spacing: 4
            visible: windowModel.git_branch.length > 0

            Text {
                text: "\u2387"  // branch symbol
                font.pixelSize: 12
                color: theme.accent
                Layout.alignment: Qt.AlignVCenter
            }
            Text {
                text: windowModel.git_branch
                font.pixelSize: 12
                color: theme.accent
                elide: Text.ElideRight
                Layout.maximumWidth: 160
                Layout.alignment: Qt.AlignVCenter
            }
        }

        StatusDivider { visible: windowModel.git_branch.length > 0 }

        // Current directory (abbreviated)
        Text {
            text: {
                var dir = windowModel.current_directory
                var home = StandardPaths.writableLocation(StandardPaths.HomeLocation)
                if (home && dir.indexOf(home) === 0) {
                    dir = "~" + dir.substring(home.length)
                }
                return dir
            }
            font.pixelSize: 12
            color: theme.fg_muted
            elide: Text.ElideMiddle
            Layout.maximumWidth: 300
            Layout.alignment: Qt.AlignVCenter
        }

        // ── Spacer ────────────────────────────────────────────────────────
        Item { Layout.fillWidth: true }

        // ── Right side ────────────────────────────────────────────────────

        // Blame info
        Text {
            text: windowModel.blame_info
            font.pixelSize: 11
            font.italic: true
            color: theme.fg_muted
            visible: windowModel.blame_info.length > 0
            elide: Text.ElideRight
            Layout.maximumWidth: 240
            Layout.alignment: Qt.AlignVCenter
        }

        StatusDivider { visible: windowModel.blame_info.length > 0 }

        // Preview toggle (for markdown/SVG editors)
        ToolButton {
            id: previewToggle
            visible: {
                var ca = contentArea
                return ca && ca.activeEditorPath.length > 0 && editorBridge.is_previewable_file(ca.activeEditorPath)
            }
            text: "Preview"
            font.pixelSize: 11
            Layout.preferredHeight: 20
            padding: 2
            leftPadding: 6
            rightPadding: 6
            palette.buttonText: theme.fg_muted
            background: Rectangle {
                color: previewToggle.hovered ? theme.bg_highlight : "transparent"
                radius: 3
            }
            onClicked: {
                var ca = contentArea
                if (ca) {
                    var view = ca.contentItems[windowModel.active_tab_index]
                    if (view && view.togglePreview) {
                        view.togglePreview()
                    }
                }
            }

            ToolTip.visible: hovered
            ToolTip.text: "Toggle Preview"
            ToolTip.delay: 600
        }

        StatusDivider {
            visible: previewToggle.visible
        }

        // Cursor position
        Text {
            text: "Ln " + windowModel.cursor_line + ", Col " + windowModel.cursor_column
            font.pixelSize: 12
            color: theme.fg_muted
            visible: windowModel.cursor_line > 0
            Layout.alignment: Qt.AlignVCenter
        }

        StatusDivider { visible: windowModel.cursor_line > 0 }

        // Language
        StatusLabel {
            text: windowModel.language
            visible: windowModel.language.length > 0
        }

        StatusDivider { visible: windowModel.language.length > 0 }

        // Encoding
        StatusLabel {
            text: windowModel.encoding
            visible: windowModel.encoding.length > 0
        }

        StatusDivider { visible: windowModel.encoding.length > 0 }

        // Indent info
        StatusLabel {
            text: windowModel.indent_info
            visible: windowModel.indent_info.length > 0
        }
    }

    // ── Reusable sub-components ───────────────────────────────────────────────

    component StatusLabel : Text {
        font.pixelSize: 12
        color: theme.fg_muted
        elide: Text.ElideRight
        Layout.alignment: Qt.AlignVCenter
    }

    component StatusDivider : Rectangle {
        Layout.preferredWidth: 1
        Layout.preferredHeight: 14
        Layout.leftMargin: 8
        Layout.rightMargin: 8
        Layout.alignment: Qt.AlignVCenter
        color: theme.border
    }
}
