import Foundation

/// Editor files on disk, read and written without losing anything:
/// - strictly UTF-8: a file in another encoding isn't opened, rather than
///   opening empty and being overwritten on save;
/// - a UTF-8 byte-order mark is remembered and written back;
/// - writes go through a symlink to its target instead of replacing the
///   link (and keep the file's permissions).
enum TextFile {
  struct Contents: Equatable {
    let text: String
    let bom: Bool
  }

  /// Identity of what's on disk; changes whenever anything rewrites it.
  struct Stamp: Equatable {
    let modified: Date?
    let size: Int?
    let inode: Int?
  }

  static let utf8BOM: [UInt8] = [0xEF, 0xBB, 0xBF]

  /// The file's text, or nil when it's unreadable or not valid UTF-8.
  static func read(_ path: String) -> Contents? {
    guard let data = FileManager.default.contents(atPath: path) else { return nil }
    return decode(data)
  }

  static func decode(_ data: Data) -> Contents? {
    let bom = data.starts(with: utf8BOM)
    guard let text = String(data: bom ? data.dropFirst(utf8BOM.count) : data, encoding: .utf8) else {
      return nil
    }
    return Contents(text: text, bom: bom)
  }

  static func write(_ text: String, bom: Bool, to path: String) throws {
    var data = Data(capacity: text.utf8.count + (bom ? utf8BOM.count : 0))
    if bom { data.append(contentsOf: utf8BOM) }
    data.append(contentsOf: text.utf8)
    try data.write(to: URL(fileURLWithPath: target(of: path)), options: .atomic)
  }

  /// Where a write to `path` should land: the real file behind any symlinks.
  static func target(of path: String) -> String {
    guard let resolved = realpath(path, nil) else { return path }
    defer { free(resolved) }
    return String(cString: resolved)
  }

  static func stamp(_ path: String) -> Stamp? {
    guard let attributes = try? FileManager.default.attributesOfItem(atPath: target(of: path)) else {
      return nil
    }
    return Stamp(
      modified: attributes[.modificationDate] as? Date,
      size: (attributes[.size] as? NSNumber)?.intValue,
      inode: (attributes[.systemFileNumber] as? NSNumber)?.intValue)
  }
}
