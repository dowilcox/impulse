import Foundation

/// A review comment anchored to lines of one side of a file's diff.
public struct ReviewComment: Codable, Equatable, Identifiable, Sendable {
  public enum Side: String, Codable, Sendable { case old, new }

  public let id: String
  public var path: String
  public var side: Side
  /// 1-based first and last line on `side`.
  public var line: Int
  public var endLine: Int
  /// The anchored lines' text when the comment was written ("\n"-joined).
  public var snippet: String
  public var text: String
  public var createdAt: Date
  /// Set when the comment came from a pull request review thread.
  public var remote: RemoteThread?

  /// A review thread imported from the code host.
  public struct RemoteThread: Codable, Equatable, Sendable {
    /// Who started the thread.
    public var author: String
    /// The thread's first comment on the web.
    public var url: String
    /// The host says the thread's lines have since changed.
    public var isOutdated: Bool

    public init(author: String, url: String, isOutdated: Bool) {
      self.author = author
      self.url = url
      self.isOutdated = isOutdated
    }
  }

  public init(
    id: String = UUID().uuidString, path: String, side: Side, line: Int, endLine: Int,
    snippet: String, text: String, createdAt: Date = Date(), remote: RemoteThread? = nil
  ) {
    self.id = id
    self.path = path
    self.side = side
    self.line = line
    self.endLine = max(line, endLine)
    self.snippet = snippet
    self.text = text
    self.createdAt = createdAt
    self.remote = remote
  }

  /// "path:12" or "path:12-14".
  public var location: String {
    endLine > line ? "\(path):\(line)-\(endLine)" : "\(path):\(line)"
  }
}

public enum ReviewCommentAnchoring {
  /// A comment is outdated when the lines it was written on no longer read
  /// the same at the same place. `lines` maps line number → text for the
  /// comment's side in the current diff; lines not present in the diff (the
  /// hunk was staged, reverted or reshaped) also make it outdated.
  public static func isOutdated(_ comment: ReviewComment, lines: [Int: String]) -> Bool {
    // Imported threads carry no snippet: trust the host, and float ones
    // whose line isn't part of this diff.
    if let remote = comment.remote { return remote.isOutdated || lines[comment.endLine] == nil }
    guard !comment.snippet.isEmpty else { return false }
    let expected = comment.snippet.components(separatedBy: "\n")
    for (offset, text) in expected.enumerated() {
      guard let current = lines[comment.line + offset], current == text else { return true }
    }
    return false
  }

  /// Format comments as one prompt for a coding agent (or the clipboard).
  /// Comments are grouped by file in path order, then by line.
  public static func prompt(for comments: [ReviewComment]) -> String {
    guard !comments.isEmpty else { return "" }
    let sorted = comments.sorted {
      $0.path != $1.path ? $0.path < $1.path : $0.line < $1.line
    }
    var out = "Please address these review comments on the current changes:\n"
    for (index, comment) in sorted.enumerated() {
      out += "\n\(index + 1). \(comment.location)"
      if comment.side == .old { out += " (removed lines)" }
      out += "\n"
      if !comment.snippet.isEmpty {
        let fence = comment.snippet.contains("```") ? "````" : "```"
        out += "\(fence)\n\(comment.snippet)\n\(fence)\n"
      }
      out += comment.text.trimmingCharacters(in: .whitespacesAndNewlines) + "\n"
    }
    return out
  }
}
