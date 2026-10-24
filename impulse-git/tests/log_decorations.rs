//! Decorations read from ref tips, and the tips history starts from
//! (`GitLogDecorationTests.swift`).

mod common;

use common::{setup, TempRepo};
use impulse_git::log::{self, LogScope};
use impulse_git::HistoryQuery;

/// The decorations read from ref tips match what `git log` puts on the same
/// commits, before and after refs move.
#[test]
fn decorations_match_the_log() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "one");
    repo.git(&["checkout", "-q", "-b", "topic"]).unwrap();
    repo.commit(&[("b.txt", "topic\n")], "topic");
    repo.git(&["checkout", "-q", "main"]).unwrap();
    repo.commit(&[("a.txt", "two\n")], "two");
    repo.git(&["tag", "v1"]).unwrap();
    repo.git(&["tag", "-a", "v1-notes", "-m", "Annotated", "HEAD~1"])
        .unwrap();
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD~1"])
        .unwrap();
    repo.git(&["update-ref", "refs/impulse/snapshots/test", "HEAD"])
        .unwrap();

    let expect_match = || {
        let entries = log::entries(
            &repo.root,
            &LogScope::All,
            None,
            &HistoryQuery::default(),
            0,
            300,
        )
        .unwrap();
        let decorations = log::decorations(&repo.root).unwrap();
        for entry in &entries {
            assert_eq!(
                decorations.get(&entry.sha).cloned().unwrap_or_default(),
                entry.refs,
                "{}",
                entry.subject
            );
        }
        assert!(entries.iter().any(|e| !e.refs.is_empty()));
    };
    expect_match();

    // A new tag, a moved remote branch, a deleted branch, detached HEAD.
    repo.git(&["tag", "v2", "topic"]).unwrap();
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"])
        .unwrap();
    repo.git(&["branch", "-D", "topic"]).unwrap();
    repo.git(&["checkout", "-q", "--detach", "HEAD~1"]).unwrap();
    expect_match();
}

/// Tips change with the commits history starts from, not with refs added to
/// commits already in it.
#[test]
fn tips_follow_where_history_starts() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "one");
    repo.commit(&[("a.txt", "two\n")], "two");
    let head = repo.head();
    assert_eq!(
        log::tips(&repo.root, &LogScope::Head),
        Some(vec![head.clone()])
    );

    let all = log::tips(&repo.root, &LogScope::All).unwrap();
    repo.git(&["branch", "old", "HEAD~1"]).unwrap();
    assert_eq!(
        log::tips(&repo.root, &LogScope::Head),
        Some(vec![head]),
        "a branch elsewhere"
    );
    assert_ne!(
        log::tips(&repo.root, &LogScope::All),
        Some(all),
        "a new branch is a new start"
    );
    assert_eq!(
        log::tips(&repo.root, &LogScope::Branch("old".into())),
        Some(vec![repo.git(&["rev-parse", "HEAD~1"]).unwrap()])
    );

    repo.commit(&[("a.txt", "three\n")], "three");
    assert_eq!(
        log::tips(&repo.root, &LogScope::Head),
        Some(vec![repo.head()])
    );
    assert_eq!(
        log::tips(&repo.root, &LogScope::Branch("missing".into())),
        None
    );
}
