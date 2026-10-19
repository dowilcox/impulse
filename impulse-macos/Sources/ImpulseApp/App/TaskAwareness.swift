import AppKit
import ImpulseGit
import ImpulseKit
import ImpulseProtocol

// Letting agents see each other. Each agent only knows its own folder; the
// `impulse tasks` command and Impulse's hooks for Claude Code and opencode
// tell it about the repository's other workspaces: a summary when it
// starts, a note when it edits a file another workspace changes, a question
// before it merges a branch a task is still working on (opencode can only
// be stopped, once), and dependency files that changed under it.

extension MainWindowController {
  // MARK: The listing

  /// The repository's workspaces as `impulse tasks` shows them, from the
  /// point of view of the workspace at `callerPath`. Off the main thread,
  /// with agents filled in on it; nil outside a repository.
  func taskListing(cwd: String, completion: @escaping (TaskListing?) -> Void) {
    DispatchQueue.global(qos: .userInitiated).async {
      guard let listing = Self.buildListing(cwd: cwd) else {
        return DispatchQueue.main.async { completion(nil) }
      }
      DispatchQueue.main.async {
        var listing = listing
        for index in listing.workspaces.indices {
          let agents = AppDelegate.shared?.agents(inFolder: listing.workspaces[index].path) ?? []
          let agent = agents.first { $0.state == .working || $0.state == .needsInput } ?? agents.first
          listing.workspaces[index].agent = agent?.name
          listing.workspaces[index].agentState = agent.map { Self.describe($0.state) }
        }
        completion(listing)
      }
    }
  }

  private static func describe(_ state: AgentState) -> String {
    switch state {
    case .working: return "working"
    case .needsInput: return "needs input"
    case .done: return "finished"
    default: return "idle"
    }
  }

  private static func buildListing(cwd: String) -> TaskListing? {
    guard let callerRoot = GitClient.repoRoot(forPath: cwd) else { return nil }
    let main = mainCheckoutRoot(of: callerRoot)
    guard let registry = TaskRegistryStore.registry(root: main) else { return nil }
    let caller = TaskRegistry.canonical(callerRoot)
    let settings = (try? loadProjectConfig(root: main)?.config.get()) ?? ProjectConfig()
    let changes = OverlapMonitor.changes(root: main)
    let callerFiles = changes.first { $0.path == caller }?.files ?? []
    func shared(_ path: String) -> [String] {
      guard path != caller else { return [] }
      let files = changes.first { $0.path == path }?.files ?? []
      return files.intersection(callerFiles).filter { $0 != settings.envFile }.sorted()
    }

    let mainSnapshot = GitClient.snapshot(forPath: main)
    let mainPath = TaskRegistry.canonical(main)
    let common = GitClient.commonGitDirectory(forPath: main)
    var workspaces = [
      TaskListing.Workspace(
        name: (main as NSString).lastPathComponent, path: mainPath, branch: mainSnapshot?.branch, base: nil,
        head: mainSnapshot?.headOid, isMainCheckout: true, isCaller: mainPath == caller,
        uncommitted: mainSnapshot?.changedFileCount ?? 0, ahead: mainSnapshot?.upstream == nil ? nil : mainSnapshot?.ahead,
        behind: mainSnapshot?.upstream == nil ? nil : mainSnapshot?.behind, sharedWithCaller: shared(mainPath))
    ]
    for task in registry.tasks {
      let path = TaskRegistry.canonical(task.path)
      let branch = "refs/heads/\(task.branch)"
      let counts = task.baseRef.flatMap { GitOperations.aheadBehind(branch, base: $0, root: main) }
      let name = (task.path as NSString).lastPathComponent
      let brief = common.flatMap { TaskBrief.read(task: name, commonGitDirectory: $0) }
      workspaces.append(
        .init(
          name: name, path: path, branch: task.branch, base: task.baseRef,
          head: GitClient.resolveCommit(repoPath: main, revision: branch), isMainCheckout: false, isCaller: path == caller,
          uncommitted: GitClient.snapshot(forPath: task.path)?.changedFileCount ?? 0, ahead: counts?.ahead,
          behind: counts?.behind, sharedWithCaller: shared(path), createdBy: task.createdBy,
          requestedBy: task.requestedBy, brief: brief.flatMap { TaskBrief.summary($0) }))
    }
    return TaskListing(repository: (main as NSString).lastPathComponent, workspaces: workspaces)
  }

