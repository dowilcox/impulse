//! Timing budgets against a large repository (`GitPerformanceTests.swift`).
//! Ignored by default; run them in release with the repository named:
//!
//! ```text
//! IMPULSE_PERF_REPO=/path/to/repo cargo test --release --test performance -- --ignored --nocapture
//! ```

use std::time::Instant;

use impulse_git::scoped_diff::{changed_files_in_scope, file_diff};
use impulse_git::{snapshot, DiffOptions, DiffScope};

/// "git snapshot refresh, 50k-file repo, warm < 150 ms".
#[test]
#[ignore]
fn warm_snapshot_of_a_large_repository() {
    let Ok(root) = std::env::var("IMPULSE_PERF_REPO") else {
        return;
    };
    // Cold: the index and caches.
    let _ = snapshot(&root, true);
    let mut samples: Vec<f64> = (0..5)
        .map(|_| {
            let start = Instant::now();
            assert!(snapshot(&root, true).is_some());
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    println!("git snapshot warm: median {median:.1} ms, samples {samples:?}");
    assert!(median < 150.0, "budget: < 150 ms warm");
}

/// The data behind the review's first paint (budget: first paint < 300 ms
/// for 200 files and 20k changed lines): the file list, then the first
/// screenful of diffs.
#[test]
#[ignore]
fn review_data_for_many_changed_files() {
    let Ok(root) = std::env::var("IMPULSE_PERF_REPO") else {
        return;
    };
    let start = Instant::now();
    let files = changed_files_in_scope(&root, &DiffScope::Unstaged).unwrap();
    let listed = start.elapsed().as_millis();
    for file in files.iter().take(5) {
        let _ = file_diff(
            &root,
            &file.path,
            None,
            &DiffScope::Unstaged,
            &DiffOptions::default(),
        );
    }
    let total = start.elapsed().as_millis();
    let lines: usize = files.iter().filter_map(|f| f.added).sum();
    println!(
        "review data: {} files, {lines} added lines; list {listed} ms, list + 5 diffs {total} ms",
        files.len()
    );
    assert!(total < 300);
}
