#if canImport(Testing)
  import Testing

  @testable import ImpulseKit

  struct CommitGraphTests {
    private func c(_ sha: String, _ parents: String...) -> GraphCommit {
      GraphCommit(sha: sha, parents: parents)
    }

    @Test func linearHistoryStaysInOneLane() {
      let rows = CommitGraph.layout([c("c", "b"), c("b", "a"), c("a")])
      #expect(rows.map(\.lane) == [0, 0, 0])
      #expect(rows[0].top.isEmpty)
      #expect(rows[0].bottom == [GraphSegment(0, 0)])
      #expect(rows[1].top == [GraphSegment(0, 0)])
      #expect(rows[2].bottom.isEmpty, "the root has nothing below")
      #expect(rows.map(\.width) == [1, 1, 1])
    }

    @Test func mergeOpensASecondLaneThatRejoins() {
      // m merges topic (t) into main (b); both come from a.
      //   m
      //   |\
      //   b t
      //   |/
      //   a
      let rows = CommitGraph.layout([c("m", "b", "t"), c("b", "a"), c("t", "a"), c("a")])
      #expect(rows.map(\.lane) == [0, 0, 1, 0])
      #expect(rows[0].isMerge)
      #expect(Set(rows[0].bottom) == [GraphSegment(0, 0), GraphSegment(0, 1)])
      // b: lane 1 (waiting for t) passes through.
      #expect(Set(rows[1].top) == [GraphSegment(0, 0), GraphSegment(1, 1)])
      #expect(Set(rows[1].bottom) == [GraphSegment(0, 0), GraphSegment(1, 1)])
      // t's parent a is already expected in lane 0: its line bends over.
      #expect(rows[2].bottom == [GraphSegment(1, 0), GraphSegment(0, 0)])
      #expect(rows[3].top == [GraphSegment(0, 0)])
      #expect(rows[3].width == 1)
    }

    @Test func branchesConvergeOnTheirCommonParent() {
      // Two tips (x, y) with the same parent p.
      let rows = CommitGraph.layout([c("x", "p"), c("y", "p"), c("p")])
      #expect(rows.map(\.lane) == [0, 1, 0])
      // p is already expected in lane 0, so y's line bends over right away.
      #expect(Set(rows[1].bottom) == [GraphSegment(1, 0), GraphSegment(0, 0)])
      #expect(rows[2].top == [GraphSegment(0, 0)])
      #expect(rows[1].width == 2)
      #expect(rows[2].width == 1)
    }

    @Test func unrelatedRootsGetTheirOwnLanes() {
      let rows = CommitGraph.layout([c("a"), c("b")])
      #expect(rows.map(\.lane) == [0, 0], "a's lane is free again for b")
      let side = CommitGraph.layout([c("tip", "base"), c("other"), c("base")])
      #expect(side.map(\.lane) == [0, 1, 0])
    }

    @Test func octopusMergesFanOut() {
      let rows = CommitGraph.layout([c("o", "a", "b", "c"), c("a"), c("b"), c("c")])
      #expect(Set(rows[0].bottom) == [GraphSegment(0, 0), GraphSegment(0, 1), GraphSegment(0, 2)])
      #expect(rows.map(\.lane) == [0, 0, 1, 2])
    }
  }
#endif
