import AppKit
import os.log

// MARK: - Pointer Button

/// NSButton subclass that shows a pointing hand cursor on hover.
private final class PointerButton: NSButton {
    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: .pointingHand)
    }
}

// MARK: - Double-Click-to-Zoom Window

/// NSWindow subclass that restores double-click-to-zoom/minimize behavior
/// when using a transparent, hidden-title titlebar with fullSizeContentView.
private final class ImpulseWindow: NSWindow {
    override func mouseUp(with event: NSEvent) {
        super.mouseUp(with: event)
        guard event.clickCount == 2 else { return }
        // Only act on clicks in the titlebar region (above contentLayoutRect).
        let location = event.locationInWindow
        guard location.y > contentLayoutRect.maxY else { return }
        let action = UserDefaults.standard.string(forKey: "AppleActionOnDoubleClick") ?? "Maximize"
        switch action {
        case "Minimize": miniaturize(nil)
        case "Maximize": zoom(nil)
        default: break
        }
    }
}

// MARK: - Sidebar Toggle Button

/// Custom toggle button for the sidebar Files/Search mode switch.
/// Styled with rounded corners, theme-aware colors, and hover tracking.
private final class SidebarToggleButton: NSButton {

    var isActive: Bool = false { didSet { updateVisualState() } }

    private var isHovered: Bool = false { didSet { updateVisualState() } }
    private var trackingArea: NSTrackingArea?

    // Theme colors
    private var bgHighlight: NSColor = .controlAccentColor
    private var fgDarkColor: NSColor = .secondaryLabelColor
    private var accentColor: NSColor = .systemCyan

    init(title: String) {
        super.init(frame: .zero)
        self.title = title
        isBordered = false
        bezelStyle = .inline
        font = NSFont.systemFont(ofSize: 12, weight: .semibold)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        layer?.cornerRadius = 6
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    func applyTheme(bgHighlight: NSColor, fgDark: NSColor, cyan: NSColor) {
        self.bgHighlight = bgHighlight
        self.fgDarkColor = fgDark
        self.accentColor = cyan
        updateVisualState()
    }

    private func updateVisualState() {
        if isActive {
            layer?.backgroundColor = bgHighlight.cgColor
            contentTintColor = accentColor
        } else if isHovered {
            layer?.backgroundColor = bgHighlight.cgColor
            contentTintColor = fgDarkColor
        } else {
            layer?.backgroundColor = NSColor.clear.cgColor
            contentTintColor = fgDarkColor
        }
    }

    override var intrinsicContentSize: NSSize {
        let base = super.intrinsicContentSize
        return NSSize(width: base.width + 24, height: max(26, base.height + 8))
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let existing = trackingArea { removeTrackingArea(existing) }
        let area = NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        trackingArea = area
    }

    override func mouseEntered(with event: NSEvent) { isHovered = true }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: .pointingHand)
    }
}

// MARK: - Main Window Controller

/// The primary window controller for Impulse. Each window contains:
///   - An NSSplitView with a sidebar (file tree + search) on the left and a
///     content area on the right.
///   - The content area is itself split vertically: a TabManager-driven
///     content region on top and a StatusBar at the bottom.
///   - An NSToolbar hosting the tab bar (via TabManager).
///
/// Multiple windows can coexist; each owns its own TabManager and sidebar state.
final class MainWindowController: NSWindowController, NSWindowDelegate, NSSplitViewDelegate {

    // MARK: - Titlebar Buttons

    private let sidebarToggleButton: PointerButton = {
        let btn = PointerButton()
        btn.bezelStyle = .inline
        btn.isBordered = false
        btn.toolTip = "Toggle Sidebar"
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.imageScaling = .scaleProportionallyDown
        btn.setContentHuggingPriority(.defaultLow, for: .horizontal)
        btn.setContentHuggingPriority(.defaultLow, for: .vertical)
        btn.setContentCompressionResistancePriority(.defaultLow, for: .vertical)
        btn.wantsLayer = true
        btn.layer?.cornerRadius = 6
        return btn
    }()

    private let newTabButton: PointerButton = {
        let btn = PointerButton()
        btn.bezelStyle = .inline
        btn.isBordered = false
        btn.toolTip = "New Terminal Tab"
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.imageScaling = .scaleProportionallyDown
        btn.setContentHuggingPriority(.required, for: .horizontal)
        return btn
    }()

    // MARK: - State

    private var settings: Settings

    /// The shared Rust backend (impulse-ffi) instance.
    ///
    /// `internal` (not `private`) because it is accessed from the
    /// `MainWindowController+LSP` extension in a separate file.
    let core: ImpulseCore

    private(set) var theme: Theme

    private let tabBarContainer: NSView = {
        let v = NSView()
        v.wantsLayer = true
        v.translatesAutoresizingMaskIntoConstraints = false
        return v
    }()
    private let splitView = NSSplitView()
    private let sidebarContainer: NSView = {
        let v = NSView()
        v.wantsLayer = true
        return v
    }()
    private let contentContainer = NSView()

    /// The sidebar file tree using the NSOutlineView-based FileTreeView.
    private let fileTreeView: FileTreeView

    /// The sidebar search panel for project-wide search.
    private let searchPanel: SearchPanel

    /// Toggle buttons in the sidebar header to switch between files and search.
    private let filesToggle = SidebarToggleButton(title: "Files")
    private let searchToggle = SidebarToggleButton(title: "Search")

    /// Button to toggle visibility of hidden (dot) files in the file tree.
    private let toggleHiddenButton: PointerButton = {
        let btn = PointerButton()
        btn.bezelStyle = .texturedRounded
        btn.isBordered = false
        btn.toolTip = "Toggle Hidden Files"
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        return btn
    }()

    /// Button to collapse all expanded directories in the file tree.
    private let collapseAllButton: PointerButton = {
        let btn = PointerButton()
        btn.bezelStyle = .texturedRounded
        btn.isBordered = false
        btn.toolTip = "Collapse All"
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        return btn
    }()

    /// The project header row (toolbar buttons), hidden when Search is active.
    private let projectHeaderView: NSView = {
        let v = NSView()
        v.translatesAutoresizingMaskIntoConstraints = false
        return v
    }()

    /// Button to create a new file in the project root.
    private let newFileButton: PointerButton = {
        let btn = PointerButton()
        btn.bezelStyle = .texturedRounded
        btn.isBordered = false
        btn.toolTip = "New File"
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        return btn
    }()

    /// Button to create a new folder in the project root.
    private let newFolderButton: PointerButton = {
        let btn = PointerButton()
        btn.bezelStyle = .texturedRounded
        btn.isBordered = false
        btn.toolTip = "New Folder"
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        return btn
    }()

    /// Button to refresh the file tree.
    private let refreshButton: PointerButton = {
        let btn = PointerButton()
        btn.bezelStyle = .texturedRounded
        btn.isBordered = false
        btn.toolTip = "Refresh File Tree"
        btn.translatesAutoresizingMaskIntoConstraints = false
        btn.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        return btn
    }()

    /// Manages the tab bar and tab content lifecycle.
    let tabManager: TabManager

    /// The status bar at the bottom of the content area.
    private let statusBar = StatusBar()

    /// Terminal search bar (hidden by default, toggled with Cmd+F on terminal tabs).
    private let termSearchBar = NSView()
    private let termSearchField = NSSearchField()
    private var termSearchBarVisible = false
    private var termSearchHeightConstraint: NSLayoutConstraint?

    /// The command palette, lazily created on first use.
    private lazy var commandPalette: CommandPaletteWindow = {
        let palette = CommandPaletteWindow()
        if let delegate = NSApp.delegate as? AppDelegate {
            palette.registerCustomCommands(delegate.settings.customKeybindings)
        }
        return palette
    }()

    /// Whether the sidebar is currently visible.
    private var sidebarVisible: Bool

    /// Persisted sidebar width used to restore after collapse/expand.
    private var sidebarTargetWidth: CGFloat

    /// Local event monitor for custom keybinding interception.
    private var customKeybindingMonitor: Any?

    /// Observer tokens from NotificationCenter, removed on window close and deinit.
    private var notificationObservers: [Any] = []

    /// Dictionary mapping file paths to open editor tabs for O(1) lookup.
    ///
    /// `internal` (not `private`) because it is accessed from the
    /// `MainWindowController+LSP` extension in a separate file.
    var editorTabsByPath: [String: EditorTab] = [:]

    // MARK: File Tree State

    /// The root path currently displayed in the file tree. Used to avoid
    /// unnecessary rebuilds (which lose expansion state) when switching tabs.
    private var fileTreeRootPath: String = ""

    /// Cached file tree nodes keyed by root path for instant tab switching.
    private var fileTreeCache: [String: [FileTreeNode]] = [:]

    /// Tracks access order for LRU eviction of fileTreeCache entries.
    private var fileTreeCacheOrder: [String] = []

    /// Maximum number of entries in the file tree cache before LRU eviction.
    private let fileTreeCacheMaxSize = 20

    // MARK: Git State

    /// Cached git branch name for the current working directory.
    private var cachedGitBranch: String?
    /// The directory path for which `cachedGitBranch` was computed.
    private var cachedGitBranchDir: String = ""

    // MARK: LSP State (internal for MainWindowController+LSP extension)

    /// Per-URI document version counter for LSP.
    var lspDocVersions: [String: Int32] = [:]

    /// Tracks the latest completion request ID per URI for deduplication.
    var latestCompletionReq: [String: UInt64] = [:]

    /// Tracks the latest hover request ID per URI for deduplication.
    var latestHoverReq: [String: UInt64] = [:]

    /// In-flight completion work items per URI, cancelled when a newer request arrives.
    var completionWorkItems: [String: DispatchWorkItem] = [:]

    /// In-flight hover work items per URI, cancelled when a newer request arrives.
    var hoverWorkItems: [String: DispatchWorkItem] = [:]

    /// Tracks the latest formatting request ID per URI for deduplication.
    var latestFormattingReq: [String: UInt64] = [:]

    /// Tracks the latest signature help request ID per URI for deduplication.
    var latestSignatureHelpReq: [String: UInt64] = [:]

    /// Tracks the latest references request ID per URI for deduplication.
    var latestReferencesReq: [String: UInt64] = [:]

    /// Tracks the latest code action request ID per URI for deduplication.
    var latestCodeActionReq: [String: UInt64] = [:]

