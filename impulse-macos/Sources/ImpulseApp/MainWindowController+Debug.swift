import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
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
    case _ where action.hasPrefix("review-"):
      for case .diffReview(_, let review) in tabManager.allSurfaces {
        review.debugAction(String(action.dropFirst(7)))
      }
    default:
      if action.hasPrefix("open=") {
        let relative = String(action.dropFirst(5))
        let base = DebugSnapshot.initialDirectory ?? fileTreeRootPath
        openFile(path: (base as NSString).appendingPathComponent(relative))
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
        reportWorkspaceEdit(applyWorkspaceEdit(edit), verb: "Renamed")
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
      } else if action.hasPrefix("pr-threads=") {
        // A saved `gh api graphql` reviewThreads answer, applied to the open review.
        let data = FileManager.default.contents(atPath: String(action.dropFirst(11))) ?? Data()
        for case .diffReview(_, let review) in tabManager.allSurfaces {
          review.applyImportedThreads(PullRequestThreads.parse(data), number: 7)
        }
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
      } else if action == "agent-submit" {
        tabManager.selectedTerminal?.activeTerminal?.agentEvent(.submit)
      } else if action == "review-agent" {
        reviewLastAgentTurn()
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
}
