// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Popup {
    id: quickOpenRoot
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    width: Math.min(560, parent.width * 0.6)
    height: Math.min(420, parent.height * 0.6)
    padding: 0

    signal fileSelected(string path)

    property var fileResults: []
    property int selectedIndex: 0

    background: Rectangle {
        color: theme.bg_surface
        border.color: theme.border
        border.width: 1
        radius: 8

        // Shadow
        layer.enabled: true
        layer.effect: Item {}  // placeholder; real shadow requires ShaderEffect or GraphicalEffects
    }

    // Overlay dimming
    Overlay.modal: Rectangle {
        color: Qt.rgba(0, 0, 0, 0.4)
    }

    onOpened: {
        searchInput.text = ""
        fileResults = []
        selectedIndex = 0
        searchInput.forceActiveFocus()
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

            // Only round top corners
            Rectangle {
                anchors.bottom: parent.bottom
                anchors.left: parent.left
                anchors.right: parent.right
                height: parent.radius
                color: parent.color
            }

            TextField {
                id: searchInput
                anchors.fill: parent
                anchors.margins: 8
                placeholderText: "Type to search files..."
                color: theme.fg
                font.pixelSize: 14
                leftPadding: 12
                rightPadding: 12

                background: Rectangle {
                    color: theme.bg
                    border.color: searchInput.activeFocus ? theme.accent : theme.border
                    border.width: 1
                    radius: 6
                }

                onTextChanged: {
                    quickOpenDebounce.restart()
                }

                Keys.onDownPressed: {
                    if (selectedIndex < fileResults.length - 1) {
                        selectedIndex++
                        resultsList.positionViewAtIndex(selectedIndex, ListView.Contain)
                    }
                }
                Keys.onUpPressed: {
                    if (selectedIndex > 0) {
                        selectedIndex--
                        resultsList.positionViewAtIndex(selectedIndex, ListView.Contain)
                    }
                }
                Keys.onReturnPressed: {
                    acceptSelection()
                }
                Keys.onEnterPressed: {
                    acceptSelection()
                }
            }
        }

        Timer {
            id: quickOpenDebounce
            interval: 150
            repeat: false
            onTriggered: {
                var query = searchInput.text.trim()
                if (query.length === 0) {
                    fileResults = []
                    selectedIndex = 0
                    return
                }
                searchModel.search_files(query)
            }
        }

        // Listen for search results
        Connections {
            target: searchModel
            function onSearch_completed() {
                try {
                    quickOpenRoot.fileResults = JSON.parse(searchModel.results_json)
                } catch (e) {
                    quickOpenRoot.fileResults = []
                }
                quickOpenRoot.selectedIndex = 0
            }
        }

        // ── Separator ─────────────────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: theme.border
        }

        // ── Results list ──────────────────────────────────────────────────
        ListView {
            id: resultsList
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: fileResults.length
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
                id: fileResultDelegate
                width: resultsList.width
                height: 36
                color: {
                    if (index === selectedIndex) return theme.bg_highlight
                    if (fileResultMouse.containsMouse) return Qt.rgba(
                        parseInt(theme.bg_highlight.substring(1, 3), 16) / 255,
                        parseInt(theme.bg_highlight.substring(3, 5), 16) / 255,
                        parseInt(theme.bg_highlight.substring(5, 7), 16) / 255,
                        0.5
                    )
                    return "transparent"
                }

                readonly property var fileData: fileResults[index] || {}
                readonly property string fullPath: fileData.path || ""
                readonly property string fileName: {
                    var parts = fullPath.split("/")
                    return parts[parts.length - 1]
                }
                readonly property string relativePath: {
                    var root = searchModel.root_path || windowModel.current_directory
                    if (root.length > 0 && fullPath.indexOf(root) === 0) {
                        return fullPath.substring(root.length + 1)
                    }
                    return fullPath
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    spacing: 8

                    // File icon
                    Text {
                        text: {
                            var ext = fileName.split(".").pop().toLowerCase()
                            switch (ext) {
                                case "rs":   return "R"
                                case "js":   return "J"
                                case "ts":   return "T"
                                case "py":   return "P"
                                case "qml":  return "Q"
                                case "html": return "H"
                                case "css":  return "C"
                                case "json": return "{}"
                                case "md":   return "M"
                                case "sh":   return "$"
                                default:     return "\uD83D\uDCC4"
                            }
                        }
                        font.pixelSize: 14
                        color: {
                            var ext = fileName.split(".").pop().toLowerCase()
                            switch (ext) {
                                case "rs":   return theme.orange
                                case "js":   return theme.yellow
                                case "ts":   return theme.blue
                                case "py":   return theme.green
                                case "qml":  return theme.magenta
                                default:     return theme.fg_muted
                            }
                        }
                        Layout.preferredWidth: 20
                        horizontalAlignment: Text.AlignHCenter
                        Layout.alignment: Qt.AlignVCenter
                    }

                    // File name
                    Text {
                        text: fileName
                        font.pixelSize: 13
                        font.bold: true
                        color: theme.fg
                        elide: Text.ElideRight
                        Layout.alignment: Qt.AlignVCenter
                    }

                    // Relative path
                    Text {
                        text: relativePath
                        font.pixelSize: 11
                        color: theme.fg_muted
                        elide: Text.ElideMiddle
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                    }
                }

                MouseArea {
                    id: fileResultMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: {
                        selectedIndex = index
                        acceptSelection()
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

            // Only round bottom corners
            Rectangle {
                anchors.top: parent.top
                anchors.left: parent.left
                anchors.right: parent.right
                height: parent.radius
                color: parent.color
            }

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
                anchors.leftMargin: 12
                anchors.rightMargin: 12
                anchors.topMargin: 1

                Text {
                    text: fileResults.length + " file" + (fileResults.length !== 1 ? "s" : "")
                    font.pixelSize: 11
                    color: theme.fg_muted
                }

                Item { Layout.fillWidth: true }

                Text {
                    text: "\u2191\u2193 Navigate  \u23CE Open  Esc Close"
                    font.pixelSize: 11
                    color: theme.fg_muted
                }
            }
        }
    }

    function acceptSelection() {
        if (fileResults.length > 0 && selectedIndex >= 0 && selectedIndex < fileResults.length) {
            var path = fileResults[selectedIndex].path || ""
            if (path.length > 0) {
                fileSelected(path)
                close()
            }
        }
    }
}
