//! Git status letters for the file tree (the original Rust's
//! `get_git_status_for_directory` and `get_all_git_statuses`): a file gets
//! its own status, a folder the highest-priority status among what's inside
//! it. Ignored files are marked but never mark their folders.
//!
//! Neither read writes the index unless `refresh_index` says so: writing
//! takes `index.lock`, which then collides with git commands the user or an
//! agent runs in a terminal at the same moment.

use std::collections::HashMap;

use git2::{Repository, Status, StatusOptions, StatusShow};

use crate::raw::{
    canonical_path, lexically_normalized, lossy, open_repo, path_components,
    relative_path_components, trim_trailing_slash, workdir,
};

/// Higher wins when a folder takes its contents' status.
pub fn status_priority(code: &str) -> u8 {
    match code {
        "C" => 6,
        "D" => 5,
        "A" => 4,
        "?" => 3,
        "R" => 2,
        "M" => 1,
        _ => 0,
    }
}

/// One letter for a status: I(gnored), C(onflict), A(dded, staged),
/// ?(untracked), D(eleted), R(enamed), M(odified).
pub fn status_code(status: Status) -> Option<&'static str> {
    if status.intersects(Status::IGNORED) {
        Some("I")
    } else if status.intersects(Status::CONFLICTED) {
        Some("C")
    } else if status.intersects(Status::WT_NEW | Status::INDEX_NEW) {
        Some(if status.intersects(Status::INDEX_NEW) {
            "A"
        } else {
            "?"
        })
    } else if status.intersects(Status::WT_DELETED | Status::INDEX_DELETED) {
        Some("D")
    } else if status.intersects(Status::INDEX_RENAMED | Status::WT_RENAMED) {
        Some("R")
    } else if status.intersects(
        Status::WT_MODIFIED
            | Status::INDEX_MODIFIED
            | Status::WT_TYPECHANGE
            | Status::INDEX_TYPECHANGE,
    ) {
        Some("M")
    } else {
        None
    }
}

fn collect_entries(
    repo: &Repository,
    options: &mut StatusOptions,
) -> Option<Vec<(String, &'static str)>> {
    let statuses = repo.statuses(Some(options)).ok()?;
    Some(
        statuses
            .iter()
            .filter_map(|entry| {
                let code = status_code(entry.status())?;
                Some((lossy(entry.path_bytes()), code))
            })
            .collect(),
    )
}

/// Status of what's directly in `path`, by name; a folder gets the
/// highest-priority status among its contents. Empty when `path` isn't in a
/// repository; `None` for a bare one or on error.
pub fn status_for_directory(path: &str, refresh_index: bool) -> Option<HashMap<String, String>> {
    // Canonical, so paths match the root libgit2 reports (/var is
    // /private/var on macOS).
    let dir_path = canonical_path(path).unwrap_or_else(|| lexically_normalized(path));
    let Ok(repo) = open_repo(&dir_path) else {
        return Some(HashMap::new());
    };
    let repo_root = workdir(&repo).ok()?;

    let mut options = StatusOptions::new();
    options
        .show(StatusShow::IndexAndWorkdir)
        .include_untracked(true);
    if refresh_index {
        options.update_index(true);
    }
    // Only the requested folder; at the root, every status.
    if let Some(rel) = relative_path_components(&dir_path, &repo_root).filter(|r| !r.is_empty()) {
        options.pathspec(format!("{}/", rel.join("/")));
    }
    let entries = collect_entries(&repo, &mut options)?;

    let dir_components = path_components(&dir_path);
    let root_components = path_components(&repo_root);
    let mut status_map: HashMap<String, String> = HashMap::new();
    for (rel_path, code) in entries {
        let trimmed = trim_trailing_slash(&rel_path);
        let abs: Vec<&str> = root_components
            .iter()
            .copied()
            .chain(path_components(&trimmed))
            .collect();
        // Only files within the requested folder.
        if abs.len() <= dir_components.len() || abs[..dir_components.len()] != dir_components[..] {
            continue;
        }
        let rel_to_dir = &abs[dir_components.len()..];
        if rel_to_dir.len() == 1 {
            status_map.insert(rel_to_dir[0].to_string(), code.to_string());
        } else {
            // In a subfolder: mark the folder, never as ignored.
            if code == "I" {
                continue;
            }
            merge(&mut status_map, rel_to_dir[0], code);
        }
    }
    Some(status_map)
}

/// Status of the whole repository: folder path, then name, then status
/// letter. Folders take the highest-priority status of their contents all
/// the way up to the root. Keys start with `root` as given, not as libgit2
/// spells the working directory.
pub fn all_statuses(
    root: &str,
    refresh_index: bool,
) -> Option<HashMap<String, HashMap<String, String>>> {
    let dir_path = canonical_path(root).unwrap_or_else(|| lexically_normalized(root));
    let Ok(repo) = open_repo(&dir_path) else {
        return Some(HashMap::new());
    };
    let repo_root = workdir(&repo).ok()?;

    let mut options = StatusOptions::new();
    options
        .show(StatusShow::IndexAndWorkdir)
        .include_untracked(true)
        .include_ignored(true);
    if refresh_index {
        options.update_index(true);
    }
    let entries = collect_entries(&repo, &mut options)?;

    let mut result: HashMap<String, HashMap<String, String>> = HashMap::new();
    let root_components = path_components(&repo_root);
    for (rel_path, code) in entries {
        let trimmed = trim_trailing_slash(&rel_path);
        let rel_components = path_components(&trimmed);
        let Some(file_name) = rel_components.last() else {
            continue;
        };
        let abs: Vec<&str> = root_components
            .iter()
            .copied()
            .chain(rel_components.iter().copied())
            .collect();
        let parent = &abs[..abs.len() - 1];
        merge(
            result.entry(format!("/{}", parent.join("/"))).or_default(),
            file_name,
            code,
        );

        // Ignored files don't mark their folders.
        if code == "I" {
            continue;
        }
        let mut child = parent;
        while child.len() > root_components.len()
            && child[..root_components.len()] == root_components[..]
        {
            let ancestor = &child[..child.len() - 1];
            let name = child[child.len() - 1];
            merge(
                result
                    .entry(format!("/{}", ancestor.join("/")))
                    .or_default(),
                name,
                code,
            );
            child = ancestor;
        }
    }

    // Keys under the caller's spelling of the root.
    let root_prefix = trim_trailing_slash(&repo_root);
    let caller_prefix = trim_trailing_slash(root);
    if root_prefix != caller_prefix {
        return Some(
            result
                .into_iter()
                .map(|(key, value)| match key.strip_prefix(&root_prefix) {
                    Some(rest) => (format!("{caller_prefix}{rest}"), value),
                    None => (key, value),
                })
                .collect(),
        );
    }
    Some(result)
}

fn merge(map: &mut HashMap<String, String>, name: &str, code: &str) {
    match map.get(name) {
        Some(existing) if status_priority(code) <= status_priority(existing) => {}
        _ => {
            map.insert(name.to_string(), code.to_string());
        }
    }
}
