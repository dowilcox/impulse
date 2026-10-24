//! Blame and base content, commit history and branch details
//! (`BlameAndBaseTests`, `GitLogTests` and `BranchDetailsTests` in
//! `GitCoreTests.swift`).

mod common;

use std::collections::{HashMap, HashSet};

use common::{setup, unique, TempRepo};
use impulse_git::git_blame::blame;
use impulse_git::log::{self, LogScope};
use impulse_git::safety::{self, OPLOG_PREFIX};
use impulse_git::{base_content, operations, HistoryQuery};

fn entries(root: &str) -> Vec<log::LogEntry> {
    log::entries(
        root,
        &LogScope::Head,
        None,
        &HistoryQuery::default(),
        0,
        300,
    )
    .unwrap()
}

fn shas(entries: &[log::LogEntry]) -> Vec<String> {
    entries.iter().map(|e| e.sha.clone()).collect()
}

fn subjects(entries: &[log::LogEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.subject.as_str()).collect()
}

fn lines(text: &str) -> Vec<String> {
    text.split('\n')
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

// MARK: Blame and base content

#[test]
fn blame_attributes_lines_and_marks_uncommitted() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\ntwo\n")], "first lines");
    repo.write("a.txt", "one\ntwo\nthree\n");
    let lines = blame("a.txt", &repo.root);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[&1].summary, "first lines");
    assert_eq!(lines[&1].author, "Impulse Test");
    assert!(!lines[&2].is_uncommitted());
    assert!(lines[&3].is_uncommitted());
}

#[test]
fn base_content_is_the_index_version() {
    setup();
    let repo = TempRepo::create();
    repo.commit(
        &[(".gitignore", "*.log\n"), ("a.txt", "committed\n")],
        "commit",
    );
    repo.write("a.txt", "staged\n");
    repo.git(&["add", "a.txt"]).unwrap();
    repo.write("a.txt", "working\n");
    repo.write("new.txt", "fresh\n");
    repo.write("debug.log", "ignored\n");
    assert_eq!(
        base_content(&format!("{}/a.txt", repo.root)).as_deref(),
        Some("staged\n")
    );
    assert_eq!(
        base_content(&format!("{}/new.txt", repo.root)).as_deref(),
        Some("")
    );
    assert_eq!(base_content(&format!("{}/debug.log", repo.root)), None);
    let outside = std::env::temp_dir().join(format!("not-in-a-repo-{}.txt", unique()));
    assert_eq!(base_content(&outside.to_string_lossy()), None);
}

// MARK: History

#[test]
fn reads_history_with_parents_and_refs() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    repo.git(&["checkout", "-q", "-b", "topic"]).unwrap();
    repo.commit(&[("b.txt", "topic\n")], "commit");
    repo.git(&["checkout", "-q", "main"]).unwrap();
    repo.commit(&[("a.txt", "two\n")], "commit");
    repo.git(&["merge", "-q", "--no-ff", "-m", "Merge topic", "topic"])
        .unwrap();
    repo.git(&["tag", "v1"]).unwrap();

    let all = entries(&repo.root);
    assert_eq!(all.len(), 4);
    let merge = &all[0];
    assert_eq!(merge.subject, "Merge topic");
    assert_eq!(merge.parents.len(), 2);
    assert!(merge.refs.contains(&"HEAD -> main".to_string()));
    assert!(merge.refs.contains(&"tag: v1".to_string()));
    assert_eq!(merge.author, "Impulse Test");
    assert!(all.last().unwrap().parents.is_empty());

    // Paging.
    let page = log::entries(
        &repo.root,
        &LogScope::Head,
        None,
        &HistoryQuery::default(),
        1,
        2,
    )
    .unwrap();
    assert_eq!(shas(&page), shas(&all[1..=2]));

    // Path filter: only commits that touched b.txt.
    let file = log::entries(
        &repo.root,
        &LogScope::Head,
        Some("b.txt"),
        &HistoryQuery::default(),
        0,
        300,
    )
    .unwrap();
    assert_eq!(file.len(), 1);
    assert!(file[0].refs.contains(&"topic".to_string()));
}

