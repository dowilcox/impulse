//! Repo snapshot, scoped diffs, hunk and line staging and write operations,
//! checked against the git CLI as the oracle (`GitCoreTests` in
//! `GitCoreTests.swift`).

mod common;

use std::collections::HashMap;

use common::{setup, RemoveOnDrop, TempRepo};
use impulse_git::operations::{
    self, CommitOptions, OperationError, PatchTarget, TRUNCATED_DIFF_MESSAGE,
};
use impulse_git::patch::{self, BuildError, PatchSelection};
use impulse_git::scoped_diff::{
    change_identity, changed_files_in_scope, default_base_branch, file_diff, hunk_identity,
    remote_branches_at, resolve_commit, resolve_commits,
};
use impulse_git::{
    idle_handle_count, snapshot, ChangeStatus, DiffLineKind, DiffOptions, DiffScope, FileChange,
    FileDiff, RepoOperation, MAX_DIFF_TOTAL_LINES,
};

fn paths(changes: &[FileChange]) -> Vec<&str> {
    changes.iter().map(|c| c.path.as_str()).collect()
}

fn sorted(mut items: Vec<&str>) -> Vec<&str> {
    items.sort();
    items
}

fn diff(repo: &TempRepo, path: &str, scope: DiffScope) -> FileDiff {
    file_diff(&repo.root, path, None, &scope, &DiffOptions::default()).unwrap()
}

fn apply(
    target: PatchTarget,
    selection: PatchSelection,
    path: &str,
    root: &str,
) -> operations::GitResult {
    operations::apply(
        target,
        &selection,
        path,
        None,
        &HashMap::new(),
        &DiffOptions::default(),
        root,
    )
}

// MARK: Snapshot

#[test]
fn snapshot_separates_staged_unstaged_and_untracked() {
    setup();
    let repo = TempRepo::create();
    repo.commit(
        &[("a.txt", "one\ntwo\nthree\n"), ("b.txt", "b\n")],
        "commit",
    );
    repo.write("a.txt", "one\nTWO\nthree\n");
    repo.git(&["add", "a.txt"]).unwrap();
    repo.write("a.txt", "one\nTWO\nthree\nfour\n");
    repo.write("new.txt", "fresh\n");
    repo.git(&["rm", "-q", "b.txt"]).unwrap();

    let snap = snapshot(&repo.root, true).unwrap();
    assert_eq!(snap.branch.as_deref(), Some("main"));
    assert!(!snap.is_detached);
    assert_eq!(sorted(paths(&snap.staged)), ["a.txt", "b.txt"]);
    assert_eq!(
        snap.staged
            .iter()
            .find(|c| c.path == "b.txt")
            .unwrap()
            .status,
        ChangeStatus::Deleted
    );
    let staged_a = snap.staged.iter().find(|c| c.path == "a.txt").unwrap();
    assert_eq!((staged_a.added, staged_a.removed), (Some(1), Some(1)));
    assert_eq!(paths(&snap.unstaged), ["a.txt"]);
    assert_eq!(
        (snap.unstaged[0].added, snap.unstaged[0].removed),
        (Some(1), Some(0))
    );
    assert_eq!(paths(&snap.untracked), ["new.txt"]);
    assert_eq!(snap.untracked[0].added, Some(1));
    assert_eq!(snap.operation, None);
}

#[test]
fn snapshot_reports_ahead_behind_and_unborn() {
    setup();
    let origin = TempRepo::create();
    origin.commit(&[("a.txt", "1\n")], "commit");
    let clone = TempRepo::create();
    clone
        .git(&["remote", "add", "origin", &origin.root])
        .unwrap();
    clone.git(&["fetch", "-q", "origin"]).unwrap();
    clone
        .git(&["reset", "-q", "--hard", "origin/main"])
        .unwrap();
    clone
        .git(&["branch", "-q", "--set-upstream-to=origin/main", "main"])
        .unwrap();
    clone.commit(&[("b.txt", "2\n")], "local");
    origin.commit(&[("c.txt", "3\n")], "remote");
    clone.git(&["fetch", "-q", "origin"]).unwrap();

    let snap = snapshot(&clone.root, true).unwrap();
    assert_eq!(snap.upstream.as_deref(), Some("origin/main"));
    assert_eq!((snap.ahead, snap.behind), (1, 1));

    let fresh = TempRepo::create_on("trunk");
    let unborn = snapshot(&fresh.root, true).unwrap();
    assert!(unborn.is_unborn);
    assert_eq!(unborn.branch.as_deref(), Some("trunk"));
    assert_eq!(unborn.head_oid, None);
}

