//! Unified-diff hunks for one file, HEAD against the index and working tree
//! (the original Rust's `file_hunks`), with size guards and word-level spans.
//! Only changed regions and their context are materialized. The JSON keys are
//! the original's (snake_case, explicit nulls), which the fixtures pin.

use git2::{DiffLine as Git2Line, Patch};
use serde::{Deserialize, Serialize};

use crate::language::language_for_path;
use crate::path_validation::validate_rel_path_lexically;
use crate::raw::{file_size, head_tree, join_path, lossy, open_repo, workdir};
use crate::word_diff::assign_word_spans;
use crate::{MAX_DIFF_CONTENT_SIZE, MAX_DIFF_HUNKS, MAX_DIFF_LINE_LENGTH, MAX_DIFF_TOTAL_LINES};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

/// A changed range within a diff line, in UTF-16 code units so it maps onto
/// JavaScript string offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WordSpan {
    /// Inclusive.
    pub start: u32,
    /// Exclusive.
    pub end: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// 1-based old-file line (context and removed lines).
    pub old_lineno: Option<u32>,
    /// 1-based new-file line (context and added lines).
    pub new_lineno: Option<u32>,
    /// Without the trailing newline.
    pub content: String,
    /// Word-diff ranges for changed lines (empty otherwise).
    pub spans: Vec<WordSpan>,
}

/// A run of changed lines plus the context around it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// The `@@ ... @@` header, with any function context after it.
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHunks {
    /// Monaco language id.
    pub language: String,
    /// Either side is binary (hunks left out).
    pub is_binary: bool,
    /// Over a size or complexity guard (hunks left out).
    pub too_large: bool,
    /// More hunks or lines exist than were included.
    pub truncated: bool,
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<DiffHunk>,
}

/// The hunks of the repo-relative `file_path`, HEAD against the index and
/// working tree. `None` on error.
pub fn file_hunks(repo_path: &str, file_path: &str) -> Option<FileHunks> {
    let repo = open_repo(repo_path).ok()?;
    let workdir = workdir(&repo).ok()?;
    // Lexically only: a deleted file is a valid target.
    validate_rel_path_lexically(&workdir, file_path).ok()?;
    let language = language_for_path(&join_path(&workdir, file_path));
    let blank = |is_binary: bool, too_large: bool| FileHunks {
        language: language.clone(),
        is_binary,
        too_large,
        truncated: false,
        added: 0,
        removed: 0,
        hunks: Vec::new(),
    };

    let head = head_tree(&repo);
    let mut options = git2::DiffOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    let mut diff = repo
        .diff_tree_to_workdir_with_index(head.as_ref(), Some(&mut options))
        .ok()?;
    // Pair renames so a rename diffs old to new content, not all added.
    let _ = diff.find_similar(None);

    // The delta for this path (the new side for most, the old for
    // deletions).
    let found = diff.deltas().enumerate().find(|(_, delta)| {
        delta.new_file().path_bytes().map(lossy).as_deref() == Some(file_path)
            || delta.old_file().path_bytes().map(lossy).as_deref() == Some(file_path)
    });
    let Some((index, delta)) = found else {
        // Nothing changed (or it's committed already).
        return Some(blank(false, false));
    };
    if delta.flags().is_binary() {
        return Some(blank(true, false));
    }
    // Size guard: either side over the byte limit isn't diffed.
    let worktree_too_big = delta
        .new_file()
        .path_bytes()
        .and_then(|p| file_size(&join_path(&workdir, &lossy(p))))
        .is_some_and(|size| size > MAX_DIFF_CONTENT_SIZE);
    let old_id = delta.old_file().id();
    let blob_too_big = !old_id.is_zero()
        && repo
            .find_blob(old_id)
            .is_ok_and(|blob| blob.size() as u64 > MAX_DIFF_CONTENT_SIZE);
    if worktree_too_big || blob_too_big {
        return Some(blank(false, true));
    }

    // libgit2 makes no patch for a binary delta.
    let Some(patch) = Patch::from_diff(&diff, index).ok()? else {
        return Some(blank(true, false));
    };
    // The binary flag is only reliable once the patch content is computed.
    if patch.delta().flags().is_binary() {
        return Some(blank(true, false));
    }

    let mut hunks = Vec::new();
    let mut added = 0u32;
    let mut removed = 0u32;
    let mut total_lines = 0usize;
    let mut truncated = false;
    'outer: for hunk_index in 0..patch.num_hunks() {
        if hunks.len() >= MAX_DIFF_HUNKS || total_lines >= MAX_DIFF_TOTAL_LINES {
            truncated = true;
            break;
        }
        let (raw_hunk, lines_in_hunk) = patch.hunk(hunk_index).ok()?;
        let header = hunk_header(raw_hunk.header());
        let (old_start, old_lines) = (raw_hunk.old_start(), raw_hunk.old_lines());
        let (new_start, new_lines) = (raw_hunk.new_start(), raw_hunk.new_lines());

        let mut lines = Vec::new();
        for line_index in 0..lines_in_hunk {
            if total_lines >= MAX_DIFF_TOTAL_LINES {
                truncated = true;
                hunks.push(DiffHunk {
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                    header,
                    lines,
                });
                break 'outer;
            }
            let raw_line = patch.line_in_hunk(hunk_index, line_index).ok()?;
            let kind = match raw_line.origin() {
                '+' | '>' => DiffLineKind::Added,
                '-' | '<' => DiffLineKind::Removed,
                _ => DiffLineKind::Context,
            };
            let content = line_content(&raw_line);
            // An overlong line would choke the renderer: the whole file is
            // too large instead.
            if content.len() > MAX_DIFF_LINE_LENGTH {
                return Some(blank(false, true));
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
        }
        assign_word_spans(&mut lines);
        hunks.push(DiffHunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
            header,
            lines,
        });
    }

    Some(FileHunks {
        language,
        is_binary: false,
        too_large: false,
        truncated,
        added,
        removed,
        hunks,
    })
}

/// A hunk's `@@ ... @@` header with its trailing newlines trimmed.
pub(crate) fn hunk_header(header: &[u8]) -> String {
    let mut text = lossy(header);
    while text.ends_with('\n') {
        text.pop();
    }
    text
}

/// A line's text without its newline (or CRLF), invalid UTF-8 replaced.
pub(crate) fn line_content(line: &Git2Line) -> String {
    let mut content = lossy(line.content());
    if content.ends_with('\n') {
        content.pop();
        if content.ends_with('\r') {
            content.pop();
        }
    }
    content
}
