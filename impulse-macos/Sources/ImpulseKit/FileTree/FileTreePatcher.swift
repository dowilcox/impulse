import Foundation

// Ported from impulse-core/src/file_tree.rs: stable node ids, incremental
// child-list diffing (remove/upsert operations), and patch-batch building
// from filesystem watcher events. The directory reader is injected so the
// caller can compose git-status enrichment (ImpulseGit) with the plain
// lister without a dependency cycle.

public struct FileTreeNode: Codable, Equatable {
  public var id: String
  public var parentId: String?
  public var name: String
  public var path: String
  public var isDir: Bool
  public var isSymlink: Bool
  public var size: UInt64
  public var modified: UInt64
  public var gitStatus: String?

  enum CodingKeys: String, CodingKey {
    case id
    case parentId = "parent_id"
    case name
    case path
    case isDir = "is_dir"
    case isSymlink = "is_symlink"
    case size
    case modified
    case gitStatus = "git_status"
  }
}

public struct FileTreePatch: Codable, Equatable {
  public var parentId: String
  public var operations: [FileTreeOperation]

  enum CodingKeys: String, CodingKey {
    case parentId = "parent_id"
    case operations
  }
}

public struct FileTreePatchBatch: Codable, Equatable {
  public var rootId: String
  public var patches: [FileTreePatch]

  enum CodingKeys: String, CodingKey {
    case rootId = "root_id"
    case patches
  }
}

/// Mirrors the Rust serde tagged enum: `{"type": "remove"|"upsert", ...}`.
public enum FileTreeOperation: Codable, Equatable {
  case remove(id: String)
  case upsert(parentId: String, index: Int, node: FileTreeNode)

  enum CodingKeys: String, CodingKey {
    case type
    case id
    case parentId = "parent_id"
    case index
    case node
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    switch try container.decode(String.self, forKey: .type) {
    case "remove":
      self = .remove(id: try container.decode(String.self, forKey: .id))
    case "upsert":
      self = .upsert(
        parentId: try container.decode(String.self, forKey: .parentId),
        index: try container.decode(Int.self, forKey: .index),
        node: try container.decode(FileTreeNode.self, forKey: .node))
    case let other:
      throw DecodingError.dataCorruptedError(
        forKey: .type, in: container, debugDescription: "unknown operation type \(other)")
    }
  }

  public func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    switch self {
    case .remove(let id):
      try container.encode("remove", forKey: .type)
      try container.encode(id, forKey: .id)
    case .upsert(let parentId, let index, let node):
      try container.encode("upsert", forKey: .type)
      try container.encode(parentId, forKey: .parentId)
      try container.encode(index, forKey: .index)
      try container.encode(node, forKey: .node)
    }
  }
}

public enum FileTreeWatchEventKind: String, Codable {
  case create
  case modify
  case remove
  case rename
  case any
}

public struct FileTreeWatchEvent: Codable, Equatable {
  public var kind: FileTreeWatchEventKind
  public var paths: [String]

  public init(kind: FileTreeWatchEventKind, paths: [String]) {
    self.kind = kind
    self.paths = paths
  }
}

public enum FileTreePatcher {
  /// Reads one directory level; `nil` when the directory cannot be read.
  public typealias DirectoryReader = (_ path: String, _ showHidden: Bool) -> [FileEntry]?

  public static func stableNodeId(_ path: String) -> String {
    var trimmed = Substring(path)
    while let last = trimmed.last, last == "/" || last == "\\" {
      trimmed = trimmed.dropLast()
    }
    return trimmed.isEmpty ? path : String(trimmed)
  }

  public static func nodeFromEntry(parentPath: String, entry: FileEntry) -> FileTreeNode {
    FileTreeNode(
      id: stableNodeId(entry.path),
      parentId: stableNodeId(parentPath),
      name: entry.name,
      path: entry.path,
      isDir: entry.isDir,
      isSymlink: entry.isSymlink,
      size: entry.size,
      modified: entry.modified,
      gitStatus: entry.gitStatus
    )
  }

