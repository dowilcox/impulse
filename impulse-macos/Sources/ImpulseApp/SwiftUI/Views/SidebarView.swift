import SwiftUI
import AppKit

/// Sidebar showing the file tree or search results.
/// Clean content only — action buttons live in the window toolbar (titlebar).
struct SidebarView: View {
    var model: WindowModel

    var body: some View {
        if model.sidebarPanel == .search && !model.searchQuery.isEmpty {
            SearchPanelView(model: model)
        } else {
            FileTreeListView(model: model)
        }
    }
}
