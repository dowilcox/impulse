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
  /// Under From: unpushed commits the default base leaves out.
  var baseNote: String?
  /// The remote From's default comes from (`origin`), when there is one.
  var remote: String?
  /// Fetching `remote`, or what went wrong.
  var isFetching = false
  var fetchError: String?
  /// The repository is trusted, so the sheet fetches on its own; otherwise
  /// From has a Fetch button.
  var fetchesOnItsOwn = false
  /// From as the sheet filled it in; once the user types, it's left alone.
  var defaultBase: String?

  init(repoRoot: String, base: String) {
    self.repoRoot = repoRoot
    draft.base = base
  }

  /// The project's settings and the slot the task would get, for the
  /// preview of its own values.
  var settings: ProjectConfig?
  var slot: Int?
  /// Folders that will be cloned in (the ones the main checkout has).
  var clones: [String] = []
  /// Lock files in the repository, when it has no task settings yet.
  var lockFiles: [String] = []

  /// The project has nothing set up for tasks yet.
  var needsSetup: Bool {
    guard let settings else { return true }
    return settings.setupScript == nil && settings.worktreeClone.isEmpty && !settings.hasTaskValues
      && settings.databaseFolder == nil && !settings.composeOverride
  }

  /// "APP_PORT 8100 · VITE_PORT 5273 · .env: APP_URL, DB_DATABASE".
  var valuesPreview: String? {
    guard let settings, settings.hasTaskValues, let slot else { return nil }
    let ports = TaskEnvironment.ports(settings.ports, slot: slot, offset: settings.portOffset)
    var parts = ports.keys.sorted().map { "\($0) \(ports[$0]!)" }
    if !settings.worktreeEnv.isEmpty {
      parts.append("\(settings.envFile): " + settings.worktreeEnv.keys.sorted().joined(separator: ", "))
    }
    return parts.joined(separator: " · ")
  }

  /// From still holds what the sheet put there (or nothing), so a better
  /// default may replace it.
  var canReplaceBase: Bool { defaultBase.map { draft.base == $0 } ?? draft.base.isEmpty }

  var branch: String { WorktreeTasks.branchName(for: draft.title, taken: takenBranches) }
  var path: String { WorktreeTasks.worktreePath(repoRoot: repoRoot, branch: branch) }
}

