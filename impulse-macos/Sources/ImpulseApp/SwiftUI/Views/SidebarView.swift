import AppKit
import SwiftUI

/// Sidebar showing the file tree or search results, with the vertical tab
/// list and a file-action bar stacked above it.
struct SidebarView: View {
  var model: WindowModel

  var body: some View {
    VStack(spacing: 0) {
      // Warp-style vertical tab list above the file tree.
      if model.tabBarPosition == "sidebar" {
        SidebarTabListView(windowModel: model)
        Divider()
          .padding(.horizontal, 12)
          .padding(.top, 4)
      }

      if model.sidebarPanel == .search {
        SearchPanelView(model: model)
      } else {
        // File-action bar sits between the tabs and the file tree.
        SidebarActionBarView(model: model)
        FileTreeListView(model: model)
      }
    }
    // Card-surface themes (Harbor): the sidebar is part of the slate canvas,
    // not a separate glass panel — paint over the system sidebar material so
    // the whole chrome reads as one continuous surface.
    .background(
      model.theme.surfaceStyle == "card" ? model.theme.colorBgDark : Color.clear
    )
  }
}
