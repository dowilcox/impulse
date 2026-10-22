import Foundation

/// Reordering a list by dragging one of its rows: where the row lands, and
/// which rows move aside to make room for it meanwhile.
public enum DragReorder {
  /// The position the row dragged from `source` lands at, from every row's
  /// middle along the drag (in list order, the dragged row's where it
  /// started) and the span the dragged row covers now: past each row whose
  /// middle its leading edge has crossed (its start dragged back, its end
  /// dragged forward). A row held against either end of the list lands
  /// first or last.
  public static func target(middles: [Double], source: Int, draggedSpan: ClosedRange<Double>) -> Int {
    guard middles.indices.contains(source) else { return source }
    let after = middles.indices.filter { $0 > source && middles[$0] < draggedSpan.upperBound }
    let before = middles.indices.filter { $0 < source && middles[$0] > draggedSpan.lowerBound }
    return after.max() ?? before.min() ?? source
  }

  /// Which way the row at `index` moves aside while the one dragged from
  /// `source` is over `target`: -1 back (up or left), 1 forward, 0 not at
  /// all.
  public static func shift(of index: Int, source: Int, target: Int) -> Int {
    if source < index && index <= target { return -1 }
    if target <= index && index < source { return 1 }
    return 0
  }
}
