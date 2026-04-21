// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

ChromeDialog {
    id: goToLineRoot
    title: "Go to Line"
    standardButtons: Dialog.Ok | Dialog.Cancel
    width: 320
    anchors.centerIn: Overlay.overlay

    onOpened: {
        lineInput.text = ""
        lineInput.forceActiveFocus()
    }

    onAccepted: {
        var line = parseInt(lineInput.text)
        if (isNaN(line) || line < 1) return

        var view = contentArea.activeView()
        if (view && view.goToLine) {
            view.goToLine(line)
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 8

        Label {
            text: "Line number:"
            color: theme.fg_muted
        }

        ChromeTextField {
            id: lineInput
            Layout.fillWidth: true
            placeholderText: "Line number"
            font.pixelSize: 14
            inputMethodHints: Qt.ImhDigitsOnly
            validator: IntValidator { bottom: 1 }

            Keys.onReturnPressed: goToLineRoot.accept()
            Keys.onEnterPressed: goToLineRoot.accept()
        }
    }
}
