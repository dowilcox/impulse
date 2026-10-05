// Watches a repository's working tree and git directories with FSEvents and
// reports debounced changes. Replaces watching a hardcoded `.git/index` plus a
// polling timer: it sees working-tree edits (including from TUIs and agents),
// index writes, HEAD/ref moves, and operation state files, in normal checkouts
// and linked worktrees alike.

import CoreServices
import Foundation

public final class RepoWatcher {
  public struct Change: OptionSet, Sendable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }
    /// A file in the working tree changed.
    public static let workingTree = Change(rawValue: 1 << 0)
    /// The index changed (stage/unstage/commit/checkout).
    public static let index = Change(rawValue: 1 << 1)
    /// HEAD, a branch, tag, remote ref or stash moved.
    public static let refs = Change(rawValue: 1 << 2)
    /// An operation started/advanced/finished (MERGE_HEAD, rebase-merge/…).
    public static let operation = Change(rawValue: 1 << 3)
  }

  private let root: String
  private let gitDir: String
  private let commonDir: String
  private let debounce: TimeInterval
  private let handler: (Change) -> Void
  private let queue = DispatchQueue(label: "impulse.repo-watcher", qos: .utility)
  private var stream: FSEventStreamRef?
  private var pending: Change = []
  private var flushScheduled = false

  /// - Parameters:
  ///   - handler: called on the main queue with the accumulated changes.
  public init(
    root: String, gitDir: String, commonDir: String, debounce: TimeInterval = 0.25,
    handler: @escaping (Change) -> Void
  ) {
    self.root = Self.canonical(root)
    self.gitDir = Self.canonical(gitDir)
    self.commonDir = Self.canonical(commonDir)
    self.debounce = debounce
    self.handler = handler
  }

  deinit { stop() }

  public func start() {
    guard stream == nil else { return }
    var paths = [root]
    // Linked worktrees keep their gitdir outside the working tree.
    for dir in [gitDir, commonDir] where !dir.hasPrefix(root + "/") && !paths.contains(dir) {
      paths.append(dir)
    }
    var context = FSEventStreamContext(
      version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil,
      copyDescription: nil)
    let callback: FSEventStreamCallback = { _, info, count, eventPaths, _, _ in
      guard let info else { return }
      let watcher = Unmanaged<RepoWatcher>.fromOpaque(info).takeUnretainedValue()
      let cfPaths = unsafeBitCast(eventPaths, to: NSArray.self)
      var changed: [String] = []
      changed.reserveCapacity(count)
      for case let path as String in cfPaths { changed.append(path) }
      watcher.receive(changed)
    }
    let flags = UInt32(
      kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer
        | kFSEventStreamCreateFlagUseCFTypes)
    guard
      let created = FSEventStreamCreate(
        nil, callback, &context, paths as CFArray,
        FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.1, flags)
    else { return }
    FSEventStreamSetDispatchQueue(created, queue)
    FSEventStreamStart(created)
    stream = created
  }

  public func stop() {
    guard let stream else { return }
    FSEventStreamStop(stream)
    FSEventStreamInvalidate(stream)
    FSEventStreamRelease(stream)
    self.stream = nil
  }

  // MARK: - Classification

  private func receive(_ paths: [String]) {
    var change: Change = []
    for path in paths {
      change.formUnion(classify(path))
    }
    guard !change.isEmpty else { return }
    pending.formUnion(change)
    guard !flushScheduled else { return }
    flushScheduled = true
    queue.asyncAfter(deadline: .now() + debounce) { [weak self] in
      guard let self else { return }
      let changes = self.pending
      self.pending = []
      self.flushScheduled = false
      DispatchQueue.main.async { self.handler(changes) }
    }
  }

  /// Exposed for tests.
  func classify(_ path: String) -> Change {
    if let relative = Self.relative(path, to: gitDir) ?? Self.relative(path, to: commonDir) {
      return Self.classifyGitPath(relative)
    }
    if let relative = Self.relative(path, to: root) {
      // The `.git` directory of a normal checkout is under the root.
      if relative == ".git" || relative.hasPrefix(".git/") { return [] }
      return .workingTree
    }
    return []
  }

  static func classifyGitPath(_ relative: String) -> Change {
    // Object writes and lock files are noise; the meaningful change follows.
    if relative.hasPrefix("objects/") || relative.hasSuffix(".lock")
      || relative.hasPrefix("logs/") || relative.contains("impulse-snapshot-")
    {
      return []
    }
    if relative == "index" { return .index }
    if relative == "HEAD" || relative == "ORIG_HEAD" || relative == "FETCH_HEAD"
      || relative == "packed-refs" || relative.hasPrefix("refs/")
    {
      return .refs
    }
    if relative.hasPrefix("rebase-merge") || relative.hasPrefix("rebase-apply")
      || ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "BISECT_LOG", "sequencer"].contains(
        where: { relative == $0 || relative.hasPrefix($0 + "/") })
    {
      return .operation
    }
    // Linked worktree metadata lives under worktrees/<name>/ in the common dir.
    if relative.hasPrefix("worktrees/") {
      let rest = relative.split(separator: "/").dropFirst(2).joined(separator: "/")
      return rest.isEmpty ? .refs : classifyGitPath(rest)
    }
    return []
  }

  static func relative(_ path: String, to base: String) -> String? {
    if path == base { return "" }
    guard path.hasPrefix(base + "/") else { return nil }
    return String(path.dropFirst(base.count + 1))
  }

  static func canonical(_ path: String) -> String {
    let resolved = (path as NSString).resolvingSymlinksInPath
    // FSEvents reports /private/var for /var etc.
    if resolved.hasPrefix("/var/") || resolved.hasPrefix("/tmp/") {
      return "/private" + resolved
    }
    return resolved
  }
}