#[test]
fn details_carry_the_whole_message() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "First");
    repo.commit(
        &[("a.txt", "two\n")],
        "Second change\n\nWhy it matters.\nMore detail.",
    );
    let head = repo.head();
    let details = log::details(&repo.root, "HEAD").unwrap();
    assert_eq!(details.sha, head);
    assert_eq!(
        details.parents,
        [repo.git(&["rev-parse", "HEAD~1"]).unwrap()]
    );
    assert_eq!(details.subject(), "Second change");
    assert_eq!(details.body(), "Why it matters.\nMore detail.");
    assert_eq!(details.author, "Impulse Test");
    assert_eq!(log::details(&repo.root, "nope"), None);
}

#[test]
fn divergence_splits_incoming_and_outgoing() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    assert!(
        log::divergence(&repo.root, 2000).0.is_empty(),
        "no upstream yet"
    );
    repo.git(&["branch", "up"]).unwrap();
    repo.git(&["branch", "--set-upstream-to=up"]).unwrap();
    repo.commit(&[("a.txt", "mine\n")], "commit");
    let mine = repo.head();
    repo.git(&["checkout", "-q", "up"]).unwrap();
    repo.commit(&[("b.txt", "theirs\n")], "commit");
    let theirs = repo.head();
    repo.git(&["checkout", "-q", "main"]).unwrap();

    let (outgoing, incoming) = log::divergence(&repo.root, 2000);
    assert_eq!(outgoing, HashSet::from([mine]));
    assert_eq!(incoming, HashSet::from([theirs]));
}

#[test]
fn query_limits_author_dates_and_path() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "First");
    repo.write("b.txt", "two\n");
    repo.git(&["add", "b.txt"]).unwrap();
    repo.git(&[
        "commit",
        "-q",
        "--author=Jane Doe <jane@example.com>",
        "-m",
        "Jane's change",
    ])
    .unwrap();

    let query = |text: &str| {
        log::entries(
            &repo.root,
            &LogScope::Head,
            None,
            &HistoryQuery::parse(text),
            0,
            300,
        )
        .unwrap()
    };
    assert_eq!(
        subjects(&query("author:JANE")),
        ["Jane's change"],
        "author matches case-insensitively"
    );
    assert_eq!(subjects(&query("path:a.txt")), ["First"]);
    // Commits are dated 2026-01-01 (UTC); days well away from it keep the
    // test independent of the local time zone.
    assert_eq!(query("since:2025-12-25").len(), 2);
    assert!(query("since:2026-01-05").is_empty());
    assert!(query("until:2025-12-25").is_empty());
    assert_eq!(query("until:2026-01-05").len(), 2);
}

#[test]
fn fork_point_from_the_default_branch() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "base");
    let base = repo.head();
    assert_eq!(
        log::fork_point(&repo.root, 2000),
        None,
        "on the default branch itself"
    );
    repo.git(&["checkout", "-q", "-b", "topic"]).unwrap();
    repo.commit(&[("b.txt", "work\n")], "topic work");
    let work = repo.head();
    repo.git(&["checkout", "-q", "main"]).unwrap();
    repo.commit(&[("c.txt", "later\n")], "main moves on");
    repo.git(&["checkout", "-q", "topic"]).unwrap();

    let fork = log::fork_point(&repo.root, 2000).unwrap();
    assert_eq!(fork.base, "main");
    assert_eq!(fork.sha, base);
    assert_eq!(fork.branch_only, HashSet::from([work]));
    assert_eq!(log::fork_point(&repo.root, 0), None, "too long to list");
}

#[test]
fn all_branches_skips_private_refs() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "commit");
    repo.write("a.txt", "dirty\n");
    safety::create("test", &repo.root, OPLOG_PREFIX).unwrap();
    repo.git(&["checkout", "-q", "-b", "side"]).unwrap();
    repo.git(&["checkout", "-q", "-"]).unwrap();
    let all = log::entries(
        &repo.root,
        &LogScope::All,
        None,
        &HistoryQuery::default(),
        0,
        300,
    )
    .unwrap();
    assert_eq!(all.len(), 1, "the snapshot commit isn't history");
}

