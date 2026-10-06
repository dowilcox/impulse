import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

extension MainWindowController {

  // MARK: - Workbench actions

  func showCommandPalette() {
    showPalette(prefix: ">")
  }

  /// Open the palette with a mode prefix: "" files, ">" commands, ":" line,
  /// "%" text, "b:" branches, "t:" tabs.
  /// Run the palette command `id` (menu items and keybindings that map to
  /// a command rather than an action of their own).
  func runCommand(id: String) {
    CommandRegistry.commands(for: self, customKeybindings: settings.customKeybindings)
      .first { $0.id == id }?.action()
  }

  func showPalette(prefix: String) {
    guard let window else { return }
    let model = palette.model
    model.host = self
    model.iconCache = tabManager.iconCache
    model.shortcutOverrides = settings.keybindingOverrides
    model.commands = CommandRegistry.commands(
      for: self, customKeybindings: settings.customKeybindings)
    palette.show(in: window, prefix: prefix, palette: windowModel.palette)
  }

  /// Add the shells' own history files to Impulse's history.
  func importShellHistory() {
    CommandHistory.shared.importShellHistory { [weak self] count in
      self?.toasts.show(
        Toast(
          kind: .success,
          message: count == 0
            ? "No zsh, bash or fish history found." : "Imported \(count) commands from shell history."))
    }
  }

  /// Move the input bar into the focused terminal, carrying each terminal's
  /// unsent draft with it. Outside a terminal the bar leaves the hierarchy.
  func attachInputBar() {
    guard let host = inputBarHost else { return }
    let container = tabManager.selectedTerminal
    let terminal = container?.activeTerminal
    if terminal !== inputBarTerminal {
      inputBarTerminal?.inputDraft = windowModel.inputDraft
      windowModel.inputDraft = terminal?.inputDraft ?? ""
      windowModel.inputDraftRestoreToken += 1
      inputBarTerminal = terminal
    }
    if let container {
      container.attachAccessory(host)
    } else {
      host.removeFromSuperview()
    }
  }

  /// Look for dev servers every few seconds while the window is visible:
  /// listening ports of anything running in the window's terminals.
  func startPortScanning() {
    portTimer?.invalidate()
    portTimer = Timer.scheduledTimer(withTimeInterval: 3, repeats: true) { [weak self] _ in
      guard let self, let window = self.window, window.isVisible, !window.isMiniaturized else { return }
      let trees = self.tabManager.processTrees()
      DispatchQueue.global(qos: .utility).async { [weak self] in
        var ports: [UUID: [ListeningPort]] = [:]
        for tree in trees where !tree.pids.isEmpty {
          ports[tree.workspace] = PortScanner.listeningPorts(of: tree.pids)
        }
        DispatchQueue.main.async { self?.tabManager.setPorts(ports) }
      }
    }
  }

  /// Send text to the agent most likely meant: one waiting on the user in
  /// the active workspace first, then any other agent. Queued while busy.
  func sendToBestAgent(_ text: String, excluding source: TerminalTab? = nil) {
    let workspaceName = tabManager.activeWorkspace.name
    let candidates = windowModel.agents.filter { $0.id != source?.id && $0.state != .exited }
    let target =
      candidates.first { $0.workspaceName == workspaceName && $0.state.wantsUser }
      ?? candidates.first { $0.workspaceName == workspaceName }
      ?? candidates.first
    guard let target else {
      toasts.show(Toast(kind: .info, message: "No agent is running in this window."))
      return
    }
    sendToAgent(text, terminalID: target.id)
  }

  /// The editor's selection, with where it came from, to an agent.
  func sendEditorSelectionToAgent() {
    guard let editor = tabManager.selectedEditor, let webView = editor.webView else {
      toasts.show(Toast(kind: .info, message: "Select some code in an editor first."))
      return
    }
    let script = """
      (function(){const s=editor.getSelection();const m=editor.getModel();
      return JSON.stringify({text:m.getValueInRange(s),start:s.startLineNumber,end:s.endLineNumber,
      language:m.getLanguageId()});})()
      """
    webView.evaluateJavaScript(script) { [weak self] result, _ in
      guard let self, let json = result as? String,
        let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
        let text = object["text"] as? String, !text.isEmpty
      else {
        self?.toasts.show(Toast(kind: .info, message: "Select some code in the editor first."))
        return
      }
      let start = object["start"] as? Int ?? 1
      let end = object["end"] as? Int ?? start
      let language = object["language"] as? String ?? ""
      let path = editor.filePath.map { self.relativeToWorkspace($0) } ?? "an unsaved file"
      let lines = start == end ? "line \(start)" : "lines \(start)–\(end)"
      self.sendToBestAgent("In @\(path), \(lines):\n```\(language)\n\(text)\n```\n")
    }
  }

