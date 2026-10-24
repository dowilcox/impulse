//! Diffs in a scope (unstaged, staged, branch, commit, range, stash,
//! snapshot) for the review surface and hunk and line staging. The hunks the
//! UI shows and the patch text staging applies come from the same libgit2
//! diff, so hunk and line indices line up exactly (git's own diff can split
//! hunks differently because of its heuristics).

use std::collections::HashMap;

use git2::{
    Commit, Delta, Diff, DiffDelta, Index, IndexEntry, IndexTime, ObjectType, Oid, Patch,
    Repository, Tree, TreeWalkMode, TreeWalkResult,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::hunks::{hunk_header, line_content, DiffHunk, DiffLine, DiffLineKind};
use crate::language::language_for_path;
use crate::patch::BuildError;
use crate::path_validation::validate_rel_path_lexically;
use crate::raw::{head_tree, join_path, lossy, open_repo, with_repo, workdir};
use crate::snapshot::{line_stats, MAX_COUNTED_FILES};
use crate::types::{ChangeStatus, DiffScope, FileChange};
use crate::word_diff::assign_word_spans;
use crate::{
    GitError, MAX_DIFF_CONTENT_SIZE, MAX_DIFF_HUNKS, MAX_DIFF_LINE_LENGTH, MAX_DIFF_TOTAL_LINES,
};

/// Presentation options that also move hunk boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffOptions {
    pub context_lines: i32,
    pub ignore_whitespace: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        DiffOptions {
            context_lines: 3,
            ignore_whitespace: false,
        }
    }
}

/// One file's diff in a scope. `hunk_ids` parallels `hunks`: a hash of each
/// hunk's changes, to make sure a hunk the user acted on still exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub language: String,
    pub is_binary: bool,
    pub too_large: bool,
    pub truncated: bool,
    pub added: usize,
    pub removed: usize,
    pub hunks: Vec<DiffHunk>,
    pub hunk_ids: Vec<String>,
    /// The old side has no newline at the end of the file.
    pub old_missing_newline_at_end: bool,
    /// The new side has none.
    pub new_missing_newline_at_end: bool,
}

/// Files changed in `scope`, with line counts (skipped above
/// `MAX_COUNTED_FILES`).
pub fn changed_files_in_scope(
    repo_path: &str,
    scope: &DiffScope,
) -> Result<Vec<FileChange>, GitError> {
    let repo = open_repo(repo_path)?;
    let mut diff = make_diff(&repo, scope, &[], &DiffOptions::default())?;
    let _ = diff.find_similar(None);

    let count = diff.deltas().len();
    let stats = if count <= MAX_COUNTED_FILES {
        line_stats(&diff)
    } else {
        HashMap::new()
    };
    let mut changes = Vec::with_capacity(count);
    for delta in diff.deltas() {
        let new_path = delta.new_file().path_bytes().map(lossy);
        let old_path = delta.old_file().path_bytes().map(lossy);
        let Some(path) = new_path.or_else(|| old_path.clone()) else {
            continue;
        };
        let status = change_status(delta.status());
        let stat = stats.get(&path);
        changes.push(FileChange {
            old_path: if status == ChangeStatus::Renamed {
                old_path
            } else {
                None
            },
            status,
            added: stat.filter(|s| !s.is_binary).map(|s| s.added),
            removed: stat.filter(|s| !s.is_binary).map(|s| s.removed),
            is_binary: stat.map_or(delta.flags().is_binary(), |s| s.is_binary),
            path,
        });
    }
    Ok(changes)
}

