//! What agents' hook events mean to Impulse: Claude Code's hook events,
//! Codex's notify, and the events Impulse's opencode plugin and Pi
//! extension send (Pi's under Claude Code's names where they mean the
//! same). An event may change the agent's state, and Claude Code, the
//! opencode plugin and the Pi extension also read what Impulse answers (a
//! summary of the other tasks, notes about shared files and dependencies, a
//! question before a risky merge, and for Pi whether to trust the folder).

use super::state_machine::AgentHook;

/// The state change `event` reports; `None` for events that don't change
/// it (tools starting, subagents, compaction, a session's summary). A Stop
/// with subagents still running in the background is
/// `AgentHook::AwaitingSubagents`, which the caller, knowing that, makes it.
pub fn hook(
    agent: Option<&str>,
    event: &str,
    tool: Option<&str>,
    message: Option<&str>,
) -> Option<AgentHook> {
    let message = || message.unwrap_or("").to_string();
    match event {
        "SessionStart" => return Some(AgentHook::SessionStarted),
        "UserPromptSubmit" => return Some(AgentHook::PromptSubmitted),
        "PostToolUse" => return Some(AgentHook::UsedTool(tool.map(String::from))),
        "Stop" | "agent-turn-complete" => return Some(AgentHook::Stopped),
        "StopFailure" => return Some(AgentHook::Failed(message())),
        "Notification" => return Some(AgentHook::Notification(message())),
        _ => {}
    }
    if agent == Some("pi") {
        return Some(match event {
            // A dialog waits for you mid-turn (titled, when it is), then
            // it's answered.
            "prompt.start" => AgentHook::Notification(match message() {
                title if title.is_empty() => "Pi needs your input".into(),
                title => format!("Pi needs your input: {title}"),
            }),
            "prompt.end" => AgentHook::UsedTool(None),
            _ => return None,
        });
    }
    if agent != Some("opencode") {
        return None;
    }
    Some(match event {
        // The plugin loaded in the terminal: opencode started.
        "start" => AgentHook::SessionStarted,
        "session.execution.started" => AgentHook::PromptSubmitted,
        "permission.asked" => AgentHook::Notification(format!(
            "opencode needs your permission to use {}",
            tool.unwrap_or("a tool")
        )),
        // opencode's question tool asks with a form.
        "form.created" => AgentHook::Notification("opencode has a question for you".into()),
        "tool.execute.after" => AgentHook::UsedTool(tool.map(String::from)),
        // Answered (or dismissed): back to work, or to the turn's end.
        "permission.replied" | "form.replied" | "form.cancelled" => AgentHook::UsedTool(None),
        // An interrupted turn ends like any other.
        "session.execution.succeeded" | "session.execution.interrupted" => AgentHook::Stopped,
        "session.execution.failed" => AgentHook::Failed(message()),
        _ => return None,
    })
}

/// What Impulse can answer an event with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The repository's other workspaces, when a session starts.
    SessionSummary,
    /// Whether another workspace changes the file just edited.
    EditedFile,
    /// Dependency files a checkout, merge or pull just changed.
    RanShell,
    /// A question before a command that merges a branch a task is still on.
    BeforeShell,
    /// An edit stopped, once, when it would overwrite a file another agent
    /// in the same checkout changed meanwhile.
    BeforeEdit,
    /// A note when an Agent call starts a subagent in a checkout others
    /// are editing.
    BeforeSubagent,
    /// The same note for the subagent itself, as it starts.
    SubagentStarted,
    /// Whether Pi loads the folder's own extensions and settings: yes in a
    /// folder Impulse trusts, no in one it keeps restricted.
    ProjectTrust,
}

/// Claude Code's tool that starts a subagent (`Task` in older versions).
pub const CLAUDE_SUBAGENT_TOOLS: [&str; 2] = ["Agent", "Task"];

/// Claude Code's tools that edit files.
pub const CLAUDE_EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// opencode's tools that edit files (the plugin reports each file a patch
/// changes).
pub const OPENCODE_EDIT_TOOLS: [&str; 3] = ["edit", "write", "patch"];

