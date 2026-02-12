import AppKit

// MARK: - Tab Bar Delegate

/// Delegate protocol for handling tab bar interactions.
protocol CustomTabBarDelegate: AnyObject {
    func tabItemClicked(index: Int)
    func tabItemCloseClicked(index: Int)
    func tabItemContextMenu(index: Int) -> NSMenu?
}

// MARK: - Tab Item Data

/// Lightweight value type describing a single tab for display purposes.
struct TabItemData {
    let title: String
    let icon: NSImage?
    let isPinned: Bool
}

// MARK: - Tab Item View

/// A single tab in the custom tab bar. Displays an icon, title, and close
/// button with hover and selection states.
private final class TabItemView: NSView {

    // MARK: Properties

    private let iconView: NSImageView = {
        let iv = NSImageView()
        iv.translatesAutoresizingMaskIntoConstraints = false
        iv.imageScaling = .scaleProportionallyUpOrDown
        return iv
    }()

    private let pinIcon: NSImageView = {
        let iv = NSImageView()
        iv.translatesAutoresizingMaskIntoConstraints = false
        iv.imageScaling = .scaleProportionallyUpOrDown
        iv.image = NSImage(systemSymbolName: "pin.fill", accessibilityDescription: "Pinned")
        iv.isHidden = true
        return iv
    }()

    private let titleField: NSTextField = {
        let tf = NSTextField(labelWithString: "")
        tf.translatesAutoresizingMaskIntoConstraints = false
        tf.font = NSFont.systemFont(ofSize: 12)
        tf.lineBreakMode = .byTruncatingTail
        tf.isEditable = false
        tf.isSelectable = false
        tf.drawsBackground = false
        tf.isBezeled = false
        return tf
    }()

    private let closeButton: NSButton = {
        let btn = NSButton()
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.bezelStyle = .inline
        btn.isBordered = false
        btn.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: "Close Tab")
        btn.imageScaling = .scaleProportionallyDown
        btn.setContentHuggingPriority(.required, for: .horizontal)
        btn.toolTip = "Close Tab"
        btn.alphaValue = 0
        return btn
    }()

    private let accentBar: NSView = {
        let v = NSView()
        v.translatesAutoresizingMaskIntoConstraints = false
        v.wantsLayer = true
        v.isHidden = true
        return v
    }()

    var index: Int = 0
    var isTabSelected: Bool = false { didSet { needsDisplay = true; updateVisualState() } }
    private var isHovered: Bool = false { didSet { needsDisplay = true; updateVisualState() } }
    private var trackingArea: NSTrackingArea?

    weak var delegate: CustomTabBarDelegate?

    // Theme colors (set via applyTheme)
    private var bgHighlight: NSColor = .controlAccentColor
    private var fgColor: NSColor = .labelColor
    private var fgDarkColor: NSColor = .secondaryLabelColor
    private var accentColor: NSColor = .systemCyan

    // MARK: Initialization

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setup()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setup()
    }

    private func setup() {
        wantsLayer = true
        layer?.cornerRadius = 4
        layer?.maskedCorners = [.layerMinXMaxYCorner, .layerMaxXMaxYCorner,
                                .layerMinXMinYCorner, .layerMaxXMinYCorner]

        setAccessibilityRole(.radioButton)

        addSubview(pinIcon)
        addSubview(iconView)
        addSubview(titleField)
        addSubview(closeButton)
        addSubview(accentBar)

        closeButton.target = self
        closeButton.action = #selector(closeClicked(_:))

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 34),
            widthAnchor.constraint(greaterThanOrEqualToConstant: 80),
            widthAnchor.constraint(lessThanOrEqualToConstant: 200),

            pinIcon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            pinIcon.centerYAnchor.constraint(equalTo: centerYAnchor),
            pinIcon.widthAnchor.constraint(equalToConstant: 10),
            pinIcon.heightAnchor.constraint(equalToConstant: 10),

            iconView.leadingAnchor.constraint(equalTo: pinIcon.trailingAnchor, constant: 4),
            iconView.centerYAnchor.constraint(equalTo: centerYAnchor),
            iconView.widthAnchor.constraint(equalToConstant: 16),
            iconView.heightAnchor.constraint(equalToConstant: 16),

            titleField.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: 6),
            titleField.centerYAnchor.constraint(equalTo: centerYAnchor),

            closeButton.leadingAnchor.constraint(greaterThanOrEqualTo: titleField.trailingAnchor, constant: 4),
            closeButton.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            closeButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            closeButton.widthAnchor.constraint(equalToConstant: 14),
            closeButton.heightAnchor.constraint(equalToConstant: 14),

            accentBar.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            accentBar.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            accentBar.bottomAnchor.constraint(equalTo: bottomAnchor),
            accentBar.heightAnchor.constraint(equalToConstant: 2),
        ])

        // Compress title, not close button
        titleField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        titleField.setContentHuggingPriority(.defaultLow, for: .horizontal)
    }

    // MARK: Configuration

    func configure(data: TabItemData, theme: Theme) {
        titleField.stringValue = data.title
        iconView.image = data.icon
        pinIcon.isHidden = !data.isPinned
        applyTheme(theme)
    }

    func updateTitle(_ title: String) {
        titleField.stringValue = title
    }

    func applyTheme(_ theme: Theme) {
        bgHighlight = theme.bgHighlight
        fgColor = theme.fg
        fgDarkColor = theme.fgDark
        accentColor = theme.cyan
        accentBar.layer?.backgroundColor = accentColor.cgColor
        pinIcon.contentTintColor = fgDarkColor
        updateVisualState()
    }

    // MARK: Visual State

    private func updateVisualState() {
        if isTabSelected {
            layer?.backgroundColor = bgHighlight.cgColor
            titleField.textColor = fgColor
            iconView.contentTintColor = fgColor
            closeButton.contentTintColor = fgColor
            closeButton.alphaValue = 1
            accentBar.isHidden = false
        } else if isHovered {
            layer?.backgroundColor = bgHighlight.withAlphaComponent(0.4).cgColor
            titleField.textColor = fgColor
            iconView.contentTintColor = fgColor
            closeButton.contentTintColor = fgDarkColor
            closeButton.alphaValue = 1
            accentBar.isHidden = true
        } else {
            layer?.backgroundColor = NSColor.clear.cgColor
            titleField.textColor = fgDarkColor
            iconView.contentTintColor = fgDarkColor
            closeButton.alphaValue = 0
            accentBar.isHidden = true
        }
    }

    // MARK: Tracking

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let existing = trackingArea {
            removeTrackingArea(existing)
        }
        let area = NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
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

    override func mouseDown(with event: NSEvent) {
        delegate?.tabItemClicked(index: index)
    }

    override func rightMouseDown(with event: NSEvent) {
        guard let menu = delegate?.tabItemContextMenu(index: index) else { return }
        NSMenu.popUpContextMenu(menu, with: event, for: self)
    }

    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: .pointingHand)
    }

    @objc private func closeClicked(_ sender: Any?) {
        delegate?.tabItemCloseClicked(index: index)
    }
}

