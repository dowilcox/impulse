import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

/// What the diff list asks of the review it belongs to.
protocol ReviewDiffHandler: AnyObject {
  func reviewToggleExpanded(_ path: String)
  func reviewSetViewed(_ path: String, viewed: Bool)
  func reviewFileAction(_ action: ReviewAction, path: String)
  func reviewHunkAction(_ action: ReviewAction, path: String, hunk: Int)
  func reviewToggleLine(path: String, hunk: Int, line: Int, extend: Bool)
  /// Write a comment after `line` (nil: for the hunk or its selection).
  func reviewOpenComposer(path: String, hunk: Int, line: Int?)
  func reviewSaveComposer(path: String, text: String)
  func reviewCancelComposer(path: String)
  /// Start (id) or stop (nil) editing a comment inline.
  func reviewEditComment(_ id: String?, path: String)
  func reviewSaveComment(id: String, text: String)
  func reviewDeleteComment(id: String)
  func reviewOpenURL(_ url: String)
  func reviewOpenFile(path: String, line: Int?, diff: Bool)
  func reviewCopyPath(_ path: String)
  func reviewNeedsDiff(_ path: String)
  func reviewFocusHunk(path: String, hunk: Int)
  /// Keys the list doesn't handle itself (j/k, s, v, …); true when handled.
  func reviewKey(_ event: NSEvent) -> Bool
  /// ⌘C: copy the selected lines.
  func reviewCopy()
  /// The file at the top of the list changed (navigator highlight).
  func reviewTopFileChanged(_ path: String?)
}

/// Colors for the review's AppKit drawing, derived from the theme the way
/// ChromePalette derives the chrome.
struct ReviewColors {
  let theme: Theme
  let palette: ChromePalette
  let page: NSColor
  let card: NSColor
  let header: NSColor
  let hunkHeader: NSColor
  let border: NSColor
  let borderStrong: NSColor
  let text: NSColor
  let text2: NSColor
  let text3: NSColor
  let accent: NSColor
  let onAccent: NSColor
  let added: NSColor
  let removed: NSColor
  let addedBackground: NSColor
  let removedBackground: NSColor
  let addedWord: NSColor
  let removedWord: NSColor
  let selection: NSColor
  let hover: NSColor

  init(theme: Theme) {
    self.theme = theme
    let palette = ChromePalette(theme: theme)
    self.palette = palette
    let bg = NSColor(hex: theme.bg)
    let fg = NSColor(hex: theme.fg)
    let light = theme.isLight
    page = bg
    card = bg
    header = palette.nsPanel
    hunkHeader = ChromePalette.mix(bg, toward: NSColor(hex: theme.accent), amount: 0.06)
    border = palette.nsHairline
    borderStrong = ChromePalette.mix(bg, toward: fg, amount: 0.2)
    text = fg
    text2 = NSColor(hex: theme.fgMuted)
    text3 = NSColor(hex: theme.fgComment)
    accent = NSColor(hex: theme.accent)
    onAccent = ChromePalette.readableText(on: accent)
    added = NSColor(hex: theme.gitAdded)
    removed = NSColor(hex: theme.gitDeleted)
    addedBackground = added.withAlphaComponent(light ? 0.12 : 0.1)
    removedBackground = removed.withAlphaComponent(light ? 0.12 : 0.11)
    addedWord = added.withAlphaComponent(light ? 0.3 : 0.28)
    removedWord = removed.withAlphaComponent(0.3)
    selection = accent.withAlphaComponent(light ? 0.18 : 0.22)
    hover = fg.withAlphaComponent(light ? 0.06 : 0.07)
  }
}

/// Everything a row needs to draw itself, shared by the controller's views.
final class ReviewDiffContext {
  var files: [String: ReviewFile] = [:]
  var layout: ReviewLayout = .unified
  var capabilities = ReviewCapabilities(stage: false, unstage: false, revert: false)
  var focus: (path: String, hunk: Int)?
  var colors: ReviewColors
  var metrics: ReviewMetrics
  weak var handler: ReviewDiffHandler?

  init(theme: Theme, metrics: ReviewMetrics) {
    colors = ReviewColors(theme: theme)
    self.metrics = metrics
  }

  func isFocused(_ path: String, hunk: Int?) -> Bool {
    guard let focus, let hunk else { return false }
    return focus.path == path && focus.hunk == hunk
  }
}

// MARK: - Table

