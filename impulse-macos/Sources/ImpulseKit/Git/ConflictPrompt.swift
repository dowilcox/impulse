// Turn conflicted files into a prompt asking a coding agent to resolve
// them: each conflict block with its line numbers, size-capped.

import Foundation

public enum ConflictPrompt {
  public struct Block: Equatable, Sendable {
    /// 1-based lines of the `<<<<<<<` and `>>>>>>>` markers.
    public let startLine: Int
    public let endLine: Int
    public let text: String
  }

  /// Conflict blocks in a file's text (`<<<<<<<` through `>>>>>>>`,
  /// including a diff3 `|||||||` base section).
  public static func blocks(in text: String) -> [Block] {
    let lines = text.components(separatedBy: "\n")
    var blocks: [Block] = []
    var start: Int?
    for (index, line) in lines.enumerated() {
      if line.hasPrefix("<<<<<<<") {
        start = index
      } else if line.hasPrefix(">>>>>>>"), let first = start {
        blocks.append(
          Block(
            startLine: first + 1, endLine: index + 1,
            text: lines[first...index].joined(separator: "\n")))
        start = nil
      }
    }
    return blocks
  }

  /// The prompt for `files` (path relative to the repository, contents).
  /// `operation` names what's in progress ("merge", "rebase", …). Blocks
  /// past `limit` characters are listed by line only.
  public static func make(
    files: [(path: String, text: String)], operation: String? = nil, limit: Int = 24_000
  ) -> String {
    var out = "Please resolve the merge conflicts"
    if let operation, !operation.isEmpty { out += " from this \(operation)" }
    out += " in the files below. Keep the intent of both sides where it makes sense, remove the conflict markers, and make sure the result builds. Don't stage or commit anything.\n"
    var used = out.count
    var omitted: [String] = []
    for (path, text) in files {
      let fileBlocks = blocks(in: text)
      guard !fileBlocks.isEmpty else { continue }
      out += "\n## \(path) (\(fileBlocks.count) conflict\(fileBlocks.count == 1 ? "" : "s"))\n"
      for block in fileBlocks {
        let fence = block.text.contains("```") ? "````" : "```"
        let section = "\nLines \(block.startLine)–\(block.endLine):\n\(fence)\n\(block.text)\n\(fence)\n"
        if used + section.count > limit {
          omitted.append("\(path):\(block.startLine)-\(block.endLine)")
          continue
        }
        out += section
        used += section.count
      }
    }
    if !omitted.isEmpty {
      out += "\nAlso resolve these (not shown, open the files): \(omitted.joined(separator: ", "))\n"
    }
    return out
  }
}
