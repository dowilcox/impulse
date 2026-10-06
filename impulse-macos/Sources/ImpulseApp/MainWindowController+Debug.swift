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
      } else if action == "expand-workspaces" {
        for workspace in tabManager.workspaces {
          tabManager.setWorkspaceExpanded(workspace.id, true)
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
