//! What a request asks for, typed: the app's control server matches on
//! [`Command`] instead of reading `command` and `arguments` by hand. The
//! arguments are read as leniently as the Swift app reads them (a number
//! that doesn't parse is absent, an unknown review scope is `uncommitted`,
//! an unknown status is `idle`), and in the same order of precedence (a
//! `tasks` request with `wait` waits, whatever else it says).
//!
//! It's only a view: the request itself stays the wire format, and every
//! argument here is one `impulse` or `impulse mcp` sends.

use crate::agent_tools::Severity;
use crate::control::ControlRequest;
use crate::json;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// `impulse open`/`edit` (`edit` is `request.wait`), and `open_file`.
    Open {
        path: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
        /// An agent's `open_file`: don't take focus from another app.
        background: bool,
    },
    Review {
        scope: ReviewScope,
        background: bool,
    },
    Split {
        down: bool,
        command: Option<String>,
    },
    Tab {
        command: Option<String>,
    },
    Notify {
        title: Option<String>,
        message: String,
    },
    Status {
        state: AgentStatus,
        /// Each report replaces the message; `None` clears it.
        message: Option<String>,
    },
    Checkpoint {
        /// `None`: "manual checkpoint".
        message: Option<String>,
    },
    Hook(Box<Hook>),
    Tasks(Tasks),
    Turns {
        limit: Option<usize>,
    },
    Comments(Comments),
    Problems {
        path: Option<String>,
        /// This severity and worse (`--errors` is `Error`).
        minimum: Option<Severity>,
    },
    Blocks(Blocks),
    Unsaved,
    Project,
    /// `impulse mcp` asking which tools the settings turn off; answered
    /// with `{"disabled": [names]}` as data.
    McpTools,
    /// Anything else: answered "impulse: unknown command '…'".
    Unknown(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewScope {
    LastTurn,
    Uncommitted,
    Staged,
    Unstaged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Working,
    /// "waiting": the agent needs the user.
    NeedsInput,
    Done,
    Idle,
}

/// What an agent's hook reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hook {
    pub agent: Option<String>,
    pub event: String,
    pub message: Option<String>,
    pub session: Option<String>,
    /// Tool events: which tool, and the shell command or file it acts on.
    pub tool: Option<String>,
    pub tool_command: Option<String>,
    pub tool_file: Option<String>,
    /// The folder the hook says the agent is in.
    pub hook_cwd: Option<String>,
    /// Claude Code: the subagent the event comes from, and its type.
    pub subagent: Option<String>,
    pub subagent_type: Option<String>,
    /// An Agent call: what it asks for, the type it starts, whether the
    /// subagent gets a folder of its own, and (once it returns) its id.
    pub tool_description: Option<String>,
    pub tool_subagent_type: Option<String>,
    pub tool_isolated: bool,
    pub launched: Option<String>,
    /// A Stop: the subagents still running in the background, as JSON.
    pub background: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tasks {
    /// The repository's workspaces (`impulse tasks`, `list_tasks`).
    List,
    Show(String),
    Wait {
        task: String,
        /// Seconds; `None` waits as long as it takes.
        timeout: Option<f64>,
    },
    New(NewTask),
    Finish(String),
    Archive(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewTask {
    pub title: String,
    pub base: Option<String>,
    pub agent: Option<String>,
    pub brief: String,
    /// Take the main checkout's uncommitted files along.
    pub move_changes: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Comments {
    List {
        path: Option<String>,
        /// Leave out outdated comments.
        current_only: bool,
    },
    Resolve {
        id: String,
        note: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocks {
    List,
    Show { id: String, lines: Option<usize> },
}

impl ControlRequest {
    /// `--json`: answer with the JSON as the message.
    pub fn wants_json(&self) -> bool {
        self.arguments.contains_key("json")
    }

    /// An MCP tool call rather than a command typed in a shell.
    pub fn via_mcp(&self) -> bool {
        self.argument("via") == Some("mcp")
    }

    /// What it asks for.
    pub fn parsed(&self) -> Command {
        let arg = |key: &str| self.argument(key).map(String::from);
        let has = |key: &str| self.arguments.contains_key(key);
        let number = |key: &str| self.argument(key).and_then(json::parse_int);
        match self.command.as_str() {
            "open" => Command::Open {
                path: arg("path"),
                line: number("line").and_then(|n| u32::try_from(n).ok()),
                column: number("column").and_then(|n| u32::try_from(n).ok()),
                background: has("background"),
            },
            "review" => Command::Review {
                scope: match self.argument("scope") {
                    Some("last-turn") => ReviewScope::LastTurn,
                    Some("staged") => ReviewScope::Staged,
                    Some("unstaged") => ReviewScope::Unstaged,
                    _ => ReviewScope::Uncommitted,
                },
                background: has("background"),
            },
            "split" => Command::Split {
                down: self.argument("direction") == Some("down"),
                command: arg("command"),
            },
            "tab" => Command::Tab {
                command: arg("command"),
            },
            "notify" => Command::Notify {
                title: arg("title"),
                message: arg("message").unwrap_or_default(),
            },
            "status" => Command::Status {
                state: match self.argument("state") {
                    Some("working") => AgentStatus::Working,
                    Some("waiting") => AgentStatus::NeedsInput,
                    Some("done") => AgentStatus::Done,
                    _ => AgentStatus::Idle,
                },
                message: arg("message").filter(|m| !m.is_empty()),
            },
            "checkpoint" => Command::Checkpoint {
                message: arg("message").filter(|m| !m.is_empty()),
            },
            "hook" => Command::Hook(Box::new(Hook {
                agent: arg("agent"),
                event: arg("event").unwrap_or_default(),
                message: arg("message"),
                session: arg("session"),
                tool: arg("tool"),
                tool_command: arg("toolCommand"),
                tool_file: arg("toolFile"),
                hook_cwd: arg("hookCwd"),
                subagent: arg("subagent"),
                subagent_type: arg("subagentType"),
                tool_description: arg("toolDescription"),
                tool_subagent_type: arg("toolSubagentType"),
                tool_isolated: has("toolIsolated"),
                launched: arg("launched"),
                background: arg("background"),
            })),
            "tasks" => Command::Tasks(if let Some(task) = arg("wait") {
                Tasks::Wait {
                    task,
                    timeout: self.argument("timeout").and_then(|t| t.parse().ok()),
                }
            } else if let Some(task) = arg("show") {
                Tasks::Show(task)
            } else if let Some(title) = arg("new") {
                Tasks::New(NewTask {
                    title: title.trim().to_string(),
                    base: arg("base"),
                    agent: arg("agent")
                        .map(|a| a.trim().to_string())
                        .filter(|a| !a.is_empty()),
                    brief: arg("brief").unwrap_or_default(),
                    move_changes: has("move"),
                })
            } else if let Some(task) = arg("finish") {
                Tasks::Finish(task)
            } else if let Some(task) = arg("archive") {
                Tasks::Archive(task)
            } else {
                Tasks::List
            }),
            "turns" => Command::Turns {
                limit: number("limit").and_then(|n| usize::try_from(n).ok()),
            },
            "comments" => Command::Comments(match arg("resolve") {
                Some(id) => Comments::Resolve {
                    id,
                    note: arg("note").unwrap_or_default().trim().to_string(),
                },
                None => Comments::List {
                    path: arg("path"),
                    current_only: has("current"),
                },
            }),
            "problems" => Command::Problems {
                path: arg("path"),
                minimum: if has("errors") {
                    Some(Severity::Error)
                } else {
                    self.argument("severity").and_then(Severity::from_label)
                },
            },
            "blocks" => Command::Blocks(match arg("show") {
                Some(id) => Blocks::Show {
                    id,
                    lines: number("lines").and_then(|n| usize::try_from(n).ok()),
                },
                None => Blocks::List,
            }),
            "unsaved" => Command::Unsaved,
            "project" => Command::Project,
            "mcp-tools" => Command::McpTools,
            other => Command::Unknown(other.to_string()),
        }
    }
}
