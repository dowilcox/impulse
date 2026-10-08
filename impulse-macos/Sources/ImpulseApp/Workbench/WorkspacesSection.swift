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
struct WorkspacesSection: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  /// The most a dragged height may take (leaves room for the files panel);
  /// nil while the dock's height is unknown.
  var heightLimit: CGFloat?

  private static let rowHeight: CGFloat = 26
  private static let tabRowHeight: CGFloat = 24
  private static let autoMaxHeight: CGFloat = 280

  @State private var dragStartHeight: CGFloat = 0

  var body: some View {
    VStack(spacing: 0) {
      ScrollViewReader { proxy in
        ScrollView(.vertical) {
          VStack(spacing: 1) {
            ForEach(groups, id: \.id) { group in
              if let name = group.name {
                RepoGroupHeader(name: name)
              }
              ForEach(group.workspaces) { workspace in
                WorkspaceRow(
                  model: model, workspace: workspace, indented: group.name != nil,
                  isExpanded: isExpanded(workspace), tabsPinnedOpen: tabsPinnedOpen(workspace),
                  canMoveUp: canMove(workspace, by: -1), canMoveDown: canMove(workspace, by: 1))
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
      trailing(snapshot: snapshot)
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
    if workspace.repository != nil {
      Button("New Task…") { model.onNewTask?(workspace.id) }
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
    if workspace.isTask {
      Button("Archive Task…") { model.onArchiveTask?(workspace.id) }
    }
    Button("Close Workspace") { model.onCloseWorkspace?(workspace.id) }
  }

  private func help(snapshot: RepoSnapshot?) -> String {
    var lines = [workspace.isScratch ? "Scratch — the file tree follows the active tab" : workspace.root]
    if let branch = snapshot?.branch { lines.append("Branch: \(branch)") }
    if let summary = workspace.taskSummary { lines.append(summary) }
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
    .simultaneousGesture(TapGesture().onEnded { model.onTabSelected?(tab.index) })
    .simultaneousGesture(TapGesture(count: 2).onEnded { model.onKeepTab?(tab.index) })
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
        Button(action: { model.onTabClosed?(tab.index) }) {
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
