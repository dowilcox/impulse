//! The tools `impulse mcp` offers agents. Each one is a control request the
//! `impulse` command-line tool can also send (`list_tasks` is `impulse
//! tasks`, `create_task` is `impulse tasks new`), so the app answers both
//! the same way; this file only names them, describes them to the agent and
//! turns their arguments into requests.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::{json, Map, Value};

use crate::args::absolute;
use crate::control::{ControlRequest, BRIEF_LIMIT};
use crate::json;

/// A tool's arguments, as the agent sent them.
pub type Arguments = Map<String, Value>;

type MakeRequest = fn(&Arguments, &McpContext) -> Result<ControlRequest, McpToolError>;

pub struct McpTool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    pub input_schema: Value,
    /// Changes nothing (MCP's `readOnlyHint`).
    pub read_only: bool,
    /// May remove or overwrite something (`destructiveHint`); every such
    /// tool asks you in Impulse first.
    pub destructive: bool,
    /// Calling it twice with the same arguments does no more than once.
    pub idempotent: bool,
    /// Waits for something (an agent to stop, you to answer a sheet), so
    /// the server reports progress while it does.
    pub waits: bool,
    make_request: MakeRequest,
}

/// Who is calling: the pane's token and the server's folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpContext {
    pub token: Option<String>,
    pub cwd: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpToolError(pub String);

impl McpTool {
    /// How `tools/list` describes it.
    pub fn listing(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema,
            // No outputSchema: with one, clients reject a result without
            // structured content, and some answers are only text (an older
            // app's, or "Done.").
            "annotations": {
                "title": self.title,
                "readOnlyHint": self.read_only,
                "destructiveHint": self.destructive,
                "idempotentHint": self.idempotent,
                "openWorldHint": false,
            },
        })
    }

    pub fn request(
        &self,
        arguments: &Arguments,
        context: &McpContext,
    ) -> Result<ControlRequest, McpToolError> {
        (self.make_request)(arguments, context)
    }
}

/// The longest `wait_for_task` waits by default and at most, in seconds.
/// The default stays under the MCP SDKs' 60-second request timeout, for
/// agents that don't let progress extend it.
pub const DEFAULT_WAIT: i64 = 50;
pub const MAX_WAIT: i64 = 600;

/// The tools, in the order `tools/list` gives them.
pub fn all() -> &'static [McpTool] {
    static TOOLS: OnceLock<Vec<McpTool>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        let mut tools = tasks();
        tools.extend(extras());
        tools.extend(context());
        tools.extend(task_actions());
        tools
    })
}

pub fn named(name: &str) -> Option<&'static McpTool> {
    all().iter().find(|tool| tool.name == name)
}

// MARK: Tasks

fn tasks() -> Vec<McpTool> {
    vec![
        Tool::new(
            "list_tasks",
            "List tasks",
            "The repository's workspaces: its main checkout and every task (a branch in its own worktree folder) \
             Impulse made, with each one's branch, base, how far ahead and behind its base it is, its agent and \
             that agent's state (working, needs input, finished, idle), how many files are uncommitted, and the files it \
             changes that your workspace changes too. Yours is marked isCaller. Check it before merging a branch \
             or making sweeping edits: another agent may be working on the same files.",
        )
        .read_only()
        .build(|_, context| Ok(request("tasks", context, &[], false))),
        Tool::new(
            "get_task",
            "Look at a task",
            "One workspace in detail: what list_tasks says about it, plus the files it changes against its base, \
             its commits ahead of its base, whether it's merged into its base (squash merges included), its slot \
             and port values, and the brief it was started with. Name it by its folder or branch, as list_tasks shows.",
        )
        .property("task", string("The task's folder name or branch."))
        .required(&["task"])
        .read_only()
        .build(|arguments, context| {
            let task = required("task", arguments)?;
            Ok(request("tasks", context, &[("show", task)], false))
        }),
        Tool::new(
            "wait_for_task",
            "Wait for a task's agent",
            "Wait until no agent is working (or waiting for input) in a task, so its branch has stopped moving \
             before you merge it. Answers when that happens or when timeout_seconds pass, saying which; call it \
             again to keep waiting.",
        )
        .property("task", string("The task's folder name or branch."))
        .property(
            "timeout_seconds",
            json!({
                "type": "integer", "minimum": 1, "maximum": MAX_WAIT,
                "description": format!("How long to wait at most (default {DEFAULT_WAIT})."),
            }),
        )
        .required(&["task"])
        .read_only()
        .waits()
        .build(|arguments, context| {
            let task = required("task", arguments)?;
            let timeout = json::int(arguments.get("timeout_seconds"))
                .unwrap_or(DEFAULT_WAIT)
                .clamp(1, MAX_WAIT);
            Ok(request(
                "tasks",
                context,
                &[("wait", task), ("timeout", timeout.to_string())],
                true,
            ))
        }),
    ]
}

