// Port of `list_changed_files` (plus `delta_line_stats` / `status_letter`)
// from impulse-core/src/git.rs. JSON encoding of the public types matches the
// Rust serde output byte-for-byte at the key level (snake_case, explicit nulls).

import Clibgit2
import Foundation

/// A single changed file in the working tree relative to HEAD (index + worktree).
public struct ChangedFile: Codable, Equatable, Sendable {
  /// Repo-relative path of the file (new path for renames).
  public let path: String
  /// Status letter: "A", "M", "D", "R", or "?" (untracked).
  public let status: String
  /// Original repo-relative path for renames; nil otherwise.
  public let oldPath: String?
  public let added: UInt32
  public let removed: UInt32
  public let isBinary: Bool

  public init(
    path: String, status: String, oldPath: String?, added: UInt32, removed: UInt32,
    isBinary: Bool
  ) {
    self.path = path
    self.status = status
    self.oldPath = oldPath
    self.added = added
    self.removed = removed
    self.isBinary = isBinary
  }

  enum CodingKeys: String, CodingKey {
    case path, status, added, removed
    case oldPath = "old_path"
    case isBinary = "is_binary"
  }

  public func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    try container.encode(path, forKey: .path)
    try container.encode(status, forKey: .status)
    // Explicit null to match serde's serialization of Option::None.
    try container.encode(oldPath, forKey: .oldPath)
    try container.encode(added, forKey: .added)
    try container.encode(removed, forKey: .removed)
    try container.encode(isBinary, forKey: .isBinary)
  }
}

/// The complete set of uncommitted changes in a repository.
public struct ChangeSet: Codable, Equatable, Sendable {
  /// Absolute path of the repository working directory root (no trailing slash).
  public let repoRoot: String
  /// Current branch name, or nil if detached/unavailable.
  public let branch: String?
  public let totalAdded: UInt32
  public let totalRemoved: UInt32
  public let files: [ChangedFile]

  public init(
    repoRoot: String, branch: String?, totalAdded: UInt32, totalRemoved: UInt32,
    files: [ChangedFile]
  ) {
    self.repoRoot = repoRoot
    self.branch = branch
    self.totalAdded = totalAdded
    self.totalRemoved = totalRemoved
    self.files = files
  }

  enum CodingKeys: String, CodingKey {
    case branch, files
    case repoRoot = "repo_root"
    case totalAdded = "total_added"
    case totalRemoved = "total_removed"
  }

  public func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    try container.encode(repoRoot, forKey: .repoRoot)
    try container.encode(branch, forKey: .branch)
    try container.encode(totalAdded, forKey: .totalAdded)
    try container.encode(totalRemoved, forKey: .totalRemoved)
    try container.encode(files, forKey: .files)
  }
}

/// Map a `git_delta_t` status to the contract's status letter.
func statusLetter(_ status: git_delta_t) -> String {
  switch status {
  case GIT_DELTA_UNTRACKED: return "?"
  case GIT_DELTA_ADDED: return "A"
  case GIT_DELTA_DELETED: return "D"
  case GIT_DELTA_RENAMED, GIT_DELTA_COPIED: return "R"
  // Modified, Typechange, and everything else map to modified.
  default: return "M"
  }
}

