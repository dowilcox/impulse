// The registry's copy of each open document, kept in step with the editor's
// content changes (the Rust original was `apply_lsp_content_changes_to_string`
// in impulse-ffi). LSP positions count UTF-16 code units within a line.

import Foundation

/// One `TextDocumentContentChangeEvent`. Parsed strictly, mirroring serde:
/// a single malformed element fails the whole array.
public struct ContentChange: Equatable {
  public struct Position: Equatable {
    public var line: UInt32
    public var character: UInt32

    public init(line: UInt32, character: UInt32) {
      self.line = line
      self.character = character
    }
  }

  public struct Range: Equatable {
    public var start: Position
    public var end: Position

    public init(start: Position, end: Position) {
      self.start = start
      self.end = end
    }
  }

  public var range: Range?
  public var rangeLength: UInt32?
  public var text: String

  public init(range: Range?, rangeLength: UInt32?, text: String) {
    self.range = range
    self.rangeLength = rangeLength
    self.text = text
  }

  /// Parses a JSON array of LSP `TextDocumentContentChangeEvent` objects.
  /// Returns `nil` when the JSON is invalid or any element is malformed
  /// (matching `serde_json::from_str::<Vec<_>>(..).ok()` all-or-nothing
  /// semantics in the FFI).
  public static func parseArray(_ json: String) -> [ContentChange]? {
    guard let raw = JSONUtil.parse(json) as? [Any] else { return nil }
    var out: [ContentChange] = []
    out.reserveCapacity(raw.count)
    for element in raw {
      guard let obj = element as? [String: Any],
        let text = obj["text"] as? String
      else { return nil }

      var range: Range?
      if let rawRange = obj["range"], !(rawRange is NSNull) {
        guard let rangeObj = rawRange as? [String: Any],
          let start = rangeObj["start"] as? [String: Any],
          let end = rangeObj["end"] as? [String: Any],
          let startLine = JSONUtil.asUInt32(start["line"]),
          let startCharacter = JSONUtil.asUInt32(start["character"]),
          let endLine = JSONUtil.asUInt32(end["line"]),
          let endCharacter = JSONUtil.asUInt32(end["character"])
        else { return nil }
        range = Range(
          start: Position(line: startLine, character: startCharacter),
          end: Position(line: endLine, character: endCharacter))
      }

      var rangeLength: UInt32?
      if let rawLength = obj["rangeLength"], !(rawLength is NSNull) {
        guard let length = JSONUtil.asUInt32(rawLength) else { return nil }
        rangeLength = length
      }

      out.append(ContentChange(range: range, rangeLength: rangeLength, text: text))
    }
    return out
  }

  /// Re-serializes the change the way `lsp_types::TextDocumentContentChangeEvent`
  /// serializes (camelCase keys, absent optionals skipped).
  func toJSONObject() -> [String: Any] {
    var obj: [String: Any] = ["text": text]
    if let range {
      obj["range"] = [
        "start": ["line": range.start.line, "character": range.start.character],
        "end": ["line": range.end.line, "character": range.end.character],
      ]
    }
    if let rangeLength {
      obj["rangeLength"] = rangeLength
    }
    return obj
  }
}

/// A document's text as UTF-16 code units (LSP's default position encoding,
/// and Monaco's), with where each line starts indexed only as far as a
/// position has needed. A change costs its own size plus moving the text
/// after it; the index from the change on is worked out again when a later
/// position asks for it. Lines end at "\n" ("\r" is part of the line, as in
/// the Rust original).
public struct DocumentText {
  private(set) var units: [UInt16]
  /// Where each line starts, for the lines before `scanned` (every "\n"
  /// before it is counted).
  private var lineStarts: [Int] = [0]
  private var scanned = 0

  public init(_ text: String) {
    units = Array(text.utf16)
  }

  public var text: String { String(decoding: units, as: UTF16.self) }

  /// Apply changes in order, each to the text the previous one produced (the
  /// LSP's own semantics, and how Monaco orders an edit's changes: from the
  /// end of the document backwards). A change without a range replaces the
  /// whole text; a ranged one is skipped when its start is after its end.
  /// Changes that run back to front without overlapping (multi-cursor
  /// typing, Replace All) are applied in one pass over the text.
  public mutating func apply(_ changes: [ContentChange]) {
    var ranged = changes[...]
    if let last = changes.lastIndex(where: { $0.range == nil }) {
      self = DocumentText(changes[last].text)
      ranged = changes[(last + 1)...]
    }
    if ranged.count > 1, Self.runBackwards(ranged), applyInOnePass(ranged) { return }
    for change in ranged { applyOne(change) }
  }

  /// The UTF-16 offset of an LSP position. Past the end of its line clamps
  /// to the line's end (before its "\n"), past the last line to the end of
  /// the text, and inside a surrogate pair to the pair's start.
  mutating func offset(of position: ContentChange.Position) -> Int {
    place(position).offset
  }

