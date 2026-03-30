// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Item {
    id: searchPanelRoot

    property var results: {
        try {
            return JSON.parse(searchModel.results_json)
        } catch (e) {
            return []
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 8

        // ── Search input ──────────────────────────────────────────────────
        TextField {
            id: searchInput
            Layout.fillWidth: true
            placeholderText: searchModel.search_mode === "files" ? "Search files..." : "Search in files..."
            color: theme.fg
            font.pixelSize: 13
            leftPadding: 8
            rightPadding: 8

            background: Rectangle {
                color: theme.bg
                border.color: searchInput.activeFocus ? theme.accent : theme.border
                border.width: 1
                radius: 4
            }

            onTextChanged: {
                searchDebounce.restart()
            }
            onAccepted: {
                searchDebounce.stop()
                performSearch()
            }

            Keys.onEscapePressed: {
                searchInput.text = ""
                searchModel.clear()
            }
        }

        // Debounce timer
        Timer {
            id: searchDebounce
            interval: 300
            repeat: false
            onTriggered: performSearch()
        }

        function performSearch() {
            var query = searchInput.text.trim()
            if (query.length === 0) {
                searchModel.clear()
                return
            }
            searchModel.query = query
            if (searchModel.search_mode === "files") {
                searchModel.search_files(query)
            } else {
                searchModel.search_content(query)
            }
        }

        // ── Toggle row ────────────────────────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            // Search mode toggle
            Row {
                spacing: 2

                Rectangle {
                    width: filesModeBtn.implicitWidth + 12
                    height: 24
                    radius: 4
                    color: searchModel.search_mode === "files" ? theme.accent : theme.bg_highlight

                    Text {
                        id: filesModeBtn
                        anchors.centerIn: parent
                        text: "Files"
                        font.pixelSize: 11
                        color: searchModel.search_mode === "files" ? theme.bg : theme.fg_muted
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            searchModel.search_mode = "files"
                            if (searchInput.text.length > 0) performSearch()
                        }
                    }
                }

                Rectangle {
                    width: contentModeBtn.implicitWidth + 12
                    height: 24
                    radius: 4
                    color: searchModel.search_mode === "content" ? theme.accent : theme.bg_highlight

                    Text {
                        id: contentModeBtn
                        anchors.centerIn: parent
                        text: "Content"
                        font.pixelSize: 11
                        color: searchModel.search_mode === "content" ? theme.bg : theme.fg_muted
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            searchModel.search_mode = "content"
                            if (searchInput.text.length > 0) performSearch()
                        }
                    }
                }
            }

            Item { Layout.fillWidth: true }

            // Case-sensitive toggle
            Rectangle {
                width: 24
                height: 24
                radius: 4
                color: searchModel.case_sensitive ? theme.accent : theme.bg_highlight

                Text {
                    anchors.centerIn: parent
                    text: "Aa"
                    font.pixelSize: 11
                    font.bold: true
                    color: searchModel.case_sensitive ? theme.bg : theme.fg_muted
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: {
                        searchModel.case_sensitive = !searchModel.case_sensitive
                        if (searchInput.text.length > 0) performSearch()
                    }

                    ToolTip.visible: containsMouse
                    ToolTip.text: "Case Sensitive"
                    ToolTip.delay: 600
                    hoverEnabled: true
                }
            }
        }

        // ── Result count / status ─────────────────────────────────────────
        Text {
            Layout.fillWidth: true
            text: {
                if (searchModel.is_searching) return "Searching..."
                if (searchInput.text.length === 0) return ""
                return searchModel.result_count + " result" + (searchModel.result_count !== 1 ? "s" : "")
            }
            font.pixelSize: 11
            color: theme.fg_muted
        }

        // ── Results list ──────────────────────────────────────────────────
        ListView {
            id: resultsList
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: results.length
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
                id: resultDelegate
                width: resultsList.width
                height: resultContent.implicitHeight + 10
                color: resultMouse.containsMouse ? theme.bg_highlight : "transparent"
                radius: 4

                readonly property var resultData: results[index] || {}

                ColumnLayout {
                    id: resultContent
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: 6
                    anchors.rightMargin: 6
                    spacing: 2

                    // File path (relative to root)
                    Text {
                        Layout.fillWidth: true
                        text: {
                            var path = resultData.path || ""
                            var root = searchModel.root_path
                            if (root.length > 0 && path.indexOf(root) === 0) {
                                path = path.substring(root.length + 1)
                            }
                            if (resultData.line && resultData.line > 0) {
                                path += ":" + resultData.line
                            }
                            return path
                        }
                        font.pixelSize: 12
                        color: theme.accent
                        elide: Text.ElideMiddle
                    }

                    // Preview text (for content search)
                    Text {
                        Layout.fillWidth: true
                        text: resultData.preview || ""
                        font.pixelSize: 11
                        color: theme.fg_muted
                        elide: Text.ElideRight
                        visible: (resultData.preview || "").length > 0
                        maximumLineCount: 2
                        wrapMode: Text.NoWrap
                    }
                }

                MouseArea {
                    id: resultMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: {
                        searchModel.result_selected(
                            resultData.path || "",
                            resultData.line || 0
                        )
                    }
                }
            }
        }
    }
}
