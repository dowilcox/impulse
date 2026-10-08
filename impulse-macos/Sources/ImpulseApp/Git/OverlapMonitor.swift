import AppKit
import ImpulseGit
import ImpulseKit

extension Notification.Name {
  /// A repository's overlaps changed (userInfo: "newPairs": [TaskOverlap.Pair]
  /// that just started overlapping).
  static let taskOverlapsChanged = Notification.Name("impulse.taskOverlapsChanged")
}

/// Where each repository's workspaces change the same files: the main
/// checkout and every task Impulse made (open or not), recomputed a few
/// seconds after any checkout of it changes. Only for repositories with
/// tasks.
final class OverlapMonitor {
  static let shared = OverlapMonitor()

  /// The pairs per repository, by its shared git folder (main thread).
  private(set) var pairs: [String: [TaskOverlap.Pair]] = [:]
  /// What each workspace changes, per repository (main thread).
  private(set) var changes: [String: [TaskOverlap.Changes]] = [:]
  /// Tasks whose branch is merged into their base, per repository (main
  /// thread).
  private(set) var merged: [String: Set<String>] = [:]
  private var pending: [String: DispatchWorkItem] = [:]
  private let queue = DispatchQueue(label: "impulse.overlap", qos: .utility)

  /// Recompute the overlaps of the repository `root` is in, a moment from
  /// now (main thread).
  func schedule(root: String, commonDir: String) {
    pending[commonDir]?.cancel()
    let work = DispatchWorkItem { [weak self] in self?.compute(root: root, commonDir: commonDir) }
    pending[commonDir] = work
    queue.asyncAfter(deadline: .now() + 3, execute: work)
  }

  /// Whether the task at `path` has its branch merged into its base.
  func isMerged(_ path: String) -> Bool {
    let path = TaskRegistry.canonical(path)
    return merged.values.contains { $0.contains(path) }
  }

  /// The pairs a workspace at `path` is part of.
  func pairs(involving path: String) -> [TaskOverlap.Pair] {
    let path = TaskRegistry.canonical(path)
    return pairs.values.flatMap { $0 }.filter { $0.contains(path) }
  }

  private func compute(root: String, commonDir: String) {
    let changes = Self.changes(root: root)
    // Each task's env file is its own by design: never an overlap.
    let settings = (try? MainWindowController.loadProjectConfig(root: root)?.config.get()) ?? ProjectConfig()
    let ignore = settings.overlapIgnore + [settings.envFile]
    let found = changes.count < 2 ? [] : TaskOverlap.pairs(changes, ignoring: ignore)
    let mergedNow = Self.mergedTasks(root: root)
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      let mergedChanged = self.merged[commonDir] ?? [] != mergedNow
      self.merged[commonDir] = mergedNow
      if mergedChanged, found == self.pairs[commonDir] ?? [] {
        NotificationCenter.default.post(name: .taskOverlapsChanged, object: nil, userInfo: ["newPairs": [TaskOverlap.Pair]()])
      }
      self.changes[commonDir] = changes
      let known = Set((self.pairs[commonDir] ?? []).map(\.key))
      guard found != self.pairs[commonDir] ?? [] else { return }
      self.pairs[commonDir] = found
      let started = found.filter { !known.contains($0.key) }
      // In the background, a desktop notification (once, for the app); in
      // front, the key window shows a toast.
      if !started.isEmpty, SettingsStore.shared.settings.taskOverlapNotify, !NSApp.isActive {
        for pair in started {
          DesktopNotifier.shared.post(
            title: "Tasks change the same files", subtitle: nil, body: Self.sentence(pair), thread: commonDir)
        }
      }
      NotificationCenter.default.post(name: .taskOverlapsChanged, object: nil, userInfo: ["newPairs": started])
    }
  }

  /// "interia-upgrade and pulseboard now change the same 3 files".
  static func sentence(_ pair: TaskOverlap.Pair) -> String {
    let count = pair.files.count
    return "\(pair.a.name) and \(pair.b.name) now change the same \(count == 1 ? "file" : "\(count) files")"
  }

  /// What each workspace of the repository changes: the main checkout's
  /// uncommitted files and commits not pushed, and each task's uncommitted
  /// files and commits since it left its base.
  static func changes(root: String) -> [TaskOverlap.Changes] {
    let main = MainWindowController.mainCheckoutRoot(of: root)
    guard let registry = TaskRegistryStore.registry(root: main), !registry.tasks.isEmpty else { return [] }
    let mainSnapshot = GitClient.snapshot(forPath: main)
    var list: [TaskOverlap.Changes] = []
    var mainFiles = uncommitted(mainSnapshot)
    if let upstream = mainSnapshot?.upstream, let base = GitOperations.mergeBase(upstream, "HEAD", root: main) {
      mainFiles.formUnion(GitOperations.changedPaths(from: base, to: "HEAD", root: main))
    }
    list.append(.init(path: TaskRegistry.canonical(main), name: (main as NSString).lastPathComponent, files: mainFiles))
    let fallbackBase = mainSnapshot?.upstream ?? mainSnapshot?.branch
    for task in registry.tasks {
      var files = uncommitted(GitClient.snapshot(forPath: task.path))
      let branch = "refs/heads/\(task.branch)"
      if let base = task.baseRef ?? fallbackBase, let start = GitOperations.mergeBase(base, branch, root: main) {
        files.formUnion(GitOperations.changedPaths(from: start, to: branch, root: main))
      }
      list.append(.init(path: TaskRegistry.canonical(task.path), name: (task.path as NSString).lastPathComponent, files: files))
    }
    return list
  }

  /// Tasks whose branch has work of its own (it moved past where it
  /// started) and is merged into its base, squash merges included.
  static func mergedTasks(root: String) -> Set<String> {
    let main = MainWindowController.mainCheckoutRoot(of: root)
    guard let registry = TaskRegistryStore.registry(root: main) else { return [] }
    var merged = Set<String>()
    for task in registry.tasks {
      let branch = "refs/heads/\(task.branch)"
      guard let start = task.start, let base = task.baseRef,
        let tip = GitClient.resolveCommit(repoPath: main, revision: branch), tip != start,
        GitOperations.isMerged(branch, into: base, root: main)
      else { continue }
      merged.insert(TaskRegistry.canonical(task.path))
    }
    return merged
  }

  private static func uncommitted(_ snapshot: RepoSnapshot?) -> Set<String> {
    guard let snapshot else { return [] }
    return Set((snapshot.staged + snapshot.unstaged + snapshot.untracked + snapshot.conflicted).map(\.path))
  }

  /// The files that would conflict if two workspaces' work met, uncommitted
  /// work included (off the main thread). Nil when git can't tell.
  static func conflicts(_ pair: TaskOverlap.Pair) -> [String]? {
    guard let a = SafetySnapshots.workingTreeCommit(root: pair.a.path),
      let b = SafetySnapshots.workingTreeCommit(root: pair.b.path)
    else { return nil }
    return GitOperations.predictConflicts(a, b, root: pair.a.path)
  }
}
