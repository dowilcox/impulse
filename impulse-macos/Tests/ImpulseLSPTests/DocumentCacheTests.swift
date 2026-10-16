// Tests for the document cache port (`apply_lsp_content_changes_to_string` /
// `lsp_position_to_byte_offset` from impulse-ffi).
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseLSP

  private func change(
    _ startLine: UInt32, _ startCharacter: UInt32, _ endLine: UInt32, _ endCharacter: UInt32,
    _ text: String
  ) -> ContentChange {
    ContentChange(
      range: ContentChange.Range(
        start: ContentChange.Position(line: startLine, character: startCharacter),
        end: ContentChange.Position(line: endLine, character: endCharacter)),
      rangeLength: nil,
      text: text)
  }

  struct PositionToByteOffsetTests {
    @Test func asciiPositions() {
      let content = "hello\nworld"
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 0) == 0)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 3) == 3)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 1, character: 0) == 6)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 1, character: 5) == 11)
    }

    @Test func emptyContent() {
      #expect(DocumentCache.positionToByteOffset(content: "", line: 0, character: 0) == 0)
      #expect(DocumentCache.positionToByteOffset(content: "", line: 3, character: 1) == 0)
    }

    @Test func positionPastEndOfLineClampsToLineEnd() {
      let content = "ab\ncd"
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 99) == 2)
    }

    @Test func linePastEndOfFileClampsToContentLength() {
      let content = "ab\ncd"
      #expect(DocumentCache.positionToByteOffset(content: content, line: 5, character: 0) == 5)
    }

    @Test func lineAfterTrailingNewline() {
      // The line after the final "\n" exists and is empty.
      let content = "ab\n"
      #expect(DocumentCache.positionToByteOffset(content: content, line: 1, character: 0) == 3)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 1, character: 4) == 3)
    }

    @Test func utf16SurrogatePairs() {
      // "😀" is one Unicode scalar: 2 UTF-16 code units, 4 UTF-8 bytes.
      let content = "a😀b"
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 0) == 0)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 1) == 1)
      // Character 2 lands inside the surrogate pair — clamps to the scalar
      // start, exactly like the Rust byte-offset walk.
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 2) == 1)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 3) == 5)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 4) == 6)
    }

    @Test func multiByteBmpCharacters() {
      // "é" is 1 UTF-16 unit but 2 UTF-8 bytes.
      let content = "é\néé"
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 1) == 2)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 1, character: 1) == 5)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 1, character: 2) == 7)
    }

    @Test func crlfContent() {
      let content = "ab\r\ncd\r\n"
      // '\r' counts as one UTF-16 unit and does not end the line.
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 2) == 2)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 3) == 3)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 0, character: 9) == 3)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 1, character: 0) == 4)
      #expect(DocumentCache.positionToByteOffset(content: content, line: 1, character: 1) == 5)
    }
  }

  struct ApplyContentChangesTests {
    @Test func asciiSingleLineEdit() {
      var content = "hello world"
      DocumentCache.applyContentChanges(to: &content, changes: [change(0, 0, 0, 5, "bye")])
      #expect(content == "bye world")
    }

    @Test func multiLineRange() {
      var content = "line1\nline2\nline3"
      DocumentCache.applyContentChanges(to: &content, changes: [change(0, 2, 2, 2, "X")])
      #expect(content == "liXne3")
    }

    @Test func emojiEdit() {
      var content = "a😀b\nc"
      DocumentCache.applyContentChanges(to: &content, changes: [change(0, 1, 0, 3, "")])
      #expect(content == "ab\nc")
    }

    @Test func crlfEdit() {
      var content = "ab\r\ncd\r\n"
      DocumentCache.applyContentChanges(to: &content, changes: [change(1, 0, 1, 2, "XY")])
      #expect(content == "ab\r\nXY\r\n")
    }

    @Test func insertionAtPositionPastEndOfLine() {
      var content = "ab\ncd"
      DocumentCache.applyContentChanges(to: &content, changes: [change(0, 99, 0, 99, "!")])
      #expect(content == "ab!\ncd")
    }

    @Test func multipleChangesAppliedInOrder() {
      // Monaco lists an edit's changes from the end of the document back,
      // and each applies to the result of the one before (LSP semantics).
      var content = "abcdef"
      DocumentCache.applyContentChanges(
        to: &content,
        changes: [change(0, 2, 0, 3, "Y"), change(0, 0, 0, 1, "X")])
      #expect(content == "XbYdef")
      // Two cursors typing X in "ab".
      var typed = "ab"
      DocumentCache.applyContentChanges(
        to: &typed, changes: [change(0, 1, 0, 1, "X"), change(0, 0, 0, 0, "X")])
      #expect(typed == "XaXb")
    }

    @Test func fullDocumentReplacement() {
      var content = "old text"
      DocumentCache.applyContentChanges(
        to: &content,
        changes: [ContentChange(range: nil, rangeLength: nil, text: "brand new")])
      #expect(content == "brand new")
    }

    @Test func fullReplacementThenRangedEdit() {
      // In order: the full replace first, then the ranged edit on its text.
      var content = "irrelevant"
      DocumentCache.applyContentChanges(
        to: &content,
        changes: [
          ContentChange(range: nil, rangeLength: nil, text: "abc"),
          change(0, 0, 0, 1, "Z"),
        ])
      #expect(content == "Zbc")
    }

    @Test func invertedRangeIsIgnored() {
      var content = "abcdef"
      DocumentCache.applyContentChanges(to: &content, changes: [change(0, 4, 0, 1, "X")])
      #expect(content == "abcdef")
    }
  }

  /// The Rust original, a change at a time over UTF-8 bytes: the reference
  /// `DocumentText` must agree with.
  private func referenceApply(_ content: String, _ changes: [ContentChange]) -> String {
    var content = content
    for change in changes {
      guard let range = change.range else {
        content = change.text
        continue
      }
      let start = DocumentCache.positionToByteOffset(
        content: content, line: range.start.line, character: range.start.character)
      let end = DocumentCache.positionToByteOffset(
        content: content, line: range.end.line, character: range.end.character)
      var bytes = Array(content.utf8)
      guard start <= end, end <= bytes.count else { continue }
      bytes.replaceSubrange(start..<end, with: Array(change.text.utf8))
      content = String(decoding: bytes, as: UTF8.self)
    }
    return content
  }

  /// splitmix64: the same "random" cases every run.
  private struct SeededGenerator: RandomNumberGenerator {
    var state: UInt64
    mutating func next() -> UInt64 {
      state &+= 0x9E37_79B9_7F4A_7C15
      var z = state
      z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
      z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
      return z ^ (z >> 31)
    }
  }

  struct DocumentTextTests {
    private let pieces = ["a", "b", "é", "😀", "中", " ", "\n", "\r\n"]

    private func randomText(_ rng: inout SeededGenerator, length: Int) -> String {
      (0..<length).map { _ in pieces.randomElement(using: &rng)! }.joined()
    }

    /// A position anywhere in `text`, or a little past a line's end or the
    /// last line.
    private func randomPosition(_ rng: inout SeededGenerator, in text: String) -> ContentChange.Position {
      let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
      let line = Int.random(in: 0...(lines.count + 1), using: &rng)
      let length = line < lines.count ? lines[line].utf16.count : 0
      return ContentChange.Position(
        line: UInt32(line), character: UInt32(Int.random(in: 0...(length + 2), using: &rng)))
    }

    private func before(_ a: ContentChange.Position, _ b: ContentChange.Position) -> Bool {
      a.line < b.line || (a.line == b.line && a.character < b.character)
    }

    @Test func multibyteEdits() {
      var document = DocumentText("é😀x\n中文")
      document.apply([change(0, 1, 0, 3, "ü")])
      #expect(document.text == "éüx\n中文")
      document.apply([change(1, 1, 1, 2, "😀😀")])
      #expect(document.text == "éüx\n中😀😀")
      // Inside a surrogate pair: from the pair's start.
      document.apply([change(1, 2, 1, 3, "")])
      #expect(document.text == "éüx\n中😀")
    }

    @Test func crlfEdits() {
      var document = DocumentText("ab\r\ncd\r\nef")
      document.apply([change(1, 0, 1, 2, "XY")])
      #expect(document.text == "ab\r\nXY\r\nef")
      // Past the end of a line: before its "\n" ("\r" is part of the line).
      document.apply([change(0, 9, 0, 9, "!")])
      #expect(document.text == "ab\r!\nXY\r\nef")
      // Joining lines across a CRLF.
      document.apply([change(1, 2, 2, 0, "")])
      #expect(document.text == "ab\r!\nXYef")
    }

    @Test func editsAfterEditsUseTheMovedLines() {
      // Each change shifts the lines after it; later changes are placed on
      // the text as it is then.
      var document = DocumentText("one\ntwo\nthree\nfour")
      document.apply([change(0, 3, 0, 3, "\ninserted")])
      document.apply([change(4, 0, 4, 4, "FOUR")])
      document.apply([change(2, 0, 3, 0, "")])
      #expect(document.text == "one\ninserted\nthree\nFOUR")
    }

    @Test func replaceAllInOneEvent() {
      // Monaco lists them from the end of the document back.
      let line = "let foo = foo + 1 // é😀\r\n"
      let original = String(repeating: line, count: 3000)
      var changes: [ContentChange] = []
      for row in (0..<3000).reversed() {
        changes.append(change(UInt32(row), 10, UInt32(row), 13, "barbaz"))
        changes.append(change(UInt32(row), 4, UInt32(row), 7, "barbaz"))
      }
      var document = DocumentText(original)
      document.apply(changes)
      #expect(document.text == original.replacingOccurrences(of: "foo", with: "barbaz"))
      // And the index after it still finds lines.
      document.apply([change(2999, 0, 2999, 3, "var")])
      #expect(document.text.hasSuffix("var barbaz = barbaz + 1 // é😀\r\n"))
    }

    @Test func multiCursorTypingInOneEvent() {
      var document = DocumentText("a\nb\nc")
      document.apply([change(2, 1, 2, 1, "!"), change(1, 1, 1, 1, "!"), change(0, 1, 0, 1, "!")])
      #expect(document.text == "a!\nb!\nc!")
    }

    @Test func agreesWithApplyingOneByOne() {
      var rng = SeededGenerator(state: 42)
      for _ in 0..<300 {
        let original = randomText(&rng, length: Int.random(in: 0...60, using: &rng))
        var document = DocumentText(original)
        var reference = original
        // Several events in a row on one document: the line index carries
        // over from one to the next.
        for _ in 0..<4 {
          let changes: [ContentChange]
          if Bool.random(using: &rng) {
            // Back to front without overlapping (the one-pass path).
            let count = Int.random(in: 1...6, using: &rng)
            let positions = (0..<(count * 2)).map { _ in randomPosition(&rng, in: reference) }
              .sorted(by: before)
            changes = stride(from: 0, to: positions.count, by: 2).map { index in
              ContentChange(
                range: ContentChange.Range(start: positions[index], end: positions[index + 1]), rangeLength: nil,
                text: randomText(&rng, length: Int.random(in: 0...4, using: &rng)))
            }.reversed()
          } else {
            // Anywhere, in any order (inverted ranges included).
            changes = (0..<Int.random(in: 1...4, using: &rng)).map { _ in
              ContentChange(
                range: ContentChange.Range(
                  start: randomPosition(&rng, in: reference), end: randomPosition(&rng, in: reference)),
                rangeLength: nil, text: randomText(&rng, length: Int.random(in: 0...4, using: &rng)))
            }
          }
          let before = reference
          document.apply(changes)
          reference = referenceApply(reference, changes)
          #expect(
            document.text == reference,
            "\(before.debugDescription) + \(changes): \(document.text.debugDescription) ≠ \(reference.debugDescription)")
        }
      }
    }
  }

  struct ContentChangeParsingTests {
    @Test func parsesRangedAndFullChanges() {
      let json = """
        [
          {"range":{"start":{"line":1,"character":2},"end":{"line":1,"character":5}},"rangeLength":3,"text":"abc"},
          {"text":"full"}
        ]
        """
      let changes = ContentChange.parseArray(json)
      #expect(
        changes == [
          ContentChange(
            range: ContentChange.Range(
              start: ContentChange.Position(line: 1, character: 2),
              end: ContentChange.Position(line: 1, character: 5)),
            rangeLength: 3,
            text: "abc"),
          ContentChange(range: nil, rangeLength: nil, text: "full"),
        ])
    }

    @Test func malformedElementFailsWholeArray() {
      // Missing "text" — serde fails the whole Vec, so the FFI fell back to
      // an empty change list.
      #expect(ContentChange.parseArray("[{\"range\":null}]") == nil)
      // Fractional line numbers are not valid u32s for serde.
      #expect(
        ContentChange.parseArray(
          "[{\"range\":{\"start\":{\"line\":1.5,\"character\":0},\"end\":{\"line\":2,\"character\":0}},\"text\":\"x\"}]"
        ) == nil)
      // Negative values are rejected.
      #expect(
        ContentChange.parseArray(
          "[{\"range\":{\"start\":{\"line\":-1,\"character\":0},\"end\":{\"line\":2,\"character\":0}},\"text\":\"x\"}]"
        ) == nil)
      #expect(ContentChange.parseArray("not json") == nil)
      #expect(ContentChange.parseArray("{}") == nil)
    }
  }
#endif
