// Commit history for the History surface, read with `git log` in pages.

import Foundation
import ImpulseKit

public struct LogEntry: Equatable, Sendable {
  public let sha: String
  public let parents: [String]
  public let author: String
  public let email: String
  public let date: Date
  public let subject: String
  /// Decorations: "HEAD -> main", "origin/main", "tag: v1.0".
  public let refs: [String]

  public var shortSha: String { String(sha.prefix(7)) }
}

public enum GitLog {
  public enum Scope: Equatable, Sendable {
    /// HEAD's history.
    case head
    /// Every branch, remote branch and tag (not Impulse's private refs).
    case all
    /// One local branch's history.
    case branch(String)
  }

  /// A page of history, newest first in topological order. `path` limits it
  /// to commits touching that path (following renames for a single file);
  /// `query` adds author/date limits and a path of its own.
  public static func entries(
    root: String, scope: Scope = .head, path: String? = nil, query: HistoryQuery = HistoryQuery(),
    skip: Int = 0, limit: Int = 300
  ) -> Result<[LogEntry], GitOperationError> {
    var args = [
      "log", "--topo-order", "--no-color", "--decorate=short",
      "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1e",
      "--skip=\(max(0, skip))", "-n", "\(max(1, limit))",
    ] + query.gitArguments
    switch scope {
    case .head: args.append("HEAD")
    case .all: args += ["--branches", "--tags", "--remotes", "HEAD"]
    case .branch(let name): args.append("refs/heads/\(name)")
    }
    if let path {
      // --follow only works for a single file; on a folder it follows
      // whichever file git happens to pick.
      if scope != .all, !isDirectory(path, root: root) { args.insert("--follow", at: 1) }
      args += ["--", path]
    } else if let path = query.path {
      args += ["--", path]
    } else {
      // Revisions only, even when a file has the same name.
      args.append("--")
    }
    return GitOperations.git(args, in: root, timeout: 60).map { parse($0.stdout) }
  }

  /// `path` is a folder: on disk, or (once deleted) in history — a pathspec
  /// ending in "/" only matches folders.
  static func isDirectory(_ path: String, root: String) -> Bool {
    if path.hasSuffix("/") || path.isEmpty || path == "." { return true }
    var isDirectory: ObjCBool = false
    if FileManager.default.fileExists(
      atPath: (root as NSString).appendingPathComponent(path), isDirectory: &isDirectory)
    {
      return isDirectory.boolValue
    }
    guard
      case .success(let result) = GitOperations.git(
        GitOperations.literal(["rev-list", "-1", "HEAD", "--", path + "/"]), in: root)
    else { return false }
    return !result.stdout.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }

  /// Everything about one commit, for its details header.
  public static func details(root: String, sha: String) -> CommitDetails? {
    guard
      case .success(let result) = GitOperations.git(
        ["show", "-s", "--no-color", "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ct%x1f%B", sha],
        in: root)
    else { return nil }
    let fields = result.stdout.components(separatedBy: "\u{1f}")
    guard fields.count >= 8 else { return nil }
    return CommitDetails(
      sha: fields[0],
      parents: fields[1].split(separator: " ").map(String.init),
      author: fields[2], email: fields[3],
      date: Date(timeIntervalSince1970: Double(fields[4]) ?? 0),
      committer: fields[5],
      committerDate: Date(timeIntervalSince1970: Double(fields[6]) ?? 0),
      message: fields[7...].joined(separator: "\u{1f}").trimmingCharacters(in: .whitespacesAndNewlines))
  }

  /// Commits on HEAD that its upstream doesn't have yet (outgoing) and the
  /// other way round (incoming). Both empty without an upstream.
  public static func divergence(root: String, limit: Int = 2000) -> (outgoing: Set<String>, incoming: Set<String>) {
    guard
      case .success(let result) = GitOperations.git(
        ["rev-list", "--left-right", "--max-count=\(limit)", "HEAD...@{upstream}"], in: root)
    else { return ([], []) }
    var outgoing = Set<String>()
    var incoming = Set<String>()
    for line in result.stdout.split(separator: "\n") {
      if line.hasPrefix("<") {
        outgoing.insert(String(line.dropFirst()))
      } else if line.hasPrefix(">") {
        incoming.insert(String(line.dropFirst()))
      }
    }
    return (outgoing, incoming)
  }

  /// `user.name` for this repository ("My Commits").
  public static func userName(root: String) -> String? {
    guard case .success(let result) = GitOperations.git(["config", "user.name"], in: root) else { return nil }
    let name = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    return name.isEmpty ? nil : name
  }

  /// Where the current branch left the default branch.
  public struct ForkPoint: Equatable, Sendable {
    /// The default branch ("origin/main").
    public let base: String
    /// The merge base of HEAD and `base`.
    public let sha: String
    /// Commits on HEAD that `base` doesn't have.
    public let branchOnly: Set<String>
  }

  /// HEAD's fork point from the default branch; nil on the default branch
  /// itself, without one, or when the branch is too long to list.
  public static func forkPoint(root: String, limit: Int = 2000) -> ForkPoint? {
    guard let base = GitClient.defaultBaseBranch(repoPath: root),
      case .success(let head) = GitOperations.git(["rev-parse", "--abbrev-ref", "HEAD"], in: root)
    else { return nil }
    let branch = head.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    if branch == base || base.hasSuffix("/" + branch) { return nil }
    guard case .success(let mergeBase) = GitOperations.git(["merge-base", "HEAD", base], in: root),
      case .success(let list) = GitOperations.git(
        ["rev-list", "--max-count=\(limit + 1)", "HEAD", "^\(base)"], in: root)
    else { return nil }
    let sha = mergeBase.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
    let commits = list.stdout.split(separator: "\n").map(String.init)
    guard !sha.isEmpty, commits.count <= limit else { return nil }
    return ForkPoint(base: base, sha: sha, branchOnly: Set(commits))
  }

  static func parse(_ output: String) -> [LogEntry] {
    output.split(separator: "\u{1e}").compactMap { record in
      let fields = record.trimmingCharacters(in: .whitespacesAndNewlines)
        .components(separatedBy: "\u{1f}")
      guard fields.count >= 7, fields[0].count >= 7 else { return nil }
      return LogEntry(
        sha: fields[0],
        parents: fields[1].split(separator: " ").map(String.init),
        author: fields[2],
        email: fields[3],
        date: Date(timeIntervalSince1970: Double(fields[4]) ?? 0),
        subject: fields[5],
        refs: fields[6].isEmpty
          ? [] : fields[6].components(separatedBy: ", ").filter { !$0.isEmpty })
    }
  }
}

public struct CommitDetails: Equatable, Sendable {
  public let sha: String
  public let parents: [String]
  public let author: String
  public let email: String
  public let date: Date
  public let committer: String
  public let committerDate: Date
  /// Subject and body.
  public let message: String

  public var subject: String { message.components(separatedBy: "\n").first ?? "" }
  /// The message after the subject line.
  public var body: String {
    message.components(separatedBy: "\n").dropFirst().joined(separator: "\n")
      .trimmingCharacters(in: .whitespacesAndNewlines)
  }
}
