import AppKit

// SearchResult is defined in Bridge/ImpulseCore.swift

// MARK: - Search Panel

/// Project-wide file name and content search panel for the sidebar.
/// Uses the FFI bridge to `impulse-core` search functions and displays results
/// in an NSTableView. Searches both file names and content simultaneously.
final class SearchPanel: NSView {

    // MARK: Properties

    private var searchField: NSSearchField!
    private var caseSensitiveButton: NSButton!
    private var resultsTableView: NSTableView!
    private var resultsScrollView: NSScrollView!
    private var statusLabel: NSTextField!

    private var results: [SearchResult] = []
    private var rootPath: String = ""

    /// Debounce timer for search-as-you-type.
    private var debounceTimer: Timer?
    private let debounceInterval: TimeInterval = 0.3

    /// Cached theme for styling.
    private var currentTheme: Theme?

    // Table view identifiers
    private let resultCellID = NSUserInterfaceItemIdentifier("ResultCell")

    // MARK: Initialisation

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setup()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setup()
    }

    private func setup() {
        // --- Search field ---
        let field = NSSearchField()
        field.placeholderString = "Search files and content..."
        field.translatesAutoresizingMaskIntoConstraints = false
        field.sendsWholeSearchString = false
        field.sendsSearchStringImmediately = false
        field.delegate = self
        field.focusRingType = .none
        self.searchField = field

        // --- Case-sensitive toggle (placed beside the search field) ---
        let caseBtn = NSButton(title: "Aa", target: self, action: #selector(caseSensitiveToggled(_:)))
        caseBtn.setButtonType(.toggle)
        caseBtn.bezelStyle = .inline
        caseBtn.isBordered = false
        caseBtn.controlSize = .regular
        caseBtn.toolTip = "Case sensitive"
        caseBtn.translatesAutoresizingMaskIntoConstraints = false
        caseBtn.state = .off
        caseBtn.wantsLayer = true
        caseBtn.layer?.cornerRadius = 4
        caseBtn.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .medium)
        self.caseSensitiveButton = caseBtn

        // Search row: field + case toggle
        let searchRow = NSStackView(views: [field, caseBtn])
        searchRow.orientation = .horizontal
        searchRow.spacing = 4
        searchRow.translatesAutoresizingMaskIntoConstraints = false
        searchRow.distribution = .fill

        // --- Results table ---
        let table = NSTableView()
        table.headerView = nil
        table.rowHeight = 44
        table.focusRingType = .none
        table.allowsMultipleSelection = false
        table.usesAlternatingRowBackgroundColors = false
        table.style = .plain
        table.backgroundColor = .clear
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(resultClicked(_:))

        let column = NSTableColumn(identifier: resultCellID)
        column.isEditable = false
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)

        self.resultsTableView = table

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = false
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        self.resultsScrollView = scroll

        // --- Status label ---
        let label = NSTextField(labelWithString: "")
        label.font = NSFont.systemFont(ofSize: 11)
        label.textColor = .secondaryLabelColor
        label.translatesAutoresizingMaskIntoConstraints = false
        label.alignment = .center
        self.statusLabel = label

        // --- Layout ---
        let stack = NSStackView(views: [searchRow, scroll, label])
        stack.orientation = .vertical
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.edgeInsets = NSEdgeInsets(top: 8, left: 8, bottom: 8, right: 8)

        addSubview(stack)

        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),

            field.heightAnchor.constraint(equalToConstant: 28),
            caseBtn.widthAnchor.constraint(equalToConstant: 28),
            caseBtn.heightAnchor.constraint(equalToConstant: 28),
        ])
    }

    // MARK: Public API

    /// Set the project root used for search queries.
    func setRootPath(_ path: String) {
        rootPath = path
    }

    /// Focus the search field.
    func focus() {
        window?.makeFirstResponder(searchField)
    }

    /// Re-apply theme colours to the search panel.
    func applyTheme(_ theme: Theme) {
        currentTheme = theme
        resultsTableView.backgroundColor = .clear
        statusLabel.textColor = theme.fgDark

        // Style the case-sensitive toggle
        caseSensitiveButton.contentTintColor = theme.fgDark

        // Style the search field to match the theme
        searchField.appearance = NSAppearance(named: .darkAqua)
    }

    // MARK: Actions

    @objc private func caseSensitiveToggled(_ sender: NSButton) {
        // Update button visual state
        if let theme = currentTheme {
            if sender.state == .on {
                caseSensitiveButton.layer?.backgroundColor = theme.bgHighlight.cgColor
                caseSensitiveButton.contentTintColor = theme.cyan
            } else {
                caseSensitiveButton.layer?.backgroundColor = NSColor.clear.cgColor
                caseSensitiveButton.contentTintColor = theme.fgDark
            }
        }
        triggerSearch()
    }

    @objc private func resultClicked(_ sender: Any?) {
        let row = resultsTableView.clickedRow
        guard row >= 0, row < results.count else { return }
        let result = results[row]

        var userInfo: [String: Any] = ["path": result.path]
        if let line = result.lineNumber {
            userInfo["line"] = line
        }

        NotificationCenter.default.post(
            name: .impulseOpenFile,
            object: self,
            userInfo: userInfo
        )
    }

    // MARK: Search Execution

    private func triggerSearch() {
        debounceTimer?.invalidate()

        let query = searchField.stringValue.trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else {
            results = []
            statusLabel.stringValue = ""
            resultsTableView.reloadData()
            return
        }

        debounceTimer = Timer.scheduledTimer(withTimeInterval: debounceInterval, repeats: false) { [weak self] _ in
            self?.executeSearch(query: query)
        }
    }

    private func executeSearch(query: String) {
        guard !rootPath.isEmpty else { return }

        let root = self.rootPath
        let caseSensitive = (self.caseSensitiveButton.state == .on)

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard self != nil else { return }

            // Run both searches in parallel
            let fileResults = ImpulseCore.searchFiles(root: root, query: query)
            let contentResults = ImpulseCore.searchContent(root: root,
                                                            query: query,
                                                            caseSensitive: caseSensitive)

            // Combine: file matches first, then content matches.
            // Deduplicate: if a file appears in both, keep the content result
            // (which has line info) and skip the file-only result.
            let contentPaths = Set(contentResults.map { $0.path })
            let uniqueFileResults = fileResults.filter { !contentPaths.contains($0.path) }
            let combined = uniqueFileResults + contentResults

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.results = combined
                self.resultsTableView.reloadData()
                if combined.isEmpty {
                    self.statusLabel.stringValue = "No results"
                } else {
                    let fileCount = uniqueFileResults.count
                    let contentCount = contentResults.count
                    var parts: [String] = []
                    if fileCount > 0 {
                        parts.append("\(fileCount) \(fileCount == 1 ? "file" : "files")")
                    }
                    if contentCount > 0 {
                        parts.append("\(contentCount) \(contentCount == 1 ? "match" : "matches")")
                    }
                    self.statusLabel.stringValue = parts.joined(separator: ", ")
                }
            }
        }
    }

}

