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
