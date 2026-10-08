import Foundation
import ImpulseGit
import ImpulseKit

/// The repository's list of the tasks Impulse made (`ImpulseKit.TaskRegistry`
/// in `.git/impulse/tasks.json`), reached from any checkout of it. Calls do
/// file and git work: run them off the main thread.
enum TaskRegistryStore {
  /// The registry of the repository `root` is in, brought in line with its
  /// worktrees first (tasks removed by hand dropped, older tasks in
  /// `<repo>.worktrees/` adopted). Nil outside a repository.
  static func registry(root: String) -> TaskRegistry? {
    guard let common = GitClient.commonGitDirectory(forPath: root) else { return nil }
    let main = MainWindowController.mainCheckoutRoot(of: root)
    let worktrees = GitOperations.worktrees(root: main).filter { !$0.isBare }.map { ($0.path, $0.branch) }
    let registry = TaskRegistry.load(commonGitDirectory: common)
    var reconciled = registry
    guard reconciled.reconcile(worktrees: worktrees, repoRoot: main) else { return registry }
    return (try? TaskRegistry.update(commonGitDirectory: common) { stored in
      stored.reconcile(worktrees: worktrees, repoRoot: main)
      return stored
    }) ?? reconciled
  }

  /// The record of the task at `path`, if Impulse made it.
  static func record(forPath path: String) -> TaskRecord? {
    registry(root: path)?.record(forPath: path)
  }

  /// Record a new task, giving it the lowest free slot that `available`
  /// accepts (its ports free). `base` is what the New Task sheet's From
  /// held: `origin/main` is split into the remote and its branch, anything
  /// else is kept as typed (empty: the main checkout's branch).
  @discardableResult
  static func recordCreated(
    path: String, branch: String, base: String, root: String, available: @escaping (Int) -> Bool = { _ in true }
  ) -> TaskRecord? {
    guard let common = GitClient.commonGitDirectory(forPath: root) else { return nil }
    let main = MainWindowController.mainCheckoutRoot(of: root)
    var baseBranch: String? = base.isEmpty ? GitOperations.currentBranch(root: main) : base
    var remote: String?
    if let typed = baseBranch,
      let match = GitOperations.remotes(root: main).first(where: { typed.hasPrefix("\($0)/") })
    {
      remote = match
      baseBranch = String(typed.dropFirst(match.count + 1))
    }
    return try? TaskRegistry.update(commonGitDirectory: common) { registry in
      let record = TaskRecord(
        path: path, branch: branch, base: baseBranch, remote: remote, slot: registry.nextSlot(available: available),
        start: GitClient.resolveCommit(repoPath: path, revision: "HEAD"))
      registry.add(record)
      return record
    }
  }

  /// Whether every port of `slot` is free on this Mac.
  static func portsAreFree(_ config: ProjectConfig?, slot: Int) -> Bool {
    guard let config, !config.ports.isEmpty else { return true }
    return TaskEnvironment.ports(config.ports, slot: slot, offset: config.portOffset).values.allSatisfy(PortProbe.isFree)
  }

  /// Who a terminal in `directory` belongs to, for its environment
  /// (`IMPULSE_TASK`, `IMPULSE_TASK_SLOT`, `IMPULSE_REPO_ROOT`): cheap
  /// enough for each new terminal (no git processes, nothing reconciled).
  static func identity(forDirectory directory: String) -> [String: String] {
    guard let common = GitClient.commonGitDirectory(forPath: directory),
      (common as NSString).lastPathComponent == ".git"
    else { return [:] }
    var environment = ["IMPULSE_REPO_ROOT": (common as NSString).deletingLastPathComponent]
    if let worktree = GitClient.repoRoot(forPath: directory),
      let record = TaskRegistry.load(commonGitDirectory: common).record(forPath: worktree)
    {
      environment["IMPULSE_TASK"] = (record.path as NSString).lastPathComponent
      if let slot = record.slot { environment["IMPULSE_TASK_SLOT"] = String(slot) }
    }
    return environment
  }

  /// Forget the task at `path` (archived); the record, for Undo.
  @discardableResult
  static func remove(path: String, root: String) -> TaskRecord? {
    guard let common = GitClient.commonGitDirectory(forPath: root) else { return nil }
    return (try? TaskRegistry.update(commonGitDirectory: common) { $0.remove(path: path) }) ?? nil
  }

  /// Put an archived task's record back (Undo), with its slot when no task
  /// took it meanwhile.
  static func restore(_ record: TaskRecord, root: String) {
    guard let common = GitClient.commonGitDirectory(forPath: root) else { return }
    _ = try? TaskRegistry.update(commonGitDirectory: common) { registry in
      var record = record
      if let slot = record.slot, registry.tasks.contains(where: { $0.slot == slot }) {
        record.slot = registry.nextSlot()
      }
      registry.add(record)
    }
  }
}
