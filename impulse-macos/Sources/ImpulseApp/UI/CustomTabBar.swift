import AppKit

// MARK: - Tab Bar Delegate

protocol CustomTabBarDelegate: AnyObject {
    func tabItemClicked(index: Int)
    func tabItemCloseClicked(index: Int)
    func tabItemContextMenu(index: Int) -> NSMenu?
    func tabItemMoved(from sourceIndex: Int, to destinationIndex: Int)
}

// MARK: - Tab Item Data

struct TabItemData {
    let title: String
    let icon: NSImage?
    let isPinned: Bool
}

// MARK: - Tab Item View

/// A single tab drawn entirely with Core Graphics. Has zero subviews so
/// nothing can intercept mouse events. All interaction goes through mouseDown.
private final class TabItemView: NSView {

    // MARK: Data

    var title: String = "" {
        didSet {
            if title != oldValue { invalidateDrawingCache() }
            needsDisplay = true
        }
    }
    var icon: NSImage? {
        didSet {
            if icon !== oldValue { invalidateIconCache() }
            needsDisplay = true
        }
    }
    var isPinned: Bool = false { didSet { needsDisplay = true } }
    var index: Int = 0
    var isSelected: Bool = false { didSet { needsDisplay = true } }
    var isHovered: Bool = false { didSet { needsDisplay = true } }

    weak var tabDelegate: CustomTabBarDelegate?
    weak var barDelegate: CustomTabBar?

    // Theme
    var bgColor: NSColor = .controlBackgroundColor
    var bgDarkColor: NSColor = .windowBackgroundColor
    var bgHighlight: NSColor = .controlAccentColor
    var fgColor: NSColor = .labelColor
    var fgDarkColor: NSColor = .secondaryLabelColor
    var accentColor: NSColor = .systemCyan

    // Layout
    private let iconSize: CGFloat = 16
    private let closeSize: CGFloat = 12
    private let padding: CGFloat = 14
    private let iconTextGap: CGFloat = 6
    private let textCloseGap: CGFloat = 8

    private var trackingArea: NSTrackingArea?

    // Cached drawing state
    private var cachedAttributedTitle: NSAttributedString?
    private var cachedTitleSize: NSSize?
    private var cachedTintedIcon: NSImage?
    private var lastTitle: String?
    private var lastTintColor: NSColor?
    private var lastIcon: NSImage?

    private func invalidateDrawingCache() {
        cachedAttributedTitle = nil
        cachedTitleSize = nil
        lastTitle = nil
        lastTintColor = nil
    }

    private func invalidateIconCache() {
        cachedTintedIcon = nil
        lastIcon = nil
        lastTintColor = nil
    }

