// Ports of `get_git_branch`, `list_git_branches`, and `get_git_root` from
// impulse-core/src/git.rs.

import Clibgit2
import Foundation

extension GitClient {
  /// Current branch name for the repository containing `path`, or an
  /// abbreviated commit hash when HEAD is detached. Nil when the path is not
  /// in a git repository or HEAD is unborn (matching the FFI's null result).
  public static func branch(forPath path: String) -> String? {
    guard let repo = try? openRepo(at: path) else { return nil }

    var ref: OpaquePointer?
    guard git_repository_head(&ref, repo.raw) == 0, let head = ref else { return nil }
    defer { git_reference_free(head) }

    if git_reference_is_branch(head) == 1 {
      guard let shorthand = git_reference_shorthand(head) else { return nil }
      return String(cString: shorthand)
    }
    // Detached HEAD — return abbreviated commit hash.
    guard let target = git_reference_target(head) else { return nil }
    return String(oidHex(target.pointee).prefix(7))
  }

  /// Local branch names for the repository containing `path`, sorted
  /// alphabetically (byte order, matching Rust's `sort()`). Empty when the
  /// path is not in a git repository or on error (matching the FFI).
  public static func branches(forPath path: String) -> [String] {
    guard let repo = try? openRepo(at: path) else { return [] }

    var iterator: OpaquePointer?
    guard git_branch_iterator_new(&iterator, repo.raw, GIT_BRANCH_LOCAL) == 0,
      let iter = iterator
    else { return [] }
    defer { git_branch_iterator_free(iter) }

    var names: [String] = []
    while true {
      var ref: OpaquePointer?
      var branchType = GIT_BRANCH_LOCAL
      let rc = git_branch_next(&ref, &branchType, iter)
      if rc == GIT_ITEROVER.rawValue { break }
      guard rc == 0, let branch = ref else { return [] }
      defer { git_reference_free(branch) }

      var name: UnsafePointer<CChar>?
      if git_branch_name(&name, branch) == 0, let cName = name {
        names.append(String(cString: cName))
      }
    }
    return names.sorted { $0.utf8.lexicographicallyPrecedes($1.utf8) }
  }

  /// Git working directory root (no trailing slash) for the given path, or nil
  /// when the path is not inside a git repository.
  public static func repoRoot(forPath path: String) -> String? {
    guard let repo = try? openRepo(at: path) else { return nil }
    return try? repo.workdir()
  }
}
