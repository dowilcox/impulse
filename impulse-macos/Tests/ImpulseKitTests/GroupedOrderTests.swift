#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct GroupedOrderTests {
    /// Names whose first letter is the group ("a1" and "a2" share one).
    private func key(_ name: String) -> Character { name.first! }

    @Test func groupsGatherAtTheFirstMember() {
      #expect(GroupedOrder.groups(["a1", "b", "a2", "c"], key: key) == [["a1", "a2"], ["b"], ["c"]])
      #expect(GroupedOrder.groups([String](), key: key).isEmpty)
    }

    @Test func ungroupedItemsSwapWithTheirNeighbor() {
      let items = ["a", "b", "c"]
      #expect(GroupedOrder.moving(items, at: 1, by: -1, key: key) == ["b", "a", "c"])
      #expect(GroupedOrder.moving(items, at: 1, by: 1, key: key) == ["a", "c", "b"])
      #expect(GroupedOrder.moving(items, at: 0, by: -1, key: key) == nil, "already first")
      #expect(GroupedOrder.moving(items, at: 2, by: 1, key: key) == nil, "already last")
    }

    @Test func membersMoveWithinTheirGroupFirst() {
      // Shown as a1, a2, b.
      let items = ["a1", "b", "a2"]
      #expect(GroupedOrder.moving(items, at: 2, by: -1, key: key) == ["a2", "a1", "b"])
      #expect(GroupedOrder.moving(items, at: 0, by: 1, key: key) == ["a2", "a1", "b"])
    }

    @Test func groupEdgesMoveTheWholeGroup() {
      // Shown as a1, a2, b: a2 is the group's last row, b a group of one.
      let items = ["a1", "b", "a2"]
      #expect(GroupedOrder.moving(items, at: 2, by: 1, key: key) == ["b", "a1", "a2"])
      #expect(GroupedOrder.moving(items, at: 1, by: -1, key: key) == ["b", "a1", "a2"])
      #expect(GroupedOrder.moving(items, at: 0, by: -1, key: key) == nil, "first row shown")
      #expect(GroupedOrder.moving(items, at: 1, by: 1, key: key) == nil, "last row shown")
    }

    @Test func invalidMovesAreRefused() {
      #expect(GroupedOrder.moving(["a", "b"], at: 5, by: 1, key: key) == nil)
      #expect(GroupedOrder.moving(["a", "b"], at: 0, by: 2, key: key) == nil)
    }

    @Test func droppedMembersStayInTheirGroup() {
      // Shown as a1, a2, a3, b.
      let items = ["a1", "b", "a2", "a3"]
      #expect(GroupedOrder.moving(items, at: 0, toPosition: 2, key: key) == ["a2", "a3", "a1", "b"])
      #expect(GroupedOrder.moving(items, at: 3, toPosition: 0, key: key) == ["a3", "a1", "a2", "b"])
      #expect(GroupedOrder.moving(items, at: 2, toPosition: 1, key: key) == nil, "already there")
      #expect(GroupedOrder.moving(items, at: 0, toPosition: 3, key: key) == nil, "past the group's end")
      #expect(GroupedOrder.moving(items, at: 1, toPosition: 0, key: key) == nil, "alone in its group")
    }

    @Test func droppedGroupsMoveWhole() {
      // Shown as a1, a2, b, c.
      let items = ["a1", "b", "a2", "c"]
      #expect(GroupedOrder.movingGroup(items, at: 2, toPosition: 2, key: key) == ["b", "c", "a1", "a2"])
      #expect(GroupedOrder.movingGroup(items, at: 3, toPosition: 0, key: key) == ["c", "a1", "a2", "b"])
      #expect(GroupedOrder.movingGroup(items, at: 1, toPosition: 1, key: key) == nil, "already there")
      #expect(GroupedOrder.movingGroup(items, at: 1, toPosition: 3, key: key) == nil, "only three groups")
      #expect(GroupedOrder.movingGroup(items, at: 9, toPosition: 0, key: key) == nil)
    }

    @Test func arrangingPutsListedItemsInTheirOrder() {
      // A restore: Scratch is first, the folders were appended after it.
      #expect(
        GroupedOrder.arranging(["scratch", "A", "B"], inOrder: ["A", "scratch", "B"], id: { $0 })
          == ["A", "scratch", "B"])
      #expect(
        GroupedOrder.arranging(["scratch", "A", "B"], inOrder: ["A", "B", "scratch"], id: { $0 })
          == ["A", "B", "scratch"])
      #expect(
        GroupedOrder.arranging(["scratch", "A"], inOrder: ["scratch", "A"], id: { $0 }) == ["scratch", "A"])
    }

    @Test func arrangingLeavesUnlistedItemsInPlace() {
      // x isn't listed: the others fill the slots they hold around it.
      #expect(GroupedOrder.arranging(["a", "x", "b", "c"], inOrder: ["c", "a", "b"], id: { $0 }) == ["c", "x", "a", "b"])
      // Ids that aren't there, and repeats, are skipped.
      #expect(GroupedOrder.arranging(["a", "b"], inOrder: ["z", "b", "b", "a"], id: { $0 }) == ["b", "a"])
      #expect(GroupedOrder.arranging(["a", "b"], inOrder: [String](), id: { $0 }) == ["a", "b"])
    }
  }
#endif
