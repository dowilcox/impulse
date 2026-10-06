// Scope-aware diffs (unstaged, staged, branch, commit, range, stash, snapshot)
// for the review surface and hunk/line staging. The hunks the UI shows and
// the patch text used for staging come from the same libgit2 diff, so hunk
// indices and line indices line up exactly (git's own diff can split hunks
// differently because of its heuristics).

import Clibgit2
import CryptoKit
import Foundation
import ImpulseKit

/// Diff presentation options that also affect hunk boundaries.
public struct DiffOptions: Equatable, Hashable, Sendable, Codable {
  public var contextLines: Int
  public var ignoreWhitespace: Bool

  public init(contextLines: Int = 3, ignoreWhitespace: Bool = false) {
    self.contextLines = contextLines
    self.ignoreWhitespace = ignoreWhitespace
  }
}

/// One file's diff in a scope. `hunkIds` parallels `hunks`: a content hash
/// used to make sure a hunk the user acted on still exists.
public struct FileDiff: Codable, Equatable, Sendable {
  public let path: String
  public let oldPath: String?
  public let status: ChangeStatus
  public let language: String
  public let isBinary: Bool
  public let tooLarge: Bool
  public let truncated: Bool
  public let added: Int
  public let removed: Int
  public let hunks: [DiffHunk]
  public let hunkIds: [String]
  /// Old side has no newline at end of file / new side has none.
  public let oldMissingNewlineAtEnd: Bool
  public let newMissingNewlineAtEnd: Bool
}

extension GitClient {
  // MARK: - Public API

  /// Files changed in `scope`, with line counts (skipped above
  /// `maxCountedFiles`).
  public static func changedFiles(repoPath: String, scope: DiffScope) throws -> [FileChange] {
    let repo = try openRepo(at: repoPath)
    let diff = try makeDiff(repo: repo, scope: scope, pathspec: [], options: DiffOptions())
    defer { git_diff_free(diff) }
    _ = git_diff_find_similar(diff, nil)

    let count = git_diff_num_deltas(diff)
    let stats = count <= maxCountedFiles ? lineStats(diff: diff) : [:]
    var changes: [FileChange] = []
    changes.reserveCapacity(count)
    for index in 0..<count {
      guard let delta = git_diff_get_delta(diff, index) else { continue }
      let newPath = delta.pointee.new_file.path.map { String(cString: $0) }
      let oldPath = delta.pointee.old_file.path.map { String(cString: $0) }
      guard let path = newPath ?? oldPath else { continue }
      let status = changeStatus(delta.pointee.status)
      let stat = stats[path]
      changes.append(
        FileChange(
          path: path, oldPath: status == .renamed ? oldPath : nil, status: status,
          added: stat.flatMap { $0.isBinary ? nil : $0.added },
          removed: stat.flatMap { $0.isBinary ? nil : $0.removed },
          isBinary: stat?.isBinary ?? (delta.pointee.flags & GIT_DIFF_FLAG_BINARY.rawValue != 0)))
    }
    return changes
  }

  /// The diff of one file in `scope`. `oldPath` (renames) widens the
  /// pathspec so the rename pairs up instead of showing as add + delete.
  public static func fileDiff(
    repoPath: String, path: String, oldPath: String? = nil, scope: DiffScope,
    options: DiffOptions = DiffOptions()
  ) throws -> FileDiff {
    let repo = try openRepo(at: repoPath)
    let workdir = try repo.workdir()
    try validateRelPathLexically(root: workdir, rel: path)
    let paths = [path] + (oldPath.map { [$0] } ?? [])
    let diff = try makeDiff(repo: repo, scope: scope, pathspec: paths, options: options)
    defer { git_diff_free(diff) }
    _ = git_diff_find_similar(diff, nil)
    let language = languageForPath(joinPath(workdir, path))

    guard let (index, delta) = findDelta(diff: diff, path: path, oldPath: oldPath) else {
      return FileDiff(
        path: path, oldPath: oldPath, status: .modified, language: language, isBinary: false,
        tooLarge: false, truncated: false, added: 0, removed: 0, hunks: [], hunkIds: [],
        oldMissingNewlineAtEnd: false, newMissingNewlineAtEnd: false)
    }
    return extractFileDiff(
      repo: repo, diff: diff, index: index, delta: delta, path: path, language: language)
  }

