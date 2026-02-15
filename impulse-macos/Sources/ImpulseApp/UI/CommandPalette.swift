import AppKit

// MARK: - Palette Command

/// A single entry in the command palette, associating a user-visible title
/// and optional shortcut with an action closure.
struct PaletteCommand {
    let id: String
    let title: String
    let shortcut: String?
    let action: () -> Void
}

// MARK: - Command Palette Window

/// A floating command palette overlay activated with Cmd+Shift+P.
///
/// Displays a search field and a scrollable table of commands. Typing in the
/// search field filters commands by title (case-insensitive substring match).
/// Pressing Enter or clicking a row executes the command's action and
/// dismisses the palette. Pressing Escape dismisses without executing.
final class CommandPaletteWindow: NSPanel, NSSearchFieldDelegate, NSTableViewDataSource, NSTableViewDelegate {

    // MARK: - Properties

    private let searchField = NSSearchField()
    private let scrollView = NSScrollView()
    private let tableView = NSTableView()

    private(set) var commands: [PaletteCommand] = []
    private(set) var filteredCommands: [PaletteCommand] = []

    /// The palette width as a fraction of the parent window width, capped
    /// to reasonable bounds.
    private static let paletteWidth: CGFloat = 500
    private static let maxVisibleRows: Int = 12
    private static let rowHeight: CGFloat = 32

