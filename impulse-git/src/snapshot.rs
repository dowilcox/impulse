//! A repository at a glance: everything the git panel, status bar and chips
//! show, read in one pass with libgit2. It never writes the index, so it is
//! safe to run in the background while the user or an agent runs git in a
//! terminal.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use git2::{
    BranchType, Diff, ErrorCode, Patch, Repository, RepositoryState, Status, StatusOptions,
    StatusShow,
};

use crate::raw::{git_dir, lossy, open_repo, trim_trailing_slash, with_repo, workdir};
use crate::scoped_diff::{make_diff, DiffOptions};
use crate::types::{ChangeStatus, DiffScope, FileChange, RepoOperation};
use crate::MAX_DIFF_CONTENT_SIZE;

/// Untracked entries listed at most (a stray build folder shouldn't make the
/// panel unusable).
pub const MAX_UNTRACKED_ENTRIES: usize = 2_000;
/// Per-file line counts are skipped for a section with more files than this.
pub const MAX_COUNTED_FILES: usize = 400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSnapshot {
    /// The working tree root (no trailing slash).
    pub root: String,
    /// This checkout's git directory.
    pub git_dir: String,
    /// The shared git directory (differs from `git_dir` in linked worktrees).
    pub common_dir: String,
    /// The current branch's short name; `None` when HEAD is detached.
    pub branch: Option<String>,
    /// The full HEAD commit id; `None` when the branch is unborn.
    pub head_oid: Option<String>,
    pub is_detached: bool,
    pub is_unborn: bool,
    /// The upstream's short name ("origin/main"), when there is one.
    pub upstream: Option<String>,
    /// Commits on HEAD not on the upstream, and the other way round.
    pub ahead: usize,
    pub behind: usize,
    pub operation: Option<RepoOperation>,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub untracked: Vec<FileChange>,
    pub conflicted: Vec<FileChange>,
    pub stash_count: usize,
    /// The untracked list was cut off at `MAX_UNTRACKED_ENTRIES`.
    pub untracked_truncated: bool,
}

impl RepoSnapshot {
    pub fn has_changes(&self) -> bool {
        !self.staged.is_empty()
            || !self.unstaged.is_empty()
            || !self.untracked.is_empty()
            || !self.conflicted.is_empty()
    }

    fn distinct_paths<'a>(sections: impl IntoIterator<Item = &'a Vec<FileChange>>) -> usize {
        sections
            .into_iter()
            .flatten()
            .map(|change| change.path.as_str())
            .collect::<HashSet<_>>()
            .len()
    }

    /// Distinct changed paths across all sections.
    pub fn changed_file_count(&self) -> usize {
        Self::distinct_paths([
            &self.staged,
            &self.unstaged,
            &self.untracked,
            &self.conflicted,
        ])
    }

    /// Distinct changed paths git tracks: staged, unstaged and conflicted,
    /// not untracked. Git refuses on its own to overwrite an untracked file.
    pub fn tracked_change_count(&self) -> usize {
        Self::distinct_paths([&self.staged, &self.unstaged, &self.conflicted])
    }

    /// Known line counts across staged, unstaged and untracked files.
    pub fn total_added(&self) -> usize {
        [&self.staged, &self.unstaged, &self.untracked]
            .into_iter()
            .flatten()
            .filter_map(|c| c.added)
            .sum()
    }

    pub fn total_removed(&self) -> usize {
        [&self.staged, &self.unstaged]
            .into_iter()
            .flatten()
            .filter_map(|c| c.removed)
            .sum()
    }
}

