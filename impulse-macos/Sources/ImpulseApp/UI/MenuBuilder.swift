import AppKit

// MARK: - Menu Builder

/// Constructs the standard macOS menu bar for Impulse.
///
/// All actions are dispatched either through the first-responder chain (so that
/// the frontmost window's controller handles them) or through `NotificationCenter`
/// for actions that are not tied to the responder chain.
enum MenuBuilder {

    /// Builds and returns the complete main menu bar.
    static func buildMainMenu() -> NSMenu {
        let mainMenu = NSMenu()

        mainMenu.addItem(buildAppMenu())
        mainMenu.addItem(buildFileMenu())
        mainMenu.addItem(buildEditMenu())
        mainMenu.addItem(buildViewMenu())
        mainMenu.addItem(buildTerminalMenu())
        mainMenu.addItem(buildWindowMenu())
        mainMenu.addItem(buildHelpMenu())

        return mainMenu
    }

    // MARK: - Impulse (App) Menu

    private static func buildAppMenu() -> NSMenuItem {
        let menu = NSMenu(title: "Impulse")
        let item = NSMenuItem()
        item.submenu = menu

        menu.addItem(withTitle: "About Impulse",
                     action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
                     keyEquivalent: "")

        menu.addItem(.separator())

        let prefsItem = NSMenuItem(title: "Preferences...",
                                   action: #selector(AppDelegate.showPreferences(_:)),
                                   keyEquivalent: ",")
        prefsItem.keyEquivalentModifierMask = [.command]
        menu.addItem(prefsItem)

        menu.addItem(.separator())

        let servicesMenu = NSMenu(title: "Services")
        let servicesItem = NSMenuItem(title: "Services", action: nil, keyEquivalent: "")
        servicesItem.submenu = servicesMenu
        menu.addItem(servicesItem)
        NSApp.servicesMenu = servicesMenu

        menu.addItem(.separator())

        menu.addItem(withTitle: "Hide Impulse",
                     action: #selector(NSApplication.hide(_:)),
                     keyEquivalent: "h")

        let hideOthersItem = NSMenuItem(title: "Hide Others",
                                        action: #selector(NSApplication.hideOtherApplications(_:)),
                                        keyEquivalent: "h")
        hideOthersItem.keyEquivalentModifierMask = [.command, .option]
        menu.addItem(hideOthersItem)

        menu.addItem(withTitle: "Show All",
                     action: #selector(NSApplication.unhideAllApplications(_:)),
                     keyEquivalent: "")

        menu.addItem(.separator())

        menu.addItem(withTitle: "Quit Impulse",
                     action: #selector(NSApplication.terminate(_:)),
                     keyEquivalent: "q")

        return item
    }

    // MARK: - File Menu

    private static func buildFileMenu() -> NSMenuItem {
        let menu = NSMenu(title: "File")
        let item = NSMenuItem()
        item.submenu = menu

        let newTabItem = NSMenuItem(title: "New Tab",
                                    action: #selector(MenuActions.menuNewTab(_:)),
                                    keyEquivalent: "t")
        newTabItem.keyEquivalentModifierMask = [.command]
        menu.addItem(newTabItem)

        let newWindowItem = NSMenuItem(title: "New Window",
                                       action: #selector(AppDelegate.newWindow(_:)),
                                       keyEquivalent: "N")
        newWindowItem.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(newWindowItem)

        menu.addItem(.separator())

        let openItem = NSMenuItem(title: "Open...",
                                  action: #selector(MenuActions.menuOpenFile(_:)),
                                  keyEquivalent: "o")
        openItem.keyEquivalentModifierMask = [.command]
        menu.addItem(openItem)

        menu.addItem(.separator())

        let closeTabItem = NSMenuItem(title: "Close Tab",
                                      action: #selector(MenuActions.menuCloseTab(_:)),
                                      keyEquivalent: "w")
        closeTabItem.keyEquivalentModifierMask = [.command]
        menu.addItem(closeTabItem)

        let closeWindowItem = NSMenuItem(title: "Close Window",
                                         action: #selector(NSWindow.performClose(_:)),
                                         keyEquivalent: "W")
        closeWindowItem.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(closeWindowItem)

        menu.addItem(.separator())

        let saveItem = NSMenuItem(title: "Save",
                                  action: #selector(MenuActions.menuSaveFile(_:)),
                                  keyEquivalent: "s")
        saveItem.keyEquivalentModifierMask = [.command]
        menu.addItem(saveItem)

        return item
    }

    // MARK: - Edit Menu

