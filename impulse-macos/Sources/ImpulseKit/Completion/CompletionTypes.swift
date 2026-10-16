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
  /// How `value` goes into the input. Impulse's own candidates are literal
  /// text (a file name, a branch), escaped when inserted, after this leading
  /// part kept as is: the `~/` or `$HOME/` a path completion read as the
  /// home folder, usually nothing. Nil for the shell's own answers, which
  /// come as the shell prints them (see `ShellQuoting.completionText`).
  public var keptPrefix: String?

  public init(
    value: String, display: String, kind: String, isDir: Bool, gitStatus: String?, detail: String? = nil,
    keptPrefix: String? = ""
  ) {
    self.value = value
    self.display = display
    self.kind = kind
    self.isDir = isDir
    self.gitStatus = gitStatus
    self.detail = detail
    self.keptPrefix = keptPrefix
  }

  /// The shell text that replaces the token `typed` (as typed, up to the
  /// cursor) when this candidate is accepted.
  public func insertion(typed: String, shell: ShellType) -> String {
    guard let keptPrefix else { return ShellQuoting.completionText(value, typed: typed, for: shell) }
    guard value.utf8.starts(with: keptPrefix.utf8) else { return ShellQuoting.escaped(value, for: shell) }
    let rest = String(decoding: Array(value.utf8.dropFirst(keptPrefix.utf8.count)), as: UTF8.self)
    return keptPrefix + ShellQuoting.escaped(rest, for: shell)
  }

  enum CodingKeys: String, CodingKey {
    case value, display, kind, detail
    case isDir = "is_dir"
    case gitStatus = "git_status"
    case keptPrefix = "kept_prefix"
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
