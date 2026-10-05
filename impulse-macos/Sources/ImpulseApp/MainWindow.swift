import AppKit
import ImpulseGit
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
      self?.tabManager.selectedTerminal?.activeTerminal?.presentCommandHistory()
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

    windowModel.onShowCommandPalette = { [weak self] in
      self?.showCommandPalette()
    }
    windowModel.onToggleRightDock = { [weak self] in
      self?.toggleRightDock()
    }

    // AppKit owns the layout (docks, dividers, focus); SwiftUI draws the
    // chrome inside hosting views. See WorkbenchView.
    let centerContent = ContentContainer(content: tabManager.contentView)
    // Hosting views here are sized by AppKit constraints: no SwiftUI-driven
    // min/max size (which clamps the window) and no safe-area padding (the
    // titlebar band would otherwise push the chrome down by its height).
    // The banner and input bar keep their intrinsic height.
    let inputHost = WorkbenchHosting.make(TerminalInputHost(model: windowModel), intrinsicHeight: true)
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

    let center = workbench.centerColumn
    for view in [centerContent, inputHost] as [NSView] {
      view.translatesAutoresizingMaskIntoConstraints = false
      center.addSubview(view)
    }
    NSLayoutConstraint.activate([
      centerContent.topAnchor.constraint(equalTo: center.topAnchor),
      centerContent.leadingAnchor.constraint(equalTo: center.leadingAnchor),
      centerContent.trailingAnchor.constraint(equalTo: center.trailingAnchor),
      inputHost.topAnchor.constraint(equalTo: centerContent.bottomAnchor),
      inputHost.leadingAnchor.constraint(equalTo: center.leadingAnchor),
      inputHost.trailingAnchor.constraint(equalTo: center.trailingAnchor),
      inputHost.bottomAnchor.constraint(equalTo: center.bottomAnchor),
    ])

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
    default: NSLog("DebugSnapshot: unknown action '\(action)'")
    }
  }

  // MARK: - Public API

  /// Opens a file in an editor tab. Called by AppDelegate for Finder "Open With"
  /// and CLI file arguments. Bypasses the notification path (which requires
  /// isKeyWindow) so it works during startup before the window is key.
  func openFile(path: String) {
    // Switch the file tree to the file's parent directory.
    let dir = (path as NSString).deletingLastPathComponent
    if !dir.isEmpty, dir != fileTreeRootPath {
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
    for tab in tabManager.tabs {
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

  /// Closes a tab at the given index, showing a save confirmation dialog if
  /// the tab is an editor with unsaved changes. Used by both the Cmd+W
  /// shortcut and the tab bar close button / context menu.
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

    if case .editor(let editor) = tabManager.tabs[index], editor.isModified {
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

      guard let window = self.window else { return }
      alert.beginSheetModal(for: window) { [weak self] response in
        guard let self else { return }
        // Re-find the tab by identity — the index may be stale if
        // other tabs were closed while the alert was visible.
        guard
          let currentIndex = self.tabManager.tabs.firstIndex(where: {
            if case .editor(let e) = $0 { return e === editor }
            return false
          })
        else { return }
        switch response {
        case .alertFirstButtonReturn:
          // Save & Close
          self.saveEditorTab(editor)
          if let path = editor.filePath {
            self.untrackEditorTab(forPath: path)
          }
          self.lspDidClose(editor: editor)
          self.tabManager.closeTab(index: currentIndex)
        case .alertSecondButtonReturn:
          // Don't Save
          if let path = editor.filePath {
            self.untrackEditorTab(forPath: path)
          }
          self.lspDidClose(editor: editor)
          self.tabManager.closeTab(index: currentIndex)
        default:
          // Cancel — do nothing
          break
        }
      }
    } else if case .terminal(let container) = tabManager.tabs[index] {
      confirmClosingTerminalTabIfNeeded(container) { [weak self] shouldClose in
        guard let self, shouldClose else { return }
        guard
          let currentIndex = self.tabManager.tabs.firstIndex(where: {
            if case .terminal(let currentContainer) = $0 {
              return currentContainer === container
            }
            return false
          })
        else { return }
        self.tabManager.closeTab(index: currentIndex)
      }
    } else {
      // Non-editor tabs without terminal processes can close immediately.
      if case .editor(let editor) = tabManager.tabs[index] {
        if let path = editor.filePath {
          untrackEditorTab(forPath: path)
        }
        lspDidClose(editor: editor)
      }
      tabManager.closeTab(index: index)
    }
  }

  private func confirmClosingTerminalTabIfNeeded(
    _ container: TerminalContainer,
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
        runningTerminalProcessCount: container.runningDescendantProcessCount(),
        runningCommands: container.runningCloseRiskCommands()
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
        // Rebuild the file tree when the active tab's directory context differs
        // from the current root. Terminal tabs use their CWD; editor tabs use
        // the parent directory of the open file.
        if let tab = self.tabManager.selectedTab {
          let dir: String?
          switch tab {
          case .terminal(let container):
            dir = container.activeTerminal?.currentWorkingDirectory
          case .editor(let editor):
            dir = editor.projectDirectory
          case .imagePreview:
            dir = nil
          case .diffReview(let repoRoot, _):
            dir = repoRoot
          }
          if let dir, !dir.isEmpty, dir != self.fileTreeRootPath {
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
          let line = Self.lineNumber(from: notification.userInfo)
          self.tabManager.addEditorTab(
            path: path,
            projectDirectory: self.fileTreeRootPath,
            goToLine: line,
            goToColumn: line == nil ? nil : 1
          )
          // Navigate to specific line if provided (e.g. from search results).
          if let editor = self.findEditorTab(forPath: path) {
            self.trackEditorTab(editor, forPath: path)
            self.lspDidOpenIfNeeded(path: path)
            if let line {
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
      nc.addObserver(forName: .impulseToggleRightDock, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, self.window?.isKeyWindow == true else { return }
        self.toggleRightDock()
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
          if dir == self.fileTreeRootPath {
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
        guard let tab = self.tabManager.selectedTab else { return }
        switch tab {
        case .editor(let editor):
          editor.webView?.evaluateJavaScript(
            "editor.getAction('actions.find').run()",
            completionHandler: nil
          )
        case .terminal:
          self.toggleTerminalSearch()
        case .imagePreview, .diffReview:
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
        self.tabManager.refreshSegmentLabels()
      }
    )
    notificationObservers.append(
      nc.addObserver(forName: .editorDirtyStateChanged, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, let editor = notification.object as? EditorTab,
          self.tabManager.tabs.contains(where: {
            if case .editor(let e) = $0 { return e === editor } else { return false }
          })
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
        for (index, tab) in self.tabManager.tabs.enumerated() {
          if case .terminal(let container) = tab,
            container.terminals.contains(where: { $0 === terminalTab })
          {
            self.tabManager.closeTab(index: index)
            break
          }
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
      nc.addObserver(forName: .impulseSelectTab, object: nil, queue: .main) {
        [weak self] notification in
        guard let self, self.window?.isKeyWindow == true else { return }
        guard let index = notification.userInfo?["index"] as? Int else { return }
        if index >= 0, index < self.tabManager.tabs.count {
          self.tabManager.selectTab(index: index)
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
    guard let tab = tabManager.selectedTab else { return nil }
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
    }
    return nil
  }

  // MARK: - Review Changes

  /// Opens the Review Changes tab for the current workspace's git repository.
  ///
  /// Resolves the repo root from the active tab's directory (or the file-tree
  /// root) off the main thread via `listChangedFiles`, which returns the
  /// repository root. If the directory is not inside a git repository, an
  /// alert is shown and no tab is created.
  private func openDiffReview() {
    let candidate = getActiveCwd() ?? fileTreeRootPath
    guard !candidate.isEmpty else {
      presentNotAGitRepoAlert()
      return
    }
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let changeSet = ImpulseCore.listChangedFiles(repoPath: candidate)
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        guard let changeSet, !changeSet.repoRoot.isEmpty else {
          self.presentNotAGitRepoAlert()
          return
        }
        self.tabManager.addDiffReviewTab(repoRoot: changeSet.repoRoot)
      }
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

    for tab in tabManager.tabs {
      switch tab {
      case .editor(let editor):
        editor.applySettings(editorOptions)
      case .terminal(let container):
        container.applySettings(settings: termSettings)
      case .imagePreview, .diffReview:
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

    for tab in tabManager.tabs {
      switch tab {
      case .editor(let editor):
        editor.applySettings(editorOptions)
      case .terminal(let container):
        container.applySettings(settings: termSettings)
      case .imagePreview, .diffReview:
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
  private func bindRepository(forDirectory dir: String) {
    guard !dir.isEmpty else {
      setRepository(nil)
      return
    }
    if let current = windowModel.repository,
      dir == current.root || dir.hasPrefix(current.root + "/")
    {
      return
    }
    GitRepositoryStore.shared.resolve(directory: dir) { [weak self] state in
      guard let self, self.windowModel.currentCwd == dir else { return }
      self.setRepository(state)
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
      let token = state.addChangeListener { [weak self] _ in
        self?.fileTreeData.refreshGitStatus()
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
    repository.run("Switching to \(branch)…") { root in
      GitOperations.switchBranch(branch, root: root)
    } completion: { [weak self] result, _ in
      guard let self else { return }
      switch result {
      case .success:
        self.fileTreeData.refreshGitStatus()
        self.tabManager.syncToWindowModel()
      case .failure(let error):
        self.presentGitError(error, title: "Couldn't switch to \(branch)")
      }
    }
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
  private func applyGitDiffDecorations(editor: EditorTab) {
    guard let path = editor.filePath else { return }
    DispatchQueue.global(qos: .utility).async {
      let markers = ImpulseCore.gitDiffMarkers(filePath: path)
      DispatchQueue.main.async {
        editor.applyDiffDecorations(markers)
      }
    }
  }

  // MARK: - Window State

  func sessionWindowState() -> SessionWindowState {
    tabManager.sessionWindowState(projectRoot: fileTreeRootPath)
  }

  @discardableResult
  func restoreSessionWindow(_ state: SessionWindowState) -> Bool {
    if let projectRoot = state.projectRoot,
      FileManager.default.fileExists(atPath: projectRoot)
    {
      switchFileTreeRoot(projectRoot)
    }

    // Editor files are read off the main thread first; then every tab is
    // inserted on main in saved order (inserting editors asynchronously as
    // they finished loading used to reorder tabs and lose pinned state).
    let restorable = state.tabs.filter { tab in
      switch tab.kind {
      case "terminal": return true
      case "editor":
        guard let path = tab.path else { return false }
        return FileManager.default.fileExists(atPath: path)
      default: return false
      }
    }
    guard !restorable.isEmpty else { return false }

    let projectRoot = state.projectRoot
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      var contents: [String: (String, Bool)] = [:]
      for tab in restorable where tab.kind == "editor" {
        guard let path = tab.path, !TabManager.isBinaryFile(path) else { continue }
        let size =
          (try? FileManager.default.attributesOfItem(atPath: path)[.size] as? Int).flatMap { $0 }
          ?? 0
        let text = (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""
        contents[path] = (text, size > 5 * 1024 * 1024)
      }
      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        var pinnedIndices: [Int] = []
        for tab in restorable {
          let countBefore = self.tabManager.tabs.count
          switch tab.kind {
          case "terminal":
            self.tabManager.addRestoredTerminalTab(tab)
          case "editor":
            guard let path = tab.path else { continue }
            if TabManager.isImageFile(path) {
              self.tabManager.addEditorTab(path: path, projectDirectory: projectRoot)
            } else if let (text, large) = contents[path] {
              self.tabManager.insertLoadedEditorTab(
                path: path, content: text, largeFile: large, projectDirectory: projectRoot)
            }
          default:
            continue
          }
          if tab.pinned, self.tabManager.tabs.count > countBefore {
            pinnedIndices.append(self.tabManager.tabs.count - 1)
          }
        }
        for index in pinnedIndices {
          self.tabManager.setPinned(true, index: index)
        }
        if self.tabManager.tabs.isEmpty {
          self.tabManager.addTerminalTab()
        } else if let activeTabIndex = state.activeTabIndex,
          self.tabManager.tabs.indices.contains(activeTabIndex)
        {
          self.tabManager.selectTab(index: activeTabIndex)
        }
      }
    }
    return true
  }

  func restorableOpenFiles() -> [String] {
    let paths = tabManager.tabs.compactMap { tab -> String? in
      switch tab {
      case .editor(let editor):
        return editor.filePath
      case .imagePreview(let path, _):
        return path
      case .terminal, .diffReview:
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
    tabManager.tabs.compactMap { tab in
      if case .editor(let editor) = tab, editor.isModified {
        return editor
      }
      return nil
    }
  }

  func runningTerminalProcessCount() -> Int {
    tabManager.tabs.reduce(0) { count, tab in
      if case .terminal(let container) = tab {
        return count + container.runningDescendantProcessCount()
      }
      return count
    }
  }

  func runningCloseRiskCommands() -> [CloseRiskCommand] {
    tabManager.tabs.flatMap { tab in
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
      guard
        let tabIndex = self.tabManager.tabs.firstIndex(where: {
          if case .editor(let candidate) = $0 { return candidate === editor }
          return false
        })
      else {
        next()
        return
      }

      self.tabManager.selectTab(index: tabIndex)
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
    tabManager.tabs.compactMap { tab in
      if case .editor(let editor) = tab { return editor.filePath }
      return nil
    }
  }

  var paletteTabs: [TabDisplayInfo] { windowModel.tabDisplayInfos }
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

  func paletteSelectTab(_ index: Int) {
    tabManager.selectTab(index: index)
  }
}
