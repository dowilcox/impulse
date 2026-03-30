// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import dev.impulse.app

QQC2.ApplicationWindow {
    id: settingsRoot
    title: "Impulse Settings"
    width: 700
    height: 560
    minimumWidth: 560
    minimumHeight: 420
    color: theme.bg

    // Parsed JSON data for list-based settings
    property var availableThemes: {
        try { return JSON.parse(theme.available_themes_json) } catch (e) { return [] }
    }
    property var fileTypeOverrides: {
        try { return JSON.parse(settings.file_type_overrides_json) } catch (e) { return [] }
    }
    property var commandsOnSave: {
        try { return JSON.parse(settings.commands_on_save_json) } catch (e) { return [] }
    }
    property var keybindingOverrides: {
        try { return JSON.parse(settings.keybinding_overrides_json) } catch (e) { return ({}) }
    }
    property var customKeybindings: {
        try { return JSON.parse(settings.custom_keybindings_json) } catch (e) { return [] }
    }

    // Auto-save changes when any setting is modified
    function settingChanged() {
        saveDebounce.restart()
    }

    Timer {
        id: saveDebounce
        interval: 500
        repeat: false
        onTriggered: settings.save()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // ── Tab bar ───────────────────────────────────────────────────────
        QQC2.TabBar {
            id: settingsTabBar
            Layout.fillWidth: true
            background: Rectangle { color: theme.bg_dark }

            QQC2.TabButton {
                text: "Appearance"
                width: implicitWidth
                font.pixelSize: 13
                palette.buttonText: settingsTabBar.currentIndex === 0 ? theme.fg : theme.fg_muted
                background: Rectangle {
                    color: settingsTabBar.currentIndex === 0 ? theme.bg : theme.bg_dark
                }
            }
            QQC2.TabButton {
                text: "Editor"
                width: implicitWidth
                font.pixelSize: 13
                palette.buttonText: settingsTabBar.currentIndex === 1 ? theme.fg : theme.fg_muted
                background: Rectangle {
                    color: settingsTabBar.currentIndex === 1 ? theme.bg : theme.bg_dark
                }
            }
            QQC2.TabButton {
                text: "Terminal"
                width: implicitWidth
                font.pixelSize: 13
                palette.buttonText: settingsTabBar.currentIndex === 2 ? theme.fg : theme.fg_muted
                background: Rectangle {
                    color: settingsTabBar.currentIndex === 2 ? theme.bg : theme.bg_dark
                }
            }
            QQC2.TabButton {
                text: "Keybindings"
                width: implicitWidth
                font.pixelSize: 13
                palette.buttonText: settingsTabBar.currentIndex === 3 ? theme.fg : theme.fg_muted
                background: Rectangle {
                    color: settingsTabBar.currentIndex === 3 ? theme.bg : theme.bg_dark
                }
            }
            QQC2.TabButton {
                text: "File Types"
                width: implicitWidth
                font.pixelSize: 13
                palette.buttonText: settingsTabBar.currentIndex === 4 ? theme.fg : theme.fg_muted
                background: Rectangle {
                    color: settingsTabBar.currentIndex === 4 ? theme.bg : theme.bg_dark
                }
            }
            QQC2.TabButton {
                text: "Automation"
                width: implicitWidth
                font.pixelSize: 13
                palette.buttonText: settingsTabBar.currentIndex === 5 ? theme.fg : theme.fg_muted
                background: Rectangle {
                    color: settingsTabBar.currentIndex === 5 ? theme.bg : theme.bg_dark
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: theme.border
        }

        // ── Tab content ───────────────────────────────────────────────────
        StackLayout {
            id: settingsStack
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: settingsTabBar.currentIndex

            // ═══════════════════════════════════════════════════════════════
            // Tab 0: Appearance
            // ═══════════════════════════════════════════════════════════════
            QQC2.ScrollView {
                clip: true
                QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

                ColumnLayout {
                    width: settingsStack.width - 40
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 16

                    Item { Layout.preferredHeight: 12 }

                    SettingsSection { title: "Theme" }

                    SettingsRow {
                        label: "Color Theme"
                        RowLayout {
                            spacing: 8
                            QQC2.ComboBox {
                                id: themeCombo
                                model: availableThemes.map(function(t) { return t.name || t.id })
                                currentIndex: {
                                    for (var i = 0; i < availableThemes.length; i++) {
                                        if (availableThemes[i].id === theme.theme_id) return i
                                    }
                                    return 0
                                }
                                onCurrentIndexChanged: {
                                    if (currentIndex >= 0 && currentIndex < availableThemes.length) {
                                        var id = availableThemes[currentIndex].id
                                        settings.set_setting("colorScheme", id)
                                        theme.set_theme(id)
                                        settingChanged()
                                    }
                                }
                                Layout.preferredWidth: 200
                                palette.window: theme.bg
                                palette.windowText: theme.fg
                                palette.button: theme.bg_surface
                                palette.buttonText: theme.fg
                                palette.highlight: theme.accent
                            }

                            // Theme preview swatch
                            Row {
                                spacing: 4
                                Repeater {
                                    model: [theme.bg, theme.bg_dark, theme.accent, theme.red, theme.yellow, theme.green, theme.blue, theme.magenta]
                                    Rectangle {
                                        width: 16; height: 16; radius: 3
                                        color: modelData
                                        border.color: theme.border; border.width: 1
                                    }
                                }
                            }
                        }
                    }

                    SettingsSection { title: "Font" }

                    SettingsRow {
                        label: "Font Family"
                        QQC2.TextField {
                            text: settings.font_family
                            Layout.preferredWidth: 200
                            color: theme.fg
                            font.pixelSize: 13
                            background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                            onTextChanged: {
                                settings.set_setting("fontFamily", text)
                                settingChanged()
                            }
                        }
                    }

                    SettingsRow {
                        label: "Font Size"
                        QQC2.SpinBox {
                            value: settings.font_size
                            from: 8
                            to: 32
                            onValueChanged: {
                                settings.set_setting("fontSize", value.toString())
                                settingChanged()
                            }
                            palette.window: theme.bg
                            palette.windowText: theme.fg
                            palette.button: theme.bg_surface
                            palette.buttonText: theme.fg
                        }
                    }

                    Item { Layout.fillHeight: true }
                }
            }

            // ═══════════════════════════════════════════════════════════════
            // Tab 1: Editor
            // ═══════════════════════════════════════════════════════════════
            QQC2.ScrollView {
                clip: true
                QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

                ColumnLayout {
                    width: settingsStack.width - 40
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 16

                    Item { Layout.preferredHeight: 12 }

                    SettingsSection { title: "Indentation" }

                    SettingsRow {
                        label: "Tab Width"
                        QQC2.SpinBox {
                            value: settings.tab_width
                            from: 1
                            to: 8
                            onValueChanged: {
                                settings.set_setting("tabWidth", value.toString())
                                settingChanged()
                            }
                            palette.window: theme.bg
                            palette.windowText: theme.fg
                            palette.button: theme.bg_surface
                            palette.buttonText: theme.fg
                        }
                    }

                    SettingsRow {
                        label: "Use Spaces"
                        QQC2.Switch {
                            checked: settings.use_spaces
                            onCheckedChanged: {
                                settings.set_setting("useSpaces", checked ? "true" : "false")
                                settingChanged()
                            }
                            palette.highlight: theme.accent
                        }
                    }

                    SettingsSection { title: "Display" }

                    SettingsRow {
                        label: "Word Wrap"
                        QQC2.Switch {
                            checked: settings.word_wrap
                            onCheckedChanged: {
                                settings.set_setting("wordWrap", checked ? "true" : "false")
                                settingChanged()
                            }
                            palette.highlight: theme.accent
                        }
                    }

                    SettingsRow {
                        label: "Minimap"
                        QQC2.Switch {
                            checked: settings.minimap_enabled
                            onCheckedChanged: {
                                settings.set_setting("minimapEnabled", checked ? "true" : "false")
                                settingChanged()
                            }
                            palette.highlight: theme.accent
                        }
                    }

                    SettingsRow {
                        label: "Line Numbers"
                        QQC2.Switch {
                            checked: settings.show_line_numbers
                            onCheckedChanged: {
                                settings.set_setting("showLineNumbers", checked ? "true" : "false")
                                settingChanged()
                            }
                            palette.highlight: theme.accent
                        }
                    }

                    SettingsSection { title: "Behavior" }

                    SettingsRow {
                        label: "Auto Save"
                        QQC2.Switch {
                            checked: settings.auto_save
                            onCheckedChanged: {
                                settings.set_setting("autoSave", checked ? "true" : "false")
                                settingChanged()
                            }
                            palette.highlight: theme.accent
                        }
                    }

                    Item { Layout.fillHeight: true }
                }
            }

            // ═══════════════════════════════════════════════════════════════
            // Tab 2: Terminal
            // ═══════════════════════════════════════════════════════════════
            QQC2.ScrollView {
                clip: true
                QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

                ColumnLayout {
                    width: settingsStack.width - 40
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 16

                    Item { Layout.preferredHeight: 12 }

                    SettingsSection { title: "Font" }

                    SettingsRow {
                        label: "Font Family"
                        QQC2.TextField {
                            text: settings.terminal_font_family
                            Layout.preferredWidth: 200
                            color: theme.fg
                            font.pixelSize: 13
                            background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                            onTextChanged: {
                                settings.set_setting("terminalFontFamily", text)
                                settingChanged()
                            }
                        }
                    }

                    SettingsRow {
                        label: "Font Size"
                        QQC2.SpinBox {
                            value: settings.terminal_font_size
                            from: 8
                            to: 32
                            onValueChanged: {
                                settings.set_setting("terminalFontSize", value.toString())
                                settingChanged()
                            }
                            palette.window: theme.bg
                            palette.windowText: theme.fg
                            palette.button: theme.bg_surface
                            palette.buttonText: theme.fg
                        }
                    }

                    SettingsSection { title: "Behavior" }

                    SettingsRow {
                        label: "Scrollback Lines"
                        QQC2.SpinBox {
                            value: settings.terminal_scrollback
                            from: 100
                            to: 100000
                            stepSize: 500
                            onValueChanged: {
                                settings.set_setting("terminalScrollback", value.toString())
                                settingChanged()
                            }
                            palette.window: theme.bg
                            palette.windowText: theme.fg
                            palette.button: theme.bg_surface
                            palette.buttonText: theme.fg
                        }
                    }

                    SettingsRow {
                        label: "Cursor Shape"
                        RowLayout {
                            spacing: 8
                            Repeater {
                                model: ["block", "underline", "beam"]
                                Rectangle {
                                    width: cursorLabel.implicitWidth + 20
                                    height: 28
                                    radius: 6
                                    color: settings.terminal_cursor_shape === modelData ? theme.accent : theme.bg_highlight
                                    border.color: settings.terminal_cursor_shape === modelData ? theme.accent : theme.border
                                    border.width: 1

                                    Text {
                                        id: cursorLabel
                                        anchors.centerIn: parent
                                        text: modelData.charAt(0).toUpperCase() + modelData.slice(1)
                                        font.pixelSize: 12
                                        color: settings.terminal_cursor_shape === modelData ? theme.bg : theme.fg
                                    }

                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: {
                                            settings.set_setting("terminalCursorShape", modelData)
                                            settingChanged()
                                        }
                                    }
                                }
                            }
                        }
                    }

                    SettingsRow {
                        label: "Cursor Blink"
                        QQC2.Switch {
                            checked: settings.terminal_cursor_blink
                            onCheckedChanged: {
                                settings.set_setting("terminalCursorBlink", checked ? "true" : "false")
                                settingChanged()
                            }
                            palette.highlight: theme.accent
                        }
                    }

                    SettingsRow {
                        label: "Copy on Select"
                        QQC2.Switch {
                            checked: settings.terminal_copy_on_select
                            onCheckedChanged: {
                                settings.set_setting("terminalCopyOnSelect", checked ? "true" : "false")
                                settingChanged()
                            }
                            palette.highlight: theme.accent
                        }
                    }

                    Item { Layout.fillHeight: true }
                }
            }

            // ═══════════════════════════════════════════════════════════════
            // Tab 3: Keybindings
            // ═══════════════════════════════════════════════════════════════
            QQC2.ScrollView {
                clip: true
                QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

                ColumnLayout {
                    width: settingsStack.width - 40
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 8

                    Item { Layout.preferredHeight: 12 }

                    SettingsSection { title: "Built-in Keybinding Overrides" }

                    Text {
                        text: "Override default keybindings by entering new key sequences below.\nLeave blank to keep the default."
                        font.pixelSize: 12
                        color: theme.fg_muted
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }

                    Repeater {
                        model: [
                            { id: "newTerminal",     label: "New Terminal",     def: "Ctrl+T" },
                            { id: "closeTab",        label: "Close Tab",        def: "Ctrl+W" },
                            { id: "toggleSidebar",   label: "Toggle Sidebar",   def: "Ctrl+B" },
                            { id: "quickOpen",       label: "Quick Open",       def: "Ctrl+P" },
                            { id: "commandPalette",  label: "Command Palette",  def: "Ctrl+Shift+P" },
                            { id: "save",            label: "Save",             def: "Ctrl+S" },
                            { id: "goToLine",        label: "Go to Line",       def: "Ctrl+G" },
                            { id: "nextTab",         label: "Next Tab",         def: "Ctrl+Tab" },
                            { id: "prevTab",         label: "Previous Tab",     def: "Ctrl+Shift+Tab" },
                            { id: "settings",        label: "Settings",         def: "Ctrl+," }
                        ]

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 12

                            Text {
                                text: modelData.label
                                font.pixelSize: 13
                                color: theme.fg
                                Layout.preferredWidth: 160
                                Layout.alignment: Qt.AlignVCenter
                            }

                            QQC2.TextField {
                                Layout.preferredWidth: 160
                                placeholderText: modelData.def
                                text: keybindingOverrides[modelData.id] || ""
                                color: theme.fg
                                font.pixelSize: 12
                                background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                                onTextChanged: {
                                    var overrides = keybindingOverrides
                                    if (text.length > 0) {
                                        overrides[modelData.id] = text
                                    } else {
                                        delete overrides[modelData.id]
                                    }
                                    settings.set_setting("keybindingOverridesJson", JSON.stringify(overrides))
                                    settingChanged()
                                }
                            }

                            Text {
                                text: "(default: " + modelData.def + ")"
                                font.pixelSize: 11
                                color: theme.fg_muted
                                Layout.alignment: Qt.AlignVCenter
                            }
                        }
                    }

                    Item { Layout.preferredHeight: 16 }
                    SettingsSection { title: "Custom Keybindings" }

                    Text {
                        text: "Define custom keybindings that run shell commands."
                        font.pixelSize: 12
                        color: theme.fg_muted
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }

                    Repeater {
                        model: customKeybindings.length

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8

                            readonly property var binding: customKeybindings[index] || {}

                            QQC2.TextField {
                                Layout.preferredWidth: 120
                                placeholderText: "Key"
                                text: binding.key || ""
                                color: theme.fg
                                font.pixelSize: 12
                                background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                            }

                            QQC2.TextField {
                                Layout.fillWidth: true
                                placeholderText: "Command"
                                text: binding.command || ""
                                color: theme.fg
                                font.pixelSize: 12
                                background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                            }

                            QQC2.ToolButton {
                                text: "\u2212"
                                font.pixelSize: 14
                                palette.buttonText: theme.red
                                background: Rectangle { color: hovered ? theme.bg_highlight : "transparent"; radius: 4 }
                                onClicked: {
                                    var list = customKeybindings.slice()
                                    list.splice(index, 1)
                                    settings.set_setting("customKeybindingsJson", JSON.stringify(list))
                                    settingChanged()
                                }
                            }
                        }
                    }

                    QQC2.Button {
                        text: "+ Add Custom Keybinding"
                        font.pixelSize: 12
                        palette.buttonText: theme.accent
                        background: Rectangle {
                            color: parent.hovered ? theme.bg_highlight : "transparent"
                            border.color: theme.border
                            border.width: 1
                            radius: 4
                        }
                        onClicked: {
                            var list = customKeybindings.slice()
                            list.push({ key: "", command: "" })
                            settings.set_setting("customKeybindingsJson", JSON.stringify(list))
                            settingChanged()
                        }
                    }

                    Item { Layout.fillHeight: true }
                }
            }

            // ═══════════════════════════════════════════════════════════════
            // Tab 4: File Types
            // ═══════════════════════════════════════════════════════════════
            QQC2.ScrollView {
                clip: true
                QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

                ColumnLayout {
                    width: settingsStack.width - 40
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 12

                    Item { Layout.preferredHeight: 12 }

                    SettingsSection { title: "File Type Overrides" }

                    Text {
                        text: "Override editor settings for specific file patterns (e.g., *.py, Makefile)."
                        font.pixelSize: 12
                        color: theme.fg_muted
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }

                    Repeater {
                        model: fileTypeOverrides.length

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: ftContent.implicitHeight + 16
                            color: theme.bg_dark
                            radius: 6
                            border.color: theme.border
                            border.width: 1

                            readonly property var ftData: fileTypeOverrides[index] || {}

                            RowLayout {
                                id: ftContent
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 12

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6

                                    RowLayout {
                                        spacing: 8
                                        Text { text: "Pattern:"; font.pixelSize: 12; color: theme.fg_muted }
                                        QQC2.TextField {
                                            text: ftData.pattern || ""
                                            Layout.preferredWidth: 140
                                            color: theme.fg
                                            font.pixelSize: 12
                                            background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                                        }
                                    }
                                    RowLayout {
                                        spacing: 8
                                        Text { text: "Tab Width:"; font.pixelSize: 12; color: theme.fg_muted }
                                        QQC2.SpinBox {
                                            value: ftData.tabWidth || 4
                                            from: 1; to: 8
                                            palette.window: theme.bg
                                            palette.windowText: theme.fg
                                            palette.button: theme.bg_surface
                                            palette.buttonText: theme.fg
                                        }
                                        Text { text: "Spaces:"; font.pixelSize: 12; color: theme.fg_muted }
                                        QQC2.Switch {
                                            checked: ftData.useSpaces !== false
                                            palette.highlight: theme.accent
                                        }
                                    }
                                }

                                QQC2.ToolButton {
                                    text: "\u2212"
                                    font.pixelSize: 16
                                    palette.buttonText: theme.red
                                    background: Rectangle { color: hovered ? theme.bg_highlight : "transparent"; radius: 4 }
                                    onClicked: {
                                        settings.remove_file_type_override(index)
                                        settingChanged()
                                    }
                                }
                            }
                        }
                    }

                    QQC2.Button {
                        text: "+ Add File Type Override"
                        font.pixelSize: 12
                        palette.buttonText: theme.accent
                        background: Rectangle {
                            color: parent.hovered ? theme.bg_highlight : "transparent"
                            border.color: theme.border
                            border.width: 1
                            radius: 4
                        }
                        onClicked: {
                            settings.add_file_type_override(JSON.stringify({
                                pattern: "*.ext",
                                tabWidth: 4,
                                useSpaces: true
                            }))
                            settingChanged()
                        }
                    }

                    Item { Layout.fillHeight: true }
                }
            }

            // ═══════════════════════════════════════════════════════════════
            // Tab 5: Automation
            // ═══════════════════════════════════════════════════════════════
            QQC2.ScrollView {
                clip: true
                QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

                ColumnLayout {
                    width: settingsStack.width - 40
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 12

                    Item { Layout.preferredHeight: 12 }

                    SettingsSection { title: "Commands on Save" }

                    Text {
                        text: "Run commands automatically when files matching a pattern are saved."
                        font.pixelSize: 12
                        color: theme.fg_muted
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }

                    Repeater {
                        model: commandsOnSave.length

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: cosContent.implicitHeight + 16
                            color: theme.bg_dark
                            radius: 6
                            border.color: theme.border
                            border.width: 1

                            readonly property var cosData: commandsOnSave[index] || {}

                            RowLayout {
                                id: cosContent
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 12

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 6

                                    RowLayout {
                                        spacing: 8
                                        Text { text: "File Pattern:"; font.pixelSize: 12; color: theme.fg_muted }
                                        QQC2.TextField {
                                            text: cosData.pattern || ""
                                            Layout.preferredWidth: 140
                                            color: theme.fg
                                            font.pixelSize: 12
                                            background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                                        }
                                    }
                                    RowLayout {
                                        spacing: 8
                                        Text { text: "Command:"; font.pixelSize: 12; color: theme.fg_muted }
                                        QQC2.TextField {
                                            text: cosData.command || ""
                                            Layout.fillWidth: true
                                            color: theme.fg
                                            font.pixelSize: 12
                                            background: Rectangle { color: theme.bg; border.color: theme.border; border.width: 1; radius: 4 }
                                        }
                                    }
                                    RowLayout {
                                        spacing: 8
                                        Text { text: "Reload After:"; font.pixelSize: 12; color: theme.fg_muted }
                                        QQC2.Switch {
                                            checked: !!cosData.reloadAfter
                                            palette.highlight: theme.accent
                                        }
                                    }
                                }

                                QQC2.ToolButton {
                                    text: "\u2212"
                                    font.pixelSize: 16
                                    palette.buttonText: theme.red
                                    background: Rectangle { color: hovered ? theme.bg_highlight : "transparent"; radius: 4 }
                                    onClicked: {
                                        settings.remove_command_on_save(index)
                                        settingChanged()
                                    }
                                }
                            }
                        }
                    }

                    QQC2.Button {
                        text: "+ Add Command on Save"
                        font.pixelSize: 12
                        palette.buttonText: theme.accent
                        background: Rectangle {
                            color: parent.hovered ? theme.bg_highlight : "transparent"
                            border.color: theme.border
                            border.width: 1
                            radius: 4
                        }
                        onClicked: {
                            settings.add_command_on_save(JSON.stringify({
                                pattern: "*",
                                command: "",
                                reloadAfter: false
                            }))
                            settingChanged()
                        }
                    }

                    Item { Layout.fillHeight: true }
                }
            }
        }

        // ── Bottom bar with Reset ─────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: theme.border
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 44
            color: theme.bg_dark

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 12
                anchors.rightMargin: 12

                Item { Layout.fillWidth: true }

                QQC2.Button {
                    text: "Reset to Defaults"
                    font.pixelSize: 13
                    palette.buttonText: theme.red
                    background: Rectangle {
                        color: parent.hovered ? theme.bg_highlight : "transparent"
                        border.color: theme.border
                        border.width: 1
                        radius: 6
                    }
                    onClicked: resetConfirm.open()
                }
            }
        }
    }

    // ── Reset confirmation dialog ─────────────────────────────────────────────
    QQC2.Dialog {
        id: resetConfirm
        title: "Reset Settings"
        anchors.centerIn: QQC2.Overlay.overlay
        modal: true
        standardButtons: QQC2.Dialog.Yes | QQC2.Dialog.No
        background: Rectangle { color: theme.bg_surface; border.color: theme.border; border.width: 1; radius: 6 }

        QQC2.Label {
            text: "Reset all settings to their defaults?\nThis cannot be undone."
            color: theme.fg
            wrapMode: Text.WordWrap
        }

        onAccepted: {
            settings.reset_to_defaults()
            settings.save()
        }
    }

    // ── Reusable inline components ────────────────────────────────────────────

    component SettingsSection : ColumnLayout {
        property string title: ""
        Layout.fillWidth: true
        spacing: 4

        Text {
            text: title
            font.pixelSize: 15
            font.bold: true
            color: theme.fg
        }
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: theme.border
        }
    }

    component SettingsRow : RowLayout {
        property string label: ""
        Layout.fillWidth: true
        spacing: 16

        Text {
            text: label
            font.pixelSize: 13
            color: theme.fg
            Layout.preferredWidth: 160
            Layout.alignment: Qt.AlignVCenter
        }
    }
}
