import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

/// The top of the left dock: the window's workspaces as top-level rows
/// (no section header), with worktrees of the same repository grouped under
/// it. Each row shows the branch, pending changes and tabs that need
/// attention; expanding a row lists its tabs, and its + makes a new
/// workspace (a task from that repository, or another folder). With
/// `sidebar_tabs` on, the active workspace's tabs are always listed here
/// instead of in the titlebar.
///
/// The section fits its rows (up to `autoMaxHeight`) until the hairline under
/// it is dragged; double-clicking the hairline goes back to fitting.
///
/// Rows drag to reorder: a workspace takes its listed tabs along, a worktree
/// stays in its repository's group, and a group's header moves the group.
struct WorkspacesSection: View {
  @Environment(\.chrome) private var chrome
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  var model: WindowModel
  /// The most a dragged height may take (leaves room for the files panel);
  /// nil while the dock's height is unknown.
  var heightLimit: CGFloat?

  private static let rowHeight: CGFloat = 26
  private static let tabRowHeight: CGFloat = 24
  private static let autoMaxHeight: CGFloat = 280
  private static let spacing: CGFloat = 1
  private static let coordinateSpace = "workspaces"
  private static var settle: Animation { .interactiveSpring(response: 0.25, dampingFraction: 0.85) }

  @State private var dragStartHeight: CGFloat = 0
  /// The group or workspace being dragged to a new place, and how far.
  @State private var dragged: DragUnit?
  @State private var dragOffset: CGFloat = 0
  /// Where each group and workspace (with its listed tabs) is laid out.
  @State private var frames: [DragUnit: CGRect] = [:]

  var body: some View {
    let drop = dragPlacement
    VStack(spacing: 0) {
      ScrollViewReader { proxy in
        ScrollView(.vertical) {
          VStack(spacing: Self.spacing) {
            ForEach(groups, id: \.id) { group in
              movable(as: .group(group.id), drop: drop) {
                if let name = group.name {
                  RepoGroupHeader(name: name)
                    .contextMenu {
                      if let first = group.workspaces.first, first.repositoryHasMergedTasks {
                        Button("Archive Merged Tasks…") { model.onArchiveMergedTasks?(first.id) }
                      }
                    }
                    .simultaneousGesture(dragGesture(for: .group(group.id)))
                }
                ForEach(group.workspaces) { workspace in
                  movable(as: .workspace(workspace.id), drop: drop) {
                    WorkspaceRow(
                      model: model, workspace: workspace, indented: group.name != nil,
                      isExpanded: isExpanded(workspace), tabsPinnedOpen: tabsPinnedOpen(workspace),
                      canMoveUp: canMove(workspace, by: -1), canMoveDown: canMove(workspace, by: 1)
                    )
                    // Alone in its group, the workspace is the group.
                    .simultaneousGesture(
                      dragGesture(for: group.name == nil ? .group(group.id) : .workspace(workspace.id)))
                    if isExpanded(workspace) {
                      ForEach(workspace.tabs) { tab in
                        WorkspaceTabRow(model: model, tab: tab).id(tab.id)
                      }
                      if tabsPinnedOpen(workspace) {
                        NewTabRow(model: model)
                      }
                    }
                  }
                }
              }
            }
          }
          .coordinateSpace(.named(Self.coordinateSpace))
          .onPreferenceChange(DragFramesKey.self) { frames = $0 }
          .padding(.vertical, 6)
        }
        .scrollIndicators(.never)
        .onChange(of: model.selectedTabIndex) {
          guard model.showsTabsInSidebar, let selected = model.selectedTabInfo else { return }
          proxy.scrollTo(selected.id)
        }
      }
      .frame(height: height)
      SectionResizeHandle(
        onDragBegan: { dragStartHeight = height },
        onDrag: { delta in model.workspacesHeight = clamped(dragStartHeight + delta) },
        onReset: { model.workspacesHeight = nil }
      )
    }
  }

  private var height: CGFloat {
    guard let dragged = model.workspacesHeight else { return min(contentHeight, autoMaxHeight) }
    return clamped(dragged)
  }

