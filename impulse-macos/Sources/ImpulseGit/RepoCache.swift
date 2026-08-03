// Port of the `REPO_ROOT_CACHE` LRU in impulse-core/src/git.rs: a thread-safe
// cache mapping directory paths to their discovered git repo root, avoiding
// repeated `git_repository_discover` walks up the directory tree.

import Foundation

final class RepoCache {
  static let shared = RepoCache(capacity: 64)

  private let capacity: Int
  private let lock = NSLock()
  private var map: [String: String] = [:]
  /// Least-recently-used first.
  private var order: [String] = []

  init(capacity: Int) {
    precondition(capacity > 0)
    self.capacity = capacity
  }

  /// Cached repo root for a directory, marking the entry most-recently used.
  func root(forDirectory dir: String) -> String? {
    lock.lock()
    defer { lock.unlock() }
    guard let root = map[dir] else { return nil }
    touch(dir)
    return root
  }

  /// Insert/update an entry, evicting the least-recently-used one when full.
  func store(root: String, forDirectory dir: String) {
    lock.lock()
    defer { lock.unlock() }
    if map[dir] == nil, map.count >= capacity, let oldest = order.first {
      order.removeFirst()
      map.removeValue(forKey: oldest)
    }
    map[dir] = root
    touch(dir)
  }

  func removeAll() {
    lock.lock()
    defer { lock.unlock() }
    map.removeAll()
    order.removeAll()
  }

  var count: Int {
    lock.lock()
    defer { lock.unlock() }
    return map.count
  }

  private func touch(_ key: String) {
    if let index = order.firstIndex(of: key) {
      order.remove(at: index)
    }
    order.append(key)
  }
}