#[test]
fn snapshot_detects_merge_conflict_state() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "base\n")], "commit");
    repo.git(&["switch", "-q", "-c", "other"]).unwrap();
    repo.commit(&[("a.txt", "other\n")], "commit");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("a.txt", "main\n")], "commit");
    let _ = repo.git(&["merge", "-q", "other"]);

    let snap = snapshot(&repo.root, true).unwrap();
    assert_eq!(snap.operation, Some(RepoOperation::Merge));
    assert_eq!(paths(&snap.conflicted), ["a.txt"]);
}

// MARK: Scoped diffs

#[test]
fn scoped_diffs_match_git() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n2\n3\n")], "commit");
    repo.write("a.txt", "1\nTWO\n3\n");
    repo.git(&["add", "a.txt"]).unwrap();
    repo.write("a.txt", "1\nTWO\n3\n4\n");

    let staged = diff(&repo, "a.txt", DiffScope::Staged);
    assert_eq!((staged.added, staged.removed), (1, 1));
    let unstaged = diff(&repo, "a.txt", DiffScope::Unstaged);
    assert_eq!((unstaged.added, unstaged.removed), (1, 0));
    let all = diff(&repo, "a.txt", DiffScope::Uncommitted);
    assert_eq!((all.added, all.removed), (2, 1));
    assert_eq!(all.hunk_ids.len(), all.hunks.len());

    let files = changed_files_in_scope(&repo.root, &DiffScope::Uncommitted).unwrap();
    assert_eq!(
        files,
        [FileChange {
            added: Some(2),
            removed: Some(1),
            ..FileChange::new("a.txt", ChangeStatus::Modified)
        }]
    );
}

#[test]
fn more_context_merges_hunks_but_keeps_the_change_identity() {
    setup();
    let repo = TempRepo::create();
    let lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
    repo.commit(&[("a.txt", &(lines.join("\n") + "\n"))], "commit");
    let mut edited = lines.clone();
    edited[2] = "changed 3".into();
    edited[20] = "changed 21".into();
    repo.write("a.txt", &(edited.join("\n") + "\n"));

    let narrow = diff(&repo, "a.txt", DiffScope::Unstaged);
    let wide = file_diff(
        &repo.root,
        "a.txt",
        None,
        &DiffScope::Unstaged,
        &DiffOptions {
            context_lines: 25,
            ignore_whitespace: false,
        },
    )
    .unwrap();
    assert_eq!(narrow.hunks.len(), 2);
    assert_eq!(wide.hunks.len(), 1);
    assert_eq!(
        wide.hunks[0].lines.len(),
        32,
        "the whole file, both changes"
    );
    assert_eq!(change_identity(&narrow.hunks), change_identity(&wide.hunks));
    assert_eq!(hunk_identity(&wide.hunks[0]), change_identity(&wide.hunks));
}

#[test]
fn branch_scope_compares_against_merge_base() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "base\n")], "commit");
    repo.git(&["switch", "-q", "-c", "feature"]).unwrap();
    repo.commit(&[("f.txt", "feature\n")], "feature work");
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("m.txt", "main only\n")], "main moves on");
    repo.git(&["switch", "-q", "feature"]).unwrap();
    repo.write("wip.txt", "uncommitted\n");

    let files = changed_files_in_scope(
        &repo.root,
        &DiffScope::Branch {
            base: "main".into(),
        },
    )
    .unwrap();
    // main's later commit isn't part of this branch's diff.
    assert_eq!(sorted(paths(&files)), ["f.txt", "wip.txt"]);
    assert_eq!(default_base_branch(&repo.root).as_deref(), Some("main"));
}

