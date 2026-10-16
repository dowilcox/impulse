// Moving uncommitted work from one checkout into another on the same
// commit (the main checkout's changes into a new task). The work is saved
// in a safety snapshot first; the target gets the files as they were,
// unstaged (deleted files deleted); then the source puts those files back
// to HEAD. Untracked files that didn't move, and ignored files, stay put,
// and so do folders holding a repository of their own, and files edited
// while the move ran.

import Foundation

public enum ChangeMove {
  public struct Moved: Equatable, Sendable {
    /// The source's working tree and index before the move, for Undo.
    public let snapshot: SafetySnapshot
    /// Files that were changed or added, and files that were deleted: the
    /// target has all of them.
    public let changed: [String]
    public let deleted: [String]
    /// Of those, files edited while the move ran: the target has them as
    /// they were before, and the source kept the edits.
    public var editedDuring: [String] = []
    /// Folders with a git repository of their own (a clone, a submodule).
    /// A snapshot records one only as a commit id, so they can't move:
    /// they stayed in the source.
    public var nestedRepositories: [String] = []

    public var count: Int { changed.count + deleted.count }
  }

  /// One file's difference between two commits.
  struct Change: Equatable {
    /// A, M, D or T.
    let status: String
    let path: String
    /// A gitlink (mode 160000) on either side: a folder holding its own
    /// repository.
    let isRepository: Bool
  }

  /// Move `paths` (nil: every uncommitted file) from the checkout at
  /// `source` to the one at `target`, which has to be on the same commit.
  /// A renamed file needs both its old and new path.
  public static func move(paths: [String]?, from source: String, to target: String) -> Result<Moved, GitOperationError> {
    guard let head = head(source), head == self.head(target) else {
      return .failure(.invalid("The new task isn't on the same commit as the files' checkout."))
    }
    let snapshot: SafetySnapshot
    switch SafetySnapshots.create(reason: "move changes to a new task", root: source) {
    case .success(let created): snapshot = created
    case .failure(let error): return .failure(error)
    }
    var changes = differences(from: head, to: snapshot.commit, root: source)
    if let paths {
      let chosen = Set(paths)
      changes = changes.filter { chosen.contains($0.path) }
    }
    // Moving one would leave an empty folder in the target, and removing it
    // from the source would delete its files and history for good.
    let nested = changes.filter(\.isRepository).map(\.path)
    changes.removeAll(where: \.isRepository)
    guard !changes.isEmpty else {
      if !nested.isEmpty {
        return .failure(
          .invalid("Only folders with a git repository of their own changed (\(naming(nested))), and those can't move."))
      }
      return .failure(.invalid("There are no uncommitted changes to move."))
    }
    let changed = changes.filter { $0.status != "D" }.map(\.path)
    let deleted = changes.filter { $0.status == "D" }.map(\.path)

    // Into the task, unstaged.
    if !changed.isEmpty {
      let written = GitOperations.git(
        GitOperations.literal(["restore", "--overlay", "--source=\(snapshot.commit)", "--worktree", "--"] + changed),
        in: target)
      if case .failure(let error) = written { return .failure(error) }
    }
    for path in deleted { removeFile((target as NSString).appendingPathComponent(path)) }

    // Out of the source: tracked files back to HEAD, new files removed. Only
    // files still as the snapshot has them: an edit made since (by an agent,
    // or the editor saving) stays.
    let edited = Set(editedSince(snapshot, changed: changed, deleted: deleted, root: source))
    let added = changes.filter { $0.status == "A" && !edited.contains($0.path) }.map(\.path)
    let modified = changes.filter { $0.status != "A" && !edited.contains($0.path) }.map(\.path)
    if !modified.isEmpty, case .failure(let error) = GitOperations.discardAll(paths: modified, root: source) {
      return .failure(error)
    }
    if !added.isEmpty {
      _ = GitOperations.git(GitOperations.literal(["rm", "-q", "--cached", "--ignore-unmatch", "--"] + added), in: source)
      for path in added { remove(path, in: source) }
    }
    return .success(
      Moved(
        snapshot: snapshot, changed: changed, deleted: deleted,
        editedDuring: changes.map(\.path).filter(edited.contains), nestedRepositories: nested))
  }

  /// Put moved files back in `source` as they were before the move (the
  /// target is left alone). Files edited during the move were never taken
  /// out, so they're left as they are. So is a file changed in `source`
  /// since the move (it no longer matches HEAD, as the move left it): those
  /// are returned.
  public static func undo(_ moved: Moved, source: String) -> Result<[String], GitOperationError> {
    let edited = Set(moved.editedDuring)
    guard let since = changedFromHead((moved.changed + moved.deleted).filter { !edited.contains($0) }, root: source)
    else { return .failure(.invalid("Couldn't tell which files changed since the move, so nothing was put back.")) }
    let left = edited.union(since)
    let changed = moved.changed.filter { !left.contains($0) }
    let deleted = moved.deleted.filter { !left.contains($0) }
    if !changed.isEmpty, case .failure(let error) = SafetySnapshots.restore(moved.snapshot, paths: changed, root: source) {
      return .failure(error)
    }
    guard !deleted.isEmpty else { return .success(since) }
    // A deletion that was staged is staged again.
    let staged = deleted.filter { path in
      guard let tree = moved.snapshot.indexTree else { return false }
      let listed = GitOperations.git(GitOperations.literal(["ls-tree", "--name-only", tree, "--", path]), in: source)
      if case .success(let result) = listed { return result.stdout.isEmpty }
      return false
    }
    if !staged.isEmpty {
      _ = GitOperations.git(GitOperations.literal(["rm", "-q", "--cached", "--ignore-unmatch", "--"] + staged), in: source)
    }
    for path in deleted { removeFile((source as NSString).appendingPathComponent(path)) }
    return .success(since)
  }

