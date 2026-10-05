import AppKit
import SwiftUI

/// The AppKit root of a window's content. It replaces SwiftUI's
/// `NavigationSplitView` (which brought a floating glass sidebar, a private
/// `NSSplitView` we had to dig out to read its width, and first-layout and
/// focus races) with a layout the app owns:
///
/// ```
/// ┌──────────────────────── titlebar (chrome bar) ────────────────────────┐
/// ├─────────────── banner (settings load warning, when any) ──────────────┤
/// │ left dock │ center column                         │ right dock        │
/// │           │  (tab content + terminal input)       │                   │
/// │           ├───────────── bottom dock ──────────────┤                   │
/// ├──────────────────────────── status bar ───────────────────────────────┤
/// ```
///
/// AppKit owns geometry and focus; SwiftUI renders the chrome inside
/// `NSHostingView`s. Docks resize by dragging the hairline dividers.
final class WorkbenchView: NSView {
  let model: WindowModel

  private let titlebarHost: NSView
  private let bannerHost: NSView
  private let statusHost: NSView
  private let leftDock = DockContainer()
  private let rightDock = DockContainer()
  private let bottomDock = DockContainer()
  /// Center column: the active tab's content above the terminal input bar.
  let centerColumn = NSView()

  private let leftDivider = DockDivider(axis: .vertical)
  private let rightDivider = DockDivider(axis: .vertical)
  private let bottomDivider = DockDivider(axis: .horizontal)

  private var leftWidth: NSLayoutConstraint!
  private var rightWidth: NSLayoutConstraint!
  private var bottomHeight: NSLayoutConstraint!

  private var observation: ObservationLoop?

  static let leftDockRange: ClosedRange<CGFloat> = 180...520
  static let rightDockRange: ClosedRange<CGFloat> = 280...1100
  static let bottomDockRange: ClosedRange<CGFloat> = 100...700

