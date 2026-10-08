import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

/// The git panel in the left dock: branch and sync, operation banner,
/// Conflicts / Staged / Changes / Untracked / Stashes, and the commit composer.
struct ChangesPanelView: View {
  var model: WindowModel

  var body: some View {
    Group {
      if let repository = model.repository, let host = model.gitHost {
        ChangesPanelContent(
          repository: repository, actions: GitActions(repository: repository, host: host),
          model: model)
      } else {
        EmptyState(
          icon: .gitBranch, title: "Not a git repository",
          message: "Open a folder inside a git repository to see its changes.")
      }
    }
    .environment(\.chrome, model.palette)
  }
}

private struct ChangesPanelContent: View {
  @Environment(\.chrome) private var chrome
  var repository: GitRepositoryState
  let actions: GitActions
  var model: WindowModel

  @State private var conflictsExpanded = true
  @State private var stagedExpanded = true
  @State private var changesExpanded = true
  @State private var untrackedExpanded = true
  @State private var stashesExpanded = false
  @State private var stashes: [GitOperations.StashEntry] = []
  @State private var selection: String? = nil
  @FocusState private var listFocused: Bool

  var body: some View {
    let snapshot = repository.snapshot
    VStack(spacing: 0) {
      header(snapshot)
      if let operation = snapshot?.operation {
        OperationBanner(
          operation: operation, hasConflicts: !(snapshot?.conflicted.isEmpty ?? true), actions: actions)
      }
      if let activity = repository.activity {
        activityBar(activity)
      }
      Hairline()
      ScrollViewReader { proxy in
        ScrollView {
          LazyVStack(alignment: .leading, spacing: 0) {
            if let snapshot {
              sections(snapshot)
            }
          }
          .padding(.bottom, 8)
        }
        .focusable()
        .focused($listFocused)
        .focusEffectDisabled()
        .onKeyPress(phases: .down) { press in
          guard let snapshot else { return .ignored }
          return handleKey(press, snapshot: snapshot, proxy: proxy)
        }
        .onChange(of: model.changesFocusToken) { _, _ in focusList(snapshot) }
        .onChange(of: listFocused) { _, focused in model.noteSidebarFocus(.changes, focused: focused) }
        .onAppear { if model.changesFocusToken > 0 { focusList(snapshot) } }
      }
      .overlay {
        if let snapshot, !snapshot.hasChanges, snapshot.operation == nil {
          EmptyState(
            icon: .circleCheck, title: "No changes",
            message: snapshot.branch.map { "Working tree clean on \($0)." } ?? "Working tree clean.")
        }
      }
      Hairline()
      CommitComposer(repository: repository, actions: actions)
    }
  }

  // MARK: Header

  private func header(_ snapshot: RepoSnapshot?) -> some View {
    HStack(spacing: 6) {
      Button {
        model.onShowBranchSwitcher?()
      } label: {
        HStack(spacing: 5) {
          Icon(.gitBranch, size: 13).foregroundStyle(chrome.textSecondary)
          Text(branchLabel(snapshot))
            .font(ChromeFont.ui(12, weight: .semibold))
            .foregroundStyle(chrome.text)
            .lineLimit(1)
            .truncationMode(.middle)
          Icon(.chevronDown, size: 10).foregroundStyle(chrome.textTertiary)
        }
        .padding(.horizontal, 6)
        .frame(height: 24)
        .contentShape(Rectangle())
      }
      .buttonStyle(ChromePressStyle())
      .help("Switch branch (⌃⌘B)")
      Spacer(minLength: 4)
      if let snapshot {
        syncIndicator(snapshot)
      }
      ChromeMenuButton(help: "More git actions") {
        [
          ChromeMenuItem("Fetch") { actions.fetch() },
          ChromeMenuItem("Fetch All Remotes") { actions.fetch(allRemotes: true) },
          ChromeMenuItem("Pull") { actions.pull() },
          ChromeMenuItem("Pull (Rebase)") { actions.pull(mode: .rebase) },
          ChromeMenuItem("Push") { actions.push() },
          ChromeMenuItem("Force Push (With Lease)…") { actions.forcePush() },
          .separator,
          ChromeMenuItem("Create Tag…") { (model.gitHost as? MainWindowController)?.createTagAtHead() },
          ChromeMenuItem("Push All Tags") { actions.pushAllTags() },
          .separator,
          ChromeMenuItem("Stage All Changes") { actions.stageAll() },
          ChromeMenuItem("Unstage All Changes") { actions.unstageAll() },
          .separator,
          ChromeMenuItem("Stash All Changes") { actions.stashAll() },
          ChromeMenuItem(
            "Move All Changes to New Task…",
            isEnabled: actions.canMoveChangesToTask && (snapshot?.changedFileCount ?? 0) > 0
          ) { actions.moveToNewTask() },
          ChromeMenuItem("Pop Latest Stash") { (model.gitHost as? MainWindowController)?.popLatestStash() },
          ChromeMenuItem("Undo Last Commit", isEnabled: snapshot?.headOid != nil) { actions.undoLastCommit() },
          .separator,
          ChromeMenuItem("Review Uncommitted Changes") {
            model.gitHost?.gitOpenReview(scope: .uncommitted, focusPath: nil)
          },
          ChromeMenuItem("Refresh") { repository.refresh() },
        ]
      } label: {
        Icon(.ellipsis, size: 14).foregroundStyle(chrome.textSecondary)
      }
    }
    .padding(.horizontal, 8)
    .frame(height: 34)
  }

