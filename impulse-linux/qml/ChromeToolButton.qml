// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls

ToolButton {
    id: control

    implicitHeight: 32
    implicitWidth: Math.max(32, implicitContentWidth + leftPadding + rightPadding)
    leftPadding: text.length > 0 ? 12 : 8
    rightPadding: text.length > 0 ? 12 : 8
    topPadding: 7
    bottomPadding: 7
    spacing: 6
    icon.width: 16
    icon.height: 16
    font.pixelSize: 12

    contentItem: IconLabel {
        spacing: control.spacing
        mirrored: control.mirrored
        display: control.display
        alignment: Qt.AlignCenter
        icon: control.icon
        text: control.text
        font: control.font
        color: control.enabled ? theme.fg : theme.fg_muted
    }

    background: Rectangle {
        radius: 9
        border.width: 1
        border.color: {
            if (!control.enabled)
                return "transparent"
            if (control.checked || control.down)
                return theme.accent
            if (control.hovered)
                return theme.border
            return "transparent"
        }
        color: {
            if (control.checked || control.down)
                return theme.bg_highlight
            if (control.hovered)
                return theme.bg_surface
            return "transparent"
        }
        opacity: control.enabled ? 1 : 0.45

        Behavior on color { ColorAnimation { duration: 100 } }
        Behavior on border.color { ColorAnimation { duration: 100 } }
    }
}
