// SPDX-License-Identifier: GPL-3.0-only
// TODO: Migrate to Material Icon Theme (JSON-driven lookup from assets/icons/material/).
//       See impulse-macos FileIcons.swift for reference implementation with folder-specific
//       icons and 1158+ extension mappings.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import dev.impulse.app

Rectangle {
    id: nodeRoot

    // Incoming data
    property var nodeData: ({})
    property int depth: 0

    readonly property string nodeName: nodeData.name || ""
    readonly property string nodePath: nodeData.path || ""
    readonly property bool isDir: !!nodeData.isDir
    readonly property bool isExpanded: !!nodeData.isExpanded
    readonly property string gitStatus: nodeData.gitStatus || ""
    readonly property int childCount: Math.max(0, nodeData.childCount || 0)
    readonly property bool isActive: nodePath.length > 0 && nodePath === fileTreeModel.active_file_path

    // Icons directory resolved from project root
    readonly property string iconsPath: "file://" + windowModel.project_root + "assets/icons/"

    height: 30
    color: {
        if (isActive) return theme.bg_highlight
        if (mouseArea.containsMouse) return theme.bg_dark
        return "transparent"
    }
    radius: 9
    border.width: isActive || mouseArea.containsMouse ? 1 : 0
    border.color: isActive ? theme.accent : theme.border

    Rectangle {
        width: 3
        radius: 2
        color: theme.accent
        visible: isActive
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.leftMargin: 4
        anchors.topMargin: 6
        anchors.bottomMargin: 6
    }

    function lookupIcon(name, isDirectory, expanded) {
        if (isDirectory) return lookupDirectoryIcon(name, expanded)
        var ext = name.split(".").pop().toLowerCase()
        switch (ext) {
            case "rs": return "rust"
            case "py": case "pyi": case "pyw": return "python"
            case "js": case "mjs": case "cjs": return "javascript"
            case "ts": case "mts": case "cts": return "typescript"
            case "go": return "go"
            case "c": case "h": return "c"
            case "cpp": case "cc": case "cxx": case "hpp": case "hxx": return "cpp"
            case "java": return "java"
            case "kt": case "kts": return "kotlin"
            case "swift": return "swift"
            case "rb": case "erb": return "ruby"
            case "php": return "php"
            case "cs": return "csharp"
            case "zig": return "zig"
            case "hs": case "lhs": return "haskell"
            case "lua": return "lua"
            case "dart": return "dart"
            case "ex": case "exs": case "heex": return "elixir"
            case "scala": case "sc": return "scala"
            case "clj": case "cljs": case "cljc": return "clojure"
            case "erl": case "hrl": return "erlang"
            case "nim": case "nims": return "nim"
            case "jl": return "julia"
            case "r": case "rmd": return "r"
            case "tex": case "sty": case "cls": case "bib": return "tex"
            case "html": case "htm": return "html"
            case "css": return "css"
            case "scss": case "sass": case "less": return "sass"
            case "vue": return "vue"
            case "svelte": return "svelte"
            case "jsx": case "tsx": return "react"
            case "json": case "jsonc": case "json5": return "json"
            case "yaml": case "yml": return "yaml"
            case "toml": return "toml"
            case "xml": case "xsl": case "xslt": return "xml"
            case "md": case "mdx": case "markdown": return "markdown"
            case "ini": case "cfg": case "conf": case "ron": return "settings"
            case "sh": case "bash": case "zsh": case "fish": case "ps1": return "console"
            case "qml": return "settings"
            case "lock": return "lock"
            case "sql": case "sqlite": case "db": return "database"
            case "png": case "jpg": case "jpeg": case "gif": case "svg": case "ico": case "webp": return "image"
            case "mp3": case "wav": case "flac": case "ogg": return "audio"
            case "mp4": case "mkv": case "avi": case "webm": case "mov": return "video"
            case "pdf": return "pdf"
            case "zip": case "tar": case "gz": case "bz2": case "xz": case "7z": case "zst": return "archive"
            case "exe": case "dll": case "so": case "dylib": case "a": case "o": case "wasm": return "binary"
            default: return lookupByFilename(name)
        }
    }

    function lookupDirectoryIcon(name, expanded) {
        var lower = name.toLowerCase()
        switch (lower) {
            case ".git": case ".github": case ".gitlab": return "git"
            case ".vscode": case ".idea": case "config": case ".config": return "settings"
            case "assets": case "images": case "img": return "image"
            case "docs": case "doc": return "markdown"
            case "scripts": case "bin": return "console"
            case "docker": return "docker"
            case "db": case "data": case "database": return "database"
            case "dist": case "build": case "out": case "target": return "archive"
            default: return expanded ? "folder-open" : "folder"
        }
    }

    function lookupByFilename(name) {
        var lower = name.toLowerCase()
        switch (lower) {
            case "dockerfile": case "containerfile": return "docker"
            case "makefile": case "justfile": return "console"
            case ".gitignore": case ".gitmodules": case ".gitattributes": return "git"
            case "cargo.toml": case "cargo.lock": return "rust"
            case "package.json": case "package-lock.json": return "javascript"
            case "tsconfig.json": return "typescript"
            case "go.mod": case "go.sum": return "go"
            default: return "document"
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 8 + depth * 14
        anchors.rightMargin: 8
        spacing: 6

        // ── Expand chevron (directories only) ─────────────────────────────
        Label {
            Layout.preferredWidth: 16
            horizontalAlignment: Text.AlignHCenter
            text: isDir ? "\u25B6" : ""
            font.pixelSize: 8
            color: isActive ? theme.fg : theme.fg_muted
            opacity: isDir ? 0.75 : 0
            rotation: isExpanded ? 90 : 0
            Behavior on rotation { NumberAnimation { duration: 120 } }
        }

        // ── File/folder SVG icon ──────────────────────────────────────────
        Image {
            Layout.preferredWidth: 16
            Layout.preferredHeight: 16
            source: iconsPath + lookupIcon(nodeName, isDir, isExpanded) + ".svg"
            sourceSize: Qt.size(16, 16)
            fillMode: Image.PreserveAspectFit
            smooth: true
            opacity: isActive ? 1 : 0.95
        }

        // ── File name ─────────────────────────────────────────────────────
        Label {
            Layout.fillWidth: true
            text: nodeName
            elide: Text.ElideRight
            font.pixelSize: 13
            font.bold: isDir
            color: {
                switch (gitStatus) {
                    case "M": return theme.yellow
                    case "A": return theme.green
                    case "D": return theme.red
                    case "?": return theme.fg_muted
                    default:  return isActive ? theme.fg : theme.fg
                }
            }
        }

        Label {
            visible: isDir && childCount > 0
            text: childCount
            font.pixelSize: 10
            color: isActive ? theme.fg_muted : theme.fg_muted
            opacity: 0.8
        }

        // ── Git status badge ──────────────────────────────────────────────
        Rectangle {
            visible: gitStatus.length > 0
            radius: 8
            implicitWidth: gitBadgeLabel.implicitWidth + 8
            implicitHeight: gitBadgeLabel.implicitHeight + 4
            color: {
                switch (gitStatus) {
                    case "M": return "#33" + theme.yellow.substring(1)
                    case "A": return "#33" + theme.green.substring(1)
                    case "D": return "#33" + theme.red.substring(1)
                    default:  return theme.bg_dark
                }
            }
            border.width: 1
            border.color: {
                switch (gitStatus) {
                    case "M": return theme.yellow
                    case "A": return theme.green
                    case "D": return theme.red
                    default:  return theme.border
                }
            }

            Label {
                id: gitBadgeLabel
                anchors.centerIn: parent
                text: gitStatus
                font.pixelSize: 10
                font.bold: true
                color: {
                    switch (gitStatus) {
                        case "M": return theme.yellow
                        case "A": return theme.green
                        case "D": return theme.red
                        default:  return theme.fg_muted
                    }
                }
            }
        }
    }

    // ── Mouse interaction ─────────────────────────────────────────────────────
    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.RightButton

        onClicked: function(mouse) {
            if (mouse.button === Qt.RightButton) {
                contextMenu.popup()
                return
            }
            if (isDir) {
                fileTreeModel.toggle_expand(nodePath)
            } else {
                fileTreeModel.active_file_path = nodePath
                fileTreeModel.file_activated(nodePath)
            }
        }
    }

    // ── Context menu ──────────────────────────────────────────────────────────
    ChromeMenu {
        id: contextMenu

        ChromeMenuItem { text: "New File"; icon.name: "document-new"; onTriggered: newItemDialog.openForFile() }
        ChromeMenuItem { text: "New Folder"; icon.name: "folder-new"; onTriggered: newItemDialog.openForFolder() }
        ChromeMenuSeparator {}
        ChromeMenuItem { text: "Rename"; icon.name: "edit-rename"; onTriggered: renameDialog.openForNode() }
        ChromeMenuItem { text: "Delete"; icon.name: "edit-delete"; onTriggered: deleteConfirmDialog.open() }
        ChromeMenuSeparator {}
        ChromeMenuItem { text: "Copy Path"; onTriggered: { var cb = Qt.application.clipboard; if (cb) cb.text = nodePath } }
        ChromeMenuItem { text: "Open in File Manager"; icon.name: "system-file-manager"; onTriggered: Qt.openUrlExternally("file://" + (isDir ? nodePath : nodePath.substring(0, nodePath.lastIndexOf("/")))) }
    }

    // ── Inline dialogs ────────────────────────────────────────────────────────
    ChromeDialog {
        id: newItemDialog
        property bool isFolder: false
        function openForFile() { isFolder = false; newItemInput.text = ""; open(); newItemInput.forceActiveFocus() }
        function openForFolder() { isFolder = true; newItemInput.text = ""; open(); newItemInput.forceActiveFocus() }
        title: isFolder ? "New Folder" : "New File"
        anchors.centerIn: Overlay.overlay
        standardButtons: Dialog.Ok | Dialog.Cancel
        ColumnLayout {
            spacing: 8
            Label { text: "Name:"; color: theme.fg_muted }
            ChromeTextField { id: newItemInput; Layout.preferredWidth: 260; onAccepted: newItemDialog.accept() }
        }
        onAccepted: {
            if (newItemInput.text.length === 0) return
            var pd = isDir ? nodePath : nodePath.substring(0, nodePath.lastIndexOf("/"))
            if (isFolder) fileTreeModel.create_folder(pd, newItemInput.text)
            else fileTreeModel.create_file(pd, newItemInput.text)
        }
    }

    ChromeDialog {
        id: renameDialog
        function openForNode() { renameInput.text = nodeName; open(); renameInput.forceActiveFocus(); renameInput.selectAll() }
        title: "Rename"
        anchors.centerIn: Overlay.overlay
        standardButtons: Dialog.Ok | Dialog.Cancel
        ColumnLayout {
            spacing: 8
            Label { text: "New name:"; color: theme.fg_muted }
            ChromeTextField { id: renameInput; Layout.preferredWidth: 260; onAccepted: renameDialog.accept() }
        }
        onAccepted: {
            if (renameInput.text.length > 0 && renameInput.text !== nodeName)
                fileTreeModel.rename_item(nodePath, renameInput.text)
        }
    }

    ChromeDialog {
        id: deleteConfirmDialog
        title: "Delete"
        anchors.centerIn: Overlay.overlay
        standardButtons: Dialog.Yes | Dialog.No
        Label {
            text: "Delete \"" + nodeName + "\"?"
            wrapMode: Text.WordWrap
            color: theme.fg
        }
        onAccepted: fileTreeModel.delete_item(nodePath)
    }
}
