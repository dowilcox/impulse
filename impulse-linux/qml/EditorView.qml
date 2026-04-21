// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtWebEngine
import dev.impulse.app

Item {
    id: editorViewRoot

    property int tabId: -1
    property string filePath: ""
    property bool isModified: false
    property bool previewVisible: false
    property bool isPreviewable: false
    // Cache latest content from the editor for saves
    property string cachedContent: ""
    // Track whether this editor's Monaco is ready
    property bool monacoReady: false

    // ── Public API ────────────────────────────────────────────────────────────
    function syncModifiedState() {
        if (tabId >= 0) {
            windowModel.set_tab_modified_by_id(tabId, isModified)
        }
    }

    function openFile(path) {
        filePath = path
        isModified = false
        syncModifiedState()
        isPreviewable = editorBridge.is_previewable_file(path)
        // open_file reads the file and returns an OpenFile command JSON
        var cmdJson = editorBridge.open_file(path)
        if (cmdJson.length > 0 && cmdJson.indexOf("error") < 0) {
            // Cache the initial content
            try {
                var cmd = JSON.parse(cmdJson)
                cachedContent = cmd.content || ""
            } catch (e) {}
            // If Monaco is already loaded, send the command
            if (monacoReady) {
                sendToMonaco(cmdJson)
            }
            // Otherwise, the Ready handler will re-open via filePath
            var lang = editorBridge.language_from_path(path)
            windowModel.language = lang
        }
    }

    function saveFile() {
        if (filePath.length > 0 && cachedContent.length > 0) {
            editorBridge.save_file(filePath, cachedContent)
        }
    }

    function goToLine(line) {
        var cmdJson = editorBridge.make_command_json("GoToPosition", JSON.stringify({ line: line - 1, column: 0 }))
        if (cmdJson.length > 0 && monacoReady) {
            sendToMonaco(cmdJson)
        }
    }

    function togglePreview() {
        previewVisible = !previewVisible
        if (previewVisible) {
            updatePreview()
        }
    }

    // ── Send a command object to this editor's Monaco ─────────────────────────
    function sendToMonaco(cmdJson) {
        monacoWebView.runJavaScript(
            "window.handleCommand(" + cmdJson + ")",
            function(result) {}
        )
    }

    // ── Apply theme to Monaco ─────────────────────────────────────────────────
    function applyTheme() {
        var monacoTheme = theme.monaco_theme_json
        if (monacoTheme.length > 0 && monacoReady) {
            var cmdJson = editorBridge.make_command_json("SetTheme", monacoTheme)
            if (cmdJson.length > 0) {
                sendToMonaco(cmdJson)
            }
        }
    }

    // ── Apply editor settings ─────────────────────────────────────────────────
    function applySettings() {
        if (!monacoReady) return
        var opts = {
            font_family: settings.font_family,
            font_size: settings.font_size,
            tab_size: settings.tab_width,
            insert_spaces: settings.use_spaces,
            word_wrap: settings.word_wrap ? "on" : "off",
            minimap_enabled: settings.minimap_enabled,
            line_numbers: settings.show_line_numbers ? "on" : "off",
            render_whitespace: settings.render_whitespace,
            sticky_scroll: settings.sticky_scroll,
            bracket_pair_colorization: settings.bracket_pair_colorization,
            indent_guides: settings.indent_guides,
            font_ligatures: settings.font_ligatures,
            folding: settings.folding,
            scroll_beyond_last_line: settings.scroll_beyond_last_line,
            smooth_scrolling: settings.smooth_scrolling,
            cursor_style: settings.editor_cursor_style,
            cursor_blinking: settings.editor_cursor_blinking
        }
        var cmdJson = editorBridge.make_command_json("UpdateSettings", JSON.stringify(opts))
        if (cmdJson.length > 0) {
            sendToMonaco(cmdJson)
        }
    }

    // ── Update markdown/SVG preview ───────────────────────────────────────────
    function updatePreview() {
        if (!previewVisible || filePath.length === 0 || cachedContent.length === 0) return
        refreshPreviewFromContent(cachedContent)
    }

    // ── Handle events directly from THIS editor's Monaco (no shared bridge) ──
    function handleJsEvent(json) {
        var evt
        try {
            evt = JSON.parse(json)
        } catch (e) {
            return
        }

        var eventType = evt.type || ""

        switch (eventType) {
            case "Ready":
                monacoReady = true
                applyTheme()
                applySettings()
                syncModifiedState()
                if (filePath.length > 0) {
                    var cmdJson = editorBridge.open_file(filePath)
                    if (cmdJson.length > 0 && cmdJson.indexOf("error") < 0) {
                        sendToMonaco(cmdJson)
                    }
                }
                break

            case "FileOpened":
                var lang = editorBridge.language_from_path(filePath)
                windowModel.language = lang
                break

            case "ContentChanged":
                isModified = true
                syncModifiedState()
                cachedContent = evt.content || ""
                if (previewVisible) {
                    refreshPreviewFromContent(cachedContent)
                }
                break

            case "CursorMoved":
                windowModel.cursor_line = evt.line || 0
                windowModel.cursor_column = evt.column || 0
                break

            case "SaveRequested":
                saveFile()
                break

            case "FocusChanged":
                break
        }
    }

    // ── Only listen for file_saved from bridge (stateless, safe to share) ─────
    Connections {
        target: editorBridge
        function onFile_saved(path) {
            if (path === editorViewRoot.filePath) {
                editorViewRoot.isModified = false
                editorViewRoot.syncModifiedState()
            }
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

        // ── Monaco editor ─────────────────────────────────────────────────
        WebEngineView {
            id: monacoWebView
            SplitView.fillWidth: true
            SplitView.minimumWidth: 200

            Component.onCompleted: {
                editorBridge.ensure_monaco_extracted()
                var html = editorBridge.get_editor_html()
                var baseUrl = editorBridge.monaco_base_url
                if (html.length > 0) {
                    loadHtml(html, baseUrl)
                }
            }

            // Handle events directly from THIS Monaco instance — no shared signal broadcast
            onJavaScriptConsoleMessage: function(level, message, lineNumber, sourceId) {
                if (message.indexOf("IMPULSE_EVENT:") === 0) {
                    var json = message.substring(14)
                    editorViewRoot.handleJsEvent(json)
                }
            }

            settings.javascriptEnabled: true
            settings.localContentCanAccessRemoteUrls: false
            settings.localContentCanAccessFileUrls: true
            settings.errorPageEnabled: false

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

            settings.javascriptEnabled: true
            settings.localContentCanAccessFileUrls: true
        }
    }
}