// MARK: Showing you things, status, checkpoints

fn extras() -> Vec<McpTool> {
    vec![
        Tool::new(
            "open_file",
            "Open a file for the user",
            "Open a file in Impulse's editor, in your window, at a line if given: to show the user where \
             something is. It doesn't take focus from another app. The file must exist.",
        )
        .property("path", string("The file, absolute or relative to your working directory."))
        .property("line", json!({"type": "integer", "minimum": 1}))
        .property("column", json!({"type": "integer", "minimum": 1}))
        .required(&["path"])
        .idempotent()
        .build(|arguments, context| {
            let path = required("path", arguments)?;
            let mut values = vec![
                ("path", absolute(&path, &context.cwd)),
                ("background", "1".to_string()),
            ];
            if let Some(line) = json::int(arguments.get("line")).filter(|line| *line > 0) {
                values.push(("line", line.to_string()));
            }
            if let Some(column) = json::int(arguments.get("column")).filter(|column| *column > 0) {
                values.push(("column", column.to_string()));
            }
            Ok(request("open", context, &values, false))
        }),
        Tool::new(
            "open_review",
            "Open Review for the user",
            "Open Impulse's Review on changes in your repository, to hand your work to the user: last-turn \
             (what your last turn changed), uncommitted (default), staged or unstaged.",
        )
        .property(
            "scope",
            json!({
                "type": "string", "enum": ["last-turn", "uncommitted", "staged", "unstaged"],
                "description": "Which changes (default uncommitted).",
            }),
        )
        .idempotent()
        .build(|arguments, context| {
            let scope = json::string(arguments.get("scope")).unwrap_or("uncommitted");
            if !["last-turn", "uncommitted", "staged", "unstaged"].contains(&scope) {
                return Err(McpToolError(
                    "scope must be last-turn, uncommitted, staged or unstaged.".into(),
                ));
            }
            Ok(request(
                "review",
                context,
                &[("scope", scope.to_string()), ("background", "1".to_string())],
                false,
            ))
        }),
        Tool::new(
            "set_status",
            "Report your state",
            "Tell Impulse what you're doing, for your terminal's tab and the agent list: working, waiting (you \
             need the user), done or idle, with a short message. Only for agents without Impulse's hooks; when \
             hooks report your state, they win and this changes nothing.",
        )
        .property(
            "state",
            json!({"type": "string", "enum": ["working", "waiting", "done", "idle"]}),
        )
        .property("message", string("Shown with the state; empty clears it."))
        .required(&["state"])
        .idempotent()
        .build(|arguments, context| {
            let state = required("state", arguments)?;
            if !["working", "waiting", "done", "idle"].contains(&state.as_str()) {
                return Err(McpToolError(
                    "state must be working, waiting, done or idle.".into(),
                ));
            }
            Ok(request(
                "status",
                context,
                &[("state", state), ("message", optional("message", arguments))],
                false,
            ))
        }),
        Tool::new(
            "notify",
            "Notify the user",
            "Flag your terminal for the user's attention and send a desktop notification.",
        )
        .property("title", string("The notification's title."))
        .property("message", string("Its text."))
        .required(&["title"])
        .build(|arguments, context| {
            let title = required("title", arguments)?;
            Ok(request(
                "notify",
                context,
                &[("title", title), ("message", optional("message", arguments))],
                false,
            ))
        }),
        Tool::new(
            "create_checkpoint",
            "Checkpoint the repository",
            "Snapshot the repository (tracked and untracked files, without touching them or the index) under \
             refs/impulse/checkpoints/manual/, before a risky step. Answers with the ref; the user can review or \
             restore it from Impulse.",
        )
        .property("message", string("What the checkpoint is for."))
        .build(|arguments, context| {
            Ok(request(
                "checkpoint",
                context,
                &[("message", optional("message", arguments))],
                false,
            ))
        }),
        Tool::new(
            "list_turns",
            "List your turns",
            "The latest agent turns Impulse recorded in your terminal (10 unless you ask for more), oldest first, \
             and how many earlier ones it left out: when each started and ended, and its start and end checkpoint \
             refs, so `git diff <start> <end>` shows what that turn changed.",
        )
        .property(
            "limit",
            json!({"type": "integer", "minimum": 1, "maximum": 500, "description": "How many of the latest turns (default 10)."}),
        )
        .read_only()
        .build(|arguments, context| {
            let mut values = Vec::new();
            if let Some(limit) = json::int(arguments.get("limit")) {
                values.push(("limit", limit.clamp(1, 500).to_string()));
            }
            Ok(request("turns", context, &values, false))
        }),
    ]
}

