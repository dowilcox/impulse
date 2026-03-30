// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Popup {
    id: commandPaletteRoot
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    width: Math.min(520, parent.width * 0.55)
    height: Math.min(400, parent.height * 0.55)
    padding: 0

    property int selectedIndex: 0
    property var filteredCommands: []

    // ── Command registry ──────────────────────────────────────────────────────
    readonly property var commands: [
        { name: "New Terminal",           shortcut: "Ctrl+T",       action: function() { windowModel.create_tab("terminal") } },
        { name: "Close Tab",             shortcut: "Ctrl+W",       action: function() { if (windowModel.tab_count > 0) windowModel.close_tab(windowModel.active_tab_index) } },
        { name: "Toggle Sidebar",        shortcut: "Ctrl+B",       action: function() { windowModel.toggle_sidebar() } },
        { name: "Quick Open",            shortcut: "Ctrl+P",       action: function() { quickOpenDialog.open() } },
        { name: "Go to Line",            shortcut: "Ctrl+G",       action: function() { goToLineDialog.open() } },
        { name: "Save File",             shortcut: "Ctrl+S",       action: function() { contentArea.saveActiveEditor() } },
        { name: "Settings",              shortcut: "Ctrl+,",       action: function() { root.openSettings() } },
        { name: "Next Tab",              shortcut: "Ctrl+Tab",     action: function() {
            if (windowModel.tab_count > 1) windowModel.select_tab((windowModel.active_tab_index + 1) % windowModel.tab_count)
        }},
        { name: "Previous Tab",          shortcut: "Ctrl+Shift+Tab", action: function() {
            if (windowModel.tab_count > 1) windowModel.select_tab((windowModel.active_tab_index - 1 + windowModel.tab_count) % windowModel.tab_count)
        }},
        { name: "New File",              shortcut: "",              action: function() { /* trigger sidebar new file dialog */ } },
        { name: "New Folder",            shortcut: "",              action: function() { /* trigger sidebar new folder dialog */ } },
        { name: "Refresh File Tree",     shortcut: "",              action: function() { fileTreeModel.refresh() } },
        { name: "Toggle Hidden Files",   shortcut: "",              action: function() { fileTreeModel.show_hidden = !fileTreeModel.show_hidden; fileTreeModel.refresh() } },
        { name: "Search in Files",       shortcut: "",              action: function() { sidebar.searchMode = true } },
        { name: "Toggle Preview",        shortcut: "",              action: function() {
            var view = contentArea.contentItems[windowModel.active_tab_index]
            if (view && view.togglePreview) view.togglePreview()
        }},
        { name: "Reset Settings",        shortcut: "",              action: function() { settings.reset_to_defaults() } },
        { name: "Install LSP Servers",   shortcut: "",              action: function() { lspBridge.install_servers() } },
        { name: "Check LSP Status",      shortcut: "",              action: function() { lspBridge.check_server_status() } }
    ]

    background: Rectangle {
        color: theme.bg_surface
        border.color: theme.border
        border.width: 1
        radius: 8
    }

    Overlay.modal: Rectangle {
        color: Qt.rgba(0, 0, 0, 0.4)
    }

    onOpened: {
        cmdInput.text = ""
        filterCommands("")
        selectedIndex = 0
        cmdInput.forceActiveFocus()
    }

    function filterCommands(query) {
        if (query.length === 0) {
            filteredCommands = commands.slice()
            return
        }
        var q = query.toLowerCase()
        filteredCommands = commands.filter(function(cmd) {
            return cmd.name.toLowerCase().indexOf(q) >= 0
        })
    }

    function executeSelected() {
        if (filteredCommands.length > 0 && selectedIndex >= 0 && selectedIndex < filteredCommands.length) {
            var cmd = filteredCommands[selectedIndex]
            close()
            if (cmd.action) cmd.action()
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 0
        spacing: 0

        // ── Search input ──────────────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 44
            color: theme.bg_surface
            radius: 8

            Rectangle {
                anchors.bottom: parent.bottom
                anchors.left: parent.left
                anchors.right: parent.right
                height: parent.radius
                color: parent.color
            }

            RowLayout {
                anchors.fill: parent
                anchors.margins: 8
                spacing: 6

                Text {
                    text: ">"
                    font.pixelSize: 14
                    font.bold: true
                    color: theme.accent
                    Layout.alignment: Qt.AlignVCenter
                }

                TextField {
                    id: cmdInput
                    Layout.fillWidth: true
                    placeholderText: "Type a command..."
                    color: theme.fg
                    font.pixelSize: 14
                    leftPadding: 4
                    rightPadding: 4

                    background: Rectangle {
                        color: theme.bg
                        border.color: cmdInput.activeFocus ? theme.accent : theme.border
                        border.width: 1
                        radius: 6
                    }

                    onTextChanged: {
                        filterCommands(text.trim())
                        selectedIndex = 0
                    }

                    Keys.onDownPressed: {
                        if (selectedIndex < filteredCommands.length - 1) {
                            selectedIndex++
                            cmdList.positionViewAtIndex(selectedIndex, ListView.Contain)
                        }
                    }
                    Keys.onUpPressed: {
                        if (selectedIndex > 0) {
                            selectedIndex--
                            cmdList.positionViewAtIndex(selectedIndex, ListView.Contain)
                        }
                    }
                    Keys.onReturnPressed: executeSelected()
                    Keys.onEnterPressed: executeSelected()
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: theme.border
        }

        // ── Command list ──────────────────────────────────────────────────
        ListView {
            id: cmdList
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: filteredCommands.length
            boundsBehavior: Flickable.StopAtBounds

            ScrollBar.vertical: ScrollBar {
                policy: ScrollBar.AsNeeded
                background: Rectangle { color: "transparent" }
                contentItem: Rectangle {
                    implicitWidth: 6
                    radius: 3
                    color: theme.fg_muted
                    opacity: 0.4
                }
            }

            delegate: Rectangle {
                width: cmdList.width
                height: 34
                color: {
                    if (index === selectedIndex) return theme.bg_highlight
                    if (cmdMouse.containsMouse) return Qt.rgba(
                        parseInt(theme.bg_highlight.substring(1, 3), 16) / 255,
                        parseInt(theme.bg_highlight.substring(3, 5), 16) / 255,
                        parseInt(theme.bg_highlight.substring(5, 7), 16) / 255,
                        0.5
                    )
                    return "transparent"
                }

                readonly property var cmdData: filteredCommands[index] || {}

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 16
                    anchors.rightMargin: 16
                    spacing: 8

                    Text {
                        text: cmdData.name || ""
                        font.pixelSize: 13
                        color: theme.fg
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                    }

                    Rectangle {
                        visible: (cmdData.shortcut || "").length > 0
                        Layout.preferredWidth: shortcutText.implicitWidth + 12
                        Layout.preferredHeight: 20
                        radius: 4
                        color: theme.bg_highlight

                        Text {
                            id: shortcutText
                            anchors.centerIn: parent
                            text: cmdData.shortcut || ""
                            font.pixelSize: 11
                            color: theme.fg_muted
                        }
                    }
                }

                MouseArea {
                    id: cmdMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: {
                        selectedIndex = index
                        executeSelected()
                    }
                }
            }
        }

        // ── Footer ────────────────────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            color: theme.bg_dark
            radius: 8

            Rectangle {
                anchors.top: parent.top
                anchors.left: parent.left
                anchors.right: parent.right
                height: parent.radius
                color: parent.color
            }

            Rectangle {
                anchors.top: parent.top
                anchors.left: parent.left
                anchors.right: parent.right
                height: 1
                color: theme.border
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 12
                anchors.rightMargin: 12
                anchors.topMargin: 1

                Text {
                    text: filteredCommands.length + " command" + (filteredCommands.length !== 1 ? "s" : "")
                    font.pixelSize: 11
                    color: theme.fg_muted
                }
                Item { Layout.fillWidth: true }
                Text {
                    text: "\u2191\u2193 Navigate  \u23CE Execute  Esc Close"
                    font.pixelSize: 11
                    color: theme.fg_muted
                }
            }
        }
    }
}
