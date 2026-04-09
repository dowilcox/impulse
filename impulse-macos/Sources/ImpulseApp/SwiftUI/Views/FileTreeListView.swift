import SwiftUI
import AppKit

/// Displays the project file tree as a flat, virtualized scrollable list.
/// Each expanded directory's children appear as separate entries with
/// calculated indentation — no recursive view nesting. This ensures
/// LazyVStack only materializes visible rows regardless of how many
/// folders are expanded.
struct FileTreeListView: View {
    var model: WindowModel
    @State private var isRootDropTarget = false

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(model.flatFileTree) { entry in
                    FlatFileRowView(node: entry.node, depth: entry.depth, model: model)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .onDrop(of: [.fileURL], isTargeted: $isRootDropTarget) { providers in
            FileDropHelper.handleDrop(
                providers: providers,
                targetDir: model.fileTreeRootPath,
                projectRoot: model.fileTreeRootPath,
                onComplete: { model.onRefreshTree?() }
            )
            return true
        }
        .background(isRootDropTarget ? Color.accentColor.opacity(0.08) : Color.clear)
    }
}

/// A single flat row in the file tree. Not recursive — children are separate
/// entries in the flat list with incremented depth.
private struct FlatFileRowView: View {
    @Bindable var node: FileTreeNode
    let depth: Int
    var model: WindowModel
    @State private var isHovered = false
    @State private var isDropTarget = false

    private var isActiveFile: Bool {
        !node.isDirectory && node.path == model.activeFilePath
    }

    private var rowBackground: Color {
        if isDropTarget && node.isDirectory {
            return Color.accentColor.opacity(0.3)
        } else if isActiveFile {
            return Color.accentColor.opacity(0.2)
        } else if isHovered {
            return Color.primary.opacity(0.06)
        }
        return Color.clear
    }

    var body: some View {
        HStack(spacing: 0) {
            if depth > 0 {
                Spacer()
                    .frame(width: CGFloat(depth) * 16)
            }

            if node.isDirectory {
                Image(systemName: node.isExpanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.tertiary)
                    .frame(width: 16, height: 16)
            } else {
                Spacer().frame(width: 16)
            }

            FileTreeRow(node: node, iconCache: model.iconCache)
        }
        .padding(.vertical, 3)
        .padding(.horizontal, 8)
        .background(
            RoundedRectangle(cornerRadius: 5)
                .fill(rowBackground)
        )
        .contentShape(Rectangle())
        .onHover { hovering in
            isHovered = hovering
        }
        .onDrag { [nodePath = node.path] in
            NSItemProvider(object: nodePath as NSString)
        }
        .onDrop(of: [.fileURL, .text], isTargeted: $isDropTarget) { providers in
            handleDrop(providers: providers)
        }
        .onTapGesture {
            if node.isDirectory {
                handleDirectoryTap()
            } else {
                model.onOpenFile?(node.path, nil)
            }
        }
        .contextMenu { nodeContextMenu(for: node) }
    }

    // MARK: - Directory Expand/Collapse

    private func handleDirectoryTap() {
        if node.isExpanded {
            // Collapse — always immediate.
            withAnimation(.easeInOut(duration: 0.15)) {
                node.isExpanded = false
                model.rebuildFlatTree()
            }
        } else if node.isLoaded {
            // Already loaded — expand immediately.
            withAnimation(.easeInOut(duration: 0.15)) {
                node.isExpanded = true
                model.rebuildFlatTree()
            }
            triggerGitStatusRefresh()
        } else {
            // Need to load children off main thread.
            // Flip chevron immediately so the user knows their click registered.
            node.isExpanded = true
            let showHidden = model.showHiddenFiles
            let path = node.path
            DispatchQueue.global(qos: .userInitiated).async {
                let children = FileTreeNode.buildChildren(path: path, showHidden: showHidden)
                DispatchQueue.main.async {
                    node.children = children
                    withAnimation(.easeInOut(duration: 0.15)) {
                        model.rebuildFlatTree()
                    }
                    triggerGitStatusRefresh()
                }
            }
        }
    }

    private func triggerGitStatusRefresh() {
        guard let children = node.children, !children.isEmpty else { return }
        let nodePath = node.path
        let rootPath = model.fileTreeRootPath
        DispatchQueue.global(qos: .utility).async {
            FileTreeNode.refreshGitStatus(
                nodes: children, repoPath: rootPath, dirPath: nodePath
            )
        }
    }