  /// Unified-diff text for one file in a stageable scope (`.unstaged` or
  /// `.staged`), suitable for `git apply`. Nil when the file has no changes.
  public static func patchText(
    repoPath: String, path: String, oldPath: String? = nil, scope: DiffScope,
    options: DiffOptions = DiffOptions()
  ) throws -> String? {
    let repo = try openRepo(at: repoPath)
    let workdir = try repo.workdir()
    try validateRelPathLexically(root: workdir, rel: path)
    let paths = [path] + (oldPath.map { [$0] } ?? [])
    let diff = try makeDiff(repo: repo, scope: scope, pathspec: paths, options: options)
    defer { git_diff_free(diff) }
    _ = git_diff_find_similar(diff, nil)
    guard let (index, _) = findDelta(diff: diff, path: path, oldPath: oldPath) else { return nil }

    var patchPointer: OpaquePointer?
    guard git_patch_from_diff(&patchPointer, diff, index) == 0, let patch = patchPointer else {
      return nil
    }
    defer { git_patch_free(patch) }
    var buffer = git_buf()
    defer { git_buf_dispose(&buffer) }
    guard git_patch_to_buf(&buffer, patch) == 0, let pointer = buffer.ptr else { return nil }
    // Strictly UTF-8: a lossy decode would turn other encodings' bytes into
    // U+FFFD, which a partial patch then writes into the index or file.
    guard
      let text = String(
        data: Data(UnsafeRawBufferPointer(start: pointer, count: buffer.size)), encoding: .utf8)
    else { throw PatchBuilder.BuildError.notUTF8 }
    return text
  }

  /// Resolve a revision (branch, tag, sha, `HEAD~2`, ...) to a full commit id.
  public static func resolveCommit(repoPath: String, revision: String) -> String? {
    guard let repo = try? openRepo(at: repoPath),
      let commit = try? peelCommit(repo: repo, revision: revision)
    else { return nil }
    defer { git_commit_free(commit) }
    guard let oid = git_commit_id(commit) else { return nil }
    return oidHex(oid.pointee)
  }

  /// The default branch to compare against: the upstream's remote HEAD
  /// (`origin/HEAD` → `origin/main`), else a local `main`/`master`.
  public static func defaultBaseBranch(repoPath: String) -> String? {
    guard let repo = try? openRepo(at: repoPath) else { return nil }
    var ref: OpaquePointer?
    if git_reference_lookup(&ref, repo.raw, "refs/remotes/origin/HEAD") == 0, let symbolic = ref {
      defer { git_reference_free(symbolic) }
      if let target = git_reference_symbolic_target(symbolic) {
        let name = String(cString: target)
        let prefix = "refs/remotes/"
        return name.hasPrefix(prefix) ? String(name.dropFirst(prefix.count)) : name
      }
    }
    for candidate in ["main", "master", "trunk", "develop"] {
      var branch: OpaquePointer?
      if git_branch_lookup(&branch, repo.raw, candidate, GIT_BRANCH_LOCAL) == 0, let found = branch {
        git_reference_free(found)
        return candidate
      }
    }
    return nil
  }

  // MARK: - Diff construction