// MARK: What Impulse knows about the workspace

fn context() -> Vec<McpTool> {
    vec![
        Tool::new(
            "list_review_comments",
            "List review comments",
            "The user's review comments on this repository's changes, written in Impulse's Review: each one's \
             file and lines, the code it was written on, its text, whether it's outdated (those lines changed \
             since), and whether it's been marked addressed. Use it when asked to address the review.",
        )
        .property(
            "path",
            string("Only this file's comments (absolute or relative to your working directory)."),
        )
        .property(
            "include_outdated",
            json!({"type": "boolean", "description": "Include outdated comments (default true)."}),
        )
        .read_only()
        .build(|arguments, context| {
            let mut values = Vec::new();
            if let Some(path) = json::string(arguments.get("path")).filter(|p| !p.is_empty()) {
                values.push(("path", absolute(path, &context.cwd)));
            }
            if json::bool(arguments.get("include_outdated")) == Some(false) {
                values.push(("current", "1".to_string()));
            }
            Ok(request("comments", context, &values, false))
        }),
        Tool::new(
            "resolve_review_comment",
            "Mark a review comment addressed",
            "Mark a review comment addressed, with a note saying what you did. The comment stays: the user \
             sees your note in Review and accepts it (which deletes the comment) or reopens it.",
        )
        .property("id", string("The comment's id, from list_review_comments."))
        .property("note", string("What you changed, or why you didn't."))
        .required(&["id", "note"])
        .idempotent()
        .build(|arguments, context| {
            let id = required("id", arguments)?;
            let note = required("note", arguments)?;
            Ok(request(
                "comments",
                context,
                &[("resolve", id), ("note", note)],
                false,
            ))
        }),
        Tool::new(
            "get_diagnostics",
            "Get language-server problems",
            "The problems Impulse's language servers report in your workspace (what its Problems panel shows): \
             path, line, column, severity, message, source and code, errors first, at most 200. It names the \
             files whose diagnostics are current; a file it doesn't name is unknown, not clean. It never starts \
             a server: in a folder the user hasn't trusted, none run, and the answer says so.",
        )
        .property(
            "path",
            string("Only this file (absolute or relative to your working directory)."),
        )
        .property(
            "severity",
            json!({
                "type": "string", "enum": ["error", "warning", "info", "hint"],
                "description": "This severity and worse (default: all).",
            }),
        )
        .read_only()
        .build(|arguments, context| {
            let mut values = Vec::new();
            if let Some(path) = json::string(arguments.get("path")).filter(|p| !p.is_empty()) {
                values.push(("path", absolute(path, &context.cwd)));
            }
            if let Some(severity) = json::string(arguments.get("severity")) {
                if !["error", "warning", "info", "hint"].contains(&severity) {
                    return Err(McpToolError(
                        "severity must be error, warning, info or hint.".into(),
                    ));
                }
                values.push(("severity", severity.to_string()));
            }
            Ok(request("problems", context, &values, false))
        }),
        Tool::new(
            "list_command_blocks",
            "List terminal commands",
            "The terminals in your workspace (other than yours, and yours too) with their recent commands: each \
             command's block id, text, folder, exit status, start time and duration, and whether it's still \
             running. No output: get it with get_block_output. Use it to see a dev server, test watcher or build \
             the user runs beside you.",
        )
        .read_only()
        .build(|_, context| Ok(request("blocks", context, &[], false))),
        Tool::new(
            "get_block_output",
            "Get a command's output",
            "One command's output, from list_command_blocks: the last 200 lines by default.",
        )
        .property("block", string("The block's id, from list_command_blocks."))
        .property(
            "lines",
            json!({"type": "integer", "minimum": 1, "maximum": 2000, "description": "Last lines to return (default 200)."}),
        )
        .required(&["block"])
        .read_only()
        .build(|arguments, context| {
            let block = required("block", arguments)?;
            let mut values = vec![("show", block)];
            if let Some(lines) = json::int(arguments.get("lines")) {
                values.push(("lines", lines.clamp(1, 2000).to_string()));
            }
            Ok(request("blocks", context, &values, false))
        }),
        Tool::new(
            "list_unsaved_files",
            "List unsaved files",
            "Files in your workspace open in Impulse's editor with edits the user hasn't saved, and whether each \
             also changed on disk. Don't rewrite one: the user would have to reconcile it with their edits. Ask \
             them to save it first.",
        )
        .read_only()
        .build(|_, context| Ok(request("unsaved", context, &[], false))),
        Tool::new(
            "get_project_config",
            "Get the project's scripts",
            "How this repository is set up and checked, from its Impulse project settings: the check and setup \
             scripts, its actions, the commands to run when dependency files change, the env file's name, and \
             for a task its slot and port values. Only from settings the user has trusted; before that, only \
             what Impulse detected (package manager, lock files, a Compose file).",
        )
        .read_only()
        .build(|_, context| Ok(request("project", context, &[], false))),
    ]
}

