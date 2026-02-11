import AppKit

// MARK: - Main Window Controller

/// The primary window controller for Impulse. Each window contains:
///   - An NSSplitView with a sidebar (file tree + search) on the left and a
///     content area on the right.
///   - The content area is itself split vertically: a TabManager-driven
///     content region on top and a StatusBar at the bottom.
///   - An NSToolbar hosting the tab bar (via TabManager).
///
/// Multiple windows can coexist; each owns its own TabManager and sidebar state.
final class MainWindowController: NSWindowController, NSWindowDelegate, NSToolbarDelegate, NSSplitViewDelegate {

    // MARK: - Toolbar Identifiers

    private static let toolbarIdentifier = NSToolbar.Identifier("ImpulseMainToolbar")
    private static let tabBarItemIdentifier = NSToolbarItem.Identifier("TabBar")
    private static let sidebarToggleItemIdentifier = NSToolbarItem.Identifier("SidebarToggle")
    private static let newTabItemIdentifier = NSToolbarItem.Identifier("NewTab")

    // MARK: - State

    private let settings: Settings
    private let core: ImpulseCore
    private(set) var theme: Theme

    private let splitView = NSSplitView()
    private let sidebarContainer = NSView()
    private let contentContainer = NSView()

    /// The sidebar file tree using the NSOutlineView-based FileTreeView.
    private let fileTreeView: FileTreeView

    /// The sidebar search panel for project-wide search.
    private let searchPanel: SearchPanel

    /// Segmented control in the sidebar header to switch between files and search.
    private let sidebarModeControl: NSSegmentedControl

    /// Manages the tab bar and tab content lifecycle.
    let tabManager: TabManager

    /// The status bar at the bottom of the content area.
    private let statusBar = StatusBar()

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
        self.sidebarModeControl = NSSegmentedControl(
            labels: ["Files", "Search"],
            trackingMode: .selectOne,
            target: nil,
            action: nil
        )

