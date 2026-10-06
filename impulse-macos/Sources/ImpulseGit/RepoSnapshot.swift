// Repository snapshot: everything the git panel, status bar and chips show,
// read in one pass with libgit2. Never writes the index (no
// GIT_STATUS_OPT_UPDATE_INDEX), so it is safe to run in the background while
// the user or an agent runs git in a terminal.

import Clibgit2
import Foundation
import ImpulseKit

public struct RepoSnapshot: Equatable, Sendable {
  /// Working tree root (no trailing slash).
  public let root: String
  /// Per-worktree git directory.
  public let gitDir: String
  /// Shared git directory (differs from gitDir in linked worktrees).
  public let commonDir: String
  /// Current branch short name; nil when HEAD is detached.
  public let branch: String?
  /// Full HEAD commit id; nil when the branch is unborn.
  public let headOid: String?
  public let isDetached: Bool
  public let isUnborn: Bool
  /// Upstream short name (e.g. "origin/main"), when configured.
  public let upstream: String?
  /// Commits on HEAD not on the upstream / on the upstream not on HEAD.
  public let ahead: Int
  public let behind: Int
  public let operation: RepoOperation?
  public let staged: [FileChange]
  public let unstaged: [FileChange]
  public let untracked: [FileChange]
  public let conflicted: [FileChange]
  public let stashCount: Int
  /// True when the untracked list was cut off at `maxUntrackedEntries`.
  public let untrackedTruncated: Bool

  public var hasChanges: Bool {
    !staged.isEmpty || !unstaged.isEmpty || !untracked.isEmpty || !conflicted.isEmpty
  }

  /// Distinct changed paths across all sections.
  public var changedFileCount: Int {
    Set((staged + unstaged + untracked + conflicted).map(\.path)).count
  }

  /// Sum of known line counts across staged + unstaged + untracked.
  public var totalAdded: Int { (staged + unstaged + untracked).compactMap(\.added).reduce(0, +) }
  public var totalRemoved: Int { (staged + unstaged).compactMap(\.removed).reduce(0, +) }
}

extension GitClient {
  /// Cap on untracked entries listed (a stray build directory shouldn't make
  /// the panel unusable).
  static let maxUntrackedEntries = 2_000
  /// Skip per-file line counts when a section has more files than this.
  static let maxCountedFiles = 400

  /// Read a snapshot of the repository containing `path`, or nil when it is
  /// not in a (non-bare) repository.
  public static func snapshot(forPath path: String) -> RepoSnapshot? {
    guard let repo = try? openRepo(at: path), let root = try? repo.workdir() else { return nil }
    let gitDir = trimTrailingSlash(String(cString: git_repository_path(repo.raw)))
    let commonDir = trimTrailingSlash(String(cString: git_repository_commondir(repo.raw)))

    // HEAD / branch.
    var branch: String?
    var headOid: String?
    let isUnborn = git_repository_head_unborn(repo.raw) == 1
    let isDetached = git_repository_head_detached(repo.raw) == 1
    var headRef: OpaquePointer?
    if git_repository_head(&headRef, repo.raw) == 0, let head = headRef {
      defer { git_reference_free(head) }
      if !isDetached, let name = git_reference_shorthand(head) {
        branch = String(cString: name)
      }
      if let target = git_reference_target(head) {
        headOid = oidHex(target.pointee)
      }
    } else if isUnborn {
      // Unborn branch: HEAD names a branch that has no commits yet.
      var unbornRef: OpaquePointer?
      if git_reference_lookup(&unbornRef, repo.raw, "HEAD") == 0, let ref = unbornRef {
        defer { git_reference_free(ref) }
        if let target = git_reference_symbolic_target(ref) {
          let full = String(cString: target)
          branch = full.hasPrefix("refs/heads/") ? String(full.dropFirst(11)) : full
        }
      }
    }

    // Upstream + ahead/behind.
    var upstream: String?
    var ahead = 0
    var behind = 0
    if let branch, !isDetached, !isUnborn {
      var local: OpaquePointer?
      if git_branch_lookup(&local, repo.raw, branch, GIT_BRANCH_LOCAL) == 0, let localRef = local {
        defer { git_reference_free(localRef) }
        var upstreamRef: OpaquePointer?
        if git_branch_upstream(&upstreamRef, localRef) == 0, let up = upstreamRef {
          defer { git_reference_free(up) }
          if let name = git_reference_shorthand(up) { upstream = String(cString: name) }
          if let localTarget = git_reference_target(localRef), let upTarget = git_reference_target(up)
          {
            var a = 0
            var b = 0
            var l = localTarget.pointee
            var u = upTarget.pointee
            if git_graph_ahead_behind(&a, &b, repo.raw, &l, &u) == 0 {
              ahead = a
              behind = b
            }
          }
        }
      }
    }

    let operation = readOperation(repo: repo, gitDir: gitDir)
    let sections = readStatus(repo: repo)
    let stashCount = countStashes(repo: repo)

    return RepoSnapshot(
      root: root, gitDir: gitDir, commonDir: commonDir, branch: branch, headOid: headOid,
      isDetached: isDetached, isUnborn: isUnborn, upstream: upstream, ahead: ahead,
      behind: behind, operation: operation, staged: sections.staged,
      unstaged: sections.unstaged, untracked: sections.untracked,
      conflicted: sections.conflicted, stashCount: stashCount,
      untrackedTruncated: sections.untrackedTruncated)
  }

