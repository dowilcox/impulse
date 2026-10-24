//! Finish Task's landing: a merge commit pushed as the base from a throwaway
//! worktree, against a bare repository standing in for the remote. The git
//! CLI is the oracle (`TaskLandingTests.swift`).

mod common;

use common::{setup, TempRepo};
use impulse_git::landing::{self, BranchCleanup, Failure};
use impulse_git::operations::{self, OperationError};

/// A clone of a bare "origin" with one pushed commit on main, a task branch
/// `fix` with one commit of its own, and a second clone ("someone else")
/// that can push to origin too.
fn set_up() -> (TempRepo, TempRepo, TempRepo) {
    setup();
    let origin = TempRepo::create();
    origin.git(&["config", "core.bare", "true"]).unwrap();
    let other = TempRepo::create();
    other.commit(&[("a.txt", "1\n2\n3\n")], "first");
    let origin_url = format!("{}/.git", origin.root);
    other
        .git(&["remote", "add", "origin", &origin_url])
        .unwrap();
    other.git(&["push", "-q", "-u", "origin", "main"]).unwrap();
    let repo = TempRepo::create();
    repo.git(&["remote", "add", "origin", &origin_url]).unwrap();
    repo.git(&["fetch", "-q", "origin"]).unwrap();
    repo.git(&["checkout", "-q", "-B", "main", "--track", "origin/main"])
        .unwrap();
    repo.git(&["branch", "-q", "--no-track", "fix", "origin/main"])
        .unwrap();
    repo.git(&["switch", "-q", "fix"]).unwrap();
    repo.commit(&[("b.txt", "fix\n")], "fix");
    repo.git(&["switch", "-q", "main"]).unwrap();
    (repo, origin, other)
}

fn worktree_count(repo: &TempRepo) -> usize {
    repo.git(&["worktree", "list", "--porcelain"])
        .unwrap()
        .split('\n')
        .filter(|l| l.starts_with("worktree "))
        .count()
}

fn merge_and_push(branch: &str, repo: &TempRepo) -> Result<landing::Landed, Failure> {
    landing::merge_and_push(branch, "main", "origin", &repo.root, None)
}

#[test]
fn lands_a_merge_commit_without_touching_the_main_checkout() {
    let (repo, origin, _other) = set_up();
    repo.write("a.txt", "local edit\n");
    let head = repo.head();

    let landed = merge_and_push("fix", &repo).unwrap();
    assert!(!landed.retried);
    assert_eq!(origin.git(&["rev-parse", "main"]).unwrap(), landed.commit);
    assert_eq!(
        repo.git(&["rev-parse", "origin/main"]).unwrap(),
        landed.commit,
        "the push updates the remote-tracking branch"
    );
    let parents = repo
        .git(&["rev-list", "--parents", "-n", "1", &landed.commit])
        .unwrap();
    assert_eq!(parents.split(' ').count(), 3);
    assert_eq!(
        repo.git(&["log", "-1", "--format=%s", &landed.commit])
            .unwrap(),
        "Merge branch 'fix'"
    );
    assert_eq!(repo.head(), head);
    assert_eq!(repo.read("a.txt"), "local edit\n");
    assert_eq!(worktree_count(&repo), 1, "the throwaway worktree is gone");
}

#[test]
fn merges_again_when_the_base_moved_meanwhile() {
    let (repo, origin, other) = set_up();
    // Someone else pushes after this clone last fetched.
    other.commit(&[("c.txt", "other\n")], "other");
    other.git(&["push", "-q", "origin", "main"]).unwrap();

    let landed = merge_and_push("fix", &repo).unwrap();
    assert!(landed.retried);
    assert_eq!(origin.git(&["rev-parse", "main"]).unwrap(), landed.commit);
    assert_eq!(
        origin
            .git(&["show", &format!("{}:c.txt", landed.commit)])
            .unwrap(),
        "other"
    );
    assert_eq!(
        origin
            .git(&["show", &format!("{}:b.txt", landed.commit)])
            .unwrap(),
        "fix"
    );
    assert_eq!(worktree_count(&repo), 1);
}

