// Lane layout for a commit graph: given commits newest-first (topological
// order) with their parents, assign each a lane and work out the line
// segments drawn in its row. Lanes keep their column for as long as they
// live, so lines run straight down.

import Foundation

public struct GraphCommit: Equatable, Sendable {
  public let sha: String
  public let parents: [String]

  public init(sha: String, parents: [String]) {
    self.sha = sha
    self.parents = parents
  }
}

public struct GraphRow: Equatable, Sendable {
  /// The commit's lane (its dot).
  public let lane: Int
  /// Segments in the top half: from a lane at the row's top to a lane at its
  /// middle (the dot's lane when the line ends at this commit).
  public let top: [GraphSegment]
  /// Segments in the bottom half: from the middle to a lane at the bottom.
  public let bottom: [GraphSegment]
  /// Lanes the row spans (for sizing).
  public let width: Int

  public var isMerge: Bool { bottom.filter { $0.from == lane }.count > 1 }
}

public struct GraphSegment: Equatable, Hashable, Sendable {
  public let from: Int
  public let to: Int

  public init(_ from: Int, _ to: Int) {
    self.from = from
    self.to = to
  }
}

public enum CommitGraph {
  public static func layout(_ commits: [GraphCommit]) -> [GraphRow] {
    var lanes: [String?] = []
    var rows: [GraphRow] = []
    rows.reserveCapacity(commits.count)

    func freeSlot() -> Int {
      if let index = lanes.firstIndex(where: { $0 == nil }) { return index }
      lanes.append(nil)
      return lanes.count - 1
    }

    for commit in commits {
      let before = lanes
      let node = before.firstIndex(where: { $0 == commit.sha }) ?? freeSlot()

      // Lines arriving from above: lanes waiting for this commit converge
      // on its dot; everything else passes through.
      var top: [GraphSegment] = []
      for (lane, sha) in before.enumerated() {
        guard let sha else { continue }
        if sha == commit.sha {
          top.append(GraphSegment(lane, node))
          lanes[lane] = nil
        } else {
          top.append(GraphSegment(lane, lane))
        }
      }

      // Lines leaving downward toward the parents.
      var bottom: [GraphSegment] = []
      lanes[node] = nil
      for (index, parent) in commit.parents.enumerated() {
        if let existing = lanes.firstIndex(where: { $0 == parent }) {
          bottom.append(GraphSegment(node, existing))
        } else {
          let slot = index == 0 ? node : freeSlot()
          lanes[slot] = parent
          bottom.append(GraphSegment(node, slot))
        }
      }
      // Lanes that pass through this row unchanged.
      for (lane, sha) in lanes.enumerated() where lane != node && sha != nil {
        if before.indices.contains(lane), before[lane] == sha {
          bottom.append(GraphSegment(lane, lane))
        }
      }

      while let last = lanes.last, last == nil { lanes.removeLast() }
      rows.append(
        GraphRow(
          lane: node, top: top, bottom: bottom,
          width: max(before.count, lanes.count, node + 1)))
    }
    return rows
  }
}