/// opencode's tool that starts a subagent (a child session).
pub const OPENCODE_SUBAGENT_TOOL: &str = "subagent";

/// Pi's tools that edit files.
pub const PI_EDIT_TOOLS: [&str; 2] = ["edit", "write"];

/// Pi's shell tool.
pub const PI_SHELL_TOOL: &str = "bash";

/// Whether `agent`'s `tool` starts a subagent.
pub fn starts_subagent(agent: Option<&str>, tool: Option<&str>) -> bool {
    match (agent, tool) {
        (Some("claude"), Some(tool)) => CLAUDE_SUBAGENT_TOOLS.contains(&tool),
        (Some("opencode"), Some(tool)) => tool == OPENCODE_SUBAGENT_TOOL,
        _ => false,
    }
}

/// Whether `agent`'s `tool` edits files.
pub fn edits_files(agent: Option<&str>, tool: Option<&str>) -> bool {
    match (agent, tool) {
        (Some("claude"), Some(tool)) => CLAUDE_EDIT_TOOLS.contains(&tool),
        (Some("opencode"), Some(tool)) => OPENCODE_EDIT_TOOLS.contains(&tool),
        (Some("pi"), Some(tool)) => PI_EDIT_TOOLS.contains(&tool),
        _ => false,
    }
}

/// Whether Impulse's answers to `agent` are the simple JSON its plugin or
/// extension reads (`{"context": …}`, `{"ask": …}`, `{"deny": …}`), not
/// Claude Code's hook output.
pub fn reads_plugin_replies(agent: Option<&str>) -> bool {
    matches!(agent, Some("opencode" | "pi"))
}

/// The Claude Code hook event an event of `agent`'s stands for, as far as
/// its subagents go: opencode's plugin reports the same moments under
/// opencode's names (a turn's end is its session's execution ending, a
/// subagent's start and stop its child session's).
pub fn subagent_event<'a>(agent: Option<&str>, event: &'a str) -> &'a str {
    if agent != Some("opencode") {
        return event;
    }
    match event {
        "start" => "SessionStart",
        "session.execution.started" => "UserPromptSubmit",
        "session.execution.succeeded"
        | "session.execution.interrupted"
        | "session.execution.failed" => "Stop",
        "tool.execute.before" => "PreToolUse",
        "tool.execute.after" => "PostToolUse",
        "subagent.start" => "SubagentStart",
        "subagent.stop" => "SubagentStop",
        other => other,
    }
}