#[test]
fn folder_history_matches_git_log() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("src/a.txt", "one\n")], "add a");
    repo.commit(&[("README.md", "readme\n")], "outside src");
    repo.commit(&[("src/b.txt", "two\n")], "add b");
    repo.git(&["mv", "src/a.txt", "src/c.txt"]).unwrap();
    repo.git(&["commit", "-q", "-m", "rename a"]).unwrap();
    repo.commit(&[("src/c.txt", "three\n")], "edit c");

    let history = |path: &str| {
        shas(
            &log::entries(
                &repo.root,
                &LogScope::Head,
                Some(path),
                &HistoryQuery::default(),
                0,
                300,
            )
            .unwrap(),
        )
    };
    // A folder (as the file tree passes it, without a trailing slash) gets
    // every commit that touched anything in it.
    let expected = lines(&repo.git(&["log", "--format=%H", "--", "src"]).unwrap());
    assert_eq!(expected.len(), 4);
    assert_eq!(history("src"), expected);
    assert_eq!(history("src/"), expected);

    // A deleted folder is still a folder.
    repo.git(&["rm", "-q", "-r", "src"]).unwrap();
    repo.git(&["commit", "-q", "-m", "drop src"]).unwrap();
    let deleted = lines(&repo.git(&["log", "--format=%H", "--", "src"]).unwrap());
    assert_eq!(history("src"), deleted);
}

#[test]
fn file_history_follows_renames() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("old.txt", "a\nb\nc\nd\n")], "add old");
    repo.git(&["mv", "old.txt", "new.txt"]).unwrap();
    repo.git(&["commit", "-q", "-m", "rename"]).unwrap();
    repo.commit(&[("new.txt", "a\nb\nc\nd\ne\n")], "edit new");
    let expected = lines(
        &repo
            .git(&["log", "--follow", "--format=%H", "--", "new.txt"])
            .unwrap(),
    );
    assert_eq!(expected.len(), 3);
    let history = log::entries(
        &repo.root,
        &LogScope::Head,
        Some("new.txt"),
        &HistoryQuery::default(),
        0,
        300,
    )
    .unwrap();
    assert_eq!(shas(&history), expected);
}

#[test]
fn branch_scope_lists_that_branch() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "base");
    repo.git(&["checkout", "-q", "-b", "topic"]).unwrap();
    repo.commit(&[("b.txt", "topic\n")], "topic work");
    repo.git(&["checkout", "-q", "main"]).unwrap();
    repo.commit(&[("a.txt", "two\n")], "main work");
    // A file named like the branch doesn't make the revision ambiguous.
    repo.write("topic", "not a branch\n");

    let topic = log::entries(
        &repo.root,
        &LogScope::Branch("topic".into()),
        None,
        &HistoryQuery::default(),
        0,
        300,
    )
    .unwrap();
    assert_eq!(subjects(&topic), ["topic work", "base"]);
    let expected = lines(&repo.git(&["log", "--format=%H", "topic", "--"]).unwrap());
    assert_eq!(shas(&topic), expected);
    assert_eq!(subjects(&entries(&repo.root)), ["main work", "base"]);
}

// MARK: Branch details

#[test]
fn reports_merged_and_current_branches() {
    setup();
    let repo = TempRepo::create();
    repo.commit(&[("a.txt", "one\n")], "first");
    repo.git(&["branch", "merged-topic"]).unwrap();
    repo.git(&["checkout", "-q", "-b", "open-topic"]).unwrap();
    repo.commit(&[("b.txt", "work\n")], "topic work");
    repo.git(&["checkout", "-q", "main"]).unwrap();

    let branches = operations::branch_details(&repo.root, Some("main"));
    let by_name: HashMap<&str, &operations::BranchInfo> =
        branches.iter().map(|b| (b.name.as_str(), b)).collect();
    assert_eq!(
        by_name.keys().copied().collect::<HashSet<_>>(),
        HashSet::from(["main", "merged-topic", "open-topic"])
    );
    assert!(by_name["main"].is_current);
    assert!(!by_name["main"].is_merged, "the base itself isn't 'merged'");
    assert!(by_name["merged-topic"].is_merged);
    assert!(!by_name["open-topic"].is_merged);
    assert_eq!(by_name["open-topic"].subject, "topic work");
    assert_eq!(by_name["open-topic"].upstream, None);
}
