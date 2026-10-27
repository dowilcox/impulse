//! Impulse's git layer, the Rust port of the Swift `ImpulseGit` target
//! (docs/plans/rust-migration.md).
//!
//! Reads (status, diffs and hunks with word-level spans, blame, refs) go
//! through libgit2, built from the `git2` crate's vendored libgit2 1.9 with no
//! HTTPS or SSH transport and so no OpenSSL. Writes (stage, commit, branches,
//! stash, fetch, pull, push, worktrees, conflict resolution) run the user's
//! `git`, so they behave exactly as in a terminal: hooks run, commits are
//! signed per config, filters apply, credential helpers and SSH agents work.
//! Destructive writes take a safety snapshot under `refs/impulse/` first so
//! they can be undone (`safety`).
//!
//! Behavior is checked against the golden fixtures (`tests/fixtures/`,
//! generated from the original Rust at tag `v0.29.0`, which the Swift port
//! answered to as well), and against the real `git` as the oracle for
//! everything newer.

pub mod blame;
pub mod branches;
pub mod buffer_diff;
pub mod change_move;
pub mod changed_files;
pub mod cli;
pub mod file_index;
pub mod git_blame;
pub mod history_query;
pub mod hunks;
pub mod ignored;
pub mod landing;
pub mod language;
pub mod log;
pub mod markers;
pub mod operation_abort;
pub mod operations;
pub mod patch;
pub mod path_validation;
pub mod process;
mod raw;
pub mod repo_cache;
pub mod safety;
pub mod scoped_diff;
pub mod search;
pub mod snapshot;
pub mod status;
pub mod types;
pub mod watcher;
pub mod word_diff;

pub use blame::{line_blame, BlameInfo};
pub use branches::{
    branch_for_path, branches_for_path, common_git_directory, git_directory, repo_root,
};
pub use changed_files::{changed_files, ChangeSet, ChangedFile};
pub use cli::{CliError, CliOutput, ErrorKind};
pub use git_blame::{base_content, BlameLine};
pub use history_query::HistoryQuery;
pub use hunks::{file_hunks, DiffHunk, DiffLine, DiffLineKind, FileHunks, WordSpan};
pub use markers::{diff_markers, DiffMarker};
pub use operations::{GitResult, OperationError};
pub use raw::idle_handle_count;
pub use safety::SafetySnapshot;
pub use scoped_diff::{DiffOptions, FileDiff};
pub use snapshot::{snapshot, RepoSnapshot};
pub use status::{all_statuses, status_for_directory};
pub use types::{ChangeStatus, DiffScope, FileChange, RepoOperation};
pub use watcher::RepoWatcher;

/// Largest file or blob (bytes) whose contents are diffed.
pub const MAX_DIFF_CONTENT_SIZE: u64 = 1_048_576;
/// Longest single line (bytes) before a file counts as too complex to diff
/// inline.
pub const MAX_DIFF_LINE_LENGTH: usize = 20_000;
/// Hunks shown per file before the diff is marked truncated.
pub const MAX_DIFF_HUNKS: usize = 1_500;
/// Diff lines shown per file before the diff is marked truncated.
pub const MAX_DIFF_TOTAL_LINES: usize = 30_000;
/// Intra-line word diffs are skipped when the old and new line together are
/// longer than this (UTF-16 code units).
pub const MAX_WORD_DIFF_LINE_LEN: usize = 2_000;

/// A libgit2 read that failed, with git's own message (the original Rust's
/// `Result<T, String>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitError(pub String);

impl GitError {
    pub fn new(message: impl Into<String>) -> Self {
        GitError(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

impl From<git2::Error> for GitError {
    fn from(error: git2::Error) -> Self {
        GitError(error.message().to_string())
    }
}
