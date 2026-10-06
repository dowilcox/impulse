// Naming and setup rules for task worktrees: a branch per task, checked
// out next to the repository in `<repo>.worktrees/<branch>`, with the
// untracked files a checkout lacks (.env and friends) copied over.

import Foundation

public enum WorktreeTasks {
  /// A branch name for a task title: lowercase words joined by dashes,
  /// made unique against `taken`.
  public static func branchName(for title: String, taken: Set<String>) -> String {
    var slug = ""
    let folded = title.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: nil)
    for scalar in folded.lowercased().unicodeScalars {
      if scalar.isASCII, CharacterSet.alphanumerics.contains(scalar) || scalar == "/" {
        slug.unicodeScalars.append(scalar)
      } else if !slug.isEmpty, !slug.hasSuffix("-"), !slug.hasSuffix("/") {
        slug.append("-")
      }
    }
    while slug.hasSuffix("-") || slug.hasSuffix("/") { slug.removeLast() }
    slug = String(slug.prefix(48))
    while slug.hasSuffix("-") { slug.removeLast() }
    if slug.isEmpty { slug = "task" }
    var candidate = slug
    var suffix = 2
    while taken.contains(candidate) {
      candidate = "\(slug)-\(suffix)"
      suffix += 1
    }
    return candidate
  }

  /// `<parent>/<repo>.worktrees/<branch>` (slashes in the branch become
  /// dashes so each task is one folder).
  public static func worktreePath(repoRoot: String, branch: String) -> String {
    let root = (repoRoot as NSString).standardizingPath
    let parent = (root as NSString).deletingLastPathComponent
    let name = (root as NSString).lastPathComponent
    let folder = branch.replacingOccurrences(of: "/", with: "-")
    return ((parent as NSString).appendingPathComponent("\(name).worktrees") as NSString)
      .appendingPathComponent(folder)
  }

  /// Files to copy into a new worktree: the patterns in `.worktreeinclude`
  /// (one per line, `#` comments), or `.env` and `.env.local` when there's
  /// no such file.
  public static func includePatterns(fromFile text: String?) -> [String] {
    guard let text else { return [".env", ".env.local"] }
    return text.split(whereSeparator: \.isNewline)
      .map { $0.trimmingCharacters(in: .whitespaces) }
      .filter { !$0.isEmpty && !$0.hasPrefix("#") }
      .map { $0.hasPrefix("/") ? String($0.dropFirst()) : $0 }
  }

  /// Relative paths under `root` matching `patterns`. A pattern names a file
  /// or uses shell wildcards within one folder (`.env*`, `config/*.json`);
  /// folders aren't searched recursively.
  public static func matchingFiles(patterns: [String], root: String) -> [String] {
    let fm = FileManager.default
    var found: [String] = []
    // Only files inside the repository: a pattern from a cloned repo's
    // project.toml mustn't copy `../../.ssh/id_ed25519` into a task.
    for pattern in patterns
    where !pattern.hasPrefix("/") && !pattern.hasPrefix("~")
      && !pattern.split(separator: "/").contains("..")
    {
      let folder = (pattern as NSString).deletingLastPathComponent
      let leaf = (pattern as NSString).lastPathComponent
      let base = folder.isEmpty ? root : (root as NSString).appendingPathComponent(folder)
      let candidates: [String]
      if leaf.contains(where: { "*?[".contains($0) }) {
        candidates = ((try? fm.contentsOfDirectory(atPath: base)) ?? []).filter {
          fnmatch(leaf, $0, 0) == 0
        }
      } else {
        candidates = [leaf]
      }
      for name in candidates {
        let relative = folder.isEmpty ? name : (folder as NSString).appendingPathComponent(name)
        var isDirectory: ObjCBool = false
        let absolute = (root as NSString).appendingPathComponent(relative)
        if fm.fileExists(atPath: absolute, isDirectory: &isDirectory), !isDirectory.boolValue,
          !found.contains(relative)
        {
          found.append(relative)
        }
      }
    }
    return found
  }
}