  public static func buildChildPatch(
    parentPath: String, before: [FileEntry], after: [FileEntry]
  ) -> FileTreePatch {
    let parentId = stableNodeId(parentPath)
    let beforeNodes = before.map { nodeFromEntry(parentPath: parentPath, entry: $0) }
    let afterNodes = after.map { nodeFromEntry(parentPath: parentPath, entry: $0) }

    var beforeById: [String: (index: Int, node: FileTreeNode)] = [:]
    for (index, node) in beforeNodes.enumerated() {
      beforeById[node.id] = (index, node)
    }
    var afterById: [String: FileTreeNode] = [:]
    for node in afterNodes {
      afterById[node.id] = node
    }

    var operations: [FileTreeOperation] = []
    var replaceIds = Set<String>()

    for node in beforeNodes {
      if let afterNode = afterById[node.id] {
        if requiresReplacement(before: node, after: afterNode) {
          replaceIds.insert(node.id)
          operations.append(.remove(id: node.id))
        }
      } else {
        operations.append(.remove(id: node.id))
      }
    }

    for (index, node) in afterNodes.enumerated() {
      let shouldUpsert: Bool
      if let (beforeIndex, beforeNode) = beforeById[node.id] {
        shouldUpsert = replaceIds.contains(node.id) || beforeIndex != index || beforeNode != node
      } else {
        shouldUpsert = true
      }
      if shouldUpsert {
        operations.append(.upsert(parentId: parentId, index: index, node: node))
      }
    }

    return FileTreePatch(parentId: parentId, operations: operations)
  }

  public static func buildPatchBatch(
    rootPath: String,
    events: [FileTreeWatchEvent],
    beforeByParent: [String: [FileEntry]],
    afterByParent: [String: [FileEntry]]
  ) -> FileTreePatchBatch {
    let parentPaths = affectedParentPaths(
      rootPath: rootPath, events: events,
      beforeByParent: beforeByParent, afterByParent: afterByParent)
    let patches = parentPaths.compactMap { parentPath -> FileTreePatch? in
      let patch = buildChildPatch(
        parentPath: parentPath,
        before: beforeByParent[parentPath] ?? [],
        after: afterByParent[parentPath] ?? [])
      return patch.operations.isEmpty ? nil : patch
    }
    return FileTreePatchBatch(rootId: stableNodeId(rootPath), patches: patches)
  }

  public static func buildPatchBatchFromFilesystem(
    rootPath: String,
    events: [FileTreeWatchEvent],
    beforeByParent: [String: [FileEntry]],
    showHidden: Bool,
    readDirectory: DirectoryReader = { path, showHidden in
      try? DirectoryLister.readDirectoryEntries(path: path, showHidden: showHidden)
    }
  ) -> FileTreePatchBatch? {
    let parentPaths = affectedParentPaths(
      rootPath: rootPath, events: events, beforeByParent: beforeByParent, afterByParent: [:])
    var afterByParent: [String: [FileEntry]] = [:]

    for parentPath in parentPaths {
      var isDirectory: ObjCBool = false
      if FileManager.default.fileExists(atPath: parentPath, isDirectory: &isDirectory),
        isDirectory.boolValue
      {
        guard let after = readDirectory(parentPath, showHidden) else { return nil }
        afterByParent[parentPath] = after
      } else {
        afterByParent[parentPath] = []
      }
    }

    return buildPatchBatch(
      rootPath: rootPath, events: events,
      beforeByParent: beforeByParent, afterByParent: afterByParent)
  }

  public static func affectedParentPaths(
    rootPath: String,
    events: [FileTreeWatchEvent],
    beforeByParent: [String: [FileEntry]] = [:],
    afterByParent: [String: [FileEntry]] = [:]
  ) -> [String] {
    let root = normalizedPath(rootPath)
    var parents = Set<String>()

    for event in events {
      for rawPath in event.paths {
        let path = normalizedPath(rawPath)
        parents.insert(eventParentPath(root: root, path: path))
        if beforeByParent[path] != nil || afterByParent[path] != nil {
          parents.insert(path)
        }
      }
    }

    return parents.sorted { left, right in
      let dl = pathDepth(left)
      let dr = pathDepth(right)
      if dl != dr { return dl < dr }
      return Array(left.utf8).lexicographicallyPrecedes(Array(right.utf8))
    }
  }

  // MARK: - Helpers (mirroring the Rust path semantics)

  private static func requiresReplacement(before: FileTreeNode, after: FileTreeNode) -> Bool {
    before.isDir != after.isDir || before.isSymlink != after.isSymlink
  }

  private static func eventParentPath(root: String, path: String) -> String {
    if path == root || !pathIsWithinRoot(root: root, path: path) {
      return root
    }
    let parent = (path as NSString).deletingLastPathComponent
    if !parent.isEmpty, pathIsWithinRoot(root: root, path: parent) {
      return parent
    }
    return root
  }

  private static func normalizedPath(_ path: String) -> String {
    stableNodeId(path.isEmpty ? "." : path)
  }

  private static func pathDepth(_ path: String) -> Int {
    let segments = path.split(separator: "/").count
    return path.hasPrefix("/") ? segments + 1 : max(segments, path.isEmpty ? 0 : 1)
  }

  private static func pathIsWithinRoot(root: String, path: String) -> Bool {
    path == root || path.hasPrefix(root.hasSuffix("/") ? root : root + "/")
  }
}
