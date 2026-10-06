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

/// The primary window controller for Impulse. Each window is a
/// `WorkbenchView`: SwiftUI chrome (titlebar tabs, workspaces sidebar with
/// files/changes/search, right outline panel, status bar) around the
/// TabManager-driven AppKit content region of tabs and split panes.
///
/// Multiple windows can coexist; each owns its own TabManager and workspaces.
final class MainWindowController: NSWindowController, NSWindowDelegate {

  // MARK: - State

  /// Backed by `SettingsStore.shared` (no private copy to keep in sync).
  var settings: Settings {
    get { SettingsStore.shared.settings }
    set { SettingsStore.shared.settings = newValue }
  }

  /// The shared Rust backend (impulse-ffi) instance.
  ///
  /// `internal` (not `private`) because it is accessed from the
  /// `MainWindowController+LSP` extension in a separate file.
  let core: ImpulseCore

  var theme: Theme

  /// Observable state shared with SwiftUI views.
  let windowModel = WindowModel()

  /// Headless owner of the sidebar file tree data: nodes, filesystem and
  /// git watchers, and expansion-state persistence. Rendering happens in
  /// the SwiftUI `FileTreeListView` via `windowModel`.
  let fileTreeData: FileTreeDataController

  /// Manages the tab bar and tab content lifecycle.
  let tabManager: TabManager

  /// The window's AppKit layout root (docks, chrome, center column).
  var workbench: WorkbenchView?

  /// Terminal search bar (hidden by default, toggled with Cmd+F on terminal tabs).
  let termSearchBar = NSView()
  lazy var termFind = TerminalFindModel(palette: windowModel.palette)
  var termSearchBarVisible = false
  /// The window's terminal input bar (see `attachInputBar`).
  var inputBarHost: NSView?
  /// The directory the window's repository was last resolved from.
  var repositoryAnchor = ""
  /// Drives `startPortScanning`.
  var portTimer: Timer?
  weak var inputBarTerminal: TerminalTab?
  var termSearchHeightConstraint: NSLayoutConstraint?

  /// The command palette, lazily created on first use.
  /// Command palette / quick open (see PaletteModel for its modes).
  let palette = PalettePanelController()


  /// Allows a deferred close after dirty editors have been reviewed without
  /// re-triggering the same review loop.
  var closingAfterDirtyReview = false
  /// The user confirmed closing over running processes (the sheet's answer).
  var closeRiskConfirmed = false
  /// The agent state last announced to VoiceOver, per terminal.
  var announcedAgentStates: [UUID: AgentState] = [:]
  var reviewingDirtyWindowClose = false

  /// Local event monitor for custom keybinding interception.
  private var customKeybindingMonitor: Any?

  /// Observer tokens from NotificationCenter, removed on window close and deinit.
  var notificationObservers: [Any] = []

  /// Dictionary mapping file paths to open editor tabs for O(1) lookup.
  ///
  /// `internal` (not `private`) because it is accessed from the
  /// `MainWindowController+LSP` extension in a separate file.
  var editorTabsByPath: [String: EditorTab] = [:]

  // MARK: File Tree State

  /// The root path currently displayed in the file tree. Used to avoid
  /// unnecessary rebuilds (which lose expansion state) when switching tabs.
  var fileTreeRootPath: String = ""

  /// Cached file tree nodes keyed by root path for instant tab switching.
  var fileTreeCache: [String: [FileTreeNode]] = [:]

  /// Tracks access order for LRU eviction of fileTreeCache entries.
  var fileTreeCacheOrder: [String] = []

  /// Maximum number of entries in the file tree cache before LRU eviction.
  let fileTreeCacheMaxSize = 20

  // MARK: Git State

  /// Mirrors the active repository's snapshot into the window model.
  var repositoryObservation: ObservationLoop?
  /// Change listener on the active repository (file tree badges).
  var repositoryListener: (state: GitRepositoryState, token: UUID)?
  /// Files asked to open in the diff view before their tab existed.
  var pendingDiffViewPaths: Set<String> = []

  // MARK: LSP State (internal for MainWindowController+LSP extension)

  /// Per-URI document version counter for LSP.

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
  /// Code actions Swift carries out when picked, by token (see
  /// `MonacoCodeAction.commandToken`).
  var lspCodeActions: [String: LspCodeAction] = [:]

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

