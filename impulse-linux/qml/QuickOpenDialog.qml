// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

ChromeDialog {
    id: quickOpenRoot
    title: "Quick Open"
    standardButtons: Dialog.NoButton
    width: Math.min(560, parent ? parent.width * 0.6 : 560)
    height: Math.min(420, parent ? parent.height * 0.6 : 420)
    anchors.centerIn: Overlay.overlay

    signal fileSelected(string path)

    property var fileResults: []
    property int selectedIndex: 0

    SearchModel {
        id: quickOpenSearchModel
        root_path: windowModel.current_directory
    }

    onOpened: {
        searchInput.text = ""
        fileResults = []
        selectedIndex = 0
        quickOpenSearchModel.clear()
        searchInput.forceActiveFocus()
    }

    onClosed: {
        quickOpenDebounce.stop()
        quickOpenSearchModel.clear()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 10

        // ── Search input ──────────────────────────────────────────────────
        ChromeTextField {
            id: searchInput
            Layout.fillWidth: true
            placeholderText: "Type to search files..."
            font.pixelSize: 14

            onTextChanged: quickOpenDebounce.restart()

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
            Keys.onReturnPressed: acceptSelection()
            Keys.onEnterPressed: acceptSelection()
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
                    quickOpenSearchModel.clear()
                    return
                }
                quickOpenSearchModel.search_files(query)
            }
        }

        Connections {
            target: quickOpenSearchModel
            function onSearch_completed() {
                try {
                    quickOpenRoot.fileResults = JSON.parse(quickOpenSearchModel.results_json)
                } catch (e) {
                    quickOpenRoot.fileResults = []
                }
                quickOpenRoot.selectedIndex = 0
            }
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
            }

            delegate: ItemDelegate {
                id: resultDelegate
                width: resultsList.width
                highlighted: index === selectedIndex
                hoverEnabled: true
                leftPadding: 12
                rightPadding: 12
                topPadding: 10
                bottomPadding: 10

                background: Rectangle {
                    radius: 10
                    color: {
                        if (resultDelegate.highlighted)
                            return theme.bg_highlight
                        if (resultDelegate.hovered)
                            return theme.bg_dark
                        return "transparent"
                    }
                    border.width: resultDelegate.highlighted || resultDelegate.hovered ? 1 : 0
                    border.color: resultDelegate.highlighted ? theme.accent : theme.border
                }

                readonly property var fileData: fileResults[index] || {}
                readonly property string fullPath: fileData.path || ""
                readonly property string fileName: fullPath.split("/").pop()
                readonly property string relativePath: {
                    var root = quickOpenSearchModel.root_path || windowModel.current_directory
                    if (root.length > 0 && fullPath.indexOf(root) === 0) {
                        return fullPath.substring(root.length + 1)
                    }
                    return fullPath
                }

                contentItem: RowLayout {
                    spacing: 8

                    Label {
                        text: fileName
                        font.pixelSize: 13
                        font.bold: true
                        elide: Text.ElideRight
                        color: theme.fg
                    }

                    Label {
                        text: relativePath
                        font.pixelSize: 11
                        color: theme.fg_muted
                        elide: Text.ElideMiddle
                        Layout.fillWidth: true
                    }
                }

                onClicked: {
                    selectedIndex = index
                    acceptSelection()
                }
            }
        }

        // ── Footer ────────────────────────────────────────────────────────
        Label {
            Layout.fillWidth: true
            text: fileResults.length + " file" + (fileResults.length !== 1 ? "s" : "") + "   \u2191\u2193 Navigate  \u23CE Open  Esc Close"
            font.pixelSize: 11
            color: theme.fg_muted
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