/// Per-file (added, removed, isBinary) stats for one delta of a diff, with the
/// same size guards as the Rust `delta_line_stats`.
func deltaLineStats(
  repo: GitRepo, workdir: String, diff: OpaquePointer, index: Int,
  delta: UnsafePointer<git_diff_delta>
) -> (added: UInt32, removed: UInt32, isBinary: Bool) {
  if delta.pointee.flags & GIT_DIFF_FLAG_BINARY.rawValue != 0 {
    return (0, 0, true)
  }

  // Size guard: stat the worktree file and inspect the HEAD blob length.
  if let pathPointer = delta.pointee.new_file.path {
    let absolute = joinPath(workdir, String(cString: pathPointer))
    if let size = fileSize(absolute), size > maxDiffContentSize {
      return (0, 0, false)
    }
  }
  let oldId = delta.pointee.old_file.id
  if !oidIsZero(oldId) {
    var oid = oldId
    var blobPointer: OpaquePointer?
    if git_blob_lookup(&blobPointer, repo.raw, &oid) == 0, let blob = blobPointer {
      defer { git_blob_free(blob) }
      if git_blob_rawsize(blob) > maxDiffContentSize {
        return (0, 0, false)
      }
    }
  }

  var patchPointer: OpaquePointer?
  guard git_patch_from_diff(&patchPointer, diff, index) == 0 else {
    return (0, 0, false)
  }
  // No patch produced -> treat as binary (libgit2 yields no patch for binary deltas).
  guard let patch = patchPointer else {
    return (0, 0, true)
  }
  defer { git_patch_free(patch) }

  // The binary flag is only reliable once the patch content is computed.
  if let patchDelta = git_patch_get_delta(patch),
    patchDelta.pointee.flags & GIT_DIFF_FLAG_BINARY.rawValue != 0
  {
    return (0, 0, true)
  }

  var context = 0
  var additions = 0
  var deletions = 0
  guard git_patch_line_stats(&context, &additions, &deletions, patch) == 0 else {
    return (0, 0, false)
  }
  return (UInt32(additions), UInt32(deletions), false)
}

extension GitClient {
  /// List all uncommitted changes in the repository containing `repoPath`
  /// (HEAD vs index + working tree), including untracked files and renames.
  /// Nil when the path is not in a git repository or on error.
  public static func changedFiles(repoPath: String) -> ChangeSet? {
    guard let repo = try? openRepo(at: repoPath) else { return nil }
    guard let repoRoot = try? repo.workdir() else { return nil }

    let headTree = repo.headTree()
    defer {
      if let tree = headTree { git_object_free(tree) }
    }

    var options = git_diff_options()
    git_diff_options_init(&options, UInt32(GIT_DIFF_OPTIONS_VERSION))
    // show_untracked_content is required so untracked files produce line
    // stats and binary detection.
    options.flags |=
      GIT_DIFF_INCLUDE_UNTRACKED.rawValue | GIT_DIFF_RECURSE_UNTRACKED_DIRS.rawValue
      | GIT_DIFF_SHOW_UNTRACKED_CONTENT.rawValue

    var diffPointer: OpaquePointer?
    guard git_diff_tree_to_workdir_with_index(&diffPointer, repo.raw, headTree, &options) == 0,
      let diff = diffPointer
    else { return nil }
    defer { git_diff_free(diff) }

    // Detect renames so renamed files report status "R" + old_path.
    guard git_diff_find_similar(diff, nil) == 0 else { return nil }

    var files: [ChangedFile] = []
    var totalAdded: UInt64 = 0
    var totalRemoved: UInt64 = 0

    for index in 0..<git_diff_num_deltas(diff) {
      guard let delta = git_diff_get_delta(diff, index) else { continue }
      let stats = deltaLineStats(
        repo: repo, workdir: repoRoot, diff: diff, index: index, delta: delta)

      let newPath = delta.pointee.new_file.path.map { String(cString: $0) }
      let oldPath = delta.pointee.old_file.path.map { String(cString: $0) }
      // Prefer the new path; fall back to the old path (e.g. deletions).
      guard let path = newPath ?? oldPath else { continue }

      let status = statusLetter(delta.pointee.status)
      let oldPathValue = status == "R" ? oldPath : nil

      totalAdded += UInt64(stats.added)
      totalRemoved += UInt64(stats.removed)

      files.append(
        ChangedFile(
          path: path, status: status, oldPath: oldPathValue, added: stats.added,
          removed: stats.removed, isBinary: stats.isBinary))
    }

    let branch = branch(forPath: repoPath)

    return ChangeSet(
      repoRoot: repoRoot,
      branch: branch,
      totalAdded: UInt32(min(totalAdded, UInt64(UInt32.max))),
      totalRemoved: UInt32(min(totalRemoved, UInt64(UInt32.max))),
      files: files)
  }
}