/// The review's diff list: a single-column table whose rows are file
/// headers (floating while their file scrolls), hunk headers, diff lines,
/// comments and notices. Only on-screen rows have views.
final class ReviewDiffController: NSObject, NSTableViewDataSource, NSTableViewDelegate {
  let context: ReviewDiffContext
  let tableView = ReviewTableView()
  let scrollView = NSScrollView()
  private(set) var rows: [ReviewRow] = []
  private var heights: [CGFloat] = []
  private var heightsWidth: CGFloat = 0
  private var lastTopPath: String?
  private var resizeWork: DispatchWorkItem?

  init(context: ReviewDiffContext) {
    self.context = context
    super.init()
    let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("diff"))
    column.resizingMask = .autoresizingMask
    tableView.addTableColumn(column)
    tableView.headerView = nil
    tableView.style = .plain
    tableView.rowSizeStyle = .custom
    tableView.intercellSpacing = .zero
    tableView.selectionHighlightStyle = .none
    tableView.allowsEmptySelection = true
    tableView.floatsGroupRows = true
    tableView.usesAutomaticRowHeights = false
    tableView.gridStyleMask = []
    tableView.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
    tableView.dataSource = self
    tableView.delegate = self
    tableView.controller = self
    tableView.backgroundColor = context.colors.page
    tableView.focusRingType = .none

    scrollView.documentView = tableView
    scrollView.hasVerticalScroller = true
    scrollView.hasHorizontalScroller = false
    scrollView.autohidesScrollers = true
    scrollView.drawsBackground = true
    scrollView.backgroundColor = context.colors.page
    scrollView.scrollerStyle = .overlay
    scrollView.automaticallyAdjustsContentInsets = false
    // No top inset: floating headers pin to the inset edge, and content
    // would show through the gap above them (`.pageTop` pads instead).
    scrollView.contentInsets = NSEdgeInsets(top: 0, left: 0, bottom: 240, right: 0)
    scrollView.contentView.postsBoundsChangedNotifications = true
    NotificationCenter.default.addObserver(
      self, selector: #selector(boundsChanged), name: NSView.boundsDidChangeNotification,
      object: scrollView.contentView)
    tableView.postsFrameChangedNotifications = true
    NotificationCenter.default.addObserver(
      self, selector: #selector(frameChanged), name: NSView.frameDidChangeNotification, object: tableView)
  }

  deinit {
    NotificationCenter.default.removeObserver(self)
  }

  func applyColors() {
    tableView.backgroundColor = context.colors.page
    scrollView.backgroundColor = context.colors.page
  }

  // MARK: Reloading

  /// Show `newRows`, keeping the row at the top of the view where it was.
  func reload(_ newRows: [ReviewRow]) {
    let anchor = topAnchor()
    rows = newRows.isEmpty ? [] : [.pageTop] + newRows
    heights = []
    tableView.reloadData()
    if let anchor { restore(anchor) }
    boundsChanged()
  }

  /// Redraw rows in place (selection, focus, busy) without re-measuring.
  func redrawVisibleRows() {
    let visible = tableView.rows(in: tableView.visibleRect)
    guard visible.length > 0 else { return }
    tableView.reloadData(
      forRowIndexes: IndexSet(integersIn: visible.location..<(visible.location + visible.length)),
      columnIndexes: IndexSet(integer: 0))
    tableView.enumerateAvailableRowViews { rowView, _ in rowView.needsDisplay = true }
  }

  private func topAnchor() -> (row: ReviewRow, offset: CGFloat)? {
    guard !rows.isEmpty else { return nil }
    // The first row the reader sees: below the inset and the floating header.
    let visible = tableView.visibleRect
    let y = visible.minY + scrollView.contentView.contentInsets.top + ReviewMetrics.fileHeaderHeight + 1
    let index = tableView.row(at: NSPoint(x: 1, y: y))
    guard rows.indices.contains(index) else { return nil }
    return (rows[index], visible.minY - tableView.rect(ofRow: index).minY)
  }

  private func restore(_ anchor: (row: ReviewRow, offset: CGFloat)) {
    let index =
      rows.firstIndex(of: anchor.row)
      ?? rows.firstIndex(of: .fileHeader(anchor.row.path))
    guard let index else { return }
    let offset = rows[index] == anchor.row ? anchor.offset : 0
    scroll(toY: tableView.rect(ofRow: index).minY + offset)
  }

  /// Scroll so `path`'s header (or one of its hunks) is at the top.
  func reveal(path: String, hunk: Int? = nil, onlyIfNeeded: Bool = false) {
    let target: ReviewRow = hunk.map { .hunkHeader(path, hunk: $0) } ?? .fileHeader(path)
    guard let index = rows.firstIndex(of: target) ?? rows.firstIndex(of: .fileHeader(path)) else {
      return
    }
    let rect = tableView.rect(ofRow: index)
    // The floating header sits below the top inset.
    let top = tableView.visibleRect.minY + scrollView.contentView.contentInsets.top
    if onlyIfNeeded {
      // Below the floating header and above the bottom edge: leave it.
      if rect.minY >= top + ReviewMetrics.fileHeaderHeight, rect.maxY <= tableView.visibleRect.maxY { return }
    }
    // Leave the file's floating header above a hunk.
    let headerRoom = hunk == nil ? 0 : ReviewMetrics.fileHeaderHeight
    scroll(toY: rect.minY - headerRoom - scrollView.contentView.contentInsets.top)
  }