/// A snapshot of the repository containing `path`, or `None` when it isn't
/// in a (non-bare) repository. Without `line_counts` its files carry no
/// added and removed counts, for callers that only need which files changed.
pub fn snapshot(path: &str, line_counts: bool) -> Option<RepoSnapshot> {
    let mut repo = open_repo(path).ok()?;
    let root = workdir(&repo).ok()?;
    let git_dir = git_dir(&repo);
    let common_dir = trim_trailing_slash(&repo.commondir().to_string_lossy());

    let mut branch = None;
    let mut head_oid = None;
    let is_detached = repo.head_detached().unwrap_or(false);
    let mut is_unborn = false;
    match repo.head() {
        Ok(head) => {
            if !is_detached {
                branch = Some(lossy(head.shorthand_bytes()));
            }
            head_oid = head.target().map(|oid| oid.to_string());
        }
        Err(error) if error.code() == ErrorCode::UnbornBranch => {
            // HEAD names a branch with no commits yet.
            is_unborn = true;
            if let Ok(reference) = repo.find_reference("HEAD") {
                if let Some(target) = reference.symbolic_target_bytes() {
                    let full = lossy(target);
                    branch = Some(
                        full.strip_prefix("refs/heads/")
                            .map(str::to_string)
                            .unwrap_or(full),
                    );
                }
            }
        }
        Err(_) => {}
    }

    let mut upstream = None;
    let mut ahead = 0;
    let mut behind = 0;
    if let (Some(name), false, false) = (&branch, is_detached, is_unborn) {
        if let Ok(local) = repo.find_branch(name, BranchType::Local) {
            if let Ok(up) = local.upstream() {
                upstream = Some(lossy(up.get().shorthand_bytes()));
                if let (Some(local_target), Some(up_target)) =
                    (local.get().target(), up.get().target())
                {
                    if let Ok((a, b)) = repo.graph_ahead_behind(local_target, up_target) {
                        ahead = a;
                        behind = b;
                    }
                }
            }
        }
    }

    let operation = read_operation(&repo, &git_dir);
    let sections = read_status(&repo, line_counts);
    let stash_count = count_stashes(&mut repo);

    Some(RepoSnapshot {
        root,
        git_dir,
        common_dir,
        branch,
        head_oid,
        is_detached,
        is_unborn,
        upstream,
        ahead,
        behind,
        operation,
        staged: sections.staged,
        unstaged: sections.unstaged,
        untracked: sections.untracked,
        conflicted: sections.conflicted,
        stash_count,
        untracked_truncated: sections.untracked_truncated,
    })
}

/// The operation in progress in the repository containing `repo_path`.
pub fn current_operation(repo_path: &str) -> Option<RepoOperation> {
    with_repo(repo_path, |repo| read_operation(repo, &git_dir(repo)))
        .ok()
        .flatten()
}