  /// "@path" relative to the active workspace (or repository) root.
  func relativeToWorkspace(_ path: String) -> String {
    let roots = [tabManager.activeWorkspace.kind == .folder ? tabManager.activeWorkspace.root : nil,
      windowModel.repository?.root, fileTreeRootPath].compactMap { $0 }
    for root in roots where path.hasPrefix(root + "/") {
      return String(path.dropFirst(root.count + 1))
    }
    return path
  }

  /// ⌘I: a prompt editor over the program in the focused terminal (an
  /// agent's TUI). Closes again when already open.
  func toggleAgentComposer() {
    guard let terminal = tabManager.selectedTerminal?.activeTerminal else {
      toasts.show(Toast(kind: .info, message: "The composer writes to the program in a terminal."))
      return
    }
    guard terminal.isDirectInteraction else {
      // The input bar is already the place to type.
      windowModel.inputBarFocusToken += 1
      return
    }
    windowModel.composerTarget = terminal.agent?.displayName ?? ""
    windowModel.composerVisible.toggle()
    if windowModel.composerVisible {
      windowModel.composerFocusToken += 1
    } else {
      terminal.focus()
    }
  }

  /// Bring a terminal forward: its window, workspace, tab and pane.
  @discardableResult
  func revealTerminal(id: UUID) -> Bool {
    guard
      let location = tabManager.locate(where: {
        if case .terminal(let container) = $0 { return container.activeTerminal?.id == id }
        return false
      })
    else { return false }
    window?.makeKeyAndOrderFront(nil)
    tabManager.reveal(location)
    return true
  }

  /// History of the window's repository, or of one file or folder in it.
  func showHistory(path absolutePath: String? = nil) {
    let anchor = absolutePath ?? windowModel.repository?.root ?? fileTreeRootPath
    GitRepositoryStore.shared.resolve(directory: anchor) { [weak self] state in
      guard let self else { return }
      guard let state else {
        self.toasts.show(Toast(kind: .info, message: "Not in a git repository."))
        return
      }
      var relative: String?
      if let absolutePath, absolutePath != state.root, absolutePath.hasPrefix(state.root + "/") {
        relative = String(absolutePath.dropFirst(state.root.count + 1))
      }
      self.tabManager.addHistoryTab(repository: state, path: relative, host: self)
    }
  }

  /// Act on a hint picked in a terminal: open it (URL or port in the
  /// browser, file in the editor, SHA in History), copy it, or insert it
  /// at the prompt.
  func performHint(_ info: [AnyHashable: Any], terminal: TerminalTab) {
    let text = info["text"] as? String ?? ""
    let path = info["path"] as? String
    switch info["action"] as? String {
    case "copy":
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(path ?? text, forType: .string)
      toasts.show(Toast(kind: .success, message: "Copied \(path.map { ($0 as NSString).lastPathComponent } ?? text)"))
    case "insert":
      if terminal.wantsGridFocus {
        terminal.insertInput(text)
        terminal.focus()
      } else {
        insertIntoInputBar(text)
      }
    default:
      switch TerminalHintMatch.Kind(rawValue: info["kind"] as? String ?? "") {
      case .url, .port:
        let hint = TerminalHintMatch(
          kind: info["kind"] as? String == "port" ? .port : .url, range: 0..<1, text: text)
        if let url = hint.url { NSWorkspace.shared.open(url) }
      case .path:
        if let path {
          paletteOpenFile(
            path, line: (info["line"] as? Int).map(UInt32.init), column: (info["column"] as? Int).map(UInt32.init))
        }
      case .sha:
        let cwd = info["cwd"] as? String ?? fileTreeRootPath
        GitRepositoryStore.shared.resolve(directory: cwd.isEmpty ? fileTreeRootPath : cwd) { [weak self] state in
          guard let self else { return }
          guard let state else {
            self.toasts.show(Toast(kind: .info, message: "\(text) isn't in a git repository here."))
            return
          }
          self.tabManager.addHistoryTab(repository: state, host: self, reveal: text)
        }
      case nil:
        break
      }
    }
  }

  /// The agent's terminal, by terminal id.
  func agentTerminal(id: UUID) -> TerminalTab? {
    for case .terminal(let container) in tabManager.allSurfaces {
      if let terminal = container.activeTerminal, terminal.id == id { return terminal }
    }
    return nil
  }

  /// Open the review on the agent's most recent turn (the focused agent, or
  /// the latest turn in the window's repository).
  func reviewLastAgentTurn(terminalID: UUID? = nil) {
    let id = terminalID ?? tabManager.selectedTerminal?.activeTerminal?.id
    let turn =
      id.flatMap { AgentCheckpoints.shared.lastTurn(terminalID: $0) }
      ?? windowModel.repository.flatMap { AgentCheckpoints.shared.lastTurn(inRepo: $0.root) }
    guard let turn else {
      toasts.show(Toast(kind: .info, message: "No agent turns recorded yet in this repository."))
      return
    }
    GitRepositoryStore.shared.resolve(directory: turn.repoRoot) { [weak self] state in
      guard let self, let state else { return }
      self.tabManager.addReviewTab(repository: state, scope: turn.scope, focusPath: nil, host: self)
    }
  }