#[test]
fn conflicts_change_nothing() {
    let (repo, origin, other) = set_up();
    other.commit(&[("b.txt", "theirs\n")], "theirs");
    other.git(&["push", "-q", "origin", "main"]).unwrap();
    repo.git(&["fetch", "-q", "origin"]).unwrap();
    let before = origin.git(&["rev-parse", "main"]).unwrap();

    assert_eq!(
        merge_and_push("fix", &repo),
        Err(Failure::Conflicts(vec!["b.txt".into()]))
    );
    assert_eq!(origin.git(&["rev-parse", "main"]).unwrap(), before);
    assert_eq!(worktree_count(&repo), 1);
}

#[test]
fn a_server_refusal_is_told_apart() {
    let (repo, origin, _other) = set_up();
    origin.write(
        ".git/hooks/pre-receive",
        "#!/bin/sh\necho 'You are not allowed to push to main.'\nexit 1\n",
    );
    set_executable(&origin.path(".git/hooks/pre-receive"));

    match merge_and_push("fix", &repo) {
        Err(Failure::Refused(output)) => {
            assert!(output.contains("You are not allowed to push to main."))
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_branch_with_nothing_new_has_nothing_to_land() {
    let (repo, _origin, _other) = set_up();
    repo.git(&["branch", "-q", "--no-track", "empty", "origin/main"])
        .unwrap();
    assert_eq!(merge_and_push("empty", &repo), Err(Failure::NothingToLand));
    assert_eq!(worktree_count(&repo), 1);
}

#[test]
fn without_a_remote_it_merges_into_a_clean_main_checkout_only() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "first");
    repo.git(&["switch", "-q", "-c", "fix"]).unwrap();
    repo.commit(&[("b.txt", "fix\n")], "fix");
    repo.git(&["switch", "-q", "-c", "other"]).unwrap();
    assert_eq!(
        landing::merge_locally("fix", "main", &repo.root),
        Err(Failure::NotOnBase {
            current: Some("other".into())
        })
    );
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.write("a.txt", "draft\n");
    assert_eq!(
        landing::merge_locally("fix", "main", &repo.root),
        Err(Failure::Uncommitted(1))
    );
    repo.git(&["checkout", "--", "a.txt"]).unwrap();

    // An untracked file the merge would overwrite stops it, changing
    // nothing.
    let before = repo.head();
    repo.write("b.txt", "mine\n");
    assert_eq!(
        landing::merge_locally("fix", "main", &repo.root),
        Err(Failure::Untracked(vec!["b.txt".into()]))
    );
    assert_eq!(repo.head(), before);
    assert_eq!(repo.read("b.txt"), "mine\n");
    repo.remove("b.txt");

    // Any other untracked file doesn't count.
    repo.write("notes.txt", "draft\n");
    let landed = landing::merge_locally("fix", "main", &repo.root).unwrap();
    assert_eq!(repo.head(), landed.commit);
    assert_eq!(
        repo.git(&["log", "-1", "--format=%s"]).unwrap(),
        "Merge branch 'fix'"
    );
    assert!(repo.exists("b.txt"));
    assert!(repo.exists("notes.txt"));
    assert_eq!(
        landing::merge_locally("fix", "main", &repo.root),
        Err(Failure::NothingToLand)
    );
}

#[test]
fn a_fast_forward_names_the_untracked_files_in_its_way() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "first");
    repo.git(&["switch", "-q", "-c", "ahead"]).unwrap();
    repo.commit(&[("b.txt", "b\n"), ("c.txt", "c\n")], "ahead");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.write("b.txt", "mine\n");
    repo.write("c.txt", "mine\n");
    let error = operations::fast_forward("ahead", &repo.root).unwrap_err();
    assert_eq!(
        landing::untracked_in_the_way(error.output().unwrap_or("")),
        ["b.txt", "c.txt"]
    );
    assert_eq!(
        landing::naming(&["b.txt".into(), "c.txt".into()]),
        "b.txt and 1 more file"
    );
    assert!(
        landing::untracked_in_the_way("fatal: Not possible to fast-forward, aborting.").is_empty()
    );

    repo.remove("b.txt");
    repo.remove("c.txt");
    repo.write("notes.txt", "draft\n");
    operations::fast_forward("ahead", &repo.root).unwrap();
}

