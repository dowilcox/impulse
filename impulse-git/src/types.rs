//! The git types the Swift app keeps in ImpulseKit (`DiffScope.swift`,
//! `GitRefs.swift`) because the review UI and the control protocol share
//! them. In Rust only the git layer needs them so far, so they live here.
//! Their JSON is the Swift `Codable` form: a case with values is an object
//! under its name (`{"branch":{"base":"main"}}`), one without is an empty
//! object (`{"uncommitted":{}}`), and absent optionals are left out.

use serde::{Deserialize, Serialize};

/// What a diff compares.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "ScopeForm", into = "ScopeForm")]
pub enum DiffScope {
    /// HEAD to the working tree (staged and unstaged together).
    Uncommitted,
    /// Index to working tree (not yet staged), including untracked files.
    Unstaged,
    /// HEAD to index (what the next commit contains).
    Staged,
    /// merge-base(base, HEAD) to the working tree: everything this branch
    /// changed relative to `base` ("main", "origin/main"), uncommitted work
    /// included.
    Branch { base: String },
    /// One commit against its first parent.
    Commit { sha: String },
    /// Tree of `from` to tree of `to`.
    Range { from: String, to: String },
    /// A stash entry against the commit it was made on.
    Stash { index: usize },
    /// A snapshot ref (an agent-turn checkpoint) to another ref, or to the
    /// working tree when `to` is `None`.
    Snapshot { from: String, to: Option<String> },
}

impl DiffScope {
    /// Whether the right-hand side is the live working tree (so the diff
    /// changes as files are edited, and hunks can be reverted on disk).
    pub fn includes_working_tree(&self) -> bool {
        match self {
            DiffScope::Uncommitted | DiffScope::Unstaged | DiffScope::Branch { .. } => true,
            DiffScope::Snapshot { to, .. } => to.is_none(),
            DiffScope::Staged
            | DiffScope::Commit { .. }
            | DiffScope::Range { .. }
            | DiffScope::Stash { .. } => false,
        }
    }

    /// Whether hunks and lines in this diff can be staged (unstaged) or
    /// unstaged (staged) against the index.
    pub fn supports_staging(&self) -> bool {
        matches!(self, DiffScope::Unstaged | DiffScope::Staged)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ScopeForm {
    Uncommitted {},
    Unstaged {},
    Staged {},
    Branch {
        base: String,
    },
    Commit {
        sha: String,
    },
    Range {
        from: String,
        to: String,
    },
    Stash {
        index: usize,
    },
    Snapshot {
        from: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
}

impl From<ScopeForm> for DiffScope {
    fn from(form: ScopeForm) -> Self {
        match form {
            ScopeForm::Uncommitted {} => DiffScope::Uncommitted,
            ScopeForm::Unstaged {} => DiffScope::Unstaged,
            ScopeForm::Staged {} => DiffScope::Staged,
            ScopeForm::Branch { base } => DiffScope::Branch { base },
            ScopeForm::Commit { sha } => DiffScope::Commit { sha },
            ScopeForm::Range { from, to } => DiffScope::Range { from, to },
            ScopeForm::Stash { index } => DiffScope::Stash { index },
            ScopeForm::Snapshot { from, to } => DiffScope::Snapshot { from, to },
        }
    }
}

impl From<DiffScope> for ScopeForm {
    fn from(scope: DiffScope) -> Self {
        match scope {
            DiffScope::Uncommitted => ScopeForm::Uncommitted {},
            DiffScope::Unstaged => ScopeForm::Unstaged {},
            DiffScope::Staged => ScopeForm::Staged {},
            DiffScope::Branch { base } => ScopeForm::Branch { base },
            DiffScope::Commit { sha } => ScopeForm::Commit { sha },
            DiffScope::Range { from, to } => ScopeForm::Range { from, to },
            DiffScope::Stash { index } => ScopeForm::Stash { index },
            DiffScope::Snapshot { from, to } => ScopeForm::Snapshot { from, to },
        }
    }
}

/// The kind of change to a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
    Untracked,
    Conflicted,
}

impl ChangeStatus {
    /// One-letter badge, git-status style.
    pub fn letter(self) -> &'static str {
        match self {
            ChangeStatus::Added => "A",
            ChangeStatus::Modified => "M",
            ChangeStatus::Deleted => "D",
            ChangeStatus::Renamed => "R",
            ChangeStatus::TypeChanged => "T",
            ChangeStatus::Untracked => "U",
            ChangeStatus::Conflicted => "C",
        }
    }
}

/// One changed file in a scope or status section.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Repo-relative path (new side for renames).
    pub path: String,
    /// Repo-relative previous path for renames and copies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    /// Line counts; `None` when not computed (binary, too large, or skipped
    /// for very large change sets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<usize>,
    #[serde(default)]
    pub is_binary: bool,
}

impl FileChange {
    pub fn new(path: impl Into<String>, status: ChangeStatus) -> Self {
        FileChange {
            path: path.into(),
            old_path: None,
            status,
            added: None,
            removed: None,
            is_binary: false,
        }
    }
}

