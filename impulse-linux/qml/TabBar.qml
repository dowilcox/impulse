// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

// Custom tab bar for Impulse. This file shadows QtQuick.Controls.TabBar
// within the dev.impulse.app module. Files that need the Controls TabBar
// should import QtQuick.Controls with a namespace alias (e.g., QQC2).

Rectangle {
    id: tabBarRoot
    height: windowModel.tab_count <= 1 ? 0 : 36
    visible: windowModel.tab_count > 1
    color: theme.bg_dark
    clip: true

    Behavior on height { NumberAnimation { duration: 120 } }

    property var tabs: {
        try {
            return JSON.parse(windowModel.tab_display_infos_json)
        } catch (e) {
            return []
        }
    }

    // Bottom border
    Rectangle {
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        height: 1
        color: theme.border
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 4
        anchors.rightMargin: 4
        spacing: 2

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

                    Rectangle {
                        id: tabDelegate
                        width: tabContent.implicitWidth + 24
                        height: tabBarRoot.height - 1  // leave room for bottom border
                        radius: 6

                        readonly property var tabInfo: tabBarRoot.tabs[index] || {}
                        readonly property bool isActive: index === windowModel.active_tab_index
                        readonly property string tabTitle: tabInfo.title || "Tab"
                        readonly property string tabType: tabInfo.tabType || "terminal"
                        readonly property bool isModified: !!tabInfo.isModified

                        color: {
                            if (isActive) return theme.bg
                            if (tabMouse.containsMouse) return theme.bg_highlight
                            return "transparent"
                        }

                        // Active tab accent underline
                        Rectangle {
                            anchors.bottom: parent.bottom
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.leftMargin: 8
                            anchors.rightMargin: 8
                            height: 2
                            radius: 1
                            color: theme.accent
                            visible: isActive
                        }

                        // Drag support
                        Drag.active: tabDragHandler.active
                        Drag.source: tabDelegate
                        Drag.hotSpot.x: width / 2
                        Drag.hotSpot.y: height / 2

                        property int dragIndex: index

                        DragHandler {
                            id: tabDragHandler
                            target: null  // we handle position manually if needed
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

                        RowLayout {
                            id: tabContent
                            anchors.centerIn: parent
                            spacing: 4

                            // Tab type icon
                            Text {
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
                                color: isActive ? theme.accent : theme.fg_muted
                            }

                            // Tab title
                            Text {
                                text: tabTitle
                                font.pixelSize: 12
                                color: isActive ? theme.fg : theme.fg_muted
                                elide: Text.ElideRight
                                Layout.maximumWidth: 160
                            }

                            // Close button (visible on hover or active)
                            Rectangle {
                                Layout.preferredWidth: 18
                                Layout.preferredHeight: 18
                                radius: 9
                                visible: isActive || tabMouse.containsMouse
                                color: closeHover.containsMouse ? theme.bg_highlight : "transparent"

                                Text {
                                    anchors.centerIn: parent
                                    text: "\u00D7"
                                    font.pixelSize: 14
                                    color: closeHover.containsMouse ? theme.fg : theme.fg_muted
                                }

                                MouseArea {
                                    id: closeHover
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    onClicked: windowModel.close_tab(index)
                                }
                            }
                        }

                        MouseArea {
                            id: tabMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            acceptedButtons: Qt.LeftButton | Qt.MiddleButton | Qt.RightButton
                            z: -1  // below close button

                            onClicked: function(mouse) {
                                if (mouse.button === Qt.MiddleButton) {
                                    windowModel.close_tab(index)
                                } else if (mouse.button === Qt.RightButton) {
                                    tabContextMenu.tabIndex = index
                                    tabContextMenu.popup()
                                } else {
                                    windowModel.select_tab(index)
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── New tab button ────────────────────────────────────────────────
        ToolButton {
            id: addTabBtn
            Layout.preferredWidth: 28
            Layout.preferredHeight: 28
            text: "+"
            font.pixelSize: 16
            palette.buttonText: theme.fg_muted
            background: Rectangle {
                color: addTabBtn.hovered ? theme.bg_highlight : "transparent"
                radius: 6
            }
            onClicked: windowModel.create_tab("terminal")

            ToolTip.visible: hovered
            ToolTip.text: "New Terminal (Ctrl+T)"
            ToolTip.delay: 600
        }
    }

    // ── Tab context menu ──────────────────────────────────────────────────────
    Menu {
        id: tabContextMenu
        property int tabIndex: -1

        background: Rectangle {
            color: theme.bg_surface
            border.color: theme.border
            border.width: 1
            radius: 6
        }

        MenuItem {
            text: "Close"
            onTriggered: windowModel.close_tab(tabContextMenu.tabIndex)
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuItem {
            text: "Close Others"
            enabled: windowModel.tab_count > 1
            onTriggered: {
                var idx = tabContextMenu.tabIndex
                for (var i = windowModel.tab_count - 1; i >= 0; i--) {
                    if (i !== idx) windowModel.close_tab(i)
                }
            }
            contentItem: Text { text: parent.text; color: parent.enabled ? theme.fg : theme.fg_muted; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
        MenuItem {
            text: "Close All"
            onTriggered: {
                for (var i = windowModel.tab_count - 1; i >= 0; i--) {
                    windowModel.close_tab(i)
                }
            }
            contentItem: Text { text: parent.text; color: theme.fg; font.pixelSize: 13 }
            background: Rectangle { color: parent.highlighted ? theme.bg_highlight : "transparent" }
        }
    }
}
