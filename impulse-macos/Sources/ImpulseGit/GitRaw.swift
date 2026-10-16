// Internal libgit2 plumbing shared by the GitClient extensions: RAII wrappers,
// error/string helpers, and small path utilities matching Rust `Path` semantics.

import Clibgit2
import Foundation

/// Latest libgit2 error message on this thread, mirroring how git2-rs formats
/// `git2::Error` (message only).
func gitLastError(_ fallback: String = "unknown error") -> String {
  if let err = git_error_last(), let msg = err.pointee.message {
    return String(cString: msg)
  }
  return fallback
}

/// RAII wrapper for a `git_repository`.
final class GitRepo {
  let raw: OpaquePointer

  init(openingAt path: String) throws {
    var repo: OpaquePointer?
    guard git_repository_open(&repo, path) == 0, let opened = repo else {
      throw GitError(gitLastError())
    }
    raw = opened
  }

  init(raw: OpaquePointer) {
    self.raw = raw
  }

  deinit {
    git_repository_free(raw)
  }

  /// Working directory exactly as libgit2 reports it (trailing slash), or nil
  /// for bare repositories.
  var workdirRaw: String? {
    guard let cString = git_repository_workdir(raw) else { return nil }
    return String(cString: cString)
  }

  /// Working directory without a trailing slash. Throws "Bare repository"
  /// exactly like the Rust code's `.ok_or("Bare repository")`.
  func workdir() throws -> String {
    guard let raw = workdirRaw else { throw GitError("Bare repository") }
    return trimTrailingSlash(raw)
  }

  /// HEAD peeled to a tree, or nil (unborn branch / error). Caller frees with
  /// `git_object_free`. Mirrors `repo.head().ok().and_then(|h| h.peel_to_tree().ok())`.
  func headTree() -> OpaquePointer? {
    headPeeled(to: GIT_OBJECT_TREE)
  }

  /// HEAD peeled to a commit, or nil. Caller frees with `git_object_free`.
  func headCommit() -> OpaquePointer? {
    headPeeled(to: GIT_OBJECT_COMMIT)
  }

  private func headPeeled(to type: git_object_t) -> OpaquePointer? {
    var ref: OpaquePointer?
    guard git_repository_head(&ref, raw) == 0, let head = ref else { return nil }
    defer { git_reference_free(head) }
    var object: OpaquePointer?
    guard git_reference_peel(&object, head, type) == 0 else { return nil }
    return object
  }
}

/// Open a git repository for the given path using the cached repo-root lookup,
/// falling back to `git_repository_discover` on cache miss. Port of
/// `git::open_repo`.
func openRepo(at path: String) throws -> GitRepo {
  try openRepo(at: path, lookupDir: repoLookupDirectory(path))
}

/// The directory a path's repository root is cached by: a file's parent
/// (most lookups are for files).
private func repoLookupDirectory(_ path: String) -> String {
  var isDirectory: ObjCBool = false
  let exists = FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory)
  let isFile = exists && !isDirectory.boolValue
  return isFile ? (path as NSString).deletingLastPathComponent : path
}

private func openRepo(at path: String, lookupDir: String) throws -> GitRepo {
  precondition(LibGit2.initialized, "libgit2 failed to initialize")

  if let cachedRoot = RepoCache.shared.root(forDirectory: lookupDir),
    let repo = try? GitRepo(openingAt: cachedRoot)
  {
    return repo
  }
  // Root no longer valid (or never cached) — discover and cache.

  var buf = git_buf()
  defer { git_buf_dispose(&buf) }
  guard git_repository_discover(&buf, path, 1, nil) == 0, let discovered = buf.ptr else {
    throw GitError("Not a git repo: \(gitLastError())")
  }
  let repoDir = String(cString: discovered)

  var rawRepo: OpaquePointer?
  guard git_repository_open(&rawRepo, repoDir) == 0, let opened = rawRepo else {
    throw GitError("Not a git repo: \(gitLastError())")
  }
  let repo = GitRepo(raw: opened)
  guard let root = repo.workdirRaw else { throw GitError("Bare repository") }

  RepoCache.shared.store(root: root, forDirectory: lookupDir)
  return repo
}

/// Run `body` on the repository containing `path`, reusing the handle an
/// earlier short read left idle (see `RepoHandles`). For quick reads of
/// refs and commits only: nothing libgit2 hands out from the repository
/// may outlive `body`.
func withRepo<R>(at path: String, _ body: (GitRepo) throws -> R) throws -> R {
  let lookupDir = repoLookupDirectory(path)
  let reused = RepoCache.shared.root(forDirectory: lookupDir).flatMap { RepoHandles.shared.take(root: $0) }
  let repo = try reused ?? openRepo(at: path, lookupDir: lookupDir)
  defer { RepoHandles.shared.put(repo) }
  return try body(repo)
}

/// Repositories opened for short reads, kept idle for a few seconds so a
/// burst of them (a revision per task, a lookup per branch) opens each
/// repository once instead of re-reading its config every time. A handle
/// serves one caller at a time; one whose folders are gone, or that has
/// been idle a while, is closed instead (an open repository holds on to
/// pack files and some config from when it opened).
final class RepoHandles: @unchecked Sendable {
  static let shared = RepoHandles()

