import AppKit
import SwiftUI

/// Warp-style vertical tab list shown at the top of the sidebar when
/// `tab_bar_position` is "sidebar". Each row shows the tab icon, title, and
/// a git-branch (or directory) subtitle, styled to match the file tree so
/// the sidebar reads as one native surface.
struct SidebarTabListView: View {
  var windowModel: WindowModel
  @State private var hoveredTabId: Int? = nil
  @State private var draggedTabId: Int? = nil
  @State private var dragOffset: CGFloat = 0
  @State private var rowFrames: [Int: CGRect] = [:]

  private let rowHeight: CGFloat = 40

  var body: some View {
    VStack(spacing: 2) {
      header

      ScrollView(.vertical) {
        VStack(spacing: 1) {
          ForEach(windowModel.tabDisplayInfos) { tab in
            tabRow(tab)
              .background(
                GeometryReader { geo in
                  Color.clear.preference(
                    key: TabRowFrameKey.self,
                    value: [tab.id: geo.frame(in: .named("sidebarTabList"))]
                  )
                }
              )
              .offset(y: rowOffset(for: tab))
              .animation(
                draggedTabId == tab.id
                  ? nil : .interactiveSpring(response: 0.25, dampingFraction: 0.8),
                value: rowOffset(for: tab)
              )
              .zIndex(draggedTabId == tab.id ? 1 : 0)
              .simultaneousGesture(
                DragGesture(minimumDistance: 5, coordinateSpace: .named("sidebarTabList"))
                  .onChanged { value in
                    if draggedTabId == nil {
                      draggedTabId = tab.id
                      windowModel.onTabSelected?(tab.index)
                    }
                    dragOffset = value.translation.height
                  }
                  .onEnded { _ in
                    commitDrag()
                  }
              )
          }
        }
        .padding(.horizontal, 8)
      }
      .coordinateSpace(name: "sidebarTabList")
      .onPreferenceChange(TabRowFrameKey.self) { rowFrames = $0 }
      .frame(maxHeight: listMaxHeight)
    }
    .padding(.top, 4)
  }

  /// Grow with content up to a cap so the file tree keeps most of the sidebar.
  private var listMaxHeight: CGFloat {
    min(CGFloat(windowModel.tabDisplayInfos.count) * (rowHeight + 1) + 8, 320)
  }

  // MARK: - Header

  private var header: some View {
    HStack {
      Text("Tabs")
        .font(.system(size: 11, weight: .semibold))
        .foregroundStyle(.secondary)
      Spacer()
      Button(action: { windowModel.onNewTab?() }) {
        Image(systemName: "plus")
          .font(.system(size: 11, weight: .semibold))
          .foregroundStyle(.secondary)
          .frame(width: 20, height: 20)
          .contentShape(Rectangle())
      }
      .buttonStyle(.plain)
      .help("New Terminal Tab")
      .accessibilityLabel("New Tab")
    }
    .padding(.horizontal, 16)
    .padding(.bottom, 2)
  }

  // MARK: - Row

