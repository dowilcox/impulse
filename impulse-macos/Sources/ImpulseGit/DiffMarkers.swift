// Port of `get_file_diff` from impulse-core/src/git.rs plus the marker JSON
// shaping done by `impulse_git_diff_markers` in impulse-ffi.

import Clibgit2
import Foundation

/// A per-line gutter marker (1-based line, status "added"/"modified"/"deleted").
/// "deleted" markers anchor at the line AFTER a pure-deletion hunk.
public struct DiffMarker: Codable, Equatable, Sendable {
  public let line: UInt32
  public let status: String

  public init(line: UInt32, status: String) {
    self.line = line
    self.status = status
  }
}

extension GitClient {
  struct FileDiffResult {
    /// 1-based line number -> "added" | "modified".
    var changedLines: [UInt32: String]
    /// 1-based anchors of pure-deletion hunks.
    var deletedLines: [UInt32]
  }

  /// Diff markers for a file (working tree vs HEAD), sorted deterministically
  /// by (line, status). Nil on error (not in a repo, unreadable file, ...);
  /// empty for clean or >1MB files — matching the Rust/FFI null-vs-empty split.
  public static func diffMarkers(filePath: String) -> [DiffMarker]? {
    guard let diff = fileDiff(filePath: filePath) else { return nil }
    var markers = diff.changedLines.map { DiffMarker(line: $0.key, status: $0.value) }
    markers.append(contentsOf: diff.deletedLines.map { DiffMarker(line: $0, status: "deleted") })
    markers.sort { $0.line != $1.line ? $0.line < $1.line : $0.status < $1.status }
    return markers
  }

  /// Port of `get_file_diff`: per-line diff status for a file vs HEAD.
  static func fileDiff(filePath: String) -> FileDiffResult? {
    // Skip diff for files larger than 1MB.
    if let size = fileSize(filePath), size > maxDiffContentSize {
      return FileDiffResult(changedLines: [:], deletedLines: [])
    }

    guard let repo = try? openRepo(at: filePath) else { return nil }
    guard let workdirRaw = repo.workdirRaw else { return nil }  // Bare repository

    // Make the file path relative to the repo root (canonicalizing both).
    let repoRoot = trimTrailingSlash(workdirRaw)
    let canonicalFile = canonicalPath(filePath) ?? filePath
    let canonicalRoot = canonicalPath(repoRoot) ?? repoRoot
    guard let relComponents = relativePathComponents(of: canonicalFile, under: canonicalRoot),
      !relComponents.isEmpty
    else { return nil }  // "File not in repo"
    let rel = relComponents.joined(separator: "/")

    // Untracked / newly-staged files: every line is an addition.
    var statusFlags: UInt32 = 0
    if git_status_file(&statusFlags, repo.raw, rel) == 0,
      statusFlags & (GIT_STATUS_WT_NEW.rawValue | GIT_STATUS_INDEX_NEW.rawValue) != 0
    {
      return allLinesAdded(filePath: filePath)
    }

    // No HEAD (empty repo) — all lines are added.
    var headRef: OpaquePointer?
    guard git_repository_head(&headRef, repo.raw) == 0, let head = headRef else {
      return allLinesAdded(filePath: filePath)
    }
    defer { git_reference_free(head) }
    var treeObject: OpaquePointer?
    guard git_reference_peel(&treeObject, head, GIT_OBJECT_TREE) == 0, let tree = treeObject
    else { return nil }  // "Failed to get HEAD tree"
    defer { git_object_free(tree) }

    var changedLines: [UInt32: String] = [:]
    var deletedLines: [UInt32] = []

    let ok = withGitStrarray([rel]) { pathspec -> Bool in
      var options = git_diff_options()
      git_diff_options_init(&options, UInt32(GIT_DIFF_OPTIONS_VERSION))
      options.pathspec = pathspec

      var diffPointer: OpaquePointer?
      guard git_diff_tree_to_workdir(&diffPointer, repo.raw, tree, &options) == 0,
        let diff = diffPointer
      else { return false }
      defer { git_diff_free(diff) }

      for deltaIndex in 0..<git_diff_num_deltas(diff) {
        var patchPointer: OpaquePointer?
        guard git_patch_from_diff(&patchPointer, diff, deltaIndex) == 0 else { return false }
        guard let patch = patchPointer else { continue }
        defer { git_patch_free(patch) }

        for hunkIndex in 0..<git_patch_num_hunks(patch) {
          var hunkPointer: UnsafePointer<git_diff_hunk>?
          var linesInHunk = 0
          guard git_patch_get_hunk(&hunkPointer, &linesInHunk, patch, hunkIndex) == 0,
            let hunk = hunkPointer
          else { return false }

          // Collect additions/deletions for this hunk, then classify: mixed
          // hunks mark the first N additions (N = deletion count) Modified and
          // the rest Added; pure-deletion hunks record an anchor line.
          var hunkAdded: [UInt32] = []
          var hunkRemovedCount: UInt32 = 0
          for lineIndex in 0..<linesInHunk {
            var linePointer: UnsafePointer<git_diff_line>?
            guard git_patch_get_line_in_hunk(&linePointer, patch, hunkIndex, lineIndex) == 0,
              let line = linePointer
            else { return false }
            switch UInt8(bitPattern: line.pointee.origin) {
            case UInt8(ascii: "+"):
              if line.pointee.new_lineno >= 0 {
                hunkAdded.append(UInt32(line.pointee.new_lineno))
              }
            case UInt8(ascii: "-"):
              hunkRemovedCount += 1
            default:
              break
            }
          }

          if !hunkAdded.isEmpty && hunkRemovedCount > 0 {
            let modifyCount = min(hunkAdded.count, Int(hunkRemovedCount))
            for (offset, lineno) in hunkAdded.enumerated() {
              changedLines[lineno] = offset < modifyCount ? "modified" : "added"
            }
          } else if !hunkAdded.isEmpty {
            for lineno in hunkAdded {
              changedLines[lineno] = "added"
            }
          } else if hunkRemovedCount > 0 {
            deletedLines.append(UInt32(max(0, hunk.pointee.new_start)))
          }
        }
      }
      return true
    }

    guard ok else { return nil }
    return FileDiffResult(changedLines: changedLines, deletedLines: deletedLines)
  }

  /// Port of `file_diff_all_lines_added`: mark every line of the on-disk file
  /// as added. Nil when the file cannot be read as UTF-8 (matching
  /// `read_to_string` failing in Rust).
  static func allLinesAdded(filePath: String) -> FileDiffResult? {
    guard let data = FileManager.default.contents(atPath: filePath),
      let content = String(data: data, encoding: .utf8)
    else { return nil }
    var changedLines: [UInt32: String] = [:]
    let lineCount = rustLineCount(content)
    if lineCount > 0 {
      for lineNumber in 1...lineCount {
        changedLines[UInt32(lineNumber)] = "added"
      }
    }
    return FileDiffResult(changedLines: changedLines, deletedLines: [])
  }
}

/// Number of lines as counted by Rust's `str::lines()` (a trailing newline
/// does not produce an extra empty line).
func rustLineCount(_ content: String) -> Int {
  if content.isEmpty { return 0 }
  var lines = content.split(separator: "\n", omittingEmptySubsequences: false)
  if content.hasSuffix("\n") {
    lines.removeLast()
  }
  return lines.count
}
