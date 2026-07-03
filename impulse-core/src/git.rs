use lru::LruCache;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

/// Cache mapping directory paths to their discovered git repo root.
/// This avoids repeated `Repository::discover()` calls which walk up the
/// directory tree on every invocation.
static REPO_ROOT_CACHE: std::sync::LazyLock<Mutex<LruCache<PathBuf, PathBuf>>> =
    std::sync::LazyLock::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap())));

/// Open a git repository for the given path, using a cached repo-root lookup.
/// Falls back to `Repository::discover()` on cache miss and caches the result.
pub fn open_repo(path: &Path) -> Result<git2::Repository, String> {
    // Try parent directory for files (most lookups are for files, not directories)
    let lookup_dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    let lookup_dir_buf = lookup_dir.to_path_buf();

    // Check cache
    {
        let mut cache = REPO_ROOT_CACHE.lock();
        if let Some(root) = cache.get(&lookup_dir_buf) {
            if let Ok(repo) = git2::Repository::open(root) {
                return Ok(repo);
            }
            // Root no longer valid, fall through to re-discover
        }
    }

    // Discover and cache
    let repo = git2::Repository::discover(path).map_err(|e| format!("Not a git repo: {}", e))?;
    let root = repo.workdir().ok_or("Bare repository")?.to_path_buf();

    {
        let mut cache = REPO_ROOT_CACHE.lock();
        cache.put(lookup_dir_buf, root);
    }

    Ok(repo)
}

/// Status of a line relative to HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DiffLineStatus {
    Added,
    Modified,
    Unchanged,
}

/// Diff information for a file, mapping line numbers to their status.
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// Map of 1-based line numbers to their diff status. Only changed lines are included.
    pub changed_lines: std::collections::HashMap<u32, DiffLineStatus>,
    /// 1-based line numbers where pure-deletion hunks anchor (the line after the deletion).
    pub deleted_lines: Vec<u32>,
}

/// Blame information for a single line.
#[derive(Debug, Clone)]
pub struct BlameInfo {
    pub author: String,
    pub date: String,
    pub commit_hash: String,
    pub summary: String,
}

/// Maximum file/blob size (bytes) for which we read full diff contents.
const MAX_DIFF_CONTENT_SIZE: u64 = 1_048_576;

/// Maximum single-line length (bytes) before a file is treated as too complex to
/// diff inline. Minified/generated files (bundles, lockfiles, single-line JSON)
/// can sit under `MAX_DIFF_CONTENT_SIZE` yet contain one gigantic line; feeding
/// those to the WebView renderer hangs it, and they aren't human-reviewable
/// line-by-line anyway.
const MAX_DIFF_LINE_LENGTH: usize = 20_000;

/// Maximum number of hunks emitted per file before the diff is marked truncated.
const MAX_DIFF_HUNKS: usize = 1_500;

/// Maximum number of diff lines emitted per file before the diff is marked
/// truncated. Bounds the DOM the WebView must build for a pathological diff.
const MAX_DIFF_TOTAL_LINES: usize = 30_000;

/// Skip intra-line word-diffing when the combined old+new line length (UTF-16
/// units) exceeds this — the quadratic word diff isn't worth it on long lines.
const MAX_WORD_DIFF_LINE_LEN: usize = 2_000;

/// A single changed file in the working tree relative to HEAD (index + worktree).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFile {
    /// Repo-relative path of the file (new path for renames).
    pub path: String,
    /// Status letter: "A" (added/index-new), "M" (modified), "D" (deleted),
    /// "R" (renamed), or "?" (untracked).
    pub status: String,
    /// Original repo-relative path for renames; `None` otherwise.
    pub old_path: Option<String>,
    /// Lines added in this file.
    pub added: u32,
    /// Lines removed in this file.
    pub removed: u32,
    /// Whether the file is binary (no textual diff).
    pub is_binary: bool,
}

/// The complete set of uncommitted changes in a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeSet {
    /// Absolute path of the repository working directory root.
    pub repo_root: String,
    /// Current branch name, or `None` if detached/unavailable.
    pub branch: Option<String>,
    /// Total lines added across all files.
    pub total_added: u32,
    /// Total lines removed across all files.
    pub total_removed: u32,
    /// The changed files.
    pub files: Vec<ChangedFile>,
}

/// The kind of a single line in a unified diff hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffLineKind {
    /// Unchanged context line (present on both sides).
    Context,
    /// Line present only on the new side.
    Added,
    /// Line present only on the old side.
    Removed,
}

/// A changed sub-range within a diff line, expressed in UTF-16 code units so it
/// maps directly onto JavaScript string offsets in the WebView renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordSpan {
    /// Inclusive start offset (UTF-16 code units).
    pub start: u32,
    /// Exclusive end offset (UTF-16 code units).
    pub end: u32,
}

/// A single line within a [`DiffHunk`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// 1-based old-file line number (present for `Context` and `Removed`).
    pub old_lineno: Option<u32>,
    /// 1-based new-file line number (present for `Context` and `Added`).
    pub new_lineno: Option<u32>,
    /// Line text without the trailing newline.
    pub content: String,
    /// Intra-line word-diff ranges for changed lines (empty otherwise).
    pub spans: Vec<WordSpan>,
}

/// A contiguous hunk of a unified diff (changed lines plus surrounding context).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// The `@@ ... @@` header, including any trailing function-context text.
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// The unified-diff hunks for a single file (HEAD vs index + working tree).
///
/// Only changed regions plus a few context lines are materialized — never the
/// whole file — so a small change in a large file stays cheap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHunks {
    /// Monaco language id.
    pub language: String,
    /// Whether either side is binary/non-UTF-8 (hunks blanked).
    pub is_binary: bool,
    /// Whether the file exceeded a size/complexity guard (hunks blanked).
    pub too_large: bool,
    /// Whether the diff was capped (more hunks/lines exist than were emitted).
    pub truncated: bool,
    /// Lines added.
    pub added: u32,
    /// Lines removed.
    pub removed: u32,
    /// The diff hunks (empty when binary/too_large).
    pub hunks: Vec<DiffHunk>,
}

fn file_diff_all_lines_added(path: &Path) -> Result<FileDiff, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let changed_lines = content
        .lines()
        .enumerate()
        .map(|(idx, _)| ((idx + 1) as u32, DiffLineStatus::Added))
        .collect();

    Ok(FileDiff {
        changed_lines,
        deleted_lines: Vec::new(),
    })
}

