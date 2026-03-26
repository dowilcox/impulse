import AppKit

// MARK: - AppDelegate

final class AppDelegate: NSObject, NSApplicationDelegate {
    /// The current application settings. Mutated at runtime by the settings
    /// window and saved on quit.
    var settings: Settings = .default

    /// The current color theme, derived from `settings.colorScheme`.
    var theme: Theme = ThemeManager.theme(forName: "nord")

    /// The FFI bridge to impulse-core/impulse-editor Rust code.
    let core = ImpulseCore()

    /// File paths to open once the first window is ready (from Finder or CLI).
    var pendingFiles: [String] = []

    /// All open main windows. We keep strong references so they survive the
    /// run loop.
    private var windowControllers: [MainWindowController] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        settings = Settings.load()
        theme = ThemeManager.theme(forName: settings.colorScheme)

        // Pre-warm a WebView with Monaco so the first editor tab opens instantly.
        EditorWebViewPool.shared.warmUp()

        // Initialize LSP with the last known directory, or home.
        let rootDir: String
        if !settings.lastDirectory.isEmpty,
           FileManager.default.fileExists(atPath: settings.lastDirectory) {
            rootDir = settings.lastDirectory
        } else {
            rootDir = NSHomeDirectory()
        }
        let rootUri = URL(fileURLWithPath: rootDir).absoluteString
        core.initializeLsp(rootUri: rootUri)

        openNewWindow(skipInitialTerminal: !pendingFiles.isEmpty)

        // Open any files queued before the window was created (CLI args or Finder).
        // Dispatch to the next run loop iteration so the window is fully visible
        // and the tab manager has completed its initial layout.
        if !pendingFiles.isEmpty {
            let files = pendingFiles
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
                        userInfo: ["version": update.version, "currentVersion": update.currentVersion, "url": update.url])
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

    func applicationWillTerminate(_ notification: Notification) {
        // Persist window geometry from the frontmost window.
        if let front = windowControllers.first(where: { $0.window?.isKeyWindow == true })
            ?? windowControllers.first {
            if let frame = front.window?.frame {
                settings.windowWidth = Int(frame.width)
                settings.windowHeight = Int(frame.height)
            }
        }
        settings.save()
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
}
