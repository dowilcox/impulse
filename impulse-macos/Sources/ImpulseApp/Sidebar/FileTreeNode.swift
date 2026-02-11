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
        guard !statusMap.isEmpty else { return }
        Self.applyGitStatus(statusMap, to: nodes, basePath: rootPath)
    }

    // MARK: Private Helpers

    /// Parse `git status --porcelain` output into a dictionary mapping absolute
    /// file paths to their `GitStatus` value.
    private static func fetchGitStatus(rootPath: String) -> [String: GitStatus] {
        // Find the git repository root.
        let gitRootResult = Self.shell("git", arguments: ["rev-parse", "--show-toplevel"],
                                       currentDirectory: rootPath)
        guard let gitRoot = gitRootResult else { return [:] }

        let statusResult = Self.shell("git", arguments: ["status", "--porcelain", "-uall"],
                                      currentDirectory: rootPath)
        guard let output = statusResult, !output.isEmpty else { return [:] }

        var map: [String: GitStatus] = [:]

        for line in output.components(separatedBy: "\n") {
            guard line.count >= 4 else { continue }

            let statusChars = String(line.prefix(2)).trimmingCharacters(in: .whitespaces)
            let relativePath = String(line.dropFirst(3)).trimmingCharacters(in: .whitespaces)

            // Handle renames: "R  old -> new"
            let effectivePath: String
            if let arrowRange = relativePath.range(of: " -> ") {
                effectivePath = String(relativePath[arrowRange.upperBound...])
            } else {
                effectivePath = relativePath
            }

            let absPath = (gitRoot as NSString).appendingPathComponent(effectivePath)

            let status: GitStatus
            switch statusChars {
            case "M", "MM", "AM":
                // "AM" means added then modified in working tree; show as modified
                if statusChars == "AM" {
                    status = .added
                } else {
                    status = .modified
                }
            case "A":
                status = .added
            case "D", "DD":
                status = .deleted
            case "R":
                status = .renamed
            case "??":
                status = .untracked
            case "UU", "AA":
                status = .conflict
            default:
                // Best-effort: any other index/working-tree change is treated as modified.
                status = .modified
            }

            map[absPath] = status
        }

        return map
    }

    /// Walk the node tree and apply status from the map. Directories inherit the
    /// "highest priority" status of their children when applicable.
    private static func applyGitStatus(_ map: [String: GitStatus],
                                       to nodes: [FileTreeNode],
                                       basePath: String) {
        for node in nodes {
            if let directStatus = map[node.path] {
                node.gitStatus = directStatus
            } else if node.isDirectory {
                // Check if any file under this directory has a status.
                let prefix = node.path.hasSuffix("/") ? node.path : node.path + "/"
                let hasChild = map.keys.contains(where: { $0.hasPrefix(prefix) })
                if hasChild {
                    node.gitStatus = .modified
                } else {
                    node.gitStatus = .none
                }
            } else {
                node.gitStatus = .none
            }

            if let children = node.children {
                applyGitStatus(map, to: children, basePath: basePath)
            }
        }
    }

    /// Run a command synchronously and return trimmed stdout, or nil on failure.
    private static func shell(_ command: String,
                              arguments: [String],
                              currentDirectory: String) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = [command] + arguments
        process.currentDirectoryURL = URL(fileURLWithPath: currentDirectory)

        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = Pipe()

        do {
            try process.run()
            process.waitUntilExit()
        } catch {
            return nil
        }

        guard process.terminationStatus == 0 else { return nil }

        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        return String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
