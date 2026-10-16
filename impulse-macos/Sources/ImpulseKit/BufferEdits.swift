import Foundation

/// Keeps a copy of an editor buffer in step with Monaco's content changes.
/// Offsets and lengths count UTF-16 code units (JavaScript string
/// positions), so a change can start or end inside a character: between a
/// letter and its combining accent, or an emoji and its skin tone. Those
/// must apply exactly too, or the copy drifts from what's on screen.
public enum BufferEdits {
  public struct Change: Equatable, Sendable {
    /// UTF-16 offset of the replaced range in the text before the event.
    public var offset: Int
    /// UTF-16 length of the replaced range.
    public var length: Int
    public var text: String

    public init(offset: Int, length: Int, text: String) {
      self.offset = offset
      self.length = length
      self.text = text
    }
  }

  /// Apply one content event's `changes` to `text`. Each change's range is
  /// in the text as it was before the event, and ranges don't overlap
  /// (Monaco lists them last to first, so applying them in that order one
  /// by one gives the same result). Returns false, leaving `text` as it
  /// was, when a change doesn't fit: the copy is out of step and needs the
  /// whole buffer again.
  ///
  /// The result is built in one pass, so an event with thousands of
  /// changes (Replace All) costs about one copy of the text.
  @discardableResult
  public static func apply(_ changes: [Change], to text: inout String) -> Bool {
    guard !changes.isEmpty else { return true }
    // Front to back. Changes at the same offset keep the order applying
    // Monaco's list one by one would give them: the later one first.
    let ordered = changes.enumerated().sorted { a, b in
      a.element.offset != b.element.offset ? a.element.offset < b.element.offset : a.offset > b.offset
    }.map(\.element)

    let utf16 = text.utf16
    let total = utf16.count
    var end = 0
    for change in ordered {
      guard change.offset >= end, change.length >= 0, change.offset + change.length <= total else {
        return false
      }
      end = change.offset + change.length
    }

    // Positions on Unicode scalar boundaries can be cut and joined as
    // Swift strings; one inside a surrogate pair can't.
    var bounds: [(start: String.Index, end: String.Index)] = []
    bounds.reserveCapacity(ordered.count)
    var position = utf16.startIndex
    var positionOffset = 0
    func scalarIndex(_ offset: Int) -> String.Index? {
      position = utf16.index(position, offsetBy: offset - positionOffset)
      positionOffset = offset
      return String.Index(position, within: text.unicodeScalars)
    }
    for change in ordered {
      guard let start = scalarIndex(change.offset), let stop = scalarIndex(change.offset + change.length) else {
        applySplittingSurrogates(ordered, to: &text)
        return true
      }
      bounds.append((start, stop))
    }

    var result = ""
    result.reserveCapacity(text.utf8.count + ordered.reduce(0) { $0 + $1.text.utf8.count })
    var kept = text.startIndex
    for (change, range) in zip(ordered, bounds) {
      result.append(contentsOf: Substring(text.unicodeScalars[kept..<range.start]))
      result.append(change.text)
      kept = range.end
    }
    result.append(contentsOf: Substring(text.unicodeScalars[kept...]))
    text = result
    return true
  }

  /// A change that splits a surrogate pair (half an astral character) can
  /// only be applied to UTF-16 text; the joined result is valid again once
  /// the event's other half lands, and a lone half that's left becomes
  /// U+FFFD, the same length.
  private static func applySplittingSurrogates(_ ordered: [Change], to text: inout String) {
    let buffer = NSMutableString(string: text)
    for change in ordered.reversed() {
      buffer.replaceCharacters(in: NSRange(location: change.offset, length: change.length), with: change.text)
    }
    var units = [UInt16](repeating: 0, count: buffer.length)
    buffer.getCharacters(&units, range: NSRange(location: 0, length: buffer.length))
    text = String(decoding: units, as: UTF16.self)
  }
}
