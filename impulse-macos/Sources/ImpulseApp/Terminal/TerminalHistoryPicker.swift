import AppKit

final class TerminalHistoryPicker: NSObject, NSTableViewDataSource, NSTableViewDelegate,
  NSSearchFieldDelegate, NSWindowDelegate
{
  private let search: (String) -> [TerminalCommandHistorySearchResult]
  private let insertCommand: (String) -> Void
  private let runCommand: (String) -> Bool

  private var results: [TerminalCommandHistorySearchResult] = []
  private var panel: NSPanel?
  private let searchField = NSSearchField()
  private let tableView = NSTableView()
  private let insertButton = NSButton(title: "Insert", target: nil, action: nil)
  private let runButton = NSButton(title: "Run", target: nil, action: nil)

  init(
    search: @escaping (String) -> [TerminalCommandHistorySearchResult],
    insertCommand: @escaping (String) -> Void,
    runCommand: @escaping (String) -> Bool
  ) {
    self.search = search
    self.insertCommand = insertCommand
    self.runCommand = runCommand
    super.init()
  }

  func show(attachedTo view: NSView) {
    let panel = ensurePanel()
    reloadResults()
    if let window = view.window {
      let frame = window.frame
      let size = panel.frame.size
      let origin = NSPoint(
        x: frame.midX - size.width / 2,
        y: frame.midY - size.height / 2
      )
      panel.setFrameOrigin(origin)
      if panel.parent !== window {
        panel.parent?.removeChildWindow(panel)
        window.addChildWindow(panel, ordered: .above)
      }
    }
    panel.makeKeyAndOrderFront(nil)
    panel.makeFirstResponder(searchField)
  }

  func close() {
    panel?.close()
  }

  private func ensurePanel() -> NSPanel {
    if let panel {
      return panel
    }

    let panel = NSPanel(
      contentRect: NSRect(x: 0, y: 0, width: 680, height: 360),
      styleMask: [.titled, .closable, .utilityWindow],
      backing: .buffered,
      defer: false
    )
    panel.title = "Command History"
    panel.isFloatingPanel = true
    panel.hidesOnDeactivate = false
    panel.isReleasedWhenClosed = false
    panel.delegate = self

    let content = NSView()
    content.translatesAutoresizingMaskIntoConstraints = false
    panel.contentView = content

    searchField.placeholderString = "Search command history"
    searchField.delegate = self
    searchField.translatesAutoresizingMaskIntoConstraints = false

    let commandColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("command"))
    commandColumn.title = "Command"
    commandColumn.width = 430

    let detailColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("detail"))
    detailColumn.title = "Context"
    detailColumn.width = 210

    tableView.addTableColumn(commandColumn)
    tableView.addTableColumn(detailColumn)
    tableView.headerView = nil
    tableView.usesAlternatingRowBackgroundColors = true
    tableView.rowHeight = 24
    tableView.delegate = self
    tableView.dataSource = self
    tableView.target = self
    tableView.doubleAction = #selector(runSelectedCommand(_:))

    let scrollView = NSScrollView()
    scrollView.translatesAutoresizingMaskIntoConstraints = false
    scrollView.hasVerticalScroller = true
    scrollView.documentView = tableView

    insertButton.target = self
    insertButton.action = #selector(insertSelectedCommand(_:))
    insertButton.translatesAutoresizingMaskIntoConstraints = false

    runButton.target = self
    runButton.action = #selector(runSelectedCommand(_:))
    runButton.keyEquivalent = "\r"
    runButton.translatesAutoresizingMaskIntoConstraints = false

    content.addSubview(searchField)
    content.addSubview(scrollView)
    content.addSubview(insertButton)
    content.addSubview(runButton)

    NSLayoutConstraint.activate([
      searchField.topAnchor.constraint(equalTo: content.topAnchor, constant: 14),
      searchField.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 14),
      searchField.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -14),

      scrollView.topAnchor.constraint(equalTo: searchField.bottomAnchor, constant: 10),
      scrollView.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 14),
      scrollView.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -14),
      scrollView.bottomAnchor.constraint(equalTo: runButton.topAnchor, constant: -12),

      runButton.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -14),
      runButton.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -14),
      insertButton.trailingAnchor.constraint(equalTo: runButton.leadingAnchor, constant: -8),
      insertButton.centerYAnchor.constraint(equalTo: runButton.centerYAnchor),
    ])

    self.panel = panel
    return panel
  }

  func numberOfRows(in tableView: NSTableView) -> Int {
    results.count
  }

  func tableView(
    _ tableView: NSTableView,
    viewFor tableColumn: NSTableColumn?,
    row: Int
  ) -> NSView? {
    guard row >= 0, row < results.count else { return nil }
    let identifier = tableColumn?.identifier ?? NSUserInterfaceItemIdentifier("command")
    let textField =
      tableView.makeView(withIdentifier: identifier, owner: self) as? NSTextField
      ?? NSTextField(labelWithString: "")
    textField.identifier = identifier
    textField.lineBreakMode = .byTruncatingMiddle
    textField.font = .systemFont(ofSize: 12)

    let result = results[row]
    if identifier.rawValue == "detail" {
      textField.stringValue = detailText(for: result)
      textField.textColor = .secondaryLabelColor
    } else {
      textField.stringValue = result.record.command
      textField.textColor = .labelColor
    }
    return textField
  }

  func tableViewSelectionDidChange(_ notification: Notification) {
    updateButtonState()
  }

  func controlTextDidChange(_ obj: Notification) {
    reloadResults()
  }

  func windowWillClose(_ notification: Notification) {
    if let child = notification.object as? NSWindow, let parent = child.parent {
      parent.removeChildWindow(child)
    }
  }

  private func reloadResults() {
    results = search(searchField.stringValue)
    tableView.reloadData()
    if !results.isEmpty {
      tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
    } else {
      tableView.selectRowIndexes(IndexSet(), byExtendingSelection: false)
    }
    updateButtonState()
  }

  private func updateButtonState() {
    let hasSelection = selectedResult() != nil
    insertButton.isEnabled = hasSelection
    runButton.isEnabled = hasSelection
  }

  private func selectedResult() -> TerminalCommandHistorySearchResult? {
    let row = tableView.selectedRow
    guard row >= 0, row < results.count else { return nil }
    return results[row]
  }

  private func detailText(for result: TerminalCommandHistorySearchResult) -> String {
    var parts: [String] = []
    switch result.kind {
    case .recent:
      parts.append("Recent")
    case .prefix:
      parts.append("Prefix")
    case .fuzzy:
      parts.append("Fuzzy")
    }
    if let exitCode = result.record.exitCode {
      parts.append(exitCode == 0 ? "Exit 0" : "Exit \(exitCode)")
    }
    if let cwd = result.record.cwd, !cwd.isEmpty {
      parts.append((cwd as NSString).lastPathComponent)
    }
    return parts.joined(separator: " - ")
  }

  @objc private func insertSelectedCommand(_ sender: Any?) {
    guard let result = selectedResult() else { return }
    insertCommand(result.record.command)
    close()
  }

  @objc private func runSelectedCommand(_ sender: Any?) {
    guard let result = selectedResult() else { return }
    if runCommand(result.record.command) {
      close()
    }
  }
}
