import AppKit

// MARK: - Pointer Outline View

/// NSOutlineView subclass that shows a pointing hand cursor over rows.
private final class PointerOutlineView: NSOutlineView {
    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(visibleRect, cursor: .pointingHand)
    }
}

// MARK: - Hover Row View

/// Custom row view that draws subtle hover and selection backgrounds with
/// rounded corners, giving the file tree a polished native appearance.
private final class HoverRowView: NSTableRowView {

    // Static cached colors to avoid per-frame NSColor allocation.
    private static let guideColor = NSColor.white.withAlphaComponent(0.25)
    private static let hoverColor = NSColor.white.withAlphaComponent(0.05)
    private static let selectionColor = NSColor.white.withAlphaComponent(0.10)

    var indentLevel: Int = 0
    private var isHovered = false
    private var trackingArea: NSTrackingArea?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let existing = trackingArea {
            removeTrackingArea(existing)
        }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        trackingArea = area

        // When rows scroll under a stationary cursor, updateTrackingAreas is
        // called but mouseExited may not fire. Re-check the actual position.
        if let window = window {
            let loc = convert(window.mouseLocationOutsideOfEventStream, from: nil)
            let nowInside = bounds.contains(loc)
            if nowInside != isHovered {
                isHovered = nowInside
                needsDisplay = true
            }
        }
    }

    override func mouseEntered(with event: NSEvent) {
        isHovered = true
        needsDisplay = true
    }

    override func mouseExited(with event: NSEvent) {
        isHovered = false
        needsDisplay = true
    }

    override func drawBackground(in dirtyRect: NSRect) {
        // Draw indent guide lines
        if indentLevel > 0 {
            Self.guideColor.setFill()
            let indentPerLevel: CGFloat = 16
            // The outline view adds its own indentation offset; the guides should
            // align with the start of each indentation level relative to the row's
            // own coordinate system.  The first guide starts at indentPerLevel.
            for level in 0..<indentLevel {
                let x = indentPerLevel * CGFloat(level) + indentPerLevel * 0.5
                let guideRect = NSRect(x: x, y: bounds.minY, width: 1, height: bounds.height)
                guideRect.fill()
            }
        }

        if isHovered && !isSelected {
            let inset = bounds.insetBy(dx: 4, dy: 1)
            let path = NSBezierPath(roundedRect: inset, xRadius: 4, yRadius: 4)
            Self.hoverColor.setFill()
            path.fill()
        }
    }

    override func drawSelection(in dirtyRect: NSRect) {
        let inset = bounds.insetBy(dx: 4, dy: 1)
        let path = NSBezierPath(roundedRect: inset, xRadius: 4, yRadius: 4)
        Self.selectionColor.setFill()
        path.fill()
    }
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

    // Icon cache for themed file icons
    private var iconCache: IconCache?

    // Path-to-node lookup for O(1) node search instead of O(n) tree walk.
    private var nodeByPath: [String: FileTreeNode] = [:]

    // Column identifier
    private let fileColumnID = NSUserInterfaceItemIdentifier("FileColumn")
    private let cellID = NSUserInterfaceItemIdentifier("FileCell")

    // Internal drag pasteboard type for distinguishing internal vs external drops
    private static let internalDragType = NSPasteboard.PasteboardType("dev.impulse.fileTreeDrag")

    // Re-entrancy guard to prevent infinite recursion when reloadItem
    // triggers outlineViewItemDidExpand during a reload.
    private var isReloadingItem = false

    // Set during bulk state restoration so the expand/collapse delegates
    // skip heavy work (reloadItem, watchers, UserDefaults persistence).
    private var isBulkRestoring = false

    // File watcher (root directory)
    private var watchedFileDescriptor: Int32 = -1
    private var dispatchSource: DispatchSourceFileSystemObject?
    private var debounceWorkItem: DispatchWorkItem?

    // Subdirectory watchers — keyed by path
    private var subdirWatchers: [String: (fd: Int32, source: DispatchSourceFileSystemObject)] = [:]

    // .git/index watcher — fires on stage/commit/reset/checkout
    private var gitIndexDescriptor: Int32 = -1
    private var gitIndexSource: DispatchSourceFileSystemObject?
    private var gitIndexDebounce: DispatchWorkItem?

    // Periodic git status timer — catches content changes that don't trigger
    // directory watchers (editing a file doesn't fire the parent dir's DispatchSource).
    private var gitStatusTimer: DispatchSourceTimer?
    private var lastGitStatusHash: Int = 0

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
        // Outline view (uses PointerOutlineView for pointing hand cursor on hover)
        let outline = PointerOutlineView()
        outline.headerView = nil
        outline.indentationPerLevel = 16
        outline.rowHeight = 24
        outline.focusRingType = .none
        outline.allowsMultipleSelection = false
        outline.autoresizesOutlineColumn = true
        outline.usesAlternatingRowBackgroundColors = false
        outline.style = .plain
        outline.backgroundColor = .clear
        outline.dataSource = self
        outline.delegate = self

        let column = NSTableColumn(identifier: fileColumnID)
        column.isEditable = false
        column.resizingMask = .autoresizingMask
        outline.addTableColumn(column)
        outline.outlineTableColumn = column

        // Context menu
        outline.menu = makeContextMenu()

        // Drag and drop
        outline.registerForDraggedTypes([.fileURL, FileTreeView.internalDragType])
        outline.setDraggingSourceOperationMask(.copy, forLocal: false)
        outline.setDraggingSourceOperationMask(.move, forLocal: true)

        self.outlineView = outline

        // Scroll view
        let scroll = NSScrollView()
        scroll.documentView = outline
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = false
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.automaticallyAdjustsContentInsets = false
        scroll.contentInsets = NSEdgeInsets(top: 0, left: 6, bottom: 0, right: 0)

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

    override func layout() {
        super.layout()
        outlineView.sizeLastColumnToFit()
    }

    // MARK: Public API

    /// The best target directory for new file/folder creation: the selected
    /// node's directory (or its parent if a file is selected), falling back
    /// to the project root.
    var selectedDirectory: String {
        let row = outlineView.selectedRow
        if row >= 0, let node = outlineView.item(atRow: row) as? FileTreeNode {
            return targetDirectory(for: node)
        }
        return rootPath
    }

    /// Set (or change) the root project directory. Shows the current (possibly
    /// empty) state immediately, then rebuilds the tree on a background queue
    /// to avoid blocking the main thread on large directories.
    func setRootPath(_ path: String) {
        rootPath = path

        // Start root watcher first — this stops all previous watchers.
        startWatching(path: path)

        // Clear stale nodes so the UI doesn't briefly show the previous
        // directory's contents while the background build runs.
        rootNodes = []
        outlineView.reloadData()

        let hidden = showHidden
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let nodes = FileTreeNode.buildTree(rootPath: path, showHidden: hidden)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.rootPath == path else { return }
                self.rootNodes = nodes

                // Suppress expand/collapse animations and heavy delegate work
                // during the full reload so the tree doesn't visibly flash.
                NSAnimationContext.beginGrouping()
                NSAnimationContext.current.duration = 0
                self.isBulkRestoring = true
                self.outlineView.reloadData()

                // Restore persisted expansion state.
                let savedPaths = self.loadExpandedPaths()
                if !savedPaths.isEmpty {
                    self.restoreExpandedPaths(savedPaths, in: self.rootNodes)
                }
                self.isBulkRestoring = false
                NSAnimationContext.endGrouping()

                self.rebuildNodeIndex()

                // Batch-set up subdirectory watchers for all expanded directories.
                self.watchExpandedSubdirectories(self.rootNodes)

                // Children loaded during bulk restore skipped git status — refresh now.
                self.refreshGitStatus()
            }
        }
    }

    /// Accept a pre-built tree (constructed off the main thread) and update
    /// the UI. Call this from a `DispatchQueue.main.async` block after
    /// building the tree on a background queue.
    func updateTree(nodes: [FileTreeNode], rootPath: String, skipGitRefresh: Bool = false) {
        // Preserve expansion state from the current tree, the incoming
        // (possibly cached) tree, and persisted UserDefaults.
        let expandedPaths = collectExpandedPaths(rootNodes)
        let incomingExpanded = collectExpandedPaths(nodes)
        let savedPaths = loadExpandedPaths()
        let allExpanded = expandedPaths.union(incomingExpanded).union(savedPaths)

        self.rootPath = rootPath
        self.rootNodes = nodes

        // Start root watcher first — this stops all previous watchers.
        startWatching(path: rootPath)

        NSAnimationContext.beginGrouping()
        NSAnimationContext.current.duration = 0
        isBulkRestoring = true
        outlineView.reloadData()
        if !allExpanded.isEmpty {
            restoreExpandedPaths(allExpanded, in: rootNodes)
        }
        isBulkRestoring = false
        NSAnimationContext.endGrouping()

        rebuildNodeIndex()

        // Batch-set up subdirectory watchers for all expanded directories.
        watchExpandedSubdirectories(rootNodes)

        // Children loaded during bulk restore skipped git status — refresh now.
        if !skipGitRefresh {
            refreshGitStatus()
        }
    }

    /// Re-fetch git status for the current tree and reload visible cells to
    /// reflect any changes. Heavy work runs on a background queue.
    func refreshGitStatus() {
        let nodes = rootNodes
        let root = rootPath
        DispatchQueue.global(qos: .utility).async {
            FileTreeNode.refreshGitStatus(nodes: nodes, repoPath: root, dirPath: root)
            DispatchQueue.main.async { [weak self] in
                self?.reloadVisibleRows()
            }
        }
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

    /// Re-apply theme colours so the sidebar background shows through.
    func applyTheme(_ theme: Theme) {
        outlineView.backgroundColor = .clear
        if let cache = iconCache {
            cache.rebuild(theme: theme)
        } else {
            iconCache = IconCache(theme: theme)
        }
        reloadVisibleRows()
    }

    /// Rebuild the tree from disk, preserving expansion state. Heavy work
    /// (filesystem scan + git status) runs on a background queue.
    func refreshTree() {
        guard !rootPath.isEmpty else { return }

        // Collect expanded paths before rebuilding.
        let expandedPaths = collectExpandedPaths(rootNodes)
        let root = rootPath
        let hidden = showHidden

        DispatchQueue.global(qos: .utility).async {
            let newNodes = FileTreeNode.buildTree(rootPath: root, showHidden: hidden)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.rootNodes = newNodes
                NSAnimationContext.beginGrouping()
                NSAnimationContext.current.duration = 0
                self.isBulkRestoring = true
                self.outlineView.reloadData()
                self.restoreExpandedPaths(expandedPaths, in: self.rootNodes)
                self.isBulkRestoring = false
                NSAnimationContext.endGrouping()
                self.rebuildNodeIndex()
                self.watchExpandedSubdirectories(self.rootNodes)
                // Single git status refresh after expansion restoration ensures
                // all expanded children are covered by the batch API call.
                self.refreshGitStatus()
            }
        }
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
            guard let self, !name.isEmpty, !name.contains("/") else { return }
            let fullPath = (dirPath as NSString).appendingPathComponent(name)
            // Path traversal protection: ensure result stays within the target directory.
            let resolvedPath = (fullPath as NSString).standardizingPath
            let resolvedDir = (dirPath as NSString).standardizingPath
            guard resolvedPath.hasPrefix(resolvedDir) else { return }
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
            guard let self, !name.isEmpty, !name.contains("/") else { return }
            let fullPath = (dirPath as NSString).appendingPathComponent(name)
            // Path traversal protection: ensure result stays within the target directory.
            let resolvedPath = (fullPath as NSString).standardizingPath
            let resolvedDir = (dirPath as NSString).standardizingPath
            guard resolvedPath.hasPrefix(resolvedDir) else { return }
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
            guard let self, !newName.isEmpty, newName != oldName, !newName.contains("/") else { return }
            let newPath = (parentDir as NSString).appendingPathComponent(newName)
            // Path traversal protection: ensure result stays within the parent directory.
            let resolvedNew = (newPath as NSString).standardizingPath
            let resolvedParent = (parentDir as NSString).standardizingPath
            guard resolvedNew.hasPrefix(resolvedParent) else { return }
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

        if ImpulseCore.gitDiscardChanges(filePath: node.path, workspaceRoot: rootPath) {
            // Notify any open editor tab to reload from disk.
            NotificationCenter.default.post(
                name: .impulseReloadEditorFile,
                object: self,
                userInfo: ["path": node.path]
            )
            refreshTree()
        } else {
            NSLog("FileTreeView: failed to discard changes for \(node.path)")
        }
    }

    // MARK: Alert Helper

    /// Show a modal alert with a text field for entering a name. Calls
    /// `completion` with the trimmed text (or empty string if cancelled).
    func showNameInputAlert(title: String,
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
        collectExpandedPaths(nodes, into: &paths)
        return paths
    }

    private func collectExpandedPaths(_ nodes: [FileTreeNode], into paths: inout Set<String>) {
        for node in nodes {
            if node.isDirectory && node.isExpanded {
                paths.insert(node.path)
                if let children = node.children {
                    collectExpandedPaths(children, into: &paths)
                }
            }
        }
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

    // MARK: Expansion Persistence

    private static let expandedPathsKeyPrefix = "impulse.fileTree.expandedPaths"

    /// Per-root UserDefaults key so switching projects doesn't clobber
    /// expansion state.
    private var expandedPathsKey: String {
        "\(Self.expandedPathsKeyPrefix).\(rootPath)"
    }

    /// Save the current set of expanded paths to UserDefaults.
    private func saveExpandedPaths() {
        let paths = collectExpandedPaths(rootNodes)
        UserDefaults.standard.set(Array(paths), forKey: expandedPathsKey)
    }

    /// Load the saved set of expanded paths from UserDefaults.
    private func loadExpandedPaths() -> Set<String> {
        let paths = UserDefaults.standard.stringArray(forKey: expandedPathsKey) ?? []
        return Set(paths)
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

        // Also watch .git/index for staging/commit changes and start the
        // periodic git status timer.
        startGitIndexWatcher()
        startGitStatusTimer()
    }

    /// Start watching an expanded subdirectory. Capped at 64 file descriptors
    /// to avoid exhausting the per-process FD limit on deeply nested trees.
    private func watchSubdirectory(_ path: String) {
        guard subdirWatchers[path] == nil else { return }
        guard subdirWatchers.count < 64 else { return }

        let fd = open(path, O_EVTONLY)
        guard fd >= 0 else { return }

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
        subdirWatchers[path] = (fd: fd, source: source)
        source.resume()
    }

    /// Stop watching a collapsed subdirectory.
    private func unwatchSubdirectory(_ path: String) {
        guard let entry = subdirWatchers.removeValue(forKey: path) else { return }
        entry.source.cancel()
    }

    /// Recursively unwatch any expanded children being collapsed.
    private func unwatchExpandedChildren(_ nodes: [FileTreeNode]) {
        for node in nodes {
            if node.isDirectory && node.isExpanded {
                unwatchSubdirectory(node.path)
                if let children = node.children {
                    unwatchExpandedChildren(children)
                }
            }
        }
    }

    /// Set up watchers for all currently expanded subdirectories in a batch.
    private func watchExpandedSubdirectories(_ nodes: [FileTreeNode]) {
        for node in nodes {
            if node.isDirectory && node.isExpanded {
                watchSubdirectory(node.path)
                if let children = node.children {
                    watchExpandedSubdirectories(children)
                }
            }
        }
    }

    /// Stop all subdirectory watchers.
    private func stopAllSubdirWatchers() {
        for (_, entry) in subdirWatchers {
            entry.source.cancel()
        }
        subdirWatchers.removeAll()
    }

    /// Stop the current filesystem watcher and close the file descriptor.
    private func stopWatching() {
        debounceWorkItem?.cancel()
        debounceWorkItem = nil

        stopAllSubdirWatchers()
        stopGitIndexWatcher()
        stopGitStatusTimer()

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

    // MARK: .git/index Watcher

    /// Find the `.git/index` file for the current root and watch it.
    /// Fires on stage, commit, reset, checkout — any index mutation.
    private func startGitIndexWatcher() {
        stopGitIndexWatcher()
        guard !rootPath.isEmpty else { return }

        let currentRootPath = rootPath

        // Run git rev-parse on a background thread to avoid blocking the main
        // thread.  Calling `waitUntilExit` on the main thread pumps the run
        // loop, which can trigger NSOutlineView layout while the tree data is
        // still being updated — leading to index-out-of-range crashes.
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let pipe = Pipe()
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/usr/bin/git")
            proc.arguments = ["rev-parse", "--show-toplevel"]
            proc.currentDirectoryURL = URL(fileURLWithPath: currentRootPath)
            proc.standardOutput = pipe
            proc.standardError = FileHandle.nullDevice
            do { try proc.run() } catch { return }
            proc.waitUntilExit()
            guard proc.terminationStatus == 0 else { return }
            let gitRoot = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard !gitRoot.isEmpty else { return }

            let indexPath = (gitRoot as NSString).appendingPathComponent(".git/index")

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                // If the root changed while we were resolving, bail out.
                guard self.rootPath == currentRootPath else { return }

                let fd = open(indexPath, O_EVTONLY)
                guard fd >= 0 else { return }
                self.gitIndexDescriptor = fd

                let source = DispatchSource.makeFileSystemObjectSource(
                    fileDescriptor: fd,
                    eventMask: [.write, .rename, .delete],
                    queue: .main
                )
                source.setEventHandler { [weak self] in
                    self?.handleGitIndexEvent()
                }
                source.setCancelHandler { [fd] in
                    close(fd)
                }
                self.gitIndexSource = source
                source.resume()
            }
        }
    }

    private func stopGitIndexWatcher() {
        gitIndexDebounce?.cancel()
        gitIndexDebounce = nil
        if let source = gitIndexSource {
            source.cancel()
            gitIndexSource = nil
            gitIndexDescriptor = -1
        } else if gitIndexDescriptor >= 0 {
            close(gitIndexDescriptor)
            gitIndexDescriptor = -1
        }
    }

    /// Debounced handler for .git/index changes — refreshes git status only
    /// (no full tree rebuild needed).
    private func handleGitIndexEvent() {
        gitIndexDebounce?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.refreshGitStatus()
            // The index file may have been replaced (atomic write); rewatch.
            self.startGitIndexWatcher()
        }
        gitIndexDebounce = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
    }

    // MARK: Periodic Git Status Timer

    /// Start a repeating timer that polls git status every 2 seconds.
    /// Catches file-content edits that directory watchers can't see.
    private func startGitStatusTimer() {
        stopGitStatusTimer()
        guard !rootPath.isEmpty else { return }

        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now() + 2, repeating: 2, leeway: .milliseconds(500))
        timer.setEventHandler { [weak self] in
            self?.pollGitStatus()
        }
        gitStatusTimer = timer
        timer.resume()
    }

    private func stopGitStatusTimer() {
        gitStatusTimer?.cancel()
        gitStatusTimer = nil
    }

    /// Lightweight poll: fetch batch git statuses via libgit2 and only update
    /// the tree if the status map changed since the last poll.
    private func pollGitStatus() {
        guard !rootPath.isEmpty else { return }
        let root = rootPath
        let nodes = rootNodes
        let previousHash = lastGitStatusHash
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let batchStatuses = ImpulseCore.getAllGitStatuses(repoPath: root)

            // Compute a stable hash from sorted status entries.
            var hasher = Hasher()
            for dirPath in batchStatuses.keys.sorted() {
                hasher.combine(dirPath)
                let entries = batchStatuses[dirPath]!
                for name in entries.keys.sorted() {
                    hasher.combine(name)
                    hasher.combine(entries[name])
                }
            }
            let hash = hasher.finalize()

            guard hash != previousHash else { return }

            // Hash changed — apply statuses directly from the batch result.
            FileTreeNode.refreshGitStatus(nodes: nodes, repoPath: root, dirPath: root)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.lastGitStatusHash = hash
                self.reloadVisibleRows()
            }
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

    /// Rebuild the `nodeByPath` lookup dictionary from the current tree.
    private func rebuildNodeIndex() {
        nodeByPath.removeAll()
        indexNodes(rootNodes)
    }

    private func indexNodes(_ nodes: [FileTreeNode]) {
        for node in nodes {
            nodeByPath[node.path] = node
            if let children = node.children {
                indexNodes(children)
            }
        }
    }

    /// Find a node by its file path using the O(1) lookup dictionary.
    /// Falls back to tree walk if the index is stale.
    private func findNode(withPath path: String, in nodes: [FileTreeNode]) -> FileTreeNode? {
        if let node = nodeByPath[path] { return node }
        // Fallback: linear scan (index may be stale after lazy load).
        for node in nodes {
            if node.path == path { return node }
            if let children = node.children,
               let found = findNode(withPath: path, in: children) {
                return found
            }
        }
        return nil
    }

    private func reloadVisibleRows() {
        outlineView.sizeLastColumnToFit()
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
            if index < children.count {
                return children[index]
            }
            // Stale outline view state — schedule a reload and return a
            // placeholder so we don't crash.
            DispatchQueue.main.async { [weak outlineView] in
                outlineView?.reloadData()
            }
            return FileTreeNode(name: "", path: "", isDirectory: false)
        }
        if index < rootNodes.count {
            return rootNodes[index]
        }
        DispatchQueue.main.async { [weak outlineView] in
            outlineView?.reloadData()
        }
        return FileTreeNode(name: "", path: "", isDirectory: false)
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        if let node = item as? FileTreeNode {
            return node.isDirectory
        }
        return false
    }

    // MARK: Drag Source

    func outlineView(_ outlineView: NSOutlineView,
                     pasteboardWriterForItem item: Any) -> NSPasteboardWriting? {
        guard let node = item as? FileTreeNode else { return nil }
        return NSURL(fileURLWithPath: node.path)
    }

    func outlineView(_ outlineView: NSOutlineView,
                     draggingSession session: NSDraggingSession,
                     willBeginAt screenPoint: NSPoint,
                     forItems draggedItems: [Any]) {
        // Mark the drag as internal so validateDrop can distinguish it
        session.draggingPasteboard.setString("internal", forType: FileTreeView.internalDragType)
    }

    // MARK: Drop Target

    func outlineView(_ outlineView: NSOutlineView,
                     validateDrop info: NSDraggingInfo,
                     proposedItem item: Any?,
                     proposedChildIndex index: Int) -> NSDragOperation {
        // Determine the target directory
        var targetDir: String
        if let node = item as? FileTreeNode {
            if node.isDirectory {
                targetDir = node.path
            } else {
                // Retarget: drop on a file -> target its parent directory
                let parentPath = (node.path as NSString).deletingLastPathComponent
                if let parentNode = findNode(withPath: parentPath, in: rootNodes) {
                    outlineView.setDropItem(parentNode, dropChildIndex: NSOutlineViewDropOnItemIndex)
                } else {
                    outlineView.setDropItem(nil, dropChildIndex: NSOutlineViewDropOnItemIndex)
                }
                targetDir = parentPath
            }
        } else {
            // Drop on root
            targetDir = rootPath
        }

        let isInternal = info.draggingPasteboard.string(forType: FileTreeView.internalDragType) != nil

        if isInternal {
            guard let urls = info.draggingPasteboard.readObjects(
                forClasses: [NSURL.self],
                options: [.urlReadingFileURLsOnly: true]
            ) as? [NSURL],
                  let sourceURL = urls.first,
                  let sourcePath = sourceURL.path else {
                return []
            }

            // Don't move a directory into itself
            if targetDir.hasPrefix(sourcePath + "/") || targetDir == sourcePath {
                return []
            }

            // No-op: already in target directory
            let sourceParent = (sourcePath as NSString).deletingLastPathComponent
            if sourceParent == targetDir {
                return []
            }

            // Don't overwrite existing items
            let fileName = (sourcePath as NSString).lastPathComponent
            let destPath = (targetDir as NSString).appendingPathComponent(fileName)
            if FileManager.default.fileExists(atPath: destPath) {
                return []
            }

            return .move
        } else {
            return .copy
        }
    }

    func outlineView(_ outlineView: NSOutlineView,
                     acceptDrop info: NSDraggingInfo,
                     item: Any?,
                     childIndex index: Int) -> Bool {
        let targetDir: String
        if let node = item as? FileTreeNode {
            targetDir = node.isDirectory
                ? node.path
                : (node.path as NSString).deletingLastPathComponent
        } else {
            targetDir = rootPath
        }

        let isInternal = info.draggingPasteboard.string(forType: FileTreeView.internalDragType) != nil

        guard let urls = info.draggingPasteboard.readObjects(
            forClasses: [NSURL.self],
            options: [.urlReadingFileURLsOnly: true]
        ) as? [NSURL] else {
            return false
        }

        let fm = FileManager.default
        var anySuccess = false

        for url in urls {
            guard let sourcePath = url.path else { continue }
            let fileName = (sourcePath as NSString).lastPathComponent
            let destPath = (targetDir as NSString).appendingPathComponent(fileName)

            guard !fm.fileExists(atPath: destPath) else {
                NSLog("FileTreeView: skipping drop — \(fileName) already exists in target")
                continue
            }

            do {
                if isInternal {
                    try fm.moveItem(atPath: sourcePath, toPath: destPath)
                } else {
                    try fm.copyItem(atPath: sourcePath, toPath: destPath)
                }
                anySuccess = true
            } catch {
                NSLog("FileTreeView: drop failed for \(sourcePath): \(error)")
            }
        }

        if anySuccess {
            refreshTree()
        }
        return anySuccess
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

        // Icon: use themed SVG icons from the cache, with system fallback
        let isExpanded = node.isDirectory && outlineView.isItemExpanded(node)
        if let themedIcon = iconCache?.icon(filename: node.name, isDirectory: node.isDirectory, expanded: isExpanded) {
            cell.imageView?.image = themedIcon
        } else {
            let fallback: NSImage
            if node.isDirectory {
                fallback = NSImage(systemSymbolName: "folder.fill", accessibilityDescription: "Folder")
                    ?? NSImage(named: NSImage.folderName)!
            } else {
                fallback = NSWorkspace.shared.icon(forFile: node.path)
            }
            fallback.size = NSSize(width: 14, height: 14)
            cell.imageView?.image = fallback
        }

        // Name
        cell.textField?.stringValue = node.name
        cell.textField?.textColor = textColor(for: node.gitStatus)
        cell.textField?.lineBreakMode = .byTruncatingMiddle

        // Git status badge (right-aligned letter)
        if let badge = cell.viewWithTag(FileTreeView.gitBadgeTag) as? NSTextField {
            let (badgeText, badgeColor) = badgeInfo(for: node.gitStatus)
            if let text = badgeText {
                badge.stringValue = text
                badge.textColor = badgeColor
                badge.isHidden = false
            } else {
                badge.stringValue = ""
                badge.isHidden = true
            }
        }

        return cell
    }

    func outlineView(_ outlineView: NSOutlineView, rowViewForItem item: Any) -> NSTableRowView? {
        let rowID = NSUserInterfaceItemIdentifier("HoverRow")
        let rowView: HoverRowView
        if let reused = outlineView.makeView(withIdentifier: rowID, owner: self) as? HoverRowView {
            rowView = reused
        } else {
            let newRow = HoverRowView()
            newRow.identifier = rowID
            rowView = newRow
        }
        rowView.indentLevel = outlineView.level(forItem: item)
        return rowView
    }

    func outlineViewItemDidExpand(_ notification: Notification) {
        guard !isReloadingItem else { return }
        guard let node = notification.userInfo?["NSObject"] as? FileTreeNode else { return }
        node.isExpanded = true

        // Lazy-load children on first expansion.
        var didLoadChildren = false
        if !node.isLoaded {
            node.loadChildren(showHidden: showHidden)
            didLoadChildren = true
            // Index newly loaded children for O(1) path lookup.
            if let children = node.children {
                indexNodes(children)
            }
            if !isBulkRestoring {
                // Dispatch git status to background to avoid blocking the main thread.
                let children = node.children ?? []
                let root = rootPath
                let nodePath = node.path
                DispatchQueue.global(qos: .utility).async { [weak self] in
                    FileTreeNode.refreshGitStatus(
                        nodes: children, repoPath: root, dirPath: nodePath
                    )
                    DispatchQueue.main.async {
                        self?.reloadVisibleRows()
                    }
                }
            }
        }

        // During bulk restore, only reload if we just loaded children for the
        // first time. For already-loaded (cached) nodes, reloadData() +
        // expandItem() is sufficient — no per-node reload needed.
        if didLoadChildren || !isBulkRestoring {
            isReloadingItem = true
            outlineView.reloadItem(node, reloadChildren: true)
            isReloadingItem = false
        }

        if !isBulkRestoring {
            // Watch this subdirectory for changes.
            watchSubdirectory(node.path)
            saveExpandedPaths()
        }
    }

    func outlineViewItemWillCollapse(_ notification: Notification) {
        guard let node = notification.userInfo?["NSObject"] as? FileTreeNode else { return }
        node.isExpanded = false

        // Stop watching this subdirectory and any expanded children.
        unwatchSubdirectory(node.path)
        if let children = node.children {
            unwatchExpandedChildren(children)
        }
        // Reload to update the folder icon (open -> closed).
        // Use async to let the collapse animation complete first.
        DispatchQueue.main.async { [weak self] in
            self?.outlineView.reloadItem(node, reloadChildren: false)
        }

        // Persist expansion state.
        saveExpandedPaths()
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        // File opening is handled by outlineViewClicked to avoid double-firing.
        // This delegate method is intentionally left empty.
    }

    // MARK: Cell Construction

    /// Tag used to identify the git status badge label within a cell view.
    private static let gitBadgeTag = 42

    private func makeCellView() -> NSTableCellView {
        let cell = NSTableCellView()
        cell.identifier = cellID

        let imageView = NSImageView()
        imageView.translatesAutoresizingMaskIntoConstraints = false
        imageView.imageScaling = .scaleProportionallyUpOrDown

        let textField = NSTextField(labelWithString: "")
        textField.translatesAutoresizingMaskIntoConstraints = false
        textField.font = NSFont.appFont(ofSize: 13)
        textField.cell?.truncatesLastVisibleLine = true
        textField.lineBreakMode = .byTruncatingMiddle

        let badge = NSTextField(labelWithString: "")
        badge.translatesAutoresizingMaskIntoConstraints = false
        badge.font = NSFont.appFont(ofSize: 11, weight: .semibold)
        badge.alignment = .center
        badge.tag = FileTreeView.gitBadgeTag

        cell.addSubview(imageView)
        cell.addSubview(textField)
        cell.addSubview(badge)
        cell.imageView = imageView
        cell.textField = textField

        NSLayoutConstraint.activate([
            imageView.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2),
            imageView.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            imageView.widthAnchor.constraint(equalToConstant: 14),
            imageView.heightAnchor.constraint(equalToConstant: 14),

            badge.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -8),
            badge.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            badge.widthAnchor.constraint(greaterThanOrEqualToConstant: 14),

            textField.leadingAnchor.constraint(equalTo: imageView.trailingAnchor, constant: 8),
            textField.trailingAnchor.constraint(lessThanOrEqualTo: badge.leadingAnchor, constant: -4),
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
        case .untracked:  return NSColor.systemGreen
        case .deleted:    return NSColor.systemRed
        case .renamed:    return NSColor.systemBlue
        case .conflict:   return NSColor.systemOrange
        }
    }

    private func badgeInfo(for status: FileTreeNode.GitStatus) -> (String?, NSColor) {
        switch status {
        case .none:       return (nil, .labelColor)
        case .modified:   return ("M", NSColor.systemYellow)
        case .added:      return ("A", NSColor.systemGreen)
        case .untracked:  return ("?", NSColor.systemGreen)
        case .deleted:    return ("D", NSColor.systemRed)
        case .renamed:    return ("R", NSColor.systemBlue)
        case .conflict:   return ("C", NSColor.systemOrange)
        }
    }
}
