// Whole-file blame (git CLI, porcelain) and the "base" content an editor
// diffs its buffer against (the index version).

import Clibgit2
import Foundation

/// Who last changed a line.
public struct BlameLine: Equatable, Codable, Sendable {
  public let sha: String
  public let author: String
  public let authorTime: Date
  public let summary: String

  /// Lines not committed yet (git reports the zero id).
  public var isUncommitted: Bool { sha.allSatisfy { $0 == "0" } }
}

public enum GitBlameParser {
  /// Parse `git blame --porcelain` output into final line number → blame.
  public static func parsePorcelain(_ text: String) -> [Int: BlameLine] {
    struct CommitInfo {
      var author = ""
      var time = Date(timeIntervalSince1970: 0)
      var summary = ""
    }
    var commits: [String: CommitInfo] = [:]
    var result: [Int: BlameLine] = [:]
    var currentSha: String?
    var currentLine: Int?

    for rawLine in text.split(separator: "\n", omittingEmptySubsequences: false) {
      if rawLine.hasPrefix("\t") {
        // Content line closes the current entry.
        if let sha = currentSha, let line = currentLine {
          let info = commits[sha] ?? CommitInfo()
          result[line] = BlameLine(
            sha: sha, author: info.author, authorTime: info.time, summary: info.summary)
        }
        currentSha = nil
        currentLine = nil
        continue
      }
      let parts = rawLine.split(separator: " ", maxSplits: 3)
      if parts.count >= 3, parts[0].count >= 40, parts[0].allSatisfy(\.isHexDigit),
        let finalLine = Int(parts[2])
      {
        currentSha = String(parts[0])
        currentLine = finalLine
        if commits[currentSha!] == nil { commits[currentSha!] = CommitInfo() }
        continue
      }
      guard let sha = currentSha else { continue }
      if rawLine.hasPrefix("author ") {
        commits[sha]?.author = String(rawLine.dropFirst(7))
      } else if rawLine.hasPrefix("author-time "), let seconds = Double(rawLine.dropFirst(12)) {
        commits[sha]?.time = Date(timeIntervalSince1970: seconds)
      } else if rawLine.hasPrefix("summary ") {
        commits[sha]?.summary = String(rawLine.dropFirst(8))
      }
    }
    return result
  }
}

extension GitOperations {
  /// Blame of the file as saved on disk. Empty on failure (untracked, binary).
  public static func blame(path: String, root: String) -> [Int: BlameLine] {
    guard case .success(let result) = git(["blame", "--porcelain", "--", path], in: root, timeout: 30)
    else { return [:] }
    return GitBlameParser.parsePorcelain(result.stdout)
  }
}

extension GitClient {
  /// What the editor should diff its buffer against: the file's index
  /// version (what's staged, or HEAD's when nothing is staged). Returns ""
  /// for untracked files (every line is new) and nil when the file isn't in
  /// a repository, is ignored, or is binary.
  public static func baseContent(forFile absolutePath: String) -> String? {
    guard let repo = try? openRepo(at: absolutePath), let workdir = try? repo.workdir() else {
      return nil
    }
    let prefix = workdir.hasSuffix("/") ? workdir : workdir + "/"
    let canonical = canonicalPath(absolutePath) ?? absolutePath
    guard canonical.hasPrefix(prefix) else { return nil }
    let relative = String(canonical.dropFirst(prefix.count))

    var indexPointer: OpaquePointer?
    guard git_repository_index(&indexPointer, repo.raw) == 0, let index = indexPointer else {
      return nil
    }
    defer { git_index_free(index) }
    guard let entry = git_index_get_bypath(index, relative, 0) else {
      // Not in the index: untracked (all new) unless ignored.
      var ignored: Int32 = 0
      if git_ignore_path_is_ignored(&ignored, repo.raw, relative) == 0, ignored == 1 { return nil }
      return ""
    }
    var oid = entry.pointee.id
    var blobPointer: OpaquePointer?
    guard git_blob_lookup(&blobPointer, repo.raw, &oid) == 0, let blob = blobPointer else {
      return nil
    }
    defer { git_blob_free(blob) }
    guard git_blob_is_binary(blob) == 0, git_blob_rawsize(blob) <= maxDiffContentSize,
      let raw = git_blob_rawcontent(blob)
    else { return nil }
    let data = Data(bytes: raw, count: Int(git_blob_rawsize(blob)))
    return String(data: data, encoding: .utf8)
  }
}
