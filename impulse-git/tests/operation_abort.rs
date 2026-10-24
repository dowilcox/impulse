//! Aborting after edits, against real merges. The git CLI is the oracle
//! (`OperationAbortTests.swift`).

mod common;

use std::time::{Duration, SystemTime};

use common::{setup, TempRepo};
use impulse_git::operation_abort::{
    self, incoming_head, load_record, record_path, record_start_if_needed, Outcome, StartRecord,
    DEFAULT_RECORD_AGE,
};
use impulse_git::operations::{self, OperationAction};
use impulse_git::RepoOperation;

fn incoming_name(root: &str) -> Option<String> {
    incoming_head(root).map(|(name, _)| name)
}

/// main and upgrade both change page.tsx (a conflict); upgrade also changes
/// link.tsx (merges cleanly). notes.txt has uncommitted work from before the
/// merge.
fn conflicted_merge() -> TempRepo {
    setup();
    let repo = TempRepo::create();
    repo.commit(
        &[
            ("page.tsx", "a\n"),
            ("link.tsx", "Link\n"),
            ("other.txt", "x\n"),
            ("notes.txt", "n\n"),
        ],
        "base",
    );
    repo.git(&["switch", "-q", "-c", "upgrade"]).unwrap();
    repo.commit(
        &[("page.tsx", "upgrade\n"), ("link.tsx", "RouterLink\n")],
        "upgrade",
    );
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("page.tsx", "overhaul\n")], "overhaul");
    repo.write("notes.txt", "local work\n");
    let _ = repo.git(&["merge", "upgrade"]);
    assert_eq!(incoming_name(&repo.root).as_deref(), Some("MERGE_HEAD"));
    repo
}

#[test]
fn gits_own_abort_is_used_when_it_works() {
    let repo = conflicted_merge();
    repo.write("page.tsx", "resolved\n");
    let result = operation_abort::abort(&RepoOperation::Merge, &repo.root).unwrap();
    assert_eq!(result.outcome, Outcome::Aborted);
    assert_eq!(repo.read("notes.txt"), "local work\n");
    assert_eq!(repo.read("page.tsx"), "overhaul\n");
    assert_eq!(incoming_head(&repo.root), None);
}

#[test]
fn a_refused_abort_resets_and_keeps_exactly_the_work_from_before() {
    let repo = conflicted_merge();
    record_start_if_needed(&repo.root, DEFAULT_RECORD_AGE);
    // Resolving: a cleanly merged file edited again makes git refuse.
    repo.write("page.tsx", "resolved\n");
    repo.write("link.tsx", "fixed Link\n");
    repo.write("other.txt", "edited while resolving\n");
    assert!(
        operations::perform(OperationAction::Abort, &RepoOperation::Merge, &repo.root).is_err(),
        "git's own abort should refuse here"
    );

    let result = operation_abort::abort(&RepoOperation::Merge, &repo.root).unwrap();
    assert_eq!(
        result.outcome,
        Outcome::Reset {
            kept: vec!["notes.txt".into()],
            exact: true
        }
    );
    assert_eq!(incoming_head(&repo.root), None, "the merge is over");
    assert_eq!(
        repo.read("notes.txt"),
        "local work\n",
        "work from before the merge is back"
    );
    assert_eq!(repo.read("page.tsx"), "overhaul\n");
    assert_eq!(repo.read("link.tsx"), "Link\n");
    assert_eq!(
        repo.read("other.txt"),
        "x\n",
        "edits made while resolving aren't mistaken for earlier work"
    );
    assert!(result.undo.is_some());
}

#[test]
fn without_a_record_edits_outside_the_merge_are_kept() {
    let repo = conflicted_merge();
    repo.write("link.tsx", "fixed Link\n");
    repo.write("other.txt", "edited while resolving\n");
    let result = operation_abort::abort(&RepoOperation::Merge, &repo.root).unwrap();
    assert_eq!(
        result.outcome,
        Outcome::Reset {
            kept: vec!["notes.txt".into(), "other.txt".into()],
            exact: false
        }
    );
    assert_eq!(repo.read("other.txt"), "edited while resolving\n");
    assert_eq!(repo.read("link.tsx"), "Link\n");
}

