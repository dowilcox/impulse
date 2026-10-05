import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI
import os.log

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

// MARK: - Main Window Controller

/// The primary window controller for Impulse. Each window contains:
///   - An NSSplitView with a sidebar (file tree + search) on the left and a
///     content area on the right.
///   - SwiftUI chrome for the tab bar, sidebar, and status bar around a
///     TabManager-driven AppKit content region.
///
/// Multiple windows can coexist; each owns its own TabManager and sidebar state.
final class MainWindowController: NSWindowController, NSWindowDelegate {

  // MARK: - State

  /// Backed by `SettingsStore.shared` (no private copy to keep in sync).
  private var settings: Settings {
    get { SettingsStore.shared.settings }
    set { SettingsStore.shared.settings = newValue }
  }

  /// The shared Rust backend (impulse-ffi) instance.
  ///
  /// `internal` (not `private`) because it is accessed from the
  /// `MainWindowController+LSP` extension in a separate file.
  let core: ImpulseCore

  private(set) var theme: Theme

  /// Observable state shared with SwiftUI views.
  let windowModel = WindowModel()

  /// Headless owner of the sidebar file tree data: nodes, filesystem and
  /// git watchers, and expansion-state persistence. Rendering happens in
  /// the SwiftUI `FileTreeListView` via `windowModel`.
  private let fileTreeData: FileTreeDataController

  // Old AppKit sidebar buttons removed — replaced by NSToolbar items.

  /// Manages the tab bar and tab content lifecycle.
  let tabManager: TabManager

  /// The window's AppKit layout root (docks, chrome, center column).
  private var workbench: WorkbenchView?

  /// Terminal search bar (hidden by default, toggled with Cmd+F on terminal tabs).
  private let termSearchBar = NSView()
  private let termSearchField = NSSearchField()
  private var termSearchBarVisible = false
  /// The window's terminal input bar (see `attachInputBar`).
  private var inputBarHost: NSView?
  /// The directory the window's repository was last resolved from.
  private var repositoryAnchor = ""
  private weak var inputBarTerminal: TerminalTab?
  private var termSearchHeightConstraint: NSLayoutConstraint?

  /// The command palette, lazily created on first use.
  /// Command palette / quick open (see PaletteModel for its modes).
  private let palette = PalettePanelController()


  /// Allows a deferred close after dirty editors have been reviewed without
  /// re-triggering the same review loop.
  private var closingAfterDirtyReview = false
  private var reviewingDirtyWindowClose = false

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
  private(set) var fileTreeRootPath: String = ""

  /// Cached file tree nodes keyed by root path for instant tab switching.
  private var fileTreeCache: [String: [FileTreeNode]] = [:]

  /// Tracks access order for LRU eviction of fileTreeCache entries.
  private var fileTreeCacheOrder: [String] = []

  /// Maximum number of entries in the file tree cache before LRU eviction.
  private let fileTreeCacheMaxSize = 20

  // MARK: Git State

  /// Mirrors the active repository's snapshot into the window model.
  private var repositoryObservation: ObservationLoop?
  /// Change listener on the active repository (file tree badges).
  private var repositoryListener: (state: GitRepositoryState, token: UUID)?

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

  /// Serial queue for dispatching blocking LSP FFI calls off the main thread.
  let lspQueue: DispatchQueue

  /// Set of file URIs for which didOpen has been sent.
  var lspOpenFiles: Set<String> = []

  // MARK: - Initialization

