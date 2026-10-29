//! `impulse`'s command line turned into a request: which words each command
//! takes, its options, and the usage text. Kept here, beside the protocol,
//! so the requests the tool makes are tested headless.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::control::{ControlRequest, BRIEF_LIMIT, TOKEN_KEY};
use crate::json;

pub const USAGE: &str = "impulse — talk to the Impulse window this terminal is in

  impulse open <file>[:line[:column]]   open in the editor
  impulse edit <file>                   open, and wait until its tab closes ($EDITOR)
  impulse review [last-turn|uncommitted|staged|unstaged]
  impulse split [right|down] [command…] split this pane (optionally running a command)
  impulse tab [command…]                new tab in this workspace
  impulse notify <title> [message…]     flag this pane and notify
  impulse status <working|waiting|done|idle> [message…]
                                        report an agent state from a program running
                                        in this pane (it lasts while that program runs)
  impulse checkpoint [message…]         snapshot the repository
  impulse tasks [--json]                this repository's workspaces: branches, agents,
                                        uncommitted files, files shared with yours
  impulse tasks wait <task>             wait until that task's agent stops working
  impulse tasks show <task> [--json]    one task: its files, commits, merged state, brief
  impulse tasks new <title…> [--from <base>] [--agent <id>] [--brief-file <path> | --brief -]
                    [--move-changes]    ask to start a task (Impulse shows New Task first)
  impulse tasks finish <task>           open Finish for a task (landing asks you)
  impulse tasks archive <task>          archive a task (asks you, with Undo)
  impulse turns [--limit <n>] [--json]  this pane's latest agent turns (10) and their checkpoints
  impulse comments [<file>] [--json]    the repository's review comments
  impulse comments resolve <id> <note…> mark a comment addressed, for you to accept
  impulse problems [<file>] [--errors] [--json]
                                        the language servers' problems in this workspace
  impulse blocks [--json]               this workspace's terminals and their commands
  impulse blocks show <id> [--lines <n>] one command's output
  impulse unsaved [--json]              files open with unsaved edits
  impulse project [--json]              the trusted project settings: scripts, actions
  impulse hook <claude|codex|opencode> [event]
                                        for agent hooks (reads hook input on stdin)
  impulse mcp                           an MCP server (stdio) for agents, with the
                                        same commands as tools

Outside an Impulse terminal, `impulse hook` does nothing and exits 0, and
`impulse mcp` offers no tools.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageError {
    pub message: String,
    /// `-h` / `--help`: the usage was asked for (not a mistake).
    pub is_help: bool,
}

impl UsageError {
    pub fn new(message: impl Into<String>) -> Self {
        UsageError {
            message: message.into(),
            is_help: false,
        }
    }

    fn help() -> Self {
        UsageError {
            message: USAGE.to_string(),
            is_help: true,
        }
    }
}

/// A word where an option could go: `-h`/`--help` asks for the usage, and
/// any other `-x` is an option `impulse` doesn't have (rather than, say, a
/// file named `--wait`). A lone `-` isn't an option.
fn option_error(word: Option<&str>, command: &str) -> Option<UsageError> {
    let word = word?;
    if !word.starts_with('-') || word == "-" {
        return None;
    }
    if word == "-h" || word == "--help" {
        return Some(UsageError::help());
    }
    Some(UsageError::new(format!(
        "impulse {command}: unknown option '{word}'\n\n{USAGE}"
    )))
}

/// A command's arguments without the `--` that ends options, or the first
/// option among them as an error. Only the first word is checked unless
/// `check_all` (the rest may be a command with options of its own).
fn operands(args: &[String], command: &str, check_all: bool) -> Result<Vec<String>, UsageError> {
    let Some(first) = args.first() else {
        return Ok(Vec::new());
    };
    if first == "--" {
        return Ok(args[1..].to_vec());
    }
    if !check_all {
        return match option_error(Some(first), command) {
            Some(error) => Err(error),
            None => Ok(args.to_vec()),
        };
    }
    let mut operands = Vec::new();
    let mut options_ended = false;
    for word in args {
        if !options_ended && word == "--" {
            options_ended = true;
            continue;
        }
        if !options_ended {
            if let Some(error) = option_error(Some(word), command) {
                return Err(error);
            }
        }
        operands.push(word.clone());
    }
    Ok(operands)
}

