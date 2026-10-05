#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct TerminalHintsTests {
    @Test func findsEachKindWithoutOverlaps() {
      let line = "see https://example.com/a(b). at src/app.ts:12:4 on localhost:5173 from 1a2b3c4d"
      let hints = TerminalHints.matches(in: line)
      #expect(hints.map(\.kind) == [.url, .path, .port, .sha])
      #expect(hints[0].text == "https://example.com/a(b)", "balanced parens stay, the period goes")
      #expect(hints[1].text == "src/app.ts:12:4")
      #expect(hints[1].path?.line == 12)
      #expect(hints[2].text == "localhost:5173")
      #expect(hints[2].url == URL(string: "http://localhost:5173"))
      #expect(hints[3].text == "1a2b3c4d")
    }

    @Test func urlsOwnTheirHostAndPort() {
      let hints = TerminalHints.matches(in: "open http://localhost:3000/docs now")
      #expect(hints.map(\.kind) == [.url])
      #expect(hints[0].text == "http://localhost:3000/docs")
    }

    @Test func shaNeedsDigitsAndLetters() {
      let kinds = TerminalHints.matches(in: "1234567 deadbeef cafe123 abc1234def").map(\.text)
      #expect(kinds == ["cafe123", "abc1234def"])
    }

    @Test func labelsArePrefixFree() {
      #expect(TerminalHints.labels(count: 3) == ["a", "s", "d"])
      let many = TerminalHints.labels(count: 12)
      #expect(many.count == 12)
      #expect(many.allSatisfy { $0.count == 2 })
      #expect(Set(many).count == 12)
      #expect(TerminalHints.labels(count: 0).isEmpty)
    }
  }
#endif
