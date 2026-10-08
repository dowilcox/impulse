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
          workspace: id, root: root, move: move, rules: rules, known: known, settings: settings)
      }
    }
  }

  private func showDependencyToast(
    workspace: UUID, root: String, move: DependencyChanges.Move, rules: [(file: String, command: String)],
    known: [String], settings: ProjectConfig?
  ) {
    let files = Array(Set(rules.map(\.file) + known)).sorted()
    let names = files.map { ($0 as NSString).lastPathComponent }
    let listed = names.count <= 2 ? names.joined(separator: " and ") : "\(names[0]), \(names[1]) and \(names.count - 2) more"
    var title: String?
    var command: String?
    if !rules.isEmpty {
      title = "Update Dependencies"
      command = rules.map(\.command).joined(separator: " && ")
    } else if let setup = settings?.setupScript {
      title = "Run Setup"
      command = setup
    }
    let check = move == .merge ? settings?.checkScript : nil
    var toast = Toast(
      kind: .info, message: "\(listed) changed",
      detail: command.map { "Update with: \($0)" } ?? "Reinstall dependencies before running tests.",
      lifetime: 20)
    if let title, let command {
      toast.actionTitle = title
      toast.action = { [weak self] in self?.runProjectCommand(command, root: root, workspace: workspace) }
    }
    if let check {
      toast.secondaryTitle = "Run Check"
      toast.secondaryAction = { [weak self] in self?.runProjectCommand(check, root: root, workspace: workspace) }
    }
    toasts.show(toast)
  }

  /// Run a command from the project settings (once they're trusted) in a new
  /// terminal tab of `workspace`.
  private func runProjectCommand(_ command: String, root: String, workspace: UUID) {
    withTrustedProjectConfig(root: root) { [weak self] _ in
      guard let self else { return }
      self.tabManager.activateWorkspace(workspace)
      self.tabManager.addTerminalTab(directory: root, initialCommand: command)
    }
  }
}
