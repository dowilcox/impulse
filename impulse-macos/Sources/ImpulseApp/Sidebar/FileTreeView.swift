import AppKit

// MARK: - Notifications

extension Notification.Name {
    /// Posted when the user selects a file in the file tree. The `userInfo`
    /// dictionary contains `"path"` (String) and optionally `"line"` (Int).
    static let impulseOpenFile = Notification.Name("dev.impulse.openFile")
}

// MARK: - File Tree View

/// NSOutlineView-based file tree for the sidebar. Supports lazy-loading of
/// directory children, git status colouring, and hidden file toggling.
final class FileTreeView: NSView {

    // MARK: Properties

    private(set) var outlineView: NSOutlineView!
    private(set) var scrollView: NSScrollView!

    private(set) var rootNodes: [FileTreeNode] = []
    private(set) var rootPath: String = ""
    var showHidden: Bool = false

    // Column identifier
    private let fileColumnID = NSUserInterfaceItemIdentifier("FileColumn")
    private let cellID = NSUserInterfaceItemIdentifier("FileCell")

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
        // Outline view
        let outline = NSOutlineView()
        outline.headerView = nil
        outline.indentationPerLevel = 16
        outline.rowHeight = 22
        outline.focusRingType = .none
        outline.allowsMultipleSelection = false
        outline.autoresizesOutlineColumn = true
        outline.usesAlternatingRowBackgroundColors = false
        outline.style = .sourceList
        outline.dataSource = self
        outline.delegate = self

        let column = NSTableColumn(identifier: fileColumnID)
        column.isEditable = false
        column.resizingMask = .autoresizingMask
        outline.addTableColumn(column)
        outline.outlineTableColumn = column

        self.outlineView = outline

        // Scroll view
        let scroll = NSScrollView()
        scroll.documentView = outline
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = false
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false

        addSubview(scroll)
        self.scrollView = scroll

        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: topAnchor),
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        // Single-click action
        outline.target = self
        outline.action = #selector(outlineViewClicked(_:))
    }

    // MARK: Public API

    /// Set (or change) the root project directory. Rebuilds the entire tree,
    /// fetches git status, and reloads the outline view.
    func setRootPath(_ path: String) {
        rootPath = path
        rootNodes = FileTreeNode.buildTree(rootPath: path, showHidden: showHidden)
        FileTreeNode.refreshGitStatus(nodes: rootNodes, rootPath: rootPath)
        outlineView.reloadData()
    }

    /// Re-fetch git status for the current tree and reload visible cells to
    /// reflect any changes.
    func refreshGitStatus() {
        FileTreeNode.refreshGitStatus(nodes: rootNodes, rootPath: rootPath)
        reloadVisibleRows()
    }

    /// Toggle whether hidden (dot) files are shown, then rebuild the tree.
    func toggleHiddenFiles() {
        showHidden.toggle()
        guard !rootPath.isEmpty else { return }
        setRootPath(rootPath)
    }

    // MARK: Click Handling

    @objc private func outlineViewClicked(_ sender: Any?) {
        let row = outlineView.clickedRow
        guard row >= 0, let node = outlineView.item(atRow: row) as? FileTreeNode else { return }

        if node.isDirectory {
            if outlineView.isItemExpanded(node) {
                outlineView.collapseItem(node)
            } else {
                outlineView.expandItem(node)
            }
        } else {
            NotificationCenter.default.post(
                name: .impulseOpenFile,
                object: self,
                userInfo: ["path": node.path]
            )
        }
    }

    // MARK: Private Helpers

    private func reloadVisibleRows() {
        let visibleRange = outlineView.rows(in: outlineView.visibleRect)
        guard visibleRange.length > 0 else {
            outlineView.reloadData()
            return
        }
        let indexSet = IndexSet(integersIn: visibleRange.location ..< (visibleRange.location + visibleRange.length))
        let columnSet = IndexSet(integer: 0)
        outlineView.reloadData(forRowIndexes: indexSet, columnIndexes: columnSet)
    }
}

// MARK: - NSOutlineViewDataSource

