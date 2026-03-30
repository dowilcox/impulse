// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Rectangle {
    id: sidebarRoot
    color: theme.bg_dark

    // Whether we are showing the search panel instead of the file tree
    property bool searchMode: false

    // Right-side border
    Rectangle {
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: 1
        color: theme.border
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.rightMargin: 1  // leave room for border
        spacing: 0

        // ── Toolbar ───────────────────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 36
            color: theme.bg_dark

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 6
                anchors.rightMargin: 6
                spacing: 2

                ToolButton {
                    id: filesBtn
                    text: "Files"
                    font.pixelSize: 12
                    font.bold: !sidebarRoot.searchMode
                    palette.buttonText: sidebarRoot.searchMode ? theme.fg_muted : theme.fg
                    background: Rectangle {
                        color: !sidebarRoot.searchMode ? theme.bg_highlight : "transparent"
                        radius: 4
                    }
                    onClicked: sidebarRoot.searchMode = false

                    ToolTip.visible: hovered
                    ToolTip.text: "File Explorer"
                    ToolTip.delay: 600
                }

                ToolButton {
                    id: searchBtn
                    text: "Search"
                    font.pixelSize: 12
                    font.bold: sidebarRoot.searchMode
                    palette.buttonText: sidebarRoot.searchMode ? theme.fg : theme.fg_muted
                    background: Rectangle {
                        color: sidebarRoot.searchMode ? theme.bg_highlight : "transparent"
                        radius: 4
                    }
                    onClicked: sidebarRoot.searchMode = true

                    ToolTip.visible: hovered
                    ToolTip.text: "Project Search"
                    ToolTip.delay: 600
                }

                Item { Layout.fillWidth: true }

                ToolButton {
                    id: newFileBtn
                    text: "\uFF0B"  // full-width plus
                    font.pixelSize: 14
                    visible: !sidebarRoot.searchMode
                    palette.buttonText: theme.fg_muted
                    background: Rectangle {
                        color: newFileBtn.hovered ? theme.bg_highlight : "transparent"
                        radius: 4
                    }
                    onClicked: newFileDialog.open()

                    ToolTip.visible: hovered
                    ToolTip.text: "New File"
                    ToolTip.delay: 600
                }

                ToolButton {
                    id: newFolderBtn
                    text: "\uD83D\uDCC1"
                    font.pixelSize: 13
                    visible: !sidebarRoot.searchMode
                    palette.buttonText: theme.fg_muted
                    background: Rectangle {
                        color: newFolderBtn.hovered ? theme.bg_highlight : "transparent"
                        radius: 4
                    }
                    onClicked: newFolderDialog.open()

                    ToolTip.visible: hovered
                    ToolTip.text: "New Folder"
                    ToolTip.delay: 600
                }

                ToolButton {
                    id: hiddenBtn
                    text: fileTreeModel.show_hidden ? "H" : "h"
                    font.pixelSize: 12
                    font.bold: fileTreeModel.show_hidden
                    visible: !sidebarRoot.searchMode
                    palette.buttonText: fileTreeModel.show_hidden ? theme.accent : theme.fg_muted
                    background: Rectangle {
                        color: hiddenBtn.hovered ? theme.bg_highlight : "transparent"
                        radius: 4
                    }
                    onClicked: {
                        fileTreeModel.show_hidden = !fileTreeModel.show_hidden
                        fileTreeModel.refresh()
                    }

                    ToolTip.visible: hovered
                    ToolTip.text: fileTreeModel.show_hidden ? "Hide Hidden Files" : "Show Hidden Files"
                    ToolTip.delay: 600
                }

                ToolButton {
                    id: refreshBtn
                    text: "\u21BB"
                    font.pixelSize: 14
                    visible: !sidebarRoot.searchMode
                    palette.buttonText: theme.fg_muted
                    background: Rectangle {
                        color: refreshBtn.hovered ? theme.bg_highlight : "transparent"
                        radius: 4
                    }
                    onClicked: fileTreeModel.refresh()

                    ToolTip.visible: hovered
                    ToolTip.text: "Refresh"
                    ToolTip.delay: 600
                }
            }

            // Bottom border for toolbar
            Rectangle {
                anchors.bottom: parent.bottom
                anchors.left: parent.left
                anchors.right: parent.right
                height: 1
                color: theme.border
            }
        }

        // ── Content ───────────────────────────────────────────────────────
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            FileTreeView {
                id: fileTreeView
                anchors.fill: parent
                visible: !sidebarRoot.searchMode
            }

            SearchPanel {
                id: searchPanel
                anchors.fill: parent
                visible: sidebarRoot.searchMode
            }
        }
    }

    // ── New-file dialog ───────────────────────────────────────────────────────
    Dialog {
        id: newFileDialog
        title: "New File"
        anchors.centerIn: Overlay.overlay
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        background: Rectangle { color: theme.bg_surface; border.color: theme.border; border.width: 1; radius: 6 }

        ColumnLayout {
            spacing: 8
            Label {
                text: "File name:"
                color: theme.fg
            }
            TextField {
                id: newFileInput
                Layout.preferredWidth: 280
                color: theme.fg
                placeholderText: "filename.ext"
                background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                onAccepted: newFileDialog.accept()
            }
        }

        onOpened: {
            newFileInput.text = ""
            newFileInput.forceActiveFocus()
        }
        onAccepted: {
            if (newFileInput.text.length > 0) {
                fileTreeModel.create_file(fileTreeModel.root_path, newFileInput.text)
            }
        }
    }

    // ── New-folder dialog ─────────────────────────────────────────────────────
    Dialog {
        id: newFolderDialog
        title: "New Folder"
        anchors.centerIn: Overlay.overlay
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        background: Rectangle { color: theme.bg_surface; border.color: theme.border; border.width: 1; radius: 6 }

        ColumnLayout {
            spacing: 8
            Label {
                text: "Folder name:"
                color: theme.fg
            }
            TextField {
                id: newFolderInput
                Layout.preferredWidth: 280
                color: theme.fg
                placeholderText: "folder-name"
                background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                onAccepted: newFolderDialog.accept()
            }
        }

        onOpened: {
            newFolderInput.text = ""
            newFolderInput.forceActiveFocus()
        }
        onAccepted: {
            if (newFolderInput.text.length > 0) {
                fileTreeModel.create_folder(fileTreeModel.root_path, newFolderInput.text)
            }
        }
    }
}
