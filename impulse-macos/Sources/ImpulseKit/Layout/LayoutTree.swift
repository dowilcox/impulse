// Split-pane layout as a pure value tree: leaves are pane ids, splits lay
// their children out along one axis with ratios that sum to 1. Every
// operation returns a new tree; the app renders frames from it.

import Foundation

/// How a split arranges its children.
public enum SplitAxis: String, Codable, Sendable {
  /// Side by side, with vertical dividers between children.
  case horizontal
  /// Stacked top to bottom, with horizontal dividers between children.
  case vertical
}

/// A direction on screen, for focus movement and resizing.
public enum PaneDirection: String, Codable, Sendable, CaseIterable {
  case left, right, up, down

  /// The split axis a move in this direction crosses.
  public var axis: SplitAxis { self == .left || self == .right ? .horizontal : .vertical }
  /// Whether the direction points toward later children (right or down).
  public var isForward: Bool { self == .right || self == .down }
}

/// A rectangle in a top-left-origin coordinate space (y grows downward), so
/// frames read in the same order as the tree's children.
public struct LayoutRect: Equatable, Sendable {
  public var x: Double
  public var y: Double
  public var width: Double
  public var height: Double

  public init(x: Double, y: Double, width: Double, height: Double) {
    self.x = x
    self.y = y
    self.width = width
    self.height = height
  }

  public var maxX: Double { x + width }
  public var maxY: Double { y + height }
  public var midX: Double { x + width / 2 }
  public var midY: Double { y + height / 2 }
}

/// The boundary between two adjacent children of a split.
public struct LayoutDivider: Equatable, Sendable {
  /// Child indexes from the root to the split that owns the divider.
  public let path: [Int]
  /// The divider sits after child `index` (between `index` and `index + 1`).
  public let index: Int
  /// The owning split's axis: `.horizontal` dividers are vertical lines.
  public let axis: SplitAxis
  /// The gap the divider occupies (zero thickness when the gap is zero).
  public let rect: LayoutRect
  /// The owning split's frame, for converting a drag into a ratio.
  public let container: LayoutRect
}