  init(
    settings: Settings,
    theme: Theme,
    core: ImpulseCore,
    lspQueue: DispatchQueue,
    skipInitialTerminal: Bool = false
  ) {
    self.theme = theme
    self.core = core
    self.lspQueue = lspQueue
    self.tabManager = TabManager(theme: theme, core: core)
    self.tabManager.windowModel = windowModel
    self.windowModel.theme = theme
    self.windowModel.iconCache = tabManager.iconCache
    self.windowModel.settingsLoadWarning = Settings.loadWarning
    self.windowModel.showHiddenFiles = settings.sidebarShowHidden
    self.windowModel.sidebarVisible = settings.sidebarVisible
    self.windowModel.sidebarWidth = CGFloat(settings.sidebarWidth)
    self.windowModel.tabBarPosition = settings.tabBarPosition
    self.windowModel.contextBarEnabled = settings.terminalContextBar
    self.fileTreeData = FileTreeDataController()

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
    window.title = AppState.isDev ? "Impulse [DEV]" : "Impulse"
    window.minSize = NSSize(width: 600, height: 400)
    window.center()
    // Restore the previous session's frame and auto-persist moves/resizes.
    // Falls back to the settings-based size + center() above when no saved
    // frame exists (first launch) or the name is taken by another window.
    window.setFrameAutosaveName("ImpulseMainWindow")
    window.isReleasedWhenClosed = false
    window.titlebarSeparatorStyle = .none
    window.titlebarAppearsTransparent = true
    window.titleVisibility = .hidden
    window.appearance = NSAppearance(named: theme.isLight ? .aqua : .darkAqua)
    window.backgroundColor = theme.bgSurfaceColor

    super.init(window: window)
    window.delegate = self

    // Impulse owns its tabs; keep macOS window tabbing (and its "Show Tab
    // Bar" / "Merge All Windows" menu items) out of the way.
    window.tabbingMode = .disallowed

    // An empty compact toolbar only to size the titlebar band so the traffic
    // lights sit vertically centered in the chrome bar; the chrome itself is
    // drawn by WorkbenchView under the transparent titlebar.
    let toolbar = NSToolbar(identifier: "ImpulseChromeSizing")
    toolbar.showsBaselineSeparator = false
    window.toolbar = toolbar
    window.toolbarStyle = .unifiedCompact

    setupLayout()
    setupNotificationObservers()
    setupCustomKeybindingMonitor()

    // Set initial root path for the file tree.
    // Always start at home; the sidebar will update once the terminal's CWD
    // is detected via OSC 7.
    let rootPath = NSHomeDirectory()
    // Dispatch the initial tree build off the main thread to avoid blocking
    // startup with heavy filesystem + git status work.
    let showHidden = settings.sidebarShowHidden
    fileTreeRootPath = rootPath
    fileTreeData.onTreeRefreshed = { [weak self] nodes in
      guard let self else { return }
      self.windowModel.updateFileTree(nodes, rootPath: self.fileTreeRootPath)
      self.fileTreeCacheInsert(key: self.fileTreeRootPath, nodes: nodes)
    }
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let nodes = FileTreeNode.buildTree(rootPath: rootPath, showHidden: showHidden)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard self.fileTreeRootPath == rootPath else { return }
        self.fileTreeData.showHidden = showHidden
        self.fileTreeData.updateTree(nodes: nodes, rootPath: rootPath)
        self.fileTreeCacheInsert(key: rootPath, nodes: nodes)
        // Push to SwiftUI sidebar
        self.windowModel.updateFileTree(nodes, rootPath: rootPath)
      }
    }
    // Wire the tab close handler for save confirmation on unsaved editor tabs.
    tabManager.tabCloseHandler = { [weak self] index in
      self?.requestCloseTab(index: index)
    }

    // Open a default terminal tab (skipped when launching with file arguments).
    if !skipInitialTerminal {
      tabManager.addTerminalTab(directory: DebugSnapshot.initialDirectory)
    }

  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is not supported")
  }

  deinit {
    teardownCustomKeybindingMonitor()
    notificationObservers.forEach { NotificationCenter.default.removeObserver($0) }
  }

  // MARK: - Layout

  private func setupLayout() {
    guard let contentView = window?.contentView else { return }

    // Wire SwiftUI callbacks → AppKit actions
    windowModel.onTabSelected = { [weak self] index in
      self?.tabManager.selectTab(index: index)
    }
    windowModel.onTabClosed = { [weak self] index in
      if let handler = self?.tabManager.tabCloseHandler {
        handler(index)
      } else {
        self?.tabManager.closeTab(index: index)
      }
    }
    windowModel.onNewTab = { [weak self] in
      self?.tabManager.addTerminalTab()
    }
    windowModel.onShowCommandHistory = { [weak self] in
      self?.showPalette(prefix: "h:")
    }
    windowModel.onClearTerminal = { [weak self] in
      self?.tabManager.selectedTerminal?.activeTerminal?.clearScreen()
    }
    windowModel.onRunCommand = { [weak self] command in
      self?.tabManager.selectedTerminal?.activeTerminal?.runCommand(command)
    }
    windowModel.onSwitchBranch = { [weak self] branch in
      self?.switchBranch(to: branch)
    }
    windowModel.onSendSecureInput = { [weak self] text in
      self?.tabManager.selectedTerminal?.activeTerminal?.sendSecureLine(text)
    }
    windowModel.onInputSuggestion = { [weak self] text in
      self?.tabManager.selectedTerminal?.activeTerminal?.historySuggestion(for: text)
    }
    windowModel.onCompletionCandidates = { [weak self] text in
      self?.tabManager.selectedTerminal?.activeTerminal?.completionCandidates(for: text)
    }
    windowModel.onRecentCommands = { [weak self] limit in
      self?.tabManager.selectedTerminal?.activeTerminal?.recentCommands(limit: limit) ?? []
    }
    windowModel.onSendInterrupt = { [weak self] in
      self?.tabManager.selectedTerminal?.activeTerminal?.sendInterrupt()
    }
    windowModel.onFocusTerminal = { [weak self] in
      self?.tabManager.selectedTerminal?.activeTerminal?.focus()
    }
    windowModel.onTabMoved = { [weak self] from, to in
      self?.tabManager.moveTab(from: from, to: to)
    }
    windowModel.onTabPinToggled = { [weak self] index in
      self?.tabManager.togglePin(index: index)
    }
    windowModel.onPreviewToggle = { [weak self] in
      self?.previewButtonClicked(nil)
    }
    windowModel.onOpenFile = { path, line in
      NotificationCenter.default.post(
        name: .impulseOpenFile,
        object: nil,
        userInfo: ["path": path, "line": line as Any]
      )
    }
    windowModel.onOpenFileBeside = { path in
      NotificationCenter.default.post(
        name: .impulseOpenFile, object: nil, userInfo: ["path": path, "beside": true])
    }
    windowModel.onRefreshTree = { [weak self] in
      guard let self else { return }
      let root = self.fileTreeRootPath
      let showHidden = self.windowModel.showHiddenFiles
      guard !root.isEmpty else { return }
      // Collect expanded paths to restore after rebuild.
      let expandedPaths = Self.collectExpandedPaths(self.windowModel.fileTreeNodes)
      DispatchQueue.global(qos: .userInitiated).async {
        let nodes = FileTreeNode.buildTree(rootPath: root, showHidden: showHidden)
        // Restore expanded state and load children for expanded dirs.
        Self.restoreExpandedPaths(expandedPaths, in: nodes, showHidden: showHidden)
        FileTreeNode.refreshGitStatus(nodes: nodes, repoPath: root, dirPath: root)
        DispatchQueue.main.async { [weak self] in
          guard let self else { return }
          self.fileTreeData.showHidden = showHidden
          self.fileTreeData.updateTree(nodes: nodes, rootPath: root)
          self.windowModel.updateFileTree(nodes)
          self.fileTreeCacheInsert(key: root, nodes: nodes)
        }
      }
    }
    windowModel.onCollapseAll = { [weak self] in
      guard let self else { return }
      self.fileTreeData.collapseAll()
      self.windowModel.updateFileTree(self.fileTreeData.rootNodes, rootPath: self.fileTreeRootPath)
      self.fileTreeCacheInsert(key: self.fileTreeRootPath, nodes: self.fileTreeData.rootNodes)
    }
    windowModel.onFileTreeExpansionChanged = { [weak self] in
      guard let self else { return }
      self.fileTreeData.persistCurrentExpandedPaths()
      self.fileTreeCacheInsert(key: self.fileTreeRootPath, nodes: self.windowModel.fileTreeNodes)
    }
    windowModel.onToggleHidden = { [weak self] in
      guard let self else { return }
      self.windowModel.showHiddenFiles.toggle()
      let showHidden = self.windowModel.showHiddenFiles
      let root = self.fileTreeRootPath
      self.settings.sidebarShowHidden = showHidden
      guard !root.isEmpty else { return }
      DispatchQueue.global(qos: .userInitiated).async {
        let nodes = FileTreeNode.buildTree(rootPath: root, showHidden: showHidden)
        FileTreeNode.refreshGitStatus(nodes: nodes, repoPath: root, dirPath: root)
        DispatchQueue.main.async { [weak self] in
          guard let self else { return }
          self.fileTreeData.showHidden = showHidden
          self.fileTreeData.updateTree(nodes: nodes, rootPath: root)
          self.windowModel.updateFileTree(nodes)
          self.fileTreeCacheInsert(key: root, nodes: nodes)
        }
      }
    }
    windowModel.onOpenSettingsFile = {
      NSWorkspace.shared.open(Settings.filePath)
    }
    windowModel.onDismissSettingsWarning = { [weak self] in
      self?.windowModel.settingsLoadWarning = nil
    }
    windowModel.onNewFile = { [weak self] (dirPath: String) in
      guard let self, !dirPath.isEmpty else { return }
      NameInputDialog.show(
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
        guard FileManager.default.createFile(atPath: fullPath, contents: nil) else { return }
        self.windowModel.onRefreshTree?()
      }
    }
    windowModel.onNewFolder = { [weak self] (dirPath: String) in
      guard let self, !dirPath.isEmpty else { return }
      NameInputDialog.show(
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
        try? FileManager.default.createDirectory(
          atPath: fullPath, withIntermediateDirectories: false)
        self.windowModel.onRefreshTree?()
      }
    }
    // Sidebar action-bar shortcuts: create in the selected tree dir (or root).
    windowModel.onCreateFile = { [weak self] in self?.newFileAction(nil) }
    windowModel.onCreateFolder = { [weak self] in self?.newFolderAction(nil) }
    windowModel.onOpenDiffReview = { [weak self] in self?.openDiffReview() }

    windowModel.gitHost = self
    if let window {
      let model = windowModel
      windowModel.toasts.attach(to: window) { model.palette }
    }
    windowModel.onShowCommandPalette = { [weak self] in
      self?.showCommandPalette()
    }
    windowModel.onShowBranchSwitcher = { [weak self] in
      self?.showBranchSwitcher()
    }
    windowModel.onToggleRightDock = { [weak self] in
      self?.toggleRightDock()
    }
    windowModel.onJoinTab = { [weak self] index, below in
      self?.tabManager.joinTab(at: index, axis: below ? .vertical : .horizontal)
    }
    windowModel.onPaneCommand = { [weak self] command in
      self?.performPaneCommand(command)
    }
    windowModel.onSelectWorkspace = { [weak self] id in
      self?.tabManager.activateWorkspace(id)
    }
    windowModel.onCloseWorkspace = { [weak self] id in
      self?.requestCloseWorkspace(id)
    }
    windowModel.onRenameWorkspace = { [weak self] id in
      self?.presentRenameWorkspace(id)
    }
    windowModel.onSetWorkspaceExpanded = { [weak self] id, expanded in
      self?.tabManager.setWorkspaceExpanded(id, expanded)
    }
    windowModel.onOpenWorkspace = { [weak self] in
      self?.presentOpenWorkspacePanel()
    }
    windowModel.onShowWorkspaceSwitcher = { [weak self] in
      self?.showPalette(prefix: "w:")
    }
    windowModel.onRevealTerminal = { [weak self] id in
      self?.revealTerminal(id: id)
    }

    // AppKit owns the layout (docks, dividers, focus); SwiftUI draws the
    // chrome inside hosting views. See WorkbenchView.
    let centerContent = ContentContainer(content: tabManager.contentView)
    // Hosting views here are sized by AppKit constraints: no SwiftUI-driven
    // min/max size (which clamps the window) and no safe-area padding (the
    // titlebar band would otherwise push the chrome down by its height).
    // The banner and input bar keep their intrinsic height.
    let inputHost = WorkbenchHosting.make(TerminalInputHost(model: windowModel), intrinsicHeight: true)
    inputBarHost = inputHost
    let workbench = WorkbenchView(
      model: windowModel,
      titlebar: WorkbenchHosting.make(ChromeBarView(model: windowModel)),
      banner: WorkbenchHosting.make(WorkbenchBanner(model: windowModel), intrinsicHeight: true),
      statusBar: WorkbenchHosting.make(WorkbenchStatusBar(model: windowModel)),
      leftDockContent: WorkbenchHosting.make(LeftDockView(model: windowModel)),
      rightDockContent: nil,
      bottomDockContent: nil
    )
    self.workbench = workbench
    workbench.translatesAutoresizingMaskIntoConstraints = false
    contentView.addSubview(workbench)
    NSLayoutConstraint.activate([
      workbench.topAnchor.constraint(equalTo: contentView.topAnchor),
      workbench.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
      workbench.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
      workbench.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
    ])

    // The terminal input bar lives inside whichever terminal has focus
    // (see attachInputBar), so the content fills the center column.
    let center = workbench.centerColumn
    centerContent.translatesAutoresizingMaskIntoConstraints = false
    center.addSubview(centerContent)
    NSLayoutConstraint.activate([
      centerContent.topAnchor.constraint(equalTo: center.topAnchor),
      centerContent.leadingAnchor.constraint(equalTo: center.leadingAnchor),
      centerContent.trailingAnchor.constraint(equalTo: center.trailingAnchor),
      centerContent.bottomAnchor.constraint(equalTo: center.bottomAnchor),
    ])
    attachInputBar()

    setupTerminalSearchBar()
  }

  // MARK: - Workbench actions

  func showCommandPalette() {
    showPalette(prefix: ">")
  }

  /// Open the palette with a mode prefix: "" files, ">" commands, ":" line,
  /// "%" text, "b:" branches, "t:" tabs.
  func showPalette(prefix: String) {
    guard let window else { return }
    let model = palette.model
    model.host = self
    model.iconCache = tabManager.iconCache
    model.shortcutOverrides = settings.keybindingOverrides
    model.commands = CommandRegistry.commands(
      for: self, customKeybindings: settings.customKeybindings)
    palette.show(in: window, prefix: prefix, palette: windowModel.palette)
  }

  /// Show or hide the right dock. Until a panel is installed there it has
  /// nothing to show, so this is a no-op beyond flipping the flag.
  /// Add the shells' own history files to Impulse's history.
  func importShellHistory() {
    CommandHistory.shared.importShellHistory { [weak self] count in
      self?.toasts.show(
        Toast(
          kind: .success,
          message: count == 0
            ? "No zsh, bash or fish history found." : "Imported \(count) commands from shell history."))
    }
  }

  /// Move the input bar into the focused terminal, carrying each terminal's
  /// unsent draft with it. Outside a terminal the bar leaves the hierarchy.
  func attachInputBar() {
    guard let host = inputBarHost else { return }
    let container = tabManager.selectedTerminal
    let terminal = container?.activeTerminal
    if terminal !== inputBarTerminal {
      inputBarTerminal?.inputDraft = windowModel.inputDraft
      windowModel.inputDraft = terminal?.inputDraft ?? ""
      windowModel.inputDraftRestoreToken += 1
      inputBarTerminal = terminal
    }
    if let container {
      container.attachAccessory(host)
    } else {
      host.removeFromSuperview()
    }
  }

  /// Bring a terminal forward: its window, workspace, tab and pane.
  @discardableResult
  func revealTerminal(id: UUID) -> Bool {
    guard
      let location = tabManager.locate(where: {
        if case .terminal(let container) = $0 { return container.activeTerminal?.id == id }
        return false
      })
    else { return false }
    window?.makeKeyAndOrderFront(nil)
    tabManager.reveal(location)
    return true
  }

  /// ⌘⇧U: the next agent waiting on the user (needs input first), cycling
  /// past the one already in front.
  func revealNextWaitingAgent() {
    let waiting = windowModel.agents.filter { $0.state.wantsUser }
    guard !waiting.isEmpty else {
      toasts.show(Toast(kind: .info, message: "No agents are waiting for you."))
      return
    }
    let current = tabManager.selectedTerminal?.activeTerminal?.id
    let start = waiting.firstIndex { $0.id == current }.map { $0 + 1 } ?? 0
    revealTerminal(id: waiting[start % waiting.count].id)
  }

  /// Split, focus, resize and zoom panes of the selected tab. `command` is
  /// the pane keybinding id ("split_right", "focus_pane_left", …).
  func performPaneCommand(_ command: String) {
    let directions: [String: PaneDirection] = [
      "left": .left, "right": .right, "up": .up, "down": .down,
    ]
    switch command {
    case "split_right", "split_down":
      let container = tabManager.makeTerminalContainer(directory: getActiveCwd())
      tabManager.splitSelectedTab(
        with: .terminal(container), axis: command == "split_right" ? .horizontal : .vertical)
    case "next_pane":
      tabManager.cyclePane(by: 1)
    case "prev_pane":
      tabManager.cyclePane(by: -1)
    case "zoom_pane":
      tabManager.toggleZoomSelectedPane()
    case "equalize_panes":
      tabManager.equalizeSelectedPanes()
    case "move_pane_to_tab":
      tabManager.movePaneToNewTab()
    default:
      if command.hasPrefix("focus_pane_"),
        let direction = directions[String(command.dropFirst("focus_pane_".count))]
      {
        if !tabManager.focusNeighborPane(direction) { NSSound.beep() }
      } else if command.hasPrefix("resize_pane_"),
        let direction = directions[String(command.dropFirst("resize_pane_".count))]
      {
        tabManager.resizeFocusedPane(toward: direction)
      }
    }
  }

  func toggleRightDock() {
    windowModel.rightDockVisible.toggle()
  }

  // MARK: - Debug Snapshot Actions

  /// Named UI actions for `--impulse-snapshot-actions` (headless visual checks).
  func performDebugAction(_ action: String) {
    switch action {
    case "sidebar": setSidebarVisible(true)
    case "palette": showPalette(prefix: ">")
    case "quickopen": showPalette(prefix: "")
    case "quickopen-query": showPalette(prefix: "wbv")
    case "branches": showPalette(prefix: "b:")
    case "review": openDiffReview()
    case "search": NotificationCenter.default.post(name: .impulseFindInProject, object: nil)
    case "changes": showChangesPanel()
    case "review-split":
      for case .diffReview(_, let review) in tabManager.allSurfaces { review.setLayout("split") }
    default:
      if action.hasPrefix("open=") {
        let relative = String(action.dropFirst(5))
        let base = DebugSnapshot.initialDirectory ?? fileTreeRootPath
        openFile(path: (base as NSString).appendingPathComponent(relative))
      } else if action.hasPrefix("beside=") {
        let relative = String(action.dropFirst(7))
        let base = DebugSnapshot.initialDirectory ?? fileTreeRootPath
        tabManager.addEditorTab(
          path: (base as NSString).appendingPathComponent(relative),
          projectDirectory: fileTreeRootPath, beside: true)
      } else if action.hasPrefix("workspace=") {
        let relative = String(action.dropFirst(10))
        let base = DebugSnapshot.initialDirectory ?? fileTreeRootPath
        tabManager.openWorkspace(
          folder: relative.hasPrefix("/") ? relative : (base as NSString).appendingPathComponent(relative))
      } else if action == "expand-workspaces" {
        for workspace in tabManager.workspaces {
          tabManager.setWorkspaceExpanded(workspace.id, true)
        }
      } else if action.hasPrefix("palette=") {
        showPalette(prefix: String(action.dropFirst(8)))
      } else if action.hasPrefix("run=") {
        tabManager.selectedTerminal?.activeTerminal?.runCommand(String(action.dropFirst(4)))
      } else if action == "close-pane" {
        requestCloseFocusedPane()
      } else if action == "reopen" {
        tabManager.reopenLastClosedTab()
      } else if action == "newtab" {
        tabManager.addTerminalTab()
      } else if action.hasPrefix("pane=") {
        performPaneCommand(String(action.dropFirst(5)))
      } else {
        NSLog("DebugSnapshot: unknown action '\(action)'")
      }
    }
  }

  // MARK: - Public API

  /// Opens a file in an editor tab. Called by AppDelegate for Finder "Open With"
  /// and CLI file arguments. Bypasses the notification path (which requires
  /// isKeyWindow) so it works during startup before the window is key.
  func openFile(path: String) {
    var isDirectory: ObjCBool = false
    if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue {
      tabManager.openWorkspace(folder: path)
      return
    }
    // Outside a folder workspace, the file tree follows the file.
    let dir = (path as NSString).deletingLastPathComponent
    if followsActiveDirectory, !dir.isEmpty, dir != fileTreeRootPath {
      switchFileTreeRoot(dir)
    }
    tabManager.addEditorTab(path: path, projectDirectory: fileTreeRootPath)
    if let editor = findEditorTab(forPath: path) {
      trackEditorTab(editor, forPath: path)
      lspDidOpenIfNeeded(path: path)
    }
  }

  // MARK: - Actions

  @objc private func toggleHiddenAction(_ sender: Any?) {
    windowModel.onToggleHidden?()
  }

  @objc private func collapseAllAction(_ sender: Any?) {
    windowModel.onCollapseAll?()
  }

  @objc private func refreshTreeAction(_ sender: Any?) {
    windowModel.onRefreshTree?()
  }

  private func selectedDirectoryForFileTreeAction() -> String {
    if let selectedPath = windowModel.selectedFileTreePath, !selectedPath.isEmpty {
      var isDirectory: ObjCBool = false
      if FileManager.default.fileExists(atPath: selectedPath, isDirectory: &isDirectory) {
        return isDirectory.boolValue
          ? selectedPath
          : (selectedPath as NSString).deletingLastPathComponent
      }
    }
    return fileTreeRootPath
  }

  @objc private func newFileAction(_ sender: Any?) {
    let dirPath = selectedDirectoryForFileTreeAction()
    guard !dirPath.isEmpty else { return }
    NameInputDialog.show(
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
      self.windowModel.onRefreshTree?()
    }
  }

  @objc private func newFolderAction(_ sender: Any?) {
    let dirPath = selectedDirectoryForFileTreeAction()
    guard !dirPath.isEmpty else { return }
    NameInputDialog.show(
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
        try FileManager.default.createDirectory(
          atPath: fullPath,
          withIntermediateDirectories: false)
      } catch {
        NSLog("MainWindow: failed to create folder at \(fullPath): \(error)")
        return
      }
      self.windowModel.onRefreshTree?()
    }
  }

  /// Toggles the sidebar visibility via NavigationSplitView's responder chain.
  func toggleSidebar() {
    setSidebarVisible(!windowModel.sidebarVisible)
  }

  private func setSidebarVisible(_ visible: Bool) {
    windowModel.sidebarVisible = visible
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

    customKeybindingMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) {
      [weak self] event in
      guard let self, self.window?.isKeyWindow == true else { return event }

      for kb in self.settings.customKeybindings {
        let parsed = Keybindings.parseShortcut(kb.key)
        guard !parsed.keyEquivalent.isEmpty else { continue }
        if Keybindings.eventMatchesShortcut(
          event, keyEquivalent: parsed.keyEquivalent, modifierFlags: parsed.modifierFlags)
        {
          self.executeCustomCommand(command: kb.command, args: kb.args)
          return nil  // consume the event
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
    let host = tabManager.contentView

    termSearchBar.translatesAutoresizingMaskIntoConstraints = false
    termSearchBar.wantsLayer = true
    termSearchBar.layer?.backgroundColor = theme.bgSurfaceColor.cgColor
    // Clip contents to the bar's bounds so the field/buttons are wiped into
    // view as it grows, instead of overflowing fully-formed while the height
    // animates (which read as a "pop"). The field + focus ring fit within the
    // 32pt open height, so this never clips them at rest.
    termSearchBar.layer?.masksToBounds = true
    termSearchBar.isHidden = true

    let separator = NSBox()
    separator.boxType = .custom
    separator.borderWidth = 0
    separator.fillColor = theme.borderColor
    separator.translatesAutoresizingMaskIntoConstraints = false

    termSearchField.translatesAutoresizingMaskIntoConstraints = false
    termSearchField.placeholderString = "Find in terminal"
    termSearchField.sendsSearchStringImmediately = true
    termSearchField.sendsWholeSearchString = false
    termSearchField.controlSize = .regular
    termSearchField.delegate = self
    termSearchField.font = NSFont.appFont(ofSize: 13)

    let prevButton = NSButton()
    prevButton.translatesAutoresizingMaskIntoConstraints = false
    prevButton.image = NSImage(
      systemSymbolName: "chevron.up", accessibilityDescription: "Previous Match")
    prevButton.bezelStyle = .rounded
    prevButton.isBordered = true
    prevButton.toolTip = "Previous Match"
    prevButton.target = self
    prevButton.action = #selector(termSearchPrev(_:))
    prevButton.setContentHuggingPriority(.defaultHigh, for: .horizontal)

    let nextButton = NSButton()
    nextButton.translatesAutoresizingMaskIntoConstraints = false
    nextButton.image = NSImage(
      systemSymbolName: "chevron.down", accessibilityDescription: "Next Match")
    nextButton.bezelStyle = .rounded
    nextButton.isBordered = true
    nextButton.toolTip = "Next Match"
    nextButton.target = self
    nextButton.action = #selector(termSearchNext(_:))
    nextButton.setContentHuggingPriority(.defaultHigh, for: .horizontal)

    let doneButton = NSButton(title: "Done", target: self, action: #selector(termSearchClose(_:)))
    doneButton.translatesAutoresizingMaskIntoConstraints = false
    doneButton.bezelStyle = .rounded
    doneButton.keyEquivalent = "\u{1b}"
    doneButton.toolTip = "Close Find Bar"
    doneButton.setContentHuggingPriority(.defaultHigh, for: .horizontal)

    termSearchBar.addSubview(separator)
    termSearchBar.addSubview(termSearchField)
    termSearchBar.addSubview(prevButton)
    termSearchBar.addSubview(nextButton)
    termSearchBar.addSubview(doneButton)

    host.addSubview(termSearchBar)

    let heightConstraint = termSearchBar.heightAnchor.constraint(equalToConstant: 0)
    termSearchHeightConstraint = heightConstraint

    NSLayoutConstraint.activate([
      termSearchBar.topAnchor.constraint(equalTo: host.topAnchor),
      termSearchBar.leadingAnchor.constraint(equalTo: host.leadingAnchor),
      termSearchBar.trailingAnchor.constraint(equalTo: host.trailingAnchor),
      heightConstraint,

      separator.leadingAnchor.constraint(equalTo: termSearchBar.leadingAnchor),
      separator.trailingAnchor.constraint(equalTo: termSearchBar.trailingAnchor),
      separator.bottomAnchor.constraint(equalTo: termSearchBar.bottomAnchor),
      separator.heightAnchor.constraint(equalToConstant: 1),

      termSearchField.leadingAnchor.constraint(equalTo: termSearchBar.leadingAnchor, constant: 10),
      termSearchField.centerYAnchor.constraint(equalTo: termSearchBar.centerYAnchor),
      termSearchField.widthAnchor.constraint(greaterThanOrEqualToConstant: 180),

      prevButton.leadingAnchor.constraint(equalTo: termSearchField.trailingAnchor, constant: 8),
      prevButton.centerYAnchor.constraint(equalTo: termSearchBar.centerYAnchor),
      prevButton.widthAnchor.constraint(equalToConstant: 28),

      nextButton.leadingAnchor.constraint(equalTo: prevButton.trailingAnchor, constant: 4),
      nextButton.centerYAnchor.constraint(equalTo: termSearchBar.centerYAnchor),
      nextButton.widthAnchor.constraint(equalToConstant: 28),

      doneButton.leadingAnchor.constraint(
        greaterThanOrEqualTo: nextButton.trailingAnchor, constant: 12),
      doneButton.trailingAnchor.constraint(equalTo: termSearchBar.trailingAnchor, constant: -10),
      doneButton.centerYAnchor.constraint(equalTo: termSearchBar.centerYAnchor),
    ])
  }

  /// Toggles the terminal search bar visibility.
  func toggleTerminalSearch() {
    guard tabManager.selectedTerminal != nil else {
      NSSound.beep()
      return
    }

    if termSearchBarVisible {
      hideTerminalSearch()
    } else {
      showTerminalSearch()
    }
  }

  private func showTerminalSearch() {
    termSearchBarVisible = true
    termSearchBar.isHidden = false
    // The active tab's view is added to `contentView` on every tab switch
    // (TabManager), so it sits above the search bar (added once at setup) and
    // occludes the field + buttons — only the field's focus ring, which draws
    // outside the view bounds, escapes. Re-raise the bar to the front so its
    // contents are visible.
    termSearchBar.superview?.addSubview(termSearchBar, positioned: .above, relativeTo: nil)
    termSearchBar.alphaValue = 0
    NSAnimationContext.runAnimationGroup(
      { context in
        context.duration = 0.2
        context.timingFunction = CAMediaTimingFunction(name: .easeOut)
        context.allowsImplicitAnimation = true
        termSearchHeightConstraint?.constant = 32
        termSearchBar.alphaValue = 1
        tabManager.contentView.layoutSubtreeIfNeeded()
      },
      completionHandler: { [weak self] in
        guard let self else { return }
        self.window?.makeFirstResponder(self.termSearchField)
        let editor = self.termSearchField.currentEditor() as? NSTextView
        editor?.selectAll(nil)
      })

    let query = termSearchField.stringValue
    if !query.isEmpty,
      let terminal = tabManager.selectedTerminal?.activeTerminal
    {
      terminal.search(query)
    }
  }

  /// Hides the terminal search bar, clears search state, and returns focus
  /// to the active terminal.
  private func hideTerminalSearch() {
    termSearchBarVisible = false
    if let terminal = tabManager.selectedTerminal?.activeTerminal {
      terminal.searchClear()
    }

    NSAnimationContext.runAnimationGroup(
      { context in
        context.duration = 0.16
        context.timingFunction = CAMediaTimingFunction(name: .easeIn)
        context.allowsImplicitAnimation = true
        termSearchHeightConstraint?.constant = 0
        termSearchBar.alphaValue = 0
        tabManager.contentView.layoutSubtreeIfNeeded()
      },
      completionHandler: { [weak self] in
        guard let self else { return }
        self.termSearchBar.isHidden = true
        self.termSearchBar.alphaValue = 1
        if let terminal = self.tabManager.selectedTerminal?.activeTerminal {
          terminal.focus()
        }
      })
  }

  @objc private func termSearchNext(_ sender: Any?) {
    tabManager.selectedTerminal?.activeTerminal?.searchNext()
  }

  @objc private func termSearchPrev(_ sender: Any?) {
    tabManager.selectedTerminal?.activeTerminal?.searchPrev()
  }

  @objc private func termSearchClose(_ sender: Any?) {
    hideTerminalSearch()
  }

  /// Updates the status bar with information from the currently active tab.
  func updateStatusBar() {
    guard let tabInfo = tabManager.activeTabInfo else { return }

    if let shellName = tabInfo.shellName {
      let cwd = tabInfo.cwd ?? NSHomeDirectory()
      // Sync to SwiftUI
      windowModel.shellName = shellName
      windowModel.currentCwd = cwd
      bindRepository(forDirectory: cwd)
      windowModel.cursorLine = nil
      windowModel.cursorCol = nil
      windowModel.currentLanguage = nil
      windowModel.currentIndent = nil
      windowModel.isPreviewable = false
      windowModel.isPreviewing = false
      let active = tabManager.selectedTerminal?.activeTerminal
      windowModel.commandRunning = active?.isCommandRunning ?? false
      windowModel.passwordInputActive = active?.isPasswordInput ?? false
      windowModel.lastCommandExitCode = active?.lastCommandExitCode
      windowModel.lastCommandDurationMs = active?.lastCommandDurationMs
      windowModel.terminalDirectInteraction = active?.isDirectInteraction ?? false
    } else if let language = tabInfo.language {
      let cwd = tabInfo.cwd ?? ""
      // Sync to SwiftUI
      windowModel.shellName = ""
      // An editor tab never owns a TUI — keep the status-bar pills interactive.
      windowModel.terminalDirectInteraction = false
      windowModel.currentCwd = cwd
      bindRepository(forDirectory: cwd)
      windowModel.cursorLine = tabInfo.cursorLine
      windowModel.cursorCol = tabInfo.cursorCol
      windowModel.currentLanguage = language
      windowModel.currentIndent =
        settings.useSpaces
        ? "Spaces: \(settings.tabWidth)" : "Tab Size: \(settings.tabWidth)"
      // Show/hide preview button based on file type
      if let editor = tabManager.selectedEditor,
        let fp = editor.filePath,
        EditorTab.isPreviewableFile(fp)
      {
        windowModel.isPreviewable = true
        windowModel.isPreviewing = editor.isPreviewing
      } else {
        windowModel.isPreviewable = false
        windowModel.isPreviewing = false
      }
    }
  }

  /// Re-applies theme colors to all child views.
  func handleThemeChange(_ newTheme: Theme) {
    theme = newTheme
    windowModel.theme = newTheme
    windowModel.iconCache = tabManager.iconCache

    // Switch window chrome between light and dark appearance
    window?.appearance = NSAppearance(named: newTheme.isLight ? .aqua : .darkAqua)

    // Toolbar buttons: flat icons for card-surface themes, bordered otherwise.
    let bordered = newTheme.surfaceStyle != "card"
    for item in window?.toolbar?.items ?? []
    where !(item is NSSearchToolbarItem) && !(item is NSTrackingSeparatorToolbarItem) {
      item.isBordered = bordered
    }

    // Window background — use bgSurface so the titlebar blends with the tab bar
    window?.backgroundColor = newTheme.bgSurfaceColor

    // Tab manager (propagates to all tabs)
    tabManager.applyTheme(newTheme)

    // Re-render previews with updated theme
    let themeJSON = ThemeManager.markdownThemeJSON(forName: newTheme.id)
    for tab in tabManager.allSurfaces {
      if case .editor(let editor) = tab {
        editor.refreshPreview(themeJSON: themeJSON, bgColor: newTheme.bg)
      }
    }
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

  /// Closes a whole tab (every pane), after confirming unsaved editors and
  /// running processes. Used by the tab strip's close button and menus.
  func requestCloseTab(index: Int) {
    guard index >= 0, index < tabManager.tabs.count else { return }

    // Confirm before closing pinned tabs
    if tabManager.pinnedTabs[index] {
      let alert = NSAlert()
      alert.messageText = "Pinned Tab"
      alert.informativeText = "This tab is pinned. Close anyway?"
      alert.alertStyle = .warning
      alert.addButton(withTitle: "Close")
      alert.addButton(withTitle: "Cancel")

      guard let window = self.window else { return }
      alert.beginSheetModal(for: window) { [weak self] response in
        guard let self else { return }
        if response == .alertFirstButtonReturn {
          // Unpin, then re-enter requestCloseTab for unsaved-changes handling
          self.tabManager.unpin(index: index)
          self.requestCloseTab(index: index)
        }
      }
      return
    }

    let tabView = tabManager.tabs[index].view
    let surfaces = tabManager.tabs[index].surfaces
    confirmClosing(surfaces) { [weak self] in
      guard let self,
        // Re-find the tab by identity — the index may be stale if other
        // tabs were closed while a confirmation was up.
        let currentIndex = self.tabManager.tabs.firstIndex(where: { $0.view === tabView })
      else { return }
      for surface in surfaces { self.willCloseSurface(surface) }
      self.tabManager.closeTab(index: currentIndex)
    }
  }

  /// ⌘W: closes the focused pane of a split tab, otherwise the whole tab.
  func requestCloseFocusedPane() {
    guard let split = tabManager.selectedSplit else {
      requestCloseTab(index: tabManager.selectedIndex)
      return
    }
    let surface = split.focused
    confirmClosing([surface]) { [weak self] in
      guard let self,
        let location = self.tabManager.locate(where: { $0.view === surface.view })
      else { return }
      self.willCloseSurface(surface)
      self.tabManager.closePane(location.paneID, inTabAt: location.tabIndex)
    }
  }

  /// Bookkeeping before a surface goes away.
  private func willCloseSurface(_ surface: TabEntry) {
    guard case .editor(let editor) = surface else { return }
    if let path = editor.filePath {
      untrackEditorTab(forPath: path)
    }
    lspDidClose(editor: editor)
  }

  /// Walks the surfaces' unsaved editors one sheet at a time, then confirms
  /// running terminal processes, then calls `proceed`. Cancelling anywhere
  /// stops.
  private func confirmClosing(_ surfaces: [TabEntry], proceed: @escaping () -> Void) {
    var dirty = surfaces.compactMap { surface -> EditorTab? in
      if case .editor(let editor) = surface, editor.isModified { return editor }
      return nil
    }
    let terminals = surfaces.compactMap { surface -> TerminalContainer? in
      if case .terminal(let container) = surface { return container }
      return nil
    }

    func confirmTerminals() {
      guard !terminals.isEmpty else {
        proceed()
        return
      }
      confirmClosingTerminalsIfNeeded(terminals) { shouldClose in
        if shouldClose { proceed() }
      }
    }

    func next() {
      guard !dirty.isEmpty else {
        confirmTerminals()
        return
      }
      let editor = dirty.removeFirst()
      confirmClosingEditor(editor) { shouldClose in
        if shouldClose { next() }
      }
    }
    next()
  }

  private func confirmClosingEditor(_ editor: EditorTab, completion: @escaping (Bool) -> Void) {
    let filename =
      editor.filePath.map {
        ($0 as NSString).lastPathComponent
      } ?? "Untitled"

    let alert = NSAlert()
    alert.messageText = "Unsaved Changes"
    alert.informativeText = "\"\(filename)\" has unsaved changes. Close anyway?"
    alert.alertStyle = .warning
    alert.addButton(withTitle: "Save & Close")
    alert.addButton(withTitle: "Don't Save")
    alert.addButton(withTitle: "Cancel")

    guard let window = self.window else {
      completion(false)
      return
    }
    alert.beginSheetModal(for: window) { [weak self] response in
      guard let self else { return }
      switch response {
      case .alertFirstButtonReturn:
        self.saveEditorTab(editor)
        completion(true)
      case .alertSecondButtonReturn:
        completion(true)
      default:
        completion(false)
      }
    }
  }

  private func confirmClosingTerminalsIfNeeded(
    _ containers: [TerminalContainer],
    completion: @escaping (Bool) -> Void
  ) {
    guard settings.confirmCloseWarnings else {
      completion(true)
      return
    }

    guard
      let summary = closeRiskSummary(
        action: .closeTab,
        unsavedEditorCount: 0,
        runningTerminalProcessCount: containers.reduce(0) {
          $0 + $1.runningDescendantProcessCount()
        },
        runningCommands: containers.flatMap { $0.runningCloseRiskCommands() }
      ),
      summary.hasRisk
    else {
      completion(true)
      return
    }

    let alert = NSAlert()
    alert.messageText = summary.title
    alert.informativeText = closeRiskInformativeText(summary)
    alert.alertStyle = .warning
    alert.addButton(withTitle: summary.destructiveActionTitle)
    alert.addButton(withTitle: summary.cancelTitle)

    guard let window = self.window else {
      completion(alert.runModal() == .alertFirstButtonReturn)
      return
    }

    alert.beginSheetModal(for: window) { response in
      completion(response == .alertFirstButtonReturn)
    }
  }

  /// Presents the standard per-document "Do you want to save the changes
  /// made to X?" sheet for a dirty editor and invokes `completion` with
  /// `true` if the user chose Save or Don't Save, `false` if they cancelled.
  /// Used by the quit flow to walk dirty tabs one at a time.
  func reviewAndSave(editor: EditorTab, completion: @escaping (Bool) -> Void) {
    let filename = editor.filePath.map { ($0 as NSString).lastPathComponent } ?? "Untitled"

    let alert = NSAlert()
    alert.messageText = "Do you want to save the changes made to \(filename)?"
    alert.informativeText = "Your changes will be lost if you don't save them."
    alert.alertStyle = .warning
    alert.addButton(withTitle: "Save")
    alert.addButton(withTitle: "Cancel")
    alert.addButton(withTitle: "Don't Save")

    guard let window = self.window else {
      completion(false)
      return
    }
    alert.beginSheetModal(for: window) { [weak self, weak editor] response in
      guard let self, let editor else {
        completion(false)
        return
      }
      switch response {
      case .alertFirstButtonReturn:
        if editor.filePath != nil {
          editor.fetchContentAndSave { success in completion(success) }
        } else {
          self.showSaveAsDialog(for: editor) { success in completion(success) }
        }
      case .alertThirdButtonReturn:
        completion(true)
      default:
        completion(false)
      }
    }
  }

  // MARK: - Notification Observers

  private func ownedEditor(from notification: Notification) -> EditorTab? {
    guard let editor = notification.object as? EditorTab,
      tabManager.ownsEditor(editor)
    else {
      return nil
    }
    return editor
  }

  private static func lineNumber(from userInfo: [AnyHashable: Any]?) -> UInt32? {
    if let line = userInfo?["line"] as? UInt32 {
      return line
    }
    if let line = userInfo?["line"] as? Int, line > 0 {
      return UInt32(line)
    }
    return nil
  }

  private func openCommandPaletteSearchResult(path: String, line: UInt32?, column: UInt32? = nil) {
    let column = line == nil ? nil : (column ?? 1)
    tabManager.addEditorTab(
      path: path,
      projectDirectory: fileTreeRootPath,
      goToLine: line,
      goToColumn: column
    )
    if let editor = findEditorTab(forPath: path) {
      trackEditorTab(editor, forPath: path)
      lspDidOpenIfNeeded(path: path)
      if let line, let column {
        editor.goToPosition(line: line, column: column)
      }
    }
  }

  private func showSearchSidebarAndFocus() {
    setSidebarVisible(true)
    // Enter search mode and ask the in-sidebar search field to take focus.
    windowModel.beginSearch()
  }

  private func setupNotificationObservers() {
    let nc = NotificationCenter.default

    notificationObservers.append(
      nc.addObserver(forName: .impulseToggleSidebar, object: nil, queue: .main) { [weak self] _ in
        guard self?.window?.isKeyWindow == true else { return }
        self?.toggleSidebar()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseNewTerminalTab, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.tabManager.addTerminalTab()
        // If a directory was specified (e.g. "Open in Terminal" from file tree),
        // navigate the new terminal to that directory.
        if let dir = notification.userInfo?["directory"] as? String,
          let container = self.tabManager.selectedTerminal,
          let terminal = container.activeTerminal
        {
          terminal.sendCommand("cd \(dir.shellEscaped)")
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseNewFile, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        let cwd = self.getActiveCwd()
        self.tabManager.addUntitledEditorTab(cwd: cwd)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseCloseTab, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        let index = self.tabManager.selectedIndex
        guard index >= 0, index < self.tabManager.tabs.count else { return }
        self.requestCloseFocusedPane()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseReopenTab, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.tabManager.reopenLastClosedTab()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseActiveTabDidChange, object: self.tabManager, queue: .main) {
        [weak self] _ in
        guard let self else { return }
        // Close the window when the last tab is closed (covers tab bar X button,
        // context menu "Close Tab", etc.).
        if self.tabManager.tabs.isEmpty {
          self.window?.close()
          return
        }
        self.updateStatusBar()
        self.attachInputBar()
        // Rebuild the file tree when the active tab's directory context differs
        // from the current root. Terminal tabs use their CWD; editor tabs use
        // the parent directory of the open file.
        if let tab = self.tabManager.selectedTab?.focused {
          let dir: String?
          switch tab {
          case .terminal(let container):
            dir = container.activeTerminal?.currentWorkingDirectory
          case .editor(let editor):
            dir = editor.projectDirectory
          case .imagePreview, .split:
            dir = nil
          case .diffReview(let repoRoot, _):
            dir = repoRoot
          }
          if self.followsActiveDirectory, let dir, !dir.isEmpty, dir != self.fileTreeRootPath {
            self.switchFileTreeRoot(dir, updateStatusBar: false)
          }
        }
        // Refresh git diff decorations for the newly-active editor tab
        // (they may be stale after terminal git operations).
        if let editor = self.tabManager.selectedEditor {
          self.applyGitDiffDecorations(editor: editor)
        }
        // Hide terminal search bar when switching away from a terminal tab.
        if self.termSearchBarVisible {
          if self.tabManager.selectedTerminal == nil {
            self.hideTerminalSearch()
          } else {
            // Still on a terminal: the freshly-shown tab view was added above
            // the search bar, so re-raise the bar to keep it visible.
            self.termSearchBar.superview?.addSubview(
              self.termSearchBar, positioned: .above, relativeTo: nil)
          }
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseOpenFile, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true else { return }
        if let path = notification.userInfo?["path"] as? String {
          var isDirectory: ObjCBool = false
          if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory),
            isDirectory.boolValue
          {
            self.tabManager.openWorkspace(folder: path)
            return
          }
          let line = Self.lineNumber(from: notification.userInfo)
          let column = (notification.userInfo?["column"] as? Int).map { UInt32(max(1, $0)) }
          self.tabManager.addEditorTab(
            path: path,
            projectDirectory: self.fileTreeRootPath,
            goToLine: line,
            goToColumn: line == nil ? nil : (column ?? 1),
            beside: notification.userInfo?["beside"] as? Bool ?? false
          )
          // Navigate to specific line if provided (e.g. from search results).
          if let editor = self.findEditorTab(forPath: path) {
            self.trackEditorTab(editor, forPath: path)
            self.lspDidOpenIfNeeded(path: path)
            if let line {
              editor.goToPosition(line: line, column: column ?? 1)
            }
          }
        }
      }
    )
    // Apply git diff decorations once Monaco confirms it has processed
    // the OpenFile command and set up the model. This avoids the race
    // condition where decorations arrive before the model is ready.
    notificationObservers.append(
      nc.addObserver(forName: .editorFileOpened, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        if let editor = notification.object as? EditorTab {
          guard self.tabManager.ownsEditor(editor) else { return }
          // Send LSP didOpen now that the tab and Monaco model are ready.
          if let path = editor.filePath {
            self.trackEditorTab(editor, forPath: path)
            self.lspDidOpenIfNeeded(path: path)
          }
          self.applyGitDiffDecorations(editor: editor)
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseReloadEditorFile, object: nil, queue: .main) {
        [weak self] notification in
        // Not gated on key window: the file changed on disk, so every window
        // with it open must reload.
        guard let self else { return }
        if let path = notification.userInfo?["path"] as? String {
          // Find the open editor tab for this file and reload from disk.
          // Reading a single source file is fast enough to do synchronously
          // on the main thread, and avoids the delayed repaint caused by
          // dispatching back from a background queue.
          if let editor = self.findEditorTab(forPath: path) {
            do {
              let content = try String(contentsOfFile: path, encoding: .utf8)
              editor.openFile(path: path, content: content, language: editor.language)
              editor.webView?.setNeedsDisplay(editor.webView?.bounds ?? .zero)
            } catch {
              os_log(
                .error, "Failed to reload file '%{public}@': %{public}@",
                path, error.localizedDescription)
            }
          }
        }
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseFindInProject, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showPalette(prefix: "")
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseActiveWorkspaceDidChange, object: tabManager, queue: .main) {
        [weak self] _ in
        self?.activeWorkspaceDidChange()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseShowCommandHistory, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        self.showPalette(prefix: "h:")
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseSwitchWorkspace, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showPalette(prefix: "w:")
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseOpenWorkspace, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.presentOpenWorkspacePanel()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulsePaneCommand, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true,
          let command = notification.userInfo?["command"] as? String
        else { return }
        self.performPaneCommand(command)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseToggleRightDock, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.toggleRightDock()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseShowChanges, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showChangesPanel()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseSwitchBranch, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showBranchSwitcher()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalCommandBlockChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let tab = notification.object as? TerminalTab,
          self.tabManager.selectedTerminal?.activeTerminal === tab
        else { return }
        self.updateStatusBar()
        self.tabManager.syncToWindowModel()
      })
    notificationObservers.append(
      nc.addObserver(forName: .terminalInteractionModeChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let tab = notification.object as? TerminalTab,
          self.tabManager.selectedTerminal?.activeTerminal === tab,
          let interactive = notification.userInfo?["interactive"] as? Bool
        else { return }
        self.windowModel.terminalDirectInteraction = interactive
        // Re-sync tab subtitles so the working folder appears alongside the
        // branch while a program/TUI owns the grid.
        self.tabManager.syncToWindowModel()
        // Leaving a TUI: the input bar reappears and should reclaim focus
        // (unless it's disabled, in which case the grid keeps the keyboard).
        if !interactive {
          if tab.wantsGridFocus {
            tab.focus()
          } else {
            self.windowModel.inputBarFocusToken += 1
          }
        }
      })
    notificationObservers.append(
      nc.addObserver(forName: .terminalRequestInputFocus, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let tab = notification.object as? TerminalTab,
          self.tabManager.selectedTerminal?.activeTerminal === tab
        else { return }
        self.windowModel.inputBarFocusToken += 1
      })
    notificationObservers.append(
      nc.addObserver(forName: .terminalPasswordInputChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let tab = notification.object as? TerminalTab,
          self.tabManager.selectedTerminal?.activeTerminal === tab,
          let active = notification.userInfo?["active"] as? Bool
        else { return }
        self.windowModel.passwordInputActive = active
        // Focus is re-grabbed by the input bar itself: it must wait out the
        // plain↔secure field swap, so a token bump here would fire too early.
      })
    notificationObservers.append(
      nc.addObserver(forName: .terminalCwdChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        if let dir = notification.userInfo?["directory"] as? String {
          self.settings.lastDirectory = dir
          let isSelected = self.tabManager.selectedTerminal?.activeTerminal === terminal
          if dir == self.fileTreeRootPath || !self.followsActiveDirectory || !isSelected {
            // Same directory — just refresh git status (a command
            // may have changed git state without changing CWD).
            self.fileTreeData.refreshGitStatus()
            self.windowModel.repository?.refresh()
          } else {
            self.switchFileTreeRoot(dir)
          }
          // Push the new CWD/branch into the status bar right away
          // if this terminal is the selected one; otherwise the
          // status bar would lag until the next explicit refresh.
          if let selected = self.tabManager.selectedTerminal?.activeTerminal,
            selected === terminal
          {
            self.updateStatusBar()
          }
        }
      }
    )

    // Command palette
    notificationObservers.append(
      nc.addObserver(forName: .impulseShowCommandPalette, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showPalette(prefix: ">")
      }
    )

    // Quick Open — show sidebar in search mode
    notificationObservers.append(
      nc.addObserver(forName: .impulseQuickOpen, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showSearchSidebarAndFocus()
      }
    )

    // Update available — surface the updater on the visible SwiftUI status bar.
    notificationObservers.append(
      nc.addObserver(forName: .impulseUpdateAvailable, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        self.windowModel.updateAvailableVersion = notification.userInfo?["version"] as? String
        self.windowModel.updateCurrentVersion = notification.userInfo?["currentVersion"] as? String
        if let urlString = notification.userInfo?["url"] as? String {
          self.windowModel.updateURL = URL(string: urlString)
        } else {
          self.windowModel.updateURL = nil
        }
      }
    )

    // Install LSP — install managed web LSP servers
    notificationObservers.append(
      nc.addObserver(forName: .impulseInstallLsp, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        DispatchQueue.global(qos: .userInitiated).async {
          let result = ImpulseCore.lspInstall()
          DispatchQueue.main.async {
            let alert = NSAlert()
            switch result {
            case .success(let path):
              alert.messageText = "LSP Servers Installed"
              alert.informativeText = "Web LSP servers installed to \(path)"
              alert.alertStyle = .informational
            case .failure(let error):
              alert.messageText = "LSP Install Failed"
              alert.informativeText = error.message
              alert.alertStyle = .warning
            }
            alert.addButton(withTitle: "OK")
            alert.runModal()
          }
        }
      }
    )

    // Save file — fired from menu Cmd+S or from EditorTab's SaveRequested event
    notificationObservers.append(
      nc.addObserver(forName: .impulseSaveFile, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }

        // If the notification came from an EditorTab (Monaco Cmd+S path),
        // save that specific editor directly — no key-window check needed
        // because the editor itself initiated the save.
        if notification.object is EditorTab {
          guard let sourceEditor = self.ownedEditor(from: notification) else { return }
          self.saveEditorTab(sourceEditor)
          return
        }

        // Menu path: save the currently selected editor tab.
        guard self.window?.isKeyWindow == true else { return }
        if let editor = self.tabManager.selectedEditor {
          self.saveEditorTab(editor)
        }
      }
    )

    // Find — editor: Monaco find widget; terminal: search bar toggle
    notificationObservers.append(
      nc.addObserver(forName: .impulseFind, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        guard let tab = self.tabManager.selectedTab?.focused else { return }
        switch tab {
        case .editor(let editor):
          editor.webView?.evaluateJavaScript(
            "editor.getAction('actions.find').run()",
            completionHandler: nil
          )
        case .terminal:
          self.toggleTerminalSearch()
        case .imagePreview, .diffReview, .split:
          break
        }
      }
    )

    // Editor cursor position tracking for the status bar
    notificationObservers.append(
      nc.addObserver(forName: .editorCursorMoved, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let line = notification.userInfo?["line"] as? UInt32,
          let col = notification.userInfo?["column"] as? UInt32
        else { return }
        // Only update if the notification came from the active editor tab
        guard let editor = self.ownedEditor(from: notification),
          editor === self.tabManager.selectedEditor
        else { return }
        let filePath = editor.filePath ?? ""
        let cwd =
          editor.projectDirectory
          ?? (filePath as NSString).deletingLastPathComponent
        // Sync to SwiftUI
        self.windowModel.cursorLine = Int(line)
        self.windowModel.cursorCol = Int(col)
        self.windowModel.currentCwd = cwd
        self.bindRepository(forDirectory: cwd)
        self.windowModel.currentLanguage = editor.language
      }
    )

    // Terminal title changed — update tab segment labels
    notificationObservers.append(
      nc.addObserver(forName: .terminalTitleChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        self.tabManager.refreshSegmentLabels()
      }
    )

    // Terminal attention changed — update tab indicators
    notificationObservers.append(
      nc.addObserver(forName: .terminalAttentionChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        if !terminal.needsAttention { DesktopNotifier.shared.clear(terminalID: terminal.id) }
        self.tabManager.refreshSegmentLabels()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalWantsNotification, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, !NSApp.isActive,
          let terminal = notification.object as? TerminalTab,
          let location = self.tabManager.location(ofTerminal: terminal),
          let workspaceID = self.tabManager.workspaceID(ofTabAt: location.tabIndex)
        else { return }
        let workspace = self.tabManager.workspace(workspaceID)
        DesktopNotifier.shared.post(
          title: notification.userInfo?["title"] as? String ?? terminal.tabTitle,
          subtitle: [workspace?.name, terminal.tabTitle].compactMap { $0 }.joined(separator: " · "),
          body: notification.userInfo?["body"] as? String ?? "",
          terminalID: terminal.id, thread: workspaceID.uuidString)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: DesktopNotifier.revealTerminal, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let id = (notification.userInfo?["terminal"] as? String).flatMap(UUID.init)
        else { return }
        self.revealTerminal(id: id)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalAgentChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        self.tabManager.refreshSegmentLabels()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseNextAgent, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.revealNextWaitingAgent()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .editorGitAction, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let editor = self.ownedEditor(from: notification),
          let action = notification.userInfo?["action"] as? String,
          let line = notification.userInfo?["line"] as? Int
        else { return }
        self.handleEditorGitAction(editor: editor, action: action, line: line)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .editorDirtyStateChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let editor = notification.object as? EditorTab,
          self.tabManager.ownsEditor(editor)
        else { return }
        self.tabManager.refreshSegmentLabels()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .terminalProgressChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let terminal = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminal)
        else { return }
        self.tabManager.refreshSegmentLabels()
      }
    )

    // Terminal process terminated — close the tab.
    notificationObservers.append(
      nc.addObserver(forName: .terminalProcessTerminated, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let terminalTab = notification.object as? TerminalTab,
          self.tabManager.ownsTerminal(terminalTab)
        else { return }
        if let location = self.tabManager.locate(where: {
          if case .terminal(let container) = $0 {
            return container.terminals.contains { $0 === terminalTab }
          }
          return false
        }) {
          self.tabManager.closePane(location.paneID, inTabAt: location.tabIndex)
        }
      }
    )

    // Editor content changed — refresh tab labels and notify LSP
    notificationObservers.append(
      nc.addObserver(forName: .editorContentChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        guard let editor = self.ownedEditor(from: notification) else { return }
        self.tabManager.refreshSegmentLabels()
        let changes = notification.userInfo?["changes"] as? [MonacoContentChange] ?? []
        self.lspDidChange(editor: editor, changes: changes)
      }
    )

    // Editor focus changed — auto-save on focus loss if enabled
    notificationObservers.append(
      nc.addObserver(forName: .editorFocusChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.settings.autoSave else { return }
        guard let editor = self.ownedEditor(from: notification) else { return }
        guard let focused = notification.userInfo?["focused"] as? Bool, !focused else { return }
        guard editor.isModified else { return }
        self.saveEditorTab(editor)
      }
    )

    // Custom keybinding command execution
    notificationObservers.append(
      nc.addObserver(forName: Notification.Name("impulseCustomCommand"), object: nil, queue: .main)
      { [weak self] notification in
        guard let self, self.window?.isKeyWindow == true else { return }
        guard let command = notification.userInfo?["command"] as? String,
          !command.isEmpty
        else { return }
        let args = notification.userInfo?["args"] as? [String] ?? []
        self.executeCustomCommand(command: command, args: args)
      }
    )

    // LSP: completion requested
    notificationObservers.append(
      nc.addObserver(forName: .editorCompletionRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleCompletionRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // LSP: hover requested
    notificationObservers.append(
      nc.addObserver(forName: .editorHoverRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleHoverRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // Go to line
    notificationObservers.append(
      nc.addObserver(forName: .impulseGoToLine, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.showGoToLineDialog()
      }
    )

    // Toggle Preview (Markdown / SVG)
    notificationObservers.append(
      nc.addObserver(forName: .impulseToggleMarkdownPreview, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.togglePreview()
      }
    )

    // Review Changes — open the git diff review tab for the current workspace.
    notificationObservers.append(
      nc.addObserver(forName: .impulseReviewChanges, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.openDiffReview()
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
        self.cycleTab(by: 1)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulsePrevTab, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.cycleTab(by: -1)
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .impulseSelectTab, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true else { return }
        guard let index = notification.userInfo?["index"] as? Int else { return }
        // ⌘1…⌘9 count tabs in the active workspace.
        let visible = self.tabManager.visibleTabIndices
        if visible.indices.contains(index) {
          self.tabManager.selectTab(index: visible[index])
        }
      }
    )

    // Settings changed (from SettingsWindow)
    notificationObservers.append(
      nc.addObserver(forName: .impulseSettingsDidChange, object: nil, queue: .main) {
        [weak self] notification in
        guard let self else { return }
        self.applyAllSettings()
        // Rebuild custom keybinding monitor so new/changed bindings take effect.
        self.setupCustomKeybindingMonitor()
      }
    )

    // LSP: go-to-definition requested
    notificationObservers.append(
      nc.addObserver(forName: .editorDefinitionRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleDefinitionRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // Monaco: cross-file navigation (fired by registerEditorOpener on actual click)
    notificationObservers.append(
      nc.addObserver(forName: .editorOpenFileRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          self.ownedEditor(from: notification) != nil,
          let uri = notification.userInfo?["uri"] as? String,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleOpenFileRequested(uri: uri, line: line, character: character)
      }
    )

    // LSP: formatting requested
    notificationObservers.append(
      nc.addObserver(forName: .editorFormattingRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let tabSize = notification.userInfo?["tabSize"] as? UInt32,
          let insertSpaces = notification.userInfo?["insertSpaces"] as? Bool
        else { return }
        self.handleFormattingRequest(
          editor: editor, requestId: requestId, tabSize: tabSize, insertSpaces: insertSpaces)
      }
    )

    // LSP: signature help requested
    notificationObservers.append(
      nc.addObserver(forName: .editorSignatureHelpRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleSignatureHelpRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // LSP: references requested
    notificationObservers.append(
      nc.addObserver(forName: .editorReferencesRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handleReferencesRequest(
          editor: editor, requestId: requestId, line: line, character: character)
      }
    )

    // LSP: code action requested
    notificationObservers.append(
      nc.addObserver(forName: .editorCodeActionRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let startLine = notification.userInfo?["startLine"] as? UInt32,
          let startColumn = notification.userInfo?["startColumn"] as? UInt32,
          let endLine = notification.userInfo?["endLine"] as? UInt32,
          let endColumn = notification.userInfo?["endColumn"] as? UInt32
        else { return }
        let diagnostics = notification.userInfo?["diagnostics"] as? [[String: Any]] ?? []
        self.handleCodeActionRequest(
          editor: editor, requestId: requestId, startLine: startLine, startColumn: startColumn,
          endLine: endLine, endColumn: endColumn, diagnostics: diagnostics)
      }
    )

    // LSP: rename requested
    notificationObservers.append(
      nc.addObserver(forName: .editorRenameRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32,
          let newName = notification.userInfo?["newName"] as? String
        else { return }
        self.handleRenameRequest(
          editor: editor, requestId: requestId, line: line, character: character, newName: newName)
      }
    )

    // LSP: prepare rename requested
    notificationObservers.append(
      nc.addObserver(forName: .editorPrepareRenameRequested, object: nil, queue: .main) {
        [weak self] notification in
        guard let self,
          let editor = self.ownedEditor(from: notification),
          let requestId = notification.userInfo?["requestId"] as? UInt64,
          let line = notification.userInfo?["line"] as? UInt32,
          let character = notification.userInfo?["character"] as? UInt32
        else { return }
        self.handlePrepareRenameRequest(
          editor: editor, requestId: requestId, line: line, character: character)
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
    guard let path = editor.filePath else {
      showSaveAsDialog(for: editor)
      return
    }

    // Fetch the latest content from Monaco (content changes are debounced
    // in JS, so the Swift property may be stale when saving via menu Cmd+S).
    editor.fetchContentAndSave { [weak self, weak editor] success in
      guard let self, let editor, success else { return }

      // Format on save — find applicable formatter
      let formatter = self.resolveFormatOnSave(forPath: path)
      if let fmt = formatter, !fmt.command.isEmpty {
        self.runExternalCommand(
          command: fmt.command, args: fmt.args, cwd: (path as NSString).deletingLastPathComponent
        ) { [weak self, weak editor] in
          guard let self, let editor else { return }
          // Reload the file after formatting off the main thread.
          let language = editor.language
          let currentContent = editor.content
          DispatchQueue.global(qos: .userInitiated).async {
            let newContent: String
            do {
              newContent = try String(contentsOfFile: path, encoding: .utf8)
            } catch {
              os_log(
                .error, "Failed to reload file after formatting '%{public}@': %{public}@",
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
    applyGitDiffDecorations(editor: editor)
    // Direct git status refresh (skip the debounce — saves are explicit
    // user actions that warrant immediate feedback).
    let nodes = fileTreeData.rootNodes
    let root = fileTreeData.rootPath
    if !root.isEmpty {
      DispatchQueue.global(qos: .userInitiated).async {
        FileTreeNode.refreshGitStatus(nodes: nodes, repoPath: root, dirPath: root)
      }
    }

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
              os_log(
                .error, "Failed to reload file after command-on-save '%{public}@': %{public}@",
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

  /// Shows a save-as dialog for an untitled editor tab, then transitions it
  /// to a file-backed editor on successful save. The optional completion is
  /// called with `true` if the user saved, `false` if the panel was cancelled
  /// or the save failed.
  private func showSaveAsDialog(for editor: EditorTab, completion: ((Bool) -> Void)? = nil) {
    let panel = NSSavePanel()
    panel.nameFieldStringValue = "Untitled"
    panel.canCreateDirectories = true

    if let cwd = editor.untitledCwd ?? editor.projectDirectory {
      panel.directoryURL = URL(fileURLWithPath: cwd)
    }

    guard let window = self.window else {
      completion?(false)
      return
    }
    panel.beginSheetModal(for: window) { [weak self, weak editor] response in
      guard let self, let editor, response == .OK, let url = panel.url else {
        completion?(false)
        return
      }

      let chosenPath = url.path

      // Set filePath first so fetchContentAndSave writes to the correct location.
      editor.filePath = chosenPath

      editor.fetchContentAndSave { [weak self, weak editor] success in
        guard let self, let editor, success else {
          completion?(false)
          return
        }

        // Transition to file-backed editor: re-open in Monaco with correct URI and language
        let language = self.tabManager.detectLanguage(forPath: chosenPath)
        editor.untitledCwd = nil
        editor.projectDirectory = (chosenPath as NSString).deletingLastPathComponent
        editor.openFile(path: chosenPath, content: editor.content, language: language)

        // Register in dedup set
        self.tabManager.registerOpenFilePath(chosenPath)

        // Post-save actions (refresh tab bar, LSP didOpen, git diff, etc.)
        self.postSaveActions(editor: editor, path: chosenPath)

        // Track the editor tab
        self.trackEditorTab(editor, forPath: chosenPath)
        self.lspDidOpenIfNeeded(path: chosenPath)

        completion?(true)
      }
    }
  }

  /// Resolves the `FormatOnSave` configuration for a file path, checking
  /// file-type overrides first, then falling back to the global setting.
  private func resolveFormatOnSave(forPath path: String) -> FormatOnSave? {
    // Check file-type-specific overrides first
    for override_ in settings.fileTypeOverrides {
      if Settings.matchesFilePattern(path, pattern: override_.pattern),
        let fmt = override_.formatOnSave, !fmt.command.isEmpty
      {
        return fmt
      }
    }
    return nil
  }

  /// Runs an external command asynchronously. Calls `completion` on the main
  /// thread when the process finishes.
  ///
  /// The command name is validated to be either an absolute path or a plain
  /// executable name (letters, digits, `-`, `_`, `.` only). Arguments and
  /// `cwd` must not contain null bytes. Failures are logged and `completion`
  /// is still invoked so the caller's control flow continues.
  private func runExternalCommand(
    command: String, args: [String], cwd: String,
    completion: (() -> Void)?
  ) {
    guard Self.isSafeExternalCommand(command) else {
      NSLog("Refusing to run command with unsafe name: %@", command)
      if let completion = completion { DispatchQueue.main.async { completion() } }
      return
    }
    guard !cwd.contains("\0"), args.allSatisfy({ !$0.contains("\0") }) else {
      NSLog("Refusing to run command with null byte in args/cwd")
      if let completion = completion { DispatchQueue.main.async { completion() } }
      return
    }

    DispatchQueue.global(qos: .userInitiated).async {
      let process = Process()
      if command.hasPrefix("/") {
        // Absolute path: invoke directly, skip PATH lookup via env.
        process.executableURL = URL(fileURLWithPath: command)
        process.arguments = args
      } else {
        // Bare name: use env to honor PATH. Name has been validated.
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = [command] + args
      }
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

  /// Validates a command name for `runExternalCommand`. Absolute paths are
  /// permitted (but must not contain `..`); bare names must be a plain
  /// identifier — no slashes, shell metacharacters, or leading dashes.
  private static func isSafeExternalCommand(_ command: String) -> Bool {
    if command.isEmpty || command.contains("\0") { return false }
    if command.hasPrefix("/") {
      return !command.contains("..")
    }
    if command == "." || command == ".." { return false }
    let allowed = CharacterSet(
      charactersIn:
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.")
    return command.unicodeScalars.allSatisfy { allowed.contains($0) }
  }

  // MARK: - Custom Command Execution

  /// Executes a custom keybinding command by opening a new terminal tab
  /// with the command running in it, using the active tab's working directory.
  private func executeCustomCommand(command: String, args: [String]) {
    let fullCommand = ([command.shellEscaped] + args.map(\.shellEscaped)).joined(separator: " ")

    // Get the CWD from the active tab (terminal CWD or editor file's parent)
    let cwd = getActiveCwd()

    // Pass the command through so it's sent right after the shell process
    // starts (shell spawn is deferred to the next run loop tick for layout).
    tabManager.addTerminalTab(directory: cwd, initialCommand: fullCommand)
  }

  /// Returns the current working directory from the active tab:
  /// terminal CWD, or the parent directory of the active editor file.
  private func getActiveCwd() -> String? {
    guard let tab = tabManager.selectedTab?.focused else { return nil }
    switch tab {
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
    case .diffReview(let repoRoot, _):
      return repoRoot.isEmpty ? nil : repoRoot
    case .split:
      break
    }
    return nil
  }

  // MARK: - Review Changes

  /// Opens the review for the active repository (resolving it from the active
  /// tab's directory or the file-tree root when needed). Outside a git
  /// repository, explains why instead.
  private func openDiffReview() {
    if let repository = windowModel.repository {
      tabManager.addReviewTab(repository: repository, host: self)
      return
    }
    let candidate = getActiveCwd() ?? fileTreeRootPath
    guard !candidate.isEmpty else {
      presentNotAGitRepoAlert()
      return
    }
    GitRepositoryStore.shared.resolve(directory: candidate) { [weak self] state in
      guard let self else { return }
      guard let state else {
        self.presentNotAGitRepoAlert()
        return
      }
      self.tabManager.addReviewTab(repository: state, host: self)
    }
  }

  private func presentNotAGitRepoAlert() {
    let alert = NSAlert()
    alert.messageText = "Not a Git Repository"
    alert.informativeText =
      "The current workspace is not inside a git repository, so there are no changes to review."
    alert.alertStyle = .informational
    alert.addButton(withTitle: "OK")
    if let window = self.window {
      alert.beginSheetModal(for: window, completionHandler: nil)
    } else {
      alert.runModal()
    }
  }

  // MARK: - Go to Line

  /// Shows a dialog asking for a line number and navigates the active editor to it.
  // MARK: - Markdown Preview

  @objc private func previewButtonClicked(_ sender: Any?) {
    togglePreview()
  }

  /// Toggle preview for the active editor tab (markdown or SVG).
  private func togglePreview() {
    guard let editor = tabManager.selectedEditor,
      let fp = editor.filePath,
      EditorTab.isPreviewableFile(fp)
    else { return }

    let themeJSON = ThemeManager.markdownThemeJSON(forName: theme.id)
    if let isPreviewing = editor.togglePreview(themeJSON: themeJSON, bgColor: theme.bg) {
      windowModel.isPreviewing = isPreviewing
    }
  }

  private func showGoToLineDialog() {
    guard let editor = tabManager.selectedEditor else { return }

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
    applyFontSizeToAllTabs()
  }

  /// Resets font sizes to defaults (14 for both editor and terminal).
  private func resetFontSize() {
    settings.fontSize = 14
    settings.terminalFontSize = 14
    applyFontSizeToAllTabs()
  }

  /// Applies the current font size settings to all open tabs.
  private func applyFontSizeToAllTabs() {
    // Build EditorOptions with updated font size from our local settings,
    // since TabManager's settings copy may not reflect the change yet.
    let editorOptions = EditorOptions(
      fontSize: UInt32(settings.fontSize),
      fontFamily: settings.fontFamily
    )
    let termSettings = settings.terminalSettings()

    for tab in tabManager.allSurfaces {
      switch tab {
      case .editor(let editor):
        editor.applySettings(editorOptions)
      case .terminal(let container):
        container.applySettings(settings: termSettings)
      case .imagePreview, .diffReview, .split:
        break
      }
    }
  }

  // MARK: - Apply All Settings

  /// Re-applies all settings to every open tab. Called when settings change
  /// via the preferences window.
  private func applyAllSettings() {
    let editorOptions = tabManager.editorOptionsFromSettings()
    let termSettings = settings.terminalSettings()

    for tab in tabManager.allSurfaces {
      switch tab {
      case .editor(let editor):
        editor.applySettings(editorOptions)
      case .terminal(let container):
        container.applySettings(settings: termSettings)
      case .imagePreview, .diffReview, .split:
        break
      }
    }

    // Re-apply tab strip placement (sidebar vertical list vs top bar).
    if windowModel.tabBarPosition != settings.tabBarPosition {
      windowModel.tabBarPosition = settings.tabBarPosition
    }
    if windowModel.contextBarEnabled != settings.terminalContextBar {
      windowModel.contextBarEnabled = settings.terminalContextBar
    }

    // Re-apply sidebar show-hidden preference.
    if windowModel.showHiddenFiles != settings.sidebarShowHidden {
      windowModel.showHiddenFiles = settings.sidebarShowHidden
      fileTreeData.showHidden = settings.sidebarShowHidden
      if !fileTreeRootPath.isEmpty {
        windowModel.onRefreshTree?()
      }
    }
  }

  // MARK: - File Tree Expansion Helpers

  /// Collects paths of all expanded directories in the tree.
  private static func collectExpandedPaths(_ nodes: [FileTreeNode]) -> Set<String> {
    var paths = Set<String>()
    for node in nodes {
      if node.isDirectory && node.isExpanded {
        paths.insert(node.path)
        if let children = node.children {
          paths.formUnion(collectExpandedPaths(children))
        }
      }
    }
    return paths
  }

  /// Restores expanded state for directories whose paths are in the set.
  /// Loads children for expanded dirs so the tree shows content.
  private static func restoreExpandedPaths(
    _ paths: Set<String>, in nodes: [FileTreeNode], showHidden: Bool
  ) {
    for node in nodes where node.isDirectory && paths.contains(node.path) {
      node.isExpanded = true
      if !node.isLoaded {
        node.loadChildren(showHidden: showHidden)
      }
      if let children = node.children {
        restoreExpandedPaths(paths, in: children, showHidden: showHidden)
      }
    }
  }

  // MARK: - File Tree Root Switching

  /// Switches the sidebar file tree to a new directory. Caches the current
  /// tree, shows a cached tree instantly if available, then rebuilds from
  /// disk on a background queue and updates the status bar with the git
  /// branch.
  private func switchFileTreeRoot(_ dir: String, updateStatusBar: Bool = true) {
    // Cache current tree before switching away.
    if !fileTreeRootPath.isEmpty {
      fileTreeCacheInsert(key: fileTreeRootPath, nodes: fileTreeData.rootNodes)
    }
    fileTreeRootPath = dir
    windowModel.fileTreeRootPath = dir
    // Drop any active search and its results so stale matches from the old
    // root don't linger against the new project.
    windowModel.resetSearch()

    let shellName = LoginShell.defaultShellName()
    if updateStatusBar {
      windowModel.currentCwd = dir
      windowModel.shellName = shellName
      bindRepository(forDirectory: dir)
    }

    // Show cached tree instantly if available. Skip git refresh since the
    // background rebuild below will fetch fresh git status anyway.
    if let cached = fileTreeCache[dir] {
      fileTreeData.updateTree(nodes: cached, rootPath: dir, skipGitRefresh: true)
      fileTreeCacheTouch(key: dir)
      windowModel.updateFileTree(cached, rootPath: dir)
    }

    // Refresh from disk in the background.
    let showHidden = fileTreeData.showHidden
    DispatchQueue.global(qos: .userInitiated).async {
      let nodes = FileTreeNode.buildTree(rootPath: dir, showHidden: showHidden)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard self.fileTreeRootPath == dir else { return }
        self.fileTreeData.updateTree(nodes: nodes, rootPath: dir)
        self.fileTreeCacheInsert(key: dir, nodes: nodes)
        self.windowModel.updateFileTree(nodes, rootPath: dir)
      }
    }
  }

  // MARK: - Active repository

  /// Make the repository containing `dir` the window's active repository
  /// (titlebar breadcrumb, status bar, diff pill all follow it live).
  /// In a folder workspace the repository is the folder's, whatever
  /// directory the active tab is in.
  private func bindRepository(forDirectory dir: String) {
    let workspace = tabManager.activeWorkspace
    let anchor = workspace.kind == .folder ? workspace.root : dir
    repositoryAnchor = anchor
    guard !anchor.isEmpty else {
      setRepository(nil)
      return
    }
    if let current = windowModel.repository,
      anchor == current.root || anchor.hasPrefix(current.root + "/")
    {
      return
    }
    GitRepositoryStore.shared.resolve(directory: anchor) { [weak self] state in
      guard let self, self.repositoryAnchor == anchor else { return }
      self.setRepository(state)
    }
  }

  // MARK: - Workspaces

  /// Outside folder workspaces the file tree and repository follow the
  /// active tab's directory.
  var followsActiveDirectory: Bool { tabManager.activeWorkspace.kind == .scratch }

  /// ⌃Tab / ⌃⇧Tab: cycle through the active workspace's tabs.
  private func cycleTab(by step: Int) {
    let visible = tabManager.visibleTabIndices
    guard visible.count > 1, let position = visible.firstIndex(of: tabManager.selectedIndex)
    else { return }
    tabManager.selectTab(index: visible[(position + step + visible.count) % visible.count])
  }

  private func activeWorkspaceDidChange() {
    let workspace = tabManager.activeWorkspace
    if workspace.kind == .folder, workspace.root != fileTreeRootPath {
      switchFileTreeRoot(workspace.root, updateStatusBar: false)
    }
    updateStatusBar()
  }

  /// Choose a folder to open as a workspace.
  func presentOpenWorkspacePanel() {
    guard let window else { return }
    let panel = NSOpenPanel()
    panel.canChooseFiles = false
    panel.canChooseDirectories = true
    panel.allowsMultipleSelection = false
    panel.prompt = "Open Workspace"
    panel.message = "Choose a folder to work in. It gets its own tabs, file tree and git state."
    panel.beginSheetModal(for: window) { [weak self] response in
      guard response == .OK, let url = panel.url else { return }
      self?.tabManager.openWorkspace(folder: url.path)
    }
  }

  /// Close a workspace after confirming its unsaved files and running
  /// processes.
  func requestCloseWorkspace(_ id: UUID) {
    let surfaces = tabManager.tabIndices(inWorkspace: id).flatMap { tabManager.tabs[$0].surfaces }
    confirmClosing(surfaces) { [weak self] in
      guard let self else { return }
      for surface in surfaces { self.willCloseSurface(surface) }
      self.tabManager.closeWorkspace(id)
    }
  }

  func presentRenameWorkspace(_ id: UUID) {
    guard let window, let workspace = tabManager.workspace(id) else { return }
    let alert = NSAlert()
    alert.messageText = "Rename Workspace"
    alert.informativeText = "Leave empty to use the folder name."
    alert.addButton(withTitle: "Rename")
    alert.addButton(withTitle: "Cancel")
    let field = NSTextField(string: workspace.customName ?? workspace.name)
    field.frame = NSRect(x: 0, y: 0, width: 260, height: 24)
    alert.accessoryView = field
    alert.window.initialFirstResponder = field
    alert.beginSheetModal(for: window) { [weak self] response in
      guard response == .alertFirstButtonReturn else { return }
      self?.tabManager.renameWorkspace(id, to: field.stringValue)
    }
  }

  private func setRepository(_ state: GitRepositoryState?) {
    guard windowModel.repository !== state else { return }
    if let previous = repositoryListener {
      previous.state.removeChangeListener(previous.token)
      repositoryListener = nil
    }
    windowModel.repository = state
    if let state {
      // Working-tree and index changes recolor the file tree's git badges.
      let token = state.addChangeListener { [weak self] change in
        guard let self else { return }
        self.fileTreeData.refreshGitStatus()
        // Staging/committing/switching changes the editor's diff base.
        if !change.isDisjoint(with: [.index, .refs]), let editor = self.tabManager.selectedEditor {
          self.applyGitDiffDecorations(editor: editor)
        }
      }
      repositoryListener = (state, token)
    }
    if repositoryObservation == nil {
      repositoryObservation = ObservationLoop(owner: self) { [weak self] in
        self?.syncRepositoryToModel()
      }
    }
  }

  private func syncRepositoryToModel() {
    let snapshot = windowModel.repository?.snapshot
    let branch = snapshot.map { snap in
      snap.branch ?? snap.headOid.map { String($0.prefix(7)) } ?? ""
    }
    windowModel.gitBranch = (branch?.isEmpty ?? true) ? nil : branch
    windowModel.reviewChangedFileCount = snapshot?.changedFileCount ?? 0
    windowModel.reviewAddedLines = snapshot?.totalAdded ?? 0
    windowModel.reviewRemovedLines = snapshot?.totalRemoved ?? 0
  }

  /// Switch the active tab's repository to `branch` with `git switch`, off the
  /// main thread. On failure, explains why in a sheet (dirty tree, unknown
  /// branch, index.lock held by another process, ...).
  func switchBranch(to branch: String) {
    guard let repository = windowModel.repository else { return }
    GitActions(repository: repository, host: self).switchBranch(branch)
  }

  /// Open the branch switcher (the palette in branch mode).
  func showBranchSwitcher() {
    showPalette(prefix: "b:")
  }

  /// Shows a git failure as a window-modal sheet: the plain-English message,
  /// with git's own output as detail.
  func presentGitError(_ error: GitOperationError, title: String) {
    guard let window else { return }
    let alert = NSAlert()
    alert.alertStyle = .warning
    alert.messageText = title
    let detail = (error.output ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let isGeneric: Bool = {
      if case .cli(let cli) = error { return cli.kind == .other }
      return false
    }()
    alert.informativeText =
      detail.isEmpty || isGeneric ? error.message : "\(error.message)\n\n\(detail)"
    alert.addButton(withTitle: "OK")
    alert.beginSheetModal(for: window)
  }

  // MARK: - Git Diff Decorations

  /// Applies git diff gutter decorations to an editor tab by querying
  /// the FFI bridge for diff markers.
  /// Send the editor its file's git base (index version) and blame; Monaco
  /// computes change marks against the live buffer from it.
  private func applyGitDiffDecorations(editor: EditorTab) {
    guard let path = editor.filePath else { return }
    DispatchQueue.global(qos: .utility).async {
      let base = GitClient.baseContent(forFile: path)
      var blame: [EditorBlameLine] = []
      if base != nil, let root = GitClient.repoRoot(forPath: path) {
        let relative = path.hasPrefix(root + "/") ? String(path.dropFirst(root.count + 1)) : path
        blame = GitOperations.blame(path: relative, root: root).map { line, info in
          EditorBlameLine(
            line: line, author: info.author, time: info.authorTime.timeIntervalSince1970,
            summary: info.summary, sha: info.sha)
        }
      }
      DispatchQueue.main.async {
        editor.setGitBase(base, blame: blame)
      }
    }
  }

  /// Stage the hunk at `line` (from the editor's git peek) or open it in the
  /// review.
  private func handleEditorGitAction(editor: EditorTab, action: String, line: Int) {
    guard let path = editor.filePath, let repository = windowModel.repository,
      path.hasPrefix(repository.root + "/")
    else { return }
    let relative = String(path.dropFirst(repository.root.count + 1))
    if action == "review" {
      gitOpenReview(scope: .unstaged, focusPath: relative)
      return
    }
    guard !editor.isModified else {
      toasts.show(
        Toast(
          kind: .info, message: "Save the file to stage this change.", actionTitle: "Save",
          action: { [weak self] in
            if let location = self?.tabManager.location(of: editor) {
              self?.tabManager.reveal(location)
            }
            NotificationCenter.default.post(name: .impulseSaveFile, object: nil)
          }))
      return
    }
    let root = repository.root
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let diff = try? GitClient.fileDiff(repoPath: root, path: relative, scope: .unstaged)
      DispatchQueue.main.async {
        guard let self, let diff else { return }
        guard
          let index = diff.hunks.firstIndex(where: { hunk in
            let start = Int(hunk.newStart)
            let count = hunk.lines.filter { $0.kind != .removed }.count
            return line >= start - 1 && line <= start + max(count, 1)
          })
        else {
          self.toasts.show(Toast(kind: .warning, message: "That change isn't in the unstaged diff."))
          return
        }
        let change = FileChange(path: relative, status: diff.status)
        GitActions(repository: repository, host: self).apply(
          .stage, selection: .wholeHunks([index]), change: change,
          expectedHunkIds: [index: diff.hunkIds[index]], options: DiffOptions()
        ) { [weak self] success in
          if success {
            self?.toasts.show(Toast(kind: .success, message: "Staged the change"))
            self?.applyGitDiffDecorations(editor: editor)
          }
        }
      }
    }
  }

  // MARK: - Window State

  func sessionWindowState() -> SessionWindowState {
    var (workspaces, activeIndex) = tabManager.sessionWorkspaces()
    // A scratch workspace remembers where its file tree was.
    for index in workspaces.indices where workspaces[index].kind == "scratch" {
      if tabManager.activeWorkspace.kind == .scratch { workspaces[index].fileTreeRoot = fileTreeRootPath }
    }
    return SessionWindowState(
      workspaces: workspaces, activeWorkspaceIndex: activeIndex,
      frame: window.map { NSStringFromRect($0.frame) },
      sidebarVisible: windowModel.sidebarVisible,
      sidebarWidth: Double(windowModel.sidebarWidth))
  }

  /// Rebuild workspaces, tabs and split layouts from a saved window. File
  /// contents are read off the main thread first, then everything is
  /// inserted on main in saved order. Returns false when nothing in the
  /// session can be restored.
  @discardableResult
  func restoreSessionWindow(_ state: SessionWindowState) -> Bool {
    let savedWorkspaces = (state.workspaces ?? []).filter { workspace in
      workspace.kind == "scratch" || FileManager.default.fileExists(atPath: workspace.root)
    }
    guard !savedWorkspaces.isEmpty else { return false }

    if let frame = state.frame, let window {
      let rect = NSRectFromString(frame)
      if rect.width > 200, rect.height > 200,
        NSScreen.screens.contains(where: { $0.visibleFrame.intersects(rect) })
      {
        window.setFrame(rect, display: false)
      }
    }
    if let visible = state.sidebarVisible { setSidebarVisible(visible) }
    if let width = state.sidebarWidth, width > 120 { windowModel.sidebarWidth = CGFloat(width) }

    let paths = savedWorkspaces.flatMap { $0.tabs.flatMap { $0.panes.compactMap(\.path) } }
    let withScrollback = settings.restoreScrollback
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let contents = TabManager.preloadFileContents(paths)
      var workspaces = savedWorkspaces
      if withScrollback { SessionScrollback.load(into: &workspaces) }
      DispatchQueue.main.async { [weak self] in
        self?.insertRestoredWorkspaces(
          workspaces, activeIndex: state.activeWorkspaceIndex, contents: contents)
      }
    }
    return true
  }

  private func insertRestoredWorkspaces(
    _ saved: [SessionWorkspaceState], activeIndex: Int?,
    contents: [String: (text: String, large: Bool)]
  ) {
    var activeWorkspaceID: UUID?
    var activeTabIndex: Int?
    for (position, savedWorkspace) in saved.enumerated() {
      let workspace: Workspace
      if savedWorkspace.kind == "scratch", let scratch = tabManager.scratchWorkspace {
        workspace = scratch
        if let root = savedWorkspace.fileTreeRoot, FileManager.default.fileExists(atPath: root) {
          switchFileTreeRoot(root)
        }
      } else {
        workspace = Workspace(kind: .folder, root: savedWorkspace.root)
        tabManager.addWorkspace(workspace)
      }
      workspace.customName = savedWorkspace.name
      workspace.isExpanded = savedWorkspace.expanded ?? false
      let projectDirectory = workspace.kind == .folder ? workspace.root : nil

      var restoredIndexes: [Int?] = []
      for tab in savedWorkspace.tabs {
        var panes: [Int: TabEntry] = [:]
        for (id, surface) in tab.panes.enumerated() {
          if let entry = tabManager.makeRestoredSurface(
            surface, contents: contents, projectDirectory: projectDirectory)
          {
            panes[id] = entry
          }
        }
        let index = tabManager.appendRestoredTab(
          panes: panes, layout: tab.layout, focusedPane: tab.focusedPane, pinned: tab.pinned,
          workspaceID: workspace.id)
        restoredIndexes.append(index)
        for case .editor(let editor) in panes.values {
          if let path = editor.filePath {
            trackEditorTab(editor, forPath: path)
            lspDidOpenIfNeeded(path: path)
          }
        }
      }
      if position == (activeIndex ?? 0) {
        activeWorkspaceID = workspace.id
        // Saved index → index among the tabs that actually came back.
        if let saved = savedWorkspace.activeTabIndex, restoredIndexes.indices.contains(saved),
          restoredIndexes[saved] != nil
        {
          activeTabIndex = restoredIndexes[..<saved].compactMap { $0 }.count
        }
      }
    }
    tabManager.finishRestore(activeWorkspaceID: activeWorkspaceID, activeTabIndex: activeTabIndex)
  }

  func restorableOpenFiles() -> [String] {
    let paths = tabManager.allSurfaces.compactMap { tab -> String? in
      switch tab {
      case .editor(let editor):
        return editor.filePath
      case .imagePreview(let path, _):
        return path
      case .terminal, .diffReview, .split:
        return nil
      }
    }
    var seen = Set<String>()
    return paths.filter { path in
      guard FileManager.default.fileExists(atPath: path), !seen.contains(path) else {
        return false
      }
      seen.insert(path)
      return true
    }
  }

  private func dirtyEditors() -> [EditorTab] {
    tabManager.allSurfaces.compactMap { tab in
      if case .editor(let editor) = tab, editor.isModified {
        return editor
      }
      return nil
    }
  }

  func runningTerminalProcessCount() -> Int {
    tabManager.allSurfaces.reduce(0) { count, tab in
      if case .terminal(let container) = tab {
        return count + container.runningDescendantProcessCount()
      }
      return count
    }
  }

  func runningCloseRiskCommands() -> [CloseRiskCommand] {
    tabManager.allSurfaces.flatMap { tab in
      if case .terminal(let container) = tab {
        return container.runningCloseRiskCommands()
      }
      return []
    }
  }

  private func closeRiskSummary(action: CloseRiskAction) -> CloseRiskSummary? {
    closeRiskSummary(
      action: action,
      unsavedEditorCount: 0,
      runningTerminalProcessCount: runningTerminalProcessCount(),
      runningCommands: runningCloseRiskCommands()
    )
  }

  private func closeRiskSummary(
    action: CloseRiskAction,
    unsavedEditorCount: Int,
    runningTerminalProcessCount: Int,
    runningCommands: [CloseRiskCommand]
  ) -> CloseRiskSummary? {
    let input = CloseRiskInput(
      action: action,
      unsavedEditorCount: unsavedEditorCount,
      runningTerminalProcessCount: runningTerminalProcessCount,
      runningCommands: runningCommands,
      nowMs: currentUnixTimeMs(),
      longCommandThresholdSeconds: UInt64(max(1, settings.terminalLongCommandSeconds))
    )
    return input.summarize()
  }

  private func confirmClosingTerminalProcessesIfNeeded() -> Bool {
    guard settings.confirmCloseWarnings else { return true }
    guard let summary = closeRiskSummary(action: .closeWindow), summary.hasRisk else {
      return true
    }

    let alert = NSAlert()
    alert.messageText = summary.title
    alert.informativeText = closeRiskInformativeText(summary)
    alert.alertStyle = .warning
    alert.addButton(withTitle: summary.destructiveActionTitle)
    alert.addButton(withTitle: summary.cancelTitle)
    return alert.runModal() == .alertFirstButtonReturn
  }

  private func closeRiskInformativeText(_ summary: CloseRiskSummary) -> String {
    let details = summary.detailLines.joined(separator: "\n")
    if summary.informativeText.isEmpty {
      return details
    }
    if details.isEmpty {
      return summary.informativeText
    }
    return "\(summary.informativeText)\n\n\(details)"
  }

  private func reviewDirtyEditorsBeforeWindowClose(_ dirty: [EditorTab]) {
    reviewingDirtyWindowClose = true
    var remaining = dirty

    func next() {
      guard !remaining.isEmpty else {
        self.reviewingDirtyWindowClose = false
        self.closingAfterDirtyReview = true
        self.window?.close()
        return
      }

      let editor = remaining.removeFirst()
      guard let location = self.tabManager.location(of: editor) else {
        next()
        return
      }

      self.tabManager.reveal(location)
      self.reviewAndSave(editor: editor) { proceed in
        if proceed {
          DispatchQueue.main.async { next() }
        } else {
          self.reviewingDirtyWindowClose = false
          self.closingAfterDirtyReview = false
        }
      }
    }

    next()
  }

  // MARK: - NSWindowDelegate

  func windowShouldClose(_ sender: NSWindow) -> Bool {
    if (NSApp.delegate as? AppDelegate)?.isApplicationTerminating == true {
      return true
    }
    if closingAfterDirtyReview {
      closingAfterDirtyReview = false
      return confirmClosingTerminalProcessesIfNeeded()
    }
    guard !reviewingDirtyWindowClose else { return false }

    let dirty = dirtyEditors()
    guard !dirty.isEmpty else {
      return confirmClosingTerminalProcessesIfNeeded()
    }
    reviewDirtyEditorsBeforeWindowClose(dirty)
    return false
  }

  func windowWillEnterFullScreen(_ notification: Notification) {
    windowModel.isFullScreen = true
    // The sizing toolbar has no items; in full screen it would only appear as
    // an empty reveal strip over the chrome bar.
    window?.toolbar?.isVisible = false
  }

  func windowDidExitFullScreen(_ notification: Notification) {
    windowModel.isFullScreen = false
    window?.toolbar?.isVisible = true
  }

  func windowDidBecomeKey(_ notification: Notification) {
    // Refresh git state when the window regains focus — git status may
    // have changed externally (e.g. commits from another terminal).
    if let editor = tabManager.selectedEditor {
      applyGitDiffDecorations(editor: editor)
    }
    fileTreeData.refreshGitStatus()
  }

  func windowWillClose(_ notification: Notification) {
    teardownCustomKeybindingMonitor()

    // Persist restorable window state before tab cleanup clears it.
    if let delegate = NSApp.delegate as? AppDelegate {
      delegate.settings.sidebarVisible = windowModel.sidebarVisible
      delegate.settings.sidebarWidth = Int(windowModel.sidebarWidth)
      delegate.persistSessionStateFromOpenWindows()
    }

    // Remove all notification observers.
    notificationObservers.forEach { NotificationCenter.default.removeObserver($0) }
    notificationObservers.removeAll()

    // Clean up all remaining tabs (kill terminal processes, tear down
    // editor WebViews) so resources are freed immediately.
    tabManager.cleanupAllTabs()

    // Clear editor tab tracking.
    editorTabsByPath.removeAll()

    (NSApp.delegate as? AppDelegate)?.windowControllerDidClose(self)
  }
}

// MARK: - Terminal Search Field Delegate

extension MainWindowController: NSSearchFieldDelegate {
  func controlTextDidChange(_ obj: Notification) {
    guard let field = obj.object as? NSSearchField, field === termSearchField else { return }
    let query = field.stringValue
    guard let terminal = tabManager.selectedTerminal?.activeTerminal else { return }
    if query.isEmpty {
      terminal.searchClear()
    } else {
      terminal.search(query)
    }
  }

  func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector)
    -> Bool
  {
    guard control === termSearchField else { return false }
    switch commandSelector {
    case #selector(NSResponder.cancelOperation(_:)):
      hideTerminalSearch()
      return true
    case #selector(NSResponder.insertNewline(_:)):
      tabManager.selectedTerminal?.activeTerminal?.searchNext()
      return true
    case #selector(NSResponder.insertBacktab(_:)):
      tabManager.selectedTerminal?.activeTerminal?.searchPrev()
      return true
    default:
      return false
    }
  }
}


// MARK: - Palette host

extension MainWindowController: PaletteHost {
  var paletteRoot: String { fileTreeRootPath }

  var paletteOpenFiles: [String] {
    tabManager.allSurfaces.compactMap { tab in
      if case .editor(let editor) = tab { return editor.filePath }
      return nil
    }
  }

  var paletteTabs: [TabDisplayInfo] { windowModel.allTabs }

  var paletteHistoryContext: (cwd: String?, repo: String?) {
    let cwd = tabManager.selectedTerminal?.activeTerminal?.currentWorkingDirectory
    return (cwd?.isEmpty == false ? cwd : nil, windowModel.repository?.root)
  }

  /// Into the input bar, or typed at the shell prompt when the grid owns
  /// input (classic mode, a TUI).
  func paletteInsertCommand(_ command: String) {
    guard let terminal = tabManager.selectedTerminal?.activeTerminal else {
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(command, forType: .string)
      toasts.show(Toast(kind: .info, message: "Copied the command (no terminal is focused)."))
      return
    }
    if terminal.wantsGridFocus {
      terminal.insertInput(command)
      terminal.focus()
    } else {
      windowModel.inputDraft = command
      windowModel.inputDraftRestoreToken += 1
      windowModel.inputBarFocusToken += 1
    }
  }
  var paletteWorkspaces: [WorkspaceInfo] { windowModel.workspaces }
  var paletteVisibleTabIndices: [Int] { tabManager.visibleTabIndices }

  func paletteSelectWorkspace(_ id: UUID) {
    tabManager.activateWorkspace(id)
  }

  func paletteOpenWorkspace(folder: String?) {
    if let folder {
      tabManager.openWorkspace(folder: folder)
    } else {
      presentOpenWorkspacePanel()
    }
  }
  var paletteCurrentBranch: String? { windowModel.gitBranch }
  var paletteHasEditor: Bool { tabManager.selectedEditor != nil }

  func paletteOpenFile(_ path: String, line: UInt32?, column: UInt32?) {
    openCommandPaletteSearchResult(path: path, line: line, column: column)
  }

  func paletteGoToLine(_ line: UInt32, column: UInt32?) {
    guard let editor = tabManager.selectedEditor else { return }
    editor.goToPosition(line: line, column: column ?? 1)
    editor.focus()
  }

  func paletteSwitchBranch(_ branch: String) {
    switchBranch(to: branch)
  }

  func paletteCreateBranch(_ name: String) {
    guard let repository = windowModel.repository else { return }
    GitActions(repository: repository, host: self).createBranch(name)
  }

  func paletteSelectTab(_ index: Int) {
    tabManager.selectTab(index: index)
  }
}


// MARK: - Git panel host

extension MainWindowController: GitPanelHost {
  var toasts: ToastCenter { windowModel.toasts }

  func gitOpenFile(_ absolutePath: String) {
    openCommandPaletteSearchResult(path: absolutePath, line: nil)
  }

  func gitOpenReview(scope: DiffScope, focusPath: String?) {
    guard let repository = windowModel.repository else { return }
    tabManager.addReviewTab(
      repository: repository, scope: scope, focusPath: focusPath, host: self)
  }

  func gitPresentError(_ error: GitOperationError, title: String) {
    presentGitError(error, title: title)
  }

  func gitConfirm(
    title: String, message: String, confirmTitle: String, destructive: Bool,
    completion: @escaping (Bool) -> Void
  ) {
    guard let window else { return completion(false) }
    let alert = NSAlert()
    alert.alertStyle = destructive ? .warning : .informational
    alert.messageText = title
    alert.informativeText = message
    alert.addButton(withTitle: confirmTitle)
    alert.addButton(withTitle: "Cancel")
    alert.buttons.first?.hasDestructiveAction = destructive
    alert.beginSheetModal(for: window) { response in
      completion(response == .alertFirstButtonReturn)
    }
  }

  /// Show the Changes panel in the left dock (⌃⇧G).
  func showChangesPanel() {
    windowModel.resetSearch()
    windowModel.sidebarPanel = .changes
    windowModel.sidebarVisible = true
  }
}