/// Which answer `event` may get; `None` for events, and agents, that don't
/// read one.
pub fn reply(agent: Option<&str>, event: &str, tool: Option<&str>) -> Option<Reply> {
    match (agent?, event, tool) {
        ("claude", "SessionStart", _) => Some(Reply::SessionSummary),
        ("claude", "PostToolUse", Some(tool)) if CLAUDE_EDIT_TOOLS.contains(&tool) => {
            Some(Reply::EditedFile)
        }
        ("claude", "PostToolUse", Some("Bash")) => Some(Reply::RanShell),
        ("claude", "PreToolUse", Some("Bash")) => Some(Reply::BeforeShell),
        ("claude", "PreToolUse", Some(tool)) if CLAUDE_EDIT_TOOLS.contains(&tool) => {
            Some(Reply::BeforeEdit)
        }
        ("claude", "PreToolUse", Some(tool)) if CLAUDE_SUBAGENT_TOOLS.contains(&tool) => {
            Some(Reply::BeforeSubagent)
        }
        ("claude", "SubagentStart", _) => Some(Reply::SubagentStarted),
        ("opencode", "session.start", _) => Some(Reply::SessionSummary),
        ("opencode", "tool.execute.after", Some(tool)) if OPENCODE_EDIT_TOOLS.contains(&tool) => {
            Some(Reply::EditedFile)
        }
        ("opencode", "tool.execute.after", Some("shell")) => Some(Reply::RanShell),
        ("opencode", "tool.execute.before", Some("shell")) => Some(Reply::BeforeShell),
        ("opencode", "tool.execute.before", Some(OPENCODE_SUBAGENT_TOOL)) => {
            Some(Reply::BeforeSubagent)
        }
        ("opencode", "tool.execute.before", Some(tool)) if OPENCODE_EDIT_TOOLS.contains(&tool) => {
            Some(Reply::BeforeEdit)
        }
        ("opencode", "subagent.start", _) => Some(Reply::SubagentStarted),
        ("pi", "SessionStart", _) => Some(Reply::SessionSummary),
        ("pi", "ProjectTrust", _) => Some(Reply::ProjectTrust),
        ("pi", "PostToolUse", Some(tool)) if PI_EDIT_TOOLS.contains(&tool) => {
            Some(Reply::EditedFile)
        }
        ("pi", "PostToolUse", Some(PI_SHELL_TOOL)) => Some(Reply::RanShell),
        ("pi", "PreToolUse", Some(PI_SHELL_TOOL)) => Some(Reply::BeforeShell),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_events_stand_for_claude_codes() {
        let opencode = |event| subagent_event(Some("opencode"), event);
        assert_eq!(opencode("session.execution.succeeded"), "Stop");
        assert_eq!(opencode("tool.execute.before"), "PreToolUse");
        assert_eq!(opencode("subagent.stop"), "SubagentStop");
        assert_eq!(opencode("permission.asked"), "permission.asked");
        // Claude Code's are its own.
        assert_eq!(subagent_event(Some("claude"), "Stop"), "Stop");
        assert_eq!(
            subagent_event(Some("claude"), "subagent.stop"),
            "subagent.stop"
        );
        assert!(starts_subagent(Some("opencode"), Some("subagent")));
        assert!(!starts_subagent(Some("opencode"), Some("Agent")));
        assert!(starts_subagent(Some("claude"), Some("Task")));
        assert!(edits_files(Some("opencode"), Some("patch")));
        assert!(!edits_files(Some("claude"), Some("patch")));
        assert!(!edits_files(Some("codex"), Some("Edit")));
    }

    #[test]
    fn only_claude_code_and_opencode_get_answers() {
        assert_eq!(
            reply(Some("claude"), "SessionStart", None),
            Some(Reply::SessionSummary)
        );
        assert_eq!(
            reply(Some("claude"), "PostToolUse", Some("MultiEdit")),
            Some(Reply::EditedFile)
        );
        assert_eq!(
            reply(Some("claude"), "PreToolUse", Some("Bash")),
            Some(Reply::BeforeShell)
        );
        assert_eq!(
            reply(Some("claude"), "PreToolUse", Some("Write")),
            Some(Reply::BeforeEdit)
        );
        assert_eq!(reply(Some("claude"), "PostToolUse", Some("Read")), None);
        assert_eq!(
            reply(Some("claude"), "PreToolUse", Some("Agent")),
            Some(Reply::BeforeSubagent)
        );
        assert_eq!(
            reply(Some("claude"), "SubagentStart", None),
            Some(Reply::SubagentStarted)
        );
        assert_eq!(reply(Some("claude"), "PostToolUse", Some("Agent")), None);
        assert_eq!(hook(Some("claude"), "SubagentStop", None, None), None);
        assert_eq!(
            reply(Some("opencode"), "session.start", None),
            Some(Reply::SessionSummary)
        );
        assert_eq!(
            reply(Some("opencode"), "tool.execute.after", Some("patch")),
            Some(Reply::EditedFile)
        );
        assert_eq!(
            reply(Some("opencode"), "tool.execute.after", Some("shell")),
            Some(Reply::RanShell)
        );
        assert_eq!(
            reply(Some("opencode"), "tool.execute.before", Some("shell")),
            Some(Reply::BeforeShell)
        );
        assert_eq!(
            reply(Some("opencode"), "tool.execute.before", Some("edit")),
            Some(Reply::BeforeEdit)
        );
        assert_eq!(
            reply(Some("opencode"), "tool.execute.before", Some("subagent")),
            Some(Reply::BeforeSubagent)
        );
        assert_eq!(
            reply(Some("opencode"), "subagent.start", None),
            Some(Reply::SubagentStarted)
        );
        assert_eq!(
            reply(Some("opencode"), "tool.execute.before", Some("read")),
            None
        );
        assert_eq!(reply(Some("opencode"), "PreToolUse", Some("Bash")), None);
        assert_eq!(reply(Some("codex"), "agent-turn-complete", None), None);
        assert_eq!(reply(None, "SessionStart", None), None);
    }

    #[test]
    fn pi_events_are_claude_codes_where_they_mean_the_same() {
        let pi = |event, tool| hook(Some("pi"), event, tool, None);
        assert_eq!(pi("SessionStart", None), Some(AgentHook::SessionStarted));
        assert_eq!(
            pi("UserPromptSubmit", None),
            Some(AgentHook::PromptSubmitted)
        );
        assert_eq!(pi("Stop", None), Some(AgentHook::Stopped));
        assert_eq!(
            hook(Some("pi"), "StopFailure", None, Some("429 rate limited")),
            Some(AgentHook::Failed("429 rate limited".into()))
        );
        assert_eq!(
            hook(
                Some("pi"),
                "prompt.start",
                None,
                Some("Impulse asks before this merge")
            ),
            Some(AgentHook::Notification(
                "Pi needs your input: Impulse asks before this merge".into()
            ))
        );
        assert_eq!(
            pi("prompt.start", None),
            Some(AgentHook::Notification("Pi needs your input".into()))
        );
        assert_eq!(pi("prompt.end", None), Some(AgentHook::UsedTool(None)));
        assert_eq!(pi("ProjectTrust", None), None);
        // opencode's names mean nothing from Pi.
        assert_eq!(pi("session.execution.started", None), None);
        assert_eq!(hook(Some("claude"), "prompt.start", None, None), None);
        assert!(edits_files(Some("pi"), Some("write")));
        assert!(!edits_files(Some("pi"), Some("bash")));
        assert!(!starts_subagent(Some("pi"), Some("subagent")));
        assert!(reads_plugin_replies(Some("pi")));
        assert!(reads_plugin_replies(Some("opencode")));
        assert!(!reads_plugin_replies(Some("claude")));
    }

    #[test]
    fn pi_gets_answers_for_its_extensions_events() {
        let pi = |event, tool| reply(Some("pi"), event, tool);
        assert_eq!(pi("SessionStart", None), Some(Reply::SessionSummary));
        assert_eq!(pi("ProjectTrust", None), Some(Reply::ProjectTrust));
        assert_eq!(pi("PostToolUse", Some("edit")), Some(Reply::EditedFile));
        assert_eq!(pi("PostToolUse", Some("write")), Some(Reply::EditedFile));
        assert_eq!(pi("PostToolUse", Some("bash")), Some(Reply::RanShell));
        assert_eq!(pi("PreToolUse", Some("bash")), Some(Reply::BeforeShell));
        // No subagents in Pi, so no edit is stopped for one.
        assert_eq!(pi("PreToolUse", Some("edit")), None);
        assert_eq!(pi("PostToolUse", Some("read")), None);
        // Claude Code's tool names aren't Pi's.
        assert_eq!(pi("PreToolUse", Some("Bash")), None);
        assert_eq!(reply(Some("claude"), "ProjectTrust", None), None);
    }

    #[test]
    fn opencode_permissions_name_the_tool() {
        assert_eq!(
            hook(Some("opencode"), "permission.asked", None, None),
            Some(AgentHook::Notification(
                "opencode needs your permission to use a tool".into()
            ))
        );
        assert_eq!(
            hook(None, "Notification", None, None),
            Some(AgentHook::Notification(String::new()))
        );
    }
}
