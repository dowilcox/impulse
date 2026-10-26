//! What a change on disk means for a checkout, and the watcher end to end
//! (`RepoWatcherTests` in `GitCoreTests.swift`).

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{setup, TempRepo};
use impulse_git::snapshot;
use impulse_git::watcher::{Change, IgnoreFilter, RepoWatcher};

#[test]
fn classifies_git_directory_paths() {
    let classify = RepoWatcher::classify_git_path;
    assert_eq!(classify("index"), Change::INDEX);
    assert_eq!(classify("HEAD"), Change::REFS);
    assert_eq!(classify("refs/heads/main"), Change::REFS);
    assert_eq!(classify("refs/tags/v1.0"), Change::REFS);
    assert_eq!(classify("packed-refs"), Change::REFS);
    assert_eq!(classify("refs/stash"), Change::REFS);
    assert_eq!(classify("MERGE_HEAD"), Change::OPERATION);
    assert_eq!(classify("rebase-merge/msgnum"), Change::OPERATION);
    assert_eq!(classify("objects/ab/cdef"), Change::NONE);
    assert_eq!(classify("index.lock"), Change::NONE);
    assert_eq!(classify("info/exclude"), Change::WORKING_TREE);
    // Linked worktrees' own index and HEAD are theirs, not the main
    // checkout's.
    assert_eq!(classify("worktrees/feature/index"), Change::NONE);
    assert_eq!(classify("worktrees/feature/HEAD"), Change::NONE);
}

#[test]
fn a_linked_worktree_only_shares_the_refs() {
    let classify = |path: &str| {
        RepoWatcher::classify(
            path,
            "/r.worktrees/task",
            "/r/.git/worktrees/task",
            "/r/.git",
            &mut |_| false,
        )
    };
    assert_eq!(classify("/r/.git/worktrees/task/index"), Change::INDEX);
    assert_eq!(classify("/r/.git/worktrees/task/HEAD"), Change::REFS);
    assert_eq!(classify("/r/.git/worktrees/task/FETCH_HEAD"), Change::REFS);
    assert_eq!(
        classify("/r/.git/worktrees/task/MERGE_HEAD"),
        Change::OPERATION
    );
    assert_eq!(classify("/r/.git/refs/heads/main"), Change::REFS);
    assert_eq!(classify("/r/.git/refs/remotes/origin/main"), Change::REFS);
    assert_eq!(classify("/r/.git/packed-refs"), Change::REFS);
    assert_eq!(classify("/r/.git/info/exclude"), Change::WORKING_TREE);
    // The main checkout's own state, and another task's.
    for path in [
        "/r/.git/index",
        "/r/.git/HEAD",
        "/r/.git/FETCH_HEAD",
        "/r/.git/MERGE_HEAD",
        "/r/.git/rebase-merge/msgnum",
        "/r/.git/worktrees/other/index",
        "/r/.git/worktrees/other/HEAD",
        "/r/.git/objects/ab/cdef",
        "/r/.git/refs/heads/main.lock",
        "/r.worktrees/task/.git",
        "/r.worktrees/other/a.swift",
    ] {
        assert_eq!(classify(path), Change::NONE, "{path}");
    }
    assert_eq!(
        classify("/r.worktrees/task/src/a.swift"),
        Change::WORKING_TREE
    );
}

#[test]
fn the_main_checkout_skips_its_tasks_own_files() {
    let classify =
        |path: &str| RepoWatcher::classify(path, "/r", "/r/.git", "/r/.git", &mut |_| false);
    assert_eq!(classify("/r/.git/index"), Change::INDEX);
    assert_eq!(classify("/r/.git/HEAD"), Change::REFS);
    assert_eq!(classify("/r/.git/refs/heads/task"), Change::REFS);
    assert_eq!(classify("/r/.git/worktrees/task/index"), Change::NONE);
    assert_eq!(classify("/r/.git/worktrees/task/HEAD"), Change::NONE);
    assert_eq!(classify("/r/.git/worktrees/task"), Change::NONE);
    assert_eq!(classify("/r/src/a.swift"), Change::WORKING_TREE);
    assert_eq!(classify("/r"), Change::WORKING_TREE);
}

#[test]
fn ignored_folders_are_checked_from_the_top() {
    let ignored: HashSet<&str> = ["node_modules/", "debug.log", "secret/.gitignore"].into();
    let mut asked: Vec<String> = Vec::new();
    let classify_asking = |relative: &str, asked: &mut Vec<String>| {
        RepoWatcher::classify_working_tree_path(relative, &mut |path| {
            asked.push(path.to_string());
            ignored.contains(path)
        })
    };
    assert_eq!(
        classify_asking("node_modules/react/cjs/index.js", &mut asked),
        Change::NONE
    );
    assert_eq!(asked, ["node_modules/"]);
    let mut classify = |relative: &str| classify_asking(relative, &mut asked);
    assert_eq!(classify("debug.log"), Change::NONE);
    assert_eq!(classify("src/app/main.swift"), Change::WORKING_TREE);
    assert_eq!(classify("node_modules/pkg/.gitignore"), Change::NONE);
    // Ignore rules count even where they match themselves.
    assert_eq!(classify("secret/.gitignore"), Change::WORKING_TREE);
    assert_eq!(classify(".gitignore"), Change::WORKING_TREE);
    assert!(RepoWatcher::changes_ignore_rules("/r/web/.gitignore"));
    assert!(RepoWatcher::changes_ignore_rules("/r/.git/info/exclude"));
    assert!(!RepoWatcher::changes_ignore_rules("/r/web/gitignore.md"));
}

