// Hints mode: everything on screen you might want to act on — URLs, file
// references, git SHAs, local ports — each given a short label to type.

import Foundation

public struct TerminalHintMatch: Equatable, Sendable {
  public enum Kind: String, Sendable {
    case url, path, sha, port
  }

  public let kind: Kind
  /// UTF-16 offsets in the scanned line.
  public let range: Range<Int>
  /// The URL, path reference, SHA or `host:port` as it appears.
  public let text: String
  /// For paths: the parsed reference (path, line, column).
  public let path: TerminalPathMatch?

  public init(kind: Kind, range: Range<Int>, text: String, path: TerminalPathMatch? = nil) {
    self.kind = kind
    self.range = range
    self.text = text
    self.path = path
  }

  /// What opening it means: the URL (ports become http://localhost:N).
  public var url: URL? {
    switch kind {
    case .url: return URL(string: text)
    case .port:
      guard let port = text.split(separator: ":").last else { return nil }
      return URL(string: "http://localhost:\(port)")
    case .path, .sha: return nil
    }
  }
}

public enum TerminalHints {
  private static let urlRegex = try! NSRegularExpression(pattern: #"(?:https?|file)://[^\s<>"'`]+"#)
  private static let portRegex = try! NSRegularExpression(
    pattern: #"(?<![\w/.])(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\]):\d{2,5}\b"#)
  private static let shaRegex = try! NSRegularExpression(pattern: #"(?<![\w/.-])[0-9a-f]{7,40}(?![\w/-])"#)

  /// Every hint target in `line`, left to right, without overlaps (URLs win
  /// over ports, ports over paths, paths over SHAs).
  public static func matches(in line: String) -> [TerminalHintMatch] {
    let ns = line as NSString
    let whole = NSRange(location: 0, length: ns.length)
    var found: [TerminalHintMatch] = []
    func add(_ match: TerminalHintMatch) {
      guard !match.range.isEmpty, !found.contains(where: { $0.range.overlaps(match.range) }) else { return }
      found.append(match)
    }

    for result in urlRegex.matches(in: line, range: whole) {
      var length = result.range.length
      // Sentence punctuation and an unbalanced closing bracket aren't part
      // of the URL.
      while length > 0 {
        let last = ns.character(at: result.range.location + length - 1)
        let scalar = Character(UnicodeScalar(last) ?? " ")
        if ".,;:!?'\"".contains(scalar) {
          length -= 1
        } else if ")]}".contains(scalar) {
          let text = ns.substring(with: NSRange(location: result.range.location, length: length))
          let open: Character = scalar == ")" ? "(" : scalar == "]" ? "[" : "{"
          guard text.filter({ $0 == scalar }).count > text.filter({ $0 == open }).count else { break }
          length -= 1
        } else {
          break
        }
      }
      let range = result.range.location..<(result.range.location + length)
      add(TerminalHintMatch(kind: .url, range: range, text: ns.substring(with: NSRange(range))))
    }
    for result in portRegex.matches(in: line, range: whole) {
      let range = result.range.location..<(result.range.location + result.range.length)
      add(TerminalHintMatch(kind: .port, range: range, text: ns.substring(with: result.range)))
    }
    for reference in TerminalPathDetector.matches(in: line) {
      add(
        TerminalHintMatch(
          kind: .path, range: reference.range, text: ns.substring(with: NSRange(reference.range)),
          path: reference))
    }
    for result in shaRegex.matches(in: line, range: whole) {
      let text = ns.substring(with: result.range)
      // Hex that's all digits is a number; all letters is a word.
      guard text.contains(where: \.isNumber), text.contains(where: \.isLetter) else { continue }
      add(
        TerminalHintMatch(
          kind: .sha, range: result.range.location..<(result.range.location + result.range.length),
          text: text))
    }
    return found.sorted { $0.range.lowerBound < $1.range.lowerBound }
  }

  /// `count` prefix-free labels from home-row keys: single letters when
  /// they suffice, otherwise two letters each.
  public static func labels(count: Int, alphabet: [Character] = Array("asdfghjkl")) -> [String] {
    guard count > 0 else { return [] }
    if count <= alphabet.count { return alphabet.prefix(count).map { String($0) } }
    var labels: [String] = []
    outer: for first in alphabet {
      for second in alphabet {
        labels.append(String([first, second]))
        if labels.count == count { break outer }
      }
    }
    return labels
  }
}
