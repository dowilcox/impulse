import SwiftUI
import AppKit

/// Displays the project file tree as a scrollable outline.
/// Uses a recursive VStack approach instead of List+DisclosureGroup
/// to avoid the known NSOutlineView/DisclosureGroup click conflict.
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
            // The row itself
            HStack(spacing: 0) {
                // Indent
                if depth > 0 {
                    Spacer()
                        .frame(width: CGFloat(depth) * 16)
                }

                // Disclosure indicator for directories
                if node.isDirectory {
                    Image(systemName: node.isExpanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(.tertiary)
                        .frame(width: 16, height: 16)
                } else {
                    Spacer().frame(width: 16)
                }

                FileTreeRow(node: node)
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
                } else {
                    model.onOpenFile?(node.path, nil)
                }
            }
            .contextMenu { nodeContextMenu(for: node) }

            // Children (when expanded)
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
    }
}

/// A single row in the file tree showing an icon, file name, and optional
/// git status badge.
struct FileTreeRow: View {
    let node: FileTreeNode

    var body: some View {
        HStack(spacing: 6) {
            fileIcon
                .frame(width: 16, height: 16)

            Text(node.name)
                .font(.system(size: 13))
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
        if node.isDirectory {
            Image(systemName: node.isExpanded ? "folder.fill" : "folder.fill")
                .font(.system(size: 13))
                .foregroundStyle(Color.accentColor)
        } else {
            let nsImage = NSWorkspace.shared.icon(forFile: node.path)
            Image(nsImage: nsImage)
                .resizable()
                .interpolation(.high)
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
