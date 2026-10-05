import AppKit
import ImpulseProtocol
import ImpulseGit

// MARK: - AppDelegate

final class AppDelegate: NSObject, NSApplicationDelegate {
  private struct LspDiagnosticsEvent {
    let uri: String
    let diagnostics: [[String: Any]]
  }

  /// The current application settings (backed by `SettingsStore.shared`).
  var settings: Settings {
    get { SettingsStore.shared.settings }
    set { SettingsStore.shared.settings = newValue }
  }

  /// The current color theme, derived from `settings.colorScheme`.
  var theme: Theme = ThemeManager.theme(forName: "nord")

  /// The FFI bridge to impulse-core/impulse-editor Rust code.
  let core = ImpulseCore()

  /// Shared serial queue for all LSP FFI calls. The LSP backend is owned by
  /// `core`, so all windows must use one queue rather than issuing requests
  /// concurrently from per-window queues.
  let lspQueue = DispatchQueue(label: "dev.impulse.lsp", qos: .userInitiated)

  /// Single app-level LSP event poller. Diagnostics are fanned out to the
  /// window that still owns the target document.
  private var lspPollTimer: Timer?
  private var isPollingLspEvents = false
  private var lspPollAgain = false
  private var settingsObserver: NSObjectProtocol?

  /// File paths to open once the first window is ready (from Finder or CLI).
  var pendingFiles: [String] = []

  /// True once the app-level termination flow has already handled dirty
  /// editor review, so individual windows should not prompt again.
  private(set) var isApplicationTerminating = false

  /// All open main windows. We keep strong references so they survive the
  /// run loop.
  private var windowControllers: [MainWindowController] = []

