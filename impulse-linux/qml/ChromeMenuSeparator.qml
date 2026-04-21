// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls

MenuSeparator {
    id: control

    topPadding: 5
    bottomPadding: 5
    contentItem: Rectangle {
        implicitHeight: 1
        color: theme.border
        opacity: 0.9
    }
}
