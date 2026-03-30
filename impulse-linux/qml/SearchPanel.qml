// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Pane {
    id: searchPanelRoot
    padding: 8

    property var results: {
        try {
            return JSON.parse(searchModel.results_json)
        } catch (e) {
            return []
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 8

        // ── Search input ──────────────────────────────────────────────────
        TextField {
            id: searchInput
            Layout.fillWidth: true
            placeholderText: searchModel.search_mode === "files" ? "Search files..." : "Search in files..."
            font.pixelSize: 13

            onTextChanged: searchDebounce.restart()
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

            ToolButton {
                text: "Files"
                checked: searchModel.search_mode === "files"
                onClicked: {
                    searchModel.search_mode = "files"
                    if (searchInput.text.length > 0) performSearch()
                }
            }

            ToolButton {
                text: "Content"
                checked: searchModel.search_mode === "content"
                onClicked: {
                    searchModel.search_mode = "content"
                    if (searchInput.text.length > 0) performSearch()
                }
            }

            Item { Layout.fillWidth: true }

            ToolButton {
                text: "Aa"
                checked: searchModel.case_sensitive
                font.bold: true
                onClicked: {
                    searchModel.case_sensitive = !searchModel.case_sensitive
                    if (searchInput.text.length > 0) performSearch()
                }
                ToolTip.visible: hovered
                ToolTip.text: "Case Sensitive"
                ToolTip.delay: 600
            }
        }

        // ── Result count / status ─────────────────────────────────────────
        Label {
            Layout.fillWidth: true
            text: {
                if (searchModel.is_searching) return "Searching..."
                if (searchInput.text.length === 0) return ""
                return searchModel.result_count + " result" + (searchModel.result_count !== 1 ? "s" : "")
            }
            font.pixelSize: 11
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
            }

            delegate: ItemDelegate {
                width: resultsList.width

                readonly property var resultData: results[index] || {}

                contentItem: ColumnLayout {
                    spacing: 2

                    Label {
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
                        color: palette.highlight
                        elide: Text.ElideMiddle
                    }

                    Label {
                        Layout.fillWidth: true
                        text: resultData.preview || ""
                        font.pixelSize: 11
                        elide: Text.ElideRight
                        visible: (resultData.preview || "").length > 0
                        maximumLineCount: 2
                        wrapMode: Text.NoWrap
                    }
                }

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
