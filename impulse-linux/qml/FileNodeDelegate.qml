// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Rectangle {
    id: nodeRoot

    // Incoming data
    required property var nodeData   // { name, path, isDir, isExpanded, depth, gitStatus, childCount }
    required property int depth

    readonly property bool isActive: nodeData.path === fileTreeModel.active_file_path
    readonly property string nodeName: nodeData.name || ""
    readonly property string nodePath: nodeData.path || ""
    readonly property bool isDir: !!nodeData.isDir
    readonly property bool isExpanded: !!nodeData.isExpanded
    readonly property string gitStatus: nodeData.gitStatus || ""

    height: 26
    color: {
        if (isActive) return Qt.rgba(
            parseInt(theme.accent.substring(1, 3), 16) / 255,
            parseInt(theme.accent.substring(3, 5), 16) / 255,
            parseInt(theme.accent.substring(5, 7), 16) / 255,
            0.18
        )
        if (mouseArea.containsMouse) return theme.bg_highlight
        return "transparent"
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 4 + depth * 16
        anchors.rightMargin: 4
        spacing: 4

        // ── Expand chevron (directories only) ─────────────────────────────
        Item {
            Layout.preferredWidth: 16
            Layout.preferredHeight: 16

            Text {
                anchors.centerIn: parent
                visible: isDir
                text: "\u25B6"  // right-pointing triangle
                font.pixelSize: 8
                color: theme.fg_muted
                rotation: isExpanded ? 90 : 0
                Behavior on rotation { NumberAnimation { duration: 120 } }
            }
        }

        // ── File/folder icon ──────────────────────────────────────────────
        Text {
            Layout.preferredWidth: 16
            Layout.preferredHeight: 16
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            font.pixelSize: 13
            text: {
                if (isDir) return isExpanded ? "\uD83D\uDCC2" : "\uD83D\uDCC1"
                // Pick icon character based on extension
                var ext = nodeName.split(".").pop().toLowerCase()
                switch (ext) {
                    case "rs":    return "R"
                    case "js":    return "J"
                    case "ts":    return "T"
                    case "py":    return "P"
                    case "qml":   return "Q"
                    case "html":  return "H"
                    case "css":   return "C"
                    case "json":  return "{}"
                    case "toml":  return "T"
                    case "yaml":
                    case "yml":   return "Y"
                    case "md":    return "M"
                    case "sh":    return "$"
                    case "svg":   return "S"
                    case "png":
                    case "jpg":
                    case "jpeg":
                    case "gif":
                    case "webp":  return "\uD83D\uDDBC"
                    default:      return "\uD83D\uDCC4"
                }
            }
            color: {
                if (isDir) return theme.accent
                var ext = nodeName.split(".").pop().toLowerCase()
                switch (ext) {
                    case "rs":   return theme.orange
                    case "js":   return theme.yellow
                    case "ts":   return theme.blue
                    case "py":   return theme.green
                    case "qml":  return theme.magenta
                    case "html": return theme.red
                    case "css":  return theme.cyan
                    case "json": return theme.yellow
                    case "md":   return theme.blue
                    default:     return theme.fg_muted
                }
            }
        }

        // ── File name ─────────────────────────────────────────────────────
        Text {
            Layout.fillWidth: true
            text: nodeName
            elide: Text.ElideRight
            font.pixelSize: 13
            color: {
                switch (gitStatus) {
                    case "M": return theme.yellow
                    case "A": return theme.green
                    case "D": return theme.red
                    case "?": return theme.fg_muted
                    default:  return theme.fg
                }
            }
        }

        // ── Git status badge ──────────────────────────────────────────────
        Rectangle {
            visible: gitStatus.length > 0
            Layout.preferredWidth: 16
            Layout.preferredHeight: 16
            radius: 3
            color: {
                switch (gitStatus) {
                    case "M": return Qt.rgba(
                        parseInt(theme.yellow.substring(1, 3), 16) / 255,
                        parseInt(theme.yellow.substring(3, 5), 16) / 255,
                        parseInt(theme.yellow.substring(5, 7), 16) / 255,
                        0.2
                    )
                    case "A": return Qt.rgba(
                        parseInt(theme.green.substring(1, 3), 16) / 255,
                        parseInt(theme.green.substring(3, 5), 16) / 255,
                        parseInt(theme.green.substring(5, 7), 16) / 255,
                        0.2
                    )
                    case "D": return Qt.rgba(
                        parseInt(theme.red.substring(1, 3), 16) / 255,
                        parseInt(theme.red.substring(3, 5), 16) / 255,
                        parseInt(theme.red.substring(5, 7), 16) / 255,
                        0.2
                    )
                    case "?": return Qt.rgba(0.5, 0.5, 0.5, 0.15)
                    default:  return "transparent"
                }
            }
            Text {
                anchors.centerIn: parent
                text: gitStatus
                font.pixelSize: 10
                font.bold: true
                color: {
                    switch (gitStatus) {
                        case "M": return theme.yellow
                        case "A": return theme.green
                        case "D": return theme.red
                        case "?": return theme.fg_muted
                        default:  return theme.fg_muted
                    }
                }
            }
        }
    }

    // ── Mouse interaction ─────────────────────────────────────────────────────
    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton

        onClicked: function(mouse) {
            if (mouse.button === Qt.RightButton) {
                contextMenu.popup()
                return
            }
            if (isDir) {
                fileTreeModel.toggle_expand(nodePath)
            } else {
                fileTreeModel.active_file_path = nodePath
                fileTreeModel.file_activated(nodePath)
            }
        }
    }

    // ── Context menu ──────────────────────────────────────────────────────────
    Menu {
        id: contextMenu

        background: Rectangle {
            color: theme.bg_surface
            border.color: theme.border
            border.width: 1
            radius: 6
        }

        MenuItem {
            text: "New File"
            onTriggered: newItemDialog.openForFile()
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuItem {
            text: "New Folder"
            onTriggered: newItemDialog.openForFolder()
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuSeparator {
            contentItem: Rectangle { implicitHeight: 1; color: theme.border }
        }
        MenuItem {
            text: "Rename"
            onTriggered: renameDialog.openForNode()
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuItem {
            text: "Delete"
            onTriggered: deleteConfirmDialog.open()
            contentItem: Text { text: parent.text; color: theme.red; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuSeparator {
            contentItem: Rectangle { implicitHeight: 1; color: theme.border }
        }
        MenuItem {
            text: "Copy Path"
            onTriggered: {
                var cb = Qt.application.clipboard
                if (cb) cb.text = nodePath
            }
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuItem {
            text: "Reveal in File Manager"
            onTriggered: Qt.openUrlExternally("file://" + (isDir ? nodePath : nodePath.substring(0, nodePath.lastIndexOf("/"))))
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
    }

    // ── Inline dialogs ────────────────────────────────────────────────────────
    Dialog {
        id: newItemDialog
        property bool isFolder: false

        function openForFile()   { isFolder = false; newItemInput.text = ""; open(); newItemInput.forceActiveFocus() }
        function openForFolder() { isFolder = true;  newItemInput.text = ""; open(); newItemInput.forceActiveFocus() }

        title: isFolder ? "New Folder" : "New File"
        anchors.centerIn: Overlay.overlay
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        background: Rectangle { color: theme.bg_surface; border.color: theme.border; border.width: 1; radius: 6 }

        ColumnLayout {
            spacing: 8
            Label { text: "Name:"; color: theme.fg }
            TextField {
                id: newItemInput
                Layout.preferredWidth: 260
                color: theme.fg
                background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                onAccepted: newItemDialog.accept()
            }
        }

        onAccepted: {
            if (newItemInput.text.length === 0) return
            var parent_dir = isDir ? nodePath : nodePath.substring(0, nodePath.lastIndexOf("/"))
            if (isFolder) {
                fileTreeModel.create_folder(parent_dir, newItemInput.text)
            } else {
                fileTreeModel.create_file(parent_dir, newItemInput.text)
            }
        }
    }

    Dialog {
        id: renameDialog
        function openForNode() {
            renameInput.text = nodeName
            open()
            renameInput.forceActiveFocus()
            renameInput.selectAll()
        }

        title: "Rename"
        anchors.centerIn: Overlay.overlay
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        background: Rectangle { color: theme.bg_surface; border.color: theme.border; border.width: 1; radius: 6 }

        ColumnLayout {
            spacing: 8
            Label { text: "New name:"; color: theme.fg }
            TextField {
                id: renameInput
                Layout.preferredWidth: 260
                color: theme.fg
                background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                onAccepted: renameDialog.accept()
            }
        }

        onAccepted: {
            if (renameInput.text.length > 0 && renameInput.text !== nodeName) {
                fileTreeModel.rename_item(nodePath, renameInput.text)
            }
        }
    }

    Dialog {
        id: deleteConfirmDialog
        title: "Delete"
        anchors.centerIn: Overlay.overlay
        modal: true
        standardButtons: Dialog.Yes | Dialog.No
        background: Rectangle { color: theme.bg_surface; border.color: theme.border; border.width: 1; radius: 6 }

        Label {
            text: "Delete \"" + nodeName + "\"?"
            color: theme.fg
            wrapMode: Text.WordWrap
        }

        onAccepted: fileTreeModel.delete_item(nodePath)
    }
}
