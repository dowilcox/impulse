// Persistent command history shared by every terminal: one SQLite row per
// command run, with where and how it ran. Queries return one hit per
// distinct command (its most recent run and how often it ran).

import Foundation
import SQLite3

public struct HistoryEntry: Equatable, Sendable {
  public var command: String
  public var cwd: String?
  /// Repository root the command ran in.
  public var repo: String?
  public var branch: String?
  public var exitCode: Int?
  public var durationMs: Int?
  public var startedAt: Date
  /// Which terminal session ran it (for "this session first" ordering).
  public var session: String?

  public init(
    command: String, cwd: String? = nil, repo: String? = nil, branch: String? = nil,
    exitCode: Int? = nil, durationMs: Int? = nil, startedAt: Date = Date(),
    session: String? = nil
  ) {
    self.command = command
    self.cwd = cwd
    self.repo = repo
    self.branch = branch
    self.exitCode = exitCode
    self.durationMs = durationMs
    self.startedAt = startedAt
    self.session = session
  }

  /// Commands worth keeping: not empty, and not hidden with a leading space
  /// (the shells' "don't record this" convention).
  public static func shouldRecord(_ command: String) -> Bool {
    guard let first = command.first, first != " ", first != "\t" else { return false }
    return !command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }
}

/// One distinct command: its most recent run, and how many times it ran
/// (within the query's filter).
public struct HistoryHit: Equatable, Sendable {
  public let entry: HistoryEntry
  public let uses: Int
}

public struct HistoryFilter: Equatable, Sendable {
  public var cwd: String?
  public var repo: String?
  public var failedOnly: Bool
  public var since: Date?

  public init(cwd: String? = nil, repo: String? = nil, failedOnly: Bool = false, since: Date? = nil) {
    self.cwd = cwd
    self.repo = repo
    self.failedOnly = failedOnly
    self.since = since
  }
}

public final class CommandHistoryDatabase: @unchecked Sendable {
  private var db: OpaquePointer?
  private let queue = DispatchQueue(label: "impulse.history.db")
  /// Rows kept; older ones are pruned now and then.
  public static let rowLimit = 100_000

  /// Open (creating if needed) the database at `path`; ":memory:" for a
  /// throwaway one. Nil if it can't be opened.
  public init?(path: String) {
    var handle: OpaquePointer?
    let flags = SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX
    guard sqlite3_open_v2(path, &handle, flags, nil) == SQLITE_OK, let handle else {
      if let handle { sqlite3_close(handle) }
      return nil
    }
    db = handle
    sqlite3_busy_timeout(handle, 2000)
    let schema = """
      PRAGMA journal_mode = WAL;
      CREATE TABLE IF NOT EXISTS commands (
        id INTEGER PRIMARY KEY,
        command TEXT NOT NULL,
        cwd TEXT,
        repo TEXT,
        branch TEXT,
        exit_code INTEGER,
        duration_ms INTEGER,
        started_at REAL NOT NULL,
        session TEXT
      );
      CREATE INDEX IF NOT EXISTS commands_started ON commands(started_at);
      CREATE INDEX IF NOT EXISTS commands_command ON commands(command);
      """
    guard sqlite3_exec(handle, schema, nil, nil, nil) == SQLITE_OK else {
      sqlite3_close(handle)
      db = nil
      return nil
    }
    if path != ":memory:" {
      try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
    }
  }

  deinit {
    if let db { sqlite3_close(db) }
  }

  // MARK: Writing

  /// Add runs (skipping ones `shouldRecord` rejects). Synchronous.
  public func insert(_ entries: [HistoryEntry]) {
    queue.sync {
      guard let db else { return }
      sqlite3_exec(db, "BEGIN", nil, nil, nil)
      let sql = """
        INSERT INTO commands (command, cwd, repo, branch, exit_code, duration_ms, started_at, session)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """
      var statement: OpaquePointer?
      guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else {
        sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
        return
      }
      defer { sqlite3_finalize(statement) }
      for entry in entries where HistoryEntry.shouldRecord(entry.command) {
        sqlite3_reset(statement)
        bind(statement, 1, entry.command)
        bind(statement, 2, entry.cwd)
        bind(statement, 3, entry.repo)
        bind(statement, 4, entry.branch)
        bind(statement, 5, entry.exitCode)
        bind(statement, 6, entry.durationMs)
        sqlite3_bind_double(statement, 7, entry.startedAt.timeIntervalSince1970)
        bind(statement, 8, entry.session)
        _ = sqlite3_step(statement)
      }
      sqlite3_exec(db, "COMMIT", nil, nil, nil)
    }
  }