  /// `impulse tasks`: the listing (`--json`), and `wait`, `show`, `new`,
  /// `finish` and `archive` for one task. Answers carry the listing (or the
  /// task) as JSON data too, for the MCP server.
  func handleTasksCommand(
    _ request: ControlRequest, cwd: String, terminal: TerminalTab?, caller: ControlCaller,
    reply: @escaping (ControlResponse) -> Void
  ) {
    let args = request.arguments
    if let name = args["wait"] {
      let timeout = args["timeout"].flatMap { TimeInterval($0) }
      return waitForTask(name, cwd: cwd, timeout: timeout, caller: caller, reply: reply)
    }
    if let name = args["show"] { return showTask(name, cwd: cwd, json: args["json"] != nil, reply: reply) }
    if args["new"] != nil { return requestTaskFromAgent(request, cwd: cwd, terminal: terminal, caller: caller, reply: reply) }
    if let name = args["finish"] {
      return finishTaskFromAgent(name, cwd: cwd, terminal: terminal, caller: caller, reply: reply)
    }
    if let name = args["archive"] { return archiveTaskFromAgent(name, cwd: cwd, caller: caller, reply: reply) }
    taskListing(cwd: cwd) { listing in
      guard let listing else { return reply(ControlResponse(ok: false, message: "Not in a git repository.")) }
      let json = listing.json()
      reply(ControlResponse(ok: true, message: args["json"] == nil ? listing.text() : json, data: Data(json.utf8)))
    }
  }

  /// `impulse tasks show <task>` (the MCP server's get_task): the listing's
  /// entry plus its files, commits, merged state, values and brief.
  private func showTask(_ name: String, cwd: String, json: Bool, reply: @escaping (ControlResponse) -> Void) {
    taskListing(cwd: cwd) { listing in
      guard let listing else { return reply(ControlResponse(ok: false, message: "Not in a git repository.")) }
      guard let workspace = listing.workspaces.first(where: { $0.name == name || $0.branch == name }) else {
        return reply(ControlResponse(ok: false, message: "impulse tasks: no task named \(name)"))
      }
      // Git reads in a folder that isn't trusted run no hooks or filters.
      let trusted = Trust.shared.isTrusted(workspace.path)
      DispatchQueue.global(qos: .userInitiated).async {
        let detail = Self.taskDetail(workspace, cwd: cwd, trusted: trusted)
        let data = AgentToolJSON.data(detail)
        DispatchQueue.main.async {
          reply(
            ControlResponse(
              ok: true, message: json ? AgentToolJSON.string(detail) : detail.text(), data: data))
        }
      }
    }
  }

  /// One workspace in detail. Off the main thread.
  static func taskDetail(_ workspace: TaskListing.Workspace, cwd: String, trusted: Bool) -> TaskDetail {
    var detail = TaskDetail(workspace: workspace)
    detail.uncommittedFiles = OverlapMonitor.uncommitted(GitClient.snapshot(forPath: workspace.path)).sorted()
    guard !workspace.isMainCheckout, let callerRoot = GitClient.repoRoot(forPath: cwd) else { return detail }
    let main = mainCheckoutRoot(of: callerRoot)
    guard let registry = TaskRegistryStore.registry(root: main),
      let record = registry.record(forPath: workspace.path)
    else { return detail }
    let branch = "refs/heads/\(record.branch)"
    if let base = record.baseRef {
      if let mergeBase = GitOperations.mergeBase(base, branch, root: main) {
        detail.changedFiles = GitOperations.changedPaths(from: mergeBase, to: branch, root: main).sorted()
      }
      detail.commits = GitOperations.commitsAhead(branch, base: base, root: main, limit: TaskDetail.commitLimit)
        .map { TaskDetail.Commit(id: $0.id, subject: $0.subject) }
      detail.moreCommits = max(0, (workspace.ahead ?? 0) - detail.commits.count)
      // A branch with no commits of its own isn't "merged".
      let head = GitClient.resolveCommit(repoPath: main, revision: branch)
      detail.merged = head != nil && head != record.start
        && GitOperations.isMerged(branch, into: base, root: main, trusted: trusted)
    }
    detail.slot = record.slot
    if let slot = record.slot, let settings = try? loadProjectConfig(root: main)?.config.get(), !settings.ports.isEmpty {
      detail.ports = TaskEnvironment.ports(settings.ports, slot: slot, offset: settings.portOffset) ?? [:]
    }
    if let common = GitClient.commonGitDirectory(forPath: main) {
      detail.brief = TaskBrief.read(task: workspace.name, commonGitDirectory: common)
    }
    return detail
  }

