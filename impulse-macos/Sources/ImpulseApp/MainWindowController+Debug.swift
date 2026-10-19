import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import WebKit
import os.log

extension MainWindowController {

  // MARK: - Debug Snapshot Actions

  /// Named UI actions for `--impulse-snapshot-actions` (headless visual checks).
  func performDebugAction(_ action: String) {
    switch action {
    case "sidebar": setSidebarVisible(true)
    case "no-sidebar": setSidebarVisible(false)
    case "palette": showPalette(prefix: ">")
    case "quickopen": showPalette(prefix: "")
    case "quickopen-query": showPalette(prefix: "wbv")
    case "branches": showPalette(prefix: "b:")
    case "manage-branches": presentBranchManager()
    case "review": openDiffReview()
    case "search": NotificationCenter.default.post(name: .impulseFindInProject, object: nil)
    case "changes": showChangesPanel()
    case "review-split":
      for case .diffReview(_, let review) in tabManager.allSurfaces { review.setLayout("split") }
    case "review-agent": reviewLastAgentTurn()
    case _ where action.hasPrefix("review-"):
      for case .diffReview(_, let review) in tabManager.allSurfaces {
        review.debugAction(String(action.dropFirst(7)))
      }
    default:
      if action.hasPrefix("open=") {
        openFile(path: debugPath(String(action.dropFirst(5))))
      } else if action.hasPrefix("beside=") {
        let relative = String(action.dropFirst(7))
        let base = DebugSnapshot.initialDirectory ?? fileTreeRootPath
        tabManager.addEditorTab(
          path: (base as NSString).appendingPathComponent(relative),
          projectDirectory: fileTreeRootPath, beside: true)
      } else if action.hasPrefix("workspace=") {
        let relative = String(action.dropFirst(10))
        let base = DebugSnapshot.initialDirectory ?? fileTreeRootPath
        tabManager.openWorkspace(
          folder: relative.hasPrefix("/") ? relative : (base as NSString).appendingPathComponent(relative))
      } else if action == "new-terminal" {
        tabManager.addTerminalTab()
      } else if action == "close-workspace" {
        tabManager.closeWorkspace(tabManager.activeWorkspaceID)
      } else if action.hasPrefix("click="), let window {
        // A click at x:y (points from the window's top-left; snapshot checks).
        let parts = action.dropFirst(6).split(separator: ":").compactMap { Double($0) }
        if parts.count == 2 {
          let point = NSPoint(x: parts[0], y: window.frame.height - parts[1])
          for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            if let event = NSEvent.mouseEvent(
              with: type, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
              windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)
            {
              // Queued, so tracking loops (a table's mouse-down) see the up.
              NSApp.postEvent(event, atStart: false)
            }
          }
        }
      } else if action.hasPrefix("key="), let window {
        // A key press to whatever has the keyboard (snapshot checks).
        let keys: [String: (code: UInt16, chars: String)] = [
          "down": (125, String(UnicodeScalar(NSDownArrowFunctionKey)!)),
          "up": (126, String(UnicodeScalar(NSUpArrowFunctionKey)!)),
        ]
        if let key = keys[String(action.dropFirst(4))],
          let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [.numericPad, .function], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, characters: key.chars,
            charactersIgnoringModifiers: key.chars, isARepeat: false, keyCode: key.code)
        {
          window.sendEvent(event)
        }
      } else if action == "trust-on" {
        // Workspace trust is off in snapshots; turn it on to see restricted mode.
        Trust.enabledForSnapshot = true
        Trust.shared.isEnabled = true
        MainWindowController.trustDidChange()
      } else if action == "trust-after-sheet", let window {
        // What the Open panel does: its completion opens the folder while
        // the panel sheet is still closing.
        Trust.enabledForSnapshot = true
        Trust.shared.isEnabled = true
        let folder = trustFolderForActiveContext() ?? DebugSnapshot.initialDirectory ?? fileTreeRootPath
        let placeholder = NSAlert()
        placeholder.messageText = "Choose a folder"
        placeholder.beginSheetModal(for: window) { [weak self] _ in self?.presentTrustPrompt(for: folder) }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { window.endSheet(placeholder.window) }
      } else if action == "trust-prompt" {
        Trust.enabledForSnapshot = true
        Trust.shared.isEnabled = true
        presentTrustPrompt(for: trustFolderForActiveContext() ?? DebugSnapshot.initialDirectory ?? fileTreeRootPath)
      } else if action.hasPrefix("focus-pane="), let id = Int(action.dropFirst(11)) {
        tabManager.focusPane(id, inTabAt: tabManager.selectedIndex)
      } else if action == "expand-workspaces" {
        for workspace in tabManager.workspaces {
          tabManager.setWorkspaceExpanded(workspace.id, true)
        }
      } else if action.hasPrefix("setting-on="),
        let item = SettingsCatalog.items.first(where: { $0.key == String(action.dropFirst(11)) }),
        case .toggle(let keyPath) = item.control
      {
        // In memory only: snapshot runs never save settings.
        SettingsStore.shared.settings[keyPath: keyPath] = true
      } else if action.hasPrefix("setting="), let eq = action.dropFirst(8).firstIndex(of: "="),
        let item = SettingsCatalog.items.first(where: { $0.key == String(action[action.index(action.startIndex, offsetBy: 8)..<eq]) })
      {
        // Text and folder settings, in memory only.
        let value = String(action[action.index(after: eq)...])
        switch item.control {
        case .text(let keyPath, _), .folder(let keyPath, _): SettingsStore.shared.settings[keyPath: keyPath] = value
        default: break
        }
      } else if action.hasPrefix("command=") {
        runCommand(id: String(action.dropFirst(8)))
      } else if action == "tag-sheet" {
        createTagAtHead()
      } else if action.hasPrefix("tag=") || action.hasPrefix("tag-push=") {
        // Tag HEAD without the sheet (name[:message]); tag-push= pushes it too.
        let push = action.hasPrefix("tag-push=")
        let parts = action.drop { $0 != "=" }.dropFirst().split(separator: ":", maxSplits: 1).map(String.init)
        repositoryActions()?.createTag(parts[0], message: parts.count > 1 ? parts[1] : nil, push: push)
      } else if action.hasPrefix("theme=") {
        // This window only; settings are left alone.
        handleThemeChange(ThemeManager.theme(forName: String(action.dropFirst(6))))
      } else if action == "history" {
        showHistory()
      } else if action == "history-all" {
        // History with every branch (snapshot checks).
        for case .history(_, let view) in tabManager.allSurfaces {
          view.model.scope = .all
          view.model.reload()
        }
      } else if action.hasPrefix("history-filter="), let repository = windowModel.repository {
        tabManager.addHistoryTab(repository: repository, host: self)
        if case let index = tabManager.selectedIndex, tabManager.tabs.indices.contains(index),
          case .history(_, let view) = tabManager.tabs[index].focused
        {
          view.model.filter = String(action.dropFirst(15))
        }
      } else if action.hasPrefix("history-scroll="), let y = Double(action.dropFirst(15)) {
        for case .history(_, let view) in tabManager.allSurfaces { view.debugScrollChanges(toY: y) }
      } else if action.hasPrefix("history="), let repository = windowModel.repository {
        tabManager.addHistoryTab(repository: repository, host: self, reveal: String(action.dropFirst(8)))
      } else if action.hasPrefix("replace="), let colon = action.firstIndex(of: ":") {
        windowModel.beginSearch()
        windowModel.searchQuery = String(action[action.index(action.startIndex, offsetBy: 8)..<colon])
        windowModel.searchReplacement = String(action[action.index(after: colon)...])
        windowModel.searchReplaceVisible = true
        windowModel.runSearchNow()
      } else if action == "quick-terminal" {
        QuickTerminal.shared.toggle()

      } else if action == "preview-beside" {
        togglePreviewBeside()
      } else if action == "diff-view" {
        toggleDiffView()
      } else if action.hasPrefix("workspace-edit="), let editor = tabManager.selectedEditor,
        let path = editor.filePath
      {
        // An edit to the open file plus one on disk.
        let other = (fileTreeRootPath as NSString).appendingPathComponent(String(action.dropFirst(15)))
        let edit = WorkspaceEdit(operations: [
          .edit(
            uri: URL(fileURLWithPath: path).absoluteString,
            edits: [LSPTextEdit(startLine: 0, startCharacter: 0, endLine: 0, endCharacter: 0, newText: "// edited\n")]),
          .edit(
            uri: URL(fileURLWithPath: other).absoluteString,
            edits: [LSPTextEdit(startLine: 0, startCharacter: 0, endLine: 0, endCharacter: 0, newText: "edited ")]),
        ])
        applyWorkspaceEdit(edit, root: fileTreeRootPath, title: "Rename") { [weak self] outcome in
          self?.reportWorkspaceEdit(outcome, verb: "Renamed")
        }
      } else if action.hasPrefix("git-peek="), let line = Int(action.dropFirst(9)) {
        // What clicking the change mark at that line does.
        tabManager.selectedEditor?.webView?.evaluateJavaScript("renderGitPeek(gitHunkAtLine(\(line)))")
      } else if action.hasPrefix("tree-expand=") {
        // A folder in the file tree, by root-relative path (its parent must be open).
        let path = (fileTreeRootPath as NSString).appendingPathComponent(String(action.dropFirst(12)))
        if let node = windowModel.flatFileTree.first(where: { $0.node.path == path })?.node {
          windowModel.expandDirectory(node)
        }
      } else if action.hasPrefix("search=") {
        windowModel.beginSearch()
        windowModel.searchQuery = String(action.dropFirst(7))
        windowModel.runSearchNow()
      } else if action.hasPrefix("task-sheet=") {
        // The New Task sheet filled in: `title[|command[|base]]`.
        let parts = action.dropFirst(11).split(separator: "|", omittingEmptySubsequences: false).map(String.init)
        presentNewTaskSheet(
          title: parts.first ?? "", command: parts.count > 1 ? parts[1] : "", base: parts.count > 2 ? parts[2] : nil)
      } else if action == "project-trust" {
        // The trust question for the active workspace's .impulse/project.toml.
        trustProjectConfig(root: tabManager.activeWorkspace.root) { _ in }
      } else if action.hasPrefix("select-workspace=") {
        // By name, as the sidebar shows it.
        let name = String(action.dropFirst(17))
        if let workspace = tabManager.workspaces.first(where: { $0.name == name }) {
          tabManager.activateWorkspace(workspace.id)
        }
      } else if action.hasPrefix("problems=") {
        // A JSON list of {path (root-relative), line, column, severity, message, source, code?}.
        let data = FileManager.default.contents(atPath: String(action.dropFirst(9))) ?? Data()
        let list = (try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]) ?? []
        var byPath: [String: [Problem]] = [:]
        for item in list {
          guard let relative = item["path"] as? String, let message = item["message"] as? String else { continue }
          let path = (fileTreeRootPath as NSString).appendingPathComponent(relative)
          let severity: Problem.Severity =
            switch item["severity"] as? String {
            case "error": .error
            case "warning": .warning
            case "hint": .hint
            default: .info
            }
          byPath[path, default: []].append(
            Problem(
              path: path, line: item["line"] as? Int ?? 1, column: item["column"] as? Int ?? 1,
              severity: severity, message: message, source: item["source"] as? String,
              code: item["code"] as? String))
        }
        windowModel.problemsByPath = byPath
        showProblems()
      } else if action == "problems" {
        windowModel.problemsByPath = [
          (fileTreeRootPath as NSString).appendingPathComponent("search.swift"): [
            Problem(
              path: (fileTreeRootPath as NSString).appendingPathComponent("search.swift"), line: 3, column: 10,
              severity: .error, message: "Value of optional type '[String]?' must be unwrapped", source: "sourcekit"),
            Problem(
              path: (fileTreeRootPath as NSString).appendingPathComponent("search.swift"), line: 2, column: 7,
              severity: .warning, message: "Initialization of immutable value 'words' was never used",
              source: "sourcekit"),
          ],
          (fileTreeRootPath as NSString).appendingPathComponent("main.swift"): [
            Problem(
              path: (fileTreeRootPath as NSString).appendingPathComponent("main.swift"), line: 1, column: 1,
              severity: .info, message: "Consider using 'let'", source: "swiftlint", code: "prefer_let"),
          ],
        ]
        showProblems()
      } else if action == "keybindings" {
        openKeybindings()
      } else if action == "settings" {
        openSettings()
      } else if action.hasPrefix("settings=") {
        openSettings(query: String(action.dropFirst(9)))
      } else if action.hasPrefix("settings-category="),
        let category = SettingItem.Category(rawValue: String(action.dropFirst(18)))
      {
        openSettings()
        if case .tool(let view) = tabManager.selectedTab?.focused, let settings = view as? SettingsSurface {
          settings.model.category = category
        }
      } else if action.hasPrefix("find=") {
        if !termSearchBarVisible { toggleTerminalSearch() }
        termFind.query.text = String(action.dropFirst(5))
      } else if action == "find-word" {
        termFind.query.wholeWord = true
      } else if action.hasPrefix("block="), let terminal = tabManager.selectedTerminal?.activeTerminal {
        terminal.performBlockCommand(String(action.dropFirst(6)))
      } else if action.hasPrefix("select-blocks="), let terminal = tabManager.selectedTerminal?.activeTerminal {
        terminal.beginBlockSelection()
        for _ in 1..<max(1, Int(action.dropFirst(14)) ?? 1) { terminal.handleBlockSelectionKey(.up(extend: true)) }
      } else if action.hasPrefix("composer=") {
        windowModel.composerDraft = String(action.dropFirst(9))
        toggleAgentComposer()
      } else if action == "complete" {
        windowModel.completionRequestToken += 1
      } else if action.hasPrefix("draft=") {
        windowModel.inputDraft = String(action.dropFirst(6)).replacingOccurrences(of: "\\n", with: "\n")
        windowModel.inputDraftRestoreToken += 1
      } else if action.hasPrefix("palette=") {
        showPalette(prefix: String(action.dropFirst(8)))
      } else if action.hasPrefix("run=") {
        tabManager.selectedTerminal?.activeTerminal?.runCommand(String(action.dropFirst(4)))
      } else if action.hasPrefix("task=") {
        debugCreateTask(title: String(action.dropFirst(5)), command: "echo task ready")
      } else if action.hasPrefix("move-task=") {
        debugCreateTask(title: String(action.dropFirst(10)), command: "", moving: true)
      } else if action == "move-sheet" {
        presentNewTaskSheet(title: "Move my work", movingChanges: true)
      } else if action == "archive-merged" {
        presentArchiveMergedTasks(from: tabManager.activeWorkspaceID)
      } else if action == "trust-project-settings" {
        // Trust the active workspace's project settings without asking.
        let root = tabManager.activeWorkspace.root
        if let loaded = Self.loadProjectConfig(root: root) {
          var trust = ProjectTrustStore.current
          for source in loaded.sources {
            trust.trust(root: Self.trustKey(for: source, in: loaded, root: root), digest: source.digest)
          }
          ProjectTrustStore.current = trust
        }
      } else if action == "finish-task" {
        openFinishTask()
      } else if action.hasPrefix("finish-run") {
        // Press Finish (or Continue); `finish-run=review` answers the
        // landing question first.
        for case .tool(let view) in tabManager.allSurfaces {
          guard let model = (view as? TaskFinishSurface)?.model else { continue }
          if let landing = ProjectConfig.Landing(rawValue: String(action.dropFirst(11))) { model.chosenLanding = landing }
          model.onFinish?()
        }
      } else if action == "project-setup" {
        openProjectSetup()
      } else if action == "project-setup-save" {
        // Save what Project Setup proposed, which trusts it.
        for case .tool(let view) in tabManager.allSurfaces {
          (view as? ProjectSetupSurface)?.model.onSave?(nil)
        }
      } else if action == "agent-submit" {
        tabManager.selectedTerminal?.activeTerminal?.agentEvent(.submit)
      } else if action == "close-pane" {
        requestCloseFocusedPane()
      } else if action == "reopen" {
        tabManager.reopenLastClosedTab()
      } else if action.hasPrefix("preview=") {
        // What a single click in the file tree does (snapshot windows aren't key).
        tabManager.addEditorTab(
          path: (fileTreeRootPath as NSString).appendingPathComponent(String(action.dropFirst(8))),
          projectDirectory: fileTreeRootPath, preview: true)
      } else if action.hasPrefix("gallery") {
        // gallery, gallery=light, gallery=dark, gallery=light+contrast …
        let options = action.split(separator: "=").dropFirst().first.map(String.init) ?? ""
        ComponentGalleryWindowController.show(
          light: options.contains("light") ? true : options.contains("dark") ? false : nil,
          increaseContrast: options.contains("contrast"))
      } else if action == "vim" {
        var options = EditorOptions()
        options.vimMode = true
        tabManager.selectedEditor?.applySettings(options)
      } else if action == "undo" {
        // What Edit ▸ Undo ends up sending (snapshot windows are never key,
        // so straight to the window).
        _ = window?.perform(Selector(("undo:")), with: nil)
      } else if action == "newtab" {
        tabManager.addTerminalTab()
      } else if action.hasPrefix("pane=") {
        performPaneCommand(String(action.dropFirst(5)))
      } else {
        NSLog("DebugSnapshot: unknown action '\(action)'")
      }
    }
  }

  // MARK: - Smoke Checks

  /// `performDebugAction` for the smoke checks (scripts/smoke/run.py), with
  /// actions that take time: `done` runs when the action has finished.
  ///
  ///     sleep=<seconds>
  ///     until=[<seconds>:]<condition>   wait until it holds (30 s by default; see
  ///                                     `debugCondition`); a timeout is reported
  ///                                     as a failure and the run goes on
  ///     js=<label>:<body>               an async function body run in the selected
  ///                                     editor's page; its result is reported
  ///     dump=<name>                     the window's state, as <name>.json
  ///     save                            save the selected editor as ⌘S does
  ///                                     (reports save:<file name> when written)
  ///     write-file=<path>|<text>        rewrite a file in place, as `>` does
  ///     read-file=<label>|<path>        report a file's text (null if missing)
  ///     sheet-button=<title>            press a button in a sheet (or a window)
  ///     task-sheet-create, task-sheet-cancel
  ///                                     answer the New Task sheet that's up
  ///     agent-sheet=<hooks|mcp>:<claude|codex|other>[:project]
  ///                                     open Agent Integration on that choice
  ///     agent-sheet-apply               press its Install/Remove (waits for it;
  ///                                     reports agent-sheet:<outcome>)
  ///     agent-sheet-close               close it
  ///     toast-action=<text>             press the action of the last toast with text
  ///     save-session                    save the session now (needs
  ///                                     --impulse-snapshot-persist)
  ///     run-quoted=<command>|<path>     run command with path quoted for the shell
  ///     select-tab=<index>              select a tab (across all workspaces)
  ///     select-terminal                 select the last tab with a terminal
  ///     select-editor=<path suffix>     select the tab showing that file
  func performDebugAction(_ action: String, done: @escaping () -> Void) {
    if action.hasPrefix("sleep="), let seconds = Double(action.dropFirst(6)) {
      DispatchQueue.main.asyncAfter(deadline: .now() + seconds, execute: done)
    } else if action.hasPrefix("until=") {
      debugWait(until: String(action.dropFirst(6)), done: done)
    } else if action.hasPrefix("js="), let colon = action.firstIndex(of: ":") {
      let label = String(action[action.index(action.startIndex, offsetBy: 3)..<colon])
      debugJavaScript(String(action[action.index(after: colon)...])) { value in
        DebugSnapshot.results[label] = value ?? NSNull()
        done()
      }
    } else if action.hasPrefix("dump=") {
      debugDump(name: String(action.dropFirst(5)), done: done)
    } else if action == "save" {
      // Doesn't wait: a save can ask first (a sheet the next actions answer).
      if let editor = tabManager.selectedEditor {
        let name = ((editor.filePath ?? "untitled") as NSString).lastPathComponent
        saveEditorTab(editor) { saved in DebugSnapshot.results["save:\(name)"] = saved }
      } else {
        DebugSnapshot.failures.append("save: no editor is selected")
      }
      done()
    } else if action.hasPrefix("write-file="), let bar = action.firstIndex(of: "|") {
      let path = debugPath(String(action[action.index(action.startIndex, offsetBy: 11)..<bar]))
      let data = Data(action[action.index(after: bar)...].utf8)
      if let handle = FileHandle(forWritingAtPath: path) {
        try? handle.truncate(atOffset: 0)
        try? handle.write(contentsOf: data)
        try? handle.close()
      } else {
        FileManager.default.createFile(atPath: path, contents: data)
      }
      done()
    } else if action.hasPrefix("read-file="), let bar = action.firstIndex(of: "|") {
      let label = String(action[action.index(action.startIndex, offsetBy: 10)..<bar])
      let path = debugPath(String(action[action.index(after: bar)...]))
      DebugSnapshot.results[label] = (try? String(contentsOfFile: path, encoding: .utf8)) ?? NSNull()
      done()
    } else if action == "task-sheet-create" || action == "task-sheet-cancel" {
      if let sheet = DebugSnapshot.taskSheet {
        action == "task-sheet-create" ? sheet.create() : sheet.cancel()
      } else {
        DebugSnapshot.failures.append("\(action): no New Task sheet is up")
      }
      done()
    } else if action.hasPrefix("agent-sheet=") {
      let parts = action.dropFirst(12).split(separator: ":").map(String.init)
      presentAgentHooksSheet(part: parts.first == "mcp" ? .mcp : .hooks)
      if let model = DebugSnapshot.agentSheet?.model {
        model.agent =
          AgentHooksModel.Agent.allCases.first { $0.rawValue.lowercased().hasPrefix(parts.count > 1 ? parts[1] : "") }
          ?? .claude
        if parts.count > 2, parts[2] == "project" { model.scope = .project }
      } else {
        DebugSnapshot.failures.append("\(action): the sheet didn't open")
      }
      done()
    } else if action == "agent-sheet-apply" {
      guard let sheet = DebugSnapshot.agentSheet else {
        DebugSnapshot.failures.append("agent-sheet-apply: no Agent Integration sheet is up")
        return done()
      }
      // What its Install/Remove button does.
      sheet.model.applyInBackground { outcome in
        DebugSnapshot.results["agent-sheet"] = "\(outcome)"
        if outcome == .written { sheet.close() }
        done()
      }
    } else if action == "agent-sheet-close" {
      DebugSnapshot.agentSheet?.close()
      done()
    } else if action.hasPrefix("sheet-button=") {
      let title = String(action.dropFirst(13))
      if !Self.debugPressButton(title) { DebugSnapshot.failures.append("sheet-button: no “\(title)” button") }
      done()
    } else if action.hasPrefix("toast-action=") {
      let text = String(action.dropFirst(13))
      if let run = DebugSnapshot.toastLog.last(where: { $0.message.contains(text) })?.action {
        run()
      } else {
        DebugSnapshot.failures.append("toast-action: no toast with “\(text)” and an action")
      }
      done()
    } else if action == "save-session" {
      (NSApp.delegate as? AppDelegate)?.persistSessionStateFromOpenWindows(synchronously: true)
      done()
    } else if action.hasPrefix("run-quoted="), let bar = action.firstIndex(of: "|"),
      let terminal = tabManager.selectedTerminal?.activeTerminal
    {
      let command = String(action[action.index(action.startIndex, offsetBy: 11)..<bar])
      let path = String(action[action.index(after: bar)...])
      terminal.runCommand(command + " " + path.shellEscaped(for: terminal.shellType))
      done()
    } else if action.hasPrefix("select-tab="), let index = Int(action.dropFirst(11)) {
      tabManager.selectTab(index: index)
      done()
    } else if action == "select-terminal" {
      let index = tabManager.tabs.lastIndex { tab in
        tab.surfaces.contains { surface in
          if case .terminal = surface { return true }
          return false
        }
      }
      if let index { tabManager.selectTab(index: index) }
      done()
    } else if action.hasPrefix("select-editor=") {
      let suffix = String(action.dropFirst(14))
      let index = tabManager.tabs.firstIndex { tab in
        tab.surfaces.contains { surface in
          if case .editor(let editor) = surface { return editor.filePath?.hasSuffix(suffix) == true }
          return false
        }
      }
      if let index { tabManager.selectTab(index: index) }
      done()
    } else {
      performDebugAction(action)
      done()
    }
  }

  private func debugWait(until spec: String, done: @escaping () -> Void) {
    let parts = spec.split(separator: ":", maxSplits: 1).map(String.init)
    let timeout = parts.count == 2 ? Double(parts[0]) : nil
    let condition = timeout == nil ? spec : parts[1]
    let deadline = Date().addingTimeInterval(timeout ?? 30)
    func poll() {
      debugCondition(condition) { holds in
        if holds {
          done()
        } else if Date() > deadline {
          DebugSnapshot.failures.append("until \(condition): timed out")
          done()
        } else {
          DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { poll() }
        }
      }
    }
    poll()
  }

  /// Conditions for `until=`:
  ///
  ///     prompt              the selected terminal's shell is at a prompt
  ///     blocks:<n>          the selected terminal has n finished command blocks
  ///     editor              the selected editor has its file loaded
  ///     js:<expression>     true in the selected editor's page
  ///     file:<path>, no-file:<path>
  ///     sheet, no-sheet     a sheet is (not) showing
  ///     agent:<state>       the selected terminal's agent is in that state
  ///     toast:<text>        a toast with that text was shown
  ///     result:<label>      something was reported under label
  ///     tabs:<n>            at least n tabs
  ///     panes:<n>           at least n panes in the selected tab
  ///     tool:<title>        a tool tab whose title contains title is open
  private func debugCondition(_ condition: String, answer: @escaping (Bool) -> Void) {
    let parts = condition.split(separator: ":", maxSplits: 1).map(String.init)
    let argument = parts.count > 1 ? parts[1] : ""
    let count = Int(argument) ?? 1
    let terminal = tabManager.selectedTerminal?.activeTerminal
    switch parts.first ?? "" {
    case "prompt":
      answer(terminal.map { $0.renderer.tracksPrompts && !$0.isCommandRunning } ?? false)
    case "blocks":
      answer((terminal?.backend?.commandBlocks().filter { $0.endedAtMs != nil }.count ?? 0) >= count)
    case "editor":
      debugJavaScript("return editor !== null && editor.getModel() !== null && currentFilePath !== ''") {
        answer($0 as? Bool == true)
      }
    case "js":
      debugJavaScript("return !!(\(argument))") { answer($0 as? Bool == true) }
    case "file":
      answer(FileManager.default.fileExists(atPath: debugPath(argument)))
    case "no-file":
      answer(!FileManager.default.fileExists(atPath: debugPath(argument)))
    case "sheet":
      answer(NSApp.windows.contains { $0.attachedSheet != nil })
    case "no-sheet":
      answer(!NSApp.windows.contains { $0.attachedSheet != nil })
    case "agent":
      answer(terminal?.agentState?.rawValue == argument)
    case "toast":
      answer(DebugSnapshot.toastLog.contains { $0.message.contains(argument) || $0.detail?.contains(argument) == true })
    case "result":
      answer(DebugSnapshot.results[argument] != nil)
    case "tabs":
      answer(tabManager.tabs.count >= count)
    case "panes":
      answer((tabManager.selectedTab?.surfaces.count ?? 0) >= count)
    case "tool":
      answer(
        tabManager.allSurfaces.contains { surface in
          if case .tool(let view) = surface { return view.toolTitle.contains(argument) }
          return false
        })
    default:
      DebugSnapshot.failures.append("until: unknown condition “\(condition)”")
      answer(true)
    }
  }

  /// Runs an async function body in the selected editor's page. Nil without
  /// an editor; a script error comes back as {"error": message}.
  private func debugJavaScript(_ body: String, result: @escaping (Any?) -> Void) {
    guard let webView = tabManager.selectedEditor?.webView else { return result(nil) }
    webView.callAsyncJavaScript(body, arguments: [:], in: nil, in: .page) { outcome in
      switch outcome {
      case .success(let value):
        result(value)
      case .failure(let error):
        let info = (error as NSError).userInfo
        result(["error": info["WKJavaScriptExceptionMessage"] as? String ?? error.localizedDescription])
      }
    }
  }

  /// A path from an action: absolute, under ~ (the demo home), or relative
  /// to the starting folder.
  private func debugPath(_ path: String) -> String {
    let expanded = (path as NSString).expandingTildeInPath
    if expanded.hasPrefix("/") { return expanded }
    return ((DebugSnapshot.initialDirectory ?? fileTreeRootPath) as NSString).appendingPathComponent(expanded)
  }

  /// Writes `<name>.json`: workspaces, tabs and what each pane holds
  /// (terminals with their blocks and output, editors with both Impulse's
  /// copy of the text and Monaco's), toasts, sheets, results and failures.
  private func debugDump(name: String, done: @escaping () -> Void) {
    guard let directory = DebugSnapshot.outputDirectory else { return done() }
    var editors: [(EditorTab, NSMutableDictionary)] = []
    var tabs: [[String: Any]] = []
    for (index, tab) in tabManager.tabs.enumerated() {
      let workspace = tabManager.workspaceID(ofTabAt: index).flatMap { tabManager.workspace($0) }
      tabs.append([
        "workspace": workspace?.name ?? NSNull(),
        "selected": index == tabManager.selectedIndex,
        "title": tab.title,
        "surfaces": tab.surfaces.flatMap { debugDescription(of: $0, editors: &editors) },
      ])
    }
    let state: [String: Any] = [
      "workspaces": tabManager.workspaces.map { workspace -> [String: Any] in
        [
          "name": workspace.name, "root": workspace.root, "kind": "\(workspace.kind)", "task": workspace.isTask,
          "active": workspace.id == tabManager.activeWorkspaceID, "trusted": Trust.shared.isTrusted(workspace.root),
        ]
      },
      "trustEnabled": Trust.shared.isEnabled,
      "tabs": tabs,
      "toasts": DebugSnapshot.toastLog.map { toast -> [String: Any] in
        [
          "kind": "\(toast.kind)", "message": toast.message, "detail": toast.detail ?? NSNull(),
          "action": toast.actionTitle ?? NSNull(),
        ]
      },
      "sheets": NSApp.windows.compactMap(\.attachedSheet).map { Self.debugContents(of: $0) },
      "agentSheet": DebugSnapshot.agentSheet.map { sheet -> [String: Any] in
        let model = sheet.model
        return [
          "part": model.part.rawValue, "agent": model.agent.rawValue, "installed": model.installed,
          "outdated": model.outdated, "path": model.path, "diff": model.diff, "error": model.error ?? NSNull(),
        ]
      } ?? NSNull(),
      "taskSheet": DebugSnapshot.taskSheet.map { sheet -> [String: Any] in
        let model = sheet.model
        return [
          "title": model.draft.title, "base": model.draft.base, "command": model.draft.command,
          "brief": model.draft.brief, "requestedBy": model.agentRequest?.requestedBy ?? NSNull(),
          "branch": model.branch, "error": model.error ?? NSNull(),
        ]
      } ?? NSNull(),
      "results": Self.debugJSONSafe(DebugSnapshot.results),
      "failures": DebugSnapshot.failures,
    ]
    // Monaco's own text, asked of each editor's page.
    let group = DispatchGroup()
    for (editor, entry) in editors {
      guard let webView = editor.webView else { continue }
      group.enter()
      var left = false
      let leave = {
        guard !left else { return }
        left = true
        group.leave()
      }
      DispatchQueue.main.asyncAfter(deadline: .now() + 5) { leave() }
      webView.evaluateJavaScript("editor ? editor.getValue() : null") { value, _ in
        entry["monaco"] = value as? String ?? NSNull()
        leave()
      }
    }
    group.notify(queue: .main) {
      if let data = try? JSONSerialization.data(withJSONObject: state, options: [.prettyPrinted, .sortedKeys]) {
        try? data.write(to: directory.appendingPathComponent("\(name).json"))
      }
      done()
    }
  }

  private func debugDescription(of surface: TabEntry, editors: inout [(EditorTab, NSMutableDictionary)])
    -> [Any]
  {
    switch surface {
    case .terminal(let container):
      return container.terminals.map { terminal -> [String: Any] in
        let blocks = (terminal.backend?.commandBlocks() ?? []).map { block -> [String: Any] in
          [
            "command": block.command ?? NSNull(), "cwd": block.cwd ?? NSNull(),
            "exit": block.exitCode.map { Int($0) } ?? NSNull(), "finished": block.endedAtMs != nil,
          ]
        }
        return [
          "kind": "terminal", "shell": terminal.shellType.rawValue, "cwd": terminal.currentWorkingDirectory,
          "running": terminal.isCommandRunning, "agent": terminal.agent?.id ?? NSNull(),
          "agentState": terminal.agentState?.rawValue ?? NSNull(), "blocks": blocks,
          "transcript": terminal.backend?.transcript(maxRows: 400) ?? "",
        ]
      }
    case .editor(let editor):
      let entry: NSMutableDictionary = [
        "kind": "editor", "path": editor.filePath ?? NSNull(), "modified": editor.isModified,
        "language": editor.language, "largeFile": editor.isLargeFile, "swift": editor.content,
      ]
      editors.append((editor, entry))
      return [entry]
    case .imagePreview(let path, _):
      return [["kind": "image", "path": path]]
    case .diffReview(let root, _):
      return [["kind": "review", "root": root]]
    case .history(let root, _):
      return [["kind": "history", "root": root]]
    case .tool(let view):
      return [["kind": "tool", "title": view.toolTitle]]
    case .split:
      return []
    }
  }

  /// Texts and buttons in a window, read through accessibility so SwiftUI
  /// content counts as well as AppKit's.
  private static func debugContents(of window: NSWindow) -> [String: Any] {
    var texts: [String] = []
    var buttons: [String] = []
    _ = debugWalk(window) { role, element in
      switch role {
      case .staticText?, .textArea?, .textField?:
        if let text = element.accessibilityValue() as? String ?? element.accessibilityLabel(), !text.isEmpty {
          texts.append(text)
        }
      case .button?, .checkBox?:
        if let title = element.accessibilityTitle() ?? element.accessibilityLabel(), !title.isEmpty {
          buttons.append(title)
        }
      default:
        break
      }
      return false
    }
    return ["texts": texts, "buttons": buttons]
  }

  /// Presses the first button titled `title`, looking in sheets first.
  /// (An alert's buttons say they weren't pressed when they end the sheet.)
  private static func debugPressButton(_ title: String) -> Bool {
    let windows = NSApp.windows.compactMap(\.attachedSheet) + NSApp.windows.filter(\.isVisible)
    return windows.contains { window in
      debugWalk(window) { role, element in
        guard role == .button, (element.accessibilityTitle() ?? element.accessibilityLabel()) == title else {
          return false
        }
        _ = element.accessibilityPerformPress()
        return true
      }
    }
  }

  /// Visits the accessibility tree under `root` until `visit` returns true.
  private static func debugWalk(
    _ root: Any, visit: (NSAccessibility.Role?, NSAccessibilityProtocol) -> Bool
  ) -> Bool {
    var stack: [Any] = [root]
    var visited = 0
    while let next = stack.popLast(), visited < 5000 {
      visited += 1
      guard let element = next as? NSAccessibilityProtocol else { continue }
      if visit(element.accessibilityRole(), element) { return true }
      stack.append(contentsOf: (element.accessibilityChildren() ?? []).reversed())
    }
    return false
  }

  /// What JSONSerialization takes: anything else becomes its description.
  private static func debugJSONSafe(_ value: Any) -> Any {
    switch value {
    case let dictionary as [String: Any]: return dictionary.mapValues { debugJSONSafe($0) }
    case let array as [Any]: return array.map { debugJSONSafe($0) }
    case is String, is NSNumber, is NSNull: return value
    default: return String(describing: value)
    }
  }
}
