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
