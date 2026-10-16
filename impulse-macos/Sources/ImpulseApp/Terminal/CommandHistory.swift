import Foundation
import ImpulseGit
import ImpulseKit

/// Impulse's command history across every terminal and launch: finished
/// commands go into a SQLite store (`history.sqlite3` in the data directory)
/// with their folder, repository, branch, exit status and duration. A list
/// of recent distinct commands stays in memory for per-keystroke lookups
/// (ghost suggestions, ↑/↓ in the input bar).
///
/// The Rust terminal core keeps its own per-session history; that stays the
/// source for "this session first" ordering, and this store adds everything
/// else. With "Keep command history" off nothing is read from or written to
/// the store, apart from clearing it.
final class CommandHistory {
  static let shared = CommandHistory()

  private let queue = DispatchQueue(label: "impulse.history", qos: .utility)
  private static let cacheSize = 2000

  /// The store, opened on first use. Only touched on `queue`.
  private lazy var database: CommandHistoryDatabase? = CommandHistoryDatabase(
    path: AppState.persistenceEnabled
      ? AppPaths.dataDirectory.appendingPathComponent("history.sqlite3").path : ":memory:")

  /// Distinct commands, newest first; empty while history is off. Main
  /// thread only.
  var recentCommands: [String] {
    guard isEnabled else { return [] }
    loadIfNeeded()
    return cache
  }
  /// Bumped whenever `recentCommands` changes (cheap check for caches),
  /// including when history is turned off or on.
  var revision: Int { isEnabled ? cacheRevision : -1 }

  private var cache: [String] = []
  private var cacheRevision = 0
  private var loaded = false

  private init() {
    if isEnabled { loadIfNeeded() }
  }

  private var isEnabled: Bool { SettingsStore.shared.settings.terminalPersistentHistory }

  /// Read the newest commands into the cache, the first time they're needed
  /// with history on.
  private func loadIfNeeded() {
    guard !loaded else { return }
    loaded = true
    queue.async { [weak self] in
      guard let self, let database = self.database else { return }
      database.prune()
      let recent = database.recent(limit: Self.cacheSize).map(\.entry.command)
      DispatchQueue.main.async { self.mergeIntoCache(recent, atFront: false) }
    }
  }

  /// Store a finished command. Repository and branch are looked up in the
  /// background.
  func record(
    command: String, cwd: String?, exitCode: Int?, durationMs: Int?, session: String?
  ) {
    guard isEnabled, HistoryEntry.shouldRecord(command) else { return }
    loadIfNeeded()
    mergeIntoCache([command], atFront: true)
    let started = Date().addingTimeInterval(-Double(durationMs ?? 0) / 1000)
    queue.async { [weak self] in
      let repo = cwd.flatMap { GitClient.repoRoot(forPath: $0) }
      let branch = repo.flatMap { GitClient.branch(forPath: $0) }
      self?.database?.insert([
        HistoryEntry(
          command: command, cwd: cwd, repo: repo, branch: branch, exitCode: exitCode,
          durationMs: durationMs, startedAt: started, session: session)
      ])
    }
  }

  /// Distinct commands matching `filter`, fuzzily ranked by `text` (most
  /// recent first when `text` is empty). Delivered on the main queue; none
  /// while history is off.
  func search(
    _ text: String, filter: HistoryFilter, limit: Int = 200,
    completion: @escaping ([(hit: HistoryHit, positions: [Int])]) -> Void
  ) {
    guard isEnabled else {
      DispatchQueue.main.async { completion([]) }
      return
    }
    queue.async { [weak self] in
      let candidates = self?.database?.recent(filter: filter, limit: text.isEmpty ? limit : 5000) ?? []
      let results: [(hit: HistoryHit, positions: [Int])]
      if text.isEmpty {
        results = candidates.map { ($0, []) }
      } else {
        results = FuzzyMatcher.rank(candidates, query: text, limit: limit) { $0.entry.command }
          .map { ($0.item, $0.match.positions) }
      }
      DispatchQueue.main.async { completion(results) }
    }
  }

  /// Import zsh, bash and fish history files found in the home folder.
  /// Reports how many commands the files had and how many of them were new
  /// (importing again adds only what's new). Nothing happens while history
  /// is off.
  func importShellHistory(completion: @escaping (_ found: Int, _ added: Int) -> Void) {
    guard isEnabled else { return }
    loadIfNeeded()
    queue.async { [weak self] in
      guard let self else { return }
      let home = URL(fileURLWithPath: NSHomeDirectory())
      var entries: [HistoryEntry] = []
      if let data = try? Data(contentsOf: home.appendingPathComponent(".zsh_history")) {
        entries += ShellHistoryImport.zsh(data)
      }
      if let data = try? Data(contentsOf: home.appendingPathComponent(".bash_history")) {
        entries += ShellHistoryImport.bash(data)
      }
      if let data = try? Data(contentsOf: home.appendingPathComponent(".local/share/fish/fish_history")) {
        entries += ShellHistoryImport.fish(data)
      }
      let added = self.database?.importEntries(entries) ?? 0
      let recent = self.database?.recent(limit: Self.cacheSize).map(\.entry.command) ?? []
      DispatchQueue.main.async {
        self.cache = []
        self.mergeIntoCache(recent, atFront: false)
        completion(entries.count, added)
      }
    }
  }

  /// When the history was last cleared (milliseconds since 1970; 0 for
  /// never). Each terminal's own session history leaves out what ran
  /// before it. Main thread only.
  private(set) var clearedAtMs: UInt64 = 0

  /// Delete every stored command. Delivered on the main queue.
  func clear(completion: @escaping () -> Void) {
    clearedAtMs = UInt64(Date().timeIntervalSince1970 * 1000)
    cacheRevision += 1
    queue.async { [weak self] in
      self?.database?.removeAll()
      DispatchQueue.main.async {
        guard let self else { return }
        self.cache = []
        self.cacheRevision += 1
        completion()
      }
    }
  }

  private func mergeIntoCache(_ commands: [String], atFront: Bool) {
    if atFront {
      let incoming = Set(commands)
      cache = commands + cache.filter { !incoming.contains($0) }
    } else {
      let existing = Set(cache)
      cache += commands.filter { !existing.contains($0) }
    }
    if cache.count > Self.cacheSize {
      cache.removeLast(cache.count - Self.cacheSize)
    }
    cacheRevision += 1
  }
}
