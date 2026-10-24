//! Persistent command history (the Swift `CommandHistoryDatabase`): one
//! SQLite row per command run, with where and how it ran. Queries return one
//! hit per distinct command, its most recent run and how often it ran.
//!
//! The file, schema and values are the Swift app's: times are seconds since
//! 1970 as a REAL, and a command imported without a time has Swift's
//! `Date.distantPast` (`DISTANT_PAST`), so either app can read, import into
//! and prune the same database.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OpenFlags};

/// Swift's `Date.distantPast` (1 January 0001) in seconds since 1970: the
/// time of an imported command whose history file had none.
pub const DISTANT_PAST: f64 = -62_135_769_600.0;

/// Seconds since 1970, now.
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64()
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub command: String,
    pub cwd: Option<String>,
    /// Repository root the command ran in.
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    /// Seconds since 1970.
    pub started_at: f64,
    /// Which terminal session ran it (for "this session first" ordering).
    pub session: Option<String>,
}

impl HistoryEntry {
    /// A command and when it started, with nothing else known.
    pub fn new(command: impl Into<String>, started_at: f64) -> Self {
        HistoryEntry {
            command: command.into(),
            cwd: None,
            repo: None,
            branch: None,
            exit_code: None,
            duration_ms: None,
            started_at,
            session: None,
        }
    }

    /// Commands worth keeping: not empty, and not hidden with a leading
    /// space or tab (the shells' "don't record this" convention).
    pub fn should_record(command: &str) -> bool {
        match command.chars().next() {
            None | Some(' ') | Some('\t') => false,
            Some(_) => !command.trim().is_empty(),
        }
    }
}