public indirect enum LayoutTree<ID: Hashable & Codable & Sendable>: Equatable, Sendable {
  case leaf(ID)
  case split(SplitAxis, [LayoutTree], ratios: [Double])

  /// Smallest share a child may be resized down to.
  public static var minimumRatio: Double { 0.05 }

  // MARK: Queries

  /// Pane ids in reading order (left to right, top to bottom).
  public var leaves: [ID] {
    switch self {
    case .leaf(let id): return [id]
    case .split(_, let children, _): return children.flatMap(\.leaves)
    }
  }

  public var paneCount: Int { leaves.count }

  public func contains(_ id: ID) -> Bool {
    switch self {
    case .leaf(let leaf): return leaf == id
    case .split(_, let children, _): return children.contains { $0.contains(id) }
    }
  }

  /// The subtree at a child-index path (an empty path is the root).
  public func node(at path: [Int]) -> LayoutTree? {
    guard let first = path.first else { return self }
    guard case .split(_, let children, _) = self, children.indices.contains(first) else {
      return nil
    }
    return children[first].node(at: Array(path.dropFirst()))
  }

  /// Child-index path to a pane, or nil when it isn't in the tree.
  public func path(to id: ID) -> [Int]? {
    switch self {
    case .leaf(let leaf):
      return leaf == id ? [] : nil
    case .split(_, let children, _):
      for (index, child) in children.enumerated() {
        if let rest = child.path(to: id) { return [index] + rest }
      }
      return nil
    }
  }

  // MARK: Structure edits

  /// Split `target`, placing `newID` after it (or before it). Splitting in
  /// the same direction as the enclosing split adds a sibling there instead
  /// of nesting, halving the target's share.
  public func splitting(
    _ target: ID, with newID: ID, axis: SplitAxis, before: Bool = false
  ) -> LayoutTree {
    switch self {
    case .leaf(let id):
      guard id == target else { return self }
      let pair: [LayoutTree] = before ? [.leaf(newID), self] : [self, .leaf(newID)]
      return .split(axis, pair, ratios: [0.5, 0.5])

    case .split(let splitAxis, var children, var ratios):
      guard let index = children.firstIndex(where: { $0.contains(target) }) else { return self }
      if splitAxis == axis, case .leaf(target) = children[index] {
        let half = ratios[index] / 2
        ratios[index] = half
        let insertAt = before ? index : index + 1
        children.insert(.leaf(newID), at: insertAt)
        ratios.insert(half, at: insertAt)
        return .split(splitAxis, children, ratios: ratios)
      }
      children[index] = children[index].splitting(target, with: newID, axis: axis, before: before)
      return .split(splitAxis, children, ratios: ratios)
    }
  }

  /// Remove a pane, giving its space to the adjacent sibling. Returns nil
  /// when the pane was the whole tree.
  public func removing(_ id: ID) -> LayoutTree? {
    switch self {
    case .leaf(let leaf):
      return leaf == id ? nil : self

    case .split(let axis, var children, var ratios):
      guard let index = children.firstIndex(where: { $0.contains(id) }) else { return self }
      if let remaining = children[index].removing(id) {
        children[index] = remaining
      } else {
        let freed = ratios.remove(at: index)
        children.remove(at: index)
        let heir = index > 0 ? index - 1 : 0
        if ratios.indices.contains(heir) { ratios[heir] += freed }
      }
      if children.count == 1 { return children[0] }
      return LayoutTree.split(axis, children, ratios: ratios).flattened()
    }
  }

  /// Swap the positions of two panes.
  public func swapping(_ a: ID, _ b: ID) -> LayoutTree {
    mapLeaves { id in id == a ? b : (id == b ? a : id) }
  }

  /// Replace a pane id, keeping its position and size.
  public func replacing(_ id: ID, with newID: ID) -> LayoutTree {
    mapLeaves { $0 == id ? newID : $0 }
  }

  /// The same arrangement with every pane id transformed (e.g. renumbered
  /// for saving).
  public func mapPanes<NewID: Hashable & Codable & Sendable>(_ transform: (ID) -> NewID)
    -> LayoutTree<NewID>
  {
    switch self {
    case .leaf(let id): return .leaf(transform(id))
    case .split(let axis, let children, let ratios):
      return .split(axis, children.map { $0.mapPanes(transform) }, ratios: ratios)
    }
  }

  /// Move a pane next to another one, on the given side of it.
  public func moving(_ id: ID, nextTo target: ID, side: PaneDirection) -> LayoutTree {
    guard id != target, contains(id), contains(target), let without = removing(id) else {
      return self
    }
    return without.splitting(target, with: id, axis: side.axis, before: !side.isForward)
  }

  /// Every split's children share its space equally.
  public func equalized() -> LayoutTree {
    switch self {
    case .leaf: return self
    case .split(let axis, let children, _):
      let share = 1 / Double(children.count)
      return .split(
        axis, children.map { $0.equalized() }, ratios: Array(repeating: share, count: children.count))
    }
  }

  // MARK: Sizing

  /// Move divider `index` of the split at `path` so it sits at `position`
  /// (0...1, a fraction of that split's length). Neighbors keep at least
  /// `minimum` of the split each.
  public func movingDivider(at path: [Int], index: Int, to position: Double, minimum: Double? = nil)
    -> LayoutTree
  {
    if let first = path.first {
      guard case .split(let axis, var children, let ratios) = self, children.indices.contains(first)
      else { return self }
      children[first] = children[first].movingDivider(
        at: Array(path.dropFirst()), index: index, to: position, minimum: minimum)
      return .split(axis, children, ratios: ratios)
    }
    guard case .split(let axis, let children, var ratios) = self,
      index >= 0, index + 1 < ratios.count
    else { return self }
    let floor = minimum ?? Self.minimumRatio
    let start = ratios[..<index].reduce(0, +)
    let end = start + ratios[index] + ratios[index + 1]
    let clamped = min(max(position, start + floor), end - floor)
    guard clamped >= start, clamped <= end else { return self }
    ratios[index] = clamped - start
    ratios[index + 1] = end - clamped
    return .split(axis, children, ratios: ratios)
  }

  /// Grow (positive `delta`) or shrink a pane by moving its edge on the
  /// `direction` side. `delta` is a fraction of the enclosing split. A pane
  /// whose edge is the window's edge on that side doesn't change.
  public func resizing(_ id: ID, toward direction: PaneDirection, by delta: Double) -> LayoutTree {
    guard let path = path(to: id) else { return self }
    // Walk up from the pane to the nearest split along the right axis that
    // has a neighbor on that side.
    for depth in stride(from: path.count - 1, through: 0, by: -1) {
      let splitPath = Array(path[..<depth])
      let childIndex = path[depth]
      guard case .split(let axis, _, let ratios)? = node(at: splitPath), axis == direction.axis
      else { continue }
      let neighbor = direction.isForward ? childIndex + 1 : childIndex - 1
      guard ratios.indices.contains(neighbor) else { continue }
      let dividerIndex = direction.isForward ? childIndex : childIndex - 1
      let boundary = ratios[...dividerIndex].reduce(0, +)
      let moved = direction.isForward ? boundary + delta : boundary - delta
      return movingDivider(at: splitPath, index: dividerIndex, to: moved)
    }
    return self
  }

  // MARK: Geometry

  /// Each pane's frame inside `rect`, with `gap` points between siblings.
  /// Edges are rounded to whole points so neighbors never overlap or leave
  /// a seam.
  public func frames(in rect: LayoutRect, gap: Double = 0) -> [ID: LayoutRect] {
    var result: [ID: LayoutRect] = [:]
    walk(rect: rect, gap: gap, path: []) { node, frame, _ in
      if case .leaf(let id) = node { result[id] = frame }
    }
    return result
  }

  /// Every divider inside `rect`, for hit testing and drawing.
  public func dividers(in rect: LayoutRect, gap: Double = 0) -> [LayoutDivider] {
    var result: [LayoutDivider] = []
    walk(rect: rect, gap: gap, path: []) { node, frame, path in
      guard case .split(let axis, let children, _) = node else { return }
      let childFrames = Self.childFrames(
        of: node, in: frame, gap: gap)
      for index in 0..<(children.count - 1) {
        let a = childFrames[index]
        let b = childFrames[index + 1]
        let divider: LayoutRect =
          axis == .horizontal
          ? LayoutRect(x: a.maxX, y: frame.y, width: b.x - a.maxX, height: frame.height)
          : LayoutRect(x: frame.x, y: a.maxY, width: frame.width, height: b.y - a.maxY)
        result.append(
          LayoutDivider(path: path, index: index, axis: axis, rect: divider, container: frame))
      }
    }
    return result
  }

  /// The pane next to `id` in a direction: the closest pane on that side
  /// that overlaps it, preferring the one level with its center.
  public func neighbor(of id: ID, toward direction: PaneDirection) -> ID? {
    let all = frames(in: LayoutRect(x: 0, y: 0, width: 10_000, height: 10_000))
    guard let current = all[id] else { return nil }
    let epsilon = 0.5

    var candidates: [(id: ID, distance: Double, coversCenter: Bool, start: Double)] = []
    for (other, frame) in all where other != id {
      let distance: Double
      let overlapStart: Double
      let overlapEnd: Double
      let center: Double
      let start: Double
      switch direction {
      case .left:
        distance = current.x - frame.maxX
        (overlapStart, overlapEnd) = (max(current.y, frame.y), min(current.maxY, frame.maxY))
        center = current.midY
        start = frame.y
      case .right:
        distance = frame.x - current.maxX
        (overlapStart, overlapEnd) = (max(current.y, frame.y), min(current.maxY, frame.maxY))
        center = current.midY
        start = frame.y
      case .up:
        distance = current.y - frame.maxY
        (overlapStart, overlapEnd) = (max(current.x, frame.x), min(current.maxX, frame.maxX))
        center = current.midX
        start = frame.x
      case .down:
        distance = frame.y - current.maxY
        (overlapStart, overlapEnd) = (max(current.x, frame.x), min(current.maxX, frame.maxX))
        center = current.midX
        start = frame.x
      }
      guard distance > -epsilon, overlapEnd - overlapStart > epsilon else { continue }
      candidates.append(
        (
          id: other, distance: distance,
          coversCenter: overlapStart <= center && center <= overlapEnd, start: start
        ))
    }
    return candidates.min { a, b in
      if abs(a.distance - b.distance) > epsilon { return a.distance < b.distance }
      if a.coversCenter != b.coversCenter { return a.coversCenter }
      return a.start < b.start
    }?.id
  }

  // MARK: Internals

  private func mapLeaves(_ transform: (ID) -> ID) -> LayoutTree {
    switch self {
    case .leaf(let id): return .leaf(transform(id))
    case .split(let axis, let children, let ratios):
      return .split(axis, children.map { $0.mapLeaves(transform) }, ratios: ratios)
    }
  }

  /// Merge child splits that run along the same axis as their parent, and
  /// renormalize ratios.
  private func flattened() -> LayoutTree {
    guard case .split(let axis, let children, let ratios) = self else { return self }
    var mergedChildren: [LayoutTree] = []
    var mergedRatios: [Double] = []
    for (child, ratio) in zip(children, ratios) {
      if case .split(let childAxis, let grandchildren, let childRatios) = child, childAxis == axis {
        mergedChildren.append(contentsOf: grandchildren)
        mergedRatios.append(contentsOf: childRatios.map { $0 * ratio })
      } else {
        mergedChildren.append(child)
        mergedRatios.append(ratio)
      }
    }
    return .split(axis, mergedChildren, ratios: Self.normalized(mergedRatios))
  }

  private func walk(
    rect: LayoutRect, gap: Double, path: [Int],
    _ visit: (LayoutTree, LayoutRect, [Int]) -> Void
  ) {
    visit(self, rect, path)
    guard case .split(_, let children, _) = self else { return }
    let frames = Self.childFrames(of: self, in: rect, gap: gap)
    for (index, child) in children.enumerated() {
      child.walk(rect: frames[index], gap: gap, path: path + [index], visit)
    }
  }

  private static func childFrames(of node: LayoutTree, in rect: LayoutRect, gap: Double)
    -> [LayoutRect]
  {
    guard case .split(let axis, let children, let ratios) = node else { return [rect] }
    let length = axis == .horizontal ? rect.width : rect.height
    let origin = axis == .horizontal ? rect.x : rect.y
    let gaps = gap * Double(children.count - 1)
    let available = max(0, length - gaps)
    var frames: [LayoutRect] = []
    var cumulative = 0.0
    for index in children.indices {
      let startOffset = Double(index) * gap
      let start = (origin + cumulative * available + startOffset).rounded()
      cumulative += ratios[index]
      let end =
        index == children.count - 1
        ? origin + length
        : (origin + cumulative * available + startOffset).rounded()
      let size = max(0, end - start)
      frames.append(
        axis == .horizontal
          ? LayoutRect(x: start, y: rect.y, width: size, height: rect.height)
          : LayoutRect(x: rect.x, y: start, width: rect.width, height: size))
    }
    return frames
  }

  static func normalized(_ ratios: [Double]) -> [Double] {
    let positive = ratios.map { $0.isFinite && $0 > 0 ? $0 : 0 }
    let total = positive.reduce(0, +)
    guard total > 0 else { return ratios.map { _ in 1 / Double(max(ratios.count, 1)) } }
    return positive.map { $0 / total }
  }
}

// MARK: - Codable

extension LayoutTree: Codable {
  private enum CodingKeys: String, CodingKey {
    case pane, axis, children, ratios
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    if let id = try container.decodeIfPresent(ID.self, forKey: .pane) {
      self = .leaf(id)
      return
    }
    let axis = try container.decode(SplitAxis.self, forKey: .axis)
    let children = try container.decode([LayoutTree].self, forKey: .children)
    guard !children.isEmpty else {
      throw DecodingError.dataCorruptedError(
        forKey: .children, in: container, debugDescription: "A split needs at least one child")
    }
    if children.count == 1 {
      self = children[0]
      return
    }
    var ratios = try container.decodeIfPresent([Double].self, forKey: .ratios) ?? []
    if ratios.count != children.count {
      ratios = Array(repeating: 1, count: children.count)
    }
    self = LayoutTree.split(axis, children, ratios: Self.normalized(ratios)).flattened()
  }

  public func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    switch self {
    case .leaf(let id):
      try container.encode(id, forKey: .pane)
    case .split(let axis, let children, let ratios):
      try container.encode(axis, forKey: .axis)
      try container.encode(children, forKey: .children)
      try container.encode(ratios, forKey: .ratios)
    }
  }
}