    // MARK: Init

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not implemented")
    }

    // MARK: Drawing

    override func draw(_ dirtyRect: NSRect) {
        let cornerRadius: CGFloat = 6

        // Tab shape: fully rounded rectangle
        let tabPath = NSBezierPath(roundedRect: bounds, xRadius: cornerRadius, yRadius: cornerRadius)

        // Background color
        let fillColor: NSColor
        if isSelected {
            fillColor = bgColor        // theme.bg — visual continuity with content
        } else if isHovered {
            fillColor = bgHighlight     // theme.bgHighlight
        } else {
            fillColor = bgDarkColor     // theme.bgDark
        }
        fillColor.setFill()
        tabPath.fill()

        // Text color: selected = accent (cyan), others = fgDark
        let textColor = isSelected ? accentColor : fgDarkColor
        let y = bounds.midY

        // Build or reuse cached attributed string
        if cachedAttributedTitle == nil || lastTitle != title || lastTintColor != textColor {
            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 13, weight: .medium),
                .foregroundColor: textColor,
            ]
            let str = NSAttributedString(string: title, attributes: attrs)
            cachedAttributedTitle = str
            cachedTitleSize = str.size()
            lastTitle = title
            lastTintColor = textColor
        }

        guard let str = cachedAttributedTitle, let textSize = cachedTitleSize else { return }

        let hasIcon = icon != nil
        let iconWidth = hasIcon ? (iconSize + iconTextGap) : 0
        let contentWidth = iconWidth + textSize.width

        // Always reserve space for the close button so icon+text don't shift on hover
        let closeReserved: CGFloat = closeSize + textCloseGap + padding
        let availableWidth = bounds.width - padding - closeReserved

        // Center the icon+text group within the available area
        let contentX: CGFloat
        if contentWidth <= availableWidth {
            contentX = padding + (availableWidth - contentWidth) / 2
        } else {
            contentX = padding  // fall back to left-aligned if it doesn't fit
        }

        var x = contentX

        // Draw icon
        if let icon = icon {
            let iconRect = NSRect(
                x: x,
                y: y - iconSize / 2,
                width: iconSize,
                height: iconSize
            )
            if icon.isTemplate {
                let tintColor = isSelected ? accentColor : fgDarkColor
                // Reuse cached tinted icon if valid
                if cachedTintedIcon == nil || lastIcon !== icon || lastTintColor != tintColor {
                    let size = NSSize(width: iconSize, height: iconSize)
                    cachedTintedIcon = NSImage(size: size, flipped: false) { [tintColor] rect in
                        icon.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1.0)
                        tintColor.set()
                        rect.fill(using: .sourceAtop)
                        return true
                    }
                    lastIcon = icon
                }
                cachedTintedIcon?.draw(in: iconRect, from: .zero, operation: .sourceOver, fraction: 1.0)
            } else {
                icon.draw(in: iconRect, from: .zero, operation: .sourceOver, fraction: 1.0)
            }
            x += iconSize + iconTextGap
        }

        // Draw title
        let maxTextWidth = bounds.width - x - closeReserved
        if maxTextWidth > 0 {
            let textRect = NSRect(
                x: x,
                y: y - textSize.height / 2,
                width: min(textSize.width, maxTextWidth),
                height: textSize.height
            )

            NSGraphicsContext.current?.saveGraphicsState()
            NSBezierPath(rect: NSRect(x: x, y: 0, width: maxTextWidth, height: bounds.height)).addClip()
            str.draw(in: textRect)
            NSGraphicsContext.current?.restoreGraphicsState()
        }

        // Draw close button (X) when hovered or selected
        if isSelected || isHovered {
            let closeColor = isSelected ? fgColor : fgDarkColor
            let closeX = bounds.width - padding - closeSize
            let closeY = y - closeSize / 2

            closeColor.setStroke()
            let path = NSBezierPath()
            path.lineWidth = 1.5
            path.lineCapStyle = .round
            let inset: CGFloat = 2
            path.move(to: NSPoint(x: closeX + inset, y: closeY + inset))
            path.line(to: NSPoint(x: closeX + closeSize - inset, y: closeY + closeSize - inset))
            path.move(to: NSPoint(x: closeX + closeSize - inset, y: closeY + inset))
            path.line(to: NSPoint(x: closeX + inset, y: closeY + closeSize - inset))
            path.stroke()
        }
    }

    // MARK: Hit Testing & Mouse

    override func hitTest(_ point: NSPoint) -> NSView? {
        let local = convert(point, from: superview)
        return bounds.contains(local) ? self : nil
    }

    override func mouseDown(with event: NSEvent) {
        let local = convert(event.locationInWindow, from: nil)

        // Check close button hit area
        if isSelected || isHovered {
            let closeX = bounds.width - padding - closeSize
            let closeY = bounds.midY - closeSize / 2
            let closeRect = NSRect(x: closeX - 4, y: closeY - 4, width: closeSize + 8, height: closeSize + 8)
            if closeRect.contains(local) {
                tabDelegate?.tabItemCloseClicked(index: index)
                return
            }
        }

        barDelegate?.beginPotentialDrag(tabIndex: index, event: event)
    }

    override func mouseDragged(with event: NSEvent) {
        barDelegate?.continueDrag(event: event)
    }

    override func mouseUp(with event: NSEvent) {
        barDelegate?.endDrag(event: event)
    }

    override func rightMouseDown(with event: NSEvent) {
        guard let menu = tabDelegate?.tabItemContextMenu(index: index) else { return }
        NSMenu.popUpContextMenu(menu, with: event, for: self)
    }

    // MARK: Tracking

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let existing = trackingArea {
            removeTrackingArea(existing)
        }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .cursorUpdate],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        trackingArea = area
    }

    override func mouseEntered(with event: NSEvent) {
        isHovered = true
    }

    override func mouseExited(with event: NSEvent) {
        isHovered = false
    }

    override func cursorUpdate(with event: NSEvent) {
        NSCursor.pointingHand.set()
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }

    override var mouseDownCanMoveWindow: Bool { false }
}