struct TaskSheetView: View {
  @Environment(\.chrome) private var chrome
  @Bindable var model: TaskSheetModel
  let onCancel: () -> Void
  let onCreate: () -> Void
  let onFetch: () -> Void
  let onSetUp: () -> Void
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
        if model.isFetching {
          ProgressView().controlSize(.small)
          Text("Fetching \(model.remote ?? "")…")
            .font(ChromeFont.ui(11))
            .foregroundStyle(chrome.textTertiary)
        } else if !model.fetchesOnItsOwn, model.remote != nil {
          ChromeButton(title: "Fetch", kind: .secondary) { onFetch() }
            .help("Fetch \(model.remote ?? "the remote") so From starts from its latest commits")
        }
      }
      if let note = model.fetchError ?? model.baseNote {
        Text(note)
          .font(ChromeFont.ui(11))
          .foregroundStyle(chrome.textTertiary)
          .fixedSize(horizontal: false, vertical: true)
          .padding(.leading, 54)
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
        if !model.clones.isEmpty {
          detail("Clones", model.clones.map { $0 + "/" }.joined(separator: ", "))
        }
        if let values = model.valuesPreview {
          detail("Values", values)
        }
      }
      .padding(10)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(RoundedRectangle(cornerRadius: Metrics.radius).fill(chrome.raised))

      if !model.isLoading, model.needsSetup {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
          Text(
            model.lockFiles.isEmpty
              ? "No setup for tasks yet."
              : "No setup script. This repository has \(model.lockFiles.joined(separator: " and "))."
          )
          .font(ChromeFont.ui(11))
          .foregroundStyle(chrome.textTertiary)
          ChromeButton(title: "Set Up This Project for Tasks…", kind: .ghost) { onSetUp() }
        }
      }

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
    // their folders go beside it, and what its branch tracks is the
    // default base.
    let root = Self.mainCheckoutRoot(of: repository.root)
    let known = root == repository.root ? repository.snapshot : nil
    let initial = known.map { WorktreeTasks.defaultBase(branch: $0.branch, upstream: $0.upstream, ahead: $0.ahead) }
    let model = TaskSheetModel(repoRoot: root, base: base ?? initial?.base ?? "")
    if base == nil {
      model.defaultBase = initial?.base
      model.baseNote = initial?.note
    }
    model.draft.title = title
    model.draft.command = command
    model.fetchesOnItsOwn = Trust.shared.isTrusted(root)
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let main = known ?? GitClient.snapshot(forPath: root)
      let computed = main.map { WorktreeTasks.defaultBase(branch: $0.branch, upstream: $0.upstream, ahead: $0.ahead) }
      let remote = main?.upstream.flatMap { upstream in
        GitOperations.remotes(root: root).first { upstream.hasPrefix("\($0)/") }
      }
      let taken = Set(GitOperations.branches(root: root).local)
      let copies = Self.taskCopies(root: root)
      let agents = KnownAgents.builtIn.compactMap { kind -> (name: String, command: String)? in
        guard let name = kind.names.first, LoginShell.which(name) != nil else { return nil }
        return (kind.displayName, name)
      }
      let settings = try? Self.loadProjectConfig(root: root)?.config.get()
      let slot =
        settings?.hasTaskValues == true
        ? TaskRegistryStore.registry(root: root)?.nextSlot { TaskRegistryStore.portsAreFree(settings, slot: $0) }
        : nil
      let clones = ((settings?.worktreeClone ?? []) + [settings?.databaseFolder].compactMap { $0 }).filter {
        FileManager.default.fileExists(atPath: (root as NSString).appendingPathComponent($0))
      }
      let lockFiles = ProjectDetector.installCommands.map(\.lockFile).filter {
        FileManager.default.fileExists(atPath: (root as NSString).appendingPathComponent($0))
      }
      DispatchQueue.main.async {
        model.settings = settings
        model.slot = slot
        model.clones = clones
        model.lockFiles = lockFiles
        if base == nil, let computed, model.canReplaceBase {
          model.draft.base = computed.base
          model.defaultBase = computed.base
          model.baseNote = computed.note
        }
        model.remote = remote
        model.takenBranches = taken
        model.copies = copies
        model.agents = agents
        model.isLoading = false
        // Start from what's on the remote now; creating doesn't wait.
        if model.fetchesOnItsOwn { self?.fetchForTaskSheet(model) }
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
        },
        onFetch: { [weak self] in self?.fetchForTaskSheet(model) },
        onSetUp: { [weak self, weak window, weak sheet] in
          if let sheet { window?.endSheet(sheet) }
          self?.openProjectSetup(from: workspaceID)
        }
      ))
  }

  /// Fetch the remote From's default comes from, quietly (no credential
  /// prompts). A failure shows under From; the task can be created anyway.
  private func fetchForTaskSheet(_ model: TaskSheetModel) {
    guard !model.isFetching, let remote = model.remote else { return }
    model.isFetching = true
    model.fetchError = nil
    let root = model.repoRoot
    DispatchQueue.global(qos: .userInitiated).async {
      let result = GitOperations.fetch(root: root, timeout: 60)
      DispatchQueue.main.async {
        model.isFetching = false
        if case .failure(let error) = result {
          let reason = error.message.split(whereSeparator: \.isNewline).first.map(String.init) ?? ""
          model.fetchError = "Couldn't fetch \(remote)\(reason.isEmpty ? "" : ": \(reason)")"
        }
      }
    }
  }

  /// Snapshot runs: create a task without the sheet.
  func debugCreateTask(title: String, command: String) {
    guard let repository = taskRepository(from: nil) else {
      NSLog("DebugSnapshot: no repository for a task")
      return
    }
    let root = Self.mainCheckoutRoot(of: repository.root)
    let main = root == repository.root ? repository.snapshot : GitClient.snapshot(forPath: root)
    let base = main.map { WorktreeTasks.defaultBase(branch: $0.branch, upstream: $0.upstream, ahead: $0.ahead).base }
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
      var record: TaskRecord?
      if failure == nil {
        // Read (not yet trusted) only to pick a slot whose ports are free.
        let settings = try? Self.loadProjectConfig(root: path)?.config.get()
        Self.copyFiles(copies + Self.envFileCopy(settings, root: root, copies: copies), from: root, to: path)
        for folder in settings?.worktreeClone ?? [] { FolderClone.clone(folder, from: root, to: path) }
        record = TaskRegistryStore.recordCreated(path: path, branch: branch, base: base, root: root) { slot in
          TaskRegistryStore.portsAreFree(settings, slot: slot)
        }
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
        // Once the project's settings are trusted: the task's own values go
        // into its env file, then the setup script runs before the agent.
        self.trustProjectConfig(root: path) { [weak self] config in
          Self.prepareTask(config, root: root, path: path, slot: record?.slot) {
            let first = [config?.setupScript, command.isEmpty ? nil : command].compactMap { $0 }
            self?.tabManager.openWorkspace(
              folder: path, initialCommand: first.isEmpty ? nil : first.joined(separator: " && "))
            self?.toasts.show(Toast(kind: .success, message: "Started task \(branch)."))
          }
        }
      }
    }
  }

  /// "New Task from Branch…": an existing branch opened as a task, in a
  /// folder beside the repository like New Task… (copies, the task list,
  /// setup). A remote branch gets a local branch of the same name that
  /// tracks it, so pushing goes back to it.
  func openBranchAsTask(_ name: String, isRemote: Bool) {
    guard let repository = taskRepository(from: nil) else {
      toasts.show(Toast(kind: .info, message: "Open a folder in a git repository first."))
      return
    }
    let root = Self.mainCheckoutRoot(of: repository.root)
    let branch = isRemote ? PaletteModel.localName(forRemote: name) : name
    let path = WorktreeTasks.worktreePath(repoRoot: root, branch: branch)
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let copies = Self.taskCopies(root: root)
      var failure: String?
      if FileManager.default.fileExists(atPath: path) {
        failure = "\(TabManager.abbreviateHomePath(path)) already exists."
      } else {
        try? FileManager.default.createDirectory(
          atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        let result =
          isRemote
          ? GitOperations.addWorktree(path: path, branch: branch, newBranch: true, base: name, track: true, root: root)
          : GitOperations.addWorktree(path: path, branch: name, newBranch: false, root: root)
        if case .failure(let error) = result { failure = error.message }
      }
      var record: TaskRecord?
      if failure == nil {
        let settings = try? Self.loadProjectConfig(root: path)?.config.get()
        Self.copyFiles(copies + Self.envFileCopy(settings, root: root, copies: copies), from: root, to: path)
        for folder in settings?.worktreeClone ?? [] { FolderClone.clone(folder, from: root, to: path) }
        // The base it will be merged into: what New Task would start from.
        let main = GitClient.snapshot(forPath: root)
        let base = main.map { WorktreeTasks.defaultBase(branch: $0.branch, upstream: $0.upstream, ahead: $0.ahead).base }
        record = TaskRegistryStore.recordCreated(path: path, branch: branch, base: base ?? "", root: root) { slot in
          TaskRegistryStore.portsAreFree(settings, slot: slot)
        }
      }
      DispatchQueue.main.async {
        guard let self else { return }
        if let failure {
          self.toasts.show(Toast(kind: .warning, message: failure))
          return
        }
        let open: (String?) -> Void = { [weak self] setup in
          self?.tabManager.openWorkspace(folder: path, initialCommand: setup)
          self?.toasts.show(Toast(kind: .success, message: "Opened \(branch) as a task."))
        }
        if isRemote {
          // Someone else's branch: its folder isn't trusted like yours, its
          // values come only from settings you trusted already, and its
          // setup script is asked about every time.
          Self.prepareTask(self.alreadyTrustedProjectConfig(root: path), root: root, path: path, slot: record?.slot) {
            [weak self] in self?.confirmBranchSetup(name, root: path, completion: open)
          }
        } else {
          if Trust.shared.isTrusted(root) { Trust.shared.trust(path) }
          self.trustProjectConfig(root: path) { config in
            Self.prepareTask(config, root: root, path: path, slot: record?.slot) { open(config?.setupScript) }
          }
        }
      }
    }
  }

  /// A remote branch's setup script, once the user says to run it. Always
  /// asked, whatever project settings were trusted: the branch can change
  /// what the script runs (package.json scripts, say) without touching
  /// them. The answer isn't remembered. Nil when there's no script or the
  /// user declines.
  private func confirmBranchSetup(_ branch: String, root: String, completion: @escaping (String?) -> Void) {
    guard let setup = projectConfig(root: root)?.setupScript else { return completion(nil) }
    let alert = NSAlert()
    alert.alertStyle = .warning
    alert.messageText = "Run the setup script for \(branch)?"
    alert.informativeText = """
      The task for \(branch) is set up with this script:

        \(setup)

      It runs in the branch's checkout, where the branch decides what these commands do (a \
      changed package.json script, for example). Run it only if you trust the branch's changes. \
      The task opens either way.
      """
    alert.addButton(withTitle: "Run Setup Script")
    alert.addButton(withTitle: "Don't Run")
    presentWhenNoSheet { window in
      alert.beginSheetModal(for: window) { response in
        completion(response == .alertFirstButtonReturn ? setup : nil)
      }
    }
  }

  /// The env file, when the settings give tasks values of their own and the
  /// main checkout has one that `copies` doesn't already include: a task's
  /// values are written into a copy of it, secrets and all.
  static func envFileCopy(_ settings: ProjectConfig?, root: String, copies: [String]) -> [String] {
    guard let settings, settings.hasTaskValues, isInside(settings.envFile), !copies.contains(settings.envFile),
      FileManager.default.fileExists(atPath: (root as NSString).appendingPathComponent(settings.envFile))
    else { return [] }
    return [settings.envFile]
  }

  /// A relative path that stays inside the folder it's relative to.
  private static func isInside(_ path: String) -> Bool {
    !path.isEmpty && !path.hasPrefix("/") && !path.hasPrefix("~") && !path.split(separator: "/").contains("..")
  }

  /// What a new task needs from trusted settings before its first terminal
  /// opens: the database's data folder cloned (its service stopped in the
  /// main checkout meanwhile), then its own values written into its env
  /// file. Off the main thread; `done` runs on the main thread.
  static func prepareTask(
    _ config: ProjectConfig?, root: String, path: String, slot: Int?, done: @escaping () -> Void
  ) {
    guard let config, config.databaseFolder != nil || config.hasTaskValues || config.composeOverride else {
      return done()
    }
    DispatchQueue.global(qos: .userInitiated).async {
      if let folder = config.databaseFolder {
        cloneDatabase(folder, service: config.databaseService, from: root, to: path)
      }
      if let slot { writeTaskValues(config, path: path, slot: slot) }
      DispatchQueue.main.async(execute: done)
    }
  }

  /// Write a task's own values into its env file: its ports and
  /// `[worktrees.env]`, and with `compose_override`, `COMPOSE_FILE` naming
  /// the override written for it. False when there was nothing to write.
  @discardableResult
  static func writeTaskValues(_ config: ProjectConfig, path: String, slot: Int) -> Bool {
    guard isInside(config.envFile) else { return false }
    let task = (path as NSString).lastPathComponent
    var values = config.hasTaskValues ? TaskEnvironment.values(config: config, task: task, slot: slot) : []
    if config.composeOverride, let files = writeComposeOverride(config, path: path, task: task, slot: slot) {
      values.append(.init("COMPOSE_FILE", files))
    }
    guard !values.isEmpty else { return false }
    let file = (path as NSString).appendingPathComponent(config.envFile)
    let current = (try? String(contentsOfFile: file, encoding: .utf8)) ?? ""
    try? FileManager.default.createDirectory(
      atPath: (file as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
    try? TaskEnvironment.applying(values, to: current, comment: "Impulse task \(task)")
      .write(toFile: file, atomically: true, encoding: .utf8)
    return true
  }

  /// The task's Compose override (containers renamed, fixed ports moved),
  /// in `.git/impulse/tasks/<task>/compose.override.yml` so the project's
  /// own files don't change. Returns `COMPOSE_FILE` for it: the project's
  /// file, its override file if it has one, then this one.
  private static func writeComposeOverride(_ config: ProjectConfig, path: String, task: String, slot: Int) -> String? {
    guard let found = ComposeFile.find(in: path), let common = GitClient.commonGitDirectory(forPath: path),
      let override = ComposeFile.parse(found.text).override(task: task, slot: slot, offset: config.portOffset)
    else { return nil }
    let folder = taskFolder(task, common: common)
    let file = (folder as NSString).appendingPathComponent("compose.override.yml")
    try? FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
    guard (try? override.write(toFile: file, atomically: true, encoding: .utf8)) != nil else { return nil }
    let own = ["compose.override.yaml", "compose.override.yml", "docker-compose.override.yaml", "docker-compose.override.yml"]
      .first { FileManager.default.fileExists(atPath: (path as NSString).appendingPathComponent($0)) }
    return ([found.name] + [own].compactMap { $0 } + [file]).joined(separator: ":")
  }

  /// Impulse's own files for a task, outside its folder:
  /// `.git/impulse/tasks/<task>`.
  static func taskFolder(_ task: String, common: String) -> String {
    ((TaskRegistry.directory(commonGitDirectory: common) as NSString).appendingPathComponent("tasks") as NSString)
      .appendingPathComponent(task)
  }

  /// Clone a database's data folder into a task. A running database's
  /// files can't be copied safely, so its Compose service in the main
  /// checkout is stopped for the copy (a second or two) and started again.
  private static func cloneDatabase(_ folder: String, service: String?, from root: String, to path: String) {
    var restart = false
    if let service {
      let running = compose(["ps", "--status", "running", "--services"], in: root) ?? ""
      if running.split(separator: "\n").contains(where: { $0 == service }) {
        restart = compose(["stop", service], in: root) != nil
      }
    }
    FolderClone.clone(folder, from: root, to: path)
    if restart, let service { _ = compose(["start", service], in: root) }
  }

  /// `docker compose <arguments>` in `directory`, with the login shell's
  /// PATH; its output, or nil when it fails or takes over a minute.
  private static func compose(_ arguments: [String], in directory: String) -> String? {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    process.arguments = ["docker", "compose"] + arguments
    process.currentDirectoryURL = URL(fileURLWithPath: directory)
    var environment = ProcessInfo.processInfo.environment
    environment["PATH"] = LoginShell.loginPath()
    process.environment = environment
    let output = Pipe()
    process.standardOutput = output
    process.standardError = FileHandle.nullDevice
    let finished = DispatchSemaphore(value: 0)
    process.terminationHandler = { _ in finished.signal() }
    guard (try? process.run()) != nil else { return nil }
    let data = output.fileHandleForReading.readDataToEndOfFile()
    if finished.wait(timeout: .now() + 60) == .timedOut {
      process.terminate()
      return nil
    }
    return process.terminationStatus == 0 ? String(decoding: data, as: UTF8.self) : nil
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
      if let archiveScript,
        !Self.runScript(archiveScript, in: root, extra: TaskRegistryStore.identity(forDirectory: root))
      {
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
      // Impulse's own files for it (a Compose override) go with the folder.
      if case .success = result, let common = GitClient.commonGitDirectory(forPath: mainRoot) {
        try? FileManager.default.removeItem(atPath: Self.taskFolder((root as NSString).lastPathComponent, common: common))
      }
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
