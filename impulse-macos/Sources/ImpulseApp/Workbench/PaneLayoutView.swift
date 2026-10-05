import AppKit
import ImpulseKit

/// Renders a split tab: positions pane views from a `LayoutTree`, draws the
/// 1pt gaps between them as dividers, lets dividers be dragged (double-click
/// evens out a split), dims inactive panes, marks the focused one, and shows
/// a single pane full-size while zoomed. Clicking anywhere in a pane focuses
/// it.
final class PaneLayoutView: NSView {
  static let gap: CGFloat = 1
  /// Panes can't be dragged narrower or shorter than this.
  static let minimumPaneSize: CGFloat = 80

  private(set) var layoutTree: LayoutTree<Int>
  private var paneViews: [Int: NSView] = [:]
  private var dimViews: [Int: DimView] = [:]
  private var dividerHandles: [DividerHandle] = []
  private let focusMarker = NSView()
  private var mouseMonitor: Any?

  var focusedPane: Int {
    didSet { if focusedPane != oldValue { updateDecorations() } }
  }
  var zoomedPane: Int? {
    didSet { if zoomedPane != oldValue { needsLayout = true } }
  }
  var dimsInactivePanes = true {
    didSet { updateDecorations() }
  }

  /// A click landed in a pane other than the focused one.
  var onFocusPane: ((Int) -> Void)?
  /// The user dragged a divider (or reset one).
  var onLayoutChange: ((LayoutTree<Int>) -> Void)?

  private var palette: ChromePalette
  private var dimColor: CGColor = NSColor.black.withAlphaComponent(0.3).cgColor

  init(layout: LayoutTree<Int>, focusedPane: Int, palette: ChromePalette) {
    self.layoutTree = layout
    self.focusedPane = focusedPane
    self.palette = palette
    super.init(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
    wantsLayer = true
    focusMarker.wantsLayer = true
    applyPalette(palette)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  override var isFlipped: Bool { true }

  // MARK: Content

  /// Replace the tree and the pane views it refers to. Views no longer in
  /// the tree are removed from the hierarchy (not torn down — the owner
  /// decides that).
  func update(layout: LayoutTree<Int>, panes: [Int: NSView]) {
    layoutTree = layout
    for (id, view) in paneViews where panes[id] !== view {
      view.removeFromSuperview()
      dimViews.removeValue(forKey: id)?.removeFromSuperview()
    }
    for (id, view) in panes where view.superview !== self {
      view.removeFromSuperview()
      view.translatesAutoresizingMaskIntoConstraints = true
      view.autoresizingMask = []
      addSubview(view, positioned: .below, relativeTo: nil)
      let dim = DimView()
      dim.layer?.backgroundColor = dimColor
      dimViews[id]?.removeFromSuperview()
      dimViews[id] = dim
      addSubview(dim, positioned: .above, relativeTo: view)
    }
    paneViews = panes
    if let zoomed = zoomedPane, panes[zoomed] == nil { zoomedPane = nil }
    rebuildDividers()
    needsLayout = true
    updateDecorations()
  }

  func applyPalette(_ palette: ChromePalette) {
    self.palette = palette
    layer?.backgroundColor = palette.nsHairline.cgColor
    focusMarker.layer?.backgroundColor = palette.nsAccent.cgColor
    dimColor = palette.nsChrome.withAlphaComponent(palette.isLight ? 0.28 : 0.34).cgColor
    for dim in dimViews.values { dim.layer?.backgroundColor = dimColor }
  }

  /// The pane under a point in this view's coordinates.
  func pane(at point: NSPoint) -> Int? {
    if let zoomed = zoomedPane { return paneViews[zoomed]?.frame.contains(point) == true ? zoomed : nil }
    return paneViews.first { $0.value.frame.contains(point) }?.key
  }

  // MARK: Layout

  override func layout() {
    super.layout()
    let rect = LayoutRect(
      x: 0, y: 0, width: Double(bounds.width), height: Double(bounds.height))
    if let zoomed = zoomedPane, let view = paneViews[zoomed] {
      for (id, other) in paneViews {
        other.isHidden = id != zoomed
        dimViews[id]?.isHidden = true
      }
      view.frame = bounds
      for handle in dividerHandles { handle.isHidden = true }
    } else {
      let frames = layoutTree.frames(in: rect, gap: Double(Self.gap))
      for (id, view) in paneViews {
        view.isHidden = false
        guard let frame = frames[id] else { continue }
        let nsFrame = NSRect(x: frame.x, y: frame.y, width: frame.width, height: frame.height)
        if view.frame != nsFrame { view.frame = nsFrame }
        dimViews[id]?.frame = nsFrame
      }
      let dividers = layoutTree.dividers(in: rect, gap: Double(Self.gap))
      for (handle, divider) in zip(dividerHandles, dividers) {
        handle.divider = divider
        handle.isHidden = false
        let r = divider.rect
        let slop: CGFloat = 3
        handle.frame =
          divider.axis == .horizontal
          ? NSRect(x: r.x - slop, y: r.y, width: r.width + slop * 2, height: r.height)
          : NSRect(x: r.x, y: r.y - slop, width: r.width, height: r.height + slop * 2)
      }
    }
    updateDecorations()
  }

  private func rebuildDividers() {
    for handle in dividerHandles { handle.removeFromSuperview() }
    let count = layoutTree.dividers(in: LayoutRect(x: 0, y: 0, width: 1000, height: 1000)).count
    dividerHandles = (0..<count).map { _ in
      let handle = DividerHandle()
      handle.owner = self
      addSubview(handle, positioned: .above, relativeTo: nil)
      return handle
    }
  }

  private func updateDecorations() {
    let split = paneViews.count > 1 && zoomedPane == nil
    for (id, dim) in dimViews {
      dim.isHidden = !split || !dimsInactivePanes || id == focusedPane
    }
    if split, let frame = paneViews[focusedPane]?.frame {
      if focusMarker.superview !== self {
        addSubview(focusMarker, positioned: .above, relativeTo: nil)
      }
      focusMarker.isHidden = false
      focusMarker.frame = NSRect(x: frame.minX, y: frame.minY, width: frame.width, height: 2)
    } else {
      focusMarker.isHidden = true
    }
  }

  // MARK: Divider dragging

  fileprivate func drag(_ divider: LayoutDivider, to point: NSPoint) {
    let container = divider.container
    let length = divider.axis == .horizontal ? container.width : container.height
    let origin = divider.axis == .horizontal ? container.x : container.y
    let coordinate = Double(divider.axis == .horizontal ? point.x : point.y)
    let gaps = Double(Self.gap) * Double(childCount(at: divider.path) - 1)
    let available = max(1, length - gaps)
    let position = (coordinate - origin - Double(divider.index) * Double(Self.gap)) / available
    let minimum = min(0.45, Double(Self.minimumPaneSize) / available)
    let updated = layoutTree.movingDivider(
      at: divider.path, index: divider.index, to: position, minimum: minimum)
    guard updated != layoutTree else { return }
    layoutTree = updated
    needsLayout = true
    onLayoutChange?(updated)
  }

  fileprivate func resetSplit(at path: [Int]) {
    guard let node = layoutTree.node(at: path) else { return }
    let even = node.equalized()
    guard case .split(_, _, let ratios) = even else { return }
    var updated = layoutTree
    // Apply the even ratios divider by divider.
    var boundary = 0.0
    for index in 0..<(ratios.count - 1) {
      boundary += ratios[index]
      updated = updated.movingDivider(at: path, index: index, to: boundary, minimum: 0)
    }
    layoutTree = updated
    needsLayout = true
    onLayoutChange?(updated)
  }

  private func childCount(at path: [Int]) -> Int {
    if case .split(_, let children, _)? = layoutTree.node(at: path) { return children.count }
    return 1
  }

  // MARK: Click-to-focus

  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    if let monitor = mouseMonitor {
      NSEvent.removeMonitor(monitor)
      mouseMonitor = nil
    }
    guard window != nil else { return }
    mouseMonitor = NSEvent.addLocalMonitorForEvents(
      matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]
    ) { [weak self] event in
      self?.noteMouseDown(event)
      return event
    }
  }

