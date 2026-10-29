//! `impulse`'s command line as requests, and the lines on the socket (the
//! Swift `ControlProtocolTests`, plus the wire format byte for byte).

use std::collections::{BTreeMap, HashMap};

use impulse_protocol::args::{self, split_location, UsageError};
use impulse_protocol::command::{
    AgentStatus, Blocks, Command, Comments, NewTask, ReviewScope, Tasks,
};
use impulse_protocol::control::{self, BRIEF_LIMIT, TOKEN_KEY};
use impulse_protocol::{ControlRequest, ControlResponse};

fn env() -> HashMap<String, String> {
    HashMap::from([(TOKEN_KEY.to_string(), "TOKEN-1".to_string())])
}

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

fn run(args: &[&str], cwd: &str, stdin: Option<&str>) -> Result<ControlRequest, UsageError> {
    let mut input = || stdin.map(|s| s.as_bytes().to_vec());
    args::request(&words(args), &env(), cwd, &mut input)
}

fn parse(args: &[&str]) -> Option<ControlRequest> {
    run(args, "/repo/sub", None).ok()
}

fn parse_stdin(args: &[&str], stdin: &str) -> Option<ControlRequest> {
    run(args, "/repo/sub", Some(stdin)).ok()
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn arguments(args: &[&str]) -> Option<BTreeMap<String, String>> {
    parse(args).map(|r| r.arguments)
}

#[test]
fn open_and_edit_resolve_paths_and_positions() {
    let open = parse(&["open", "../src/a.swift:12:3"]).unwrap();
    assert_eq!(open.command, "open");
    assert_eq!(
        open.arguments,
        map(&[
            ("path", "/repo/src/a.swift"),
            ("line", "12"),
            ("column", "3")
        ])
    );
    assert_eq!(open.token.as_deref(), Some("TOKEN-1"));
    assert!(!open.wait);
    let edit = parse(&["edit", "~/notes.md"]).unwrap();
    assert_eq!(edit.command, "open");
    assert!(edit.wait);
    let home = std::env::var("HOME").unwrap();
    assert_eq!(
        edit.argument("path"),
        Some(format!("{}/notes.md", home.trim_end_matches('/')).as_str())
    );
}

#[test]
fn pane_commands() {
    assert_eq!(
        arguments(&["split", "down", "npm", "test"]),
        Some(map(&[("direction", "down"), ("command", "npm test")]))
    );
    assert_eq!(arguments(&["split"]), Some(map(&[("direction", "right")])));
    assert_eq!(
        arguments(&["tab", "htop"]),
        Some(map(&[("command", "htop")]))
    );
    assert_eq!(
        arguments(&["review", "last-turn"]),
        Some(map(&[("scope", "last-turn")]))
    );
    assert_eq!(parse(&["review", "everything"]), None);
    assert_eq!(
        arguments(&["status", "waiting", "need", "a", "key"]),
        Some(map(&[("state", "waiting"), ("message", "need a key")]))
    );
    assert_eq!(parse(&["status", "busy"]), None);
    assert_eq!(
        arguments(&["notify", "Build", "done"]),
        Some(map(&[("title", "Build"), ("message", "done")]))
    );
}

#[test]
fn claude_hooks_read_stdin() {
    let stop = parse_stdin(
        &["hook", "claude", "Stop"],
        r#"{"session_id":"s1","hook_event_name":"Stop"}"#,
    )
    .unwrap();
    assert_eq!(stop.argument("event"), Some("Stop"));
    assert_eq!(stop.argument("session"), Some("s1"));
    let notification = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"Notification","message":"Claude needs your permission to use Bash"}"#,
    )
    .unwrap();
    assert_eq!(notification.argument("event"), Some("Notification"));
    assert_eq!(
        notification.argument("message"),
        Some("Claude needs your permission to use Bash")
    );
    let failure = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"StopFailure","error":"rate_limit","last_assistant_message":"API Error: Rate limit reached"}"#,
    )
    .unwrap();
    assert_eq!(failure.argument("event"), Some("StopFailure"));
    assert_eq!(
        failure.argument("message"),
        Some("API Error: Rate limit reached")
    );
    let bare = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"StopFailure","error":"overloaded"}"#,
    )
    .unwrap();
    assert_eq!(bare.argument("message"), Some("overloaded"));
}

