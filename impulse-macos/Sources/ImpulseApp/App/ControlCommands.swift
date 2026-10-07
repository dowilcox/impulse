import AppKit
import ImpulseGit
import ImpulseKit
import ImpulseProtocol

// What `impulse` subcommands do in a window. The calling pane (found by its
// token) is the anchor: files open in its window, splits split it, hooks
// and statuses describe the agent running in it.

extension MainWindowController {
  /// The terminal with a control token, if it's in this window.
  func terminal(controlToken token: String) -> TerminalTab? {
    for case .terminal(let container) in tabManager.allSurfaces {
      if let terminal = container.activeTerminal, terminal.controlToken == token { return terminal }
    }
    return nil
  }

  func handleControl(
    _ request: ControlRequest, terminal: TerminalTab?, reply: @escaping (ControlResponse) -> Void
  ) {
    let args = request.arguments
    let cwd = request.cwd ?? terminal?.currentWorkingDirectory ?? NSHomeDirectory()

    switch request.command {
    case "open":
      guard let path = args["path"] else { return reply(ControlResponse(ok: false, message: "No file given.")) }
      guard FileManager.default.fileExists(atPath: path) || request.wait else {
        return reply(ControlResponse(ok: false, message: "No such file: \(path)"))
      }
      if !FileManager.default.fileExists(atPath: path) {
        // `impulse edit new-file`: start it empty.
        FileManager.default.createFile(atPath: path, contents: nil)
      }
      window?.makeKeyAndOrderFront(nil)
      NSApp.activate(ignoringOtherApps: true)
      // `impulse open .`: a folder opens as a workspace.
      var isDirectory: ObjCBool = false
      if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue {
        tabManager.openWorkspace(folder: path)
        return reply(ControlResponse(ok: true))
      }
      // Images and binaries never get an editor tab to close: don't wait.
      if request.wait, TabManager.isImageFile(path) || TabManager.isBinaryFile(path) {
        openFile(path: path)
        return reply(ControlResponse(ok: true))
      }
      let line = args["line"].flatMap(UInt32.init)
      let column = args["column"].flatMap(UInt32.init)
      tabManager.addEditorTab(
        path: path, projectDirectory: fileTreeRootPath, goToLine: line,
        goToColumn: line == nil ? nil : (column ?? 1))
      guard request.wait else { return reply(ControlResponse(ok: true)) }
      // Answer when the tab closes (for $EDITOR=impulse edit).
      let observer = ObserverToken()
      observer.token = NotificationCenter.default.addObserver(
        forName: .impulseEditorClosed, object: nil, queue: .main
      ) { notification in
        guard notification.userInfo?["path"] as? String == path, let token = observer.token else { return }
        NotificationCenter.default.removeObserver(token)
        observer.token = nil
        reply(ControlResponse(ok: true))
      }

    case "review":
      if args["scope"] == "last-turn" {
        reviewLastAgentTurn(terminalID: terminal?.id)
        return reply(ControlResponse(ok: true))
      }
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
        self.window?.makeKeyAndOrderFront(nil)
        self.tabManager.addReviewTab(repository: state, scope: scope, focusPath: nil, host: self)
        reply(ControlResponse(ok: true))
      }

    case "split" where terminal == nil, "tab" where terminal == nil:
      // These start commands, which then run as Impulse (with whatever
      // privacy access it has), so only a pane's own token may ask — not
      // any process that can reach the socket.
      reply(ControlResponse(ok: false, message: "Run this inside an Impulse terminal."))

    case "split":
      if let terminal, let location = tabManager.location(ofTerminal: terminal) {
        tabManager.reveal(location)
      }
      let container = tabManager.makeTerminalContainer(directory: cwd, initialCommand: args["command"])
      tabManager.splitSelectedTab(
        with: .terminal(container), axis: args["direction"] == "down" ? .vertical : .horizontal)
      reply(ControlResponse(ok: true))

    case "tab":
      tabManager.addTerminalTab(directory: cwd, initialCommand: args["command"])
      reply(ControlResponse(ok: true))

    case "notify":
      guard let terminal else { return reply(ControlResponse(ok: false, message: "Run this inside an Impulse terminal.")) }
      terminal.setAttentionFromAgent()
      NotificationCenter.default.post(
        name: .terminalWantsNotification, object: terminal,
        userInfo: ["title": args["title"] ?? terminal.tabTitle, "body": args["message"] ?? ""])
      reply(ControlResponse(ok: true))

    case "status":
      guard let terminal else { return reply(ControlResponse(ok: false, message: "Run this inside an Impulse terminal.")) }
      let hook: AgentHook
      switch args["state"] {
      case "working": hook = .promptSubmitted
      case "waiting": hook = .notification("needs your input " + (args["message"] ?? ""))
      case "done": hook = .stopped
      default: hook = .sessionStarted
      }
      // Each report replaces the message (none clears it).
      let message = args["message"].flatMap { $0.isEmpty ? nil : $0 }
      let messageChanged = terminal.agentStatusMessage != message
      terminal.agentStatusMessage = message
      terminal.agentHook(hook, agentID: nil)
      if messageChanged { NotificationCenter.default.post(name: .terminalAgentChanged, object: terminal) }
      // Typed at a prompt, the report ends with this very command.
      let command = terminal.currentCommand ?? ""
      let words = command.split(separator: " ")
      let alone =
        words.count >= 2 && (words[0] == "impulse" || words[0].hasSuffix("/impulse")) && words[1] == "status"
        && !command.contains(where: { ";&|".contains($0) })
      reply(
        ControlResponse(
          ok: true,
          message: alone
            ? "impulse status lasts only while the program that runs it does, so at a prompt it ends right away. Call it from a script or wrapper running in this pane."
            : nil))

    case "hook":
      guard let terminal else { return reply(ControlResponse(ok: true)) }  // not ours: ignore quietly
      let agentID = args["agent"]
      let message = args["message"] ?? ""
      let hook: AgentHook?
      switch args["event"] {
      case "SessionStart": hook = .sessionStarted
      case "UserPromptSubmit": hook = .promptSubmitted
      case "Stop", "agent-turn-complete": hook = .stopped
      case "Notification": hook = .notification(message)
      default: hook = nil  // other events (tool use, subagents) don't change state
      }
      if let hook { terminal.agentHook(hook, agentID: agentID) }
      if let agentID, let session = args["session"], !session.isEmpty {
        terminal.agentSession = (agentID, session)
      }
      reply(ControlResponse(ok: true))

    case "checkpoint":
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
}

/// Holds a block observer's token so the block can remove itself.
private final class ObserverToken: @unchecked Sendable {
  var token: NSObjectProtocol?
}