  func scroll(toY y: CGFloat) {
    let clip = scrollView.contentView
    let maxY = max(-clip.contentInsets.top, tableView.frame.height - clip.bounds.height + clip.contentInsets.bottom)
    let target = min(max(y, -clip.contentInsets.top), maxY)
    clip.scroll(to: NSPoint(x: 0, y: target))
    scrollView.reflectScrolledClipView(clip)
  }

  @objc private func boundsChanged() {
    let visible = tableView.visibleRect
    let top = visible.minY + scrollView.contentView.contentInsets.top
    let index = tableView.row(at: NSPoint(x: 1, y: top + ReviewMetrics.fileHeaderHeight + 2))
    let path = rows.indices.contains(index) && rows[index] != .pageTop ? rows[index].path : rows.dropFirst().first?.path
    if path != lastTopPath {
      lastTopPath = path
      context.handler?.reviewTopFileChanged(path)
    }
  }

  @objc private func frameChanged() {
    guard abs(tableView.bounds.width - heightsWidth) > 0.5, !rows.isEmpty else { return }
    // Re-wrap once the width settles.
    resizeWork?.cancel()
    let work = DispatchWorkItem { [weak self] in
      guard let self else { return }
      let anchor = self.topAnchor()
      self.heights = []
      self.tableView.noteHeightOfRows(withIndexesChanged: IndexSet(integersIn: 0..<self.rows.count))
      if let anchor { self.restore(anchor) }
    }
    resizeWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: work)
  }

  // MARK: Heights

  func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

  func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
    let width = tableView.bounds.width > 0 ? tableView.bounds.width : 800
    if heights.count != rows.count || abs(width - heightsWidth) > 0.5 {
      heightsWidth = width
      heights = rows.map { height(of: $0, width: width) }
    }
    return rows.indices.contains(row) ? heights[row] : 20
  }

  private func height(of row: ReviewRow, width: CGFloat) -> CGFloat {
    let metrics = context.metrics
    switch row {
    case .fileHeader: return ReviewMetrics.fileHeaderHeight
    case .fileEnd: return ReviewMetrics.fileEndHeight
    case .pageTop: return ReviewMetrics.pageTopHeight
    case .notice: return ReviewMetrics.noticeHeight
    case .outdatedTitle: return ReviewMetrics.outdatedTitleHeight
    case .hunkHeader: return ReviewMetrics.hunkHeaderHeight
    case .composer: return ReviewMetrics.composerHeight
    case .comment(let path, let id):
      guard let file = context.files[path], let comment = file.comments.first(where: { $0.id == id }) else {
        return 40
      }
      if file.editingComment == id { return ReviewMetrics.editingCommentHeight }
      return metrics.commentHeight(
        ReviewCommentRow.displayText(comment), tableWidth: width, layout: context.layout)
    case .line(let path, let hunk, let line):
      guard let text = lineText(path: path, hunk: hunk, line: line) else { return metrics.lineHeight }
      let codeWidth = metrics.codeWidth(tableWidth: width, layout: .unified)
      return CGFloat(metrics.wrappedLineCount(text, width: codeWidth)) * metrics.lineHeight
    case .split(let path, let hunk, let old, let new):
      let codeWidth = metrics.codeWidth(tableWidth: width, layout: .split)
      let counts = [old, new].compactMap { $0 }.map { index in
        metrics.wrappedLineCount(lineText(path: path, hunk: hunk, line: index) ?? "", width: codeWidth)
      }
      return CGFloat(counts.max() ?? 1) * metrics.lineHeight
    }
  }

  private func lineText(path: String, hunk: Int, line: Int) -> String? {
    guard let diff = context.files[path]?.diff, diff.hunks.indices.contains(hunk),
      diff.hunks[hunk].lines.indices.contains(line)
    else { return nil }
    return diff.hunks[hunk].lines[line].content
  }

  // MARK: Views

  func tableView(_ tableView: NSTableView, isGroupRow row: Int) -> Bool {
    if case .fileHeader = rows[row] { return true }
    return false
  }

  func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
    let view =
      tableView.makeView(withIdentifier: ReviewRowView.identifier, owner: nil) as? ReviewRowView
      ?? ReviewRowView()
    view.identifier = ReviewRowView.identifier
    view.configure(row: rows[row], context: context)
    return view
  }

  func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
    let item = rows[row]
    switch item {
    case .line, .split:
      let view =
        tableView.makeView(withIdentifier: DiffLineCellView.identifier, owner: nil) as? DiffLineCellView
        ?? DiffLineCellView()
      view.identifier = DiffLineCellView.identifier
      view.configure(row: item, context: context)
      return view
    case .fileEnd, .pageTop:
      return nil
    default:
      if case .notice(let path, .loading) = item { context.handler?.reviewNeedsDiff(path) }
      if case .fileHeader(let path) = item, let file = context.files[path], file.expanded, file.stale,
        file.diff != nil
      {
        context.handler?.reviewNeedsDiff(path)
      }
      let view =
        tableView.makeView(withIdentifier: ReviewHostedCellView.identifier, owner: nil) as? ReviewHostedCellView
        ?? ReviewHostedCellView()
      view.identifier = ReviewHostedCellView.identifier
      view.set(ReviewRowContent.view(for: item, context: context))
      return view
    }
  }
}

