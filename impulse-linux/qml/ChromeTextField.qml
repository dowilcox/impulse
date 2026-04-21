// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls

TextField {
    id: control

    color: theme.fg
    font.pixelSize: 13
    placeholderTextColor: theme.fg_muted
    selectedTextColor: theme.bg
    selectionColor: theme.accent
    leftPadding: 12
    rightPadding: 12
    topPadding: 10
    bottomPadding: 10

    background: Rectangle {
        radius: 10
        color: theme.bg_dark
        border.width: 1
        border.color: control.activeFocus ? theme.accent : theme.border

        Behavior on border.color { ColorAnimation { duration: 100 } }
    }
}