    /// Tracks the latest rename request ID per URI for deduplication.
    var latestRenameReq: [String: UInt64] = [:]

    /// In-flight formatting work items per URI, cancelled when a newer request arrives.
    var formattingWorkItems: [String: DispatchWorkItem] = [:]

    /// In-flight signature help work items per URI, cancelled when a newer request arrives.
    var signatureHelpWorkItems: [String: DispatchWorkItem] = [:]

    /// In-flight references work items per URI, cancelled when a newer request arrives.
    var referencesWorkItems: [String: DispatchWorkItem] = [:]

    /// In-flight code action work items per URI, cancelled when a newer request arrives.
    var codeActionWorkItems: [String: DispatchWorkItem] = [:]

    /// In-flight rename work items per URI, cancelled when a newer request arrives.
    var renameWorkItems: [String: DispatchWorkItem] = [:]

    /// In-flight prepare rename work items per URI, cancelled when a newer request arrives.
    var prepareRenameWorkItems: [String: DispatchWorkItem] = [:]

    /// Timer for polling LSP events (diagnostics, lifecycle).
    var lspPollTimer: Timer?

    /// Serial queue for dispatching blocking LSP FFI calls off the main thread.
    let lspQueue = DispatchQueue(label: "dev.impulse.lsp", qos: .userInitiated)

    /// Set of file URIs for which didOpen has been sent.
    var lspOpenFiles: Set<String> = []

    // MARK: - Initialization

    init(settings: Settings, theme: Theme, core: ImpulseCore) {
        self.settings = settings
        self.theme = theme
        self.core = core
        self.sidebarVisible = settings.sidebarVisible
        self.sidebarTargetWidth = CGFloat(settings.sidebarWidth)
        self.tabManager = TabManager(settings: settings, theme: theme, core: core)
        self.fileTreeView = FileTreeView()
        self.searchPanel = SearchPanel()

        let window = ImpulseWindow(
            contentRect: NSRect(
                x: 0, y: 0,
                width: CGFloat(settings.windowWidth),
                height: CGFloat(settings.windowHeight)
            ),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "Impulse"
        window.minSize = NSSize(width: 600, height: 400)
        window.center()
        window.isReleasedWhenClosed = false
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.appearance = NSAppearance(named: .darkAqua)
        window.backgroundColor = theme.bgDark

        super.init(window: window)
        window.delegate = self

        filesToggle.target = self
        filesToggle.action = #selector(filesToggleClicked(_:))
        filesToggle.isActive = true
        searchToggle.target = self
        searchToggle.action = #selector(searchToggleClicked(_:))
        filesToggle.applyTheme(bgHighlight: theme.bgHighlight, fgDark: theme.fgDark, cyan: theme.cyan)
        searchToggle.applyTheme(bgHighlight: theme.bgHighlight, fgDark: theme.fgDark, cyan: theme.cyan)

        // Set toolbar button icons from shared SVG icon cache, falling back to
        // SF Symbols if the cache hasn't loaded (e.g. missing bundle resources).
        sidebarToggleButton.image = tabManager.iconCache?.toolbarIcon(name: "toolbar-sidebar")
            ?? NSImage(systemSymbolName: "sidebar.left", accessibilityDescription: "Toggle Sidebar")
        newTabButton.image = tabManager.iconCache?.toolbarIcon(name: "toolbar-plus")
            ?? NSImage(systemSymbolName: "plus", accessibilityDescription: "New Tab")
        toggleHiddenButton.image = tabManager.iconCache?.toolbarIcon(name: "toolbar-eye-closed")
            ?? NSImage(systemSymbolName: "eye.slash", accessibilityDescription: "Toggle Hidden Files")
        collapseAllButton.image = tabManager.iconCache?.toolbarIcon(name: "toolbar-collapse")
            ?? NSImage(systemSymbolName: "arrow.up.left.and.arrow.down.right", accessibilityDescription: "Collapse All")
        newFileButton.image = tabManager.iconCache?.toolbarIcon(name: "toolbar-new-file")
            ?? NSImage(systemSymbolName: "doc.badge.plus", accessibilityDescription: "New File")
        newFolderButton.image = tabManager.iconCache?.toolbarIcon(name: "toolbar-new-folder")
            ?? NSImage(systemSymbolName: "folder.badge.plus", accessibilityDescription: "New Folder")
        refreshButton.image = tabManager.iconCache?.toolbarIcon(name: "toolbar-refresh")
            ?? NSImage(systemSymbolName: "arrow.clockwise", accessibilityDescription: "Refresh")

        sidebarToggleButton.target = self
        sidebarToggleButton.action = #selector(toggleSidebarAction(_:))
        sidebarToggleButton.contentTintColor = theme.fgDark

        newTabButton.target = self
        newTabButton.action = #selector(newTabAction(_:))
        newTabButton.contentTintColor = theme.fgDark

        toggleHiddenButton.target = self
        toggleHiddenButton.action = #selector(toggleHiddenAction(_:))
        toggleHiddenButton.contentTintColor = theme.fgDark

        collapseAllButton.target = self
        collapseAllButton.action = #selector(collapseAllAction(_:))
        collapseAllButton.contentTintColor = theme.fgDark

        newFileButton.target = self
        newFileButton.action = #selector(newFileAction(_:))
        newFileButton.contentTintColor = theme.fgDark

        newFolderButton.target = self
        newFolderButton.action = #selector(newFolderAction(_:))
        newFolderButton.contentTintColor = theme.fgDark

        refreshButton.target = self
        refreshButton.action = #selector(refreshTreeAction(_:))
        refreshButton.contentTintColor = theme.fgDark


        setupLayout()
        setupNotificationObservers()
        setupCustomKeybindingMonitor()

        // Apply initial theme to sidebar views.
        fileTreeView.applyTheme(theme)
        searchPanel.applyTheme(theme)

        // Set initial root path for the file tree and search panel.
        // Always start at home; the sidebar will update once the terminal's CWD
        // is detected via OSC 7.
        let rootPath = NSHomeDirectory()
        // Dispatch the initial tree build off the main thread to avoid blocking
        // startup with heavy filesystem + git status work.
        let showHidden = settings.sidebarShowHidden
        fileTreeRootPath = rootPath
        searchPanel.setRootPath(rootPath)
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let nodes = FileTreeNode.buildTree(rootPath: rootPath, showHidden: showHidden)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                guard self.fileTreeRootPath == rootPath else { return }
                self.fileTreeView.updateTree(nodes: nodes, rootPath: rootPath)
                self.fileTreeView.showHidden = showHidden
                self.fileTreeCacheInsert(key: rootPath, nodes: nodes)
            }
        }
        if settings.sidebarShowHidden {
            toggleHiddenButton.image = tabManager.iconCache?.toolbarIcon(name: "toolbar-eye-open")
                ?? NSImage(systemSymbolName: "eye", accessibilityDescription: "Toggle Hidden Files")
        }

        // Wire the tab close handler for save confirmation on unsaved editor tabs.
        tabManager.tabCloseHandler = { [weak self] index in
            self?.requestCloseTab(index: index)
        }

        // Open a default terminal tab.
        tabManager.addTerminalTab()

        // Start polling for asynchronous LSP events (diagnostics, etc.).
        startLspPolling()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    deinit {
        lspPollTimer?.invalidate()
        teardownCustomKeybindingMonitor()
        notificationObservers.forEach { NotificationCenter.default.removeObserver($0) }
    }

    // MARK: - Layout

