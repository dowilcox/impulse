// Port of the per-registry LSP document cache glue from impulse-ffi:
// `apply_lsp_content_changes_to_string` and `lsp_position_to_byte_offset`.
// LSP positions count UTF-16 code units within a line; offsets returned here
// are UTF-8 byte offsets into the document.

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

public enum DocumentCache {
  /// Apply changes in order, each to the text the previous one produced (the
  /// LSP's own semantics, and how Monaco orders an edit's changes: from the
  /// end of the document backwards). The Rust original applied them in
  /// reverse, which turned multi-cursor edits into the wrong text. A change
  /// without a range replaces the whole content; a ranged change replaces
  /// the UTF-8 byte span when `start <= end` and `end` is within bounds.
  public static func applyContentChanges(to content: inout String, changes: [ContentChange]) {
    for change in changes {
      guard let range = change.range else {
        content = change.text
        continue
      }
      let start = positionToByteOffset(
        content: content, line: range.start.line, character: range.start.character)
      let end = positionToByteOffset(
        content: content, line: range.end.line, character: range.end.character)
      let bytes = Array(content.utf8)
      if start <= end && end <= bytes.count {
        var newBytes = Array(bytes[0..<start])
        newBytes.append(contentsOf: Array(change.text.utf8))
        newBytes.append(contentsOf: bytes[end...])
        content = String(decoding: newBytes, as: UTF8.self)
      }
    }
  }

  /// Port of `lsp_position_to_byte_offset`. Iterates Unicode scalars (the
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