/// The diff of one file in `scope`. `old_path` (renames) widens the pathspec
/// so the rename pairs up instead of showing as an add and a delete.
pub fn file_diff(
    repo_path: &str,
    path: &str,
    old_path: Option<&str>,
    scope: &DiffScope,
    options: &DiffOptions,
) -> Result<FileDiff, GitError> {
    let repo = open_repo(repo_path)?;
    let workdir = workdir(&repo)?;
    validate_rel_path_lexically(&workdir, path)?;
    let paths: Vec<&str> = std::iter::once(path).chain(old_path).collect();
    let mut diff = make_diff(&repo, scope, &paths, options)?;
    let _ = diff.find_similar(None);
    let language = language_for_path(&join_path(&workdir, path));

    let Some((index, delta)) = find_delta(&diff, path, old_path) else {
        return Ok(FileDiff {
            path: path.into(),
            old_path: old_path.map(Into::into),
            status: ChangeStatus::Modified,
            language,
            is_binary: false,
            too_large: false,
            truncated: false,
            added: 0,
            removed: 0,
            hunks: Vec::new(),
            hunk_ids: Vec::new(),
            old_missing_newline_at_end: false,
            new_missing_newline_at_end: false,
        });
    };
    Ok(extract_file_diff(&diff, index, &delta, path, language))
}

/// Unified-diff text of one file in a stageable scope (`Unstaged` or
/// `Staged`), for `git apply`. `None` when the file has no changes.
pub fn patch_text(
    repo_path: &str,
    path: &str,
    old_path: Option<&str>,
    scope: &DiffScope,
    options: &DiffOptions,
) -> Result<Option<String>, PatchTextError> {
    let repo = open_repo(repo_path)?;
    let workdir = workdir(&repo)?;
    validate_rel_path_lexically(&workdir, path)?;
    let paths: Vec<&str> = std::iter::once(path).chain(old_path).collect();
    let mut diff = make_diff(&repo, scope, &paths, options)?;
    let _ = diff.find_similar(None);
    let Some((index, _)) = find_delta(&diff, path, old_path) else {
        return Ok(None);
    };
    let Ok(Some(mut patch)) = Patch::from_diff(&diff, index) else {
        return Ok(None);
    };
    let Ok(buffer) = patch.to_buf() else {
        return Ok(None);
    };
    // Strictly UTF-8: a lossy decode would turn other encodings' bytes into
    // U+FFFD, which a partial patch then writes into the index or file.
    match std::str::from_utf8(&buffer) {
        Ok(text) => Ok(Some(text.to_string())),
        Err(_) => Err(PatchTextError::Build(BuildError::NotUtf8)),
    }
}

/// Why `patch_text` failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchTextError {
    Git(GitError),
    Build(BuildError),
}

impl From<GitError> for PatchTextError {
    fn from(error: GitError) -> Self {
        PatchTextError::Git(error)
    }
}

impl std::fmt::Display for PatchTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatchTextError::Git(error) => error.fmt(f),
            PatchTextError::Build(error) => error.fmt(f),
        }
    }
}

/// A revision (branch, tag, id, `HEAD~2`, ...) as a full commit id.
pub fn resolve_commit(repo_path: &str, revision: &str) -> Option<String> {
    resolve_commits(repo_path, &[revision]).remove(revision)
}

/// `resolve_commit` for several revisions, opening the repository once.
/// Revisions that don't resolve are left out.
pub fn resolve_commits(repo_path: &str, revisions: &[&str]) -> HashMap<String, String> {
    with_repo(repo_path, |repo| {
        let mut ids = HashMap::new();
        for revision in revisions {
            if ids.contains_key(*revision) {
                continue;
            }
            if let Ok(commit) = peel_commit(repo, revision) {
                ids.insert(revision.to_string(), commit.id().to_string());
            }
        }
        ids
    })
    .unwrap_or_default()
}

/// Remote-tracking branches (`origin/main`) whose tip is `commit` (a full
/// id), in one pass over the refs. `<remote>/HEAD` aliases are left out.
pub fn remote_branches_at(commit: &str, repo_path: &str) -> Vec<String> {
    if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Vec::new();
    }
    let Ok(wanted) = Oid::from_str(commit) else {
        return Vec::new();
    };
    with_repo(repo_path, |repo| {
        let Ok(references) = repo.references_glob("refs/remotes/*") else {
            return Vec::new();
        };
        let prefix = "refs/remotes/";
        references
            .flatten()
            .filter(|reference| {
                reference.kind() == Some(git2::ReferenceType::Direct)
                    && reference.target() == Some(wanted)
            })
            .map(|reference| lossy(reference.name_bytes()))
            .filter(|name| name.starts_with(prefix) && !name.ends_with("/HEAD"))
            .map(|name| name[prefix.len()..].to_string())
            .collect()
    })
    .unwrap_or_default()
}

