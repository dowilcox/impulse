import Foundation

/// Lists a project's files for quick open, honoring .gitignore.
public enum FileIndex {
  /// Paths of files under `root`, relative to it, sorted. Inside a git
  /// repository this asks git (`ls-files`: tracked + untracked, excluding
  /// ignored), which is fast even for large trees; elsewhere it walks the
  /// directory with the same ignore rules as project search. At most `limit`.
  /// Unless `trusted`, git runs nothing the repository's configuration
  /// names (`GitCLI.restrictedEnvironment`); when that can't be made sure
  /// of, the folder is walked instead.
  public static func files(root: String, limit: Int = 200_000, trusted: Bool = false) -> [String] {
    if GitClient.repoRoot(forPath: root) != nil,
      let environment = trusted ? [String: String]() : GitCLI.restrictedEnvironment(root: root),
      case .success(let result) = GitCLI.run(
        ["ls-files", "--cached", "--others", "--exclude-standard", "-z"], in: root, environment: environment,
        timeout: 15)
    {
      var seen = Set<String>()
      var paths: [String] = []
      for entry in result.stdout.split(separator: "\0", omittingEmptySubsequences: true) {
        let path = String(entry)
        // ls-files lists a file twice while it has unresolved conflicts.
        guard seen.insert(path).inserted else { continue }
        paths.append(path)
        if paths.count >= limit { break }
      }
      return paths
    }

    let base = canonicalPath(root) ?? root
    let prefix = base.hasSuffix("/") ? base : base + "/"
    var paths: [String] = []
    FileSearch.walk(root: root) { path, _, isDirectory in
      guard paths.count < limit else { return false }
      if !isDirectory, path.hasPrefix(prefix) {
        paths.append(String(path.dropFirst(prefix.count)))
      }
      return true
    }
    return paths.sorted()
  }
}