  private func branchLabel(_ snapshot: RepoSnapshot?) -> String {
    guard let snapshot else { return "…" }
    if let branch = snapshot.branch { return branch }
    if let head = snapshot.headOid { return "detached at \(head.prefix(7))" }
    return "no branch"
  }

  @ViewBuilder
  private func syncIndicator(_ snapshot: RepoSnapshot) -> some View {
    if snapshot.upstream == nil, snapshot.branch != nil, !snapshot.isUnborn {
      ChromeButton(title: "Publish", icon: .upload, kind: .ghost, help: "Push and track this branch") {
        actions.push()
      }
    } else if snapshot.ahead > 0 || snapshot.behind > 0 {
      HStack(spacing: 2) {
        if snapshot.behind > 0 {
          ChromeButton(title: "\(snapshot.behind)", icon: .arrowDown, kind: .ghost, help: "Pull \(snapshot.behind) commit(s)") {
            actions.pull()
          }
        }
        if snapshot.ahead > 0 {
          ChromeButton(title: "\(snapshot.ahead)", icon: .arrowUp, kind: .ghost, help: "Push \(snapshot.ahead) commit(s)") {
            actions.push()
          }
        }
      }
    } else {
      ChromeIconButton(icon: .refreshCw, help: "Fetch", size: 22, iconSize: 12) {
        actions.fetch()
      }
    }
  }

  private func activityBar(_ activity: String) -> some View {
    HStack(spacing: 8) {
      ProgressRing(progress: nil, color: chrome.accent, size: 11, lineWidth: 1.5)
      Text(repository.activityDetail ?? activity)
        .font(ChromeFont.ui(11))
        .foregroundStyle(chrome.textSecondary)
        .lineLimit(1)
        .truncationMode(.tail)
      Spacer(minLength: 0)
    }
    .padding(.horizontal, 12)
    .frame(height: 24)
  }

  // MARK: Sections

