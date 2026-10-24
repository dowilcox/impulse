//! What a diff compares, the kinds of change to a file, and the multi-step
//! operations git can be in the middle of. Shared by the review UI (scope
//! picker), the git layer (which builds the diff) and the review protocol.

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffScope {
    /// HEAD to working tree (staged and unstaged together).
    Uncommitted,
    /// Index to working tree (not yet staged), including untracked files.
    Unstaged,
    /// HEAD to index (what the next commit contains).
    Staged,
    /// merge-base(base, HEAD) to working tree: everything this branch
    /// changed relative to `base` (e.g. "main" or "origin/main"), including
    /// uncommitted work.
    Branch { base: String },
    /// A single commit against its first parent.
    Commit { sha: String },
    /// Tree of `from` to tree of `to`.
    Range { from: String, to: String },
    /// A stash entry against the commit it was made on.
    Stash { index: usize },
    /// A snapshot ref (e.g. an agent-turn checkpoint) to another ref, or the
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

    /// Whether hunks/lines in this diff can be staged (unstaged) or unstaged
    /// (staged) against the index.
    pub fn supports_staging(&self) -> bool {
        matches!(self, DiffScope::Unstaged | DiffScope::Staged)
    }

    /// Short label for pickers and headers, with times in the local time
    /// zone as `HH:MM`.
    pub fn title(&self) -> String {
        self.title_with(short_time)
    }

    /// The label, with checkpoint times written by `time` (the app's own
    /// short time style).
    pub fn title_with(&self, time: impl Fn(DateTime<Utc>) -> String) -> String {
        match self {
            DiffScope::Uncommitted => "Uncommitted changes".into(),
            DiffScope::Unstaged => "Unstaged".into(),
            DiffScope::Staged => "Staged".into(),
            DiffScope::Branch { base } => format!("vs {base}"),
            DiffScope::Commit { sha } => {
                format!("Commit {}", sha.chars().take(7).collect::<String>())
            }
            DiffScope::Range { from, to } => {
                format!("{}…{}", short_revision(from), short_revision(to))
            }
            DiffScope::Stash { index } => format!("stash@{{{index}}}"),
            DiffScope::Snapshot { from, to } => {
                if let Some(reviewed) = checkpoint_date(from, "reviews") {
                    return format!("Since your review at {}", time(reviewed));
                }
                if let Some(started) = checkpoint_date(from, "checkpoints") {
                    let verb = if to.is_none() { "since" } else { "at" };
                    return format!("Agent turn {verb} {}", time(started));
                }
                match to {
                    None => format!("Since {}", short_ref(from)),
                    Some(to) => format!("{}…{}", short_ref(from), short_ref(to)),
                }
            }
        }
    }
}

/// `date` in the local time zone, as `HH:MM`.
pub fn short_time(date: DateTime<Utc>) -> String {
    Local
        .from_utc_datetime(&date.naive_utc())
        .format("%H:%M")
        .to_string()
}

/// When an agent-turn checkpoint ref (`.../impulse/checkpoints/<id>/<millis>-...`)
/// or a review ref (`folder` "reviews") was taken.
pub fn checkpoint_date(reference: &str, folder: &str) -> Option<DateTime<Utc>> {
    if !reference.contains(&format!("/impulse/{folder}/")) {
        return None;
    }
    let last = reference.split('/').rfind(|p| !p.is_empty())?;
    let millis: f64 = last.split('-').find(|p| !p.is_empty())?.parse().ok()?;
    if !millis.is_finite() {
        return None;
    }
    DateTime::from_timestamp_millis(millis as i64)
}

fn short_ref(reference: &str) -> String {
    short_revision(
        reference
            .split('/')
            .rfind(|p| !p.is_empty())
            .unwrap_or(reference),
    )
}

/// A full commit ID shortened to 7 characters, as git shows it; branch and
/// tag names stay whole.
pub fn short_revision(revision: &str) -> String {
    let is_object_id = revision.len() >= 40 && revision.chars().all(|c| c.is_ascii_hexdigit());
    if is_object_id {
        revision[..7].to_string()
    } else {
        revision.to_string()
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
    /// Repo-relative previous path for renames/copies.
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
        Self {
            path: path.into(),
            old_path: None,
            status,
            added: None,
            removed: None,
            is_binary: false,
        }
    }
}

/// An in-progress multi-step operation (git's "repository state").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(from: &str, to: Option<&str>) -> DiffScope {
        DiffScope::Snapshot {
            from: from.into(),
            to: to.map(String::from),
        }
    }

    #[test]
    fn checkpoint_and_review_refs_get_friendly_titles() {
        let millis = "1700000000000";
        let time = short_time(DateTime::from_timestamp(1_700_000_000, 0).unwrap());
        let start = format!("refs/impulse/checkpoints/ABC-123/{millis}-turn-start");
        assert_eq!(
            snapshot(&start, None).title(),
            format!("Agent turn since {time}")
        );
        assert_eq!(
            snapshot(&start, Some(&start)).title(),
            format!("Agent turn at {time}")
        );
        assert_eq!(
            snapshot(&format!("refs/impulse/reviews/{millis}-reviewed"), None).title(),
            format!("Since your review at {time}")
        );
        // Other refs keep the short form.
        assert_eq!(
            snapshot("refs/impulse/oplog/123-discard", None).title(),
            "Since 123-discard"
        );
        // The app can write the time its own way.
        assert_eq!(
            snapshot(&start, None).title_with(|date| date.timestamp().to_string()),
            "Agent turn since 1700000000"
        );
    }

    #[test]
    fn full_commit_ids_are_shortened() {
        let a = "4e1b9c2d0a5f6e7d8c9b0a1f2e3d4c5b6a7f8e9d";
        let b = "9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b";
        assert_eq!(snapshot(a, None).title(), "Since 4e1b9c2");
        assert_eq!(snapshot(a, Some(b)).title(), "4e1b9c2…9a8b7c6");
        let range = |from: &str, to: &str| DiffScope::Range {
            from: from.into(),
            to: to.into(),
        };
        assert_eq!(range(a, b).title(), "4e1b9c2…9a8b7c6");
        // Names (and short IDs) are left as they are.
        assert_eq!(range("main", "feature/cache").title(), "main…feature/cache");
        assert_eq!(snapshot("abc1234", None).title(), "Since abc1234");
    }

    #[test]
    fn scopes_and_operations() {
        assert!(DiffScope::Branch {
            base: "main".into()
        }
        .includes_working_tree());
        assert!(!snapshot("a", Some("b")).includes_working_tree());
        assert!(DiffScope::Staged.supports_staging() && !DiffScope::Uncommitted.supports_staging());
        assert_eq!(DiffScope::Stash { index: 2 }.title(), "stash@{2}");
        assert_eq!(
            RepoOperation::Rebase {
                step: Some(2),
                total: Some(5)
            }
            .title(),
            "Rebasing 2/5"
        );
        assert_eq!(
            RepoOperation::Rebase {
                step: None,
                total: Some(5)
            }
            .title(),
            "Rebasing"
        );
        assert!(RepoOperation::CherryPick.can_skip() && !RepoOperation::Merge.can_skip());
        assert_eq!(ChangeStatus::TypeChanged.letter(), "T");
        assert_eq!(
            serde_json::to_string(&ChangeStatus::TypeChanged).unwrap(),
            r#""type_changed""#
        );
    }
}