    /// Handles dropping files/folders onto this node.
    /// Internal project files are moved; external files (from Finder) are copied.
    private func handleDrop(providers: [NSItemProvider]) -> Bool {
        let targetDir: String
        if node.isDirectory {
            targetDir = node.path
        } else {
            targetDir = (node.path as NSString).deletingLastPathComponent
        }
        FileDropHelper.handleDrop(
            providers: providers,
            targetDir: targetDir,
            projectRoot: model.fileTreeRootPath,
            onComplete: { model.onRefreshTree?() }
        )
        return true
    }

    @ViewBuilder
    private func nodeContextMenu(for node: FileTreeNode) -> some View {
        if node.isDirectory {
            Button("New File...") {
                model.onNewFile?(node.path)
            }
            Button("New Folder...") {
                model.onNewFolder?(node.path)
            }
            Divider()
        }

        Button("Reveal in Finder") {
            if node.isDirectory {
                NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: node.path)
            } else {
                NSWorkspace.shared.activateFileViewerSelecting(
                    [URL(fileURLWithPath: node.path)]
                )
            }
        }

        Button("Copy Path") {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(node.path, forType: .string)
        }

        Button("Copy Relative Path") {
            let relativePath: String
            if node.path.hasPrefix(model.fileTreeRootPath) {
                relativePath = String(
                    node.path.dropFirst(model.fileTreeRootPath.count)
                        .drop(while: { $0 == "/" })
                )
            } else {
                relativePath = node.name
            }
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(relativePath, forType: .string)
        }

        if !node.isDirectory {
            Divider()
            Button("Open with Default App") {
                NSWorkspace.shared.open(URL(fileURLWithPath: node.path))
            }
        }

        if !node.isDirectory,
           node.gitStatus == .modified || node.gitStatus == .added {
            Divider()
            Button("Discard Changes", role: .destructive) {
                let alert = NSAlert()
                alert.messageText = "Discard Changes"
                alert.informativeText = "Are you sure you want to discard all changes to \"\(node.name)\"? This cannot be undone."
                alert.alertStyle = .warning
                alert.addButton(withTitle: "Discard")
                alert.addButton(withTitle: "Cancel")
                guard alert.runModal() == .alertFirstButtonReturn else { return }
                if ImpulseCore.gitDiscardChanges(filePath: node.path, workspaceRoot: model.fileTreeRootPath) {
                    NotificationCenter.default.post(
                        name: .impulseReloadEditorFile,
                        object: nil,
                        userInfo: ["filePath": node.path]
                    )
                    model.onRefreshTree?()
                }
            }
        }

        Divider()

        Button("Rename…") {
            let currentName = (node.path as NSString).lastPathComponent
            let alert = NSAlert()
            alert.messageText = "Rename"
            alert.informativeText = "Enter a new name:"
            alert.addButton(withTitle: "Rename")
            alert.addButton(withTitle: "Cancel")
            let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
            input.stringValue = currentName
            alert.accessoryView = input
            alert.window.initialFirstResponder = input
            guard alert.runModal() == .alertFirstButtonReturn else { return }
            let newName = input.stringValue.trimmingCharacters(in: .whitespaces)
            guard !newName.isEmpty, newName != currentName,
                  !newName.contains("/"), !newName.contains("\0"),
                  !newName.contains("..") else { return }
            let parentDir = (node.path as NSString).deletingLastPathComponent
            let newPath = ((parentDir as NSString).appendingPathComponent(newName) as NSString).standardizingPath
            let normalizedParent = (parentDir as NSString).standardizingPath
            guard (newPath as NSString).deletingLastPathComponent == normalizedParent else { return }
            do {
                try FileManager.default.moveItem(atPath: node.path, toPath: newPath)
                model.onRefreshTree?()
            } catch {
                let errAlert = NSAlert()
                errAlert.messageText = "Rename Failed"
                errAlert.informativeText = error.localizedDescription
                errAlert.alertStyle = .warning
                errAlert.runModal()
            }
        }

        Button("Move to Trash", role: .destructive) {
            do {
                try FileManager.default.trashItem(
                    at: URL(fileURLWithPath: node.path),
                    resultingItemURL: nil
                )
                model.onRefreshTree?()
            } catch {
                let errAlert = NSAlert()
                errAlert.messageText = "Move to Trash Failed"
                errAlert.informativeText = error.localizedDescription
                errAlert.alertStyle = .warning
                errAlert.runModal()
            }
        }
    }
}

/// A single row: themed icon + file name + git status badge.
struct FileTreeRow: View {
    let node: FileTreeNode
    var iconCache: IconCache?

    var body: some View {
        HStack(spacing: 6) {
            fileIcon
                .frame(width: 16, height: 16)

            Text(node.name)
                .font(.system(size: 13))
                .foregroundStyle(gitNameColor)
                .lineLimit(1)
                .truncationMode(.middle)

            Spacer()

            if let info = gitInfo {
                Text(info.letter)
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundStyle(info.color)
            }
        }
    }