  private func tabRow(_ tab: TabDisplayInfo) -> some View {
    let isSelected = tab.index == windowModel.selectedTabIndex
    let isHovered = hoveredTabId == tab.id
    let isDragging = draggedTabId == tab.id

    return HStack(spacing: 8) {
      if let icon = tab.icon {
        Image(nsImage: icon)
          .resizable()
          .interpolation(.high)
          .frame(width: 16, height: 16)
          .accessibilityHidden(true)
      }

      VStack(alignment: .leading, spacing: 1) {
        Text(tab.title)
          .font(.system(size: 12.5, weight: isSelected ? .medium : .regular))
          .lineLimit(1)
          .truncationMode(.middle)
          .foregroundStyle(isSelected ? Color.primary : Color.secondary)

        if let subtitle = subtitleContent(tab) {
          HStack(spacing: 3) {
            Image(systemName: subtitle.symbol)
              .font(.system(size: 8.5, weight: .medium))
            Text(subtitle.text)
              .font(.system(size: 10.5))
              .lineLimit(1)
              .truncationMode(.middle)
          }
          .foregroundStyle(.tertiary)
        }
      }

      Spacer(minLength: 0)

      if tab.needsAttention && !isSelected {
        Circle()
          .fill(Color.accentColor)
          .frame(width: 6, height: 6)
          .accessibilityHidden(true)
      }

      if tab.isPinned && !isHovered {
        Image(systemName: "pin.fill")
          .font(.system(size: 8))
          .foregroundStyle(.tertiary)
          .accessibilityHidden(true)
      }

      if !tab.isPinned && isHovered {
        Button(action: { windowModel.onTabClosed?(tab.index) }) {
          Image(systemName: "xmark")
            .font(.system(size: 8, weight: .bold))
            .foregroundStyle(.secondary)
            .frame(width: 16, height: 16)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Close \(tab.isTerminal ? "Terminal" : "Editor") Tab \(tab.title)")
      }
    }
    .padding(.horizontal, 8)
    .frame(height: rowHeight)
    .background(rowBackground(isSelected: isSelected || isDragging, isHovered: isHovered))
    .contentShape(Rectangle())
    .simultaneousGesture(
      TapGesture().onEnded {
        windowModel.onTabSelected?(tab.index)
      }
    )
    .contextMenu {
      Button(tab.isPinned ? "Unpin Tab" : "Pin Tab") {
        windowModel.onTabPinToggled?(tab.index)
      }
      Button("Close Tab") {
        windowModel.onTabClosed?(tab.index)
      }
      Divider()
      Button("New Tab") {
        windowModel.onNewTab?()
      }
    }
    .onHover { hovering in
      hoveredTabId = hovering ? tab.id : nil
    }
    .accessibilityElement(children: .combine)
    .accessibilityLabel("\(tab.isTerminal ? "Terminal" : "Editor"): \(tab.title)")
    .accessibilityAddTraits(isSelected ? [.isSelected, .isButton] : [.isButton])
  }

  @ViewBuilder
  private func rowBackground(isSelected: Bool, isHovered: Bool) -> some View {
    if isSelected {
      if windowModel.theme.surfaceStyle == "card" {
        // Card themes: the selected tab is a raised content-colored pill,
        // matching the floating content card.
        RoundedRectangle(cornerRadius: 6, style: .continuous)
          .fill(windowModel.theme.colorBg)
          .shadow(color: .black.opacity(0.12), radius: 1.5, x: 0, y: 1)
      } else {
        RoundedRectangle(cornerRadius: 6, style: .continuous)
          .fill(windowModel.theme.colorAccent.opacity(0.22))
      }
    } else if isHovered {
      RoundedRectangle(cornerRadius: 6, style: .continuous)
        .fill(Color.primary.opacity(0.06))
    }
  }

  private func subtitleContent(_ tab: TabDisplayInfo) -> (symbol: String, text: String)? {
    if let branch = tab.gitBranch, !branch.isEmpty {
      return ("arrow.triangle.branch", branch)
    }
    // Skip the directory when the title already shows it (e.g. fish's
    // "~ - fish" title with a "~" working directory).
    if let directory = tab.directory, !directory.isEmpty, !tab.title.contains(directory) {
      return ("folder", directory)
    }
    return nil
  }

  // MARK: - Drag Reorder (vertical)

  private func rowOffset(for tab: TabDisplayInfo) -> CGFloat {
    if tab.id == draggedTabId {
      return dragOffset
    }

    guard let draggedId = draggedTabId,
      let draggedFrame = rowFrames[draggedId],
      let rowFrame = rowFrames[tab.id]
    else { return 0 }

    let tabs = windowModel.tabDisplayInfos
    guard let draggedIdx = tabs.firstIndex(where: { $0.id == draggedId }),
      let rowIdx = tabs.firstIndex(where: { $0.id == tab.id })
    else { return 0 }

    let draggedCenter = draggedFrame.midY + dragOffset
    let shiftAmount = draggedFrame.height + 1

    if draggedIdx < rowIdx && draggedCenter > rowFrame.midY {
      return -shiftAmount
    }
    if draggedIdx > rowIdx && draggedCenter < rowFrame.midY {
      return shiftAmount
    }
    return 0
  }

  private func commitDrag() {
    guard let draggedId = draggedTabId else { return }

    let tabs = windowModel.tabDisplayInfos
    guard let sourceIdx = tabs.firstIndex(where: { $0.id == draggedId }),
      let draggedFrame = rowFrames[draggedId]
    else {
      draggedTabId = nil
      dragOffset = 0
      return
    }

    let draggedCenter = draggedFrame.midY + dragOffset

    var targetIdx = sourceIdx
    for (idx, tab) in tabs.enumerated() {
      if tab.id == draggedId { continue }
      guard let frame = rowFrames[tab.id] else { continue }
      if sourceIdx < idx && draggedCenter > frame.midY {
        targetIdx = idx
      } else if sourceIdx > idx && draggedCenter < frame.midY && idx < targetIdx {
        targetIdx = idx
      }
    }

    draggedTabId = nil
    dragOffset = 0

    if sourceIdx != targetIdx {
      windowModel.onTabMoved?(sourceIdx, targetIdx)
    }
  }
}

// MARK: - Preference Key for Row Frames

private struct TabRowFrameKey: PreferenceKey {
  static var defaultValue: [Int: CGRect] = [:]
  static func reduce(value: inout [Int: CGRect], nextValue: () -> [Int: CGRect]) {
    value.merge(nextValue()) { $1 }
  }
}