  func applicationDidFinishLaunching(_ notification: Notification) {
    SettingsStore.shared.load()
    NSApp.servicesProvider = serviceProvider
    NSApp.registerServicesMenuSendTypes([.string], returnTypes: [])
    QuickTerminal.shared.configure(
      enabled: SettingsStore.shared.settings.quickTerminalEnabled,
      shortcut: SettingsStore.shared.settings.quickTerminalShortcut)
    EditorTab.jsonSchemaProvider = { path in
      guard path == Settings.filePath.path,
        let data = try? JSONSerialization.data(
          withJSONObject: SettingsCatalog.jsonSchema(), options: [.sortedKeys])
      else { return nil }
      return String(decoding: data, as: UTF8.self)
    }
    DesktopNotifier.shared.activate()
    // Before any terminal starts, so they get IMPULSE_SOCKET.
    ControlServer.shared.handler = { [weak self] request, reply in
      self?.handleControl(request, reply: reply)
    }
    ControlServer.shared.start()
    // The Dock badge counts terminals (in any window) that need attention.
    NotificationCenter.default.addObserver(
      forName: .terminalAttentionChanged, object: nil, queue: .main
    ) { [weak self] _ in
      guard let self else { return }
      let count = self.windowControllers.reduce(0) { total, controller in
        total
          + controller.tabManager.allSurfaces.filter {
            if case .terminal(let container) = $0 { return container.needsAttention }
            return false
          }.count
      }
      DesktopNotifier.shared.setBadge(count: count)
    }
    theme = ThemeManager.theme(forName: settings.colorScheme)
    rebuildMainMenu()
    observeSettingsChanges()

    // Pre-warm a WebView with Monaco so the first editor tab opens instantly.
    EditorWebViewPool.shared.warmUp()

    // Capture the login-shell PATH (and resolve git) in the background so the
    // first git operation doesn't pay for spawning a login shell.
    DispatchQueue.global(qos: .utility).async {
      _ = GitCLI.gitPath()
    }

    // Pre-scan PATH so the first input-bar completion keystroke is instant,
    // and install the bundled terminal/UI fonts into ~/Library/Fonts.
    DispatchQueue.global(qos: .utility).async {
      InputCompletion.warmCache()
      EditorAssets.installUserFontsIfNeeded()
    }

    // Initialize LSP with the last known directory, or home.
    let rootDir: String
    if !settings.lastDirectory.isEmpty,
      FileManager.default.fileExists(atPath: settings.lastDirectory)
    {
      rootDir = settings.lastDirectory
    } else {
      rootDir = NSHomeDirectory()
    }
    let rootUri = URL(fileURLWithPath: rootDir).absoluteString
    core.initializeLsp(rootUri: rootUri)
    startLspPolling()

    let sessionToRestore: SessionState?
    if let file = DebugSnapshot.sessionFile {
      sessionToRestore = SessionState.load(from: file)
    } else if pendingFiles.isEmpty && settings.restoreSession && !DebugSnapshot.isActive,
      let saved = SessionState.load(), !saved.windows.isEmpty
    {
      sessionToRestore = saved
    } else {
      sessionToRestore = nil
    }

    let filesToOpen: [String]
    if sessionToRestore != nil {
      filesToOpen = []
    } else if pendingFiles.isEmpty && settings.restoreSession && !DebugSnapshot.isActive {
      filesToOpen = settings.openFiles.filter {
        FileManager.default.fileExists(atPath: $0)
      }
    } else if pendingFiles.isEmpty {
      filesToOpen = []
    } else {
      filesToOpen = pendingFiles
    }

    openNewWindow(skipInitialTerminal: sessionToRestore != nil || !filesToOpen.isEmpty)

    if let sessionToRestore {
      DispatchQueue.main.async { [weak self] in
        self?.restoreWindows(from: sessionToRestore)
      }
    }

    // Open any files queued before the window was created (CLI args or Finder).
    // Dispatch to the next run loop iteration so the window is fully visible
    // and the tab manager has completed its initial layout.
    if sessionToRestore == nil && !filesToOpen.isEmpty {
      let files = filesToOpen
      pendingFiles.removeAll()
      DispatchQueue.main.async { [weak self] in
        guard let controller = self?.windowControllers.first else { return }
        for path in files {
          controller.openFile(path: path)
        }
      }
    }

    if DebugSnapshot.isActive {
      NSApp.setActivationPolicy(.accessory)
      DebugSnapshot.run { [weak self] action in
        self?.windowControllers.first?.performDebugAction(action)
      }
    } else {
      NSApp.activate(ignoringOtherApps: true)
    }

    // Check for updates in background if enabled.
    if settings.checkForUpdates && !DebugSnapshot.isActive {
      DispatchQueue.global(qos: .utility).async {
        guard let update = UpdateChecker.checkForUpdate(currentVersion: AppVersion.current)
        else { return }
        DispatchQueue.main.async {
          NotificationCenter.default.post(
            name: .impulseUpdateAvailable,
            object: nil,
            userInfo: [
              "version": update.version, "currentVersion": update.currentVersion, "url": update.url,
            ])
        }
      }
    }
  }

  func application(_ sender: NSApplication, openFiles filenames: [String]) {
    // AppKit hands non-flag command-line arguments to openFiles; in snapshot
    // mode those are the snapshot options' values, not documents.
    guard !DebugSnapshot.isActive else {
      sender.reply(toOpenOrPrint: .success)
      return
    }
    if let controller = windowControllers.first {
      for path in filenames {
        controller.openFile(path: path)
      }
      sender.reply(toOpenOrPrint: .success)
    } else {
      // Window not yet created — queue for applicationDidFinishLaunching.
      pendingFiles.append(contentsOf: filenames)
      sender.reply(toOpenOrPrint: .success)
    }
  }

  func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    let dirty = collectDirtyEditors()
    if dirty.isEmpty {
      guard confirmTerminatingTerminalProcessesIfNeeded() else {
        isApplicationTerminating = false
        return .terminateCancel
      }
      isApplicationTerminating = true
      return .terminateNow
    }