  /// Tabs listed here take more of the dock before the list scrolls.
  private var autoMaxHeight: CGFloat {
    guard model.showsTabsInSidebar, let heightLimit else { return Self.autoMaxHeight }
    return max(Self.autoMaxHeight, heightLimit * 0.6)
  }

  private func clamped(_ height: CGFloat) -> CGFloat {
    min(max(minimumHeight, height), heightLimit ?? .greatestFiniteMagnitude)
  }

  /// With tabs listed here they're shown nowhere else, so the section can't
  /// be dragged shut: it keeps room for the active workspace's row and a
  /// few of its tabs (or all of its rows, when they take less).
  private var minimumHeight: CGFloat {
    guard model.showsTabsInSidebar else { return 0 }
    return min(contentHeight, Self.rowHeight + 1 + 4 * (Self.tabRowHeight + 1) + 12)
  }

  /// The active workspace's tabs are always listed when tabs live in the
  /// sidebar.
  private func tabsPinnedOpen(_ workspace: WorkspaceInfo) -> Bool {
    model.showsTabsInSidebar && workspace.isActive
  }

  private func isExpanded(_ workspace: WorkspaceInfo) -> Bool {
    workspace.isExpanded || tabsPinnedOpen(workspace)
  }

  private func canMove(_ workspace: WorkspaceInfo, by step: Int) -> Bool {
    guard let index = model.workspaces.firstIndex(where: { $0.id == workspace.id }) else { return false }
    return GroupedOrder.moving(model.workspaces, at: index, by: step, key: Self.groupKey) != nil
  }

  // MARK: Drag to reorder

  /// A drag under way: what the dragged unit moves among, where it came
  /// from and where it would land now.
  private struct DragPlacement {
    let siblings: [DragUnit]
    let source: Int
    let target: Int
    /// How far the others move aside: the dragged unit's height and a gap.
    let room: CGFloat
  }

  private var dragPlacement: DragPlacement? {
    guard let dragged else { return nil }
    let siblings = siblings(of: dragged)
    let rects = siblings.compactMap { frames[$0] }
    guard rects.count == siblings.count, let source = siblings.firstIndex(of: dragged) else { return nil }
    let rect = rects[source]
    let target = DragReorder.target(
      middles: rects.map { Double($0.midY) }, source: source,
      draggedSpan: Double(rect.minY + dragOffset)...Double(rect.maxY + dragOffset))
    return DragPlacement(siblings: siblings, source: source, target: target, room: rects[source].height + Self.spacing)
  }

  /// A group or workspace, its rows stacked: it follows the pointer while
  /// it's dragged, and moves aside while a sibling is dragged past it.
  private func movable(
    as unit: DragUnit, drop: DragPlacement?, @ViewBuilder rows: () -> some View
  ) -> some View {
    let isDragged = dragged == unit
    let offset = displacement(of: unit, drop: drop)
    return VStack(spacing: Self.spacing, content: rows)
      .background {
        if isDragged {
          RoundedRectangle(cornerRadius: Metrics.radiusSmall + 1, style: .continuous)
            .fill(chrome.raised)
            .shadow(color: .black.opacity(0.18), radius: 6, y: 2)
            .padding(.horizontal, 6)
        }
      }
      .offset(y: offset)
      .animation(isDragged || reduceMotion ? nil : Self.settle, value: offset)
      // Measured after the offset: where it's laid out, not where it's drawn.
      .background {
        GeometryReader { geo in
          Color.clear.preference(
            key: DragFramesKey.self, value: [unit: geo.frame(in: .named(Self.coordinateSpace))])
        }
      }
      .zIndex(isDragged ? 1 : 0)
  }

  private func displacement(of unit: DragUnit, drop: DragPlacement?) -> CGFloat {
    if unit == dragged { return dragOffset }
    guard let drop, let index = drop.siblings.firstIndex(of: unit) else { return 0 }
    return CGFloat(DragReorder.shift(of: index, source: drop.source, target: drop.target)) * drop.room
  }

