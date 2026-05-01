import AppKit

// MARK: - AppDelegate

final class AppDelegate: NSObject, NSApplicationDelegate {
  private struct LspDiagnosticsEvent {
    let uri: String
    let diagnostics: [[String: Any]]
  }

  /// The current application settings. Mutated at runtime by the settings
  /// window and saved on quit.
  var settings: Settings = .default

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
    settings = Settings.load()
    theme = ThemeManager.theme(forName: settings.colorScheme)
    rebuildMainMenu()
    observeSettingsChanges()

    // Pre-warm a WebView with Monaco so the first editor tab opens instantly.
    EditorWebViewPool.shared.warmUp()

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

    let filesToOpen: [String]
    if pendingFiles.isEmpty {
      filesToOpen = settings.openFiles.filter {
        FileManager.default.fileExists(atPath: $0)
      }
    } else {
      filesToOpen = pendingFiles
    }

    openNewWindow(skipInitialTerminal: !filesToOpen.isEmpty)

    // Open any files queued before the window was created (CLI args or Finder).
    // Dispatch to the next run loop iteration so the window is fully visible
    // and the tab manager has completed its initial layout.
    if !filesToOpen.isEmpty {
      let files = filesToOpen
      pendingFiles.removeAll()
      DispatchQueue.main.async { [weak self] in
        guard let controller = self?.windowControllers.first else { return }
        for path in files {
          controller.openFile(path: path)
        }
      }
    }

    NSApp.activate(ignoringOtherApps: true)

    // Check for updates in background if enabled.
    if settings.checkForUpdates {
      DispatchQueue.global(qos: .utility).async {
        guard let update = ImpulseCore.checkForUpdate() else { return }
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
      for tab in controller.tabManager.tabs {
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
      guard
        let tabIndex = ref.controller.tabManager.tabs.firstIndex(where: {
          if case .editor(let e) = $0 { return e === ref.editor }
          return false
        })
      else {
        next()
        return
      }
      ref.controller.window?.makeKeyAndOrderFront(nil)
      ref.controller.tabManager.selectTab(index: tabIndex)
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
    let count = windowControllers.reduce(0) { result, controller in
      result + controller.runningTerminalProcessCount()
    }
    guard count > 0 else { return true }

    let alert = NSAlert()
    alert.messageText =
      count == 1
      ? "Quit Impulse with 1 running terminal process?"
      : "Quit Impulse with \(count) running terminal processes?"
    alert.informativeText = "Quitting will terminate running processes in terminal tabs."
    alert.alertStyle = .warning
    alert.addButton(withTitle: "Quit")
    alert.addButton(withTitle: "Cancel")
    return alert.runModal() == .alertFirstButtonReturn
  }

  func applicationWillTerminate(_ notification: Notification) {
    persistSessionStateFromOpenWindows()

    // Persist window geometry from the frontmost window.
    if let front = windowControllers.first(where: { $0.window?.isKeyWindow == true })
      ?? windowControllers.first
    {
      if let frame = front.window?.frame {
        settings.windowWidth = Int(frame.width)
        settings.windowHeight = Int(frame.height)
      }
    }
    settings.save()
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

  func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }

  // MARK: Window Management

  /// Creates and shows a new main window.
  @objc func openNewWindow(skipInitialTerminal: Bool = false) {
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
  }

  /// Removes the window controller from our list when its window closes.
  func windowControllerDidClose(_ controller: MainWindowController) {
    windowControllers.removeAll { $0 === controller }
  }

  /// Captures the restorable editor/image session while windows still own
  /// their tabs. If every window has already closed, keep the most recent
  /// snapshot written by the closing window.
  func persistSessionStateFromOpenWindows() {
    guard !windowControllers.isEmpty else { return }
    var seen = Set<String>()
    settings.openFiles = windowControllers.flatMap { $0.restorableOpenFiles() }.filter { path in
      guard !seen.contains(path) else { return false }
      seen.insert(path)
      return true
    }
  }

  /// Changes the active theme across all windows and persists the choice.
  func applyTheme(named name: String) {
    theme = ThemeManager.theme(forName: name)
    settings.colorScheme = name
    for controller in windowControllers {
      controller.handleThemeChange(theme)
    }
    NotificationCenter.default.post(name: .impulseThemeDidChange, object: theme)
  }

  // MARK: Menu Actions

  @objc func showPreferences(_ sender: Any?) {
    SettingsWindowController.show(settings: settings)
  }

  @objc func newWindow(_ sender: Any?) {
    openNewWindow()
  }

  private func startLspPolling() {
    guard lspPollTimer == nil else { return }
    // 25 ms floor: fast enough that typing -> diagnostics/completions feels
    // instant, but still much cheaper than the editor repaint budget.
    lspPollTimer = Timer.scheduledTimer(withTimeInterval: 0.025, repeats: true) { [weak self] _ in
      self?.pollLspEventsInBackground()
    }
  }

  private func stopLspPolling() {
    lspPollTimer?.invalidate()
    lspPollTimer = nil
  }

  private func pollLspEventsInBackground() {
    guard !isPollingLspEvents else { return }
    isPollingLspEvents = true

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
      guard let self, let settings = notification.object as? Settings else { return }
      self.settings = settings
      self.rebuildMainMenu()
    }
  }

  private func rebuildMainMenu() {
    NSApp.mainMenu = MenuBuilder.buildMainMenu(overrides: settings.keybindingOverrides)
  }
}
