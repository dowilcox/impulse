// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtWebEngine
import dev.impulse.app

Item {
    id: editorViewRoot

    property string filePath: ""
    property bool isModified: false
    property bool previewVisible: false
    property bool isPreviewable: false

    // ── Public API ────────────────────────────────────────────────────────────
    function openFile(path) {
        filePath = path
        isPreviewable = editorBridge.is_previewable_file(path)
        editorBridge.open_file(path)
    }

    function saveFile() {
        if (filePath.length > 0) {
            sendCommand("getContent", "{}")
        }
    }

    function goToLine(line) {
        var params = JSON.stringify({ lineNumber: line })
        sendCommand("revealLine", params)
    }

    function togglePreview() {
        previewVisible = !previewVisible
        if (previewVisible) {
            updatePreview()
        }
    }

    // ── Send a command to Monaco via JavaScript ───────────────────────────────
    function sendCommand(commandType, paramsJson) {
        var cmdJson = editorBridge.make_command_json(commandType, paramsJson || "{}")
        if (cmdJson.length > 0 && monacoWebView.loading === false) {
            monacoWebView.runJavaScript(
                "window.handleCommand(" + cmdJson + ")",
                function(result) {}
            )
        }
    }

    // ── Apply theme to Monaco ─────────────────────────────────────────────────
    function applyTheme() {
        var monacoTheme = theme.monaco_theme_json
        if (monacoTheme.length > 0) {
            sendCommand("setTheme", monacoTheme)
        }
    }

    // ── Apply editor settings ─────────────────────────────────────────────────
    function applySettings() {
        var opts = {
            fontFamily: settings.font_family,
            fontSize: settings.font_size,
            tabSize: settings.tab_width,
            insertSpaces: settings.use_spaces,
            wordWrap: settings.word_wrap ? "on" : "off",
            minimap: { enabled: settings.minimap_enabled },
            lineNumbers: settings.show_line_numbers ? "on" : "off"
        }
        sendCommand("updateOptions", JSON.stringify(opts))
    }

    // ── Update markdown/SVG preview ───────────────────────────────────────────
    function updatePreview() {
        if (!previewVisible || filePath.length === 0) return

        var ext = filePath.split(".").pop().toLowerCase()
        if (ext === "md" || ext === "markdown") {
            sendCommand("getContent", "{}")
            // The content will come back via editorEvent, handled below
        } else if (ext === "svg") {
            sendCommand("getContent", "{}")
        }
    }

    // ── Connections to EditorBridge ────────────────────────────────────────────
    Connections {
        target: editorBridge
        function onEditor_event(eventType, payloadJson) {
            editorViewRoot.handleEditorEvent(eventType, payloadJson)
        }
        function onFile_saved(path) {
            if (path === editorViewRoot.filePath) {
                editorViewRoot.isModified = false
            }
        }
    }

    function handleEditorEvent(eventType, payloadJson) {
        var payload
        try {
            payload = JSON.parse(payloadJson)
        } catch (e) {
            payload = {}
        }

        switch (eventType) {
            case "ready":
                applyTheme()
                applySettings()
                if (filePath.length > 0) {
                    var lang = editorBridge.language_from_path(filePath)
                    sendCommand("openFile", JSON.stringify({
                        path: filePath,
                        content: payload.content || "",
                        language: lang
                    }))
                }
                break

            case "contentChanged":
                isModified = true
                editorBridge.content_changed(filePath, payload.content || "")
                if (previewVisible) {
                    refreshPreviewFromContent(payload.content || "")
                }
                break

            case "content":
                // Response to getContent command — used for save and preview
                if (payload.content !== undefined) {
                    // Save flow
                    if (filePath.length > 0) {
                        editorBridge.save_file(filePath, payload.content)
                    }
                    // Preview flow
                    if (previewVisible) {
                        refreshPreviewFromContent(payload.content)
                    }
                }
                break

            case "cursorChanged":
                windowModel.cursor_line = payload.lineNumber || 0
                windowModel.cursor_column = payload.column || 0
                break

            case "languageChanged":
                windowModel.language = payload.languageId || ""
                break

            case "scroll":
                // Could sync preview scroll position
                break
        }
    }

    function refreshPreviewFromContent(content) {
        if (!previewVisible || content.length === 0) return

        var ext = filePath.split(".").pop().toLowerCase()
        var html = ""
        if (ext === "md" || ext === "markdown") {
            html = editorBridge.render_markdown_preview(content, theme.get_markdown_theme_json())
        } else if (ext === "svg") {
            html = editorBridge.render_svg_preview(content, theme.bg)
        }
        if (html.length > 0) {
            previewWebView.loadHtml(html)
        }
    }

    // ── React to theme/settings changes ───────────────────────────────────────
    Connections {
        target: theme
        function onTheme_id_changed() {
            editorViewRoot.applyTheme()
        }
    }
    Connections {
        target: settings
        function onSettings_changed() {
            editorViewRoot.applySettings()
        }
    }

    // ── Layout ────────────────────────────────────────────────────────────────
    SplitView {
        anchors.fill: parent
        orientation: Qt.Horizontal

        handle: Rectangle {
            implicitWidth: previewVisible ? 1 : 0
            color: theme.border
        }

        // ── Monaco editor ─────────────────────────────────────────────────
        WebEngineView {
            id: monacoWebView
            SplitView.fillWidth: true
            SplitView.minimumWidth: 200

            backgroundColor: theme.bg

            // Load the editor HTML from the embedded assets
            Component.onCompleted: {
                editorBridge.ensure_monaco_extracted()
                var html = editorBridge.get_editor_html()
                var baseUrl = editorBridge.monaco_base_url
                if (html.length > 0) {
                    loadHtml(html, baseUrl)
                }
            }

            // Handle JS messages from Monaco
            onJavaScriptConsoleMessage: function(level, message, lineNumber, sourceId) {
                // Monaco sends events by calling console.log with a JSON prefix
                if (message.indexOf("IMPULSE_EVENT:") === 0) {
                    var json = message.substring(14)
                    try {
                        var evt = JSON.parse(json)
                        editorBridge.handle_event(json)
                    } catch (e) {
                        // Not a valid event
                    }
                }
            }

            // WebChannel setup for bidirectional messaging
            webChannel: WebChannel {
                id: editorChannel
            }

            settings.javascriptEnabled: true
            settings.localContentCanAccessRemoteUrls: false
            settings.localContentCanAccessFileUrls: true
            settings.errorPageEnabled: false

            // Prevent context menu from appearing (Monaco has its own)
            onContextMenuRequested: function(request) {
                request.accepted = true
            }
        }

        // ── Preview panel ─────────────────────────────────────────────────
        WebEngineView {
            id: previewWebView
            visible: previewVisible
            SplitView.preferredWidth: previewVisible ? parent.width * 0.4 : 0
            SplitView.minimumWidth: previewVisible ? 200 : 0
            backgroundColor: theme.bg

            settings.javascriptEnabled: true
            settings.localContentCanAccessFileUrls: true
        }
    }
}