// MARK: - Custom Tab Bar

/// A horizontally scrollable tab bar that replaces the NSSegmentedControl.
/// Each tab is a `TabItemView` hosted in a horizontal stack view inside
/// an NSScrollView.
final class CustomTabBar: NSView {

    // MARK: Properties

    private let scrollView: NSScrollView = {
        let sv = NSScrollView()
        sv.translatesAutoresizingMaskIntoConstraints = false
        sv.hasHorizontalScroller = true
        sv.hasVerticalScroller = false
        sv.autohidesScrollers = true
        sv.drawsBackground = false
        sv.horizontalScrollElasticity = .allowed
        sv.borderType = .noBorder
        return sv
    }()

    private let stackView: NSStackView = {
        let sv = NSStackView()
        sv.translatesAutoresizingMaskIntoConstraints = false
        sv.orientation = .horizontal
        sv.spacing = 1
        sv.alignment = .bottom
        sv.distribution = .fillProportionally
        return sv
    }()

    private var tabViews: [TabItemView] = []
    private var selectedIndex: Int = -1
    private var currentTheme: Theme?

    weak var delegate: CustomTabBarDelegate?

    // MARK: Initialization

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setup()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setup()
    }

    private func setup() {
        setAccessibilityRole(.tabGroup)

        scrollView.documentView = stackView
        addSubview(scrollView)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),

            stackView.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            stackView.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            stackView.bottomAnchor.constraint(equalTo: scrollView.contentView.bottomAnchor),
            // Don't pin trailing — let the stack grow beyond the clip view.
        ])
    }

    // MARK: Public API

    /// Full rebuild of all tabs from the given data array.
    func rebuild(tabs: [TabItemData], selectedIndex: Int, theme: Theme) {
        self.currentTheme = theme
        self.selectedIndex = selectedIndex

        // Remove old tab views.
        for view in tabViews {
            stackView.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        tabViews.removeAll()

        // Build new tab views.
        for (i, data) in tabs.enumerated() {
            let tabView = TabItemView()
            tabView.index = i
            tabView.delegate = delegate
            tabView.configure(data: data, theme: theme)
            tabView.isTabSelected = (i == selectedIndex)
            stackView.addArrangedSubview(tabView)
            tabViews.append(tabView)
        }

        scrollToSelected()
    }

    /// Update only the titles of existing tabs (no full rebuild).
    func updateLabels(tabs: [TabItemData]) {
        guard let theme = currentTheme else { return }
        for (i, data) in tabs.enumerated() where i < tabViews.count {
            tabViews[i].updateTitle(data.title)
            tabViews[i].configure(data: data, theme: theme)
        }
    }

    /// Change which tab is selected (visual state only).
    func selectTab(index: Int) {
        guard index >= 0, index < tabViews.count else { return }
        let oldIndex = selectedIndex
        selectedIndex = index

        if oldIndex >= 0, oldIndex < tabViews.count {
            tabViews[oldIndex].isTabSelected = false
        }
        tabViews[index].isTabSelected = true
        scrollToSelected()
    }

    /// Re-apply theme colors to all tab views.
    func applyTheme(_ theme: Theme) {
        currentTheme = theme
        for tabView in tabViews {
            tabView.applyTheme(theme)
        }
    }

    // MARK: Private

    /// Scrolls the scroll view to make the selected tab visible.
    private func scrollToSelected() {
        guard selectedIndex >= 0, selectedIndex < tabViews.count else { return }
        let tabView = tabViews[selectedIndex]
        // Use async to let layout complete first.
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.scrollView.contentView.scrollToVisible(tabView.frame)
        }
    }
}
