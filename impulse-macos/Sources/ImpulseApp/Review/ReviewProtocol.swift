import CryptoKit
import Foundation
import ImpulseGit
import ImpulseKit

// Review surface protocol v2 (Swift <-> web/review.js). JSON with a "type"
// tag and camelCase keys. The web side keeps its DOM between messages: file
// lists and diffs are applied incrementally so expansion, scroll position
// and selections survive refreshes.

// MARK: - Swift -> JS

struct ReviewFileItem: Encodable, Equatable {
  let path: String
  let oldPath: String?
  /// One-letter status (A/M/D/R/T/U/C).
  let status: String
  let added: Int?
  let removed: Int?
  let binary: Bool
  let viewed: Bool
  /// Viewed earlier, but the diff changed since.
  let changedSinceViewed: Bool
  let commentCount: Int
}

struct ReviewLine: Encodable {
  /// "context" | "added" | "removed"
  let kind: String
  let old: UInt32?
  let new: UInt32?
  let text: String
  let spans: [[UInt32]]
}

struct ReviewHunk: Encodable {
  let id: String
  let header: String
  let oldStart: UInt32
  let newStart: UInt32
  let lines: [ReviewLine]
}

struct ReviewCommentItem: Encodable {
  let id: String
  /// "old" | "new"
  let side: String
  let line: Int
  let endLine: Int
  let text: String
  let outdated: Bool
}

struct ReviewFileDiff: Encodable {
  let path: String
  let diffHash: String
  let language: String
  let binary: Bool
  let tooLarge: Bool
  let truncated: Bool
  let added: Int
  let removed: Int
  let hunks: [ReviewHunk]
  let comments: [ReviewCommentItem]
}

struct ReviewCapabilities: Encodable {
  /// Hunks/lines can be staged (unstaged scope).
  let stage: Bool
  /// Hunks/lines can be unstaged (staged scope).
  let unstage: Bool
  /// Hunks/lines can be reverted in the working tree (unstaged scope).
  let revert: Bool
}

struct ReviewOptions: Codable, Equatable {
  var layout: String = "unified"  // "unified" | "split"
  var ignoreWhitespace: Bool = false
  var contextLines: Int = 3
}

enum ReviewCommand: Encodable {
  case configure(capabilities: ReviewCapabilities, options: ReviewOptions, scopeTitle: String)
  case setFiles(generation: Int, files: [ReviewFileItem], emptyMessage: String)
  case setFileDiff(ReviewFileDiff)
  case diffError(path: String, message: String)
  case setViewed(path: String, viewed: Bool)
  case focus(path: String, line: Int?)
  case setTheme(theme: MonacoThemeDefinition, chrome: [String: String])
  case setBusy(path: String, busy: Bool)

  private enum Keys: String, CodingKey {
    case type, capabilities, options, scopeTitle, generation, files, emptyMessage, diff, path,
      message, viewed, line, theme, chrome, busy
  }

  func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case let .configure(capabilities, options, scopeTitle):
      try c.encode("Configure", forKey: .type)
      try c.encode(capabilities, forKey: .capabilities)
      try c.encode(options, forKey: .options)
      try c.encode(scopeTitle, forKey: .scopeTitle)
    case let .setFiles(generation, files, emptyMessage):
      try c.encode("SetFiles", forKey: .type)
      try c.encode(generation, forKey: .generation)
      try c.encode(files, forKey: .files)
      try c.encode(emptyMessage, forKey: .emptyMessage)
    case let .setFileDiff(diff):
      try c.encode("SetFileDiff", forKey: .type)
      try c.encode(diff, forKey: .diff)
    case let .diffError(path, message):
      try c.encode("DiffError", forKey: .type)
      try c.encode(path, forKey: .path)
      try c.encode(message, forKey: .message)
    case let .setViewed(path, viewed):
      try c.encode("SetViewed", forKey: .type)
      try c.encode(path, forKey: .path)
      try c.encode(viewed, forKey: .viewed)
    case let .focus(path, line):
      try c.encode("Focus", forKey: .type)
      try c.encode(path, forKey: .path)
      try c.encodeIfPresent(line, forKey: .line)
    case let .setTheme(theme, chrome):
      try c.encode("SetTheme", forKey: .type)
      try c.encode(theme, forKey: .theme)
      try c.encode(chrome, forKey: .chrome)
    case let .setBusy(path, busy):
      try c.encode("SetBusy", forKey: .type)
      try c.encode(path, forKey: .path)
      try c.encode(busy, forKey: .busy)
    }
  }
}

