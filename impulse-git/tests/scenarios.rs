//! Repository states that trip up git tooling (renames, binaries, conflicts
//! mid-rebase, worktrees, submodules, detached and unborn HEAD, ignored
//! files, CRLF, Unicode paths), each checked against `git status --porcelain
//! -z` as the oracle (`GitScenarioTests.swift`).

mod common;

use std::collections::BTreeSet;

use common::{environment, run_git_raw, setup, RemoveOnDrop, TempRepo};
use impulse_git::{snapshot, ChangeStatus, RepoOperation, RepoSnapshot};

/// What `git status` says, by section.
#[derive(Debug, Default, PartialEq, Eq)]
struct Oracle {
    staged: BTreeSet<String>,
    unstaged: BTreeSet<String>,
    untracked: BTreeSet<String>,
    conflicted: BTreeSet<String>,
}

fn oracle(root: &str) -> Oracle {
    let output = run_git_raw(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        None,
        &environment(),
    )
    .unwrap_or_default();
    let fields: Vec<&str> = output.split('\0').collect();
    let conflict_codes = ["DD", "AU", "UD", "UA", "DU", "AA", "UU"];
    let mut result = Oracle::default();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        index += 1;
        if field.len() <= 3 {
            continue;
        }
        let code = &field[..2];
        let path = field[3..].to_string();
        let (x, y) = (code.as_bytes()[0], code.as_bytes()[1]);
        // The old path follows a rename or copy.
        if x == b'R' || x == b'C' {
            index += 1;
        }
        if code == "??" {
            result.untracked.insert(path);
        } else if conflict_codes.contains(&code) {
            result.conflicted.insert(path);
        } else {
            if x != b' ' {
                result.staged.insert(path.clone());
            }
            if y != b' ' {
                result.unstaged.insert(path);
            }
        }
    }
    result
}

fn sections(snap: &RepoSnapshot) -> Oracle {
    let paths =
        |changes: &[impulse_git::FileChange]| changes.iter().map(|c| c.path.clone()).collect();
    Oracle {
        staged: paths(&snap.staged),
        unstaged: paths(&snap.unstaged),
        untracked: paths(&snap.untracked),
        conflicted: paths(&snap.conflicted),
    }
}

fn expect_matches_git(root: &str) -> RepoSnapshot {
    let snap = snapshot(root, true).unwrap();
    assert_eq!(sections(&snap), oracle(root));
    snap
}

fn paths(changes: &[impulse_git::FileChange]) -> Vec<&str> {
    changes.iter().map(|c| c.path.as_str()).collect()
}

// MARK: Index and working tree

#[test]
fn staged_only() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]).unwrap();
    let snap = expect_matches_git(&repo.root);
    assert!(snap.unstaged.is_empty());
    assert_eq!(
        (snap.staged[0].added, snap.staged[0].removed),
        (Some(1), Some(1))
    );
}

#[test]
fn staged_and_unstaged_in_one_file() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n2\n3\n")], "commit");
    repo.write("a.txt", "1\nTWO\n3\n");
    repo.git(&["add", "a.txt"]).unwrap();
    repo.write("a.txt", "1\nTWO\n3\n4\n");
    let snap = expect_matches_git(&repo.root);
    assert_eq!(paths(&snap.staged), ["a.txt"]);
    assert_eq!(paths(&snap.unstaged), ["a.txt"]);
    assert_eq!(
        (snap.unstaged[0].added, snap.unstaged[0].removed),
        (Some(1), Some(0))
    );
}

#[test]
fn rename_plus_edit() {
    setup();
    let repo = TempRepo::create();
    let body: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    repo.commit(&[("old.txt", &body)], "commit");
    repo.git(&["mv", "old.txt", "new.txt"]).unwrap();
    repo.write("new.txt", &format!("{body}one more\n"));
    repo.git(&["add", "new.txt"]).unwrap();
    let snap = expect_matches_git(&repo.root);
    let renamed = &snap.staged[0];
    assert_eq!(renamed.status, ChangeStatus::Renamed);
    assert_eq!(renamed.path, "new.txt");
    assert_eq!(renamed.old_path.as_deref(), Some("old.txt"));
    assert_eq!((renamed.added, renamed.removed), (Some(1), Some(0)));
}

