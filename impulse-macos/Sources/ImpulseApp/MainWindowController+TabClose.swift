import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

extension MainWindowController {

  // MARK: - Tab Close with Save Confirmation

  /// Closes a whole tab (every pane), after confirming unsaved editors and
  /// running processes. Used by the tab strip's close button and menus.
  func requestCloseTab(index: Int) {
    guard index >= 0, index < tabManager.tabs.count else { return }

    // Confirm before closing pinned tabs
    if tabManager.pinnedTabs[index] {
      let alert = NSAlert()
      alert.messageText = "Pinned Tab"
      alert.informativeText = "This tab is pinned. Close anyway?"
      alert.alertStyle = .warning
      alert.addButton(withTitle: "Close")
      alert.addButton(withTitle: "Cancel")

      guard let window = self.window else { return }
      let tabView = tabManager.tabs[index].view
      alert.beginSheetModal(for: window) { [weak self] response in
        // Re-find the tab: others may have closed or opened while asking.
        guard let self, response == .alertFirstButtonReturn,
          let current = self.tabManager.tabs.firstIndex(where: { $0.view === tabView })
        else { return }
        // Unpin, then re-enter requestCloseTab for unsaved-changes handling
        self.tabManager.unpin(index: current)
        self.requestCloseTab(index: current)
      }
      return
    }

    let tabView = tabManager.tabs[index].view
    let surfaces = tabManager.tabs[index].surfaces
    confirmClosing(surfaces) { [weak self] in
      guard let self,
        // Re-find the tab by identity — the index may be stale if other
        // tabs were closed while a confirmation was up.
        let currentIndex = self.tabManager.tabs.firstIndex(where: { $0.view === tabView })
      else { return }
      for surface in surfaces { self.willCloseSurface(surface) }
      self.tabManager.closeTab(index: currentIndex)
    }
  }

  /// ⌘W: closes the focused pane of a split tab, otherwise the whole tab.
  func requestCloseFocusedPane() {
    guard let split = tabManager.selectedSplit else {
      requestCloseTab(index: tabManager.selectedIndex)
      return
    }
    let surface = split.focused
    confirmClosing([surface]) { [weak self] in
      guard let self,
        let location = self.tabManager.locate(where: { $0.view === surface.view })
      else { return }
      self.willCloseSurface(surface)
      self.tabManager.closePane(location.paneID, inTabAt: location.tabIndex)
    }
  }

  /// Closes the pane showing `editor` (its whole tab when it's the only
  /// pane), with the usual confirmations.
  func requestClose(editor: EditorTab) {
    guard let location = tabManager.location(of: editor) else { return }
    guard location.paneID != nil else {
      requestCloseTab(index: location.tabIndex)
      return
    }
    let surface = TabEntry.editor(editor)
    confirmClosing([surface]) { [weak self] in
      guard let self, let location = self.tabManager.location(of: editor) else { return }
      self.willCloseSurface(surface)
      self.tabManager.closePane(location.paneID, inTabAt: location.tabIndex)
    }
  }

  /// Bookkeeping before a surface goes away.
  func willCloseSurface(_ surface: TabEntry) {
    guard case .editor(let editor) = surface else { return }
    if let path = editor.filePath {
      untrackEditorTab(forPath: path)
    }
    lspDidClose(editor: editor)
  }

  /// Walks the surfaces' unsaved editors one sheet at a time, then confirms
  /// running terminal processes, then calls `proceed`. Cancelling anywhere
  /// stops.
  func confirmClosing(_ surfaces: [TabEntry], proceed: @escaping () -> Void) {
    var dirty = surfaces.compactMap { surface -> EditorTab? in
      if case .editor(let editor) = surface, editor.isModified { return editor }
      return nil
    }
    let terminals = surfaces.compactMap { surface -> TerminalContainer? in
      if case .terminal(let container) = surface { return container }
      return nil
    }

    func confirmTerminals() {
      guard !terminals.isEmpty else {
        proceed()
        return
      }
      confirmClosingTerminalsIfNeeded(terminals) { shouldClose in
        if shouldClose { proceed() }
      }
    }

    func next() {
      guard !dirty.isEmpty else {
        confirmTerminals()
        return
      }
      let editor = dirty.removeFirst()
      confirmClosingEditor(editor) { shouldClose in
        if shouldClose { next() }
      }
    }
    next()
  }

