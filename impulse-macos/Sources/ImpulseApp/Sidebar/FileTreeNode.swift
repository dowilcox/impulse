import Foundation
import Observation

// MARK: - File Tree Node

/// Model node for the sidebar file tree. Each node represents a file or directory
/// at a given path. Directory children are lazily loaded on first expansion.
/// @Observable so SwiftUI re-renders when children load, expand state changes,
/// or git status updates.
@Observable
final class FileTreeNode: Identifiable {

    let name: String
    let path: String
    let isDirectory: Bool

    /// Stable identity for SwiftUI (uses the full path).
    var id: String { path }

    /// `nil` means not yet loaded. An empty array means loaded but the directory is empty.
    var children: [FileTreeNode]?
    var isExpanded: Bool = false
    var gitStatus: GitStatus = .none

    var isLoaded: Bool { children != nil }

    // MARK: Git Status

    enum GitStatus: String {
        case none
        case modified
        case added
        case untracked
        case deleted
        case renamed
        case conflict
        case ignored
    }

    // MARK: Initialisation

    init(name: String, path: String, isDirectory: Bool) {
        self.name = name
        self.path = path
        self.isDirectory = isDirectory
    }

    // MARK: Loading Children

    /// Populate `children` by reading the directory at `path` with FileManager.
    /// Directories are sorted before files; both groups are sorted alphabetically
    /// (case-insensitive).
    func loadChildren(showHidden: Bool) {
        guard isDirectory else { return }

        let fm = FileManager.default
        let url = URL(fileURLWithPath: path, isDirectory: true)

        guard let contents = try? fm.contentsOfDirectory(
            at: url,
            includingPropertiesForKeys: [.isDirectoryKey, .isSymbolicLinkKey],
            options: showHidden ? [] : [.skipsHiddenFiles]
        ) else {
            children = []
            return
        }

        var nodes: [FileTreeNode] = []
        for itemURL in contents {
            let itemName = itemURL.lastPathComponent

            // Skip hidden files when not requested.
            if !showHidden && itemName.hasPrefix(".") {
                continue
            }

            // Always skip .DS_Store — macOS metadata, never useful in a file tree.
            if itemName == ".DS_Store" {
                continue
            }

            let isDir = (try? itemURL.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory ?? false
            nodes.append(FileTreeNode(name: itemName, path: itemURL.path, isDirectory: isDir))
        }

        nodes.sort { lhs, rhs in
            if lhs.isDirectory != rhs.isDirectory {
                return lhs.isDirectory
            }
            return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
        }

        children = nodes
    }

    /// Build children for a directory path without mutating any node.
    /// Intended for background-thread use: call this off main thread, then
    /// assign the result to `node.children` on the main thread.
    static func buildChildren(path: String, showHidden: Bool) -> [FileTreeNode] {
        let fm = FileManager.default
        let url = URL(fileURLWithPath: path, isDirectory: true)

        guard let contents = try? fm.contentsOfDirectory(
            at: url,
            includingPropertiesForKeys: [.isDirectoryKey, .isSymbolicLinkKey],
            options: showHidden ? [] : [.skipsHiddenFiles]
        ) else {
            return []
        }

        var nodes: [FileTreeNode] = []
        for itemURL in contents {
            let itemName = itemURL.lastPathComponent
            if !showHidden && itemName.hasPrefix(".") { continue }
            if itemName == ".DS_Store" { continue }
            let isDir = (try? itemURL.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory ?? false
            nodes.append(FileTreeNode(name: itemName, path: itemURL.path, isDirectory: isDir))
        }

        nodes.sort { lhs, rhs in
            if lhs.isDirectory != rhs.isDirectory { return lhs.isDirectory }
            return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
        }

        return nodes
    }

    // MARK: Building a Top-Level Tree

    /// Create the top-level nodes for the given root directory.
    static func buildTree(rootPath: String, showHidden: Bool) -> [FileTreeNode] {
        let root = FileTreeNode(name: URL(fileURLWithPath: rootPath).lastPathComponent,
                                path: rootPath,
                                isDirectory: true)
        root.loadChildren(showHidden: showHidden)
        return root.children ?? []
    }

    // MARK: Git Status Enrichment

    /// Fetch git status for nodes under `dirPath` and propagate status markers.
    /// Uses the batch `getAllGitStatuses` FFI call to fetch all statuses in a
    /// single pass (instead of N+1 per-directory calls).
    ///
    /// - Parameters:
    ///   - nodes: The tree nodes to update.
    ///   - repoPath: The git repository root (used to fetch all statuses).
    ///   - dirPath: The directory that `nodes` are children of (used for status lookup).
    static func refreshGitStatus(
        nodes: [FileTreeNode],
        repoPath: String,
        dirPath: String
    ) {
        // The batch call runs on the current thread (expected to be background).
        let batchStatuses = ImpulseCore.getAllGitStatuses(repoPath: repoPath)
        applyGitStatuses(nodes: nodes, dirPath: dirPath, batchStatuses: batchStatuses)
    }

    /// Apply pre-fetched batch git statuses to nodes. Use this when the caller
    /// already has the batch statuses (e.g. from a poll that computed a hash)
    /// to avoid a redundant FFI call.
    static func applyGitStatuses(
        nodes: [FileTreeNode],
        dirPath: String,
        batchStatuses: [String: [String: String]]
    ) {
        let updates = collectStatusUpdates(
            nodes: nodes,
            dirPath: dirPath,
            batchStatuses: batchStatuses
        )
        DispatchQueue.main.async {
            for (node, status) in updates {
                node.gitStatus = status
            }
        }
    }

    // MARK: Private Helpers

    /// Collect (node, status) pairs using the pre-computed batch status map.
    /// Recurses into all directories with loaded children — this is cheap since
    /// it only does dictionary lookups, no I/O.
    private static func collectStatusUpdates(
        nodes: [FileTreeNode],
        dirPath: String,
        batchStatuses: [String: [String: String]]
    ) -> [(FileTreeNode, GitStatus)] {
        var updates: [(FileTreeNode, GitStatus)] = []
        let dirStatuses = batchStatuses[dirPath] ?? [:]

        for node in nodes {
            let status = statusFromCode(dirStatuses[node.name])
            updates.append((node, status))
        }

        // Recurse into all directories with loaded children.
        for node in nodes {
            if node.isDirectory, let children = node.children {
                updates.append(contentsOf: collectStatusUpdates(
                    nodes: children,
                    dirPath: node.path,
                    batchStatuses: batchStatuses
                ))
            }
        }
        return updates
    }

    /// Convert a status code string to a GitStatus enum value.
    private static func statusFromCode(_ code: String?) -> GitStatus {
        guard let code else { return .none }
        switch code {
        case "M":  return .modified
        case "A":  return .added
        case "D":  return .deleted
        case "R":  return .renamed
        case "C":  return .conflict
        case "?":  return .untracked
        case "I":  return .ignored
        default:   return .modified
        }
    }

}

// MARK: - Flat Tree Entry

/// A single entry in the flattened file tree. Used by FileTreeListView for
/// efficient virtualized rendering via LazyVStack (instead of recursive views).
struct FlatTreeEntry: Identifiable {
    let id: String
    let node: FileTreeNode
    let depth: Int
}

extension FileTreeNode {
    /// Walk the tree and produce a flat array of all visible (expanded) nodes
    /// with their depth levels. O(n) where n = total visible nodes.
    static func flatten(_ nodes: [FileTreeNode], depth: Int = 0) -> [FlatTreeEntry] {
        var result: [FlatTreeEntry] = []
        result.reserveCapacity(nodes.count * 2)
        flattenInto(&result, nodes: nodes, depth: depth)
        return result
    }

    private static func flattenInto(
        _ result: inout [FlatTreeEntry],
        nodes: [FileTreeNode],
        depth: Int
    ) {
        for node in nodes {
            result.append(FlatTreeEntry(id: node.path, node: node, depth: depth))
            if node.isDirectory && node.isExpanded, let children = node.children {
                flattenInto(&result, nodes: children, depth: depth + 1)
            }
        }
    }
}
