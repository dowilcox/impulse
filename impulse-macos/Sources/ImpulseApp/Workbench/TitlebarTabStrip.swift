import AppKit
import SwiftUI

/// Compact tabs in the titlebar: icon, title, attention dot or progress ring,
/// close-on-hover. Pinned tabs collapse to their icon. Tabs drag to reorder;
/// the strip scrolls horizontally when they overflow.
struct TitlebarTabStrip: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel

  @State private var hoveredId: Int? = nil
  @State private var draggedId: Int? = nil
  @State private var dragOffset: CGFloat = 0
  @State private var frames: [Int: CGRect] = [:]

  private let spacing: CGFloat = 2

  var body: some View {
    HStack(spacing: 4) {
      ScrollViewReader { proxy in
        ScrollView(.horizontal, showsIndicators: false) {
          HStack(spacing: spacing) {
            ForEach(model.tabDisplayInfos) { tab in
              tabView(tab)
                .id(tab.id)
                .background(
                  GeometryReader { geo in
                    Color.clear.preference(
                      key: TabFramesKey.self, value: [tab.id: geo.frame(in: .named("strip"))])
                  }
                )
                .offset(x: offset(for: tab))
                .animation(
                  draggedId == tab.id
                    ? nil : .interactiveSpring(response: 0.25, dampingFraction: 0.85),
                  value: offset(for: tab)
                )
                .zIndex(draggedId == tab.id ? 1 : 0)
                .simultaneousGesture(dragGesture(for: tab))
            }
            ChromeIconButton(icon: .plus, help: "New Terminal Tab (⌘T)") {
              model.onNewTab?()
            }
          }
          .coordinateSpace(name: "strip")
          .onPreferenceChange(TabFramesKey.self) { frames = $0 }
          .padding(.vertical, 6)
        }
        .onChange(of: model.selectedTabIndex) { _, index in
          guard index >= 0, index < model.tabDisplayInfos.count else { return }
          withAnimation(.easeOut(duration: 0.15)) {
            proxy.scrollTo(model.tabDisplayInfos[index].id)
          }
        }
      }
      .fixedSize(horizontal: false, vertical: true)
    }
    .padding(.leading, 4)
    .accessibilityElement(children: .contain)
    .accessibilityLabel("Tabs")
  }

  // MARK: Tab

  private func tabView(_ tab: TabDisplayInfo) -> some View {
    let selected = tab.index == model.selectedTabIndex
    let hovered = hoveredId == tab.id
    let dragging = draggedId == tab.id
    let compact = tab.isPinned

    return HStack(spacing: 6) {
      tabIcon(tab, selected: selected)
      if !compact {
        Text(tab.title)
          .font(ChromeFont.ui(12, weight: selected ? .medium : .regular))
          .foregroundStyle(selected || dragging ? chrome.text : chrome.textSecondary)
          .lineLimit(1)
          .truncationMode(.middle)
          .frame(maxWidth: 180, alignment: .leading)
        if tab.paneCount > 1 {
          HStack(spacing: 2) {
            Icon(tab.isZoomed ? .maximize2 : .columns2, size: 10)
            Text("\(tab.paneCount)").font(ChromeFont.mono(10))
          }
          .foregroundStyle(chrome.textTertiary)
          .help(tab.isZoomed ? "Zoomed pane (⌘⇧↩ to restore)" : "\(tab.paneCount) panes")
        }
        trailingAccessory(tab, selected: selected, hovered: hovered)
      }
    }
    .padding(.leading, compact ? 8 : 9)
    .padding(.trailing, compact ? 8 : 5)
    .frame(height: Metrics.tabHeight)
    .background(
      RoundedRectangle(cornerRadius: Metrics.radius, style: .continuous)
        .fill(selected || dragging ? chrome.raised : hovered ? chrome.hover : .clear)
    )
    .overlay(alignment: .bottom) {
      if selected {
        Capsule().fill(chrome.accent).frame(height: 2).padding(.horizontal, 8)
          .offset(y: 0)
      }
    }
    .contentShape(Rectangle())
    .simultaneousGesture(TapGesture().onEnded { model.onTabSelected?(tab.index) })
    .onHover { hoveredId = $0 ? tab.id : (hoveredId == tab.id ? nil : hoveredId) }
    .contextMenu { contextMenu(tab) }
    .help(tabHelp(tab))
    .accessibilityElement(children: .combine)
    .accessibilityLabel(accessibilityLabel(tab))
    .accessibilityAddTraits(selected ? [.isSelected, .isButton] : [.isButton])
  }

  @ViewBuilder
  private func tabIcon(_ tab: TabDisplayInfo, selected: Bool) -> some View {
    if let progress = tab.progress {
      ProgressRing(
        progress: progress.fraction, color: progressColor(progress), size: 12, lineWidth: 1.6)
    } else if tab.isTerminal {
      Icon(tab.isDirectInteractionActive ? .squareTerminal : .terminal, size: 13)
        .foregroundStyle(selected ? chrome.text : chrome.textTertiary)
    } else if let icon = tab.icon {
      Image(nsImage: icon)
        .resizable()
        .interpolation(.high)
        .frame(width: 14, height: 14)
        .accessibilityHidden(true)
    } else {
      Icon(.file, size: 13).foregroundStyle(chrome.textTertiary)
    }
  }

  @ViewBuilder
  private func trailingAccessory(_ tab: TabDisplayInfo, selected: Bool, hovered: Bool) -> some View {
    ZStack {
      if hovered && !tab.isPinned {
        Button(action: { model.onTabClosed?(tab.index) }) {
          Icon(.x, size: 11, strokeWidth: 2.2)
            .foregroundStyle(chrome.textSecondary)
            .frame(width: 16, height: 16)
            .background(Circle().fill(chrome.hover))
            .contentShape(Circle())
        }
        .buttonStyle(ChromePressStyle())
        .accessibilityLabel("Close \(tab.title)")
      } else if tab.needsAttention && !selected {
        StatusDot(color: chrome.attention, size: 7)
      } else if tab.isDirty {
        StatusDot(color: chrome.textSecondary, size: 7)
      }
    }
    .frame(width: 16, height: 16)
  }

  private func progressColor(_ progress: TerminalProgress) -> Color {
    switch progress.state {
    case .error: return chrome.danger
    case .paused: return chrome.warning
    default: return chrome.working
    }
  }

  private func tabHelp(_ tab: TabDisplayInfo) -> String {
    var parts = [tab.title]
    if let dir = tab.directory { parts.append(dir) }
    if let branch = tab.gitBranch { parts.append("⎇ \(branch)") }
    return parts.joined(separator: " — ")
  }

  private func accessibilityLabel(_ tab: TabDisplayInfo) -> String {
    var label = "\(tab.isTerminal ? "Terminal" : "Editor"): \(tab.title)"
    if tab.needsAttention { label += ", needs attention" }
    if let progress = tab.progress, let fraction = progress.fraction {
      label += ", \(Int(fraction * 100)) percent"
    }
    return label
  }

  @ViewBuilder
  private func contextMenu(_ tab: TabDisplayInfo) -> some View {
    Button(tab.isPinned ? "Unpin Tab" : "Pin Tab") { model.onTabPinToggled?(tab.index) }
    Button("Close Tab") { model.onTabClosed?(tab.index) }
    Divider()
    if tab.index != model.selectedTabIndex {
      Button("Move into Current Tab, Right") { model.onJoinTab?(tab.index, false) }
      Button("Move into Current Tab, Below") { model.onJoinTab?(tab.index, true) }
      Divider()
    } else if tab.paneCount > 1 {
      Button("Even Out Panes") { model.onPaneCommand?("equalize_panes") }
      Button("Move Pane to New Tab") { model.onPaneCommand?("move_pane_to_tab") }
      Divider()
    }
    Button("New Terminal Tab") { model.onNewTab?() }
  }

  // MARK: Drag to reorder

  private func dragGesture(for tab: TabDisplayInfo) -> some Gesture {
    DragGesture(minimumDistance: 6, coordinateSpace: .named("strip"))
      .onChanged { value in
        if draggedId == nil {
          draggedId = tab.id
          model.onTabSelected?(tab.index)
        }
        dragOffset = value.translation.width
      }
      .onEnded { _ in commitDrag() }
  }

  private func offset(for tab: TabDisplayInfo) -> CGFloat {
    if tab.id == draggedId { return dragOffset }
    guard let draggedId, let draggedFrame = frames[draggedId], let frame = frames[tab.id],
      let draggedIndex = model.tabDisplayInfos.firstIndex(where: { $0.id == draggedId }),
      let index = model.tabDisplayInfos.firstIndex(where: { $0.id == tab.id })
    else { return 0 }
    let center = draggedFrame.midX + dragOffset
    let shift = draggedFrame.width + spacing
    if draggedIndex < index && center > frame.midX { return -shift }
    if draggedIndex > index && center < frame.midX { return shift }
    return 0
  }

  private func commitDrag() {
    defer {
      draggedId = nil
      dragOffset = 0
    }
    guard let draggedId, let draggedFrame = frames[draggedId],
      let source = model.tabDisplayInfos.firstIndex(where: { $0.id == draggedId })
    else { return }
    let center = draggedFrame.midX + dragOffset
    var target = source
    for (index, tab) in model.tabDisplayInfos.enumerated() where tab.id != draggedId {
      guard let frame = frames[tab.id] else { continue }
      if source < index && center > frame.midX {
        target = index
      } else if source > index && center < frame.midX && index < target {
        target = index
      }
    }
    if target != source { model.onTabMoved?(source, target) }
  }
}

private struct TabFramesKey: PreferenceKey {
  static var defaultValue: [Int: CGRect] = [:]
  static func reduce(value: inout [Int: CGRect], nextValue: () -> [Int: CGRect]) {
    value.merge(nextValue()) { $1 }
  }
}

extension TerminalProgress {
  /// 0...1 for determinate states, nil for indeterminate.
  var fraction: Double? {
    switch state {
    case .indeterminate, .hidden: return nil
    default: return percent.map { Double($0) / 100 }
    }
  }
}
