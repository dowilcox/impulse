import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

// Task worktrees: "New Task…" makes a branch checked out in its own folder
// beside the repository (`<repo>.worktrees/<branch>`), copies over the
// untracked files a fresh checkout lacks, and opens it as a workspace —
// optionally with an agent already running. "Archive Task…" removes the
// folder but keeps the branch, with Undo.

/// What the New Task sheet collects.
struct TaskDraft {
  var title = ""
  var base = ""
  /// Command for the first terminal ("" for none).
  var command = ""
}

@Observable
final class TaskSheetModel {
  var draft = TaskDraft()
  let repoRoot: String
  var takenBranches: Set<String> = []
  /// Files `.worktreeinclude` (or the defaults) would copy.
  var copies: [String] = []
  /// Agents found on PATH: (display name, command).
  var agents: [(name: String, command: String)] = []
  var isCreating = false
  var error: String?

  init(repoRoot: String, base: String) {
    self.repoRoot = repoRoot
    draft.base = base
  }

  var branch: String { WorktreeTasks.branchName(for: draft.title, taken: takenBranches) }
  var path: String { WorktreeTasks.worktreePath(repoRoot: repoRoot, branch: branch) }
}

struct TaskSheetView: View {
  @Environment(\.chrome) private var chrome
  @Bindable var model: TaskSheetModel
  let onCancel: () -> Void
  let onCreate: () -> Void
  @FocusState private var titleFocused: Bool

  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      Text("New Task")
        .font(ChromeFont.ui(15, weight: .semibold))
        .foregroundStyle(chrome.text)
      Text("A branch checked out in its own folder, opened as a workspace — work on it (or let an agent) without touching your current checkout.")
        .font(ChromeFont.ui(11.5))
        .foregroundStyle(chrome.textSecondary)
        .fixedSize(horizontal: false, vertical: true)

      field("Task") {
        TextField("e.g. Fix the flaky login test", text: $model.draft.title)
          .textFieldStyle(.roundedBorder)
          .focused($titleFocused)
          .onSubmit { if canCreate { onCreate() } }
      }
      field("From") {
        TextField("base branch", text: $model.draft.base)
          .textFieldStyle(.roundedBorder)
          .frame(width: 220)
      }
      field("Start") {
        Picker("", selection: $model.draft.command) {
          Text("Just a terminal").tag("")
          ForEach(model.agents, id: \.command) { agent in
            Text(agent.name).tag(agent.command)
          }
        }
        .labelsHidden()
        .frame(width: 220)
      }

      VStack(alignment: .leading, spacing: 4) {
        detail("Branch", model.draft.title.isEmpty ? "—" : model.branch)
        detail("Folder", model.draft.title.isEmpty ? "—" : TabManager.abbreviateHomePath(model.path))
        detail("Copies", model.copies.isEmpty ? "nothing (add patterns to .worktreeinclude)" : model.copies.joined(separator: ", "))
      }
      .padding(10)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(RoundedRectangle(cornerRadius: Metrics.radius).fill(chrome.raised))

      if let error = model.error {
        Text(error)
          .font(ChromeFont.ui(11.5))
          .foregroundStyle(chrome.danger)
          .fixedSize(horizontal: false, vertical: true)
      }

      HStack {
        Spacer()
        ChromeButton(title: "Cancel", kind: .secondary) { onCancel() }
          .keyboardShortcut(.cancelAction)
        ChromeButton(
          title: model.isCreating ? "Creating…" : "Create Task", icon: .gitBranchPlus, kind: .primary
        ) { onCreate() }
        .disabled(!canCreate)
        .keyboardShortcut(.defaultAction)
      }
    }
    .padding(20)
    .frame(width: 480)
    .background(chrome.overlay)
    .onAppear { titleFocused = true }
  }

  private var canCreate: Bool {
    !model.isCreating && !model.draft.title.trimmingCharacters(in: .whitespaces).isEmpty
  }

  private func field<Content: View>(_ label: String, @ViewBuilder content: () -> Content) -> some View {
    HStack(spacing: 10) {
      Text(label)
        .font(ChromeFont.ui(12))
        .foregroundStyle(chrome.textSecondary)
        .frame(width: 44, alignment: .trailing)
      content()
    }
  }

  private func detail(_ label: String, _ value: String) -> some View {
    HStack(alignment: .firstTextBaseline, spacing: 8) {
      Text(label)
        .font(ChromeFont.ui(11))
        .foregroundStyle(chrome.textTertiary)
        .frame(width: 44, alignment: .trailing)
      Text(value)
        .font(ChromeFont.mono(11))
        .foregroundStyle(chrome.textSecondary)
        .lineLimit(2)
        .truncationMode(.middle)
    }
  }
}