  /// What a unit moves among: the groups, or the workspaces of its group.
  private func siblings(of unit: DragUnit) -> [DragUnit] {
    switch unit {
    case .group:
      return groups.map { .group($0.id) }
    case .workspace(let id):
      let group = groups.first { $0.workspaces.contains { $0.id == id } }
      return group?.workspaces.map { .workspace($0.id) } ?? []
    }
  }

  private func dragGesture(for unit: DragUnit) -> some Gesture {
    DragGesture(minimumDistance: 6, coordinateSpace: .named(Self.coordinateSpace))
      .onChanged { value in
        // Only one drag runs at a time: one that never ended gives way.
        if dragged != unit {
          guard siblings(of: unit).count > 1 else { return }
          dragged = unit
        }
        dragOffset = clampedOffset(value.translation.height, for: unit)
      }
      .onEnded { _ in
        guard dragged == unit else { return }
        finishDrag()
      }
  }

  /// `offset` kept within the rows the unit moves among, so it shows where
  /// it can go: a worktree stays in its group.
  private func clampedOffset(_ offset: CGFloat, for unit: DragUnit) -> CGFloat {
    let rects = siblings(of: unit).compactMap { frames[$0] }
    guard let rect = frames[unit], let top = rects.map(\.minY).min(), let bottom = rects.map(\.maxY).max()
    else { return offset }
    return min(max(offset, top - rect.minY), bottom - rect.maxY)
  }

  /// Drops the dragged unit where it was let go; it settles into its new
  /// place as the others close up behind it.
  private func finishDrag() {
    let order = dragPlacement.flatMap(droppedOrder)
    withAnimation(reduceMotion ? nil : Self.settle) {
      if let order { model.onArrangeWorkspaces?(order) }
      dragged = nil
      dragOffset = 0
    }
  }

  /// The workspaces' ids in their new order, or nil when the drop leaves
  /// them as they are.
  private func droppedOrder(_ drop: DragPlacement) -> [UUID]? {
    guard drop.target != drop.source, let dragged else { return nil }
    let workspaces = model.workspaces
    switch dragged {
    case .group(let key):
      guard let index = workspaces.firstIndex(where: { Self.groupKey($0) == key }) else { return nil }
      return GroupedOrder.movingGroup(workspaces, at: index, toPosition: drop.target, key: Self.groupKey)?
        .map(\.id)
    case .workspace(let id):
      guard let index = workspaces.firstIndex(where: { $0.id == id }) else { return nil }
      return GroupedOrder.moving(workspaces, at: index, toPosition: drop.target, key: Self.groupKey)?.map(\.id)
    }
  }

  // MARK: Grouping

  private struct Group {
    let id: String
    /// Repository name when two or more workspaces share it.
    let name: String?
    let workspaces: [WorkspaceInfo]
  }

  private static func groupKey(_ workspace: WorkspaceInfo) -> String {
    Workspace.sidebarGroup(repository: workspace.repository, id: workspace.id)
  }

  /// Workspaces in sidebar order, with those that share a repository (its
  /// worktrees) gathered under the first one's position.
  private var groups: [Group] {
    GroupedOrder.groups(model.workspaces, key: Self.groupKey).map { list in
      let key = Self.groupKey(list[0])
      let name: String? =
        list.count > 1
        ? ((key as NSString).deletingLastPathComponent as NSString).lastPathComponent : nil
      return Group(id: key, name: name, workspaces: list)
    }
  }

  private var contentHeight: CGFloat {
    let groupHeaders = groups.filter { $0.name != nil }.count
    let rows = model.workspaces.count
    let tabRows = model.workspaces.filter(isExpanded).reduce(0) { $0 + $1.tabs.count }
    let newTabRow = model.showsTabsInSidebar ? 1 : 0
    return CGFloat(groupHeaders) * 22 + CGFloat(rows) * (Self.rowHeight + 1)
      + CGFloat(tabRows + newTabRow) * (Self.tabRowHeight + 1) + 12
  }
}

/// What a drag in the section moves.
private enum DragUnit: Hashable {
  /// A repository's group of worktrees, or a workspace alone in its group.
  case group(String)
  /// A worktree within its repository's group.
  case workspace(UUID)
}