#[test]
fn commit_and_stash_scopes() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    repo.commit(&[("a.txt", "1\n2\n"), ("b.txt", "b\n")], "second");
    let head = repo.head();
    let commit_files =
        changed_files_in_scope(&repo.root, &DiffScope::Commit { sha: head }).unwrap();
    assert_eq!(sorted(paths(&commit_files)), ["a.txt", "b.txt"]);

    repo.write("a.txt", "1\n2\n3\n");
    repo.git(&["stash", "-q"]).unwrap();
    let stash_files = changed_files_in_scope(&repo.root, &DiffScope::Stash { index: 0 }).unwrap();
    assert_eq!(paths(&stash_files), ["a.txt"]);
    assert_eq!(operations::stash_list(&repo.root).len(), 1);
}

#[test]
fn stash_review_shows_the_untracked_files_it_carries() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    repo.write("a.txt", "1\n2\n");
    repo.write("notes/fresh.txt", "new\n");
    repo.git(&["stash", "-q", "--include-untracked"]).unwrap();

    let files = changed_files_in_scope(&repo.root, &DiffScope::Stash { index: 0 }).unwrap();
    assert_eq!(sorted(paths(&files)), ["a.txt", "notes/fresh.txt"]);
    assert_eq!(
        files
            .iter()
            .find(|c| c.path == "notes/fresh.txt")
            .unwrap()
            .status,
        ChangeStatus::Added
    );
    let fresh = diff(&repo, "notes/fresh.txt", DiffScope::Stash { index: 0 });
    let contents: Vec<&str> = fresh
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| l.content.as_str())
        .collect();
    assert_eq!(contents, ["new"]);
}

// MARK: Patch building and staging

fn two_hunk_repo() -> TempRepo {
    let repo = TempRepo::create();
    let lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
    repo.commit(&[("f.txt", &(lines.join("\n") + "\n"))], "commit");
    let mut edited = lines;
    edited[1] = "line 2 changed".into();
    edited[24] = "line 25 changed".into();
    edited.insert(25, "inserted after 25".into());
    repo.write("f.txt", &(edited.join("\n") + "\n"));
    repo
}

#[test]
fn staging_one_hunk_stages_only_that_hunk() {
    setup();
    let repo = two_hunk_repo();
    let unstaged = diff(&repo, "f.txt", DiffScope::Unstaged);
    assert_eq!(unstaged.hunks.len(), 2);

    operations::apply(
        PatchTarget::Stage,
        &PatchSelection::whole_hunks(&[0]),
        "f.txt",
        None,
        &HashMap::from([(0, unstaged.hunk_ids[0].clone())]),
        &DiffOptions::default(),
        &repo.root,
    )
    .unwrap();
    let cached = repo.git(&["diff", "--cached"]).unwrap();
    assert!(cached.contains("+line 2 changed"));
    assert!(!cached.contains("line 25 changed"));
    let remaining = repo.git(&["diff"]).unwrap();
    assert!(remaining.contains("+line 25 changed"));
    assert!(!remaining.contains("line 2 changed"));
}

#[test]
fn staging_selected_lines_inside_a_hunk() {
    setup();
    let repo = two_hunk_repo();
    let unstaged = diff(&repo, "f.txt", DiffScope::Unstaged);
    // Only the inserted line, not the "line 25" modification.
    let inserted = unstaged.hunks[1]
        .lines
        .iter()
        .position(|l| l.kind == DiffLineKind::Added && l.content == "inserted after 25")
        .unwrap();

    apply(
        PatchTarget::Stage,
        PatchSelection::lines(&[inserted], 1),
        "f.txt",
        &repo.root,
    )
    .unwrap();
    let cached = repo.git(&["diff", "--cached"]).unwrap();
    assert!(cached.contains("+inserted after 25"));
    assert!(!cached.contains("line 25 changed"));
    assert!(!cached.contains("line 2 changed"));
}

