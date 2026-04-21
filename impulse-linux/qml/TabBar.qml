// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

// Custom tab bar for Impulse. This file shadows QtQuick.Controls.TabBar
// within the dev.impulse.app module.

ToolBar {
    id: tabBarRoot
    height: windowModel.tab_count <= 1 ? 0 : 36
    visible: windowModel.tab_count > 1
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

    Behavior on height { NumberAnimation { duration: 120 } }

    property var tabs: {
        try {
            return JSON.parse(windowModel.tab_display_infos_json)
        } catch (e) {
            return []
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 6

        // ── Tab list ──────────────────────────────────────────────────────
        Flickable {
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: tabRow.width
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            Row {
                id: tabRow
                spacing: 2
                height: parent.height

                Repeater {
                    model: tabBarRoot.tabs.length

                    ToolButton {
                        id: tabDelegate
                        height: tabBarRoot.height
                        padding: 0

                        readonly property var tabInfo: tabBarRoot.tabs[index] || {}
                        readonly property bool isActive: index === windowModel.active_tab_index
                        readonly property string tabTitle: tabInfo.title || "Tab"
                        readonly property string tabType: tabInfo.tabType || "terminal"
                        readonly property bool isModified: !!tabInfo.isModified

                        checked: isActive
                        flat: !isActive

                        background: Rectangle {
                            radius: 10
                            color: {
                                if (tabDelegate.checked)
                                    return theme.bg_surface
                                if (tabDelegate.hovered)
                                    return theme.bg_highlight
                                return "transparent"
                            }
                            border.width: 1
                            border.color: tabDelegate.checked ? theme.border : "transparent"
                        }

                        contentItem: RowLayout {
                            spacing: 6

                            // Tab type icon
                            Label {
                                text: {
                                    switch (tabType) {
                                        case "terminal": return ">"
                                        case "editor":   return isModified ? "\u25CF" : "\u25CB"
                                        case "image":    return "\uD83D\uDDBC"
                                        default:         return "\u25CB"
                                    }
                                }
                                font.pixelSize: tabType === "terminal" ? 13 : 10
                                font.bold: tabType === "terminal"
                                color: isActive ? theme.fg : theme.fg_muted
                            }

                            // Tab title
                            Label {
                                text: tabTitle
                                font.pixelSize: 12
                                elide: Text.ElideRight
                                Layout.maximumWidth: 160
                                color: isActive ? theme.fg : theme.fg_muted
                            }

                            // Close button
                            ChromeToolButton {
                                Layout.preferredWidth: 22
                                Layout.preferredHeight: 22
                                visible: isActive || tabDelegate.hovered
                                icon.name: "window-close"
                                icon.width: 12
                                icon.height: 12
                                leftPadding: 5
                                rightPadding: 5
                                topPadding: 5
                                bottomPadding: 5
                                onClicked: windowModel.close_tab(index)

                                ToolTip.visible: hovered
                                ToolTip.text: "Close Tab"
                                ToolTip.delay: 600
                            }
                        }

                        // Drag support
                        Drag.active: tabDragHandler.active
                        Drag.source: tabDelegate
                        Drag.hotSpot.x: width / 2
                        Drag.hotSpot.y: height / 2

                        property int dragIndex: index

                        DragHandler {
                            id: tabDragHandler
                            target: null
                        }

                        DropArea {
                            anchors.fill: parent
                            onEntered: function(drag) {
                                var src = drag.source
                                if (src && src.dragIndex !== undefined && src.dragIndex !== index) {
                                    windowModel.move_tab(src.dragIndex, index)
                                }
                            }
                        }

                        onClicked: windowModel.select_tab(index)

                        MouseArea {
                            anchors.fill: parent
                            acceptedButtons: Qt.MiddleButton | Qt.RightButton
                            z: -1

                            onClicked: function(mouse) {
                                if (mouse.button === Qt.MiddleButton) {
                                    windowModel.close_tab(index)
                                } else if (mouse.button === Qt.RightButton) {
                                    tabContextMenu.tabIndex = index
                                    tabContextMenu.popup()
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── New tab button ────────────────────────────────────────────────
        ChromeToolButton {
            icon.name: "list-add"
            onClicked: windowModel.create_tab("terminal")
            ToolTip.visible: hovered
            ToolTip.text: "New Terminal (Ctrl+T)"
            ToolTip.delay: 600
        }
    }

    // ── Tab context menu ──────────────────────────────────────────────────────
    ChromeMenu {
        id: tabContextMenu
        property int tabIndex: -1

        ChromeMenuItem {
            text: "Close"
            icon.name: "window-close"
            onTriggered: windowModel.close_tab(tabContextMenu.tabIndex)
        }
        ChromeMenuItem {
            text: "Close Others"
            enabled: windowModel.tab_count > 1
            onTriggered: {
                var idx = tabContextMenu.tabIndex
                for (var i = windowModel.tab_count - 1; i >= 0; i--) {
                    if (i !== idx) windowModel.close_tab(i)
                }
            }
        }
        ChromeMenuItem {
            text: "Close All"
            onTriggered: {
                for (var i = windowModel.tab_count - 1; i >= 0; i--) {
                    windowModel.close_tab(i)
                }
            }
        }
    }
}
