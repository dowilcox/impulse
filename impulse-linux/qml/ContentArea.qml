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
        if (info && info.tabType === "editor") {
            var view = contentItems[info.id]
            return view ? !!view.isModified : false
        }
        return false
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

    // Container for dynamically created tab content views, keyed by tab id.
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
        var imageExts = ["png", "jpg", "jpeg", "gif", "webp", "bmp"]
        if (imageExts.indexOf(ext) >= 0) {
            windowModel.create_image_tab(path)
            pendingFilePath = ""
            pendingLine = 0
        } else {
            windowModel.create_editor_tab(path)
            pendingFilePath = path
            pendingLine = line || 0
        }
    }

    property string pendingFilePath: ""
    property int pendingLine: 0

    // Get the content view for the currently active tab
    function activeView() {
        var info = currentTabInfo()
        return info ? (contentItems[info.id] || null) : null
    }

    function saveActiveEditor() {
        var info = currentTabInfo()
        if (info && info.tabType === "editor" && info.filePath) {
            var view = contentItems[info.id]
            if (view && view.saveFile) {
                view.saveFile()
            }
        }
    }

    function goToLineInEditor(line) {
        var info = currentTabInfo()
        if (info) {
            var view = contentItems[info.id]
            if (view && view.goToLine) {
                view.goToLine(line)
            }
        }
    }

    function resetEditorStatus() {
        windowModel.cursor_line = 0
        windowModel.cursor_column = 0
        windowModel.language = ""
        windowModel.encoding = ""
        windowModel.indent_info = ""
        windowModel.blame_info = ""
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
        var tabId = info.id
        var tabType = info.tabType || "terminal"

        // Hide all content views
        for (var key in contentItems) {
            if (contentItems[key]) {
                contentItems[key].visible = false
            }
        }

        // Create if not existing
        if (!contentItems[tabId]) {
            var component = contentComponents[tabType]

            if (tabType === "image") {
                var imgItem = imageComponent.createObject(contentContainer, {
                    "source": "file://" + (info.filePath || "")
                })
                if (imgItem) {
                    contentItems[tabId] = imgItem
                }
            } else if (component && component.status === Component.Ready) {
                var initialProps = {}
                if (tabType === "terminal" || tabType === "editor") {
                    initialProps.tabId = tabId
                }
                var item = component.createObject(contentContainer, initialProps)
                if (item) {
                    contentItems[tabId] = item
                    if (pendingFilePath.length > 0 && tabType === "editor" && item.openFile) {
                        item.openFile(pendingFilePath)
                        if (pendingLine > 0) {
                            Qt.callLater(function() { item.goToLine(pendingLine) })
                        }
                        pendingFilePath = ""
                        pendingLine = 0
                    }
                }
            } else if (component && component.status === Component.Error) {
                console.warn("Failed to load component for tab type:", tabType, component.errorString())
            }
        }

        // Show active content
        if (contentItems[tabId]) {
            contentItems[tabId].visible = true
            contentItems[tabId].anchors.fill = contentContainer
        }

        if (tabType !== "editor") {
            resetEditorStatus()
        }
    }

    // Clean up views whose tabs no longer exist
    onTabsChanged: {
        // Build set of current tab IDs
        var liveIds = {}
        for (var i = 0; i < tabs.length; i++) {
            liveIds[tabs[i].id] = true
        }

        // Destroy views for closed tabs
        for (var key in contentItems) {
            if (!liveIds[key] && contentItems[key]) {
                contentItems[key].visible = false
                contentItems[key].destroy()
                delete contentItems[key]
            }
        }

        ensureContentView()
    }

    // ── Placeholder when no tabs ──────────────────────────────────────────────
    Pane {
        anchors.fill: parent
        visible: windowModel.tab_count === 0

        Column {
            anchors.centerIn: parent
            spacing: 16

            Label {
                anchors.horizontalCenter: parent.horizontalCenter
                text: "Impulse"
                font.pixelSize: 28
                font.bold: true
                opacity: 0.4
            }

            Label {
                anchors.horizontalCenter: parent.horizontalCenter
                text: "Ctrl+T  New Terminal\nCtrl+P  Quick Open\nCtrl+,  Settings"
                font.pixelSize: 13
                opacity: 0.3
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

    // ── Image viewer component ────────────────────────────────────────────────
    Component {
        id: imageComponent

        Pane {
            property alias source: imgElement.source
            padding: 16

            Image {
                id: imgElement
                anchors.centerIn: parent
                width: Math.min(sourceSize.width, parent.width - 32)
                height: Math.min(sourceSize.height, parent.height - 32)
                fillMode: Image.PreserveAspectFit
                smooth: true
                mipmap: true
            }

            Label {
                anchors.bottom: parent.bottom
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.bottomMargin: 12
                text: imgElement.sourceSize.width + " x " + imgElement.sourceSize.height
                font.pixelSize: 12
                visible: imgElement.status === Image.Ready
            }
        }
    }
}