  // MARK: - Operation state

  static func readOperation(repo: GitRepo, gitDir: String) -> RepoOperation? {
    let state = git_repository_state(repo.raw)
    func readInt(_ relative: String) -> Int? {
      let url = URL(fileURLWithPath: gitDir).appendingPathComponent(relative)
      guard let text = try? String(contentsOf: url, encoding: .utf8) else { return nil }
      return Int(text.trimmingCharacters(in: .whitespacesAndNewlines))
    }
    switch Int(state) {
    case Int(GIT_REPOSITORY_STATE_MERGE.rawValue):
      return .merge
    case Int(GIT_REPOSITORY_STATE_REVERT.rawValue), Int(GIT_REPOSITORY_STATE_REVERT_SEQUENCE.rawValue):
      return .revert
    case Int(GIT_REPOSITORY_STATE_CHERRYPICK.rawValue),
      Int(GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE.rawValue):
      return .cherryPick
    case Int(GIT_REPOSITORY_STATE_BISECT.rawValue):
      return .bisect
    case Int(GIT_REPOSITORY_STATE_REBASE_MERGE.rawValue),
      Int(GIT_REPOSITORY_STATE_REBASE_INTERACTIVE.rawValue):
      return .rebase(step: readInt("rebase-merge/msgnum"), total: readInt("rebase-merge/end"))
    case Int(GIT_REPOSITORY_STATE_REBASE.rawValue):
      return .rebase(step: readInt("rebase-apply/next"), total: readInt("rebase-apply/last"))
    case Int(GIT_REPOSITORY_STATE_APPLY_MAILBOX.rawValue),
      Int(GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE.rawValue):
      return .applyMailbox
    default:
      return nil
    }
  }

  // MARK: - Status sections

  struct StatusSections {
    var staged: [FileChange] = []
    var unstaged: [FileChange] = []
    var untracked: [FileChange] = []
    var conflicted: [FileChange] = []
    var untrackedTruncated = false
  }

  static func readStatus(repo: GitRepo) -> StatusSections {
    var options = git_status_options()
    git_status_options_init(&options, UInt32(GIT_STATUS_OPTIONS_VERSION))
    options.show = GIT_STATUS_SHOW_INDEX_AND_WORKDIR
    options.flags =
      GIT_STATUS_OPT_INCLUDE_UNTRACKED.rawValue | GIT_STATUS_OPT_RECURSE_UNTRACKED_DIRS.rawValue
      | GIT_STATUS_OPT_RENAMES_HEAD_TO_INDEX.rawValue | GIT_STATUS_OPT_SORT_CASE_SENSITIVELY.rawValue

    var listPointer: OpaquePointer?
    guard git_status_list_new(&listPointer, repo.raw, &options) == 0, let list = listPointer else {
      return StatusSections()
    }
    defer { git_status_list_free(list) }

    var sections = StatusSections()
    for index in 0..<git_status_list_entrycount(list) {
      guard let entry = git_status_byindex(list, index) else { continue }
      let status = entry.pointee.status.rawValue

      if status & GIT_STATUS_CONFLICTED.rawValue != 0 {
        if let path = deltaPath(entry.pointee.index_to_workdir ?? entry.pointee.head_to_index) {
          sections.conflicted.append(FileChange(path: path, status: .conflicted))
        }
        continue
      }

      let indexBits =
        GIT_STATUS_INDEX_NEW.rawValue | GIT_STATUS_INDEX_MODIFIED.rawValue
        | GIT_STATUS_INDEX_DELETED.rawValue | GIT_STATUS_INDEX_RENAMED.rawValue
        | GIT_STATUS_INDEX_TYPECHANGE.rawValue
      if status & indexBits != 0, let delta = entry.pointee.head_to_index,
        let path = deltaPath(delta)
      {
        let changeStatus: ChangeStatus
        if status & GIT_STATUS_INDEX_NEW.rawValue != 0 {
          changeStatus = .added
        } else if status & GIT_STATUS_INDEX_DELETED.rawValue != 0 {
          changeStatus = .deleted
        } else if status & GIT_STATUS_INDEX_RENAMED.rawValue != 0 {
          changeStatus = .renamed
        } else if status & GIT_STATUS_INDEX_TYPECHANGE.rawValue != 0 {
          changeStatus = .typeChanged
        } else {
          changeStatus = .modified
        }
        let oldPath = changeStatus == .renamed ? oldDeltaPath(delta) : nil
        sections.staged.append(FileChange(path: path, oldPath: oldPath, status: changeStatus))
      }

      if status & GIT_STATUS_WT_NEW.rawValue != 0 {
        if sections.untracked.count >= maxUntrackedEntries {
          sections.untrackedTruncated = true
        } else if let delta = entry.pointee.index_to_workdir, let path = deltaPath(delta) {
          sections.untracked.append(FileChange(path: path, status: .untracked))
        }
        continue
      }

      let worktreeBits =
        GIT_STATUS_WT_MODIFIED.rawValue | GIT_STATUS_WT_DELETED.rawValue
        | GIT_STATUS_WT_TYPECHANGE.rawValue | GIT_STATUS_WT_RENAMED.rawValue
      if status & worktreeBits != 0, let delta = entry.pointee.index_to_workdir,
        let path = deltaPath(delta)
      {
        let changeStatus: ChangeStatus
        if status & GIT_STATUS_WT_DELETED.rawValue != 0 {
          changeStatus = .deleted
        } else if status & GIT_STATUS_WT_TYPECHANGE.rawValue != 0 {
          changeStatus = .typeChanged
        } else {
          changeStatus = .modified
        }
        sections.unstaged.append(FileChange(path: path, status: changeStatus))
      }
    }

    // Line counts per section, unless the section is huge.
    if sections.staged.count <= maxCountedFiles {
      sections.staged = withLineCounts(sections.staged, repo: repo, scope: .staged)
    }
    if sections.unstaged.count <= maxCountedFiles {
      sections.unstaged = withLineCounts(sections.unstaged, repo: repo, scope: .unstaged)
    }
    if sections.untracked.count <= maxCountedFiles {
      sections.untracked = withLineCounts(sections.untracked, repo: repo, scope: .unstaged)
    }
    return sections
  }