/// Keys and copy go to the review.
final class ReviewTableView: NSTableView {
  weak var controller: ReviewDiffController?

  override var acceptsFirstResponder: Bool { true }

  override func keyDown(with event: NSEvent) {
    if controller?.context.handler?.reviewKey(event) == true { return }
    super.keyDown(with: event)
  }

  @objc func copy(_ sender: Any?) {
    controller?.context.handler?.reviewCopy()
  }

  // No row selection: clicks belong to the rows' own controls.
  override func mouseDown(with event: NSEvent) {
    window?.makeFirstResponder(self)
    super.mouseDown(with: event)
  }

  override func validateProposedFirstResponder(_ responder: NSResponder, for event: NSEvent?) -> Bool {
    true
  }
}

// MARK: - Row background (cards)

/// Draws the page and each file's card: the header's top corners, the
/// sides, and the bottom corners on the file's last row.
final class ReviewRowView: NSTableRowView {
  static let identifier = NSUserInterfaceItemIdentifier("ReviewRowView")
  private var row: ReviewRow = .fileEnd("")
  private weak var context: ReviewDiffContext?

  func configure(row: ReviewRow, context: ReviewDiffContext) {
    self.row = row
    self.context = context
    needsDisplay = true
  }

  override func drawBackground(in dirtyRect: NSRect) {
    guard let context else { return }
    let colors = context.colors
    colors.page.setFill()
    bounds.fill()
    let inset = ReviewMetrics.cardInset
    var card = bounds.insetBy(dx: inset, dy: 0)
    let radius: CGFloat = 8
    colors.border.setStroke()
    switch row {
    case .fileHeader(let path):
      let expanded = context.files[path]?.expanded ?? true
      // A collapsed file's header is the whole card.
      let shape = NSBezierPath()
      let rect = expanded ? card.insetBy(dx: 0.5, dy: 0).offsetBy(dx: 0, dy: 0) : card.insetBy(dx: 0.5, dy: 0.5)
      if expanded {
        shape.appendRoundedTop(rect: rect, radius: radius, flipped: isFlipped)
      } else {
        shape.appendRoundedRect(rect, xRadius: radius, yRadius: radius)
      }
      colors.header.setFill()
      shape.fill()
      shape.lineWidth = 1
      shape.stroke()
      if expanded {
        colors.border.setFill()
        NSRect(x: card.minX, y: isFlipped ? card.maxY - 1 : card.minY, width: card.width, height: 1).fill()
      }
    case .pageTop:
      return
    case .fileEnd(let path):
      guard context.files[path]?.expanded ?? false else { return }
      card.size.height = 6
      if !isFlipped { card.origin.y = bounds.maxY - 6 }
      let shape = NSBezierPath()
      shape.appendRoundedBottom(rect: card.insetBy(dx: 0.5, dy: 0).offsetBy(dx: 0, dy: isFlipped ? -0.5 : 0.5), radius: 6, flipped: isFlipped)
      colors.card.setFill()
      shape.fill()
      shape.lineWidth = 1
      shape.stroke()
    default:
      colors.card.setFill()
      card.fill()
      colors.border.setFill()
      NSRect(x: card.minX, y: card.minY, width: 1, height: card.height).fill()
      NSRect(x: card.maxX - 1, y: card.minY, width: 1, height: card.height).fill()
      if case .hunkHeader(_, let hunk) = row, hunk > 0 {
        NSRect(x: card.minX, y: isFlipped ? card.minY : card.maxY - 1, width: card.width, height: 1).fill()
      }
    }
  }