/// The branch to compare against: the remote's HEAD (`origin/HEAD` to
/// `origin/main`), else a local `main`, `master`, `trunk` or `develop`.
pub fn default_base_branch(repo_path: &str) -> Option<String> {
    with_repo(repo_path, default_base_branch_in).ok().flatten()
}

pub(crate) fn default_base_branch_in(repo: &Repository) -> Option<String> {
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Some(target) = reference.symbolic_target_bytes() {
            let name = lossy(target);
            return Some(
                name.strip_prefix("refs/remotes/")
                    .map(str::to_string)
                    .unwrap_or(name),
            );
        }
    }
    ["main", "master", "trunk", "develop"]
        .into_iter()
        .find(|candidate| repo.find_branch(candidate, git2::BranchType::Local).is_ok())
        .map(str::to_string)
}

// MARK: Diff construction

/// A libgit2 diff for `scope`, limited to `pathspec` when it isn't empty.
pub(crate) fn make_diff<'r>(
    repo: &'r Repository,
    scope: &DiffScope,
    pathspec: &[&str],
    diff_options: &DiffOptions,
) -> Result<Diff<'r>, GitError> {
    let mut options = git2::DiffOptions::new();
    options
        .context_lines(diff_options.context_lines.max(0) as u32)
        .ignore_whitespace(diff_options.ignore_whitespace);
    if scope.includes_working_tree() {
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true);
    }
    if !pathspec.is_empty() {
        options.disable_pathspec_match(true);
        for path in pathspec {
            options.pathspec(path);
        }
    }
    let options = Some(&mut options);

    let diff = match scope {
        DiffScope::Uncommitted => {
            repo.diff_tree_to_workdir_with_index(head_tree(repo).as_ref(), options)
        }
        DiffScope::Unstaged => repo.diff_index_to_workdir(None, options),
        DiffScope::Staged => repo.diff_tree_to_index(head_tree(repo).as_ref(), None, options),
        DiffScope::Branch { base } => {
            let base_tree = merge_base_tree(repo, base)?;
            repo.diff_tree_to_workdir_with_index(Some(&base_tree), options)
        }
        DiffScope::Commit { sha } => {
            let commit = peel_commit(repo, sha)?;
            let new_tree = commit.tree()?;
            let parent_tree = if commit.parent_count() > 0 {
                commit.parent(0).ok().and_then(|parent| parent.tree().ok())
            } else {
                None
            };
            repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&new_tree), options)
        }
        DiffScope::Range { from, to } => {
            let from_tree = peel_commit(repo, from)?.tree()?;
            let to_tree = peel_commit(repo, to)?.tree()?;
            repo.diff_tree_to_tree(Some(&from_tree), Some(&to_tree), options)
        }
        DiffScope::Stash { index } => {
            let stash = peel_commit(repo, &format!("stash@{{{index}}}"))?;
            let stash_tree = stash.tree()?;
            let base = stash
                .parent(0)
                .map_err(|_| GitError::new("Stash has no base commit"))?;
            let base_tree = base.tree()?;
            // `--include-untracked` keeps the untracked files in a third
            // parent; without them, dropping a reviewed stash could lose
            // files never seen.
            match stash.parent(2).ok().filter(|_| stash.parent_count() >= 3) {
                Some(untracked) => {
                    let combined = combined_index(&stash_tree, &untracked.tree()?)?;
                    repo.diff_tree_to_index(Some(&base_tree), Some(&combined), options)
                }
                None => repo.diff_tree_to_tree(Some(&base_tree), Some(&stash_tree), options),
            }
        }
        DiffScope::Snapshot { from, to } => {
            let from_tree = peel_commit(repo, from)?.tree()?;
            match to {
                Some(to) => {
                    let to_tree = peel_commit(repo, to)?.tree()?;
                    repo.diff_tree_to_tree(Some(&from_tree), Some(&to_tree), options)
                }
                None => repo.diff_tree_to_workdir_with_index(Some(&from_tree), options),
            }
        }
    };
    Ok(diff?)
}