        let window = NSWindow(
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

        super.init(window: window)
        window.delegate = self

        sidebarModeControl.target = self
        sidebarModeControl.action = #selector(sidebarModeChanged(_:))
        sidebarModeControl.selectedSegment = 0

        setupToolbar()
        setupLayout()
        setupNotificationObservers()

        // Set initial root path for the file tree and search panel.
        let rootPath: String
        if !settings.lastDirectory.isEmpty,
           FileManager.default.fileExists(atPath: settings.lastDirectory) {
            rootPath = settings.lastDirectory
        } else {
            rootPath = NSHomeDirectory()
        }
        fileTreeView.setRootPath(rootPath)
        searchPanel.setRootPath(rootPath)
        fileTreeView.showHidden = settings.sidebarShowHidden

        // Open a default terminal tab.
        tabManager.addTerminalTab()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: - Layout

    private func setupLayout() {
        guard let contentView = window?.contentView else { return }

        // Main split: sidebar | content
        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.delegate = self
        splitView.translatesAutoresizingMaskIntoConstraints = false

        // Sidebar: header (mode switch) + file tree / search panel
        sidebarContainer.translatesAutoresizingMaskIntoConstraints = false
        sidebarContainer.wantsLayer = true

        sidebarModeControl.translatesAutoresizingMaskIntoConstraints = false
        sidebarModeControl.segmentStyle = .texturedRounded
        sidebarModeControl.controlSize = .small

        fileTreeView.translatesAutoresizingMaskIntoConstraints = false
        searchPanel.translatesAutoresizingMaskIntoConstraints = false
        searchPanel.isHidden = true

        sidebarContainer.addSubview(sidebarModeControl)
        sidebarContainer.addSubview(fileTreeView)
        sidebarContainer.addSubview(searchPanel)

        NSLayoutConstraint.activate([
            sidebarModeControl.topAnchor.constraint(equalTo: sidebarContainer.topAnchor, constant: 8),
            sidebarModeControl.leadingAnchor.constraint(equalTo: sidebarContainer.leadingAnchor, constant: 8),
            sidebarModeControl.trailingAnchor.constraint(equalTo: sidebarContainer.trailingAnchor, constant: -8),

            fileTreeView.topAnchor.constraint(equalTo: sidebarModeControl.bottomAnchor, constant: 6),
            fileTreeView.leadingAnchor.constraint(equalTo: sidebarContainer.leadingAnchor),
            fileTreeView.trailingAnchor.constraint(equalTo: sidebarContainer.trailingAnchor),
            fileTreeView.bottomAnchor.constraint(equalTo: sidebarContainer.bottomAnchor),

            searchPanel.topAnchor.constraint(equalTo: sidebarModeControl.bottomAnchor, constant: 6),
            searchPanel.leadingAnchor.constraint(equalTo: sidebarContainer.leadingAnchor),
            searchPanel.trailingAnchor.constraint(equalTo: sidebarContainer.trailingAnchor),
            searchPanel.bottomAnchor.constraint(equalTo: sidebarContainer.bottomAnchor),
        ])

        // Content area: tab content + status bar stacked vertically
        contentContainer.translatesAutoresizingMaskIntoConstraints = false

        let tabContentView = tabManager.contentView
        tabContentView.translatesAutoresizingMaskIntoConstraints = false

        statusBar.translatesAutoresizingMaskIntoConstraints = false

        contentContainer.addSubview(tabContentView)
        contentContainer.addSubview(statusBar)

        NSLayoutConstraint.activate([
            tabContentView.topAnchor.constraint(equalTo: contentContainer.topAnchor),
            tabContentView.leadingAnchor.constraint(equalTo: contentContainer.leadingAnchor),
            tabContentView.trailingAnchor.constraint(equalTo: contentContainer.trailingAnchor),
            tabContentView.bottomAnchor.constraint(equalTo: statusBar.topAnchor),

            statusBar.leadingAnchor.constraint(equalTo: contentContainer.leadingAnchor),
            statusBar.trailingAnchor.constraint(equalTo: contentContainer.trailingAnchor),
            statusBar.bottomAnchor.constraint(equalTo: contentContainer.bottomAnchor),
        ])

        splitView.addArrangedSubview(sidebarContainer)
        splitView.addArrangedSubview(contentContainer)

        contentView.addSubview(splitView)

        NSLayoutConstraint.activate([
            splitView.topAnchor.constraint(equalTo: contentView.topAnchor),
            splitView.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            splitView.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])

        // Set initial sidebar width.
        sidebarContainer.setFrameSize(NSSize(width: sidebarTargetWidth, height: sidebarContainer.frame.height))

        // Apply initial visibility.
        if !sidebarVisible {
            splitView.setPosition(0, ofDividerAt: 0)
            sidebarContainer.isHidden = true
        }

        // Content area has a minimum width so the sidebar cannot push it off screen.
        contentContainer.setContentHuggingPriority(.defaultLow, for: .horizontal)
    }

    // MARK: - Toolbar

    private func setupToolbar() {
        let toolbar = NSToolbar(identifier: Self.toolbarIdentifier)
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        window?.toolbar = toolbar
        window?.toolbarStyle = .unified
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        return [
            Self.sidebarToggleItemIdentifier,
            .flexibleSpace,
            Self.tabBarItemIdentifier,
            .flexibleSpace,
            Self.newTabItemIdentifier,
        ]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        return toolbarDefaultItemIdentifiers(toolbar)
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier, willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        switch itemIdentifier {
        case Self.sidebarToggleItemIdentifier:
            let item = NSToolbarItem(itemIdentifier: itemIdentifier)
            item.image = NSImage(systemSymbolName: "sidebar.left", accessibilityDescription: "Toggle Sidebar")
            item.label = "Sidebar"
            item.toolTip = "Toggle Sidebar"
            item.target = self
            item.action = #selector(toggleSidebarAction(_:))
            return item

        case Self.tabBarItemIdentifier:
            let item = NSToolbarItem(itemIdentifier: itemIdentifier)
            item.view = tabManager.segmentedControl
            item.label = "Tabs"
            return item

        case Self.newTabItemIdentifier:
            let item = NSToolbarItem(itemIdentifier: itemIdentifier)
            item.image = NSImage(systemSymbolName: "plus", accessibilityDescription: "New Tab")
            item.label = "New Tab"
            item.toolTip = "New Terminal Tab"
            item.target = self
            item.action = #selector(newTabAction(_:))
            return item

        default:
            return nil
        }
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

    @objc private func sidebarModeChanged(_ sender: NSSegmentedControl) {
        let showSearch = sender.selectedSegment == 1
        fileTreeView.isHidden = showSearch
        searchPanel.isHidden = !showSearch
        if showSearch {
            searchPanel.focus()
        }
    }

    /// Toggles the sidebar visibility with animation.
    func toggleSidebar() {
        sidebarVisible.toggle()
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.2
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            if sidebarVisible {
                sidebarContainer.isHidden = false
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

    /// Updates the status bar with information from the currently active tab.
    func updateStatusBar() {
        guard let tabInfo = tabManager.activeTabInfo else { return }

        if let shellName = tabInfo.shellName {
            statusBar.updateForTerminal(
                cwd: tabInfo.cwd ?? NSHomeDirectory(),
                gitBranch: tabInfo.gitBranch,
                shellName: shellName
            )
        } else if let language = tabInfo.language {
            statusBar.updateForEditor(
                filePath: tabInfo.cwd ?? "",
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

        // Window background
        window?.backgroundColor = newTheme.bg

        // Sidebar
        sidebarContainer.layer?.backgroundColor = newTheme.bgDark.cgColor

        // Status bar
        statusBar.applyTheme(newTheme)

        // Content background
        contentContainer.layer?.backgroundColor = newTheme.bg.cgColor

        // Tab manager (propagates to all tabs)
        tabManager.applyTheme(newTheme)
    }

    // MARK: - Notification Observers

    private func setupNotificationObservers() {
        let nc = NotificationCenter.default

        nc.addObserver(forName: .impulseToggleSidebar, object: nil, queue: .main) { [weak self] _ in
            guard self?.window?.isKeyWindow == true else { return }
            self?.toggleSidebar()
        }
        nc.addObserver(forName: .impulseNewTerminalTab, object: nil, queue: .main) { [weak self] _ in
            guard self?.window?.isKeyWindow == true else { return }
            self?.tabManager.addTerminalTab()
        }
        nc.addObserver(forName: .impulseCloseTab, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            self.tabManager.closeTab(index: self.tabManager.selectedIndex)
        }
        nc.addObserver(forName: .impulseActiveTabDidChange, object: nil, queue: .main) { [weak self] _ in
            self?.updateStatusBar()
        }
        nc.addObserver(forName: .impulseOpenFile, object: nil, queue: .main) { [weak self] notification in
            guard let self, self.window?.isKeyWindow == true else { return }
            if let path = notification.userInfo?["path"] as? String {
                self.tabManager.addEditorTab(path: path)
            }
        }
        nc.addObserver(forName: .impulseSplitHorizontal, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            self.tabManager.splitTerminalHorizontally()
        }
        nc.addObserver(forName: .impulseSplitVertical, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            self.tabManager.splitTerminalVertically()
        }
        nc.addObserver(forName: .impulseFindInProject, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            // Switch sidebar to search mode and show it if hidden.
            if !self.sidebarVisible {
                self.toggleSidebar()
            }
            self.sidebarModeControl.selectedSegment = 1
            self.sidebarModeChanged(self.sidebarModeControl)
        }
        nc.addObserver(forName: .terminalCwdChanged, object: nil, queue: .main) { [weak self] notification in
            guard let self else { return }
            if let dir = notification.userInfo?["directory"] as? String {
                self.fileTreeView.setRootPath(dir)
                self.searchPanel.setRootPath(dir)
                self.statusBar.updateForTerminal(
                    cwd: dir,
                    gitBranch: nil,
                    shellName: ImpulseCore.getUserLoginShellName()
                )
            }
        }

        // Command palette
        nc.addObserver(forName: .impulseShowCommandPalette, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true, let window = self.window else { return }
            self.commandPalette.show(relativeTo: window)
        }

        // Save file
        nc.addObserver(forName: .impulseSaveFile, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            guard self.tabManager.selectedIndex >= 0,
                  self.tabManager.selectedIndex < self.tabManager.tabs.count else { return }
            if case .editor(let editor) = self.tabManager.tabs[self.tabManager.selectedIndex] {
                editor.saveFile()
                self.tabManager.refreshSegmentLabels()
            }
        }

        // Find in editor — trigger Monaco's built-in find widget via JS
        nc.addObserver(forName: .impulseFind, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            guard self.tabManager.selectedIndex >= 0,
                  self.tabManager.selectedIndex < self.tabManager.tabs.count else { return }
            if case .editor(let editor) = self.tabManager.tabs[self.tabManager.selectedIndex] {
                editor.webView.evaluateJavaScript(
                    "editor.getAction('actions.find').run()",
                    completionHandler: nil
                )
            }
        }

        // Editor cursor position tracking for status bar
        nc.addObserver(forName: .editorCursorMoved, object: nil, queue: .main) { [weak self] notification in
            guard let self else { return }
            guard let line = notification.userInfo?["line"] as? UInt32,
                  let col = notification.userInfo?["column"] as? UInt32 else { return }
            // Only update if the notification came from the active editor tab
            guard self.tabManager.selectedIndex >= 0,
                  self.tabManager.selectedIndex < self.tabManager.tabs.count,
                  case .editor(let editor) = self.tabManager.tabs[self.tabManager.selectedIndex],
                  editor === notification.object as? EditorTab else { return }
            self.statusBar.updateForEditor(
                filePath: editor.filePath ?? "",
                cursorLine: Int(line) + 1,
                cursorCol: Int(col) + 1,
                language: editor.language,
                tabWidth: self.settings.tabWidth,
                useSpaces: self.settings.useSpaces
            )
        }

        // Terminal title changed — update tab segment labels
        nc.addObserver(forName: .terminalTitleChanged, object: nil, queue: .main) { [weak self] _ in
            self?.tabManager.refreshSegmentLabels()
        }

        // Terminal process terminated — close the tab if it was the only terminal
        nc.addObserver(forName: .terminalProcessTerminated, object: nil, queue: .main) { [weak self] notification in
            guard let self else { return }
            guard let terminalTab = notification.object as? TerminalTab else { return }
            // Find the container that owns this terminal and check if we should close
            for (index, tab) in self.tabManager.tabs.enumerated() {
                if case .terminal(let container) = tab {
                    if container.terminals.count == 1 && container.terminals.first === terminalTab {
                        self.tabManager.closeTab(index: index)
                        break
                    }
                }
            }
        }
    }

    // MARK: - NSWindowDelegate

    func windowWillClose(_ notification: Notification) {
        (NSApp.delegate as? AppDelegate)?.windowControllerDidClose(self)
    }
}

