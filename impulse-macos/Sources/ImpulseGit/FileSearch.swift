import Clibgit2
import Foundation
import ImpulseKit

// Ported from impulse-core/src/search.rs. The Rust version walked with the
// `ignore` crate (gitignore-aware); this port walks with a plain recursive
// DFS and asks libgit2's ignore engine (`git_ignore_path_is_ignored`) — the
// exact semantics git itself uses (nested .gitignore, negations, global
// excludes, info/exclude). Outside a git repository no ignore filtering
// applies (the Rust `ignore` crate honored stray .gitignore files even
// without a repo; searching non-repo trees with .gitignore files is the one
// intentional divergence).

public enum FileSearch {
  static let maxDepth = 15
  static let maxContentFileSize: UInt64 = 1_048_576  // 1 MB
  static let binarySniffBytes = 8192
  static let maxLineContentChars = 500

  /// Search for files by name (substring, case-insensitive).
  public static func searchFilenames(root: String, query: String, limit: Int) -> [SearchResult] {
    let queryLower = query.lowercased()
    var results: [SearchResult] = []
    walk(root: root) { path, name, isDirectory in
      guard results.count < limit else { return false }
      if !isDirectory, name.lowercased().contains(queryLower) {
        results.append(SearchResult(path: path, name: name, matchType: "file"))
      }
      return true
    }
    return results
  }

  /// Search file contents for a text substring. Column positions are in
  /// characters (matching the Rust implementation).
  public static func searchContents(
    root: String, query: String, limit: Int, caseSensitive: Bool
  ) -> [SearchResult] {
    let queryMatch = caseSensitive ? query : query.lowercased()
    let matchCharLen = query.count
    var results: [SearchResult] = []

    walk(root: root) { path, name, isDirectory in
      guard results.count < limit else { return false }
      guard !isDirectory else { return true }

      guard let attrs = try? FileManager.default.attributesOfItem(atPath: path),
        ((attrs[.size] as? UInt64) ?? 0) <= maxContentFileSize,
        let data = FileManager.default.contents(atPath: path)
      else { return true }

      // Binary sniff: NUL byte in the first 8 KB.
      if data.prefix(binarySniffBytes).contains(0) { return true }

      var lineNumber: UInt32 = 0
      for lineData in data.split(separator: 0x0A, omittingEmptySubsequences: false) {
        guard results.count < limit else { break }
        lineNumber += 1
        var slice = lineData
        if slice.last == 0x0D { slice = slice.dropLast() }
        // Rust's BufRead::lines skips lines that are not valid UTF-8.
        guard let line = String(data: slice, encoding: .utf8) else { continue }

        let haystack = caseSensitive ? line : line.lowercased()
        var lineContent: String?
        var searchStart = haystack.startIndex
        var prevMatchIndex = haystack.startIndex
        var prevCharPos = 0

        while results.count < limit,
          let range = haystack.range(of: queryMatch, range: searchStart..<haystack.endIndex)
        {
          // Incremental character-offset computation, like the Rust port.
          let colStart =
            prevCharPos + haystack.distance(from: prevMatchIndex, to: range.lowerBound)
          prevMatchIndex = range.lowerBound
          prevCharPos = colStart

          let content = lineContent ?? String(line.prefix(maxLineContentChars))
          lineContent = content

          results.append(
            SearchResult(
              path: path,
              name: name,
              lineNumber: lineNumber,
              lineContent: content,
              columnStart: UInt32(colStart),
              columnEnd: UInt32(colStart + matchCharLen),
              matchType: "content"
            ))
          searchStart = range.upperBound
        }
      }
      return true
    }
    return results
  }

