import AppKit

// MARK: - Tab Bar Delegate

protocol CustomTabBarDelegate: AnyObject {
    func tabItemClicked(index: Int)
    func tabItemCloseClicked(index: Int)
    func tabItemContextMenu(index: Int) -> NSMenu?
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

    var title: String = "" { didSet { needsDisplay = true } }
    var icon: NSImage? { didSet { needsDisplay = true } }
    var isPinned: Bool = false { didSet { needsDisplay = true } }
    var index: Int = 0
    var isSelected: Bool = false { didSet { needsDisplay = true } }
    var isHovered: Bool = false { didSet { needsDisplay = true } }

    weak var tabDelegate: CustomTabBarDelegate?

    // Theme
    var bgHighlight: NSColor = .controlAccentColor
    var fgColor: NSColor = .labelColor
    var fgDarkColor: NSColor = .secondaryLabelColor
    var accentColor: NSColor = .systemCyan

    // Layout
    private let iconSize: CGFloat = 16
    private let closeSize: CGFloat = 12
    private let padding: CGFloat = 12
    private let iconTextGap: CGFloat = 6
    private let textCloseGap: CGFloat = 8

    private var trackingArea: NSTrackingArea?

    // MARK: Init

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 4
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not implemented")
    }

    // MARK: Drawing

    override func draw(_ dirtyRect: NSRect) {
        // Background
        let bgColor: NSColor
        if isSelected {
            bgColor = bgHighlight
        } else if isHovered {
            bgColor = bgHighlight.withAlphaComponent(0.4)
        } else {
            bgColor = .clear
        }
        bgColor.setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 4, yRadius: 4).fill()

        let textColor = (isSelected || isHovered) ? fgColor : fgDarkColor
        let y = bounds.midY

        var x = padding

        // Draw icon
        if let icon = icon {
            let iconRect = NSRect(
                x: x,
                y: y - iconSize / 2,
                width: iconSize,
                height: iconSize
            )
            if icon.isTemplate {
                // SF Symbol / template icon — tint in an isolated image context
                // so sourceAtop doesn't interact with the tab's background fill.
                let size = NSSize(width: iconSize, height: iconSize)
                let tinted = NSImage(size: size, flipped: false) { [textColor] rect in
                    icon.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1.0)
                    textColor.set()
                    rect.fill(using: .sourceAtop)
                    return true
                }
                tinted.draw(in: iconRect, from: .zero, operation: .sourceOver, fraction: 1.0)
            } else {
                // Pre-colored icon (themed file icons) — draw directly.
                icon.draw(in: iconRect, from: .zero, operation: .sourceOver, fraction: 1.0)
            }
            x += iconSize + iconTextGap
        }

        // Draw title
        let closeSpace = (isSelected || isHovered) ? (closeSize + textCloseGap + padding) : padding
        let maxTextWidth = bounds.width - x - closeSpace
        if maxTextWidth > 0 {
            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 12),
                .foregroundColor: textColor,
            ]
            let str = NSAttributedString(string: title, attributes: attrs)
            let textSize = str.size()
            let textRect = NSRect(
                x: x,
                y: y - textSize.height / 2,
                width: min(textSize.width, maxTextWidth),
                height: textSize.height
            )

            // Clip to prevent text overflow
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

        // Draw accent bar for selected tab
        if isSelected {
            accentColor.setFill()
            let barRect = NSRect(x: 6, y: 0, width: bounds.width - 12, height: 2)
            NSBezierPath(roundedRect: barRect, xRadius: 1, yRadius: 1).fill()
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

        tabDelegate?.tabItemClicked(index: index)
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

    private let tabHeight: CGFloat = 38
    private let tabMinWidth: CGFloat = 120

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
            heightAnchor.constraint(equalToConstant: tabHeight),
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
            containerView.frame = NSRect(x: 0, y: 0, width: max(1, scrollView.contentView.bounds.width), height: tabHeight)
            return
        }

        let spacing: CGFloat = 1
        let totalSpacing = CGFloat(max(0, tabViews.count - 1)) * spacing
        let rawWidth = (visibleWidth - totalSpacing) / CGFloat(tabViews.count)
        let tabWidth = max(rawWidth, tabMinWidth)
        let totalWidth = CGFloat(tabViews.count) * tabWidth + totalSpacing

        containerView.frame = NSRect(x: 0, y: 0, width: max(totalWidth, visibleWidth), height: tabHeight)

        for (i, tv) in tabViews.enumerated() {
            let x = CGFloat(i) * (tabWidth + spacing)
            tv.frame = NSRect(x: x, y: 0, width: tabWidth, height: tabHeight)
        }
    }

    // MARK: Private

    private func applyThemeToTab(_ tv: TabItemView, theme: Theme) {
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
