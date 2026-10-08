// The tasks Impulse created, kept in the repository's shared git folder
// (`.git/impulse/tasks.json`) so every worktree, window and launch sees the
// same list: each task's folder, branch, the base it was made from, and its
// slot (a small number that per-task ports are worked out from). Worktrees
// made any other way aren't listed, and task features leave them alone.

import Foundation

public struct TaskRecord: Codable, Equatable, Sendable {
  /// The task's folder.
  public var path: String
  public var branch: String
  /// The branch the task was made from (`main`); nil when unknown.
  public var base: String?
  /// The remote `base` is on (`origin`); nil for a local base.
  public var remote: String?
  /// Unique among the repository's tasks while the task exists; the main
  /// checkout is 0. Nil for tasks adopted from before the registry.
  public var slot: Int?
  public var created: Date
  /// The commit the task started at: a branch still there has no work of
  /// its own yet (so it isn't "merged"). Nil for adopted tasks.
  public var start: String?

  public init(
    path: String, branch: String, base: String? = nil, remote: String? = nil, slot: Int? = nil,
    created: Date = Date(), start: String? = nil
  ) {
    self.path = path
    self.branch = branch
    self.base = base
    self.remote = remote
    self.slot = slot
    self.created = created
    self.start = start
  }

  /// `origin/main`, or `main` for a local base; nil when unknown.
  public var baseRef: String? {
    guard let base else { return nil }
    return remote.map { "\($0)/\(base)" } ?? base
  }
}

public struct TaskRegistry: Codable, Equatable, Sendable {
  public var tasks: [TaskRecord] = []

  public init(tasks: [TaskRecord] = []) {
    self.tasks = tasks
  }

  /// `<common git dir>/impulse`: Impulse's folder inside the repository's
  /// shared git folder, never committed and shared by every worktree.
  public static func directory(commonGitDirectory: String) -> String {
    (commonGitDirectory as NSString).appendingPathComponent("impulse")
  }

  public static func filePath(commonGitDirectory: String) -> String {
    (directory(commonGitDirectory: commonGitDirectory) as NSString).appendingPathComponent("tasks.json")
  }

  public func record(forPath path: String) -> TaskRecord? {
    let path = Self.canonical(path)
    return tasks.first { Self.canonical($0.path) == path }
  }

  /// The task with `branch` checked out, if any.
  public func record(forBranch branch: String) -> TaskRecord? {
    tasks.first { $0.branch == branch }
  }

  /// The lowest slot from 1 up that no task holds and `available` accepts.
  public func nextSlot(available: (Int) -> Bool = { _ in true }) -> Int {
    let taken = Set(tasks.compactMap(\.slot))
    var slot = 1
    while taken.contains(slot) || !available(slot) { slot += 1 }
    return slot
  }

  /// Add a task, replacing any record for the same folder.
  public mutating func add(_ record: TaskRecord) {
    remove(path: record.path)
    var record = record
    record.path = Self.canonical(record.path)
    tasks.append(record)
  }

  @discardableResult
  public mutating func remove(path: String) -> TaskRecord? {
    let path = Self.canonical(path)
    guard let index = tasks.firstIndex(where: { Self.canonical($0.path) == path }) else { return nil }
    return tasks.remove(at: index)
  }

  /// Bring the list in line with the repository's worktrees: drop tasks
  /// whose worktree is gone (removed by hand), and adopt worktrees in
  /// `<repo>.worktrees/` that aren't listed, which Impulse made before it
  /// kept this list. Adopted tasks have no base or slot. True when anything
  /// changed.
  @discardableResult
  public mutating func reconcile(
    worktrees: [(path: String, branch: String?)], repoRoot: String, now: Date = Date()
  ) -> Bool {
    let before = tasks
    let present = Set(worktrees.map { Self.canonical($0.path) })
    tasks.removeAll { !present.contains(Self.canonical($0.path)) }
    let folder = Self.canonical(
      (WorktreeTasks.worktreePath(repoRoot: repoRoot, branch: "x") as NSString).deletingLastPathComponent)
    for worktree in worktrees {
      let path = Self.canonical(worktree.path)
      guard let branch = worktree.branch, (path as NSString).deletingLastPathComponent == folder,
        record(forPath: path) == nil
      else { continue }
      tasks.append(TaskRecord(path: path, branch: branch, created: now))
    }
    return tasks != before
  }

  /// The registry in `commonGitDirectory`; empty when there's none yet.
  public static func load(commonGitDirectory: String) -> TaskRegistry {
    let path = filePath(commonGitDirectory: commonGitDirectory)
    guard let data = FileManager.default.contents(atPath: path) else { return TaskRegistry() }
    return (try? decoder.decode(TaskRegistry.self, from: data)) ?? TaskRegistry()
  }

  /// Read, change and write the registry under a lock, so two windows (or
  /// Impulse and Impulse Dev) don't lose each other's changes. A file that
  /// doesn't parse is kept beside it as `tasks.json.bad` before being
  /// replaced.
  @discardableResult
  public static func update<T>(commonGitDirectory: String, _ body: (inout TaskRegistry) -> T) throws -> T {
    let folder = directory(commonGitDirectory: commonGitDirectory)
    try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
    let lockPath = (folder as NSString).appendingPathComponent("tasks.lock")
    let lock = open(lockPath, O_CREAT | O_RDWR, 0o644)
    guard lock >= 0 else { throw CocoaError(.fileWriteUnknown) }
    defer { close(lock) }
    flock(lock, LOCK_EX)
    defer { flock(lock, LOCK_UN) }

    let path = filePath(commonGitDirectory: commonGitDirectory)
    var registry = TaskRegistry()
    if let data = FileManager.default.contents(atPath: path) {
      if let decoded = try? decoder.decode(TaskRegistry.self, from: data) {
        registry = decoded
      } else {
        let bad = path + ".bad"
        try? FileManager.default.removeItem(atPath: bad)
        try? FileManager.default.moveItem(atPath: path, toPath: bad)
      }
    }
    let before = registry
    let result = body(&registry)
    if registry != before {
      try encoder.encode(registry).write(to: URL(fileURLWithPath: path), options: .atomic)
    }
    return result
  }

  /// A path with `.`/`..` and symlinks resolved, also for a folder that no
  /// longer exists (an archived task): its nearest existing parent is
  /// resolved instead. `/var/…` and `/private/var/…` compare equal.
  public static func canonical(_ path: String) -> String {
    let standard = (path as NSString).standardizingPath
    if FileManager.default.fileExists(atPath: standard) {
      return URL(fileURLWithPath: standard).resolvingSymlinksInPath().path
    }
    let parent = (standard as NSString).deletingLastPathComponent
    guard parent != standard, !parent.isEmpty else { return standard }
    return (canonical(parent) as NSString).appendingPathComponent((standard as NSString).lastPathComponent)
  }

  private static let encoder: JSONEncoder = {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    encoder.dateEncodingStrategy = .iso8601
    return encoder
  }()

  private static let decoder: JSONDecoder = {
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    return decoder
  }()
}
