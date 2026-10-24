//! Keeping paths inside a repository (the original Rust's
//! `util::validate_path_within_root` and `util::validate_rel_path_lexically`),
//! used by the discard operations and scoped diffs.

use std::path::Path;

use crate::raw::{canonical_path, has_path_prefix, join_path};
use crate::GitError;

/// `path` once canonicalized, when it's inside `root`. A path that doesn't
/// exist yet is its canonical parent plus its name.
pub fn validate_path_within_root(path: &str, root: &str) -> Result<String, GitError> {
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| GitError(format!("Failed to canonicalize root '{root}': {e}")))?
        .to_string_lossy()
        .into_owned();

    let canonical_target = match canonical_path(path) {
        Some(canonical) => canonical,
        None => {
            // Doesn't exist yet: canonicalize the parent and append the name.
            let (parent, name) = split_last_component(path);
            if parent.is_empty() {
                return Err(GitError(format!("Path '{path}' has no parent directory")));
            }
            if name.is_empty() || name == "/" {
                return Err(GitError(format!(
                    "Path '{path}' has no file name component"
                )));
            }
            let canonical_parent = std::fs::canonicalize(&parent)
                .map_err(|e| GitError(format!("Failed to canonicalize parent of '{path}': {e}")))?;
            join_path(&canonical_parent.to_string_lossy(), &name)
        }
    };
    if !has_path_prefix(&canonical_target, &canonical_root) {
        return Err(GitError(format!(
            "Path '{path}' is outside the workspace root '{root}'"
        )));
    }
    Ok(canonical_target)
}

/// `root` joined with the repo-relative `rel`, lexically normalized, without
/// touching the filesystem: `rel` may be neither absolute nor contain `..`.
pub fn validate_rel_path_lexically(root: &str, rel: &str) -> Result<String, GitError> {
    if rel.starts_with('/') {
        return Err(GitError(format!("Path '{rel}' must be relative")));
    }
    let mut normalized = root.to_string();
    for component in rel.split('/').filter(|c| !c.is_empty()) {
        match component {
            "." => continue,
            ".." => {
                return Err(GitError(format!(
                    "Path '{rel}' must not contain '..' components"
                )))
            }
            _ => normalized = join_path(&normalized, component),
        }
    }
    // Defensive: the lexical join can't have left the root.
    if !has_path_prefix(&normalized, root) {
        return Err(GitError(format!(
            "Path '{rel}' is outside the workspace root '{root}'"
        )));
    }
    Ok(normalized)
}

/// `NSString`'s `deletingLastPathComponent` and `lastPathComponent`.
fn split_last_component(path: &str) -> (String, String) {
    let trimmed = crate::raw::trim_trailing_slash(path);
    let p = Path::new(&trimmed);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| trimmed.clone());
    let parent = match p.parent() {
        Some(parent) if parent.as_os_str().is_empty() => String::new(),
        Some(parent) => parent.to_string_lossy().into_owned(),
        None => String::new(),
    };
    (parent, name)
}
