import Foundation

/// Folders whose code Impulse may run on its own. Some language servers run
/// a project's build scripts and tools (rust-analyzer runs build.rs, ESLint
/// loads the project's config and plugins), formatters on save load project
/// config, and background fetch follows the repository's own git
/// configuration. In a folder that isn't trusted, all of that stays off;
/// what the user runs themselves (the terminal, git actions) is unaffected.
///
/// Trusting a folder trusts everything inside it, except a folder marked
/// restricted (a task checked out from someone else's branch), which stays
/// restricted until it's trusted itself. Thread-safe: language servers ask
/// from background threads.
public final class WorkspaceTrust: @unchecked Sendable {
  private let lock = NSLock()
  private var folders: Set<String> = []
  /// Restricted even inside a trusted folder.
  private var restricted: Set<String> = []
  private var enabled = true
  private let file: URL?

  /// Whether the store existed before this launch (it didn't on the first
  /// launch with workspace trust: folders already in use can be trusted
  /// without asking).
  public let existedBefore: Bool

  /// `file`: where the trusted folders are kept (nil: in memory only).
  public init(file: URL?) {
    self.file = file
    if let file, let data = try? Data(contentsOf: file),
      let stored = try? JSONDecoder().decode(Stored.self, from: data)
    {
      folders = Set(stored.trusted.map(Self.normalize))
      restricted = Set((stored.restricted ?? []).map(Self.normalize))
      existedBefore = true
    } else {
      existedBefore = file.map { FileManager.default.fileExists(atPath: $0.path) } ?? false
    }
  }

  /// Off: every folder is trusted (the "Ask before trusting folders" setting).
  public var isEnabled: Bool {
    get { lock.withLock { enabled } }
    set { lock.withLock { enabled = newValue } }
  }

  /// Whether Impulse may run code from `path` (a file or folder).
  public func isTrusted(_ path: String) -> Bool {
    let path = Self.normalize(path)
    return lock.withLock {
      guard enabled else { return true }
      return trustingFolder(of: path) != nil
    }
  }

  /// The trusted folder `path` is in, if any.
  public func trustingFolder(_ path: String) -> String? {
    let path = Self.normalize(path)
    return lock.withLock { trustingFolder(of: path) }
  }

  public var trustedFolders: [String] {
    lock.withLock { folders.sorted() }
  }

  public var restrictedFolders: [String] {
    lock.withLock { restricted.sorted() }
  }

  /// Trust `folder`. A restriction on it goes too; one on a folder inside
  /// it stays (trusting `~/Code` doesn't trust a checkout of someone else's
  /// branch there).
  public func trust(_ folder: String) {
    let folder = Self.normalize(folder)
    lock.withLock {
      restricted.remove(folder)
      guard trustingFolder(of: folder) == nil else { return }
      // Folders inside it are covered now.
      folders = folders.filter { !Self.contains(folder, $0) }
      folders.insert(folder)
    }
    save()
  }

  /// Keep `folder` restricted even inside a trusted folder, until it's
  /// trusted itself.
  public func restrict(_ folder: String) {
    let folder = Self.normalize(folder)
    lock.withLock {
      folders = folders.filter { !Self.contains(folder, $0) }
      restricted.insert(folder)
    }
    save()
  }

  /// Stop trusting `folder`, and anything trusted inside it. Returns the
  /// folder still trusting it from above (only removing that one would
  /// restrict `folder`), or nil when it's restricted now.
  @discardableResult
  public func revoke(_ folder: String) -> String? {
    let folder = Self.normalize(folder)
    let above: String? = lock.withLock {
      folders = folders.filter { !Self.contains(folder, $0) }
      return trustingFolder(of: folder)
    }
    save()
    return above
  }

  /// Write the list (so the store exists even with nothing trusted).
  public func persist() {
    save()
  }

  public func revokeAll() {
    lock.withLock {
      folders = []
      restricted = []
    }
    save()
  }

  /// Absolute, standardized and with symlinks resolved, so /tmp and
  /// /private/tmp are the same folder; no trailing slash.
  public static func normalize(_ path: String) -> String {
    let expanded = (path as NSString).expandingTildeInPath
    var resolved = expanded
    if let real = realpath(expanded, nil) {
      resolved = String(cString: real)
      free(real)
    }
    let standardized = (resolved as NSString).standardizingPath
    return standardized.count > 1 && standardized.hasSuffix("/")
      ? String(standardized.dropLast()) : standardized
  }

  /// `folder` is `path` or one of its ancestors.
  static func contains(_ folder: String, _ path: String) -> Bool {
    folder == "/" || path == folder || path.hasPrefix(folder + "/")
  }

  /// The trusted folder `path` is in, the innermost one, unless a
  /// restricted folder at or inside it holds `path`.
  private func trustingFolder(of path: String) -> String? {
    guard let folder = folders.filter({ Self.contains($0, path) }).max(by: { $0.count < $1.count }) else {
      return nil
    }
    return restricted.contains { Self.contains($0, path) && Self.contains(folder, $0) } ? nil : folder
  }

  private struct Stored: Codable {
    var version = 1
    var trusted: [String]
    var restricted: [String]?
  }

  private func save() {
    guard let file else { return }
    let stored = Stored(trusted: trustedFolders, restricted: restrictedFolders)
    guard let data = try? JSONEncoder().encode(stored) else { return }
    try? FileManager.default.createDirectory(
      at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
    try? data.write(to: file, options: .atomic)
  }
}