extension FileTreeView: NSOutlineViewDataSource {

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        if let node = item as? FileTreeNode {
            return node.children?.count ?? 0
        }
        return rootNodes.count
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        if let node = item as? FileTreeNode, let children = node.children {
            return children[index]
        }
        return rootNodes[index]
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        if let node = item as? FileTreeNode {
            return node.isDirectory
        }
        return false
    }
}

// MARK: - NSOutlineViewDelegate

extension FileTreeView: NSOutlineViewDelegate {

    func outlineView(_ outlineView: NSOutlineView,
                     viewFor tableColumn: NSTableColumn?,
                     item: Any) -> NSView? {
        guard let node = item as? FileTreeNode else { return nil }

        let cell: NSTableCellView
        if let reused = outlineView.makeView(withIdentifier: cellID, owner: self) as? NSTableCellView {
            cell = reused
        } else {
            cell = makeCellView()
        }

        // Icon
        let icon: NSImage
        if node.isDirectory {
            icon = NSImage(systemSymbolName: "folder.fill", accessibilityDescription: "Folder")
                ?? NSImage(named: NSImage.folderName)!
        } else {
            icon = NSWorkspace.shared.icon(forFile: node.path)
        }
        icon.size = NSSize(width: 16, height: 16)
        cell.imageView?.image = icon

        // Name
        cell.textField?.stringValue = node.name
        cell.textField?.textColor = textColor(for: node.gitStatus)
        cell.textField?.lineBreakMode = .byTruncatingMiddle

        return cell
    }

    func outlineViewItemDidExpand(_ notification: Notification) {
        guard let node = notification.userInfo?["NSObject"] as? FileTreeNode else { return }
        node.isExpanded = true

        // Lazy-load children on first expansion.
        if !node.isLoaded {
            node.loadChildren(showHidden: showHidden)
            FileTreeNode.refreshGitStatus(nodes: node.children ?? [], rootPath: rootPath)
            outlineView.reloadItem(node, reloadChildren: true)
        }
    }

    func outlineViewItemWillCollapse(_ notification: Notification) {
        guard let node = notification.userInfo?["NSObject"] as? FileTreeNode else { return }
        node.isExpanded = false
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        let row = outlineView.selectedRow
        guard row >= 0, let node = outlineView.item(atRow: row) as? FileTreeNode else { return }
        guard !node.isDirectory else { return }

        NotificationCenter.default.post(
            name: .impulseOpenFile,
            object: self,
            userInfo: ["path": node.path]
        )
    }

    // MARK: Cell Construction

    private func makeCellView() -> NSTableCellView {
        let cell = NSTableCellView()
        cell.identifier = cellID

        let imageView = NSImageView()
        imageView.translatesAutoresizingMaskIntoConstraints = false
        imageView.imageScaling = .scaleProportionallyUpOrDown

        let textField = NSTextField(labelWithString: "")
        textField.translatesAutoresizingMaskIntoConstraints = false
        textField.font = NSFont.systemFont(ofSize: 13)
        textField.cell?.truncatesLastVisibleLine = true
        textField.lineBreakMode = .byTruncatingMiddle

        cell.addSubview(imageView)
        cell.addSubview(textField)
        cell.imageView = imageView
        cell.textField = textField

        NSLayoutConstraint.activate([
            imageView.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2),
            imageView.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            imageView.widthAnchor.constraint(equalToConstant: 16),
            imageView.heightAnchor.constraint(equalToConstant: 16),

            textField.leadingAnchor.constraint(equalTo: imageView.trailingAnchor, constant: 6),
            textField.trailingAnchor.constraint(lessThanOrEqualTo: cell.trailingAnchor, constant: -4),
            textField.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])

        return cell
    }

    // MARK: Status Colouring

    private func textColor(for status: FileTreeNode.GitStatus) -> NSColor {
        switch status {
        case .none:       return .labelColor
        case .modified:   return NSColor.systemYellow
        case .added:      return NSColor.systemGreen
        case .untracked:  return NSColor.secondaryLabelColor
        case .deleted:    return NSColor.systemRed
        case .renamed:    return NSColor.systemBlue
        case .conflict:   return NSColor.systemOrange
        }
    }
}