  /// Answer once the task's agents stop working (checked every 2 seconds),
  /// or after `timeout` seconds (the MCP server's wait_for_task), or stop
  /// checking when the caller gives up.
  private func waitForTask(
    _ name: String, cwd: String, timeout: TimeInterval?, caller: ControlCaller,
    reply: @escaping (ControlResponse) -> Void
  ) {
    DispatchQueue.global(qos: .userInitiated).async {
      let registry = GitClient.repoRoot(forPath: cwd).flatMap { TaskRegistryStore.registry(root: $0) }
      let task = registry?.record(named: name)
      DispatchQueue.main.async {
        guard let task else {
          return reply(ControlResponse(ok: false, message: "impulse tasks: no task named \(name)"))
        }
        func busy() -> Bool {
          (AppDelegate.shared?.agents(inFolder: task.path) ?? []).contains { $0.state == .working || $0.state == .needsInput }
        }
        func answer(_ stopped: Bool, _ text: String) {
          reply(ControlResponse(ok: true, message: text, data: AgentToolJSON.data(WaitAnswer(task: name, stopped: stopped))))
        }
        guard busy() else { return answer(true, "\(name): no agent is working.") }
        let deadline = timeout.map { Date().addingTimeInterval($0) }
        let timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { timer in
          if !busy() {
            timer.invalidate()
            answer(true, "\(name): its agent stopped working.")
          } else if let deadline, Date() >= deadline {
            timer.invalidate()
            answer(false, "\(name): its agent is still working after \(Int(timeout ?? 0)) seconds. Wait again, or check on it with get_task.")
          }
        }
        caller.onHangUp = { timer.invalidate() }
      }
    }
  }

  // MARK: Hooks

  /// What Impulse answers a Claude Code hook, or an event from its
  /// opencode plugin, with (nil: nothing to say).
  func agentHookReply(_ request: ControlRequest, terminal: TerminalTab, completion: @escaping (String?) -> Void) {
    let args = request.arguments
    let settings = SettingsStore.shared.settings
    let cwd = args["hookCwd"] ?? terminal.currentWorkingDirectory ?? NSHomeDirectory()
    let opencode = args["agent"] == "opencode"
    // Claude Code takes a note as JSON (a SessionStart hook's plain output
    // is context already); the opencode plugin, as its own JSON.
    let context = { (text: String) in opencode ? AgentHookReply.opencodeContext(text) : AgentHookReply.context(text) }
    switch AgentHookEvents.reply(agent: args["agent"], event: args["event"] ?? "", tool: args["tool"]) {
    case .sessionSummary where settings.agentHookTaskSummary:
      taskListing(cwd: cwd) { completion($0?.sessionSummary().map { opencode ? context($0) : $0 }) }

    case .editedFile:
      guard settings.agentHookSharedFiles, let file = args["toolFile"] else { return completion(nil) }
      completion(sharedFileNote(file: file, terminal: terminal).map(context))

    case .ranShell:
      guard settings.agentHookDependencies, let command = args["toolCommand"], GitCommandInspector.movesHead(command)
      else { return completion(nil) }
      DispatchQueue.global(qos: .userInitiated).async { [weak self] in
        let change = Self.dependencyChange(cwd: cwd)
        DispatchQueue.main.async { [weak self] in
          guard let self, let change else { return completion(nil) }
          // Commands go to the agent only from settings the user trusts.
          let config = self.alreadyTrustedProjectConfig(root: change.root)
          let note = Self.dependencyNote(changed: change.changed, trusted: config)
          completion(note.map(context))
        }
      }

    case .beforeShell:
      guard settings.agentHookMergeGuard, let command = args["toolCommand"] else { return completion(nil) }
      let refs = GitCommandInspector.mergedRefs(in: command)
      guard !refs.isEmpty else { return completion(nil) }
      // opencode's plugin can't ask you, only stop the command: once, so
      // the agent asks, then the same command again goes ahead.
      if opencode, terminal.stoppedMerges.remove(command) != nil { return completion(nil) }
      mergeGuard(refs: refs, cwd: cwd) { reason in
        guard let reason else { return completion(nil) }
        guard opencode else { return completion(AgentHookReply.ask(reason)) }
        terminal.stoppedMerges.insert(command)
        completion(AgentHookReply.opencodeStop(reason))
      }

    default:
      completion(nil)
    }
  }