#[test]
fn unstaging_and_discarding_hunks() {
    setup();
    let repo = two_hunk_repo();
    repo.git(&["add", "f.txt"]).unwrap();
    let staged = diff(&repo, "f.txt", DiffScope::Staged);
    operations::apply(
        PatchTarget::Unstage,
        &PatchSelection::whole_hunks(&[1]),
        "f.txt",
        None,
        &HashMap::from([(1, staged.hunk_ids[1].clone())]),
        &DiffOptions::default(),
        &repo.root,
    )
    .unwrap();
    assert!(repo
        .git(&["diff", "--cached"])
        .unwrap()
        .contains("line 2 changed"));
    assert!(!repo
        .git(&["diff", "--cached"])
        .unwrap()
        .contains("line 25 changed"));
    assert!(repo.git(&["diff"]).unwrap().contains("line 25 changed"));

    // Discard the remaining unstaged hunk from the working tree.
    apply(
        PatchTarget::Discard,
        PatchSelection::whole_hunks(&[0]),
        "f.txt",
        &repo.root,
    )
    .unwrap();
    assert_eq!(repo.git(&["diff"]).unwrap(), "");
    assert!(!repo.read("f.txt").contains("line 25 changed"));
    assert!(repo.read("f.txt").contains("line 2 changed"));
}

#[test]
fn reverting_a_later_hunk_lands_in_its_own_block() {
    // Hunk 0 adds lines at the top; hunks 1 and 2 delete the same line in two
    // identical blocks. Reverting only hunk 2 (or unstaging it) must restore
    // block 2, not the identical block 1.
    setup();
    let repo = TempRepo::create();
    let block = "begin\nalpha\nbeta\nTARGET\ngamma\ndelta\nend\n";
    let filler: String = (1..=12).map(|i| format!("filler {i}\n")).collect();
    repo.commit(
        &[("f.txt", &format!("{filler}{block}{filler}{block}"))],
        "commit",
    );
    let added: String = (1..=30).map(|i| format!("new {i}\n")).collect();
    let edited = block.replace("TARGET\n", "");
    repo.write("f.txt", &format!("{added}{filler}{edited}{filler}{edited}"));

    let unstaged = diff(&repo, "f.txt", DiffScope::Unstaged);
    assert_eq!(unstaged.hunks.len(), 3);
    apply(
        PatchTarget::Discard,
        PatchSelection::whole_hunks(&[2]),
        "f.txt",
        &repo.root,
    )
    .unwrap();
    assert_eq!(
        repo.read("f.txt"),
        format!("{added}{filler}{edited}{filler}{block}")
    );

    // The same through the index: stage everything, unstage hunk 2.
    repo.write("f.txt", &format!("{added}{filler}{edited}{filler}{edited}"));
    repo.git(&["add", "f.txt"]).unwrap();
    apply(
        PatchTarget::Unstage,
        PatchSelection::whole_hunks(&[2]),
        "f.txt",
        &repo.root,
    )
    .unwrap();
    assert_eq!(
        repo.git(&["show", ":f.txt"]).unwrap() + "\n",
        format!("{added}{filler}{edited}{filler}{block}")
    );
}

#[test]
fn hunks_of_non_utf8_files_are_refused_not_corrupted() {
    setup();
    let repo = TempRepo::create();
    repo.write_bytes("latin1.txt", b"caf\xE9\nline\n");
    repo.git(&["add", "latin1.txt"]).unwrap();
    repo.git(&["commit", "-q", "-m", "latin1"]).unwrap();
    repo.write_bytes("latin1.txt", b"caf\xE9 au lait\nline\n");

    let result = apply(
        PatchTarget::Stage,
        PatchSelection::whole_hunks(&[0]),
        "latin1.txt",
        &repo.root,
    );
    assert!(
        matches!(result, Err(OperationError::Invalid(_))),
        "{result:?}"
    );
    assert_eq!(
        repo.git(&["diff", "--cached", "--name-only"]).unwrap(),
        "",
        "nothing reached the index"
    );
}

