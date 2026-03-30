// SPDX-License-Identifier: GPL-3.0-only

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
    readonly property bool isActive: nodePath.length > 0 && nodePath === fileTreeModel.active_file_path

    // Icons directory resolved from project root
    readonly property string iconsPath: "file://" + windowModel.project_root + "assets/icons/"

    height: 28
    color: {
        if (isActive) return palette.highlight
        if (mouseArea.containsMouse) return palette.mid
        return "transparent"
    }

    function lookupIcon(name, isDirectory, expanded) {
        if (isDirectory) return expanded ? "folder-open" : "folder"
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
        anchors.leftMargin: 4 + depth * 16
        anchors.rightMargin: 4
        spacing: 4

        // ── Expand chevron (directories only) ─────────────────────────────
        Label {
            Layout.preferredWidth: 16
            horizontalAlignment: Text.AlignHCenter
            text: isDir ? "\u25B6" : ""
            font.pixelSize: 8
            opacity: 0.6
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
        }

        // ── File name ─────────────────────────────────────────────────────
        Label {
            Layout.fillWidth: true
            text: nodeName
            elide: Text.ElideRight
            font.pixelSize: 13
            color: {
                switch (gitStatus) {
                    case "M": return theme.yellow
                    case "A": return theme.green
                    case "D": return theme.red
                    case "?": return palette.placeholderText
                    default:  return isActive ? palette.highlightedText : palette.windowText
                }
            }
        }

        // ── Git status badge ──────────────────────────────────────────────
        Label {
            visible: gitStatus.length > 0
            text: gitStatus
            font.pixelSize: 10
            font.bold: true
            color: {
                switch (gitStatus) {
                    case "M": return theme.yellow
                    case "A": return theme.green
                    case "D": return theme.red
                    default:  return palette.placeholderText
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
    Menu {
        id: contextMenu

        MenuItem { text: "New File"; icon.name: "document-new"; onTriggered: newItemDialog.openForFile() }
        MenuItem { text: "New Folder"; icon.name: "folder-new"; onTriggered: newItemDialog.openForFolder() }
        MenuSeparator {}
        MenuItem { text: "Rename"; icon.name: "edit-rename"; onTriggered: renameDialog.openForNode() }
        MenuItem { text: "Delete"; icon.name: "edit-delete"; onTriggered: deleteConfirmDialog.open() }
        MenuSeparator {}
        MenuItem { text: "Copy Path"; onTriggered: { var cb = Qt.application.clipboard; if (cb) cb.text = nodePath } }
        MenuItem { text: "Open in File Manager"; icon.name: "system-file-manager"; onTriggered: Qt.openUrlExternally("file://" + (isDir ? nodePath : nodePath.substring(0, nodePath.lastIndexOf("/")))) }
    }

    // ── Inline dialogs ────────────────────────────────────────────────────────
    Dialog {
        id: newItemDialog
        property bool isFolder: false
        function openForFile() { isFolder = false; newItemInput.text = ""; open(); newItemInput.forceActiveFocus() }
        function openForFolder() { isFolder = true; newItemInput.text = ""; open(); newItemInput.forceActiveFocus() }
        title: isFolder ? "New Folder" : "New File"
        anchors.centerIn: Overlay.overlay
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        ColumnLayout {
            spacing: 8
            Label { text: "Name:" }
            TextField { id: newItemInput; Layout.preferredWidth: 260; onAccepted: newItemDialog.accept() }
        }
        onAccepted: {
            if (newItemInput.text.length === 0) return
            var pd = isDir ? nodePath : nodePath.substring(0, nodePath.lastIndexOf("/"))
            if (isFolder) fileTreeModel.create_folder(pd, newItemInput.text)
            else fileTreeModel.create_file(pd, newItemInput.text)
        }
    }

    Dialog {
        id: renameDialog
        function openForNode() { renameInput.text = nodeName; open(); renameInput.forceActiveFocus(); renameInput.selectAll() }
        title: "Rename"
        anchors.centerIn: Overlay.overlay
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        ColumnLayout {
            spacing: 8
            Label { text: "New name:" }
            TextField { id: renameInput; Layout.preferredWidth: 260; onAccepted: renameDialog.accept() }
        }
        onAccepted: {
            if (renameInput.text.length > 0 && renameInput.text !== nodeName)
                fileTreeModel.rename_item(nodePath, renameInput.text)
        }
    }

    Dialog {
        id: deleteConfirmDialog
        title: "Delete"
        anchors.centerIn: Overlay.overlay
        modal: true
        standardButtons: Dialog.Yes | Dialog.No
        Label { text: "Delete \"" + nodeName + "\"?"; wrapMode: Text.WordWrap }
        onAccepted: fileTreeModel.delete_item(nodePath)
    }
}
