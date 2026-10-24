//! Project search: gitignore semantics through libgit2 (nested files,
//! negations), hidden and binary files skipped, substring matching with
//! character columns (`SearchTests.swift`).

mod common;

use std::collections::HashSet;

use common::{environment, run_git, temp_dir, RemoveOnDrop};
use impulse_git::search::{files_containing, search_contents, search_filenames};

/// A git repository with nested .gitignore files, a negation, and hidden and
/// binary files.
fn make_tree() -> (String, RemoveOnDrop) {
    let root = temp_dir("impulse-search");
    let cleanup = RemoveOnDrop(root.clone());
    run_git(&root, &["init", "-q", "-b", "main"], None, &environment()).unwrap();
    let write = |relative: &str, content: &[u8]| {
        let path = std::path::Path::new(&root).join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    };
    write(".gitignore", b"build/\n*.log\n!keep.log\n");
    write("src/needle_main.rs", b"fn main() { let needle = 1; }\n");
    write("src/.gitignore", b"generated.rs\n");
    write("src/generated.rs", b"// needle generated\n");
    write("build/needle_out.txt", b"needle output\n");
    write("debug.log", b"needle in log\n");
    write("keep.log", b"needle kept\n");
    write(".hidden/needle_hidden.txt", b"needle hidden\n");
    write("notes.txt", b"Needle here and nEEdle there\n");
    // A binary file holding the query's bytes and a NUL.
    write("blob.bin", b"needle\x00\x01\x02");
    (root, cleanup)
}

#[test]
fn filename_search_honors_gitignore_and_negations() {
    let (root, _cleanup) = make_tree();
    let mut names: Vec<String> = search_filenames(&root, "needle", 100)
        .into_iter()
        .map(|r| r.name)
        .collect();
    names.sort();
    // build/needle_out.txt is ignored (build/), needle_hidden is hidden.
    assert_eq!(names, ["needle_main.rs"]);
}

#[test]
fn content_search_honors_ignore_hidden_and_binary() {
    let (root, _cleanup) = make_tree();
    let results = search_contents(&root, "needle", 100, true);
    let files: HashSet<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert!(files.contains("needle_main.rs"));
    assert!(
        files.contains("keep.log"),
        "a negated ignore pattern is searchable"
    );
    assert!(
        !files.contains("generated.rs"),
        "the nested .gitignore applies"
    );
    assert!(!files.contains("needle_out.txt"), "build/ is ignored");
    assert!(!files.contains("debug.log"), "*.log is ignored");
    assert!(
        !files.contains("needle_hidden.txt"),
        "hidden folders are skipped"
    );
    assert!(!files.contains("blob.bin"), "binary files are skipped");
    assert!(!files.contains(".gitignore"), "hidden files are skipped");
}

#[test]
fn case_insensitive_columns_are_character_based() {
    let (root, _cleanup) = make_tree();
    // "Needle here and nEEdle there": two matches ignoring case.
    let results: Vec<_> = search_contents(&root, "needle", 100, false)
        .into_iter()
        .filter(|r| r.name == "notes.txt")
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(
        (results[0].column_start, results[0].column_end),
        (Some(0), Some(6))
    );
    assert_eq!(
        (results[1].column_start, results[1].column_end),
        (Some(16), Some(22))
    );
    assert_eq!(
        results[0].line_content.as_deref(),
        Some("Needle here and nEEdle there")
    );
}

#[test]
fn unicode_columns_count_characters_not_bytes() {
    let root = temp_dir("impulse-search-uni");
    let _cleanup = RemoveOnDrop(root.clone());
    std::fs::write(format!("{root}/uni.txt"), "héllo wörld target\n").unwrap();
    let results = search_contents(&root, "target", 10, true);
    assert_eq!(results.len(), 1);
    // "héllo wörld " is 12 characters (14 UTF-8 bytes).
    assert_eq!(
        (results[0].column_start, results[0].column_end),
        (Some(12), Some(18))
    );
}

#[test]
fn limit_stops_early() {
    let (root, _cleanup) = make_tree();
    assert_eq!(search_contents(&root, "needle", 1, false).len(), 1);
}

#[test]
fn files_containing_matches_the_content_search_without_a_limit() {
    let (root, _cleanup) = make_tree();
    for case_sensitive in [true, false] {
        let mut seen = HashSet::new();
        let expected: Vec<String> = search_contents(&root, "needle", usize::MAX, case_sensitive)
            .into_iter()
            .map(|r| r.path)
            .filter(|p| seen.insert(p.clone()))
            .collect();
        assert_eq!(files_containing(&root, "needle", case_sensitive), expected);
    }
    // notes.txt only matches ignoring case; a cap of one result still lists
    // every file.
    assert!(!files_containing(&root, "needle", true)
        .iter()
        .any(|p| p.ends_with("notes.txt")));
    assert_eq!(search_contents(&root, "needle", 1, false).len(), 1);
    assert_eq!(files_containing(&root, "needle", false).len(), 3);
    assert!(files_containing(&root, "", false).is_empty());
}