  /// Speak an agent needing input or finishing (VoiceOver), once per change.
  func announceAgentState(of terminal: TerminalTab) {
    let state = terminal.agentState
    defer { announcedAgentStates[terminal.id] = state }
    guard NSWorkspace.shared.isVoiceOverEnabled, let state, state != announcedAgentStates[terminal.id],
      let name = terminal.agent?.displayName
    else { return }
    let message: String
    switch state {
    case .needsInput: message = "\(name) needs your input in \(terminal.tabTitle)"
    case .done: message = "\(name) finished in \(terminal.tabTitle)"
    default: return
    }
    NSAccessibility.post(
      element: window as Any, notification: .announcementRequested,
      userInfo: [.announcement: message, .priority: NSAccessibilityPriorityLevel.high.rawValue])
  }

  /// Put the repository's files back to how they were when an agent turn
  /// started (snapshot first, so Undo brings the newer state back).
  func restoreAgentTurn(terminalID: UUID, index: Int) {
    let turns = AgentCheckpoints.shared.turns(terminalID: terminalID)
    guard turns.indices.contains(index) else { return }
    let turn = turns[index]
    gitConfirm(
      title: "Restore files to before turn \(index + 1)?",
      message: "Every file in the repository goes back to how it was when \(turn.agentName) started that turn. Commits are kept, and you can undo this right after.",
      confirmTitle: "Restore", destructive: true
    ) { [weak self] proceed in
      guard proceed, let self else { return }
      GitRepositoryStore.shared.resolve(directory: turn.repoRoot) { [weak self] state in
        guard let self, let state else { return }
        state.run("Restoring…", snapshotReason: "restore agent checkpoint", requireSnapshot: true) {
          SafetySnapshots.restore(turn.start, root: $0)
        } completion: { [weak self] result, snapshot in
          guard let self else { return }
          if case .failure(let error) = result {
            self.gitPresentError(error, title: "Couldn't restore")
            return
          }
          self.toasts.show(
            Toast(
              kind: .success, message: "Restored files to before turn \(index + 1)",
              actionTitle: snapshot == nil ? nil : "Undo",
              action: snapshot.map { snapshot in
                { state.run { SafetySnapshots.restore(snapshot, root: $0) } completion: { _, _ in } }
              }, lifetime: 15))
        }
      }
    }
  }

  /// ⌘⇧U: the next agent waiting on the user (needs input first), cycling
  /// past the one already in front.
  func revealNextWaitingAgent() {
    let waiting = windowModel.agents.filter { $0.state.wantsUser }
    guard !waiting.isEmpty else {
      toasts.show(Toast(kind: .info, message: "No agents are waiting for you."))
      return
    }
    let current = tabManager.selectedTerminal?.activeTerminal?.id
    let start = waiting.firstIndex { $0.id == current }.map { $0 + 1 } ?? 0
    revealTerminal(id: waiting[start % waiting.count].id)
  }

  /// Append text to the input bar's draft (a space apart) and focus it.
  func insertIntoInputBar(_ text: String) {
    let draft = windowModel.inputDraft
    windowModel.inputDraft = draft.isEmpty || draft.hasSuffix(" ") ? draft + text : draft + " " + text
    windowModel.inputDraftRestoreToken += 1
    windowModel.inputBarFocusToken += 1
  }

  /// Split, focus, resize and zoom panes of the selected tab. `command` is
  /// the pane keybinding id ("split_right", "focus_pane_left", …).
  func performPaneCommand(_ command: String) {
    let directions: [String: PaneDirection] = [
      "left": .left, "right": .right, "up": .up, "down": .down,
    ]
    switch command {
    case "split_right", "split_down":
      let container = tabManager.makeTerminalContainer(directory: getActiveCwd())
      tabManager.splitSelectedTab(
        with: .terminal(container), axis: command == "split_right" ? .horizontal : .vertical)
    case "next_pane":
      tabManager.cyclePane(by: 1)
    case "prev_pane":
      tabManager.cyclePane(by: -1)
    case "zoom_pane":
      tabManager.toggleZoomSelectedPane()
    case "equalize_panes":
      tabManager.equalizeSelectedPanes()
    case "move_pane_to_tab":
      tabManager.movePaneToNewTab()
    default:
      if command.hasPrefix("focus_pane_"),
        let direction = directions[String(command.dropFirst("focus_pane_".count))]
      {
        if !tabManager.focusNeighborPane(direction) { NSSound.beep() }
      } else if command.hasPrefix("resize_pane_"),
        let direction = directions[String(command.dropFirst("resize_pane_".count))]
      {
        tabManager.resizeFocusedPane(toward: direction)
      }
    }
  }
}
