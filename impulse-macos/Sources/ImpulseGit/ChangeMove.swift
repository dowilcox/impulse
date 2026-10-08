// Moving uncommitted work from one checkout into another on the same
// commit (the main checkout's changes into a new task). The work is saved
// in a safety snapshot first; the target gets the files as they were,
// unstaged (deleted files deleted); then the source puts those files back
// to HEAD. Untracked files that didn't move, and ignored files, stay put.

import Foundation

public enum ChangeMove {
  public struct Moved: Equatable, Sendable {
    /// The source's working tree and index before the move, for Undo.
    public let snapshot: SafetySnapshot
    /// Files that were changed or added, and files that were deleted.
    public let changed: [String]
    public let deleted: [String]

    public var count: Int { changed.count + deleted.count }
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
    let changed = changes.filter { $0.status != "D" }.map(\.path)
    let deleted = changes.filter { $0.status == "D" }.map(\.path)
    let added = changes.filter { $0.status == "A" }.map(\.path)
    let modified = changes.filter { $0.status != "A" }.map(\.path)
    guard !changes.isEmpty else { return .failure(.invalid("There are no uncommitted changes to move.")) }

    // Into the task, unstaged.
    if !changed.isEmpty {
      let written = GitOperations.git(
        GitOperations.literal(["restore", "--overlay", "--source=\(snapshot.commit)", "--worktree", "--"] + changed),
        in: target)
      if case .failure(let error) = written { return .failure(error) }
    }
    for path in deleted { try? FileManager.default.removeItem(atPath: (target as NSString).appendingPathComponent(path)) }

    // Out of the source: tracked files back to HEAD, new files removed.
    if !modified.isEmpty, case .failure(let error) = GitOperations.discardAll(paths: modified, root: source) {
      return .failure(error)
    }
    if !added.isEmpty {
      _ = GitOperations.git(GitOperations.literal(["rm", "-q", "--cached", "--ignore-unmatch", "--"] + added), in: source)
      for path in added { remove(path, in: source) }
    }
    return .success(Moved(snapshot: snapshot, changed: changed, deleted: deleted))
  }

  /// Put moved files back in `source` as they were before the move (the
  /// target is left alone).
  public static func undo(_ moved: Moved, source: String) -> GitResult {
    if !moved.changed.isEmpty, case .failure(let error) = SafetySnapshots.restore(moved.snapshot, paths: moved.changed, root: source) {
      return .failure(error)
    }
    guard !moved.deleted.isEmpty else { return .success(()) }
    // A deletion that was staged is staged again.
    let staged = moved.deleted.filter { path in
      guard let tree = moved.snapshot.indexTree else { return false }
      let listed = GitOperations.git(GitOperations.literal(["ls-tree", "--name-only", tree, "--", path]), in: source)
      if case .success(let result) = listed { return result.stdout.isEmpty }
      return false
    }
    if !staged.isEmpty {
      _ = GitOperations.git(GitOperations.literal(["rm", "-q", "--cached", "--ignore-unmatch", "--"] + staged), in: source)
    }
    for path in moved.deleted { try? FileManager.default.removeItem(atPath: (source as NSString).appendingPathComponent(path)) }
    return .success(())
  }

  /// Files that differ between two commits: A, M, D or T, renames as a
  /// deletion and an addition.
  static func differences(from a: String, to b: String, root: String) -> [(status: String, path: String)] {
    guard case .success(let result) = GitOperations.git(["diff", "--name-status", "-z", "--no-renames", a, b], in: root)
    else { return [] }
    let fields = result.stdout.split(separator: "\0", omittingEmptySubsequences: true).map(String.init)
    var list: [(status: String, path: String)] = []
    var index = 0
    while index + 1 < fields.count {
      list.append((String(fields[index].prefix(1)), fields[index + 1]))
      index += 2
    }
    return list
  }

  private static func head(_ root: String) -> String? {
    guard case .success(let result) = GitOperations.git(["rev-parse", "--verify", "-q", "HEAD"], in: root) else { return nil }
    let value = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    return value.isEmpty ? nil : value
  }

  /// Delete a file, then any folders it leaves empty (not `root` itself).
  private static func remove(_ path: String, in root: String) {
    let fileManager = FileManager.default
    var target = (root as NSString).appendingPathComponent(path)
    try? fileManager.removeItem(atPath: target)
    while true {
      target = (target as NSString).deletingLastPathComponent
      guard target.count > root.count, (try? fileManager.contentsOfDirectory(atPath: target))?.isEmpty == true else { return }
      try? fileManager.removeItem(atPath: target)
    }
  }
}