/// Get diff status for each line of a file (working tree vs HEAD).
/// Returns changed lines with their status.
pub fn get_file_diff(file_path: &str) -> Result<FileDiff, String> {
    // Skip diff for files larger than 1MB
    let metadata = std::fs::metadata(file_path).ok();
    if let Some(meta) = metadata {
        if meta.len() > 1_048_576 {
            return Ok(FileDiff {
                changed_lines: std::collections::HashMap::new(),
                deleted_lines: Vec::new(),
            });
        }
    }

    let path = Path::new(file_path);
    let repo = open_repo(path)?;

    // Make file_path relative to repo root
    let repo_root = repo.workdir().ok_or("Bare repository")?;
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let canonical_repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let rel_path = canonical_path
        .strip_prefix(&canonical_repo_root)
        .map_err(|_| "File not in repo".to_string())?;

    if repo
        .status_file(rel_path)
        .map(|status| {
            status.contains(git2::Status::WT_NEW) || status.contains(git2::Status::INDEX_NEW)
        })
        .unwrap_or(false)
    {
        return file_diff_all_lines_added(path);
    }

    let head = match repo.head() {
        Ok(head) => head,
        Err(_) => {
            // No HEAD (empty repo) -- all lines are added
            return file_diff_all_lines_added(path);
        }
    };
    let head_tree = head
        .peel_to_tree()
        .map_err(|e| format!("Failed to get HEAD tree: {}", e))?;

    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.pathspec(rel_path.to_string_lossy().as_ref());

    let diff = repo
        .diff_tree_to_workdir(Some(&head_tree), Some(&mut diff_opts))
        .map_err(|e| format!("Diff failed: {}", e))?;

    let mut changed_lines = HashMap::new();
    let mut deleted_lines: Vec<u32> = Vec::new();

    // Single-pass diff: collect additions per hunk and classify them as Added or
    // Modified depending on whether the hunk also has deletions. Pure-deletion
    // hunks (no additions) are recorded in `deleted_lines`.
    let mut hunk_added: Vec<u32> = Vec::new();
    let mut hunk_removed_count: u32 = 0;
    let mut last_hunk_header: Option<(u32, u32, u32, u32)> = None;

    let classify_hunk = |added: &mut Vec<u32>,
                         removed: &mut u32,
                         lines: &mut HashMap<u32, DiffLineStatus>,
                         deleted: &mut Vec<u32>,
                         hunk_header: &Option<(u32, u32, u32, u32)>| {
        if !added.is_empty() && *removed > 0 {
            // Mixed hunk: first N additions (matching deletion count) are Modified,
            // the rest are Added.
            let modify_count = added.len().min(*removed as usize);
            for (i, &lineno) in added.iter().enumerate() {
                if i < modify_count {
                    lines.insert(lineno, DiffLineStatus::Modified);
                } else {
                    lines.insert(lineno, DiffLineStatus::Added);
                }
            }
        } else if !added.is_empty() {
            // Pure addition hunk
            for &lineno in added.iter() {
                lines.insert(lineno, DiffLineStatus::Added);
            }
        } else if *removed > 0 {
            // Pure deletion hunk — anchor at the new-file line where the deletion occurred
            if let Some((_, _, new_start, _)) = hunk_header {
                deleted.push(*new_start);
            }
        }
        added.clear();
        *removed = 0;
    };

    diff.foreach(
        &mut |_, _| true,
        None,
        None,
        Some(&mut |_delta, hunk, line| {
            // Detect hunk transitions by comparing hunk header values
            let current_hunk =
                hunk.map(|h| (h.old_start(), h.old_lines(), h.new_start(), h.new_lines()));
            if current_hunk != last_hunk_header {
                // New hunk — classify previous hunk's collected lines
                classify_hunk(
                    &mut hunk_added,
                    &mut hunk_removed_count,
                    &mut changed_lines,
                    &mut deleted_lines,
                    &last_hunk_header,
                );
                last_hunk_header = current_hunk;
            }

            match line.origin() {
                '+' => {
                    if let Some(lineno) = line.new_lineno() {
                        hunk_added.push(lineno);
                    }
                }
                '-' => {
                    hunk_removed_count += 1;
                }
                _ => {}
            }
            true
        }),
    )
    .map_err(|e| format!("Diff iteration failed: {}", e))?;

    // Classify final hunk
    classify_hunk(
        &mut hunk_added,
        &mut hunk_removed_count,
        &mut changed_lines,
        &mut deleted_lines,
        &last_hunk_header,
    );

    Ok(FileDiff {
        changed_lines,
        deleted_lines,
    })
}

/// Discard working-tree changes for a single file, restoring it to the HEAD version.
/// For untracked files this is a no-op (returns Ok).
/// `workspace_root` is used to validate that the file is within the workspace.
pub fn discard_file_changes(file_path: &str, workspace_root: &str) -> Result<(), String> {
    // Validate file is within workspace
    if let Err(e) = crate::util::validate_path_within_root(file_path, workspace_root) {
        return Err(format!("Cannot discard changes: {}", e));
    }

    let path = Path::new(file_path);
    let repo = open_repo(path)?;
    let repo_root = repo.workdir().ok_or("Bare repository")?;
    let rel_path = path
        .strip_prefix(repo_root)
        .map_err(|_| "File not in repo".to_string())?;

    let mut checkout_opts = git2::build::CheckoutBuilder::new();
    checkout_opts.path(rel_path);
    checkout_opts.force();
    repo.checkout_head(Some(&mut checkout_opts))
        .map_err(|e| format!("Checkout failed: {}", e))
}

/// Compute the per-file `(added, removed, is_binary)` stats for a single delta
/// of a diff. Binary deltas (or those without a textual patch) report
/// `(0, 0, true)`.
///
/// `repo`/`workdir` are used to size-guard large files: if either the
/// worktree file or the HEAD-side blob exceeds [`MAX_DIFF_CONTENT_SIZE`], we
/// skip the (potentially expensive) `Patch::from_diff` computation and report
/// `(0, 0, false)`. This matters for large untracked text files, whose size is
/// not reflected in `delta.new_file().size()`.
fn delta_line_stats(
    repo: &git2::Repository,
    workdir: &Path,
    diff: &git2::Diff,
    index: usize,
    delta: &git2::DiffDelta,
) -> (u32, u32, bool) {
    if delta.flags().is_binary() {
        return (0, 0, true);
    }

    // Size guard: stat the worktree file and inspect the HEAD blob length.
    if let Some(p) = delta.new_file().path() {
        if let Ok(meta) = std::fs::metadata(workdir.join(p)) {
            if meta.len() > MAX_DIFF_CONTENT_SIZE {
                return (0, 0, false);
            }
        }
    }
    let old_id = delta.old_file().id();
    if !old_id.is_zero() {
        if let Ok(blob) = repo.find_blob(old_id) {
            if blob.size() as u64 > MAX_DIFF_CONTENT_SIZE {
                return (0, 0, false);
            }
        }
    }

    match git2::Patch::from_diff(diff, index) {
        Ok(Some(patch)) => {
            // git2 only sets the binary flag once the patch content is computed,
            // so re-check it on the patch's own delta.
            if patch.delta().flags().is_binary() {
                return (0, 0, true);
            }
            match patch.line_stats() {
                Ok((_context, additions, deletions)) => (additions as u32, deletions as u32, false),
                Err(_) => (0, 0, false),
            }
        }
        // No patch produced -> treat as binary (git2 returns None for binary deltas).
        Ok(None) => (0, 0, true),
        Err(_) => (0, 0, false),
    }
}

/// Map a git2 delta status to the contract's status letter.
fn status_letter(status: git2::Delta) -> &'static str {
    match status {
        git2::Delta::Untracked => "?",
        git2::Delta::Added => "A",
        git2::Delta::Deleted => "D",
        git2::Delta::Renamed | git2::Delta::Copied => "R",
        // Modified, Typechange, and everything else map to modified.
        _ => "M",
    }
}