// MARK: - Non-Draggable Helpers

/// NSView that refuses to let mouseDown start a window drag.
/// Used inside the tab bar so clicks reach TabItemView.mouseDown
/// instead of being hijacked by the window's `isMovableByWindowBackground`.
private final class NonDraggableView: NSView {
    override var mouseDownCanMoveWindow: Bool { false }
}

private final class NonDraggableScrollView: NSScrollView {
    override var mouseDownCanMoveWindow: Bool { false }
}

// MARK: - Custom Tab Bar

/// Horizontally scrollable tab bar. Tabs are custom-drawn NSViews with no
/// subviews, ensuring reliable click handling everywhere.
final class CustomTabBar: NSView {

    weak var delegate: CustomTabBarDelegate?

    override var mouseDownCanMoveWindow: Bool { false }

    private let scrollView: NSScrollView = {
        let sv = NonDraggableScrollView()
        sv.translatesAutoresizingMaskIntoConstraints = false
        sv.hasHorizontalScroller = true
        sv.hasVerticalScroller = false
        sv.autohidesScrollers = true
        sv.drawsBackground = false
        sv.horizontalScrollElasticity = .allowed
        sv.borderType = .noBorder
        return sv
    }()

    private let containerView = NonDraggableView()
    private var tabViews: [TabItemView] = []
    private var selectedIndex: Int = -1
    private var currentTheme: Theme?

    private let tabBarHeight: CGFloat = 44
    private let tabItemHeight: CGFloat = 30
    private let tabMinWidth: CGFloat = 120
    private let tabTopPadding: CGFloat = 4
    private let tabBottomPadding: CGFloat = 10
    private let tabSpacing: CGFloat = 4
    private let tabHorizontalInset: CGFloat = 6

    // Drag-to-reorder state
    private struct DragState {
        let sourceIndex: Int
        let initialMouseX: CGFloat
        let initialTabX: CGFloat
        let tabWidth: CGFloat
        var isDragging: Bool = false
        var currentDropIndex: Int
    }
    private var dragState: DragState?
    private let dragThreshold: CGFloat = 5

    // MARK: Init

    override init(frame: NSRect) {
        super.init(frame: frame)
        setup()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setup()
    }

    private func setup() {
        setAccessibilityRole(.tabGroup)
        scrollView.documentView = containerView
        addSubview(scrollView)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: tabBarHeight),
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    // MARK: Public API

    func rebuild(tabs: [TabItemData], selectedIndex: Int, theme: Theme) {
        self.currentTheme = theme
        self.selectedIndex = selectedIndex

        for v in tabViews { v.removeFromSuperview() }
        tabViews.removeAll()

        for (i, data) in tabs.enumerated() {
            let tv = TabItemView(frame: .zero)
            tv.index = i
            tv.title = data.title
            tv.icon = data.icon
            tv.isPinned = data.isPinned
            tv.tabDelegate = delegate
            tv.barDelegate = self
            tv.isSelected = (i == selectedIndex)
            applyThemeToTab(tv, theme: theme)
            containerView.addSubview(tv)
            tabViews.append(tv)
        }

        needsLayout = true
    }

    func updateLabels(tabs: [TabItemData]) {
        for (i, data) in tabs.enumerated() where i < tabViews.count {
            tabViews[i].title = data.title
            tabViews[i].icon = data.icon
            tabViews[i].isPinned = data.isPinned
        }
    }

