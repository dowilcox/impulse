// What the terminal's find bar asks for, turned into the regex the terminal
// core searches with (alacritty's regex syntax, smart case by default).

import Foundation

public struct TerminalFindQuery: Equatable, Sendable {
  public var text: String
  public var caseSensitive: Bool
  /// Treat `text` as a regular expression instead of literal text.
  public var regex: Bool
  public var wholeWord: Bool

  public init(text: String, caseSensitive: Bool = false, regex: Bool = false, wholeWord: Bool = false) {
    self.text = text
    self.caseSensitive = caseSensitive
    self.regex = regex
    self.wholeWord = wholeWord
  }

  /// The pattern for the terminal core, or nil when there's nothing to find.
  public var pattern: String? {
    guard !text.isEmpty else { return nil }
    var body = regex ? text : Self.escape(text)
    if wholeWord {
      // ASCII boundaries: the core's lazy DFA can't do Unicode ones.
      body = "(?-u:\\b)(?:\(body))(?-u:\\b)"
    }
    // Explicit either way, overriding the core's smart case.
    return (caseSensitive ? "(?-i)" : "(?i)") + body
  }

  /// Escape regex metacharacters (the same set as Rust's `regex::escape`).
  public static func escape(_ text: String) -> String {
    let meta: Set<Character> = ["\\", ".", "+", "*", "?", "(", ")", "|", "[", "]", "{", "}", "^", "$", "#", "&", "-", "~"]
    var out = ""
    for character in text {
      if meta.contains(character) { out.append("\\") }
      out.append(character)
    }
    return out
  }
}