/// List all uncommitted changes in the repository containing `repo_path`
/// (HEAD vs index + working tree), including untracked files and renames.
pub fn list_changed_files(repo_path: &str) -> Result<ChangeSet, String> {
    let repo = open_repo(Path::new(repo_path))?;
    let workdir = repo.workdir().ok_or("Bare repository")?.to_path_buf();
    let repo_root = workdir.to_string_lossy().trim_end_matches('/').to_string();

    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true);
    opts.recurse_untracked_dirs(true);
    // Required so untracked files produce line stats and binary detection.
    opts.show_untracked_content(true);

    let mut diff = repo
        .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
        .map_err(|e| format!("Diff failed: {}", e))?;

    // Detect renames so renamed files report status "R" + old_path.
    diff.find_similar(None)
        .map_err(|e| format!("find_similar failed: {}", e))?;

    let mut files: Vec<ChangedFile> = Vec::new();
    let mut total_added: u32 = 0;
    let mut total_removed: u32 = 0;

    for (index, delta) in diff.deltas().enumerate() {
        let (added, removed, is_binary) = delta_line_stats(&repo, &workdir, &diff, index, &delta);

        let new_path = delta.new_file().path().map(|p| p.to_path_buf());
        let old_path = delta.old_file().path().map(|p| p.to_path_buf());

        // Prefer new path; fall back to old path (e.g. deletions).
        let path = match new_path.clone().or_else(|| old_path.clone()) {
            Some(p) => p.to_string_lossy().to_string(),
            None => continue,
        };

        let status = status_letter(delta.status());
        let old_path_str = if status == "R" {
            old_path.map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };

        total_added = total_added.saturating_add(added);
        total_removed = total_removed.saturating_add(removed);

        files.push(ChangedFile {
            path,
            status: status.to_string(),
            old_path: old_path_str,
            added,
            removed,
            is_binary,
        });
    }

    let branch = get_git_branch(repo_path).unwrap_or(None);

    Ok(ChangeSet {
        repo_root,
        branch,
        total_added,
        total_removed,
        files,
    })
}

/// Compute the unified-diff hunks for a single repo-relative `file_path`
/// (HEAD vs index + working tree). Only changed regions plus a few context
/// lines are materialized, so a small change in a large file stays cheap.
///
/// Rename detection (`find_similar`) is applied so a renamed file diffs its old
/// blob against the new content rather than reporting a 100% rewrite.
pub fn file_hunks(repo_path: &str, file_path: &str) -> Result<FileHunks, String> {
    let repo = open_repo(Path::new(repo_path))?;
    let workdir = repo.workdir().ok_or("Bare repository")?.to_path_buf();
    let rel = Path::new(file_path);

    // Path-traversal guard: validate lexically (NOT via the disk-based
    // `validate_path_within_root`, which fails on missing files — deleted files
    // are valid diff targets here).
    crate::util::validate_rel_path_lexically(&workdir, rel)
        .map_err(|e| format!("Cannot read diff: {}", e))?;

    let abs = workdir.join(rel);
    let language = crate::util::file_path_to_uri(&abs)
        .map(|uri| crate::util::language_from_uri(&uri))
        .unwrap_or_default();

    let blank = |is_binary: bool, too_large: bool| FileHunks {
        language: language.clone(),
        is_binary,
        too_large,
        truncated: false,
        added: 0,
        removed: 0,
        hunks: Vec::new(),
    };

    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true);
    opts.recurse_untracked_dirs(true);
    opts.show_untracked_content(true);

    let mut diff = repo
        .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
        .map_err(|e| format!("Diff failed: {}", e))?;
    // Pair renames so a rename diffs old→new content instead of all-added.
    let _ = diff.find_similar(None);

    // Locate the delta for this path (new side for most, old side for deletions).
    let found = diff
        .deltas()
        .enumerate()
        .find(|(_, d)| d.new_file().path() == Some(rel) || d.old_file().path() == Some(rel));
    let (index, delta) = match found {
        Some(pair) => pair,
        // No delta for this path: nothing changed (or already committed).
        None => return Ok(blank(false, false)),
    };

    if delta.flags().is_binary() {
        return Ok(blank(true, false));
    }

    // Size guard: skip diffing when either side exceeds the byte limit.
    let worktree_too_big = delta
        .new_file()
        .path()
        .and_then(|p| std::fs::metadata(workdir.join(p)).ok())
        .map(|m| m.len() > MAX_DIFF_CONTENT_SIZE)
        .unwrap_or(false);
    let old_id = delta.old_file().id();
    let blob_too_big = (!old_id.is_zero())
        .then(|| repo.find_blob(old_id).ok())
        .flatten()
        .map(|b| b.size() as u64 > MAX_DIFF_CONTENT_SIZE)
        .unwrap_or(false);
    if worktree_too_big || blob_too_big {
        return Ok(blank(false, true));
    }

    let patch = match git2::Patch::from_diff(&diff, index) {
        Ok(Some(p)) => p,
        // git2 returns None for binary deltas.
        Ok(None) => return Ok(blank(true, false)),
        Err(e) => return Err(format!("Patch failed: {}", e)),
    };
    // The binary flag is only reliable once the patch content is computed.
    if patch.delta().flags().is_binary() {
        return Ok(blank(true, false));
    }

    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut added: u32 = 0;
    let mut removed: u32 = 0;
    let mut total_lines: usize = 0;
    let mut truncated = false;

    let num_hunks = patch.num_hunks();
    'hunks: for h in 0..num_hunks {
        if hunks.len() >= MAX_DIFF_HUNKS || total_lines >= MAX_DIFF_TOTAL_LINES {
            truncated = true;
            break;
        }
        let (gh, _) = patch
            .hunk(h)
            .map_err(|e| format!("Hunk read failed: {}", e))?;
        let header = String::from_utf8_lossy(gh.header())
            .trim_end_matches('\n')
            .to_string();
        let mut lines: Vec<DiffLine> = Vec::new();
        let num_lines = patch
            .num_lines_in_hunk(h)
            .map_err(|e| format!("Hunk size failed: {}", e))?;
        for l in 0..num_lines {
            if total_lines >= MAX_DIFF_TOTAL_LINES {
                truncated = true;
                hunks.push(DiffHunk {
                    old_start: gh.old_start(),
                    old_lines: gh.old_lines(),
                    new_start: gh.new_start(),
                    new_lines: gh.new_lines(),
                    header,
                    lines,
                });
                break 'hunks;
            }
            let dl = patch
                .line_in_hunk(h, l)
                .map_err(|e| format!("Line read failed: {}", e))?;
            let kind = match dl.origin() {
                '+' | '>' => DiffLineKind::Added,
                '-' | '<' => DiffLineKind::Removed,
                _ => DiffLineKind::Context,
            };
            let mut content = String::from_utf8_lossy(dl.content()).into_owned();
            if content.ends_with('\n') {
                content.pop();
                if content.ends_with('\r') {
                    content.pop();
                }
            }
            // Overlong-line guard: bail out to a too-large placeholder rather
            // than ship a line that would choke the WebView renderer.
            if content.len() > MAX_DIFF_LINE_LENGTH {
                return Ok(blank(false, true));
            }
            match kind {
                DiffLineKind::Added => added += 1,
                DiffLineKind::Removed => removed += 1,
                DiffLineKind::Context => {}
            }
            lines.push(DiffLine {
                kind,
                old_lineno: dl.old_lineno(),
                new_lineno: dl.new_lineno(),
                content,
                spans: Vec::new(),
            });
            total_lines += 1;
        }
        assign_word_spans(&mut lines);
        hunks.push(DiffHunk {
            old_start: gh.old_start(),
            old_lines: gh.old_lines(),
            new_start: gh.new_start(),
            new_lines: gh.new_lines(),
            header,
            lines,
        });
    }

    Ok(FileHunks {
        language,
        is_binary: false,
        too_large: false,
        truncated,
        added,
        removed,
        hunks,
    })
}