    private func setupLayout() {
        guard let contentView = window?.contentView else { return }

        // Main split: sidebar | content
        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.delegate = self
        splitView.translatesAutoresizingMaskIntoConstraints = false

        // Sidebar: segmented control → project header → file tree / search panel
        sidebarContainer.translatesAutoresizingMaskIntoConstraints = false

        let sidebarModeStack = NSStackView(views: [filesToggle, searchToggle])
        sidebarModeStack.orientation = .horizontal
        sidebarModeStack.spacing = 6
        sidebarModeStack.distribution = .fillEqually
        sidebarModeStack.translatesAutoresizingMaskIntoConstraints = false

        // Project header: project name label + toolbar buttons (new file, new folder, hidden, refresh, collapse)
        let toolbarStack = NSStackView(views: [newFileButton, newFolderButton, toggleHiddenButton, refreshButton, collapseAllButton])
        toolbarStack.orientation = .horizontal
        toolbarStack.spacing = 2
        toolbarStack.translatesAutoresizingMaskIntoConstraints = false

        projectHeaderView.addSubview(toolbarStack)

        NSLayoutConstraint.activate([
            toolbarStack.trailingAnchor.constraint(equalTo: projectHeaderView.trailingAnchor, constant: -8),
            toolbarStack.centerYAnchor.constraint(equalTo: projectHeaderView.centerYAnchor),

            newFileButton.widthAnchor.constraint(equalToConstant: 24),
            newFolderButton.widthAnchor.constraint(equalToConstant: 24),
            toggleHiddenButton.widthAnchor.constraint(equalToConstant: 24),
            refreshButton.widthAnchor.constraint(equalToConstant: 24),
            collapseAllButton.widthAnchor.constraint(equalToConstant: 24),

            projectHeaderView.heightAnchor.constraint(equalToConstant: 24),
        ])

        fileTreeView.translatesAutoresizingMaskIntoConstraints = false
        searchPanel.translatesAutoresizingMaskIntoConstraints = false
        searchPanel.isHidden = true

        sidebarContainer.addSubview(sidebarModeStack)
        sidebarContainer.addSubview(projectHeaderView)
        sidebarContainer.addSubview(fileTreeView)
        sidebarContainer.addSubview(searchPanel)

        NSLayoutConstraint.activate([
            sidebarModeStack.topAnchor.constraint(equalTo: sidebarContainer.safeAreaLayoutGuide.topAnchor, constant: 10),
            sidebarModeStack.leadingAnchor.constraint(equalTo: sidebarContainer.leadingAnchor, constant: 10),
            sidebarModeStack.trailingAnchor.constraint(equalTo: sidebarContainer.trailingAnchor, constant: -10),

            projectHeaderView.topAnchor.constraint(equalTo: sidebarModeStack.bottomAnchor, constant: 6),
            projectHeaderView.leadingAnchor.constraint(equalTo: sidebarContainer.leadingAnchor),
            projectHeaderView.trailingAnchor.constraint(equalTo: sidebarContainer.trailingAnchor),

            fileTreeView.topAnchor.constraint(equalTo: projectHeaderView.bottomAnchor, constant: 4),
            fileTreeView.leadingAnchor.constraint(equalTo: sidebarContainer.leadingAnchor),
            fileTreeView.trailingAnchor.constraint(equalTo: sidebarContainer.trailingAnchor),
            fileTreeView.bottomAnchor.constraint(equalTo: sidebarContainer.bottomAnchor),

            searchPanel.topAnchor.constraint(equalTo: sidebarModeStack.bottomAnchor, constant: 6),
            searchPanel.leadingAnchor.constraint(equalTo: sidebarContainer.leadingAnchor),
            searchPanel.trailingAnchor.constraint(equalTo: sidebarContainer.trailingAnchor),
            searchPanel.bottomAnchor.constraint(equalTo: sidebarContainer.bottomAnchor),
        ])

        // Content area: terminal search bar + tab content + status bar
        contentContainer.translatesAutoresizingMaskIntoConstraints = false

        // Terminal search bar (hidden by default)
        setupTerminalSearchBar()
        contentContainer.addSubview(termSearchBar)

        let tabContentView = tabManager.contentView
        tabContentView.translatesAutoresizingMaskIntoConstraints = false

        statusBar.translatesAutoresizingMaskIntoConstraints = false

        contentContainer.addSubview(tabContentView)
        contentContainer.addSubview(statusBar)

        let searchHeight = termSearchBar.heightAnchor.constraint(equalToConstant: 0)
        termSearchHeightConstraint = searchHeight

        NSLayoutConstraint.activate([
            termSearchBar.topAnchor.constraint(equalTo: contentContainer.topAnchor),
            termSearchBar.leadingAnchor.constraint(equalTo: contentContainer.leadingAnchor),
            termSearchBar.trailingAnchor.constraint(equalTo: contentContainer.trailingAnchor),
            searchHeight,

            tabContentView.topAnchor.constraint(equalTo: termSearchBar.bottomAnchor),
            tabContentView.leadingAnchor.constraint(equalTo: contentContainer.leadingAnchor),
            tabContentView.trailingAnchor.constraint(equalTo: contentContainer.trailingAnchor),
            tabContentView.bottomAnchor.constraint(equalTo: statusBar.topAnchor),

            statusBar.leadingAnchor.constraint(equalTo: contentContainer.leadingAnchor),
            statusBar.trailingAnchor.constraint(equalTo: contentContainer.trailingAnchor),
            statusBar.bottomAnchor.constraint(equalTo: contentContainer.bottomAnchor),
        ])

        splitView.addArrangedSubview(sidebarContainer)
        splitView.addArrangedSubview(contentContainer)

        // Tab bar container: below the titlebar safe area (in normal content space)
        // so that mouse events are delivered through the standard responder chain
        // without interference from the titlebar's event routing.
        tabBarContainer.layer?.backgroundColor = theme.bgDark.cgColor
        let tabSegment = tabManager.tabBar

        tabBarContainer.addSubview(sidebarToggleButton)
        tabBarContainer.addSubview(tabSegment)
        tabBarContainer.addSubview(newTabButton)

        contentView.addSubview(tabBarContainer)
        contentView.addSubview(splitView)

        NSLayoutConstraint.activate([
            // Pin to safe area top (below the titlebar) — clicks work reliably here
            tabBarContainer.topAnchor.constraint(equalTo: contentView.safeAreaLayoutGuide.topAnchor),
            tabBarContainer.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            tabBarContainer.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            tabBarContainer.heightAnchor.constraint(equalToConstant: 48),

            // Sidebar toggle: left side, same height as tab items (34px)
            sidebarToggleButton.leadingAnchor.constraint(equalTo: tabBarContainer.leadingAnchor, constant: 8),
            sidebarToggleButton.bottomAnchor.constraint(equalTo: tabBarContainer.bottomAnchor, constant: -10),
            sidebarToggleButton.widthAnchor.constraint(equalToConstant: 34),
            sidebarToggleButton.heightAnchor.constraint(equalToConstant: 34),

            // Tab strip: fills between sidebar toggle and + button
            tabSegment.leadingAnchor.constraint(equalTo: sidebarToggleButton.trailingAnchor, constant: 6),
            tabSegment.topAnchor.constraint(equalTo: tabBarContainer.topAnchor),
            tabSegment.bottomAnchor.constraint(equalTo: tabBarContainer.bottomAnchor),
            tabSegment.trailingAnchor.constraint(equalTo: newTabButton.leadingAnchor, constant: -6),

            // New tab button: right side, vertically aligned with tab items
            newTabButton.trailingAnchor.constraint(equalTo: tabBarContainer.trailingAnchor, constant: -8),
            newTabButton.bottomAnchor.constraint(equalTo: tabBarContainer.bottomAnchor, constant: -13),
            newTabButton.widthAnchor.constraint(equalToConstant: 28),
            newTabButton.heightAnchor.constraint(equalToConstant: 28),

            splitView.topAnchor.constraint(equalTo: tabBarContainer.bottomAnchor),
            splitView.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            splitView.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])

        // Start with sidebar hidden; apply the saved state after layout
        // so the split view has valid geometry when setPosition is called.
        sidebarContainer.isHidden = true
        splitView.setPosition(0, ofDividerAt: 0)
        updateSidebarToggleIcon()

        // Defer sidebar restore so the window has laid out first.
        if sidebarVisible {
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.sidebarContainer.isHidden = false
                self.splitView.setPosition(self.sidebarTargetWidth, ofDividerAt: 0)
            }
        }