// MARK: Starting and ending tasks

fn task_actions() -> Vec<McpTool> {
    vec![
        Tool::new(
            "create_task",
            "Start a task",
            "Ask Impulse to start a task: a new branch in its own worktree folder beside the repository, opened \
             as a workspace, optionally with an agent started there on a brief you write. Use it to split work \
             into independent pieces that can run in parallel. Impulse shows the user its New Task sheet filled \
             in from this, and waits: the answer is the task they created (with any changes they made) or that \
             they declined. Setup scripts and copied files are the project's own, as for any new task.",
        )
        .property(
            "title",
            string("What the task is about; becomes its branch and folder name."),
        )
        .property(
            "base",
            string("The branch or commit to start from (default: what New Task would pick, usually origin/main)."),
        )
        .property(
            "agent",
            string("The agent to start there: claude, codex, gemini, aider, opencode, amp, copilot, cursor, goose, qwen or crush. Omit for a plain terminal."),
        )
        .property(
            "brief",
            string("The agent's first prompt, and the task's description (at most 16 KB)."),
        )
        .property(
            "move_changes",
            json!({"type": "boolean", "description": "Take the main checkout's uncommitted files into the task."}),
        )
        .required(&["title"])
        .waits()
        .build(|arguments, context| {
            let title = required("title", arguments)?;
            let mut values = vec![("new", title)];
            if let Some(base) = json::string(arguments.get("base")).filter(|b| !b.is_empty()) {
                values.push(("base", base.to_string()));
            }
            if let Some(agent) = json::string(arguments.get("agent")).filter(|a| !a.is_empty()) {
                values.push(("agent", agent.to_string()));
            }
            if let Some(brief) = json::string(arguments.get("brief")).filter(|b| !b.is_empty()) {
                if brief.len() > BRIEF_LIMIT {
                    return Err(McpToolError("The brief holds more than 16 KB.".into()));
                }
                values.push(("brief", brief.to_string()));
            }
            if json::bool(arguments.get("move_changes")) == Some(true) {
                values.push(("move", "1".to_string()));
            }
            Ok(request("tasks", context, &values, true))
        }),
        Tool::new(
            "finish_task",
            "Finish a task",
            "Open Impulse's Finish for a task and run its steps: everything committed, synced with its base, \
             the project's check passed, then landed (merged and pushed, or pushed for review) and cleaned up. \
             Landing (for a task already merged, the clean-up) always waits for the user. Answers with the step \
             it stopped at and why, so you can fix a failing check and call again.",
        )
        .property("task", string("The task's folder name or branch."))
        .required(&["task"])
        .destructive()
        .waits()
        .build(|arguments, context| {
            let task = required("task", arguments)?;
            Ok(request("tasks", context, &[("finish", task)], true))
        }),
        Tool::new(
            "archive_task",
            "Archive a task",
            "Archive a task: close its workspace, snapshot any uncommitted work and remove its folder, keeping \
             its branch. Impulse asks the user first, and offers Undo.",
        )
        .property("task", string("The task's folder name or branch."))
        .required(&["task"])
        .destructive()
        .waits()
        .build(|arguments, context| {
            let task = required("task", arguments)?;
            Ok(request("tasks", context, &[("archive", task)], true))
        }),
    ]
}

