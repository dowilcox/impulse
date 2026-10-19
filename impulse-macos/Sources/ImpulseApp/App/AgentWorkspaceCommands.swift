import AppKit
import ImpulseGit
import ImpulseKit
import ImpulseProtocol

// What Impulse knows about a workspace that an agent's shell can't easily
// find out, for `impulse turns|comments|problems|blocks|unsaved|project`
// and the MCP server's tools of the same names: the pane's agent turns,
// the user's review comments, the language servers' problems, the other
// terminals' commands and output, files with unsaved edits, and the
// trusted project settings. Each reads the caller's workspace (its
// repository, terminals, open files), never starts a server or runs a
// project command, and answers with text plus the same as JSON data.

extension MainWindowController {
  func handleWorkspaceCommand(
    _ request: ControlRequest, terminal: TerminalTab?, cwd: String?, reply: @escaping (ControlResponse) -> Void
  ) {
    let json = request.arguments["json"] != nil
    /// Text (or JSON with --json), with the JSON as data either way.
    func answer<T: Encodable>(_ value: T, text: String) {
      reply(ControlResponse(ok: true, message: json ? AgentToolJSON.string(value) : text, data: AgentToolJSON.data(value)))
    }
    let insideTerminal = ControlResponse(ok: false, message: "Run this inside an Impulse terminal.")

    switch request.command {
    case "mcp-tools":
      reply(ControlResponse(ok: true, data: AgentToolJSON.data(["disabled": Self.disabledMCPTools()])))

    case "turns":
      guard let terminal else { return reply(insideTerminal) }
      let turns = AgentCheckpoints.shared.turns(terminalID: terminal.id).map {
        TurnList.Turn(agent: $0.agentName, started: $0.start.date, ended: $0.end?.date, start: $0.start.ref, end: $0.end?.ref)
      }
      let limit = request.arguments["limit"].flatMap(Int.init) ?? TurnList.defaultLimit
      let list = TurnList(latest: turns, limit: limit)
      answer(list, text: list.text())

    case "comments":
      guard let cwd else { return reply(insideTerminal) }
      reviewComments(request, cwd: cwd, terminal: terminal) { result in
        switch result {
        case .success(let list): answer(list, text: list.text())
        case .failure(let failure): reply(ControlResponse(ok: false, message: failure.message))
        }
      }

    case "problems":
      let list = problems(request, terminal: terminal, cwd: cwd)
      answer(list, text: list.text())

    case "blocks":
      switch commandBlocks(request, terminal: terminal) {
      case .success(.list(let list)): answer(list, text: list.text())
      case .success(.output(let output)): answer(output, text: output.text())
      case .failure(let failure): reply(ControlResponse(ok: false, message: failure.message))
      }

    case "unsaved":
      let root = workspaceRoot(of: terminal, cwd: cwd)
      // Outside a workspace (the Quick Terminal): the window's editors
      // under the caller's folder.
      let editors =
        workspaceOf(terminal).map { tabManager.editors(inWorkspace: $0.id) }
        ?? allEditorTabs.filter { $0.filePath?.hasPrefix(root + "/") == true }
      let files = editors.filter(\.isModified).compactMap { editor -> UnsavedList.File? in
        guard let path = editor.filePath else { return nil }
        return UnsavedList.File(path: Self.relative(path, to: root), changedOnDisk: editor.changedOnDisk)
      }
      let list = UnsavedList(files: files)
      answer(list, text: list.text())

    case "project":
      guard let cwd else { return reply(insideTerminal) }
      projectSummary(cwd: cwd) { summary in answer(summary, text: summary.text()) }

    default:
      reply(ControlResponse(ok: false, message: "impulse: unknown command '\(request.command)'"))
    }
  }

  /// The MCP tools the settings turn off.
  static func disabledMCPTools() -> [String] {
    let settings = SettingsStore.shared.settings
    var off: [String] = []
    if AgentTaskPolicy.Setting(setting: settings.agentTaskCreation) == .off { off.append("create_task") }
    if TerminalOutputScope(setting: settings.agentToolTerminalOutput) == .off {
      off += ["list_command_blocks", "get_block_output"]
    }
    return off
  }

  // MARK: Where the caller is

  private func workspaceOf(_ terminal: TerminalTab?) -> Workspace? {
    terminal.flatMap { tabManager.workspace(ofTerminal: $0) }
  }

  /// The caller's workspace folder (a folder workspace's root), else its
  /// repository's, else its own folder.
  private func workspaceRoot(of terminal: TerminalTab?, cwd: String?) -> String {
    if let workspace = workspaceOf(terminal), workspace.kind == .folder { return workspace.root }
    let folder = cwd ?? terminal?.currentWorkingDirectory ?? NSHomeDirectory()
    return GitClient.repoRoot(forPath: folder) ?? folder
  }

