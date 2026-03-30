// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Dialog {
    id: commandPaletteRoot
    modal: true
    title: "Command Palette"
    standardButtons: Dialog.Close
    width: Math.min(520, parent ? parent.width * 0.55 : 520)
    height: Math.min(400, parent ? parent.height * 0.55 : 400)
    anchors.centerIn: Overlay.overlay

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
        { name: "Next Tab",              shortcut: "Ctrl+Tab",     action: function() {
            if (windowModel.tab_count > 1) windowModel.select_tab((windowModel.active_tab_index + 1) % windowModel.tab_count)
        }},
        { name: "Previous Tab",          shortcut: "Ctrl+Shift+Tab", action: function() {
            if (windowModel.tab_count > 1) windowModel.select_tab((windowModel.active_tab_index - 1 + windowModel.tab_count) % windowModel.tab_count)
        }},
        { name: "Refresh File Tree",     shortcut: "",              action: function() { fileTreeModel.refresh() } },
        { name: "Toggle Hidden Files",   shortcut: "",              action: function() { fileTreeModel.show_hidden = !fileTreeModel.show_hidden; fileTreeModel.refresh() } },
        { name: "Search in Files",       shortcut: "Ctrl+Shift+F", action: function() { sidebar.searchMode = true; windowModel.sidebar_visible = true } },
        { name: "Toggle Preview",        shortcut: "",              action: function() {
            var view = contentArea.contentItems[windowModel.active_tab_index]
            if (view && view.togglePreview) view.togglePreview()
        }},
        { name: "Reset Settings",        shortcut: "",              action: function() { settings.reset_to_defaults() } },
        { name: "Install LSP Servers",   shortcut: "",              action: function() { lspBridge.install_servers() } },
        { name: "Check LSP Status",      shortcut: "",              action: function() { lspBridge.check_server_status() } }
    ]

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
        spacing: 8

        // ── Search input ──────────────────────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            Label {
                text: ">"
                font.pixelSize: 14
                font.bold: true
                color: palette.highlight
            }

            TextField {
                id: cmdInput
                Layout.fillWidth: true
                placeholderText: "Type a command..."
                font.pixelSize: 14

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
            }

            delegate: ItemDelegate {
                width: cmdList.width
                highlighted: index === selectedIndex

                readonly property var cmdData: filteredCommands[index] || {}

                contentItem: RowLayout {
                    spacing: 8

                    Label {
                        text: cmdData.name || ""
                        font.pixelSize: 13
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }

                    Label {
                        visible: (cmdData.shortcut || "").length > 0
                        text: cmdData.shortcut || ""
                        font.pixelSize: 11
                        opacity: 0.6
                    }
                }

                onClicked: {
                    selectedIndex = index
                    executeSelected()
                }
            }
        }

        // ── Footer ────────────────────────────────────────────────────────
        Label {
            Layout.fillWidth: true
            text: filteredCommands.length + " command" + (filteredCommands.length !== 1 ? "s" : "") + "   \u2191\u2193 Navigate  \u23CE Execute  Esc Close"
            font.pixelSize: 11
            opacity: 0.6
        }
    }
}
