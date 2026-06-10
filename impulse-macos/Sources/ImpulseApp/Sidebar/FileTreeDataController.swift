import AppKit

/// Headless owner of the sidebar file tree data.
///
/// Holds the root nodes, filesystem watchers (root + expanded subdirectories
/// + .git/index), the periodic git status poll, incremental patch
/// application, and expansion-state persistence. The SwiftUI sidebar
/// (`FileTreeListView`) renders the nodes via `WindowModel`; this class never
/// touches a view.
///
/// All public entry points must be called on the main thread; heavy work
/// (filesystem scans, git status) is dispatched to background queues
/// internally, mirroring the behaviour of the retired AppKit `FileTreeView`.
final class FileTreeDataController {

    // MARK: Properties

    private(set) var rootNodes: [FileTreeNode] = []
    private(set) var rootPath: String = ""
    var showHidden: Bool = false

    /// Called on the main thread after the tree is rebuilt or patched from a
    /// filesystem watcher event. Passes the new root nodes so the caller can
    /// sync them to WindowModel for the SwiftUI sidebar.
    var onTreeRefreshed: (([FileTreeNode]) -> Void)?

    // Path-to-node lookup for O(1) node search instead of O(n) tree walk.
    private var nodeByPath: [String: FileTreeNode] = [:]

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

    // App activation observers — the git poll timer is paused while the app
    // is in the background to avoid periodic full-repo scans on battery.
    private var appActiveObservers: [NSObjectProtocol] = []

    // Debounce work item for git status refresh.
    private var gitRefreshDebounce: DispatchWorkItem?

    // Guards against overlapping tree rebuilds. If a refresh is requested while
    // one is already in progress, we set needsAnotherRefresh and re-trigger
    // when the current rebuild completes.
    private var isRefreshingTree = false
    private var needsAnotherRefresh = false
    private var pendingFileTreeEvents: [ImpulseCore.FileTreeWatchEvent] = []

    // Guard against overlapping git status calls from poll timer and
    // refreshGitStatus() running concurrently.
    private var isGitStatusInProgress = false
    private var needsAnotherGitStatus = false

    // MARK: Initialisation

    init() {
        // Pause the periodic git status poll while the app is inactive; one
        // immediate poll on reactivation catches changes made while away
        // (e.g. git operations run from another app).
        appActiveObservers.append(
            NotificationCenter.default.addObserver(
                forName: NSApplication.didResignActiveNotification, object: nil, queue: .main
            ) { [weak self] _ in
                self?.stopGitStatusTimer()
            }
        )
        appActiveObservers.append(
            NotificationCenter.default.addObserver(
                forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
            ) { [weak self] _ in
                guard let self, !self.rootPath.isEmpty else { return }
                self.startGitStatusTimer()
                self.pollGitStatus()
            }
        )
    }

    deinit {
        stopWatching()
        appActiveObservers.forEach { NotificationCenter.default.removeObserver($0) }
    }

    // MARK: Public API

    /// Accept a pre-built tree (constructed off the main thread) and adopt it,
    /// preserving expansion state from the current tree, the incoming
    /// (possibly cached) tree, and persisted UserDefaults.
    func updateTree(nodes: [FileTreeNode], rootPath: String, skipGitRefresh: Bool = false) {
        let expandedPaths = collectExpandedPaths(rootNodes)
        let incomingExpanded = collectExpandedPaths(nodes)
        let savedPaths = loadExpandedPaths(forRoot: rootPath)
        let allExpanded = expandedPaths.union(incomingExpanded).union(savedPaths)

        self.rootPath = rootPath
        self.rootNodes = nodes

        // Start root watcher first — this stops all previous watchers.
        startWatching(path: rootPath)

        if !allExpanded.isEmpty {
            applyExpansionState(allExpanded, to: rootNodes)
        }

        rebuildNodeIndex()

        // Batch-set up subdirectory watchers for all expanded directories.
        watchExpandedSubdirectories(rootNodes)

        // Children loaded during expansion restore skipped git status — refresh now.
        if !skipGitRefresh {
            refreshGitStatus()
        }
    }

