// Line and range math for exposing screen text to VoiceOver as one string
// of lines joined by "\n" (NSAccessibility text-area semantics: UTF-16
// indexes, 0-based lines).

import Foundation

public struct AccessibleText: Equatable, Sendable {
  public let lines: [String]
  public let string: String
  /// UTF-16 offset where each line starts.
  private let starts: [Int]

  public init(lines: [String]) {
    // Trailing blank lines are just empty screen.
    var trimmed = lines.map { line -> String in
      var line = line
      while line.last == " " { line.removeLast() }
      return line
    }
    while trimmed.count > 1, trimmed.last?.isEmpty == true { trimmed.removeLast() }
    self.lines = trimmed
    string = trimmed.joined(separator: "\n")
    var starts: [Int] = []
    var offset = 0
    for line in trimmed {
      starts.append(offset)
      offset += line.utf16.count + 1
    }
    self.starts = starts
  }

  public var length: Int { string.utf16.count }

  /// The line containing UTF-16 index `index` (the last line past the end).
  public func line(for index: Int) -> Int {
    guard !starts.isEmpty else { return 0 }
    var low = 0
    var high = starts.count - 1
    while low < high {
      let mid = (low + high + 1) / 2
      if starts[mid] <= index { low = mid } else { high = mid - 1 }
    }
    return low
  }

  /// A line's range, without its newline.
  public func range(forLine line: Int) -> NSRange {
    guard lines.indices.contains(line) else { return NSRange(location: NSNotFound, length: 0) }
    return NSRange(location: starts[line], length: lines[line].utf16.count)
  }

  public func substring(_ range: NSRange) -> String? {
    let ns = string as NSString
    guard range.location != NSNotFound, range.location >= 0, NSMaxRange(range) <= ns.length else { return nil }
    return ns.substring(with: range)
  }
}
