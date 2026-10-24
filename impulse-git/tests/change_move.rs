//! Moving uncommitted work into another checkout on the same commit (Move
//! Changes to New Task). The git CLI is the oracle (`ChangeMoveTests.swift`).

mod common;

use common::{setup, RemoveOnDrop, TempRepo};
use impulse_git::change_move::{self, edited_since, move_changes, remove_file, Moved};
use impulse_git::safety::{self, OPLOG_PREFIX};

/// A repository with every kind of uncommitted change, and a second worktree
/// on the same commit.
fn set_up() -> (TempRepo, String, RemoveOnDrop) {
    setup();
    let repo = TempRepo::create();
    repo.commit(
        &[
            ("a.txt", "a\n"),
            ("b.txt", "b\n"),
            ("c.txt", "c\n"),
            ("old.txt", "moving\n"),
            (".gitignore", ".env\n"),
        ],
        "first",
    );
    repo.write("a.txt", "a changed\n");
    repo.write("b.txt", "b staged\n");
    repo.git(&["add", "b.txt"]).unwrap();
    repo.git(&["rm", "-q", "c.txt"]).unwrap();
    repo.git(&["mv", "old.txt", "new.txt"]).unwrap();
    repo.write("src/d.txt", "new and untracked\n");
    repo.write("e.txt", "new and staged\n");
    repo.git(&["add", "e.txt"]).unwrap();
    repo.write(".env", "SECRET=1\n");
    let target = format!("{}-task", repo.root);
    repo.git(&["worktree", "add", "-q", "--detach", &target, "HEAD"])
        .unwrap();
    let cleanup = RemoveOnDrop(target.clone());
    (repo, target, cleanup)
}

/// `git status --porcelain` lines, trimmed (unstaged " M" reads "M", staged
/// "M " reads "M  "), sorted.
fn status(root: &str) -> Vec<String> {
    let mut lines: Vec<String> = TempRepo::at(root)
        .git(&["status", "--porcelain", "--untracked-files=all"])
        .unwrap()
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| l.trim_matches(' ').to_string())
        .collect();
    lines.sort();
    lines
}

fn sorted(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items
}

#[test]
fn everything_moves_unstaged_and_the_source_is_clean() {
    let (repo, target, _cleanup) = set_up();
    let before = status(&repo.root);

    let moved = move_changes(None, &repo.root, &target).unwrap();
    assert_eq!(moved.count(), 7);
    assert_eq!(
        status(&target),
        [
            "?? e.txt",
            "?? new.txt",
            "?? src/d.txt",
            "D c.txt",
            "D old.txt",
            "M a.txt",
            "M b.txt"
        ]
    );
    assert_eq!(
        std::fs::read_to_string(format!("{target}/src/d.txt")).unwrap(),
        "new and untracked\n"
    );
    assert!(status(&repo.root).is_empty());
    assert!(repo.exists(".env"), "ignored files stay");
    assert!(!repo.exists("src"), "folders left empty go too");

    change_move::undo(&moved, &repo.root).unwrap();
    assert_eq!(status(&repo.root), before);
}

#[test]
fn only_the_chosen_files_move() {
    let (repo, target, _cleanup) = set_up();
    let moved = move_changes(
        Some(&["a.txt", "old.txt", "new.txt", "src/d.txt"]),
        &repo.root,
        &target,
    )
    .unwrap();
    assert_eq!(
        sorted(moved.changed.clone()),
        ["a.txt", "new.txt", "src/d.txt"]
    );
    assert_eq!(moved.deleted, ["old.txt"]);
    assert_eq!(
        status(&target),
        ["?? new.txt", "?? src/d.txt", "D old.txt", "M a.txt"]
    );
    assert_eq!(status(&repo.root), ["A  e.txt", "D  c.txt", "M  b.txt"]);
}

#[test]
fn the_target_has_to_be_on_the_same_commit() {
    let (repo, target, _cleanup) = set_up();
    TempRepo::at(target.as_str())
        .git(&["commit", "-q", "--allow-empty", "-m", "moved on"])
        .unwrap();
    assert!(move_changes(None, &repo.root, &target).is_err());
    assert_eq!(
        status(&repo.root).len(),
        6,
        "nothing was touched (the rename is one line)"
    );
}

