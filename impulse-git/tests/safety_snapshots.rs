//! Safety snapshots: recording, listing, restoring and pruning
//! (`SafetySnapshotTests` in `GitCoreTests.swift`).

mod common;

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{setup, TempRepo};
use impulse_git::operations;
use impulse_git::safety::{self, CHECKPOINT_PREFIX, DEFAULT_MAX_AGE, OPLOG_PREFIX};

#[test]
fn restoring_an_untracked_file_succeeds() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    repo.write("new.txt", "draft\n");
    let snapshot = safety::create("discard", &repo.root, OPLOG_PREFIX).unwrap();
    repo.remove("new.txt");
    // The file is in neither index; the index step mustn't fail the restore.
    safety::restore(&snapshot, &["new.txt"], &repo.root).unwrap();
    assert_eq!(repo.read("new.txt"), "draft\n");
    assert_eq!(repo.git(&["status", "--porcelain"]).unwrap(), "?? new.txt");
}

#[test]
fn snapshots_leave_the_real_index_unlocked() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]).unwrap();
    // Someone else's git command holds the index lock.
    let lock = repo.path(".git/index.lock");
    std::fs::write(&lock, "").unwrap();
    let snapshot = safety::create("turn start", &repo.root, OPLOG_PREFIX).unwrap();
    std::fs::remove_file(&lock).unwrap();
    let staged = repo.git(&["write-tree"]).unwrap();
    assert_eq!(
        snapshot.index_tree,
        Some(staged),
        "the index was still recorded"
    );
}

fn seconds_apart(a: SystemTime, b: SystemTime) -> f64 {
    match a.duration_since(b) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => e.duration().as_secs_f64(),
    }
}

#[test]
fn nested_checkpoints_list_and_survive_pruning() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    let prefix = format!("{CHECKPOINT_PREFIX}TERMINAL-UUID-1234/");
    let start = safety::create("turn start", &repo.root, &prefix).unwrap();
    // Refs are named by the millisecond.
    std::thread::sleep(Duration::from_millis(5));
    repo.write("a.txt", "two\n");
    let end = safety::create("turn end", &repo.root, &prefix).unwrap();

    let listed = safety::list(&repo.root, CHECKPOINT_PREFIX);
    let refs: Vec<&str> = listed.iter().map(|s| s.ref_name.as_str()).collect();
    assert_eq!(refs, [end.ref_name.as_str(), start.ref_name.as_str()]);
    assert!(seconds_apart(listed[1].date, start.date) < 1.0);
    // Fresh checkpoints aren't mistaken for ancient ones.
    assert!(safety::prune(&repo.root, CHECKPOINT_PREFIX, 200, DEFAULT_MAX_AGE).is_empty());
    assert_eq!(safety::list(&repo.root, CHECKPOINT_PREFIX).len(), 2);
}

#[test]
fn pruning_deletes_old_and_excess_snapshots_in_one_batch() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    let head = repo.head();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let day = 24 * 3600 * 1000;
    // 30 ancient snapshots, 12 recent ones, and one outside the prefix.
    let old: Vec<String> = (0..30)
        .map(|i| format!("{OPLOG_PREFIX}{}-discard", now - 20 * day - i))
        .collect();
    let recent: Vec<String> = (0..12)
        .map(|i| format!("{OPLOG_PREFIX}{}-discard", now - i * 1000))
        .collect();
    let other = format!("{CHECKPOINT_PREFIX}{}-turn-start", now - 20 * day);
    let commands: String = old
        .iter()
        .chain(&recent)
        .chain(std::iter::once(&other))
        .map(|r| format!("create {r} {head}\n"))
        .collect();
    repo.git_stdin(&["update-ref", "--stdin"], &commands)
        .unwrap();
    repo.git(&["pack-refs", "--all"]).unwrap();

    let deleted: HashSet<String> = safety::prune(&repo.root, OPLOG_PREFIX, 10, DEFAULT_MAX_AGE)
        .into_iter()
        .collect();
    let expected: HashSet<String> = old.iter().chain(&recent[10..]).cloned().collect();
    assert_eq!(
        deleted, expected,
        "the old ones, and the recent ones past the newest 10"
    );
    let left: HashSet<String> = repo
        .git(&["for-each-ref", "--format=%(refname)", "refs/impulse/"])
        .unwrap()
        .split('\n')
        .map(str::to_string)
        .collect();
    let kept: HashSet<String> = recent[..10]
        .iter()
        .chain(std::iter::once(&other))
        .cloned()
        .collect();
    assert_eq!(left, kept);
    assert!(safety::prune(&repo.root, OPLOG_PREFIX, 10, DEFAULT_MAX_AGE).is_empty());
}

#[test]
fn snapshot_restores_discarded_and_untracked_work() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "committed\n")], "commit");
    repo.write("a.txt", "staged edit\n");
    repo.git(&["add", "a.txt"]).unwrap();
    repo.write("a.txt", "worktree edit\n");
    repo.write("notes.md", "untracked notes\n");
    let index_before = repo.git(&["diff", "--cached"]).unwrap();

    let snapshot = safety::create("Discard everything", &repo.root, OPLOG_PREFIX).unwrap();
    // Taking a snapshot leaves the real index alone.
    assert_eq!(repo.git(&["diff", "--cached"]).unwrap(), index_before);
    assert!(repo
        .git(&["status", "--porcelain"])
        .unwrap()
        .contains("?? notes.md"));

    // Destroy the work.
    repo.git(&["reset", "-q", "--hard"]).unwrap();
    repo.remove("notes.md");
    assert_eq!(repo.read("a.txt"), "committed\n");

    safety::restore(&snapshot, &[] as &[&str], &repo.root).unwrap();
    assert_eq!(repo.read("a.txt"), "worktree edit\n");
    assert_eq!(repo.read("notes.md"), "untracked notes\n");
    assert_eq!(repo.git(&["diff", "--cached"]).unwrap(), index_before);

    let listed = safety::list(&repo.root, OPLOG_PREFIX);
    assert_eq!(listed[0].ref_name, snapshot.ref_name);
    assert_eq!(listed[0].reason, "Discard everything");
    assert_eq!(listed[0].index_tree, snapshot.index_tree);
    // Snapshot refs aren't branches.
    assert_eq!(operations::branches(&repo.root).0, ["main"]);

    safety::prune(&repo.root, OPLOG_PREFIX, 0, DEFAULT_MAX_AGE);
    assert!(safety::list(&repo.root, OPLOG_PREFIX).is_empty());
}

#[test]
fn snapshot_works_without_any_commits() {
    setup();
    let repo = TempRepo::create();
    repo.write("first.txt", "hello\n");
    let snapshot = safety::create("before", &repo.root, OPLOG_PREFIX).unwrap();
    repo.remove("first.txt");
    safety::restore(&snapshot, &[] as &[&str], &repo.root).unwrap();
    assert_eq!(repo.read("first.txt"), "hello\n");
}
