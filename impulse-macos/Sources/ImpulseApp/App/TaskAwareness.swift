import AppKit
import ImpulseGit
import ImpulseKit
import ImpulseProtocol

// Letting agents see each other. Each agent only knows its own folder; the
// `impulse tasks` command and Impulse's Claude Code hooks tell it about the
// repository's other workspaces: a summary when it starts, a note when it
// edits a file another workspace changes, a question before it merges a
// branch a task is still working on, and dependency files that changed
// under it.

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
      workspaces.append(
        .init(
          name: (task.path as NSString).lastPathComponent, path: path, branch: task.branch, base: task.baseRef,
          head: GitClient.resolveCommit(repoPath: main, revision: branch), isMainCheckout: false, isCaller: path == caller,
          uncommitted: GitClient.snapshot(forPath: task.path)?.changedFileCount ?? 0, ahead: counts?.ahead,
          behind: counts?.behind, sharedWithCaller: shared(path)))
    }
    return TaskListing(repository: (main as NSString).lastPathComponent, workspaces: workspaces)
  }

  /// `impulse tasks [--json]` and `impulse tasks wait <task>`.
  func handleTasksCommand(_ request: ControlRequest, cwd: String, reply: @escaping (ControlResponse) -> Void) {
    if let name = request.arguments["wait"] {
      return waitForTask(name, cwd: cwd, reply: reply)
    }
    taskListing(cwd: cwd) { listing in
      guard let listing else { return reply(ControlResponse(ok: false, message: "Not in a git repository.")) }
      reply(ControlResponse(ok: true, message: request.arguments["json"] == nil ? listing.text() : listing.json()))
    }
  }

  /// Answer once the task's agents stop working (checked every 2 seconds).
  private func waitForTask(_ name: String, cwd: String, reply: @escaping (ControlResponse) -> Void) {
    DispatchQueue.global(qos: .userInitiated).async {
      let registry = GitClient.repoRoot(forPath: cwd).flatMap { TaskRegistryStore.registry(root: $0) }
      let task = registry?.tasks.first { ($0.path as NSString).lastPathComponent == name || $0.branch == name }
      DispatchQueue.main.async {
        guard let task else {
          return reply(ControlResponse(ok: false, message: "impulse tasks: no task named \(name)"))
        }
        func busy() -> Bool {
          (AppDelegate.shared?.agents(inFolder: task.path) ?? []).contains { $0.state == .working || $0.state == .needsInput }
        }
        guard busy() else { return reply(ControlResponse(ok: true, message: "\(name): no agent is working.")) }
        Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { timer in
          guard !busy() else { return }
          timer.invalidate()
          reply(ControlResponse(ok: true, message: "\(name): its agent stopped working."))
        }
      }
    }
  }

  // MARK: Hooks

  /// What Impulse answers a Claude Code hook with (nil: nothing to say).
  func agentHookReply(_ request: ControlRequest, terminal: TerminalTab, completion: @escaping (String?) -> Void) {
    let args = request.arguments
    let settings = SettingsStore.shared.settings
    let cwd = args["hookCwd"] ?? terminal.currentWorkingDirectory ?? NSHomeDirectory()
    switch (args["event"], args["tool"]) {
    case ("SessionStart", _) where settings.agentHookTaskSummary:
      taskListing(cwd: cwd) { completion($0?.sessionSummary()) }

    case ("PostToolUse", let tool?) where ["Edit", "Write", "MultiEdit", "NotebookEdit"].contains(tool):
      guard settings.agentHookSharedFiles, let file = args["toolFile"] else { return completion(nil) }
      completion(sharedFileNote(file: file, terminal: terminal).map { AgentHookReply.context($0) })

    case ("PostToolUse", "Bash"?):
      guard settings.agentHookDependencies, let command = args["toolCommand"], GitCommandInspector.movesHead(command)
      else { return completion(nil) }
      DispatchQueue.global(qos: .userInitiated).async {
        let note = Self.dependencyNote(cwd: cwd)
        DispatchQueue.main.async { completion(note.map { AgentHookReply.context($0) }) }
      }

    case ("PreToolUse", "Bash"?):
      guard settings.agentHookMergeGuard, let command = args["toolCommand"] else { return completion(nil) }
      let refs = GitCommandInspector.mergedRefs(in: command)
      guard !refs.isEmpty else { return completion(nil) }
      mergeGuard(refs: refs, cwd: cwd) { reason in completion(reason.map(AgentHookReply.ask)) }

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

  /// After the agent's own git command moved HEAD (just now) and brought
  /// dependency files with it: which, and what to run.
  private static func dependencyNote(cwd: String) -> String? {
    guard let root = GitClient.repoRoot(forPath: cwd), let move = GitOperations.lastHeadMove(root: root),
      abs(move.time.timeIntervalSinceNow) < 120, DependencyChanges.move(reflog: move.subject) != .ownCommit
    else { return nil }
    let changed = GitOperations.changedPaths(from: move.from, to: move.to, root: root)
    let settings = (try? loadProjectConfig(root: root)?.config.get()) ?? ProjectConfig()
    let rules = DependencyChanges.matches(rules: settings.onChange, changed: changed)
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
