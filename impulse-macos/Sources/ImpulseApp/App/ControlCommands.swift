import AppKit
import ImpulseGit
import ImpulseKit
import ImpulseProtocol

// What `impulse` subcommands do in a window. The calling pane (found by its
// token) is the anchor: files open in its window, splits split it, hooks
// and statuses describe the agent running in it.
//
// The token only names a pane; it isn't proof the request comes from it
// (any process that sees a shell's environment can read it). So commands
// that start processes, write files or run git where the caller says
// (split, tab, checkpoint, creating a file to edit) also need the caller to
// be the pane's shell or a program started from it. The rest only change
// what Impulse shows, and keep working with the token alone, from inside
// tmux too (whose sessions leave the shell's process tree).

extension MainWindowController {
  /// The terminal with a control token, if it's in this window.
  func terminal(controlToken token: String) -> TerminalTab? {
    for case .terminal(let container) in tabManager.allSurfaces {
      if let terminal = container.activeTerminal, terminal.controlToken == token { return terminal }
    }
    return nil
  }

  func handleControl(
    _ request: ControlRequest, terminal: TerminalTab?, caller: ControlCaller,
    reply: @escaping (ControlResponse) -> Void
  ) {
    let args = request.arguments
    let fromShell = terminal.map { caller.descends(from: $0.backend?.childPid() ?? 0) } ?? false
    // Where git runs: the caller's folder only when it's the pane's own
    // shell (or started from it), else the pane's.
    let cwd = fromShell ? request.cwd ?? terminal?.currentWorkingDirectory : terminal?.currentWorkingDirectory
    let insideTerminal = ControlResponse(ok: false, message: "Run this inside an Impulse terminal.")

    switch request.command {
    case "open":
      guard let path = args["path"] else { return reply(ControlResponse(ok: false, message: "No file given.")) }
      // Here, /dev/fd/5 would be one of Impulse's own files, not the
      // caller's (`impulse` sends a copy instead).
      guard !Self.isUnderDev(path) else {
        return reply(ControlResponse(ok: false, message: "\(path) isn't a regular file."))
      }
      let exists = FileManager.default.fileExists(atPath: path)
      guard exists || request.wait else {
        return reply(ControlResponse(ok: false, message: "No such file: \(path)"))
      }
      if !exists {
        // `impulse edit new-file`: start it empty.
        guard fromShell else {
          return reply(
            ControlResponse(
              ok: false, message: "No such file: \(path) (only the terminal's own shell can create one)"))
        }
        guard FileManager.default.createFile(atPath: path, contents: nil) else {
          return reply(ControlResponse(ok: false, message: "Couldn't create \(path)."))
        }
      }
      // `impulse open .`: a folder opens as a workspace. Anything else that
      // isn't a regular file (a FIFO, a device) would block the read.
      let type = Self.fileType(path)
      guard type == S_IFREG || type == S_IFDIR else {
        return reply(ControlResponse(ok: false, message: "\(path) isn't a regular file."))
      }
      if args["background"] == nil {
        window?.makeKeyAndOrderFront(nil)
        // A headless snapshot run never takes focus from the app you're in.
        if !DebugSnapshot.isActive { NSApp.activate(ignoringOtherApps: true) }
      } else if !NSApp.isActive {
        // An agent's open_file: the tab opens where it is, and the pane
        // asks for attention instead of taking focus from another app.
        terminal?.setAttentionFromAgent()
      }
      if type == S_IFDIR {
        tabManager.openWorkspace(folder: path)
        return reply(ControlResponse(ok: true))
      }
      // Images never get an editor tab to close: don't wait.
      if request.wait, TabManager.isImageFile(path) {
        openFile(path: path)
        return reply(ControlResponse(ok: true))
      }
      let line = args["line"].flatMap(UInt32.init)
      let column = args["column"].flatMap(UInt32.init)
      let opened: (() -> Void)? =
        request.wait ? { [weak self] in self?.replyWhenEditorCloses(path: path, caller: caller, reply: reply) } : nil
      tabManager.addEditorTab(
        path: path, projectDirectory: fileTreeRootPath, goToLine: line,
        goToColumn: line == nil ? nil : (column ?? 1), opened: opened)
      if !request.wait { reply(ControlResponse(ok: true)) }

    case "review":
      if args["scope"] == "last-turn" {
        let recorded =
          terminal.flatMap { AgentCheckpoints.shared.lastTurn(terminalID: $0.id) }
          ?? windowModel.repository.flatMap { AgentCheckpoints.shared.lastTurn(inRepo: $0.root) }
        guard recorded != nil || args["via"] == nil else {
          return reply(ControlResponse(ok: false, message: "No agent turns recorded yet in this repository."))
        }
        reviewLastAgentTurn(terminalID: terminal?.id)
        return reply(ControlResponse(ok: true))
      }
      guard let cwd else { return reply(insideTerminal) }
      let scope: DiffScope = {
        switch args["scope"] {
        case "staged": return .staged
        case "unstaged": return .unstaged
        default: return .uncommitted
        }
      }()
      GitRepositoryStore.shared.resolve(directory: cwd) { [weak self] state in
        guard let self, let state else {
          return reply(ControlResponse(ok: false, message: "Not in a git repository."))
        }
        if args["background"] == nil { self.window?.makeKeyAndOrderFront(nil) }
        self.tabManager.addReviewTab(repository: state, scope: scope, focusPath: nil, host: self)
        reply(ControlResponse(ok: true))
      }

    case "split" where !fromShell, "tab" where !fromShell, "checkpoint" where !fromShell,
      "comments" where !fromShell && args["resolve"] != nil,
      "tasks" where !fromShell && (args["new"] ?? args["finish"] ?? args["archive"]) != nil:
      // These start commands, run git or change what's saved, as Impulse
      // (with whatever privacy access it has), so only the pane's own
      // shell, or a program it started (an agent's `impulse mcp`), may ask:
      // not any process that can reach the socket.
      guard terminal != nil else { return reply(insideTerminal) }
      reply(
        ControlResponse(
          ok: false,
          message: "impulse \(request.command) only works from the terminal's own shell, not inside tmux or screen."))

    case "split":
      guard let terminal, let location = tabManager.location(ofTerminal: terminal) else {
        return reply(ControlResponse(ok: false, message: "This terminal can't be split."))
      }
      tabManager.reveal(location)
      let container = tabManager.makeTerminalContainer(
        directory: cwd ?? NSHomeDirectory(), initialCommand: args["command"])
      tabManager.splitSelectedTab(
        with: .terminal(container), axis: args["direction"] == "down" ? .vertical : .horizontal)
      reply(ControlResponse(ok: true))

    case "tab":
      tabManager.addTerminalTab(directory: cwd ?? NSHomeDirectory(), initialCommand: args["command"])
      reply(ControlResponse(ok: true))

    case "status" where args["via"] == "mcp" && terminal?.agentReportsByHook == true:
      // Hooks are exact: an MCP status mustn't argue with them.
      reply(
        ControlResponse(
          ok: true, message: "Impulse's hooks report this agent's state, so set_status changed nothing."))

    case "notify", "status":
      guard let terminal, let response = Self.handlePaneControl(request, terminal: terminal) else {
        return reply(insideTerminal)
      }
      reply(response)

    case "hook":
      guard let terminal else { return reply(ControlResponse(ok: true)) }  // not ours: ignore quietly
      _ = Self.handlePaneControl(request, terminal: terminal)
      // What the agent should know (other tasks, shared files, dependency
      // changes), or a question before it merges a branch still moving.
      guard args["agent"] == "claude" else { return reply(ControlResponse(ok: true)) }
      var request = request
      // From outside the pane's shell, the hook's folder isn't where git
      // runs: the pane's is.
      if !fromShell { request.arguments["hookCwd"] = nil }
      agentHookReply(request, terminal: terminal) { text in reply(ControlResponse(ok: true, message: text)) }

    case "tasks":
      guard let cwd else { return reply(insideTerminal) }
      handleTasksCommand(request, cwd: cwd, terminal: terminal, caller: caller, reply: reply)

    case "turns", "comments", "problems", "blocks", "unsaved", "project", "mcp-tools":
      handleWorkspaceCommand(request, terminal: terminal, cwd: cwd, reply: reply)

    case "checkpoint":
      guard let cwd else { return reply(insideTerminal) }
      DispatchQueue.global(qos: .userInitiated).async {
        guard let root = GitClient.repoRoot(forPath: cwd) else {
          return reply(ControlResponse(ok: false, message: "Not in a git repository."))
        }
        let reason = args["message"].flatMap { $0.isEmpty ? nil : $0 } ?? "manual checkpoint"
        switch SafetySnapshots.create(
          reason: reason, root: root, prefix: SafetySnapshots.checkpointPrefix + "manual/")
        {
        case .success(let snapshot):
          reply(ControlResponse(ok: true, message: snapshot.ref))
          AgentCheckpoints.shared.prune(root: root)
        case .failure(let error):
          reply(ControlResponse(ok: false, message: error.message))
        }
      }

    default:
      reply(ControlResponse(ok: false, message: "impulse: unknown command '\(request.command)'"))
    }
  }