#[test]
fn part_of_a_rename_is_refused_not_undone() {
    setup();
    let repo = TempRepo::create();
    let lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
    repo.commit(&[("old.txt", &(lines.join("\n") + "\n"))], "commit");
    repo.git(&["mv", "old.txt", "new.txt"]).unwrap();
    let mut edited = lines;
    edited[1] = "line 2 changed".into();
    edited[35] = "line 36 changed".into();
    repo.write("new.txt", &(edited.join("\n") + "\n"));
    repo.git(&["add", "-A"]).unwrap();
    let staged = file_diff(
        &repo.root,
        "new.txt",
        Some("old.txt"),
        &DiffScope::Staged,
        &DiffOptions::default(),
    )
    .unwrap();
    assert_eq!(staged.hunks.len(), 2);

    let result = operations::apply(
        PatchTarget::Unstage,
        &PatchSelection::whole_hunks(&[1]),
        "new.txt",
        Some("old.txt"),
        &HashMap::new(),
        &DiffOptions::default(),
        &repo.root,
    );
    assert!(
        matches!(result, Err(OperationError::Invalid(_))),
        "{result:?}"
    );
    assert_eq!(
        repo.git(&["status", "--porcelain"]).unwrap(),
        "R  old.txt -> new.txt",
        "the rename is intact"
    );
}

#[test]
fn stale_hunk_id_is_rejected() {
    setup();
    let repo = two_hunk_repo();
    let result = operations::apply(
        PatchTarget::Stage,
        &PatchSelection::whole_hunks(&[0]),
        "f.txt",
        None,
        &HashMap::from([(0, "deadbeefdeadbeef".to_string())]),
        &DiffOptions::default(),
        &repo.root,
    );
    assert!(
        matches!(result, Err(OperationError::Stale(_))),
        "{result:?}"
    );
}

#[test]
fn staging_at_end_of_file_without_newline() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("n.txt", "a\nb")], "commit");
    repo.write("n.txt", "a\nb\nc");
    let unstaged = diff(&repo, "n.txt", DiffScope::Unstaged);
    assert!(unstaged.old_missing_newline_at_end);
    assert!(unstaged.new_missing_newline_at_end);
    apply(
        PatchTarget::Stage,
        PatchSelection::whole_hunks(&[0]),
        "n.txt",
        &repo.root,
    )
    .unwrap();
    assert_eq!(repo.git(&["diff"]).unwrap(), "");
}

#[test]
fn zero_context_hunks_and_lines_stage_unstage_and_revert_in_place() {
    // "Changed Lines Only": hunks carry no context, so pure insertions and
    // deletions have nothing to match and must land at their own line.
    setup();
    let repo = TempRepo::create();
    let base: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
    repo.commit(&[("f.txt", &(base.join("\n") + "\n"))], "commit");
    fn inserting(line: &str, anchor: &str, lines: &[String]) -> Vec<String> {
        let mut result = lines.to_vec();
        let at = result.iter().position(|l| l == anchor).unwrap() + 1;
        result.insert(at, line.into());
        result
    }
    let mut working = base.clone();
    working[1] = "line 2 changed".into();
    working = inserting("inserted after 10", "line 10", &working);
    working.retain(|l| l != "line 20");
    let at = working.iter().position(|l| l == "line 25").unwrap();
    working[at] = "line 25 changed".into();
    working = inserting("inserted after 25", "line 25 changed", &working);
    repo.write("f.txt", &(working.join("\n") + "\n"));

    let zero = DiffOptions {
        context_lines: 0,
        ignore_whitespace: false,
    };
    let hunk = |scope: DiffScope, text: &str| -> (usize, FileDiff) {
        let diff = file_diff(&repo.root, "f.txt", None, &scope, &zero).unwrap();
        let index = diff
            .hunks
            .iter()
            .position(|h| h.lines.iter().any(|l| l.content == text))
            .unwrap();
        assert!(diff.hunks[index]
            .lines
            .iter()
            .all(|l| l.kind != DiffLineKind::Context));
        (index, diff)
    };
    let run = |target: PatchTarget, selection: PatchSelection| {
        operations::apply(
            target,
            &selection,
            "f.txt",
            None,
            &HashMap::new(),
            &zero,
            &repo.root,
        )
        .unwrap();
    };
    let index = || repo.git(&["show", ":f.txt"]).unwrap();

    // Stage a pure insertion, one line of a mixed hunk, and a pure deletion.
    let mut expected = inserting("inserted after 10", "line 10", &base);
    run(
        PatchTarget::Stage,
        PatchSelection::whole_hunks(&[hunk(DiffScope::Unstaged, "inserted after 10").0]),
    );
    assert_eq!(index(), expected.join("\n"));

    let (mixed, mixed_diff) = hunk(DiffScope::Unstaged, "inserted after 25");
    let line = mixed_diff.hunks[mixed]
        .lines
        .iter()
        .position(|l| l.content == "inserted after 25")
        .unwrap();
    run(PatchTarget::Stage, PatchSelection::lines(&[line], mixed));
    expected = inserting("inserted after 25", "line 25", &expected);
    assert_eq!(index(), expected.join("\n"));

    run(
        PatchTarget::Stage,
        PatchSelection::whole_hunks(&[hunk(DiffScope::Unstaged, "line 20").0]),
    );
    expected.retain(|l| l != "line 20");
    assert_eq!(index(), expected.join("\n"));

    // Unstage the deletion and the insertion again.
    run(
        PatchTarget::Unstage,
        PatchSelection::whole_hunks(&[hunk(DiffScope::Staged, "line 20").0]),
    );
    expected = inserting("line 20", "line 19", &expected);
    assert_eq!(index(), expected.join("\n"));
    run(
        PatchTarget::Unstage,
        PatchSelection::whole_hunks(&[hunk(DiffScope::Staged, "inserted after 10").0]),
    );
    expected.retain(|l| l != "inserted after 10");
    assert_eq!(index(), expected.join("\n"));

    // Revert the working tree's deletion and modification.
    run(
        PatchTarget::Discard,
        PatchSelection::whole_hunks(&[hunk(DiffScope::Unstaged, "line 20").0]),
    );
    working = inserting("line 20", "line 19", &working);
    assert_eq!(repo.read("f.txt"), working.join("\n") + "\n");
    run(
        PatchTarget::Discard,
        PatchSelection::whole_hunks(&[hunk(DiffScope::Unstaged, "line 2 changed").0]),
    );
    working[1] = "line 2".into();
    assert_eq!(repo.read("f.txt"), working.join("\n") + "\n");
    assert_eq!(
        index(),
        expected.join("\n"),
        "reverting leaves the index alone"
    );
}

