import AppKit

// MARK: - Notification Names

extension Notification.Name {
    /// Posted when the application-wide color theme changes.
    static let impulseThemeDidChange = Notification.Name("impulseThemeDidChange")
    /// Posted when the active tab changes (used by status bar).
    static let impulseActiveTabDidChange = Notification.Name("impulseActiveTabDidChange")
    /// Posted when settings are changed (e.g. from the settings window).
    static let impulseSettingsDidChange = Notification.Name("impulseSettingsDidChange")
    /// Requests a new terminal tab in the frontmost window.
    static let impulseNewTerminalTab = Notification.Name("impulseNewTerminalTab")
    /// Requests closing the current tab in the frontmost window.
    static let impulseCloseTab = Notification.Name("impulseCloseTab")
    /// Requests reopening the most recently closed tab.
    static let impulseReopenTab = Notification.Name("impulseReopenTab")
    /// Requests saving the current editor tab.
    static let impulseSaveFile = Notification.Name("impulseSaveFile")
    /// Requests toggling the sidebar.
    static let impulseToggleSidebar = Notification.Name("impulseToggleSidebar")
    /// Requests showing the command palette.
    static let impulseShowCommandPalette = Notification.Name("impulseShowCommandPalette")
    /// Requests project-wide find.
    static let impulseFindInProject = Notification.Name("impulseFindInProject")
    /// Requests splitting the terminal horizontally.
    static let impulseSplitHorizontal = Notification.Name("impulseSplitHorizontal")
    /// Requests splitting the terminal vertically.
    static let impulseSplitVertical = Notification.Name("impulseSplitVertical")
    /// Requests toggling find in the terminal or editor.
    static let impulseFind = Notification.Name("impulseFind")
    /// Requests showing the go-to-line dialog.
    static let impulseGoToLine = Notification.Name("impulseGoToLine")
    /// Requests increasing the editor and terminal font size.
    static let impulseFontIncrease = Notification.Name("impulseFontIncrease")
    /// Requests decreasing the editor and terminal font size.
    static let impulseFontDecrease = Notification.Name("impulseFontDecrease")
    /// Requests resetting the editor and terminal font size to defaults.
    static let impulseFontReset = Notification.Name("impulseFontReset")
    /// Requests switching to the next tab.
    static let impulseNextTab = Notification.Name("impulseNextTab")
    /// Requests switching to the previous tab.
    static let impulsePrevTab = Notification.Name("impulsePrevTab")
    /// Requests switching to a specific tab by index (0-based in userInfo "index").
    static let impulseSelectTab = Notification.Name("impulseSelectTab")
    /// Requests reloading an editor tab from disk (e.g. after discarding git changes).
    /// The `userInfo` dictionary contains `"path"` (String).
    static let impulseReloadEditorFile = Notification.Name("impulseReloadEditorFile")
    /// Requests moving focus to the previous terminal split pane.
    static let impulseFocusPrevSplit = Notification.Name("impulseFocusPrevSplit")
    /// Requests moving focus to the next terminal split pane.
    static let impulseFocusNextSplit = Notification.Name("impulseFocusNextSplit")
}

// MARK: - AppDelegate

final class AppDelegate: NSObject, NSApplicationDelegate {
    /// The current application settings. Mutated at runtime by the settings
    /// window and saved on quit.
    var settings: Settings = .default

    /// The current color theme, derived from `settings.colorScheme`.
    var theme: Theme = ThemeManager.theme(forName: "nord")

    /// The FFI bridge to impulse-core/impulse-editor Rust code.
    let core = ImpulseCore()

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

        openNewWindow()

        NSApp.activate(ignoringOtherApps: true)
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
    @objc func openNewWindow() {
        let controller = MainWindowController(
            settings: settings,
            theme: theme,
            core: core
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