  /// `notify`, `status`, and what a hook says about the agent's state: they
  /// concern the pane alone, so they need no window (the Quick Terminal's
  /// work with none open). Nil for any other command.
  static func handlePaneControl(_ request: ControlRequest, terminal: TerminalTab) -> ControlResponse? {
    let args = request.arguments
    switch request.command {
    case "notify":
      terminal.setAttentionFromAgent()
      NotificationCenter.default.post(
        name: .terminalWantsNotification, object: terminal,
        userInfo: ["title": args["title"] ?? terminal.tabTitle, "body": args["message"] ?? ""])
      return ControlResponse(ok: true)

    case "status":
      let state: AgentState
      switch args["state"] {
      case "working": state = .working
      case "waiting": state = .needsInput
      case "done": state = .done
      default: state = .idle
      }
      // Each report replaces the message (none clears it).
      let message = args["message"].flatMap { $0.isEmpty ? nil : $0 }
      let messageChanged = terminal.agentStatusMessage != message
      terminal.agentStatusMessage = message
      terminal.agentHook(.status(state), agentID: nil)
      if messageChanged { NotificationCenter.default.post(name: .terminalAgentChanged, object: terminal) }
      // Typed at a prompt, the report ends with this very command.
      let command = terminal.currentCommand ?? ""
      let words = command.split(separator: " ")
      let alone =
        args["via"] != "mcp" && words.count >= 2 && (words[0] == "impulse" || words[0].hasSuffix("/impulse")) && words[1] == "status"
        && !command.contains(where: { ";&|".contains($0) })
      return ControlResponse(
        ok: true,
        message: alone
          ? "impulse status lasts only while the program that runs it does, so at a prompt it ends right away. Call it from a script or wrapper running in this pane."
          : nil)

    case "hook":
      let agentID = args["agent"]
      let hook: AgentHook?
      switch args["event"] {
      case "SessionStart": hook = .sessionStarted
      case "UserPromptSubmit": hook = .promptSubmitted
      case "PostToolUse": hook = .usedTool(args["tool"])
      case "Stop", "agent-turn-complete": hook = .stopped
      case "Notification": hook = .notification(args["message"] ?? "")
      default: hook = nil  // other events (tools starting, subagents, compaction) don't change state
      }
      if let hook { terminal.agentHook(hook, agentID: agentID) }
      terminal.agentReportsByHook = true
      if let agentID, let session = args["session"], !session.isEmpty {
        terminal.agentSession = (agentID, session)
      }
      return ControlResponse(ok: true)

    default:
      return nil
    }
  }

