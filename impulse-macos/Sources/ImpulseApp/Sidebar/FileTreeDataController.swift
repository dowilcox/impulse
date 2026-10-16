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

    /// At most this many expanded folders get a watcher of their own (each
    /// holds a file descriptor); past it, an FSEvents stream on the root
    /// covers the rest.
    private static let maxSubdirWatchers = 64
    private var overflowStream: FSEventStreamRef?

    /// Bumped whenever the whole tree is replaced, so a background load
    /// finishing for an older tree doesn't refresh the sidebar.
    private var treeGeneration = 0
    /// Folders whose children are being listed in the background.
    private var loadingNodes = Set<ObjectIdentifier>()

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
        let savedPaths = Self.savedExpandedPaths(forRoot: rootPath)
        let allExpanded = expandedPaths.union(incomingExpanded).union(savedPaths)

        self.rootPath = rootPath
        self.rootNodes = nodes
        treeGeneration += 1

        // Start root watcher first — this stops all previous watchers.
        startWatching(path: rootPath)

        // Expanded folders not loaded yet fill in once listed (off main);
        // that refreshes git status for them too.
        if !allExpanded.isEmpty {
            applyExpansionState(allExpanded, to: rootNodes)
        }

        rebuildNodeIndex()
        syncSubdirectoryWatchers()

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
        syncSubdirectoryWatchers()
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
        syncSubdirectoryWatchers()
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
            Self.loadExpanded(expandedPaths, in: newNodes, showHidden: hidden)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.isRefreshingTree = false
                // Re-rooted while building: this tree is for another folder.
                guard self.rootPath == root else { return }

                // If more events arrived during the rebuild, start a fresh
                // refresh with the latest filesystem state instead of applying
                // the now-stale result.
                if self.needsAnotherRefresh {
                    self.needsAnotherRefresh = false
                    self.refreshTree()
                    return
                }

                self.rootNodes = newNodes
                self.treeGeneration += 1
                self.applyExpansionState(expandedPaths, to: self.rootNodes)
                self.rebuildNodeIndex()
                self.syncSubdirectoryWatchers()
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

    /// Re-apply a set of expanded paths to the tree. Folders whose children
    /// are loaded open right away; the others are listed off the main
    /// thread and filled in when that's done.
    private func applyExpansionState(_ paths: Set<String>, to nodes: [FileTreeNode]) {
        var unloaded: [FileTreeNode] = []
        markExpanded(paths, in: nodes, unloaded: &unloaded)
        loadChildren(of: unloaded, expanding: paths)
    }

    private func markExpanded(_ paths: Set<String>, in nodes: [FileTreeNode], unloaded: inout [FileTreeNode]) {
        for node in nodes where node.isDirectory && paths.contains(node.path) {
            node.isExpanded = true
            if let children = node.children {
                markExpanded(paths, in: children, unloaded: &unloaded)
            } else {
                unloaded.append(node)
            }
        }
    }

    /// List folders in the background (and the expanded folders inside
    /// them), then put their children in place on main.
    private func loadChildren(of nodes: [FileTreeNode], expanding paths: Set<String>) {
        let nodes = nodes.filter { loadingNodes.insert(ObjectIdentifier($0)).inserted }
        guard !nodes.isEmpty else { return }
        let folders = nodes.map(\.path)
        let hidden = showHidden
        let generation = treeGeneration
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            // Fresh nodes: nothing on screen changes until they're in place.
            let loaded = folders.map { path -> [FileTreeNode] in
                let children = FileTreeNode.buildChildren(path: path, showHidden: hidden)
                Self.loadExpanded(paths, in: children, showHidden: hidden)
                return children
            }
            DispatchQueue.main.async { [weak self] in
                var filled = false
                for (node, children) in zip(nodes, loaded) {
                    self?.loadingNodes.remove(ObjectIdentifier(node))
                    // Loaded meanwhile (expanded by hand): keep that.
                    guard node.children == nil else { continue }
                    node.children = children
                    filled = true
                }
                guard let self, filled, self.treeGeneration == generation else { return }
                self.treeDidChange(filesChanged: false)
            }
        }
    }

    /// A root's top-level nodes with the folders open there already listed
    /// (those saved for the root, and `expanding`, the tree on screen), so
    /// `updateTree` shows them filled in rather than open and empty until a
    /// background listing lands. Call it off the main thread.
    static func buildTree(rootPath: String, showHidden: Bool, expanding: Set<String> = []) -> [FileTreeNode] {
        let nodes = FileTreeNode.buildTree(rootPath: rootPath, showHidden: showHidden)
        loadExpanded(expanding.union(savedExpandedPaths(forRoot: rootPath)), in: nodes, showHidden: showHidden)
        return nodes
    }

    /// Open the expanded folders among `nodes`, listing their children (and
    /// theirs). For nodes nothing shows yet: it runs off the main thread.
    private static func loadExpanded(_ paths: Set<String>, in nodes: [FileTreeNode], showHidden: Bool) {
        guard !paths.isEmpty else { return }
        for node in nodes where node.isDirectory && paths.contains(node.path) {
            node.isExpanded = true
            if node.children == nil {
                node.loadChildren(showHidden: showHidden)
            }
            loadExpanded(paths, in: node.children ?? [], showHidden: showHidden)
        }
    }

    /// Children were added or replaced: refresh the lookup, the watchers,
    /// the sidebar and git badges. `filesChanged`: it came from the disk
    /// (a search re-runs), not from listing a folder that was opened.
    private func treeDidChange(filesChanged: Bool) {
        rebuildNodeIndex()
        syncSubdirectoryWatchers()
        onTreeRefreshed?(rootNodes)
        if filesChanged {
            NotificationCenter.default.post(name: .impulseFileTreeChanged, object: nil)
        }
        refreshGitStatus()
    }

    // MARK: Expansion Persistence

    private static let expandedPathsKeyPrefix = "impulse.fileTree.expandedPaths"

    /// Per-root UserDefaults key so switching projects doesn't clobber
    /// expansion state.
    private static func expandedPathsKey(forRoot root: String) -> String {
        "\(expandedPathsKeyPrefix).\(root)"
    }

    /// Save the current set of expanded paths to UserDefaults.
    private func saveExpandedPaths() {
        let paths = collectExpandedPaths(rootNodes)
        UserDefaults.standard.set(Array(paths), forKey: Self.expandedPathsKey(forRoot: rootPath))
    }

    /// Load the saved set of expanded paths from UserDefaults (any thread).
    private static func savedExpandedPaths(forRoot root: String) -> Set<String> {
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

    /// Start watching an expanded subdirectory.
    private func watchSubdirectory(_ path: String) {
        guard subdirWatchers[path] == nil else { return }

        let fd = open(path, O_EVTONLY)
        guard fd >= 0 else { return }

        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: fd,
            eventMask: [.write, .rename, .delete, .link],
            queue: .main
        )
        source.setEventHandler { [weak self, weak source] in
            guard let self else { return }
            // The folder itself went (or moved): this descriptor watches
            // nothing now. The next sync opens one on whatever is there.
            if let source, !source.data.isDisjoint(with: [.delete, .rename]) {
                source.cancel()
                if self.subdirWatchers[path]?.source === source {
                    self.subdirWatchers[path] = nil
                }
            }
            self.handleFileSystemEvent(path: path)
        }
        source.setCancelHandler { [fd] in
            close(fd)
        }
        subdirWatchers[path] = (fd: fd, source: source)
        source.resume()
    }

    /// Watch exactly the expanded folders: new ones get a watcher, collapsed
    /// or removed ones lose theirs, and the rest keep the one they have.
    /// Past the cap, an FSEvents stream on the root covers the others.
    private func syncSubdirectoryWatchers() {
        var wanted: [String] = []
        expandedFolders(rootNodes, into: &wanted)
        let wantedSet = Set(wanted)
        for (path, entry) in subdirWatchers where !wantedSet.contains(path) {
            entry.source.cancel()
            subdirWatchers[path] = nil
        }
        var overflow = false
        for path in wanted where subdirWatchers[path] == nil {
            guard subdirWatchers.count < Self.maxSubdirWatchers else {
                overflow = true
                break
            }
            watchSubdirectory(path)
        }
        if overflow {
            startOverflowStream()
        } else {
            stopOverflowStream()
        }
    }

    private func expandedFolders(_ nodes: [FileTreeNode], into paths: inout [String]) {
        for node in nodes where node.isDirectory && node.isExpanded {
            paths.append(node.path)
            if let children = node.children {
                expandedFolders(children, into: &paths)
            }
        }
    }

    /// Stop all subdirectory watchers.
    private func stopAllSubdirWatchers() {
        for (_, entry) in subdirWatchers {
            entry.source.cancel()
        }
        subdirWatchers.removeAll()
        stopOverflowStream()
    }

    // MARK: Overflow Stream

    /// Holds the controller weakly for the FSEvents callback.
    private final class StreamTarget {
        weak var controller: FileTreeDataController?
        init(_ controller: FileTreeDataController) { self.controller = controller }
    }

    /// Folder-level FSEvents for the whole root, for the expanded folders
    /// beyond the watcher cap.
    private func startOverflowStream() {
        guard overflowStream == nil, !rootPath.isEmpty else { return }
        var context = FSEventStreamContext(
            version: 0, info: Unmanaged.passRetained(StreamTarget(self)).toOpaque(),
            retain: nil, release: { info in
                guard let info else { return }
                Unmanaged<StreamTarget>.fromOpaque(info).release()
            }, copyDescription: nil)
        let callback: FSEventStreamCallback = { _, info, _, eventPaths, _, _ in
            guard let info,
                  let controller = Unmanaged<StreamTarget>.fromOpaque(info).takeUnretainedValue().controller
            else { return }
            let paths = unsafeBitCast(eventPaths, to: NSArray.self).compactMap { $0 as? String }
            controller.handleOverflowEvents(paths)
        }
        guard
            let stream = FSEventStreamCreate(
                nil, callback, &context, [rootPath] as CFArray,
                FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.3,
                UInt32(kFSEventStreamCreateFlagUseCFTypes))
        else { return }
        FSEventStreamSetDispatchQueue(stream, .main)
        FSEventStreamStart(stream)
        overflowStream = stream
    }

    private func stopOverflowStream() {
        guard let stream = overflowStream else { return }
        FSEventStreamStop(stream)
        FSEventStreamInvalidate(stream)
        FSEventStreamRelease(stream)
        overflowStream = nil
    }

    /// Changed folders reported by the overflow stream: refresh the loaded
    /// ones that have no watcher of their own.
    private func handleOverflowEvents(_ paths: [String]) {
        // FSEvents reports real paths (symlinks resolved, /private/var).
        let realRoot = TextFile.target(of: rootPath)
        for reported in paths {
            var path = reported
            while path.count > 1, path.hasSuffix("/") { path.removeLast() }
            guard path.hasPrefix(realRoot + "/") else { continue }
            let treePath = rootPath + path.dropFirst(realRoot.count)
            guard subdirWatchers[treePath] == nil, nodeByPath[treePath]?.isLoaded == true else { continue }
            handleFileSystemEvent(path: treePath)
        }
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

        guard changed else {
            // A folder whose watcher went with it may be back.
            syncSubdirectoryWatchers()
            return
        }
        treeDidChange(filesChanged: true)
    }

    private func applyFileTreePatch(_ patch: ImpulseCore.FileTreePatch) -> Bool {
        if patch.parent_id == Self.stableNodeID(rootPath) {
            rootNodes = Self.applying(patch.operations, to: rootNodes)
            return true
        }

        guard let parent = nodeByPath[patch.parent_id],
              parent.isDirectory,
              let children = parent.children else {
            return false
        }

        parent.children = Self.applying(patch.operations, to: children)
        return true
    }

    /// The children after a patch's operations: removed ones go, upserted
    /// ones land at their indices (in ascending order), and the rest keep
    /// their order. One pass with lookups by id, so a big folder costs the
    /// same however many entries the patch names.
    static func applying(
        _ operations: [ImpulseCore.FileTreeOperation],
        to children: [FileTreeNode]
    ) -> [FileTreeNode] {
        var removedIDs = Set<String>()
        var upserts: [(index: Int, node: ImpulseCore.FileTreePatchNode)] = []
        for operation in operations {
            switch operation {
            case .remove(let id):
                removedIDs.insert(id)
            case .upsert(_, let index, let node):
                upserts.append((index, node))
            }
        }
        let upsertedIDs = Set(upserts.map(\.node.id))
        var existing: [String: FileTreeNode] = [:]
        var kept: [FileTreeNode] = []
        kept.reserveCapacity(children.count)
        for child in children {
            let id = stableNodeID(child.path)
            if removedIDs.contains(id) { continue }
            if upsertedIDs.contains(id) {
                existing[id] = child
            } else {
                kept.append(child)
            }
        }

        upserts.sort { $0.index < $1.index }
        var result: [FileTreeNode] = []
        result.reserveCapacity(kept.count + upserts.count)
        var next = 0
        for (index, patchNode) in upserts {
            // Kept children fill the places before this one.
            while result.count < index, next < kept.count {
                result.append(kept[next])
                next += 1
            }
            // A removed-and-upserted id was replaced: a new node.
            result.append(nodeForUpsert(
                patchNode,
                existing: existing[patchNode.id],
                preserveExisting: !removedIDs.contains(patchNode.id)
            ))
        }
        result.append(contentsOf: kept[next...])
        return result
    }

    private static func nodeForUpsert(
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

    private static func stableNodeID(_ path: String) -> String {
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