  /// Build a libgit2 diff for `scope`, limited to `pathspec` when non-empty.
  /// Caller frees with `git_diff_free`.
  static func makeDiff(
    repo: GitRepo, scope: DiffScope, pathspec: [String], options diffOptions: DiffOptions
  ) throws -> OpaquePointer {
    var options = git_diff_options()
    git_diff_options_init(&options, UInt32(GIT_DIFF_OPTIONS_VERSION))
    options.context_lines = UInt32(max(0, diffOptions.contextLines))
    if diffOptions.ignoreWhitespace {
      options.flags |= GIT_DIFF_IGNORE_WHITESPACE.rawValue
    }
    if scope.includesWorkingTree {
      options.flags |=
        GIT_DIFF_INCLUDE_UNTRACKED.rawValue | GIT_DIFF_RECURSE_UNTRACKED_DIRS.rawValue
        | GIT_DIFF_SHOW_UNTRACKED_CONTENT.rawValue
    }
    if !pathspec.isEmpty {
      options.flags |= GIT_DIFF_DISABLE_PATHSPEC_MATCH.rawValue
    }

    return try withGitStrarray(pathspec) { array -> OpaquePointer in
      options.pathspec = array
      var diffPointer: OpaquePointer?
      let rc: Int32
      switch scope {
      case .uncommitted:
        let head = repo.headTree()
        defer { if let head { git_object_free(head) } }
        rc = git_diff_tree_to_workdir_with_index(&diffPointer, repo.raw, head, &options)

      case .unstaged:
        rc = git_diff_index_to_workdir(&diffPointer, repo.raw, nil, &options)

      case .staged:
        let head = repo.headTree()
        defer { if let head { git_object_free(head) } }
        rc = git_diff_tree_to_index(&diffPointer, repo.raw, head, nil, &options)

      case .branch(let base):
        let baseTree = try mergeBaseTree(repo: repo, base: base)
        defer { git_tree_free(baseTree) }
        rc = git_diff_tree_to_workdir_with_index(&diffPointer, repo.raw, baseTree, &options)

      case .commit(let sha):
        let commit = try peelCommit(repo: repo, revision: sha)
        defer { git_commit_free(commit) }
        let newTree = try commitTree(commit)
        defer { git_tree_free(newTree) }
        var parentTree: OpaquePointer?
        if git_commit_parentcount(commit) > 0 {
          var parent: OpaquePointer?
          if git_commit_parent(&parent, commit, 0) == 0, let p = parent {
            defer { git_commit_free(p) }
            parentTree = try? commitTree(p)
          }
        }
        defer { if let parentTree { git_tree_free(parentTree) } }
        rc = git_diff_tree_to_tree(&diffPointer, repo.raw, parentTree, newTree, &options)

      case .range(let from, let to):
        let fromCommit = try peelCommit(repo: repo, revision: from)
        defer { git_commit_free(fromCommit) }
        let toCommit = try peelCommit(repo: repo, revision: to)
        defer { git_commit_free(toCommit) }
        let fromTree = try commitTree(fromCommit)
        defer { git_tree_free(fromTree) }
        let toTree = try commitTree(toCommit)
        defer { git_tree_free(toTree) }
        rc = git_diff_tree_to_tree(&diffPointer, repo.raw, fromTree, toTree, &options)

      case .stash(let index):
        let stash = try peelCommit(repo: repo, revision: "stash@{\(index)}")
        defer { git_commit_free(stash) }
        let stashTree = try commitTree(stash)
        defer { git_tree_free(stashTree) }
        var base: OpaquePointer?
        guard git_commit_parent(&base, stash, 0) == 0, let baseCommit = base else {
          throw GitError("Stash has no base commit")
        }
        defer { git_commit_free(baseCommit) }
        let baseTree = try commitTree(baseCommit)
        defer { git_tree_free(baseTree) }
        // `--include-untracked` keeps the untracked files in a third parent;
        // without them, dropping a reviewed stash could lose files never seen.
        var untracked: OpaquePointer?
        if git_commit_parentcount(stash) >= 3, git_commit_parent(&untracked, stash, 2) == 0,
          let untrackedCommit = untracked
        {
          defer { git_commit_free(untrackedCommit) }
          let untrackedTree = try commitTree(untrackedCommit)
          defer { git_tree_free(untrackedTree) }
          let combined = try combinedIndex(stashTree, untrackedTree)
          defer { git_index_free(combined) }
          rc = git_diff_tree_to_index(&diffPointer, repo.raw, baseTree, combined, &options)
        } else {
          rc = git_diff_tree_to_tree(&diffPointer, repo.raw, baseTree, stashTree, &options)
        }

      case .snapshot(let from, let to):
        let fromCommit = try peelCommit(repo: repo, revision: from)
        defer { git_commit_free(fromCommit) }
        let fromTree = try commitTree(fromCommit)
        defer { git_tree_free(fromTree) }
        if let to {
          let toCommit = try peelCommit(repo: repo, revision: to)
          defer { git_commit_free(toCommit) }
          let toTree = try commitTree(toCommit)
          defer { git_tree_free(toTree) }
          rc = git_diff_tree_to_tree(&diffPointer, repo.raw, fromTree, toTree, &options)
        } else {
          rc = git_diff_tree_to_workdir_with_index(&diffPointer, repo.raw, fromTree, &options)
        }
      }
      guard rc == 0, let diff = diffPointer else { throw GitError(gitLastError()) }
      return diff
    }
  }

  /// An in-memory index holding `tree`'s files plus `extra`'s.
  static func combinedIndex(_ tree: OpaquePointer, _ extra: OpaquePointer) throws -> OpaquePointer {
    var index: OpaquePointer?
    guard git_index_new(&index) == 0, let index else { throw GitError(gitLastError()) }
    let add: git_treewalk_cb = { root, entry, payload in
      guard let entry, let payload, git_tree_entry_type(entry) == GIT_OBJECT_BLOB else { return 0 }
      let path = String(cString: root!) + String(cString: git_tree_entry_name(entry))
      var indexEntry = git_index_entry()
      indexEntry.mode = git_tree_entry_filemode(entry).rawValue
      indexEntry.id = git_tree_entry_id(entry).pointee
      return path.withCString { cPath in
        indexEntry.path = cPath
        return git_index_add(OpaquePointer(payload), &indexEntry)
      }
    }
    guard git_index_read_tree(index, tree) == 0,
      git_tree_walk(extra, GIT_TREEWALK_PRE, add, UnsafeMutableRawPointer(index)) == 0
    else {
      git_index_free(index)
      throw GitError(gitLastError())
    }
    return index
  }