  /// Every file `searchContents` would report at least one match in, with
  /// no limit (for replacing across the project, where the result list's
  /// cap would leave files out).
  public static func filesContaining(root: String, query: String, caseSensitive: Bool) -> [String] {
    guard !query.isEmpty else { return [] }
    let queryMatch = caseSensitive ? query : query.lowercased()
    var paths: [String] = []
    walk(root: root) { path, _, isDirectory in
      guard !isDirectory else { return true }
      guard let attrs = try? FileManager.default.attributesOfItem(atPath: path),
        ((attrs[.size] as? UInt64) ?? 0) <= maxContentFileSize,
        let data = FileManager.default.contents(atPath: path),
        !data.prefix(binarySniffBytes).contains(0)
      else { return true }
      for lineData in data.split(separator: 0x0A, omittingEmptySubsequences: false) {
        var slice = lineData
        if slice.last == 0x0D { slice = slice.dropLast() }
        guard let line = String(data: slice, encoding: .utf8) else { continue }
        if (caseSensitive ? line : line.lowercased()).range(of: queryMatch) != nil {
          paths.append(path)
          break
        }
      }
      return true
    }
    return paths
  }

  // MARK: - Walk

  /// Depth-first walk skipping hidden entries, `.git`, other filesystems,
  /// gitignored paths (when inside a repo), and anything deeper than
  /// `maxDepth`. The visitor returns false to stop the walk entirely.
  static func walk(
    root: String, visit: (_ path: String, _ name: String, _ isDirectory: Bool) -> Bool
  ) {
    let fm = FileManager.default
    // Canonicalize once (realpath, e.g. /tmp → /private/tmp) so every derived
    // entry path matches libgit2's canonical workdir; symlinked directories
    // are never descended, so children stay canonical. (canonicalPath is the
    // realpath(3) helper from GitRaw.swift.)
    let root = canonicalPath(root) ?? root
    var rootIsDir: ObjCBool = false
    guard fm.fileExists(atPath: root, isDirectory: &rootIsDir), rootIsDir.boolValue else {
      return
    }

    let ignore = IgnoreChecker(root: root)
    let rootDevice = (try? fm.attributesOfItem(atPath: root))?[.systemNumber] as? NSNumber

    var stopped = false
    func descend(_ dir: String, depth: Int) {
      guard !stopped, depth <= maxDepth else { return }
      guard let names = try? fm.contentsOfDirectory(atPath: dir) else { return }
      for name in names.sorted() {
        guard !stopped else { return }
        if name.hasPrefix(".") { continue }
        let path = (dir as NSString).appendingPathComponent(name)
        guard let attrs = try? fm.attributesOfItem(atPath: path) else { continue }
        let type = attrs[.type] as? FileAttributeType
        let isDirectory = type == .typeDirectory
        if type == .typeSymbolicLink { continue }
        if isDirectory, let rootDevice,
          let device = attrs[.systemNumber] as? NSNumber, device != rootDevice
        {
          continue
        }
        if ignore.isIgnored(path: path, isDirectory: isDirectory) { continue }
        if !visit(path, name, isDirectory) {
          stopped = true
          return
        }
        if isDirectory {
          descend(path, depth: depth + 1)
        }
      }
    }
    descend(root, depth: 0)
  }
}

/// Wraps libgit2's ignore engine for a search root. When the root is not
/// inside a git repository every path is considered not-ignored.
final class IgnoreChecker {
  private var repo: OpaquePointer?
  private let workdir: String?

  init(root: String) {
    guard LibGit2.initialized else {
      repo = nil
      workdir = nil
      return
    }
    var repoPointer: OpaquePointer?
    if git_repository_open_ext(&repoPointer, root, 0, nil) == 0, let repoPointer {
      repo = repoPointer
      if let wd = git_repository_workdir(repoPointer) {
        var path = String(cString: wd)
        if path.hasSuffix("/") { path.removeLast() }
        workdir = path
      } else {
        workdir = nil
      }
    } else {
      repo = nil
      workdir = nil
    }
  }

  deinit {
    if let repo {
      git_repository_free(repo)
    }
  }

  /// `path` must already be canonical (derived from a realpath'd walk root).
  func isIgnored(path: String, isDirectory: Bool) -> Bool {
    guard let repo, let workdir else { return false }
    guard path.hasPrefix(workdir + "/") else { return false }
    var rel = String(path.dropFirst(workdir.count + 1))
    // libgit2 matches directory patterns (`build/`) only when the path is
    // marked as a directory.
    if isDirectory { rel += "/" }
    var ignored: Int32 = 0
    guard git_ignore_path_is_ignored(&ignored, repo, rel) == 0 else { return false }
    return ignored != 0
  }
}
