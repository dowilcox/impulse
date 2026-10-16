#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct BufferEditsTests {
    /// Monaco's semantics: the changes applied one by one in the order
    /// given (last to first), on UTF-16 text.
    private func oracle(_ changes: [BufferEdits.Change], _ text: String) -> String {
      let buffer = NSMutableString(string: text)
      for change in changes {
        buffer.replaceCharacters(in: NSRange(location: change.offset, length: change.length), with: change.text)
      }
      return buffer as String
    }

    private func applied(_ changes: [BufferEdits.Change], _ text: String) -> String? {
      var copy = text
      return BufferEdits.apply(changes, to: &copy) ? copy : nil
    }

    @Test func deletesACombiningAccentAlone() {
      // "é" as e + U+0301: backspace removes only the accent (one unit).
      let text = "cafe\u{301}!"
      #expect(applied([.init(offset: 4, length: 1, text: "")], text) == "cafe!")
      // Typing the accent back after the e.
      #expect(applied([.init(offset: 4, length: 0, text: "\u{301}")], "cafe!") == text)
    }

    @Test func editsInsideAnEmojiWithASkinTone() {
      // 👍🏽 is two surrogate pairs (4 units); drop just the modifier.
      let text = "a\u{1F44D}\u{1F3FD}b"
      #expect(applied([.init(offset: 3, length: 2, text: "")], text) == "a\u{1F44D}b")
      #expect(applied([.init(offset: 3, length: 0, text: "\u{1F3FB}")], "a\u{1F44D}b") == "a\u{1F44D}\u{1F3FB}b")
    }

    @Test func editsBetweenCarriageReturnAndLineFeedApply() {
      // "\r\n" is one Character in Swift but two units in Monaco.
      #expect(applied([.init(offset: 2, length: 0, text: "x")], "a\r\nb") == "a\rx\nb")
      #expect(applied([.init(offset: 1, length: 1, text: "")], "a\r\nb") == "a\nb")
    }

    @Test func manyChangesInOneEventMatchApplyingThemOneByOne() {
      // Replace All: Monaco lists the matches last to first.
      let text = String(repeating: "foo e\u{301} 👍🏽 bar\r\n", count: 200)
      let units = Array(text.utf16)
      var offsets: [Int] = []
      var i = 0
      while i + 3 <= units.count {
        if units[i] == 0x66, units[i + 1] == 0x6F, units[i + 2] == 0x6F { offsets.append(i) }
        i += 1
      }
      let changes = offsets.reversed().map { BufferEdits.Change(offset: $0, length: 3, text: "qux") }
      #expect(changes.count == 200)
      #expect(applied(changes, text) == oracle(changes, text))
      #expect(applied(changes, text) == String(repeating: "qux e\u{301} 👍🏽 bar\r\n", count: 200))
    }

    @Test func insertionsAtTheSameOffsetKeepMonacosOrder() {
      // Applied one by one, the second insert lands before the first.
      let changes: [BufferEdits.Change] = [.init(offset: 1, length: 0, text: "B"), .init(offset: 1, length: 0, text: "A")]
      #expect(applied(changes, "xy") == oracle(changes, "xy"))
      #expect(applied(changes, "xy") == "xABy")
      // An insert and a replacement starting at the same place.
      let mixed: [BufferEdits.Change] = [.init(offset: 1, length: 2, text: "R"), .init(offset: 1, length: 0, text: "I")]
      #expect(applied(mixed, "abcd") == oracle(mixed, "abcd"))
    }

    @Test func randomEditsMatchUTF16Semantics() {
      // A fixed pseudo-random walk over text mixing ASCII, accents,
      // emoji with modifiers, CJK and CRLF, cutting anywhere but inside a
      // surrogate pair.
      var seed: UInt64 = 0x9E37_79B9_7F4A_7C15
      func next(_ bound: Int) -> Int {
        seed = seed &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
        return Int((seed >> 33) % UInt64(max(bound, 1)))
      }
      let pieces = ["a", "Z", " ", "e\u{301}", "\u{1F44D}\u{1F3FD}", "漢", "\r\n", "\n", "\u{1F1FA}\u{1F1F8}", "ñ"]
      func cuttable(_ units: [UInt16], _ offset: Int) -> Bool {
        offset == 0 || offset == units.count || !UTF16.isTrailSurrogate(units[offset])
      }
      var text = "start"
      for _ in 0..<400 {
        let units = Array(text.utf16)
        var cuts = Set<Int>()
        let count = 1 + next(4)
        for _ in 0..<(count * 2) {
          let offset = next(units.count + 1)
          if cuttable(units, offset) { cuts.insert(offset) }
        }
        let sorted = cuts.sorted()
        var changes: [BufferEdits.Change] = []
        var index = 0
        while index + 1 < sorted.count {
          let insert = (0..<next(3)).map { _ in pieces[next(pieces.count)] }.joined()
          changes.append(.init(offset: sorted[index], length: sorted[index + 1] - sorted[index], text: insert))
          index += 2
        }
        if changes.isEmpty { changes = [.init(offset: 0, length: 0, text: pieces[next(pieces.count)])] }
        changes.reverse()
        let expected = oracle(changes, text)
        let result = applied(changes, text)
        #expect(result == expected)
        guard let result, result == expected else { return }
        text = result
      }
    }

    @Test func aChangeThatDoesntFitLeavesTheTextAlone() {
      var text = "abc"
      #expect(!BufferEdits.apply([.init(offset: 2, length: 5, text: "x")], to: &text))
      #expect(!BufferEdits.apply([.init(offset: 0, length: 2, text: ""), .init(offset: 1, length: 1, text: "")], to: &text))
      #expect(text == "abc")
      #expect(BufferEdits.apply([], to: &text))
      #expect(text == "abc")
    }

    @Test func splittingASurrogatePairKeepsOffsetsInStep() {
      // Half of 👍 deleted: the lone half left becomes U+FFFD, still one
      // unit, so later offsets keep lining up with Monaco's.
      let text = "x\u{1F44D}y"
      #expect(applied([.init(offset: 2, length: 1, text: "")], text) == "x\u{FFFD}y")
      #expect(applied([.init(offset: 2, length: 1, text: ""), .init(offset: 0, length: 1, text: "z")], text) == "z\u{FFFD}y")
    }
  }
#endif