  init(
    model: WindowModel,
    titlebar: NSView,
    banner: NSView,
    statusBar: NSView,
    leftDockContent: NSView,
    rightDockContent: NSView?,
    bottomDockContent: NSView?
  ) {
    self.model = model
    self.titlebarHost = titlebar
    self.bannerHost = banner
    self.statusHost = statusBar
    super.init(frame: .zero)
    wantsLayer = true

    leftDock.setContent(leftDockContent)
    if let rightDockContent { rightDock.setContent(rightDockContent) }
    if let bottomDockContent { bottomDock.setContent(bottomDockContent) }

    for view in [
      centerColumn, bottomDock, leftDock, rightDock, leftDivider, rightDivider, bottomDivider,
      bannerHost, statusHost, titlebarHost,
    ] as [NSView] {
      view.translatesAutoresizingMaskIntoConstraints = false
      addSubview(view)
    }
    centerColumn.wantsLayer = true

    leftWidth = leftDock.widthAnchor.constraint(equalToConstant: model.sidebarWidth)
    rightWidth = rightDock.widthAnchor.constraint(equalToConstant: model.rightDockWidth)
    bottomHeight = bottomDock.heightAnchor.constraint(equalToConstant: model.bottomDockHeight)

    NSLayoutConstraint.activate([
      titlebarHost.topAnchor.constraint(equalTo: topAnchor),
      titlebarHost.leadingAnchor.constraint(equalTo: leadingAnchor),
      titlebarHost.trailingAnchor.constraint(equalTo: trailingAnchor),
      titlebarHost.heightAnchor.constraint(equalToConstant: Metrics.titlebarHeight),

      bannerHost.topAnchor.constraint(equalTo: titlebarHost.bottomAnchor),
      bannerHost.leadingAnchor.constraint(equalTo: leadingAnchor),
      bannerHost.trailingAnchor.constraint(equalTo: trailingAnchor),

      statusHost.leadingAnchor.constraint(equalTo: leadingAnchor),
      statusHost.trailingAnchor.constraint(equalTo: trailingAnchor),
      statusHost.bottomAnchor.constraint(equalTo: bottomAnchor),
      statusHost.heightAnchor.constraint(equalToConstant: Metrics.statusBarHeight),

      leftDock.topAnchor.constraint(equalTo: bannerHost.bottomAnchor),
      leftDock.bottomAnchor.constraint(equalTo: statusHost.topAnchor),
      leftDock.leadingAnchor.constraint(equalTo: leadingAnchor),
      leftWidth,

      leftDivider.topAnchor.constraint(equalTo: leftDock.topAnchor),
      leftDivider.bottomAnchor.constraint(equalTo: leftDock.bottomAnchor),
      leftDivider.centerXAnchor.constraint(equalTo: leftDock.trailingAnchor),
      leftDivider.widthAnchor.constraint(equalToConstant: DockDivider.hitThickness),

      rightDock.topAnchor.constraint(equalTo: bannerHost.bottomAnchor),
      rightDock.bottomAnchor.constraint(equalTo: statusHost.topAnchor),
      rightDock.trailingAnchor.constraint(equalTo: trailingAnchor),
      rightWidth,

      rightDivider.topAnchor.constraint(equalTo: rightDock.topAnchor),
      rightDivider.bottomAnchor.constraint(equalTo: rightDock.bottomAnchor),
      rightDivider.centerXAnchor.constraint(equalTo: rightDock.leadingAnchor),
      rightDivider.widthAnchor.constraint(equalToConstant: DockDivider.hitThickness),

      centerColumn.topAnchor.constraint(equalTo: bannerHost.bottomAnchor),
      centerColumn.leadingAnchor.constraint(equalTo: leftDock.trailingAnchor),
      centerColumn.trailingAnchor.constraint(equalTo: rightDock.leadingAnchor),
      centerColumn.bottomAnchor.constraint(equalTo: bottomDock.topAnchor),

      bottomDock.leadingAnchor.constraint(equalTo: centerColumn.leadingAnchor),
      bottomDock.trailingAnchor.constraint(equalTo: centerColumn.trailingAnchor),
      bottomDock.bottomAnchor.constraint(equalTo: statusHost.topAnchor),
      bottomHeight,

      bottomDivider.leadingAnchor.constraint(equalTo: bottomDock.leadingAnchor),
      bottomDivider.trailingAnchor.constraint(equalTo: bottomDock.trailingAnchor),
      bottomDivider.centerYAnchor.constraint(equalTo: bottomDock.topAnchor),
      bottomDivider.heightAnchor.constraint(equalToConstant: DockDivider.hitThickness),
    ])

    configureDividers()
    observation = ObservationLoop(owner: self) { [weak self] in
      self?.applyModel()
    }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  override var isFlipped: Bool { true }

  /// Install the content for the right or bottom dock (e.g. Review, Problems).
  func setRightDockContent(_ view: NSView?) { rightDock.setContent(view) }
  func setBottomDockContent(_ view: NSView?) { bottomDock.setContent(view) }

  // MARK: - Model → layout

  private func applyModel() {
    let palette = model.palette
    let leftVisible = model.sidebarVisible
    let rightVisible = model.rightDockVisible && rightDock.hasContent
    let bottomVisible = model.bottomDockVisible && bottomDock.hasContent

    leftWidth.constant = leftVisible ? clamp(model.sidebarWidth, Self.leftDockRange) : 0
    rightWidth.constant = rightVisible ? clamp(model.rightDockWidth, Self.rightDockRange) : 0
    bottomHeight.constant =
      bottomVisible ? clamp(model.bottomDockHeight, Self.bottomDockRange) : 0
    leftDock.isHidden = !leftVisible
    rightDock.isHidden = !rightVisible
    bottomDock.isHidden = !bottomVisible
    leftDivider.isHidden = !leftVisible
    rightDivider.isHidden = !rightVisible
    bottomDivider.isHidden = !bottomVisible

    layer?.backgroundColor = palette.nsChrome.cgColor
    centerColumn.layer?.backgroundColor = palette.nsContent.cgColor
    leftDock.backgroundColor = palette.nsChrome
    rightDock.backgroundColor = palette.nsPanel
    bottomDock.backgroundColor = palette.nsPanel
    for divider in [leftDivider, rightDivider, bottomDivider] {
      divider.lineColor = palette.nsHairline
      divider.highlightColor = palette.nsAccent
    }
    needsLayout = true
  }

  private func clamp(_ value: CGFloat, _ range: ClosedRange<CGFloat>) -> CGFloat {
    min(max(value, range.lowerBound), range.upperBound)
  }

  // MARK: - Dividers

  private func configureDividers() {
    leftDivider.lineEdge = .center
    rightDivider.lineEdge = .center
    bottomDivider.lineEdge = .center

    var leftStart: CGFloat = 0
    leftDivider.onDragBegan = { [weak self] in leftStart = self?.leftWidth.constant ?? 0 }
    leftDivider.onDrag = { [weak self] delta in
      guard let self else { return }
      self.model.sidebarWidth = self.clamp(leftStart + delta, Self.leftDockRange)
    }
    leftDivider.onDoubleClick = { [weak self] in
      self?.model.sidebarWidth = Metrics.leftDockDefaultWidth
    }

    var rightStart: CGFloat = 0
    rightDivider.onDragBegan = { [weak self] in rightStart = self?.rightWidth.constant ?? 0 }
    rightDivider.onDrag = { [weak self] delta in
      guard let self else { return }
      self.model.rightDockWidth = self.clamp(rightStart - delta, Self.rightDockRange)
    }
    rightDivider.onDoubleClick = { [weak self] in
      self?.model.rightDockWidth = Metrics.rightDockDefaultWidth
    }

    var bottomStart: CGFloat = 0
    bottomDivider.onDragBegan = { [weak self] in bottomStart = self?.bottomHeight.constant ?? 0 }
    bottomDivider.onDrag = { [weak self] delta in
      guard let self else { return }
      self.model.bottomDockHeight = self.clamp(bottomStart - delta, Self.bottomDockRange)
    }
    bottomDivider.onDoubleClick = { [weak self] in
      self?.model.bottomDockHeight = Metrics.bottomDockDefaultHeight
    }
  }

  /// Current left-dock width (for persisting on window close).
  var currentLeftDockWidth: CGFloat { leftWidth.constant }
}

// MARK: - Dock container

/// Hosts one dock's content view with a themed background and clipping.
final class DockContainer: NSView {
  private(set) var content: NSView?

