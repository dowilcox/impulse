//! A pane's agent turns (`impulse turns`, `list_turns`): the latest ones,
//! oldest first, with the checkpoints each started and ended at.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::json::iso8601;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnList {
    pub turns: Vec<Turn>,
    /// Earlier turns left out.
    pub older: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub agent: String,
    #[serde(with = "iso8601")]
    pub started: SystemTime,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "iso8601::option"
    )]
    pub ended: Option<SystemTime>,
    /// Checkpoint refs: `git diff <start> <end>` is what the turn changed.
    pub start: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

impl TurnList {
    /// How many turns `impulse turns` and `list_turns` show unless asked:
    /// the latest ones, oldest first.
    pub const DEFAULT_LIMIT: usize = 10;

    /// The latest `limit` of `all`.
    pub fn latest(all: Vec<Turn>, limit: usize) -> Self {
        let limit = limit.max(1);
        let older = all.len().saturating_sub(limit);
        TurnList {
            turns: all.into_iter().skip(older).collect(),
            older: older as i64,
        }
    }

    pub fn text(&self) -> String {
        if self.turns.is_empty() {
            return "No agent turns recorded in this terminal yet.".to_string();
        }
        // Numbered as the terminal's turns, earlier ones counted.
        let mut lines: Vec<String> = Vec::new();
        if self.older > 0 {
            lines.push(format!(
                "({} earlier turn{} not shown)",
                self.older,
                if self.older == 1 { "" } else { "s" }
            ));
        }
        for (index, turn) in self.turns.iter().enumerate() {
            let span = match &turn.end {
                Some(end) => format!("{} {end}", turn.start),
                None => format!("{} (still working)", turn.start),
            };
            lines.push(format!(
                "{}. {}, {}: {span}",
                self.older + index as i64 + 1,
                turn.agent,
                iso8601::format(turn.started)
            ));
        }
        lines.join("\n")
    }
}
