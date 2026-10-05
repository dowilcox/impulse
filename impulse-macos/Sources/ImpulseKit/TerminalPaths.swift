// Finds file references in a line of terminal output — compiler errors,
// stack traces, grep hits, plain paths — so the terminal can open them in
// the editor. Candidates only: the caller checks the file exists (relative
// to the terminal's directory) before treating one as a link.

import Foundation

public struct TerminalPathMatch: Equatable, Sendable {
  /// UTF-16 offsets of the whole reference (including `:line:col`) in the
  /// scanned text.
  public let range: Range<Int>
  public let path: String
  /// 1-based, when the reference names a position.
  public let line: Int?
  public let column: Int?

  public init(range: Range<Int>, path: String, line: Int?, column: Int?) {
    self.range = range
    self.path = path
    self.line = line
    self.column = column
  }
}

public enum TerminalPathDetector {
  /// Characters a path token is made of (no spaces, quotes or brackets).
  private static let pathChars = #"[A-Za-z0-9_.@+~/\-]"#

  /// `path:line[:col]`, `path(line[,col])` and bare paths that contain a
  /// slash or end in an extension.
  private static let referenceRegex: NSRegularExpression = {
    let path = "(?<path>\(pathChars)*(?:/\(pathChars)+|\\.[A-Za-z0-9]+)\(pathChars)*)"
    let position = #"(?::(?<line>\d+)(?::(?<col>\d+))?|\((?<pline>\d+)(?:,\s?(?<pcol>\d+))?\))?"#
    // swiftlint:disable:next force_try
    return try! NSRegularExpression(pattern: path + position)
  }()

  /// Python tracebacks: `File "path", line N`.
  private static let pythonRegex: NSRegularExpression = {
    // swiftlint:disable:next force_try
    try! NSRegularExpression(pattern: #"File "(?<path>[^"]+)", line (?<line>\d+)"#)
  }()

  /// Every candidate reference in `text`, left to right.
  public static func matches(in text: String) -> [TerminalPathMatch] {
    let ns = text as NSString
    let whole = NSRange(location: 0, length: ns.length)
    var results: [TerminalPathMatch] = []

    for match in pythonRegex.matches(in: text, range: whole) {
      let path = ns.substring(with: match.range(withName: "path"))
      results.append(
        TerminalPathMatch(
          range: match.range.location..<(match.range.location + match.range.length),
          path: path, line: int(ns, match.range(withName: "line")), column: nil))
    }

    for match in referenceRegex.matches(in: text, range: whole) {
      var pathRange = match.range(withName: "path")
      // Right after a colon is a URL's "//host/…" (the URL detector owns
      // those) or a key:value, not a path.
      if pathRange.location > 0, ns.character(at: pathRange.location - 1) == 0x3A { continue }
      // Trailing sentence punctuation isn't part of a path.
      while pathRange.length > 1 {
        let last = ns.character(at: pathRange.location + pathRange.length - 1)
        guard last == 0x2E || last == 0x2C || last == 0x3A else { break }  // . , :
        pathRange.length -= 1
      }
      let path = ns.substring(with: pathRange)
      guard isPlausible(path) else { continue }

      var line = int(ns, match.range(withName: "line")) ?? int(ns, match.range(withName: "pline"))
      var column = int(ns, match.range(withName: "col")) ?? int(ns, match.range(withName: "pcol"))
      var end = match.range.location + match.range.length
      if pathRange.length < match.range(withName: "path").length {
        // Punctuation was trimmed; the position (if any) went with it.
        line = nil
        column = nil
        end = pathRange.location + pathRange.length
      }
      let range = pathRange.location..<end
      if results.contains(where: { $0.range.overlaps(range) }) { continue }
      results.append(TerminalPathMatch(range: range, path: path, line: line, column: column))
    }
    return results.sorted { $0.range.lowerBound < $1.range.lowerBound }
  }

  /// The candidate covering UTF-16 offset `offset`.
  public static func match(in text: String, at offset: Int) -> TerminalPathMatch? {
    matches(in: text).first { $0.range.contains(offset) }
  }

  /// Resolve a candidate against a directory: absolute, `~`, or relative.
  public static func resolve(_ path: String, in directory: String) -> String {
    if path.hasPrefix("~") { return (path as NSString).expandingTildeInPath }
    if path.hasPrefix("/") { return (path as NSString).standardizingPath }
    return ((directory as NSString).appendingPathComponent(path) as NSString).standardizingPath
  }

  private static func isPlausible(_ path: String) -> Bool {
    guard path.count >= 2, path.contains(where: { $0.isLetter }) else { return false }
    // Version numbers and decimals ("1.2.3", "v2.0", "0.5s") aren't files.
    if !path.contains("/") {
      let stem = path.prefix { $0 != "." }
      let digits = stem.hasPrefix("v") ? stem.dropFirst() : stem[...]
      if path.first?.isNumber == true || (!digits.isEmpty && digits.allSatisfy(\.isNumber)) {
        return false
      }
    }
    if path.allSatisfy({ $0 == "." || $0 == "/" || $0 == "~" || $0 == "-" }) { return false }
    return true
  }

  private static func int(_ ns: NSString, _ range: NSRange) -> Int? {
    guard range.location != NSNotFound, range.length > 0 else { return nil }
    return Int(ns.substring(with: range))
  }
}