    func selectTab(index: Int) {
        guard index >= 0, index < tabViews.count else { return }
        if selectedIndex >= 0, selectedIndex < tabViews.count {
            tabViews[selectedIndex].isSelected = false
        }
        selectedIndex = index
        tabViews[index].isSelected = true
        scrollToSelected()
    }

    func applyTheme(_ theme: Theme) {
        currentTheme = theme
        for tv in tabViews {
            applyThemeToTab(tv, theme: theme)
        }
    }

    // MARK: Layout

    override func layout() {
        super.layout()

        let visibleWidth = scrollView.contentView.bounds.width
        guard visibleWidth > 0, !tabViews.isEmpty else {
            containerView.frame = NSRect(x: 0, y: 0, width: max(1, scrollView.contentView.bounds.width), height: tabBarHeight)
            return
        }

        let availableWidth = visibleWidth - 2 * tabHorizontalInset
        let totalSpacing = CGFloat(max(0, tabViews.count - 1)) * tabSpacing
        let rawWidth = (availableWidth - totalSpacing) / CGFloat(tabViews.count)
        let tabWidth = max(rawWidth, tabMinWidth)
        let totalWidth = CGFloat(tabViews.count) * tabWidth + totalSpacing + 2 * tabHorizontalInset

        containerView.frame = NSRect(x: 0, y: 0, width: max(totalWidth, visibleWidth), height: tabBarHeight)

        // Position tabs with more bottom margin (y=0 is bottom in AppKit)
        let tabY = tabBottomPadding
        for (i, tv) in tabViews.enumerated() {
            let x = tabHorizontalInset + CGFloat(i) * (tabWidth + tabSpacing)
            tv.frame = NSRect(x: x, y: tabY, width: tabWidth, height: tabItemHeight)
        }
    }

    // MARK: Drag-to-Reorder

    func beginPotentialDrag(tabIndex: Int, event: NSEvent) {
        guard tabIndex >= 0, tabIndex < tabViews.count else { return }

        let tv = tabViews[tabIndex]
        let mouseInContainer = containerView.convert(event.locationInWindow, from: nil)

        dragState = DragState(
            sourceIndex: tabIndex,
            initialMouseX: mouseInContainer.x,
            initialTabX: tv.frame.origin.x,
            tabWidth: tv.frame.width,
            currentDropIndex: tabIndex
        )

        // Immediately select the tab for click feedback.
        delegate?.tabItemClicked(index: tabIndex)
    }

    func continueDrag(event: NSEvent) {
        guard var state = dragState else { return }
        let mouseInContainer = containerView.convert(event.locationInWindow, from: nil)
        let deltaX = mouseInContainer.x - state.initialMouseX

        // Check if we've passed the drag threshold
        if !state.isDragging {
            if abs(deltaX) < dragThreshold { return }
            state.isDragging = true
            dragState = state

            // Elevate the dragged tab
            let draggedTab = tabViews[state.sourceIndex]
            draggedTab.layer?.zPosition = 10
            draggedTab.shadow = {
                let s = NSShadow()
                s.shadowBlurRadius = 8
                s.shadowOffset = NSSize(width: 0, height: -2)
                s.shadowColor = NSColor.black.withAlphaComponent(0.3)
                return s
            }()
        }

        let draggedTab = tabViews[state.sourceIndex]

        // Move the dragged tab to follow the cursor
        let newX = state.initialTabX + deltaX
        let clampedX = max(0, min(newX, containerView.bounds.width - state.tabWidth))
        draggedTab.frame.origin.x = clampedX

        // Calculate drop index from the center of the dragged tab
        let draggedCenter = clampedX + state.tabWidth / 2
        var dropIndex = state.sourceIndex
        for (i, tv) in tabViews.enumerated() where i != state.sourceIndex {
            let tabCenter = restingX(forIndex: i, excludingIndex: state.sourceIndex, state: state) + state.tabWidth / 2
            if state.sourceIndex < i {
                // Dragging right: shift left when we pass center
                if draggedCenter > tabCenter { dropIndex = i }
            } else {
                // Dragging left: shift right when we pass center
                if draggedCenter < tabCenter && dropIndex > i { dropIndex = i }
            }
        }

        // Animate other tabs to shift and make room
        if dropIndex != state.currentDropIndex {
            state.currentDropIndex = dropIndex
            dragState = state
        }

        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.15
            ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            for (i, tv) in tabViews.enumerated() where i != state.sourceIndex {
                let targetX = shiftedX(forIndex: i, dropIndex: dropIndex, state: state)
                tv.animator().frame.origin.x = targetX
            }
        }