    /// Re-fetch git status for the current tree. Heavy work runs on a
    /// background queue; SwiftUI rows observe the node mutations directly.
    /// Coalesces overlapping requests: if a git status call is already in
    /// progress, the request is deferred until the current one completes.
    func refreshGitStatus() {
        gitRefreshDebounce?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            guard !self.isGitStatusInProgress else {
                self.needsAnotherGitStatus = true
                return
            }
            self.isGitStatusInProgress = true
            let nodes = self.rootNodes
            let root = self.rootPath
            DispatchQueue.global(qos: .utility).async {
                FileTreeNode.refreshGitStatus(nodes: nodes, repoPath: root, dirPath: root)
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }
                    self.isGitStatusInProgress = false
                    if self.needsAnotherGitStatus {
                        self.needsAnotherGitStatus = false
                        self.refreshGitStatus()
                    }
                }
            }
        }
        gitRefreshDebounce = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1, execute: work)
    }

    /// Collapse all expanded directories back to root-level only.
    func collapseAll() {
        for node in rootNodes {
            collapseRecursively(node)
        }
        stopAllSubdirWatchers()
        saveExpandedPaths()
    }

    private func collapseRecursively(_ node: FileTreeNode) {
        if node.isDirectory {
            if let children = node.children {
                for child in children {
                    collapseRecursively(child)
                }
            }
            node.isExpanded = false
        }
    }

    /// Persist expansion state after SwiftUI toggles directory rows, and
    /// resync the subdirectory watchers to the new expansion set.
    func persistCurrentExpandedPaths() {
        saveExpandedPaths()
        stopAllSubdirWatchers()
        watchExpandedSubdirectories(rootNodes)
    }

    /// Rebuild the tree from disk, preserving expansion state. Heavy work
    /// (filesystem scan + git status) runs on a background queue.
    /// Coalesces overlapping requests: if called while a rebuild is already
    /// in progress, the current rebuild finishes and then a fresh one starts.
    func refreshTree() {
        guard !rootPath.isEmpty else { return }

        if isRefreshingTree {
            needsAnotherRefresh = true
            return
        }
        isRefreshingTree = true

        // Collect expanded paths before rebuilding.
        let expandedPaths = collectExpandedPaths(rootNodes)
        let root = rootPath
        let hidden = showHidden

        DispatchQueue.global(qos: .utility).async {
            let newNodes = FileTreeNode.buildTree(rootPath: root, showHidden: hidden)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.isRefreshingTree = false

                // If more events arrived during the rebuild, start a fresh
                // refresh with the latest filesystem state instead of applying
                // the now-stale result.
                if self.needsAnotherRefresh {
                    self.needsAnotherRefresh = false
                    self.refreshTree()
                    return
                }

                self.rootNodes = newNodes
                self.applyExpansionState(expandedPaths, to: self.rootNodes)
                self.rebuildNodeIndex()
                self.watchExpandedSubdirectories(self.rootNodes)
                self.onTreeRefreshed?(self.rootNodes)
                NotificationCenter.default.post(name: .impulseFileTreeChanged, object: nil)
                // Single git status refresh after expansion restoration ensures
                // all expanded children are covered by the batch API call.
                self.refreshGitStatus()
            }
        }
    }

    // MARK: Expansion State

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

    /// Re-apply a set of expanded paths to a (freshly built) tree, loading
    /// children for each expanded directory. The retired NSOutlineView did
    /// this implicitly through `expandItem` + the expand delegate; the
    /// headless tree loads children synchronously here, exactly like the old
    /// bulk-restore path did on the main thread.
    private func applyExpansionState(_ paths: Set<String>, to nodes: [FileTreeNode]) {
        for node in nodes where node.isDirectory && paths.contains(node.path) {
            if !node.isLoaded {
                node.loadChildren(showHidden: showHidden)
            }
            node.isExpanded = true
            if let children = node.children {
                applyExpansionState(paths, to: children)
            }
        }
    }

    // MARK: Expansion Persistence

    private static let expandedPathsKeyPrefix = "impulse.fileTree.expandedPaths"

    /// Per-root UserDefaults key so switching projects doesn't clobber
    /// expansion state.
    private func expandedPathsKey(forRoot root: String) -> String {
        "\(Self.expandedPathsKeyPrefix).\(root)"
    }

    /// Save the current set of expanded paths to UserDefaults.
    private func saveExpandedPaths() {
        let paths = collectExpandedPaths(rootNodes)
        UserDefaults.standard.set(Array(paths), forKey: expandedPathsKey(forRoot: rootPath))
    }

    /// Load the saved set of expanded paths from UserDefaults.
    private func loadExpandedPaths(forRoot root: String) -> Set<String> {
        let paths = UserDefaults.standard.stringArray(forKey: expandedPathsKey(forRoot: root)) ?? []
        return Set(paths)
    }

    // MARK: File System Watching

    /// Start watching the root directory for filesystem changes.
    private func startWatching(path: String) {
        stopWatching()

        let fd = open(path, O_EVTONLY)
        guard fd >= 0 else {
            NSLog("FileTreeDataController: failed to open \(path) for watching (errno \(errno))")
            return
        }
        watchedFileDescriptor = fd

        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: fd,
            eventMask: [.write, .rename, .delete, .link],
            queue: .main
        )

        source.setEventHandler { [weak self] in
            self?.handleFileSystemEvent(path: path)
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
            self?.handleFileSystemEvent(path: path)
        }
        source.setCancelHandler { [fd] in
            close(fd)
        }
        subdirWatchers[path] = (fd: fd, source: source)
        source.resume()
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
        pendingFileTreeEvents.removeAll()
        gitRefreshDebounce?.cancel()
        gitRefreshDebounce = nil

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
        // thread (waitUntilExit on the main thread pumps the run loop).
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
            let gitRoot = String(
                data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
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

    /// Start a repeating timer that polls git status periodically.
    /// Catches file-content edits that directory watchers can't see.
    private func startGitStatusTimer() {
        stopGitStatusTimer()
        guard !rootPath.isEmpty else { return }

        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now() + 10, repeating: 10, leeway: .seconds(2))
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
        // Skip if a git status call is already in progress (from refreshGitStatus
        // or a previous poll). The next timer tick will pick up the change.
        guard !isGitStatusInProgress else { return }
        isGitStatusInProgress = true
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

            guard hash != previousHash else {
                DispatchQueue.main.async { [weak self] in
                    self?.isGitStatusInProgress = false
                }
                return
            }

            // Hash changed — apply the already-fetched statuses directly
            // (avoids a redundant getAllGitStatuses FFI call).
            FileTreeNode.applyGitStatuses(nodes: nodes, dirPath: root, batchStatuses: batchStatuses)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.isGitStatusInProgress = false
                self.lastGitStatusHash = hash
            }
        }
    }

    // MARK: Filesystem Events → Tree Patches

    /// Called when a watched directory dispatch source fires. Debounces rapid
    /// events and applies a patch for the loaded parent directories instead of
    /// rebuilding the full tree.
    private func handleFileSystemEvent(path: String) {
        pendingFileTreeEvents.append(ImpulseCore.FileTreeWatchEvent(kind: "any", paths: [path]))
        debounceWorkItem?.cancel()
        let work = DispatchWorkItem { [weak self] in
            self?.refreshTreePatches()
        }
        debounceWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: work)
    }

    private func refreshTreePatches() {
        guard !rootPath.isEmpty else { return }
        guard !pendingFileTreeEvents.isEmpty else { return }

        if isRefreshingTree {
            needsAnotherRefresh = true
            return
        }
        isRefreshingTree = true

        let root = rootPath
        let hidden = showHidden
        let events = pendingFileTreeEvents
        pendingFileTreeEvents.removeAll()
        let beforeByParent = loadedDirectorySnapshots()

        DispatchQueue.global(qos: .utility).async { [weak self] in
            let batch = ImpulseCore.buildFileTreePatchBatch(
                rootPath: root,
                events: events,
                beforeByParent: beforeByParent,
                showHidden: hidden
            )

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                guard self.rootPath == root else {
                    self.isRefreshingTree = false
                    return
                }

                self.isRefreshingTree = false
                guard let batch else {
                    self.refreshTree()
                    return
                }

                self.applyFileTreePatchBatch(batch)

                if self.needsAnotherRefresh || !self.pendingFileTreeEvents.isEmpty {
                    self.needsAnotherRefresh = false
                    self.refreshTreePatches()
                }
            }
        }
    }

    private func loadedDirectorySnapshots() -> [String: [ImpulseCore.FileEntryFFI]] {
        var snapshots: [String: [ImpulseCore.FileEntryFFI]] = [
            rootPath: rootNodes.map { $0.fileEntrySnapshot() }
        ]

        for node in nodeByPath.values where node.isDirectory {
            if let children = node.children {
                snapshots[node.path] = children.map { $0.fileEntrySnapshot() }
            }
        }

        return snapshots
    }

    private func applyFileTreePatchBatch(_ batch: ImpulseCore.FileTreePatchBatch) {
        guard !batch.patches.isEmpty else { return }

        let expandedPaths = collectExpandedPaths(rootNodes)
        var changed = false

        for patch in batch.patches {
            changed = applyFileTreePatch(patch) || changed
        }
        if !expandedPaths.isEmpty {
            applyExpansionState(expandedPaths, to: rootNodes)
        }

        guard changed else { return }
        rebuildNodeIndex()
        stopAllSubdirWatchers()
        watchExpandedSubdirectories(rootNodes)
        onTreeRefreshed?(rootNodes)
        NotificationCenter.default.post(name: .impulseFileTreeChanged, object: nil)
        refreshGitStatus()
    }

    private func applyFileTreePatch(_ patch: ImpulseCore.FileTreePatch) -> Bool {
        if patch.parent_id == stableNodeID(rootPath) {
            var children = rootNodes
            applyFileTreeOperations(patch.operations, to: &children)
            rootNodes = children
            return true
        }

        guard let parent = nodeByPath[patch.parent_id],
              parent.isDirectory,
              parent.children != nil else {
            return false
        }

        var children = parent.children ?? []
        applyFileTreeOperations(patch.operations, to: &children)
        parent.children = children
        return true
    }

    private func applyFileTreeOperations(
        _ operations: [ImpulseCore.FileTreeOperation],
        to children: inout [FileTreeNode]
    ) {
        let removedIDs = Set(operations.compactMap { operation -> String? in
            if case .remove(let id) = operation { return id }
            return nil
        })

        for operation in operations {
            switch operation {
            case .remove(let id):
                if let index = children.firstIndex(where: { stableNodeID($0.path) == id }) {
                    children.remove(at: index)
                }
            case .upsert(_, let index, let patchNode):
                let existingIndex = children.firstIndex {
                    stableNodeID($0.path) == patchNode.id
                }
                let existing = existingIndex.map { children.remove(at: $0) }
                let node = nodeForUpsert(
                    patchNode,
                    existing: existing,
                    preserveExisting: !removedIDs.contains(patchNode.id)
                )
                children.insert(node, at: min(index, children.count))
            }
        }
    }

    private func nodeForUpsert(
        _ patchNode: ImpulseCore.FileTreePatchNode,
        existing: FileTreeNode?,
        preserveExisting: Bool
    ) -> FileTreeNode {
        if preserveExisting,
           let existing,
           existing.isDirectory == patchNode.is_dir {
            existing.updateMetadata(from: patchNode)
            return existing
        }
        return FileTreeNode.fromPatchNode(patchNode)
    }

    private func stableNodeID(_ path: String) -> String {
        var id = path
        while id.count > 1 && (id.hasSuffix("/") || id.hasSuffix("\\")) {
            id.removeLast()
        }
        return id
    }

    // MARK: Node Index

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
}
