//! The user's review comments as agents see them (`impulse comments`,
//! `list_review_comments`), and whether each still matches its lines.
//!
//! [`ReviewComment`] and the anchoring checks are the Swift ImpulseKit's
//! (`Review/ReviewComments.swift`), ported here, with the same JSON, until
//! Review itself is ported.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::json::iso8601;

/// A review comment anchored to lines of one side of a file's diff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: String,
    pub path: String,
    pub side: Side,
    /// 1-based first and last line on `side`.
    pub line: i64,
    pub end_line: i64,
    /// The anchored lines' text when the comment was written
    /// (newline-joined).
    pub snippet: String,
    pub text: String,
    #[serde(with = "iso8601")]
    pub created_at: SystemTime,
    /// An agent marked it addressed (`impulse comments resolve`), for you
    /// to accept (delete) or reopen. Comments saved before this read as
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Old,
    New,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// Who addressed it: the agent's name ("Claude Code").
    pub by: String,
    pub note: String,
    #[serde(with = "iso8601")]
    pub at: SystemTime,
}

impl ReviewComment {
    pub fn new(
        id: impl Into<String>,
        path: impl Into<String>,
        side: Side,
        line: i64,
        end_line: i64,
        snippet: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        ReviewComment {
            id: id.into(),
            path: path.into(),
            side,
            line,
            end_line: end_line.max(line),
            snippet: snippet.into(),
            text: text.into(),
            created_at: SystemTime::now(),
            resolution: None,
        }
    }

    /// "path:12" or "path:12-14".
    pub fn location(&self) -> String {
        if self.end_line > self.line {
            format!("{}:{}-{}", self.path, self.line, self.end_line)
        } else {
            format!("{}:{}", self.path, self.line)
        }
    }

    /// Outdated when the lines it was written on no longer read the same at
    /// the same place. `line_text` gives a line's text (1-based) for the
    /// comment's side; a line it doesn't have also makes it outdated.
    pub fn is_outdated(&self, line_text: impl Fn(i64) -> Option<String>) -> bool {
        if self.snippet.is_empty() {
            return false;
        }
        self.snippet
            .split('\n')
            .enumerate()
            .any(|(offset, expected)| {
                line_text(self.line + offset as i64).as_deref() != Some(expected)
            })
    }

    /// Whether its lines still read as they did, against a file's current
    /// text: only for comments on the new side (the working tree's lines);
    /// `None` for removed lines. No text (the file is gone) is outdated.
    pub fn freshness(&self, current_text: Option<&str>) -> Option<bool> {
        if self.side != Side::New {
            return None;
        }
        let Some(current_text) = current_text else {
            return Some(true);
        };
        let lines: Vec<&str> = current_text.split('\n').collect();
        Some(self.is_outdated(|number| {
            usize::try_from(number - 1)
                .ok()
                .and_then(|index| lines.get(index))
                .map(|line| line.to_string())
        }))
    }
}

/// A fence the snippet can't close: one backtick longer than its longest
/// run of backticks, and at least three.
pub fn code_fence(snippet: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for character in snippet.chars() {
        run = if character == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    "`".repeat((longest + 1).max(3))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentList {
    pub repository: String,
    pub comments: Vec<CommentEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentEntry {
    pub id: String,
    pub path: String,
    /// "src/a.ts:12-14".
    pub location: String,
    pub side: Side,
    pub line: i64,
    pub end_line: i64,
    pub snippet: String,
    pub text: String,
    /// Its lines read differently now (or `None` when that can't be told: a
    /// comment on removed lines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outdated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
}

impl CommentEntry {
    pub fn new(comment: &ReviewComment, outdated: Option<bool>) -> Self {
        CommentEntry {
            id: comment.id.clone(),
            path: comment.path.clone(),
            location: comment.location(),
            side: comment.side,
            line: comment.line,
            end_line: comment.end_line,
            snippet: comment.snippet.clone(),
            text: comment.text.clone(),
            outdated,
            resolution: comment.resolution.clone(),
        }
    }
}

impl CommentList {
    pub fn text(&self) -> String {
        if self.comments.is_empty() {
            return format!("No review comments in {}.", self.repository);
        }
        let count = self.comments.len();
        let mut out = format!(
            "{count} review comment{} in {}:\n",
            if count == 1 { "" } else { "s" },
            self.repository
        );
        for comment in &self.comments {
            let mut title = format!("\n[{}] {}", comment.id, comment.location);
            if comment.side == Side::Old {
                title += " (removed lines)";
            }
            if comment.outdated == Some(true) {
                title += " (outdated)";
            }
            if let Some(resolution) = &comment.resolution {
                title += &format!(" (addressed by {})", resolution.by);
            }
            out += &title;
            out.push('\n');
            if !comment.snippet.is_empty() {
                let fence = code_fence(&comment.snippet);
                out += &format!("{fence}\n{}\n{fence}\n", comment.snippet);
            }
            out += comment.text.trim();
            out.push('\n');
            if let Some(resolution) = &comment.resolution {
                out += &format!("Addressed: {}\n", resolution.note);
            }
        }
        out
    }
}