private struct DragFramesKey: PreferenceKey {
  static var defaultValue: [DragUnit: CGRect] = [:]
  static func reduce(value: inout [DragUnit: CGRect], nextValue: () -> [DragUnit: CGRect]) {
    value.merge(nextValue()) { $1 }
  }
}

// MARK: - Resize handle

/// The hairline under the workspaces, with a taller invisible grab band.
/// Like the dock dividers, it turns accent while hovered or dragged.
private struct SectionResizeHandle: View {
  @Environment(\.chrome) private var chrome
  let onDragBegan: () -> Void
  /// Vertical distance from where the drag began (down is positive).
  let onDrag: (CGFloat) -> Void
  let onReset: () -> Void

  @State private var hovering = false
  @State private var dragging = false

  var body: some View {
    let active = hovering || dragging
    Rectangle()
      .fill(active ? chrome.accent.opacity(0.8) : chrome.hairline)
      .frame(height: active ? 2 : 1)
      .frame(height: 1)
      .overlay {
        Color.clear
          .frame(height: DockDivider.hitThickness)
          .contentShape(Rectangle())
          .pointerStyle(.rowResize)
          .onHover { hovering = $0 }
          .onTapGesture(count: 2) { onReset() }
          .gesture(
            // Global coordinates: the handle moves with the drag.
            DragGesture(minimumDistance: 1, coordinateSpace: .global)
              .onChanged { value in
                if !dragging {
                  dragging = true
                  onDragBegan()
                }
                onDrag(value.translation.height)
              }
              .onEnded { _ in dragging = false }
          )
      }
      .accessibilityHidden(true)
  }
}

// MARK: - Rows

private struct RepoGroupHeader: View {
  @Environment(\.chrome) private var chrome
  let name: String

  var body: some View {
    HStack(spacing: 5) {
      Icon(.gitFork, size: 11).foregroundStyle(chrome.textTertiary)
      Text(name)
        .font(ChromeFont.ui(11, weight: .medium))
        .foregroundStyle(chrome.textSecondary)
        .lineLimit(1)
      Spacer(minLength: 0)
    }
    .padding(.horizontal, 14)
    .frame(height: 22)
    // The whole row drags the group.
    .contentShape(Rectangle())
  }
}