extension MainWindowController {
  /// The repository tasks are made from: the given workspace's, else the
  /// active workspace's, else the window's.
  private func taskRepository(from workspaceID: UUID?) -> GitRepositoryState? {
    workspaceID.flatMap { tabManager.workspace($0)?.repository } ?? tabManager.activeWorkspace.repository
      ?? windowModel.repository
  }

  /// "New Task…": ask for a title and what to start, then create it (from
  /// `workspaceID`'s repository, or the active workspace's).
  func presentNewTaskSheet(from workspaceID: UUID? = nil) {
    guard let window, let repository = taskRepository(from: workspaceID) else {
      toasts.show(Toast(kind: .info, message: "Open a folder in a git repository to start a task."))
      return
    }
    // Tasks branch from the main checkout, even when started from a task.
    let root = repository.root
    let model = TaskSheetModel(repoRoot: root, base: repository.snapshot?.branch ?? "HEAD")
    DispatchQueue.global(qos: .userInitiated).async {
      let taken = Set(GitOperations.branches(root: root).local)
      let include = try? String(
        contentsOfFile: (root as NSString).appendingPathComponent(".worktreeinclude"), encoding: .utf8)
      let projectCopies = ProjectConfig.load(root: root).flatMap { try? $0.config.get().worktreeCopy } ?? []
      let copies = WorktreeTasks.matchingFiles(
        patterns: WorktreeTasks.includePatterns(fromFile: include) + projectCopies, root: root)
      let agents = KnownAgents.builtIn.compactMap { kind -> (name: String, command: String)? in
        guard let name = kind.names.first, LoginShell.which(name) != nil else { return nil }
        return (kind.displayName, name)
      }
      DispatchQueue.main.async {
        model.takenBranches = taken
        model.copies = copies
        model.agents = agents
      }
    }

    let sheet = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 480, height: 360), styleMask: [.titled],
      backing: .buffered, defer: true)
    let palette = windowModel.palette
    let host = NSHostingView(
      rootView: TaskSheetView(
        model: model,
        onCancel: { [weak window, weak sheet] in
          if let sheet { window?.endSheet(sheet) }
        },
        onCreate: { [weak self, weak window, weak sheet] in
          self?.createTask(model) {
            if let sheet { window?.endSheet(sheet) }
          }
        }
      ).environment(\.chrome, palette))
    host.sizingOptions = [.preferredContentSize]
    sheet.contentView = host
    window.beginSheet(sheet)
  }

  /// Snapshot runs: create a task without the sheet.
  func debugCreateTask(title: String, command: String) {
    guard let repository = taskRepository(from: nil) else {
      NSLog("DebugSnapshot: no repository for a task")
      return
    }
    let model = TaskSheetModel(repoRoot: repository.root, base: repository.snapshot?.branch ?? "HEAD")
    model.draft.title = title
    model.draft.command = command
    let include = try? String(
      contentsOfFile: (repository.root as NSString).appendingPathComponent(".worktreeinclude"),
      encoding: .utf8)
    model.copies = WorktreeTasks.matchingFiles(
      patterns: WorktreeTasks.includePatterns(fromFile: include), root: repository.root)
    model.takenBranches = Set(GitOperations.branches(root: repository.root).local)
    createTask(model) {}
  }

  private func createTask(_ model: TaskSheetModel, done: @escaping () -> Void) {
    model.isCreating = true
    model.error = nil
    let root = model.repoRoot
    let branch = model.branch
    let path = model.path
    let base = model.draft.base.trimmingCharacters(in: .whitespaces)
    let copies = model.copies
    let command = model.draft.command
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      var failure: String?
      if FileManager.default.fileExists(atPath: path) {
        failure = "\(TabManager.abbreviateHomePath(path)) already exists."
      } else {
        try? FileManager.default.createDirectory(
          atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        if case .failure(let error) = GitOperations.addWorktree(
          path: path, branch: branch, newBranch: true, base: base.isEmpty ? nil : base, root: root)
        {
          failure = error.message
        }
      }
      if failure == nil {
        Self.copyFiles(copies, from: root, to: path)
      }
      DispatchQueue.main.async {
        model.isCreating = false
        if let failure {
          model.error = failure
          if DebugSnapshot.isActive { NSLog("DebugSnapshot: task failed: \(failure)") }
          return
        }
        done()
        guard let self else { return }
        // The project's setup script (once trusted) runs before the agent.
        self.trustProjectConfig(root: path) { [weak self] config in
          let first = [config?.setupScript, command.isEmpty ? nil : command].compactMap { $0 }
          self?.tabManager.openWorkspace(
            folder: path, initialCommand: first.isEmpty ? nil : first.joined(separator: " && "))
          self?.toasts.show(Toast(kind: .success, message: "Started task \(branch)."))
        }
      }
    }
  }

  /// A pull request as a task: a worktree beside the repository with the
  /// PR checked out by gh (which also sets up fork remotes), opened as a
  /// workspace.
  func checkOutPullRequestAsTask(_ pullRequest: PullRequestSummary) {
    guard let repository = taskRepository(from: nil) else {
      toasts.show(Toast(kind: .info, message: "Open a folder in a git repository first."))
      return
    }
    let root = repository.root
    toasts.show(Toast(kind: .info, message: "Checking out #\(pullRequest.number)…"))
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let taken = Set(GitOperations.branches(root: root).local)
      let branch = pullRequest.localBranch(taken: taken)
      let path = WorktreeTasks.worktreePath(repoRoot: root, branch: branch)
      var failure: String?
      if FileManager.default.fileExists(atPath: path) {
        failure = "\(TabManager.abbreviateHomePath(path)) already exists."
      } else {
        try? FileManager.default.createDirectory(
          atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        // Start detached at HEAD; gh then makes the PR's branch.
        if case .failure(let error) = GitOperations.addWorktree(
          path: path, branch: "HEAD", newBranch: false, root: root)
        {
          failure = error.message
        }
      }
      DispatchQueue.main.async {
        guard let self else { return }
        if let failure {
          self.toasts.show(Toast(kind: .warning, message: failure))
          return
        }
        PullRequestMonitor.shared.checkout(number: pullRequest.number, branch: branch, in: path) {
          [weak self] result in
          guard let self else { return }
          if case .failure(let message) = result {
            // Leave nothing half-made behind.
            _ = GitOperations.removeWorktree(path: path, force: true, root: root)
            self.toasts.show(Toast(kind: .warning, message: "gh: \(message)", lifetime: 12))
            return
          }
          let include = try? String(
            contentsOfFile: (root as NSString).appendingPathComponent(".worktreeinclude"), encoding: .utf8)
          Self.copyFiles(
            WorktreeTasks.matchingFiles(patterns: WorktreeTasks.includePatterns(fromFile: include), root: root),
            from: root, to: path)
          self.tabManager.openWorkspace(folder: path)
          self.toasts.show(Toast(kind: .success, message: "Checked out #\(pullRequest.number) as \(branch)."))
        }
      }
    }
  }

  private static func copyFiles(_ files: [String], from root: String, to destination: String) {
    let fm = FileManager.default
    for relative in files {
      let target = (destination as NSString).appendingPathComponent(relative)
      guard !fm.fileExists(atPath: target) else { continue }
      try? fm.createDirectory(
        atPath: (target as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      try? fm.copyItem(atPath: (root as NSString).appendingPathComponent(relative), toPath: target)
    }
  }

  /// Whether a workspace is a linked worktree (a task) of some repository.
  func isTaskWorkspace(_ id: UUID) -> Bool {
    tabManager.workspace(id)?.isTask ?? false
  }

  /// "Archive Task…": close the workspace, snapshot any uncommitted work,
  /// remove the worktree folder, keep the branch. Undo brings it back.
  func archiveTask(_ id: UUID) {
    guard let workspace = tabManager.workspace(id), isTaskWorkspace(id) else {
      toasts.show(Toast(kind: .info, message: "Only task worktrees can be archived."))
      return
    }
    let root = workspace.root
    let snapshot = workspace.repository?.snapshot
    let branch = snapshot?.branch ?? (root as NSString).lastPathComponent
    let dirty = snapshot?.changedFileCount ?? 0
    var message = "The folder \(TabManager.abbreviateHomePath(root)) is removed; branch \(branch) is kept."
    if dirty > 0 {
      message += " \(dirty) uncommitted file\(dirty == 1 ? "" : "s") will be saved in a snapshot that Undo restores."
    }
    if let snapshot, snapshot.ahead > 0 || snapshot.upstream == nil {
      message += " The branch has commits that aren't pushed."
    }
    gitConfirm(title: "Archive \(workspace.name)?", message: message, confirmTitle: "Archive", destructive: true) {
      [weak self] confirmed in
      guard let self, confirmed else { return }
      // The project's archive script (once trusted) runs before removal.
      self.trustProjectConfig(root: root) { [weak self] config in
        guard let self else { return }
        self.tabManager.ensureScratchWorkspace()
        // Close the workspace first (it confirms unsaved files and running
        // processes), then remove the folder once it's gone.
        self.requestCloseWorkspace(id) { [weak self] in
          self?.removeTaskWorktree(root: root, branch: branch, dirty: dirty > 0, archiveScript: config?.archiveScript)
        }
      }
    }
  }

  private func removeTaskWorktree(root: String, branch: String, dirty: Bool, archiveScript: String? = nil) {
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      if let archiveScript, !Self.runScript(archiveScript, in: root) {
        DispatchQueue.main.async {
          self?.toasts.show(Toast(kind: .warning, message: "The archive script failed; archiving anyway."))
        }
      }
      // Run git from the main checkout: the worktree is about to vanish.
      let mainRoot =
        GitClient.commonGitDirectory(forPath: root).map { ($0 as NSString).deletingLastPathComponent }
        ?? root
      // Look again: closing the workspace may have just saved files
      // ("Save & Close"), which then belong in the snapshot too.
      let dirty = dirty || (GitClient.snapshot(forPath: root)?.changedFileCount ?? 0) > 0
      var saved: SafetySnapshot?
      if dirty, case .success(let snapshot) = SafetySnapshots.create(reason: "archive \(branch)", root: root) {
        saved = snapshot
      }
      let result = GitOperations.removeWorktree(path: root, force: dirty, root: mainRoot)
      DispatchQueue.main.async {
        guard let self else { return }
        if case .failure(let error) = result {
          self.presentGitError(error, title: "Couldn't archive the task")
          return
        }
        self.toasts.show(
          Toast(
            kind: .success, message: "Archived \(branch). The branch is kept.", actionTitle: "Undo",
            action: { [weak self] in self?.restoreTask(root: root, branch: branch, snapshot: saved, mainRoot: mainRoot) }))
      }
    }
  }

  private func restoreTask(root: String, branch: String, snapshot: SafetySnapshot?, mainRoot: String) {
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let added = GitOperations.addWorktree(path: root, branch: branch, newBranch: false, root: mainRoot)
      if case .success = added, let snapshot {
        _ = SafetySnapshots.restore(snapshot, root: root)
      }
      DispatchQueue.main.async {
        guard let self else { return }
        if case .failure(let error) = added {
          self.presentGitError(error, title: "Couldn't restore the task")
          return
        }
        self.tabManager.openWorkspace(folder: root)
      }
    }
  }
}
