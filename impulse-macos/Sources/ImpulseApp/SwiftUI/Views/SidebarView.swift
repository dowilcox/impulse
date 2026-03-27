import SwiftUI
import AppKit

/// Sidebar showing the file tree. Follows Apple's pattern — clean list with
/// section headers, no custom chrome. Search is handled by the toolbar
/// `.searchable()` modifier on the parent NavigationSplitView.
struct SidebarView: View {
    var model: WindowModel

    var body: some View {
        Group {
            if model.sidebarPanel == .search && !model.searchQuery.isEmpty {
                SearchPanelView(model: model)
            } else {
                fileTree
            }
        }
        .toolbar {
            ToolbarItemGroup(placement: .automatic) {
                Button(action: { model.onRefreshTree?() }) {
                    Image(systemName: "arrow.clockwise")
                }
                .help("Refresh")

                Menu {
                    Button(action: { model.onNewFile?(model.fileTreeRootPath) }) {
                        Label("New File", systemImage: "doc.badge.plus")
                    }
                    Button(action: { model.onNewFolder?(model.fileTreeRootPath) }) {
                        Label("New Folder", systemImage: "folder.badge.plus")
                    }
                    Divider()
                    Button(action: { model.onToggleHidden?() }) {
                        Label(
                            model.showHiddenFiles ? "Hide Hidden Files" : "Show Hidden Files",
                            systemImage: model.showHiddenFiles ? "eye.slash" : "eye"
                        )
                    }
                    Button(action: { model.onCollapseAll?() }) {
                        Label("Collapse All", systemImage: "arrow.down.right.and.arrow.up.left")
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .help("More Actions")
            }
        }
    }

    private var fileTree: some View {
        FileTreeListView(model: model)
    }
}
