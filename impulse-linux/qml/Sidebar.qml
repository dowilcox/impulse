// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Pane {
    id: sidebarRoot
    padding: 0

    background: Rectangle {
        color: theme.bg_surface
        border.color: theme.border
        border.width: 1
    }

    // Whether we are showing the search panel instead of the file tree
    property bool searchMode: false

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // ── Toolbar ───────────────────────────────────────────────────────
        ToolBar {
            Layout.fillWidth: true
            position: ToolBar.Header
            padding: 0

            background: Rectangle {
                color: theme.bg_dark

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: theme.border
                }
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                spacing: 6

                ChromeToolButton {
                    text: "Files"
                    font.bold: !sidebarRoot.searchMode
                    checked: !sidebarRoot.searchMode
                    onClicked: sidebarRoot.searchMode = false
                    ToolTip.visible: hovered
                    ToolTip.text: "File Explorer"
                    ToolTip.delay: 600
                }

                ChromeToolButton {
                    text: "Search"
                    font.bold: sidebarRoot.searchMode
                    checked: sidebarRoot.searchMode
                    onClicked: sidebarRoot.searchMode = true
                    ToolTip.visible: hovered
                    ToolTip.text: "Project Search"
                    ToolTip.delay: 600
                }

                Item { Layout.fillWidth: true }

                ChromeToolButton {
                    icon.name: "document-new"
                    visible: !sidebarRoot.searchMode
                    onClicked: newFileDialog.open()
                    ToolTip.visible: hovered
                    ToolTip.text: "New File"
                    ToolTip.delay: 600
                }

                ChromeToolButton {
                    icon.name: "folder-new"
                    visible: !sidebarRoot.searchMode
                    onClicked: newFolderDialog.open()
                    ToolTip.visible: hovered
                    ToolTip.text: "New Folder"
                    ToolTip.delay: 600
                }

                ChromeToolButton {
                    icon.name: fileTreeModel.show_hidden ? "view-visible" : "view-hidden"
                    visible: !sidebarRoot.searchMode
                    checked: fileTreeModel.show_hidden
                    onClicked: {
                        fileTreeModel.show_hidden = !fileTreeModel.show_hidden
                        fileTreeModel.refresh()
                    }
                    ToolTip.visible: hovered
                    ToolTip.text: fileTreeModel.show_hidden ? "Hide Hidden Files" : "Show Hidden Files"
                    ToolTip.delay: 600
                }

                ChromeToolButton {
                    icon.name: "view-refresh"
                    visible: !sidebarRoot.searchMode
                    onClicked: fileTreeModel.refresh()
                    ToolTip.visible: hovered
                    ToolTip.text: "Refresh"
                    ToolTip.delay: 600
                }
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
    ChromeDialog {
        id: newFileDialog
        title: "New File"
        anchors.centerIn: Overlay.overlay
        standardButtons: Dialog.Ok | Dialog.Cancel

        ColumnLayout {
            spacing: 8
            Label { text: "File name:"; color: theme.fg_muted }
            ChromeTextField {
                id: newFileInput
                Layout.preferredWidth: 280
                placeholderText: "filename.ext"
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
    ChromeDialog {
        id: newFolderDialog
        title: "New Folder"
        anchors.centerIn: Overlay.overlay
        standardButtons: Dialog.Ok | Dialog.Cancel

        ColumnLayout {
            spacing: 8
            Label { text: "Folder name:"; color: theme.fg_muted }
            ChromeTextField {
                id: newFolderInput
                Layout.preferredWidth: 280
                placeholderText: "folder-name"
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