  var hasContent: Bool { content != nil }

  var backgroundColor: NSColor = .clear {
    didSet { layer?.backgroundColor = backgroundColor.cgColor }
  }

  init() {
    super.init(frame: .zero)
    wantsLayer = true
    layer?.masksToBounds = true
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  func setContent(_ view: NSView?) {
    content?.removeFromSuperview()
    content = view
    guard let view else { return }
    view.translatesAutoresizingMaskIntoConstraints = false
    addSubview(view)
    NSLayoutConstraint.activate([
      view.topAnchor.constraint(equalTo: topAnchor),
      view.leadingAnchor.constraint(equalTo: leadingAnchor),
      view.trailingAnchor.constraint(equalTo: trailingAnchor),
      view.bottomAnchor.constraint(equalTo: bottomAnchor),
    ])
  }
}

// MARK: - Dock divider

/// A hairline between regions with a wider invisible grab band. Highlights in
/// the accent color while hovered or dragged; double-click resets the size.
final class DockDivider: NSView {
  enum Axis { case vertical, horizontal }
  enum LineEdge { case center }

  static let hitThickness: CGFloat = 7

  let axis: Axis
  var lineEdge: LineEdge = .center
  var lineColor: NSColor = .separatorColor { didSet { needsDisplay = true } }
  var highlightColor: NSColor = .controlAccentColor { didSet { needsDisplay = true } }

  var onDragBegan: (() -> Void)?
  var onDrag: ((CGFloat) -> Void)?
  var onDoubleClick: (() -> Void)?

  private var hovering = false { didSet { needsDisplay = true } }
  private var dragging = false { didSet { needsDisplay = true } }
  private var dragOrigin: NSPoint = .zero
  private var trackingArea: NSTrackingArea?

  init(axis: Axis) {
    self.axis = axis
    super.init(frame: .zero)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  override var isFlipped: Bool { true }
  override var mouseDownCanMoveWindow: Bool { false }

  override func draw(_ dirtyRect: NSRect) {
    // Clip to bounds: under NSHostingView siblings on macOS 26, dirty rects can
    // extend past the view.
    NSBezierPath(rect: bounds).setClip()
    let active = hovering || dragging
    let thickness: CGFloat = active ? 2 : 1
    (active ? highlightColor.withAlphaComponent(0.8) : lineColor).setFill()
    switch axis {
    case .vertical:
      NSRect(x: bounds.midX - thickness / 2, y: 0, width: thickness, height: bounds.height).fill()
    case .horizontal:
      NSRect(x: 0, y: bounds.midY - thickness / 2, width: bounds.width, height: thickness).fill()
    }
  }

  override func resetCursorRects() {
    addCursorRect(bounds, cursor: axis == .vertical ? .resizeLeftRight : .resizeUpDown)
  }

  override func updateTrackingAreas() {
    super.updateTrackingAreas()
    if let trackingArea { removeTrackingArea(trackingArea) }
    let area = NSTrackingArea(
      rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
      owner: self)
    addTrackingArea(area)
    trackingArea = area
  }

  override func mouseEntered(with event: NSEvent) { hovering = true }
  override func mouseExited(with event: NSEvent) { hovering = false }

  override func mouseDown(with event: NSEvent) {
    if event.clickCount == 2 {
      onDoubleClick?()
      return
    }
    dragging = true
    dragOrigin = NSEvent.mouseLocation
    onDragBegan?()
  }

  override func mouseDragged(with event: NSEvent) {
    guard dragging else { return }
    let now = NSEvent.mouseLocation
    // Screen coordinates are y-up; the workbench is flipped (y-down).
    let delta = axis == .vertical ? now.x - dragOrigin.x : dragOrigin.y - now.y
    onDrag?(delta)
  }

  override func mouseUp(with event: NSEvent) {
    dragging = false
  }
}
