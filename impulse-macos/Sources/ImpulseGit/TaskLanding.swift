// Landing a task's branch (Finish Task): merging it into its base with a
// merge commit and pushing the base, without touching the main checkout.
// The merge happens in a throwaway worktree on the remote's base, so it
// works whatever state the main checkout is in. Without a remote, the
// merge goes into the main checkout itself, and only when that's safe.

import Foundation

public enum TaskLanding {
  public enum Failure: Error, Equatable {
    /// The branch has no commits the base lacks.
    case nothingToLand
    /// Merging conflicts in these files; nothing was changed.
    case conflicts([String])
    /// The remote had newer commits, also after merging them in once more.
    case rejected(String)
    /// The server turned the push down (a protected branch, a hook).
    case refused(String)
    /// The main checkout isn't on the base branch (no remote).
    case notOnBase(current: String?)
    /// The main checkout has uncommitted changes to tracked files (no
    /// remote).
    case uncommitted(Int)
    /// Untracked files in the main checkout that the merge would overwrite
    /// (no remote); nothing was changed.
    case untracked([String])
    case git(GitOperationError)

    public var message: String {
      switch self {
      case .nothingToLand: return "The branch has no commits that aren't in its base."
      case .conflicts(let files):
        return "Merging conflicts in \(files.count == 1 ? files[0] : "\(files.count) files"). Nothing was changed."
      case .rejected: return "The base kept moving on the remote while merging; try again."
      case .refused(let output): return "The server refused the push. \(output)"
      case .notOnBase(let current):
        return "The main checkout is on \(current ?? "a detached commit"), not the base branch."
      case .uncommitted(let count):
        return "The main checkout has \(count) uncommitted file\(count == 1 ? "" : "s")."
      case .untracked(let files):
        return "The main checkout has untracked \(TaskLanding.naming(files)), which the merge would overwrite. Nothing was changed."
      case .git(let error): return error.message
      }
    }
  }

  public struct Landed: Equatable, Sendable {
    /// The merge commit.
    public let commit: String
    /// The remote moved during the merge, so it was merged and pushed
    /// again.
    public let retried: Bool
    /// What the server said on the push (`remote:` lines included).
    public let output: [String]
  }

  /// git's own message for merging `branch` into `base`.
  public static func message(branch: String, base: String) -> String {
    "Merge branch '\(branch)'" + (["main", "master"].contains(base) ? "" : " into \(base)")
  }

  /// Merge `branch` into `remote`/`base` with a merge commit (`--no-ff`) in
  /// a throwaway detached worktree and push it as `base`. When the push is
  /// refused because the remote moved, fetch, merge again and push once
  /// more. The worktree is removed whatever happens. Hooks run as for any
  /// merge and push, except post-checkout for the throwaway folder.
  public static func mergeAndPush(
    branch: String, base: String, remote: String, root: String, onProgress: ((String) -> Void)? = nil
  ) -> Result<Landed, Failure> {
    let target = "\(remote)/\(base)"
    if isAncestor(branch, of: target, root: root) { return .failure(.nothingToLand) }
    let scratch = (NSTemporaryDirectory() as NSString).appendingPathComponent("impulse-land-\(UUID().uuidString)")
    let added = GitOperations.git(
      ["-c", "core.hooksPath=/dev/null", "worktree", "add", "--detach", "--end-of-options", scratch, target],
      in: root, timeout: 600)
    if case .failure(let error) = added { return .failure(.git(error)) }
    defer {
      _ = GitOperations.git(["worktree", "remove", "--force", scratch], in: root)
      try? FileManager.default.removeItem(atPath: scratch)
      _ = GitOperations.git(["worktree", "prune"], in: root)
    }
    var retried = false
    while true {
      if case .failure(let failure) = merge(branch, base: base, in: scratch) { return .failure(failure) }
      guard case .success(let head) = GitOperations.git(["rev-parse", "HEAD"], in: scratch) else {
        return .failure(.git(.invalid("The merge commit couldn't be read.")))
      }
      var output: [String] = []
      let pushed = GitOperations.git(
        ["push", "--progress", remote, "HEAD:refs/heads/\(base)"], in: scratch, timeout: 600,
        onOutputLine: { line in
          output.append(line)
          onProgress?(line)
        })
      switch pushed {
      case .success:
        return .success(Landed(commit: head.stdout.trimmingCharacters(in: .whitespacesAndNewlines), retried: retried, output: output))
      case .failure(let error):
        let said = error.output ?? error.message
        if said.contains("[remote rejected]") { return .failure(.refused(said)) }
        guard isRejection(said) else { return .failure(.git(error)) }
        guard !retried else { return .failure(.rejected(error.output ?? error.message)) }
        retried = true
        if case .failure(let error) = GitOperations.fetch(remote: remote, root: scratch, onProgress: onProgress) {
          return .failure(.git(error))
        }
        if case .failure(let error) = GitOperations.git(["checkout", "-q", "--detach", target], in: scratch) {
          return .failure(.git(error))
        }
      }
    }
  }

