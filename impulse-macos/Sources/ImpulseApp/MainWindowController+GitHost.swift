import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

// MARK: - Git panel host

extension MainWindowController: GitPanelHost {
  var toasts: ToastCenter { windowModel.toasts }

  var agentTargets: [AgentSummary] {
    // Agents in the window's repository first, then the rest.
    windowModel.agents.filter { $0.state != .exited }
  }

  func sendToAgent(_ text: String, terminalID: UUID) {
    guard let terminal = agentTerminal(id: terminalID), let agent = terminal.agent else {
      toasts.show(Toast(kind: .warning, message: "That agent isn't running anymore."))
      return
    }
    let busy = terminal.agentState == .working || terminal.agentState == .needsInput
    terminal.sendToAgent(text)
    toasts.show(
      Toast(
        kind: .success,
        message: busy
          ? "Queued for \(agent.displayName); it's sent when the current turn ends."
          : "Sent to \(agent.displayName). Review it in its prompt, then press Return.",
        actionTitle: "Show",
        action: { [weak self] in self?.revealTerminal(id: terminalID) }))
  }

  func gitOpenFile(_ absolutePath: String) {
    openCommandPaletteSearchResult(path: absolutePath, line: nil)
  }

  func gitOpenDiffEditor(_ absolutePath: String) {
    openCommandPaletteSearchResult(path: absolutePath, line: nil)
    guard let editor = findEditorTab(forPath: absolutePath) else { return }
    setDiffView(editor, enabled: true)
  }

  func gitOpenReview(scope: DiffScope, focusPath: String?) {
    guard let repository = windowModel.repository else { return }
    tabManager.addReviewTab(
      repository: repository, scope: scope, focusPath: focusPath, host: self)
  }

  func gitPresentError(_ error: GitOperationError, title: String) {
    presentGitError(error, title: title)
  }

  func gitConfirm(
    title: String, message: String, confirmTitle: String, destructive: Bool,
    completion: @escaping (Bool) -> Void
  ) {
    guard let window else { return completion(false) }
    let alert = NSAlert()
    alert.alertStyle = destructive ? .warning : .informational
    alert.messageText = title
    alert.informativeText = message
    alert.addButton(withTitle: confirmTitle)
    alert.addButton(withTitle: "Cancel")
    alert.buttons.first?.hasDestructiveAction = destructive
    alert.beginSheetModal(for: window) { response in
      completion(response == .alertFirstButtonReturn)
    }
  }

  /// Show the Changes panel in the left dock (⌃⇧G).
  func showChangesPanel() {
    windowModel.resetSearch()
    windowModel.sidebarPanel = .changes
    windowModel.sidebarVisible = true
    windowModel.changesFocusToken += 1
  }
}