// MARK: - JS -> Swift

enum ReviewAction: String, Decodable {
  case stage, unstage, revert
}

enum ReviewEvent: Decodable {
  case ready
  case requestDiff(path: String)
  /// Act on whole hunks or selected lines (lines nil = whole hunk).
  case hunkAction(
    action: ReviewAction, path: String, hunkIndex: Int, hunkId: String, lines: [Int]?)
  case fileAction(action: ReviewAction, path: String)
  case toggleViewed(path: String, viewed: Bool)
  case openFile(path: String, line: Int?)
  case addComment(path: String, side: String, line: Int, endLine: Int, text: String, snippet: String)
  case editComment(id: String, text: String)
  case deleteComment(id: String)
  case copyPath(path: String)

  private enum Keys: String, CodingKey {
    case type, path, action, hunkIndex, hunkId, lines, viewed, line, side, endLine, text, snippet, id
  }

  init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .type) {
    case "Ready":
      self = .ready
    case "RequestDiff":
      self = .requestDiff(path: try c.decode(String.self, forKey: .path))
    case "HunkAction":
      self = .hunkAction(
        action: try c.decode(ReviewAction.self, forKey: .action),
        path: try c.decode(String.self, forKey: .path),
        hunkIndex: try c.decode(Int.self, forKey: .hunkIndex),
        hunkId: try c.decode(String.self, forKey: .hunkId),
        lines: try c.decodeIfPresent([Int].self, forKey: .lines))
    case "FileAction":
      self = .fileAction(
        action: try c.decode(ReviewAction.self, forKey: .action),
        path: try c.decode(String.self, forKey: .path))
    case "ToggleViewed":
      self = .toggleViewed(
        path: try c.decode(String.self, forKey: .path),
        viewed: try c.decode(Bool.self, forKey: .viewed))
    case "OpenFile":
      self = .openFile(
        path: try c.decode(String.self, forKey: .path),
        line: try c.decodeIfPresent(Int.self, forKey: .line))
    case "AddComment":
      self = .addComment(
        path: try c.decode(String.self, forKey: .path),
        side: try c.decode(String.self, forKey: .side),
        line: try c.decode(Int.self, forKey: .line),
        endLine: try c.decode(Int.self, forKey: .endLine),
        text: try c.decode(String.self, forKey: .text),
        snippet: try c.decodeIfPresent(String.self, forKey: .snippet) ?? "")
    case "EditComment":
      self = .editComment(
        id: try c.decode(String.self, forKey: .id), text: try c.decode(String.self, forKey: .text))
    case "DeleteComment":
      self = .deleteComment(id: try c.decode(String.self, forKey: .id))
    case "CopyPath":
      self = .copyPath(path: try c.decode(String.self, forKey: .path))
    default:
      throw DecodingError.dataCorruptedError(
        forKey: .type, in: c, debugDescription: "Unknown review event")
    }
  }
}

// MARK: - Conversion

extension ReviewFileDiff {
  init(_ diff: FileDiff, diffHash: String, comments: [ReviewCommentItem]) {
    self.init(
      path: diff.path, diffHash: diffHash, language: diff.language, binary: diff.isBinary,
      tooLarge: diff.tooLarge, truncated: diff.truncated, added: diff.added,
      removed: diff.removed,
      hunks: zip(diff.hunks, diff.hunkIds).map { hunk, id in
        ReviewHunk(
          id: id, header: hunk.header, oldStart: hunk.oldStart, newStart: hunk.newStart,
          lines: hunk.lines.map { line in
            ReviewLine(
              kind: line.kind.rawValue, old: line.oldLineno, new: line.newLineno,
              text: line.content, spans: line.spans.map { [$0.start, $0.end] })
          })
      },
      comments: comments)
  }
}

extension FileDiff {
  /// Identity of the diff's content (changes, not positions). Stable across
  /// launches, so "viewed" marks can be persisted against it.
  var contentHash: String {
    let text = "\(path)|\(isBinary)|\(tooLarge)|" + hunkIds.joined(separator: ",")
    let digest = SHA256.hash(data: Data(text.utf8))
    return digest.prefix(8).map { String(format: "%02x", $0) }.joined()
  }
}
