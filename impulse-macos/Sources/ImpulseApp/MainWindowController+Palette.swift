import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

// MARK: - Palette host

extension MainWindowController: PaletteHost {
  var paletteRoot: String { fileTreeRootPath }

  var paletteOpenFiles: [String] {
    tabManager.allSurfaces.compactMap { tab in
      if case .editor(let editor) = tab { return editor.filePath }
      return nil
    }
  }

  var paletteTabs: [TabDisplayInfo] { windowModel.allTabs }

  var paletteHistoryContext: (cwd: String?, repo: String?) {
    let cwd = tabManager.selectedTerminal?.activeTerminal?.currentWorkingDirectory
    return (cwd?.isEmpty == false ? cwd : nil, windowModel.repository?.root)
  }

  /// Into the input bar, or typed at the shell prompt when the grid owns
  /// input (classic mode, a TUI).
  func palettePullRequests(_ completion: @escaping ([PullRequestSummary]?) -> Void) {
    guard let repository = tabManager.activeWorkspace.repository ?? windowModel.repository,
      PullRequestMonitor.shared.isAvailable
    else { return completion(nil) }
    PullRequestMonitor.shared.list(root: repository.root, completion: completion)
  }

  func paletteDocumentSymbols(_ completion: @escaping ([OutlineSymbol]?) -> Void) {
    guard let editor = tabManager.selectedEditor else { return completion(nil) }
    documentSymbols(for: editor, completion: completion)
  }

  func paletteWorkspaceSymbols(
    _ query: String, completion: @escaping ([(symbol: OutlineSymbol, path: String)]?) -> Void
  ) {
    guard let editor = tabManager.selectedEditor else { return completion(nil) }
    workspaceSymbols(query: query, editor: editor, completion: completion)
  }

  var paletteProjectActions: [ProjectConfig.Action] {
    projectRoot.flatMap { projectConfig(root: $0)?.actions } ?? []
  }

  func paletteRunProjectAction(_ action: ProjectConfig.Action) {
    guard let root = projectRoot else { return }
    runProjectAction(action, root: root)
  }

  func paletteEditProjectConfig() {
    editProjectConfig()
  }

  func paletteOpenSetting(_ key: String) {
    openSettings(query: key)
  }

  func paletteCheckOutPullRequest(_ pullRequest: PullRequestSummary) {
    checkOutPullRequestAsTask(pullRequest)
  }

  func paletteInsertCommand(_ command: String) {
    guard let terminal = tabManager.selectedTerminal?.activeTerminal else {
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(command, forType: .string)
      toasts.show(Toast(kind: .info, message: "Copied the command (no terminal is focused)."))
      return
    }
    if terminal.wantsGridFocus {
      terminal.insertInput(command)
      terminal.focus()
    } else {
      windowModel.inputDraft = command
      windowModel.inputDraftRestoreToken += 1
      windowModel.inputBarFocusToken += 1
    }
  }
  var paletteWorkspaces: [WorkspaceInfo] { windowModel.workspaces }
  var paletteVisibleTabIndices: [Int] { tabManager.visibleTabIndices }

  func paletteSelectWorkspace(_ id: UUID) {
    tabManager.activateWorkspace(id)
  }

  func paletteOpenWorkspace(folder: String?) {
    if let folder {
      tabManager.openWorkspace(folder: folder)
    } else {
      presentOpenWorkspacePanel()
    }
  }
  var paletteCurrentBranch: String? { windowModel.gitBranch }
  var paletteHasEditor: Bool { tabManager.selectedEditor != nil }

  func paletteOpenFile(_ path: String, line: UInt32?, column: UInt32?) {
    openCommandPaletteSearchResult(path: path, line: line, column: column)
  }

  func paletteGoToLine(_ line: UInt32, column: UInt32?) {
    guard let editor = tabManager.selectedEditor else { return }
    editor.goToPosition(line: line, column: column ?? 1)
    editor.focus()
  }

  func paletteSwitchBranch(_ branch: String) {
    switchBranch(to: branch)
  }

  func paletteCreateBranch(_ name: String) {
    guard let repository = windowModel.repository else { return }
    GitActions(repository: repository, host: self).createBranch(name)
  }

  func paletteSelectTab(_ index: Int) {
    tabManager.selectTab(index: index)
  }
}