  /// Of `paths`, those that differ from HEAD in `root`: staged, edited,
  /// deleted or created. Nil when git can't tell.
  static func changedFromHead(_ paths: [String], root: String) -> [String]? {
    guard !paths.isEmpty else { return [] }
    let status = GitOperations.git(
      GitOperations.literal(
        ["status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all", "--ignore-submodules=all", "--"] + paths),
      in: root)
    guard case .success(let result) = status else { return nil }
    // "XY path" each.
    return result.stdout.split(separator: "\0").filter { $0.count > 3 }.map { String($0.dropFirst(3)) }
  }

  /// Files that differ between two commits, renames as a deletion and an
  /// addition.
  static func differences(from a: String, to b: String, root: String) -> [Change] {
    // ":100644 100644 <id> <id> M\0path\0" per file.
    guard case .success(let result) = GitOperations.git(["diff", "--raw", "-z", "--no-renames", a, b], in: root)
    else { return [] }
    let fields = result.stdout.split(separator: "\0", omittingEmptySubsequences: true).map(String.init)
    var list: [Change] = []
    var index = 0
    while index + 1 < fields.count {
      let meta = fields[index].split(separator: " ")
      if meta.count >= 5 {
        list.append(
          Change(
            status: String(meta[4].prefix(1)), path: fields[index + 1],
            isRepository: meta[0] == ":160000" || meta[1] == "160000"))
      }
      index += 2
    }
    return list
  }

  /// Of the files a move took (`changed` and `deleted`), those that no
  /// longer match `snapshot` in `root`: edited, deleted or created again
  /// since it was taken. All of them when git can't tell.
  static func editedSince(_ snapshot: SafetySnapshot, changed: [String], deleted: [String], root: String) -> [String] {
    var edited = deleted.filter { exists((root as NSString).appendingPathComponent($0)) }
    guard !changed.isEmpty else { return edited }
    // The files as they are now, recorded the way the snapshot recorded
    // them (in a temporary index, so the real one is untouched), compared
    // with it.
    guard case .success(let dirResult) = GitOperations.git(["rev-parse", "--absolute-git-dir"], in: root) else {
      return changed + edited
    }
    let gitDir = dirResult.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    let tempIndex = (gitDir as NSString).appendingPathComponent("impulse-move-\(UUID().uuidString).index")
    defer { try? FileManager.default.removeItem(atPath: tempIndex) }
    var env = GitOperations.environment
    env["GIT_INDEX_FILE"] = tempIndex
    func git(_ args: [String]) -> Result<GitCLIResult, GitCLIError> {
      GitCLI.run(args, in: root, environment: env, timeout: 120)
    }
    guard case .success = git(["read-tree", snapshot.commit]),
      case .success = git(GitOperations.literal(["add", "--all", "--"] + changed)),
      case .success(let diff) = git(
        GitOperations.literal(["diff-index", "--cached", "--name-only", "-z", "--no-renames", snapshot.commit, "--"] + changed))
    else { return changed + edited }
    edited += diff.stdout.split(separator: "\0").map(String.init)
    return edited
  }

  private static func head(_ root: String) -> String? {
    guard case .success(let result) = GitOperations.git(["rev-parse", "--verify", "-q", "HEAD"], in: root) else { return nil }
    let value = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    return value.isEmpty ? nil : value
  }

  /// Delete a file, then any folders it leaves empty (not `root` itself).
  private static func remove(_ path: String, in root: String) {
    var target = (root as NSString).appendingPathComponent(path)
    guard removeFile(target) else { return }
    while true {
      target = (target as NSString).deletingLastPathComponent
      // rmdir only takes an empty folder: nothing that appears meanwhile goes.
      guard target.count > root.count, rmdir(target) == 0 else { return }
    }
  }

  /// Delete a file or symbolic link, never a folder (one where a file was
  /// holds what no snapshot recorded). False when nothing was deleted.
  @discardableResult
  static func removeFile(_ path: String) -> Bool {
    guard let type = (try? FileManager.default.attributesOfItem(atPath: path))?[.type] as? FileAttributeType,
      type != .typeDirectory
    else { return false }
    return unlink(path) == 0
  }

  /// Something is at `path` (a broken symbolic link counts).
  private static func exists(_ path: String) -> Bool {
    (try? FileManager.default.attributesOfItem(atPath: path)) != nil
  }

  /// "vendor/lib", or "vendor/lib and 2 more".
  private static func naming(_ paths: [String]) -> String {
    guard let first = paths.first else { return "" }
    return paths.count == 1 ? first : "\(first) and \(paths.count - 1) more"
  }
}
