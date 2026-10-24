//! What's ignored in a working tree, and which folders hold a repository of
//! their own: what removing the folder (archiving a task worktree) deletes
//! for good, since safety snapshots only hold what git would track.
//!
//! Unless `trusted`, these reads take `cli::restricted_environment`, so git
//! runs nothing the repository's configuration names; when that can't be
//! made sure of, they answer `None` rather than read anyway.

use std::collections::HashMap;
use std::path::Path;

use crate::cli;
use crate::operations::{environment, git_in};

/// Ignored files and folders in the working tree, relative to `root`. A
/// folder that's ignored as a whole is one entry ending in "/"
/// (`node_modules/`). Empty when there are none or git fails
/// (`ignored_entry_list` tells those apart).
pub fn ignored_entries(root: &str, trusted: bool) -> Vec<String> {
    ignored_entry_list(root, trusted).unwrap_or_default()
}

/// `ignored_entries`, or `None` when git couldn't list them.
pub fn ignored_entry_list(root: &str, trusted: bool) -> Option<Vec<String>> {
    // `matching`: only what an ignore pattern names, not an untracked folder
    // that merely holds nothing but ignored files.
    let env = read_environment(root, trusted)?;
    status_entries(
        &["--ignored=matching", "--untracked-files=normal"],
        "!! ",
        root,
        &env,
    )
}

/// Folders holding a git repository of their own, relative to `root` and
/// ending in "/": untracked ones (a clone made inside the checkout), and
/// checked-out submodules and committed repositories (a gitlink whose folder
/// has a `.git`). A safety snapshot records one only as a commit id, and
/// removing a linked worktree takes its submodules' repositories with it
/// (they live in its own git folder), so their files and any commits not
/// pushed would go for good. `None` when git can't tell.
pub fn nested_repositories(root: &str, trusted: bool) -> Option<Vec<String>> {
    let env = read_environment(root, trusted)?;
    // With every untracked file listed, git still stops at another
    // repository's folder and lists the folder itself.
    let untracked = status_entries(&["--untracked-files=all"], "?? ", root, &env)?;
    let index = git_in(&["ls-files", "-s", "-z"], root, &env, 120).ok()?;
    // "160000 <id> <stage>\t<path>" is a gitlink (listed once per stage while
    // it's in conflict).
    let mut gitlinks: Vec<String> = Vec::new();
    for entry in index
        .stdout
        .split('\0')
        .filter(|e| e.starts_with("160000 "))
    {
        let Some((_, path)) = entry.split_once('\t') else {
            continue;
        };
        let folder = format!("{path}/");
        if Path::new(root).join(path).join(".git").exists() && !gitlinks.contains(&folder) {
            gitlinks.push(folder);
        }
    }
    Some(
        untracked
            .into_iter()
            .filter(|e| e.ends_with('/'))
            .chain(gitlinks)
            .collect(),
    )
}

/// The environment for a read in `root`; `None` when it can't be made safe.
fn read_environment(root: &str, trusted: bool) -> Option<HashMap<String, String>> {
    if trusted {
        Some(environment())
    } else {
        cli::restricted_environment(root, &environment())
    }
}

fn status_entries(
    options: &[&str],
    prefix: &str,
    root: &str,
    env: &HashMap<String, String>,
) -> Option<Vec<String>> {
    let mut args = vec!["status", "--porcelain=v1", "-z", "--ignore-submodules=all"];
    args.extend(options);
    let result = git_in(&args, root, env, 120).ok()?;
    Some(
        result
            .stdout
            .split('\0')
            .filter_map(|e| e.strip_prefix(prefix))
            .map(str::to_string)
            .collect(),
    )
}