/// An in-memory index holding `tree`'s files plus `extra`'s.
fn combined_index(tree: &Tree, extra: &Tree) -> Result<Index, GitError> {
    let mut index = Index::new()?;
    index.read_tree(tree)?;
    let mut failure = None;
    extra.walk(TreeWalkMode::PreOrder, |root, entry| {
        if entry.kind() != Some(ObjectType::Blob) {
            return TreeWalkResult::Ok;
        }
        let mut path = root.as_bytes().to_vec();
        path.extend_from_slice(entry.name_bytes());
        let added = index.add(&IndexEntry {
            ctime: IndexTime::new(0, 0),
            mtime: IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: entry.filemode() as u32,
            uid: 0,
            gid: 0,
            file_size: 0,
            id: entry.id(),
            flags: 0,
            flags_extended: 0,
            path,
        });
        match added {
            Ok(()) => TreeWalkResult::Ok,
            Err(error) => {
                failure = Some(error);
                TreeWalkResult::Abort
            }
        }
    })?;
    match failure {
        Some(error) => Err(error.into()),
        None => Ok(index),
    }
}

pub(crate) fn peel_commit<'r>(
    repo: &'r Repository,
    revision: &str,
) -> Result<Commit<'r>, GitError> {
    let object = repo
        .revparse_single(revision)
        .map_err(|_| GitError(format!("Unknown revision '{revision}'")))?;
    object
        .peel_to_commit()
        .map_err(|_| GitError(format!("'{revision}' is not a commit")))
}

/// The tree of merge-base(base, HEAD).
fn merge_base_tree<'r>(repo: &'r Repository, base: &str) -> Result<Tree<'r>, GitError> {
    let base_commit = peel_commit(repo, base)?;
    let head_commit = peel_commit(repo, "HEAD")?;
    let merge_base = repo
        .merge_base(base_commit.id(), head_commit.id())
        .map_err(|_| GitError(format!("No common ancestor with '{base}'")))?;
    Ok(repo.find_commit(merge_base)?.tree()?)
}

// MARK: Extraction

fn find_delta<'d>(
    diff: &'d Diff,
    path: &str,
    old_path: Option<&str>,
) -> Option<(usize, DiffDelta<'d>)> {
    diff.deltas().enumerate().find(|(_, delta)| {
        let new = delta.new_file().path_bytes().map(lossy);
        let old = delta.old_file().path_bytes().map(lossy);
        new.as_deref() == Some(path)
            || old.as_deref() == Some(path)
            || (old_path.is_some() && old.as_deref() == old_path)
    })
}

pub(crate) fn change_status(status: Delta) -> ChangeStatus {
    match status {
        Delta::Added => ChangeStatus::Added,
        Delta::Deleted => ChangeStatus::Deleted,
        Delta::Renamed | Delta::Copied => ChangeStatus::Renamed,
        Delta::Typechange => ChangeStatus::TypeChanged,
        Delta::Untracked => ChangeStatus::Untracked,
        Delta::Conflicted => ChangeStatus::Conflicted,
        _ => ChangeStatus::Modified,
    }
}