  /// "src/page.tsx is also changed in the task interia-upgrade, where Claude
  /// Code is working", once per file per terminal.
  private func sharedFileNote(file: String, terminal: TerminalTab) -> String? {
    guard let root = GitClient.repoRoot(forPath: (file as NSString).deletingLastPathComponent),
      let common = GitClient.commonGitDirectory(forPath: root),
      let changes = OverlapMonitor.shared.changes[common]
    else { return nil }
    let me = TaskRegistry.canonical(root)
    let relative = String(TaskRegistry.canonical(file).dropFirst(me.count + 1))
    let others = changes.filter { $0.path != me && $0.files.contains(relative) }
    guard !others.isEmpty, terminal.sharedFileNotes.insert(relative).inserted else { return nil }
    let described = others.map { other -> String in
      // The main checkout comes first in the list.
      let isMain = other.path == changes.first?.path
      var text = (isMain ? "the main checkout (\(other.name))" : "the task \(other.name)")
      let agents = AppDelegate.shared?.agents(inFolder: other.path) ?? []
      if let agent = agents.first(where: { $0.state == .working || $0.state == .needsInput }) {
        text += ", where \(agent.name) is \(agent.state == .working ? "working" : "waiting for input")"
      }
      return text
    }
    return "Impulse: \(relative) is also changed in \(described.joined(separator: " and ")). "
      + "Two workspaces changing the same file will conflict when merged: keep your changes there focused, or check with the user."
  }

  /// The files the agent's own git command changed when it moved HEAD
  /// (just now), in the repository at `root`. Off the main thread.
  private static func dependencyChange(cwd: String) -> (root: String, changed: [String])? {
    guard let root = GitClient.repoRoot(forPath: cwd), let move = GitOperations.lastHeadMove(root: root),
      abs(move.time.timeIntervalSinceNow) < 120, DependencyChanges.move(reflog: move.subject) != .ownCommit
    else { return nil }
    return (root, GitOperations.changedPaths(from: move.from, to: move.to, root: root))
  }

  /// Which dependency files came with that change, and what to run. The
  /// project's `[on_change]` commands are named only from `trusted`
  /// settings: an untrusted file's text mustn't reach the agent as
  /// Impulse's advice.
  private static func dependencyNote(changed: [String], trusted: ProjectConfig?) -> String? {
    let rules = DependencyChanges.matches(rules: trusted?.onChange ?? [:], changed: changed)
    let known = DependencyChanges.knownChanged(changed)
    guard !rules.isEmpty || !known.isEmpty else { return nil }
    let files = Array(Set(rules.map(\.file) + known)).sorted().joined(separator: ", ")
    if !rules.isEmpty {
      return "Impulse: that changed \(files). Bring the dependencies up to date before running tests: \(rules.map(\.command).joined(separator: " && "))"
    }
    return "Impulse: that changed \(files). Reinstall dependencies (or rebuild images) before running tests."
  }

  /// Why to ask before merging `refs`: one of them is a task's branch that's
  /// still moving.
  private func mergeGuard(refs: [String], cwd: String, completion: @escaping (String?) -> Void) {
    DispatchQueue.global(qos: .userInitiated).async {
      guard let root = GitClient.repoRoot(forPath: cwd) else { return DispatchQueue.main.async { completion(nil) } }
      let remotes = GitOperations.remotes(root: root)
      let me = TaskRegistry.canonical(root)
      let candidates = refs.map { ref in
        remotes.first { ref.hasPrefix("\($0)/") }.map { String(ref.dropFirst($0.count + 1)) } ?? ref
      }
      let tasks = candidates.compactMap { TaskActivity.find(branch: $0, root: root) }
        .filter { TaskRegistry.canonical($0.record.path) != me }
      DispatchQueue.main.async {
        for var task in tasks {
          if let question = task.question() {
            return completion("\(question.title): \(question.message)")
          }
        }
        completion(nil)
      }
    }
  }
}

/// What `tasks wait` answers with, as data.
private struct WaitAnswer: Encodable {
  let task: String
  /// False when it gave up waiting (its timeout).
  let stopped: Bool
}
