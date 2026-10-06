import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

extension MainWindowController {

  // MARK: - Window State

  func sessionWindowState() -> SessionWindowState {
    var (workspaces, activeIndex) = tabManager.sessionWorkspaces()
    // A scratch workspace remembers where its file tree was.
    for index in workspaces.indices where workspaces[index].kind == "scratch" {
      if tabManager.activeWorkspace.kind == .scratch { workspaces[index].fileTreeRoot = fileTreeRootPath }
    }
    return SessionWindowState(
      workspaces: workspaces, activeWorkspaceIndex: activeIndex,
      frame: window.map { NSStringFromRect($0.frame) },
      sidebarVisible: windowModel.sidebarVisible,
      sidebarWidth: Double(windowModel.sidebarWidth))
  }

  /// Rebuild workspaces, tabs and split layouts from a saved window. File
  /// contents are read off the main thread first, then everything is
  /// inserted on main in saved order. Returns false when nothing in the
  /// session can be restored.
  @discardableResult
  /// `then` runs once the restored tabs are in (e.g. files to open on top).
  func restoreSessionWindow(_ state: SessionWindowState, then: (() -> Void)? = nil) -> Bool {
    let savedWorkspaces = (state.workspaces ?? []).filter { workspace in
      workspace.kind == "scratch" || FileManager.default.fileExists(atPath: workspace.root)
    }
    guard !savedWorkspaces.isEmpty else { return false }

    if let frame = state.frame, let window {
      let rect = NSRectFromString(frame)
      if rect.width > 200, rect.height > 200,
        NSScreen.screens.contains(where: { $0.visibleFrame.intersects(rect) })
      {
        window.setFrame(rect, display: false)
      }
    }
    if let visible = state.sidebarVisible { setSidebarVisible(visible) }
    if let width = state.sidebarWidth, width > 120 { windowModel.sidebarWidth = CGFloat(width) }

    let paths = savedWorkspaces.flatMap { $0.tabs.flatMap { $0.panes.compactMap(\.path) } }
    let withScrollback = settings.restoreScrollback
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let contents = TabManager.preloadFileContents(paths)
      var workspaces = savedWorkspaces
      if withScrollback { SessionScrollback.load(into: &workspaces) }
      DispatchQueue.main.async { [weak self] in
        self?.insertRestoredWorkspaces(
          workspaces, activeIndex: state.activeWorkspaceIndex, contents: contents)
        then?()
      }
    }
    return true
  }

  private func insertRestoredWorkspaces(
    _ saved: [SessionWorkspaceState], activeIndex: Int?,
    contents: [String: (text: String, large: Bool, bom: Bool)]
  ) {
    var activeWorkspaceID: UUID?
    var activeTabIndex: Int?
    for (position, savedWorkspace) in saved.enumerated() {
      let workspace: Workspace
      if savedWorkspace.kind == "scratch", let scratch = tabManager.scratchWorkspace {
        workspace = scratch
        if let root = savedWorkspace.fileTreeRoot, FileManager.default.fileExists(atPath: root) {
          switchFileTreeRoot(root)
        }
      } else {
        workspace = Workspace(kind: .folder, root: savedWorkspace.root)
        tabManager.addWorkspace(workspace)
      }
      workspace.customName = savedWorkspace.name
      workspace.isExpanded = savedWorkspace.expanded ?? false
      let projectDirectory = workspace.kind == .folder ? workspace.root : nil

      var restoredIndexes: [Int?] = []
      for tab in savedWorkspace.tabs {
        var panes: [Int: TabEntry] = [:]
        for (id, surface) in tab.panes.enumerated() {
          if let entry = tabManager.makeRestoredSurface(
            surface, contents: contents, projectDirectory: projectDirectory)
          {
            panes[id] = entry
          }
        }
        let index = tabManager.appendRestoredTab(
          panes: panes, layout: tab.layout, focusedPane: tab.focusedPane, pinned: tab.pinned,
          workspaceID: workspace.id)
        restoredIndexes.append(index)
        for case .editor(let editor) in panes.values {
          if let path = editor.filePath {
            trackEditorTab(editor, forPath: path)
            lspDidOpenIfNeeded(path: path)
          }
        }
      }
      if position == (activeIndex ?? 0) {
        activeWorkspaceID = workspace.id
        // Saved index → index among the tabs that actually came back.
        if let saved = savedWorkspace.activeTabIndex, restoredIndexes.indices.contains(saved),
          restoredIndexes[saved] != nil
        {
          activeTabIndex = restoredIndexes[..<saved].compactMap { $0 }.count
        }
      }
    }
    tabManager.finishRestore(activeWorkspaceID: activeWorkspaceID, activeTabIndex: activeTabIndex)
  }

  func restorableOpenFiles() -> [String] {
    let paths = tabManager.allSurfaces.compactMap { tab -> String? in
      switch tab {
      case .editor(let editor):
        return editor.filePath
      case .imagePreview(let path, _):
        return path
      case .terminal, .diffReview, .history, .tool, .split:
        return nil
      }
    }
    var seen = Set<String>()
    return paths.filter { path in
      guard FileManager.default.fileExists(atPath: path), !seen.contains(path) else {
        return false
      }
      seen.insert(path)
      return true
    }
  }

  private func dirtyEditors() -> [EditorTab] {
    tabManager.allSurfaces.compactMap { tab in
      if case .editor(let editor) = tab, editor.isModified {
        return editor
      }
      return nil
    }
  }

  func runningTerminalProcessCount() -> Int {
    tabManager.allSurfaces.reduce(0) { count, tab in
      if case .terminal(let container) = tab {
        return count + container.runningDescendantProcessCount()
      }
      return count
    }
  }

  func runningCloseRiskCommands() -> [CloseRiskCommand] {
    tabManager.allSurfaces.flatMap { tab in
      if case .terminal(let container) = tab {
        return container.runningCloseRiskCommands()
      }
      return []
    }
  }

  func closeRiskSummary(action: CloseRiskAction) -> CloseRiskSummary? {
    closeRiskSummary(
      action: action,
      unsavedEditorCount: 0,
      runningTerminalProcessCount: runningTerminalProcessCount(),
      runningCommands: runningCloseRiskCommands()
    )
  }

  func closeRiskSummary(
    action: CloseRiskAction,
    unsavedEditorCount: Int,
    runningTerminalProcessCount: Int,
    runningCommands: [CloseRiskCommand]
  ) -> CloseRiskSummary? {
    let input = CloseRiskInput(
      action: action,
      unsavedEditorCount: unsavedEditorCount,
      runningTerminalProcessCount: runningTerminalProcessCount,
      runningCommands: runningCommands,
      nowMs: currentUnixTimeMs(),
      longCommandThresholdSeconds: UInt64(max(1, settings.terminalLongCommandSeconds))
    )
    return input.summarize()
  }

  /// True to close now. Otherwise a sheet asks about the running
  /// processes, and confirming it closes the window.
  private func confirmClosingTerminalProcessesIfNeeded() -> Bool {
    if closeRiskConfirmed {
      closeRiskConfirmed = false
      return true
    }
    guard settings.confirmCloseWarnings, let window else { return true }
    guard let summary = closeRiskSummary(action: .closeWindow), summary.hasRisk else {
      return true
    }

    let alert = NSAlert()
    alert.messageText = summary.title
    alert.informativeText = closeRiskInformativeText(summary)
    alert.alertStyle = .warning
    alert.addButton(withTitle: summary.destructiveActionTitle)
    alert.addButton(withTitle: summary.cancelTitle)
    alert.beginSheetModal(for: window) { [weak self, weak window] response in
      guard response == .alertFirstButtonReturn, let self, let window else { return }
      // Unsaved files were already dealt with; close straight through.
      self.closeRiskConfirmed = true
      self.closingAfterDirtyReview = true
      window.performClose(nil)
    }
    return false
  }

  func closeRiskInformativeText(_ summary: CloseRiskSummary) -> String {
    let details = summary.detailLines.joined(separator: "\n")
    if summary.informativeText.isEmpty {
      return details
    }
    if details.isEmpty {
      return summary.informativeText
    }
    return "\(summary.informativeText)\n\n\(details)"
  }

  private func reviewDirtyEditorsBeforeWindowClose(_ dirty: [EditorTab]) {
    reviewingDirtyWindowClose = true
    var remaining = dirty

    func next() {
      guard !remaining.isEmpty else {
        self.reviewingDirtyWindowClose = false
        self.closingAfterDirtyReview = true
        // performClose, not close: close() skips windowShouldClose, and with
        // it the running-processes check that follows the dirty review.
        self.window?.performClose(nil)
        return
      }

      let editor = remaining.removeFirst()
      guard let location = self.tabManager.location(of: editor) else {
        next()
        return
      }

      self.tabManager.reveal(location)
      self.reviewAndSave(editor: editor) { proceed in
        if proceed {
          DispatchQueue.main.async { next() }
        } else {
          self.reviewingDirtyWindowClose = false
          self.closingAfterDirtyReview = false
        }
      }
    }

    next()
  }

  // MARK: - NSWindowDelegate

  func windowShouldClose(_ sender: NSWindow) -> Bool {
    if (NSApp.delegate as? AppDelegate)?.isApplicationTerminating == true {
      return true
    }
    if closingAfterDirtyReview {
      closingAfterDirtyReview = false
      return confirmClosingTerminalProcessesIfNeeded()
    }
    guard !reviewingDirtyWindowClose else { return false }

    let dirty = dirtyEditors()
    guard !dirty.isEmpty else {
      return confirmClosingTerminalProcessesIfNeeded()
    }
    reviewDirtyEditorsBeforeWindowClose(dirty)
    return false
  }

  func windowWillEnterFullScreen(_ notification: Notification) {
    windowModel.isFullScreen = true
    // The sizing toolbar has no items; in full screen it would only appear as
    // an empty reveal strip over the chrome bar.
    window?.toolbar?.isVisible = false
  }

  func windowDidExitFullScreen(_ notification: Notification) {
    windowModel.isFullScreen = false
    window?.toolbar?.isVisible = true
  }

  func windowDidBecomeKey(_ notification: Notification) {
    // Refresh git state when the window regains focus — git status may
    // have changed externally (e.g. commits from another terminal).
    if let editor = tabManager.selectedEditor {
      applyGitDiffDecorations(editor: editor)
    }
    fileTreeData.refreshGitStatus()
  }

  func windowWillClose(_ notification: Notification) {
    teardownCustomKeybindingMonitor()

    // Persist restorable window state before tab cleanup clears it.
    if let delegate = NSApp.delegate as? AppDelegate {
      delegate.settings.sidebarVisible = windowModel.sidebarVisible
      delegate.settings.sidebarWidth = Int(windowModel.sidebarWidth)
      delegate.persistSessionStateFromOpenWindows()
    }

    // Remove all notification observers.
    notificationObservers.forEach { NotificationCenter.default.removeObserver($0) }
    notificationObservers.removeAll()
    // A closed window must not keep timers and shared-repository listeners
    // (and through them, itself) alive.
    portTimer?.invalidate()
    portTimer = nil
    if let listener = repositoryListener {
      listener.state.removeChangeListener(listener.token)
      repositoryListener = nil
    }
    repositoryObservation?.cancel()
    repositoryObservation = nil

    // Clean up all remaining tabs (kill terminal processes, tear down
    // editor WebViews) so resources are freed immediately.
    tabManager.cleanupAllTabs()

    // Clear editor tab tracking.
    editorTabsByPath.removeAll()

    (NSApp.delegate as? AppDelegate)?.windowControllerDidClose(self)
  }
}