#[test]
fn tool_hooks_carry_the_tool_and_what_it_acts_on() {
    let bash = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git merge upgrade"},"cwd":"/repo"}"#,
    )
    .unwrap();
    assert_eq!(bash.argument("event"), Some("PreToolUse"));
    assert_eq!(bash.argument("tool"), Some("Bash"));
    assert_eq!(bash.argument("toolCommand"), Some("git merge upgrade"));
    assert_eq!(bash.argument("hookCwd"), Some("/repo"));
    let edit = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"PostToolUse","tool_name":"Edit","tool_input":{"file_path":"/repo/src/a.ts"}}"#,
    )
    .unwrap();
    assert_eq!(edit.argument("toolFile"), Some("/repo/src/a.ts"));
}

#[test]
fn subagent_hooks_say_which_subagent() {
    let call = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"PreToolUse","tool_name":"Agent","tool_input":{"description":"write c","prompt":"…","isolation":"worktree","subagent_type":"general-purpose"}}"#,
    )
    .unwrap();
    assert_eq!(call.argument("toolDescription"), Some("write c"));
    assert_eq!(call.argument("toolSubagentType"), Some("general-purpose"));
    assert_eq!(call.argument("toolIsolated"), Some("1"));
    let launched = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"PostToolUse","tool_name":"Agent","tool_input":{"description":"write b"},"tool_response":{"status":"async_launched","agentId":"ae2e9"}}"#,
    )
    .unwrap();
    assert_eq!(launched.argument("launched"), Some("ae2e9"));
    assert_eq!(launched.argument("toolIsolated"), None);
    assert_eq!(launched.argument("launchedAsync"), None);
    let in_background = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"PostToolUse","tool_name":"Agent","tool_input":{"description":"write b"},"tool_response":{"isAsync":true,"status":"async_launched","agentId":"ae2e9"}}"#,
    )
    .unwrap();
    assert_eq!(in_background.argument("launchedAsync"), Some("1"));
    // The turn Claude Code takes when subagents in the background finish
    // names them; one the user typed names none.
    let notification = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"<task-notification>\n<task-id>ae2e9</task-id>\n<status>completed</status>\n</task-notification>\n<task-notification>\n<task-id>a7cfe</task-id>\n</task-notification>"}"#,
    )
    .unwrap();
    assert_eq!(notification.argument("notified"), Some("ae2e9,a7cfe"));
    let typed = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"UserPromptSubmit","prompt":"what does <task-id>x</task-id> mean?"}"#,
    )
    .unwrap();
    assert_eq!(typed.argument("notified"), None);
    // opencode's subagent tool, as Impulse's plugin reports it, and a
    // background subagent's result reaching the agent.
    let opencode = parse_stdin(
        &["hook", "opencode", "tool.execute.after"],
        r#"{"tool_name":"subagent","tool_input":{"description":"write b","subagent_type":"general"},"tool_response":{"agentId":"ses_b","isAsync":true}}"#,
    )
    .unwrap();
    assert_eq!(opencode.argument("toolDescription"), Some("write b"));
    assert_eq!(opencode.argument("toolSubagentType"), Some("general"));
    assert_eq!(opencode.argument("launched"), Some("ses_b"));
    assert_eq!(opencode.argument("launchedAsync"), Some("1"));
    let reported = parse_stdin(
        &["hook", "opencode", "subagent.stop"],
        r#"{"agent_id":"ses_b","last_assistant_message":"wrote b","subagents_reported":["ses_b"]}"#,
    )
    .unwrap();
    assert_eq!(reported.argument("subagent"), Some("ses_b"));
    assert_eq!(reported.argument("notified"), Some("ses_b"));
    assert_eq!(reported.argument("message"), Some("wrote b"));
    // A shell command's description is no Agent call's.
    let bash = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls","description":"List files"}}"#,
    )
    .unwrap();
    assert_eq!(bash.argument("toolDescription"), None);
    let inside = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"PostToolUse","tool_name":"Write","agent_id":"ae2e9","agent_type":"general-purpose","tool_input":{"file_path":"/r/b.txt"}}"#,
    )
    .unwrap();
    assert_eq!(inside.argument("subagent"), Some("ae2e9"));
    assert_eq!(inside.argument("subagentType"), Some("general-purpose"));
    let stopped = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"SubagentStop","agent_id":"ae2e9","agent_type":"general-purpose","last_assistant_message":"Created b.txt"}"#,
    )
    .unwrap();
    assert_eq!(stopped.argument("message"), Some("Created b.txt"));
    let stop = parse_stdin(
        &["hook", "claude"],
        r#"{"hook_event_name":"Stop","background_tasks":[{"id":"a1","type":"subagent","status":"running","description":"write c","agent_type":"general-purpose"},{"id":"b2","type":"shell","status":"running","description":"npm run dev","command":"npm run dev"}]}"#,
    )
    .unwrap();
    let background: serde_json::Value =
        serde_json::from_str(stop.argument("background").unwrap()).unwrap();
    assert_eq!(
        background,
        serde_json::json!([{"id": "a1", "agentType": "general-purpose", "description": "write c"}])
    );
    let quiet = parse_stdin(&["hook", "claude"], r#"{"hook_event_name":"Stop"}"#).unwrap();
    assert_eq!(quiet.argument("background"), None);
}

