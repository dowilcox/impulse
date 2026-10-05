// Commit history for the History surface, read with `git log` in pages.

import Foundation

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
  }

  /// A page of history, newest first in topological order. `path` limits it
  /// to commits touching that path (following renames for a single file).
  public static func entries(
    root: String, scope: Scope = .head, path: String? = nil, skip: Int = 0, limit: Int = 300
  ) -> Result<[LogEntry], GitOperationError> {
    var args = [
      "log", "--topo-order", "--no-color", "--decorate=short",
      "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1e",
      "--skip=\(max(0, skip))", "-n", "\(max(1, limit))",
    ]
    switch scope {
    case .head: args.append("HEAD")
    case .all: args += ["--branches", "--tags", "--remotes", "HEAD"]
    }
    if let path {
      if scope == .head, !path.hasSuffix("/") { args.insert("--follow", at: 1) }
      args += ["--", path]
    }
    return GitOperations.git(args, in: root, timeout: 60).map { parse($0.stdout) }
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
