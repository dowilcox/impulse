//! One task in detail (`impulse tasks show`, `get_task`): what the listing
//! says about it, plus its files, commits, merged state, slot and brief.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::task_listing::{prefix, TaskListing, Workspace};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    pub workspace: Workspace,
    /// Files its branch changes against its base.
    pub changed_files: Vec<String>,
    /// Files with uncommitted changes in its folder.
    pub uncommitted_files: Vec<String>,
    /// Its commits that its base lacks, newest first (at most
    /// [`TaskDetail::COMMIT_LIMIT`]).
    pub commits: Vec<Commit>,
    /// More commits than listed.
    pub more_commits: i64,
    /// Merged into its base (squash merges included); `None` for the main
    /// checkout, or when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<i64>,
    /// Its own port values (`APP_PORT` = 8100), when the project gives
    /// tasks values.
    pub ports: BTreeMap<String, i64>,
    /// What it was started with, when an agent's request made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub id: String,
    pub subject: String,
}

impl TaskDetail {
    pub const COMMIT_LIMIT: usize = 50;

    pub fn new(workspace: Workspace) -> Self {
        TaskDetail {
            workspace,
            ..Default::default()
        }
    }

    pub fn text(&self) -> String {
        let listing = TaskListing {
            repository: String::new(),
            workspaces: vec![self.workspace.clone()],
        };
        let mut lines: Vec<String> = listing.text().split('\n').map(String::from).collect();
        // Drop the listing's "repo: 1 workspace" header and blank line.
        lines.drain(..lines.len().min(2));
        if let Some(merged) = self.merged {
            lines.push(
                if merged {
                    "  merged into its base"
                } else {
                    "  not merged into its base yet"
                }
                .to_string(),
            );
        }
        if let Some(slot) = self.slot {
            let mut line = format!("  slot {slot}");
            if !self.ports.is_empty() {
                line += ": ";
                line += &ports_text(&self.ports);
            }
            lines.push(line);
        }
        if !self.commits.is_empty() {
            lines.push(String::new());
            lines.push("Commits ahead of its base:".to_string());
            for commit in &self.commits {
                lines.push(format!("  {} {}", prefix(&commit.id, 7), commit.subject));
            }
            if self.more_commits > 0 {
                lines.push(format!("  …and {} more", self.more_commits));
            }
        }
        if !self.changed_files.is_empty() {
            lines.push(String::new());
            lines.push("Changed against its base:".to_string());
            lines.extend(self.changed_files.iter().map(|file| format!("  {file}")));
        }
        if !self.uncommitted_files.is_empty() {
            lines.push(String::new());
            lines.push("Uncommitted:".to_string());
            lines.extend(
                self.uncommitted_files
                    .iter()
                    .map(|file| format!("  {file}")),
            );
        }
        if let Some(brief) = self.brief.as_deref().filter(|b| !b.is_empty()) {
            lines.push(String::new());
            lines.push("Brief:".to_string());
            lines.extend(brief.split('\n').map(|line| format!("  {line}")));
        }
        lines.join("\n")
    }
}

/// "APP_PORT 8100, DB_PORT 5532", by name.
pub(crate) fn ports_text(ports: &BTreeMap<String, i64>) -> String {
    ports
        .iter()
        .map(|(name, port)| format!("{name} {port}"))
        .collect::<Vec<_>>()
        .join(", ")
}