/// Compute intra-line word-diff spans for paired removed/added lines within a
/// hunk. A maximal run of consecutive `Removed` lines immediately followed by
/// `Added` lines is paired index-for-index; each pair gets word-level spans so
/// the renderer can emphasize only the characters that actually changed.
fn assign_word_spans(lines: &mut [DiffLine]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != DiffLineKind::Removed {
            i += 1;
            continue;
        }
        let r_start = i;
        while i < lines.len() && lines[i].kind == DiffLineKind::Removed {
            i += 1;
        }
        let r_end = i;
        let a_start = i;
        while i < lines.len() && lines[i].kind == DiffLineKind::Added {
            i += 1;
        }
        let a_end = i;

        let pairs = (r_end - r_start).min(a_end - a_start);
        for k in 0..pairs {
            let old_idx = r_start + k;
            let new_idx = a_start + k;
            let old_content = lines[old_idx].content.clone();
            let new_content = lines[new_idx].content.clone();
            if old_content.encode_utf16().count() + new_content.encode_utf16().count()
                > MAX_WORD_DIFF_LINE_LEN
            {
                continue;
            }
            let (old_spans, new_spans) = word_spans(&old_content, &new_content);
            lines[old_idx].spans = old_spans;
            lines[new_idx].spans = new_spans;
        }
    }
}

/// Word-level diff of two lines, returning the changed UTF-16 ranges on the old
/// and new side respectively.
fn word_spans(old: &str, new: &str) -> (Vec<WordSpan>, Vec<WordSpan>) {
    use similar::{ChangeTag, TextDiff};

    let push = |spans: &mut Vec<WordSpan>, start: u32, end: u32| {
        if let Some(last) = spans.last_mut() {
            if last.end == start {
                last.end = end;
                return;
            }
        }
        spans.push(WordSpan { start, end });
    };

    let diff = TextDiff::from_words(old, new);
    let mut old_off: u32 = 0;
    let mut new_off: u32 = 0;
    let mut old_spans: Vec<WordSpan> = Vec::new();
    let mut new_spans: Vec<WordSpan> = Vec::new();
    for change in diff.iter_all_changes() {
        let len = change.value().encode_utf16().count() as u32;
        match change.tag() {
            ChangeTag::Equal => {
                old_off += len;
                new_off += len;
            }
            ChangeTag::Delete => {
                if len > 0 {
                    push(&mut old_spans, old_off, old_off + len);
                }
                old_off += len;
            }
            ChangeTag::Insert => {
                if len > 0 {
                    push(&mut new_spans, new_off, new_off + len);
                }
                new_off += len;
            }
        }
    }
    (old_spans, new_spans)
}

/// Stage all changes (additions, modifications, deletions) and create a commit
/// on HEAD. Returns the new commit's OID as a hex string.
pub fn commit_all(repo_path: &str, message: &str) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("Commit message is empty".to_string());
    }

    let repo = open_repo(Path::new(repo_path))?;

    // Refuse to commit while a merge/rebase/cherry-pick/etc. is in progress, or
    // while there are unresolved conflicts. Committing here would bake conflict
    // markers into the tree and drop the in-progress operation's extra parent
    // (e.g. MERGE_HEAD), corrupting history.
    if repo.state() != git2::RepositoryState::Clean {
        return Err(
            "Cannot commit: a merge, rebase, or other operation is in progress. \
             Resolve it first."
                .to_string(),
        );
    }

    let mut index = repo.index().map_err(|e| format!("Index error: {}", e))?;
    if index.has_conflicts() {
        return Err("Cannot commit: there are unresolved merge conflicts.".to_string());
    }
    // Stage new + modified files.
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .map_err(|e| format!("Failed to stage files: {}", e))?;
    // Stage deletions of tracked files (add_all does not remove them).
    index
        .update_all(["*"].iter(), None)
        .map_err(|e| format!("Failed to stage deletions: {}", e))?;
    index
        .write()
        .map_err(|e| format!("Failed to write index: {}", e))?;

    let tree_id = index
        .write_tree()
        .map_err(|e| format!("Failed to write tree: {}", e))?;
    let tree = repo
        .find_tree(tree_id)
        .map_err(|e| format!("Failed to find tree: {}", e))?;

    let parent_commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());

    // If nothing changed relative to the parent, refuse.
    if let Some(parent) = &parent_commit {
        if let Ok(parent_tree) = parent.tree() {
            if parent_tree.id() == tree_id {
                return Err("nothing to commit".to_string());
            }
        }
    }

    let sig = repo
        .signature()
        .map_err(|e| format!("No git signature (configure user.name/user.email): {}", e))?;

    let oid = match &parent_commit {
        Some(parent) => repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[parent])
            .map_err(|e| format!("Commit failed: {}", e))?,
        None => repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
            .map_err(|e| format!("Commit failed: {}", e))?,
    };

    Ok(oid.to_string())
}

/// Discard a single repo-relative path back to a clean state:
/// - tracked modified/deleted: checkout from HEAD
/// - untracked/new: delete the file (and unstage if staged)
pub fn discard_path(repo_path: &str, file_path: &str) -> Result<(), String> {
    let repo = open_repo(Path::new(repo_path))?;
    let workdir = repo.workdir().ok_or("Bare repository")?.to_path_buf();

    let rel = Path::new(file_path);

    // Determine status FIRST so we can pick the right validation strategy.
    // (Existence-based validation fails for files whose parent dir was also
    // removed, which is a legitimate discard target.)
    let status = repo
        .status_file(rel)
        .map_err(|e| format!("Failed to get status: {}", e))?;

    if status.contains(git2::Status::WT_NEW) || status.contains(git2::Status::INDEX_NEW) {
        // Per-file status does NO rename detection, so the NEW side of a staged
        // rename (old -> new) reports as INDEX_NEW here. Blindly deleting `rel`
        // and unstaging it would destroy the renamed content and leave the old
        // path staged-as-deleted. Detect that case via repo-wide status with
        // rename detection enabled, and if so restore the original path instead.
        if let Some(old_path) = staged_rename_original(&repo, rel)? {
            return restore_rename(&repo, &workdir, rel, &old_path);
        }

        // Genuinely untracked / brand-new staged file. Validate via disk
        // (we are about to touch the filesystem with remove_file).
        let abs = workdir.join(rel);
        crate::util::validate_path_within_root(&abs.to_string_lossy(), &workdir.to_string_lossy())
            .map_err(|e| format!("Cannot discard: {}", e))?;

        if abs.is_file() {
            std::fs::remove_file(&abs)
                .map_err(|e| format!("Failed to remove {}: {}", abs.display(), e))?;
        }
        // If it was staged, unstage it from the index.
        if status.contains(git2::Status::INDEX_NEW) {
            let mut index = repo.index().map_err(|e| format!("Index error: {}", e))?;
            index
                .remove_path(rel)
                .map_err(|e| format!("Failed to unstage {}: {}", rel.display(), e))?;
            index
                .write()
                .map_err(|e| format!("Failed to write index: {}", e))?;
        }
        return Ok(());
    }

    // Tracked modified/deleted: restore from HEAD.
    //
    // Use LEXICAL containment validation here (not the disk-based
    // `validate_path_within_root`): the target file may be deleted, possibly
    // along with its parent directory, in which case canonicalizing the parent
    // would fail with ENOENT and we'd never reach checkout_head (which recreates
    // missing directories).
    crate::util::validate_rel_path_lexically(&workdir, rel)
        .map_err(|e| format!("Cannot discard: {}", e))?;

    let mut checkout_opts = git2::build::CheckoutBuilder::new();
    checkout_opts.path(rel);
    checkout_opts.force();
    repo.checkout_head(Some(&mut checkout_opts))
        .map_err(|e| format!("Checkout failed: {}", e))
}