  @ViewBuilder
  private func sections(_ snapshot: RepoSnapshot) -> some View {
    if !snapshot.conflicted.isEmpty {
      SectionHeader(title: "Conflicts", count: snapshot.conflicted.count, isExpanded: $conflictsExpanded)
      if conflictsExpanded {
        ForEach(snapshot.conflicted, id: \.path) { change in
          row(change, section: .conflicted)
        }
      }
    }
    if !snapshot.staged.isEmpty {
      SectionHeader(title: "Staged", count: snapshot.staged.count, isExpanded: $stagedExpanded) {
        ChromeIconButton(icon: .minus, help: "Unstage All", size: 20, iconSize: 12) {
          actions.unstageAll()
        }
      }
      if stagedExpanded {
        ForEach(snapshot.staged, id: \.path) { change in
          row(change, section: .staged)
        }
      }
    }
    if !snapshot.unstaged.isEmpty {
      SectionHeader(title: "Changes", count: snapshot.unstaged.count, isExpanded: $changesExpanded) {
        ChromeIconButton(icon: .undo2, help: "Discard All Changes", size: 20, iconSize: 12) {
          actions.discard(snapshot.unstaged)
        }
        ChromeIconButton(icon: .plus, help: "Stage All Changes", size: 20, iconSize: 12) {
          actions.stage(snapshot.unstaged)
        }
      }
      if changesExpanded {
        ForEach(snapshot.unstaged, id: \.path) { change in
          row(change, section: .unstaged)
        }
      }
    }
    if !snapshot.untracked.isEmpty {
      SectionHeader(
        title: snapshot.untrackedTruncated ? "Untracked (first \(snapshot.untracked.count))" : "Untracked",
        count: snapshot.untrackedTruncated ? nil : snapshot.untracked.count,
        isExpanded: $untrackedExpanded
      ) {
        ChromeIconButton(icon: .plus, help: "Stage All Untracked", size: 20, iconSize: 12) {
          actions.stage(snapshot.untracked)
        }
      }
      if untrackedExpanded {
        ForEach(snapshot.untracked, id: \.path) { change in
          row(change, section: .untracked)
        }
      }
    }
    if snapshot.stashCount > 0 {
      SectionHeader(title: "Stashes", count: snapshot.stashCount, isExpanded: $stashesExpanded)
        .onChange(of: stashesExpanded) { _, expanded in if expanded { loadStashes() } }
        .onChange(of: snapshot.stashCount) { _, _ in if stashesExpanded { loadStashes() } }
      if stashesExpanded {
        ForEach(stashes, id: \.index) { entry in
          StashRow(entry: entry, actions: actions) {
            model.gitHost?.gitOpenReview(scope: .stash(index: entry.index), focusPath: nil)
          }
        }
      }
    }
  }

  private func loadStashes() {
    let root = repository.root
    DispatchQueue.global(qos: .userInitiated).async {
      let list = GitOperations.stashList(root: root)
      DispatchQueue.main.async { stashes = list }
    }
  }

  enum Section {
    case conflicted, staged, unstaged, untracked

    /// The working copy differs from the index (what the diff view shows).
    var hasWorkingCopyDiff: Bool { self == .unstaged || self == .untracked }
  }

  /// Keyboard focus to the list, with the first row selected if none is.
  private func focusList(_ snapshot: RepoSnapshot?) {
    DispatchQueue.main.async {
      listFocused = true
      let rows = (snapshot ?? repository.snapshot).map(visibleRows) ?? []
      if selection == nil || !rows.contains(where: { $0.id == selection }), let first = rows.first {
        selection = first.id
      }
    }
  }

  /// Rows in the order shown (collapsed sections left out).
  private func visibleRows(_ snapshot: RepoSnapshot) -> [(id: String, change: FileChange, section: Section)] {
    var rows: [(id: String, change: FileChange, section: Section)] = []
    func add(_ changes: [FileChange], _ section: Section, _ expanded: Bool) {
      guard expanded else { return }
      rows += changes.map { ("\(section)-\($0.path)", $0, section) }
    }
    add(snapshot.conflicted, .conflicted, conflictsExpanded)
    add(snapshot.staged, .staged, stagedExpanded)
    add(snapshot.unstaged, .unstaged, changesExpanded)
    add(snapshot.untracked, .untracked, untrackedExpanded)
    return rows
  }