    // MARK: - Initialization

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: Self.paletteWidth, height: 400),
            styleMask: [.nonactivatingPanel, .fullSizeContentView],
            backing: .buffered,
            defer: true
        )

        isOpaque = false
        hasShadow = true
        backgroundColor = .clear
        level = .floating
        isMovableByWindowBackground = false
        hidesOnDeactivate = true
        becomesKeyOnlyIfNeeded = false

        let container = NSVisualEffectView()
        container.material = .hudWindow
        container.blendingMode = .behindWindow
        container.state = .active
        container.wantsLayer = true
        container.layer?.cornerRadius = 10
        container.layer?.masksToBounds = true

        contentView = container

        setupSearchField(in: container)
        setupTableView(in: container)
        registerBuiltinCommands()
    }

    // MARK: - Setup

    private func setupSearchField(in container: NSView) {
        searchField.translatesAutoresizingMaskIntoConstraints = false
        searchField.placeholderString = "Type a command..."
        searchField.focusRingType = .none
        searchField.font = NSFont.systemFont(ofSize: 14)
        searchField.delegate = self
        searchField.target = self
        searchField.action = #selector(searchFieldAction(_:))

        container.addSubview(searchField)

        NSLayoutConstraint.activate([
            searchField.topAnchor.constraint(equalTo: container.topAnchor, constant: 12),
            searchField.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 12),
            searchField.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -12),
        ])
    }

    private func setupTableView(in container: NSView) {
        let titleColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("title"))
        titleColumn.title = ""
        titleColumn.resizingMask = .autoresizingMask
        tableView.addTableColumn(titleColumn)

        let shortcutColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("shortcut"))
        shortcutColumn.title = ""
        shortcutColumn.width = 100
        shortcutColumn.minWidth = 80
        shortcutColumn.maxWidth = 140
        shortcutColumn.resizingMask = .userResizingMask
        tableView.addTableColumn(shortcutColumn)

        tableView.headerView = nil
        tableView.rowHeight = Self.rowHeight
        tableView.intercellSpacing = NSSize(width: 0, height: 0)
        tableView.backgroundColor = .clear
        tableView.selectionHighlightStyle = .regular
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.doubleAction = #selector(rowDoubleClicked(_:))

        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder

        container.addSubview(scrollView)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: searchField.bottomAnchor, constant: 8),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
    }

    // MARK: - Command Registration

    /// Registers all built-in commands from the Keybindings registry and any
    /// custom keybindings from the current settings.
    func registerBuiltinCommands() {
        var result: [PaletteCommand] = []

        // Map built-in keybinding IDs to Notification-based actions.
        let actionMap: [String: Notification.Name] = [
            "new_tab":           .impulseNewTerminalTab,
            "close_tab":         .impulseCloseTab,
            "reopen_tab":        .impulseReopenTab,
            "save":              .impulseSaveFile,
            "find":              .impulseFind,
            "toggle_sidebar":    .impulseToggleSidebar,
            "project_search":    .impulseFindInProject,
            "command_palette":   .impulseShowCommandPalette,
            "split_horizontal":  .impulseSplitHorizontal,
            "split_vertical":    .impulseSplitVertical,
        ]

        for binding in Keybindings.builtins {
            let shortcut = Keybindings.shortcutDisplay(forId: binding.id)
            let notificationName = actionMap[binding.id]

            let action: () -> Void
            if let name = notificationName {
                action = {
                    NotificationCenter.default.post(name: name, object: nil)
                }
            } else {
                // For commands without a direct notification mapping, post
                // a generic action notification with the ID in userInfo.
                let commandId = binding.id
                action = {
                    NotificationCenter.default.post(
                        name: Notification.Name("impulseCommand_\(commandId)"),
                        object: nil
                    )
                }
            }

            result.append(PaletteCommand(
                id: binding.id,
                title: binding.description,
                shortcut: shortcut,
                action: action
            ))
        }

        commands = result
        filteredCommands = result
    }

    /// Appends custom keybinding commands from the application settings.
    ///
    /// - Parameter customKeybindings: The user-defined custom keybindings
    ///   from settings.
    func registerCustomCommands(_ customKeybindings: [CustomKeybinding]) {
        for custom in customKeybindings where !custom.name.isEmpty {
            let shortcut = custom.key.isEmpty ? nil : custom.key
            let command = custom.command
            let args = custom.args

            commands.append(PaletteCommand(
                id: "custom_\(custom.name)",
                title: custom.name,
                shortcut: shortcut,
                action: {
                    NotificationCenter.default.post(
                        name: Notification.Name("impulseCustomCommand"),
                        object: nil,
                        userInfo: [
                            "command": command,
                            "args": args,
                        ]
                    )
                }
            ))
        }

        filteredCommands = commands
    }

    // MARK: - Show / Dismiss

    /// Positions the palette centered horizontally near the top of the given
    /// window and makes it key.
    func show(relativeTo parentWindow: NSWindow) {
        let parentFrame = parentWindow.frame
        let paletteWidth = min(Self.paletteWidth, parentFrame.width - 40)
        let visibleRows = min(filteredCommands.count, Self.maxVisibleRows)
        let tableHeight = CGFloat(visibleRows) * Self.rowHeight
        let totalHeight = 12 + 22 + 8 + tableHeight + 8   // padding + search + gap + table + bottom

        let x = parentFrame.origin.x + (parentFrame.width - paletteWidth) / 2
        let y = parentFrame.origin.y + parentFrame.height - totalHeight - 60

        setFrame(NSRect(x: x, y: y, width: paletteWidth, height: totalHeight), display: true)

        searchField.stringValue = ""
        filteredCommands = commands
        tableView.reloadData()

        if !filteredCommands.isEmpty {
            tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        }

        makeKeyAndOrderFront(nil)
        searchField.becomeFirstResponder()
    }

    /// Dismisses the palette.
    func dismiss() {
        orderOut(nil)
        searchField.stringValue = ""
        filteredCommands = commands
    }

    // MARK: - Filtering

    private func applyFilter() {
        let query = searchField.stringValue.trimmingCharacters(in: .whitespaces)
        if query.isEmpty {
            filteredCommands = commands
        } else {
            let lowered = query.lowercased()
            filteredCommands = commands.filter {
                $0.title.lowercased().contains(lowered)
            }
        }
        tableView.reloadData()
        if !filteredCommands.isEmpty {
            tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        }
    }

    // MARK: - Execution

    private func executeSelectedCommand() {
        let row = tableView.selectedRow
        guard row >= 0, row < filteredCommands.count else { return }
        let command = filteredCommands[row]
        dismiss()
        command.action()
    }

    // MARK: - NSSearchFieldDelegate

    func controlTextDidChange(_ obj: Notification) {
        applyFilter()
    }

    // MARK: - Key Handling

    @objc private func searchFieldAction(_ sender: Any?) {
        executeSelectedCommand()
    }

    override func keyDown(with event: NSEvent) {
        switch event.keyCode {
        case 53: // Escape
            dismiss()

        case 36: // Return / Enter
            executeSelectedCommand()

        case 125: // Down arrow
            let nextRow = min(tableView.selectedRow + 1, filteredCommands.count - 1)
            if nextRow >= 0 {
                tableView.selectRowIndexes(IndexSet(integer: nextRow), byExtendingSelection: false)
                tableView.scrollRowToVisible(nextRow)
            }

        case 126: // Up arrow
            let prevRow = max(tableView.selectedRow - 1, 0)
            tableView.selectRowIndexes(IndexSet(integer: prevRow), byExtendingSelection: false)
            tableView.scrollRowToVisible(prevRow)

        default:
            super.keyDown(with: event)
        }
    }

    override var canBecomeKey: Bool { true }

    // MARK: - NSTableViewDataSource

    func numberOfRows(in tableView: NSTableView) -> Int {
        return filteredCommands.count
    }

    // MARK: - NSTableViewDelegate

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard row < filteredCommands.count else { return nil }
        let command = filteredCommands[row]

        let identifier = tableColumn?.identifier ?? NSUserInterfaceItemIdentifier("cell")

        if identifier.rawValue == "title" {
            let cellId = NSUserInterfaceItemIdentifier("TitleCell")
            let cell: NSTextField
            if let existing = tableView.makeView(withIdentifier: cellId, owner: nil) as? NSTextField {
                cell = existing
            } else {
                cell = NSTextField(labelWithString: "")
                cell.identifier = cellId
                cell.font = NSFont.systemFont(ofSize: 13)
                cell.lineBreakMode = .byTruncatingTail
            }
            cell.stringValue = command.title
            cell.textColor = .labelColor
            return cell

        } else if identifier.rawValue == "shortcut" {
            let cellId = NSUserInterfaceItemIdentifier("ShortcutCell")
            let cell: NSTextField
            if let existing = tableView.makeView(withIdentifier: cellId, owner: nil) as? NSTextField {
                cell = existing
            } else {
                cell = NSTextField(labelWithString: "")
                cell.identifier = cellId
                cell.font = NSFont.systemFont(ofSize: 12)
                cell.alignment = .right
                cell.lineBreakMode = .byClipping
            }
            cell.stringValue = command.shortcut ?? ""
            cell.textColor = .secondaryLabelColor
            return cell
        }

        return nil
    }

    func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        return Self.rowHeight
    }

    @objc private func rowDoubleClicked(_ sender: Any?) {
        executeSelectedCommand()
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        // Selection change handled by arrow keys and mouse; no additional
        // action needed.
    }

    // MARK: - Theme

    /// Applies the given theme colors to the command palette.
    func applyTheme(_ theme: Theme) {
        guard let container = contentView as? NSVisualEffectView else { return }
        container.appearance = NSAppearance(named: .darkAqua)
        tableView.reloadData()
    }
}
