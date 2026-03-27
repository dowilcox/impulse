import SwiftUI
import AppKit

/// Displays the project file tree. Directories are expandable by clicking
/// anywhere on the row (not just the disclosure triangle).
struct FileTreeListView: View {
    var model: WindowModel

    var body: some View {
        List {
            ForEach(model.fileTreeNodes) { node in
                FileNodeView(node: node, model: model)
            }
        }
        .listStyle(.sidebar)
    }
}

/// Recursive view for a single file tree node. Directories use DisclosureGroup
/// so the entire row is clickable to expand/collapse.
private struct FileNodeView: View {
    let node: FileTreeNode
    var model: WindowModel

    var body: some View {
        if node.isDirectory {
            DisclosureGroup(isExpanded: Binding(
                get: { node.isExpanded },
                set: { newValue in
                    node.isExpanded = newValue
                    if newValue && !node.isLoaded {
                        node.loadChildren(showHidden: model.showHiddenFiles)
                    }
                }
            )) {
                if let children = node.children {
                    ForEach(children) { child in
                        FileNodeView(node: child, model: model)
                    }
                }
            } label: {
                FileTreeRow(node: node, theme: model.theme)
            }
        } else {
            FileTreeRow(node: node, theme: model.theme)
                .contentShape(Rectangle())
                .onTapGesture {
                    model.onOpenFile?(node.path, nil)
                }
        }
    }
}

/// A single row in the file tree showing an icon, file name, and optional
/// git status badge.
struct FileTreeRow: View {
    let node: FileTreeNode
    let theme: Theme

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: node.isDirectory ? "folder.fill" : "doc.fill")
                .font(.system(size: 13))
                .foregroundStyle(node.isDirectory ? Color.accentColor : .secondary)
                .frame(width: 16)

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
