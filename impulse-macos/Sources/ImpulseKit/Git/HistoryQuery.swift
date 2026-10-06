// The History surface's filter field: free text plus `key:value` tokens.
//
//   author:jane  path:src/  since:2w  until:2026-01-01  fix crash
//
// Tokens go to `git log` (so they reach commits that aren't loaded yet); the
// free text filters the loaded rows by subject, author, SHA prefix and refs.

import Foundation

public struct HistoryQuery: Equatable, Sendable {
  /// Free text, matched against loaded commits.
  public var text: String = ""
  public var author: String?
  public var path: String?
  /// A git date ("2.weeks.ago", "2026-01-01", "yesterday").
  public var since: String?
  public var until: String?

  public init(
    text: String = "", author: String? = nil, path: String? = nil, since: String? = nil, until: String? = nil
  ) {
    self.text = text
    self.author = author
    self.path = path
    self.since = since
    self.until = until
  }

  /// Keys the field understands, with their aliases.
  static let keys: [String: String] = [
    "author": "author", "by": "author",
    "path": "path", "file": "path", "in": "path",
    "since": "since", "after": "since",
    "until": "until", "before": "until",
  ]

  public static func parse(_ input: String) -> HistoryQuery {
    var query = HistoryQuery()
    var words: [String] = []
    for token in tokenize(input) {
      if let colon = token.firstIndex(of: ":"), colon != token.startIndex,
        let key = keys[token[..<colon].lowercased()]
      {
        let value = unquote(String(token[token.index(after: colon)...]))
        guard !value.isEmpty else { continue }
        switch key {
        case "author": query.author = value
        case "path": query.path = value
        case "since": query.since = gitDate(value)
        default: query.until = gitDate(value, endOfDay: true)
        }
      } else {
        words.append(unquote(token))
      }
    }
    query.text = words.joined(separator: " ")
    return query
  }

  /// The parts `git log` applies (everything but the free text).
  public var server: HistoryQuery {
    HistoryQuery(author: author, path: path, since: since, until: until)
  }

  public var hasServerFilters: Bool { server != HistoryQuery() }

  /// `git log` options for the tokens (the path goes after `--` separately).
  public var gitArguments: [String] {
    var args: [String] = []
    if let author {
      args += ["--author=\(author)", "--regexp-ignore-case"]
    }
    if let since { args.append("--since=\(since)") }
    if let until { args.append("--until=\(until)") }
    return args
  }

  /// `2w` → `2.weeks.ago` (also h, d, m for months, y); a bare
  /// `2026-01-31` covers the whole day (git would otherwise read it as that
  /// date at the current time); anything else is passed to git as written.
  static func gitDate(_ value: String, endOfDay: Bool = false) -> String {
    let units: [Character: String] = ["h": "hours", "d": "days", "w": "weeks", "m": "months", "y": "years"]
    if let unit = value.last.flatMap({ units[Character($0.lowercased())] }),
      let count = Int(value.dropLast()), count >= 0
    {
      return "\(count).\(unit).ago"
    }
    if value.range(of: #"^\d{4}-\d{2}-\d{2}$"#, options: .regularExpression) != nil {
      return value + (endOfDay ? " 23:59:59" : " 00:00:00")
    }
    return value
  }

  /// `input` with `key:` set to `value` (replacing an existing token for the
  /// same key or alias), or removed when `value` is nil.
  public static func setting(_ key: String, to value: String?, in input: String) -> String {
    let canonical = keys[key.lowercased()] ?? key.lowercased()
    var tokens = tokenize(input).filter { token in
      guard let colon = token.firstIndex(of: ":"), colon != token.startIndex else { return true }
      return keys[token[..<colon].lowercased()] != canonical
    }
    if let value, !value.isEmpty {
      let quoted = value.contains(" ") ? "\"\(value)\"" : value
      tokens.insert("\(canonical):\(quoted)", at: 0)
    }
    return tokens.joined(separator: " ")
  }

  /// Whitespace-separated words; double quotes keep spaces (`author:"Jane Doe"`).
  static func tokenize(_ input: String) -> [String] {
    var tokens: [String] = []
    var current = ""
    var quoted = false
    for character in input {
      if character == "\"" {
        quoted.toggle()
        current.append(character)
      } else if character.isWhitespace, !quoted {
        if !current.isEmpty { tokens.append(current) }
        current = ""
      } else {
        current.append(character)
      }
    }
    if !current.isEmpty { tokens.append(current) }
    return tokens
  }

  static func unquote(_ value: String) -> String {
    value.replacingOccurrences(of: "\"", with: "")
  }
}