/// Turn command-line arguments into a request. `stdin` is read only by
/// commands that take input (agent hooks, a brief).
pub fn request(
    args: &[String],
    environment: &HashMap<String, String>,
    cwd: &str,
    stdin: &mut dyn FnMut() -> Option<Vec<u8>>,
) -> Result<ControlRequest, UsageError> {
    let Some(command) = args.first().cloned() else {
        return Err(UsageError::new(USAGE));
    };
    let mut rest: Vec<String> = args[1..].to_vec();
    let mut request = ControlRequest {
        command: command.clone(),
        token: environment.get(TOKEN_KEY).cloned(),
        cwd: Some(cwd.to_string()),
        ..Default::default()
    };
    let arguments = &mut request.arguments;

    if command != "hook" && matches!(rest.first().map(String::as_str), Some("-h" | "--help")) {
        return Err(UsageError::help());
    }
    // Commands whose arguments are free words: an option among them is a
    // mistake (`impulse edit --wait file` mustn't open a file named
    // `--wait`), and `--` lets a word start with a dash. (`split` checks
    // after its direction.)
    if ["open", "edit", "tab", "notify", "checkpoint"].contains(&command.as_str()) {
        rest = operands(&rest, &command, command == "open" || command == "edit")?;
    }

    match command.as_str() {
        "open" | "edit" => {
            let Some(target) = rest.first() else {
                return Err(UsageError::new(format!("usage: impulse {command} <file>")));
            };
            let (path, line, column) = split_location(target);
            request.command = "open".to_string();
            arguments.insert("path".into(), absolute(&path, cwd));
            if let Some(line) = line {
                arguments.insert("line".into(), line.to_string());
            }
            if let Some(column) = column {
                arguments.insert("column".into(), column.to_string());
            }
            request.wait = command == "edit";
        }

        "review" => {
            let scope = rest.first().map(String::as_str).unwrap_or("uncommitted");
            if !["last-turn", "uncommitted", "staged", "unstaged"].contains(&scope) {
                return Err(UsageError::new(
                    "usage: impulse review [last-turn|uncommitted|staged|unstaged]",
                ));
            }
            arguments.insert("scope".into(), scope.to_string());
        }

        "split" => {
            let mut words = rest;
            let mut direction = "right".to_string();
            if let Some(first) = words.first() {
                if first == "right" || first == "down" {
                    direction = words.remove(0);
                }
            }
            let words = operands(&words, &command, false)?;
            arguments.insert("direction".into(), direction);
            if !words.is_empty() {
                arguments.insert("command".into(), words.join(" "));
            }
        }

        "tab" => {
            if !rest.is_empty() {
                arguments.insert("command".into(), rest.join(" "));
            }
        }

        "notify" => {
            let Some(title) = rest.first() else {
                return Err(UsageError::new("usage: impulse notify <title> [message…]"));
            };
            arguments.insert("title".into(), title.clone());
            arguments.insert("message".into(), rest[1..].join(" "));
        }

        "status" => {
            let state = rest.first().map(String::as_str);
            let Some(state) = state.filter(|s| ["working", "waiting", "done", "idle"].contains(s))
            else {
                return Err(UsageError::new(
                    "usage: impulse status <working|waiting|done|idle> [message…]",
                ));
            };
            arguments.insert("state".into(), state.to_string());
            arguments.insert("message".into(), rest[1..].join(" "));
        }

        "checkpoint" => {
            arguments.insert("message".into(), rest.join(" "));
        }

        "tasks" => {
            let mut words = rest;
            if words.first().map(String::as_str) == Some("--json") {
                arguments.insert("json".into(), "1".into());
                words.remove(0);
            }
            match words.first().map(String::as_str) {
                None => {}
                Some(verb @ ("wait" | "show" | "finish" | "archive")) => {
                    let verb = verb.to_string();
                    if verb == "show" && words.last().map(String::as_str) == Some("--json") {
                        arguments.insert("json".into(), "1".into());
                        words.pop();
                    }
                    if words.len() != 2 || words[1].starts_with('-') {
                        return Err(UsageError::new(format!(
                            "usage: impulse tasks {verb} <task>"
                        )));
                    }
                    arguments.insert(verb.clone(), words[1].clone());
                    // Waiting, or until you answer Finish's or Archive's
                    // questions.
                    request.wait = verb != "show";
                }
                Some("new") => {
                    let values = new_task(&words[1..], cwd, stdin)?;
                    arguments.extend(values);
                    // Until you create the task or cancel.
                    request.wait = true;
                }
                Some(word) => {
                    return Err(UsageError::new(format!(
                        "impulse tasks: unknown argument '{word}'\n\n{USAGE}"
                    )));
                }
            }
        }

        "turns" | "unsaved" | "project" => {
            let mut words = rest;
            take_flag("--json", &mut words, arguments, "json");
            if command == "turns" {
                if let Some(index) = words.iter().position(|w| w == "--limit") {
                    let limit = words
                        .get(index + 1)
                        .and_then(|w| json::parse_int(w))
                        .filter(|limit| *limit > 0);
                    let Some(limit) = limit else {
                        return Err(UsageError::new(
                            "usage: impulse turns [--limit <n>] [--json]",
                        ));
                    };
                    arguments.insert("limit".into(), limit.to_string());
                    words.drain(index..=index + 1);
                }
            }
            if let Some(word) = words.first() {
                return Err(UsageError::new(format!(
                    "impulse {command}: unknown argument '{word}'\n\n{USAGE}"
                )));
            }
        }

        "comments" => {
            let mut words = rest;
            take_flag("--json", &mut words, arguments, "json");
            if words.first().map(String::as_str) == Some("resolve") {
                if words.len() < 2 || words[1].starts_with('-') {
                    return Err(UsageError::new(
                        "usage: impulse comments resolve <id> <note…>",
                    ));
                }
                arguments.insert("resolve".into(), words[1].clone());
                arguments.insert("note".into(), words[2..].join(" "));
            } else if let Some(word) = words.first() {
                if let Some(error) = option_error(Some(word), &command) {
                    return Err(error);
                }
                if words.len() != 1 {
                    return Err(UsageError::new("usage: impulse comments [<file>] [--json]"));
                }
                arguments.insert("path".into(), absolute(word, cwd));
            }
        }

        "problems" => {
            let mut words = rest;
            take_flag("--json", &mut words, arguments, "json");
            take_flag("--errors", &mut words, arguments, "errors");
            if let Some(word) = words.first() {
                if let Some(error) = option_error(Some(word), &command) {
                    return Err(error);
                }
                if words.len() != 1 {
                    return Err(UsageError::new(
                        "usage: impulse problems [<file>] [--errors] [--json]",
                    ));
                }
                arguments.insert("path".into(), absolute(word, cwd));
            }
        }

        "blocks" => {
            let mut words = rest;
            take_flag("--json", &mut words, arguments, "json");
            if words.first().map(String::as_str) == Some("show") {
                let usage = "usage: impulse blocks show <id> [--lines <n>]";
                let mut operands: Vec<String> = words[1..].to_vec();
                if let Some(index) = operands.iter().position(|w| w == "--lines") {
                    let lines = operands
                        .get(index + 1)
                        .and_then(|w| json::parse_int(w))
                        .filter(|lines| *lines > 0);
                    let Some(lines) = lines else {
                        return Err(UsageError::new(usage));
                    };
                    arguments.insert("lines".into(), lines.to_string());
                    operands.drain(index..=index + 1);
                }
                if operands.len() != 1 || operands[0].starts_with('-') {
                    return Err(UsageError::new(usage));
                }
                arguments.insert("show".into(), operands[0].clone());
            } else if let Some(word) = words.first() {
                return Err(UsageError::new(format!(
                    "impulse blocks: unknown argument '{word}'\n\n{USAGE}"
                )));
            }
        }

        "hook" => {
            let Some(agent) = rest.first().cloned() else {
                return Err(UsageError::new(
                    "usage: impulse hook <claude|codex|opencode> [event]",
                ));
            };
            arguments.insert("agent".into(), agent.clone());
            // An explicit event name, and/or a JSON payload: Codex passes
            // its notification as the last argument; Claude Code, and
            // Impulse's opencode plugin, write hook input to stdin.
            let mut event: Option<String> = None;
            let mut payload: Option<Vec<u8>> = None;
            for word in &rest[1..] {
                if word.starts_with('{') {
                    payload = Some(word.clone().into_bytes());
                } else if event.is_none() {
                    event = Some(word.clone());
                }
            }
            if payload.is_none() && (agent == "claude" || agent == "opencode") {
                payload = stdin();
            }
            let mut message: Option<String> = None;
            let object = payload
                .and_then(|payload| serde_json::from_slice::<serde_json::Value>(&payload).ok())
                .and_then(|value| match value {
                    serde_json::Value::Object(object) => Some(object),
                    _ => None,
                });
            if let Some(object) = object {
                let text = |key: &str| object.get(key).and_then(|v| v.as_str()).map(String::from);
                event = event
                    .or_else(|| text("hook_event_name"))
                    .or_else(|| text("type"));
                message = text("message").or_else(|| text("last-assistant-message"));
                if event.as_deref() == Some("StopFailure") {
                    // The API error that ended a Claude Code turn ("API
                    // Error: Rate limit reached"), or at least its kind
                    // ("rate_limit").
                    message = text("last_assistant_message").or_else(|| text("error"));
                }
                if event.as_deref() == Some("SubagentStop") {
                    // What the subagent reported.
                    message = text("last_assistant_message");
                }
                // Claude Code: the subagent an event comes from (absent for
                // the agent itself), and its type.
                if let Some(subagent) = text("agent_id") {
                    arguments.insert("subagent".into(), subagent);
                }
                if let Some(subagent_type) = text("agent_type") {
                    arguments.insert("subagentType".into(), subagent_type);
                }
                // The subagents a Stop leaves running in the background,
                // which wake the agent when they finish: `[{"id", "agentType",
                // "description"}]`. Absent when Claude Code doesn't say.
                if let Some(serde_json::Value::Array(tasks)) = object.get("background_tasks") {
                    let running: Vec<serde_json::Value> = tasks
                        .iter()
                        .filter(|task| {
                            matches!(
                                task.get("type").and_then(|t| t.as_str()),
                                Some("subagent" | "workflow")
                            )
                        })
                        .map(|task| {
                            let field = |key: &str| task.get(key).and_then(|v| v.as_str());
                            serde_json::json!({
                                "id": field("id").unwrap_or(""),
                                "agentType": field("agent_type").or(field("type")).unwrap_or(""),
                                "description": field("description").or(field("name")).unwrap_or(""),
                            })
                        })
                        .collect();
                    arguments.insert(
                        "background".into(),
                        serde_json::Value::Array(running).to_string(),
                    );
                }
                if let Some(session) = text("session_id").or_else(|| text("thread-id")) {
                    arguments.insert("session".into(), session);
                }
                // Tool events: which tool, on what (a shell command, a file).
                if let Some(tool) = text("tool_name") {
                    arguments.insert("tool".into(), tool);
                }
                let starts_subagent =
                    matches!(text("tool_name").as_deref(), Some("Agent" | "Task"));
                if let Some(serde_json::Value::Object(input)) = object.get("tool_input") {
                    let field =
                        |key: &str| input.get(key).and_then(|v| v.as_str()).map(String::from);
                    if let Some(command) = field("command") {
                        arguments.insert("toolCommand".into(), command);
                    }
                    if let Some(file) = field("file_path").or_else(|| field("notebook_path")) {
                        arguments.insert("toolFile".into(), file);
                    }
                    // An Agent call: what it asks for, the subagent's type,
                    // and whether it works in a folder of its own (its own
                    // worktree, or one the call names).
                    if starts_subagent {
                        if let Some(description) = field("description") {
                            arguments.insert("toolDescription".into(), description);
                        }
                        if let Some(subagent_type) = field("subagent_type") {
                            arguments.insert("toolSubagentType".into(), subagent_type);
                        }
                        if field("isolation").is_some() || field("cwd").is_some() {
                            arguments.insert("toolIsolated".into(), "1".into());
                        }
                    }
                }
                // The subagent an Agent call started.
                if starts_subagent {
                    if let Some(serde_json::Value::Object(response)) = object.get("tool_response") {
                        if let Some(id) = response.get("agentId").and_then(|v| v.as_str()) {
                            arguments.insert("launched".into(), id.to_string());
                        }
                        // In the background: Claude Code tells the agent
                        // when it finishes, in a turn of its own.
                        if response.get("isAsync").and_then(|v| v.as_bool()) == Some(true) {
                            arguments.insert("launchedAsync".into(), "1".into());
                        }
                    }
                }
                if let Some(hook_cwd) = text("cwd") {
                    arguments.insert("hookCwd".into(), hook_cwd);
                }
                // A turn Claude Code starts to tell the agent that subagents
                // in the background finished: their ids, comma-separated.
                if event.as_deref() == Some("UserPromptSubmit") {
                    let notified = text("prompt")
                        .map(|prompt| task_notifications(&prompt))
                        .unwrap_or_default();
                    if !notified.is_empty() {
                        arguments.insert("notified".into(), notified.join(","));
                    }
                }
            }
            let Some(event) = event.filter(|event| !event.is_empty()) else {
                return Err(UsageError::new(
                    "usage: impulse hook <claude|codex|opencode> <event>",
                ));
            };
            arguments.insert("event".into(), event);
            if let Some(message) = message {
                arguments.insert("message".into(), message);
            }
        }

        "-h" | "--help" | "help" => return Err(UsageError::new(USAGE)),

        _ => {
            return Err(UsageError::new(format!(
                "impulse: unknown command '{command}'\n\n{USAGE}"
            )));
        }
    }
    Ok(request)
}