  override func drawSelection(in dirtyRect: NSRect) {}
  override var isEmphasized: Bool {
    get { false }
    set {}
  }
}

extension NSBezierPath {
  /// A rectangle with its top corners rounded (top meaning the screen top).
  fileprivate func appendRoundedTop(rect: NSRect, radius: CGFloat, flipped: Bool) {
    let r = min(radius, rect.height / 2, rect.width / 2)
    let top = flipped ? rect.minY : rect.maxY
    let bottom = flipped ? rect.maxY : rect.minY
    let s: CGFloat = flipped ? 1 : -1
    move(to: NSPoint(x: rect.minX, y: bottom))
    line(to: NSPoint(x: rect.minX, y: top + s * r))
    curve(
      to: NSPoint(x: rect.minX + r, y: top), controlPoint1: NSPoint(x: rect.minX, y: top),
      controlPoint2: NSPoint(x: rect.minX, y: top))
    line(to: NSPoint(x: rect.maxX - r, y: top))
    curve(
      to: NSPoint(x: rect.maxX, y: top + s * r), controlPoint1: NSPoint(x: rect.maxX, y: top),
      controlPoint2: NSPoint(x: rect.maxX, y: top))
    line(to: NSPoint(x: rect.maxX, y: bottom))
  }

  /// A rectangle with its bottom corners rounded (bottom of the screen).
  fileprivate func appendRoundedBottom(rect: NSRect, radius: CGFloat, flipped: Bool) {
    let r = min(radius, rect.height, rect.width / 2)
    let top = flipped ? rect.minY : rect.maxY
    let bottom = flipped ? rect.maxY : rect.minY
    let s: CGFloat = flipped ? -1 : 1
    move(to: NSPoint(x: rect.minX, y: top))
    line(to: NSPoint(x: rect.minX, y: bottom + s * r))
    curve(
      to: NSPoint(x: rect.minX + r, y: bottom), controlPoint1: NSPoint(x: rect.minX, y: bottom),
      controlPoint2: NSPoint(x: rect.minX, y: bottom))
    line(to: NSPoint(x: rect.maxX - r, y: bottom))
    curve(
      to: NSPoint(x: rect.maxX, y: bottom + s * r), controlPoint1: NSPoint(x: rect.maxX, y: bottom),
      controlPoint2: NSPoint(x: rect.maxX, y: bottom))
    line(to: NSPoint(x: rect.maxX, y: top))
  }
}

// MARK: - Diff lines

/// One diff line (unified) or a pair of lines (split): gutters with line
/// numbers, a +/− marker, and the code with syntax colors and word-level
/// highlights. Click a changed line's number to select it (⇧ for a range);
/// the + in the gutter writes a comment.
final class DiffLineCellView: NSView {
  static let identifier = NSUserInterfaceItemIdentifier("DiffLineCellView")
  private var row: ReviewRow = .fileEnd("")
  private weak var context: ReviewDiffContext?
  private var hoverSide: Side?
  private var trackingArea: NSTrackingArea?

  private enum Side { case old, new, single }

  override var isFlipped: Bool { true }

  func configure(row: ReviewRow, context: ReviewDiffContext) {
    self.row = row
    self.context = context
    needsDisplay = true
  }

  override func updateTrackingAreas() {
    super.updateTrackingAreas()
    if let trackingArea { removeTrackingArea(trackingArea) }
    let area = NSTrackingArea(
      rect: bounds, options: [.mouseEnteredAndExited, .mouseMoved, .activeInKeyWindow, .inVisibleRect],
      owner: self, userInfo: nil)
    addTrackingArea(area)
    trackingArea = area
  }

  override func mouseMoved(with event: NSEvent) { updateHover(event) }
  override func mouseEntered(with event: NSEvent) { updateHover(event) }
  override func mouseExited(with event: NSEvent) {
    hoverSide = nil
    needsDisplay = true
  }

  private func updateHover(_ event: NSEvent) {
    let point = convert(event.locationInWindow, from: nil)
    let side = self.side(at: point.x)
    if side != hoverSide {
      hoverSide = side
      needsDisplay = true
    }
  }

  // MARK: Geometry

  private var card: NSRect { bounds.insetBy(dx: ReviewMetrics.cardInset, dy: 0) }

  /// The column layout for one side: gutter(s), marker, code.
  private struct Columns {
    var x: CGFloat
    var width: CGFloat
    var gutters: [NSRect]
    var marker: NSRect
    var code: NSRect
  }