/// A clone made inside the checkout, with a commit and untracked work.
fn nested_repository(repo: &TempRepo, path: &str) {
    let nested = TempRepo::at(format!("{}/{path}", repo.root));
    std::fs::create_dir_all(&nested.root).unwrap();
    nested.git(&["init", "-q", "-b", "main"]).unwrap();
    nested.commit(&[("lib.txt", "lib\n")], "lib");
    nested.write("unpushed.txt", "work\n");
}

#[test]
fn a_nested_repository_stays_where_it_is() {
    let (repo, target, _cleanup) = set_up();
    nested_repository(&repo, "vendor/lib");

    let moved = move_changes(None, &repo.root, &target).unwrap();
    assert_eq!(moved.nested_repositories, ["vendor/lib"]);
    assert!(!moved.changed.contains(&"vendor/lib".to_string()));
    assert_eq!(moved.count(), 7, "everything else moved");
    assert!(repo.exists("vendor/lib/.git"));
    assert_eq!(repo.read("vendor/lib/lib.txt"), "lib\n");
    assert_eq!(repo.read("vendor/lib/unpushed.txt"), "work\n");
    assert!(!std::path::Path::new(&format!("{target}/vendor/lib")).exists());
}

#[test]
fn only_a_nested_repository_cant_move() {
    setup();
    let repo = TempRepo::create();
    let target = format!("{}-task", repo.root);
    let _cleanup = RemoveOnDrop(target.clone());
    repo.commit(&[("a.txt", "a\n")], "first");
    repo.git(&["worktree", "add", "-q", "--detach", &target, "HEAD"])
        .unwrap();
    nested_repository(&repo, "clone");
    assert!(move_changes(None, &repo.root, &target).is_err());
    assert!(repo.exists("clone/.git"));
    assert_eq!(repo.read("clone/unpushed.txt"), "work\n");
}

#[test]
fn files_edited_since_the_snapshot_are_found() {
    let (repo, _target, _cleanup) = set_up();
    let snapshot = safety::create("test", &repo.root, OPLOG_PREFIX).unwrap();
    repo.write("a.txt", "edited again\n");
    repo.remove("src/d.txt");
    repo.write("c.txt", "back\n");
    let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let edited = edited_since(
        &snapshot,
        &strings(&["a.txt", "b.txt", "e.txt", "new.txt", "src/d.txt"]),
        &strings(&["c.txt", "old.txt"]),
        &repo.root,
    );
    assert_eq!(sorted(edited), ["a.txt", "c.txt", "src/d.txt"]);
}

#[test]
fn undo_leaves_files_edited_during_the_move_alone() {
    let (repo, target, _cleanup) = set_up();
    let moved = move_changes(None, &repo.root, &target).unwrap();
    assert!(moved.edited_during.is_empty());
    // As if a.txt had been edited while the move ran (so it stayed).
    repo.write("a.txt", "kept edit\n");
    let partly = Moved {
        edited_during: vec!["a.txt".into()],
        nested_repositories: vec![],
        ..moved
    };
    change_move::undo(&partly, &repo.root).unwrap();
    assert_eq!(repo.read("a.txt"), "kept edit\n");
    assert_eq!(
        repo.read("src/d.txt"),
        "new and untracked\n",
        "the rest came back"
    );
}

#[test]
fn undo_leaves_files_changed_since_the_move_alone() {
    let (repo, target, _cleanup) = set_up();
    let moved = move_changes(None, &repo.root, &target).unwrap();
    // After the move: one file edited again, one new file made again.
    repo.write("a.txt", "edited after the move\n");
    repo.write("src/d.txt", "made again\n");

    let left = change_move::undo(&moved, &repo.root).unwrap();
    assert_eq!(sorted(left), ["a.txt", "src/d.txt"]);
    assert_eq!(repo.read("a.txt"), "edited after the move\n");
    assert_eq!(repo.read("src/d.txt"), "made again\n");
    assert_eq!(repo.read("b.txt"), "b staged\n", "the rest came back");
    assert!(!repo.exists("c.txt"));
}

#[test]
fn removing_a_file_never_takes_a_folder() {
    setup();
    let repo = TempRepo::create();
    repo.write("folder/inside.txt", "x\n");
    assert!(!remove_file(&repo.path("folder")));
    assert!(repo.exists("folder/inside.txt"));
    assert!(remove_file(&repo.path("folder/inside.txt")));
    assert!(!repo.exists("folder/inside.txt"));
}