  /// `offset(of:)`, and whether the position was a place in the text (not
  /// clamped to a line's end or the text's, or to a surrogate pair's start).
  private mutating func place(_ position: ContentChange.Position) -> (offset: Int, exact: Bool) {
    let line = Int(position.line)
    while lineStarts.count <= line, scanned < units.count {
      if units[scanned] == 0x0A { lineStarts.append(scanned + 1) }
      scanned += 1
    }
    guard line < lineStarts.count else { return (units.count, false) }
    let start = lineStarts[line]
    let wanted = start + Int(position.character)
    var offset: Int
    if line + 1 < lineStarts.count {
      offset = min(wanted, lineStarts[line + 1] - 1)
    } else {
      let limit = min(units.count, wanted)
      offset = start
      while offset < limit, units[offset] != 0x0A { offset += 1 }
    }
    var exact = offset == wanted
    if offset > start, offset < units.count, UTF16.isTrailSurrogate(units[offset]),
      UTF16.isLeadSurrogate(units[offset - 1])
    {
      offset -= 1
      exact = false
    }
    return (offset, exact)
  }

  private mutating func applyOne(_ change: ContentChange) {
    guard let range = change.range else {
      self = DocumentText(change.text)
      return
    }
    let start = offset(of: range.start)
    let end = offset(of: range.end)
    guard start <= end else { return }
    units.replaceSubrange(start..<end, with: change.text.utf16)
    forgetLines(after: start)
  }

  /// Each change ends at or before the start of the one listed before it,
  /// so the text before it is as it was: its offsets can all be worked out
  /// first, and the new text built front to back in one go. Only when every
  /// position is a place in the text, though (Monaco's always are): a
  /// change applied before could move what a clamped one lands on. False,
  /// and nothing done, otherwise.
  private mutating func applyInOnePass(_ changes: ArraySlice<ContentChange>) -> Bool {
    var spans: [(start: Int, end: Int, text: String)] = []
    spans.reserveCapacity(changes.count)
    var inserted = 0
    for change in changes {
      guard let range = change.range else { continue }
      let start = place(range.start)
      let end = place(range.end)
      guard start.exact, end.exact else { return false }
      spans.append((start.offset, end.offset, change.text))
      inserted += change.text.utf16.count
    }
    guard let first = spans.last else { return true }
    var result: [UInt16] = []
    result.reserveCapacity(units.count + inserted)
    var cursor = 0
    for span in spans.reversed() {
      result.append(contentsOf: units[cursor..<span.start])
      result.append(contentsOf: span.text.utf16)
      cursor = span.end
    }
    result.append(contentsOf: units[cursor...])
    units = result
    forgetLines(after: first.start)
    return true
  }

  /// Lines starting after `offset` may have moved: they're indexed again
  /// when a position needs them.
  private mutating func forgetLines(after offset: Int) {
    var low = 0
    var high = lineStarts.count
    while low < high {
      let middle = (low + high) / 2
      if lineStarts[middle] <= offset { low = middle + 1 } else { high = middle }
    }
    lineStarts.removeSubrange(low...)
    scanned = min(scanned, offset)
  }

  /// Every change has a range from start to end, and ends at or before the
  /// start of the change listed before it.
  private static func runBackwards(_ changes: ArraySlice<ContentChange>) -> Bool {
    func before(_ a: ContentChange.Position, _ b: ContentChange.Position) -> Bool {
      a.line < b.line || (a.line == b.line && a.character < b.character)
    }
    var previousStart: ContentChange.Position?
    for change in changes {
      guard let range = change.range, !before(range.end, range.start) else { return false }
      if let previousStart, before(previousStart, range.end) { return false }
      previousStart = range.start
    }
    return true
  }
}

public enum DocumentCache {
  /// `changes` applied to `content` (see `DocumentText.apply`). The Rust
  /// original applied them in reverse, which turned multi-cursor edits into
  /// the wrong text.
  public static func applyContentChanges(to content: inout String, changes: [ContentChange]) {
    var document = DocumentText(content)
    document.apply(changes)
    content = document.text
  }

  /// Port of `lsp_position_to_byte_offset` (the documents themselves use
  /// `DocumentText`, which counts in UTF-16). Iterates Unicode scalars (the
  /// Rust code iterates `char`s), counting UTF-16 code units within the
  /// target line, and returns a UTF-8 byte offset. Positions past the end of
  /// a line clamp to the line end; lines past the end of the document clamp
  /// to the document length.
  public static func positionToByteOffset(content: String, line targetLine: UInt32, character: UInt32) -> Int {
    let scalars = Array(content.unicodeScalars)
    var offsets = [Int]()
    offsets.reserveCapacity(scalars.count)
    var total = 0
    for scalar in scalars {
      offsets.append(total)
      total += Int(UTF8.width(scalar))
    }

    var line: UInt32 = 0
    var lineStart = 0
    var lineStartIndex = 0
    var i = 0
    while i < scalars.count {
      if line == targetLine { break }
      if scalars[i] == "\n" {
        line = line == UInt32.max ? UInt32.max : line + 1
        lineStart = offsets[i] + Int(UTF8.width(scalars[i]))
        lineStartIndex = i + 1
      }
      i += 1
    }
    if line != targetLine {
      return total
    }

    var utf16Units: UInt32 = 0
    var j = lineStartIndex
    while j < scalars.count {
      let scalar = scalars[j]
      let relative = offsets[j] - lineStart
      if scalar == "\n" || utf16Units >= character {
        return lineStart + relative
      }
      let width = UInt32(scalar.utf16.count)
      utf16Units = utf16Units > UInt32.max - width ? UInt32.max : utf16Units + width
      if utf16Units > character {
        return lineStart + relative
      }
      j += 1
    }
    return total
  }
}