#[test]
fn undo_reopens_the_merge_with_the_edits_back() {
    let repo = conflicted_merge();
    record_start_if_needed(&repo.root, DEFAULT_RECORD_AGE);
    repo.write("page.tsx", "resolved\n");
    repo.write("link.tsx", "fixed Link\n");
    let result = operation_abort::abort(&RepoOperation::Merge, &repo.root).unwrap();
    operation_abort::undo(&result, &RepoOperation::Merge, &repo.root).unwrap();
    assert_eq!(
        incoming_name(&repo.root).as_deref(),
        Some("MERGE_HEAD"),
        "the merge is open again"
    );
    assert_eq!(repo.read("page.tsx"), "resolved\n");
    assert_eq!(repo.read("link.tsx"), "fixed Link\n");
    assert_eq!(repo.read("notes.txt"), "local work\n");
}

#[test]
fn work_from_before_with_a_non_ascii_name_is_put_back() {
    setup();
    let repo = TempRepo::create();
    repo.commit(
        &[
            ("page.tsx", "a\n"),
            ("link.tsx", "Link\n"),
            ("café.txt", "c\n"),
        ],
        "base",
    );
    repo.git(&["switch", "-q", "-c", "upgrade"]).unwrap();
    repo.commit(
        &[("page.tsx", "upgrade\n"), ("link.tsx", "RouterLink\n")],
        "upgrade",
    );
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("page.tsx", "overhaul\n")], "overhaul");
    repo.write("café.txt", "local work\n");
    let _ = repo.git(&["merge", "upgrade"]);
    record_start_if_needed(&repo.root, DEFAULT_RECORD_AGE);
    repo.write("page.tsx", "resolved\n");
    repo.write("link.tsx", "fixed Link\n");

    let result = operation_abort::abort(&RepoOperation::Merge, &repo.root).unwrap();
    assert_eq!(
        result.outcome,
        Outcome::Reset {
            kept: vec!["café.txt".into()],
            exact: true
        }
    );
    assert_eq!(result.problem, None);
    assert_eq!(repo.read("café.txt"), "local work\n");
    assert_eq!(repo.read("link.tsx"), "Link\n");
}

#[test]
fn a_failed_put_back_still_offers_undo() {
    let repo = conflicted_merge();
    // A record naming a file that doesn't exist, as one read without -z
    // would ("caf\303\251.txt" in quotes).
    let git_dir = repo.git(&["rev-parse", "--absolute-git-dir"]).unwrap();
    let record = StartRecord {
        head: repo.head(),
        incoming: repo.git(&["rev-parse", "MERGE_HEAD"]).unwrap(),
        paths: vec!["\"caf\\303\\251.txt\"".into()],
        snapshot: None,
    };
    std::fs::write(record_path(&git_dir), serde_json::to_vec(&record).unwrap()).unwrap();
    repo.write("page.tsx", "resolved\n");
    repo.write("link.tsx", "fixed Link\n");

    let result = operation_abort::abort(&RepoOperation::Merge, &repo.root).unwrap();
    assert!(result.problem.is_some());
    assert!(result.undo.is_some());
    operation_abort::undo(&result, &RepoOperation::Merge, &repo.root).unwrap();
    assert_eq!(repo.read("notes.txt"), "local work\n");
    assert_eq!(repo.read("page.tsx"), "resolved\n");
    assert_eq!(repo.read("link.tsx"), "fixed Link\n");
}

#[test]
fn a_head_file_from_the_future_is_not_recorded() {
    let repo = conflicted_merge();
    let git_dir = repo.git(&["rev-parse", "--absolute-git-dir"]).unwrap();
    let head_file = std::fs::File::options()
        .write(true)
        .open(format!("{git_dir}/MERGE_HEAD"))
        .unwrap();
    head_file
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
    record_start_if_needed(&repo.root, DEFAULT_RECORD_AGE);
    assert_eq!(load_record(&git_dir), None);
}

#[test]
fn a_cherry_pick_is_aborted_the_same_way() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "1\n"), ("b.txt", "1\n")], "base");
    repo.git(&["switch", "-q", "-c", "side"]).unwrap();
    repo.commit(&[("a.txt", "side\n"), ("b.txt", "side\n")], "side");
    let pick = repo.head();
    repo.git(&["switch", "-q", "main"]).unwrap();
    repo.commit(&[("a.txt", "main\n")], "main");
    let _ = repo.git(&["cherry-pick", &pick]);
    assert_eq!(
        incoming_name(&repo.root).as_deref(),
        Some("CHERRY_PICK_HEAD")
    );
    repo.write("b.txt", "edited\n");
    let result = operation_abort::abort(&RepoOperation::CherryPick, &repo.root).unwrap();
    if result.outcome != Outcome::Aborted {
        assert_eq!(repo.read("b.txt"), "1\n");
    }
    assert_eq!(incoming_head(&repo.root), None);
    assert_eq!(repo.read("a.txt"), "main\n");
}
