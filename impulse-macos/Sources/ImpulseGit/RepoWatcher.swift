// Watches a repository's working tree and git directories with FSEvents and
// reports debounced changes. Replaces watching a hardcoded `.git/index` plus a
// polling timer: it sees working-tree edits (including from TUIs and agents),
// index writes, HEAD/ref moves, and operation state files, in normal checkouts
// and linked worktrees alike.
//
// Events that can't change what this checkout shows are dropped here, so they
// never cost a status refresh: anything under a gitignored folder
// (node_modules, build output), and in a linked worktree everything in the
// shared git folder but the refs (the main checkout's index and HEAD, other
// worktrees' own files).

import Clibgit2
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
  /// What the event stream calls back into: the watcher can go away while
  /// events are on their way.
  private var target: Target?
  private var pending: Change = []
  private var flushScheduled = false
  /// Gitignored working-tree paths (used on `queue` only).
  private lazy var ignores = IgnoreFilter(root: root)

  private final class Target {
    weak var watcher: RepoWatcher?
    init(_ watcher: RepoWatcher) { self.watcher = watcher }
  }

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
    // Linked worktrees keep their git folders outside the working tree (their
    // own one inside the shared one).
    for dir in [commonDir, gitDir] where !paths.contains(where: { dir == $0 || dir.hasPrefix($0 + "/") }) {
      paths.append(dir)
    }
    let target = Target(self)
    self.target = target
    var context = FSEventStreamContext(
      version: 0, info: Unmanaged.passUnretained(target).toOpaque(),
      retain: { info in
        guard let info else { return nil }
        _ = Unmanaged<Target>.fromOpaque(info).retain()
        return info
      },
      release: { info in
        guard let info else { return }
        Unmanaged<Target>.fromOpaque(info).release()
      },
      copyDescription: nil)
    let callback: FSEventStreamCallback = { _, info, count, eventPaths, _, _ in
      guard let info, let watcher = Unmanaged<Target>.fromOpaque(info).takeUnretainedValue().watcher else { return }
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
      let found = Self.classify(path, root: root, gitDir: gitDir, commonDir: commonDir, isIgnored: ignores.isIgnored)
      // The index decides which ignored files are tracked anyway.
      if !found.isEmpty, found.contains(.index) || Self.changesIgnoreRules(path) { ignores.invalidate() }
      change.formUnion(found)
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

  /// What a change at `path` means for the checkout at `root`, whose own
  /// git files are in `gitDir` and shared ones in `commonDir` (all
  /// canonical). `isIgnored` gets working-tree paths relative to `root`, a
  /// folder with a trailing "/".
  static func classify(
    _ path: String, root: String, gitDir: String, commonDir: String, isIgnored: (String) -> Bool
  ) -> Change {
    // A linked worktree's own git folder is inside the shared one.
    if let relative = relative(path, to: gitDir) {
      return classifyGitPath(relative)
    }
    if let relative = relative(path, to: commonDir) {
      return classifySharedGitPath(relative)
    }
    if let relative = relative(path, to: root) {
      // The `.git` directory of a normal checkout is under the root.
      if relative == ".git" || relative.hasPrefix(".git/") { return [] }
      return classifyWorkingTreePath(relative, isIgnored: isIgnored)
    }
    return []
  }

  /// A path in the working tree matters unless it's ignored or in an
  /// ignored folder (checked from the top, so `node_modules/` is one lookup
  /// for everything in it).
  static func classifyWorkingTreePath(_ relative: String, isIgnored: (String) -> Bool) -> Change {
    let components = relative.split(separator: "/")
    guard let name = components.last else { return .workingTree }
    var folder = ""
    for component in components.dropLast() {
      folder += component + "/"
      if isIgnored(folder) { return [] }
    }
    // New ignore rules change what status lists, whatever they match.
    if name == ".gitignore" { return .workingTree }
    return isIgnored(relative) ? [] : .workingTree
  }

  /// Whether a change at `path` can change what's ignored.
  static func changesIgnoreRules(_ path: String) -> Bool {
    path.hasSuffix("/.gitignore") || path.hasSuffix("/info/exclude")
  }

  /// A path in this checkout's own git folder (all of `.git` in a normal
  /// checkout).
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
    if relative == "info/exclude" { return .workingTree }
    // Everything else, `worktrees/` included: a linked worktree's index and
    // HEAD are its own (its watcher sees them), not this checkout's.
    return []
  }

  /// A path in the git folder a linked worktree shares with the main
  /// checkout: only the refs (and the ignore rules) are shared. The index,
  /// HEAD and operation files there are the main checkout's.
  static func classifySharedGitPath(_ relative: String) -> Change {
    if relative.hasSuffix(".lock") { return [] }
    if relative == "packed-refs" || relative.hasPrefix("refs/") { return .refs }
    if relative == "info/exclude" { return .workingTree }
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

/// Which working-tree paths are ignored as far as status goes: matched by
/// an ignore rule, and nothing in them tracked (git still follows a tracked
/// file under an ignored folder). Answers are kept until the ignore rules
/// or the index change, so a burst of events under `node_modules/` costs one
/// lookup. Not thread-safe: the watcher uses it on its queue.
final class IgnoreFilter {
  private let root: String
  private var repo: GitRepo?
  private var index: OpaquePointer?
  private var indexIsCurrent = false
  private var known: [String: Bool] = [:]
  private static let limit = 4_096

  init(root: String) {
    self.root = root
  }

  deinit {
    if let index { git_index_free(index) }
  }

  /// `relative` is relative to the working tree; a folder ends in "/".
  func isIgnored(_ relative: String) -> Bool {
    if let answer = known[relative] { return answer }
    // A folder seen ignored still is once it's gone (`rm -rf build`), when
    // libgit2 can no longer tell it's a folder.
    if !relative.hasSuffix("/"), known[relative + "/"] == true { return true }
    let answer = check(relative)
    if known.count >= Self.limit { known.removeAll(keepingCapacity: true) }
    known[relative] = answer
    return answer
  }

  /// Forget every answer: the ignore rules or the index changed.
  func invalidate() {
    known.removeAll(keepingCapacity: true)
    indexIsCurrent = false
  }

  private func check(_ relative: String) -> Bool {
    guard LibGit2.initialized else { return false }
    if repo == nil { repo = try? GitRepo(openingAt: root) }
    guard let repo else { return false }
    var ignored: Int32 = 0
    guard git_ignore_path_is_ignored(&ignored, repo.raw, relative) == 0, ignored == 1 else { return false }
    return !isTracked(relative, repo: repo)
  }

  /// Whether the index has `relative` or anything under it. True when the
  /// index can't be read, so the change still counts.
  private func isTracked(_ relative: String, repo: GitRepo) -> Bool {
    if index == nil {
      var opened: OpaquePointer?
      guard git_repository_index(&opened, repo.raw) == 0 else { return true }
      index = opened
      indexIsCurrent = true
    }
    guard let index else { return true }
    if !indexIsCurrent {
      guard git_index_read(index, 0) == 0 else { return true }
      indexIsCurrent = true
    }
    let name = relative.hasSuffix("/") ? String(relative.dropLast()) : relative
    if git_index_get_bypath(index, name, 0) != nil { return true }
    var position = 0
    return git_index_find_prefix(&position, index, name + "/") == 0
  }
}