#[test]
fn binary_and_too_large() {
    setup();
    let repo = TempRepo::create();
    repo.write_bytes("image.bin", &[0x89, 0x50, 0x4E, 0x47, 0x00, 0x01, 0x02]);
    repo.write("big.txt", &"x\n".repeat(10));
    repo.git(&["add", "-A"]).unwrap();
    repo.git(&["commit", "-q", "-m", "files"]).unwrap();
    repo.write_bytes("image.bin", &[0x89, 0x50, 0x4E, 0x47, 0x00, 0x09]);
    // Over the 1 MB diff limit: listed, without line counts.
    repo.write("big.txt", &"y\n".repeat(600_000));
    let snap = expect_matches_git(&repo.root);
    let binary = snap
        .unstaged
        .iter()
        .find(|c| c.path == "image.bin")
        .unwrap();
    assert!(binary.is_binary && binary.added.is_none());
    let big = snap.unstaged.iter().find(|c| c.path == "big.txt").unwrap();
    assert!(big.added.is_none() && big.removed.is_none());
}

#[test]
fn ignored_files_stay_out() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[(".gitignore", "build/\n*.log\n")], "commit");
    repo.write("build/out.o", "obj");
    repo.write("debug.log", "log");
    repo.write("src/keep.swift", "let x = 1\n");
    let snap = expect_matches_git(&repo.root);
    assert_eq!(paths(&snap.untracked), ["src/keep.swift"]);
}

#[test]
fn crlf_files() {
    setup();
    let repo = TempRepo::create();
    repo.git(&["config", "core.autocrlf", "false"]).unwrap();
    repo.commit(&[("dos.txt", "one\r\ntwo\r\nthree\r\n")], "commit");
    repo.write("dos.txt", "one\r\nTWO\r\nthree\r\n");
    let snap = expect_matches_git(&repo.root);
    assert_eq!(
        (snap.unstaged[0].added, snap.unstaged[0].removed),
        (Some(1), Some(1))
    );

    // With autocrlf on, a CRLF working copy of an LF blob differs only in
    // line endings.
    let converted = TempRepo::create();
    converted.git(&["config", "core.autocrlf", "true"]).unwrap();
    converted.commit(&[("unix.txt", "a\nb\n")], "commit");
    converted.write("unix.txt", "a\r\nb\r\n");
    // git lists it as modified (the file was rewritten) with no changed
    // lines.
    let rewritten = expect_matches_git(&converted.root);
    assert_eq!(
        (rewritten.unstaged[0].added, rewritten.unstaged[0].removed),
        (Some(0), Some(0))
    );
}

#[test]
fn unicode_and_spaces_in_paths() {
    setup();
    let repo = TempRepo::create();
    repo.commit(
        &[("naïve résumé.txt", "v1\n"), ("日本/メモ.md", "a\n")],
        "commit",
    );
    repo.write("naïve résumé.txt", "v2\n");
    repo.write("日本/メモ.md", "b\n");
    repo.write("new folder/ünïcode.txt", "x\n");
    let snap = expect_matches_git(&repo.root);
    assert_eq!(snap.unstaged.len(), 2);
    assert_eq!(paths(&snap.untracked), ["new folder/ünïcode.txt"]);
}

#[test]
fn cached_line_counts_follow_edits_and_staging() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n2\n3\n")], "commit");
    repo.write("a.txt", "1\n2\n3\n4\n");
    assert_eq!(expect_matches_git(&repo.root).unstaged[0].added, Some(1));
    // Same file again, more lines: the cached count mustn't stick.
    repo.write("a.txt", "1\n2\n3\n4\n5\n6\n");
    assert_eq!(expect_matches_git(&repo.root).unstaged[0].added, Some(3));
    repo.git(&["add", "a.txt"]).unwrap();
    repo.write("a.txt", "1\n2\n3\n4\n5\n6\n7\n");
    let snap = expect_matches_git(&repo.root);
    assert_eq!(snap.staged[0].added, Some(3));
    assert_eq!(
        snap.unstaged[0].added,
        Some(1),
        "now against the new index blob"
    );
}

// MARK: HEAD states

