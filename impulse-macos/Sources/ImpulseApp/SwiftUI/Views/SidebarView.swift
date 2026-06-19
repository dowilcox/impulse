import AppKit
import SwiftUI

/// Sidebar showing the file tree or search results, with the vertical tab
/// list and a file-action bar stacked above it.
///
/// The boundary between the tab list and the file tree is a draggable split
/// handle: dragging it down grows the tab section (so every tab can show at
/// once) and shrinks the file tree, which then scrolls. The chosen height is
/// persisted so it survives relaunches. Until the user drags, the tab section
/// auto-sizes to its content, capped so the file tree always keeps room.
struct SidebarView: View {
  var model: WindowModel

  /// Persisted tab-section height in points. `0` means "auto" (size to the tab
  /// count, capped to roughly half the sidebar).
  @AppStorage("sidebarTabSectionHeight") private var storedTabHeight: Double = 0
  /// Live height during a drag. Kept local so dragging doesn't thrash
  /// UserDefaults — `storedTabHeight` is written once on release.
  @State private var liveHeight: CGFloat? = nil
  /// Section height captured at the moment a drag starts.
  @State private var dragAnchor: CGFloat = 0
  @State private var dividerHovered = false

  /// Per-row stride in the tab list (row height + 1pt inter-row spacing).
  private var rowStride: CGFloat { SidebarTabListView.rowHeight + 1 }
  /// Smallest the tab section may shrink to (about one row).
  private let minTabHeight: CGFloat = 48
  /// Always leave at least this much for the file tree / action bar.
  private let minTreeHeight: CGFloat = 140
  /// Height of the grabbable divider band.
  private let dividerBand: CGFloat = 12

  private var showsTabList: Bool {
    model.tabBarPosition == "sidebar" && !model.tabDisplayInfos.isEmpty
  }

  var body: some View {
    GeometryReader { geo in
      VStack(spacing: 0) {
        if showsTabList {
          let available = geo.size.height
          let tabHeight = resolvedTabHeight(available: available)
          SidebarTabListView(windowModel: model)
            .frame(height: tabHeight)
          resizableDivider(available: available, current: tabHeight)
        }

        if model.sidebarPanel == .search {
          SearchPanelView(model: model)
        } else {
          // File-action bar sits between the tabs and the file tree.
          SidebarActionBarView(model: model)
          FileTreeListView(model: model)
        }
      }
    }
    // Card-surface themes (Harbor): the sidebar is part of the slate canvas,
    // not a separate glass panel — paint over the system sidebar material so
    // the whole chrome reads as one continuous surface.
    .background(
      model.theme.surfaceStyle == "card" ? model.theme.colorBgDark : Color.clear
    )
  }

  // MARK: - Tab-section sizing

  /// Natural height needed to show every tab without scrolling.
  private var contentHeight: CGFloat {
    CGFloat(model.tabDisplayInfos.count) * rowStride + 12
  }

  /// Resolve the tab section's height for the current sidebar height, honoring
  /// the live drag value (or persisted value) when present and clamping so the
  /// file tree always survives.
  private func resolvedTabHeight(available: CGFloat) -> CGFloat {
    let maxTab = max(minTabHeight, available - minTreeHeight)
    let base: CGFloat
    if let live = liveHeight {
      base = live
    } else if storedTabHeight > 0 {
      base = CGFloat(storedTabHeight)
    } else {
      // Auto: grow with content but never take more than ~half the sidebar.
      let autoCap = min(maxTab, available * 0.5)
      return min(contentHeight, max(minTabHeight, autoCap))
    }
    return min(max(base, minTabHeight), maxTab)
  }

  // MARK: - Resizable divider

  private func resizableDivider(available: CGFloat, current: CGFloat) -> some View {
    VerticalResizeHandle(
      onHoverChanged: { dividerHovered = $0 },
      onChanged: { delta in
        if liveHeight == nil { dragAnchor = current }
        let maxTab = max(minTabHeight, available - minTreeHeight)
        liveHeight = min(max(dragAnchor + delta, minTabHeight), maxTab)
      },
      onEnded: {
        if let live = liveHeight { storedTabHeight = Double(live) }
        liveHeight = nil
      },
      onReset: {
        liveHeight = nil
        storedTabHeight = 0
      }
    )
    .frame(maxWidth: .infinity)
    .frame(height: dividerBand)
    .overlay {
      // Purely visual line, centered in the grab band. Non-interactive so the
      // AppKit handle beneath receives every mouse event.
      RoundedRectangle(cornerRadius: 1, style: .continuous)
        .fill(
          dividerHovered
            ? model.theme.colorAccent.opacity(0.7)
            : model.theme.colorBorder.opacity(0.6)
        )
        .frame(height: dividerHovered ? 2 : 1)
        .padding(.horizontal, 12)
        .allowsHitTesting(false)
        .animation(.easeOut(duration: 0.12), value: dividerHovered)
    }
  }
}