/// If `rel` is the NEW side of a staged rename, return the original (old) path.
/// Uses repo-wide statuses with rename detection enabled (per-file status never
/// reports renames).
fn staged_rename_original(repo: &git2::Repository, rel: &Path) -> Result<Option<PathBuf>, String> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);

    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| format!("Failed to compute statuses: {}", e))?;

    for entry in statuses.iter() {
        for delta in [entry.head_to_index(), entry.index_to_workdir()]
            .into_iter()
            .flatten()
        {
            if delta.status() == git2::Delta::Renamed && delta.new_file().path() == Some(rel) {
                if let Some(old) = delta.old_file().path() {
                    return Ok(Some(old.to_path_buf()));
                }
            }
        }
    }

    Ok(None)
}

/// Undo a staged rename `old_path` -> `new_path`: remove the new file from disk,
/// unstage it, and restore the original path from HEAD (unstaged, with content
/// reappearing on disk).
fn restore_rename(
    repo: &git2::Repository,
    workdir: &Path,
    new_path: &Path,
    old_path: &Path,
) -> Result<(), String> {
    // Validate both paths lexically (no disk access — `old_path` may not exist).
    crate::util::validate_rel_path_lexically(workdir, new_path)
        .map_err(|e| format!("Cannot discard: {}", e))?;
    crate::util::validate_rel_path_lexically(workdir, old_path)
        .map_err(|e| format!("Cannot discard: {}", e))?;

    // Remove the renamed-to file from disk.
    let new_abs = workdir.join(new_path);
    if new_abs.is_file() {
        std::fs::remove_file(&new_abs)
            .map_err(|e| format!("Failed to remove {}: {}", new_abs.display(), e))?;
    }

    // Reset the index so the new path is fully gone and the old path matches
    // HEAD again, then check out the old path so its content reappears.
    let mut index = repo.index().map_err(|e| format!("Index error: {}", e))?;
    index
        .remove_path(new_path)
        .map_err(|e| format!("Failed to unstage {}: {}", new_path.display(), e))?;

    // Restore the old path's index entry from HEAD.
    if let Ok(head) = repo.head() {
        if let Ok(tree) = head.peel_to_tree() {
            if let Ok(entry) = tree.get_path(old_path) {
                if let Ok(obj) = entry.to_object(repo) {
                    if let Ok(blob) = obj.peel_to_blob() {
                        let index_entry = git2::IndexEntry {
                            ctime: git2::IndexTime::new(0, 0),
                            mtime: git2::IndexTime::new(0, 0),
                            dev: 0,
                            ino: 0,
                            mode: entry.filemode() as u32,
                            uid: 0,
                            gid: 0,
                            file_size: blob.content().len() as u32,
                            id: blob.id(),
                            flags: 0,
                            flags_extended: 0,
                            path: old_path.to_string_lossy().as_bytes().to_vec(),
                        };
                        index
                            .add(&index_entry)
                            .map_err(|e| format!("Failed to restore index entry: {}", e))?;
                    }
                }
            }
        }
    }
    index
        .write()
        .map_err(|e| format!("Failed to write index: {}", e))?;

    // Check out the old path from HEAD so its content reappears on disk.
    let mut checkout_opts = git2::build::CheckoutBuilder::new();
    checkout_opts.path(old_path);
    checkout_opts.force();
    repo.checkout_head(Some(&mut checkout_opts))
        .map_err(|e| format!("Checkout failed: {}", e))
}

/// Get blame information for a specific line in a file.
/// line is 1-based.
pub fn get_line_blame(file_path: &str, line: u32) -> Result<BlameInfo, String> {
    let path = Path::new(file_path);
    let repo = open_repo(path)?;

    let repo_root = repo.workdir().ok_or("Bare repository")?;
    let rel_path = path
        .strip_prefix(repo_root)
        .map_err(|_| "File not in repo".to_string())?;

    let mut blame_opts = git2::BlameOptions::new();
    let blame = repo
        .blame_file(rel_path, Some(&mut blame_opts))
        .map_err(|e| format!("Blame failed: {}", e))?;

    // git2 blame uses 1-based line indexing in get_line()
    let hunk = blame
        .get_line(line as usize)
        .ok_or_else(|| format!("No blame info for line {}", line))?;

    let sig = hunk.final_signature();
    let author = sig.name().unwrap_or("Unknown").to_string();

    // Format the time, applying the timezone offset from git
    let time = sig.when();
    let timestamp = time.seconds();
    let tz_offset_minutes = time.offset_minutes();
    let date = format_timestamp(timestamp, tz_offset_minutes);

    let commit_hash = format!("{}", hunk.final_commit_id());
    let commit_hash_short = commit_hash[..7.min(commit_hash.len())].to_string();

    // Get commit summary
    let summary = match repo.find_commit(hunk.final_commit_id()) {
        Ok(commit) => commit.summary().unwrap_or("").to_string(),
        Err(_) => String::new(),
    };

    Ok(BlameInfo {
        author,
        date,
        commit_hash: commit_hash_short,
        summary,
    })
}

/// Format a unix timestamp into a human-readable date string.
/// `tz_offset_minutes` is the timezone offset in minutes (e.g. -300 for EST, +60 for CET).
fn format_timestamp(timestamp: i64, tz_offset_minutes: i32) -> String {
    // Apply timezone offset to get local time
    let local_timestamp = timestamp + (tz_offset_minutes as i64) * 60;

    // Handle negative timestamps (pre-epoch) gracefully
    if local_timestamp < 0 {
        return "1970-01-01".to_string();
    }

    let secs_per_day = 86400i64;

    let days_since_epoch = local_timestamp / secs_per_day;

    // Calculate year, month, day from days since epoch (1970-01-01)
    let mut year = 1970i32;
    let mut remaining_days = days_since_epoch;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let days_in_months = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u32;
    for &days in &days_in_months {
        if remaining_days < days {
            break;
        }
        remaining_days -= days;
        month += 1;
    }
    let day = remaining_days + 1;

    format!("{}-{:02}-{:02}", year, month, day)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Get current git branch name for a path using libgit2.
///
/// Returns the short branch name, or an abbreviated commit hash if HEAD is
/// detached, or `Ok(None)` if the path is not inside a git repository.
pub fn get_git_branch(path: &str) -> Result<Option<String>, String> {
    let repo = match open_repo(Path::new(path)) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };

    if head.is_branch() {
        Ok(head.shorthand().map(String::from))
    } else {
        // Detached HEAD — return abbreviated commit hash
        Ok(head.target().map(|oid| {
            let s = oid.to_string();
            s[..7.min(s.len())].to_string()
        }))
    }
}

