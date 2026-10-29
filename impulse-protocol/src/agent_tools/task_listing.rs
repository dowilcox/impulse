//! What `impulse tasks` tells an agent (or you) about a repository's
//! workspaces: the main checkout and each task Impulse made, with its
//! branch, base, agent, uncommitted files, how far it is from its base, and
//! the files it shares with the caller. Agents only know their own folder;
//! this is how one learns what the others are doing. The session-start hook
//! sends a short form of it, and the hooks' replies are built here too.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::json::{self, Style};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListing {
    pub repository: String,
    pub workspaces: Vec<Workspace>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The branch it was made from (`origin/main`); `None` for the main
    /// checkout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    pub is_main_checkout: bool,
    /// The caller's own workspace.
    pub is_caller: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// "working", "needs input", "finished" or "idle".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_state: Option<String>,
    pub uncommitted: i64,
    /// Commits on it that its base lacks, and the reverse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ahead: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind: Option<i64>,
    /// Files it changes that the caller's workspace changes too.
    pub shared_with_caller: Vec<String>,
    /// "agent" for a task an agent asked for (`impulse tasks new`), "user"
    /// for one you made; `None` for the main checkout and older tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// Who asked for it: "Claude Code in trailhead".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
    /// The first line of the brief it was started with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
}

fn plural(count: impl Into<i64>) -> &'static str {
    if count.into() == 1 {
        ""
    } else {
        "s"
    }
}

