import AppKit
import ImpulseGit
import ImpulseKit
import ImpulseProtocol

// Tasks agents ask for: `impulse tasks new|finish|archive` and the MCP
// server's create_task, finish_task and archive_task. Each goes through the
// same steps as the user's own New Task…, Finish Task… and Archive Task…,
// and asks the user first (`AgentTaskPolicy` says when New Task may skip
// the sheet). The request waits for the outcome, so the agent hears what
// happened rather than guessing.

extension MainWindowController {
  /// Windows with an agent's New Task sheet up: one request at a time.
  private static var windowsAskingForTasks = Set<ObjectIdentifier>()

  /// `impulse tasks new` / create_task.
  func requestTaskFromAgent(
    _ request: ControlRequest, cwd: String, terminal: TerminalTab?, caller: ControlCaller,
    reply: @escaping (ControlResponse) -> Void
  ) {
    let args = request.arguments
    let title = (args["new"] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let brief = args["brief"] ?? ""
    guard !title.isEmpty else { return reply(ControlResponse(ok: false, message: "A task needs a title.")) }
    guard brief.utf8.count <= ControlProtocol.briefLimit else {
      return reply(ControlResponse(ok: false, message: "The brief holds more than 16 KB."))
    }
    var agent: AgentKind?
    if let id = args["agent"]?.trimmingCharacters(in: .whitespaces), !id.isEmpty {
      guard let kind = KnownAgents.builtIn.first(where: { $0.id == id || $0.names.contains(id) }) else {
        let known = KnownAgents.builtIn.map(\.id).joined(separator: ", ")
        return reply(ControlResponse(ok: false, message: "Impulse doesn't know an agent called \(id). It knows \(known)."))
      }
      agent = kind
    }
    let moving = args["move"] != nil
    let settings = SettingsStore.shared.settings
    let callerWorkspace = terminal.flatMap { tabManager.workspace(ofTerminal: $0) }
    let agentName = terminal?.agent?.displayName ?? "An agent"

    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      guard let callerRoot = GitClient.repoRoot(forPath: cwd) else {
        return DispatchQueue.main.async { reply(ControlResponse(ok: false, message: "Not in a git repository.")) }
      }
      let main = Self.mainCheckoutRoot(of: callerRoot)
      let registry = TaskRegistryStore.registry(root: main)
      let fromAgentTask = registry?.record(forPath: callerRoot)?.isFromAgent == true
      let installed = agent.map { kind in kind.names.first.flatMap { LoginShell.which($0) } != nil } ?? true
      let hasSettings = Self.loadProjectConfig(root: main) != nil
      DispatchQueue.main.async {
        guard let self else { return reply(ControlResponse(ok: false, message: "The window closed.")) }
        if let agent, !installed {
          return reply(ControlResponse(ok: false, message: "\(agent.displayName) isn't installed here (its command isn't on PATH)."))
        }
        let where_ = callerWorkspace?.name ?? (callerRoot as NSString).lastPathComponent
        let isMain = TaskRegistry.canonical(callerRoot) == TaskRegistry.canonical(main)
        let requester = AgentTaskRequest(
          requestedBy: "\(agentName) in \(where_)\(isMain ? " (main checkout)" : "")", fromPath: callerRoot)
        let decision = AgentTaskPolicy.decide(
          setting: AgentTaskPolicy.Setting(setting: settings.agentTaskCreation),
          repositoryTrusted: Trust.shared.isTrusted(main),
          projectTrusted: !hasSettings || self.alreadyTrustedProjectConfig(root: main) != nil,
          agentTasks: registry?.agentTaskCount ?? 0, limit: settings.agentTaskLimit, fromAgentTask: fromAgentTask,
          movesChanges: moving)
        let finished: ((path: String, branch: String)?) -> Void = { [weak self] created in
          guard let created else {
            return reply(ControlResponse(ok: false, message: "The user declined: no task was created."))
          }
          self?.replyWithNewTask(created, cwd: cwd, reply: reply)
        }
        switch decision {
        case .refuse(let reason):
          reply(ControlResponse(ok: false, message: reason))
        case .ask:
          let key = ObjectIdentifier(self)
          guard !Self.windowsAskingForTasks.contains(key) else {
            return reply(ControlResponse(ok: false, message: "Another task request is waiting for the user. Try again once they've answered it."))
          }
          Self.windowsAskingForTasks.insert(key)
          // The question is in this window: make sure it's seen.
          terminal?.setAttentionFromAgent()
          if !NSApp.isActive { NSApp.requestUserAttention(.informationalRequest) }
          self.presentNewTaskSheet(
            from: callerWorkspace?.id, title: title, command: agent?.names.first ?? "", base: args["base"],
            movingChanges: moving, agentRequest: requester, brief: brief
          ) { created in
            Self.windowsAskingForTasks.remove(key)
            finished(created)
          }
        case .create:
          self.createTaskWithoutAsking(
            title: title, base: args["base"], agent: agent, brief: brief, root: main, requester: requester,
            finished: finished, failed: { reply(ControlResponse(ok: false, message: "The task wasn't created: \($0)")) })
        }
      }
    }
  }