        // Auto-scroll when near edges
        let mouseInScroll = scrollView.convert(event.locationInWindow, from: nil)
        let edgeMargin: CGFloat = 30
        let scrollAmount: CGFloat = 10
        var clipBounds = scrollView.contentView.bounds
        if mouseInScroll.x < edgeMargin {
            clipBounds.origin.x = max(0, clipBounds.origin.x - scrollAmount)
            scrollView.contentView.setBoundsOrigin(clipBounds.origin)
        } else if mouseInScroll.x > scrollView.bounds.width - edgeMargin {
            let maxX = containerView.frame.width - scrollView.bounds.width
            clipBounds.origin.x = min(maxX, clipBounds.origin.x + scrollAmount)
            scrollView.contentView.setBoundsOrigin(clipBounds.origin)
        }
    }

    func endDrag(event: NSEvent) {
        guard let state = dragState else { return }

        if state.isDragging {
            let draggedTab = tabViews[state.sourceIndex]
            draggedTab.layer?.zPosition = 0
            draggedTab.shadow = nil

            let fromIndex = state.sourceIndex
            let toIndex = state.currentDropIndex

            dragState = nil

            if fromIndex != toIndex {
                delegate?.tabItemMoved(from: fromIndex, to: toIndex)
            } else {
                // Animate back to resting position
                needsLayout = true
            }
        } else {
            // Below threshold — already handled as click in beginPotentialDrag.
            dragState = nil
        }
    }

    override func cancelOperation(_ sender: Any?) {
        guard let state = dragState, state.isDragging else { return }
        let draggedTab = tabViews[state.sourceIndex]
        draggedTab.layer?.zPosition = 0
        draggedTab.shadow = nil
        dragState = nil
        needsLayout = true
    }

    /// Returns the resting X position for a tab at `index`, as if the tab at
    /// `excludingIndex` were removed from the flow.
    private func restingX(forIndex index: Int, excludingIndex: Int, state: DragState) -> CGFloat {
        let slot = index > excludingIndex ? index - 1 : index
        return tabHorizontalInset + CGFloat(slot) * (state.tabWidth + tabSpacing)
    }

    /// Returns the target X for tab `index` while the tab at `sourceIndex` is
    /// being dragged toward `dropIndex`. Non-dragged tabs fill slots in order,
    /// skipping the gap reserved for the dragged tab.
    private func shiftedX(forIndex index: Int, dropIndex: Int, state: DragState) -> CGFloat {
        let src = state.sourceIndex

        // Build ordered list of non-dragged tab indices.
        var order = Array(0..<tabViews.count)
        order.remove(at: src)
        guard let pos = order.firstIndex(of: index) else { return 0 }

        // The gap slot in the reduced array where the dragged tab will land.
        let gapSlot = src < dropIndex ? dropIndex - 1 : dropIndex

        var slot = pos
        if slot >= gapSlot { slot += 1 }

        return tabHorizontalInset + CGFloat(slot) * (state.tabWidth + tabSpacing)
    }

    // MARK: Private

    private func applyThemeToTab(_ tv: TabItemView, theme: Theme) {
        tv.bgColor = theme.bg
        tv.bgDarkColor = theme.bgDark
        tv.bgHighlight = theme.bgHighlight
        tv.fgColor = theme.fg
        tv.fgDarkColor = theme.fgDark
        tv.accentColor = theme.cyan
        tv.needsDisplay = true
    }

    private func scrollToSelected() {
        guard selectedIndex >= 0, selectedIndex < tabViews.count else { return }
        let frame = tabViews[selectedIndex].frame
        DispatchQueue.main.async { [weak self] in
            self?.scrollView.contentView.scrollToVisible(frame)
        }
    }
}
