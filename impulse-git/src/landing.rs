//! Landing a task's branch (Finish Task): merging it into its base with a
//! merge commit and pushing the base, without touching the main checkout
//! (the Swift `TaskLanding`). The merge happens in a throwaway worktree on
//! the remote's base, so it works whatever state the main checkout is in.
//! Without a remote, the merge goes into the main checkout itself, and only
//! when that's safe.

use std::time::Duration;

use crate::operations::{self, git, git_with, Call, OperationError};
use crate::raw::unique_id;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The branch has no commits the base lacks.
    NothingToLand,
    /// Merging conflicts in these files; nothing was changed.
    Conflicts(Vec<String>),
    /// The remote had newer commits, also after merging them in once more.
    Rejected(String),
    /// The server turned the push down (a protected branch, a hook).
    Refused(String),
    /// The main checkout isn't on the base branch (no remote).
    NotOnBase {
        current: Option<String>,
    },
    /// The main checkout has uncommitted changes to tracked files (no
    /// remote).
    Uncommitted(usize),
    /// Untracked files in the main checkout that the merge would overwrite
    /// (no remote); nothing was changed.
    Untracked(Vec<String>),
    Git(OperationError),
}

impl Failure {
    pub fn message(&self) -> String {
        match self {
            Failure::NothingToLand => "The branch has no commits that aren't in its base.".into(),
            Failure::Conflicts(files) => {
                let named = if files.len() == 1 {
                    files[0].clone()
                } else {
                    format!("{} files", files.len())
                };
                format!("Merging conflicts in {named}. Nothing was changed.")
            }
            Failure::Rejected(_) => "The base kept moving on the remote while merging; try again.".into(),
            Failure::Refused(output) => format!("The server refused the push. {output}"),
            Failure::NotOnBase { current } => format!(
                "The main checkout is on {}, not the base branch.",
                current.as_deref().unwrap_or("a detached commit")
            ),
            Failure::Uncommitted(count) => format!(
                "The main checkout has {count} uncommitted file{}.",
                if *count == 1 { "" } else { "s" }
            ),
            Failure::Untracked(files) => format!(
                "The main checkout has untracked {}, which the merge would overwrite. Nothing was changed.",
                naming(files)
            ),
            Failure::Git(error) => error.message(),
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for Failure {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Landed {
    /// The merge commit.
    pub commit: String,
    /// The remote moved during the merge, so it was merged and pushed again.
    pub retried: bool,
    /// What the server said on the push (`remote:` lines included).
    pub output: Vec<String>,
}

/// git's own message for merging `branch` into `base`.
pub fn message(branch: &str, base: &str) -> String {
    if base == "main" || base == "master" {
        format!("Merge branch '{branch}'")
    } else {
        format!("Merge branch '{branch}' into {base}")
    }
}

/// Removes the throwaway worktree whatever happens.
struct Scratch<'a> {
    path: String,
    root: &'a str,
}

impl Drop for Scratch<'_> {
    fn drop(&mut self) {
        let _ = git(&["worktree", "remove", "--force", &self.path], self.root);
        let _ = std::fs::remove_dir_all(&self.path);
        let _ = git(&["worktree", "prune"], self.root);
    }
}

/// Merge `branch` into `remote`/`base` with a merge commit (`--no-ff`) in a
/// throwaway detached worktree and push it as `base`. When the push is
/// refused because the remote moved, fetch, merge again and push once more.
/// The worktree is removed whatever happens. Hooks run as for any merge and
/// push, except post-checkout for the throwaway folder.
pub fn merge_and_push(
    branch: &str,
    base: &str,
    remote: &str,
    root: &str,
    mut on_progress: Option<&mut dyn FnMut(&str)>,
) -> Result<Landed, Failure> {
    let target = format!("{remote}/{base}");
    if is_ancestor(branch, &target, root) {
        return Err(Failure::NothingToLand);
    }
    let path = std::env::temp_dir()
        .join(format!("impulse-land-{}", unique_id()))
        .to_string_lossy()
        .into_owned();
    git_with(
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "worktree",
            "add",
            "--detach",
            "--end-of-options",
            &path,
            &target,
        ],
        root,
        Call {
            timeout: Duration::from_secs(600),
            ..Default::default()
        },
    )
    .map_err(Failure::Git)?;
    let scratch = Scratch { path, root };

    let mut retried = false;
    loop {
        merge(branch, base, &scratch.path)?;
        let Ok(head) = git(&["rev-parse", "HEAD"], &scratch.path) else {
            return Err(Failure::Git(OperationError::Invalid(
                "The merge commit couldn't be read.".into(),
            )));
        };
        let mut output = Vec::new();
        let pushed = {
            let mut collect = |line: &str| {
                output.push(line.to_string());
                if let Some(callback) = on_progress.as_mut() {
                    callback(line);
                }
            };
            git_with(
                &[
                    "push",
                    "--progress",
                    remote,
                    &format!("HEAD:refs/heads/{base}"),
                ],
                &scratch.path,
                Call {
                    timeout: Duration::from_secs(600),
                    on_line: Some(&mut collect),
                    ..Default::default()
                },
            )
        };
        let error = match pushed {
            Ok(_) => {
                return Ok(Landed {
                    commit: head.stdout.trim().to_string(),
                    retried,
                    output,
                })
            }
            Err(error) => error,
        };
        let said = error
            .output()
            .map(str::to_string)
            .unwrap_or_else(|| error.message());
        if said.contains("[remote rejected]") {
            return Err(Failure::Refused(said));
        }
        if !is_rejection(&said) {
            return Err(Failure::Git(error));
        }
        if retried {
            return Err(Failure::Rejected(said));
        }
        retried = true;
        let progress = on_progress
            .as_mut()
            .map(|callback| &mut **callback as &mut dyn FnMut(&str));
        operations::fetch(
            false,
            Some(remote),
            &scratch.path,
            Duration::from_secs(600),
            progress,
        )
        .map_err(Failure::Git)?;
        git(&["checkout", "-q", "--detach", &target], &scratch.path).map_err(Failure::Git)?;
    }
}

/// Without a remote: merge `branch` into the main checkout at `root` with a
/// merge commit, when it's on `base` with no changes to tracked files.
/// Moving the base under changed files would make them look like they undo
/// the merge. Untracked files don't count: git refuses to overwrite one, and
/// nothing changes.
pub fn merge_locally(branch: &str, base: &str, root: &str) -> Result<Landed, Failure> {
    let current = operations::current_branch(root);
    if current.as_deref() != Some(base) {
        return Err(Failure::NotOnBase { current });
    }
    if let Ok(status) = git(&["status", "--porcelain", "--untracked-files=no"], root) {
        let count = status.stdout.split('\n').filter(|l| !l.is_empty()).count();
        if count > 0 {
            return Err(Failure::Uncommitted(count));
        }
    }
    if is_ancestor(branch, "HEAD", root) {
        return Err(Failure::NothingToLand);
    }
    merge(branch, base, root)?;
    let Ok(head) = git(&["rev-parse", "HEAD"], root) else {
        return Err(Failure::Git(OperationError::Invalid(
            "The merge commit couldn't be read.".into(),
        )));
    };
    Ok(Landed {
        commit: head.stdout.trim().to_string(),
        retried: false,
        output: Vec::new(),
    })
}

/// `git merge --no-ff` of `branch` in `folder`; on conflicts the merge is
/// aborted and the files are named.
fn merge(branch: &str, base: &str, folder: &str) -> Result<(), Failure> {
    let merged = git_with(
        &[
            "merge",
            "--no-ff",
            "--no-edit",
            "-m",
            &message(branch, base),
            "--end-of-options",
            branch,
        ],
        folder,
        Call {
            timeout: Duration::from_secs(600),
            ..Default::default()
        },
    );
    let Err(error) = merged else {
        return Ok(());
    };
    let conflicted: Vec<String> = git(&["diff", "--name-only", "-z", "--diff-filter=U"], folder)
        .map(|r| {
            r.stdout
                .split('\0')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let _ = git(&["merge", "--abort"], folder);
    let untracked = untracked_in_the_way(error.output().unwrap_or(""));
    if !untracked.is_empty() {
        return Err(Failure::Untracked(untracked));
    }
    Err(if conflicted.is_empty() {
        Failure::Git(error)
    } else {
        Failure::Conflicts(conflicted)
    })
}

/// The untracked files git refused to overwrite, from a merge's or a
/// fast-forward's output; empty when that isn't why it failed.
pub fn untracked_in_the_way(output: &str) -> Vec<String> {
    let lines: Vec<&str> = output.split('\n').collect();
    let Some(start) = lines
        .iter()
        .position(|l| l.contains("untracked working tree files would be overwritten"))
    else {
        return Vec::new();
    };
    lines[start + 1..]
        .iter()
        .take_while(|l| l.starts_with('\t'))
        .map(|l| l.trim_matches([' ', '\t']).to_string())
        .collect()
}

/// "notes.txt", or "notes.txt and 2 more files".
pub fn naming(files: &[String]) -> String {
    match files {
        [] => "files".into(),
        [only] => only.clone(),
        [first, rest @ ..] => format!(
            "{first} and {} more file{}",
            rest.len(),
            if rest.len() == 1 { "" } else { "s" }
        ),
    }
}

/// What deleting a landed task's branch did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchCleanup {
    /// The commit the branch was at when it was deleted (for Undo); `None`
    /// when it was kept.
    pub deleted_commit: Option<String>,
    /// It was deleted on the remote too.
    pub deleted_on_remote: bool,
    /// What was kept, and why ("fix has commits that aren't merged...").
    pub kept: Vec<String>,
}

/// Delete `branch`, and with `remote` its copy there, after checking again
/// that it's landed: what was merged when Finish or Archive Merged Tasks
/// looked may have moved since. The local branch goes only while its tip is
/// merged into one of `targets` (the merge commit, the base) and only while
/// it's still at that tip; the remote one only while its last fetched tip is
/// merged too and the remote still has it there. `trusted`: the
/// repository's merge drivers may run (`is_merged`).
pub fn delete_landed_branch(
    branch: &str,
    targets: &[&str],
    remote: Option<&str>,
    root: &str,
    trusted: bool,
) -> BranchCleanup {
    let mut cleanup = BranchCleanup::default();
    let Some(tip) = operations::resolve_ref(&format!("refs/heads/{branch}"), root) else {
        return cleanup;
    };
    let merged = |commit: &str| {
        targets
            .iter()
            .any(|target| operations::is_merged(commit, target, root, trusted))
    };
    if !merged(&tip) {
        cleanup.kept.push(format!(
            "{branch} has commits that aren't merged, so it was kept."
        ));
        return cleanup;
    }
    match operations::delete_branch_at(branch, &tip, root) {
        Ok(()) => {}
        Err(OperationError::Stale(_)) => {
            cleanup
                .kept
                .push(format!("{branch} moved meanwhile, so it was kept."));
            return cleanup;
        }
        Err(error) => {
            cleanup
                .kept
                .push(format!("{branch} was kept: {}", error.message()));
            return cleanup;
        }
    }
    cleanup.deleted_commit = Some(tip);
    let Some(remote) = remote else {
        return cleanup;
    };
    let Some(remote_tip) =
        operations::resolve_ref(&format!("refs/remotes/{remote}/{branch}"), root)
    else {
        return cleanup;
    };
    if !merged(&remote_tip) {
        cleanup.kept.push(format!(
            "{branch} on {remote} has commits that aren't merged, so it was kept there."
        ));
        return cleanup;
    }
    match operations::delete_remote_branch_at(branch, remote, &remote_tip, root) {
        Ok(()) => cleanup.deleted_on_remote = true,
        Err(error) => cleanup
            .kept
            .push(if error.output().unwrap_or("").contains("stale info") {
                format!("{branch} changed on {remote} since the last fetch, so it was kept there.")
            } else {
                format!(
                    "{branch} couldn't be deleted on {remote}: {}",
                    error.message()
                )
            }),
    }
    cleanup
}

/// Whether `commit` is already part of `other`.
pub fn is_ancestor(commit: &str, other: &str, root: &str) -> bool {
    git(&["merge-base", "--is-ancestor", commit, other], root).is_ok()
}

/// A push refused because the remote has commits this one lacks.
fn is_rejection(output: &str) -> bool {
    ["[rejected]", "non-fast-forward", "fetch first"]
        .iter()
        .any(|needle| output.contains(needle))
}
