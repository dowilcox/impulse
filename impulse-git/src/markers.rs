//! Gutter markers for a file against HEAD (the original Rust's
//! `get_file_diff`, shaped as `impulse_git_diff_markers` returned it): each
//! changed line is "added" or "modified", and a pure deletion leaves a
//! "deleted" marker on the line after it.

use std::collections::HashMap;
use std::path::Path;

use git2::{Patch, Status};
use serde::{Deserialize, Serialize};

use crate::raw::{
    canonical_path, file_size, open_repo, relative_path_components, trim_trailing_slash,
    workdir_raw,
};
use crate::MAX_DIFF_CONTENT_SIZE;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffMarker {
    /// 1-based.
    pub line: u32,
    /// "added", "modified" or "deleted".
    pub status: String,
}

struct FileDiffResult {
    changed_lines: HashMap<u32, &'static str>,
    deleted_lines: Vec<u32>,
}

/// Markers for a file (working tree against HEAD), sorted by line, then
/// status. `None` on error (not in a repository, unreadable); empty for a
/// clean file or one over 1 MB.
pub fn diff_markers(file_path: &str) -> Option<Vec<DiffMarker>> {
    let diff = file_diff(file_path)?;
    let mut markers: Vec<DiffMarker> = diff
        .changed_lines
        .into_iter()
        .map(|(line, status)| DiffMarker {
            line,
            status: status.into(),
        })
        .chain(diff.deleted_lines.into_iter().map(|line| DiffMarker {
            line,
            status: "deleted".into(),
        }))
        .collect();
    markers.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.status.cmp(&b.status)));
    Some(markers)
}

fn file_diff(file_path: &str) -> Option<FileDiffResult> {
    if file_size(file_path).is_some_and(|size| size > MAX_DIFF_CONTENT_SIZE) {
        return Some(FileDiffResult {
            changed_lines: HashMap::new(),
            deleted_lines: Vec::new(),
        });
    }
    let repo = open_repo(file_path).ok()?;
    // Bare: no markers.
    let repo_root = trim_trailing_slash(&workdir_raw(&repo)?);

    // Relative to the root, both canonical.
    let canonical_file = canonical_path(file_path).unwrap_or_else(|| file_path.to_string());
    let canonical_root = canonical_path(&repo_root).unwrap_or(repo_root);
    let rel =
        relative_path_components(&canonical_file, &canonical_root).filter(|r| !r.is_empty())?;
    let rel = rel.join("/");

    // Untracked or newly staged: every line is added.
    if repo
        .status_file(Path::new(&rel))
        .is_ok_and(|s| s.intersects(Status::WT_NEW | Status::INDEX_NEW))
    {
        return all_lines_added(file_path);
    }
    // No HEAD (an empty repository): every line is added.
    let Ok(head) = repo.head() else {
        return all_lines_added(file_path);
    };
    let tree = head.peel_to_tree().ok()?;

    let mut options = git2::DiffOptions::new();
    options.pathspec(&rel);
    let diff = repo
        .diff_tree_to_workdir(Some(&tree), Some(&mut options))
        .ok()?;

    let mut changed_lines = HashMap::new();
    let mut deleted_lines = Vec::new();
    for delta_index in 0..diff.deltas().len() {
        let Some(patch) = Patch::from_diff(&diff, delta_index).ok()? else {
            continue;
        };
        for hunk_index in 0..patch.num_hunks() {
            let (hunk, lines_in_hunk) = patch.hunk(hunk_index).ok()?;
            // A mixed hunk marks its first N additions (N = its deletions)
            // modified and the rest added; a pure deletion records an anchor.
            let mut hunk_added = Vec::new();
            let mut hunk_removed = 0usize;
            for line_index in 0..lines_in_hunk {
                let line = patch.line_in_hunk(hunk_index, line_index).ok()?;
                match line.origin() {
                    '+' => {
                        if let Some(lineno) = line.new_lineno() {
                            hunk_added.push(lineno);
                        }
                    }
                    '-' => hunk_removed += 1,
                    _ => {}
                }
            }
            if !hunk_added.is_empty() && hunk_removed > 0 {
                let modify_count = hunk_added.len().min(hunk_removed);
                for (offset, lineno) in hunk_added.into_iter().enumerate() {
                    changed_lines.insert(
                        lineno,
                        if offset < modify_count {
                            "modified"
                        } else {
                            "added"
                        },
                    );
                }
            } else if !hunk_added.is_empty() {
                for lineno in hunk_added {
                    changed_lines.insert(lineno, "added");
                }
            } else if hunk_removed > 0 {
                deleted_lines.push(hunk.new_start());
            }
        }
    }
    Some(FileDiffResult {
        changed_lines,
        deleted_lines,
    })
}

/// Every line of the file on disk as added; `None` when it isn't UTF-8.
fn all_lines_added(file_path: &str) -> Option<FileDiffResult> {
    let content = std::fs::read_to_string(file_path).ok()?;
    let changed_lines = (1..=content.lines().count() as u32)
        .map(|line| (line, "added"))
        .collect();
    Some(FileDiffResult {
        changed_lines,
        deleted_lines: Vec::new(),
    })
}
