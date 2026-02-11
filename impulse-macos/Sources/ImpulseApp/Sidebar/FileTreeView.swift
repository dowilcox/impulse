import AppKit

// MARK: - Notifications

extension Notification.Name {
    /// Posted when the user selects a file in the file tree. The `userInfo`
    /// dictionary contains `"path"` (String) and optionally `"line"` (Int).
    static let impulseOpenFile = Notification.Name("dev.impulse.openFile")
}

// MARK: - File Tree View

/// NSOutlineView-based file tree for the sidebar. Supports lazy-loading of
/// directory children, git status colouring, hidden file toggling, context
/// menus for file operations, and filesystem watching for auto-refresh.
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

    // File watcher
    private var watchedFileDescriptor: Int32 = -1
    private var dispatchSource: DispatchSourceFileSystemObject?
    private var debounceWorkItem: DispatchWorkItem?

    // MARK: Initialisation

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setup()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setup()
    }

    deinit {
        stopWatching()
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

        // Context menu
        outline.menu = makeContextMenu()

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
    /// fetches git status, reloads the outline view, and starts watching for
    /// filesystem changes.
    func setRootPath(_ path: String) {
        rootPath = path
        rootNodes = FileTreeNode.buildTree(rootPath: path, showHidden: showHidden)
        FileTreeNode.refreshGitStatus(nodes: rootNodes, rootPath: rootPath)
        outlineView.reloadData()
        startWatching(path: path)
    }

    /// Re-fetch git status for the current tree and reload visible cells to
    /// reflect any changes.
    func refreshGitStatus() {
        FileTreeNode.refreshGitStatus(nodes: rootNodes, rootPath: rootPath)
        reloadVisibleRows()
    }

    /// Collapse all expanded directories back to root-level only.
    func collapseAll() {
        for node in rootNodes {
            collapseRecursively(node)
        }
    }

    private func collapseRecursively(_ node: FileTreeNode) {
        if node.isDirectory && node.isExpanded {
            if let children = node.children {
                for child in children {
                    collapseRecursively(child)
                }
            }
            outlineView.collapseItem(node)
        }
    }

    /// Toggle whether hidden (dot) files are shown, then rebuild the tree.
    func toggleHiddenFiles() {
        showHidden.toggle()
        guard !rootPath.isEmpty else { return }
        setRootPath(rootPath)
    }

    /// Rebuild the tree from disk, preserving expansion state.
    func refreshTree() {
        guard !rootPath.isEmpty else { return }

        // Collect expanded paths before rebuilding.
        let expandedPaths = collectExpandedPaths(rootNodes)

        rootNodes = FileTreeNode.buildTree(rootPath: rootPath, showHidden: showHidden)
        FileTreeNode.refreshGitStatus(nodes: rootNodes, rootPath: rootPath)
        outlineView.reloadData()

        // Re-expand previously expanded directories.
        restoreExpandedPaths(expandedPaths, in: rootNodes)
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

    // MARK: - Context Menu

    /// Build the right-click context menu for the outline view.
    private func makeContextMenu() -> NSMenu {
        let menu = NSMenu()
        menu.delegate = self
        return menu
    }

    /// Return the `FileTreeNode` for the row that was right-clicked, or `nil` if
    /// the click landed on empty space.
    private func clickedNode() -> FileTreeNode? {
        let row = outlineView.clickedRow
        guard row >= 0 else { return nil }
        return outlineView.item(atRow: row) as? FileTreeNode
    }

    /// The directory to use for "New File" / "New Folder" when the user
    /// right-clicks a node. If the node is a file, we use its parent directory.
    private func targetDirectory(for node: FileTreeNode) -> String {
        if node.isDirectory {
            return node.path
        }
        return (node.path as NSString).deletingLastPathComponent
    }

    // MARK: Context Menu Actions

    @objc private func contextNewFile(_ sender: Any?) {
        guard let node = clickedNode() else { return }
        let dirPath = targetDirectory(for: node)
        showNameInputAlert(title: "New File",
                           message: "Enter a name for the new file:",
                           placeholder: "untitled",
                           defaultValue: "") { [weak self] name in
            guard let self, !name.isEmpty else { return }
            let fullPath = (dirPath as NSString).appendingPathComponent(name)
            let fm = FileManager.default
            guard fm.createFile(atPath: fullPath, contents: nil) else {
                NSLog("FileTreeView: failed to create file at \(fullPath)")
                return
            }
            self.refreshTree()
        }
    }

    @objc private func contextNewFolder(_ sender: Any?) {
        guard let node = clickedNode() else { return }
        let dirPath = targetDirectory(for: node)
        showNameInputAlert(title: "New Folder",
                           message: "Enter a name for the new folder:",
                           placeholder: "untitled-folder",
                           defaultValue: "") { [weak self] name in
            guard let self, !name.isEmpty else { return }
            let fullPath = (dirPath as NSString).appendingPathComponent(name)
            do {
                try FileManager.default.createDirectory(atPath: fullPath,
                                                        withIntermediateDirectories: false)
            } catch {
                NSLog("FileTreeView: failed to create folder at \(fullPath): \(error)")
                return
            }
            self.refreshTree()
        }
    }

    @objc private func contextRename(_ sender: Any?) {
        guard let node = clickedNode() else { return }
        let oldName = (node.path as NSString).lastPathComponent
        let parentDir = (node.path as NSString).deletingLastPathComponent
        showNameInputAlert(title: "Rename",
                           message: "Enter a new name:",
                           placeholder: oldName,
                           defaultValue: oldName) { [weak self] newName in
            guard let self, !newName.isEmpty, newName != oldName else { return }
            let newPath = (parentDir as NSString).appendingPathComponent(newName)
            do {
                try FileManager.default.moveItem(atPath: node.path, toPath: newPath)
            } catch {
                NSLog("FileTreeView: failed to rename \(node.path) to \(newPath): \(error)")
                return
            }
            self.refreshTree()
        }
    }

    @objc private func contextDelete(_ sender: Any?) {
        guard let node = clickedNode() else { return }
        let name = (node.path as NSString).lastPathComponent
        let isDir = node.isDirectory

        let alert = NSAlert()
        alert.messageText = "Delete \(isDir ? "Folder" : "File")"
        alert.informativeText = "Are you sure you want to delete \"\(name)\"?"
            + (isDir ? " This will delete the directory and all its contents." : "")
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Delete")
        alert.addButton(withTitle: "Cancel")

        // Style the Delete button as destructive.
        alert.buttons.first?.hasDestructiveAction = true

        guard alert.runModal() == .alertFirstButtonReturn else { return }

        do {
            try FileManager.default.removeItem(atPath: node.path)
        } catch {
            NSLog("FileTreeView: failed to delete \(node.path): \(error)")
            return
        }
        refreshTree()
    }

    @objc private func contextCopyPath(_ sender: Any?) {
        guard let node = clickedNode() else { return }
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(node.path, forType: .string)
    }

    @objc private func contextOpenInTerminal(_ sender: Any?) {
        guard let node = clickedNode(), node.isDirectory else { return }
        NotificationCenter.default.post(
            name: .impulseNewTerminalTab,
            object: self,
            userInfo: ["directory": node.path]
        )
    }

    @objc private func contextOpenInDefaultApp(_ sender: Any?) {
        guard let node = clickedNode() else { return }
        NSWorkspace.shared.open(URL(fileURLWithPath: node.path))
    }

    @objc private func contextDiscardChanges(_ sender: Any?) {
        guard let node = clickedNode() else { return }
        let name = (node.path as NSString).lastPathComponent

        let alert = NSAlert()
        alert.messageText = "Discard Changes"
        alert.informativeText = "Are you sure you want to discard all changes to \"\(name)\"? This cannot be undone."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Discard")
        alert.addButton(withTitle: "Cancel")
        alert.buttons.first?.hasDestructiveAction = true

        guard alert.runModal() == .alertFirstButtonReturn else { return }

        if ImpulseCore.gitDiscardChanges(filePath: node.path) {
            refreshTree()
        } else {
            NSLog("FileTreeView: failed to discard changes for \(node.path)")
        }
    }

    // MARK: Alert Helper

    /// Show a modal alert with a text field for entering a name. Calls
    /// `completion` with the trimmed text (or empty string if cancelled).
    private func showNameInputAlert(title: String,
                                    message: String,
                                    placeholder: String,
                                    defaultValue: String,
                                    completion: @escaping (String) -> Void) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")

        let textField = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        textField.placeholderString = placeholder
        textField.stringValue = defaultValue
        alert.accessoryView = textField

        // Make the text field the first responder so it receives focus.
        alert.window.initialFirstResponder = textField

        // If there is an extension, select just the stem so the user can
        // easily type a new name without losing the extension.
        if !defaultValue.isEmpty, let dotRange = defaultValue.range(of: ".", options: .backwards) {
            let stemLength = defaultValue.distance(from: defaultValue.startIndex, to: dotRange.lowerBound)
            textField.currentEditor()?.selectedRange = NSRange(location: 0, length: stemLength)
        }

        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            completion(textField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }

    // MARK: Expansion State Helpers

    /// Recursively collect the paths of all expanded directories.
    private func collectExpandedPaths(_ nodes: [FileTreeNode]) -> Set<String> {
        var paths = Set<String>()
        for node in nodes {
            if node.isDirectory && node.isExpanded {
                paths.insert(node.path)
                if let children = node.children {
                    paths.formUnion(collectExpandedPaths(children))
                }
            }
        }
        return paths
    }

    /// After a reload, re-expand directories whose paths match the saved set.
    /// This triggers lazy loading via the delegate as needed.
    private func restoreExpandedPaths(_ paths: Set<String>, in nodes: [FileTreeNode]) {
        for node in nodes {
            if node.isDirectory && paths.contains(node.path) {
                outlineView.expandItem(node)
                if let children = node.children {
                    restoreExpandedPaths(paths, in: children)
                }
            }
        }
    }

    // MARK: - File System Watching

    /// Start watching the root directory for filesystem changes.
    private func startWatching(path: String) {
        stopWatching()

        let fd = open(path, O_EVTONLY)
        guard fd >= 0 else {
            NSLog("FileTreeView: failed to open \(path) for watching (errno \(errno))")
            return
        }
        watchedFileDescriptor = fd

        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: fd,
            eventMask: [.write, .rename, .delete, .link],
            queue: .main
        )

        source.setEventHandler { [weak self] in
            self?.handleFileSystemEvent()
        }

        source.setCancelHandler { [fd] in
            close(fd)
        }

        self.dispatchSource = source
        source.resume()
    }

    /// Stop the current filesystem watcher and close the file descriptor.
    private func stopWatching() {
        debounceWorkItem?.cancel()
        debounceWorkItem = nil

        if let source = dispatchSource {
            source.cancel()
            dispatchSource = nil
            // The cancel handler closes the fd, so reset our copy.
            watchedFileDescriptor = -1
        } else if watchedFileDescriptor >= 0 {
            close(watchedFileDescriptor)
            watchedFileDescriptor = -1
        }
    }

    /// Called when the dispatch source fires. Debounces rapid events by
    /// scheduling a refresh 300ms in the future.
    private func handleFileSystemEvent() {
        debounceWorkItem?.cancel()
        let work = DispatchWorkItem { [weak self] in
            self?.refreshTree()
        }
        debounceWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
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

// MARK: - NSMenuDelegate

extension FileTreeView: NSMenuDelegate {

    /// Populate the context menu dynamically based on the right-clicked row.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()

        guard let node = clickedNode() else { return }

        menu.addItem(withTitle: "New File",
                     action: #selector(contextNewFile(_:)),
                     keyEquivalent: "").target = self
        menu.addItem(withTitle: "New Folder",
                     action: #selector(contextNewFolder(_:)),
                     keyEquivalent: "").target = self
        menu.addItem(.separator())

        menu.addItem(withTitle: "Rename",
                     action: #selector(contextRename(_:)),
                     keyEquivalent: "").target = self
        let deleteItem = menu.addItem(withTitle: "Delete",
                                      action: #selector(contextDelete(_:)),
                                      keyEquivalent: "")
        deleteItem.target = self

        // Show "Discard Changes" for git-modified/added files.
        if !node.isDirectory,
           node.gitStatus == .modified || node.gitStatus == .added {
            let discardItem = menu.addItem(withTitle: "Discard Changes",
                                            action: #selector(contextDiscardChanges(_:)),
                                            keyEquivalent: "")
            discardItem.target = self
        }

        menu.addItem(.separator())

        menu.addItem(withTitle: "Copy Path",
                     action: #selector(contextCopyPath(_:)),
                     keyEquivalent: "").target = self

        // "Open in Default App" for files (uses system default application).
        if !node.isDirectory {
            menu.addItem(withTitle: "Open in Default App",
                         action: #selector(contextOpenInDefaultApp(_:)),
                         keyEquivalent: "").target = self
        }

        if node.isDirectory {
            menu.addItem(withTitle: "Open in Terminal",
                         action: #selector(contextOpenInTerminal(_:)),
                         keyEquivalent: "").target = self
        }
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