/// Remove `flag` wherever it is among `words`, noting it as `key`.
fn take_flag(
    flag: &str,
    words: &mut Vec<String>,
    arguments: &mut std::collections::BTreeMap<String, String>,
    key: &str,
) {
    if !words.iter().any(|w| w == flag) {
        return;
    }
    words.retain(|w| w != flag);
    arguments.insert(key.into(), "1".into());
}

/// The task ids of the `<task-notification>`s in a prompt Claude Code
/// submits for itself: `<task-notification>\n<task-id>a1…</task-id>…`.
fn task_notifications(prompt: &str) -> Vec<String> {
    prompt
        .split("<task-notification>")
        .skip(1)
        .filter_map(|block| {
            let block = block.split("</task-notification>").next()?;
            let start = block.find("<task-id>")? + "<task-id>".len();
            let end = start + block[start..].find("</task-id>")?;
            Some(block[start..end].trim().to_string())
        })
        .filter(|id| !id.is_empty())
        .collect()
}

/// `impulse tasks new`'s words: the title, and its options anywhere among
/// them. `--brief -` reads the brief from standard input; `--brief-file` is
/// read by the command-line tool (`briefFile`).
fn new_task(
    words: &[String],
    cwd: &str,
    stdin: &mut dyn FnMut() -> Option<Vec<u8>>,
) -> Result<Vec<(String, String)>, UsageError> {
    let usage_line = "usage: impulse tasks new <title…> [--from <base>] [--agent <id>] [--brief-file <path> | --brief -] [--move-changes]";
    let mut arguments: std::collections::BTreeMap<String, String> = Default::default();
    let mut title: Vec<&str> = Vec::new();
    let mut words = words.iter();
    let mut options_ended = false;
    while let Some(word) = words.next() {
        if options_ended || !word.starts_with('-') || word == "-" {
            title.push(word);
            continue;
        }
        match word.as_str() {
            "--" => options_ended = true,
            "--move-changes" => {
                arguments.insert("move".into(), "1".into());
            }
            "--from" | "--agent" => {
                let Some(given) = words.next().filter(|given| !given.is_empty()) else {
                    return Err(UsageError::new(usage_line));
                };
                let key = if word == "--from" { "base" } else { "agent" };
                arguments.insert(key.into(), given.clone());
            }
            "--brief-file" => {
                let Some(path) = words.next() else {
                    return Err(UsageError::new(usage_line));
                };
                arguments.insert("briefFile".into(), absolute(path, cwd));
            }
            "--brief" => {
                if words.next().map(String::as_str) != Some("-") {
                    return Err(UsageError::new(
                        "impulse tasks new: --brief takes - (standard input); use --brief-file for a file",
                    ));
                }
                let data = stdin().unwrap_or_default();
                if data.len() > BRIEF_LIMIT {
                    return Err(UsageError::new(
                        "impulse tasks new: the brief holds more than 16 KB",
                    ));
                }
                arguments.insert("brief".into(), String::from_utf8_lossy(&data).into_owned());
            }
            _ => {
                if let Some(error) = option_error(Some(word), "tasks new") {
                    return Err(error);
                }
            }
        }
    }
    if arguments.contains_key("brief") && arguments.contains_key("briefFile") {
        return Err(UsageError::new(
            "impulse tasks new: give --brief or --brief-file, not both",
        ));
    }
    let joined = title.join(" ");
    let joined = joined.trim_matches(is_horizontal_space);
    if joined.is_empty() {
        return Err(UsageError::new(usage_line));
    }
    arguments.insert("new".into(), joined.to_string());
    Ok(arguments.into_iter().collect())
}

