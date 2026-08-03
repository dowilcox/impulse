import Foundation

// Ported from impulse-core/src/filesystem.rs (listing portion; git-status
// enrichment lives in ImpulseGit and is composed by the caller).

public struct FileEntry: Codable, Equatable {
  public var name: String
  public var path: String
  public var isDir: Bool
  public var isSymlink: Bool
  public var size: UInt64
  public var modified: UInt64
  public var gitStatus: String?

  public init(
    name: String, path: String, isDir: Bool, isSymlink: Bool = false,
    size: UInt64 = 0, modified: UInt64 = 0, gitStatus: String? = nil
  ) {
    self.name = name
    self.path = path
    self.isDir = isDir
    self.isSymlink = isSymlink
    self.size = size
    self.modified = modified
    self.gitStatus = gitStatus
  }

  enum CodingKeys: String, CodingKey {
    case name
    case path
    case isDir = "is_dir"
    case isSymlink = "is_symlink"
    case size
    case modified
    case gitStatus = "git_status"
  }
}

public enum DirectoryLister {
  /// Read directory contents, sorted directories-first then alphabetical
  /// (case-insensitive, byte-order like the Rust implementation). Hidden
  /// files are filtered unless `showHidden`; OS metadata files are always
  /// filtered.
  public static func readDirectoryEntries(path: String, showHidden: Bool) throws -> [FileEntry] {
    var isDirectory: ObjCBool = false
    guard FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory),
      isDirectory.boolValue
    else {
      throw ImpulseKitError("Not a directory: \(path)")
    }

    let names: [String]
    do {
      names = try FileManager.default.contentsOfDirectory(atPath: path)
    } catch {
      throw ImpulseKitError("Failed to read directory: \(error.localizedDescription)")
    }

    var entries: [FileEntry] = []
    for name in names {
      if !showHidden && name.hasPrefix(".") { continue }
      // Always hide OS metadata files that are never useful in a file tree.
      if name == ".DS_Store" || name == "Thumbs.db" || name == "desktop.ini" { continue }

      let entryPath = (path as NSString).appendingPathComponent(name)
      // lstat-equivalent for the symlink bit, stat-equivalent for the rest
      // (mirrors Rust's DirEntry::metadata which follows nothing extra).
      guard
        let lstatAttrs = try? FileManager.default.attributesOfItem(atPath: entryPath)
      else { continue }
      let isSymlink = (lstatAttrs[.type] as? FileAttributeType) == .typeSymbolicLink

      let isDir: Bool
      let size: UInt64
      let modified: UInt64
      if isSymlink {
        // Rust's entry.metadata() does not follow symlinks for DirEntry —
        // it reports the link itself.
        isDir = false
        size = (lstatAttrs[.size] as? UInt64) ?? 0
        modified = Self.mtime(from: lstatAttrs)
      } else {
        isDir = (lstatAttrs[.type] as? FileAttributeType) == .typeDirectory
        size = (lstatAttrs[.size] as? UInt64) ?? 0
        modified = Self.mtime(from: lstatAttrs)
      }

      entries.append(
        FileEntry(
          name: name,
          path: entryPath,
          isDir: isDir,
          isSymlink: isSymlink,
          size: size,
          modified: modified
        ))
    }

    entries.sort { a, b in
      if a.isDir != b.isDir { return a.isDir }
      let la = Array(a.name.lowercased().utf8)
      let lb = Array(b.name.lowercased().utf8)
      return la.lexicographicallyPrecedes(lb)
    }
    return entries
  }

  private static func mtime(from attrs: [FileAttributeKey: Any]) -> UInt64 {
    guard let date = attrs[.modificationDate] as? Date else { return 0 }
    let secs = date.timeIntervalSince1970
    return secs > 0 ? UInt64(secs) : 0
  }
}

public struct ImpulseKitError: Error, CustomStringConvertible {
  public let message: String
  public init(_ message: String) { self.message = message }
  public var description: String { message }
}
