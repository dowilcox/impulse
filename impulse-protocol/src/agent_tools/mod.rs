//! What Impulse answers agents with: the JSON the `impulse` commands print
//! with `--json` and `impulse mcp` returns as a tool's structured result,
//! and the text form of each for people and for agents that only read
//! text. The app fills them in; they're kept here so their shape and
//! wording are tested headless, and so both apps write the same JSON (the
//! Swift `AgentToolResults.swift` and `TaskListing.swift`).
//!
//! Every answer is encoded the same way ([`data`], [`pretty`]): keys sorted,
//! slashes left alone, dates as ISO 8601 in UTC, absent optional fields
//! left out.

mod blocks;
mod comments;
mod problems;
mod project;
mod task_detail;
mod task_listing;
mod turns;

use serde::Serialize;

pub use blocks::{Block, BlockList, BlockOutput, Terminal};
pub use comments::{code_fence, CommentEntry, CommentList, Resolution, ReviewComment, Side};
pub use problems::{grouped, Counts, Problem, ProblemEntry, ProblemList, Severity};
pub use project::{
    task_port, task_ports, Action, Detected, ProjectScripts, ProjectSummary, Rule, UnsavedFile,
    UnsavedList,
};
pub use task_detail::{Commit, TaskDetail};
pub use task_listing::{hook_reply, TaskListing, Workspace};
pub use turns::{Turn, TurnList};

use crate::json::{self, Style};

/// An answer as compact JSON, for `ControlResponse::data`.
pub fn data<T: Serialize>(value: &T) -> Vec<u8> {
    json::encode(value, Style::COMPACT).into_bytes()
}

/// An answer as pretty JSON, for `--json`.
pub fn pretty<T: Serialize>(value: &T) -> String {
    json::encode(value, Style::PRETTY)
}
