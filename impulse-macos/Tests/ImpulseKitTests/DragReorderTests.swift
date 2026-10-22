#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct DragReorderTests {
    /// Rows 20 points tall with no gap: row i spans 20i...20i+20.
    private let middles: [Double] = [10, 30, 50, 70]

    /// Row `index` dragged `distance` points (negative is up).
    private func span(of index: Int, draggedBy distance: Double) -> ClosedRange<Double> {
      Double(20 * index) + distance...Double(20 * index + 20) + distance
    }

    @Test func landsPastEveryMiddleItsLeadingEdgeCrossed() {
      #expect(DragReorder.target(middles: middles, source: 0, draggedSpan: span(of: 0, draggedBy: 9)) == 0)
      #expect(DragReorder.target(middles: middles, source: 0, draggedSpan: span(of: 0, draggedBy: 11)) == 1)
      #expect(DragReorder.target(middles: middles, source: 0, draggedSpan: span(of: 0, draggedBy: 200)) == 3)
      #expect(DragReorder.target(middles: middles, source: 3, draggedSpan: span(of: 3, draggedBy: -9)) == 3)
      #expect(DragReorder.target(middles: middles, source: 3, draggedSpan: span(of: 3, draggedBy: -11)) == 2)
      #expect(DragReorder.target(middles: middles, source: 3, draggedSpan: span(of: 3, draggedBy: -200)) == 0)
      #expect(DragReorder.target(middles: middles, source: 1, draggedSpan: span(of: 1, draggedBy: 0)) == 1, "not moved")
    }

    @Test func heldAgainstTheEndsLandsFirstOrLast() {
      // The sidebar stops a dragged row at the list's ends: its edge meets
      // the first row's top or the last row's bottom, its middle the other
      // row's middle (equal heights) or short of it.
      #expect(DragReorder.target(middles: middles, source: 2, draggedSpan: span(of: 2, draggedBy: -40)) == 0)
      #expect(DragReorder.target(middles: middles, source: 1, draggedSpan: span(of: 1, draggedBy: 40)) == 3)
      // A tall row (40...100) under two short ones.
      let mixed: [Double] = [10, 30, 70]
      #expect(DragReorder.target(middles: mixed, source: 2, draggedSpan: 0...60) == 0)
      #expect(DragReorder.target(middles: mixed, source: 0, draggedSpan: 80...100) == 2)
    }

    @Test func rowsOfDifferentHeights() {
      // A tall row (an expanded workspace, 20...120) between two short ones.
      let middles: [Double] = [10, 70, 130]
      #expect(DragReorder.target(middles: middles, source: 0, draggedSpan: 49...69) == 0)
      #expect(DragReorder.target(middles: middles, source: 0, draggedSpan: 51...71) == 1)
      #expect(DragReorder.target(middles: middles, source: 2, draggedSpan: 71...91) == 2)
      #expect(DragReorder.target(middles: middles, source: 2, draggedSpan: 69...89) == 1)
      // The tall row dragged down past the short one under it.
      #expect(DragReorder.target(middles: middles, source: 1, draggedSpan: 29...129) == 1)
      #expect(DragReorder.target(middles: middles, source: 1, draggedSpan: 31...131) == 2)
    }

    @Test func unknownSourceStaysPut() {
      #expect(DragReorder.target(middles: middles, source: 7, draggedSpan: 0...20) == 7)
      #expect(DragReorder.target(middles: [], source: 0, draggedSpan: 0...20) == 0)
    }

    @Test func rowsBetweenMoveAside() {
      // Dragged down from 0 to 2: rows 1 and 2 move up.
      #expect((0..<4).map { DragReorder.shift(of: $0, source: 0, target: 2) } == [0, -1, -1, 0])
      // Dragged up from 3 to 1: rows 1 and 2 move down.
      #expect((0..<4).map { DragReorder.shift(of: $0, source: 3, target: 1) } == [0, 1, 1, 0])
      #expect((0..<4).map { DragReorder.shift(of: $0, source: 2, target: 2) } == [0, 0, 0, 0])
    }
  }
#endif
