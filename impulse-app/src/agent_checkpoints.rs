//! Snapshots of the repository around each agent turn (the Swift
//! `AgentCheckpoints`), so "what did the agent just do?" is one review
//! away (and undoable). A turn's start is recorded when the agent starts
//! working and its end when it stops, under
//! `refs/impulse/checkpoints/<terminal>/`: private refs a normal push never
//! sends, pruned like the safety oplog.
//!
//! Snapshots are taken one after another on a worker thread (the Swift
//! serial queue), so a turn that ends before its start snapshot is taken
//! still gets its end.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Sender};
use std::sync::{LazyLock, Mutex, MutexGuard};

use impulse_git::safety::{self, SafetySnapshot};
use impulse_git::types::DiffScope;
use impulse_protocol::agent_tools::{Turn as ListedTurn, TurnList};

#[derive(Clone, Debug)]
pub struct Turn {
    id: u64,
    pub agent_name: String,
    pub repo_root: String,
    pub start: SafetySnapshot,
    pub end: Option<SafetySnapshot>,
}

impl Turn {
    /// The review scope for this turn (to the working tree while it runs).
    pub fn scope(&self) -> DiffScope {
        DiffScope::Snapshot {
            from: self.start.ref_name.clone(),
            to: self.end.as_ref().map(|end| end.ref_name.clone()),
        }
    }

    /// As `impulse turns` and `list_turns` show it.
    pub fn listed(&self) -> ListedTurn {
        ListedTurn {
            agent: self.agent_name.clone(),
            started: self.start.date,
            ended: self.end.as_ref().map(|end| end.date),
            start: self.start.ref_name.clone(),
            end: self.end.as_ref().map(|end| end.ref_name.clone()),
        }
    }
}