  /// Drop the oldest rows beyond `limit`.
  public func prune(keeping limit: Int = rowLimit) {
    queue.sync {
      guard let db else { return }
      let sql = """
        DELETE FROM commands WHERE id NOT IN (
          SELECT id FROM commands ORDER BY started_at DESC LIMIT \(max(0, limit)))
        """
      sqlite3_exec(db, sql, nil, nil, nil)
    }
  }

  public var count: Int {
    queue.sync {
      guard let db else { return 0 }
      var statement: OpaquePointer?
      guard sqlite3_prepare_v2(db, "SELECT COUNT(*) FROM commands", -1, &statement, nil) == SQLITE_OK
      else { return 0 }
      defer { sqlite3_finalize(statement) }
      return sqlite3_step(statement) == SQLITE_ROW ? Int(sqlite3_column_int64(statement, 0)) : 0
    }
  }

  // MARK: Reading

  /// Distinct commands, most recently run first, narrowed by `filter` and,
  /// when given, to commands containing `text` (case-insensitive).
  public func recent(matching text: String? = nil, filter: HistoryFilter = HistoryFilter(), limit: Int)
    -> [HistoryHit]
  {
    queue.sync {
      guard let db, limit > 0 else { return [] }
      var conditions: [String] = []
      var values: [Any] = []
      if let text, !text.isEmpty {
        conditions.append("instr(lower(command), lower(?)) > 0")
        values.append(text)
      }
      if let cwd = filter.cwd {
        conditions.append("cwd = ?")
        values.append(cwd)
      }
      if let repo = filter.repo {
        conditions.append("repo = ?")
        values.append(repo)
      }
      if filter.failedOnly { conditions.append("exit_code IS NOT NULL AND exit_code <> 0") }
      if let since = filter.since {
        conditions.append("started_at >= ?")
        values.append(since.timeIntervalSince1970)
      }
      let whereClause = conditions.isEmpty ? "" : "WHERE " + conditions.joined(separator: " AND ")
      // The newest row per command (SQLite returns the bare columns of the
      // MAX() row) plus its run count.
      let sql = """
        SELECT command, cwd, repo, branch, exit_code, duration_ms, MAX(started_at), session, COUNT(*)
        FROM commands \(whereClause)
        GROUP BY command ORDER BY MAX(started_at) DESC LIMIT \(limit)
        """
      var statement: OpaquePointer?
      guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else { return [] }
      defer { sqlite3_finalize(statement) }
      for (index, value) in values.enumerated() {
        let position = Int32(index + 1)
        if let string = value as? String {
          bind(statement, position, string)
        } else if let number = value as? Double {
          sqlite3_bind_double(statement, position, number)
        }
      }
      var hits: [HistoryHit] = []
      while sqlite3_step(statement) == SQLITE_ROW {
        let entry = HistoryEntry(
          command: string(statement, 0) ?? "",
          cwd: string(statement, 1),
          repo: string(statement, 2),
          branch: string(statement, 3),
          exitCode: int(statement, 4),
          durationMs: int(statement, 5),
          startedAt: Date(timeIntervalSince1970: sqlite3_column_double(statement, 6)),
          session: string(statement, 7))
        hits.append(HistoryHit(entry: entry, uses: Int(sqlite3_column_int64(statement, 8))))
      }
      return hits
    }
  }

  // MARK: SQLite helpers

  private static let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

  private func bind(_ statement: OpaquePointer?, _ index: Int32, _ value: String?) {
    if let value {
      sqlite3_bind_text(statement, index, value, -1, Self.transient)
    } else {
      sqlite3_bind_null(statement, index)
    }
  }

  private func bind(_ statement: OpaquePointer?, _ index: Int32, _ value: Int?) {
    if let value {
      sqlite3_bind_int64(statement, index, Int64(value))
    } else {
      sqlite3_bind_null(statement, index)
    }
  }

  private func string(_ statement: OpaquePointer?, _ column: Int32) -> String? {
    guard sqlite3_column_type(statement, column) != SQLITE_NULL,
      let text = sqlite3_column_text(statement, column)
    else { return nil }
    return String(cString: text)
  }

  private func int(_ statement: OpaquePointer?, _ column: Int32) -> Int? {
    guard sqlite3_column_type(statement, column) != SQLITE_NULL else { return nil }
    return Int(sqlite3_column_int64(statement, column))
  }
}
