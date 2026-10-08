import AppKit
import ImpulseGit
import ImpulseKit

extension Notification.Name {
  /// A repository's overlaps changed (userInfo: "newPairs": [TaskOverlap.Pair]
  /// that just started overlapping).
  static let taskOverlapsChanged = Notification.Name("impulse.taskOverlapsChanged")
  /// Tasks Finish pushed for review are merged now (`paths`), and no
  /// window has offered to clean them up yet.
  static let reviewedTasksMerged = Notification.Name("impulse.reviewedTasksMerged")
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
  /// Tasks whose branch's upstream was deleted on the remote (a hint that
  /// it was merged there), per repository (main thread).
  private(set) var upstreamGone: [String: Set<String>] = [:]
  /// Reviewed tasks already offered a clean-up (this launch).
  private var offeredCleanUp = Set<String>()
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

  /// Whether the task at `path` has a branch whose upstream is gone.
  func isUpstreamGone(_ path: String) -> Bool {
    let path = TaskRegistry.canonical(path)
    return upstreamGone.values.contains { $0.contains(path) }
  }

  /// Claim the one clean-up offer for the reviewed task at `path`: true
  /// the first time (that window shows it), false after.
  func claimCleanUpOffer(_ path: String) -> Bool {
    offeredCleanUp.insert(path).inserted
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
    let mergedRecords = Self.mergedRecords(root: root)
    let mergedNow = Set(mergedRecords.map { TaskRegistry.canonical($0.path) })
    let reviewed = Set(mergedRecords.filter { $0.pushedForReview != nil }.map { TaskRegistry.canonical($0.path) })
    let goneNow = Self.upstreamGoneTasks(root: root)
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      let unoffered = reviewed.subtracting(self.offeredCleanUp)
      if !unoffered.isEmpty {
        NotificationCenter.default.post(name: .reviewedTasksMerged, object: nil, userInfo: ["paths": Array(unoffered)])
      }
      let finishedChanged = self.merged[commonDir] ?? [] != mergedNow || self.upstreamGone[commonDir] ?? [] != goneNow
      self.merged[commonDir] = mergedNow
      self.upstreamGone[commonDir] = goneNow
      if finishedChanged, found == self.pairs[commonDir] ?? [] {
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
    // Tasks from before Impulse kept its list don't know their base: the
    // repository's default branch (not the main checkout's branch, which
    // may be a feature branch the task never came from).
    let fallbackBase = GitClient.defaultBaseBranch(repoPath: main)
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
    Set(mergedRecords(root: root).map { TaskRegistry.canonical($0.path) })
  }

  static func mergedRecords(root: String) -> [TaskRecord] {
    let main = MainWindowController.mainCheckoutRoot(of: root)
    guard let registry = TaskRegistryStore.registry(root: main) else { return [] }
    return registry.tasks.filter { task in
      let branch = "refs/heads/\(task.branch)"
      guard let start = task.start, let base = task.baseRef,
        let tip = GitClient.resolveCommit(repoPath: main, revision: branch), tip != start
      else { return false }
      return GitOperations.isMerged(branch, into: base, root: main)
    }
  }

  /// Tasks whose branch tracked a remote branch that's gone (deleted on
  /// the remote and pruned by a fetch).
  static func upstreamGoneTasks(root: String) -> Set<String> {
    let main = MainWindowController.mainCheckoutRoot(of: root)
    guard let registry = TaskRegistryStore.registry(root: main), !registry.tasks.isEmpty else { return [] }
    let gone = Set(GitOperations.branchDetails(root: main, base: nil).filter(\.upstreamGone).map(\.name))
    return Set(registry.tasks.filter { gone.contains($0.branch) }.map { TaskRegistry.canonical($0.path) })
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