// MARK: Deleting a landed branch

/// `fix` pushed to origin, then landed.
fn landed() -> (TempRepo, TempRepo, TempRepo, String) {
    let (repo, origin, other) = set_up();
    repo.git(&["push", "-q", "origin", "fix"]).unwrap();
    let landed = merge_and_push("fix", &repo).unwrap();
    (repo, origin, other, landed.commit)
}

fn exists(reference: &str, repo: &TempRepo) -> bool {
    repo.git(&["rev-parse", "--verify", "-q", reference])
        .is_ok()
}

#[test]
fn a_landed_branch_is_deleted_here_and_on_the_remote() {
    let (repo, origin, _other, commit) = landed();
    let tip = repo.git(&["rev-parse", "fix"]).unwrap();
    repo.git(&["config", "branch.fix.remote", "origin"])
        .unwrap();

    let cleanup = landing::delete_landed_branch(
        "fix",
        &[&commit, "origin/main"],
        Some("origin"),
        &repo.root,
        false,
    );
    assert_eq!(
        cleanup,
        BranchCleanup {
            deleted_commit: Some(tip),
            deleted_on_remote: true,
            kept: vec![],
        }
    );
    assert!(!exists("refs/heads/fix", &repo));
    assert!(!exists("refs/heads/fix", &origin));
    assert!(
        repo.git(&["config", "--get", "branch.fix.remote"]).is_err(),
        "its settings go too"
    );
}

#[test]
fn a_branch_with_commits_since_it_landed_is_kept() {
    let (repo, origin, _other, commit) = landed();
    repo.git(&["switch", "-q", "fix"]).unwrap();
    repo.commit(&[("d.txt", "more\n")], "more");
    repo.git(&["switch", "-q", "main"]).unwrap();
    let tip = repo.git(&["rev-parse", "fix"]).unwrap();

    let cleanup = landing::delete_landed_branch(
        "fix",
        &[&commit, "origin/main"],
        Some("origin"),
        &repo.root,
        false,
    );
    assert_eq!(cleanup.deleted_commit, None);
    assert_eq!(cleanup.kept.len(), 1);
    assert_eq!(repo.git(&["rev-parse", "fix"]).unwrap(), tip);
    assert!(exists("refs/heads/fix", &origin));
}

#[test]
fn a_branch_moved_only_at_the_last_moment_is_kept() {
    let (repo, _origin, _other, _) = landed();
    let tip = repo.git(&["rev-parse", "fix"]).unwrap();
    repo.git(&["switch", "-q", "fix"]).unwrap();
    repo.commit(&[("d.txt", "more\n")], "more");
    repo.git(&["switch", "-q", "main"]).unwrap();
    // The compare-and-swap: deleting it at the old tip fails.
    let result = operations::delete_branch_at("fix", &tip, &repo.root);
    assert!(
        matches!(result, Err(OperationError::Stale(_))),
        "{result:?}"
    );
    assert!(exists("refs/heads/fix", &repo));
}

#[test]
fn the_remote_branch_is_kept_when_someone_pushed_to_it() {
    let (repo, origin, other, commit) = landed();
    // Someone else adds to fix on the remote; this clone hasn't fetched it.
    other.git(&["fetch", "-q", "origin"]).unwrap();
    other
        .git(&["switch", "-q", "-c", "fix", "origin/fix"])
        .unwrap();
    other.commit(&[("e.txt", "theirs\n")], "theirs");
    other.git(&["push", "-q", "origin", "fix"]).unwrap();
    let theirs = other.head();

    let cleanup = landing::delete_landed_branch(
        "fix",
        &[&commit, "origin/main"],
        Some("origin"),
        &repo.root,
        false,
    );
    assert!(
        cleanup.deleted_commit.is_some(),
        "the local branch is merged, so it goes"
    );
    assert!(!cleanup.deleted_on_remote);
    assert_eq!(cleanup.kept.len(), 1);
    assert_eq!(
        origin.git(&["rev-parse", "refs/heads/fix"]).unwrap(),
        theirs
    );
}

fn set_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