// MARK: Building blocks

struct Tool {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    properties: Map<String, Value>,
    required: Vec<&'static str>,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
    waits: bool,
}

impl Tool {
    fn new(name: &'static str, title: &'static str, description: &'static str) -> Self {
        Tool {
            name,
            title,
            description,
            properties: Map::new(),
            required: Vec::new(),
            read_only: false,
            destructive: false,
            idempotent: false,
            waits: false,
        }
    }

    fn property(mut self, name: &str, schema: Value) -> Self {
        self.properties.insert(name.to_string(), schema);
        self
    }

    fn required(mut self, names: &[&'static str]) -> Self {
        self.required = names.to_vec();
        self
    }

    fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    fn idempotent(mut self) -> Self {
        self.idempotent = true;
        self
    }

    fn waits(mut self) -> Self {
        self.waits = true;
        self
    }

    fn build(self, make_request: MakeRequest) -> McpTool {
        let mut schema = json!({
            "type": "object",
            "properties": Value::Object(self.properties),
            "additionalProperties": false,
        });
        if !self.required.is_empty() {
            schema["required"] = json!(self.required);
        }
        McpTool {
            name: self.name,
            title: self.title,
            description: self.description.to_string(),
            input_schema: schema,
            read_only: self.read_only,
            destructive: self.destructive,
            idempotent: self.idempotent || self.read_only,
            waits: self.waits,
            make_request,
        }
    }
}

fn string(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

/// A string argument that must be there and not blank, trimmed.
fn required(key: &str, arguments: &Arguments) -> Result<String, McpToolError> {
    json::string(arguments.get(key))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
        .ok_or_else(|| McpToolError(format!("{key} is required.")))
}

/// A string argument, empty when it's not there.
fn optional(key: &str, arguments: &Arguments) -> String {
    json::string(arguments.get(key))
        .unwrap_or_default()
        .to_string()
}

fn request(
    command: &str,
    context: &McpContext,
    values: &[(&str, String)],
    wait: bool,
) -> ControlRequest {
    let mut arguments: BTreeMap<String, String> = values
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect();
    // The app tells tool calls from commands typed in a shell by this.
    arguments.insert("via".into(), "mcp".into());
    ControlRequest {
        command: command.to_string(),
        token: context.token.clone(),
        cwd: Some(context.cwd.clone()),
        arguments,
        wait,
    }
}
