import AppKit
import ImpulseGit
import ImpulseKit

// When a workspace's HEAD moves and dependency files come with it, offer the
// steps that bring the checkout up to date: the project's `[on_change]`
// commands for the files that changed (or its setup script when a known
// dependency file changed and no rule names it), and after a merge or pull,
// its check script, since a clean merge can still break code.

extension MainWindowController {
  /// Called when a repository's HEAD moved from `from` to `to`.
  func offerDependencySteps(root: String, from: String, to: String) {
    // Only for a workspace in this window.
    guard let workspace = tabManager.workspaces.first(where: { $0.kind == .folder && $0.repository?.root == root })
    else { return }
    let id = workspace.id
    DispatchQueue.global(qos: .utility).async { [weak self] in
      guard let subject = GitOperations.headReflogSubject(root: root) else { return }
      let move = DependencyChanges.move(reflog: subject)
      guard move != .ownCommit else { return }
      let changed = GitOperations.changedPaths(from: from, to: to, root: root)
      let settings = try? Self.loadProjectConfig(root: root)?.config.get()
      let rules = DependencyChanges.matches(rules: settings?.onChange ?? [:], changed: changed)
      let known = DependencyChanges.knownChanged(changed)
      guard !rules.isEmpty || !known.isEmpty else { return }
      DispatchQueue.main.async {
        self?.showDependencyToast(
          workspace: id, root: root, move: move, changed: changed, known: known, settings: settings)
      }
    }
  }

  /// The settings read here, not yet trusted, only decide what the toast
  /// says and offers; what runs is worked out again from the trusted
  /// settings when it's clicked.
  private func showDependencyToast(
    workspace: UUID, root: String, move: DependencyChanges.Move, changed: [String], known: [String],
    settings: ProjectConfig?
  ) {
    let rules = DependencyChanges.matches(rules: settings?.onChange ?? [:], changed: changed)
    let files = Array(Set(rules.map(\.file) + known)).sorted()
    let names = files.map { ($0 as NSString).lastPathComponent }
    let listed = names.count <= 2 ? names.joined(separator: " and ") : "\(names[0]), \(names[1]) and \(names.count - 2) more"
    let command = Self.updateCommand(settings, changed: changed, known: known)
    let check = move == .merge ? settings?.checkScript : nil
    var toast = Toast(
      kind: .info, message: "\(listed) changed",
      detail: command.map { "Update with: \($0)" } ?? "Reinstall dependencies before running tests.",
      lifetime: 20)
    if command != nil {
      toast.actionTitle = rules.isEmpty ? "Run Setup" : "Update Dependencies"
      toast.action = { [weak self] in
        self?.runProjectCommand(root: root, workspace: workspace) { Self.updateCommand($0, changed: changed, known: known) }
      }
    }
    if check != nil {
      toast.secondaryTitle = "Run Check"
      toast.secondaryAction = { [weak self] in self?.runProjectCommand(root: root, workspace: workspace) { $0.checkScript } }
    }
    toasts.show(toast)
  }

  /// What brings the checkout up to date after `changed`: the
  /// `[on_change]` commands for the files that changed, else the setup
  /// script when a known dependency file changed.
  private static func updateCommand(_ settings: ProjectConfig?, changed: [String], known: [String]) -> String? {
    let rules = DependencyChanges.matches(rules: settings?.onChange ?? [:], changed: changed)
    if !rules.isEmpty { return rules.map(\.command).joined(separator: " && ") }
    return known.isEmpty ? nil : settings?.setupScript
  }

  /// Run a command from the project settings in a new terminal tab of
  /// `workspace`, once they're trusted. `command` picks it from the
  /// trusted settings, so what runs is what was trusted.
  private func runProjectCommand(root: String, workspace: UUID, _ command: @escaping (ProjectConfig) -> String?) {
    withTrustedProjectConfig(root: root) { [weak self] config in
      guard let self else { return }
      guard let command = command(config) else {
        self.toasts.show(Toast(kind: .info, message: "The project settings have nothing to run for this anymore."))
        return
      }
      self.tabManager.activateWorkspace(workspace)
      self.tabManager.addTerminalTab(directory: root, initialCommand: command)
    }
  }
}
