// What's ignored in a working tree, and which folders hold a repository of
// their own: what removing the folder (archiving a task worktree) deletes
// for good, since safety snapshots only hold what git would track.

import Foundation

extension GitOperations {
  /// Ignored files and folders in the working tree, relative to `root`. A
  /// folder that's ignored as a whole is one entry ending in "/"
  /// (`node_modules/`). Empty when there are none or git fails
  /// (`ignoredEntryList` tells those apart). Unless `trusted`, git runs
  /// nothing the repository's configuration names.
  public static func ignoredEntries(root: String, trusted: Bool = false) -> [String] {
    ignoredEntryList(root: root, trusted: trusted) ?? []
  }

  /// `ignoredEntries`, or nil when git couldn't list them.
  public static func ignoredEntryList(root: String, trusted: Bool = false) -> [String]? {
    // `matching`: only what an ignore pattern names, not an untracked folder
    // that merely holds nothing but ignored files.
    guard let env = readEnvironment(root: root, trusted: trusted) else { return nil }
    return statusEntries(["--ignored=matching", "--untracked-files=normal"], prefix: "!! ", root: root, environment: env)
  }

  /// Folders holding a git repository of their own, relative to `root` and
  /// ending in "/": untracked ones (a clone made inside the checkout), and
  /// checked-out submodules and committed repositories (a gitlink whose
  /// folder has a `.git`). A safety snapshot records one only as a commit
  /// id, and removing a linked worktree takes its submodules' repositories
  /// with it (they live in its own git folder), so their files and any
  /// commits not pushed would go for good. Nil when git can't tell.
  public static func nestedRepositories(root: String, trusted: Bool = false) -> [String]? {
    guard let env = readEnvironment(root: root, trusted: trusted),
      // With every untracked file listed, git still stops at another
      // repository's folder and lists the folder itself.
      let untracked = statusEntries(["--untracked-files=all"], prefix: "?? ", root: root, environment: env),
      case .success(let index) = GitCLI.run(["ls-files", "-s", "-z"], in: root, environment: env, timeout: 120)
    else { return nil }
    // "160000 <id> <stage>\t<path>" is a gitlink (listed once per stage
    // while it's in conflict).
    var gitlinks: [String] = []
    for entry in index.stdout.split(separator: "\0") where entry.hasPrefix("160000 ") {
      guard let tab = entry.firstIndex(of: "\t") else { continue }
      let path = String(entry[entry.index(after: tab)...])
      let dotGit = ((root as NSString).appendingPathComponent(path) as NSString).appendingPathComponent(".git")
      if FileManager.default.fileExists(atPath: dotGit), !gitlinks.contains(path + "/") { gitlinks.append(path + "/") }
    }
    return untracked.filter { $0.hasSuffix("/") } + gitlinks
  }

  /// The environment for a read in `root`; nil when it can't be made safe.
  private static func readEnvironment(root: String, trusted: Bool) -> [String: String]? {
    trusted ? environment : GitCLI.restrictedEnvironment(root: root, base: environment)
  }

  private static func statusEntries(
    _ options: [String], prefix: String, root: String, environment env: [String: String]
  ) -> [String]? {
    guard
      case .success(let result) = GitCLI.run(
        ["status", "--porcelain=v1", "-z", "--ignore-submodules=all"] + options, in: root, environment: env,
        timeout: 120)
    else { return nil }
    return result.stdout.split(separator: "\0").filter { $0.hasPrefix(prefix) }.map { String($0.dropFirst(prefix.count)) }
  }
}