  static func peelCommit(repo: GitRepo, revision: String) throws -> OpaquePointer {
    var object: OpaquePointer?
    guard git_revparse_single(&object, repo.raw, revision) == 0, let found = object else {
      throw GitError("Unknown revision '\(revision)'")
    }
    defer { git_object_free(found) }
    var peeled: OpaquePointer?
    guard git_object_peel(&peeled, found, GIT_OBJECT_COMMIT) == 0, let commit = peeled else {
      throw GitError("'\(revision)' is not a commit")
    }
    return commit
  }

  static func commitTree(_ commit: OpaquePointer) throws -> OpaquePointer {
    var tree: OpaquePointer?
    guard git_commit_tree(&tree, commit) == 0, let found = tree else {
      throw GitError(gitLastError())
    }
    return found
  }

  /// Tree of merge-base(base, HEAD). Caller frees with `git_tree_free`.
  static func mergeBaseTree(repo: GitRepo, base: String) throws -> OpaquePointer {
    let baseCommit = try peelCommit(repo: repo, revision: base)
    defer { git_commit_free(baseCommit) }
    let headCommit = try peelCommit(repo: repo, revision: "HEAD")
    defer { git_commit_free(headCommit) }
    guard let baseId = git_commit_id(baseCommit), let headId = git_commit_id(headCommit) else {
      throw GitError("Could not read commit ids")
    }
    var mergeBase = git_oid()
    var b = baseId.pointee
    var h = headId.pointee
    guard git_merge_base(&mergeBase, repo.raw, &b, &h) == 0 else {
      throw GitError("No common ancestor with '\(base)'")
    }
    var commit: OpaquePointer?
    guard git_commit_lookup(&commit, repo.raw, &mergeBase) == 0, let found = commit else {
      throw GitError(gitLastError())
    }
    defer { git_commit_free(found) }
    return try commitTree(found)
  }

  // MARK: - Extraction

  static func findDelta(diff: OpaquePointer, path: String, oldPath: String?)
    -> (Int, UnsafePointer<git_diff_delta>)?
  {
    for index in 0..<git_diff_num_deltas(diff) {
      guard let delta = git_diff_get_delta(diff, index) else { continue }
      let newPath = delta.pointee.new_file.path.map { String(cString: $0) }
      let deltaOld = delta.pointee.old_file.path.map { String(cString: $0) }
      if newPath == path || deltaOld == path || (oldPath != nil && deltaOld == oldPath) {
        return (index, delta)
      }
    }
    return nil
  }

  static func changeStatus(_ status: git_delta_t) -> ChangeStatus {
    switch status {
    case GIT_DELTA_ADDED: return .added
    case GIT_DELTA_DELETED: return .deleted
    case GIT_DELTA_RENAMED, GIT_DELTA_COPIED: return .renamed
    case GIT_DELTA_TYPECHANGE: return .typeChanged
    case GIT_DELTA_UNTRACKED: return .untracked
    case GIT_DELTA_CONFLICTED: return .conflicted
    default: return .modified
    }
  }