  /// ↑/↓ move, space stages or unstages (marks a conflict resolved), ⏎ opens
  /// the diff, ⌥⏎ the editable diff view, ⌘⏎ the file, ⌫ discards, Esc goes
  /// back to the work.
  private func handleKey(_ press: KeyPress, snapshot: RepoSnapshot, proxy: ScrollViewProxy) -> KeyPress.Result {
    let rows = visibleRows(snapshot)
    guard !rows.isEmpty else { return .ignored }
    let index = rows.firstIndex { $0.id == selection }
    func select(_ i: Int) {
      let row = rows[max(0, min(rows.count - 1, i))]
      selection = row.id
      proxy.scrollTo(row.id)
    }
    switch press.key {
    case .upArrow:
      select((index ?? rows.count) - 1)
    case .downArrow:
      select((index ?? -1) + 1)
    case .space:
      guard let index else { return .ignored }
      let row = rows[index]
      switch row.section {
      case .staged:
        actions.unstage([row.change])
        selection = "\(Section.unstaged)-\(row.change.path)"
      case .unstaged, .untracked:
        actions.stage([row.change])
        selection = "\(Section.staged)-\(row.change.path)"
      case .conflicted:
        actions.markResolved([row.change])
      }
    case .return:
      guard let index else { return .ignored }
      let row = rows[index]
      let absolute = (repository.root as NSString).appendingPathComponent(row.change.path)
      if press.modifiers.contains(.command) {
        model.gitHost?.gitOpenFile(absolute)
      } else if press.modifiers.contains(.option), row.section.hasWorkingCopyDiff, row.change.status != .deleted {
        model.gitHost?.gitOpenDiffEditor(absolute)
      } else {
        let scope: DiffScope =
          row.section == .staged ? .staged : row.section == .conflicted ? .uncommitted : .unstaged
        model.gitHost?.gitOpenReview(scope: scope, focusPath: row.change.path)
      }
    case .delete, .deleteForward:
      guard let index, rows[index].section != .conflicted else { return .ignored }
      let row = rows[index]
      actions.discard([row.change], includeStaged: row.section == .staged)
    case .escape:
      listFocused = false
      model.onFocusTerminal?()
    default:
      return .ignored
    }
    return .handled
  }

  private func row(_ change: FileChange, section: Section) -> some View {
    let id = "\(section)-\(change.path)"
    return ChangeRow(
      change: change, section: section, isSelected: selection == id, isFocused: listFocused,
      root: repository.root, iconCache: model.iconCache, actions: actions,
      select: {
        selection = id
        listFocused = true
      },
      openDiff: {
        let scope: DiffScope =
          section == .staged ? .staged : section == .conflicted ? .uncommitted : .unstaged
        model.gitHost?.gitOpenReview(scope: scope, focusPath: change.path)
      },
      openFile: {
        model.gitHost?.gitOpenFile((repository.root as NSString).appendingPathComponent(change.path))
      },
      openDiffEditor: {
        model.gitHost?.gitOpenDiffEditor((repository.root as NSString).appendingPathComponent(change.path))
      }
    )
    .id(id)
  }
}

// MARK: - Row

private struct ChangeRow: View {
  @Environment(\.chrome) private var chrome
  let change: FileChange
  let section: ChangesPanelContent.Section
  let isSelected: Bool
  /// The list has keyboard focus.
  let isFocused: Bool
  let root: String
  let iconCache: IconCache?
  let actions: GitActions
  let select: () -> Void
  let openDiff: () -> Void
  let openFile: () -> Void
  let openDiffEditor: () -> Void

  @State private var hovering = false

  var body: some View {
    let name = (change.path as NSString).lastPathComponent
    let dir = (change.path as NSString).deletingLastPathComponent
    HStack(spacing: 6) {
      if let icon = iconCache?.icon(filename: name, isDirectory: false, expanded: false) {
        Image(nsImage: icon).resizable().interpolation(.high).frame(width: 14, height: 14)
      }
      Text(name)
        .font(ChromeFont.ui(12))
        .foregroundStyle(change.status == .deleted ? chrome.textSecondary : chrome.text)
        .strikethrough(change.status == .deleted, color: chrome.textTertiary)
        .lineLimit(1)
      if !dir.isEmpty {
        Text(dir)
          .font(ChromeFont.ui(11))
          .foregroundStyle(chrome.textTertiary)
          .lineLimit(1)
          .truncationMode(.head)
      }
      Spacer(minLength: 4)
      if hovering {
        hoverActions
      } else {
        if let added = change.added, let removed = change.removed {
          DiffStat(added: added, removed: removed)
        } else if change.isBinary {
          Text("bin").font(ChromeFont.mono(10)).foregroundStyle(chrome.textTertiary)
        }
      }
      Text(change.status.letter)
        .font(ChromeFont.mono(11, weight: .bold))
        .foregroundStyle(statusColor)
        .frame(width: 12)
    }
    .padding(.leading, 14)
    .padding(.trailing, 10)
    .frame(height: Metrics.rowHeight)
    .rowBackground(selected: isSelected, hovered: hovering, focused: isFocused)
    .contentShape(Rectangle())
    .onHover { hovering = $0 }
    .onTapGesture(count: 2) { openFile() }
    .simultaneousGesture(TapGesture().onEnded {
      select()
      openDiff()
    })
    .contextMenu { contextMenu }
    .help(change.oldPath.map { "\($0) → \(change.path)" } ?? change.path)
    .accessibilityElement(children: .combine)
    .accessibilityLabel("\(change.path), \(change.status.rawValue)")
  }