/// List local branch names for the repository containing `path`, sorted
/// alphabetically. Returns an empty list if the path is not in a git repo.
pub fn list_git_branches(path: &str) -> Result<Vec<String>, String> {
    let repo = match open_repo(Path::new(path)) {
        Ok(repo) => repo,
        Err(_) => return Ok(Vec::new()),
    };
    let branches = repo
        .branches(Some(git2::BranchType::Local))
        .map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    for branch in branches {
        let (branch, _) = branch.map_err(|e| e.to_string())?;
        if let Ok(Some(name)) = branch.name() {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// Return the git working directory root for the given path, or `None` if
/// the path is not inside a git repository.
pub fn get_git_root(path: &str) -> Option<String> {
    open_repo(Path::new(path)).ok().and_then(|repo| {
        repo.workdir()
            .map(|wd| wd.to_string_lossy().trim_end_matches('/').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit_file(repo: &git2::Repository, path: &str, message: &str) {
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new(path)).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Impulse Test", "impulse@example.com").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
            .unwrap();
    }

    /// Configure a deterministic identity on a repo so `signature()` and
    /// `commit_all` work without relying on global git config.
    fn configure_identity(repo: &git2::Repository) {
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Impulse Test").unwrap();
        cfg.set_str("user.email", "impulse@example.com").unwrap();
    }

    /// Find a `ChangedFile` by its new path within a `ChangeSet`.
    fn find<'a>(set: &'a ChangeSet, path: &str) -> Option<&'a ChangedFile> {
        set.files.iter().find(|f| f.path == path)
    }

    #[test]
    fn list_changed_files_modified_file() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let file = temp.path().join("a.txt");
        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
        commit_file(&repo, "a.txt", "init");

        std::fs::write(&file, "one\nTWO\nthree\n").unwrap();

        let set = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        let f = find(&set, "a.txt").expect("a.txt present");
        assert_eq!(f.status, "M");
        assert_eq!(f.added, 1);
        assert_eq!(f.removed, 1);
        assert!(!f.is_binary);
        assert_eq!(set.total_added, 1);
        assert_eq!(set.total_removed, 1);
    }

    #[test]
    fn list_changed_files_added_untracked_file() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let tracked = temp.path().join("tracked.txt");
        std::fs::write(&tracked, "x\n").unwrap();
        commit_file(&repo, "tracked.txt", "init");

        let file = temp.path().join("new.txt");
        std::fs::write(&file, "alpha\nbeta\n").unwrap();

        let set = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        let f = find(&set, "new.txt").expect("new.txt present");
        assert_eq!(f.status, "?");
        assert_eq!(f.added, 2);
        assert_eq!(f.removed, 0);
        assert!(f.old_path.is_none());
    }

    #[test]
    fn list_changed_files_deleted_file() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let file = temp.path().join("gone.txt");
        std::fs::write(&file, "line1\nline2\n").unwrap();
        commit_file(&repo, "gone.txt", "init");

        std::fs::remove_file(&file).unwrap();

        let set = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        let f = find(&set, "gone.txt").expect("gone.txt present");
        assert_eq!(f.status, "D");
        assert_eq!(f.added, 0);
        assert_eq!(f.removed, 2);
    }

    #[test]
    fn list_changed_files_renamed_file() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let original = temp.path().join("old_name.txt");
        let content = "alpha\nbeta\ngamma\ndelta\nepsilon\n";
        std::fs::write(&original, content).unwrap();
        commit_file(&repo, "old_name.txt", "init");

        // Rename: remove old, add identical new content + stage both so
        // find_similar can detect the rename.
        std::fs::remove_file(&original).unwrap();
        let renamed = temp.path().join("new_name.txt");
        std::fs::write(&renamed, content).unwrap();
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new("old_name.txt")).unwrap();
        index.add_path(Path::new("new_name.txt")).unwrap();
        index.write().unwrap();

        let set = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        let f = find(&set, "new_name.txt").expect("new_name.txt present");
        assert_eq!(f.status, "R");
        assert_eq!(f.old_path.as_deref(), Some("old_name.txt"));
    }

    #[test]
    fn list_changed_files_binary_flagged() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let placeholder = temp.path().join("placeholder.txt");
        std::fs::write(&placeholder, "x\n").unwrap();
        commit_file(&repo, "placeholder.txt", "init");

        let bin = temp.path().join("blob.bin");
        std::fs::write(&bin, [0u8, 159, 146, 150, 0, 1, 2, 3]).unwrap();

        let set = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        let f = find(&set, "blob.bin").expect("blob.bin present");
        assert!(f.is_binary);
        assert_eq!(f.added, 0);
        assert_eq!(f.removed, 0);
    }

    #[test]
    fn list_changed_files_empty_repo_lists_tracked_as_added() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let file = temp.path().join("first.txt");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        // Stage the file but do not commit -> no HEAD yet.
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("first.txt")).unwrap();
        index.write().unwrap();

        let set = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        let f = find(&set, "first.txt").expect("first.txt present");
        assert_eq!(f.status, "A");
        assert_eq!(f.added, 2);
    }

    /// Flatten every line across all hunks for assertions.
    fn all_lines(fh: &FileHunks) -> Vec<&DiffLine> {
        fh.hunks.iter().flat_map(|h| h.lines.iter()).collect()
    }

    #[test]
    fn file_hunks_modified() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let file = temp.path().join("code.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        commit_file(&repo, "code.rs", "init");
        std::fs::write(&file, "fn main() { println!(\"hi\"); }\n").unwrap();

        let fh = file_hunks(temp.path().to_str().unwrap(), "code.rs").unwrap();
        assert_eq!(fh.language, "rust");
        assert!(!fh.is_binary);
        assert!(!fh.too_large);
        assert!(!fh.truncated);
        assert_eq!(fh.added, 1);
        assert_eq!(fh.removed, 1);

        let lines = all_lines(&fh);
        let removed = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Removed)
            .unwrap();
        assert_eq!(removed.content, "fn main() {}");
        let added = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Added)
            .unwrap();
        assert_eq!(added.content, "fn main() { println!(\"hi\"); }");
        // The paired removed/added line should carry word-diff spans.
        assert!(
            !added.spans.is_empty(),
            "expected word-diff spans on the added line"
        );
    }

    #[test]
    fn file_hunks_added_is_all_added() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let tracked = temp.path().join("t.txt");
        std::fs::write(&tracked, "x\n").unwrap();
        commit_file(&repo, "t.txt", "init");
        let file = temp.path().join("added.txt");
        std::fs::write(&file, "new content\n").unwrap();

        let fh = file_hunks(temp.path().to_str().unwrap(), "added.txt").unwrap();
        assert_eq!(fh.added, 1);
        assert_eq!(fh.removed, 0);
        let lines = all_lines(&fh);
        assert!(lines.iter().all(|l| l.kind == DiffLineKind::Added));
        assert_eq!(lines[0].content, "new content");
        assert_eq!(lines[0].new_lineno, Some(1));
        assert_eq!(lines[0].old_lineno, None);
    }

    #[test]
    fn file_hunks_deleted_is_all_removed() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let file = temp.path().join("del.txt");
        std::fs::write(&file, "to be removed\n").unwrap();
        commit_file(&repo, "del.txt", "init");
        std::fs::remove_file(&file).unwrap();

        let fh = file_hunks(temp.path().to_str().unwrap(), "del.txt").unwrap();
        assert_eq!(fh.added, 0);
        assert_eq!(fh.removed, 1);
        let lines = all_lines(&fh);
        assert!(lines.iter().all(|l| l.kind == DiffLineKind::Removed));
        assert_eq!(lines[0].content, "to be removed");
        assert_eq!(lines[0].old_lineno, Some(1));
        assert_eq!(lines[0].new_lineno, None);
    }

    #[test]
    fn file_hunks_small_change_in_large_file_is_cheap() {
        // A big file with a single changed line should produce just a hunk or
        // two of context — never the whole file.
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let file = temp.path().join("big.txt");
        let mut body: String = (0..5000).map(|i| format!("line {}\n", i)).collect();
        std::fs::write(&file, &body).unwrap();
        commit_file(&repo, "big.txt", "init");
        // Change one line in the middle.
        body = body.replacen("line 2500\n", "line 2500 CHANGED\n", 1);
        std::fs::write(&file, &body).unwrap();

        let fh = file_hunks(temp.path().to_str().unwrap(), "big.txt").unwrap();
        assert_eq!(fh.added, 1);
        assert_eq!(fh.removed, 1);
        // Only the changed region + context, not 5000 lines.
        assert!(
            all_lines(&fh).len() < 20,
            "expected a small hunk, got many lines"
        );
    }

    #[test]
    fn file_hunks_overlong_line_marked_too_large() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let file = temp.path().join("bundle.js");
        std::fs::write(&file, "var a=1;\n").unwrap();
        commit_file(&repo, "bundle.js", "init");
        // A single line well past MAX_DIFF_LINE_LENGTH (minified/generated).
        let huge = format!("var data=\"{}\";\n", "x".repeat(MAX_DIFF_LINE_LENGTH + 1));
        std::fs::write(&file, huge).unwrap();

        let fh = file_hunks(temp.path().to_str().unwrap(), "bundle.js").unwrap();
        assert!(fh.too_large);
        assert!(!fh.is_binary);
        assert!(fh.hunks.is_empty());
    }

    #[test]
    fn file_hunks_binary_blanked() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let placeholder = temp.path().join("p.txt");
        std::fs::write(&placeholder, "x\n").unwrap();
        commit_file(&repo, "p.txt", "init");
        let bin = temp.path().join("b.bin");
        std::fs::write(&bin, [0u8, 1, 2, 0, 3]).unwrap();

        let fh = file_hunks(temp.path().to_str().unwrap(), "b.bin").unwrap();
        assert!(fh.is_binary);
        assert!(fh.hunks.is_empty());
    }

    #[test]
    fn commit_all_creates_initial_commit_in_empty_repo() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        configure_identity(&repo);
        std::fs::write(temp.path().join("a.txt"), "hello\n").unwrap();

        // No HEAD before the commit.
        assert!(repo.head().is_err());

        let oid = commit_all(temp.path().to_str().unwrap(), "initial").unwrap();
        assert!(!oid.is_empty());

        // After commit, HEAD exists and the change set is empty.
        let repo2 = git2::Repository::open(temp.path()).unwrap();
        assert!(repo2.head().is_ok());
        let set = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        assert!(
            set.files.is_empty(),
            "expected clean tree, got {:?}",
            set.files
        );
    }

    #[test]
    fn commit_all_stages_deletion_and_empties_changeset() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        configure_identity(&repo);
        let file = temp.path().join("doomed.txt");
        std::fs::write(&file, "bye\n").unwrap();
        commit_file(&repo, "doomed.txt", "init");

        std::fs::remove_file(&file).unwrap();

        // The deletion shows up as a change before committing.
        let before = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(
            find(&before, "doomed.txt").map(|f| f.status.as_str()),
            Some("D")
        );

        commit_all(temp.path().to_str().unwrap(), "remove file").unwrap();

        let after = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        assert!(
            after.files.is_empty(),
            "expected clean tree, got {:?}",
            after.files
        );
    }

    #[test]
    fn commit_all_rejects_empty_message() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        configure_identity(&repo);
        std::fs::write(temp.path().join("a.txt"), "x\n").unwrap();
        assert!(commit_all(temp.path().to_str().unwrap(), "   ").is_err());
    }

    #[test]
    fn discard_path_reverts_modified_file() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let file = temp.path().join("m.txt");
        std::fs::write(&file, "original\n").unwrap();
        commit_file(&repo, "m.txt", "init");
        std::fs::write(&file, "changed\n").unwrap();

        discard_path(temp.path().to_str().unwrap(), "m.txt").unwrap();

        let restored = std::fs::read_to_string(&file).unwrap();
        assert_eq!(restored, "original\n");
    }

    #[test]
    fn discard_path_deletes_untracked_file() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let tracked = temp.path().join("t.txt");
        std::fs::write(&tracked, "x\n").unwrap();
        commit_file(&repo, "t.txt", "init");
        let file = temp.path().join("scratch.txt");
        std::fs::write(&file, "junk\n").unwrap();
        assert!(file.exists());

        discard_path(temp.path().to_str().unwrap(), "scratch.txt").unwrap();

        assert!(!file.exists());
    }

    /// Stage a rename old -> new (identical content) so per-file status reports
    /// the new path as INDEX_NEW but repo-wide status detects the rename.
    fn stage_rename(repo: &git2::Repository, old: &str, new: &str) {
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new(old)).unwrap();
        index.add_path(Path::new(new)).unwrap();
        index.write().unwrap();
    }

    /// Commit the current index, parented on HEAD when it exists. Returns the
    /// new commit's OID. Unlike [`commit_file`], this preserves history so we
    /// can build divergent branches.
    fn commit_index(repo: &git2::Repository, message: &str) -> git2::Oid {
        let mut index = repo.index().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("Impulse Test", "impulse@example.com").unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .unwrap()
    }

    #[test]
    fn commit_all_refused_during_merge_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        configure_identity(&repo);

        let file = temp.path().join("conflict.txt");

        // Base commit on the default branch.
        std::fs::write(&file, "base\n").unwrap();
        repo.index()
            .unwrap()
            .add_path(Path::new("conflict.txt"))
            .unwrap();
        repo.index().unwrap().write().unwrap();
        let base = commit_index(&repo, "base");
        let base_commit = repo.find_commit(base).unwrap();

        // Branch "ours" from base: change to "ours".
        std::fs::write(&file, "ours\n").unwrap();
        repo.index()
            .unwrap()
            .add_path(Path::new("conflict.txt"))
            .unwrap();
        repo.index().unwrap().write().unwrap();
        let ours = commit_index(&repo, "ours");

        // Create branch "theirs" from base, check it out, change to "theirs".
        let theirs_branch = repo.branch("theirs", &base_commit, false).unwrap();
        repo.set_head(theirs_branch.get().name().unwrap()).unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        std::fs::write(&file, "theirs\n").unwrap();
        repo.index()
            .unwrap()
            .add_path(Path::new("conflict.txt"))
            .unwrap();
        repo.index().unwrap().write().unwrap();
        let theirs = commit_index(&repo, "theirs");

        // Merge `ours` into the checked-out `theirs` -> conflict, leaving the
        // repo mid-merge with MERGE_HEAD set.
        let ours_ac = repo.find_annotated_commit(ours).unwrap();
        repo.merge(&[&ours_ac], None, None).unwrap();

        // We should now be in a non-clean state with conflicts.
        assert_ne!(repo.state(), git2::RepositoryState::Clean);
        assert!(repo.index().unwrap().has_conflicts());

        let err = commit_all(temp.path().to_str().unwrap(), "should be refused")
            .expect_err("commit during merge conflict must be refused");
        assert!(
            err.contains("merge") || err.contains("conflict") || err.contains("in progress"),
            "unexpected error: {}",
            err
        );

        // History was not advanced past the `theirs` commit.
        let head_after = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head_after.id(), theirs);
    }

    #[test]
    fn discard_path_restores_renamed_file() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let content = "alpha\nbeta\ngamma\n";
        std::fs::write(temp.path().join("old_name.txt"), content).unwrap();
        commit_file(&repo, "old_name.txt", "init");

        // Stage a rename old_name.txt -> new_name.txt.
        std::fs::rename(
            temp.path().join("old_name.txt"),
            temp.path().join("new_name.txt"),
        )
        .unwrap();
        stage_rename(&repo, "old_name.txt", "new_name.txt");

        // Sanity: list_changed_files reports the NEW path with status "R".
        let set = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        let f = find(&set, "new_name.txt").expect("new_name.txt present");
        assert_eq!(f.status, "R");

        // Discard the NEW path -> original restored, no dangling deletion.
        discard_path(temp.path().to_str().unwrap(), "new_name.txt").unwrap();

        assert!(
            !temp.path().join("new_name.txt").exists(),
            "renamed-to file should be removed"
        );
        let restored = std::fs::read_to_string(temp.path().join("old_name.txt")).unwrap();
        assert_eq!(restored, content, "original content must reappear");

        // The tree should be clean again (no staged deletion of old_name.txt).
        let after = list_changed_files(temp.path().to_str().unwrap()).unwrap();
        assert!(
            after.files.is_empty(),
            "expected clean tree, got {:?}",
            after.files
        );
    }

    #[test]
    fn discard_path_restores_file_deleted_with_its_directory() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        std::fs::create_dir(temp.path().join("nested")).unwrap();
        let file = temp.path().join("nested").join("deep.txt");
        std::fs::write(&file, "important\n").unwrap();
        commit_file(&repo, "nested/deep.txt", "init");

        // Remove the file AND its parent directory.
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(temp.path().join("nested")).unwrap();
        assert!(!temp.path().join("nested").exists());

        discard_path(temp.path().to_str().unwrap(), "nested/deep.txt").unwrap();

        let restored = std::fs::read_to_string(&file).unwrap();
        assert_eq!(restored, "important\n");
    }

    #[test]
    fn discard_path_rejects_parent_dir_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        std::fs::write(temp.path().join("a.txt"), "x\n").unwrap();
        commit_file(&repo, "a.txt", "init");
        std::fs::write(temp.path().join("a.txt"), "y\n").unwrap();

        // A traversal path should be refused before any checkout happens.
        let err = discard_path(temp.path().to_str().unwrap(), "../escape.txt")
            .expect_err("traversal must be rejected");
        assert!(err.contains(".."), "unexpected error: {}", err);
    }

    #[test]
    fn file_hunks_pure_rename_has_no_changed_lines() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let content = "line1\nline2\nline3\nline4\n";
        std::fs::write(temp.path().join("from.txt"), content).unwrap();
        commit_file(&repo, "from.txt", "init");

        std::fs::rename(temp.path().join("from.txt"), temp.path().join("to.txt")).unwrap();
        stage_rename(&repo, "from.txt", "to.txt");

        let fh = file_hunks(temp.path().to_str().unwrap(), "to.txt").unwrap();
        // A pure rename has no added/removed lines (and thus no hunks).
        assert_eq!(fh.added, 0, "pure rename should report 0 added");
        assert_eq!(fh.removed, 0, "pure rename should report 0 removed");
        assert!(fh.hunks.is_empty(), "pure rename should have no hunks");
        assert!(!fh.is_binary);
        assert!(!fh.too_large);
    }

    #[test]
    fn file_hunks_rejects_traversal() {
        let temp = tempfile::tempdir().unwrap();
        git2::Repository::init(temp.path()).unwrap();
        let err = file_hunks(temp.path().to_str().unwrap(), "../secret.txt")
            .expect_err("traversal must be rejected");
        assert!(err.contains(".."), "unexpected error: {}", err);
    }

    #[test]
    fn get_file_diff_marks_no_head_file_lines_added() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let file = temp.path().join("new.txt");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        drop(repo);

        let diff = get_file_diff(file.to_str().unwrap()).unwrap();

        assert_eq!(diff.changed_lines.get(&1), Some(&DiffLineStatus::Added));
        assert_eq!(diff.changed_lines.get(&2), Some(&DiffLineStatus::Added));
        assert!(diff.deleted_lines.is_empty());
    }

    #[test]
    fn get_file_diff_marks_untracked_file_lines_added() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let tracked = temp.path().join("tracked.txt");
        std::fs::write(&tracked, "tracked\n").unwrap();
        commit_file(&repo, "tracked.txt", "Initial commit");

        let file = temp.path().join("untracked.txt");
        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();

        let diff = get_file_diff(file.to_str().unwrap()).unwrap();

        assert_eq!(diff.changed_lines.get(&1), Some(&DiffLineStatus::Added));
        assert_eq!(diff.changed_lines.get(&2), Some(&DiffLineStatus::Added));
        assert_eq!(diff.changed_lines.get(&3), Some(&DiffLineStatus::Added));
        assert!(diff.deleted_lines.is_empty());
    }

    #[test]
    fn is_leap_year_basic() {
        assert!(is_leap_year(2000)); // divisible by 400
        assert!(is_leap_year(2024)); // divisible by 4
        assert!(!is_leap_year(1900)); // divisible by 100 but not 400
        assert!(!is_leap_year(2023)); // not divisible by 4
    }

    #[test]
    fn format_timestamp_epoch() {
        assert_eq!(format_timestamp(0, 0), "1970-01-01");
    }

    #[test]
    fn format_timestamp_known_date() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        assert_eq!(format_timestamp(1704067200, 0), "2024-01-01");
    }

    #[test]
    fn format_timestamp_leap_day() {
        // 2024-02-29 00:00:00 UTC = 1709164800
        assert_eq!(format_timestamp(1709164800, 0), "2024-02-29");
    }

    #[test]
    fn format_timestamp_end_of_year() {
        // 2023-12-31 00:00:00 UTC = 1703980800
        assert_eq!(format_timestamp(1703980800, 0), "2023-12-31");
    }

    #[test]
    fn format_timestamp_negative_returns_fallback() {
        // Pre-epoch timestamp should return a reasonable fallback
        assert_eq!(format_timestamp(-86400, 0), "1970-01-01");
    }

    #[test]
    fn format_timestamp_with_timezone_offset() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        // With UTC+5:30 (330 minutes), still 2024-01-01
        assert_eq!(format_timestamp(1704067200, 330), "2024-01-01");
        // 2023-12-31 23:00:00 UTC with UTC+2 => 2024-01-01 01:00 local
        assert_eq!(format_timestamp(1704067200 - 3600, 120), "2024-01-01");
    }

    #[test]
    fn diff_line_status_serialization() {
        let json = serde_json::to_string(&DiffLineStatus::Added).unwrap();
        assert_eq!(json, "\"Added\"");

        let json = serde_json::to_string(&DiffLineStatus::Modified).unwrap();
        assert_eq!(json, "\"Modified\"");
    }
}