    // Set initial root path for the file tree: where Scratch starts. The
    // sidebar follows once the terminal's CWD is detected via OSC 7.
    let rootPath = Workspace.scratchRoot
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
    tabManager.onSurfaceClosing = { [weak self] entry in
      self?.willCloseSurface(entry)
    }
    tabManager.onFolderOpened = { [weak self] folder in
      self?.requestTrustIfNeeded(forFolder: folder)
    }
    tabManager.onClosedTabRecorded = { [weak self] title, isPane in
      self?.offerUndoClose(title: title, isPane: isPane)
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

  // MARK: - Public API

  /// Settings as a tab (one per window), optionally searching for `query`.
  func openSettings(query: String? = nil) {
    let palette = windowModel.palette
    let tool = tabManager.openTool(kind: "settings") { SettingsSurface(palette: palette) }
    guard let surface = tool as? SettingsSurface else { return }
    surface.model.onOpenSettingsFile = { [weak self] in self?.openSettingsFile() }
    surface.model.onOpenKeybindings = { [weak self] in self?.openKeybindings() }
    if let query { surface.reveal(query: query) } else { surface.focusTool() }
  }

  /// Replace the search query with the replacement in every file the
  /// search found it in. Files with unsaved edits are left alone; the
  /// originals are kept for Undo.
  func replaceAllInProject() {
    let query = windowModel.searchQuery
    let replacement = windowModel.searchReplacement
    let caseSensitive = windowModel.searchCaseSensitive
    var seen = Set<String>()
    let paths = windowModel.searchResults.compactMap { result -> String? in
      guard result.matchType == "content", seen.insert(result.path).inserted else { return nil }
      return result.path
    }
    guard !query.isEmpty, !paths.isEmpty else { return }
    let skipped = paths.filter { findEditorTab(forPath: $0)?.isModified == true }
    let targets = paths.filter { !skipped.contains($0) }
    gitConfirm(
      title: "Replace in \(targets.count) file\(targets.count == 1 ? "" : "s")?",
      message: "Every “\(query)” becomes “\(replacement)”."
        + (skipped.isEmpty ? "" : " \(skipped.count) file(s) with unsaved changes are skipped.")
        + " You can undo this right after.",
      confirmTitle: "Replace All", destructive: false
    ) { [weak self] proceed in
      guard proceed, let self else { return }
      DispatchQueue.global(qos: .userInitiated).async {
        var originals: [String: Data] = [:]
        var total = 0
        var failed: [String] = []
        for path in targets {
          guard let data = FileManager.default.contents(atPath: path),
            let text = String(data: data, encoding: .utf8)
          else { continue }
          let result = ProjectReplace.replace(in: text, query: query, with: replacement, caseSensitive: caseSensitive)
          guard result.count > 0 else { continue }
          // In place (not atomic) so permissions and extended attributes stay.
          do {
            try Data(result.text.utf8).write(to: URL(fileURLWithPath: path))
            originals[path] = data
            total += result.count
          } catch {
            failed.append((path as NSString).lastPathComponent)
          }
        }
        DispatchQueue.main.async {
          self.reloadEditors(Array(originals.keys))
          self.windowModel.runSearchNow()
          var message = "Replaced \(total) match\(total == 1 ? "" : "es") in \(originals.count) file\(originals.count == 1 ? "" : "s")"
          if !failed.isEmpty { message += " (couldn't write \(failed.joined(separator: ", ")))" }
          self.toasts.show(
            Toast(
              kind: failed.isEmpty ? .success : .warning, message: message, actionTitle: "Undo",
              action: { [weak self] in
                for (path, data) in originals { try? data.write(to: URL(fileURLWithPath: path)) }
                self?.reloadEditors(Array(originals.keys))
                self?.windowModel.runSearchNow()
              }, lifetime: 20))
        }
      }
    }
  }

  private func reloadEditors(_ paths: [String]) {
    for path in paths {
      NotificationCenter.default.post(name: .impulseReloadEditorFile, object: nil, userInfo: ["path": path])
    }
  }

  /// The window's language-server diagnostics as a tab.
  func showProblems() {
    let palette = windowModel.palette
    let tool = tabManager.openTool(kind: "problems") { ProblemsSurface(palette: palette) }
    guard let surface = tool as? ProblemsSurface else { return }
    surface.model.window = windowModel
    surface.model.root = { [weak self] in
      self?.windowModel.repository?.root ?? self?.fileTreeRootPath
    }
    surface.model.onOpen = { [weak self] problem in
      self?.paletteOpenFile(problem.path, line: UInt32(problem.line), column: UInt32(problem.column))
    }
    surface.model.agents = { [weak self] in self?.agentTargets ?? [] }
    surface.model.onSendToAgent = { [weak self] text, id in self?.sendToAgent(text, terminalID: id) }
  }

  /// Keyboard shortcuts as a tab.
  func openKeybindings() {
    let palette = windowModel.palette
    tabManager.openTool(kind: "keybindings") { KeybindingsSurface(palette: palette) }
  }

  /// settings.json in an editor tab, validated against the settings schema.
  func openSettingsFile() {
    let path = Settings.filePath.path
    if !FileManager.default.fileExists(atPath: path) {
      SettingsStore.shared.saveNow()
    }
    openFile(path: path)
  }

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

  @objc func newFileAction(_ sender: Any?) {
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

  @objc func newFolderAction(_ sender: Any?) {
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

  func setSidebarVisible(_ visible: Bool) {
    windowModel.sidebarVisible = visible
  }

  // MARK: - Custom Keybinding Monitor

  /// Installs a local event monitor that intercepts key-down events matching
  /// any configured custom keybinding. When a match is found the custom
  /// command is executed and the event is consumed.
  func setupCustomKeybindingMonitor() {
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
  func teardownCustomKeybindingMonitor() {
    if let monitor = customKeybindingMonitor {
      NSEvent.removeMonitor(monitor)
      customKeybindingMonitor = nil
    }
  }

  // MARK: - Editor Tab Tracking

  /// Registers an editor tab in the path-to-tab dictionary.
  func trackEditorTab(_ editor: EditorTab, forPath path: String) {
    editorTabsByPath[path] = editor
    if editor.resolveSaveConflict == nil {
      editor.resolveSaveConflict = { [weak self] editor, proceed in
        guard let self else { return proceed(true) }
        self.confirmOverwrite(editor, proceed: proceed)
      }
    }
  }

  /// Removes an editor tab from the path-to-tab dictionary.
  func untrackEditorTab(forPath path: String) {
    editorTabsByPath.removeValue(forKey: path)
  }

  // MARK: - Custom Command Execution

  /// Executes a custom keybinding command by opening a new terminal tab
  /// with the command running in it, using the active tab's working directory.
  func executeCustomCommand(command: String, args: [String]) {
    let fullCommand = ([command.shellEscaped] + args.map(\.shellEscaped)).joined(separator: " ")

    // Get the CWD from the active tab (terminal CWD or editor file's parent)
    let cwd = getActiveCwd()

    // Pass the command through so it's sent right after the shell process
    // starts (shell spawn is deferred to the next run loop tick for layout).
    tabManager.addTerminalTab(directory: cwd, initialCommand: fullCommand)
  }

  /// Returns the current working directory from the active tab:
  /// terminal CWD, or the parent directory of the active editor file.
  func getActiveCwd() -> String? {
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
    case .diffReview(let repoRoot, _), .history(let repoRoot, _):
      return repoRoot.isEmpty ? nil : repoRoot
    case .split, .tool:
      break
    }
    return nil
  }

  // MARK: - Review Changes

  /// Opens the review for the active repository (resolving it from the active
  /// tab's directory or the file-tree root when needed). Outside a git
  /// repository, explains why instead.
  func openDiffReview() {
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
  // MARK: - Font Size

  /// Changes both editor and terminal font sizes by the given delta.
  func changeFontSize(delta: Int) {
    let newEditorSize = max(6, min(72, settings.fontSize + delta))
    let newTerminalSize = max(6, min(72, settings.terminalFontSize + delta))

    settings.fontSize = newEditorSize
    settings.terminalFontSize = newTerminalSize
    applyFontSizeToAllTabs()
  }

  /// Resets font sizes to defaults (14 for both editor and terminal).
  func resetFontSize() {
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
      case .imagePreview, .diffReview, .history, .tool, .split:
        break
      }
    }
  }

  // MARK: - Apply All Settings

  /// Re-applies all settings to every open tab. Called when settings change
  /// via the preferences window.
  func applyAllSettings() {
    let editorOptions = tabManager.editorOptionsFromSettings()
    let termSettings = settings.terminalSettings()

    for tab in tabManager.allSurfaces {
      switch tab {
      case .editor(let editor):
        editor.applySettings(editorOptions)
      case .terminal(let container):
        container.applySettings(settings: termSettings)
      case .imagePreview, .diffReview, .history, .tool, .split:
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
}
