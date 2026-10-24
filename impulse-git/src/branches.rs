//! The current branch, local branches and a repository's folders (the
//! original Rust's `get_git_branch`, `list_git_branches` and
//! `get_git_root`).

use git2::BranchType;

use crate::raw::{lossy, open_repo, trim_trailing_slash, workdir};

/// The current branch of the repository holding `path`, or the abbreviated
/// commit when HEAD is detached. `None` outside a repository or on an unborn
/// branch.
pub fn branch_for_path(path: &str) -> Option<String> {
    let repo = open_repo(path).ok()?;
    let head = repo.head().ok()?;
    if head.is_branch() {
        return Some(lossy(head.shorthand_bytes()));
    }
    // Detached: the abbreviated commit.
    let target = head.target()?;
    Some(target.to_string()[..7].to_string())
}

/// Local branch names, sorted bytewise. Empty outside a repository or on
/// error.
pub fn branches_for_path(path: &str) -> Vec<String> {
    let Ok(repo) = open_repo(path) else {
        return Vec::new();
    };
    let Ok(branches) = repo.branches(Some(BranchType::Local)) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for branch in branches {
        let Ok((branch, _)) = branch else {
            return Vec::new();
        };
        if let Ok(name) = branch.name_bytes() {
            names.push(lossy(name));
        }
    }
    names.sort();
    names
}

/// The git directory of the checkout holding `path` (no trailing slash):
/// `<root>/.git` for a normal checkout, `<common>/worktrees/<name>` for a
/// linked worktree, where that worktree's `index` and `HEAD` live.
pub fn git_directory(path: &str) -> Option<String> {
    let repo = open_repo(path).ok()?;
    Some(trim_trailing_slash(&repo.path().to_string_lossy()))
}

/// The shared git directory (no trailing slash). The same as
/// `git_directory` except in linked worktrees, where it is the main
/// repository's `.git` (refs, objects, config).
pub fn common_git_directory(path: &str) -> Option<String> {
    let repo = open_repo(path).ok()?;
    Some(trim_trailing_slash(&repo.commondir().to_string_lossy()))
}

/// The working directory root (no trailing slash) of the repository holding
/// `path`.
pub fn repo_root(path: &str) -> Option<String> {
    workdir(&open_repo(path).ok()?).ok()
}