  @ViewBuilder
  private var hoverActions: some View {
    HStack(spacing: 0) {
      ChromeIconButton(icon: .fileCode, help: "Open File", size: 20, iconSize: 12, action: openFile)
      switch section {
      case .staged:
        ChromeIconButton(icon: .minus, help: "Unstage", size: 20, iconSize: 12) {
          actions.unstage([change])
        }
      case .unstaged, .untracked:
        ChromeIconButton(icon: .undo2, help: "Discard Changes", size: 20, iconSize: 12) {
          actions.discard([change])
        }
        ChromeIconButton(icon: .plus, help: "Stage", size: 20, iconSize: 12) {
          actions.stage([change])
        }
      case .conflicted:
        ChromeIconButton(icon: .arrowUp, help: "Keep Current (HEAD) for the whole file", size: 20, iconSize: 12) {
          actions.resolve([change], takeOurs: true)
        }
        ChromeIconButton(icon: .arrowDown, help: "Take Incoming for the whole file", size: 20, iconSize: 12) {
          actions.resolve([change], takeOurs: false)
        }
        ChromeIconButton(icon: .check, help: "Mark Resolved", size: 20, iconSize: 12) {
          actions.markResolved([change])
        }
      }
    }
  }

  @ViewBuilder
  private var contextMenu: some View {
    Button("Open Changes") { openDiff() }
    if section.hasWorkingCopyDiff, change.status != .deleted {
      Button("Open in Diff Editor") { openDiffEditor() }
    }
    Button("Open File") { openFile() }
    Divider()
    switch section {
    case .staged:
      Button("Unstage") { actions.unstage([change]) }
      Button("Discard Staged and Unstaged Changes…") { actions.discard([change], includeStaged: true) }
    case .unstaged, .untracked:
      Button("Stage") { actions.stage([change]) }
      Button("Discard Changes…") { actions.discard([change]) }
    case .conflicted:
      Button("Open to Resolve") { openFile() }
      Button("Keep Current (HEAD)") { actions.resolve([change], takeOurs: true) }
      Button("Take Incoming") { actions.resolve([change], takeOurs: false) }
      Button("Mark Resolved") { actions.markResolved([change]) }
      Menu("Ask Agent to Resolve") {
        ForEach(actions.agentTargets) { agent in
          Button("\(agent.agentName) · \(agent.tabTitle)") {
            actions.askAgentToResolve([change], terminalID: agent.id)
          }
        }
        if !actions.agentTargets.isEmpty { Divider() }
        Button("Copy as Prompt") { actions.askAgentToResolve([change], terminalID: nil) }
      }
    }
    if section != .conflicted, actions.canMoveChangesToTask {
      Button("Move to New Task…") { actions.moveToNewTask([change]) }
    }
    Divider()
    Button("Copy Path") {
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(change.path, forType: .string)
    }
    Button("Reveal in Finder") {
      NSWorkspace.shared.activateFileViewerSelecting([
        URL(fileURLWithPath: root).appendingPathComponent(change.path)
      ])
    }
  }

  private var statusColor: Color {
    switch change.status {
    case .added, .untracked: return chrome.gitAdded
    case .deleted: return chrome.gitDeleted
    case .renamed: return chrome.gitRenamed
    case .conflicted: return chrome.gitConflict
    default: return chrome.gitModified
    }
  }
}