#[test]
fn ignore_filter_matches_git_and_keeps_tracked_files() {
    setup();
    let repo = TempRepo::create();
    repo.commit(
        &[
            (".gitignore", "node_modules/\n*.log\nbuild/\n"),
            ("src/a.txt", "a\n"),
        ],
        "commit",
    );
    repo.write("build/keep.txt", "kept\n");
    repo.git(&["add", "-f", "build/keep.txt"]).unwrap();
    repo.git(&["commit", "-q", "-m", "keep"]).unwrap();
    repo.write("node_modules/x/y.js", "y\n");
    repo.write("build/other.o", "o\n");
    repo.write("debug.log", "d\n");

    let mut filter = IgnoreFilter::new(&repo.root);
    for path in [
        "node_modules/x/y.js",
        "debug.log",
        "src/a.txt",
        "build/keep.txt",
        "build/other.o",
    ] {
        let git_ignores = repo.git(&["check-ignore", "-q", path]).is_ok();
        assert_eq!(filter.is_ignored(path), git_ignores, "{path}");
    }
    assert!(filter.is_ignored("node_modules/"));
    assert!(!filter.is_ignored("src/"));
    // Git still follows the file it tracks in there.
    assert!(!filter.is_ignored("build/"));

    // A folder seen ignored still is once it's deleted.
    repo.remove("node_modules");
    assert!(filter.is_ignored("node_modules"));

    // New rules apply once the filter is told.
    assert!(!filter.is_ignored("tmp/"));
    repo.write(".gitignore", "node_modules/\n*.log\nbuild/\ntmp/\n");
    filter.invalidate();
    assert!(filter.is_ignored("tmp/"));
    // So does a newly tracked file.
    repo.write("tmp/tracked.txt", "t\n");
    repo.git(&["add", "-f", "tmp/tracked.txt"]).unwrap();
    filter.invalidate();
    assert!(!filter.is_ignored("tmp/"));
}

/// Changes a watcher reported, shared with its handler.
#[derive(Clone, Default)]
struct Received(Arc<Mutex<Change>>);

impl Received {
    fn value(&self) -> Change {
        *self.0.lock().unwrap()
    }

    fn reset(&self) {
        *self.0.lock().unwrap() = Change::NONE;
    }

    /// Wait up to four seconds for `change`.
    fn wait_for(&self, change: Change) {
        for _ in 0..40 {
            if self.value().contains(change) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn watch(root: &str, received: &Received) -> RepoWatcher {
    let snap = snapshot(root, true).unwrap();
    let sink = received.clone();
    let mut watcher = RepoWatcher::new(
        &snap.root,
        &snap.git_dir,
        &snap.common_dir,
        Duration::from_millis(50),
        move |change| *sink.0.lock().unwrap() |= change,
    );
    watcher.start().unwrap();
    watcher
}

#[test]
fn changes_in_ignored_folders_are_not_reported() {
    setup();
    let repo = TempRepo::create();
    repo.commit(
        &[(".gitignore", "node_modules/\n*.log\n"), ("a.txt", "1\n")],
        "commit",
    );
    let received = Received::default();
    let _watcher = watch(&repo.root, &received);
    std::thread::sleep(Duration::from_millis(300));

    for index in 0..20 {
        repo.write(&format!("node_modules/pkg{index}/index.js"), "x\n");
    }
    repo.write("build.log", "log\n");
    std::thread::sleep(Duration::from_millis(800));
    assert!(received.value().is_empty(), "{:?}", received.value());

    repo.write("a.txt", "2\n");
    received.wait_for(Change::WORKING_TREE);
    assert!(received.value().contains(Change::WORKING_TREE));
}

#[test]
fn checkouts_hear_only_their_own_index_but_every_commit() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    let task = TempRepo::at(format!("{}-task", repo.root));
    let _cleanup = common::RemoveOnDrop(task.root.clone());
    repo.git(&["worktree", "add", "-q", "-b", "task", &task.root])
        .unwrap();

    let main_changes = Received::default();
    let task_changes = Received::default();
    let _main_watcher = watch(&repo.root, &main_changes);
    let _task_watcher = watch(&task.root, &task_changes);
    std::thread::sleep(Duration::from_millis(300));

    // Staging in the task: the task hears it, the main checkout doesn't.
    task.write("b.txt", "b\n");
    task.git(&["add", "b.txt"]).unwrap();
    task_changes.wait_for(Change::INDEX);
    assert!(task_changes.value().contains(Change::INDEX));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        main_changes.value().is_empty(),
        "{:?}",
        main_changes.value()
    );

    // And the other way round.
    task_changes.reset();
    repo.write("c.txt", "c\n");
    repo.git(&["add", "c.txt"]).unwrap();
    main_changes.wait_for(Change::INDEX);
    assert!(main_changes.value().contains(Change::INDEX));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        task_changes.value().is_empty(),
        "{:?}",
        task_changes.value()
    );

    // A commit moves a branch, which every checkout hears.
    main_changes.reset();
    task.git(&["commit", "-q", "-m", "task work"]).unwrap();
    main_changes.wait_for(Change::REFS);
    task_changes.wait_for(Change::REFS);
    assert!(main_changes.value().contains(Change::REFS));
    assert!(task_changes.value().contains(Change::REFS));
}

#[test]
fn reports_working_tree_and_index_changes() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    let received = Received::default();
    let _watcher = watch(&repo.root, &received);
    std::thread::sleep(Duration::from_millis(300));

    repo.write("a.txt", "2\n");
    repo.git(&["add", "a.txt"]).unwrap();
    received.wait_for(Change::WORKING_TREE | Change::INDEX);
    assert!(received.value().contains(Change::WORKING_TREE));
    assert!(received.value().contains(Change::INDEX));
}