    private static func buildEditMenu() -> NSMenuItem {
        let menu = NSMenu(title: "Edit")
        let item = NSMenuItem()
        item.submenu = menu

        menu.addItem(withTitle: "Undo",
                     action: Selector(("undo:")),
                     keyEquivalent: "z")

        let redoItem = NSMenuItem(title: "Redo",
                                  action: Selector(("redo:")),
                                  keyEquivalent: "Z")
        redoItem.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(redoItem)

        menu.addItem(.separator())

        menu.addItem(withTitle: "Cut",
                     action: #selector(NSText.cut(_:)),
                     keyEquivalent: "x")

        menu.addItem(withTitle: "Copy",
                     action: #selector(NSText.copy(_:)),
                     keyEquivalent: "c")

        menu.addItem(withTitle: "Paste",
                     action: #selector(NSText.paste(_:)),
                     keyEquivalent: "v")

        let pasteAndMatchItem = NSMenuItem(title: "Paste and Match Style",
                                           action: #selector(NSTextView.pasteAsPlainText(_:)),
                                           keyEquivalent: "V")
        pasteAndMatchItem.keyEquivalentModifierMask = [.command, .option, .shift]
        menu.addItem(pasteAndMatchItem)

        menu.addItem(withTitle: "Select All",
                     action: #selector(NSText.selectAll(_:)),
                     keyEquivalent: "a")

        menu.addItem(.separator())

        let findItem = NSMenuItem(title: "Find...",
                                  action: #selector(MenuActions.menuFind(_:)),
                                  keyEquivalent: "f")
        findItem.keyEquivalentModifierMask = [.command]
        menu.addItem(findItem)

        let goToLineItem = NSMenuItem(title: "Go to Line...",
                                      action: #selector(MenuActions.menuGoToLine(_:)),
                                      keyEquivalent: "g")
        goToLineItem.keyEquivalentModifierMask = [.command]
        menu.addItem(goToLineItem)

        return item
    }

    // MARK: - View Menu

    private static func buildViewMenu() -> NSMenuItem {
        let menu = NSMenu(title: "View")
        let item = NSMenuItem()
        item.submenu = menu

        let sidebarItem = NSMenuItem(title: "Toggle Sidebar",
                                     action: #selector(MenuActions.menuToggleSidebar(_:)),
                                     keyEquivalent: "B")
        sidebarItem.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(sidebarItem)

        menu.addItem(.separator())

        let commandPaletteItem = NSMenuItem(title: "Command Palette",
                                            action: #selector(MenuActions.menuShowCommandPalette(_:)),
                                            keyEquivalent: "P")
        commandPaletteItem.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(commandPaletteItem)

        let findInProjectItem = NSMenuItem(title: "Find in Project",
                                           action: #selector(MenuActions.menuFindInProject(_:)),
                                           keyEquivalent: "F")
        findInProjectItem.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(findInProjectItem)

        menu.addItem(.separator())

        let fontIncreaseItem = NSMenuItem(title: "Increase Font Size",
                                          action: #selector(MenuActions.menuFontIncrease(_:)),
                                          keyEquivalent: "=")
        fontIncreaseItem.keyEquivalentModifierMask = [.command]
        menu.addItem(fontIncreaseItem)

        let fontDecreaseItem = NSMenuItem(title: "Decrease Font Size",
                                          action: #selector(MenuActions.menuFontDecrease(_:)),
                                          keyEquivalent: "-")
        fontDecreaseItem.keyEquivalentModifierMask = [.command]
        menu.addItem(fontDecreaseItem)

        let fontResetItem = NSMenuItem(title: "Reset Font Size",
                                       action: #selector(MenuActions.menuFontReset(_:)),
                                       keyEquivalent: "0")
        fontResetItem.keyEquivalentModifierMask = [.command]
        menu.addItem(fontResetItem)

        menu.addItem(.separator())

        let fullscreenItem = NSMenuItem(title: "Toggle Full Screen",
                                        action: #selector(NSWindow.toggleFullScreen(_:)),
                                        keyEquivalent: "f")
        fullscreenItem.keyEquivalentModifierMask = [.control, .command]
        menu.addItem(fullscreenItem)

        return item
    }

    // MARK: - Terminal Menu

    private static func buildTerminalMenu() -> NSMenuItem {
        let menu = NSMenu(title: "Terminal")
        let item = NSMenuItem()
        item.submenu = menu

        let splitHItem = NSMenuItem(title: "Split Horizontal",
                                    action: #selector(MenuActions.menuSplitHorizontal(_:)),
                                    keyEquivalent: "E")
        splitHItem.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(splitHItem)

        let splitVItem = NSMenuItem(title: "Split Vertical",
                                    action: #selector(MenuActions.menuSplitVertical(_:)),
                                    keyEquivalent: "O")
        splitVItem.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(splitVItem)

        return item
    }

