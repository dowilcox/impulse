// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Popup {
    id: goToLineRoot
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    width: 320
    height: 100
    padding: 0

    background: Rectangle {
        color: theme.bg_surface
        border.color: theme.border
        border.width: 1
        radius: 8
    }

    Overlay.modal: Rectangle {
        color: Qt.rgba(0, 0, 0, 0.3)
    }

    onOpened: {
        lineInput.text = ""
        lineInput.forceActiveFocus()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8

        Text {
            text: "Go to Line:"
            font.pixelSize: 13
            color: theme.fg
        }

        TextField {
            id: lineInput
            Layout.fillWidth: true
            placeholderText: "Line number"
            color: theme.fg
            font.pixelSize: 14
            leftPadding: 8
            rightPadding: 8
            inputMethodHints: Qt.ImhDigitsOnly
            validator: IntValidator { bottom: 1 }

            background: Rectangle {
                color: theme.bg
                border.color: lineInput.activeFocus ? theme.accent : theme.border
                border.width: 1
                radius: 6
            }

            Keys.onReturnPressed: goToLineRoot.accept()
            Keys.onEnterPressed: goToLineRoot.accept()
        }
    }

    function accept() {
        var line = parseInt(lineInput.text)
        if (isNaN(line) || line < 1) return

        // Navigate the active editor to the specified line
        var view = contentArea.contentItems[windowModel.active_tab_index]
        if (view && view.goToLine) {
            view.goToLine(line)
        }
        close()
    }
}
