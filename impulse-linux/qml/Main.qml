// SPDX-License-Identifier: GPL-3.0-only
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

ApplicationWindow {
    id: root
    title: windowModel.current_directory.length > 0
           ? windowModel.current_directory + " — Impulse"
           : "Impulse"
    width: 1280
    height: 800
    visible: true

    // ── Models ───────────────────────────────────────────────────────────────
    WindowModel { id: windowModel }
    ThemeBridge { id: theme }
    FileTreeModel { id: fileTreeModel }
    EditorBridge { id: editorBridge }
    LspBridge { id: lspBridge }
    SearchModel { id: searchModel }
    SettingsModel { id: settings }

    // ── Startup ──────────────────────────────────────────────────────────────
    Timer {
        interval: 100
        running: true
        repeat: false
        onTriggered: {
            theme.set_theme("nord")
            windowModel.set_directory("/home/dowilcox")
            fileTreeModel.load_root("/home/dowilcox")
        }
    }

    // ── Keyboard shortcuts ───────────────────────────────────────────────────
    Shortcut { sequence: "Ctrl+B"; onActivated: windowModel.toggle_sidebar() }
    Shortcut { sequence: "Ctrl+T"; onActivated: windowModel.create_tab("terminal") }

    // ── Main layout ──────────────────────────────────────────────────────────
    SplitView {
        anchors.fill: parent
        orientation: Qt.Horizontal

        // ── Sidebar ──────────────────────────────────────────────────────
        Pane {
            id: sidebar
            SplitView.preferredWidth: 260
            SplitView.minimumWidth: 180
            SplitView.maximumWidth: root.width * 0.5
            visible: windowModel.sidebar_visible
            padding: 0

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                // Sidebar header
                ToolBar {
                    Layout.fillWidth: true
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 4

                        Label {
                            text: "Files"
                            font.bold: true
                        }
                        Item { Layout.fillWidth: true }
                        ToolButton {
                            icon.name: "view-refresh"
                            display: AbstractButton.IconOnly
                            onClicked: fileTreeModel.refresh()
                            ToolTip.visible: hovered
                            ToolTip.text: "Refresh"
                            ToolTip.delay: 600
                        }
                    }
                }

                // File tree
                ScrollView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true

                    ListView {
                        id: fileTree
                        model: treeNodes.length
                        boundsBehavior: Flickable.StopAtBounds

                        property var treeNodes: {
                            try { return JSON.parse(fileTreeModel.tree_json) }
                            catch(e) { return [] }
                        }

                        delegate: ItemDelegate {
                            id: treeDelegate
                            width: fileTree.width
                            height: 28
                            padding: 0
                            leftPadding: 8 + (node.depth || 0) * 18

                            property var node: fileTree.treeNodes[index] || ({})

                            contentItem: RowLayout {
                                spacing: 6

                                // Expand chevron
                                Label {
                                    text: treeDelegate.node.isDir
                                          ? (treeDelegate.node.isExpanded ? "▾" : "▸") : ""
                                    font.pixelSize: 10
                                    opacity: 0.6
                                    Layout.preferredWidth: 12
                                    Layout.alignment: Qt.AlignVCenter
                                }

                                // Icon
                                Label {
                                    text: treeDelegate.node.isDir ? "📁" : "📄"
                                    font.pixelSize: 13
                                    Layout.alignment: Qt.AlignVCenter
                                }

                                // File name
                                Label {
                                    text: treeDelegate.node.name || ""
                                    font.pixelSize: 13
                                    elide: Text.ElideRight
                                    Layout.fillWidth: true
                                    Layout.alignment: Qt.AlignVCenter
                                    color: {
                                        var gs = treeDelegate.node.gitStatus
                                        if (gs === "M") return theme.yellow
                                        if (gs === "A") return theme.green
                                        if (gs === "D") return theme.red
                                        return palette.text
                                    }
                                }

                                // Git badge
                                Label {
                                    visible: !!treeDelegate.node.gitStatus
                                    text: treeDelegate.node.gitStatus || ""
                                    font.pixelSize: 9
                                    font.bold: true
                                    opacity: 0.7
                                    Layout.alignment: Qt.AlignVCenter
                                }
                            }

                            onClicked: {
                                if (treeDelegate.node.isDir) {
                                    fileTreeModel.toggle_expand(treeDelegate.node.path)
                                }
                            }
                        }

                        // Empty state
                        Label {
                            anchors.centerIn: parent
                            text: "No files"
                            opacity: 0.4
                            visible: fileTree.treeNodes.length === 0
                        }
                    }
                }
            }
        }

        // ── Content + Status bar ─────────────────────────────────────────
        Page {
            SplitView.fillWidth: true
            padding: 0

            // Content area
            Pane {
                anchors.fill: parent
                anchors.bottomMargin: statusBar.height

                Column {
                    anchors.centerIn: parent
                    spacing: 16

                    Label {
                        anchors.horizontalCenter: parent.horizontalCenter
                        text: "Impulse"
                        font.pixelSize: 28
                        font.bold: true
                        opacity: 0.4
                    }
                    Label {
                        anchors.horizontalCenter: parent.horizontalCenter
                        text: "Ctrl+T  New Terminal\nCtrl+B  Toggle Sidebar\nCtrl+P  Quick Open"
                        font.pixelSize: 13
                        opacity: 0.3
                        horizontalAlignment: Text.AlignHCenter
                        lineHeight: 1.6
                    }
                }
            }

            // Status bar
            ToolBar {
                id: statusBar
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 28
                position: ToolBar.Footer

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 10
                    anchors.rightMargin: 10
                    spacing: 0

                    Label {
                        text: windowModel.shell_name
                        font.pixelSize: 12
                    }
                    ToolSeparator { Layout.preferredHeight: 14 }
                    Label {
                        text: "⎇ " + windowModel.git_branch
                        font.pixelSize: 12
                        visible: windowModel.git_branch.length > 0
                    }
                    ToolSeparator { visible: windowModel.git_branch.length > 0; Layout.preferredHeight: 14 }
                    Label {
                        text: {
                            var dir = windowModel.current_directory
                            if (dir.indexOf("/home/dowilcox") === 0) dir = "~" + dir.substring(14)
                            return dir
                        }
                        font.pixelSize: 12
                        elide: Text.ElideMiddle
                        Layout.maximumWidth: 300
                    }
                    Item { Layout.fillWidth: true }
                    Label {
                        text: "Ln " + windowModel.cursor_line + ", Col " + windowModel.cursor_column
                        font.pixelSize: 12
                        visible: windowModel.cursor_line > 0
                    }
                    ToolSeparator { visible: windowModel.cursor_line > 0; Layout.preferredHeight: 14 }
                    Label {
                        text: windowModel.encoding
                        font.pixelSize: 12
                    }
                }
            }
        }
    }
}
