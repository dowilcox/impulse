// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls

Menu {
    id: control

    margins: 0
    topPadding: 6
    bottomPadding: 6
    leftPadding: 6
    rightPadding: 6
    overlap: 1

    background: Rectangle {
        radius: 12
        color: theme.bg_surface
        border.width: 1
        border.color: theme.border
    }
}