private struct WorkspaceRow: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  let workspace: WorkspaceInfo
  let indented: Bool
  /// Its tabs are listed under it.
  let isExpanded: Bool
  /// Listed because tabs live in the sidebar (the chevron can't hide them).
  let tabsPinnedOpen: Bool
  let canMoveUp: Bool
  let canMoveDown: Bool
  @State private var hovering = false

  var body: some View {
    let snapshot = workspace.repository?.snapshot
    HStack(spacing: 6) {
      Button {
        model.onSetWorkspaceExpanded?(workspace.id, !isExpanded)
      } label: {
        Icon(isExpanded ? .chevronDown : .chevronRight, size: 10)
          .foregroundStyle(chrome.textTertiary)
          .frame(width: 12, height: 16)
          .contentShape(Rectangle())
      }
      .buttonStyle(ChromePressStyle())
      .opacity((hovering || isExpanded) && !tabsPinnedOpen ? 1 : 0)
      .allowsHitTesting(!tabsPinnedOpen)
      .accessibilityHidden(tabsPinnedOpen)
      .help(isExpanded ? "Hide tabs" : "Show tabs")

      leadingIcon
      Text(workspace.name)
        .font(ChromeFont.ui(12, weight: workspace.isActive ? .semibold : .regular))
        .foregroundStyle(workspace.isActive ? chrome.text : chrome.textSecondary)
        .lineLimit(1)
        .truncationMode(.middle)
      if let branch = snapshot?.branch ?? snapshot?.headOid.map({ String($0.prefix(7)) }),
        branch != workspace.name
      {
        Text(branch)
          .font(ChromeFont.mono(10.5))
          .foregroundStyle(chrome.textTertiary)
          .lineLimit(1)
          .truncationMode(.middle)
          .layoutPriority(-1)
      } else if workspace.isScratch {
        Text("follows tab")
          .font(ChromeFont.ui(10.5))
          .foregroundStyle(chrome.textTertiary)
          .lineLimit(1)
          .layoutPriority(-1)
      }
      Spacer(minLength: 4)
      // The badges keep their size; the name and branch truncate instead.
      trailing(snapshot: snapshot)
        .fixedSize()
        .layoutPriority(1)
      ChromeMenuButton(help: "New workspace") { newWorkspaceItems } label: {
        Icon(.plus, size: 12).foregroundStyle(chrome.textSecondary)
      }
      .opacity(hovering ? 1 : 0)
      .allowsHitTesting(hovering)
    }
    .padding(.leading, indented ? 16 : 8)
    .padding(.trailing, 6)
    .frame(height: 26)
    // When its tabs are listed, the selected tab carries the highlight.
    .rowBackground(selected: workspace.isActive && !isExpanded, hovered: hovering)
    .overlay(alignment: .leading) {
      if workspace.isActive {
        Capsule().fill(chrome.accent).frame(width: 2, height: 14).padding(.leading, 3)
      }
    }
    .contentShape(Rectangle())
    .onTapGesture { model.onSelectWorkspace?(workspace.id) }
    .onHover { hovering = $0 }
    .contextMenu { contextMenu }
    .help(help(snapshot: snapshot))
    .accessibilityElement(children: .combine)
    .accessibilityLabel(accessibilityLabel(snapshot: snapshot))
    .accessibilityAddTraits(workspace.isActive ? [.isSelected, .isButton] : [.isButton])
  }

  @ViewBuilder
  private var leadingIcon: some View {
    if let progress = workspace.progress {
      ProgressRing(progress: progress.fraction, color: chrome.working, size: 12, lineWidth: 1.6)
    } else {
      Icon(
        workspace.isScratch ? .squareTerminal : (workspace.repository != nil ? .folderGit2 : .folder),
        size: 13
      )
      .foregroundStyle(workspace.isActive ? chrome.accent : chrome.textTertiary)
    }
  }

  @ViewBuilder
  private func trailing(snapshot: RepoSnapshot?) -> some View {
    HStack(spacing: 6) {
      if workspace.isMerged {
        Text("merged")
          .font(ChromeFont.mono(10))
          .foregroundStyle(chrome.success)
          .help("Its branch is merged into its base: archive it when you're done")
      }
      if !workspace.overlaps.isEmpty {
        OverlapChip(model: model, workspace: workspace)
      }
      if let port = workspace.ports.first {
        Text(verbatim: workspace.ports.count > 1 ? ":\(port.port)+" : ":\(port.port)")
          .font(ChromeFont.mono(10))
          .foregroundStyle(chrome.textTertiary)
          .help(workspace.ports.map { ":\($0.port) \($0.process)" }.joined(separator: "\n"))
      }
      if workspace.agentsWorking > 0 {
        ProgressRing(progress: nil, color: chrome.working, size: 10, lineWidth: 1.4)
          .help("\(workspace.agentsWorking) agent(s) working")
      }
      if workspace.agentsWaiting > 0 {
        HStack(spacing: 2) {
          Icon(.bot, size: 10)
          Text("\(workspace.agentsWaiting)").font(ChromeFont.mono(10))
        }
        .foregroundStyle(chrome.attention)
        .help("\(workspace.agentsWaiting) agent(s) waiting for you")
      }
      if let snapshot, snapshot.behind > 0, let upstream = snapshot.upstream {
        Button {
          model.onPullWorkspace?(workspace.id)
        } label: {
          Text(verbatim: "↓\(snapshot.behind)")
            .font(ChromeFont.mono(10))
            .foregroundStyle(chrome.textTertiary)
            .contentShape(Rectangle())
        }
        .buttonStyle(ChromePressStyle())
        .help("\(snapshot.behind) commit\(snapshot.behind == 1 ? "" : "s") behind \(upstream): click to pull")
      }
      if let snapshot, snapshot.changedFileCount > 0 {
        DiffStat(added: snapshot.totalAdded, removed: snapshot.totalRemoved)
      }
      if workspace.attentionCount > 0 {
        Badge(text: "\(workspace.attentionCount)", color: chrome.attention)
      } else if !isExpanded, workspace.tabs.count > 1 {
        Text("\(workspace.tabs.count)")
          .font(ChromeFont.mono(10))
          .foregroundStyle(chrome.textTertiary)
      }
    }
  }

  /// What the row's + makes: a task (a worktree workspace) from this
  /// repository, or a workspace for another folder.
  private var newWorkspaceItems: [ChromeMenuItem] {
    var items: [ChromeMenuItem] = []
    if workspace.repository != nil {
      // Named for the main checkout: a task's row makes tasks from there too.
      let repo =
        workspace.repository?.snapshot.map { snapshot -> String in
          let main =
            snapshot.gitDir != snapshot.commonDir && (snapshot.commonDir as NSString).lastPathComponent == ".git"
            ? (snapshot.commonDir as NSString).deletingLastPathComponent : snapshot.root
          return (main as NSString).lastPathComponent
        } ?? workspace.name
      items.append(ChromeMenuItem("New Task from \(repo)…") { model.onNewTask?(workspace.id) })
    }
    items.append(ChromeMenuItem("Open Folder as Workspace…") { model.onOpenWorkspace?() })
    return items
  }

  @ViewBuilder
  private var contextMenu: some View {
    if workspace.isMerged {
      Button("Archive Task…") { model.onArchiveTask?(workspace.id) }
      Divider()
    } else if workspace.isTask {
      Button("Finish Task…") { model.onFinishTask?(workspace.id) }
      Divider()
    }
    if workspace.repository != nil {
      Button("New Task…") { model.onNewTask?(workspace.id) }
      if !workspace.isTask, (workspace.repository?.snapshot?.changedFileCount ?? 0) > 0 {
        Button("Move Changes to New Task…") { model.onMoveChangesToNewTask?(workspace.id) }
      }
      Button("Project Setup…") { model.onProjectSetup?(workspace.id) }
    }
    Button("Open Folder as Workspace…") { model.onOpenWorkspace?() }
    Divider()
    Button("Rename…") { model.onRenameWorkspace?(workspace.id) }
    if !workspace.isScratch {
      Button("Reveal in Finder") {
        NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: workspace.root)
      }
      Button("Copy Path") {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(workspace.root, forType: .string)
      }
    }
    Divider()
    if !tabsPinnedOpen {
      Button(isExpanded ? "Hide Tabs" : "Show Tabs") {
        model.onSetWorkspaceExpanded?(workspace.id, !isExpanded)
      }
    }
    Button("Move Up") { model.onMoveWorkspace?(workspace.id, -1) }
      .disabled(!canMoveUp)
    Button("Move Down") { model.onMoveWorkspace?(workspace.id, 1) }
      .disabled(!canMoveDown)
    Divider()
    if workspace.isTask, !workspace.isMerged {
      Button("Archive Task…") { model.onArchiveTask?(workspace.id) }
    }
    if workspace.repositoryHasMergedTasks {
      Button("Archive Merged Tasks…") { model.onArchiveMergedTasks?(workspace.id) }
    }
    Button("Close Workspace") { model.onCloseWorkspace?(workspace.id) }
  }

  private func help(snapshot: RepoSnapshot?) -> String {
    var lines = [workspace.isScratch ? "Scratch — the file tree follows the active tab" : workspace.root]
    if let branch = snapshot?.branch { lines.append("Branch: \(branch)") }
    if let summary = workspace.taskSummary { lines.append(summary) }
    if workspace.isMerged {
      lines.append("Merged into its base")
    } else if workspace.isUpstreamGone {
      lines.append("Its branch was deleted on the remote; it may have been merged there")
    }
    if let snapshot, snapshot.changedFileCount > 0 {
      lines.append("\(snapshot.changedFileCount) changed file(s)")
    }
    lines.append("\(workspace.tabs.count) tab(s)")
    return lines.joined(separator: "\n")
  }

  private func accessibilityLabel(snapshot: RepoSnapshot?) -> String {
    var label = "Workspace \(workspace.name)"
    if let branch = snapshot?.branch { label += ", branch \(branch)" }
    if workspace.attentionCount > 0 { label += ", \(workspace.attentionCount) need attention" }
    return label
  }
}