/// A multi-step operation in progress (git's "repository state").
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "OperationForm", into = "OperationForm")]
pub enum RepoOperation {
    Merge,
    Rebase {
        step: Option<usize>,
        total: Option<usize>,
    },
    CherryPick,
    Revert,
    Bisect,
    ApplyMailbox,
}

impl RepoOperation {
    pub fn title(&self) -> String {
        match self {
            RepoOperation::Merge => "Merging".into(),
            RepoOperation::Rebase {
                step: Some(step),
                total: Some(total),
            } => format!("Rebasing {step}/{total}"),
            RepoOperation::Rebase { .. } => "Rebasing".into(),
            RepoOperation::CherryPick => "Cherry-picking".into(),
            RepoOperation::Revert => "Reverting".into(),
            RepoOperation::Bisect => "Bisecting".into(),
            RepoOperation::ApplyMailbox => "Applying patches".into(),
        }
    }

    /// Whether `--skip` is meaningful.
    pub fn can_skip(&self) -> bool {
        !matches!(self, RepoOperation::Merge | RepoOperation::Bisect)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum OperationForm {
    Merge {},
    Rebase {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total: Option<usize>,
    },
    CherryPick {},
    Revert {},
    Bisect {},
    ApplyMailbox {},
}

impl From<OperationForm> for RepoOperation {
    fn from(form: OperationForm) -> Self {
        match form {
            OperationForm::Merge {} => RepoOperation::Merge,
            OperationForm::Rebase { step, total } => RepoOperation::Rebase { step, total },
            OperationForm::CherryPick {} => RepoOperation::CherryPick,
            OperationForm::Revert {} => RepoOperation::Revert,
            OperationForm::Bisect {} => RepoOperation::Bisect,
            OperationForm::ApplyMailbox {} => RepoOperation::ApplyMailbox,
        }
    }
}

impl From<RepoOperation> for OperationForm {
    fn from(operation: RepoOperation) -> Self {
        match operation {
            RepoOperation::Merge => OperationForm::Merge {},
            RepoOperation::Rebase { step, total } => OperationForm::Rebase { step, total },
            RepoOperation::CherryPick => OperationForm::CherryPick {},
            RepoOperation::Revert => OperationForm::Revert {},
            RepoOperation::Bisect => OperationForm::Bisect {},
            RepoOperation::ApplyMailbox => OperationForm::ApplyMailbox {},
        }
    }
}

/// Names git accepts for branches and tags (the `git check-ref-format` rules
/// that matter for a single name typed by a person; the Swift `GitRefName`).
pub fn is_valid_ref_name(name: &str) -> bool {
    if name.is_empty()
        || name == "@"
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with(".lock")
        || name.ends_with('.')
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
    {
        return false;
    }
    // No component may start with a dot.
    if name.split('/').any(|component| component.starts_with('.')) {
        return false;
    }
    !name
        .chars()
        .any(|c| " ~^:?*[\\".contains(c) || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scopes_and_operations_encode_as_swift_does() {
        let cases = [
            (DiffScope::Uncommitted, json!({"uncommitted": {}})),
            (
                DiffScope::Branch {
                    base: "main".into(),
                },
                json!({"branch": {"base": "main"}}),
            ),
            (
                DiffScope::Stash { index: 2 },
                json!({"stash": {"index": 2}}),
            ),
            (
                DiffScope::Snapshot {
                    from: "refs/impulse/x".into(),
                    to: None,
                },
                json!({"snapshot": {"from": "refs/impulse/x"}}),
            ),
        ];
        for (scope, expected) in cases {
            assert_eq!(serde_json::to_value(&scope).unwrap(), expected);
            assert_eq!(
                serde_json::from_value::<DiffScope>(expected).unwrap(),
                scope
            );
        }
        let rebase = RepoOperation::Rebase {
            step: Some(2),
            total: Some(3),
        };
        assert_eq!(
            serde_json::to_value(&rebase).unwrap(),
            json!({"rebase": {"step": 2, "total": 3}})
        );
        assert_eq!(
            serde_json::to_value(RepoOperation::CherryPick).unwrap(),
            json!({"cherryPick": {}})
        );
        assert_eq!(rebase.title(), "Rebasing 2/3");

        let change = FileChange {
            old_path: Some("a".into()),
            added: Some(1),
            ..FileChange::new("b", ChangeStatus::TypeChanged)
        };
        assert_eq!(
            serde_json::to_value(&change).unwrap(),
            json!({"path": "b", "oldPath": "a", "status": "type_changed", "added": 1, "isBinary": false})
        );
    }

    #[test]
    fn ref_names() {
        for good in ["main", "feature/x", "v1.2.3", "fix-123", "a@b"] {
            assert!(is_valid_ref_name(good), "{good}");
        }
        for bad in [
            "", "@", "-x", "/x", "x/", "x.lock", "x.", "a..b", "a//b", "a@{1}", "a/.b", "bad name",
            "a~1", "a^", "a:b", "a?", "a*", "a[", "a\\b", "a\tb",
        ] {
            assert!(!is_valid_ref_name(bad), "{bad:?}");
        }
    }
}
