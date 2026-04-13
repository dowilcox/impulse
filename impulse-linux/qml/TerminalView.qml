// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import dev.impulse.app

FocusScope {
    id: termViewRoot
    clip: true

    property int tabId: -1
    property var snapshot: ({})
    readonly property string currentDirectory: terminalBridge.current_directory
    readonly property string terminalFontFamily: settings.terminal_font_family.length > 0
        ? settings.terminal_font_family
        : (jetbrainsFont.status === FontLoader.Ready ? jetbrainsFont.name : "Monospace")
    readonly property int terminalFontSize: settings.terminal_font_size > 0 ? settings.terminal_font_size : 14
    readonly property real padding: 8
    readonly property real cellWidth: Math.max(1, Math.ceil(Math.max(fontMetrics.averageCharacterWidth, textMetrics.advanceWidth)))
    readonly property real cellHeight: Math.max(1, Math.ceil(fontMetrics.height))
    readonly property real baselineOffset: Math.ceil(fontMetrics.ascent)

    function parseSnapshot() {
        try {
            snapshot = JSON.parse(terminalBridge.grid_json)
        } catch (e) {
            snapshot = ({})
        }
        terminalCanvas.requestPaint()
    }

    function resizeBackend() {
        if (width <= padding * 2 || height <= padding * 2 || cellWidth <= 0 || cellHeight <= 0)
            return
        var cols = Math.max(2, Math.floor((width - padding * 2) / cellWidth))
        var rows = Math.max(1, Math.floor((height - padding * 2) / cellHeight))
        terminalBridge.resize_terminal(cols, rows, Math.max(1, Math.round(cellWidth)), Math.max(1, Math.round(cellHeight)))
    }

    function startTerminal() {
        terminalBridge.start(
            currentDirectory.length > 0 ? currentDirectory : windowModel.current_directory,
            theme.theme_id,
            settings.terminal_scrollback,
            settings.terminal_cursor_shape,
            settings.terminal_cursor_blink
        )
        if (tabId >= 0 && terminalBridge.title.length > 0) {
            windowModel.set_tab_title_by_id(tabId, terminalBridge.title)
        }
        Qt.callLater(resizeBackend)
    }

    function bracketedPasteEnabled() {
        return (terminalBridge.mode_bits & (1 << 7)) !== 0
    }

    function normalizePastedText(text) {
        if (!text)
            return ""
        while (text.endsWith("\n") || text.endsWith("\r"))
            text = text.slice(0, -1)
        text = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
        return text
    }

    function copySelectionToClipboard() {
        var text = terminalBridge.selected_text()
        if (!text || text.length === 0)
            return
        clipboardProxy.text = text
        clipboardProxy.selectAll()
        clipboardProxy.copy()
        clipboardProxy.text = ""
        termViewRoot.forceActiveFocus()
    }

    function pasteFromClipboard() {
        clipboardProxy.text = ""
        clipboardProxy.forceActiveFocus()
        clipboardProxy.paste()
        var text = normalizePastedText(clipboardProxy.text)
        clipboardProxy.text = ""
        termViewRoot.forceActiveFocus()
        if (!text || text.length === 0) {
            text = terminalBridge.clipboard_image_path()
        }
        if (!text || text.length === 0)
            return
        if (bracketedPasteEnabled())
            terminalBridge.send_text("\u001b[200~")
        terminalBridge.send_text(text)
        if (bracketedPasteEnabled())
            terminalBridge.send_text("\u001b[201~")
    }

    function effectiveCursorShape() {
        switch (settings.terminal_cursor_shape) {
        case "beam":
        case "line":
            return 1
        case "underline":
            return 2
        default:
            return snapshot.cursorShape !== undefined ? snapshot.cursorShape : 0
        }
    }

    function sendSpecialKey(key, modifiers) {
        var appCursor = (terminalBridge.mode_bits & (1 << 1)) !== 0
        switch (key) {
        case Qt.Key_Return:
        case Qt.Key_Enter:
            if ((modifiers & Qt.ShiftModifier) !== 0 && (modifiers & Qt.ControlModifier) === 0) {
                terminalBridge.send_text("\u001b[13;2u")
            } else {
                terminalBridge.send_text("\r")
            }
            return true
        case Qt.Key_Backspace:
            terminalBridge.send_text("\u007f")
            return true
        case Qt.Key_Tab:
            terminalBridge.send_text("\t")
            return true
        case Qt.Key_Backtab:
            terminalBridge.send_text("\u001b[Z")
            return true
        case Qt.Key_Escape:
            terminalBridge.send_text("\u001b")
            return true
        case Qt.Key_Up:
            terminalBridge.send_text(appCursor ? "\u001bOA" : "\u001b[A")
            return true
        case Qt.Key_Down:
            terminalBridge.send_text(appCursor ? "\u001bOB" : "\u001b[B")
            return true
        case Qt.Key_Right:
            terminalBridge.send_text(appCursor ? "\u001bOC" : "\u001b[C")
            return true
        case Qt.Key_Left:
            terminalBridge.send_text(appCursor ? "\u001bOD" : "\u001b[D")
            return true
        case Qt.Key_Home:
            terminalBridge.send_text("\u001b[H")
            return true
        case Qt.Key_End:
            terminalBridge.send_text("\u001b[F")
            return true
        case Qt.Key_PageUp:
            terminalBridge.send_text("\u001b[5~")
            return true
        case Qt.Key_PageDown:
            terminalBridge.send_text("\u001b[6~")
            return true
        case Qt.Key_Delete:
            terminalBridge.send_text("\u001b[3~")
            return true
        case Qt.Key_F1:
            terminalBridge.send_text("\u001bOP")
            return true
        case Qt.Key_F2:
            terminalBridge.send_text("\u001bOQ")
            return true
        case Qt.Key_F3:
            terminalBridge.send_text("\u001bOR")
            return true
        case Qt.Key_F4:
            terminalBridge.send_text("\u001bOS")
            return true
        case Qt.Key_F5:
            terminalBridge.send_text("\u001b[15~")
            return true
        case Qt.Key_F6:
            terminalBridge.send_text("\u001b[17~")
            return true
        case Qt.Key_F7:
            terminalBridge.send_text("\u001b[18~")
            return true
        case Qt.Key_F8:
            terminalBridge.send_text("\u001b[19~")
            return true
        case Qt.Key_F9:
            terminalBridge.send_text("\u001b[20~")
            return true
        case Qt.Key_F10:
            terminalBridge.send_text("\u001b[21~")
            return true
        case Qt.Key_F11:
            terminalBridge.send_text("\u001b[23~")
            return true
        case Qt.Key_F12:
            terminalBridge.send_text("\u001b[24~")
            return true
        default:
            return false
        }
    }

    function handleKeyPress(event) {
        if ((event.modifiers & Qt.MetaModifier) !== 0)
            return false

        if ((event.modifiers & Qt.ControlModifier) !== 0 && (event.modifiers & Qt.ShiftModifier) !== 0) {
            if (event.key === Qt.Key_C) {
                copySelectionToClipboard()
                return true
            }
            if (event.key === Qt.Key_V) {
                pasteFromClipboard()
                return true
            }
        }

        if (sendSpecialKey(event.key, event.modifiers))
            return true

        if ((event.modifiers & Qt.ControlModifier) !== 0 && event.text.length === 1) {
            var code = event.text.toLowerCase().charCodeAt(0)
            if (code >= 97 && code <= 122) {
                terminalBridge.send_text(String.fromCharCode(code - 96))
                return true
            }
            if (event.text === "[") {
                terminalBridge.send_text("\u001b")
                return true
            }
            if (event.text === "\\") {
                terminalBridge.send_text(String.fromCharCode(0x1c))
                return true
            }
            if (event.text === "]") {
                terminalBridge.send_text(String.fromCharCode(0x1d))
                return true
            }
        }

        if ((event.modifiers & Qt.AltModifier) !== 0 && event.text.length === 1) {
            terminalBridge.send_text("\u001b" + event.text)
            return true
        }

        if (event.text.length > 0) {
            terminalBridge.send_text(event.text)
            return true
        }

        return false
    }

    function selectionForRow(row) {
        var ranges = []
        if (!snapshot.selectionRanges)
            return ranges
        for (var i = 0; i < snapshot.selectionRanges.length; ++i) {
            var range = snapshot.selectionRanges[i]
            if (range.row === row)
                ranges.push(range)
        }
        return ranges
    }

    function pointToCell(x, y) {
        return {
            col: Math.max(0, Math.floor((x - padding) / cellWidth)),
            row: Math.max(0, Math.floor((y - padding) / cellHeight))
        }
    }

    FontLoader {
        id: jetbrainsFont
        source: "file://" + windowModel.project_root + "impulse-editor/vendor/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf"
    }

    FontMetrics {
        id: fontMetrics
        font.family: termViewRoot.terminalFontFamily
        font.pixelSize: termViewRoot.terminalFontSize
    }

    TextMetrics {
        id: textMetrics
        font.family: termViewRoot.terminalFontFamily
        font.pixelSize: termViewRoot.terminalFontSize
        text: "W"
    }

    TerminalBridge {
        id: terminalBridge
    }

    TextEdit {
        id: clipboardProxy
        visible: false
        width: 0
        height: 0
        opacity: 0
    }

    Timer {
        interval: 16
        running: true
        repeat: true
        onTriggered: terminalBridge.poll()
    }

    Canvas {
        id: terminalCanvas
        anchors.fill: parent
        focus: true
        renderTarget: Canvas.FramebufferObject
        renderStrategy: Canvas.Cooperative

        onPaint: {
            var ctx = getContext("2d")
            ctx.globalAlpha = 1
            ctx.setTransform(1, 0, 0, 1, 0, 0)
            ctx.clearRect(0, 0, width, height)
            ctx.fillStyle = theme.bg
            ctx.fillRect(0, 0, width, height)

            if (!snapshot.rowsData || !snapshot.rowsData.length)
                return

            for (var row = 0; row < snapshot.rowsData.length; ++row) {
                var rowData = snapshot.rowsData[row]
                var x = padding
                var y = padding + row * cellHeight

                for (var s = 0; s < rowData.segments.length; ++s) {
                    var seg = rowData.segments[s]
                    var segWidth = seg.columns * cellWidth
                    ctx.fillStyle = seg.bg
                    ctx.fillRect(x, y, segWidth, cellHeight)
                    x += segWidth
                }

                var rowSelections = selectionForRow(row)
                for (var r = 0; r < rowSelections.length; ++r) {
                    var range = rowSelections[r]
                    ctx.fillStyle = theme.selection
                    ctx.fillRect(
                        padding + range.startCol * cellWidth,
                        y,
                        Math.max(0, (range.endCol - range.startCol + 1) * cellWidth),
                        cellHeight
                    )
                }

                x = padding
                var baseline = y + baselineOffset
                for (var t = 0; t < rowData.segments.length; ++t) {
                    var textSeg = rowData.segments[t]
                    ctx.globalAlpha = textSeg.dim ? 0.7 : 1
                    ctx.fillStyle = textSeg.fg
                    ctx.font =
                        (textSeg.italic ? "italic " : "") +
                        (textSeg.bold ? "bold " : "") +
                        terminalFontSize + "px \"" + terminalFontFamily + "\""
                    ctx.textBaseline = "alphabetic"
                    ctx.fillText(textSeg.text, x, baseline)
                    if (textSeg.underline) {
                        ctx.fillRect(x, y + cellHeight - 2, textSeg.columns * cellWidth, 1)
                    }
                    if (textSeg.strikethrough) {
                        ctx.fillRect(x, y + cellHeight / 2, textSeg.columns * cellWidth, 1)
                    }
                    x += textSeg.columns * cellWidth
                }
                ctx.globalAlpha = 1
            }

            if (termViewRoot.activeFocus && snapshot.cursorVisible) {
                var cursorX = padding + snapshot.cursorCol * cellWidth
                var cursorY = padding + snapshot.cursorRow * cellHeight
                ctx.fillStyle = theme.cursor_color
                switch (effectiveCursorShape()) {
                case 1:
                    ctx.fillRect(cursorX, cursorY, Math.max(2, Math.floor(cellWidth * 0.15)), cellHeight)
                    break
                case 2:
                    ctx.fillRect(cursorX, cursorY + cellHeight - 2, cellWidth, 2)
                    break
                default:
                    ctx.globalAlpha = 0.35
                    ctx.fillRect(cursorX, cursorY, cellWidth, cellHeight)
                    ctx.globalAlpha = 1
                    break
                }
            }
        }
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        hoverEnabled: true
        cursorShape: Qt.IBeamCursor

        onPressed: function(mouse) {
            termViewRoot.forceActiveFocus()
            var point = pointToCell(mouse.x, mouse.y)
            terminalBridge.clear_selection()
            terminalBridge.start_selection(point.col, point.row)
        }

        onPositionChanged: function(mouse) {
            if ((mouse.buttons & Qt.LeftButton) === 0)
                return
            var point = pointToCell(mouse.x, mouse.y)
            terminalBridge.update_selection(point.col, point.row)
        }

        onReleased: function(mouse) {
            if (settings.terminal_copy_on_select) {
                copySelectionToClipboard()
            }
        }

        onWheel: function(wheel) {
            var delta = wheel.angleDelta.y > 0 ? 3 : -3
            terminalBridge.scroll(delta)
            wheel.accepted = true
        }
    }

    Rectangle {
        anchors.fill: parent
        visible: terminalBridge.error_message.length > 0
        color: "#aa000000"

        Label {
            anchors.centerIn: parent
            width: Math.min(parent.width - 32, 520)
            text: terminalBridge.error_message
            wrapMode: Text.WordWrap
            color: "white"
            horizontalAlignment: Text.AlignHCenter
        }
    }

    Label {
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.rightMargin: 12
        anchors.bottomMargin: 8
        visible: !terminalBridge.is_running && terminalBridge.error_message.length === 0
        text: "Process exited"
        font.pixelSize: 11
        opacity: 0.7
    }

    Keys.onPressed: function(event) {
        if (handleKeyPress(event)) {
            event.accepted = true
            terminalCanvas.requestPaint()
        }
    }

    onWidthChanged: resizeBackend()
    onHeightChanged: resizeBackend()
    onVisibleChanged: {
        if (visible)
            termViewRoot.forceActiveFocus()
    }
    onActiveFocusChanged: terminalBridge.set_focused(activeFocus)

    Component.onCompleted: {
        startTerminal()
        parseSnapshot()
    }

    Component.onDestruction: terminalBridge.shutdown()

    Connections {
        target: terminalBridge
        function onGrid_json_changed() {
            parseSnapshot()
        }
        function onTitle_changed() {
            if (tabId >= 0 && terminalBridge.title.length > 0) {
                windowModel.set_tab_title_by_id(tabId, terminalBridge.title)
            }
        }
        function onError_message_changed() {
            if (terminalBridge.error_message.length > 0) {
                console.warn("Terminal error:", terminalBridge.error_message)
            }
        }
    }

    Connections {
        target: theme
        function onTheme_id_changed() {
            terminalBridge.apply_theme(theme.theme_id)
            terminalCanvas.requestPaint()
        }
    }

    Connections {
        target: settings
        function onSettings_changed() {
            resizeBackend()
            terminalCanvas.requestPaint()
        }
    }
}