/// A tab listed under its expanded workspace: select it, close it on hover,
/// and the same context menu as in the titlebar strip.
private struct WorkspaceTabRow: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  let tab: TabDisplayInfo
  @State private var hovering = false

  var body: some View {
    let selected = tab.index == model.selectedTabIndex
    HStack(spacing: 6) {
      icon
      Text(tab.title)
        .font(ChromeFont.ui(11.5))
        .italic(tab.isPreview)
        .foregroundStyle(selected ? chrome.text : chrome.textSecondary)
        .lineLimit(1)
        .truncationMode(.middle)
      Spacer(minLength: 4)
      trailing(selected: selected)
    }
    .padding(.leading, 40)
    .padding(.trailing, 8)
    .frame(height: 24)
    .rowBackground(selected: selected, hovered: hovering)
    .contentShape(Rectangle())
    .simultaneousGesture(TapGesture().onEnded { model.withTab(tab.id) { model.onTabSelected?($0) } })
    .simultaneousGesture(TapGesture(count: 2).onEnded { model.withTab(tab.id) { model.onKeepTab?($0) } })
    .onHover { hovering = $0 }
    .contextMenu { TabContextMenu(model: model, tab: tab) }
    .help(tab.directory ?? tab.title)
    .accessibilityElement(children: .combine)
    .accessibilityLabel(tab.accessibilityDescription)
    .accessibilityAddTraits(selected ? [.isSelected, .isButton] : [.isButton])
  }

  @ViewBuilder
  private var icon: some View {
    if let state = tab.agentState {
      AgentStatusGlyph(state: state, size: 12)
    } else if let progress = tab.progress {
      ProgressRing(progress: progress.fraction, color: chrome.working, size: 11, lineWidth: 1.5)
    } else if let dotColor = tab.sessionStatus?.dotColor {
      StatusDot(color: Color(nsColor: NSColor(hex: dotColor)), size: 7)
        .frame(width: 12, height: 12)
    } else if tab.isTerminal {
      Icon(tab.isDirectInteractionActive ? .squareTerminal : .terminal, size: 12)
        .foregroundStyle(chrome.textTertiary)
    } else if let icon = tab.icon {
      Image(nsImage: icon).resizable().interpolation(.high).frame(width: 13, height: 13)
    } else {
      Icon(.file, size: 12).foregroundStyle(chrome.textTertiary)
    }
  }

  @ViewBuilder
  private func trailing(selected: Bool) -> some View {
    ZStack {
      if hovering && !tab.isPinned {
        Button(action: { model.withTab(tab.id) { model.onTabClosed?($0) } }) {
          Icon(.x, size: 10, strokeWidth: 2.2)
            .foregroundStyle(chrome.textSecondary)
            .frame(width: 16, height: 16)
            .background(Circle().fill(chrome.hover))
            .contentShape(Circle())
        }
        .buttonStyle(ChromePressStyle())
        .help("Close Tab")
        .accessibilityLabel("Close \(tab.title)")
      } else if tab.needsAttention && !selected {
        StatusDot(color: chrome.attention, size: 6)
      } else if tab.isDirty {
        StatusDot(color: chrome.textSecondary, size: 6)
      } else if tab.isPinned {
        Icon(.pin, size: 10).foregroundStyle(chrome.textTertiary)
          .accessibilityLabel("Pinned")
      }
    }
    .frame(width: 16, height: 16)
  }
}

