// What's ignored in a working tree: the files that removing the folder
// (archiving a task worktree) deletes for good, since safety snapshots only
// hold what git would track.

import Foundation

extension GitOperations {
  /// Ignored files and folders in the working tree, relative to `root`. A
  /// folder that's ignored as a whole is one entry ending in "/"
  /// (`node_modules/`). Empty when there are none or git fails.
  public static func ignoredEntries(root: String) -> [String] {
    // `matching`: only what an ignore pattern names, not an untracked folder
    // that merely holds nothing but ignored files.
    guard
      case .success(let result) = git(
        ["status", "--porcelain=v1", "-z", "--ignored=matching", "--untracked-files=normal"], in: root,
        timeout: 30)
    else { return [] }
    return result.stdout.split(separator: "\0").filter { $0.hasPrefix("!! ") }.map { String($0.dropFirst(3)) }
  }
}
