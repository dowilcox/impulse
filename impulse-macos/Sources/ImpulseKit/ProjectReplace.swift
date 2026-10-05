// Project-wide replace: the same literal, per-line matching as project
// search, applied to whole files, plus the before/after preview of a line.

import Foundation

public enum ProjectReplace {
  public enum Segment: Equatable, Sendable {
    case same(String)
    case removed(String)
    case added(String)
  }

  private static func options(_ caseSensitive: Bool) -> String.CompareOptions {
    caseSensitive ? [] : [.caseInsensitive]
  }

  /// Replace every occurrence of `query` in `text`. Like search, matches
  /// never span lines, so a query containing a newline replaces nothing.
  public static func replace(
    in text: String, query: String, with replacement: String, caseSensitive: Bool
  ) -> (text: String, count: Int) {
    guard !query.isEmpty, !query.contains("\n"), !query.contains("\r") else { return (text, 0) }
    var out = ""
    var count = 0
    var cursor = text.startIndex
    while let range = text.range(of: query, options: options(caseSensitive), range: cursor..<text.endIndex) {
      out += text[cursor..<range.lowerBound]
      out += replacement
      count += 1
      cursor = range.upperBound
    }
    out += text[cursor...]
    return (out, count)
  }

  /// A line with each match shown removed and the replacement added.
  public static func preview(
    line: String, query: String, replacement: String, caseSensitive: Bool
  ) -> [Segment] {
    guard !query.isEmpty else { return [.same(line)] }
    var segments: [Segment] = []
    var cursor = line.startIndex
    while let range = line.range(of: query, options: options(caseSensitive), range: cursor..<line.endIndex) {
      if cursor < range.lowerBound { segments.append(.same(String(line[cursor..<range.lowerBound]))) }
      segments.append(.removed(String(line[range])))
      if !replacement.isEmpty { segments.append(.added(replacement)) }
      cursor = range.upperBound
    }
    if cursor < line.endIndex { segments.append(.same(String(line[cursor...]))) }
    return segments
  }
}
