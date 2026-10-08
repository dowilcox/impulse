import CryptoKit
import Foundation
import ImpulseKit

/// Review comments for one repository, persisted under the app's data
/// directory (never inside the repository).
final class ReviewCommentStore {
  private static var stores: [String: ReviewCommentStore] = [:]

  /// The shared store for a repository root.
  static func forRepository(_ root: String) -> ReviewCommentStore {
    if let existing = stores[root] { return existing }
    let store = ReviewCommentStore(root: root)
    stores[root] = store
    return store
  }

  let root: String
  private(set) var comments: [ReviewComment] = []
  private let fileURL: URL

  private init(root: String) {
    self.root = root
    let digest = SHA256.hash(data: Data(root.utf8)).prefix(10).map { String(format: "%02x", $0) }
      .joined()
    let dir = AppPaths.dataDirectory.appendingPathComponent("review", isDirectory: true)
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    fileURL = dir.appendingPathComponent("\(digest).json")
    load()
  }

  func comments(for path: String) -> [ReviewComment] {
    comments.filter { $0.path == path }
  }

  func add(_ comment: ReviewComment) {
    comments.append(comment)
    save()
  }

  func update(id: String, text: String) {
    guard let index = comments.firstIndex(where: { $0.id == id }) else { return }
    comments[index].text = text
    save()
  }

  func remove(id: String) {
    comments.removeAll { $0.id == id }
    save()
  }

  func removeAll() {
    comments.removeAll()
    save()
  }

  private func load() {
    guard let data = try? Data(contentsOf: fileURL) else { return }
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    comments = (try? decoder.decode([ReviewComment].self, from: data)) ?? []
  }

  private func save() {
    guard AppState.persistenceEnabled else { return }
    let encoder = JSONEncoder()
    encoder.dateEncodingStrategy = .iso8601
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    guard let data = try? encoder.encode(comments) else { return }
    try? data.write(to: fileURL, options: .atomic)
  }
}

/// Which files have been marked viewed, keyed by the diff content they were
/// viewed at — so a file that changes again comes back unviewed.
enum ReviewViewedStore {
  private static func key(root: String, scope: String) -> String {
    "reviewViewed:\(root):\(scope)"
  }

  static func viewed(root: String, scope: String) -> [String: String] {
    (UserDefaults.standard.dictionary(forKey: key(root: root, scope: scope)) as? [String: String])
      ?? [:]
  }

  static func set(path: String, hash: String?, root: String, scope: String) {
    var map = viewed(root: root, scope: scope)
    map[path] = hash
    UserDefaults.standard.set(map, forKey: key(root: root, scope: scope))
  }
}