#[test]
fn tasks_lists_and_waits() {
    assert_eq!(arguments(&["tasks"]), Some(map(&[])));
    assert_eq!(arguments(&["tasks", "--json"]), Some(map(&[("json", "1")])));
    let wait = parse(&["tasks", "wait", "interia-upgrade"]).unwrap();
    assert_eq!(wait.arguments, map(&[("wait", "interia-upgrade")]));
    assert!(wait.wait);
    assert_eq!(parse(&["tasks", "wait"]), None);
    assert_eq!(parse(&["tasks", "everything"]), None);
}

#[test]
fn tasks_show_new_finish_and_archive() {
    assert_eq!(
        arguments(&["tasks", "show", "fix-elevation"]),
        Some(map(&[("show", "fix-elevation")]))
    );
    assert_eq!(
        arguments(&["tasks", "show", "fix-elevation", "--json"]),
        Some(map(&[("show", "fix-elevation"), ("json", "1")]))
    );
    assert_eq!(parse(&["tasks", "show"]), None);
    assert!(parse(&["tasks", "finish", "x"]).unwrap().wait);
    assert_eq!(
        arguments(&["tasks", "archive", "x"]),
        Some(map(&[("archive", "x")]))
    );
    let new = parse(&[
        "tasks",
        "new",
        "Fix",
        "the",
        "units",
        "--from",
        "origin/main",
        "--agent",
        "claude",
        "--move-changes",
    ])
    .unwrap();
    assert_eq!(
        new.arguments,
        map(&[
            ("new", "Fix the units"),
            ("base", "origin/main"),
            ("agent", "claude"),
            ("move", "1")
        ])
    );
    assert!(new.wait);
    let piped = parse_stdin(&["tasks", "new", "Docs", "--brief", "-"], "Write the docs.").unwrap();
    assert_eq!(piped.argument("brief"), Some("Write the docs."));
    assert_eq!(
        parse(&["tasks", "new", "Docs", "--brief-file", "notes.md"])
            .unwrap()
            .argument("briefFile"),
        Some("/repo/sub/notes.md")
    );
    assert_eq!(
        parse(&["tasks", "new", "Docs", "--brief", "notes.md"]),
        None
    );
    assert_eq!(parse(&["tasks", "new", "--agent", "claude"]), None);
    assert_eq!(parse(&["tasks", "new", "x", "--force"]), None);
    assert_eq!(
        parse(&["tasks", "new", "--", "--odd", "title"])
            .unwrap()
            .argument("new"),
        Some("--odd title")
    );
    let large = "x".repeat(BRIEF_LIMIT + 1);
    assert_eq!(
        parse_stdin(&["tasks", "new", "x", "--brief", "-"], &large),
        None
    );
}

#[test]
fn workspace_commands() {
    assert_eq!(arguments(&["turns", "--json"]), Some(map(&[("json", "1")])));
    assert_eq!(parse(&["turns", "extra"]), None);
    assert_eq!(arguments(&["comments"]), Some(map(&[])));
    assert_eq!(
        arguments(&["comments", "../src/a.ts", "--json"]),
        Some(map(&[("path", "/repo/src/a.ts"), ("json", "1")]))
    );
    assert_eq!(
        arguments(&["comments", "resolve", "c1", "Fixed", "it"]),
        Some(map(&[("resolve", "c1"), ("note", "Fixed it")]))
    );
    assert_eq!(parse(&["comments", "resolve"]), None);
    assert_eq!(
        arguments(&["problems", "--errors"]),
        Some(map(&[("errors", "1")]))
    );
    assert_eq!(parse(&["problems", "a.ts", "b.ts"]), None);
    assert_eq!(
        arguments(&["blocks", "show", "ab12:7", "--lines", "20"]),
        Some(map(&[("show", "ab12:7"), ("lines", "20")]))
    );
    assert_eq!(
        parse(&["blocks", "show", "ab12:7", "--lines", "none"]),
        None
    );
    assert_eq!(parse(&["blocks", "everything"]), None);
    assert_eq!(
        arguments(&["turns", "--limit", "3", "--json"]),
        Some(map(&[("limit", "3"), ("json", "1")]))
    );
    assert_eq!(parse(&["turns", "--limit", "0"]), None);
    assert_eq!(
        parse(&["unsaved", "--limit", "3"]),
        None,
        "only turns takes a limit"
    );
    assert_eq!(parse(&["unsaved"]).unwrap().command, "unsaved");
    assert_eq!(
        arguments(&["project", "--json"]),
        Some(map(&[("json", "1")]))
    );
}