  static func extractFileDiff(
    repo: GitRepo, diff: OpaquePointer, index: Int, delta: UnsafePointer<git_diff_delta>,
    path: String, language: String
  ) -> FileDiff {
    let status = changeStatus(delta.pointee.status)
    let oldPath =
      status == .renamed ? delta.pointee.old_file.path.map { String(cString: $0) } : nil

    func blank(isBinary: Bool, tooLarge: Bool) -> FileDiff {
      FileDiff(
        path: path, oldPath: oldPath, status: status, language: language, isBinary: isBinary,
        tooLarge: tooLarge, truncated: false, added: 0, removed: 0, hunks: [], hunkIds: [],
        oldMissingNewlineAtEnd: false, newMissingNewlineAtEnd: false)
    }

    if delta.pointee.flags & GIT_DIFF_FLAG_BINARY.rawValue != 0 {
      return blank(isBinary: true, tooLarge: false)
    }
    if delta.pointee.new_file.size > maxDiffContentSize
      || delta.pointee.old_file.size > maxDiffContentSize
    {
      return blank(isBinary: false, tooLarge: true)
    }

    var patchPointer: OpaquePointer?
    guard git_patch_from_diff(&patchPointer, diff, index) == 0, let patch = patchPointer else {
      return blank(isBinary: true, tooLarge: false)
    }
    defer { git_patch_free(patch) }
    if let patchDelta = git_patch_get_delta(patch),
      patchDelta.pointee.flags & GIT_DIFF_FLAG_BINARY.rawValue != 0
    {
      return blank(isBinary: true, tooLarge: false)
    }

    var hunks: [DiffHunk] = []
    var hunkIds: [String] = []
    var added = 0
    var removed = 0
    var totalLines = 0
    var truncated = false
    var oldMissingNewline = false
    var newMissingNewline = false

    outer: for hunkIndex in 0..<git_patch_num_hunks(patch) {
      if hunks.count >= maxDiffHunks || totalLines >= maxDiffTotalLines {
        truncated = true
        break
      }
      var hunkPointer: UnsafePointer<git_diff_hunk>?
      var linesInHunk = 0
      guard git_patch_get_hunk(&hunkPointer, &linesInHunk, patch, hunkIndex) == 0,
        let rawHunk = hunkPointer
      else { break }
      let header = hunkHeader(rawHunk.pointee)

      var lines: [DiffLine] = []
      for lineIndex in 0..<linesInHunk {
        var linePointer: UnsafePointer<git_diff_line>?
        guard git_patch_get_line_in_hunk(&linePointer, patch, hunkIndex, lineIndex) == 0,
          let rawLine = linePointer
        else { continue }
        let origin = UInt8(bitPattern: rawLine.pointee.origin)
        // "\ No newline at end of file" markers are attributes of the line
        // before them, not lines of their own (matches the patch text).
        switch origin {
        case UInt8(ascii: "="):
          oldMissingNewline = true
          newMissingNewline = true
          continue
        case UInt8(ascii: ">"):
          // Old side lacked a final newline; the new side adds one.
          oldMissingNewline = true
          continue
        case UInt8(ascii: "<"):
          // New side lacks a final newline.
          newMissingNewline = true
          continue
        default:
          break
        }
        let kind: DiffLineKind
        switch origin {
        case UInt8(ascii: "+"): kind = .added
        case UInt8(ascii: "-"): kind = .removed
        default: kind = .context
        }
        let content = lineContent(rawLine.pointee)
        if content.utf8.count > maxDiffLineLength {
          return blank(isBinary: false, tooLarge: true)
        }
        if kind == .added { added += 1 }
        if kind == .removed { removed += 1 }
        lines.append(
          DiffLine(
            kind: kind,
            oldLineno: rawLine.pointee.old_lineno < 0 ? nil : UInt32(rawLine.pointee.old_lineno),
            newLineno: rawLine.pointee.new_lineno < 0 ? nil : UInt32(rawLine.pointee.new_lineno),
            content: content, spans: []))
        totalLines += 1
        if totalLines >= maxDiffTotalLines {
          truncated = true
        }
      }
      WordDiff.assignWordSpans(&lines)
      let hunk = DiffHunk(
        oldStart: UInt32(max(0, rawHunk.pointee.old_start)),
        oldLines: UInt32(max(0, rawHunk.pointee.old_lines)),
        newStart: UInt32(max(0, rawHunk.pointee.new_start)),
        newLines: UInt32(max(0, rawHunk.pointee.new_lines)),
        header: header, lines: lines)
      hunks.append(hunk)
      hunkIds.append(hunkIdentity(hunk))
      if truncated { break outer }
    }

    return FileDiff(
      path: path, oldPath: oldPath, status: status, language: language, isBinary: false,
      tooLarge: false, truncated: truncated, added: added, removed: removed, hunks: hunks,
      hunkIds: hunkIds, oldMissingNewlineAtEnd: oldMissingNewline,
      newMissingNewlineAtEnd: newMissingNewline)
  }

  /// Stable identity for a hunk: its changed lines (not positions, which
  /// shift when hunks above it are staged).
  public static func hunkIdentity(_ hunk: DiffHunk) -> String {
    var hasher = SHA256()
    for line in hunk.lines where line.kind != .context {
      hasher.update(data: Data((line.kind == .added ? "+" : "-").utf8))
      hasher.update(data: Data(line.content.utf8))
      hasher.update(data: Data([0x0A]))
    }
    return hasher.finalize().prefix(8).map { String(format: "%02x", $0) }.joined()
  }
}
