import Foundation

// Ported from `TextSpan` in impulse-core/src/shell_parser.rs and the
// completion result types in impulse-core/src/completion.rs. JSON encoding
// mirrors the Rust serde serialization (snake_case keys: `is_dir`,
// `git_status`), which is the contract the terminal input bar consumes.

/// A byte range within the input text (UTF-8 byte offsets, matching Rust
/// `&str` indices). Mirrors the Rust `TextSpan` serialization (`{start, end}`).
public struct TextSpan: Codable, Equatable, Hashable, Sendable {
  public var start: Int
  public var end: Int

  public init(start: Int, end: Int) {
    self.start = start
    self.end = end
  }
}

/// A single completion candidate for the terminal completion dropdown.
public struct CompletionCandidate: Codable, Equatable, Sendable {
  /// Full replacement text for the active token. Directories get a trailing
  /// `/` so accepting one re-opens the dropdown for the next segment.
  public var value: String
  /// The label shown in the dropdown (the entry's basename).
  public var display: String
  /// The candidate category: `"path"`, or for spec completions `"command"`,
  /// `"subcommand"`, `"option"`, `"branch"`, `"script"`, `"target"`, ….
  public var kind: String
  public var isDir: Bool
  /// Git status (porcelain code) when cheaply available; `nil` during the
  /// hot typeahead path scan, where computing it per-entry would be costly.
  public var gitStatus: String?
  /// What it is ("Stage changes", a script's command), for spec candidates.
  public var detail: String?

  public init(
    value: String, display: String, kind: String, isDir: Bool, gitStatus: String?, detail: String? = nil
  ) {
    self.value = value
    self.display = display
    self.kind = kind
    self.isDir = isDir
    self.gitStatus = gitStatus
    self.detail = detail
  }

  enum CodingKeys: String, CodingKey {
    case value, display, kind, detail
    case isDir = "is_dir"
    case gitStatus = "git_status"
  }
}

/// The result of `InputCompletion.completeCandidates`: the token span to
/// replace plus the matching candidates.
public struct CompletionResult: Codable, Equatable, Sendable {
  public var span: TextSpan
  public var candidates: [CompletionCandidate]

  public init(span: TextSpan, candidates: [CompletionCandidate]) {
    self.span = span
    self.candidates = candidates
  }
}
