#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct AccessibleTextTests {
    @Test func joinsTrimmedLines() {
      let text = AccessibleText(lines: ["$ ls   ", "a.txt  b.txt", "", "  ", ""])
      #expect(text.lines == ["$ ls", "a.txt  b.txt"])
      #expect(text.string == "$ ls\na.txt  b.txt")
      #expect(text.length == 17)
    }

    @Test func linesAndRanges() {
      let text = AccessibleText(lines: ["one", "", "three"])
      #expect(text.line(for: 0) == 0)
      #expect(text.line(for: 3) == 0, "the newline belongs to its line")
      #expect(text.line(for: 4) == 1)
      #expect(text.line(for: 5) == 2)
      #expect(text.line(for: 99) == 2)
      #expect(text.range(forLine: 2) == NSRange(location: 5, length: 5))
      #expect(text.range(forLine: 7).location == NSNotFound)
      #expect(text.substring(NSRange(location: 5, length: 5)) == "three")
      #expect(text.substring(NSRange(location: 8, length: 5)) == nil)
    }

    @Test func emptyScreen() {
      let text = AccessibleText(lines: [])
      #expect(text.string.isEmpty)
      #expect(text.line(for: 0) == 0)
    }
  }
#endif
