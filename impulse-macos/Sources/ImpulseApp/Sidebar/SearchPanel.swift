import AppKit

// SearchResult is defined in Bridge/ImpulseCore.swift

// MARK: - Search Mode

private enum SearchMode: Int {
    case files = 0
    case content = 1
}

// MARK: - Search Panel

/// Project-wide file name and content search panel for the sidebar.
/// Uses the FFI bridge to `impulse-core` search functions and displays results
/// in an NSTableView.
final class SearchPanel: NSView {

    // MARK: Properties

    private var searchField: NSSearchField!
    private var modeSegment: NSSegmentedControl!
    private var caseSensitiveButton: NSButton!
    private var resultsTableView: NSTableView!
    private var resultsScrollView: NSScrollView!
    private var statusLabel: NSTextField!

    private var results: [SearchResult] = []
    private var rootPath: String = ""

    private var searchMode: SearchMode = .content

    /// Debounce timer for search-as-you-type.
    private var debounceTimer: Timer?
    private let debounceInterval: TimeInterval = 0.3

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
        field.placeholderString = "Search project..."
        field.translatesAutoresizingMaskIntoConstraints = false
        field.sendsWholeSearchString = false
        field.sendsSearchStringImmediately = false
        field.delegate = self
        self.searchField = field

        // --- Mode segmented control ---
        let segment = NSSegmentedControl(labels: ["Files", "Content"], trackingMode: .selectOne, target: self, action: #selector(searchModeChanged(_:)))
        segment.selectedSegment = SearchMode.content.rawValue
        segment.translatesAutoresizingMaskIntoConstraints = false
        segment.segmentStyle = .texturedRounded
        segment.controlSize = .small
        self.modeSegment = segment

        // --- Case-sensitive toggle ---
        let caseBtn = NSButton(title: "Aa", target: self, action: #selector(caseSensitiveToggled(_:)))
        caseBtn.setButtonType(.toggle)
        caseBtn.bezelStyle = .texturedRounded
        caseBtn.controlSize = .small
        caseBtn.toolTip = "Case sensitive"
        caseBtn.translatesAutoresizingMaskIntoConstraints = false
        caseBtn.state = .off
        self.caseSensitiveButton = caseBtn

        // Options row: segment + case toggle
        let optionsRow = NSStackView(views: [segment, caseBtn])
        optionsRow.orientation = .horizontal
        optionsRow.spacing = 6
        optionsRow.translatesAutoresizingMaskIntoConstraints = false
        optionsRow.distribution = .fill

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
        let stack = NSStackView(views: [field, optionsRow, scroll, label])
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
        resultsTableView.backgroundColor = .clear
        statusLabel.textColor = theme.fgDark
    }

    // MARK: Actions

    @objc private func searchModeChanged(_ sender: NSSegmentedControl) {
        searchMode = SearchMode(rawValue: sender.selectedSegment) ?? .content
        // Hide case-sensitive button for file search (filename search is always case-insensitive).
        caseSensitiveButton.isHidden = (searchMode == .files)
        triggerSearch()
    }

    @objc private func caseSensitiveToggled(_ sender: NSButton) {
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

        // Capture UI state on the main thread before dispatching to background.
        let mode = self.searchMode
        let root = self.rootPath
        let caseSensitive = (self.caseSensitiveButton.state == .on)

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard self != nil else { return }

            let decoded: [SearchResult]

            switch mode {
            case .files:
                decoded = ImpulseCore.searchFiles(root: root, query: query)

            case .content:
                decoded = ImpulseCore.searchContent(root: root,
                                                    query: query,
                                                    caseSensitive: caseSensitive)
            }

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.results = decoded
                self.resultsTableView.reloadData()
                if decoded.isEmpty {
                    self.statusLabel.stringValue = "No results"
                } else {
                    let noun = decoded.count == 1 ? "result" : "results"
                    self.statusLabel.stringValue = "\(decoded.count) \(noun)"
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
            // Reuse existing cell, but we need to update the secondary label.
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