/// "New Tab" under the active workspace's tabs when they're listed in the
/// sidebar (the titlebar's + isn't there then).
private struct NewTabRow: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  @State private var hovering = false

  var body: some View {
    Button {
      model.onNewTab?()
    } label: {
      HStack(spacing: 6) {
        Icon(.plus, size: 12).foregroundStyle(chrome.textTertiary)
        Text("New Tab")
          .font(ChromeFont.ui(11.5))
          .foregroundStyle(chrome.textTertiary)
        Spacer(minLength: 4)
      }
      .padding(.leading, 40)
      .padding(.trailing, 8)
      .frame(height: 24)
      .rowBackground(selected: false, hovered: hovering)
      .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .onHover { hovering = $0 }
    .help("New Tab (⌘T)")
  }
}

// MARK: - Overlaps

/// "⚠ 17": this workspace changes files another workspace of the repository
/// changes too. Opens the pairs, their files, and a conflict check.
private struct OverlapChip: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  let workspace: WorkspaceInfo
  @State private var showing = false

  var body: some View {
    let path = TaskRegistry.canonical(workspace.root)
    let shared = Set(workspace.overlaps.flatMap(\.files)).count
    Button {
      showing.toggle()
    } label: {
      HStack(spacing: 2) {
        Icon(.triangleAlert, size: 10)
        Text("\(shared)").font(ChromeFont.mono(10))
      }
      .foregroundStyle(chrome.warning)
      .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .help(
      workspace.overlaps.map { "Shares \($0.files.count) file\($0.files.count == 1 ? "" : "s") with \($0.other(than: path).name)" }
        .joined(separator: "\n"))
    .popover(isPresented: $showing, arrowEdge: .trailing) {
      OverlapPopover(
        model: model, workspaceID: workspace.id, path: path, mainPath: workspace.mainCheckoutPath, pairs: workspace.overlaps)
        .environment(\.chrome, chrome)
    }
  }
}

