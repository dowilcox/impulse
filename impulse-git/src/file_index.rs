//! A project's files for quick open, honoring .gitignore (the Swift
//! `FileIndex`).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::branches::repo_root;
use crate::cli::{self, RunOptions};
use crate::search::walk;

pub const DEFAULT_LIMIT: usize = 200_000;

/// Paths of files under `root`, relative to it: at most `limit`. Inside a
/// git repository git lists them (`ls-files`: tracked and untracked, not
/// ignored), which is fast even for large trees; elsewhere the folder is
/// walked with project search's ignore rules, and the paths come sorted.
/// Unless `trusted`, git runs nothing the repository's configuration names
/// (`cli::restricted_environment`); when that can't be made sure of, the
/// folder is walked instead.
pub fn files(root: &str, limit: usize, trusted: bool) -> Vec<String> {
    if repo_root(root).is_some() {
        let environment = if trusted {
            Some(HashMap::new())
        } else {
            cli::restricted_environment(root, &HashMap::new())
        };
        if let Some(environment) = environment {
            let listed = cli::run(
                &[
                    "ls-files",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "-z",
                ],
                root,
                RunOptions {
                    environment: Some(&environment),
                    timeout: Some(Duration::from_secs(15)),
                    ..Default::default()
                },
            );
            if let Ok(result) = listed {
                let mut seen = HashSet::new();
                let mut paths = Vec::new();
                for entry in result.stdout.split('\0').filter(|e| !e.is_empty()) {
                    // ls-files lists a file twice while it has unresolved
                    // conflicts.
                    if !seen.insert(entry) {
                        continue;
                    }
                    paths.push(entry.to_string());
                    if paths.len() >= limit {
                        break;
                    }
                }
                return paths;
            }
        }
    }

    let base = crate::raw::canonical_path(root).unwrap_or_else(|| root.to_string());
    let prefix = if base.ends_with('/') {
        base
    } else {
        format!("{base}/")
    };
    let mut paths = Vec::new();
    walk(root, &mut |path, _, is_directory| {
        if paths.len() >= limit {
            return false;
        }
        if !is_directory {
            if let Some(relative) = path.strip_prefix(&prefix) {
                paths.push(relative.to_string());
            }
        }
        true
    });
    paths.sort();
    paths
}