#[test]
fn hunks_of_a_cut_short_diff_are_refused() {
    // With the whole file as context the one hunk ends where the diff was
    // cut, so staging it would take changes past what was shown.
    setup();
    let repo = TempRepo::create();
    let base: Vec<String> = (1..=MAX_DIFF_TOTAL_LINES + 100)
        .map(|i| format!("line {i}"))
        .collect();
    repo.commit(&[("big.txt", &(base.join("\n") + "\n"))], "commit");
    let mut edited = base;
    edited[0] = "first changed".into();
    let last = edited.len() - 1;
    edited[last] = "last changed".into();
    repo.write("big.txt", &(edited.join("\n") + "\n"));
    // The review's "Whole File".
    let whole = DiffOptions {
        context_lines: 100_000,
        ignore_whitespace: false,
    };
    let big = file_diff(&repo.root, "big.txt", None, &DiffScope::Unstaged, &whole).unwrap();
    assert!(big.truncated && big.hunks.len() == 1);

    for target in [PatchTarget::Stage, PatchTarget::Discard] {
        let result = operations::apply(
            target,
            &PatchSelection::whole_hunks(&[0]),
            "big.txt",
            None,
            &HashMap::from([(0, big.hunk_ids[0].clone())]),
            &whole,
            &repo.root,
        );
        assert_eq!(
            result,
            Err(OperationError::Invalid(TRUNCATED_DIFF_MESSAGE.into())),
            "{target:?}"
        );
    }
    assert_eq!(
        repo.git(&["diff", "--cached", "--name-only"]).unwrap(),
        "",
        "nothing staged"
    );
    assert_eq!(
        repo.read("big.txt"),
        edited.join("\n") + "\n",
        "nothing reverted"
    );
    // The whole file still stages.
    operations::stage(&["big.txt"], &repo.root).unwrap();
    assert_eq!(repo.git(&["diff", "--name-only"]).unwrap(), "");
}

