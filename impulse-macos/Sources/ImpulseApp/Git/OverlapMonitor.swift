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

  /// `TaskRegistry.canonical` (file system calls), remembered per path: the
  /// tab sync asks about every workspace each time it runs (main thread).
  private var canonicalPaths: [String: String] = [:]

  func canonical(_ path: String) -> String {
    if let known = canonicalPaths[path] { return known }
    let resolved = TaskRegistry.canonical(path)
    if canonicalPaths.count > 256 { canonicalPaths.removeAll(keepingCapacity: true) }
    canonicalPaths[path] = resolved
    return resolved
  }

  /// Whether a checkout going from `old` to `new` can change what it counts
  /// as its work: commits, its branch or upstream moving, or the set of
  /// files it changes. Line counts, stashes and staging can't.
  static func mayChangeOverlaps(from old: RepoSnapshot?, to new: RepoSnapshot) -> Bool {
    guard let old else { return true }
    return old.root != new.root || old.commonDir != new.commonDir || old.headOid != new.headOid
      || old.branch != new.branch || old.upstream != new.upstream || old.ahead != new.ahead
      || old.behind != new.behind || uncommitted(old) != uncommitted(new)
  }

  /// Whether the task at `path` has its branch merged into its base.
  func isMerged(_ path: String) -> Bool {
    let path = canonical(path)
    return merged.values.contains { $0.contains(path) }
  }

  /// Whether the task at `path` has a branch whose upstream is gone.
  func isUpstreamGone(_ path: String) -> Bool {
    let path = canonical(path)
    return upstreamGone.values.contains { $0.contains(path) }
  }

  /// Claim the one clean-up offer for the reviewed task at `path`: true
  /// the first time (that window shows it), false after.
  func claimCleanUpOffer(_ path: String) -> Bool {
    offeredCleanUp.insert(path).inserted
  }

  /// The pairs a workspace at `path` is part of.
  func pairs(involving path: String) -> [TaskOverlap.Pair] {
    let path = canonical(path)
    return pairs.values.flatMap { $0 }.filter { $0.contains(path) }
  }

  private func compute(root: String, commonDir: String) {
    let main = MainWindowController.mainCheckoutRoot(of: root)
    let tasks = Self.tasks(main: main, commonDir: commonDir)
    let changes = Self.changes(main: main, tasks: tasks)
    var found: [TaskOverlap.Pair] = []
    if changes.count >= 2 {
      // Each task's env file is its own by design: never an overlap.
      let settings = (try? MainWindowController.loadProjectConfig(root: root)?.config.get()) ?? ProjectConfig()
      found = TaskOverlap.pairs(changes, ignoring: settings.overlapIgnore + [settings.envFile])
    }
    let mergedRecords = Self.mergedRecords(main: main, tasks: tasks)
    let mergedNow = Set(mergedRecords.map { TaskRegistry.canonical($0.path) })
    let reviewed = Set(mergedRecords.filter { $0.pushedForReview != nil }.map { TaskRegistry.canonical($0.path) })
    let goneNow = Self.upstreamGoneTasks(main: main, tasks: tasks)
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

  /// The repository's tasks, without any git work when it has no linked
  /// worktrees (and so no tasks).
  private static func tasks(main: String, commonDir: String) -> [TaskRecord] {
    let linked = (try? FileManager.default.contentsOfDirectory(
      atPath: (commonDir as NSString).appendingPathComponent("worktrees"))) ?? []
    guard !linked.isEmpty else { return [] }
    return TaskRegistryStore.registry(root: main)?.tasks ?? []
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
    return changes(main: main, tasks: TaskRegistryStore.registry(root: main)?.tasks ?? [])
  }

  private static func changes(main: String, tasks: [TaskRecord]) -> [TaskOverlap.Changes] {
    guard !tasks.isEmpty else { return [] }
    let mainSnapshot = GitClient.snapshot(forPath: main, lineCounts: false)
    // Tasks from before Impulse kept its list don't know their base: the
    // repository's default branch (not the main checkout's branch, which
    // may be a feature branch the task never came from).
    let fallbackBase = GitClient.defaultBaseBranch(repoPath: main)
    let bases = tasks.compactMap { $0.baseRef ?? fallbackBase } + [mainSnapshot?.upstream].compactMap { $0 }
    let commits = GitClient.resolveCommits(
      repoPath: main, revisions: bases + tasks.map { "refs/heads/\($0.branch)" })
    var list: [TaskOverlap.Changes] = []
    var mainFiles = uncommitted(mainSnapshot)
    if let upstream = mainSnapshot?.upstream.flatMap({ commits[$0] }), let head = mainSnapshot?.headOid {
      mainFiles.formUnion(committedPaths(base: upstream, tip: head, root: main))
    }
    list.append(.init(path: TaskRegistry.canonical(main), name: (main as NSString).lastPathComponent, files: mainFiles))
    for task in tasks {
      var files = uncommitted(GitClient.snapshot(forPath: task.path, lineCounts: false))
      if let base = (task.baseRef ?? fallbackBase).flatMap({ commits[$0] }),
        let tip = commits["refs/heads/\(task.branch)"]
      {
        files.formUnion(committedPaths(base: base, tip: tip, root: main))
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

  /// The records of the tasks `mergedTasks` names.
  static func mergedRecords(root: String) -> [TaskRecord] {
    let main = MainWindowController.mainCheckoutRoot(of: root)
    return mergedRecords(main: main, tasks: TaskRegistryStore.registry(root: main)?.tasks ?? [])
  }

  private static func mergedRecords(main: String, tasks: [TaskRecord]) -> [TaskRecord] {
    let candidates = tasks.filter { $0.start != nil && $0.baseRef != nil }
    guard !candidates.isEmpty else { return [] }
    let commits = GitClient.resolveCommits(
      repoPath: main,
      revisions: candidates.compactMap(\.baseRef) + candidates.map { "refs/heads/\($0.branch)" })
    return candidates.filter { task in
      guard let base = task.baseRef.flatMap({ commits[$0] }), let tip = commits["refs/heads/\(task.branch)"],
        tip != task.start
      else { return false }
      return isMerged(tip: tip, base: base, root: main)
    }
  }

  /// Tasks whose branch tracked a remote branch that's gone (deleted on
  /// the remote and pruned by a fetch).
  private static func upstreamGoneTasks(main: String, tasks: [TaskRecord]) -> Set<String> {
    guard !tasks.isEmpty else { return [] }
    let gone = Set(GitOperations.branchDetails(root: main, base: nil).filter(\.upstreamGone).map(\.name))
    return Set(tasks.filter { gone.contains($0.branch) }.map { TaskRegistry.canonical($0.path) })
  }

  static func uncommitted(_ snapshot: RepoSnapshot?) -> Set<String> {
    guard let snapshot else { return [] }
    return Set((snapshot.staged + snapshot.unstaged + snapshot.untracked + snapshot.conflicted).map(\.path))
  }

  // MARK: - Commit pairs

  /// What two commits say about each other never changes, so answers are
  /// kept by (base, tip) commit ids: a recompute only asks git about
  /// branches that moved.
  private static let mergedAnswers = CommitPairCache<Bool>()
  private static let committedAnswers = CommitPairCache<[String]>()

  /// Whether `tip`'s work is in `base` (commit ids).
  private static func isMerged(tip: String, base: String, root: String) -> Bool {
    if let known = mergedAnswers.get(base: base, tip: tip) { return known }
    let answer = GitOperations.isMerged(tip, into: base, root: root)
    mergedAnswers.set(answer, base: base, tip: tip)
    return answer
  }

  /// The files `tip` changed since it left `base` (commit ids).
  private static func committedPaths(base: String, tip: String, root: String) -> [String] {
    if let known = committedAnswers.get(base: base, tip: tip) { return known }
    guard let start = GitOperations.mergeBase(base, tip, root: root) else { return [] }
    let paths = GitOperations.changedPaths(from: start, to: tip, root: root)
    committedAnswers.set(paths, base: base, tip: tip)
    return paths
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

/// Answers by (base, tip) commit ids, from any thread. Bounded: cleared when
/// full.
private final class CommitPairCache<Value>: @unchecked Sendable {
  private let lock = NSLock()
  private var entries: [String: Value] = [:]

  func get(base: String, tip: String) -> Value? {
    lock.lock()
    defer { lock.unlock() }
    return entries[base + ".." + tip]
  }

  func set(_ value: Value, base: String, tip: String) {
    lock.lock()
    defer { lock.unlock() }
    if entries.count >= 2_000 { entries.removeAll() }
    entries[base + ".." + tip] = value
  }
}