  private func columns() -> [Columns] {
    let card = self.card
    let g = ReviewMetrics.gutterWidth
    let m = ReviewMetrics.markerWidth
    switch row {
    case .split:
      let half = (card.width / 2).rounded(.down)
      return [0, 1].map { i in
        let x = card.minX + CGFloat(i) * half
        let width = i == 0 ? half : card.width - half
        return Columns(
          x: x, width: width, gutters: [NSRect(x: x, y: 0, width: g, height: bounds.height)],
          marker: NSRect(x: x + g, y: 0, width: m, height: bounds.height),
          code: NSRect(x: x + g + m, y: 0, width: width - g - m - ReviewMetrics.codeTrailing, height: bounds.height))
      }
    default:
      let x = card.minX
      return [
        Columns(
          x: x, width: card.width,
          gutters: [
            NSRect(x: x, y: 0, width: g, height: bounds.height),
            NSRect(x: x + g, y: 0, width: g, height: bounds.height),
          ],
          marker: NSRect(x: x + g * 2, y: 0, width: m, height: bounds.height),
          code: NSRect(
            x: x + g * 2 + m, y: 0, width: card.width - g * 2 - m - ReviewMetrics.codeTrailing,
            height: bounds.height))
      ]
    }
  }

  private func side(at x: CGFloat) -> Side? {
    switch row {
    case .split:
      let cols = columns()
      return x < cols[1].x ? .old : .new
    default:
      return .single
    }
  }

  /// The diff line shown on `side`, with its index in the hunk.
  private func line(on side: Side) -> (index: Int, line: DiffLine)? {
    guard let context else { return nil }
    let hunkLines: [DiffLine]
    let index: Int?
    switch row {
    case .line(let path, let hunk, let line):
      guard let diff = context.files[path]?.diff, diff.hunks.indices.contains(hunk) else { return nil }
      hunkLines = diff.hunks[hunk].lines
      index = line
    case .split(let path, let hunk, let old, let new):
      guard let diff = context.files[path]?.diff, diff.hunks.indices.contains(hunk) else { return nil }
      hunkLines = diff.hunks[hunk].lines
      index = side == .old ? old : new
    default:
      return nil
    }
    guard let index, hunkLines.indices.contains(index) else { return nil }
    // Split: a context line shows on both sides; an added line only on the
    // new side and a removed one only on the old.
    if case .split = row {
      let kind = hunkLines[index].kind
      if side == .old, kind == .added { return nil }
      if side == .new, kind == .removed { return nil }
    }
    return (index, hunkLines[index])
  }

  private var path: String { row.path }
  private var hunk: Int { row.hunk ?? 0 }

  // MARK: Drawing

  override func draw(_ dirtyRect: NSRect) {
    guard let context, let file = context.files[path] else { return }
    let colors = context.colors
    let metrics = context.metrics
    let sides: [Side] = { if case .split = row { return [.old, .new] } else { return [.single] } }()
    let cols = columns()
    let busy = file.busy

    for (i, side) in sides.enumerated() {
      let col = cols[i]
      let area = NSRect(x: col.x, y: 0, width: col.width, height: bounds.height)
      guard let (index, line) = line(on: side) else {
        // Nothing on this side of a split row.
        colors.hover.setFill()
        drawHatch(in: area.insetBy(dx: i == 0 ? 1 : 0, dy: 0), color: colors.hover)
        continue
      }
      let selected = file.selection.map { $0.hunk == hunk && $0.lines.contains(index) } ?? false
      // Background tint.
      switch line.kind {
      case .added: colors.addedBackground.setFill()
      case .removed: colors.removedBackground.setFill()
      case .context: NSColor.clear.setFill()
      }
      let tintRect = NSRect(
        x: area.minX + (i == 0 ? 1 : 0), y: 0, width: area.width - (i == 0 ? 1 : 0) - (i == sides.count - 1 ? 1 : 0),
        height: bounds.height)
      if line.kind != .context { tintRect.fill(using: .sourceOver) }
      if selected {
        colors.selection.setFill()
        tintRect.fill(using: .sourceOver)
        colors.accent.setFill()
        NSRect(x: tintRect.minX, y: 0, width: 3, height: bounds.height).fill()
      }

      // Line numbers.
      let numbers: [UInt32?] =
        side == .single ? [line.oldLineno, line.newLineno] : [side == .old ? line.oldLineno : line.newLineno]
      let hovering = hoverSide == side && line.kind != .context
      for (g, number) in numbers.enumerated() where g < col.gutters.count {
        let gutter = col.gutters[g]
        if hovering && !busy {
          colors.hover.setFill()
          gutter.fill(using: .sourceOver)
        }
        guard let number else { continue }
        let attrs: [NSAttributedString.Key: Any] = [
          .font: metrics.gutterFont,
          .foregroundColor: hovering ? colors.text : colors.text3,
        ]
        let string = NSAttributedString(string: String(number), attributes: attrs)
        let size = string.size()
        string.draw(at: NSPoint(x: gutter.maxX - 8 - size.width, y: metrics.baseline - metrics.gutterFont.ascender))
      }

      // Comment button on hover.
      if hoverSide == side, !busy {
        let button = NSRect(x: col.gutters[0].minX + 3, y: (metrics.lineHeight - 15) / 2, width: 15, height: 15)
        colors.accent.setFill()
        NSBezierPath(roundedRect: button, xRadius: 4, yRadius: 4).fill()
        let plus = NSAttributedString(
          string: "+",
          attributes: [.font: NSFont.systemFont(ofSize: 12, weight: .semibold), .foregroundColor: colors.onAccent])
        let size = plus.size()
        plus.draw(at: NSPoint(x: button.midX - size.width / 2, y: button.midY - size.height / 2))
      }

      // Marker.
      let marker = line.kind == .added ? "+" : line.kind == .removed ? "−" : " "
      let markerColor = line.kind == .added ? colors.added : line.kind == .removed ? colors.removed : colors.text3
      let markerString = NSAttributedString(
        string: marker, attributes: [.font: metrics.codeFont, .foregroundColor: markerColor])
      let markerSize = markerString.size()
      markerString.draw(at: NSPoint(x: col.marker.midX - markerSize.width / 2, y: metrics.codeTop))

      // Code.
      let code = attributedCode(line: line, index: index, file: file, colors: colors, metrics: metrics)
      code.draw(
        with: NSRect(x: col.code.minX, y: metrics.codeTop, width: col.code.width, height: bounds.height),
        options: [.usesLineFragmentOrigin])
    }

    if context.isFocused(path, hunk: hunk) {
      colors.accent.setFill()
      NSRect(x: card.minX + 1, y: 0, width: 2, height: bounds.height).fill()
    }
    if busy {
      colors.page.withAlphaComponent(0.45).setFill()
      card.fill(using: .sourceOver)
    }
  }

