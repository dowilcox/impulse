//! The commands in a workspace's terminals (`impulse blocks`,
//! `list_command_blocks`) and one command's output (`impulse blocks show`,
//! `get_block_output`).

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::json::iso8601;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockList {
    pub terminals: Vec<Terminal>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// The caller's own terminal.
    pub is_caller: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    /// "<terminal>:<block>": what `get_block_output` takes.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(with = "iso8601")]
    pub started: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub running: bool,
}

impl BlockList {
    /// The most recent blocks listed per terminal.
    pub const BLOCKS_PER_TERMINAL: usize = 20;

    pub fn text(&self) -> String {
        if self.terminals.is_empty() {
            return "No terminals in this workspace.".to_string();
        }
        let mut out: Vec<String> = Vec::new();
        for terminal in &self.terminals {
            let mut title = format!("{} [{}]", terminal.title, terminal.id);
            if terminal.is_caller {
                title += " ← you";
            }
            if let Some(agent) = &terminal.agent {
                title += &format!(" ({agent})");
            }
            out.push(title);
            if terminal.blocks.is_empty() {
                out.push("  no commands yet".to_string());
            }
            for block in &terminal.blocks {
                let mut line = format!(
                    "  {}  {}",
                    block.id,
                    block.command.as_deref().unwrap_or("(no command)")
                );
                if block.running {
                    line += "  (running)";
                } else if let Some(code) = block.exit_code {
                    line += &if code == 0 {
                        "  ok".to_string()
                    } else {
                        format!("  exit {code}")
                    };
                }
                out.push(line);
            }
            out.push(String::new());
        }
        trim_newlines(&out.join("\n")).to_string()
    }

    /// "<terminal>:<block>" → its parts.
    pub fn parse(id: &str) -> Option<(String, u64)> {
        let parts: Vec<&str> = id.split(':').collect();
        if parts.len() != 2 || parts[0].is_empty() {
            return None;
        }
        let digits = parts[1].strip_prefix('+').unwrap_or(parts[1]);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some((parts[0].to_string(), digits.parse().ok()?))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockOutput {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub running: bool,
    /// Lines the block's output has, and how many are returned (its last).
    pub total_lines: i64,
    pub shown_lines: i64,
    pub output: String,
}

impl BlockOutput {
    pub const DEFAULT_LINES: usize = 200;
    pub const MAX_LINES: usize = 2000;

    /// The last `lines` of `output` (at most [`BlockOutput::MAX_LINES`]).
    pub fn new(
        id: &str,
        command: Option<String>,
        folder: Option<String>,
        exit_code: Option<i32>,
        running: bool,
        output: &str,
        lines: usize,
    ) -> Self {
        let trimmed = trim_newlines(output);
        let all: Vec<&str> = trimmed.split('\n').collect();
        let count = if trimmed.is_empty() { 0 } else { all.len() };
        let shown = count.min(lines.clamp(1, Self::MAX_LINES));
        BlockOutput {
            id: id.to_string(),
            command,
            folder,
            exit_code,
            running,
            total_lines: count as i64,
            shown_lines: shown as i64,
            output: if count == 0 {
                String::new()
            } else {
                all[all.len() - shown..].join("\n")
            },
        }
    }

    pub fn text(&self) -> String {
        let mut head = format!("`{}`", self.command.as_deref().unwrap_or("a command"));
        if let Some(folder) = &self.folder {
            head += &format!(" in {folder}");
        }
        if self.running {
            head += " (still running)";
        } else if let Some(code) = self.exit_code {
            head += &if code == 0 {
                " (succeeded)".to_string()
            } else {
                format!(" (exit status {code})")
            };
        }
        if self.total_lines > self.shown_lines {
            head += &format!(", last {} of {} lines", self.shown_lines, self.total_lines);
        }
        if self.output.is_empty() {
            head + ": no output."
        } else {
            format!("{head}:\n{}", self.output)
        }
    }
}

/// Foundation's `.newlines` trimmed from both ends.
fn trim_newlines(text: &str) -> &str {
    text.trim_matches(|c| {
        matches!(
            c,
            '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        )
    })
}