#[test]
fn detached_head() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    repo.commit(&[("a.txt", "2\n")], "commit");
    let first = repo.git(&["rev-parse", "HEAD~1"]).unwrap();
    repo.git(&["checkout", "-q", "--detach", &first]).unwrap();
    repo.write("a.txt", "changed\n");
    let snap = expect_matches_git(&repo.root);
    assert!(snap.is_detached && snap.branch.is_none());
    assert_eq!(snap.head_oid.as_deref(), Some(first.as_str()));
}

#[test]
fn unborn_head_with_staged_files() {
    setup();
    let repo = TempRepo::create();
    repo.write("first.txt", "hello\n");
    repo.git(&["add", "first.txt"]).unwrap();
    repo.write("second.txt", "later\n");
    let snap = expect_matches_git(&repo.root);
    assert!(snap.is_unborn && snap.head_oid.is_none());
    assert_eq!(snap.staged[0].status, ChangeStatus::Added);
}

// MARK: Operations

#[test]
fn merge_conflict() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "base\n"), ("b.txt", "b\n")], "commit");
    repo.git(&["switch", "-q", "-c", "other"]).unwrap();
    repo.commit(
        &[("a.txt", "other\n"), ("c.txt", "new on other\n")],
        "commit",
    );
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("a.txt", "main\n")], "commit");
    let _ = repo.git(&["merge", "-q", "other"]);
    let snap = expect_matches_git(&repo.root);
    assert_eq!(snap.operation, Some(RepoOperation::Merge));
    assert_eq!(paths(&snap.conflicted), ["a.txt"]);
    assert_eq!(
        paths(&snap.staged),
        ["c.txt"],
        "the clean part of the merge is staged"
    );
}

#[test]
fn rebase_stopped_at_the_second_of_three() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "base\n")], "commit");
    repo.git(&["switch", "-q", "-c", "topic"]).unwrap();
    repo.commit(&[("one.txt", "1\n")], "one");
    repo.commit(&[("a.txt", "topic\n")], "two");
    repo.commit(&[("three.txt", "3\n")], "three");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("a.txt", "main\n")], "main");
    repo.git(&["switch", "-q", "topic"]).unwrap();
    let _ = repo.git(&["rebase", "main"]);
    let snap = expect_matches_git(&repo.root);
    assert_eq!(
        snap.operation,
        Some(RepoOperation::Rebase {
            step: Some(2),
            total: Some(3)
        })
    );
    assert_eq!(paths(&snap.conflicted), ["a.txt"]);
    assert!(snap.is_detached, "HEAD is detached during a rebase");
}

// MARK: Worktrees and submodules

#[test]
fn linked_worktree() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    let worktree = format!("{}-wt", repo.root);
    let _cleanup = RemoveOnDrop(worktree.clone());
    repo.git(&["worktree", "add", "-q", "-b", "feature", &worktree])
        .unwrap();
    std::fs::write(format!("{worktree}/a.txt"), "changed\n").unwrap();
    std::fs::write(format!("{worktree}/b.txt"), "new\n").unwrap();

    let snap = expect_matches_git(&worktree);
    assert_eq!(snap.root, worktree);
    assert_eq!(snap.branch.as_deref(), Some("feature"));
    assert_eq!(snap.common_dir, format!("{}/.git", repo.root));
    // The main checkout is untouched.
    assert!(!expect_matches_git(&repo.root).has_changes());
}

#[test]
fn submodule_with_new_commits() {
    setup();
    let library = TempRepo::create();
    library.commit(&[("lib.txt", "v1\n")], "commit");
    let app = TempRepo::create();
    app.commit(&[("app.txt", "app\n")], "commit");
    app.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        &library.root,
        "vendor/lib",
    ])
    .unwrap();
    app.git(&["commit", "-q", "-m", "add submodule"]).unwrap();

    // A new commit inside the submodule's checkout.
    let submodule = TempRepo::at(format!("{}/vendor/lib", app.root));
    submodule
        .git(&["commit", "-q", "--allow-empty", "-m", "bump"])
        .unwrap();

    let snap = expect_matches_git(&app.root);
    assert_eq!(paths(&snap.unstaged), ["vendor/lib"]);
}
