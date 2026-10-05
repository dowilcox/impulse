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
/// else.
final class CommandHistory {
  static let shared = CommandHistory()

  private let database: CommandHistoryDatabase?
  private let queue = DispatchQueue(label: "impulse.history", qos: .utility)
  private static let cacheSize = 2000

  /// Distinct commands, newest first. Main thread only.
  private(set) var recentCommands: [String] = []

  private init() {
    let path =
      AppState.persistenceEnabled
      ? AppPaths.dataDirectory.appendingPathComponent("history.sqlite3").path : ":memory:"
    database = CommandHistoryDatabase(path: path)
    queue.async { [weak self] in
      guard let database = self?.database else { return }
      database.prune()
      let recent = database.recent(limit: Self.cacheSize).map(\.entry.command)
      DispatchQueue.main.async { self?.mergeIntoCache(recent, atFront: false) }
    }
  }

  private var isEnabled: Bool { SettingsStore.shared.settings.terminalPersistentHistory }

  /// Store a finished command. Repository and branch are looked up in the
  /// background.
  func record(
    command: String, cwd: String?, exitCode: Int?, durationMs: Int?, session: String?
  ) {
    guard isEnabled, HistoryEntry.shouldRecord(command) else { return }
    mergeIntoCache([command], atFront: true)
    let started = Date().addingTimeInterval(-Double(durationMs ?? 0) / 1000)
    queue.async { [database] in
      let repo = cwd.flatMap { GitClient.repoRoot(forPath: $0) }
      let branch = repo.flatMap { GitClient.branch(forPath: $0) }
      database?.insert([
        HistoryEntry(
          command: command, cwd: cwd, repo: repo, branch: branch, exitCode: exitCode,
          durationMs: durationMs, startedAt: started, session: session)
      ])
    }
  }

  /// Distinct commands matching `filter`, fuzzily ranked by `text` (most
  /// recent first when `text` is empty). Delivered on the main queue.
  func search(
    _ text: String, filter: HistoryFilter, limit: Int = 200,
    completion: @escaping ([(hit: HistoryHit, positions: [Int])]) -> Void
  ) {
    queue.async { [database] in
      let candidates = database?.recent(filter: filter, limit: text.isEmpty ? limit : 5000) ?? []
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
  /// Reports how many commands were added.
  func importShellHistory(completion: @escaping (Int) -> Void) {
    queue.async { [database] in
      let home = URL(fileURLWithPath: NSHomeDirectory())
      var entries: [HistoryEntry] = []
      if let data = try? Data(contentsOf: home.appendingPathComponent(".zsh_history")) {
        entries += ShellHistoryImport.zsh(data)
      }
      if let text = try? String(contentsOf: home.appendingPathComponent(".bash_history"), encoding: .utf8) {
        entries += ShellHistoryImport.bash(text)
      }
      let fishPath = home.appendingPathComponent(".local/share/fish/fish_history")
      if let text = try? String(contentsOf: fishPath, encoding: .utf8) {
        entries += ShellHistoryImport.fish(text)
      }
      database?.insert(entries)
      let recent = database?.recent(limit: Self.cacheSize).map(\.entry.command) ?? []
      DispatchQueue.main.async { [weak self] in
        self?.recentCommands = []
        self?.mergeIntoCache(recent, atFront: false)
        completion(entries.count)
      }
    }
  }

  private func mergeIntoCache(_ commands: [String], atFront: Bool) {
    if atFront {
      let incoming = Set(commands)
      recentCommands = commands + recentCommands.filter { !incoming.contains($0) }
    } else {
      let existing = Set(recentCommands)
      recentCommands += commands.filter { !existing.contains($0) }
    }
    if recentCommands.count > Self.cacheSize {
      recentCommands.removeLast(recentCommands.count - Self.cacheSize)
    }
  }
}
