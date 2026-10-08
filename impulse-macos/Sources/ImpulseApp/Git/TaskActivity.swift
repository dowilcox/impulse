import AppKit
import ImpulseGit
import ImpulseKit

/// What's happening in a task Impulse made, to warn before its branch is
/// merged while it's still moving: an agent working or waiting there, or
/// files not committed yet. A merge then would miss what comes next.
struct TaskActivity {
  let record: TaskRecord
  /// The branch's commit now, which a merge after the warning uses.
  let commit: String
  let uncommitted: Int
  let lastCommit: Date?
  /// Agents in the task that are working or waiting for input, filled in
  /// on the main thread.
  var agents: [(name: String, state: AgentState)] = []

  var shortCommit: String { String(commit.prefix(7)) }

  /// The task with `branch` checked out, if Impulse made it (off the main
  /// thread). Nil for other branches and commits.
  static func find(branch: String, root: String) -> TaskActivity? {
    guard let record = TaskRegistryStore.registry(root: root)?.record(forBranch: branch),
      let commit = GitClient.resolveCommit(repoPath: root, revision: "refs/heads/\(branch)")
    else { return nil }
    let uncommitted = GitClient.snapshot(forPath: record.path)?.changedFileCount ?? 0
    return TaskActivity(
      record: record, commit: commit, uncommitted: uncommitted,
      lastCommit: GitOperations.commitDate(commit, root: root))
  }

  /// The warning to show before merging, or nil when the task is still.
  /// Looks up the task's agents (main thread).
  mutating func question() -> (title: String, message: String)? {
    agents = (AppDelegate.shared?.agents(inFolder: record.path) ?? []).filter {
      $0.state == .working || $0.state == .needsInput
    }
    guard !agents.isEmpty || uncommitted > 0 else { return nil }
    let task = (record.path as NSString).lastPathComponent
    var what: String
    if let agent = agents.first {
      what = "\(agent.name) is \(agent.state == .working ? "working" : "waiting for input") in the \(task) task"
    } else {
      what = "The \(task) task has \(uncommitted) uncommitted file\(uncommitted == 1 ? "" : "s")"
    }
    if let lastCommit {
      let formatter = RelativeDateTimeFormatter()
      formatter.unitsStyle = .full
      what += ", and its last commit was \(formatter.localizedString(for: lastCommit, relativeTo: Date()))"
    }
    return (
      "\(record.branch) is still being worked on",
      "\(what). Merge \(shortCommit) anyway? Anything committed to \(record.branch) after it won't be included."
    )
  }
}