#[derive(Default)]
struct State {
    /// Turns per terminal, oldest first.
    turns: HashMap<String, Vec<Turn>>,
    /// The turn each terminal's agent is working on.
    open: HashMap<String, u64>,
    /// Each turn's repository, once its start snapshot is taken.
    roots: HashMap<u64, String>,
    pruned: HashSet<String>,
    next_id: u64,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

type Job = Box<dyn FnOnce() + Send>;

/// The worker that takes the snapshots, in order.
static QUEUE: LazyLock<Mutex<Sender<Job>>> = LazyLock::new(|| {
    let (sender, receiver) = channel::<Job>();
    std::thread::Builder::new()
        .name("impulse-checkpoints".into())
        .spawn(move || {
            while let Ok(job) = receiver.recv() {
                job();
            }
        })
        .expect("checkpoint thread");
    Mutex::new(sender)
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn enqueue(job: impl FnOnce() + Send + 'static) {
    let queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    let _ = queue.send(Box::new(job));
}

fn prefix(terminal: &str) -> String {
    format!("{}{terminal}/", safety::CHECKPOINT_PREFIX)
}

/// The agent in `terminal` started working in `cwd`.
pub fn turn_started(terminal: &str, agent_name: &str, cwd: &str) {
    if cwd.is_empty() {
        return;
    }
    let id = {
        let mut state = state();
        state.next_id += 1;
        let id = state.next_id;
        state.open.insert(terminal.to_string(), id);
        id
    };
    let (terminal, agent_name, cwd) = (
        terminal.to_string(),
        agent_name.to_string(),
        cwd.to_string(),
    );
    enqueue(move || {
        let Some(root) = impulse_git::branches::repo_root(&cwd) else {
            return;
        };
        let root = root.trim_end_matches('/').to_string();
        let Ok(snapshot) = safety::create("turn start", &root, &prefix(&terminal)) else {
            return;
        };
        let first = {
            let mut state = state();
            state.roots.insert(id, root.clone());
            state.turns.entry(terminal).or_default().push(Turn {
                id,
                agent_name,
                repo_root: root.clone(),
                start: snapshot,
                end: None,
            });
            state.pruned.insert(root.clone())
        };
        if first {
            prune_now(&root);
        }
    });
}

/// The agent stopped (finished, or waits for the user).
pub fn turn_ended(terminal: &str) {
    let Some(id) = state().open.remove(terminal) else {
        return;
    };
    let terminal = terminal.to_string();
    enqueue(move || {
        // No root: the start snapshot failed or wasn't in a repository.
        let Some(root) = state().roots.remove(&id) else {
            return;
        };
        let Ok(snapshot) = safety::create("turn end", &root, &prefix(&terminal)) else {
            return;
        };
        let mut state = state();
        if let Some(turn) = state
            .turns
            .get_mut(&terminal)
            .and_then(|turns| turns.iter_mut().rev().find(|t| t.id == id))
        {
            turn.end = Some(snapshot);
        }
    });
}

/// Every recorded turn in a terminal, oldest first.
pub fn turns(terminal: &str) -> Vec<Turn> {
    state().turns.get(terminal).cloned().unwrap_or_default()
}

pub fn has_turns(terminal: &str) -> bool {
    state().turns.get(terminal).is_some_and(|t| !t.is_empty())
}

pub fn last_turn(terminal: &str) -> Option<Turn> {
    state().turns.get(terminal).and_then(|t| t.last().cloned())
}

/// The most recent turn in a repository, by any agent.
pub fn last_turn_in_repo(root: &str) -> Option<Turn> {
    let root = root.trim_end_matches('/');
    state()
        .turns
        .values()
        .filter_map(|turns| turns.iter().rev().find(|t| t.repo_root == root))
        .max_by_key(|t| t.start.date)
        .cloned()
}

/// `impulse turns` and `list_turns`: the latest `limit` of a terminal's
/// turns.
pub fn turn_list(terminal: &str, limit: usize) -> TurnList {
    TurnList::latest(turns(terminal).iter().map(Turn::listed).collect(), limit)
}

/// Prune the checkpoints of `root`'s repository, off the main thread.
pub fn prune(root: &str) {
    let root = root.to_string();
    enqueue(move || prune_now(&root));
}

/// Turns whose snapshots went are forgotten, so a review never asks for a
/// deleted ref. By ref name alone: a repository's worktrees share its refs,
/// and the names are unique.
fn prune_now(root: &str) {
    let deleted: HashSet<String> = safety::prune(
        root,
        safety::CHECKPOINT_PREFIX,
        200,
        safety::DEFAULT_MAX_AGE,
    )
    .into_iter()
    .collect();
    if deleted.is_empty() {
        return;
    }
    for turns in state().turns.values_mut() {
        turns.retain(|turn| {
            !deleted.contains(&turn.start.ref_name)
                && !turn
                    .end
                    .as_ref()
                    .is_some_and(|end| deleted.contains(&end.ref_name))
        });
    }
}

/// Wait for the snapshots queued so far (tests).
#[cfg(test)]
fn drain() {
    let (sender, receiver) = channel();
    enqueue(move || {
        let _ = sender.send(());
    });
    let _ = receiver.recv();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .expect("git");
        assert!(status.success());
    }

    #[test]
    fn a_turn_records_its_start_and_end() {
        let dir = std::env::temp_dir().join(format!("impulse-turns-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "first"]);
        let cwd = dir.to_string_lossy().into_owned();

        turn_started("T1", "Claude Code", &cwd);
        std::fs::write(dir.join("a.txt"), "two\n").unwrap();
        turn_ended("T1");
        drain();

        let turns = turns("T1");
        assert_eq!(turns.len(), 1);
        let turn = &turns[0];
        assert_eq!(turn.agent_name, "Claude Code");
        assert!(turn
            .start
            .ref_name
            .starts_with("refs/impulse/checkpoints/T1/"));
        assert!(turn.end.is_some());
        assert!(last_turn_in_repo(&turn.repo_root).is_some());
        let list = turn_list("T1", 10);
        assert_eq!(list.turns.len(), 1);
        assert_eq!(list.turns[0].start, turn.start.ref_name);

        // Ended with no turn open: nothing changes.
        turn_ended("T1");
        drain();
        assert_eq!(super::turns("T1").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outside_a_repository_no_turn_is_recorded() {
        let dir = std::env::temp_dir().join(format!("impulse-noturns-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        turn_started("T2", "Codex", &dir.to_string_lossy());
        turn_ended("T2");
        drain();
        assert!(!has_turns("T2"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
