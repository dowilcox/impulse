import AppKit
import ImpulseKit

/// A tab showing several surfaces at once. Each pane is a plain (never
/// split) `TabEntry` keyed by an id local to this tab; `layout` arranges the
/// ids and `PaneLayoutView` renders them.
final class SplitTab {
  private(set) var layout: LayoutTree<Int>
  private(set) var panes: [Int: TabEntry]
  private(set) var focusedPane: Int
  private var nextID: Int
  let view: PaneLayoutView

  /// The user clicked into another pane.
  var onFocusRequest: ((Int) -> Void)?

  init(first: TabEntry, palette: ChromePalette) {
    precondition(!first.isSplit, "panes can't nest splits")
    layout = .leaf(0)
    panes = [0: first]
    focusedPane = 0
    nextID = 1
    view = PaneLayoutView(layout: layout, focusedPane: 0, palette: palette)
    wireView()
    sync()
  }

  /// Rebuild a split from saved state. Ids missing from `panes` are dropped
  /// from the layout; returns nil when fewer than two panes survive.
  init?(layout: LayoutTree<Int>, panes: [Int: TabEntry], focusedPane: Int, palette: ChromePalette)
  {
    var tree: LayoutTree<Int>? = layout
    for id in layout.leaves where panes[id] == nil {
      tree = tree?.removing(id)
    }
    guard let tree, tree.paneCount >= 2 else { return nil }
    let kept = panes.filter { tree.contains($0.key) }
    precondition(!kept.values.contains { $0.isSplit }, "panes can't nest splits")
    self.layout = tree
    self.panes = kept
    self.focusedPane = tree.contains(focusedPane) ? focusedPane : tree.leaves[0]
    self.nextID = (tree.leaves.max() ?? 0) + 1
    view = PaneLayoutView(layout: tree, focusedPane: self.focusedPane, palette: palette)
    wireView()
    sync()
  }

  private func wireView() {
    view.onLayoutChange = { [weak self] tree in self?.layout = tree }
    view.onFocusPane = { [weak self] id in self?.onFocusRequest?(id) }
  }

  // MARK: Queries

  var count: Int { panes.count }

  /// Panes in reading order.
  var orderedPanes: [(id: Int, entry: TabEntry)] {
    layout.leaves.compactMap { id in panes[id].map { (id, $0) } }
  }

  var focused: TabEntry {
    panes[focusedPane] ?? orderedPanes[0].entry
  }

  var isZoomed: Bool { view.zoomedPane != nil }

  func paneID(where predicate: (TabEntry) -> Bool) -> Int? {
    orderedPanes.first { predicate($0.entry) }?.id
  }

  // MARK: Edits

  /// Add a pane beside `target` and return its id. Doesn't move focus.
  @discardableResult
  func insert(_ entry: TabEntry, beside target: Int, axis: SplitAxis, before: Bool = false) -> Int {
    precondition(!entry.isSplit, "panes can't nest splits")
    let id = nextID
    nextID += 1
    panes[id] = entry
    layout = layout.splitting(target, with: id, axis: axis, before: before)
    view.zoomedPane = nil
    sync()
    return id
  }

  /// Remove a pane and return it. Focus passes to a neighbor when the
  /// focused pane goes. Returns nil for unknown ids or the last pane.
  func remove(_ id: Int) -> TabEntry? {
    guard let entry = panes[id], let remaining = layout.removing(id) else { return nil }
    let heir = [PaneDirection.left, .up, .right, .down].lazy
      .compactMap { self.layout.neighbor(of: id, toward: $0) }.first
    panes[id] = nil
    layout = remaining
    if focusedPane == id { focusedPane = heir ?? remaining.leaves[0] }
    if view.zoomedPane == id { view.zoomedPane = nil }
    sync()
    return entry
  }

  func focus(_ id: Int) {
    guard panes[id] != nil else { return }
    focusedPane = id
    view.focusedPane = id
    if let zoomed = view.zoomedPane, zoomed != id { view.zoomedPane = id }
  }

  func setLayout(_ tree: LayoutTree<Int>) {
    guard Set(tree.leaves) == Set(panes.keys) else { return }
    layout = tree
    sync()
  }

  func toggleZoom() {
    view.zoomedPane = view.zoomedPane == nil ? focusedPane : nil
  }

  func applyPalette(_ palette: ChromePalette) {
    view.applyPalette(palette)
  }

  private func sync() {
    view.update(layout: layout, panes: panes.mapValues(\.view))
    view.focusedPane = focusedPane
  }
}