  private var allEditorTabs: [EditorTab] {
    tabManager.allSurfaces.compactMap {
      if case .editor(let editor) = $0 { return editor }
      return nil
    }
  }

  static func relative(_ path: String, to root: String) -> String {
    path.hasPrefix(root + "/") ? String(path.dropFirst(root.count + 1)) : path
  }

  struct ToolFailure: Error {
    let message: String
  }

  // MARK: Review comments

  /// The repository's comments (`path` limits them to one file;
  /// `current` drops outdated ones), or marks one addressed (`resolve`).
  private func reviewComments(
    _ request: ControlRequest, cwd: String, terminal: TerminalTab?,
    completion: @escaping (Result<CommentList, ToolFailure>) -> Void
  ) {
    let args = request.arguments
    DispatchQueue.global(qos: .userInitiated).async {
      let root = GitClient.repoRoot(forPath: cwd)
      DispatchQueue.main.async {
        guard let root else { return completion(.failure(ToolFailure(message: "Not in a git repository."))) }
        let store = ReviewCommentStore.forRepository(root)
        if let id = args["resolve"] {
          let note = (args["note"] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
          guard !note.isEmpty else {
            return completion(.failure(ToolFailure(message: "Say what you did in the note.")))
          }
          let by = terminal?.agent?.displayName ?? "An agent"
          guard store.resolve(id: id, resolution: ReviewComment.Resolution(by: by, note: note)) else {
            return completion(.failure(ToolFailure(message: "No review comment \(id) in this repository.")))
          }
          NotificationCenter.default.post(name: .reviewCommentsChanged, object: nil, userInfo: ["root": root])
        }
        var comments = store.comments
        if let id = args["resolve"] { comments = comments.filter { $0.id == id } }
        if let path = args["path"] {
          let relative = Self.relative(path, to: root)
          comments = comments.filter { $0.path == relative }
        }
        let currentOnly = args["current"] != nil
        let name = (root as NSString).lastPathComponent
        // Outdated or not: read each file's current text, off the main
        // thread.
        DispatchQueue.global(qos: .userInitiated).async {
          var texts: [String: String?] = [:]
          for path in Set(comments.filter { $0.side == .new }.map(\.path)) {
            texts[path] = TextFile.read((root as NSString).appendingPathComponent(path))?.text
          }
          let entries = comments.sorted { $0.path != $1.path ? $0.path < $1.path : $0.line < $1.line }.map {
            CommentList.Entry($0, outdated: CommentFreshness.isOutdated($0, currentText: texts[$0.path] ?? nil))
          }.filter { !currentOnly || $0.outdated != true }
          DispatchQueue.main.async { completion(.success(CommentList(repository: name, comments: entries))) }
        }
      }
    }
  }

  // MARK: Problems

  private func problems(_ request: ControlRequest, terminal: TerminalTab?, cwd: String?) -> ProblemList {
    let args = request.arguments
    let root = workspaceRoot(of: terminal, cwd: cwd)
    let minimum: Problem.Severity? = {
      if args["errors"] != nil { return .error }
      switch args["severity"] {
      case "error": return .error
      case "warning": return .warning
      case "info": return .info
      case "hint": return .hint
      default: return nil
      }
    }()
    let all = windowModel.problemsByPath.values.flatMap { $0 }
    let open = lspOpenFiles.map(uriToFilePath)
    let checked = Set(open + windowModel.problemsByPath.keys).filter { $0 == root || $0.hasPrefix(root + "/") }
    var note: String?
    if !Trust.shared.isTrusted(root) {
      note =
        "Language servers don't run in \((root as NSString).lastPathComponent) because the user hasn't trusted it, so there are no diagnostics."
    } else if checked.isEmpty {
      note =
        "No language server has checked a file in this workspace yet: servers check the files open in Impulse's editor (and some, the whole project)."
    }
    return ProblemList(
      problems: all, root: root, path: args["path"], minimum: minimum, checkedFiles: Array(checked), note: note)
  }

  // MARK: Terminal commands

  enum BlockAnswer {
    case list(BlockList)
    case output(BlockOutput)
  }

  /// Which terminals the block tools see (`agent_tool_terminal_output`).
  enum TerminalOutputScope: String {
    case workspace, window, off
    init(setting: String) { self = TerminalOutputScope(rawValue: setting) ?? .workspace }
  }

  /// The id a block tool names a terminal by.
  static func shortID(_ terminal: TerminalTab) -> String {
    String(terminal.id.uuidString.lowercased().prefix(8))
  }

  private func commandBlocks(_ request: ControlRequest, terminal caller: TerminalTab?) -> Result<BlockAnswer, ToolFailure> {
    let scope = TerminalOutputScope(setting: SettingsStore.shared.settings.agentToolTerminalOutput)
    guard scope != .off else {
      return .failure(ToolFailure(message: "Reading other terminals is turned off in Impulse's settings (agent_tool_terminal_output)."))
    }
    // The caller's workspace, or the window; the Quick Terminal is in no
    // window's tabs, so it's never among them (and, in no workspace, sees
    // none unless the setting says the window).
    let terminals: [TerminalTab]
    switch scope {
    case .workspace:
      guard let workspace = workspaceOf(caller) else {
        return .failure(
          ToolFailure(
            message:
              "This terminal isn't in a workspace, so there are no terminals beside it to read (agent_tool_terminal_output is workspace)."
          ))
      }
      terminals = tabManager.terminals(inWorkspace: workspace.id)
    case .window, .off:
      terminals = tabManager.workspaces.flatMap { tabManager.terminals(inWorkspace: $0.id) }
    }

    if let id = request.arguments["show"] {
      guard let parts = BlockList.parse(id),
        let terminal = terminals.first(where: { Self.shortID($0) == parts.terminal }),
        let block = terminal.backend?.commandBlock(id: parts.block)
      else {
        return .failure(ToolFailure(message: "No command block \(id) in this workspace's terminals (list them with list_command_blocks)."))
      }
      let lines = request.arguments["lines"].flatMap(Int.init) ?? BlockOutput.defaultLines
      return .success(
        .output(
          BlockOutput(
            id: id, command: block.command, folder: block.cwd, exitCode: block.exitCode,
            running: block.endedAtMs == nil, output: block.output, lines: lines)))
    }

    let list = BlockList(
      terminals: terminals.map { terminal in
        let blocks = (terminal.backend?.commandBlocks() ?? []).suffix(BlockList.blocksPerTerminal).map { block in
          BlockList.Block(
            id: "\(Self.shortID(terminal)):\(block.id)", command: block.command, folder: block.cwd,
            exitCode: block.exitCode, started: Date(timeIntervalSince1970: TimeInterval(block.startedAtMs) / 1000),
            durationMs: block.endedAtMs.map { $0 >= block.startedAtMs ? $0 - block.startedAtMs : 0 },
            running: block.endedAtMs == nil)
        }
        return BlockList.Terminal(
          id: Self.shortID(terminal), title: terminal.tabTitle, folder: terminal.currentWorkingDirectory,
          isCaller: terminal === caller, agent: terminal.agent?.displayName, blocks: Array(blocks))
      })
    return .success(.list(list))
  }

  // MARK: Project settings

  private func projectSummary(cwd: String, completion: @escaping (ProjectSummary) -> Void) {
    DispatchQueue.global(qos: .userInitiated).async {
      let root = GitClient.repoRoot(forPath: cwd) ?? cwd
      let main = Self.mainCheckoutRoot(of: root)
      let loaded = Self.loadProjectConfig(root: root)
      let slot = TaskRegistryStore.record(forPath: root)?.slot
      let exists = { (name: String) in FileManager.default.fileExists(atPath: (main as NSString).appendingPathComponent(name)) }
      let installs = ProjectDetector.installCommands.filter { exists($0.lockFile) }
      let detected = ProjectSummary.Detected(
        lockFiles: installs.map(\.lockFile), install: installs.map(\.command),
        composeFile: ComposeFile.fileNames.first(where: exists))
      DispatchQueue.main.async { [weak self] in
        let files = (loaded?.sources ?? []).map { Self.relative($0.path, to: main) }
        guard let loaded else {
          return completion(
            ProjectSummary(
              config: nil, trusted: true, files: [], slot: slot, detected: detected,
              note: "No Impulse project settings (.impulse/project.toml) in this repository."))
        }
        guard case .success(let config) = loaded.config else {
          return completion(
            ProjectSummary(
              config: nil, trusted: false, files: files, slot: slot, detected: detected,
              note: "The project settings don't parse, so nothing from them is listed."))
        }
        let trusted = self?.alreadyTrustedProjectConfig(root: root) != nil
        completion(
          ProjectSummary(
            config: config, trusted: trusted, files: files, slot: slot, detected: detected,
            note: trusted
              ? nil
              : "The user hasn't trusted these project settings yet, so their commands aren't listed: only what Impulse found itself."))
      }
    }
  }
}

extension Notification.Name {
  /// Review comments changed outside Review (an agent marked one
  /// addressed); `userInfo["root"]` is the repository.
  static let reviewCommentsChanged = Notification.Name("impulse.reviewCommentsChanged")
}
