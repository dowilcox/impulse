import AppKit

// MARK: - Palette Command

/// A single entry in the command palette, associating a user-visible title
/// and optional shortcut with an action closure.
struct PaletteCommand {
  let item: CommandPaletteItem
  let shortcut: String?
  let action: () -> Void

  var id: String { item.id }
  var title: String { item.title }
}

// MARK: - Command Palette Window

/// A floating command palette overlay activated with Cmd+Shift+P.
///
/// Displays a search field and a scrollable table of commands. Typing in the
/// search field filters commands by title (case-insensitive substring match).
/// Pressing Enter or clicking a row executes the command's action and
/// dismisses the palette. Pressing Escape dismisses without executing.
final class CommandPaletteWindow: NSPanel, NSTextFieldDelegate, NSTableViewDataSource,
  NSTableViewDelegate
{

  // MARK: - Properties

  private let searchField = NSTextField()
  private let scrollView = NSScrollView()
  private let tableView = NSTableView()

  private(set) var commands: [PaletteCommand] = []
  private(set) var filteredCommands: [PaletteCommand] = []
  private var dynamicCommands: [PaletteCommand] = []
  private var recentCommands = RecentCommandStore()
  private var dynamicSearchGeneration = 0
  private let dynamicSearchQueue = DispatchQueue(
    label: "dev.impulse.command-palette-search",
    qos: .userInitiated
  )
  private var dynamicSearchRoot: (() -> String?)?
  private var dynamicOpenFile: ((String, UInt32?) -> Void)?
  private var clickMonitor: Any?
  private weak var ownerWindow: NSWindow?

  private static let paletteWidth: CGFloat = 500
  private static let maxVisibleRows: Int = 12
  private static let rowHeight: CGFloat = 30
  private static let rowSpacing: CGFloat = 2

  // MARK: - Initialization

  init() {
    super.init(
      contentRect: NSRect(x: 0, y: 0, width: Self.paletteWidth, height: 400),
      styleMask: [.titled, .fullSizeContentView],
      backing: .buffered,
      defer: true
    )

    // Hide the title bar visually while keeping proper key window behavior.
    titlebarAppearsTransparent = true
    titleVisibility = .hidden
    standardWindowButton(.closeButton)?.isHidden = true
    standardWindowButton(.miniaturizeButton)?.isHidden = true
    standardWindowButton(.zoomButton)?.isHidden = true

    isOpaque = false
    hasShadow = true
    backgroundColor = .clear
    level = .floating
    isMovableByWindowBackground = false
    hidesOnDeactivate = false

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
    searchField.font = NSFont.appFont(ofSize: 16)
    searchField.isBezeled = false
    searchField.drawsBackground = false
    searchField.textColor = .labelColor
    searchField.delegate = self

    container.addSubview(searchField)

    NSLayoutConstraint.activate([
      searchField.topAnchor.constraint(equalTo: container.topAnchor, constant: 16),
      searchField.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
      searchField.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -16),
    ])
  }

  private func setupTableView(in container: NSView) {
    // Thin separator between search and list
    let separator = NSBox()
    separator.boxType = .separator
    separator.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(separator)

    let titleColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("title"))
    titleColumn.title = ""
    titleColumn.resizingMask = .autoresizingMask
    tableView.addTableColumn(titleColumn)

    let shortcutColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("shortcut"))
    shortcutColumn.title = ""
    shortcutColumn.width = 120
    shortcutColumn.minWidth = 80
    shortcutColumn.maxWidth = 160
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
    scrollView.scrollerStyle = .overlay

    container.addSubview(scrollView)

    NSLayoutConstraint.activate([
      separator.topAnchor.constraint(equalTo: searchField.bottomAnchor, constant: 12),
      separator.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 8),
      separator.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -8),

      scrollView.topAnchor.constraint(equalTo: separator.bottomAnchor, constant: 6),
      scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -6),
    ])
  }

  // MARK: - Command Registration

  /// Registers all built-in commands from the Keybindings registry and any
  /// custom keybindings from the current settings.
  func registerBuiltinCommands(overrides: [String: String] = [:]) {
    var result: [PaletteCommand] = []

    for item in ImpulseCore.commandPaletteBuiltinItems() {
      let shortcut = item.shortcut ?? Keybindings.shortcutDisplay(forId: item.id, overrides: overrides)
      let action = Self.builtinAction(for: item.id)

      result.append(
        PaletteCommand(
          item: item,
          shortcut: shortcut,
          action: action
        ))
    }

    commands = result
    filteredCommands = result
  }

  private static func builtinAction(for id: String) -> () -> Void {
    let notificationMap: [String: Notification.Name] = [
      "new_tab": .impulseNewTerminalTab,
      "close_tab": .impulseCloseTab,
      "reopen_tab": .impulseReopenTab,
      "next_tab": .impulseNextTab,
      "prev_tab": .impulsePrevTab,
      "new_file": .impulseNewFile,
      "save": .impulseSaveFile,
      "find": .impulseFind,
      "go_to_line": .impulseGoToLine,
      "toggle_markdown_preview": .impulseToggleMarkdownPreview,
      "toggle_sidebar": .impulseToggleSidebar,
      "project_search": .impulseFindInProject,
      "command_palette": .impulseShowCommandPalette,
      "quick_open": .impulseQuickOpen,
      "install_lsp": .impulseInstallLsp,
      "font_increase": .impulseFontIncrease,
      "font_decrease": .impulseFontDecrease,
      "font_reset": .impulseFontReset,
      "split_horizontal": .impulseSplitHorizontal,
      "split_vertical": .impulseSplitVertical,
      "focus_prev_split": .impulseFocusPrevSplit,
      "focus_next_split": .impulseFocusNextSplit,
    ]

    if let notificationName = notificationMap[id] {
      return {
        NotificationCenter.default.post(name: notificationName, object: nil)
      }
    }

    switch id {
    case "copy":
      return { NSApp.sendAction(#selector(NSText.copy(_:)), to: nil, from: nil) }
    case "paste":
      return { NSApp.sendAction(#selector(NSText.paste(_:)), to: nil, from: nil) }
    case "open_settings":
      return { (NSApp.delegate as? AppDelegate)?.showPreferences(nil) }
    case "new_window":
      return { (NSApp.delegate as? AppDelegate)?.newWindow(nil) }
    case "fullscreen":
      return { NSApp.keyWindow?.toggleFullScreen(nil) }
    default:
      return {
        NSLog("No command palette action registered for built-in command %@", id)
        NSSound.beep()
      }
    }
  }

  /// Appends custom keybinding commands from the application settings.
  ///
  /// - Parameter customKeybindings: The user-defined custom keybindings
  ///   from settings.
  func registerCustomCommands(_ customKeybindings: [CustomKeybinding]) {
    // Remove previously registered custom commands before adding new ones
    // to prevent accumulation on repeated settings changes.
    commands.removeAll { $0.item.source == "custom" || $0.id.hasPrefix("custom_") }

    for custom in customKeybindings where !custom.name.isEmpty {
      let shortcut = custom.key.isEmpty ? nil : custom.key
      let command = custom.command
      let args = custom.args

      let item = ImpulseCore.commandPaletteCustomItem(
        name: custom.name,
        shortcut: shortcut,
        command: command,
        args: args
      ) ?? CommandPaletteItem(
        id: "custom_\(custom.name)",
        title: custom.name,
        category: "Custom",
        keywords: [command],
        source: "custom",
        shortcut: shortcut,
        payload: nil
      )

      commands.append(
        PaletteCommand(
          item: item,
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

  func configureDynamicSearch(
    rootProvider: @escaping () -> String?,
    openFile: @escaping (String, UInt32?) -> Void
  ) {
    dynamicSearchRoot = rootProvider
    dynamicOpenFile = openFile
  }

  // MARK: - Show / Dismiss

  /// Positions the palette centered horizontally near the top of the given
  /// window and makes it key.
  func show(relativeTo parentWindow: NSWindow) {
    self.ownerWindow = parentWindow
    searchField.stringValue = ""
    dynamicSearchGeneration += 1
    dynamicCommands.removeAll()
    refreshFilteredCommands(for: "")

    let parentFrame = parentWindow.frame
    let paletteWidth = min(Self.paletteWidth, parentFrame.width - 40)
    let visibleRows = min(filteredCommands.count, Self.maxVisibleRows)
    let tableHeight = CGFloat(visibleRows) * Self.rowHeight
    // top padding(16) + search(~19) + gap(12) + separator(1) + gap(6) + table + bottom(6)
    let totalHeight: CGFloat = 16 + 19 + 12 + 1 + 6 + tableHeight + 6

    let x = parentFrame.origin.x + (parentFrame.width - paletteWidth) / 2
    let y = parentFrame.origin.y + parentFrame.height - totalHeight - 60

    setFrame(NSRect(x: x, y: y, width: paletteWidth, height: totalHeight), display: true)

    tableView.reloadData()

    if !filteredCommands.isEmpty {
      tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
      tableView.scrollRowToVisible(0)
    }

    // Attach as child so the palette moves with the parent window.
    parentWindow.addChildWindow(self, ordered: .above)
    makeKeyAndOrderFront(nil)
    makeFirstResponder(searchField)

    // Dismiss on click outside.
    clickMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) {
      [weak self] event in
      guard let self = self, self.isVisible else { return event }
      if event.window !== self {
        self.dismiss()
      }
      return event
    }
  }

  /// Dismisses the palette and cleans up.
  func dismiss() {
    if let monitor = clickMonitor {
      NSEvent.removeMonitor(monitor)
      clickMonitor = nil
    }
    ownerWindow?.removeChildWindow(self)
    orderOut(nil)
    searchField.stringValue = ""
    dynamicSearchGeneration += 1
    dynamicCommands.removeAll()
    filteredCommands = commands
    // Return key focus to the parent window.
    ownerWindow?.makeKeyAndOrderFront(nil)
  }

  // MARK: - Key Event Handling

  /// Intercepts key events before the responder chain so that Escape,
  /// Return, and arrow keys work even while the search field editor has focus.
  override func sendEvent(_ event: NSEvent) {
    if event.type == .keyDown {
      switch event.keyCode {
      case 53:  // Escape
        dismiss()
        return
      case 36, 76:  // Return / Enter (main + numpad)
        executeSelectedCommand()
        return
      case 125:  // Down arrow
        moveSelection(by: 1)
        return
      case 126:  // Up arrow
        moveSelection(by: -1)
        return
      default:
        break
      }
    }
    super.sendEvent(event)
  }

  override var canBecomeKey: Bool { true }

  // MARK: - Filtering

  private func applyFilter() {
    let query = searchField.stringValue.trimmingCharacters(in: .whitespaces)
    refreshFilteredCommands(for: query)
    scheduleDynamicSearch(for: query)
  }

  private func refreshFilteredCommands(for query: String) {
    filteredCommands = filteredCommandPaletteCommands(for: query)
    tableView.reloadData()
    if !filteredCommands.isEmpty {
      tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
    }
  }

  private func filteredCommandPaletteCommands(for query: String) -> [PaletteCommand] {
    let allCommands = commands + dynamicCommands
    var commandsById: [String: PaletteCommand] = [:]
    for command in allCommands {
      commandsById[command.id] = command
    }
    return ImpulseCore.filterCommandPaletteItems(
      allCommands.map(\.item),
      recents: recentCommands,
      query: query
    ).compactMap { commandsById[$0.id] }
  }

  private func scheduleDynamicSearch(for query: String) {
    dynamicSearchGeneration += 1
    let generation = dynamicSearchGeneration

    guard query.count >= 2,
      let root = dynamicSearchRoot?(),
      !root.isEmpty
    else {
      if !dynamicCommands.isEmpty {
        dynamicCommands.removeAll()
        refreshFilteredCommands(for: query)
      }
      return
    }

    dynamicSearchQueue.async {
      let items = ImpulseCore.commandPaletteSearchItems(root: root, query: query, limit: 20)
      DispatchQueue.main.async { [weak self] in
        guard let self,
          generation == self.dynamicSearchGeneration,
          query == self.searchField.stringValue.trimmingCharacters(in: .whitespaces)
        else {
          return
        }
        self.dynamicCommands = items.compactMap { self.dynamicCommand(for: $0) }
        self.refreshFilteredCommands(for: query)
      }
    }
  }

  private func dynamicCommand(for item: CommandPaletteItem) -> PaletteCommand? {
    guard item.source == "dynamic",
      let path = item.payload?["path"],
      !path.isEmpty
    else {
      return nil
    }
    let line = item.payload?["line"].flatMap { UInt32($0) }
    return PaletteCommand(
      item: item,
      shortcut: item.category,
      action: { [weak self] in
        self?.dynamicOpenFile?(path, line)
      }
    )
  }

  // MARK: - Selection & Execution

  private func moveSelection(by delta: Int) {
    let current = tableView.selectedRow
    let next = max(0, min(current + delta, filteredCommands.count - 1))
    tableView.selectRowIndexes(IndexSet(integer: next), byExtendingSelection: false)
    tableView.scrollRowToVisible(next)
  }

  private func executeSelectedCommand() {
    let row = tableView.selectedRow
    guard row >= 0, row < filteredCommands.count else { return }
    let command = filteredCommands[row]
    recentCommands = ImpulseCore.recordRecentCommand(recentCommands, item: command.item)
    dismiss()
    command.action()
  }

  // MARK: - NSSearchFieldDelegate

  func controlTextDidChange(_ obj: Notification) {
    applyFilter()
  }

  // MARK: - NSTableViewDataSource

  func numberOfRows(in tableView: NSTableView) -> Int {
    return filteredCommands.count
  }

  // MARK: - NSTableViewDelegate

  func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView?
  {
    guard row < filteredCommands.count else { return nil }
    let command = filteredCommands[row]

    let identifier = tableColumn?.identifier ?? NSUserInterfaceItemIdentifier("cell")

    if identifier.rawValue == "title" {
      let cellId = NSUserInterfaceItemIdentifier("TitleCell")
      let cellView: NSTableCellView
      if let existing = tableView.makeView(withIdentifier: cellId, owner: nil) as? NSTableCellView {
        cellView = existing
      } else {
        cellView = NSTableCellView()
        cellView.identifier = cellId
        let tf = NSTextField(labelWithString: "")
        tf.font = NSFont.appFont(ofSize: 13)
        tf.lineBreakMode = .byTruncatingTail
        tf.translatesAutoresizingMaskIntoConstraints = false
        cellView.addSubview(tf)
        cellView.textField = tf
        NSLayoutConstraint.activate([
          tf.leadingAnchor.constraint(equalTo: cellView.leadingAnchor, constant: 4),
          tf.trailingAnchor.constraint(equalTo: cellView.trailingAnchor),
          tf.centerYAnchor.constraint(equalTo: cellView.centerYAnchor),
        ])
      }
      cellView.textField?.stringValue = command.title
      cellView.textField?.textColor = .labelColor
      return cellView

    } else if identifier.rawValue == "shortcut" {
      let cellId = NSUserInterfaceItemIdentifier("ShortcutCell")
      let cellView: NSTableCellView
      if let existing = tableView.makeView(withIdentifier: cellId, owner: nil) as? NSTableCellView {
        cellView = existing
      } else {
        cellView = NSTableCellView()
        cellView.identifier = cellId
        let tf = NSTextField(labelWithString: "")
        tf.font = NSFont.appFont(ofSize: 12)
        tf.alignment = .right
        tf.lineBreakMode = .byClipping
        tf.translatesAutoresizingMaskIntoConstraints = false
        cellView.addSubview(tf)
        cellView.textField = tf
        NSLayoutConstraint.activate([
          tf.leadingAnchor.constraint(equalTo: cellView.leadingAnchor),
          tf.trailingAnchor.constraint(equalTo: cellView.trailingAnchor, constant: -4),
          tf.centerYAnchor.constraint(equalTo: cellView.centerYAnchor),
        ])
      }
      cellView.textField?.stringValue = command.shortcut ?? ""
      cellView.textField?.textColor = .secondaryLabelColor
      return cellView
    }

    return nil
  }

  func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
    return Self.rowHeight
  }

  func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
    let id = NSUserInterfaceItemIdentifier("PaletteRow")
    if let existing = tableView.makeView(withIdentifier: id, owner: nil) as? PaletteRowView {
      return existing
    }
    let rowView = PaletteRowView()
    rowView.identifier = id
    return rowView
  }

  @objc private func rowDoubleClicked(_ sender: Any?) {
    executeSelectedCommand()
  }

  func tableViewSelectionDidChange(_ notification: Notification) {
    // Selection change handled by sendEvent; no additional action needed.
  }

  // MARK: - Theme

  /// Applies the given theme colors to the command palette.
  func applyTheme(_ theme: Theme) {
    guard let container = contentView as? NSVisualEffectView else { return }
    container.appearance = NSAppearance(named: theme.isLight ? .aqua : .darkAqua)
    container.wantsLayer = true
    container.layer?.borderWidth = 1
    container.layer?.borderColor = theme.borderColor.cgColor
    tableView.reloadData()
  }
}

// MARK: - Custom Row View

/// Draws a rounded, horizontally-inset selection highlight instead of the
/// default full-bleed rectangle.
private final class PaletteRowView: NSTableRowView {
  override func drawSelection(in dirtyRect: NSRect) {
    guard isSelected else { return }
    let inset = bounds.insetBy(dx: 8, dy: 2)
    let path = NSBezierPath(roundedRect: inset, xRadius: 6, yRadius: 6)
    // labelColor adapts to the palette's effective appearance (light/dark theme).
    NSColor.labelColor.withAlphaComponent(0.10).setFill()
    path.fill()
  }
}
