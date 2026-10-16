import CryptoKit
import Foundation
import ImpulseGit
import ImpulseKit

struct ReviewCapabilities {
  /// Hunks/lines can be staged (unstaged scope).
  let stage: Bool
  /// Hunks/lines can be unstaged (staged scope).
  let unstage: Bool
  /// Hunks/lines can be reverted in the working tree (unstaged scope).
  let revert: Bool
}

struct ReviewOptions: Codable, Equatable {
  var layout: String = "unified"  // "unified" | "split"
  var ignoreWhitespace: Bool = false
  /// Unchanged lines around each change (`review_context_lines`, or what
  /// the header's context menu picked for this review).
  var contextLines: Int = 3

  /// "Whole file": more context than any file shown has lines.
  static let wholeFile = 100_000
}

enum ReviewAction: String {
  case stage, unstage, revert
}

/// A working-tree file as it is on disk (lstat): when it's the same as
/// when its diff was read, and only files changed since, the diff still
/// holds. nil for a file that's gone, and for anything but a file or a
/// symlink (a submodule's folder doesn't change when its checkout does).
struct FileSignature: Equatable, Sendable {
  let inode: UInt64
  let size: Int64
  let mode: UInt16
  let modified: [Int]
  let changed: [Int]

  init?(root: String, path: String) {
    var info = stat()
    guard lstat((root as NSString).appendingPathComponent(path), &info) == 0 else { return nil }
    let type = info.st_mode & S_IFMT
    guard type == S_IFREG || type == S_IFLNK else { return nil }
    inode = UInt64(info.st_ino)
    size = Int64(info.st_size)
    mode = UInt16(info.st_mode)
    modified = [Int(info.st_mtimespec.tv_sec), Int(info.st_mtimespec.tv_nsec)]
    changed = [Int(info.st_ctimespec.tv_sec), Int(info.st_ctimespec.tv_nsec)]
  }
}

extension FileDiff {
  /// Identity of the diff's content (changes, not positions or how many
  /// context lines group them into hunks). Stable across launches, so
  /// "viewed" marks can be persisted against it.
  var contentHash: String {
    let changes = hunks.isEmpty ? "" : GitClient.changeIdentity(hunks)
    let text = "\(path)|\(isBinary)|\(tooLarge)|" + changes
    let digest = SHA256.hash(data: Data(text.utf8))
    return digest.prefix(8).map { String(format: "%02x", $0) }.joined()
  }
}