  private func noteMouseDown(_ event: NSEvent) {
    guard let window, event.window === window, !isHiddenOrHasHiddenAncestor,
      paneViews.count > 1
    else { return }
    let point = convert(event.locationInWindow, from: nil)
    guard bounds.contains(point), let id = pane(at: point), id != focusedPane,
      let paneView = paneViews[id]
    else { return }
    // Only when the click actually reaches the pane (not an overlay above it).
    if let hit = window.contentView?.hitTest(
      window.contentView?.convert(event.locationInWindow, from: nil) ?? .zero),
      !hit.isDescendant(of: paneView)
    {
      return
    }
    onFocusPane?(id)
  }

  deinit {
    if let monitor = mouseMonitor { NSEvent.removeMonitor(monitor) }
  }
}

// MARK: - Pieces

/// Tints an inactive pane. Never takes clicks.
private final class DimView: NSView {
  override init(frame frameRect: NSRect) {
    super.init(frame: frameRect)
    wantsLayer = true
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// Invisible hit area over a divider gap: resize cursor, drag, double-click.
private final class DividerHandle: NSView {
  weak var owner: PaneLayoutView?
  var divider: LayoutDivider? {
    didSet { if divider?.axis != oldValue?.axis { window?.invalidateCursorRects(for: self) } }
  }

  override var isFlipped: Bool { true }

  override func resetCursorRects() {
    let cursor: NSCursor = divider?.axis == .vertical ? .resizeUpDown : .resizeLeftRight
    addCursorRect(bounds, cursor: cursor)
  }

  override func mouseDown(with event: NSEvent) {
    guard let divider, let owner, event.clickCount == 2 else { return }
    owner.resetSplit(at: divider.path)
  }

  override func mouseDragged(with event: NSEvent) {
    guard let divider, let owner else { return }
    owner.drag(divider, to: owner.convert(event.locationInWindow, from: nil))
  }
}