    @ViewBuilder
    private var fileIcon: some View {
        if let nsImage = iconCache?.icon(
            filename: node.name,
            isDirectory: node.isDirectory,
            expanded: node.isExpanded
        ) {
            Image(nsImage: nsImage)
                .resizable()
                .interpolation(.high)
        } else if node.isDirectory {
            Image(systemName: "folder.fill")
                .font(.system(size: 13))
                .foregroundStyle(Color.accentColor)
        } else {
            Image(systemName: "doc.fill")
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
        }
    }

    /// Git status display info: badge letter, color for both name and badge.
    private var gitInfo: (letter: String, color: Color)? {
        switch node.gitStatus {
        case .modified:  return ("M", .yellow)
        case .added:     return ("A", .green)
        case .untracked: return ("?", .green)
        case .deleted:   return ("D", .red)
        case .renamed:   return ("R", .blue)
        case .conflict:  return ("C", .orange)
        case .ignored:   return nil
        case .none:      return nil
        }
    }

    private var gitNameColor: Color {
        gitInfo?.color ?? (node.gitStatus == .ignored ? .secondary : .primary)
    }
}

// MARK: - File Drop Helper

/// Shared logic for handling file drops from Finder or internal tree moves.
enum FileDropHelper {
    /// Processes drop providers, copying external files or moving internal ones.
    static func handleDrop(
        providers: [NSItemProvider],
        targetDir: String,
        projectRoot: String,
        onComplete: @escaping () -> Void
    ) {
        for provider in providers {
            // Prefer file URL (Finder drops and modern pasteboard).
            if provider.hasItemConformingToTypeIdentifier("public.file-url") {
                provider.loadItem(forTypeIdentifier: "public.file-url", options: nil) { data, _ in
                    guard let urlData = data as? Data,
                          let url = URL(dataRepresentation: urlData, relativeTo: nil),
                          url.isFileURL else { return }
                    let sourcePath = url.path
                    DispatchQueue.main.async {
                        processFile(source: sourcePath, targetDir: targetDir,
                                    projectRoot: projectRoot, onComplete: onComplete)
                    }
                }
            } else if provider.hasItemConformingToTypeIdentifier("public.text") {
                // Fallback for internal drags that only provide text.
                provider.loadItem(forTypeIdentifier: "public.text", options: nil) { data, _ in
                    guard let pathData = data as? Data,
                          let sourcePath = String(data: pathData, encoding: .utf8),
                          sourcePath.hasPrefix("/"),
                          FileManager.default.fileExists(atPath: sourcePath) else { return }
                    DispatchQueue.main.async {
                        processFile(source: sourcePath, targetDir: targetDir,
                                    projectRoot: projectRoot, onComplete: onComplete)
                    }
                }
            }
        }
    }

    /// Moves an internal file or copies an external file to the target directory.
    /// Shows a confirmation dialog if the destination already exists.
    private static func processFile(
        source: String,
        targetDir: String,
        projectRoot: String,
        onComplete: @escaping () -> Void
    ) {
        let fm = FileManager.default
        let normalizedSource = (source as NSString).standardizingPath
        let sourceName = (normalizedSource as NSString).lastPathComponent
        let destPath = ((targetDir as NSString).appendingPathComponent(sourceName) as NSString).standardizingPath

        // Can't drop onto itself or into itself.
        guard normalizedSource != destPath,
              !destPath.hasPrefix(normalizedSource + "/") else { return }

        let isInternal = normalizedSource.hasPrefix((projectRoot as NSString).standardizingPath)

        // If destination exists, ask to replace.
        if fm.fileExists(atPath: destPath) {
            let alert = NSAlert()
            alert.messageText = "An item named \"\(sourceName)\" already exists"
            alert.informativeText = isInternal
                ? "Do you want to replace it? The original will be moved."
                : "Do you want to replace it with the one you're copying?"
            alert.alertStyle = .warning
            alert.addButton(withTitle: "Replace")
            alert.addButton(withTitle: "Cancel")
            guard alert.runModal() == .alertFirstButtonReturn else { return }
            do {
                try fm.removeItem(atPath: destPath)
            } catch {
                showError("Replace Failed", error.localizedDescription)
                return
            }
        }

        do {
            if isInternal {
                try fm.moveItem(atPath: normalizedSource, toPath: destPath)
            } else {
                try fm.copyItem(atPath: normalizedSource, toPath: destPath)
            }
            onComplete()
        } catch {
            showError(isInternal ? "Move Failed" : "Copy Failed", error.localizedDescription)
        }
    }

    private static func showError(_ title: String, _ message: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.runModal()
    }
}
