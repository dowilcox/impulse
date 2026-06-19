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

  /// Row height (plus 1pt inter-row spacing) — shared with `SidebarView` so it
  /// can size the auto-height tab section from the tab count.
  static let rowHeight: CGFloat = 40

  var body: some View {
    VStack(spacing: 2) {
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
    }
    .padding(.top, 4)
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
          .font(.system(size: 12.5, weight: isSelected ? .semibold : .medium))
          .lineLimit(1)
          .truncationMode(.middle)
          .foregroundStyle(
            isSelected ? windowModel.theme.colorFg : windowModel.theme.colorFg.opacity(0.82))

        let subtitleSegments = subtitleSegments(tab)
        if !subtitleSegments.isEmpty {
          HStack(spacing: 6) {
            ForEach(Array(subtitleSegments.enumerated()), id: \.offset) { _, segment in
              HStack(spacing: 3) {
                Image(systemName: segment.symbol)
                  .font(.system(size: 8.5, weight: .medium))
                Text(segment.text)
                  .font(.system(size: 10.5))
                  .lineLimit(1)
                  .truncationMode(.middle)
              }
              // Keep the branch fully visible; let the longer path truncate first.
              .layoutPriority(segment.symbol == "folder" ? 0 : 1)
            }
          }
          .foregroundStyle(windowModel.theme.colorFgMuted)
        }
      }

      Spacer(minLength: 0)

      if tab.needsAttention && !isSelected {
        Circle()
          .fill(windowModel.theme.colorAccent)
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
    .frame(height: Self.rowHeight)
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

  private func subtitleSegments(_ tab: TabDisplayInfo) -> [(symbol: String, text: String)] {
    // Open files (editor tabs): show the file's location path, not the branch.
    if !tab.isTerminal {
      if let directory = tab.directory, !directory.isEmpty {
        return [("folder", directory)]
      }
      return []
    }
    // Terminal tabs: always surface the working folder and the git branch
    // together so the user knows where the tab runs and on what branch — each
    // with its own icon (folder for the path, branch glyph for the branch).
    var segments: [(symbol: String, text: String)] = []
    // Skip the directory when the title already shows it (e.g. fish's
    // "~ - fish" title with a "~" working directory). When a program/TUI is
    // running the title is the program name (e.g. "Claude Code"), so the path
    // is still shown.
    if let directory = tab.directory, !directory.isEmpty, !tab.title.contains(directory) {
      segments.append(("folder", directory))
    }
    if let branch = tab.gitBranch, !branch.isEmpty {
      segments.append(("arrow.triangle.branch", branch))
    }
    return segments
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
