#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
  import Testing

  struct TerminalRendererGeometryTests {
    @Test func rowRangeCoversFullBounds() {
      let rect = NSRect(x: 0, y: 0, width: 800, height: 4 + 40 * 17)
      let range = TerminalRenderer.rowRange(
        intersecting: rect, lines: 40, padding: 4, cellHeight: 17)
      #expect(range == 0..<40)
    }

    @Test func rowRangeForSingleRowRect() {
      // Row 10 occupies y in [4 + 170, 4 + 187).
      let rect = NSRect(x: 0, y: 4 + 170, width: 800, height: 17)
      let range = TerminalRenderer.rowRange(
        intersecting: rect, lines: 40, padding: 4, cellHeight: 17)
      #expect(range.contains(10))
      // May include a clipped neighbour at the boundary, but never more.
      #expect(range.lowerBound >= 10 && range.upperBound <= 12)
    }

    @Test func rowRangeClampsAboveAndBelow() {
      let above = TerminalRenderer.rowRange(
        intersecting: NSRect(x: 0, y: -100, width: 800, height: 50),
        lines: 40, padding: 4, cellHeight: 17)
      #expect(above.lowerBound == 0)

      let below = TerminalRenderer.rowRange(
        intersecting: NSRect(x: 0, y: 10_000, width: 800, height: 50),
        lines: 40, padding: 4, cellHeight: 17)
      #expect(below.isEmpty)
    }

    @Test func rowRangeHandlesDegenerateInput() {
      let zeroLines = TerminalRenderer.rowRange(
        intersecting: NSRect(x: 0, y: 0, width: 10, height: 10),
        lines: 0, padding: 4, cellHeight: 17)
      #expect(zeroLines.isEmpty)

      let zeroCell = TerminalRenderer.rowRange(
        intersecting: NSRect(x: 0, y: 0, width: 10, height: 10),
        lines: 5, padding: 4, cellHeight: 0)
      #expect(zeroCell == 0..<5)
    }
  }
#endif
