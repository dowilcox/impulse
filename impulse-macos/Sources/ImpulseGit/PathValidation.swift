// Ports of `util::validate_path_within_root` and
// `util::validate_rel_path_lexically` from impulse-core/src/util.rs, used by
// the discard operations.

import Foundation

extension GitClient {
  /// Validates that `path` is within `root` after canonicalization. Returns
  /// the canonicalized path on success. If the path does not exist, the parent
  /// directory is canonicalized instead and the filename appended.
  @discardableResult
  public static func validatePathWithinRoot(_ path: String, root: String) throws -> String {
    guard let canonicalRoot = canonicalPath(root) else {
      throw GitError(
        "Failed to canonicalize root '\(root)': \(String(cString: strerror(errno)))")
    }

    let canonicalTarget: String
    if let canonical = canonicalPath(path) {
      canonicalTarget = canonical
    } else {
      // Path doesn't exist yet — canonicalize the parent and append the filename.
      let nsPath = path as NSString
      let parent = nsPath.deletingLastPathComponent
      let fileName = nsPath.lastPathComponent
      if parent.isEmpty {
        throw GitError("Path '\(path)' has no parent directory")
      }
      if fileName.isEmpty || fileName == "/" {
        throw GitError("Path '\(path)' has no file name component")
      }
      guard let canonicalParent = canonicalPath(parent) else {
        throw GitError(
          "Failed to canonicalize parent of '\(path)': \(String(cString: strerror(errno)))")
      }
      canonicalTarget = joinPath(canonicalParent, fileName)
    }

    guard hasPathPrefix(canonicalTarget, canonicalRoot) else {
      throw GitError("Path '\(path)' is outside the workspace root '\(root)'")
    }
    return canonicalTarget
  }

  /// Validate that a repo-relative path is lexically contained within `root`
  /// WITHOUT touching the filesystem. `rel` is rejected if it is absolute or
  /// contains any `..` component. Returns `root` joined with `rel`, normalized
  /// lexically.
  @discardableResult
  public static func validateRelPathLexically(root: String, rel: String) throws -> String {
    if rel.hasPrefix("/") {
      throw GitError("Path '\(rel)' must be relative")
    }

    var normalized = root
    for component in rel.split(separator: "/") {
      switch component {
      case ".":
        continue
      case "..":
        throw GitError("Path '\(rel)' must not contain '..' components")
      default:
        normalized = joinPath(normalized, String(component))
      }
    }

    // Defensive: ensure the lexical join did not escape the root.
    guard hasPathPrefix(normalized, root) else {
      throw GitError("Path '\(rel)' is outside the workspace root '\(root)'")
    }
    return normalized
  }
}