/// The first `count` characters.
pub(crate) fn prefix(text: &str, count: usize) -> &str {
    match text.char_indices().nth(count) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

impl TaskListing {
    /// Pretty JSON, what `impulse tasks --json` prints.
    pub fn json(&self) -> String {
        json::encode(self, Style::PRETTY)
    }

    /// One block per workspace, for people and agents alike.
    pub fn text(&self) -> String {
        let count = self.workspaces.len() as i64;
        let mut lines = vec![format!(
            "{}: {count} workspace{}",
            self.repository,
            plural(count)
        )];
        for workspace in &self.workspaces {
            lines.push(String::new());
            let mut title = workspace.name.clone()
                + if workspace.is_main_checkout {
                    " (main checkout)"
                } else {
                    " (task)"
                };
            if workspace.is_caller {
                title += " ← you";
            }
            lines.push(title);
            let mut branch = format!(
                "  branch {}",
                workspace.branch.as_deref().unwrap_or("detached")
            );
            if let Some(base) = &workspace.base {
                branch += &format!(" from {base}");
            }
            if let (Some(ahead), Some(behind)) = (workspace.ahead, workspace.behind) {
                branch += &format!(", {ahead} ahead, {behind} behind");
            }
            if let Some(head) = &workspace.head {
                branch += &format!(" at {}", prefix(head, 7));
            }
            lines.push(branch);
            lines.push(format!("  {}", workspace.path));
            if let Some(agent) = &workspace.agent {
                lines.push(format!(
                    "  {agent}: {}",
                    workspace.agent_state.as_deref().unwrap_or("running")
                ));
            }
            if let Some(requested_by) = &workspace.requested_by {
                lines.push(format!("  started at the request of {requested_by}"));
            }
            if let Some(brief) = workspace.brief.as_deref().filter(|b| !b.is_empty()) {
                lines.push(format!("  brief: {brief}"));
            }
            if workspace.uncommitted > 0 {
                lines.push(format!(
                    "  {} uncommitted file{}",
                    workspace.uncommitted,
                    plural(workspace.uncommitted)
                ));
            }
            if !workspace.shared_with_caller.is_empty() {
                let shared = &workspace.shared_with_caller;
                let shown = shared
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                let more = if shared.len() > 8 {
                    format!(" and {} more", shared.len() - 8)
                } else {
                    String::new()
                };
                lines.push(format!("  also changes files you change: {shown}{more}"));
            }
        }
        lines.join("\n")
    }

    /// A few lines for an agent starting in the caller's workspace; `None`
    /// when it's the only workspace (nothing to coordinate with).
    pub fn session_summary(&self) -> Option<String> {
        if self.workspaces.len() <= 1 {
            return None;
        }
        let me = self.workspaces.iter().find(|w| w.is_caller)?;
        let mut lines = Vec::new();
        if me.is_main_checkout {
            let tasks = self.workspaces.len() - 1;
            lines.push(format!(
                "Impulse: you're in the main checkout of {}, which has {tasks} task{} (git worktrees) other agents may be working in.",
                self.repository,
                if self.workspaces.len() == 2 { "" } else { "s" }
            ));
        } else {
            lines.push(format!(
                "Impulse: you're in the task {} of {}: a git worktree on branch {}{}.",
                me.name,
                self.repository,
                me.branch.as_deref().unwrap_or("?"),
                me.base
                    .as_ref()
                    .map(|base| format!(", made from {base}"))
                    .unwrap_or_default()
            ));
        }
        let others: Vec<String> = self
            .workspaces
            .iter()
            .filter(|w| !w.is_caller)
            .map(|other| {
                let mut parts = Vec::new();
                if let Some(agent) = &other.agent {
                    parts.push(format!(
                        "{agent} {}",
                        other.agent_state.as_deref().unwrap_or("running")
                    ));
                }
                if other.uncommitted > 0 {
                    parts.push(format!("{} uncommitted", other.uncommitted));
                }
                if !other.shared_with_caller.is_empty() {
                    parts.push(format!(
                        "changes {} of your files",
                        other.shared_with_caller.len()
                    ));
                }
                other.name.clone()
                    + if other.is_main_checkout {
                        " (main checkout)"
                    } else {
                        ""
                    }
                    + &if parts.is_empty() {
                        String::new()
                    } else {
                        " — ".to_string() + &parts.join(", ")
                    }
            })
            .collect();
        lines.push(format!("Other workspaces: {}.", others.join("; ")));
        lines.push(
            "Run `impulse tasks` (or Impulse's list_tasks tool, if you have its MCP server) before merging a branch or making sweeping edits, to see what they're changing."
                .to_string(),
        );
        Some(lines.join("\n"))
    }
}

/// The JSON Claude Code hooks reply with, and the simpler JSON Impulse's
/// opencode plugin reads.
pub mod hook_reply {
    use super::*;

    /// A note added to the agent's context after a tool ran
    /// (`PostToolUse`, unless `event` says otherwise).
    pub fn context(text: &str, event: &str) -> String {
        encode(json!({
            "hookSpecificOutput": {"hookEventName": event, "additionalContext": text}
        }))
    }

    /// Make the agent ask the user before running a tool (`PreToolUse`),
    /// with the reason shown in the prompt.
    pub fn ask(reason: &str) -> String {
        encode(json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "ask",
                "permissionDecisionReason": reason,
            }
        }))
    }

    /// Stop a tool call before it runs (`PreToolUse`), telling the agent
    /// why: it may try again.
    pub fn deny(reason: &str) -> String {
        encode(json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        }))
    }

    /// For the opencode plugin: text for the agent's context (a session's
    /// summary goes beside opencode's instructions, a note after the tool's
    /// output).
    pub fn opencode_context(text: &str) -> String {
        encode(json!({"context": text}))
    }

    /// For the opencode plugin: have opencode ask the user before running
    /// the command, with the reason shown in its permission prompt.
    pub fn opencode_ask(reason: &str) -> String {
        encode(json!({"ask": reason}))
    }

    /// For the opencode plugin: stop the tool call before it runs, telling
    /// the agent why (the plugin throws it as the tool's error).
    pub fn opencode_deny(reason: &str) -> String {
        encode(json!({"deny": reason}))
    }

    fn encode(value: serde_json::Value) -> String {
        json::write(&value, Style::COMPACT)
    }
}
