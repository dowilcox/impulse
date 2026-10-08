// Where a repository's workspaces change the same files: the main checkout
// and every task Impulse made, each with the files it changes (its commits
// since it left its base, plus its uncommitted files). Two agents rewriting
// the same files only find out when the work is merged; this tells them, and
// you, while it's still small.

import Foundation

public enum TaskOverlap {
  /// One workspace's changes.
  public struct Changes: Equatable, Sendable {
    /// Its folder.
    public let path: String
    public let name: String
    public let files: Set<String>

    public init(path: String, name: String, files: Set<String>) {
      self.path = path
      self.name = name
      self.files = files
    }
  }

  /// Two workspaces that change the same files.
  public struct Pair: Equatable, Sendable {
    public let a: Changes
    public let b: Changes
    public let files: [String]

    /// Identifies the pair, whichever order its workspaces come in.
    public var key: String { [a.path, b.path].sorted().joined(separator: "\n") }

    /// The other workspace of the pair, seen from `path`.
    public func other(than path: String) -> Changes { a.path == path ? b : a }

    public func contains(_ path: String) -> Bool { a.path == path || b.path == path }
  }

  /// Every pair of workspaces with files in common, most shared files
  /// first. Files matching `ignoring` (a name in any folder, a path, or a
  /// pattern) don't count.
  public static func pairs(_ workspaces: [Changes], ignoring patterns: [String] = []) -> [Pair] {
    var pairs: [Pair] = []
    for (index, a) in workspaces.enumerated() {
      for b in workspaces[(index + 1)...] {
        let shared = a.files.intersection(b.files).filter { !isIgnored($0, patterns) }.sorted()
        if !shared.isEmpty { pairs.append(Pair(a: a, b: b, files: shared)) }
      }
    }
    return pairs.sorted { $0.files.count != $1.files.count ? $0.files.count > $1.files.count : $0.key < $1.key }
  }

  static func isIgnored(_ path: String, _ patterns: [String]) -> Bool {
    patterns.contains { pattern in
      pattern.contains("/")
        ? fnmatch(pattern, path, 0) == 0 : fnmatch(pattern, (path as NSString).lastPathComponent, 0) == 0
    }
  }
}
