// Folders copied into a new task as APFS clones (`clonefile(2)`): a whole
// tree in one call, taking no time and no extra disk space until something
// in it changes. Dependency folders (`vendor`), build output (`public/build`)
// and a database's data folder come over this way instead of being rebuilt.

import Darwin
import Foundation

public enum FolderClone {
  /// Clone `relative` from `source` into the same place under
  /// `destination`. A folder the task already has (it holds a tracked
  /// file, say) gets the entries it lacks. Falls back to an ordinary copy
  /// where cloning can't work (another volume). False when `relative`
  /// leaves the folder, is missing in `source`, or nothing could be copied.
  @discardableResult
  public static func clone(_ relative: String, from source: String, to destination: String) -> Bool {
    guard isInside(relative) else { return false }
    let from = (source as NSString).appendingPathComponent(relative)
    let to = (destination as NSString).appendingPathComponent(relative)
    var isDirectory: ObjCBool = false
    guard FileManager.default.fileExists(atPath: from, isDirectory: &isDirectory) else { return false }
    try? FileManager.default.createDirectory(
      atPath: (to as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
    if !FileManager.default.fileExists(atPath: to) { return cloneItem(from, to) }
    guard isDirectory.boolValue, let entries = try? FileManager.default.contentsOfDirectory(atPath: from) else {
      return false
    }
    var cloned = false
    for entry in entries {
      let target = (to as NSString).appendingPathComponent(entry)
      guard !FileManager.default.fileExists(atPath: target) else { continue }
      cloned = cloneItem((from as NSString).appendingPathComponent(entry), target) || cloned
    }
    return cloned
  }

  private static func cloneItem(_ from: String, _ to: String) -> Bool {
    if clonefile(from, to, UInt32(CLONE_NOFOLLOW)) == 0 { return true }
    return (try? FileManager.default.copyItem(atPath: from, toPath: to)) != nil
  }

  /// A relative path that stays inside the folder it's relative to.
  static func isInside(_ path: String) -> Bool {
    !path.isEmpty && !path.hasPrefix("/") && !path.hasPrefix("~") && !path.split(separator: "/").contains("..")
  }
}
