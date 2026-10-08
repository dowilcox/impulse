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
  /// Taken branches, copies and agents are still being looked up.
  var isLoading = true
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
        detail(
          "Copies",
          model.isLoading
            ? "…"
            : model.copies.isEmpty ? "nothing (add patterns to .worktreeinclude)" : model.copies.joined(separator: ", "))
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

  /// Not before the lookups finish: the branch name must be checked against
  /// the taken ones, and the copies known.
  private var canCreate: Bool {
    !model.isLoading && !model.isCreating && !model.draft.title.trimmingCharacters(in: .whitespaces).isEmpty
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
  /// `workspaceID`'s repository, or the active workspace's). `title`,
  /// `command` and `base` prefill the sheet.
  func presentNewTaskSheet(
    from workspaceID: UUID? = nil, title: String = "", command: String = "", base: String? = nil
  ) {
    guard let window, let repository = taskRepository(from: workspaceID) else {
      toasts.show(Toast(kind: .info, message: "Open a folder in a git repository to start a task."))
      return
    }
    // Tasks branch from the main checkout, even when started from a task:
    // their folders go beside it, and its branch is the default base.
    let root = Self.mainCheckoutRoot(of: repository.root)
    let fromTask = root != repository.root
    let model = TaskSheetModel(
      repoRoot: root, base: base ?? (fromTask ? "" : repository.snapshot?.branch ?? "HEAD"))
    model.draft.title = title
    model.draft.command = command
    DispatchQueue.global(qos: .userInitiated).async {
      let mainBranch = fromTask && base == nil ? GitOperations.currentBranch(root: root) ?? "HEAD" : nil
      let taken = Set(GitOperations.branches(root: root).local)
      let copies = Self.taskCopies(root: root)
      let agents = KnownAgents.builtIn.compactMap { kind -> (name: String, command: String)? in
        guard let name = kind.names.first, LoginShell.which(name) != nil else { return nil }
        return (kind.displayName, name)
      }
      DispatchQueue.main.async {
        if let mainBranch, model.draft.base.isEmpty { model.draft.base = mainBranch }
        model.takenBranches = taken
        model.copies = copies
        model.agents = agents
        model.isLoading = false
      }
    }

    let palette = windowModel.palette
    let sheet = NSWindow.themedSheet(palette: palette)
    window.beginThemedSheet(
      sheet, palette: palette,
      content: TaskSheetView(
        model: model,
        onCancel: { [weak window, weak sheet] in
          if let sheet { window?.endSheet(sheet) }
        },
        onCreate: { [weak self, weak window, weak sheet] in
          self?.createTask(model) {
            if let sheet { window?.endSheet(sheet) }
          }
        }
      ))
  }

  /// Snapshot runs: create a task without the sheet.
  func debugCreateTask(title: String, command: String) {
    guard let repository = taskRepository(from: nil) else {
      NSLog("DebugSnapshot: no repository for a task")
      return
    }
    let root = Self.mainCheckoutRoot(of: repository.root)
    let base =
      root == repository.root ? repository.snapshot?.branch : GitOperations.currentBranch(root: root)
    let model = TaskSheetModel(repoRoot: root, base: base ?? "HEAD")
    model.draft.title = title
    model.draft.command = command
    model.copies = Self.taskCopies(root: root)
    model.takenBranches = Set(GitOperations.branches(root: root).local)
    model.isLoading = false
    createTask(model) {}
  }

  /// The main checkout of the repository `root` is in: for a task (a
  /// linked worktree), the folder its repository's `.git` is in; otherwise
  /// `root` itself.
  static func mainCheckoutRoot(of root: String) -> String {
    guard let gitDirectory = GitClient.gitDirectory(forPath: root),
      let common = GitClient.commonGitDirectory(forPath: root), gitDirectory != common,
      (common as NSString).lastPathComponent == ".git"
    else { return root }
    return (common as NSString).deletingLastPathComponent
  }

  /// The untracked files a new task gets from `root`: `.worktreeinclude`'s
  /// patterns (or the defaults) plus the project settings' `[worktrees] copy`.
  static func taskCopies(root: String) -> [String] {
    let include = try? String(
      contentsOfFile: (root as NSString).appendingPathComponent(".worktreeinclude"), encoding: .utf8)
    let projectCopies = loadProjectConfig(root: root).flatMap { try? $0.config.get().worktreeCopy } ?? []
    return WorktreeTasks.matchingFiles(
      patterns: WorktreeTasks.includePatterns(fromFile: include) + projectCopies, root: root)
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
        TaskRegistryStore.recordCreated(path: path, branch: branch, base: base, root: root)
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
        // The same repository: a task folder is as trusted as it is.
        if Trust.shared.isTrusted(root) { Trust.shared.trust(path) }
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
  /// PR checked out by gh (which also sets up fork remotes), set up and
  /// opened like New Task… (copies, then the setup script if the user says
  /// so for this pull request).
  func checkOutPullRequestAsTask(_ pullRequest: PullRequestSummary) {
    guard let repository = taskRepository(from: nil) else {
      toasts.show(Toast(kind: .info, message: "Open a folder in a git repository first."))
      return
    }
    let root = Self.mainCheckoutRoot(of: repository.root)
    toasts.show(Toast(kind: .info, message: "Checking out #\(pullRequest.number)…"))
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let taken = Set(GitOperations.branches(root: root).local)
      let copies = Self.taskCopies(root: root)
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
          Self.copyFiles(copies, from: root, to: path)
          // The setup script comes from the pull request's own checkout.
          self.confirmPullRequestSetup(pullRequest, root: path) { [weak self] setup in
            self?.tabManager.openWorkspace(folder: path, initialCommand: setup)
            self?.toasts.show(Toast(kind: .success, message: "Checked out #\(pullRequest.number) as \(branch)."))
          }
        }
      }
    }
  }

  /// A pull request's setup script, once the user says to run it. Always
  /// asked, whatever the repository's trusted project.toml: the pull
  /// request can change what the script runs (package.json scripts, say)
  /// without touching that file. The answer isn't remembered. Nil when
  /// there's no script or the user declines.
  private func confirmPullRequestSetup(
    _ pullRequest: PullRequestSummary, root: String, completion: @escaping (String?) -> Void
  ) {
    guard let setup = projectConfig(root: root)?.setupScript else { return completion(nil) }
    let author = pullRequest.author.isEmpty ? "" : " by \(pullRequest.author)"
    let title = pullRequest.title.isEmpty ? "" : " (“\(pullRequest.title)”)"
    let alert = NSAlert()
    alert.alertStyle = .warning
    alert.messageText = "Run the setup script of #\(pullRequest.number)?"
    alert.informativeText = """
      Pull request #\(pullRequest.number)\(author)\(title) sets up its task with this script \
      from \(ProjectConfig.relativePath):

        \(setup)

      It runs in the pull request's checkout, where the pull request decides what these commands \
      do (a changed package.json script, for example). Run it only if you trust the pull \
      request's changes. The task opens either way.
      """
    alert.addButton(withTitle: "Run Setup Script")
    alert.addButton(withTitle: "Don't Run")
    presentWhenNoSheet { window in
      alert.beginSheetModal(for: window) { response in
        completion(response == .alertFirstButtonReturn ? setup : nil)
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
  /// remove the worktree folder, keep the branch. Undo brings it back. A
  /// second request while one is under way is ignored.
  func archiveTask(_ id: UUID) {
    // A workspace that's gone was most likely archived by an earlier request.
    guard !archivingTasks.contains(id), let workspace = tabManager.workspace(id) else { return }
    guard workspace.isTask else {
      toasts.show(Toast(kind: .info, message: "Only task worktrees can be archived."))
      return
    }
    archivingTasks.insert(id)
    let root = workspace.root
    let name = workspace.name
    // Ignored files (.env, node_modules) go with the folder for good: the
    // snapshot only holds what git would track. Say so before asking.
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let ignored = GitOperations.ignoredEntries(root: root)
      DispatchQueue.main.async {
        guard let self else { return }
        guard let workspace = self.tabManager.workspace(id) else {
          self.archivingTasks.remove(id)
          return
        }
        self.confirmArchiveTask(id, root: root, name: name, snapshot: workspace.repository?.snapshot, ignored: ignored)
      }
    }
  }

  /// Whether the task workspace `id` is still open on `root`, and its folder
  /// still there (it can go while a confirmation waits).
  private func isArchivableTask(_ id: UUID, root: String) -> Bool {
    guard let workspace = tabManager.workspace(id), workspace.isTask, workspace.root == root else { return false }
    var isDirectory: ObjCBool = false
    return FileManager.default.fileExists(atPath: root, isDirectory: &isDirectory) && isDirectory.boolValue
  }

  private func confirmArchiveTask(_ id: UUID, root: String, name: String, snapshot: RepoSnapshot?, ignored: [String]) {
    let branch = snapshot?.branch ?? (root as NSString).lastPathComponent
    let dirty = snapshot?.changedFileCount ?? 0
    var message = "The folder \(TabManager.abbreviateHomePath(root)) is removed; branch \(branch) is kept."
    if dirty > 0 {
      message += " \(dirty) uncommitted file\(dirty == 1 ? "" : "s") will be saved in a snapshot that Undo restores."
    }
    if !ignored.isEmpty {
      let shown = ignored.prefix(3).joined(separator: ", ") + (ignored.count > 3 ? ", …" : "")
      message += " Ignored files in it (\(shown)) are deleted, and Undo can't bring them back."
    }
    if let snapshot, snapshot.ahead > 0 || snapshot.upstream == nil {
      message += " The branch has commits that aren't pushed."
    }
    gitConfirm(title: "Archive \(name)?", message: message, confirmTitle: "Archive", destructive: true) {
      [weak self] confirmed in
      guard let self else { return }
      guard confirmed, self.stillArchivable(id, root: root, name: name) else {
        self.archivingTasks.remove(id)
        return
      }
      // The project's archive script (once trusted) runs before removal.
      self.trustProjectConfig(root: root) { [weak self] config in
        guard let self else { return }
        guard self.stillArchivable(id, root: root, name: name) else {
          self.archivingTasks.remove(id)
          return
        }
        self.tabManager.ensureScratchWorkspace()
        // Close the workspace first (it confirms unsaved files and running
        // processes), then remove the folder once it's gone.
        self.requestCloseWorkspace(
          id, recordForUndo: false,
          then: { [weak self] in
            self?.archivingTasks.remove(id)
            self?.removeTaskWorktree(root: root, branch: branch, dirty: dirty > 0, archiveScript: config?.archiveScript)
          },
          cancelled: { [weak self] in self?.archivingTasks.remove(id) })
      }
    }
  }

  /// `isArchivableTask`, saying so when the workspace is still open but its
  /// folder went away (a request that lost to another just stops).
  private func stillArchivable(_ id: UUID, root: String, name: String) -> Bool {
    if isArchivableTask(id, root: root) { return true }
    if tabManager.workspace(id) != nil {
      toasts.show(Toast(kind: .info, message: "\(name) wasn't archived: its folder is gone."))
    }
    return false
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
      if dirty {
        switch SafetySnapshots.create(reason: "archive \(branch)", root: root) {
        case .success(let snapshot):
          saved = snapshot
        case .failure(let error):
          // The dialog promised the uncommitted files would be saved.
          DispatchQueue.main.async {
            self?.presentGitError(
              .invalid("The task wasn't archived: its uncommitted files couldn't be saved first. \(error.message)"),
              title: "Couldn't archive the task")
          }
          return
        }
      }
      let result = GitOperations.removeWorktree(path: root, force: dirty, root: mainRoot)
      let record: TaskRecord? =
        if case .success = result { TaskRegistryStore.remove(path: root, root: mainRoot) } else { nil }
      DispatchQueue.main.async {
        guard let self else { return }
        if case .failure(let error) = result {
          self.presentGitError(error, title: "Couldn't archive the task")
          return
        }
        self.toasts.show(
          Toast(
            kind: .success, message: "Archived \(branch). The branch is kept.", actionTitle: "Undo",
            action: { [weak self] in
              self?.restoreTask(root: root, branch: branch, snapshot: saved, record: record, mainRoot: mainRoot)
            },
            lifetime: 15))
      }
    }
  }

  private func restoreTask(
    root: String, branch: String, snapshot: SafetySnapshot?, record: TaskRecord?, mainRoot: String
  ) {
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let added = GitOperations.addWorktree(path: root, branch: branch, newBranch: false, root: mainRoot)
      if case .success = added, let snapshot {
        _ = SafetySnapshots.restore(snapshot, root: root)
      }
      if case .success = added, let record {
        TaskRegistryStore.restore(record, root: mainRoot)
      }
      DispatchQueue.main.async {
        guard let self else { return }
        if case .failure(let error) = added {
          self.presentGitError(error, title: "Couldn't restore the task")
          return
        }
        if Trust.shared.isTrusted(mainRoot) { Trust.shared.trust(root) }
        self.tabManager.openWorkspace(folder: root)
      }
    }
  }
}
