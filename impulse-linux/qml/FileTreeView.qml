// SPDX-License-Identifier: GPL-3.0-only

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Rectangle {
    id: fileTreeRoot
    color: theme.bg_dark

    // Parsed flat tree data from JSON provided by the Rust backend.
    // Each node has: name, path, isDir, isExpanded, depth, gitStatus, childCount
    property var flatList: []

    function refreshTree() {
        try {
            flatList = JSON.parse(fileTreeModel.tree_json)
        } catch (e) {
            flatList = []
        }
    }

    Connections {
        target: fileTreeModel
        function onTree_changed() { refreshTree() }
    }

    Component.onCompleted: refreshTree()

    ListView {
        id: treeList
        anchors.fill: parent
        clip: true
        model: flatList.length
        boundsBehavior: Flickable.StopAtBounds

        ScrollBar.vertical: ScrollBar {
            policy: ScrollBar.AsNeeded
            background: Rectangle { color: "transparent" }
            contentItem: Rectangle {
                implicitWidth: 6
                radius: 3
                color: theme.fg_muted
                opacity: 0.4
            }
        }

        delegate: FileNodeDelegate {
            width: treeList.width
            nodeData: flatList[index] || ({})
            depth: flatList[index] ? flatList[index].depth : 0
        }
    }

    // Empty-state placeholder
    Text {
        anchors.centerIn: parent
        text: "No files"
        font.pixelSize: 13
        color: theme.fg_muted
        opacity: 0.5
        visible: flatList.length === 0
    }
}
