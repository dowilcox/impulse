//! Parity with the golden fixtures generated from the original Rust git layer
//! (impulse-core's git.rs and filesystem.rs), shared with the Swift port
//! (`GitParityTests.swift`, `GitClientUnitTests`). The scenario repository is
//! rebuilt with the git CLI under a pinned author, committer and date, so
//! results are fully deterministic.

mod common;

use common::{expect_matches_fixture, first_difference, fixture, normalize, ScenarioRepo};
use impulse_git::path_validation::{validate_path_within_root, validate_rel_path_lexically};
use impulse_git::repo_cache::RepoCache;
use impulse_git::scoped_diff::file_diff;
use impulse_git::{
    all_statuses, branch_for_path, branches_for_path, changed_files, diff_markers, file_hunks,
    line_blame, repo_root, status_for_directory, DiffOptions, DiffScope,
};
use serde_json::json;

#[test]
fn changeset_matches_fixture() {
    let repo = ScenarioRepo::create();
    let change_set = changed_files(&repo.root).unwrap();
    expect_matches_fixture(&change_set, "changeset.json", &repo.root);
}

#[test]
fn hunks_match_fixtures() {
    let repo = ScenarioRepo::create();
    for (file, fixture_name) in [
        ("src/sample.rs", "hunks_src__sample.rs.json"),
        ("notes.txt", "hunks_notes.txt.json"),
        ("extra.txt", "hunks_extra.txt.json"),
    ] {
        let hunks = file_hunks(&repo.root, file)
            .unwrap_or_else(|| panic!("file_hunks({file}) returned None"));
        expect_matches_fixture(&hunks, fixture_name, &repo.root);
    }
}

#[test]
fn diff_markers_match_fixture() {
    let repo = ScenarioRepo::create();
    let markers = diff_markers(&format!("{}/src/sample.rs", repo.root)).unwrap();
    expect_matches_fixture(&markers, "markers_src__sample.rs.json", &repo.root);
}

#[test]
fn blame_matches_fixture() {
    let repo = ScenarioRepo::create();
    let blame = line_blame(&format!("{}/keep.md", repo.root), 1).unwrap();
    expect_matches_fixture(&blame, "blame_keep.md.json", &repo.root);
}

#[test]
fn branches_match_fixture() {
    let repo = ScenarioRepo::create();
    let actual = normalize(
        json!({
            "branch": branch_for_path(&repo.root),
            "branches": branches_for_path(&repo.root),
        }),
        &repo.root,
    );
    if let Some(difference) = first_difference(&actual, &fixture("branches.json"), "$") {
        panic!("branches.json: {difference}");
    }
}

#[test]
fn status_for_root_directory_matches_fixture() {
    let repo = ScenarioRepo::create();
    let status = status_for_directory(&repo.root, false).unwrap();
    expect_matches_fixture(&status, "status_root_dir.json", &repo.root);
}

#[test]
fn all_statuses_match_fixture() {
    let repo = ScenarioRepo::create();
    let statuses = all_statuses(&repo.root, false).unwrap();
    expect_matches_fixture(&statuses, "status_all.json", &repo.root);
}

#[test]
fn repo_root_discovery_and_cache() {
    let repo = ScenarioRepo::create();
    // From a nested path, twice (the second from the LRU cache).
    let src = format!("{}/src", repo.root);
    assert_eq!(repo_root(&src).as_deref(), Some(repo.root.as_str()));
    assert_eq!(repo_root(&src).as_deref(), Some(repo.root.as_str()));
    // A file is looked up by its folder.
    assert_eq!(
        repo_root(&format!("{}/src/sample.rs", repo.root)).as_deref(),
        Some(repo.root.as_str())
    );
    assert_eq!(repo_root("/"), None);
}

#[test]
fn repo_cache_evicts_least_recently_used() {
    let cache = RepoCache::new(2);
    cache.store("/r1", "/d1");
    cache.store("/r2", "/d2");
    // Touch d1 so d2 is the least recently used.
    assert_eq!(cache.root("/d1").as_deref(), Some("/r1"));
    cache.store("/r3", "/d3");
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.root("/d2"), None);
    assert_eq!(cache.root("/d1").as_deref(), Some("/r1"));
    assert_eq!(cache.root("/d3").as_deref(), Some("/r3"));
}

#[test]
fn path_validation_rejects_escapes() {
    let repo = ScenarioRepo::create();
    let root = repo.root.as_str();
    // Lexically: no `..` component and no absolute path.
    assert!(validate_rel_path_lexically(root, "../escape.txt").is_err());
    assert!(validate_rel_path_lexically(root, "a/../../escape.txt").is_err());
    assert!(validate_rel_path_lexically(root, "/etc/passwd").is_err());
    assert_eq!(
        validate_rel_path_lexically(root, "src/./sample.rs").unwrap(),
        format!("{root}/src/sample.rs")
    );

    // On disk: nothing that resolves outside the root.
    assert!(validate_path_within_root(&format!("{root}/../"), root).is_err());
    assert!(validate_path_within_root("/etc/passwd", root).is_err());
    assert_eq!(
        validate_path_within_root(&format!("{root}/keep.md"), root).unwrap(),
        format!("{root}/keep.md")
    );

    // Scoped diffs refuse traversal.
    assert!(file_diff(
        root,
        "../escape.txt",
        None,
        &DiffScope::Unstaged,
        &DiffOptions::default()
    )
    .is_err());
}
