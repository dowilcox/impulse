import SwiftUI
import AppKit

/// Displays the project file tree as a scrollable outline.
struct FileTreeListView: View {
    var model: WindowModel

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(model.fileTreeNodes) { node in
                    FileNodeView(node: node, model: model, depth: 0)
                }
            }
        }
    }
}

/// Recursive view for a single file tree node with manual expand/collapse.
private struct FileNodeView: View {
    @Bindable var node: FileTreeNode
    var model: WindowModel
    let depth: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
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
            .contentShape(Rectangle())
            .onTapGesture {
                if node.isDirectory {
                    withAnimation(.easeInOut(duration: 0.15)) {
                        node.isExpanded.toggle()
                    }
                    if node.isExpanded && !node.isLoaded {
                        node.loadChildren(showHidden: model.showHiddenFiles)
                    }
                    // Fetch git status for newly loaded children
                    if node.isExpanded, let children = node.children, !children.isEmpty {
                        let nodePath = node.path
                        let rootPath = model.fileTreeRootPath
                        DispatchQueue.global(qos: .utility).async {
                            FileTreeNode.refreshGitStatus(
                                nodes: children, repoPath: rootPath, dirPath: nodePath
                            )
                        }
                    }
                } else {
                    model.onOpenFile?(node.path, nil)
                }
            }
            .contextMenu { nodeContextMenu(for: node) }

            if node.isDirectory && node.isExpanded, let children = node.children {
                ForEach(children) { child in
                    FileNodeView(node: child, model: model, depth: depth + 1)
                }
            }
        }
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
            guard !newName.isEmpty, !newName.contains("/"), newName != currentName else { return }
            let parentDir = (node.path as NSString).deletingLastPathComponent
            let newPath = (parentDir as NSString).appendingPathComponent(newName)
            do {
                try FileManager.default.moveItem(atPath: node.path, toPath: newPath)
                model.onRefreshTree?()
            } catch {
                NSLog("Rename failed: \(error)")
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
                NSLog("Trash failed: \(error)")
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

            if let badge = gitBadge {
                Text(badge.letter)
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundStyle(badge.color)
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

    private var gitNameColor: Color {
        switch node.gitStatus {
        case .modified:  return .yellow
        case .added:     return .green
        case .untracked: return .green
        case .deleted:   return .red
        case .renamed:   return .blue
        case .conflict:  return .orange
        case .ignored:   return .secondary
        case .none:      return .primary
        }
    }

    private var gitBadge: (letter: String, color: Color)? {
        switch node.gitStatus {
        case .modified:  return ("M", .yellow)
        case .added:     return ("A", .green)
        case .untracked: return ("?", .green)
        case .deleted:   return ("D", .red)
        case .renamed:   return ("R", .blue)
        case .conflict:  return ("C", .orange)
        case .ignored, .none: return nil
        }
    }
}