    let alert = NSAlert()
    let count = dirty.count
    alert.messageText =
      count == 1
      ? "You have unsaved changes in 1 document. Do you want to review this change before quitting?"
      : "You have unsaved changes in \(count) documents. Do you want to review these changes before quitting?"
    alert.informativeText = "If you don't review your documents, all your changes will be lost."
    alert.alertStyle = .warning
    alert.addButton(withTitle: "Review Changes\u{2026}")
    alert.addButton(withTitle: "Cancel")
    alert.addButton(withTitle: "Discard Changes")

    let response = alert.runModal()
    switch response {
    case .alertFirstButtonReturn:
      reviewDirtyEditors(dirty)
      return .terminateLater
    case .alertThirdButtonReturn:
      guard confirmTerminatingTerminalProcessesIfNeeded() else {
        isApplicationTerminating = false
        return .terminateCancel
      }
      isApplicationTerminating = true
      return .terminateNow
    default:
      isApplicationTerminating = false
      return .terminateCancel
    }
  }

  /// One dirty editor scheduled for review during quit.
  private struct DirtyEditorRef {
    let controller: MainWindowController
    let editor: EditorTab
  }

  private func collectDirtyEditors() -> [DirtyEditorRef] {
    var result: [DirtyEditorRef] = []
    for window in NSApp.windows {
      guard let controller = window.windowController as? MainWindowController else { continue }
      for tab in controller.tabManager.allSurfaces {
        if case .editor(let editor) = tab, editor.isModified {
          result.append(DirtyEditorRef(controller: controller, editor: editor))
        }
      }
    }
    return result
  }

  /// Walks `dirty` sequentially, activating each editor's window+tab and
  /// presenting the per-doc save sheet. Replies to the pending
  /// `.terminateLater` once every editor has been resolved (or cancelled).
  private func reviewDirtyEditors(_ dirty: [DirtyEditorRef]) {
    var remaining = dirty
    func next() {
      guard !remaining.isEmpty else {
        guard self.confirmTerminatingTerminalProcessesIfNeeded() else {
          self.isApplicationTerminating = false
          NSApp.reply(toApplicationShouldTerminate: false)
          return
        }
        self.isApplicationTerminating = true
        NSApp.reply(toApplicationShouldTerminate: true)
        return
      }
      let ref = remaining.removeFirst()
      // The editor may have been closed while we were processing earlier
      // tabs in the same window; skip stale entries.
      guard let location = ref.controller.tabManager.location(of: ref.editor) else {
        next()
        return
      }
      ref.controller.window?.makeKeyAndOrderFront(nil)
      ref.controller.tabManager.reveal(location)
      ref.controller.reviewAndSave(editor: ref.editor) { proceed in
        if proceed {
          DispatchQueue.main.async { next() }
        } else {
          self.isApplicationTerminating = false
          NSApp.reply(toApplicationShouldTerminate: false)
        }
      }
    }
    next()
  }

  private func confirmTerminatingTerminalProcessesIfNeeded() -> Bool {
    guard settings.confirmCloseWarnings else { return true }
    let input = CloseRiskInput(
      action: .quit,
      unsavedEditorCount: 0,
      runningTerminalProcessCount: runningTerminalProcessCount(),
      runningCommands: runningCloseRiskCommands(),
      nowMs: currentUnixTimeMs(),
      longCommandThresholdSeconds: UInt64(max(1, settings.terminalLongCommandSeconds))
    )
    let summary = input.summarize()
    guard summary.hasRisk else {
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

  private func runningTerminalProcessCount() -> Int {
    windowControllers.reduce(0) { result, controller in
      result + controller.runningTerminalProcessCount()
    }
  }

  private func runningCloseRiskCommands() -> [CloseRiskCommand] {
    windowControllers.flatMap { $0.runningCloseRiskCommands() }
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

  func applicationWillTerminate(_ notification: Notification) {
    persistSessionStateFromOpenWindows()
    ControlServer.shared.stop()

    // Persist window geometry from the frontmost window.
    if let front = windowControllers.first(where: { $0.window?.isKeyWindow == true })
      ?? windowControllers.first
    {
      if let frame = front.window?.frame {
        settings.windowWidth = Int(frame.width)
        settings.windowHeight = Int(frame.height)
      }
    }
    SettingsStore.shared.saveNow()
    if let settingsObserver {
      NotificationCenter.default.removeObserver(settingsObserver)
      self.settingsObserver = nil
    }
    stopLspPolling()
    core.shutdownLsp()
  }

  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return true
  }

  /// Pause the LSP safety-net timer while the app is in the background
  /// (servers still wake the poll when they send something).
  func applicationDidResignActive(_ notification: Notification) {
    stopLspPolling()
  }

  func applicationDidBecomeActive(_ notification: Notification) {
    // Pick up git changes made while Impulse was in the background.
    GitRepositoryStore.shared.refreshAll()
    startLspPolling()
    // Drain anything that queued while inactive (e.g. diagnostics from a
    // build that touched watched files).
    pollLspEventsInBackground()
  }

  func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }

  // MARK: Window Management

  /// Route an `impulse` request to the window holding the calling pane, or
  /// the frontmost window.
  private func handleControl(_ request: ControlRequest, reply: @escaping (ControlResponse) -> Void) {
    if let token = request.token {
      for controller in windowControllers {
        if let terminal = controller.terminal(controlToken: token) {
          controller.handleControl(request, terminal: terminal, reply: reply)
          return
        }
      }
    }
    guard
      let controller = windowControllers.first(where: { $0.window?.isKeyWindow == true })
        ?? windowControllers.first
    else {
      reply(ControlResponse(ok: false, message: "No Impulse window is open."))
      return
    }
    controller.handleControl(request, terminal: nil, reply: reply)
  }

  /// Reopen every saved window (the first reuses the launch window), then
  /// bring the one that was active to the front.
  private func restoreWindows(from session: SessionState) {
    var controllers: [MainWindowController] = []
    for (index, windowState) in session.windows.enumerated() {
      let controller: MainWindowController
      if index == 0, let first = windowControllers.first {
        controller = first
      } else {
        controller = openNewWindow(skipInitialTerminal: true)
      }
      if !controller.restoreSessionWindow(windowState) {
        controller.tabManager.addTerminalTab()
      }
      controllers.append(controller)
    }
    if let active = session.activeWindowIndex, controllers.indices.contains(active) {
      controllers[active].window?.makeKeyAndOrderFront(nil)
    }
  }

  /// Creates and shows a new main window.
  @discardableResult
  @objc func openNewWindow(skipInitialTerminal: Bool = false) -> MainWindowController {
    let controller = MainWindowController(
      settings: settings,
      theme: theme,
      core: core,
      lspQueue: lspQueue,
      skipInitialTerminal: skipInitialTerminal
    )
    windowControllers.append(controller)
    controller.showWindow(nil)

    // Apply the initial theme.
    controller.handleThemeChange(theme)
    return controller
  }

  /// Removes the window controller from our list when its window closes.
  func windowControllerDidClose(_ controller: MainWindowController) {
    windowControllers.removeAll { $0 === controller }
  }

  /// Captures the restorable editor/image session while windows still own
  /// their tabs. If every window has already closed, keep the most recent
  /// snapshot written by the closing window.
  func persistSessionStateFromOpenWindows() {
    guard AppState.persistenceEnabled, !windowControllers.isEmpty else { return }
    var seen = Set<String>()
    settings.openFiles = windowControllers.flatMap { $0.restorableOpenFiles() }.filter { path in
      guard !seen.contains(path) else { return false }
      seen.insert(path)
      return true
    }
    let windows = windowControllers.map { $0.sessionWindowState() }
    let activeWindowIndex =
      windowControllers.firstIndex { $0.window?.isKeyWindow == true }
      ?? (windows.isEmpty ? nil : 0)
    var state = SessionState.snapshot(windows: windows, activeWindowIndex: activeWindowIndex)
    SessionScrollback.store(&state)
    state.save()
  }

  /// Changes the active theme across all windows and persists the choice.
  func applyTheme(named name: String) {
    theme = ThemeManager.theme(forName: name)
    settings.colorScheme = name
    for controller in windowControllers {
      controller.handleThemeChange(theme)
    }
    QuickTerminal.shared.applyTheme(theme)
  }

  // MARK: Menu Actions

  private let serviceProvider = ServiceProvider()

  /// A folder from the Finder service: a workspace in the front window.
  func openWorkspaceFromService(_ path: String) {
    let controller =
      windowControllers.first { $0.window?.isKeyWindow == true } ?? windowControllers.first ?? openNewWindow()
    controller.window?.makeKeyAndOrderFront(nil)
    controller.tabManager.openWorkspace(folder: path)
  }

  /// ⌘, opens Settings as a tab in the front window.
  @objc func showPreferences(_ sender: Any?) {
    let controller =
      windowControllers.first { $0.window?.isKeyWindow == true } ?? windowControllers.first
      ?? openNewWindow()
    controller.openSettings()
  }

  @objc func newWindow(_ sender: Any?) {
    openNewWindow()
  }

  /// Language servers say when events are waiting (see
  /// `LSPRegistry.onEventsAvailable`); this timer is only a slow safety net.
  private func startLspPolling() {
    guard lspPollTimer == nil else { return }
    core.lspEventsAvailable = { [weak self] in
      DispatchQueue.main.async { self?.pollLspEventsInBackground() }
    }
    lspPollTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
      self?.pollLspEventsInBackground()
    }
  }

  private func stopLspPolling() {
    lspPollTimer?.invalidate()
    lspPollTimer = nil
  }

  private func pollLspEventsInBackground() {
    guard !isPollingLspEvents else {
      // Events arrived mid-poll: go again once this one finishes.
      lspPollAgain = true
      return
    }
    isPollingLspEvents = true
    lspPollAgain = false

    lspQueue.async { [weak self] in
      guard let self else { return }

      var events: [LspDiagnosticsEvent] = []
      var count = 0
      let maxEventsPerTick = 50

      while count < maxEventsPerTick, let json = self.core.lspPollEvent() {
        count += 1
        guard let data = json.data(using: .utf8),
          let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let type = event["type"] as? String
        else { continue }

        switch type {
        case "diagnostics":
          guard let uri = event["uri"] as? String,
            let diagnostics = event["diagnostics"] as? [[String: Any]]
          else { continue }
          events.append(LspDiagnosticsEvent(uri: uri, diagnostics: diagnostics))
        default:
          break
        }
      }

      DispatchQueue.main.async { [weak self] in
        guard let self else { return }
        self.isPollingLspEvents = false
        // A full batch may have left more behind.
        if count == maxEventsPerTick || self.lspPollAgain { self.pollLspEventsInBackground() }
        guard !events.isEmpty else { return }

        for event in events {
          for controller in self.windowControllers {
            controller.applyLspDiagnostics(
              uri: event.uri,
              diagnosticsArray: event.diagnostics
            )
          }
        }
      }
    }
  }

  private func observeSettingsChanges() {
    settingsObserver = NotificationCenter.default.addObserver(
      forName: .impulseSettingsDidChange,
      object: nil,
      queue: .main
    ) { [weak self] notification in
      guard let self else { return }
      self.rebuildMainMenu()
      QuickTerminal.shared.configure(
        enabled: self.settings.quickTerminalEnabled, shortcut: self.settings.quickTerminalShortcut)
    }
  }

  private func rebuildMainMenu() {
    NSApp.mainMenu = MenuBuilder.buildMainMenu(overrides: settings.keybindingOverrides)
  }
}
