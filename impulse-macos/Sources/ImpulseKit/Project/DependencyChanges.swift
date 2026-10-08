// When a checkout's HEAD moves and dependency files come with it (a lock
// file, a Dockerfile), the checkout's installed dependencies are stale: tests
// then fail for the wrong reason. These rules tell which moves matter and
// what to run for the files that changed, from the project's `[on_change]`
// settings.

import Foundation

public enum DependencyChanges {
  /// How HEAD moved, from its reflog entry.
  public enum Move: Equatable, Sendable {
    /// Another branch or commit checked out (or reset to).
    case checkout
    /// Changes brought in: a pull, merge or rebase.
    case merge
    /// The checkout's own commit: whoever made it has what it needs.
    case ownCommit
  }

  /// The move a reflog subject (`git reflog -1 --format=%gs`) describes.
  public static func move(reflog subject: String) -> Move {
    let action = subject.split(separator: ":").first.map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
    if action.hasPrefix("commit") || action.hasPrefix("cherry-pick") || action.hasPrefix("revert") {
      return .ownCommit
    }
    if action.hasPrefix("pull") || action.hasPrefix("merge") || action.hasPrefix("rebase") {
      return .merge
    }
    return .checkout
  }

  /// Files whose change usually means dependencies to install or images
  /// to rebuild.
  public static let knownFiles: Set<String> = Set(ProjectDetector.installCommands.map(\.lockFile)).union([
    "Dockerfile", "docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml",
  ])

  /// The `[on_change]` rules that match `changed` paths, as (file, command)
  /// in the rules' order of names; a command shared by several files once.
  /// A rule names a file (`composer.lock`, matched in any folder), a path
  /// (`web/package-lock.json`) or a pattern (`*.lock`).
  public static func matches(rules: [String: String], changed: [String]) -> [(file: String, command: String)] {
    var result: [(file: String, command: String)] = []
    for name in rules.keys.sorted() {
      guard let command = rules[name], !command.isEmpty,
        let file = changed.first(where: { matches(name, path: $0) }),
        !result.contains(where: { $0.command == command })
      else { continue }
      result.append((file, command))
    }
    return result
  }

  /// The known dependency files among `changed`.
  public static func knownChanged(_ changed: [String]) -> [String] {
    changed.filter { knownFiles.contains(($0 as NSString).lastPathComponent) }
  }

  private static func matches(_ rule: String, path: String) -> Bool {
    if rule.contains("/") { return fnmatch(rule, path, 0) == 0 }
    return fnmatch(rule, (path as NSString).lastPathComponent, 0) == 0
  }
}