#[test]
fn responses_carry_structured_data() {
    let response = ControlResponse {
        ok: true,
        message: Some("text".into()),
        data: Some(br#"{"a":1}"#.to_vec()),
    };
    let line = control::encode_line(&response);
    let decoded: ControlResponse = control::decode(&line[..line.len() - 1]).unwrap();
    assert_eq!(decoded, response);
    // An older app's answer has no data.
    let old: ControlResponse = control::decode(br#"{"ok":true,"message":"hi"}"#).unwrap();
    assert_eq!(old.data, None);
}

#[test]
fn opencode_plugin_events_read_stdin() {
    let edit = parse_stdin(
        &["hook", "opencode", "tool.execute.after"],
        r#"{"session_id":"ses_1","cwd":"/repo","tool_name":"write","tool_input":{"file_path":"/repo/src/a.ts"}}"#,
    )
    .unwrap();
    assert_eq!(
        edit.arguments,
        map(&[
            ("agent", "opencode"),
            ("event", "tool.execute.after"),
            ("session", "ses_1"),
            ("hookCwd", "/repo"),
            ("tool", "write"),
            ("toolFile", "/repo/src/a.ts"),
        ])
    );
    let start = parse_stdin(&["hook", "opencode", "start"], r#"{"cwd":"/repo"}"#).unwrap();
    assert_eq!(start.argument("event"), Some("start"));
    assert_eq!(start.argument("session"), None);
    assert_eq!(
        parse_stdin(&["hook", "opencode"], "{}"),
        None,
        "the plugin always names its event"
    );
}

#[test]
fn codex_notify_passes_json_as_an_argument() {
    let turn = parse(&[
        "hook",
        "codex",
        r#"{"type":"agent-turn-complete","last-assistant-message":"All done"}"#,
    ]);
    assert_eq!(
        turn.map(|t| t.arguments),
        Some(map(&[
            ("agent", "codex"),
            ("event", "agent-turn-complete"),
            ("message", "All done")
        ]))
    );
}

#[test]
fn usage_errors() {
    assert_eq!(parse(&[]), None);
    assert_eq!(parse(&["open"]), None);
    assert_eq!(parse(&["frobnicate"]), None);
    let mut no_input = || None;
    let error =
        args::request(&words(&["--help"]), &HashMap::new(), "/", &mut no_input).unwrap_err();
    assert!(error.message.contains("impulse open"));
}

#[test]
fn unknown_options_are_usage_errors() {
    let error = |args: &[&str]| run(args, "/repo", None).err();
    // There's no `edit --wait`: it mustn't open a file named "--wait".
    assert!(error(&["edit", "--wait", "notes.md"])
        .unwrap()
        .message
        .starts_with("impulse edit: unknown option '--wait'"));
    assert!(!error(&["open", "notes.md", "-n"]).unwrap().is_help);
    assert!(error(&["tab", "--new"]).is_some());
    assert!(error(&["split", "down", "-x"]).is_some());
    assert!(error(&["notify", "-t", "Build"]).is_some());
    assert!(error(&["checkpoint", "--all"]).is_some());
    // Help after a command is a request for usage, not a mistake.
    assert!(error(&["edit", "--help"]).unwrap().is_help);
    assert!(error(&["review", "-h"]).unwrap().is_help);
    // `--` ends options; a command's own options pass through.
    assert_eq!(
        parse(&["open", "--", "-odd-name.txt"])
            .unwrap()
            .argument("path"),
        Some("/repo/sub/-odd-name.txt")
    );
    assert_eq!(
        arguments(&["tab", "ls", "-la"]),
        Some(map(&[("command", "ls -la")]))
    );
    assert_eq!(
        parse(&["split", "right", "npm", "test", "--", "--watch"])
            .unwrap()
            .argument("command"),
        Some("npm test -- --watch")
    );
    assert_eq!(
        parse(&["checkpoint", "--", "-v2 rework"])
            .unwrap()
            .argument("message"),
        Some("-v2 rework")
    );
}

#[test]
fn lines_round_trip() {
    let request = ControlRequest {
        command: "open".into(),
        token: Some("t".into()),
        cwd: Some("/x".into()),
        arguments: map(&[("path", "/x/y")]),
        wait: true,
    };
    let line = control::encode_line(&request);
    assert_eq!(line.last(), Some(&b'\n'));
    let decoded: ControlRequest = control::decode(&line[..line.len() - 1]).unwrap();
    assert_eq!(decoded, request);
    assert_eq!(
        split_location("a:b.swift"),
        ("a:b.swift".to_string(), None, None)
    );
}

// Beyond the Swift tests: the bytes themselves, and the typed view.

#[test]
fn lines_are_the_swift_apps_byte_for_byte() {
    let request = ControlRequest {
        command: "open".into(),
        token: Some("t".into()),
        cwd: Some("/x".into()),
        arguments: map(&[("path", "/x/y"), ("line", "3")]),
        wait: true,
    };
    assert_eq!(
        String::from_utf8(control::encode_line(&request)).unwrap(),
        "{\"arguments\":{\"line\":\"3\",\"path\":\"\\/x\\/y\"},\"command\":\"open\",\"cwd\":\"\\/x\",\"token\":\"t\",\"wait\":true}\n"
    );
    // Absent fields are left out, as Swift's encoder leaves out nil.
    assert_eq!(
        String::from_utf8(control::encode_line(&ControlRequest::new("ping"))).unwrap(),
        "{\"arguments\":{},\"command\":\"ping\",\"wait\":false}\n"
    );
    // Data is base64, as JSONEncoder writes it.
    let response = ControlResponse {
        ok: true,
        message: Some("a/b".into()),
        data: Some(br#"{"a":1}"#.to_vec()),
    };
    assert_eq!(
        String::from_utf8(control::encode_line(&response)).unwrap(),
        "{\"data\":\"eyJhIjoxfQ==\",\"message\":\"a\\/b\",\"ok\":true}\n"
    );
    // What the Swift app sends decodes, escaped slashes and all.
    let swift: ControlResponse =
        control::decode(br#"{"data":"eyJhIjoxfQ==","message":"a\/b","ok":false}"#).unwrap();
    assert_eq!(swift.message.as_deref(), Some("a/b"));
    assert_eq!(swift.data_json(), Some(serde_json::json!({"a": 1})));
    // A request without its arguments isn't one (Swift requires them too).
    assert!(control::decode::<ControlRequest>(br#"{"command":"ping"}"#).is_err());
    assert!(control::decode::<ControlResponse>(br#"{"ok":true,"data":"not base64!"}"#).is_err());
}

#[test]
fn requests_read_as_commands() {
    let command = |args: &[&str]| parse(args).unwrap().parsed();
    assert_eq!(
        command(&["open", "a.rs:3"]),
        Command::Open {
            path: Some("/repo/sub/a.rs".into()),
            line: Some(3),
            column: None,
            background: false
        }
    );
    assert_eq!(
        command(&["review", "staged"]),
        Command::Review {
            scope: ReviewScope::Staged,
            background: false
        }
    );
    assert_eq!(
        command(&["status", "waiting"]),
        Command::Status {
            state: AgentStatus::NeedsInput,
            message: None
        }
    );
    assert_eq!(command(&["tasks"]), Command::Tasks(Tasks::List));
    assert_eq!(
        command(&["tasks", "wait", "x"]),
        Command::Tasks(Tasks::Wait {
            task: "x".into(),
            timeout: None
        })
    );
    assert_eq!(
        command(&["tasks", "new", "Fix", "--agent", "claude"]),
        Command::Tasks(Tasks::New(NewTask {
            title: "Fix".into(),
            agent: Some("claude".into()),
            ..Default::default()
        }))
    );
    assert_eq!(
        command(&["comments", "resolve", "c1", "Done"]),
        Command::Comments(Comments::Resolve {
            id: "c1".into(),
            note: "Done".into()
        })
    );
    assert_eq!(
        command(&["blocks", "show", "ab:1", "--lines", "5"]),
        Command::Blocks(Blocks::Show {
            id: "ab:1".into(),
            lines: Some(5)
        })
    );
    assert_eq!(ControlRequest::new("mcp-tools").parsed(), Command::McpTools);
    assert_eq!(
        ControlRequest::new("frobnicate").parsed(),
        Command::Unknown("frobnicate".into())
    );
    assert!(parse(&["turns", "--json"]).unwrap().wants_json());
}