// MARK: - NSSearchFieldDelegate

extension SearchPanel: NSSearchFieldDelegate {

    func controlTextDidChange(_ obj: Notification) {
        triggerSearch()
    }
}

// MARK: - NSTableViewDataSource

extension SearchPanel: NSTableViewDataSource {

    func numberOfRows(in tableView: NSTableView) -> Int {
        return results.count
    }
}

// MARK: - NSTableViewDelegate

extension SearchPanel: NSTableViewDelegate {

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard row < results.count else { return nil }
        let result = results[row]

        let cell: NSTableCellView
        if let reused = tableView.makeView(withIdentifier: resultCellID, owner: self) as? NSTableCellView {
            cell = reused
        } else {
            cell = makeResultCellView()
        }

        // Primary line: file name (+ line number for content matches)
        var primary = result.name
        if let line = result.lineNumber {
            primary += ":\(line)"
        }
        cell.textField?.stringValue = primary

        // Secondary line: relative path or matching content
        let secondaryLabel = cell.viewWithTag(100) as? NSTextField
        if let content = result.lineContent {
            secondaryLabel?.stringValue = content.trimmingCharacters(in: .whitespaces)
            secondaryLabel?.textColor = .labelColor
        } else {
            secondaryLabel?.stringValue = abbreviatePath(result.path)
            secondaryLabel?.textColor = .secondaryLabelColor
        }

        // Tertiary: show path for content results below the match line
        let pathLabel = cell.viewWithTag(101) as? NSTextField
        if result.matchType == "content" {
            pathLabel?.stringValue = abbreviatePath(result.path)
            pathLabel?.isHidden = false
        } else {
            pathLabel?.stringValue = ""
            pathLabel?.isHidden = true
        }

        return cell
    }

    // MARK: Cell Construction

    private func makeResultCellView() -> NSTableCellView {
        let cell = NSTableCellView()
        cell.identifier = resultCellID

        // Primary label: file name
        let nameLabel = NSTextField(labelWithString: "")
        nameLabel.translatesAutoresizingMaskIntoConstraints = false
        nameLabel.font = NSFont.systemFont(ofSize: 12, weight: .medium)
        nameLabel.lineBreakMode = .byTruncatingMiddle

        // Secondary label: content or path
        let contentLabel = NSTextField(labelWithString: "")
        contentLabel.translatesAutoresizingMaskIntoConstraints = false
        contentLabel.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        contentLabel.lineBreakMode = .byTruncatingTail
        contentLabel.tag = 100

        // Tertiary label: path (for content results)
        let pathLabel = NSTextField(labelWithString: "")
        pathLabel.translatesAutoresizingMaskIntoConstraints = false
        pathLabel.font = NSFont.systemFont(ofSize: 10)
        pathLabel.textColor = .tertiaryLabelColor
        pathLabel.lineBreakMode = .byTruncatingHead
        pathLabel.tag = 101

        cell.addSubview(nameLabel)
        cell.addSubview(contentLabel)
        cell.addSubview(pathLabel)

        cell.textField = nameLabel

        NSLayoutConstraint.activate([
            nameLabel.topAnchor.constraint(equalTo: cell.topAnchor, constant: 3),
            nameLabel.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 8),
            nameLabel.trailingAnchor.constraint(lessThanOrEqualTo: cell.trailingAnchor, constant: -8),

            contentLabel.topAnchor.constraint(equalTo: nameLabel.bottomAnchor, constant: 1),
            contentLabel.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 8),
            contentLabel.trailingAnchor.constraint(lessThanOrEqualTo: cell.trailingAnchor, constant: -8),

            pathLabel.topAnchor.constraint(equalTo: contentLabel.bottomAnchor, constant: 1),
            pathLabel.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 8),
            pathLabel.trailingAnchor.constraint(lessThanOrEqualTo: cell.trailingAnchor, constant: -8),
        ])

        return cell
    }

    // MARK: Helpers

    /// Abbreviate an absolute path relative to the root for display.
    private func abbreviatePath(_ absolutePath: String) -> String {
        guard !rootPath.isEmpty else { return absolutePath }
        let prefix = rootPath.hasSuffix("/") ? rootPath : rootPath + "/"
        if absolutePath.hasPrefix(prefix) {
            return String(absolutePath.dropFirst(prefix.count))
        }
        return absolutePath
    }
}
