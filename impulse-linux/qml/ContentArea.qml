// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Item {
    id: contentAreaRoot

    // Expose currently active editor info for window title
    readonly property string activeEditorPath: {
        var info = currentTabInfo()
        return (info && info.tabType === "editor") ? (info.filePath || "") : ""
    }
    readonly property bool activeEditorModified: {
        var info = currentTabInfo()
        return (info && info.tabType === "editor") ? !!info.isModified : false
    }

    property var tabs: {
        try {
            return JSON.parse(windowModel.tab_display_infos_json)
        } catch (e) {
            return []
        }
    }

    function currentTabInfo() {
        var idx = windowModel.active_tab_index
        if (idx >= 0 && idx < tabs.length) {
            return tabs[idx]
        }
        return null
    }

    // Container for dynamically created tab content views.
    // We use a map keyed by tab index string for O(1) lookup.
    property var contentItems: ({})
    property var contentComponents: ({
        "terminal": Qt.createComponent("TerminalView.qml"),
        "editor":   Qt.createComponent("EditorView.qml")
    })

    function openFile(path, line) {
        // Check if file is already open in a tab
        for (var i = 0; i < tabs.length; i++) {
            if (tabs[i].filePath === path) {
                windowModel.select_tab(i)
                if (line !== undefined && line > 0) {
                    goToLineInEditor(line)
                }
                return
            }
        }

        // Check if this is an image
        var ext = path.split(".").pop().toLowerCase()
        var imageExts = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"]
        if (imageExts.indexOf(ext) >= 0 && !editorBridge.is_previewable_file(path)) {
            windowModel.create_tab("image")
        } else {
            windowModel.create_tab("editor")
        }
        // The backend will fire tabSwitched, causing us to create the view.
        // We store the pending file/line info.
        pendingFilePath = path
        pendingLine = line || 0
    }

    property string pendingFilePath: ""
    property int pendingLine: 0

    function saveActiveEditor() {
        var info = currentTabInfo()
        if (info && info.tabType === "editor" && info.filePath) {
            // Signal the editor view to save
            var view = contentItems[windowModel.active_tab_index]
            if (view && view.saveFile) {
                view.saveFile()
            }
        }
    }

    function goToLineInEditor(line) {
        var view = contentItems[windowModel.active_tab_index]
        if (view && view.goToLine) {
            view.goToLine(line)
        }
    }

    // Respond to tab switches
    Connections {
        target: windowModel
        function onTab_switched() {
            contentAreaRoot.ensureContentView()
        }
    }

    // Ensure the active tab has a content view
    function ensureContentView() {
        var idx = windowModel.active_tab_index
        if (idx < 0 || idx >= tabs.length) return

        var info = tabs[idx]
        if (!info) return

        // Hide all content views
        for (var key in contentItems) {
            if (contentItems[key]) {
                contentItems[key].visible = false
            }
        }

        // Create if not existing
        if (!contentItems[idx]) {
            var tabType = info.tabType || "terminal"
            var component = contentComponents[tabType]

            if (tabType === "image") {
                // Create image display inline
                var imgItem = imageComponent.createObject(contentContainer, {
                    "source": "file://" + (info.filePath || "")
                })
                if (imgItem) {
                    contentItems[idx] = imgItem
                }
            } else if (component && component.status === Component.Ready) {
                var item = component.createObject(contentContainer, {})
                if (item) {
                    contentItems[idx] = item
                    // If there's a pending file, open it
                    if (pendingFilePath.length > 0 && tabType === "editor" && item.openFile) {
                        item.openFile(pendingFilePath)
                        if (pendingLine > 0) {
                            Qt.callLater(function() { item.goToLine(pendingLine) })
                        }
                        pendingFilePath = ""
                        pendingLine = 0
                    }
                }
            }
        }

        // Show active content
        if (contentItems[idx]) {
            contentItems[idx].visible = true
            contentItems[idx].anchors.fill = contentContainer
        }
    }

    // Clean up views when tabs are closed
    onTabsChanged: {
        // Remove content items for indices that no longer exist
        var toRemove = []
        for (var key in contentItems) {
            var k = parseInt(key)
            if (k >= tabs.length) {
                toRemove.push(key)
            }
        }
        for (var i = 0; i < toRemove.length; i++) {
            if (contentItems[toRemove[i]]) {
                contentItems[toRemove[i]].destroy()
                delete contentItems[toRemove[i]]
            }
        }
        ensureContentView()
    }

    // ── Placeholder when no tabs ──────────────────────────────────────────────
    Rectangle {
        anchors.fill: parent
        color: theme.bg
        visible: windowModel.tab_count === 0

        Column {
            anchors.centerIn: parent
            spacing: 16

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: "Impulse"
                font.pixelSize: 28
                font.bold: true
                color: theme.fg_muted
                opacity: 0.5
            }

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: "Ctrl+T  New Terminal\nCtrl+P  Quick Open\nCtrl+,  Settings"
                font.pixelSize: 13
                color: theme.fg_muted
                opacity: 0.4
                horizontalAlignment: Text.AlignHCenter
                lineHeight: 1.6
            }
        }
    }

    // ── Content container ─────────────────────────────────────────────────────
    Item {
        id: contentContainer
        anchors.fill: parent
        visible: windowModel.tab_count > 0
    }

    // ── Image viewer component (inline, no separate file) ─────────────────────
    Component {
        id: imageComponent

        Rectangle {
            color: theme.bg
            property alias source: imgElement.source

            Image {
                id: imgElement
                anchors.centerIn: parent
                width: Math.min(sourceSize.width, parent.width - 32)
                height: Math.min(sourceSize.height, parent.height - 32)
                fillMode: Image.PreserveAspectFit
                smooth: true
                mipmap: true
            }

            Text {
                anchors.bottom: parent.bottom
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.bottomMargin: 12
                text: imgElement.sourceSize.width + " x " + imgElement.sourceSize.height
                font.pixelSize: 12
                color: theme.fg_muted
                visible: imgElement.status === Image.Ready
            }
        }
    }
}