  private func confirmClosingEditor(_ editor: EditorTab, completion: @escaping (Bool) -> Void) {
    let filename =
      editor.filePath.map {
        ($0 as NSString).lastPathComponent
      } ?? "Untitled"

    let alert = NSAlert()
    alert.messageText = "Unsaved Changes"
    alert.informativeText = "\"\(filename)\" has unsaved changes. Close anyway?"
    alert.alertStyle = .warning
    alert.addButton(withTitle: "Save & Close")
    alert.addButton(withTitle: "Don't Save")
    alert.addButton(withTitle: "Cancel")

    guard let window = self.window else {
      completion(false)
      return
    }
    alert.beginSheetModal(for: window) { [weak self] response in
      guard let self else { return }
      switch response {
      case .alertFirstButtonReturn:
        // Close only once the save has landed: the save is asynchronous
        // (the buffer comes from Monaco, or a save panel), and closing first
        // tears down the editor before anything is written. The normal save
        // runs its formatter and post-save steps; a failed save says so and
        // the tab stays open.
        self.saveEditorTab(editor, completion: completion)
      case .alertSecondButtonReturn:
        completion(true)
      default:
        completion(false)
      }
    }
  }

  private func confirmClosingTerminalsIfNeeded(
    _ containers: [TerminalContainer],
    completion: @escaping (Bool) -> Void
  ) {
    guard settings.confirmCloseWarnings else {
      completion(true)
      return
    }

    guard
      let summary = closeRiskSummary(
        action: .closeTab,
        unsavedEditorCount: 0,
        runningTerminalProcessCount: containers.reduce(0) {
          $0 + $1.runningDescendantProcessCount()
        },
        runningCommands: containers.flatMap { $0.runningCloseRiskCommands() }
      ),
      summary.hasRisk
    else {
      completion(true)
      return
    }

    let alert = NSAlert()
    alert.messageText = summary.title
    alert.informativeText = closeRiskInformativeText(summary)
    alert.alertStyle = .warning
    alert.addButton(withTitle: summary.destructiveActionTitle)
    alert.addButton(withTitle: summary.cancelTitle)

    guard let window = self.window else {
      completion(alert.runModal() == .alertFirstButtonReturn)
      return
    }

    alert.beginSheetModal(for: window) { response in
      completion(response == .alertFirstButtonReturn)
    }
  }

  /// Presents the standard per-document "Do you want to save the changes
  /// made to X?" sheet for a dirty editor and invokes `completion` with
  /// `true` if the user chose Save or Don't Save, `false` if they cancelled.
  /// Used by the quit flow to walk dirty tabs one at a time.
  func reviewAndSave(editor: EditorTab, completion: @escaping (Bool) -> Void) {
    let filename = editor.filePath.map { ($0 as NSString).lastPathComponent } ?? "Untitled"

    let alert = NSAlert()
    alert.messageText = "Do you want to save the changes made to \(filename)?"
    alert.informativeText = "Your changes will be lost if you don't save them."
    alert.alertStyle = .warning
    alert.addButton(withTitle: "Save")
    alert.addButton(withTitle: "Cancel")
    alert.addButton(withTitle: "Don't Save")

    guard let window = self.window else {
      completion(false)
      return
    }
    alert.beginSheetModal(for: window) { [weak self, weak editor] response in
      guard let self, let editor else {
        completion(false)
        return
      }
      switch response {
      case .alertFirstButtonReturn:
        // The normal save (formatter, commands on save, language servers),
        // asking for a name if it's untitled.
        self.saveEditorTab(editor, completion: completion)
      case .alertThirdButtonReturn:
        completion(true)
      default:
        completion(false)
      }
    }
  }
}