  /// The new task as `impulse tasks show` describes it.
  private func replyWithNewTask(
    _ created: (path: String, branch: String), cwd: String, reply: @escaping (ControlResponse) -> Void
  ) {
    taskListing(cwd: created.path) { listing in
      guard let workspace = listing?.workspaces.first(where: \.isCaller) else {
        return reply(ControlResponse(ok: true, message: "Started task \(created.branch) in \(created.path)."))
      }
      let trusted = Trust.shared.isTrusted(created.path)
      DispatchQueue.global(qos: .userInitiated).async {
        let detail = Self.taskDetail(workspace, cwd: cwd, trusted: trusted)
        DispatchQueue.main.async {
          reply(
            ControlResponse(
              ok: true, message: "Started task \(created.branch).\n\n" + detail.text(), data: AgentToolJSON.data(detail)))
        }
      }
    }
  }

  /// `agent_task_creation = trusted_projects`: what New Task would make,
  /// without the sheet, then a toast with Undo.
  private func createTaskWithoutAsking(
    title: String, base: String?, agent: AgentKind?, brief: String, root: String, requester: AgentTaskRequest,
    finished: @escaping ((path: String, branch: String)?) -> Void, failed: @escaping (String) -> Void
  ) {
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let snapshot = GitClient.snapshot(forPath: root)
      let defaultBase = snapshot.map {
        WorktreeTasks.defaultBase(branch: $0.branch, upstream: $0.upstream, ahead: $0.ahead).base
      }
      let copies = Self.taskCopies(root: root)
      let taken = Set(GitOperations.branches(root: root).local)
      DispatchQueue.main.async {
        guard let self else { return failed("the window closed") }
        let model = TaskSheetModel(repoRoot: root, base: base ?? defaultBase ?? "HEAD")
        model.draft.title = title
        model.draft.command = agent?.names.first ?? ""
        model.draft.brief = brief
        model.agentRequest = requester
        model.copies = copies
        model.takenBranches = taken
        model.isLoading = false
        model.createdWithoutAsking = true
        model.onCreated = { path, branch in finished((path, branch)) }
        model.onFailed = failed
        self.createTask(model) {}
      }
    }
  }

  /// For agents whose command takes no first prompt: type the brief in (and
  /// submit it) once the agent is up in `terminal`, giving up after a
  /// minute and a half.
  static func typeBriefWhenAgentStarts(_ brief: String, into terminal: TerminalTab) {
    var waited = 0.0
    Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak terminal] timer in
      waited += 1
      guard let terminal, waited < 90 else { return timer.invalidate() }
      guard terminal.agent != nil, terminal.agentState == .idle || terminal.agentState == .done else { return }
      timer.invalidate()
      // Its prompt may still be drawing.
      DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak terminal] in
        terminal?.paste(brief, submit: true)
      }
    }
  }

  /// Undo of a task created without asking: close its workspace, archive
  /// its folder (whatever it holds is kept in a snapshot), and delete its
  /// branch when nothing was committed on it.
  func undoAgentTask(path: String, branch: String, root: String, start: String?) {
    let finish = { [weak self] in
      DispatchQueue.global(qos: .userInitiated).async { [weak self] in
        let removed = Self.removeTask(root: path, branch: branch, dirty: true, archiveScript: nil).result
        // Only while it's still where it started: nothing was committed.
        var deleted = false
        if case .success = removed, let start {
          deleted = (try? GitOperations.deleteBranch(branch, at: start, root: root).get()) != nil
        }
        DispatchQueue.main.async {
          guard let self else { return }
          switch removed {
          case .failure(let error):
            self.presentGitError(error, title: "Couldn't undo \(branch)")
          case .success(let archived):
            self.toasts.show(
              Toast(
                kind: .success, message: deleted ? "Removed task \(branch)." : "Archived \(branch). The branch is kept.",
                actionTitle: deleted ? nil : "Undo", action: deleted ? nil : { [weak self] in self?.restoreTasks([archived]) },
                lifetime: 15))
          }
        }
      }
    }
    let canonical = TaskRegistry.canonical(path)
    guard let workspace = tabManager.workspaces.first(where: { TaskRegistry.canonical($0.root) == canonical }) else {
      return finish()
    }
    requestCloseWorkspace(workspace.id, recordForUndo: false, then: finish, cancelled: {})
  }

  // MARK: Finish and archive

  /// The task `name` (folder or branch) and its workspace in this window,
  /// opened if it isn't.
  private func agentTaskWorkspace(
    _ name: String, cwd: String, completion: @escaping (Result<Workspace, MainWindowController.ToolFailure>) -> Void
  ) {
    DispatchQueue.global(qos: .userInitiated).async {
      let record = GitClient.repoRoot(forPath: cwd).flatMap { TaskRegistryStore.registry(root: $0) }?.record(named: name)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard let record else { return completion(.failure(ToolFailure(message: "impulse tasks: no task named \(name)"))) }
        let canonical = TaskRegistry.canonical(record.path)
        if let open = self.tabManager.workspaces.first(where: { TaskRegistry.canonical($0.root) == canonical }) {
          open.isTask = true
          return completion(.success(open))
        }
        guard FileManager.default.fileExists(atPath: record.path) else {
          return completion(.failure(ToolFailure(message: "\(name)'s folder is gone.")))
        }
        let workspace = self.tabManager.openWorkspace(folder: record.path)
        workspace.isTask = true
        completion(.success(workspace))
      }
    }
  }

  /// `impulse tasks finish` / finish_task: Finish runs up to landing, then
  /// waits for the user. Answers each time the run stops.
  func finishTaskFromAgent(
    _ name: String, cwd: String, terminal: TerminalTab?, caller: ControlCaller,
    reply: @escaping (ControlResponse) -> Void
  ) {
    let requester = terminal?.agent?.displayName ?? "An agent"
    agentTaskWorkspace(name, cwd: cwd) { [weak self] result in
      guard let self else { return }
      let workspace: Workspace
      switch result {
      case .success(let found): workspace = found
      case .failure(let failure): return reply(ControlResponse(ok: false, message: failure.message))
      }
      guard let model = self.openFinishTask(from: workspace.id) else {
        return reply(ControlResponse(ok: false, message: "Finish Task couldn't open for \(name)."))
      }
      if !NSApp.isActive { NSApp.requestUserAttention(.informationalRequest) }
      model.stopsBeforeLanding = requester
      model.onStopped = { [weak model] in
        guard let model else { return }
        model.onStopped = nil
        reply(Self.finishAnswer(model))
      }
      caller.onHangUp = { [weak model] in model?.onStopped = nil }
      self.startFinishWhenLoaded(model) { problem in
        guard let problem else { return }
        model.onStopped = nil
        model.stopsBeforeLanding = nil
        reply(ControlResponse(ok: false, message: problem))
      }
    }
  }

  /// Where an agent's Finish stopped, step by step.
  private static func finishAnswer(_ model: TaskFinishModel) -> ControlResponse {
    struct Step: Encodable {
      let step: String
      let status: String
      let text: String
    }
    struct Answer: Encodable {
      let task: String
      let steps: [Step]
      /// The step it stopped at, if it stopped before the end.
      let stoppedAt: String?
      let waitingForUser: Bool
      let finished: Bool
    }
    var steps: [Step] = []
    var stoppedAt: String?
    var waiting = false
    for step in TaskFinishModel.Step.allCases {
      let title = model.title(of: step)
      switch model.statuses[step] ?? .pending {
      case .pending: steps.append(Step(step: title, status: "pending", text: model.plan(of: step)))
      case .running(let text): steps.append(Step(step: title, status: "running", text: text))
      case .done(let text): steps.append(Step(step: title, status: "done", text: text))
      case .skipped(let text): steps.append(Step(step: title, status: "skipped", text: text))
      case .failed(let text):
        steps.append(Step(step: title, status: "failed", text: text))
        stoppedAt = stoppedAt ?? title
      case .waiting(let text):
        steps.append(Step(step: title, status: "waiting for the user", text: text))
        stoppedAt = stoppedAt ?? title
        waiting = true
      }
    }
    let finished = stoppedAt == nil && model.resumeAt == nil
    let answer = Answer(task: model.name, steps: steps, stoppedAt: stoppedAt, waitingForUser: waiting, finished: finished)
    var text = steps.map { "\($0.step): \($0.status). \($0.text)" }.joined(separator: "\n")
    if waiting {
      text = "Ready to land; the user lands it from the Finish tab.\n\n" + text
    } else if let stoppedAt {
      text = "Finish stopped at \(stoppedAt). Fix that, then call finish_task again.\n\n" + text
    } else {
      text = "Finished \(model.name).\n\n" + text
    }
    return ControlResponse(ok: stoppedAt == nil || waiting, message: text, data: AgentToolJSON.data(answer))
  }

  /// `impulse tasks archive` / archive_task: Archive Task…, with its
  /// questions and Undo. Answers once it's archived, or the user said no.
  func archiveTaskFromAgent(
    _ name: String, cwd: String, caller: ControlCaller, reply: @escaping (ControlResponse) -> Void
  ) {
    agentTaskWorkspace(name, cwd: cwd) { [weak self] result in
      guard let self else { return }
      let workspace: Workspace
      switch result {
      case .success(let found): workspace = found
      case .failure(let failure): return reply(ControlResponse(ok: false, message: failure.message))
      }
      let id = workspace.id
      let root = workspace.root
      if !NSApp.isActive { NSApp.requestUserAttention(.informationalRequest) }
      self.archiveTask(id)
      // Archive Task… took it on (or said why not, in the window).
      guard self.archivingTasks.contains(id) else {
        return reply(ControlResponse(ok: false, message: "\(name) can't be archived now (Impulse says why in its window)."))
      }
      // It asks first; once it's past the questions the workspace closes
      // and the folder goes a moment later.
      var closedAt: Date?
      let timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] timer in
        guard let self else { return timer.invalidate() }
        if !FileManager.default.fileExists(atPath: root) {
          timer.invalidate()
          return reply(ControlResponse(ok: true, message: "Archived \(name). Its branch is kept, and the user can Undo."))
        }
        guard !self.archivingTasks.contains(id) else { return }
        guard self.tabManager.workspace(id) == nil else {
          timer.invalidate()
          return reply(ControlResponse(ok: false, message: "\(name) wasn't archived: the user declined."))
        }
        closedAt = closedAt ?? Date()
        if let closedAt, Date().timeIntervalSince(closedAt) > 120 {
          timer.invalidate()
          reply(ControlResponse(ok: false, message: "\(name)'s workspace closed, but its folder is still there (Impulse says why in its window)."))
        }
      }
      caller.onHangUp = { timer.invalidate() }
    }
  }
}