  /// Fill in added/removed counts from the scope's diff (binary-aware).
  static func withLineCounts(_ changes: [FileChange], repo: GitRepo, scope: DiffScope)
    -> [FileChange]
  {
    guard !changes.isEmpty else { return changes }
    let paths = changes.flatMap { [$0.path] + ($0.oldPath.map { [$0] } ?? []) }
    guard let diff = try? makeDiff(repo: repo, scope: scope, pathspec: paths, options: DiffOptions())
    else { return changes }
    defer { git_diff_free(diff) }
    // Pair renames, or a renamed file counts as all-new lines.
    _ = git_diff_find_similar(diff, nil)
    let stats = lineStats(diff: diff)
    return changes.map { change in
      guard let stat = stats[change.path] else { return change }
      return FileChange(
        path: change.path, oldPath: change.oldPath, status: change.status,
        added: stat.isBinary ? nil : stat.added, removed: stat.isBinary ? nil : stat.removed,
        isBinary: stat.isBinary)
    }
  }

  struct LineStat {
    var added: Int
    var removed: Int
    var isBinary: Bool
  }

  /// Per-path line stats for every delta in `diff` (new-side path), skipping
  /// files over the diff size limit.
  static func lineStats(diff: OpaquePointer) -> [String: LineStat] {
    var result: [String: LineStat] = [:]
    for index in 0..<git_diff_num_deltas(diff) {
      guard let delta = git_diff_get_delta(diff, index),
        let path = delta.pointee.new_file.path.map({ String(cString: $0) })
          ?? delta.pointee.old_file.path.map({ String(cString: $0) })
      else { continue }
      if delta.pointee.flags & GIT_DIFF_FLAG_BINARY.rawValue != 0 {
        result[path] = LineStat(added: 0, removed: 0, isBinary: true)
        continue
      }
      if delta.pointee.new_file.size > maxDiffContentSize
        || delta.pointee.old_file.size > maxDiffContentSize
      {
        continue
      }
      var patchPointer: OpaquePointer?
      guard git_patch_from_diff(&patchPointer, diff, index) == 0, let patch = patchPointer else {
        result[path] = LineStat(added: 0, removed: 0, isBinary: true)
        continue
      }
      defer { git_patch_free(patch) }
      if let patchDelta = git_patch_get_delta(patch),
        patchDelta.pointee.flags & GIT_DIFF_FLAG_BINARY.rawValue != 0
      {
        result[path] = LineStat(added: 0, removed: 0, isBinary: true)
        continue
      }
      var context = 0
      var additions = 0
      var deletions = 0
      git_patch_line_stats(&context, &additions, &deletions, patch)
      result[path] = LineStat(added: additions, removed: deletions, isBinary: false)
    }
    return result
  }

  static func countStashes(repo: GitRepo) -> Int {
    var count = 0
    _ = withUnsafeMutablePointer(to: &count) { pointer in
      git_stash_foreach(
        repo.raw,
        { _, _, _, payload in
          payload?.assumingMemoryBound(to: Int.self).pointee += 1
          return 0
        }, pointer)
    }
    return count
  }

  // MARK: - Helpers

  static func deltaPath(_ delta: UnsafeMutablePointer<git_diff_delta>?) -> String? {
    guard let delta else { return nil }
    if let path = delta.pointee.new_file.path { return String(cString: path) }
    if let path = delta.pointee.old_file.path { return String(cString: path) }
    return nil
  }

  static func oldDeltaPath(_ delta: UnsafeMutablePointer<git_diff_delta>?) -> String? {
    guard let delta, let path = delta.pointee.old_file.path else { return nil }
    return String(cString: path)
  }
}
