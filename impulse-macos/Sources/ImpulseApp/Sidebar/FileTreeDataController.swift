import AppKit
import ImpulseGit

/// Headless owner of the sidebar file tree data.
///
/// Holds the root nodes, filesystem watchers (root + expanded subdirectories),
/// git-status badge refreshes, incremental patch application, and
/// expansion-state persistence. The SwiftUI sidebar
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

    // Git badges are refreshed by the window when its repository's
    // RepoWatcher reports changes (see GitRepositoryState), not by polling.

    // Debounce work item for git status refresh.
    private var gitRefreshDebounce: DispatchWorkItem?

    // Guards against overlapping tree rebuilds. If a refresh is requested while
    // one is already in progress, we set needsAnotherRefresh and re-trigger
    // when the current rebuild completes.
    private var isRefreshingTree = false
    private var needsAnotherRefresh = false
    private var pendingFileTreeEvents: [ImpulseCore.FileTreeWatchEvent] = []

    // Guard against overlapping git status refreshes.
    private var isGitStatusInProgress = false
    private var needsAnotherGitStatus = false

    // MARK: Initialisation

    init() {}

    deinit {
        stopWatching()
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