fn read_operation(repo: &Repository, git_dir: &str) -> Option<RepoOperation> {
    let read_number = |relative: &str| -> Option<usize> {
        std::fs::read_to_string(Path::new(git_dir).join(relative))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    match repo.state() {
        RepositoryState::Merge => Some(RepoOperation::Merge),
        RepositoryState::Revert | RepositoryState::RevertSequence => Some(RepoOperation::Revert),
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => {
            Some(RepoOperation::CherryPick)
        }
        RepositoryState::Bisect => Some(RepoOperation::Bisect),
        RepositoryState::RebaseMerge | RepositoryState::RebaseInteractive => {
            Some(RepoOperation::Rebase {
                step: read_number("rebase-merge/msgnum"),
                total: read_number("rebase-merge/end"),
            })
        }
        RepositoryState::Rebase => Some(RepoOperation::Rebase {
            step: read_number("rebase-apply/next"),
            total: read_number("rebase-apply/last"),
        }),
        RepositoryState::ApplyMailbox | RepositoryState::ApplyMailboxOrRebase => {
            Some(RepoOperation::ApplyMailbox)
        }
        RepositoryState::Clean => None,
    }
}

// MARK: Status sections

#[derive(Default)]
struct StatusSections {
    staged: Vec<FileChange>,
    unstaged: Vec<FileChange>,
    untracked: Vec<FileChange>,
    conflicted: Vec<FileChange>,
    untracked_truncated: bool,
}

fn read_status(repo: &Repository, line_counts: bool) -> StatusSections {
    let mut options = StatusOptions::new();
    options
        .show(StatusShow::IndexAndWorkdir)
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .sort_case_sensitively(true);
    let Ok(statuses) = repo.statuses(Some(&mut options)) else {
        return StatusSections::default();
    };

    let path_of = |delta: Option<&git2::DiffDelta>| -> Option<String> {
        let delta = delta?;
        delta
            .new_file()
            .path_bytes()
            .or_else(|| delta.old_file().path_bytes())
            .map(lossy)
    };
    let mut sections = StatusSections::default();
    for entry in statuses.iter() {
        let status = entry.status();
        if status.intersects(Status::CONFLICTED) {
            if let Some(path) = path_of(
                entry
                    .index_to_workdir()
                    .or_else(|| entry.head_to_index())
                    .as_ref(),
            ) {
                sections
                    .conflicted
                    .push(FileChange::new(path, ChangeStatus::Conflicted));
            }
            continue;
        }

        let index_bits = Status::INDEX_NEW
            | Status::INDEX_MODIFIED
            | Status::INDEX_DELETED
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE;
        if status.intersects(index_bits) {
            if let Some(delta) = entry.head_to_index() {
                if let Some(path) = path_of(Some(&delta)) {
                    let change_status = if status.intersects(Status::INDEX_NEW) {
                        ChangeStatus::Added
                    } else if status.intersects(Status::INDEX_DELETED) {
                        ChangeStatus::Deleted
                    } else if status.intersects(Status::INDEX_RENAMED) {
                        ChangeStatus::Renamed
                    } else if status.intersects(Status::INDEX_TYPECHANGE) {
                        ChangeStatus::TypeChanged
                    } else {
                        ChangeStatus::Modified
                    };
                    let old_path = if change_status == ChangeStatus::Renamed {
                        delta.old_file().path_bytes().map(lossy)
                    } else {
                        None
                    };
                    sections.staged.push(FileChange {
                        old_path,
                        ..FileChange::new(path, change_status)
                    });
                }
            }
        }

        if status.intersects(Status::WT_NEW) {
            if sections.untracked.len() >= MAX_UNTRACKED_ENTRIES {
                sections.untracked_truncated = true;
            } else if let Some(path) = path_of(entry.index_to_workdir().as_ref()) {
                sections
                    .untracked
                    .push(FileChange::new(path, ChangeStatus::Untracked));
            }
            continue;
        }

        let worktree_bits =
            Status::WT_MODIFIED | Status::WT_DELETED | Status::WT_TYPECHANGE | Status::WT_RENAMED;
        if status.intersects(worktree_bits) {
            if let Some(path) = path_of(entry.index_to_workdir().as_ref()) {
                let change_status = if status.intersects(Status::WT_DELETED) {
                    ChangeStatus::Deleted
                } else if status.intersects(Status::WT_TYPECHANGE) {
                    ChangeStatus::TypeChanged
                } else {
                    ChangeStatus::Modified
                };
                sections.unstaged.push(FileChange::new(path, change_status));
            }
        }
    }

    // Line counts per section, unless the section is huge.
    if !line_counts {
        return sections;
    }
    if sections.staged.len() <= MAX_COUNTED_FILES {
        sections.staged = with_line_counts(
            std::mem::take(&mut sections.staged),
            repo,
            &DiffScope::Staged,
        );
    }
    if sections.unstaged.len() <= MAX_COUNTED_FILES {
        sections.unstaged = with_line_counts(
            std::mem::take(&mut sections.unstaged),
            repo,
            &DiffScope::Unstaged,
        );
    }
    if sections.untracked.len() <= MAX_COUNTED_FILES {
        sections.untracked = with_line_counts(
            std::mem::take(&mut sections.untracked),
            repo,
            &DiffScope::Unstaged,
        );
    }
    sections
}

/// Added and removed counts from the scope's diff (binary-aware). Counts for
/// staged and unstaged files are kept by what they depend on (index blob,
/// working-copy size and mtime, HEAD), so a refresh only re-diffs files that
/// changed since the last one.
fn with_line_counts(
    changes: Vec<FileChange>,
    repo: &Repository,
    scope: &DiffScope,
) -> Vec<FileChange> {
    if changes.is_empty() {
        return changes;
    }
    let signatures = line_count_signatures(&changes, repo, scope);
    let cache_scope = format!("{}|{scope:?}", workdir(repo).unwrap_or_default());
    let mut stats: HashMap<String, LineStat> = HashMap::new();
    let mut missing = Vec::new();
    for change in &changes {
        let cached = signatures
            .get(&change.path)
            .and_then(|signature| LINE_COUNTS.get(&cache_scope, &change.path, signature));
        match cached {
            Some(stat) => {
                stats.insert(change.path.clone(), stat);
            }
            None => missing.push(change),
        }
    }
    if !missing.is_empty() {
        let paths: Vec<&str> = missing
            .iter()
            .flat_map(|c| std::iter::once(c.path.as_str()).chain(c.old_path.as_deref()))
            .collect();
        if let Ok(mut diff) = make_diff(repo, scope, &paths, &DiffOptions::default()) {
            // Pair renames, or a renamed file counts as all-new lines.
            let _ = diff.find_similar(None);
            for (path, stat) in line_stats(&diff) {
                if let Some(signature) = signatures.get(&path) {
                    LINE_COUNTS.set(&cache_scope, &path, signature, stat);
                }
                stats.insert(path, stat);
            }
        }
    }
    changes
        .into_iter()
        .map(|change| match stats.get(&change.path) {
            Some(stat) => FileChange {
                added: (!stat.is_binary).then_some(stat.added),
                removed: (!stat.is_binary).then_some(stat.removed),
                is_binary: stat.is_binary,
                ..change
            },
            None => change,
        })
        .collect()
}

/// What each file's counts depend on, by path; none for scopes that aren't
/// cached or files that can't be described (renames).
fn line_count_signatures(
    changes: &[FileChange],
    repo: &Repository,
    scope: &DiffScope,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if !matches!(scope, DiffScope::Staged | DiffScope::Unstaged) {
        return out;
    }
    let Ok(index) = repo.index() else {
        return out;
    };
    let mut head = "unborn".to_string();
    if *scope == DiffScope::Staged {
        if let Ok(oid) = repo.refname_to_id("HEAD") {
            head = oid.to_string();
        }
    }
    let workdir = workdir(repo).unwrap_or_default();
    for change in changes.iter().filter(|c| c.old_path.is_none()) {
        let blob = index
            .get_path(Path::new(&change.path), 0)
            .map(|entry| entry.id.to_string())
            .unwrap_or_else(|| "-".into());
        if *scope == DiffScope::Staged {
            out.insert(change.path.clone(), format!("{head}|{blob}"));
        } else {
            let Ok(meta) = std::fs::symlink_metadata(Path::new(&workdir).join(&change.path)) else {
                continue;
            };
            out.insert(
                change.path.clone(),
                format!("{blob}|{}", stat_signature(&meta)),
            );
        }
    }
    out
}

#[cfg(unix)]
fn stat_signature(meta: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        "{}|{}.{}|{}",
        meta.size(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ino()
    )
}

#[cfg(not(unix))]
fn stat_signature(meta: &std::fs::Metadata) -> String {
    format!("{}|{:?}", meta.len(), meta.modified().ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineStat {
    pub added: usize,
    pub removed: usize,
    pub is_binary: bool,
}

/// Line stats for every delta in `diff` (by new-side path), skipping files
/// over the diff size limit.
pub(crate) fn line_stats(diff: &Diff) -> HashMap<String, LineStat> {
    let binary = LineStat {
        added: 0,
        removed: 0,
        is_binary: true,
    };
    let mut result = HashMap::new();
    for (index, delta) in diff.deltas().enumerate() {
        let Some(path) = delta
            .new_file()
            .path_bytes()
            .or_else(|| delta.old_file().path_bytes())
            .map(lossy)
        else {
            continue;
        };
        if delta.flags().is_binary() {
            result.insert(path, binary);
            continue;
        }
        if delta.new_file().size() > MAX_DIFF_CONTENT_SIZE
            || delta.old_file().size() > MAX_DIFF_CONTENT_SIZE
        {
            continue;
        }
        let Ok(Some(patch)) = Patch::from_diff(diff, index) else {
            result.insert(path, binary);
            continue;
        };
        if patch.delta().flags().is_binary() {
            result.insert(path, binary);
            continue;
        }
        let (_, added, removed) = patch.line_stats().unwrap_or((0, 0, 0));
        result.insert(
            path,
            LineStat {
                added,
                removed,
                is_binary: false,
            },
        );
    }
    result
}

fn count_stashes(repo: &mut Repository) -> usize {
    let mut count = 0;
    let _ = repo.stash_foreach(|_, _, _| {
        count += 1;
        true
    });
    count
}

/// Line counts by (repository and scope, path), valid while the signature
/// (blob, size, mtime, ...) is unchanged. Bounded; cleared when full.
struct LineCountCache {
    entries: Mutex<HashMap<String, (String, LineStat)>>,
}

const LINE_COUNT_LIMIT: usize = 20_000;

static LINE_COUNTS: LazyLock<LineCountCache> = LazyLock::new(|| LineCountCache {
    entries: Mutex::new(HashMap::new()),
});

impl LineCountCache {
    fn get(&self, scope: &str, path: &str, signature: &str) -> Option<LineStat> {
        let entries = self.entries.lock().unwrap();
        let (stored, stat) = entries.get(&format!("{scope}\0{path}"))?;
        (stored == signature).then_some(*stat)
    }

    fn set(&self, scope: &str, path: &str, signature: &str, stat: LineStat) {
        let mut entries = self.entries.lock().unwrap();
        if entries.len() >= LINE_COUNT_LIMIT {
            entries.clear();
        }
        entries.insert(format!("{scope}\0{path}"), (signature.to_string(), stat));
    }
}