  private let lock = NSLock()
  /// By working directory, as libgit2 reports it (the `RepoCache` roots).
  private var idle: [String: (repo: GitRepo, since: Date)] = [:]
  private let capacity = 8
  private let lifetime: TimeInterval = 5
  private var closeScheduled = false

  var idleCount: Int {
    lock.lock()
    defer { lock.unlock() }
    return idle.count
  }

  /// The idle handle on the repository whose working directory is `root`.
  func take(root: String) -> GitRepo? {
    lock.lock()
    let entry = idle.removeValue(forKey: root)
    lock.unlock()
    guard let entry, Date().timeIntervalSince(entry.since) < lifetime,
      access(root, F_OK) == 0, access(git_repository_path(entry.repo.raw), F_OK) == 0
    else { return nil }
    return entry.repo
  }

  /// Leave `repo` for the next short read of the same repository.
  func put(_ repo: GitRepo) {
    guard let root = repo.workdirRaw else { return }
    lock.lock()
    defer { lock.unlock() }
    idle[root] = (repo, Date())
    if idle.count > capacity, let oldest = idle.min(by: { $0.value.since < $1.value.since })?.key {
      idle[oldest] = nil
    }
    scheduleClose()
  }

  /// Close what's been idle for `lifetime` (lock held).
  private func scheduleClose() {
    guard !closeScheduled else { return }
    closeScheduled = true
    DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + lifetime) { [self] in
      lock.lock()
      defer { lock.unlock() }
      closeScheduled = false
      let now = Date()
      idle = idle.filter { now.timeIntervalSince($0.value.since) < lifetime }
      if !idle.isEmpty { scheduleClose() }
    }
  }
}

// MARK: - C interop helpers

/// Run `body` with a `git_strarray` built from `strings`. The array (and the
/// duplicated C strings backing it) are only valid inside `body`.
func withGitStrarray<R>(_ strings: [String], _ body: (inout git_strarray) throws -> R) rethrows -> R {
  var duped: [UnsafeMutablePointer<CChar>?] = strings.map { strdup($0) }
  defer {
    for pointer in duped { free(pointer) }
  }
  return try duped.withUnsafeMutableBufferPointer { buffer in
    var array = git_strarray(strings: buffer.baseAddress, count: strings.count)
    return try body(&array)
  }
}

/// Full 40-character lowercase hex string for an oid.
func oidHex(_ oid: git_oid) -> String {
  var copy = oid
  var buffer = [CChar](repeating: 0, count: 41)
  git_oid_fmt(&buffer, &copy)
  return String(cString: buffer)
}

func oidIsZero(_ oid: git_oid) -> Bool {
  var copy = oid
  return git_oid_is_zero(&copy) == 1
}

// MARK: - Path helpers (component-wise, matching Rust `Path` semantics)

/// Remove trailing slashes (but keep a lone "/").
func trimTrailingSlash(_ path: String) -> String {
  var out = path
  while out.count > 1 && out.hasSuffix("/") {
    out.removeLast()
  }
  return out
}

/// Resolve symlinks and normalize via realpath(3); nil when the path does not
/// exist. Mirrors `std::fs::canonicalize`.
func canonicalPath(_ path: String) -> String? {
  var buffer = [CChar](repeating: 0, count: Int(PATH_MAX) + 1)
  guard realpath(path, &buffer) != nil else { return nil }
  return String(cString: buffer)
}

/// Lexical normalization (collapse `//` and `.`), mirroring
/// `PathBuf::from(path).components().collect::<PathBuf>()`.
func lexicallyNormalized(_ path: String) -> String {
  let isAbsolute = path.hasPrefix("/")
  let parts = path.split(separator: "/").filter { $0 != "." }.map(String.init)
  let joined = parts.joined(separator: "/")
  if isAbsolute { return "/" + joined }
  return joined.isEmpty ? path : joined
}

func pathComponents(_ path: String) -> [String] {
  path.split(separator: "/").map(String.init)
}

/// Component-wise prefix check, like Rust's `Path::starts_with`.
func hasPathPrefix(_ path: String, _ prefix: String) -> Bool {
  let pathParts = pathComponents(path)
  let prefixParts = pathComponents(prefix)
  guard pathParts.count >= prefixParts.count else { return false }
  return Array(pathParts.prefix(prefixParts.count)) == prefixParts
}

/// Component-wise `Path::strip_prefix`: relative components of `path` under
/// `root`, or nil when `path` is not inside `root`.
func relativePathComponents(of path: String, under root: String) -> [String]? {
  let pathParts = pathComponents(path)
  let rootParts = pathComponents(root)
  guard pathParts.count >= rootParts.count,
    Array(pathParts.prefix(rootParts.count)) == rootParts
  else { return nil }
  return Array(pathParts.dropFirst(rootParts.count))
}

func joinPath(_ root: String, _ rel: String) -> String {
  if rel.isEmpty { return root }
  return trimTrailingSlash(root) + "/" + rel
}

/// stat(2)-based file size (follows symlinks like `fs::metadata`).
func fileSize(_ path: String) -> UInt64? {
  var info = stat()
  guard stat(path, &info) == 0 else { return nil }
  return UInt64(max(0, info.st_size))
}