  /// `impulse edit`: answer when the file's editor tab closes, matched by
  /// the tab (so Save As doesn't lose it), or right away when the file
  /// didn't open (not UTF-8, unreadable, too large).
  private func replyWhenEditorCloses(
    path: String, caller: ControlCaller, reply: @escaping (ControlResponse) -> Void
  ) {
    guard let editor = findEditorTab(forPath: path) else {
      return reply(ControlResponse(ok: false, message: "Impulse couldn't open \(path) in the editor."))
    }
    let observer = ObserverToken()
    observer.token = NotificationCenter.default.addObserver(
      forName: .impulseEditorClosed, object: editor, queue: .main
    ) { _ in
      observer.remove()
      reply(ControlResponse(ok: true))
    }
    // ⌃C in the waiting `impulse edit`: stop watching the tab.
    caller.onHangUp = { observer.remove() }
  }

  /// Whether `path`, or what it resolves to, is in /dev.
  private static func isUnderDev(_ path: String) -> Bool {
    var resolved = path
    if let real = realpath(path, nil) {
      resolved = String(cString: real)
      free(real)
    }
    return [path, resolved].contains { $0 == "/dev" || $0.hasPrefix("/dev/") }
  }

  /// The file type bits of `path`, following symlinks (0 when it can't be
  /// read).
  private static func fileType(_ path: String) -> mode_t {
    var info = stat()
    guard stat(path, &info) == 0 else { return 0 }
    return info.st_mode & S_IFMT
  }
}

/// Holds a block observer's token so the block can remove itself.
private final class ObserverToken: @unchecked Sendable {
  var token: NSObjectProtocol?

  func remove() {
    if let token { NotificationCenter.default.removeObserver(token) }
    token = nil
  }
}
