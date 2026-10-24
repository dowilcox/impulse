//! Impulse's command history across every terminal and launch (the Swift
//! `CommandHistory`): finished commands go into the SQLite store
//! (`history.sqlite3` in the data directory, the same file and schema as the
//! Swift app's) with their folder, repository, branch, exit status and
//! duration. A list of recent distinct commands stays in memory for
//! per-keystroke lookups (ghost suggestions, Up and Down in the input bar).
//!
//! Each terminal core keeps its own session history; that stays the source
//! for "this session first" ordering, and this store adds everything else.
//! With "Keep command history" (`terminal_persistent_history`) off nothing
//! is read from or written to the store.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use gpui_kit::{App, AppContext as _, Global};
use impulse_kit::history::{CommandHistoryDatabase, HistoryEntry};

use crate::settings_store::SettingsStore;

/// How many distinct commands the in-memory list keeps.
const CACHE_SIZE: usize = 2000;

#[derive(Default)]
pub struct CommandHistory {
    cache: Vec<String>,
    /// Bumped whenever `cache` changes (a cheap check for callers' caches).
    revision: u64,
    loaded: bool,
    /// When the history was last cleared (milliseconds since 1970).
    pub cleared_at_ms: u64,
}

impl Global for CommandHistory {}

/// The store, opened on first use; `None` if it can't be opened.
fn database() -> Option<Arc<CommandHistoryDatabase>> {
    static DATABASE: OnceLock<Option<Arc<CommandHistoryDatabase>>> = OnceLock::new();
    DATABASE
        .get_or_init(|| {
            let path = impulse_kit::paths::data_dir(cfg!(debug_assertions)).join("history.sqlite3");
            CommandHistoryDatabase::open(path).map(Arc::new)
        })
        .clone()
}

fn enabled(cx: &App) -> bool {
    SettingsStore::get(cx).terminal_persistent_history
}

impl CommandHistory {
    fn global_mut(cx: &mut App) -> &mut CommandHistory {
        if !cx.has_global::<CommandHistory>() {
            cx.set_global(CommandHistory::default());
        }
        cx.global_mut::<CommandHistory>()
    }

    /// Distinct commands, newest first; empty while history is off.
    pub fn recent(cx: &mut App) -> Vec<String> {
        if !enabled(cx) {
            return Vec::new();
        }
        Self::load_if_needed(cx);
        Self::global_mut(cx).cache.clone()
    }

    /// Changes whenever `recent` would answer differently.
    pub fn revision(cx: &mut App) -> u64 {
        if !enabled(cx) {
            return u64::MAX;
        }
        Self::global_mut(cx).revision
    }

    pub fn cleared_at_ms(cx: &mut App) -> u64 {
        Self::global_mut(cx).cleared_at_ms
    }

    /// Start reading the store (a terminal opened), so the first
    /// keystrokes already have it.
    pub fn preload(cx: &mut App) {
        if enabled(cx) {
            Self::load_if_needed(cx);
        }
    }

    /// Read the newest commands into the list, the first time they're
    /// needed with history on.
    fn load_if_needed(cx: &mut App) {
        let history = Self::global_mut(cx);
        if history.loaded {
            return;
        }
        history.loaded = true;
        let task = cx.background_spawn(async move {
            let database = database()?;
            database.prune(CommandHistoryDatabase::ROW_LIMIT);
            Some(
                database
                    .recent(None, &Default::default(), CACHE_SIZE)
                    .into_iter()
                    .map(|hit| hit.entry.command)
                    .collect::<Vec<_>>(),
            )
        });
        cx.spawn(async move |cx| {
            if let Some(recent) = task.await {
                cx.update(|cx| Self::global_mut(cx).merge(recent, false));
            }
        })
        .detach();
    }

    /// Store a finished command. Its repository and branch are looked up in
    /// the background.
    pub fn record(
        command: &str,
        cwd: Option<String>,
        exit_code: Option<i32>,
        duration_ms: Option<i64>,
        session: Option<String>,
        cx: &mut App,
    ) {
        if !enabled(cx) || !HistoryEntry::should_record(command) {
            return;
        }
        Self::load_if_needed(cx);
        Self::global_mut(cx).merge(vec![command.to_string()], true);
        let started_at =
            impulse_kit::history::database::now() - duration_ms.unwrap_or(0) as f64 / 1000.0;
        let command = command.to_string();
        cx.background_spawn(async move {
            let Some(database) = database() else { return };
            let repo = cwd.as_deref().and_then(git_root);
            let branch = repo.as_deref().and_then(git_branch);
            database.insert(&[HistoryEntry {
                command,
                cwd,
                repo,
                branch,
                exit_code,
                duration_ms,
                started_at,
                session,
            }]);
        })
        .detach();
    }

    fn merge(&mut self, commands: Vec<String>, at_front: bool) {
        if at_front {
            let incoming: HashSet<&String> = commands.iter().collect();
            let rest: Vec<String> = self
                .cache
                .iter()
                .filter(|c| !incoming.contains(c))
                .cloned()
                .collect();
            self.cache = commands.into_iter().chain(rest).collect();
        } else {
            let existing: HashSet<String> = self.cache.iter().cloned().collect();
            self.cache
                .extend(commands.into_iter().filter(|c| !existing.contains(c)));
        }
        self.cache.truncate(CACHE_SIZE);
        self.revision += 1;
    }
}

fn git(dir: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-c", "log.showSignature=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn git_root(dir: &str) -> Option<String> {
    git(dir, &["rev-parse", "--show-toplevel"])
}

fn git_branch(root: &str) -> Option<String> {
    git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).filter(|b| b != "HEAD")
}