fn extract_file_diff(
    diff: &Diff,
    index: usize,
    delta: &DiffDelta,
    path: &str,
    language: String,
) -> FileDiff {
    let status = change_status(delta.status());
    let old_path = if status == ChangeStatus::Renamed {
        delta.old_file().path_bytes().map(lossy)
    } else {
        None
    };
    let blank = |is_binary: bool, too_large: bool| FileDiff {
        path: path.into(),
        old_path: old_path.clone(),
        status,
        language: language.clone(),
        is_binary,
        too_large,
        truncated: false,
        added: 0,
        removed: 0,
        hunks: Vec::new(),
        hunk_ids: Vec::new(),
        old_missing_newline_at_end: false,
        new_missing_newline_at_end: false,
    };

    if delta.flags().is_binary() {
        return blank(true, false);
    }
    if delta.new_file().size() > MAX_DIFF_CONTENT_SIZE
        || delta.old_file().size() > MAX_DIFF_CONTENT_SIZE
    {
        return blank(false, true);
    }
    let Ok(Some(patch)) = Patch::from_diff(diff, index) else {
        return blank(true, false);
    };
    if patch.delta().flags().is_binary() {
        return blank(true, false);
    }

    let mut hunks = Vec::new();
    let mut hunk_ids = Vec::new();
    let mut added = 0;
    let mut removed = 0;
    let mut total_lines = 0;
    let mut truncated = false;
    let mut old_missing_newline = false;
    let mut new_missing_newline = false;

    for hunk_index in 0..patch.num_hunks() {
        if hunks.len() >= MAX_DIFF_HUNKS || total_lines >= MAX_DIFF_TOTAL_LINES {
            truncated = true;
            break;
        }
        let Ok((raw_hunk, lines_in_hunk)) = patch.hunk(hunk_index) else {
            break;
        };
        let mut lines = Vec::new();
        for line_index in 0..lines_in_hunk {
            let Ok(raw_line) = patch.line_in_hunk(hunk_index, line_index) else {
                continue;
            };
            // "\ No newline at end of file" markers are attributes of the
            // line before them, not lines of their own (as in the patch
            // text).
            let kind = match raw_line.origin() {
                '=' => {
                    old_missing_newline = true;
                    new_missing_newline = true;
                    continue;
                }
                // The old side lacked a final newline; the new side adds one.
                '>' => {
                    old_missing_newline = true;
                    continue;
                }
                // The new side lacks a final newline.
                '<' => {
                    new_missing_newline = true;
                    continue;
                }
                '+' => DiffLineKind::Added,
                '-' => DiffLineKind::Removed,
                _ => DiffLineKind::Context,
            };
            let content = line_content(&raw_line);
            if content.len() > MAX_DIFF_LINE_LENGTH {
                return blank(false, true);
            }
            match kind {
                DiffLineKind::Added => added += 1,
                DiffLineKind::Removed => removed += 1,
                DiffLineKind::Context => {}
            }
            lines.push(DiffLine {
                kind,
                old_lineno: raw_line.old_lineno(),
                new_lineno: raw_line.new_lineno(),
                content,
                spans: Vec::new(),
            });
            total_lines += 1;
            if total_lines >= MAX_DIFF_TOTAL_LINES {
                truncated = true;
            }
        }
        assign_word_spans(&mut lines);
        let hunk = DiffHunk {
            old_start: raw_hunk.old_start(),
            old_lines: raw_hunk.old_lines(),
            new_start: raw_hunk.new_start(),
            new_lines: raw_hunk.new_lines(),
            header: hunk_header(raw_hunk.header()),
            lines,
        };
        hunk_ids.push(hunk_identity(&hunk));
        hunks.push(hunk);
        if truncated {
            break;
        }
    }

    FileDiff {
        path: path.into(),
        old_path,
        status,
        language,
        is_binary: false,
        too_large: false,
        truncated,
        added,
        removed,
        hunks,
        hunk_ids,
        old_missing_newline_at_end: old_missing_newline,
        new_missing_newline_at_end: new_missing_newline,
    }
}

/// A hunk's stable identity: its changed lines, not its position (which
/// shifts when hunks above it are staged).
pub fn hunk_identity(hunk: &DiffHunk) -> String {
    change_identity(std::slice::from_ref(hunk))
}

/// The same identity over every changed line of `hunks`, however they're
/// grouped (more context merges neighboring hunks).
pub fn change_identity(hunks: &[DiffHunk]) -> String {
    let mut hasher = Sha256::new();
    for line in hunks.iter().flat_map(|h| &h.lines) {
        if line.kind == DiffLineKind::Context {
            continue;
        }
        hasher.update(if line.kind == DiffLineKind::Added {
            b"+"
        } else {
            b"-"
        });
        hasher.update(line.content.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