  private func drawHatch(in rect: NSRect, color: NSColor) {
    NSGraphicsContext.saveGraphicsState()
    NSBezierPath(rect: rect).addClip()
    color.setStroke()
    let path = NSBezierPath()
    path.lineWidth = 1
    var x = rect.minX - rect.height
    while x < rect.maxX {
      path.move(to: NSPoint(x: x, y: rect.maxY))
      path.line(to: NSPoint(x: x + rect.height, y: rect.minY))
      x += 5
    }
    path.stroke()
    NSGraphicsContext.restoreGraphicsState()
  }

  private func attributedCode(
    line: DiffLine, index: Int, file: ReviewFile, colors: ReviewColors, metrics: ReviewMetrics
  ) -> NSAttributedString {
    let paragraph = NSMutableParagraphStyle()
    paragraph.lineBreakMode = .byCharWrapping
    paragraph.tabStops = []
    paragraph.defaultTabInterval = metrics.charWidth * CGFloat(metrics.tabWidth)
    // Every wrapped line is exactly `lineHeight` apart, whatever fallback
    // fonts a line needs.
    paragraph.minimumLineHeight = metrics.naturalLineHeight
    paragraph.maximumLineHeight = metrics.naturalLineHeight
    paragraph.lineSpacing = metrics.lineHeight - metrics.naturalLineHeight
    let text = NSMutableAttributedString(
      string: line.content,
      attributes: [.font: metrics.codeFont, .foregroundColor: colors.text, .paragraphStyle: paragraph])
    let length = text.length
    if let spans = file.syntax?[safe: hunk]?[safe: index] {
      for span in spans where span.location < length {
        let range = NSRange(location: span.location, length: min(span.length, length - span.location))
        text.addAttribute(.foregroundColor, value: colors.theme.syntaxColor(span.token), range: range)
      }
    }
    // Word-level changes, unless most of the line changed.
    let words = Self.usefulSpans(line)
    let wordColor = line.kind == .added ? colors.addedWord : colors.removedWord
    for span in words {
      let start = Int(span.start)
      guard start < length else { continue }
      let range = NSRange(location: start, length: min(Int(span.end) - start, length - start))
      text.addAttribute(.backgroundColor, value: wordColor, range: range)
    }
    return text
  }

  /// Word highlights help when a line changed a little; when most of it
  /// changed they just box every token.
  static func usefulSpans(_ line: DiffLine) -> [WordSpan] {
    guard !line.spans.isEmpty, line.kind != .context else { return [] }
    let length = line.content.utf16.count
    guard length > 0 else { return [] }
    let covered = line.spans.reduce(0) { $0 + max(0, Int($1.end) - Int($1.start)) }
    return Double(covered) / Double(length) > 0.6 ? [] : line.spans
  }