#[test]
fn partial_selection_of_new_file_is_refused() {
    let text = "diff --git a/n.txt b/n.txt\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n";
    assert_eq!(
        patch::build(text, &PatchSelection::lines(&[0], 0), false),
        Err(BuildError::PartialAddOrDelete)
    );
    let whole = patch::build(text, &PatchSelection::whole_hunks(&[0]), false).unwrap();
    assert!(whole.contains("+two"));
}

// MARK: Commit, branches and worktrees

#[test]
fn commit_runs_through_git_and_can_amend_and_uncommit() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    repo.write("a.txt", "2\n");
    operations::stage(&["a.txt"], &repo.root).unwrap();
    let sha = operations::commit(
        "Change a\n\nWith a body.",
        &CommitOptions::default(),
        &repo.root,
    )
    .unwrap();
    assert_eq!(sha, repo.head());
    assert_eq!(
        repo.git(&["log", "-1", "--format=%B"]).unwrap(),
        "Change a\n\nWith a body."
    );

    let amended = operations::commit(
        "Change a (amended)",
        &CommitOptions {
            amend: true,
            ..Default::default()
        },
        &repo.root,
    )
    .unwrap();
    assert_ne!(amended, sha);
    assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]).unwrap(), "2");

    operations::uncommit(None, &repo.root).unwrap();
    assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]).unwrap(), "1");
    assert_eq!(
        repo.git(&["diff", "--cached", "--name-only"]).unwrap(),
        "a.txt"
    );
}

#[test]
fn commit_messages_keep_hash_lines() {
    setup();
    let repo = TempRepo::create();
    repo.write("a.txt", "1\n");
    repo.git(&["add", "a.txt"]).unwrap();
    let message = "Fix crash\n\n#482 was caused by a race.\n## Notes\nKeep";
    operations::commit(message, &CommitOptions::default(), &repo.root).unwrap();
    assert_eq!(repo.git(&["log", "-1", "--format=%B"]).unwrap(), message);
}

#[test]
fn paths_are_literal_not_globs() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("app/[id].tsx", "a\n"), ("app/d.tsx", "b\n")], "commit");
    repo.write("app/[id].tsx", "a2\n");
    repo.write("app/d.tsx", "b2\n");
    // As a glob, "[id]" would also match "d.tsx".
    operations::discard_working_tree(&["app/[id].tsx"], &repo.root).unwrap();
    assert_eq!(repo.read("app/[id].tsx"), "a\n");
    assert_eq!(repo.read("app/d.tsx"), "b2\n");
    operations::stage(&["app/[id].tsx"], &repo.root).unwrap();
    assert_eq!(repo.git(&["diff", "--cached", "--name-only"]).unwrap(), "");
}

#[test]
fn refs_that_look_like_options_stay_refs() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    repo.git(&["update-ref", "refs/heads/--orphan=x", "HEAD"])
        .unwrap();
    operations::switch_branch("--orphan=x", &repo.root).unwrap();
    assert_eq!(
        repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap(),
        "--orphan=x"
    );
    assert!(repo.exists("a.txt"), "no orphan branch emptied the tree");
}

#[test]
fn empty_message_is_rejected() {
    setup();
    let repo = TempRepo::create();
    let result = operations::commit("  \n", &CommitOptions::default(), &repo.root);
    assert!(matches!(result, Err(OperationError::Invalid(_))));
}

#[test]
fn branch_and_worktree_operations() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    operations::create_branch("topic", None, false, &repo.root).unwrap();
    let mut local = operations::branches(&repo.root).0;
    local.sort();
    assert_eq!(local, ["main", "topic"]);
    operations::rename_branch("topic", "topic-2", &repo.root).unwrap();
    operations::delete_branch("topic-2", false, &repo.root).unwrap();
    assert_eq!(operations::branches(&repo.root).0, ["main"]);

    let path = format!("{}-wt-feature", repo.root);
    let _cleanup = RemoveOnDrop(path.clone());
    operations::add_worktree(&path, "feature", true, None, false, &repo.root).unwrap();
    let trees = operations::worktrees(&repo.root);
    assert_eq!(trees.len(), 2);
    assert!(trees.iter().any(|w| w.branch.as_deref() == Some("feature")));
    operations::remove_worktree(&path, false, &repo.root).unwrap();
    assert_eq!(operations::worktrees(&repo.root).len(), 1);
}

