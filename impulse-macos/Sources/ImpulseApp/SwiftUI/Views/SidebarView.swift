import AppKit
import SwiftUI

/// Sidebar showing the file tree or search results.
/// Clean content only — action buttons live in the window toolbar (titlebar).
struct SidebarView: View {
  var model: WindowModel

  var body: some View {
    Group {
      if model.sidebarPanel == .search {
        SearchPanelView(model: model)
      } else {
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