  // MARK: Mouse

  override func mouseDown(with event: NSEvent) {
    window?.makeFirstResponder(superview?.superview as? NSTableView ?? enclosingTableView)
    guard let context, let file = context.files[path], !file.busy else { return }
    let point = convert(event.locationInWindow, from: nil)
    guard let side = side(at: point.x), let (index, line) = line(on: side) else { return }
    let cols = columns()
    let col = side == .new ? cols[cols.count - 1] : cols[0]
    let button = NSRect(x: col.gutters[0].minX + 1, y: 0, width: 19, height: context.metrics.lineHeight)
    if button.contains(point) {
      context.handler?.reviewOpenComposer(path: path, hunk: hunk, line: index)
      return
    }
    let inGutter = col.gutters.contains { $0.contains(point) } || col.marker.contains(point)
    if inGutter, line.kind != .context {
      context.handler?.reviewToggleLine(
        path: path, hunk: hunk, line: index, extend: event.modifierFlags.contains(.shift))
    } else {
      context.handler?.reviewFocusHunk(path: path, hunk: hunk)
    }
  }

  private var enclosingTableView: NSTableView? {
    var view: NSView? = superview
    while let current = view, !(current is NSTableView) { view = current.superview }
    return view as? NSTableView
  }

  override func menu(for event: NSEvent) -> NSMenu? {
    guard let context, let handler = context.handler else { return nil }
    let point = convert(event.locationInWindow, from: nil)
    guard let side = side(at: point.x), let (index, line) = line(on: side) else { return nil }
    let path = self.path
    let hunk = self.hunk
    let menu = NSMenu()
    menu.addItem(ClosureMenuItem(title: "Comment on This Line") {
      handler.reviewOpenComposer(path: path, hunk: hunk, line: index)
    })
    let caps = context.capabilities
    if caps.stage { menu.addItem(ClosureMenuItem(title: "Stage Hunk") { handler.reviewHunkAction(.stage, path: path, hunk: hunk) }) }
    if caps.unstage { menu.addItem(ClosureMenuItem(title: "Unstage Hunk") { handler.reviewHunkAction(.unstage, path: path, hunk: hunk) }) }
    if caps.revert { menu.addItem(ClosureMenuItem(title: "Revert Hunk…") { handler.reviewHunkAction(.revert, path: path, hunk: hunk) }) }
    menu.addItem(.separator())
    menu.addItem(ClosureMenuItem(title: "Copy Line") {
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(line.content, forType: .string)
    })
    let number = Int(line.newLineno ?? line.oldLineno ?? 1)
    menu.addItem(ClosureMenuItem(title: "Open File at This Line") {
      handler.reviewOpenFile(path: path, line: number, diff: false)
    })
    return menu
  }

  // MARK: Accessibility

  override func isAccessibilityElement() -> Bool { true }
  override func accessibilityRole() -> NSAccessibility.Role? { .staticText }
  override func accessibilityLabel() -> String? {
    let sides: [Side] = { if case .split = row { return [.old, .new] } else { return [.single] } }()
    return sides.compactMap { side -> String? in
      guard let (_, line) = line(on: side) else { return nil }
      let what = line.kind == .added ? "Added" : line.kind == .removed ? "Removed" : "Unchanged"
      let number = line.newLineno ?? line.oldLineno
      return "\(what) line \(number.map(String.init) ?? ""): \(line.content)"
    }.joined(separator: "; ")
  }
}

extension Array {
  fileprivate subscript(safe index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}

// MARK: - Hosted rows

/// A table cell showing SwiftUI content (headers, comments, notices).
final class ReviewHostedCellView: NSView {
  static let identifier = NSUserInterfaceItemIdentifier("ReviewHostedCellView")
  private let host = NSHostingView(rootView: AnyView(EmptyView()))

  override init(frame frameRect: NSRect) {
    super.init(frame: frameRect)
    host.translatesAutoresizingMaskIntoConstraints = false
    host.sizingOptions = []
    host.safeAreaRegions = []
    addSubview(host)
    NSLayoutConstraint.activate([
      host.topAnchor.constraint(equalTo: topAnchor),
      host.bottomAnchor.constraint(equalTo: bottomAnchor),
      host.leadingAnchor.constraint(equalTo: leadingAnchor),
      host.trailingAnchor.constraint(equalTo: trailingAnchor),
    ])
  }

  convenience init() { self.init(frame: .zero) }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  func set(_ view: AnyView) {
    host.rootView = view
  }
}