/// Foundation's `.whitespaces`: spaces and tabs, not line breaks.
fn is_horizontal_space(c: char) -> bool {
    c == '\t'
        || (c.is_whitespace()
            && !matches!(
                c,
                '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
            ))
}

/// `"src/a.swift:12:3"` → `("src/a.swift", 12, 3)`. A trailing non-number
/// isn't a position (`C:` style names stay intact).
pub fn split_location(target: &str) -> (String, Option<i64>, Option<i64>) {
    let mut parts: Vec<&str> = target.split(':').collect();
    let mut numbers: Vec<i64> = Vec::new();
    while parts.len() > 1 && numbers.len() < 2 {
        let Some(value) = parts.last().and_then(|last| json::parse_int(last)) else {
            break;
        };
        numbers.insert(0, value);
        parts.pop();
    }
    (
        parts.join(":"),
        numbers.first().copied(),
        numbers.get(1).copied(),
    )
}

/// `path` made absolute against `cwd`, `~` expanded and `.` and `..`
/// resolved, as Foundation's `standardizingPath` does it.
pub fn absolute(path: &str, cwd: &str) -> String {
    let expanded = expand_tilde(path);
    let joined = if expanded.starts_with('/') {
        expanded
    } else if cwd.ends_with('/') {
        format!("{cwd}{expanded}")
    } else {
        format!("{cwd}/{expanded}")
    };
    standardize(&joined)
}

fn expand_tilde(path: &str) -> String {
    let home = || {
        std::env::var("HOME")
            .ok()
            .filter(|home| !home.is_empty())
            .map(|home| home.trim_end_matches('/').to_string())
    };
    if path == "~" {
        if let Some(home) = home() {
            return home;
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home() {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

/// An absolute path with `.`, `..` and repeated or trailing slashes
/// removed. On the Mac, a leading `/private` goes when the path exists
/// without it (`/private/tmp/x` is `/tmp/x`), as Foundation has it.
fn standardize(path: &str) -> String {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(name) => parts.push(name),
            Component::ParentDir => {
                parts.pop();
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    let mut result = PathBuf::from("/");
    for part in parts {
        result.push(part);
    }
    let result = result.to_string_lossy().into_owned();
    if cfg!(target_os = "macos") {
        if let Some(rest) = result.strip_prefix("/private/") {
            let without = format!("/{rest}");
            if Path::new(&without).exists() {
                return without;
            }
        }
    }
    result
}