        // Content area has a minimum width so the sidebar cannot push it off screen.
        contentContainer.setContentHuggingPriority(.defaultLow, for: .horizontal)
    }

    // MARK: - NSSplitViewDelegate

    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposedMinimumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        if dividerIndex == 0 {
            return 150 // minimum sidebar width
        }
        return proposedMinimumPosition
    }

    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposedMaximumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        if dividerIndex == 0 {
            return min(proposedMaximumPosition, splitView.frame.width - 400)
        }
        return proposedMaximumPosition
    }

    func splitView(_ splitView: NSSplitView, canCollapseSubview subview: NSView) -> Bool {
        return subview === sidebarContainer
    }

    func splitViewDidResizeSubviews(_ notification: Notification) {
        if !sidebarContainer.isHidden && sidebarContainer.frame.width > 0 {
            sidebarTargetWidth = sidebarContainer.frame.width
        }
    }

    // MARK: - Actions

    @objc private func toggleSidebarAction(_ sender: Any?) {
        toggleSidebar()
    }

    @objc private func newTabAction(_ sender: Any?) {
        tabManager.addTerminalTab()
    }

    @objc private func toggleHiddenAction(_ sender: Any?) {
        fileTreeView.toggleHiddenFiles()
        // Update the button icon to reflect the current state.
        let svgName = fileTreeView.showHidden ? "toolbar-eye-open" : "toolbar-eye-closed"
        let sfName = fileTreeView.showHidden ? "eye" : "eye.slash"
        toggleHiddenButton.image = tabManager.iconCache?.toolbarIcon(name: svgName)
            ?? NSImage(systemSymbolName: sfName, accessibilityDescription: "Toggle Hidden Files")
        // Persist the setting.
        settings.sidebarShowHidden = fileTreeView.showHidden
        if let delegate = NSApp.delegate as? AppDelegate {
            delegate.settings.sidebarShowHidden = fileTreeView.showHidden
        }
    }

    @objc private func collapseAllAction(_ sender: Any?) {
        fileTreeView.collapseAll()
    }

    @objc private func refreshTreeAction(_ sender: Any?) {
        fileTreeView.refreshTree()
    }

    @objc private func newFileAction(_ sender: Any?) {
        let dirPath = fileTreeView.selectedDirectory
        guard !dirPath.isEmpty else { return }
        fileTreeView.showNameInputAlert(
            title: "New File",
            message: "Enter a name for the new file:",
            placeholder: "untitled",
            defaultValue: ""
        ) { [weak self] name in
            guard let self, !name.isEmpty, !name.contains("/") else { return }
            let fullPath = (dirPath as NSString).appendingPathComponent(name)
            let resolvedPath = (fullPath as NSString).standardizingPath
            let resolvedDir = (dirPath as NSString).standardizingPath
            guard resolvedPath.hasPrefix(resolvedDir) else { return }
            guard FileManager.default.createFile(atPath: fullPath, contents: nil) else {
                NSLog("MainWindow: failed to create file at \(fullPath)")
                return
            }
            self.fileTreeView.refreshTree()
        }
    }

    @objc private func newFolderAction(_ sender: Any?) {
        let dirPath = fileTreeView.selectedDirectory
        guard !dirPath.isEmpty else { return }
        fileTreeView.showNameInputAlert(
            title: "New Folder",
            message: "Enter a name for the new folder:",
            placeholder: "untitled-folder",
            defaultValue: ""
        ) { [weak self] name in
            guard let self, !name.isEmpty, !name.contains("/") else { return }
            let fullPath = (dirPath as NSString).appendingPathComponent(name)
            let resolvedPath = (fullPath as NSString).standardizingPath
            let resolvedDir = (dirPath as NSString).standardizingPath
            guard resolvedPath.hasPrefix(resolvedDir) else { return }
            do {
                try FileManager.default.createDirectory(atPath: fullPath,
                                                        withIntermediateDirectories: false)
            } catch {
                NSLog("MainWindow: failed to create folder at \(fullPath): \(error)")
                return
            }
            self.fileTreeView.refreshTree()
        }
    }


    @objc private func filesToggleClicked(_ sender: Any?) {
        filesToggle.isActive = true
        searchToggle.isActive = false
        fileTreeView.isHidden = false
        searchPanel.isHidden = true
        projectHeaderView.isHidden = false
    }

    @objc private func searchToggleClicked(_ sender: Any?) {
        filesToggle.isActive = false
        searchToggle.isActive = true
        fileTreeView.isHidden = true
        searchPanel.isHidden = false
        projectHeaderView.isHidden = true
        searchPanel.focus()
    }

    /// Toggles the sidebar visibility with animation.
    func toggleSidebar() {
        sidebarVisible.toggle()
        updateSidebarToggleIcon()
        // Unhide before the animation so the layout engine can process
        // the visibility change and the split view animates correctly.
        if sidebarVisible {
            sidebarContainer.isHidden = false
        }
        NSAnimationContext.runAnimationGroup({ [self] context in
            context.duration = 0.2
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            if sidebarVisible {
                splitView.animator().setPosition(sidebarTargetWidth, ofDividerAt: 0)
            } else {
                splitView.animator().setPosition(0, ofDividerAt: 0)
            }
        }, completionHandler: { [weak self] in
            guard let self else { return }
            if !self.sidebarVisible {
                self.sidebarContainer.isHidden = true
            }
        })
    }

    /// Update the sidebar toggle button icon to reflect the current state.
    private func updateSidebarToggleIcon() {
        let icon = tabManager.iconCache?.toolbarIcon(name: "toolbar-sidebar")
            ?? NSImage(systemSymbolName: "sidebar.left", accessibilityDescription: "Toggle Sidebar")
        icon?.isTemplate = true
        sidebarToggleButton.image = icon
        sidebarToggleButton.contentTintColor = sidebarVisible
            ? theme.cyan
            : theme.fgDark
        sidebarToggleButton.layer?.backgroundColor = sidebarVisible
            ? theme.bgHighlight.cgColor
            : NSColor.clear.cgColor
    }

    // MARK: - Custom Keybinding Monitor

    /// Installs a local event monitor that intercepts key-down events matching
    /// any configured custom keybinding. When a match is found the custom
    /// command is executed and the event is consumed.
    private func setupCustomKeybindingMonitor() {
        // Tear down any existing monitor first.
        if let existing = customKeybindingMonitor {
            NSEvent.removeMonitor(existing)
            customKeybindingMonitor = nil
        }

        customKeybindingMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, self.window?.isKeyWindow == true else { return event }

            for kb in self.settings.customKeybindings {
                let parsed = Keybindings.parseShortcut(kb.key)
                guard !parsed.keyEquivalent.isEmpty else { continue }
                if Keybindings.eventMatchesShortcut(event, keyEquivalent: parsed.keyEquivalent, modifierFlags: parsed.modifierFlags) {
                    self.executeCustomCommand(command: kb.command, args: kb.args)
                    return nil // consume the event
                }
            }

            return event
        }
    }

    /// Tears down the custom keybinding event monitor.
    private func teardownCustomKeybindingMonitor() {
        if let monitor = customKeybindingMonitor {
            NSEvent.removeMonitor(monitor)
            customKeybindingMonitor = nil
        }
    }

    // MARK: - Terminal Search Bar

    private func setupTerminalSearchBar() {
        termSearchBar.translatesAutoresizingMaskIntoConstraints = false
        termSearchBar.wantsLayer = true
        termSearchBar.isHidden = true

        termSearchField.translatesAutoresizingMaskIntoConstraints = false
        termSearchField.placeholderString = "Find in terminal..."
        termSearchField.sendsSearchStringImmediately = true
        termSearchField.target = self
        termSearchField.action = #selector(termSearchFieldChanged(_:))
        termSearchField.font = NSFont.systemFont(ofSize: 13)

        let prevButton = NSButton(title: "\u{25C0}", target: self, action: #selector(termSearchPrev(_:)))
        prevButton.translatesAutoresizingMaskIntoConstraints = false
        prevButton.bezelStyle = .texturedRounded
        prevButton.toolTip = "Previous Match"
        prevButton.setContentHuggingPriority(.defaultHigh, for: .horizontal)

        let nextButton = NSButton(title: "\u{25B6}", target: self, action: #selector(termSearchNext(_:)))
        nextButton.translatesAutoresizingMaskIntoConstraints = false
        nextButton.bezelStyle = .texturedRounded
        nextButton.toolTip = "Next Match"
        nextButton.setContentHuggingPriority(.defaultHigh, for: .horizontal)

        let closeButton = NSButton(title: "\u{2715}", target: self, action: #selector(termSearchClose(_:)))
        closeButton.translatesAutoresizingMaskIntoConstraints = false
        closeButton.bezelStyle = .texturedRounded
        closeButton.toolTip = "Close"
        closeButton.setContentHuggingPriority(.defaultHigh, for: .horizontal)

        termSearchBar.addSubview(termSearchField)
        termSearchBar.addSubview(prevButton)
        termSearchBar.addSubview(nextButton)
        termSearchBar.addSubview(closeButton)

        NSLayoutConstraint.activate([
            termSearchField.leadingAnchor.constraint(equalTo: termSearchBar.leadingAnchor, constant: 8),
            termSearchField.centerYAnchor.constraint(equalTo: termSearchBar.centerYAnchor),

            prevButton.leadingAnchor.constraint(equalTo: termSearchField.trailingAnchor, constant: 4),
            prevButton.centerYAnchor.constraint(equalTo: termSearchBar.centerYAnchor),

            nextButton.leadingAnchor.constraint(equalTo: prevButton.trailingAnchor, constant: 2),
            nextButton.centerYAnchor.constraint(equalTo: termSearchBar.centerYAnchor),

            closeButton.leadingAnchor.constraint(equalTo: nextButton.trailingAnchor, constant: 8),
            closeButton.trailingAnchor.constraint(lessThanOrEqualTo: termSearchBar.trailingAnchor, constant: -8),
            closeButton.centerYAnchor.constraint(equalTo: termSearchBar.centerYAnchor),
        ])
    }

    /// Toggles the terminal search bar visibility.
    func toggleTerminalSearch() {
        // Only allow terminal search on terminal tabs.
        guard tabManager.selectedIndex >= 0,
              tabManager.selectedIndex < tabManager.tabs.count,
              case .terminal = tabManager.tabs[tabManager.selectedIndex] else { return }

        termSearchBarVisible.toggle()
        if termSearchBarVisible {
            termSearchBar.isHidden = false
            termSearchHeightConstraint?.constant = 32
            window?.makeFirstResponder(termSearchField)
        } else {
            hideTerminalSearch()
        }
    }

    /// Hides the terminal search bar and clears the search state.
    private func hideTerminalSearch() {
        termSearchBarVisible = false
        termSearchBar.isHidden = true
        termSearchHeightConstraint?.constant = 0
        termSearchField.stringValue = ""

        // Return focus to the active terminal.
        if tabManager.selectedIndex >= 0,
           tabManager.selectedIndex < tabManager.tabs.count,
           case .terminal(let container) = tabManager.tabs[tabManager.selectedIndex],
           let terminal = container.activeTerminal {
            // TODO: SwiftTerm SearchService is a stub; implement terminal search when API is available.
            terminal.focus()
        }
    }

    @objc private func termSearchFieldChanged(_ sender: NSSearchField) {
        guard tabManager.selectedIndex >= 0,
              tabManager.selectedIndex < tabManager.tabs.count,
              case .terminal(let container) = tabManager.tabs[tabManager.selectedIndex],
              let terminal = container.activeTerminal else { return }

        // TODO: SwiftTerm SearchService is a stub; implement terminal search when API is available.
        let _ = sender.stringValue
    }

    @objc private func termSearchNext(_ sender: Any?) {
        guard tabManager.selectedIndex >= 0,
              tabManager.selectedIndex < tabManager.tabs.count,
              case .terminal(let container) = tabManager.tabs[tabManager.selectedIndex],
              let terminal = container.activeTerminal else { return }
        // TODO: SwiftTerm SearchService is a stub; implement terminal search when API is available.
        let _ = termSearchField.stringValue
    }

    @objc private func termSearchPrev(_ sender: Any?) {
        guard tabManager.selectedIndex >= 0,
              tabManager.selectedIndex < tabManager.tabs.count,
              case .terminal(let container) = tabManager.tabs[tabManager.selectedIndex],
              let terminal = container.activeTerminal else { return }
        // TODO: SwiftTerm SearchService is a stub; implement terminal search when API is available.
        let _ = termSearchField.stringValue
    }

    @objc private func termSearchClose(_ sender: Any?) {
        hideTerminalSearch()
    }

    /// Updates the status bar with information from the currently active tab.
    func updateStatusBar() {
        guard let tabInfo = tabManager.activeTabInfo else { return }

        if let shellName = tabInfo.shellName {
            let cwd = tabInfo.cwd ?? NSHomeDirectory()
            let branch = gitBranch(forDirectory: cwd)
            statusBar.updateForTerminal(
                cwd: cwd,
                gitBranch: branch,
                shellName: shellName
            )
        } else if let language = tabInfo.language {
            let filePath = tabInfo.cwd ?? ""
            let dir = (filePath as NSString).deletingLastPathComponent
            let branch = dir.isEmpty ? nil : gitBranch(forDirectory: dir)
            statusBar.updateForEditor(
                filePath: filePath,
                gitBranch: branch,
                cursorLine: (tabInfo.cursorLine ?? 0) + 1,
                cursorCol: (tabInfo.cursorCol ?? 0) + 1,
                language: language,
                tabWidth: settings.tabWidth,
                useSpaces: settings.useSpaces
            )
        }
    }

    /// Re-applies theme colors to all child views.
    func handleThemeChange(_ newTheme: Theme) {
        theme = newTheme

        // Window background — use bgDark so the titlebar blends with the tab bar
        window?.backgroundColor = newTheme.bgDark

        // Tab bar container and titlebar buttons
        tabBarContainer.layer?.backgroundColor = newTheme.bgDark.cgColor
        updateSidebarToggleIcon()
        newTabButton.contentTintColor = newTheme.fgDark

        // Sidebar background
        sidebarContainer.layer?.backgroundColor = newTheme.bgDark.cgColor

        // Sidebar toggle buttons
        filesToggle.applyTheme(bgHighlight: newTheme.bgHighlight, fgDark: newTheme.fgDark, cyan: newTheme.cyan)
        searchToggle.applyTheme(bgHighlight: newTheme.bgHighlight, fgDark: newTheme.fgDark, cyan: newTheme.cyan)

        // Sidebar toolbar buttons
        newFileButton.contentTintColor = newTheme.fgDark
        newFolderButton.contentTintColor = newTheme.fgDark
        toggleHiddenButton.contentTintColor = newTheme.fgDark
        refreshButton.contentTintColor = newTheme.fgDark
        collapseAllButton.contentTintColor = newTheme.fgDark

        // Sidebar views
        fileTreeView.applyTheme(newTheme)
        searchPanel.applyTheme(newTheme)

        // Status bar
        statusBar.applyTheme(newTheme)

        // Content background
        contentContainer.layer?.backgroundColor = newTheme.bg.cgColor

        // Terminal search bar
        termSearchBar.layer?.backgroundColor = newTheme.bgDark.cgColor

        // Tab manager (propagates to all tabs)
        tabManager.applyTheme(newTheme)
    }

    // MARK: - File Tree Cache (LRU)

    /// Inserts a key into the file tree cache, evicting the oldest entry if
    /// the cache exceeds `fileTreeCacheMaxSize`.
    private func fileTreeCacheInsert(key: String, nodes: [FileTreeNode]) {
        // Remove existing entry from order tracking if present.
        if let idx = fileTreeCacheOrder.firstIndex(of: key) {
            fileTreeCacheOrder.remove(at: idx)
        }
        fileTreeCacheOrder.append(key)
        fileTreeCache[key] = nodes

        // Evict oldest entries if over the limit.
        while fileTreeCacheOrder.count > fileTreeCacheMaxSize {
            let evicted = fileTreeCacheOrder.removeFirst()
            fileTreeCache.removeValue(forKey: evicted)
        }
    }

    /// Touches a cache key to mark it as recently used (moves to end of order).
    private func fileTreeCacheTouch(key: String) {
        if let idx = fileTreeCacheOrder.firstIndex(of: key) {
            fileTreeCacheOrder.remove(at: idx)
            fileTreeCacheOrder.append(key)
        }
    }

    // MARK: - Editor Tab Tracking

    /// Registers an editor tab in the path-to-tab dictionary.
    func trackEditorTab(_ editor: EditorTab, forPath path: String) {
        editorTabsByPath[path] = editor
    }

    /// Removes an editor tab from the path-to-tab dictionary.
    func untrackEditorTab(forPath path: String) {
        editorTabsByPath.removeValue(forKey: path)
    }

    // MARK: - Tab Close with Save Confirmation

    /// Closes a tab at the given index, showing a save confirmation dialog if
    /// the tab is an editor with unsaved changes. Used by both the Cmd+W
    /// shortcut and the tab bar close button / context menu.
    func requestCloseTab(index: Int) {
        guard index >= 0, index < tabManager.tabs.count else { return }

        if case .editor(let editor) = tabManager.tabs[index], editor.isModified {
            let filename = editor.filePath.map {
                ($0 as NSString).lastPathComponent
            } ?? "Untitled"

            let alert = NSAlert()
            alert.messageText = "Unsaved Changes"
            alert.informativeText = "\"\(filename)\" has unsaved changes. Close anyway?"
            alert.alertStyle = .warning
            alert.addButton(withTitle: "Save & Close")
            alert.addButton(withTitle: "Discard")
            alert.addButton(withTitle: "Cancel")

            guard let window = self.window else { return }
            alert.beginSheetModal(for: window) { [weak self] response in
                guard let self else { return }
                switch response {
                case .alertFirstButtonReturn:
                    // Save & Close
                    self.saveEditorTab(editor)
                    if let path = editor.filePath {
                        self.untrackEditorTab(forPath: path)
                    }
                    self.lspDidClose(editor: editor)
                    self.tabManager.closeTab(index: index)
                case .alertSecondButtonReturn:
                    // Discard
                    if let path = editor.filePath {
                        self.untrackEditorTab(forPath: path)
                    }
                    self.lspDidClose(editor: editor)
                    self.tabManager.closeTab(index: index)
                default:
                    // Cancel — do nothing
                    break
                }
            }
        } else {
            // Not an editor or not modified — close immediately.
            if case .editor(let editor) = tabManager.tabs[index] {
                if let path = editor.filePath {
                    untrackEditorTab(forPath: path)
                }
                lspDidClose(editor: editor)
            }
            tabManager.closeTab(index: index)
        }
    }

    // MARK: - Notification Observers

    private func setupNotificationObservers() {
        let nc = NotificationCenter.default

        notificationObservers.append(
            nc.addObserver(forName: .impulseToggleSidebar, object: nil, queue: .main) { [weak self] _ in
                guard self?.window?.isKeyWindow == true else { return }
                self?.toggleSidebar()
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseNewTerminalTab, object: nil, queue: .main) { [weak self] notification in
                guard let self, self.window?.isKeyWindow == true else { return }
                self.tabManager.addTerminalTab()
                // If a directory was specified (e.g. "Open in Terminal" from file tree),
                // navigate the new terminal to that directory.
                if let dir = notification.userInfo?["directory"] as? String,
                   self.tabManager.selectedIndex >= 0,
                   self.tabManager.selectedIndex < self.tabManager.tabs.count,
                   case .terminal(let container) = self.tabManager.tabs[self.tabManager.selectedIndex],
                   let terminal = container.activeTerminal {
                    let escapedDir: String
                    if dir.unicodeScalars.allSatisfy({ CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "/_.-")).contains($0) }) {
                        escapedDir = dir
                    } else {
                        escapedDir = "'" + dir.replacingOccurrences(of: "'", with: "'\\''") + "'"
                    }
                    terminal.sendCommand("cd \(escapedDir)")
                }
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseCloseTab, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                let index = self.tabManager.selectedIndex
                guard index >= 0, index < self.tabManager.tabs.count else { return }
                self.requestCloseTab(index: index)
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseReopenTab, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                self.tabManager.reopenLastClosedTab()
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseActiveTabDidChange, object: self.tabManager, queue: .main) { [weak self] _ in
                guard let self else { return }
                // Close the window when the last tab is closed (covers tab bar X button,
                // context menu "Close Tab", etc.).
                if self.tabManager.tabs.isEmpty {
                    self.window?.close()
                    return
                }
                self.updateStatusBar()
                // Rebuild the file tree when the active tab's directory context differs
                // from the current root. Terminal tabs use their CWD; editor tabs use
                // the parent directory of the open file.
                if self.tabManager.selectedIndex >= 0,
                   self.tabManager.selectedIndex < self.tabManager.tabs.count {
                    let tab = self.tabManager.tabs[self.tabManager.selectedIndex]
                    let dir: String?
                    switch tab {
                    case .terminal(let container):
                        dir = container.activeTerminal?.currentWorkingDirectory
                    case .editor(let editor):
                        // Restore the sidebar directory that was active when this
                        // editor tab was opened, so each editor keeps its context.
                        dir = editor.projectDirectory
                    case .imagePreview:
                        dir = nil
                    }
                    if let dir, !dir.isEmpty, dir != self.fileTreeRootPath {
                        // Save the current tree into the cache before switching away.
                        if !self.fileTreeRootPath.isEmpty {
                            self.fileTreeCacheInsert(key: self.fileTreeRootPath, nodes: self.fileTreeView.rootNodes)
                        }
                        self.fileTreeRootPath = dir
                        self.searchPanel.setRootPath(dir)

                        // If we have a cached tree for this directory, show it
                        // immediately and refresh in the background.
                        if let cached = self.fileTreeCache[dir] {
                            self.fileTreeView.updateTree(nodes: cached, rootPath: dir)
                            self.fileTreeCacheTouch(key: dir)
                        }

                        let showHidden = self.fileTreeView.showHidden
                        DispatchQueue.global(qos: .userInitiated).async {
                            let nodes = FileTreeNode.buildTree(rootPath: dir, showHidden: showHidden)
                            DispatchQueue.main.async { [weak self] in
                                guard let self else { return }
                                // Only update if we're still on this directory.
                                guard self.fileTreeRootPath == dir else { return }
                                self.fileTreeView.updateTree(nodes: nodes, rootPath: dir)
                                self.fileTreeCacheInsert(key: dir, nodes: nodes)
                            }
                        }
                    }
                }
                // Hide terminal search bar when switching away from a terminal tab.
                if self.termSearchBarVisible,
                   self.tabManager.selectedIndex >= 0,
                   self.tabManager.selectedIndex < self.tabManager.tabs.count {
                    if case .terminal = self.tabManager.tabs[self.tabManager.selectedIndex] {
                        // Still on a terminal tab — keep search bar visible.
                    } else {
                        self.hideTerminalSearch()
                    }
                }
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseOpenFile, object: nil, queue: .main) { [weak self] notification in
                guard let self, self.window?.isKeyWindow == true else { return }
                if let path = notification.userInfo?["path"] as? String {
                    self.tabManager.addEditorTab(path: path)
                    self.lspDidOpenIfNeeded(path: path)
                    // Navigate to specific line if provided (e.g. from search results).
                    if self.tabManager.selectedIndex >= 0,
                       self.tabManager.selectedIndex < self.tabManager.tabs.count,
                       case .editor(let editor) = self.tabManager.tabs[self.tabManager.selectedIndex] {
                        // Track the editor tab in the path dictionary.
                        self.trackEditorTab(editor, forPath: path)
                        // Record the sidebar directory that was active when this file was opened.
                        if editor.projectDirectory == nil {
                            editor.projectDirectory = self.fileTreeRootPath
                        }
                        if let line = notification.userInfo?["line"] as? UInt32 {
                            editor.goToPosition(line: line, column: 1)
                        }
                    }
                }
            }
        )
        // Apply git diff decorations once Monaco confirms it has processed
        // the OpenFile command and set up the model. This avoids the race
        // condition where decorations arrive before the model is ready.
        notificationObservers.append(
            nc.addObserver(forName: .editorFileOpened, object: nil, queue: .main) { [weak self] notification in
                guard let self else { return }
                if let editor = notification.object as? EditorTab {
                    // Send LSP didOpen now that the tab and Monaco model are ready.
                    if let path = editor.filePath {
                        self.lspDidOpenIfNeeded(path: path)
                    }
                    self.applyGitDiffDecorations(editor: editor)
                }
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseReloadEditorFile, object: nil, queue: .main) { [weak self] notification in
                guard let self, self.window?.isKeyWindow == true else { return }
                if let path = notification.userInfo?["path"] as? String {
                    // Find the open editor tab for this file and reload from disk
                    // off the main thread to avoid blocking UI.
                    if let editor = self.findEditorTab(forPath: path) {
                        let language = editor.language
                        DispatchQueue.global(qos: .userInitiated).async {
                            let content: String
                            do {
                                content = try String(contentsOfFile: path, encoding: .utf8)
                            } catch {
                                os_log(.error, "Failed to reload file '%{public}@': %{public}@",
                                       path, error.localizedDescription)
                                return
                            }
                            DispatchQueue.main.async { [weak editor] in
                                guard let editor else { return }
                                editor.openFile(path: path, content: content, language: language)
                            }
                        }
                    }
                }
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseSplitHorizontal, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                self.tabManager.splitTerminalHorizontally()
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseSplitVertical, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                self.tabManager.splitTerminalVertically()
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseFocusPrevSplit, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                guard self.tabManager.selectedIndex >= 0,
                      self.tabManager.selectedIndex < self.tabManager.tabs.count,
                      case .terminal(let container) = self.tabManager.tabs[self.tabManager.selectedIndex] else { return }
                container.focusPreviousSplit()
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseFocusNextSplit, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                guard self.tabManager.selectedIndex >= 0,
                      self.tabManager.selectedIndex < self.tabManager.tabs.count,
                      case .terminal(let container) = self.tabManager.tabs[self.tabManager.selectedIndex] else { return }
                container.focusNextSplit()
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseFindInProject, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                // Switch sidebar to search mode and show it if hidden.
                if !self.sidebarVisible {
                    self.toggleSidebar()
                }
                self.searchToggleClicked(nil)
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .terminalCwdChanged, object: nil, queue: .main) { [weak self] notification in
                guard let self else { return }
                guard let terminal = notification.object as? TerminalTab,
                      self.tabManager.ownsTerminal(terminal) else { return }
                if let dir = notification.userInfo?["directory"] as? String {
                    // Save current tree to cache before switching.
                    if !self.fileTreeRootPath.isEmpty {
                        self.fileTreeCacheInsert(key: self.fileTreeRootPath, nodes: self.fileTreeView.rootNodes)
                    }
                    self.fileTreeRootPath = dir
                    self.searchPanel.setRootPath(dir)
                    self.invalidateGitBranchCache()
                    // Immediate UI update with no branch yet.
                    self.statusBar.updateForTerminal(
                        cwd: dir,
                        gitBranch: nil,
                        shellName: ImpulseCore.getUserLoginShellName()
                    )

                    // Show cached tree instantly if available.
                    if let cached = self.fileTreeCache[dir] {
                        self.fileTreeView.updateTree(nodes: cached, rootPath: dir)
                        self.fileTreeCacheTouch(key: dir)
                    }

                    // Refresh from disk in the background.
                    let showHidden = self.fileTreeView.showHidden
                    DispatchQueue.global(qos: .userInitiated).async {
                        let nodes = FileTreeNode.buildTree(rootPath: dir, showHidden: showHidden)
                        let branch = ImpulseCore.gitBranch(path: dir)
                        DispatchQueue.main.async { [weak self] in
                            guard let self else { return }
                            guard self.fileTreeRootPath == dir else { return }
                            self.fileTreeView.updateTree(nodes: nodes, rootPath: dir)
                            self.fileTreeCacheInsert(key: dir, nodes: nodes)
                            self.statusBar.updateForTerminal(
                                cwd: dir,
                                gitBranch: branch,
                                shellName: ImpulseCore.getUserLoginShellName()
                            )
                        }
                    }
                }
            }
        )

        // Command palette
        notificationObservers.append(
            nc.addObserver(forName: .impulseShowCommandPalette, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true, let window = self.window else { return }
                self.commandPalette.show(relativeTo: window)
            }
        )

        // Save file — fired from menu Cmd+S or from EditorTab's SaveRequested event
        notificationObservers.append(
            nc.addObserver(forName: .impulseSaveFile, object: nil, queue: .main) { [weak self] notification in
                guard let self else { return }

                // If the notification came from an EditorTab (Monaco Cmd+S path),
                // save that specific editor directly — no key-window check needed
                // because the editor itself initiated the save.
                if let sourceEditor = notification.object as? EditorTab {
                    self.saveEditorTab(sourceEditor)
                    return
                }

                // Menu path: save the currently selected editor tab.
                guard self.window?.isKeyWindow == true else { return }
                guard self.tabManager.selectedIndex >= 0,
                      self.tabManager.selectedIndex < self.tabManager.tabs.count else { return }
                if case .editor(let editor) = self.tabManager.tabs[self.tabManager.selectedIndex] {
                    self.saveEditorTab(editor)
                }
            }
        )

        // Find — editor: Monaco find widget; terminal: search bar toggle
        notificationObservers.append(
            nc.addObserver(forName: .impulseFind, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                guard self.tabManager.selectedIndex >= 0,
                      self.tabManager.selectedIndex < self.tabManager.tabs.count else { return }
                switch self.tabManager.tabs[self.tabManager.selectedIndex] {
                case .editor(let editor):
                    editor.webView.evaluateJavaScript(
                        "editor.getAction('actions.find').run()",
                        completionHandler: nil
                    )
                case .terminal:
                    self.toggleTerminalSearch()
                case .imagePreview:
                    break
                }
            }
        )

        // Editor cursor position tracking for status bar + blame
        notificationObservers.append(
            nc.addObserver(forName: .editorCursorMoved, object: nil, queue: .main) { [weak self] notification in
                guard let self else { return }
                guard let line = notification.userInfo?["line"] as? UInt32,
                      let col = notification.userInfo?["column"] as? UInt32 else { return }
                // Only update if the notification came from the active editor tab
                guard self.tabManager.selectedIndex >= 0,
                      self.tabManager.selectedIndex < self.tabManager.tabs.count,
                      case .editor(let editor) = self.tabManager.tabs[self.tabManager.selectedIndex],
                      editor === notification.object as? EditorTab else { return }
                let filePath = editor.filePath ?? ""
                let dir = (filePath as NSString).deletingLastPathComponent
                let branch = dir.isEmpty ? nil : self.gitBranch(forDirectory: dir)
                let displayLine = Int(line) + 1
                self.statusBar.updateForEditor(
                    filePath: filePath,
                    gitBranch: branch,
                    cursorLine: displayLine,
                    cursorCol: Int(col) + 1,
                    language: editor.language,
                    tabWidth: self.settings.tabWidth,
                    useSpaces: self.settings.useSpaces
                )
                // Fetch blame asynchronously for the current line.
                self.fetchBlame(filePath: filePath, line: UInt32(displayLine))
            }
        )

        // Terminal title changed — update tab segment labels
        notificationObservers.append(
            nc.addObserver(forName: .terminalTitleChanged, object: nil, queue: .main) { [weak self] notification in
                guard let self else { return }
                guard let terminal = notification.object as? TerminalTab,
                      self.tabManager.ownsTerminal(terminal) else { return }
                self.tabManager.refreshSegmentLabels()
            }
        )

        // Terminal process terminated — close the tab or remove the split pane
        notificationObservers.append(
            nc.addObserver(forName: .terminalProcessTerminated, object: nil, queue: .main) { [weak self] notification in
                guard let self else { return }
                guard let terminalTab = notification.object as? TerminalTab,
                      self.tabManager.ownsTerminal(terminalTab) else { return }
                for (index, tab) in self.tabManager.tabs.enumerated() {
                    if case .terminal(let container) = tab {
                        guard let termIndex = container.terminals.firstIndex(where: { $0 === terminalTab }) else {
                            continue
                        }
                        if container.terminals.count == 1 {
                            self.tabManager.closeTab(index: index)
                        } else {
                            container.removeTerminal(at: termIndex)
                            self.tabManager.refreshSegmentLabels()
                        }
                        break
                    }
                }
            }
        )

        // Editor content changed — refresh tab labels and notify LSP
        notificationObservers.append(
            nc.addObserver(forName: .editorContentChanged, object: nil, queue: .main) { [weak self] notification in
                guard let self else { return }
                guard let editor = notification.object as? EditorTab,
                      self.tabManager.ownsEditor(editor) else { return }
                self.tabManager.refreshSegmentLabels()
                self.lspDidChange(editor: editor)
            }
        )

        // Editor focus changed — auto-save on focus loss if enabled
        notificationObservers.append(
            nc.addObserver(forName: .editorFocusChanged, object: nil, queue: .main) { [weak self] notification in
                guard let self, self.settings.autoSave else { return }
                guard let editor = notification.object as? EditorTab,
                      self.tabManager.ownsEditor(editor) else { return }
                guard let focused = notification.userInfo?["focused"] as? Bool, !focused else { return }
                guard editor.isModified else { return }
                self.saveEditorTab(editor)
            }
        )

        // Custom keybinding command execution
        notificationObservers.append(
            nc.addObserver(forName: Notification.Name("impulseCustomCommand"), object: nil, queue: .main) { [weak self] notification in
                guard let self, self.window?.isKeyWindow == true else { return }
                guard let command = notification.userInfo?["command"] as? String,
                      !command.isEmpty else { return }
                let args = notification.userInfo?["args"] as? [String] ?? []
                self.executeCustomCommand(command: command, args: args)
            }
        )

        // LSP: completion requested
        notificationObservers.append(
            nc.addObserver(forName: .editorCompletionRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let editor = notification.object as? EditorTab,
                      let requestId = notification.userInfo?["requestId"] as? UInt64,
                      let line = notification.userInfo?["line"] as? UInt32,
                      let character = notification.userInfo?["character"] as? UInt32 else { return }
                self.handleCompletionRequest(editor: editor, requestId: requestId, line: line, character: character)
            }
        )

        // LSP: hover requested
        notificationObservers.append(
            nc.addObserver(forName: .editorHoverRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let editor = notification.object as? EditorTab,
                      let requestId = notification.userInfo?["requestId"] as? UInt64,
                      let line = notification.userInfo?["line"] as? UInt32,
                      let character = notification.userInfo?["character"] as? UInt32 else { return }
                self.handleHoverRequest(editor: editor, requestId: requestId, line: line, character: character)
            }
        )

        // Go to line
        notificationObservers.append(
            nc.addObserver(forName: .impulseGoToLine, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                self.showGoToLineDialog()
            }
        )

        // Font size
        notificationObservers.append(
            nc.addObserver(forName: .impulseFontIncrease, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                self.changeFontSize(delta: 1)
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseFontDecrease, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                self.changeFontSize(delta: -1)
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseFontReset, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                self.resetFontSize()
            }
        )

        // Tab cycling
        notificationObservers.append(
            nc.addObserver(forName: .impulseNextTab, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                let count = self.tabManager.tabs.count
                guard count > 1 else { return }
                let next = (self.tabManager.selectedIndex + 1) % count
                self.tabManager.selectTab(index: next)
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulsePrevTab, object: nil, queue: .main) { [weak self] _ in
                guard let self, self.window?.isKeyWindow == true else { return }
                let count = self.tabManager.tabs.count
                guard count > 1 else { return }
                let prev = (self.tabManager.selectedIndex - 1 + count) % count
                self.tabManager.selectTab(index: prev)
            }
        )
        notificationObservers.append(
            nc.addObserver(forName: .impulseSelectTab, object: nil, queue: .main) { [weak self] notification in
                guard let self, self.window?.isKeyWindow == true else { return }
                guard let index = notification.userInfo?["index"] as? Int else { return }
                if index >= 0, index < self.tabManager.tabs.count {
                    self.tabManager.selectTab(index: index)
                }
            }
        )

        // Settings changed (from SettingsWindow)
        notificationObservers.append(
            nc.addObserver(forName: .impulseSettingsDidChange, object: nil, queue: .main) { [weak self] notification in
                guard let self, let newSettings = notification.object as? Settings else { return }
                self.settings = newSettings
                self.tabManager.settings = newSettings
                self.applyAllSettings()
                // Rebuild custom keybinding monitor so new/changed bindings take effect.
                self.setupCustomKeybindingMonitor()
            }
        )

        // LSP: go-to-definition requested
        notificationObservers.append(
            nc.addObserver(forName: .editorDefinitionRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let editor = notification.object as? EditorTab,
                      let requestId = notification.userInfo?["requestId"] as? UInt64,
                      let line = notification.userInfo?["line"] as? UInt32,
                      let character = notification.userInfo?["character"] as? UInt32 else { return }
                self.handleDefinitionRequest(editor: editor, requestId: requestId, line: line, character: character)
            }
        )

        // Monaco: cross-file navigation (fired by registerEditorOpener on actual click)
        notificationObservers.append(
            nc.addObserver(forName: .editorOpenFileRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let uri = notification.userInfo?["uri"] as? String,
                      let line = notification.userInfo?["line"] as? UInt32,
                      let character = notification.userInfo?["character"] as? UInt32 else { return }
                self.handleOpenFileRequested(uri: uri, line: line, character: character)
            }
        )

        // LSP: formatting requested
        notificationObservers.append(
            nc.addObserver(forName: .editorFormattingRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let editor = notification.object as? EditorTab,
                      let requestId = notification.userInfo?["requestId"] as? UInt64,
                      let tabSize = notification.userInfo?["tabSize"] as? UInt32,
                      let insertSpaces = notification.userInfo?["insertSpaces"] as? Bool else { return }
                self.handleFormattingRequest(editor: editor, requestId: requestId, tabSize: tabSize, insertSpaces: insertSpaces)
            }
        )

        // LSP: signature help requested
        notificationObservers.append(
            nc.addObserver(forName: .editorSignatureHelpRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let editor = notification.object as? EditorTab,
                      let requestId = notification.userInfo?["requestId"] as? UInt64,
                      let line = notification.userInfo?["line"] as? UInt32,
                      let character = notification.userInfo?["character"] as? UInt32 else { return }
                self.handleSignatureHelpRequest(editor: editor, requestId: requestId, line: line, character: character)
            }
        )

        // LSP: references requested
        notificationObservers.append(
            nc.addObserver(forName: .editorReferencesRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let editor = notification.object as? EditorTab,
                      let requestId = notification.userInfo?["requestId"] as? UInt64,
                      let line = notification.userInfo?["line"] as? UInt32,
                      let character = notification.userInfo?["character"] as? UInt32 else { return }
                self.handleReferencesRequest(editor: editor, requestId: requestId, line: line, character: character)
            }
        )

        // LSP: code action requested
        notificationObservers.append(
            nc.addObserver(forName: .editorCodeActionRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let editor = notification.object as? EditorTab,
                      let requestId = notification.userInfo?["requestId"] as? UInt64,
                      let startLine = notification.userInfo?["startLine"] as? UInt32,
                      let startColumn = notification.userInfo?["startColumn"] as? UInt32,
                      let endLine = notification.userInfo?["endLine"] as? UInt32,
                      let endColumn = notification.userInfo?["endColumn"] as? UInt32 else { return }
                let diagnostics = notification.userInfo?["diagnostics"] as? [[String: Any]] ?? []
                self.handleCodeActionRequest(editor: editor, requestId: requestId, startLine: startLine, startColumn: startColumn, endLine: endLine, endColumn: endColumn, diagnostics: diagnostics)
            }
        )

        // LSP: rename requested
        notificationObservers.append(
            nc.addObserver(forName: .editorRenameRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let editor = notification.object as? EditorTab,
                      let requestId = notification.userInfo?["requestId"] as? UInt64,
                      let line = notification.userInfo?["line"] as? UInt32,
                      let character = notification.userInfo?["character"] as? UInt32,
                      let newName = notification.userInfo?["newName"] as? String else { return }
                self.handleRenameRequest(editor: editor, requestId: requestId, line: line, character: character, newName: newName)
            }
        )

        // LSP: prepare rename requested
        notificationObservers.append(
            nc.addObserver(forName: .editorPrepareRenameRequested, object: nil, queue: .main) { [weak self] notification in
                guard let self,
                      let editor = notification.object as? EditorTab,
                      let requestId = notification.userInfo?["requestId"] as? UInt64,
                      let line = notification.userInfo?["line"] as? UInt32,
                      let character = notification.userInfo?["character"] as? UInt32 else { return }
                self.handlePrepareRenameRequest(editor: editor, requestId: requestId, line: line, character: character)
            }
        )
    }

    // MARK: - Save Pipeline

    /// Unified save pipeline for editor tabs. Handles:
    /// 1. Format on save (if configured)
    /// 2. Actual file save
    /// 3. LSP didSave notification
    /// 4. Commands on save
    /// 5. Git diff decoration refresh
    private func saveEditorTab(_ editor: EditorTab) {
        guard let path = editor.filePath else { return }

        // Fetch the latest content from Monaco (content changes are debounced
        // in JS, so the Swift property may be stale when saving via menu Cmd+S).
        editor.fetchContentAndSave { [weak self, weak editor] success in
            guard let self, let editor, success else { return }

            // Format on save — find applicable formatter
            let formatter = self.resolveFormatOnSave(forPath: path)
            if let fmt = formatter, !fmt.command.isEmpty {
                self.runExternalCommand(command: fmt.command, args: fmt.args, cwd: (path as NSString).deletingLastPathComponent) { [weak self, weak editor] in
                    guard let self, let editor else { return }
                    // Reload the file after formatting off the main thread.
                    let language = editor.language
                    let currentContent = editor.content
                    DispatchQueue.global(qos: .userInitiated).async {
                        let newContent: String
                        do {
                            newContent = try String(contentsOfFile: path, encoding: .utf8)
                        } catch {
                            os_log(.error, "Failed to reload file after formatting '%{public}@': %{public}@",
                                   path, error.localizedDescription)
                            DispatchQueue.main.async { [weak self, weak editor] in
                                guard let self, let editor else { return }
                                self.postSaveActions(editor: editor, path: path)
                            }
                            return
                        }
                        guard newContent != currentContent else {
                            DispatchQueue.main.async { [weak self, weak editor] in
                                guard let self, let editor else { return }
                                self.postSaveActions(editor: editor, path: path)
                            }
                            return
                        }
                        DispatchQueue.main.async { [weak self, weak editor] in
                            guard let self, let editor else { return }
                            editor.openFile(path: path, content: newContent, language: language)
                            self.postSaveActions(editor: editor, path: path)
                        }
                    }
                }
            } else {
                self.postSaveActions(editor: editor, path: path)
            }
        }
    }

    /// Actions that run after saving and optional formatting.
    private func postSaveActions(editor: EditorTab, path: String) {
        tabManager.refreshSegmentLabels()
        lspDidSave(editor: editor)
        invalidateGitBranchCache()
        applyGitDiffDecorations(editor: editor)
        fileTreeView.refreshGitStatus()

        // Commands on save: run any matching commands
        for cmd in settings.commandsOnSave {
            guard !cmd.command.isEmpty else { continue }
            guard Settings.matchesFilePattern(path, pattern: cmd.filePattern) else { continue }
            let cwd = (path as NSString).deletingLastPathComponent
            if cmd.reloadFile {
                let language = editor.language
                runExternalCommand(command: cmd.command, args: cmd.args, cwd: cwd) { [weak editor] in
                    guard let editor else { return }
                    let currentContent = editor.content
                    DispatchQueue.global(qos: .userInitiated).async {
                        let newContent: String
                        do {
                            newContent = try String(contentsOfFile: path, encoding: .utf8)
                        } catch {
                            os_log(.error, "Failed to reload file after command-on-save '%{public}@': %{public}@",
                                   path, error.localizedDescription)
                            return
                        }
                        guard newContent != currentContent else { return }
                        DispatchQueue.main.async { [weak editor] in
                            guard let editor else { return }
                            editor.openFile(path: path, content: newContent, language: language)
                        }
                    }
                }
            } else {
                runExternalCommand(command: cmd.command, args: cmd.args, cwd: cwd, completion: nil)
            }
        }
    }

    /// Resolves the `FormatOnSave` configuration for a file path, checking
    /// file-type overrides first, then falling back to the global setting.
    private func resolveFormatOnSave(forPath path: String) -> FormatOnSave? {
        // Check file-type-specific overrides first
        for override_ in settings.fileTypeOverrides {
            if Settings.matchesFilePattern(path, pattern: override_.pattern),
               let fmt = override_.formatOnSave, !fmt.command.isEmpty {
                return fmt
            }
        }
        return nil
    }

    /// Runs an external command asynchronously. Calls `completion` on the main
    /// thread when the process finishes.
    private func runExternalCommand(command: String, args: [String], cwd: String,
                                     completion: (() -> Void)?) {
        DispatchQueue.global(qos: .userInitiated).async {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
            process.arguments = [command] + args
            process.currentDirectoryURL = URL(fileURLWithPath: cwd)
            process.standardOutput = FileHandle.nullDevice
            process.standardError = FileHandle.nullDevice

            do {
                try process.run()
                process.waitUntilExit()
            } catch {
                NSLog("Failed to run command '\(command)': \(error)")
            }

            if let completion = completion {
                DispatchQueue.main.async { completion() }
            }
        }
    }

    // MARK: - Custom Command Execution

    /// Executes a custom keybinding command by opening a new terminal tab
    /// with the command running in it, using the active tab's working directory.
    private func executeCustomCommand(command: String, args: [String]) {
        // Shell-escape arguments that contain special characters.
        let escapedArgs = args.map { arg -> String in
            if arg.rangeOfCharacter(from: CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "/_.-")).inverted) != nil {
                return "'" + arg.replacingOccurrences(of: "'", with: "'\\''") + "'"
            }
            return arg
        }
        let escapedCommand: String
        if command.rangeOfCharacter(from: CharacterSet.alphanumerics
            .union(CharacterSet(charactersIn: "/_.-")).inverted) != nil {
            escapedCommand = "'" + command.replacingOccurrences(of: "'", with: "'\\''") + "'"
        } else {
            escapedCommand = command
        }
        let fullCommand = ([escapedCommand] + escapedArgs).joined(separator: " ")

        // Get the CWD from the active tab (terminal CWD or editor file's parent)
        let cwd = getActiveCwd()

        tabManager.addTerminalTab(directory: cwd)
        if tabManager.selectedIndex >= 0,
           tabManager.selectedIndex < tabManager.tabs.count,
           case .terminal(let container) = tabManager.tabs[tabManager.selectedIndex],
           let terminal = container.activeTerminal {
            terminal.sendCommand(fullCommand)
        }
    }

    /// Returns the current working directory from the active tab:
    /// terminal CWD, or the parent directory of the active editor file.
    private func getActiveCwd() -> String? {
        guard tabManager.selectedIndex >= 0,
              tabManager.selectedIndex < tabManager.tabs.count else { return nil }
        switch tabManager.tabs[tabManager.selectedIndex] {
        case .terminal(let container):
            if let cwd = container.activeTerminal?.currentWorkingDirectory, !cwd.isEmpty {
                return cwd
            }
        case .editor(let editor):
            if let path = editor.filePath {
                return (path as NSString).deletingLastPathComponent
            }
        case .imagePreview(let path, _):
            return (path as NSString).deletingLastPathComponent
        }
        return nil
    }

    // MARK: - Go to Line

    /// Shows a dialog asking for a line number and navigates the active editor to it.
    private func showGoToLineDialog() {
        guard tabManager.selectedIndex >= 0,
              tabManager.selectedIndex < tabManager.tabs.count,
              case .editor(let editor) = tabManager.tabs[tabManager.selectedIndex] else { return }

        let alert = NSAlert()
        alert.messageText = "Go to Line"
        alert.informativeText = "Enter a line number:"
        alert.alertStyle = .informational
        alert.addButton(withTitle: "Go")
        alert.addButton(withTitle: "Cancel")

        let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 200, height: 24))
        input.placeholderString = "Line number"
        alert.accessoryView = input
        alert.window.initialFirstResponder = input

        let response = alert.runModal()
        guard response == .alertFirstButtonReturn else { return }

        let text = input.stringValue.trimmingCharacters(in: .whitespaces)
        guard let lineNumber = UInt32(text), lineNumber > 0 else { return }
        editor.goToPosition(line: lineNumber, column: 1)
        editor.focus()
    }

    // MARK: - Font Size

    /// Changes both editor and terminal font sizes by the given delta.
    private func changeFontSize(delta: Int) {
        let newEditorSize = max(6, min(72, settings.fontSize + delta))
        let newTerminalSize = max(6, min(72, settings.terminalFontSize + delta))

        settings.fontSize = newEditorSize
        settings.terminalFontSize = newTerminalSize
        syncSettingsToAppDelegate()
        applyFontSizeToAllTabs()
    }

    /// Resets font sizes to defaults (14 for both editor and terminal).
    private func resetFontSize() {
        settings.fontSize = 14
        settings.terminalFontSize = 14
        syncSettingsToAppDelegate()
        applyFontSizeToAllTabs()
    }

    /// Copies the current window's settings back to the AppDelegate so they
    /// persist across quit/relaunch.
    private func syncSettingsToAppDelegate() {
        if let delegate = NSApp.delegate as? AppDelegate {
            delegate.settings.fontSize = settings.fontSize
            delegate.settings.terminalFontSize = settings.terminalFontSize
        }
    }

    /// Applies the current font size settings to all open tabs.
    private func applyFontSizeToAllTabs() {
        // Build EditorOptions with updated font size from our local settings,
        // since TabManager's settings copy may not reflect the change yet.
        let editorOptions = EditorOptions(
            fontSize: UInt32(settings.fontSize),
            fontFamily: settings.fontFamily
        )
        let termSettings = TerminalSettings(
            terminalFontSize: settings.terminalFontSize,
            terminalFontFamily: settings.terminalFontFamily,
            terminalCursorShape: settings.terminalCursorShape,
            terminalCursorBlink: settings.terminalCursorBlink,
            terminalScrollback: settings.terminalScrollback,
            lastDirectory: settings.lastDirectory,
            terminalCopyOnSelect: settings.terminalCopyOnSelect
        )

        for tab in tabManager.tabs {
            switch tab {
            case .editor(let editor):
                editor.applySettings(editorOptions)
            case .terminal(let container):
                container.applySettings(settings: termSettings)
            case .imagePreview:
                break
            }
        }
    }

    // MARK: - Apply All Settings

    /// Re-applies all settings to every open tab. Called when settings change
    /// via the preferences window.
    private func applyAllSettings() {
        let editorOptions = EditorOptions(
            fontSize: UInt32(settings.fontSize),
            fontFamily: settings.fontFamily,
            tabSize: UInt32(settings.tabWidth),
            insertSpaces: settings.useSpaces,
            wordWrap: settings.wordWrap ? "on" : "off",
            minimapEnabled: settings.minimapEnabled,
            lineNumbers: settings.showLineNumbers ? "on" : "off",
            renderWhitespace: settings.renderWhitespace,
            renderLineHighlight: settings.highlightCurrentLine ? "line" : "none",
            rulers: settings.showRightMargin ? [UInt32(settings.rightMarginPosition)] : [],
            stickyScroll: settings.stickyScroll,
            bracketPairColorization: settings.bracketPairColorization,
            indentGuides: settings.indentGuides,
            fontLigatures: settings.fontLigatures,
            folding: settings.folding,
            scrollBeyondLastLine: settings.scrollBeyondLastLine,
            smoothScrolling: settings.smoothScrolling,
            cursorStyle: settings.editorCursorStyle,
            cursorBlinking: settings.editorCursorBlinking,
            lineHeight: settings.editorLineHeight > 0 ? UInt32(settings.editorLineHeight) : nil,
            autoClosingBrackets: settings.editorAutoClosingBrackets
        )
        let termSettings = TerminalSettings(
            terminalFontSize: settings.terminalFontSize,
            terminalFontFamily: settings.terminalFontFamily,
            terminalCursorShape: settings.terminalCursorShape,
            terminalCursorBlink: settings.terminalCursorBlink,
            terminalScrollback: settings.terminalScrollback,
            lastDirectory: settings.lastDirectory,
            terminalCopyOnSelect: settings.terminalCopyOnSelect
        )

        for tab in tabManager.tabs {
            switch tab {
            case .editor(let editor):
                editor.applySettings(editorOptions)
            case .terminal(let container):
                container.applySettings(settings: termSettings)
            case .imagePreview:
                break
            }
        }

        // Re-apply sidebar show-hidden preference.
        if fileTreeView.showHidden != settings.sidebarShowHidden {
            fileTreeView.showHidden = settings.sidebarShowHidden
            let svgName = settings.sidebarShowHidden ? "toolbar-eye-open" : "toolbar-eye-closed"
            let sfName = settings.sidebarShowHidden ? "eye" : "eye.slash"
            toggleHiddenButton.image = tabManager.iconCache?.toolbarIcon(name: svgName)
                ?? NSImage(systemSymbolName: sfName, accessibilityDescription: "Toggle Hidden Files")
            if !fileTreeRootPath.isEmpty {
                fileTreeView.setRootPath(fileTreeRootPath)
            }
        }
    }

    // MARK: - Git Branch Cache

    /// Returns the git branch for a directory, using a cache to avoid
    /// redundant calls (e.g. on every cursor move).
    private func gitBranch(forDirectory dir: String) -> String? {
        if dir == cachedGitBranchDir {
            return cachedGitBranch
        }
        cachedGitBranchDir = dir
        cachedGitBranch = ImpulseCore.gitBranch(path: dir)
        return cachedGitBranch
    }

    /// Invalidates the git branch cache (e.g. after a save or CWD change).
    private func invalidateGitBranchCache() {
        cachedGitBranchDir = ""
        cachedGitBranch = nil
    }

    // MARK: - Git Blame

    /// Fetches blame info for a line asynchronously and updates the status bar.
    private func fetchBlame(filePath: String, line: UInt32) {
        guard !filePath.isEmpty else {
            statusBar.clearBlame()
            return
        }
        DispatchQueue.global(qos: .utility).async {
            let blame = ImpulseCore.gitBlame(filePath: filePath, line: line)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if let blame = blame,
                   let author = blame["author"],
                   let date = blame["date"],
                   let summary = blame["summary"] {
                    self.statusBar.updateBlame("\(author) \u{2022} \(date) \u{2022} \(summary)")
                } else {
                    self.statusBar.clearBlame()
                }
            }
        }
    }

    // MARK: - Git Diff Decorations

    /// Applies git diff gutter decorations to an editor tab by querying
    /// the FFI bridge for diff markers.
    private func applyGitDiffDecorations(editor: EditorTab) {
        guard let path = editor.filePath else { return }
        DispatchQueue.global(qos: .utility).async {
            let markers = ImpulseCore.gitDiffMarkers(filePath: path)
            DispatchQueue.main.async {
                editor.applyDiffDecorations(markers)
            }
        }
    }

    // MARK: - NSWindowDelegate

    func windowDidResize(_ notification: Notification) {
    }

    func windowDidBecomeKey(_ notification: Notification) {
    }

    func windowWillClose(_ notification: Notification) {
        stopLspPolling()
        teardownCustomKeybindingMonitor()

        // Remove all notification observers.
        notificationObservers.forEach { NotificationCenter.default.removeObserver($0) }
        notificationObservers.removeAll()

        // Clean up all remaining tabs (kill terminal processes, tear down
        // editor WebViews) so resources are freed immediately.
        tabManager.cleanupAllTabs()

        // Clear editor tab tracking.
        editorTabsByPath.removeAll()

        // Persist sidebar state back to AppDelegate settings.
        if let delegate = NSApp.delegate as? AppDelegate {
            delegate.settings.sidebarVisible = sidebarVisible
            delegate.settings.sidebarWidth = Int(sidebarTargetWidth)
        }

        (NSApp.delegate as? AppDelegate)?.windowControllerDidClose(self)
    }
}