  /// Without a remote: merge `branch` into the main checkout at `root`
  /// with a merge commit, when it's on `base` with no changes to tracked
  /// files. Moving the base under changed files would make them look like
  /// they undo the merge. Untracked files don't count: git refuses to
  /// overwrite one, and nothing changes.
  public static func mergeLocally(branch: String, base: String, root: String) -> Result<Landed, Failure> {
    let current = GitOperations.currentBranch(root: root)
    guard current == base else { return .failure(.notOnBase(current: current)) }
    if case .success(let status) = GitOperations.git(["status", "--porcelain", "--untracked-files=no"], in: root) {
      let count = status.stdout.split(separator: "\n").count
      if count > 0 { return .failure(.uncommitted(count)) }
    }
    if isAncestor(branch, of: "HEAD", root: root) { return .failure(.nothingToLand) }
    if case .failure(let failure) = merge(branch, base: base, in: root) { return .failure(failure) }
    guard case .success(let head) = GitOperations.git(["rev-parse", "HEAD"], in: root) else {
      return .failure(.git(.invalid("The merge commit couldn't be read.")))
    }
    return .success(Landed(commit: head.stdout.trimmingCharacters(in: .whitespacesAndNewlines), retried: false, output: []))
  }

  /// `git merge --no-ff` of `branch` in `folder`; on conflicts the merge
  /// is aborted and the files are named.
  private static func merge(_ branch: String, base: String, in folder: String) -> Result<Void, Failure> {
    let merged = GitOperations.git(
      ["merge", "--no-ff", "--no-edit", "-m", message(branch: branch, base: base), "--end-of-options", branch],
      in: folder, timeout: 600)
    guard case .failure(let error) = merged else { return .success(()) }
    let conflicted = (try? GitOperations.git(["diff", "--name-only", "--diff-filter=U"], in: folder).get())
      .map { $0.stdout.split(separator: "\n").map(String.init) } ?? []
    _ = GitOperations.git(["merge", "--abort"], in: folder)
    let untracked = untrackedInTheWay(error.output ?? "")
    if !untracked.isEmpty { return .failure(.untracked(untracked)) }
    return .failure(conflicted.isEmpty ? .git(error) : .conflicts(conflicted))
  }

  /// The untracked files git refused to overwrite, from a merge's or a
  /// fast-forward's output; empty when that isn't why it failed.
  public static func untrackedInTheWay(_ output: String) -> [String] {
    var lines = output.components(separatedBy: "\n")
    guard let start = lines.firstIndex(where: { $0.contains("untracked working tree files would be overwritten") }) else {
      return []
    }
    lines = Array(lines[(start + 1)...])
    return lines.prefix { $0.hasPrefix("\t") }.map { $0.trimmingCharacters(in: .whitespaces) }
  }

  /// "notes.txt", or "notes.txt and 2 more files".
  public static func naming(_ files: [String]) -> String {
    guard let first = files.first else { return "files" }
    return files.count == 1 ? first : "\(first) and \(files.count - 1) more file\(files.count == 2 ? "" : "s")"
  }

  /// Whether `commit` is already part of `other`.
  public static func isAncestor(_ commit: String, of other: String, root: String) -> Bool {
    if case .success = GitOperations.git(["merge-base", "--is-ancestor", commit, other], in: root) { return true }
    return false
  }

  /// A push refused because the remote has commits this one lacks.
  static func isRejection(_ output: String) -> Bool {
    ["[rejected]", "non-fast-forward", "fetch first"].contains { output.contains($0) }
  }
}