// MARK: Reads on a reused repository

#[test]
fn resolving_reuses_the_repository_but_sees_refs_move() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    let first = repo.head();
    assert_eq!(
        resolve_commit(&repo.root, "main").as_deref(),
        Some(first.as_str())
    );
    assert!(idle_handle_count() > 0);

    repo.commit(&[("a.txt", "2\n")], "commit");
    assert_eq!(resolve_commit(&repo.root, "main"), Some(repo.head()));
    repo.git(&["pack-refs", "--all"]).unwrap();
    repo.commit(&[("a.txt", "3\n")], "commit");
    let third = repo.head();
    assert_eq!(
        resolve_commit(&format!("{}/a.txt", repo.root), "main").as_deref(),
        Some(third.as_str())
    );
    assert_eq!(
        resolve_commits(&repo.root, &["main", "HEAD~2", "no-such-branch"]),
        HashMap::from([
            ("main".to_string(), third.clone()),
            ("HEAD~2".to_string(), first)
        ])
    );

    // A worktree that's gone resolves nothing, however recently it was read.
    let task = format!("{}-handles", repo.root);
    let _cleanup = RemoveOnDrop(task.clone());
    repo.git(&["worktree", "add", "-q", "-b", "task", &task])
        .unwrap();
    assert_eq!(
        resolve_commit(&task, "HEAD").as_deref(),
        Some(third.as_str())
    );
    repo.git(&["worktree", "remove", &task]).unwrap();
    assert_eq!(resolve_commit(&task, "HEAD"), None);
}

#[test]
fn remote_branches_at_a_commit_match_git() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n")], "commit");
    let old = repo.head();
    repo.commit(&[("a.txt", "2\n")], "commit");
    let head = repo.head();
    repo.git(&["update-ref", "refs/remotes/origin/main", &head])
        .unwrap();
    repo.git(&["update-ref", "refs/remotes/origin/feature/x", &head])
        .unwrap();
    repo.git(&["update-ref", "refs/remotes/fork/main", &old])
        .unwrap();
    repo.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ])
    .unwrap();
    let git = |commit: &str| -> Vec<String> {
        let mut names: Vec<String> = repo
            .git(&[
                "for-each-ref",
                &format!("--points-at={commit}"),
                "--format=%(refname:lstrip=2)",
                "refs/remotes",
            ])
            .unwrap()
            .split('\n')
            .filter(|n| !n.is_empty() && !n.ends_with("/HEAD"))
            .map(str::to_string)
            .collect();
        names.sort();
        names
    };
    let at = |commit: &str| {
        let mut names = remote_branches_at(commit, &repo.root);
        names.sort();
        names
    };
    assert_eq!(at(&head), ["origin/feature/x", "origin/main"]);
    assert_eq!(at(&head), git(&head));
    assert_eq!(at(&old), ["fork/main"]);
    repo.git(&["pack-refs", "--all"]).unwrap();
    assert_eq!(at(&head), git(&head));
    assert!(at("not-a-commit").is_empty());
}

#[test]
fn snapshot_without_line_counts_lists_the_same_files() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n"), ("b.txt", "1\n")], "commit");
    repo.write("a.txt", "1\n2\n");
    repo.write("b.txt", "x\n");
    repo.git(&["add", "b.txt"]).unwrap();
    repo.write("c.txt", "new\n");
    let counted = snapshot(&repo.root, true).unwrap();
    let plain = snapshot(&repo.root, false).unwrap();
    assert_eq!(paths(&plain.staged), paths(&counted.staged));
    assert_eq!(paths(&plain.unstaged), paths(&counted.unstaged));
    assert_eq!(paths(&plain.untracked), paths(&counted.untracked));
    assert_eq!(plain.head_oid, counted.head_oid);
    assert!(counted.total_added() > 0);
    assert!(plain
        .staged
        .iter()
        .chain(&plain.unstaged)
        .chain(&plain.untracked)
        .all(|c| c.added.is_none() && c.removed.is_none()));
}
