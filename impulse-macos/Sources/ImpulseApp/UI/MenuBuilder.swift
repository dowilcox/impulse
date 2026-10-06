import AppKit

// MARK: - Menu Builder

/// Constructs the standard macOS menu bar for Impulse.
///
/// All actions are dispatched either through the first-responder chain (so that
/// the frontmost window's controller handles them) or through `NotificationCenter`
/// for actions that are not tied to the responder chain.
enum MenuBuilder {

    /// Builds and returns the complete main menu bar.
    static func buildMainMenu(overrides: [String: String] = [:]) -> NSMenu {
        let mainMenu = NSMenu()

        mainMenu.addItem(buildAppMenu(overrides: overrides))
        mainMenu.addItem(buildFileMenu(overrides: overrides))
        mainMenu.addItem(buildEditMenu(overrides: overrides))
        mainMenu.addItem(buildViewMenu(overrides: overrides))
        mainMenu.addItem(buildWindowMenu(overrides: overrides))
        mainMenu.addItem(buildHelpMenu())

        return mainMenu
    }

    /// A menu item that runs palette command `id`, with that keybinding.
    private static func commandItem(_ title: String, id: String, overrides: [String: String]) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: #selector(MenuActions.menuRunCommand(_:)), keyEquivalent: "")
        item.target = MenuActions.shared
        item.representedObject = id
        applyKeybinding(id, overrides: overrides, to: item)
        return item
    }

    private static func applyKeybinding(
        _ id: String,
        overrides: [String: String],
        to item: NSMenuItem
    ) {
        // Unbound commands (empty key equivalent) clear the item's shortcut.
        guard let keybinding = Keybindings.getKeybinding(id: id, overrides: overrides) else { return }
        item.keyEquivalent = keybinding.keyEquivalent
        item.keyEquivalentModifierMask = keybinding.modifierFlags
    }

    // MARK: - Impulse (App) Menu

    private static func buildAppMenu(overrides: [String: String]) -> NSMenuItem {
        let menu = NSMenu(title: "Impulse")
        let item = NSMenuItem()
        item.submenu = menu

        menu.addItem(withTitle: "About Impulse",
                     action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
                     keyEquivalent: "")

        menu.addItem(.separator())

        let prefsItem = NSMenuItem(title: "Settings...",
                                   action: #selector(AppDelegate.showPreferences(_:)),
                                   keyEquivalent: ",")
        applyKeybinding("open_settings", overrides: overrides, to: prefsItem)
        menu.addItem(prefsItem)
        menu.addItem(commandItem("Keyboard Shortcuts…", id: "open_keybindings", overrides: overrides))

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

    private static func buildFileMenu(overrides: [String: String]) -> NSMenuItem {
        let menu = NSMenu(title: "File")
        let item = NSMenuItem()
        item.submenu = menu

        let newTabItem = NSMenuItem(title: "New Tab",
                                    action: #selector(MenuActions.menuNewTab(_:)),
                                    keyEquivalent: "t")
        newTabItem.target = MenuActions.shared
        applyKeybinding("new_tab", overrides: overrides, to: newTabItem)
        menu.addItem(newTabItem)

        let newFileItem = NSMenuItem(title: "New File",
                                      action: #selector(MenuActions.menuNewFile(_:)),
                                      keyEquivalent: "n")
        newFileItem.target = MenuActions.shared
        applyKeybinding("new_file", overrides: overrides, to: newFileItem)
        menu.addItem(newFileItem)

        let newWindowItem = NSMenuItem(title: "New Window",
                                       action: #selector(AppDelegate.newWindow(_:)),
                                       keyEquivalent: "N")
        applyKeybinding("new_window", overrides: overrides, to: newWindowItem)
        menu.addItem(newWindowItem)

        menu.addItem(.separator())

        let openItem = NSMenuItem(title: "Open...",
                                  action: #selector(MenuActions.menuOpenFile(_:)),
                                  keyEquivalent: "o")
        openItem.target = MenuActions.shared
        openItem.keyEquivalentModifierMask = [.command]
        menu.addItem(openItem)

        let openWorkspaceItem = NSMenuItem(title: "Open Folder as Workspace…",
                                           action: #selector(MenuActions.menuOpenWorkspace(_:)),
                                           keyEquivalent: "")
        openWorkspaceItem.target = MenuActions.shared
        menu.addItem(openWorkspaceItem)
        menu.addItem(commandItem("New Task…", id: "new_task", overrides: overrides))

        let composerItem = NSMenuItem(title: "Compose Message to Agent",
                                      action: #selector(MenuActions.menuAgentComposer(_:)),
                                      keyEquivalent: "i")
        composerItem.target = MenuActions.shared
        applyKeybinding("agent_composer", overrides: overrides, to: composerItem)
        menu.addItem(composerItem)

        let nextAgentItem = NSMenuItem(title: "Next Agent Needing You",
                                       action: #selector(MenuActions.menuNextAgent(_:)),
                                       keyEquivalent: "U")
        nextAgentItem.target = MenuActions.shared
        applyKeybinding("next_agent", overrides: overrides, to: nextAgentItem)
        menu.addItem(nextAgentItem)
        menu.addItem(commandItem("Review Last Agent Turn", id: "review_agent_turn", overrides: overrides))

        let switchWorkspaceItem = NSMenuItem(title: "Switch Workspace…",
                                             action: #selector(MenuActions.menuSwitchWorkspace(_:)),
                                             keyEquivalent: "o")
        switchWorkspaceItem.target = MenuActions.shared
        applyKeybinding("switch_workspace", overrides: overrides, to: switchWorkspaceItem)
        menu.addItem(switchWorkspaceItem)

        menu.addItem(.separator())

        let closeTabItem = NSMenuItem(title: "Close Tab",
                                      action: #selector(MenuActions.menuCloseTab(_:)),
                                      keyEquivalent: "w")
        closeTabItem.target = MenuActions.shared
        applyKeybinding("close_tab", overrides: overrides, to: closeTabItem)
        menu.addItem(closeTabItem)

        let reopenTabItem = NSMenuItem(title: "Reopen Closed Tab",
                                       action: #selector(MenuActions.menuReopenTab(_:)),
                                       keyEquivalent: "T")
        reopenTabItem.target = MenuActions.shared
        applyKeybinding("reopen_tab", overrides: overrides, to: reopenTabItem)
        menu.addItem(reopenTabItem)

        let closeWindowItem = NSMenuItem(title: "Close Window",
                                         action: #selector(NSWindow.performClose(_:)),
                                         keyEquivalent: "W")
        closeWindowItem.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(closeWindowItem)

        menu.addItem(.separator())

        let saveItem = NSMenuItem(title: "Save",
                                  action: #selector(MenuActions.menuSaveFile(_:)),
                                  keyEquivalent: "s")
        saveItem.target = MenuActions.shared
        applyKeybinding("save", overrides: overrides, to: saveItem)
        menu.addItem(saveItem)

        return item
    }

    // MARK: - Edit Menu

    private static func buildEditMenu(overrides: [String: String]) -> NSMenuItem {
        let menu = NSMenu(title: "Edit")
        let item = NSMenuItem()
        item.submenu = menu

        // `undo:` / `redo:` reach the first responder's undo stack (NSWindow
        // answers them with its undo manager); `UndoManager.undo` has no
        // colon and nothing in the responder chain implements it.
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

        let copyItem = NSMenuItem(title: "Copy",
                                  action: #selector(NSText.copy(_:)),
                                  keyEquivalent: "c")
        applyKeybinding("copy", overrides: overrides, to: copyItem)
        menu.addItem(copyItem)

        let pasteItem = NSMenuItem(title: "Paste",
                                   action: #selector(NSText.paste(_:)),
                                   keyEquivalent: "v")
        applyKeybinding("paste", overrides: overrides, to: pasteItem)
        menu.addItem(pasteItem)

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
        findItem.target = MenuActions.shared
        applyKeybinding("find", overrides: overrides, to: findItem)
        menu.addItem(findItem)

        let goToLineItem = NSMenuItem(title: "Go to Line...",
                                      action: #selector(MenuActions.menuGoToLine(_:)),
                                      keyEquivalent: "g")
        goToLineItem.target = MenuActions.shared
        applyKeybinding("go_to_line", overrides: overrides, to: goToLineItem)
        menu.addItem(goToLineItem)

        return item
    }

    // MARK: - View Menu

    private static func buildViewMenu(overrides: [String: String]) -> NSMenuItem {
        let menu = NSMenu(title: "View")
        let item = NSMenuItem()
        item.submenu = menu

        let sidebarItem = NSMenuItem(title: "Toggle Sidebar",
                                     action: #selector(MenuActions.menuToggleSidebar(_:)),
                                     keyEquivalent: "B")
        sidebarItem.target = MenuActions.shared
        applyKeybinding("toggle_sidebar", overrides: overrides, to: sidebarItem)
        menu.addItem(sidebarItem)

        menu.addItem(.separator())

        let commandPaletteItem = NSMenuItem(title: "Command Palette",
                                            action: #selector(MenuActions.menuShowCommandPalette(_:)),
                                            keyEquivalent: "P")
        commandPaletteItem.target = MenuActions.shared
        applyKeybinding("command_palette", overrides: overrides, to: commandPaletteItem)
        menu.addItem(commandPaletteItem)

        let quickOpenItem = NSMenuItem(title: "Go to File…",
                                       action: #selector(MenuActions.menuQuickOpen(_:)),
                                       keyEquivalent: "p")
        quickOpenItem.target = MenuActions.shared
        applyKeybinding("quick_open", overrides: overrides, to: quickOpenItem)
        menu.addItem(quickOpenItem)

        let symbolItem = NSMenuItem(title: "Go to Symbol in File…",
                                    action: #selector(MenuActions.menuGoToSymbol(_:)),
                                    keyEquivalent: "O")
        symbolItem.target = MenuActions.shared
        applyKeybinding("go_to_symbol", overrides: overrides, to: symbolItem)
        menu.addItem(symbolItem)

        let findInProjectItem = NSMenuItem(title: "Find in Project",
                                           action: #selector(MenuActions.menuFindInProject(_:)),
                                           keyEquivalent: "F")
        findInProjectItem.target = MenuActions.shared
        applyKeybinding("project_search", overrides: overrides, to: findInProjectItem)
        menu.addItem(findInProjectItem)

        let reviewChangesItem = NSMenuItem(title: "Review Changes",
                                           action: #selector(MenuActions.menuReviewChanges(_:)),
                                           keyEquivalent: "G")
        reviewChangesItem.target = MenuActions.shared
        reviewChangesItem.keyEquivalentModifierMask = [.command, .shift]
        applyKeybinding("review_changes", overrides: overrides, to: reviewChangesItem)
        menu.addItem(reviewChangesItem)

        let branchItem = NSMenuItem(title: "Switch Branch…",
                                    action: #selector(MenuActions.menuSwitchBranch(_:)),
                                    keyEquivalent: "b")
        branchItem.target = MenuActions.shared
        applyKeybinding("switch_branch", overrides: overrides, to: branchItem)
        menu.addItem(branchItem)

        let manageBranchesItem = NSMenuItem(title: "Manage Branches…",
                                            action: #selector(MenuActions.menuManageBranches(_:)),
                                            keyEquivalent: "")
        manageBranchesItem.target = MenuActions.shared
        applyKeybinding("manage_branches", overrides: overrides, to: manageBranchesItem)
        menu.addItem(manageBranchesItem)

        let changesItem = NSMenuItem(title: "Show Changes",
                                     action: #selector(MenuActions.menuShowChanges(_:)),
                                     keyEquivalent: "g")
        changesItem.target = MenuActions.shared
        applyKeybinding("show_changes", overrides: overrides, to: changesItem)
        menu.addItem(changesItem)
        menu.addItem(commandItem("Show Git History", id: "git_history", overrides: overrides))
        menu.addItem(commandItem("Show History of This File", id: "file_history", overrides: overrides))
        menu.addItem(commandItem("Toggle Diff View", id: "diff_view", overrides: overrides))

        menu.addItem(.separator())

        menu.addItem(commandItem("Go to Symbol in Project…", id: "go_to_project_symbol", overrides: overrides))
        menu.addItem(commandItem("Show Problems", id: "show_problems", overrides: overrides))
        menu.addItem(commandItem("Run Project Action…", id: "project_actions", overrides: overrides))

        let markdownPreviewItem = NSMenuItem(title: "Toggle Markdown Preview",
                                             action: #selector(MenuActions.menuToggleMarkdownPreview(_:)),
                                             keyEquivalent: "M")
        markdownPreviewItem.target = MenuActions.shared
        applyKeybinding("toggle_markdown_preview", overrides: overrides, to: markdownPreviewItem)
        menu.addItem(markdownPreviewItem)

        menu.addItem(.separator())

        let panesItem = NSMenuItem(title: "Panes", action: nil, keyEquivalent: "")
        let panesMenu = NSMenu(title: "Panes")
        let paneCommands: [(String, String)?] = [
            ("split_right", "Split Right"),
            ("split_down", "Split Down"),
            nil,
            ("focus_pane_left", "Focus Pane Left"),
            ("focus_pane_right", "Focus Pane Right"),
            ("focus_pane_up", "Focus Pane Above"),
            ("focus_pane_down", "Focus Pane Below"),
            ("next_pane", "Next Pane"),
            ("prev_pane", "Previous Pane"),
            nil,
            ("resize_pane_left", "Grow Pane Left"),
            ("resize_pane_right", "Grow Pane Right"),
            ("resize_pane_up", "Grow Pane Up"),
            ("resize_pane_down", "Grow Pane Down"),
            ("equalize_panes", "Even Out Panes"),
            nil,
            ("zoom_pane", "Zoom Pane"),
            ("move_pane_to_tab", "Move Pane to New Tab"),
        ]
        for entry in paneCommands {
            guard let (id, title) = entry else {
                panesMenu.addItem(.separator())
                continue
            }
            let paneItem = NSMenuItem(
                title: title, action: #selector(MenuActions.menuPaneCommand(_:)), keyEquivalent: "")
            paneItem.target = MenuActions.shared
            paneItem.representedObject = id
            applyKeybinding(id, overrides: overrides, to: paneItem)
            panesMenu.addItem(paneItem)
        }
        panesItem.submenu = panesMenu
        menu.addItem(panesItem)

        let blocksItem = NSMenuItem(title: "Command Blocks", action: nil, keyEquivalent: "")
        let blocksMenu = NSMenu(title: "Command Blocks")
        let blockCommands: [(String, String)?] = [
            ("terminal_hints", "Show Hints"),
            ("select_blocks", "Select Blocks"),
            nil,
            ("previous_block", "Previous Block"),
            ("next_block", "Next Block"),
            ("last_failed_block", "Last Failed Block"),
            nil,
            ("toggle_block_bookmark", "Bookmark Block"),
            ("previous_block_bookmark", "Previous Bookmark"),
            ("next_block_bookmark", "Next Bookmark"),
        ]
        for entry in blockCommands {
            guard let (id, title) = entry else {
                blocksMenu.addItem(.separator())
                continue
            }
            let blockItem = NSMenuItem(
                title: title, action: #selector(MenuActions.menuBlockCommand(_:)), keyEquivalent: "")
            blockItem.target = MenuActions.shared
            blockItem.representedObject = id
            applyKeybinding(id, overrides: overrides, to: blockItem)
            blocksMenu.addItem(blockItem)
        }
        blocksItem.submenu = blocksMenu
        menu.addItem(blocksItem)

        menu.addItem(.separator())

        let fontIncreaseItem = NSMenuItem(title: "Increase Font Size",
                                          action: #selector(MenuActions.menuFontIncrease(_:)),
                                          keyEquivalent: "=")
        fontIncreaseItem.target = MenuActions.shared
        applyKeybinding("font_increase", overrides: overrides, to: fontIncreaseItem)
        menu.addItem(fontIncreaseItem)

        let fontDecreaseItem = NSMenuItem(title: "Decrease Font Size",
                                          action: #selector(MenuActions.menuFontDecrease(_:)),
                                          keyEquivalent: "-")
        fontDecreaseItem.target = MenuActions.shared
        applyKeybinding("font_decrease", overrides: overrides, to: fontDecreaseItem)
        menu.addItem(fontDecreaseItem)

        let fontResetItem = NSMenuItem(title: "Reset Font Size",
                                       action: #selector(MenuActions.menuFontReset(_:)),
                                       keyEquivalent: "0")
        fontResetItem.target = MenuActions.shared
        applyKeybinding("font_reset", overrides: overrides, to: fontResetItem)
        menu.addItem(fontResetItem)

        menu.addItem(.separator())

        let fullscreenItem = NSMenuItem(title: "Toggle Full Screen",
                                        action: #selector(NSWindow.toggleFullScreen(_:)),
                                        keyEquivalent: "f")
        applyKeybinding("fullscreen", overrides: overrides, to: fullscreenItem)
        menu.addItem(fullscreenItem)

        return item
    }

    // MARK: - Terminal Menu

    // MARK: - Window Menu

    private static func buildWindowMenu(overrides: [String: String]) -> NSMenuItem {
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
        nextTabItem.target = MenuActions.shared
        applyKeybinding("next_tab", overrides: overrides, to: nextTabItem)
        menu.addItem(nextTabItem)

        let prevTabItem = NSMenuItem(title: "Show Previous Tab",
                                     action: #selector(MenuActions.menuPrevTab(_:)),
                                     keyEquivalent: "\u{0019}") // backtab
        prevTabItem.target = MenuActions.shared
        applyKeybinding("prev_tab", overrides: overrides, to: prevTabItem)
        menu.addItem(prevTabItem)

        menu.addItem(.separator())

        // Cmd+1 through Cmd+9 for direct tab selection
        for i in 1...9 {
            let tabItem = NSMenuItem(title: "Tab \(i)",
                                     action: #selector(MenuActions.menuSelectTab(_:)),
                                     keyEquivalent: "\(i)")
            tabItem.target = MenuActions.shared
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
///
/// Uses a shared singleton set as the explicit `target` on all menu items so
/// that responder-chain validation succeeds and keyboard shortcuts fire.
final class MenuActions: NSObject {

    /// Shared instance used as the target for all menu items.
    static let shared = MenuActions()

    @objc func menuNewTab(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseNewTerminalTab, object: nil)
    }

    @objc func menuNewFile(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseNewFile, object: nil)
    }

    @objc func menuCloseTab(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseCloseTab, object: nil)
    }

    @objc func menuReopenTab(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseReopenTab, object: nil)
    }

    @objc func menuAgentComposer(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseAgentComposer, object: nil)
    }

    @objc func menuNextAgent(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseNextAgent, object: nil)
    }

    @objc func menuOpenWorkspace(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseOpenWorkspace, object: nil)
    }

    @objc func menuSwitchWorkspace(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseSwitchWorkspace, object: nil)
    }

    /// Opens a file in an editor tab, or a folder as a workspace.
    @objc func menuOpenFile(_ sender: Any?) {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.treatsFilePackagesAsDirectories = true

        guard panel.runModal() == .OK, let url = panel.url else { return }
        NotificationCenter.default.post(
            name: .impulseOpenFile,
            object: nil,
            userInfo: ["path": url.path]
        )
    }

    @objc func menuSaveFile(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseSaveFile, object: nil)
    }

    @objc func menuFind(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFind, object: nil)
    }

    @objc func menuToggleSidebar(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseToggleSidebar, object: nil)
    }

    @objc func menuShowCommandPalette(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseShowCommandPalette, object: nil)
    }

    @objc func menuGoToSymbol(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseGoToSymbol, object: nil)
    }

    @objc func menuQuickOpen(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseQuickOpen, object: nil)
    }

    @objc func menuSwitchBranch(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseSwitchBranch, object: nil)
    }

    @objc func menuManageBranches(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseManageBranches, object: nil)
    }

    @objc func menuShowChanges(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseShowChanges, object: nil)
    }

    @objc func menuBlockCommand(_ sender: NSMenuItem) {
        guard let command = sender.representedObject as? String else { return }
        NotificationCenter.default.post(
            name: .impulseBlockCommand, object: nil, userInfo: ["command": command])
    }

    @objc func menuRunCommand(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String else { return }
        NotificationCenter.default.post(name: .impulseRunCommand, object: nil, userInfo: ["id": id])
    }

    @objc func menuPaneCommand(_ sender: NSMenuItem) {
        guard let command = sender.representedObject as? String else { return }
        NotificationCenter.default.post(
            name: .impulsePaneCommand, object: nil, userInfo: ["command": command])
    }

    @objc func menuFindInProject(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFindInProject, object: nil)
    }

    @objc func menuToggleMarkdownPreview(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseToggleMarkdownPreview, object: nil)
    }

    @objc func menuReviewChanges(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseReviewChanges, object: nil)
    }

    @objc func menuGoToLine(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseGoToLine, object: nil)
    }

    @objc func menuFontIncrease(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFontIncrease, object: nil)
    }

    @objc func menuFontDecrease(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFontDecrease, object: nil)
    }

    @objc func menuFontReset(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseFontReset, object: nil)
    }

    @objc func menuNextTab(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulseNextTab, object: nil)
    }

    @objc func menuPrevTab(_ sender: Any?) {
        NotificationCenter.default.post(name: .impulsePrevTab, object: nil)
    }

    @objc func menuSelectTab(_ sender: Any?) {
        guard let menuItem = sender as? NSMenuItem else { return }
        NotificationCenter.default.post(
            name: .impulseSelectTab,
            object: nil,
            userInfo: ["index": menuItem.tag]
        )
    }
}
