import Foundation

/// A list shown with the items that share a key gathered together at the
/// position of the first one (the sidebar groups worktrees of one
/// repository this way), and single-row moves through that shown order.
public enum GroupedOrder {
  /// Items grouped by key: groups in the order of their first item, items
  /// in their own order within a group.
  public static func groups<Item, Key: Hashable>(_ items: [Item], key: (Item) -> Key) -> [[Item]] {
    var order: [Key] = []
    var members: [Key: [Item]] = [:]
    for item in items {
      let itemKey = key(item)
      if members[itemKey] == nil { order.append(itemKey) }
      members[itemKey, default: []].append(item)
    }
    return order.map { members[$0] ?? [] }
  }

  /// `items` in shown order, with the one at `index` moved one row up
  /// (`step` -1) or down (+1): past its neighbor within its group, or, from
  /// the group's first row (up) or last row (down), the whole group past
  /// the next group. Nil when there's nowhere to go.
  public static func moving<Item, Key: Hashable>(
    _ items: [Item], at index: Int, by step: Int, key: (Item) -> Key
  ) -> [Item]? {
    guard items.indices.contains(index), step == 1 || step == -1 else { return nil }
    var shown = Self.groups(Array(items.indices)) { key(items[$0]) }
    guard let group = shown.firstIndex(where: { $0.contains(index) }),
      let position = shown[group].firstIndex(of: index)
    else { return nil }
    if shown[group].indices.contains(position + step) {
      shown[group].swapAt(position, position + step)
    } else if shown.indices.contains(group + step) {
      shown.swapAt(group, group + step)
    } else {
      return nil
    }
    return shown.flatMap { $0 }.map { items[$0] }
  }

  /// `items` with those whose id is in `order` put in that order, in the
  /// positions they hold between them; the rest stay where they are. A
  /// restored session uses it: Scratch is there before the restore (and
  /// reused), so it would otherwise keep the first row.
  public static func arranging<Item, ID: Hashable>(
    _ items: [Item], inOrder order: [ID], id: (Item) -> ID
  ) -> [Item] {
    let wanted = Set(order)
    var slots: [Int] = []
    var listed: [ID: Item] = [:]
    for (index, item) in items.enumerated() {
      let key = id(item)
      guard wanted.contains(key), listed[key] == nil else { continue }
      listed[key] = item
      slots.append(index)
    }
    var placed = Set<ID>()
    let ordered = order.compactMap { key in placed.insert(key).inserted ? listed[key] : nil }
    var result = items
    for (slot, item) in zip(slots, ordered) { result[slot] = item }
    return result
  }
}
