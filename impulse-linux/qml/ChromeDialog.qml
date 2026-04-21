// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: control

    modal: true
    padding: 18
    leftPadding: 18
    rightPadding: 18
    topPadding: 18
    bottomPadding: 18
    closePolicy: Popup.CloseOnEscape
    palette.window: theme.bg_surface
    palette.windowText: theme.fg
    palette.base: theme.bg_dark
    palette.text: theme.fg
    palette.button: theme.bg_surface
    palette.buttonText: theme.fg
    palette.highlight: theme.accent
    palette.highlightedText: theme.bg

    Overlay.modal: Rectangle {
        color: "#7a0b1020"
    }

    background: Rectangle {
        radius: 14
        color: theme.bg_surface
        border.width: 1
        border.color: theme.border
    }

    header: Item {
        visible: control.title.length > 0
        implicitHeight: visible ? 54 : 0

        Rectangle {
            anchors.fill: parent
            color: theme.bg_dark
            radius: 14
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: theme.border
        }

        Label {
            anchors.left: parent.left
            anchors.leftMargin: 18
            anchors.verticalCenter: parent.verticalCenter
            text: control.title
            font.pixelSize: 15
            font.bold: true
            color: theme.fg
        }
    }
}