/// One distinct command: its most recent run, and how many times it ran
/// (within the query's filter).
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryHit {
    pub entry: HistoryEntry,
    pub uses: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryFilter {
    pub cwd: Option<String>,
    pub repo: Option<String>,
    pub failed_only: bool,
    /// Seconds since 1970.
    pub since: Option<f64>,
}

const SCHEMA: &str = "
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
DROP INDEX IF EXISTS commands_command;
CREATE INDEX IF NOT EXISTS commands_command_started ON commands(command, started_at);
";

/// A row from a shell's history file: only the command and when.
const IMPORTED_ROW: &str =
    "session IS NULL AND cwd IS NULL AND repo IS NULL AND exit_code IS NULL AND duration_ms IS NULL";

/// Thread-safe: every call takes the connection in turn.
pub struct CommandHistoryDatabase {
    connection: Mutex<Connection>,
}

impl CommandHistoryDatabase {
    /// Rows kept; older ones are pruned now and then.
    pub const ROW_LIMIT: usize = 100_000;

    /// Open (creating if needed) the database at `path`; `":memory:"` for a
    /// throwaway one. `None` if it can't be opened.
    pub fn open(path: impl AsRef<Path>) -> Option<Self> {
        let path = path.as_ref();
        if path != Path::new(":memory:") {
            restrict_to_owner(path);
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(path, flags).ok()?;
        connection.busy_timeout(Duration::from_secs(2)).ok()?;
        connection.execute_batch(SCHEMA).ok()?;
        migrate(&connection);
        Some(CommandHistoryDatabase {
            connection: Mutex::new(connection),
        })
    }

    /// A throwaway database in memory.
    pub fn in_memory() -> Option<Self> {
        Self::open(":memory:")
    }

    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection.lock().unwrap_or_else(|e| e.into_inner())
    }

    // Writing

    /// Add runs, skipping ones `should_record` rejects.
    pub fn insert(&self, entries: &[HistoryEntry]) {
        let mut connection = self.connection();
        let Ok(transaction) = connection.transaction() else {
            return;
        };
        {
            let Ok(mut statement) = transaction.prepare(
                "INSERT INTO commands (command, cwd, repo, branch, exit_code, duration_ms, started_at, session)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            ) else {
                return;
            };
            for entry in entries
                .iter()
                .filter(|e| HistoryEntry::should_record(&e.command))
            {
                let _ = statement.execute(params![
                    entry.command,
                    entry.cwd,
                    entry.repo,
                    entry.branch,
                    entry.exit_code,
                    entry.duration_ms,
                    entry.started_at,
                    entry.session,
                ]);
            }
        }
        let _ = transaction.commit();
    }

    /// Add commands read from a shell's history file, skipping ones already
    /// stored with the same time (an earlier import, or twice in the file),
    /// so importing again adds only what's new. Returns how many were added.
    pub fn import_entries(&self, entries: &[HistoryEntry]) -> usize {
        let mut connection = self.connection();
        let Ok(transaction) = connection.transaction() else {
            return 0;
        };
        let mut added = 0;
        {
            let Ok(mut statement) = transaction.prepare(
                "INSERT INTO commands (command, started_at) SELECT ?1, ?2
                 WHERE NOT EXISTS (SELECT 1 FROM commands WHERE command = ?1 AND started_at = ?2)",
            ) else {
                return 0;
            };
            for entry in entries
                .iter()
                .filter(|e| HistoryEntry::should_record(&e.command))
            {
                if statement
                    .execute(params![entry.command, entry.started_at])
                    .is_ok_and(|changed| changed > 0)
                {
                    added += 1;
                }
            }
        }
        if transaction.commit().is_err() {
            return 0;
        }
        added
    }

    /// Delete every row, and the space they took (so nothing of them is
    /// left in the file).
    pub fn remove_all(&self) {
        let _ = self
            .connection()
            .execute_batch("DELETE FROM commands; VACUUM; PRAGMA wal_checkpoint(TRUNCATE);");
    }

    /// Drop the oldest rows beyond `limit`.
    pub fn prune(&self, limit: usize) {
        let _ = self.connection().execute(
            "DELETE FROM commands WHERE id NOT IN (
               SELECT id FROM commands ORDER BY started_at DESC LIMIT ?)",
            [limit as i64],
        );
    }

    pub fn count(&self) -> usize {
        self.connection()
            .query_row("SELECT COUNT(*) FROM commands", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_or(0, |count| count as usize)
    }

    // Reading

    /// Distinct commands, most recently run first, narrowed by `filter`
    /// and, when given, to commands containing `text` (case-insensitive).
    pub fn recent(
        &self,
        text: Option<&str>,
        filter: &HistoryFilter,
        limit: usize,
    ) -> Vec<HistoryHit> {
        if limit == 0 {
            return Vec::new();
        }
        let mut conditions: Vec<&str> = Vec::new();
        let mut values: Vec<Value> = Vec::new();
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            conditions.push("instr(lower(command), lower(?)) > 0");
            values.push(Value::Text(text.to_string()));
        }
        if let Some(cwd) = &filter.cwd {
            conditions.push("cwd = ?");
            values.push(Value::Text(cwd.clone()));
        }
        if let Some(repo) = &filter.repo {
            conditions.push("repo = ?");
            values.push(Value::Text(repo.clone()));
        }
        if filter.failed_only {
            conditions.push("exit_code IS NOT NULL AND exit_code <> 0");
        }
        if let Some(since) = filter.since {
            conditions.push("started_at >= ?");
            values.push(Value::Real(since));
        }
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };
        // The newest row per command (SQLite returns the bare columns of the
        // MAX() row) plus its run count.
        let sql = format!(
            "SELECT command, cwd, repo, branch, exit_code, duration_ms, MAX(started_at), session, COUNT(*)
             FROM commands {where_clause}
             GROUP BY command ORDER BY MAX(started_at) DESC LIMIT {limit}"
        );
        let connection = self.connection();
        let Ok(mut statement) = connection.prepare(&sql) else {
            return Vec::new();
        };
        let rows = statement.query_map(params_from_iter(values), |row| {
            Ok(HistoryHit {
                entry: HistoryEntry {
                    command: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    cwd: row.get(1)?,
                    repo: row.get(2)?,
                    branch: row.get(3)?,
                    exit_code: row.get::<_, Option<i64>>(4)?.map(|c| c as i32),
                    duration_ms: row.get(5)?,
                    started_at: row.get(6)?,
                    session: row.get(7)?,
                },
                uses: row.get::<_, i64>(8)? as usize,
            })
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Owner-only from the start: SQLite gives the -wal and -shm files the
/// database file's mode. Older files were left readable by everyone.
fn restrict_to_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let _ = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path);
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_owned();
            file.push(suffix);
            let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Version 1: imports used to add every command again each time. Keep one
/// row per imported command and time (imported rows have nothing else).
fn migrate(connection: &Connection) {
    let Ok(version) = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
    else {
        return;
    };
    if version >= 1 {
        return;
    }
    let _ = connection.execute_batch(&format!(
        "BEGIN;
         DELETE FROM commands WHERE {IMPORTED_ROW} AND id NOT IN (
           SELECT MIN(id) FROM commands WHERE {IMPORTED_ROW} GROUP BY command, started_at);
         PRAGMA user_version = 1;
         COMMIT;"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::import;

    fn database() -> CommandHistoryDatabase {
        CommandHistoryDatabase::in_memory().unwrap()
    }

    fn entry(command: &str, started_at: f64) -> HistoryEntry {
        HistoryEntry::new(command, started_at)
    }

    fn commands(hits: &[HistoryHit]) -> Vec<&str> {
        hits.iter().map(|h| h.entry.command.as_str()).collect()
    }

    fn all() -> HistoryFilter {
        HistoryFilter::default()
    }

    #[test]
    fn records_distinct_commands_newest_first() {
        let db = database();
        db.insert(&[
            HistoryEntry {
                cwd: Some("/a".into()),
                exit_code: Some(0),
                ..entry("git status", 100.0)
            },
            HistoryEntry {
                cwd: Some("/a".into()),
                exit_code: Some(2),
                ..entry("make", 200.0)
            },
            HistoryEntry {
                cwd: Some("/b".into()),
                exit_code: Some(0),
                ..entry("git status", 300.0)
            },
            entry(" secret --token x", 400.0),
            entry("   ", 500.0),
        ]);
        assert_eq!(db.count(), 3);
        let hits = db.recent(None, &all(), 10);
        assert_eq!(commands(&hits), ["git status", "make"]);
        // The most recent run's details, and how often it ran.
        assert_eq!(hits[0].entry.cwd.as_deref(), Some("/b"));
        assert_eq!(hits[0].uses, 2);
        assert_eq!(hits[1].entry.exit_code, Some(2));
    }

    #[test]
    fn filters_by_text_directory_failure_and_time() {
        let db = database();
        db.insert(&[
            HistoryEntry {
                cwd: Some("/repo".into()),
                repo: Some("/repo".into()),
                exit_code: Some(101),
                ..entry("cargo test", 100.0)
            },
            HistoryEntry {
                cwd: Some("/repo/sub".into()),
                repo: Some("/repo".into()),
                exit_code: Some(0),
                ..entry("cargo build", 200.0)
            },
            HistoryEntry {
                cwd: Some("/tmp".into()),
                exit_code: Some(0),
                ..entry("ls -la", 300.0)
            },
        ]);
        assert_eq!(
            commands(&db.recent(Some("CARGO"), &all(), 10)),
            ["cargo build", "cargo test"]
        );
        let in_tmp = HistoryFilter {
            cwd: Some("/tmp".into()),
            ..all()
        };
        assert_eq!(commands(&db.recent(None, &in_tmp, 10)), ["ls -la"]);
        let in_repo = HistoryFilter {
            repo: Some("/repo".into()),
            ..all()
        };
        assert_eq!(db.recent(None, &in_repo, 10).len(), 2);
        let failed = HistoryFilter {
            failed_only: true,
            ..all()
        };
        assert_eq!(commands(&db.recent(None, &failed, 10)), ["cargo test"]);
        let since = HistoryFilter {
            since: Some(250.0),
            ..all()
        };
        assert_eq!(commands(&db.recent(None, &since, 10)), ["ls -la"]);
        assert_eq!(db.recent(None, &all(), 1).len(), 1);
    }

    #[test]
    fn prunes_oldest_rows() {
        let db = database();
        let entries: Vec<HistoryEntry> = (0..10)
            .map(|i| entry(&format!("cmd {i}"), i as f64))
            .collect();
        db.insert(&entries);
        db.prune(3);
        assert_eq!(db.count(), 3);
        assert_eq!(
            commands(&db.recent(None, &all(), 10)),
            ["cmd 9", "cmd 8", "cmd 7"]
        );
    }

    #[test]
    fn persists_across_connections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite3");
        {
            let db = CommandHistoryDatabase::open(&path).unwrap();
            db.insert(&[entry("echo kept", 1.0)]);
        }
        let reopened = CommandHistoryDatabase::open(&path).unwrap();
        assert_eq!(commands(&reopened.recent(None, &all(), 5)), ["echo kept"]);
    }

    #[test]
    fn importing_again_adds_only_whats_new() {
        let db = database();
        let first = import::bash("#1700000000\nls\n#1700000001\nmake\nplain\nplain\n");
        // The second `plain` (same time) is the same entry.
        assert_eq!(db.import_entries(&first), 3);
        assert_eq!(db.import_entries(&first), 0);
        let more =
            import::bash("#1700000000\nls\n#1700000001\nmake\nplain\n#1700000002\ngit push\n");
        assert_eq!(db.import_entries(&more), 1);
        assert_eq!(db.count(), 4);
        // A command that ran in a terminal is a separate row.
        db.insert(&[HistoryEntry {
            cwd: Some("/a".into()),
            session: Some("s".into()),
            ..entry("ls", 1_700_000_000.0)
        }]);
        assert_eq!(db.count(), 5);
    }

    #[test]
    fn removes_everything() {
        let db = database();
        db.insert(&[entry("echo hi", 1.0)]);
        db.import_entries(&[entry("ls", 2.0)]);
        db.remove_all();
        assert_eq!(db.count(), 0);
        assert!(db.recent(None, &all(), 10).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite3");
        // An older file, readable by everyone.
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let db = CommandHistoryDatabase::open(&path).unwrap();
        db.insert(&[entry("echo secret", 1.0)]);
        for suffix in ["", "-wal", "-shm"] {
            let file = format!("{}{suffix}", path.display());
            if let Ok(meta) = std::fs::metadata(&file) {
                assert_eq!(meta.permissions().mode() & 0o777, 0o600, "{suffix}");
            }
        }
        assert!(Path::new(&format!("{}-wal", path.display())).exists());
    }

    #[test]
    fn migrates_duplicate_imports_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite3");
        {
            // A version 0 database where imports were added twice.
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(SCHEMA).unwrap();
            for _ in 0..2 {
                connection
                    .execute(
                        "INSERT INTO commands (command, started_at) VALUES ('ls', 5)",
                        [],
                    )
                    .unwrap();
            }
            connection
                .execute(
                    "INSERT INTO commands (command, started_at, session) VALUES ('ls', 5, 's')",
                    [],
                )
                .unwrap();
        }
        let db = CommandHistoryDatabase::open(&path).unwrap();
        assert_eq!(db.count(), 2);
    }
}
