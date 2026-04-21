// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

MenuItem {
    id: control

    implicitHeight: 34
    leftPadding: 10
    rightPadding: 10
    topPadding: 8
    bottomPadding: 8
    spacing: 8
    font.pixelSize: 12
    icon.width: 16
    icon.height: 16

    contentItem: RowLayout {
        spacing: control.spacing

        Item {
            Layout.preferredWidth: (control.icon.name || control.icon.source.toString().length > 0) ? 16 : 0
            Layout.preferredHeight: 16

            IconImage {
                anchors.centerIn: parent
                visible: parent.width > 0
                source: control.icon.name ? "" : control.icon.source
                name: control.icon.name
                color: control.enabled ? theme.fg_muted : theme.fg_muted
                width: 16
                height: 16
            }
        }

        Label {
            Layout.fillWidth: true
            text: control.text
            font: control.font
            color: control.enabled ? theme.fg : theme.fg_muted
            elide: Text.ElideRight
        }
    }

    arrow: Label {
        visible: control.subMenu
        text: "\u203A"
        color: theme.fg_muted
        font.pixelSize: 14
        verticalAlignment: Text.AlignVCenter
    }

    background: Rectangle {
        radius: 8
        color: {
            if (control.highlighted)
                return theme.bg_highlight
            if (control.hovered)
                return theme.bg_dark
            return "transparent"
        }
        border.width: control.highlighted ? 1 : 0
        border.color: theme.accent
        opacity: control.enabled ? 1 : 0.55
    }
}
