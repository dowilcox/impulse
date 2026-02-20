import Foundation

// MARK: - File Tree Node

/// Model node for the sidebar file tree. Each node represents a file or directory
/// at a given path. Directory children are lazily loaded on first expansion.
final class FileTreeNode {

    let name: String
    let path: String
    let isDirectory: Bool

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

            // Skip hidden files when not requested (belt-and-suspenders; the option
            // above should already handle this).
            if !showHidden && itemName.hasPrefix(".") {
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

    /// Run `git status --porcelain` relative to the repository root that contains
    /// this tree and propagate status markers down to individual nodes.
    ///
    /// Call on the root-level array via the static variant below.
    static func refreshGitStatus(nodes: [FileTreeNode], rootPath: String) {
        let statusMap = Self.fetchGitStatus(rootPath: rootPath)
        // Always apply — an empty map correctly clears all statuses to .none.
        Self.applyGitStatus(statusMap, to: nodes, basePath: rootPath)
    }

    // MARK: Private Helpers

    /// Fetch git status for files in the directory at `rootPath` using the
    /// impulse-core FFI bridge (libgit2), avoiding the overhead and parsing
    /// fragility of shelling out to `git status --porcelain`.
    private static func fetchGitStatus(rootPath: String) -> [String: GitStatus] {
        let raw = ImpulseCore.gitStatusForDirectory(path: rootPath)
        guard !raw.isEmpty else { return [:] }

        var map: [String: GitStatus] = [:]
        for (name, code) in raw {
            let status: GitStatus
            switch code {
            case "M":  status = .modified
            case "A":  status = .added
            case "D":  status = .deleted
            case "R":  status = .renamed
            case "C":  status = .conflict
            case "?":  status = .untracked
            default:   status = .modified
            }
            // The FFI returns filenames (not full paths), so reconstruct the absolute path.
            let absPath = (rootPath as NSString).appendingPathComponent(name)
            map[absPath] = status
        }
        return map
    }

    /// Walk the node tree and apply status from the map. Directories inherit the
    /// "highest priority" status of their children when applicable.
    private static func applyGitStatus(_ map: [String: GitStatus],
                                       to nodes: [FileTreeNode],
                                       basePath: String) {
        // Pre-compute directory prefixes that contain changed files for O(1) lookup.
        var directoriesWithChanges = Set<String>()
        for key in map.keys {
            var path = key
            while let range = path.range(of: "/", options: .backwards) {
                path = String(path[..<range.lowerBound])
                let prefix = path + "/"
                if directoriesWithChanges.contains(prefix) { break }
                directoriesWithChanges.insert(prefix)
            }
        }
        applyGitStatusRecursive(map, to: nodes, directoriesWithChanges: directoriesWithChanges)
    }

    private static func applyGitStatusRecursive(_ map: [String: GitStatus],
                                                to nodes: [FileTreeNode],
                                                directoriesWithChanges: Set<String>) {
        for node in nodes {
            if let directStatus = map[node.path] {
                node.gitStatus = directStatus
            } else if node.isDirectory {
                // O(1) set lookup instead of O(M) linear scan.
                let prefix = node.path.hasSuffix("/") ? node.path : node.path + "/"
                if directoriesWithChanges.contains(prefix) {
                    node.gitStatus = .modified
                } else {
                    node.gitStatus = .none
                }
            } else {
                node.gitStatus = .none
            }

            if let children = node.children {
                applyGitStatusRecursive(map, to: children, directoriesWithChanges: directoriesWithChanges)
            }
        }
    }

}
