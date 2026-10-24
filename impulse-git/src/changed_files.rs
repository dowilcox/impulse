//! Every uncommitted change in a repository, HEAD against the index and
//! working tree, untracked files and renames included (the original Rust's
//! `list_changed_files`, `delta_line_stats` and `status_letter`). The JSON
//! keys are the original's (snake_case, explicit nulls), which the fixture
//! pins.

use git2::{Delta, Diff, DiffDelta, Patch, Repository};
use serde::{Deserialize, Serialize};

use crate::branches::branch_for_path;
use crate::raw::{file_size, head_tree, join_path, lossy, open_repo, workdir};
use crate::MAX_DIFF_CONTENT_SIZE;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// Repo-relative (the new path for renames).
    pub path: String,
    /// "A", "M", "D", "R", or "?" (untracked).
    pub status: String,
    /// The original path of a rename.
    pub old_path: Option<String>,
    pub added: u32,
    pub removed: u32,
    pub is_binary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSet {
    /// The working directory (no trailing slash).
    pub repo_root: String,
    /// The current branch; `None` when detached or unborn.
    pub branch: Option<String>,
    pub total_added: u32,
    pub total_removed: u32,
    pub files: Vec<ChangedFile>,
}

/// A delta's status letter.
pub fn status_letter(status: Delta) -> &'static str {
    match status {
        Delta::Untracked => "?",
        Delta::Added => "A",
        Delta::Deleted => "D",
        Delta::Renamed | Delta::Copied => "R",
        // Modified, type changes and everything else.
        _ => "M",
    }
}

/// (added, removed, binary) for one delta, with the original's size guards.
fn delta_line_stats(
    repo: &Repository,
    workdir: &str,
    diff: &Diff,
    index: usize,
    delta: &DiffDelta,
) -> (u32, u32, bool) {
    if delta.flags().is_binary() {
        return (0, 0, true);
    }
    // Size guard: the working-tree file and the HEAD blob.
    if let Some(path) = delta.new_file().path_bytes() {
        if file_size(&join_path(workdir, &lossy(path)))
            .is_some_and(|size| size > MAX_DIFF_CONTENT_SIZE)
        {
            return (0, 0, false);
        }
    }
    let old_id = delta.old_file().id();
    if !old_id.is_zero()
        && repo
            .find_blob(old_id)
            .is_ok_and(|blob| blob.size() as u64 > MAX_DIFF_CONTENT_SIZE)
    {
        return (0, 0, false);
    }
    let patch = match Patch::from_diff(diff, index) {
        Ok(Some(patch)) => patch,
        // libgit2 makes no patch for a binary delta.
        Ok(None) => return (0, 0, true),
        Err(_) => return (0, 0, false),
    };
    // The binary flag is only reliable once the patch content is computed.
    if patch.delta().flags().is_binary() {
        return (0, 0, true);
    }
    match patch.line_stats() {
        Ok((_, additions, deletions)) => (additions as u32, deletions as u32, false),
        Err(_) => (0, 0, false),
    }
}

/// The uncommitted changes of the repository holding `repo_path`; `None`
/// when it isn't in one, or on error.
pub fn changed_files(repo_path: &str) -> Option<ChangeSet> {
    let repo = open_repo(repo_path).ok()?;
    let repo_root = workdir(&repo).ok()?;
    let head = head_tree(&repo);

    let mut options = git2::DiffOptions::new();
    // Untracked content too, for their line counts and binary detection.
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    let mut diff = repo
        .diff_tree_to_workdir_with_index(head.as_ref(), Some(&mut options))
        .ok()?;
    // Renames, so they report "R" and the old path.
    diff.find_similar(None).ok()?;

    let mut files = Vec::new();
    let mut total_added = 0u64;
    let mut total_removed = 0u64;
    for (index, delta) in diff.deltas().enumerate() {
        let (added, removed, is_binary) = delta_line_stats(&repo, &repo_root, &diff, index, &delta);
        let new_path = delta.new_file().path_bytes().map(lossy);
        let old_path = delta.old_file().path_bytes().map(lossy);
        // The new path, else the old one (deletions).
        let Some(path) = new_path.or_else(|| old_path.clone()) else {
            continue;
        };
        let status = status_letter(delta.status());
        total_added += added as u64;
        total_removed += removed as u64;
        files.push(ChangedFile {
            path,
            status: status.into(),
            old_path: if status == "R" { old_path } else { None },
            added,
            removed,
            is_binary,
        });
    }

    Some(ChangeSet {
        repo_root,
        branch: branch_for_path(repo_path),
        total_added: total_added.min(u32::MAX as u64) as u32,
        total_removed: total_removed.min(u32::MAX as u64) as u32,
        files,
    })
}
