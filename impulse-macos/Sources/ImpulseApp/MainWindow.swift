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

    private var settings: Settings
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

    // MARK: Git State

    /// Cached git branch name for the current working directory.
    private var cachedGitBranch: String?
    /// The directory path for which `cachedGitBranch` was computed.
    private var cachedGitBranchDir: String = ""

    // MARK: LSP State

    /// Per-URI document version counter for LSP.
    private var lspDocVersions: [String: Int32] = [:]

    /// Tracks the latest completion request ID per URI for deduplication.
    private var latestCompletionReq: [String: UInt64] = [:]

    /// Tracks the latest hover request ID per URI for deduplication.
    private var latestHoverReq: [String: UInt64] = [:]

    /// Timer for polling LSP events (diagnostics, lifecycle).
    private var lspPollTimer: Timer?

    /// Serial queue for dispatching blocking LSP FFI calls off the main thread.
    private let lspQueue = DispatchQueue(label: "dev.impulse.lsp", qos: .userInitiated)

    /// Set of file URIs for which didOpen has been sent.
    private var lspOpenFiles: Set<String> = []

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

        // Start polling for asynchronous LSP events (diagnostics, etc.).
        startLspPolling()
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
                terminal.sendCommand("cd \(dir.replacingOccurrences(of: " ", with: "\\ "))")
            }
        }
        nc.addObserver(forName: .impulseCloseTab, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            let index = self.tabManager.selectedIndex
            guard index >= 0, index < self.tabManager.tabs.count else { return }
            // Send LSP didClose before removing the tab.
            if case .editor(let editor) = self.tabManager.tabs[index] {
                self.lspDidClose(editor: editor)
            }
            self.tabManager.closeTab(index: index)
        }
        nc.addObserver(forName: .impulseActiveTabDidChange, object: nil, queue: .main) { [weak self] _ in
            self?.updateStatusBar()
        }
        nc.addObserver(forName: .impulseOpenFile, object: nil, queue: .main) { [weak self] notification in
            guard let self, self.window?.isKeyWindow == true else { return }
            if let path = notification.userInfo?["path"] as? String {
                self.tabManager.addEditorTab(path: path)
                self.lspDidOpenIfNeeded(path: path)
                // Navigate to specific line if provided (e.g. from search results).
                if self.tabManager.selectedIndex >= 0,
                   self.tabManager.selectedIndex < self.tabManager.tabs.count,
                   case .editor(let editor) = self.tabManager.tabs[self.tabManager.selectedIndex] {
                    if let line = notification.userInfo?["line"] as? UInt32 {
                        editor.goToPosition(line: line, column: 1)
                    }
                    // Apply git diff decorations for the opened file.
                    self.applyGitDiffDecorations(editor: editor)
                }
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
                self.invalidateGitBranchCache()
                let branch = self.gitBranch(forDirectory: dir)
                self.statusBar.updateForTerminal(
                    cwd: dir,
                    gitBranch: branch,
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
                self.saveEditorTab(editor)
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
            let filePath = editor.filePath ?? ""
            let dir = (filePath as NSString).deletingLastPathComponent
            let branch = dir.isEmpty ? nil : self.gitBranch(forDirectory: dir)
            self.statusBar.updateForEditor(
                filePath: filePath,
                gitBranch: branch,
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

        // Editor content changed — refresh tab labels and notify LSP
        nc.addObserver(forName: .editorContentChanged, object: nil, queue: .main) { [weak self] notification in
            guard let self else { return }
            self.tabManager.refreshSegmentLabels()
            if let editor = notification.object as? EditorTab {
                self.lspDidChange(editor: editor)
            }
        }

        // Editor focus changed — auto-save on focus loss if enabled
        nc.addObserver(forName: .editorFocusChanged, object: nil, queue: .main) { [weak self] notification in
            guard let self, self.settings.autoSave else { return }
            guard let focused = notification.userInfo?["focused"] as? Bool, !focused else { return }
            guard let editor = notification.object as? EditorTab,
                  editor.isModified else { return }
            self.saveEditorTab(editor)
        }

        // Custom keybinding command execution
        nc.addObserver(forName: Notification.Name("impulseCustomCommand"), object: nil, queue: .main) { [weak self] notification in
            guard let self, self.window?.isKeyWindow == true else { return }
            guard let command = notification.userInfo?["command"] as? String,
                  !command.isEmpty else { return }
            let args = notification.userInfo?["args"] as? [String] ?? []
            self.executeCustomCommand(command: command, args: args)
        }

        // LSP: completion requested
        nc.addObserver(forName: .editorCompletionRequested, object: nil, queue: .main) { [weak self] notification in
            guard let self,
                  let editor = notification.object as? EditorTab,
                  let requestId = notification.userInfo?["requestId"] as? UInt64,
                  let line = notification.userInfo?["line"] as? UInt32,
                  let character = notification.userInfo?["character"] as? UInt32 else { return }
            self.handleCompletionRequest(editor: editor, requestId: requestId, line: line, character: character)
        }

        // LSP: hover requested
        nc.addObserver(forName: .editorHoverRequested, object: nil, queue: .main) { [weak self] notification in
            guard let self,
                  let editor = notification.object as? EditorTab,
                  let requestId = notification.userInfo?["requestId"] as? UInt64,
                  let line = notification.userInfo?["line"] as? UInt32,
                  let character = notification.userInfo?["character"] as? UInt32 else { return }
            self.handleHoverRequest(editor: editor, requestId: requestId, line: line, character: character)
        }

        // Go to line
        nc.addObserver(forName: .impulseGoToLine, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            self.showGoToLineDialog()
        }

        // Font size
        nc.addObserver(forName: .impulseFontIncrease, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            self.changeFontSize(delta: 1)
        }
        nc.addObserver(forName: .impulseFontDecrease, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            self.changeFontSize(delta: -1)
        }
        nc.addObserver(forName: .impulseFontReset, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            self.resetFontSize()
        }

        // Tab cycling
        nc.addObserver(forName: .impulseNextTab, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            let count = self.tabManager.tabs.count
            guard count > 1 else { return }
            let next = (self.tabManager.selectedIndex + 1) % count
            self.tabManager.selectTab(index: next)
        }
        nc.addObserver(forName: .impulsePrevTab, object: nil, queue: .main) { [weak self] _ in
            guard let self, self.window?.isKeyWindow == true else { return }
            let count = self.tabManager.tabs.count
            guard count > 1 else { return }
            let prev = (self.tabManager.selectedIndex - 1 + count) % count
            self.tabManager.selectTab(index: prev)
        }
        nc.addObserver(forName: .impulseSelectTab, object: nil, queue: .main) { [weak self] notification in
            guard let self, self.window?.isKeyWindow == true else { return }
            guard let index = notification.userInfo?["index"] as? Int else { return }
            if index >= 0, index < self.tabManager.tabs.count {
                self.tabManager.selectTab(index: index)
            }
        }

        // LSP: go-to-definition requested
        nc.addObserver(forName: .editorDefinitionRequested, object: nil, queue: .main) { [weak self] notification in
            guard let self,
                  let editor = notification.object as? EditorTab,
                  let line = notification.userInfo?["line"] as? UInt32,
                  let character = notification.userInfo?["character"] as? UInt32 else { return }
            self.handleDefinitionRequest(editor: editor, line: line, character: character)
        }
    }

    // MARK: - LSP Integration

    private func startLspPolling() {
        lspPollTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            self?.processLspEvents()
        }
    }

    private func stopLspPolling() {
        lspPollTimer?.invalidate()
        lspPollTimer = nil
    }

    /// Polls for asynchronous LSP events and dispatches them to the appropriate
    /// editor tab. Called on a 100ms timer on the main thread.
    private func processLspEvents() {
        while let json = core.lspPollEvent() {
            guard let data = json.data(using: .utf8),
                  let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let type = event["type"] as? String else { continue }

            switch type {
            case "diagnostics":
                guard let uri = event["uri"] as? String,
                      let diagnosticsArray = event["diagnostics"] as? [[String: Any]] else { continue }

                let filePath = uriToFilePath(uri)
                guard let editorTab = findEditorTab(forPath: filePath) else { continue }

                let markers: [MonacoDiagnostic] = diagnosticsArray.compactMap { d in
                    guard let severity = (d["severity"] as? NSNumber)?.uint8Value,
                          let startLine = (d["startLine"] as? NSNumber)?.uint32Value,
                          let startColumn = (d["startColumn"] as? NSNumber)?.uint32Value,
                          let endLine = (d["endLine"] as? NSNumber)?.uint32Value,
                          let endColumn = (d["endColumn"] as? NSNumber)?.uint32Value,
                          let message = d["message"] as? String else { return nil }
                    return MonacoDiagnostic(
                        severity: diagnosticSeverityToMonaco(severity),
                        startLine: startLine + 1,   // LSP 0-based → Monaco 1-based
                        startColumn: startColumn + 1,
                        endLine: endLine + 1,
                        endColumn: endColumn + 1,
                        message: message,
                        source: d["source"] as? String
                    )
                }
                editorTab.applyDiagnostics(uri: uri, markers: markers)

            default:
                break
            }
        }
    }

    /// Sends LSP didOpen for a file if not already tracked.
    private func lspDidOpenIfNeeded(path: String) {
        let uri = filePathToUri(path)
        guard !lspOpenFiles.contains(uri) else { return }
        lspOpenFiles.insert(uri)
        lspDocVersions[uri] = 1

        guard let editorTab = findEditorTab(forPath: path) else { return }
        let language = editorTab.language
        let content = editorTab.content

        lspQueue.async { [weak self] in
            guard let self else { return }
            self.core.lspEnsureServers(languageId: language, fileUri: uri)
            let params = """
            {"textDocument":{"uri":"\(self.jsonEscape(uri))","languageId":"\(self.jsonEscape(language))","version":1,"text":"\(self.jsonEscape(content))"}}
            """
            self.core.lspNotify(languageId: language, fileUri: uri, method: "textDocument/didOpen", paramsJson: params)
        }
    }

    /// Sends LSP didChange for a content update.
    private func lspDidChange(editor: EditorTab) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        guard lspOpenFiles.contains(uri) else { return }

        let version = (lspDocVersions[uri] ?? 1) + 1
        lspDocVersions[uri] = version

        let language = editor.language
        let content = editor.content

        lspQueue.async { [weak self] in
            guard let self else { return }
            let params = """
            {"textDocument":{"uri":"\(self.jsonEscape(uri))","version":\(version)},"contentChanges":[{"text":"\(self.jsonEscape(content))"}]}
            """
            self.core.lspNotify(languageId: language, fileUri: uri, method: "textDocument/didChange", paramsJson: params)
        }
    }

    /// Sends LSP didClose when an editor tab is closed.
    private func lspDidClose(editor: EditorTab) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        guard lspOpenFiles.contains(uri) else { return }
        lspOpenFiles.remove(uri)
        lspDocVersions.removeValue(forKey: uri)

        let language = editor.language
        lspQueue.async { [weak self] in
            guard let self else { return }
            let params = """
            {"textDocument":{"uri":"\(self.jsonEscape(uri))"}}
            """
            self.core.lspNotify(languageId: language, fileUri: uri, method: "textDocument/didClose", paramsJson: params)
        }
    }

    /// Sends LSP didSave after a file is saved.
    private func lspDidSave(editor: EditorTab) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        guard lspOpenFiles.contains(uri) else { return }

        let language = editor.language
        lspQueue.async { [weak self] in
            guard let self else { return }
            let params = """
            {"textDocument":{"uri":"\(self.jsonEscape(uri))"}}
            """
            self.core.lspNotify(languageId: language, fileUri: uri, method: "textDocument/didSave", paramsJson: params)
        }
    }

    /// Handles a completion request from the editor by forwarding it to the LSP.
    private func handleCompletionRequest(editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        latestCompletionReq[uri] = requestId

        let language = editor.language
        let params = """
        {"textDocument":{"uri":"\(jsonEscape(uri))"},"position":{"line":\(line),"character":\(character)}}
        """

        lspQueue.async { [weak self] in
            guard let self else { return }
            guard let response = self.core.lspRequest(
                languageId: language, fileUri: uri,
                method: "textDocument/completion", paramsJson: params
            ) else { return }

            let items = self.parseCompletionResponse(response)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                // Only resolve if this is still the latest request for this URI.
                guard self.latestCompletionReq[uri] == requestId else { return }
                editor.resolveCompletions(requestId: requestId, items: items)
            }
        }
    }

    /// Handles a hover request from the editor by forwarding it to the LSP.
    private func handleHoverRequest(editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        latestHoverReq[uri] = requestId

        let language = editor.language
        let params = """
        {"textDocument":{"uri":"\(jsonEscape(uri))"},"position":{"line":\(line),"character":\(character)}}
        """

        lspQueue.async { [weak self] in
            guard let self else { return }
            guard let response = self.core.lspRequest(
                languageId: language, fileUri: uri,
                method: "textDocument/hover", paramsJson: params
            ) else { return }

            let contents = self.parseHoverResponse(response)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                guard self.latestHoverReq[uri] == requestId else { return }
                editor.resolveHover(requestId: requestId, contents: contents)
            }
        }
    }

    /// Handles a go-to-definition request from the editor.
    private func handleDefinitionRequest(editor: EditorTab, line: UInt32, character: UInt32) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        let language = editor.language
        let params = """
        {"textDocument":{"uri":"\(jsonEscape(uri))"},"position":{"line":\(line),"character":\(character)}}
        """

        lspQueue.async { [weak self] in
            guard let self else { return }
            guard let response = self.core.lspRequest(
                languageId: language, fileUri: uri,
                method: "textDocument/definition", paramsJson: params
            ) else { return }

            guard let def = self.parseDefinitionResponse(response) else { return }
            let targetPath = self.uriToFilePath(def.uri)

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.tabManager.addEditorTab(path: targetPath)
                self.lspDidOpenIfNeeded(path: targetPath)
                // Navigate to the definition position (convert 0-based to 1-based).
                if self.tabManager.selectedIndex >= 0,
                   self.tabManager.selectedIndex < self.tabManager.tabs.count,
                   case .editor(let targetEditor) = self.tabManager.tabs[self.tabManager.selectedIndex] {
                    targetEditor.goToPosition(line: def.line + 1, column: def.character + 1)
                }
            }
        }
    }

    // MARK: LSP Response Parsing

    private func parseCompletionResponse(_ json: String) -> [MonacoCompletionItem] {
        guard let data = json.data(using: .utf8),
              let response = try? JSONSerialization.jsonObject(with: data) else { return [] }

        let items: [[String: Any]]
        if let list = response as? [String: Any],
           let listItems = list["items"] as? [[String: Any]] {
            items = listItems
        } else if let array = response as? [[String: Any]] {
            items = array
        } else {
            return []
        }

        return items.compactMap { item in
            guard let label = item["label"] as? String else { return nil }
            let kind = (item["kind"] as? NSNumber)?.intValue ?? 1
            let insertText = (item["insertText"] as? String) ?? label
            let detail = item["detail"] as? String
            let insertTextFormat = (item["insertTextFormat"] as? NSNumber)?.intValue ?? 1

            return MonacoCompletionItem(
                label: label,
                kind: lspCompletionKindFromInt(kind),
                detail: detail,
                insertText: insertText,
                insertTextRules: insertTextFormat == 2 ? 4 : nil
            )
        }
    }

    private func parseHoverResponse(_ json: String) -> [MonacoHoverContent] {
        guard let data = json.data(using: .utf8),
              let response = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let contents = response["contents"] else { return [] }

        if let markup = contents as? [String: Any], let value = markup["value"] as? String {
            return [MonacoHoverContent(value: value, isTrusted: true)]
        } else if let str = contents as? String {
            return [MonacoHoverContent(value: str, isTrusted: true)]
        } else if let array = contents as? [[String: Any]] {
            return array.compactMap { item in
                guard let value = item["value"] as? String else { return nil }
                return MonacoHoverContent(value: value, isTrusted: true)
            }
        }
        return []
    }

    private func parseDefinitionResponse(_ json: String) -> (uri: String, line: UInt32, character: UInt32)? {
        guard let data = json.data(using: .utf8),
              let response = try? JSONSerialization.jsonObject(with: data) else { return nil }

        let location: [String: Any]?
        if let loc = response as? [String: Any], loc["uri"] != nil {
            location = loc
        } else if let array = response as? [[String: Any]], let first = array.first {
            if first["targetUri"] != nil {
                // LocationLink format
                guard let uri = first["targetUri"] as? String,
                      let range = (first["targetSelectionRange"] as? [String: Any])
                          ?? (first["targetRange"] as? [String: Any]),
                      let start = range["start"] as? [String: Any],
                      let line = (start["line"] as? NSNumber)?.uint32Value,
                      let character = (start["character"] as? NSNumber)?.uint32Value else { return nil }
                return (uri: uri, line: line, character: character)
            }
            location = first
        } else {
            return nil
        }

        guard let loc = location,
              let uri = loc["uri"] as? String,
              let range = loc["range"] as? [String: Any],
              let start = range["start"] as? [String: Any],
              let line = (start["line"] as? NSNumber)?.uint32Value,
              let character = (start["character"] as? NSNumber)?.uint32Value else { return nil }
        return (uri: uri, line: line, character: character)
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

        // 1. Format on save — find applicable formatter
        let formatter = resolveFormatOnSave(forPath: path)
        if let fmt = formatter, !fmt.command.isEmpty {
            // Save first so the formatter can read the file
            editor.saveFile()
            runExternalCommand(command: fmt.command, args: fmt.args, cwd: (path as NSString).deletingLastPathComponent) { [weak self, weak editor] in
                guard let self, let editor else { return }
                // Reload the file after formatting
                if let newContent = try? String(contentsOfFile: path, encoding: .utf8),
                   newContent != editor.content {
                    editor.openFile(path: path, content: newContent, language: editor.language)
                }
                self.postSaveActions(editor: editor, path: path)
            }
        } else {
            editor.saveFile()
            postSaveActions(editor: editor, path: path)
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
                runExternalCommand(command: cmd.command, args: cmd.args, cwd: cwd) { [weak editor] in
                    guard let editor else { return }
                    if let newContent = try? String(contentsOfFile: path, encoding: .utf8),
                       newContent != editor.content {
                        editor.openFile(path: path, content: newContent, language: editor.language)
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
    /// with the command running in it.
    private func executeCustomCommand(command: String, args: [String]) {
        // Get the CWD from the active tab
        let cwd: String
        if let tabInfo = tabManager.activeTabInfo, let dir = tabInfo.cwd {
            if FileManager.default.fileExists(atPath: dir) {
                // dir could be a file path (for editor tabs); use parent directory
                var isDir: ObjCBool = false
                if FileManager.default.fileExists(atPath: dir, isDirectory: &isDir), isDir.boolValue {
                    cwd = dir
                } else {
                    cwd = (dir as NSString).deletingLastPathComponent
                }
            } else {
                cwd = NSHomeDirectory()
            }
        } else {
            cwd = NSHomeDirectory()
        }

        // Build the full command string
        let fullCommand = ([command] + args).joined(separator: " ")

        // Post a notification to create a new terminal tab with the command
        // We'll use the existing new terminal tab flow but with a custom initial command
        tabManager.addTerminalTab()
        // Send the command to the newly created terminal
        if tabManager.selectedIndex >= 0,
           tabManager.selectedIndex < tabManager.tabs.count,
           case .terminal(let container) = tabManager.tabs[tabManager.selectedIndex],
           let terminal = container.activeTerminal {
            // Set the working directory and run the command
            terminal.sendCommand(fullCommand)
        }
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
            lastDirectory: settings.lastDirectory
        )

        for tab in tabManager.tabs {
            switch tab {
            case .editor(let editor):
                editor.applySettings(editorOptions)
            case .terminal(let container):
                container.applySettings(settings: termSettings)
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

    // MARK: LSP Helpers

    /// Maps an LSP CompletionItemKind integer to a Monaco CompletionItemKind value.
    private func lspCompletionKindFromInt(_ kind: Int) -> UInt32 {
        switch kind {
        case 1:  return 18  // Text
        case 2:  return 0   // Method
        case 3:  return 1   // Function
        case 4:  return 2   // Constructor
        case 5:  return 3   // Field
        case 6:  return 4   // Variable
        case 7:  return 5   // Class
        case 8:  return 7   // Interface
        case 9:  return 8   // Module
        case 10: return 9   // Property
        case 11: return 12  // Unit
        case 12: return 13  // Value
        case 13: return 15  // Enum
        case 14: return 17  // Keyword
        case 15: return 27  // Snippet
        case 16: return 19  // Color
        case 17: return 20  // File
        case 18: return 21  // Reference
        case 19: return 23  // Folder
        case 20: return 16  // EnumMember
        case 21: return 14  // Constant
        case 22: return 6   // Struct
        case 23: return 10  // Event
        case 24: return 11  // Operator
        case 25: return 24  // TypeParameter
        default: return 18  // Text
        }
    }

    /// Converts an absolute file path to a file:// URI.
    private func filePathToUri(_ path: String) -> String {
        return URL(fileURLWithPath: path).absoluteString
    }

    /// Extracts the file path from a file:// URI.
    private func uriToFilePath(_ uri: String) -> String {
        if let url = URL(string: uri), url.scheme == "file" {
            return url.path
        }
        return uri
    }

    /// Escapes a string for safe embedding in a JSON string literal.
    private func jsonEscape(_ str: String) -> String {
        return str
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\n", with: "\\n")
            .replacingOccurrences(of: "\r", with: "\\r")
            .replacingOccurrences(of: "\t", with: "\\t")
    }

    /// Finds the editor tab that has the given file path open.
    private func findEditorTab(forPath path: String) -> EditorTab? {
        for tab in tabManager.tabs {
            if case .editor(let editor) = tab, editor.filePath == path {
                return editor
            }
        }
        return nil
    }

    // MARK: - NSWindowDelegate

    func windowWillClose(_ notification: Notification) {
        stopLspPolling()

        // Persist sidebar state back to AppDelegate settings.
        if let delegate = NSApp.delegate as? AppDelegate {
            delegate.settings.sidebarVisible = sidebarVisible
            delegate.settings.sidebarWidth = Int(sidebarTargetWidth)
        }

        (NSApp.delegate as? AppDelegate)?.windowControllerDidClose(self)
    }
}