private struct StashRow: View {
  @Environment(\.chrome) private var chrome
  let entry: GitOperations.StashEntry
  let actions: GitActions
  let open: () -> Void
  @State private var hovering = false

  var body: some View {
    HStack(spacing: 6) {
      Icon(.package, size: 12).foregroundStyle(chrome.textTertiary)
      Text(entry.message)
        .font(ChromeFont.ui(12))
        .foregroundStyle(chrome.text)
        .lineLimit(1)
        .truncationMode(.tail)
      Spacer(minLength: 4)
      if hovering {
        ChromeIconButton(icon: .play, help: "Apply", size: 20, iconSize: 11) {
          actions.applyStash(entry.index, pop: false)
        }
        ChromeIconButton(icon: .arrowUp, help: "Pop", size: 20, iconSize: 11) {
          actions.applyStash(entry.index, pop: true)
        }
        ChromeIconButton(icon: .trash2, help: "Drop", size: 20, iconSize: 11) {
          actions.dropStash(entry.index)
        }
      } else {
        Text("stash@{\(entry.index)}").font(ChromeFont.mono(10)).foregroundStyle(chrome.textTertiary)
      }
    }
    .padding(.leading, 14)
    .padding(.trailing, 10)
    .frame(height: Metrics.rowHeight)
    .rowBackground(selected: false, hovered: hovering)
    .contentShape(Rectangle())
    .onHover { hovering = $0 }
    .onTapGesture(perform: open)
  }
}

// MARK: - Operation banner

private struct OperationBanner: View {
  @Environment(\.chrome) private var chrome
  let operation: RepoOperation
  let hasConflicts: Bool
  let actions: GitActions

  var body: some View {
    HStack(spacing: 6) {
      Icon(.triangleAlert, size: 13).foregroundStyle(chrome.warning)
      Text(operation.title)
        .font(ChromeFont.ui(12, weight: .semibold))
        .foregroundStyle(chrome.text)
      Spacer(minLength: 4)
      if hasConflicts {
        ChromeMenuButton(help: "Ask an agent to resolve the conflicts") {
          let agents = actions.agentTargets
          var items = agents.map { agent in
            ChromeMenuItem("Send to \(agent.agentName) · \(agent.tabTitle)") {
              actions.askAgentToResolve([], terminalID: agent.id)
            }
          }
          if !agents.isEmpty { items.append(.separator) }
          items.append(ChromeMenuItem("Copy as Prompt") { actions.askAgentToResolve([], terminalID: nil) })
          return items
        } label: {
          HStack(spacing: 4) {
            Icon(.bot, size: 12)
            Text("Ask Agent").font(ChromeFont.ui(11.5, weight: .medium))
          }
          .foregroundStyle(chrome.textSecondary)
          .padding(.horizontal, 6)
          .frame(height: 22)
        }
      }
      ChromeButton(title: "Continue", kind: .secondary) { actions.perform(.continue, on: operation) }
      if operation.canSkip {
        ChromeButton(title: "Skip", kind: .ghost) { actions.perform(.skip, on: operation) }
      }
      ChromeButton(title: "Abort", kind: .danger) { actions.perform(.abort, on: operation) }
    }
    .padding(.horizontal, 10)
    .padding(.vertical, 6)
    .background(chrome.warning.opacity(0.12))
  }
}

// MARK: - Empty state

struct EmptyState: View {
  @Environment(\.chrome) private var chrome
  let icon: LucideIcon
  let title: String
  let message: String

  var body: some View {
    VStack(spacing: 8) {
      Icon(icon, size: 22).foregroundStyle(chrome.textTertiary)
      Text(title).font(ChromeFont.ui(12.5, weight: .semibold)).foregroundStyle(chrome.textSecondary)
      Text(message)
        .font(ChromeFont.ui(11.5))
        .foregroundStyle(chrome.textTertiary)
        .multilineTextAlignment(.center)
        .frame(maxWidth: 220)
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .padding(16)
  }
}
