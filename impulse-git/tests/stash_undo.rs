//! Undoing stash drops and pops, uncommitting, and reads that must tell
//! "failed" from "none" (`StashUndoTests` in `GitCoreTests.swift`).

mod common;

use common::{setup, temp_dir, RemoveOnDrop, TempRepo};
use impulse_git::operations::{self, OperationError, StashOptions};
use impulse_git::safety::{self, OPLOG_PREFIX};

fn stash(message: &str, root: &str) {
    operations::stash(
        &StashOptions {
            message: Some(message.into()),
            ..Default::default()
        },
        root,
    )
    .unwrap();
}

fn messages(root: &str) -> Vec<String> {
    operations::stash_list(root)
        .into_iter()
        .map(|e| e.message)
        .collect()
}

#[test]
fn dropped_stash_can_be_stored_back() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "first");
    repo.write("a.txt", "two\n");
    repo.write("fresh.txt", "new\n");
    stash("wip", &repo.root);

    let sha = operations::stash_commit(0, &repo.root).unwrap();
    assert_eq!(
        operations::stash_untracked_paths(&sha, &repo.root),
        ["fresh.txt"]
    );
    assert_eq!(
        operations::stash_paths(&sha, &repo.root),
        ["a.txt", "fresh.txt"]
    );
    operations::stash_drop(0, &repo.root).unwrap();
    assert!(operations::stash_list(&repo.root).is_empty());

    operations::stash_store(&sha, "On main: wip", &repo.root).unwrap();
    assert_eq!(messages(&repo.root), ["On main: wip"]);
    assert_eq!(operations::stash_commit(0, &repo.root), Some(sha));
    assert_eq!(operations::stash_commit(3, &repo.root), None);
}

#[test]
fn undoing_a_pop_restores_only_the_stashs_files() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n"), ("b.txt", "b\n")], "first");
    repo.write("a.txt", "stashed\n");
    repo.write("fresh.txt", "new\n");
    stash("wip", &repo.root);
    repo.write("a.txt", "before pop\n");
    stash("keep", &repo.root);
    // stash@{0} = keep, stash@{1} = wip; pop wip onto a clean tree.
    let sha = operations::stash_commit(1, &repo.root).unwrap();
    let snapshot = safety::create("pop stash", &repo.root, OPLOG_PREFIX).unwrap();
    operations::stash_apply(1, true, &repo.root).unwrap();
    assert_eq!(
        repo.git(&["show", ":a.txt"]).unwrap(),
        "one",
        "pop --index keeps the index as it was"
    );
    assert!(repo.exists("fresh.txt"));
    repo.write("b.txt", "edited after the pop\n");

    operations::undo_stash_pop(&sha, "On main: wip", &snapshot, &repo.root).unwrap();
    assert_eq!(repo.read("a.txt"), "one\n");
    assert!(!repo.exists("fresh.txt"));
    assert_eq!(
        repo.read("b.txt"),
        "edited after the pop\n",
        "files outside the stash are left alone"
    );
    assert_eq!(messages(&repo.root), ["On main: wip", "On main: keep"]);
}

#[test]
fn a_stash_is_found_by_its_commit_wherever_it_moved() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "first");
    repo.write("a.txt", "two\n");
    stash("picked", &repo.root);
    let picked = operations::stash_list(&repo.root).remove(0);
    assert_eq!(
        picked.commit,
        repo.git(&["rev-parse", "stash@{0}"]).unwrap()
    );
    // Another stash lands on top before the drop runs.
    repo.write("a.txt", "three\n");
    stash("newer", &repo.root);

    let found = operations::stash_entry(picked.index, Some(&picked.commit), &repo.root).unwrap();
    assert_eq!(found.index, 1);
    assert_eq!(found.message, "On main: picked");
    assert_eq!(
        operations::stash_entry(0, None, &repo.root)
            .map(|e| e.message)
            .as_deref(),
        Some("On main: newer")
    );
    operations::stash_drop(found.index, &repo.root).unwrap();
    assert_eq!(
        operations::stash_entry(0, Some(&picked.commit), &repo.root),
        None
    );
    assert_eq!(messages(&repo.root), ["On main: newer"]);
}

#[test]
fn uncommit_takes_back_only_the_commit_it_was_asked_for() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "first");
    repo.commit(&[("a.txt", "two\n")], "mine");
    let mine = repo.head();
    repo.commit(&[("b.txt", "agent\n")], "an agent's");
    let result = operations::uncommit(Some(&mine), &repo.root);
    assert!(
        matches!(result, Err(OperationError::Stale(_))),
        "{result:?}"
    );
    assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]).unwrap(), "3");
    let agents = repo.head();
    operations::uncommit(Some(&agents), &repo.root).unwrap();
    assert_eq!(repo.head(), mine);
}

#[test]
fn uncommit_does_what_reset_soft_does() {
    setup();
    // Two identical repositories: git's own reset in one, Impulse's in the
    // other.
    let make = || {
        let repo = TempRepo::create();
        repo.commit(&[("a.txt", "one\n"), ("b.txt", "b\n")], "first");
        repo.write("a.txt", "two\n");
        repo.write("c.txt", "new\n");
        repo.git(&["add", "-A"]).unwrap();
        repo.git(&["commit", "-q", "-m", "second"]).unwrap();
        repo.write("a.txt", "three, unstaged\n");
        repo.write("d.txt", "untracked\n");
        repo.write("b.txt", "staged\n");
        repo.git(&["add", "b.txt"]).unwrap();
        repo
    };
    let oracle = make();
    let ours = make();
    oracle.git(&["reset", "--soft", "HEAD~1"]).unwrap();
    operations::uncommit(Some(&ours.head()), &ours.root).unwrap();

    for args in [
        &["rev-parse", "HEAD"][..],
        &["rev-parse", "ORIG_HEAD"],
        &["symbolic-ref", "HEAD"],
        &["ls-files", "-s"],
        &["status", "--porcelain", "--untracked-files=all"],
        &["reflog", "-1", "--format=%gs", "main"],
        &["reflog", "-1", "--format=%gs", "HEAD"],
    ] {
        assert_eq!(
            ours.git(args).unwrap(),
            oracle.git(args).unwrap(),
            "git {}",
            args.join(" ")
        );
    }
    assert_eq!(ours.read("a.txt"), "three, unstaged\n");
}

#[test]
fn worktree_list_failing_isnt_an_empty_list() {
    setup();
    let dir = temp_dir("impulse-not-a-repo");
    let _cleanup = RemoveOnDrop(dir.clone());
    assert_eq!(operations::worktree_list(&dir), None);
    assert!(operations::worktrees(&dir).is_empty());
}

#[test]
fn changed_paths_keep_non_ascii_names() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "a\n")], "first");
    let before = repo.head();
    repo.commit(&[("café.txt", "c\n")], "second");
    assert_eq!(
        operations::changed_paths(&before, "HEAD", &repo.root),
        ["café.txt"]
    );
}
