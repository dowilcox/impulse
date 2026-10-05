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
  @State private var showBranchPicker = false

  var body: some View {
    let snapshot = repository.snapshot
    VStack(spacing: 0) {
      header(snapshot)
      if let operation = snapshot?.operation {
        OperationBanner(operation: operation, actions: actions)
      }
      if let activity = repository.activity {
        activityBar(activity)
      }
      Hairline()
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 0) {
          if let snapshot {
            sections(snapshot)
          }
        }
        .padding(.bottom, 8)
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
        showBranchPicker.toggle()
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
      .help("Switch branch")
      .popover(isPresented: $showBranchPicker, arrowEdge: .bottom) {
        BranchPickerView(
          currentBranch: snapshot?.branch ?? "", cwd: repository.root, accent: chrome.accent
        ) { selected in
          showBranchPicker = false
          guard selected != snapshot?.branch else { return }
          actions.switchBranch(selected)
        }
      }
      Spacer(minLength: 4)
      if let snapshot {
        syncIndicator(snapshot)
      }
      ChromeMenuButton(help: "More git actions") {
        [
          ChromeMenuItem("Fetch") { actions.fetch() },
          ChromeMenuItem("Pull") { actions.pull() },
          ChromeMenuItem("Pull (Rebase)") { actions.pull(rebase: true) },
          ChromeMenuItem("Push") { actions.push() },
          .separator,
          ChromeMenuItem("Stage All Changes") { actions.stageAll() },
          ChromeMenuItem("Unstage All Changes") { actions.unstageAll() },
          .separator,
          ChromeMenuItem("Stash All Changes") { actions.stashAll() },
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

  enum Section { case conflicted, staged, unstaged, untracked }

  private func row(_ change: FileChange, section: Section) -> some View {
    let id = "\(section)-\(change.path)"
    return ChangeRow(
      change: change, section: section, isSelected: selection == id,
      root: repository.root, iconCache: model.iconCache, actions: actions,
      select: { selection = id },
      openDiff: {
        let scope: DiffScope =
          section == .staged ? .staged : section == .conflicted ? .uncommitted : .unstaged
        model.gitHost?.gitOpenReview(scope: scope, focusPath: change.path)
      },
      openFile: {
        model.gitHost?.gitOpenFile((repository.root as NSString).appendingPathComponent(change.path))
      })
  }
}

// MARK: - Row

private struct ChangeRow: View {
  @Environment(\.chrome) private var chrome
  let change: FileChange
  let section: ChangesPanelContent.Section
  let isSelected: Bool
  let root: String
  let iconCache: IconCache?
  let actions: GitActions
  let select: () -> Void
  let openDiff: () -> Void
  let openFile: () -> Void

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
    .rowBackground(selected: isSelected, hovered: hovering)
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
        ChromeIconButton(icon: .check, help: "Mark Resolved", size: 20, iconSize: 12) {
          actions.markResolved([change])
        }
      }
    }
  }

  @ViewBuilder
  private var contextMenu: some View {
    Button("Open Changes") { openDiff() }
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
      Button("Mark Resolved") { actions.markResolved([change]) }
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
  let actions: GitActions

  var body: some View {
    HStack(spacing: 6) {
      Icon(.triangleAlert, size: 13).foregroundStyle(chrome.warning)
      Text(operation.title)
        .font(ChromeFont.ui(12, weight: .semibold))
        .foregroundStyle(chrome.text)
      Spacer(minLength: 4)
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