private struct OverlapPopover: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  let workspaceID: UUID
  let path: String
  /// The repository's main checkout, whose changes can move into a task.
  let mainPath: String?
  let pairs: [TaskOverlap.Pair]
  /// Conflict checks by pair: nil while running, the files once done.
  @State private var conflicts: [String: [String]?] = [:]

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 14) {
        ForEach(pairs, id: \.key) { pair in
          let other = pair.other(than: path)
          VStack(alignment: .leading, spacing: 6) {
            Text("Also changed in \(other.name)").font(ChromeFont.ui(12, weight: .semibold)).foregroundStyle(chrome.text)
            ForEach(pair.files.prefix(20), id: \.self) { file in
              let conflicting = (conflicts[pair.key] ?? nil)?.contains(file) == true
              Button {
                model.onOpenOverlapFile?((path as NSString).appendingPathComponent(file))
              } label: {
                HStack(spacing: 4) {
                  if conflicting { Icon(.triangleAlert, size: 10).foregroundStyle(chrome.danger) }
                  Text(file).font(ChromeFont.mono(11)).foregroundStyle(conflicting ? chrome.danger : chrome.textSecondary)
                    .lineLimit(1).truncationMode(.middle)
                }
              }
              .buttonStyle(.plain)
              .help("Open \(file) in this workspace")
            }
            if pair.files.count > 20 {
              Text("…and \(pair.files.count - 20) more").font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
            }
            HStack(spacing: 8) {
              switch conflicts[pair.key] {
              case .none:
                ChromeButton(title: "Check for Conflicts", kind: .secondary) { check(pair) }
              case .some(.none):
                ProgressView().controlSize(.small)
                Text("Checking…").font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
              case .some(.some(let files)):
                Text(files.isEmpty ? "These would merge without conflicts." : "\(files.count) would conflict.")
                  .font(ChromeFont.ui(11)).foregroundStyle(files.isEmpty ? chrome.success : chrome.danger)
              }
              if let mainPath, pair.contains(mainPath) {
                ChromeButton(title: "Move to New Task…", kind: .ghost) { model.onMoveChangesToNewTask?(workspaceID) }
                  .help("Move \((mainPath as NSString).lastPathComponent)'s uncommitted changes into a new task of their own")
              }
            }
          }
        }
      }
      .padding(14)
    }
    .frame(width: 360)
    .frame(maxHeight: 420)
  }

  private func check(_ pair: TaskOverlap.Pair) {
    conflicts[pair.key] = .some(nil)
    DispatchQueue.global(qos: .userInitiated).async {
      let found = OverlapMonitor.conflicts(pair) ?? []
      DispatchQueue.main.async { conflicts[pair.key] = .some(found) }
    }
  }
}
