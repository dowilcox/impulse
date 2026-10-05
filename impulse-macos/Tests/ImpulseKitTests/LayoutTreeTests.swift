#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct LayoutTreeTests {
    typealias Tree = LayoutTree<Int>

    private func ratios(_ tree: Tree) -> [Double] {
      if case .split(_, _, let ratios) = tree { return ratios }
      return []
    }

    private func approx(_ a: [Double], _ b: [Double]) -> Bool {
      a.count == b.count && zip(a, b).allSatisfy { abs($0 - $1) < 1e-9 }
    }

    /// ┌───┬───┐
    /// │ 1 │ 2 │
    /// ├───┼───┤
    /// │ 3 │ 4 │
    /// └───┴───┘
    private var grid: Tree {
      Tree.leaf(1)
        .splitting(1, with: 2, axis: .horizontal)
        .splitting(1, with: 3, axis: .vertical)
        .splitting(2, with: 4, axis: .vertical)
    }

    @Test func splittingALeafMakesAnEvenPair() {
      let tree = Tree.leaf(1).splitting(1, with: 2, axis: .horizontal)
      #expect(tree == .split(.horizontal, [.leaf(1), .leaf(2)], ratios: [0.5, 0.5]))
      let before = Tree.leaf(1).splitting(1, with: 2, axis: .vertical, before: true)
      #expect(before.leaves == [2, 1])
    }

    @Test func splittingAlongTheSameAxisAddsASibling() {
      let tree = Tree.leaf(1)
        .splitting(1, with: 2, axis: .horizontal)
        .splitting(2, with: 3, axis: .horizontal)
      guard case .split(.horizontal, let children, let r) = tree else {
        Issue.record("expected a horizontal split")
        return
      }
      #expect(children == [.leaf(1), .leaf(2), .leaf(3)])
      #expect(approx(r, [0.5, 0.25, 0.25]))
    }

    @Test func splittingAcrossTheAxisNests() {
      #expect(grid.leaves == [1, 3, 2, 4])
      #expect(grid.paneCount == 4)
      #expect(grid.path(to: 4) == [1, 1])
      #expect(grid.node(at: [0]) == .split(.vertical, [.leaf(1), .leaf(3)], ratios: [0.5, 0.5]))
      #expect(grid.contains(3))
      #expect(!grid.contains(9))
      // Unknown targets leave the tree alone.
      #expect(grid.splitting(9, with: 10, axis: .horizontal) == grid)
    }

    @Test func removingGivesSpaceToTheAdjacentSibling() {
      let tree = Tree.leaf(1)
        .splitting(1, with: 2, axis: .horizontal)
        .splitting(2, with: 3, axis: .horizontal)  // [0.5, 0.25, 0.25]
      let withoutMiddle = tree.removing(2)
      #expect(withoutMiddle?.leaves == [1, 3])
      #expect(approx(ratios(withoutMiddle!), [0.75, 0.25]))
      let withoutFirst = tree.removing(1)
      #expect(approx(ratios(withoutFirst!), [0.75, 0.25]))
    }

    @Test func removingCollapsesSingleChildSplits() {
      let pair = Tree.leaf(1).splitting(1, with: 2, axis: .horizontal)
      #expect(pair.removing(2) == .leaf(1))
      #expect(Tree.leaf(1).removing(1) == nil)
      #expect(Tree.leaf(1).removing(2) == .leaf(1))

      let afterRemove = grid.removing(3)
      #expect(afterRemove?.leaves == [1, 2, 4])
      #expect(afterRemove?.node(at: [0]) == .leaf(1))
    }

    @Test func removingFlattensSameAxisNesting() {
      // H[1, V[2, H[3, 4]]] → remove 2 → H[1, 3, 4]
      let tree = Tree.leaf(1)
        .splitting(1, with: 2, axis: .horizontal)
        .splitting(2, with: 3, axis: .vertical)
        .splitting(3, with: 4, axis: .horizontal)
      let result = tree.removing(2)
      guard case .split(.horizontal, let children, let r)? = result else {
        Issue.record("expected a flat horizontal split")
        return
      }
      #expect(children == [.leaf(1), .leaf(3), .leaf(4)])
      #expect(approx(r, [0.5, 0.25, 0.25]))
    }

    @Test func swapReplaceAndEqualize() {
      #expect(grid.swapping(1, 4).leaves == [4, 3, 2, 1])
      #expect(grid.replacing(3, with: 7).leaves == [1, 7, 2, 4])
      let lopsided = Tree.leaf(1)
        .splitting(1, with: 2, axis: .horizontal)
        .splitting(2, with: 3, axis: .horizontal)
      #expect(approx(ratios(lopsided.equalized()), [1.0 / 3, 1.0 / 3, 1.0 / 3]))
    }

    @Test func movingAPaneNextToAnother() {
      // Move 4 to the left of 1: the left column becomes H[4, 1] over 3.
      let moved = grid.moving(4, nextTo: 1, side: .left)
      #expect(moved.leaves == [4, 1, 3, 2])
      #expect(moved.neighbor(of: 1, toward: .left) == 4)
      // Moving onto itself or an unknown pane does nothing.
      #expect(grid.moving(1, nextTo: 1, side: .right) == grid)
      #expect(grid.moving(1, nextTo: 9, side: .right) == grid)
    }

    @Test func framesTileTheRectWithGaps() {
      let rect = LayoutRect(x: 0, y: 0, width: 801, height: 601)
      let frames = grid.frames(in: rect, gap: 1)
      #expect(frames[1] == LayoutRect(x: 0, y: 0, width: 400, height: 300))
      #expect(frames[3] == LayoutRect(x: 0, y: 301, width: 400, height: 300))
      #expect(frames[2] == LayoutRect(x: 401, y: 0, width: 400, height: 300))
      #expect(frames[4]?.maxX == 801)
      #expect(frames[4]?.maxY == 601)
      // Vertical splits: the gap sits between the stacked halves.
      let column = Tree.leaf(1).splitting(1, with: 2, axis: .vertical)
      let stacked = column.frames(in: LayoutRect(x: 10, y: 20, width: 100, height: 101), gap: 1)
      #expect(stacked[1] == LayoutRect(x: 10, y: 20, width: 100, height: 50))
      #expect(stacked[2] == LayoutRect(x: 10, y: 71, width: 100, height: 50))
    }

    @Test func dividersSitInTheGaps() {
      let dividers = grid.dividers(in: LayoutRect(x: 0, y: 0, width: 801, height: 601), gap: 1)
      #expect(dividers.count == 3)
      let root = dividers.first { $0.path.isEmpty }
      #expect(root?.axis == .horizontal)
      #expect(root?.rect == LayoutRect(x: 400, y: 0, width: 1, height: 601))
      let left = dividers.first { $0.path == [0] }
      #expect(left?.axis == .vertical)
      #expect(left?.rect.y == 300)
      #expect(left?.rect.height == 1)
      #expect(left?.container.width == 400)
    }

    @Test func neighborsFollowGeometry() {
      #expect(grid.neighbor(of: 1, toward: .right) == 2)
      #expect(grid.neighbor(of: 1, toward: .down) == 3)
      #expect(grid.neighbor(of: 4, toward: .up) == 2)
      #expect(grid.neighbor(of: 4, toward: .left) == 3)
      #expect(grid.neighbor(of: 1, toward: .left) == nil)
      #expect(grid.neighbor(of: 1, toward: .up) == nil)
      #expect(grid.neighbor(of: 9, toward: .up) == nil)

      // A tall pane on the left next to two stacked panes: moving right
      // picks the one level with its center (the top one wins ties).
      let tall = Tree.leaf(1)
        .splitting(1, with: 2, axis: .horizontal)
        .splitting(2, with: 3, axis: .vertical)
      #expect(tall.neighbor(of: 1, toward: .right) == 2)
      #expect(tall.neighbor(of: 3, toward: .left) == 1)
    }

    @Test func dividersMoveWithinBounds() {
      let pair = Tree.leaf(1).splitting(1, with: 2, axis: .horizontal)
      #expect(approx(ratios(pair.movingDivider(at: [], index: 0, to: 0.7)), [0.7, 0.3]))
      #expect(approx(ratios(pair.movingDivider(at: [], index: 0, to: 0.99)), [0.95, 0.05]))
      #expect(approx(ratios(pair.movingDivider(at: [], index: 0, to: 0.2, minimum: 0.3)), [0.3, 0.7]))
      // Nested divider by path.
      let moved = grid.movingDivider(at: [1], index: 0, to: 0.25)
      #expect(approx(ratios(moved.node(at: [1])!), [0.25, 0.75]))
      // Bad paths and indexes are ignored.
      #expect(grid.movingDivider(at: [5], index: 0, to: 0.3) == grid)
      #expect(pair.movingDivider(at: [], index: 3, to: 0.3) == pair)
    }

    @Test func resizingMovesTheNearestMatchingEdge() {
      // Pane 3 (bottom-left) grows rightward: the root divider moves.
      let wider = grid.resizing(3, toward: .right, by: 0.1)
      #expect(approx(ratios(wider), [0.6, 0.4]))
      // Pane 3 grows upward: its column's divider moves up.
      let taller = grid.resizing(3, toward: .up, by: 0.1)
      #expect(approx(ratios(taller.node(at: [0])!), [0.4, 0.6]))
      // Shrinking is a negative delta.
      #expect(approx(ratios(grid.resizing(1, toward: .right, by: -0.2)), [0.3, 0.7]))
      // No neighbor on that side: unchanged.
      #expect(grid.resizing(1, toward: .left, by: 0.1) == grid)
      #expect(Tree.leaf(1).resizing(1, toward: .right, by: 0.1) == .leaf(1))
    }

    @Test func codableRoundTrip() throws {
      let data = try JSONEncoder().encode(grid)
      let decoded = try JSONDecoder().decode(Tree.self, from: data)
      #expect(decoded == grid)
      let leaf = try JSONDecoder().decode(Tree.self, from: Data(#"{"pane": 5}"#.utf8))
      #expect(leaf == .leaf(5))
    }

    @Test func decodingRepairsBadInput() throws {
      // Mismatched ratios become equal shares; ratios are renormalized.
      let mismatched = #"{"axis":"horizontal","children":[{"pane":1},{"pane":2}],"ratios":[1]}"#
      let repaired = try JSONDecoder().decode(Tree.self, from: Data(mismatched.utf8))
      #expect(approx(ratios(repaired), [0.5, 0.5]))
      let unnormalized = #"{"axis":"vertical","children":[{"pane":1},{"pane":2}],"ratios":[3,1]}"#
      let scaled = try JSONDecoder().decode(Tree.self, from: Data(unnormalized.utf8))
      #expect(approx(ratios(scaled), [0.75, 0.25]))
      // A one-child split collapses to the child.
      let single = #"{"axis":"vertical","children":[{"pane":4}]}"#
      #expect(try JSONDecoder().decode(Tree.self, from: Data(single.utf8)) == .leaf(4))
      // An empty split is an error.
      let empty = #"{"axis":"vertical","children":[]}"#
      #expect(throws: DecodingError.self) {
        try JSONDecoder().decode(Tree.self, from: Data(empty.utf8))
      }
    }
  }
#endif