    // MARK: - Window Menu

    private static func buildWindowMenu() -> NSMenuItem {
        let menu = NSMenu(title: "Window")
        let item = NSMenuItem()
        item.submenu = menu

        menu.addItem(withTitle: "Minimize",
                     action: #selector(NSWindow.performMiniaturize(_:)),
                     keyEquivalent: "m")

        menu.addItem(withTitle: "Zoom",
                     action: #selector(NSWindow.performZoom(_:)),
                     keyEquivalent: "")

        menu.addItem(.separator())

        menu.addItem(withTitle: "Bring All to Front",
                     action: #selector(NSApplication.arrangeInFront(_:)),
                     keyEquivalent: "")

        menu.addItem(.separator())

        let nextTabItem = NSMenuItem(title: "Show Next Tab",
                                     action: #selector(MenuActions.menuNextTab(_:)),
                                     keyEquivalent: "\t")
        nextTabItem.keyEquivalentModifierMask = [.control]
        menu.addItem(nextTabItem)

        let prevTabItem = NSMenuItem(title: "Show Previous Tab",
                                     action: #selector(MenuActions.menuPrevTab(_:)),
                                     keyEquivalent: "\u{0019}") // backtab
        prevTabItem.keyEquivalentModifierMask = [.control, .shift]
        menu.addItem(prevTabItem)

        menu.addItem(.separator())

        // Cmd+1 through Cmd+9 for direct tab selection
        for i in 1...9 {
            let tabItem = NSMenuItem(title: "Tab \(i)",
                                     action: #selector(MenuActions.menuSelectTab(_:)),
                                     keyEquivalent: "\(i)")
            tabItem.keyEquivalentModifierMask = [.command]
            tabItem.tag = i - 1 // 0-based index
            menu.addItem(tabItem)
        }

        NSApp.windowsMenu = menu

        return item
    }

    // MARK: - Help Menu

    private static func buildHelpMenu() -> NSMenuItem {
        let menu = NSMenu(title: "Help")
        let item = NSMenuItem()
        item.submenu = menu

        let helpItem = NSMenuItem(title: "Impulse Help",
                                  action: #selector(NSApplication.showHelp(_:)),
                                  keyEquivalent: "?")
        helpItem.keyEquivalentModifierMask = [.command]
        menu.addItem(helpItem)

        NSApp.helpMenu = menu

        return item
    }
}

// MARK: - Menu Action Trampoline

/// A helper object that provides `@objc`-visible selectors for menu items.
/// Each action posts a notification that is picked up by the appropriate
/// window controller or view. This avoids coupling the menu to any specific
/// window instance and lets the notification system route to the key window.
final class MenuActions: NSObject {

    @objc static func menuNewTab(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseNewTerminalTab, object: nil)
    }

    @objc static func menuCloseTab(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseCloseTab, object: nil)
    }

    @objc static func menuOpenFile(_ sender: Any?) {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.treatsFilePackagesAsDirectories = true

        guard panel.runModal() == .OK, let url = panel.url else { return }
        NotificationCenter.default.post(
            name: .impulseOpenFile,
            object: nil,
            userInfo: ["path": url.path]
        )
    }

    @objc static func menuSaveFile(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseSaveFile, object: nil)
    }

    @objc static func menuFind(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFind, object: nil)
    }

    @objc static func menuToggleSidebar(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseToggleSidebar, object: nil)
    }

    @objc static func menuShowCommandPalette(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseShowCommandPalette, object: nil)
    }

    @objc static func menuFindInProject(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFindInProject, object: nil)
    }

    @objc static func menuSplitHorizontal(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseSplitHorizontal, object: nil)
    }

    @objc static func menuSplitVertical(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseSplitVertical, object: nil)
    }

    @objc static func menuGoToLine(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseGoToLine, object: nil)
    }

    @objc static func menuFontIncrease(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFontIncrease, object: nil)
    }

    @objc static func menuFontDecrease(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFontDecrease, object: nil)
    }

    @objc static func menuFontReset(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFontReset, object: nil)
    }

    @objc static func menuNextTab(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseNextTab, object: nil)
    }

    @objc static func menuPrevTab(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulsePrevTab, object: nil)
    }

    @objc static func menuSelectTab(_ sender: Any?) {
        guard let menuItem = sender as? NSMenuItem else { return }
        NotificationCenter.default.post(
            name: .impulseSelectTab,
            object: nil,
            userInfo: ["index": menuItem.tag]
        )
    }
}
