//! The built `impulse` against a stand-in app on a real socket: what it
//! sends, what it prints and how it exits.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use impulse_protocol::control;
use impulse_protocol::{ControlRequest, ControlResponse};
use serde_json::{json, Value};

type Handler = Arc<dyn Fn(&ControlRequest) -> Option<ControlResponse> + Send + Sync>;

/// A socket that answers each connection's request with `handler` (no
/// answer: it hangs up), recording the requests.
struct FakeApp {
    dir: tempfile::TempDir,
    socket: String,
    requests: Arc<Mutex<Vec<ControlRequest>>>,
}

impl FakeApp {
    fn start(
        handler: impl Fn(&ControlRequest) -> Option<ControlResponse> + Send + Sync + 'static,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir
            .path()
            .join("impulse.sock")
            .to_string_lossy()
            .into_owned();
        let listener = UnixListener::bind(&socket).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let handler: Handler = Arc::new(handler);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let (recorded, handler) = (Arc::clone(&recorded), Arc::clone(&handler));
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = Vec::new();
                    if reader.read_until(b'\n', &mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let Ok(request) = control::decode::<ControlRequest>(line.trim_ascii_end())
                    else {
                        return;
                    };
                    recorded.lock().unwrap().push(request.clone());
                    if let Some(response) = handler(&request) {
                        let mut stream = stream;
                        let _ = stream.write_all(&control::encode_line(&response));
                    }
                });
            }
        });
        FakeApp {
            dir,
            socket,
            requests,
        }
    }

    fn requests(&self) -> Vec<ControlRequest> {
        self.requests.lock().unwrap().clone()
    }
}

/// `impulse` with `args`, in `cwd`, with only the given Impulse variables.
fn impulse(args: &[&str], cwd: &Path, socket: Option<&str>, stdin: &[u8]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_impulse"));
    command
        .args(args)
        .current_dir(cwd)
        .env_remove("IMPULSE_SOCKET")
        .env_remove("IMPULSE_PANE_TOKEN")
        .env_remove("IMPULSE_CLI")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(socket) = socket {
        command
            .env("IMPULSE_SOCKET", socket)
            .env("IMPULSE_PANE_TOKEN", "PANE-1");
    }
    let mut child = command.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(stdin).unwrap();
    drop(input);
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn open_sends_the_pane_and_an_absolute_path() {
    let app = FakeApp::start(|_| Some(ControlResponse::ok()));
    let cwd = app.dir.path().canonicalize().unwrap();
    let output = impulse(&["open", "src/a.rs:12:3"], &cwd, Some(&app.socket), b"");
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "");
    let request = &app.requests()[0];
    assert_eq!(request.command, "open");
    assert_eq!(request.token.as_deref(), Some("PANE-1"));
    assert_eq!(request.cwd.as_deref(), Some(cwd.to_str().unwrap()));
    let expected = cwd.join("src/a.rs");
    assert_eq!(request.argument("path"), expected.to_str());
    assert_eq!(request.argument("line"), Some("12"));
    assert_eq!(request.argument("column"), Some("3"));
    assert!(!request.wait);
}

#[test]
fn answers_print_to_stdout_and_refusals_to_stderr() {
    let app = FakeApp::start(|request| {
        Some(if request.command == "tasks" {
            ControlResponse::ok_message("trailhead: 2 workspaces")
        } else {
            ControlResponse::failure("Not in a git repository.")
        })
    });
    let cwd = app.dir.path();
    let listed = impulse(&["tasks"], cwd, Some(&app.socket), b"");
    assert_eq!(listed.status.code(), Some(0));
    assert_eq!(text(&listed.stdout), "trailhead: 2 workspaces\n");
    let refused = impulse(
        &["checkpoint", "before", "the", "move"],
        cwd,
        Some(&app.socket),
        b"",
    );
    assert_eq!(refused.status.code(), Some(1));
    assert_eq!(text(&refused.stdout), "");
    assert_eq!(text(&refused.stderr), "Not in a git repository.\n");
    assert_eq!(
        app.requests()[1].argument("message"),
        Some("before the move")
    );
}

#[test]
fn outside_impulse() {
    let dir = tempfile::tempdir().unwrap();
    let output = impulse(&["open", "a.txt"], dir.path(), None, b"");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        "impulse: not running inside an Impulse terminal (IMPULSE_SOCKET isn't set)\n"
    );
    // Hooks are silent and successful anywhere else.
    let hook = impulse(
        &["hook", "claude", "Stop"],
        dir.path(),
        None,
        br#"{"session_id":"s"}"#,
    );
    assert_eq!(hook.status.code(), Some(0));
    assert!(hook.stdout.is_empty() && hook.stderr.is_empty());
    // A socket nobody answers on.
    let gone = dir.path().join("gone.sock");
    let gone = gone.to_str().unwrap();
    let unreachable = impulse(&["tab"], dir.path(), Some(gone), b"");
    assert_eq!(unreachable.status.code(), Some(1));
    assert_eq!(
        text(&unreachable.stderr),
        "impulse: couldn't reach Impulse (is it still running?)\n"
    );
    let hook = impulse(
        &["hook", "codex", r#"{"type":"agent-turn-complete"}"#],
        dir.path(),
        Some(gone),
        b"",
    );
    assert_eq!(hook.status.code(), Some(0));
}

#[test]
fn no_answer_is_an_error() {
    let app = FakeApp::start(|_| None);
    let output = impulse(&["notify", "Build"], app.dir.path(), Some(&app.socket), b"");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stderr), "impulse: no answer from Impulse\n");
}

#[test]
fn usage() {
    let dir = tempfile::tempdir().unwrap();
    let mistake = impulse(&["frobnicate"], dir.path(), None, b"");
    assert_eq!(mistake.status.code(), Some(2));
    assert!(text(&mistake.stderr)
        .starts_with("impulse: unknown command 'frobnicate'\n\nimpulse — talk to"));
    let option = impulse(&["edit", "--wait", "x"], dir.path(), None, b"");
    assert_eq!(option.status.code(), Some(2));
    for asked in [&[][..], &["--help"][..], &["help"][..], &["open", "-h"][..]] {
        let help = impulse(asked, dir.path(), None, b"");
        assert_eq!(help.status.code(), Some(0), "{asked:?}");
        assert!(text(&help.stdout).starts_with("impulse — talk to the Impulse window"));
        assert!(text(&help.stdout).ends_with("`impulse mcp` offers no tools.\n"));
    }
    let mcp = impulse(&["mcp", "--help"], dir.path(), None, b"");
    assert_eq!(
        text(&mcp.stdout),
        "impulse mcp: an MCP server on standard input and output, for agents in Impulse terminals.\n"
    );
}

#[test]
fn a_pipe_opens_as_a_copy_and_cant_be_edited() {
    let app = FakeApp::start(|request| {
        let path = request.argument("path").unwrap_or_default();
        let contents = std::fs::read_to_string(path).unwrap_or_default();
        Some(ControlResponse::ok_message(contents))
    });
    let cwd = app.dir.path();
    let output = impulse(
        &["open", "/dev/stdin"],
        cwd,
        Some(&app.socket),
        b"from a pipe",
    );
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "from a pipe\n");
    let sent = app.requests()[0].argument("path").unwrap().to_string();
    assert!(
        sent.contains("impulse-open-") && sent.ends_with("/stdin"),
        "{sent}"
    );
    let _ = std::fs::remove_dir_all(Path::new(&sent).parent().unwrap());
    let edit = impulse(&["edit", "/dev/stdin"], cwd, Some(&app.socket), b"x");
    assert_eq!(edit.status.code(), Some(1));
    assert_eq!(
        text(&edit.stderr),
        "impulse edit: /dev/stdin can't be edited in place; impulse open shows a copy\n"
    );
    assert_eq!(app.requests().len(), 1);
}

#[test]
fn edit_waits_for_the_answer() {
    let app = FakeApp::start(|_| {
        thread::sleep(Duration::from_millis(400));
        Some(ControlResponse::ok())
    });
    let started = Instant::now();
    let output = impulse(
        &["edit", "notes.md"],
        app.dir.path(),
        Some(&app.socket),
        b"",
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(started.elapsed() >= Duration::from_millis(400));
    assert!(app.requests()[0].wait);
}

#[test]
fn a_brief_file_travels_in_the_request() {
    let app = FakeApp::start(|_| Some(ControlResponse::ok_message("Created fix-it.")));
    let cwd = app.dir.path();
    std::fs::write(cwd.join("brief.md"), "Fix it.\nCarefully.").unwrap();
    let output = impulse(
        &[
            "tasks",
            "new",
            "Fix",
            "it",
            "--brief-file",
            "brief.md",
            "--agent",
            "claude",
        ],
        cwd,
        Some(&app.socket),
        b"",
    );
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let request = &app.requests()[0];
    assert_eq!(request.argument("brief"), Some("Fix it.\nCarefully."));
    assert_eq!(request.argument("briefFile"), None);
    assert_eq!(request.argument("new"), Some("Fix it"));
    assert!(request.wait);
    let missing = impulse(
        &["tasks", "new", "x", "--brief-file", "nope.md"],
        cwd,
        Some(&app.socket),
        b"",
    );
    assert_eq!(missing.status.code(), Some(1));
    assert!(text(&missing.stderr).starts_with("impulse tasks new: couldn't read "));
}

#[test]
fn hooks_send_what_the_agent_wrote_on_stdin() {
    let app = FakeApp::start(|_| Some(ControlResponse::ok()));
    let output = impulse(
        &["hook", "claude"],
        app.dir.path(),
        Some(&app.socket),
        br#"{"hook_event_name":"PostToolUse","session_id":"s1","tool_name":"Edit","tool_input":{"file_path":"/r/a.ts"}}"#,
    );
    assert_eq!(output.status.code(), Some(0));
    let request = &app.requests()[0];
    assert_eq!(request.command, "hook");
    assert_eq!(request.argument("event"), Some("PostToolUse"));
    assert_eq!(request.argument("session"), Some("s1"));
    assert_eq!(request.argument("toolFile"), Some("/r/a.ts"));
}

/// `impulse mcp` fed `messages`; the lines it wrote once `answers` of them
/// are answers. Standard input stays open until then: closing it hangs up
/// the calls under way, as an agent quitting does.
fn mcp(messages: &[Value], answers: usize, socket: Option<&str>, cwd: &Path) -> Vec<Value> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_impulse"));
    command
        .arg("mcp")
        .current_dir(cwd)
        .env_remove("IMPULSE_SOCKET")
        .env_remove("IMPULSE_PANE_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(socket) = socket {
        command
            .env("IMPULSE_SOCKET", socket)
            .env("IMPULSE_PANE_TOKEN", "PANE-1");
    }
    let mut child = command.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    for message in messages {
        writeln!(input, "{message}").unwrap();
    }
    let mut lines = Vec::new();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    while lines
        .iter()
        .filter(|l: &&Value| l.get("id").is_some())
        .count()
        < answers
    {
        let mut line = String::new();
        if output.read_line(&mut line).unwrap() == 0 {
            break;
        }
        lines.push(serde_json::from_str(&line).unwrap());
    }
    drop(input);
    assert_eq!(child.wait().unwrap().code(), Some(0));
    lines
}

#[test]
fn mcp_outside_impulse_offers_no_tools() {
    let dir = tempfile::tempdir().unwrap();
    let lines = mcp(
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        ],
        2,
        None,
        dir.path(),
    );
    assert_eq!(lines.len(), 2);
    let initialized = lines.iter().find(|l| l["id"] == 1).unwrap();
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(initialized["result"]["serverInfo"]["name"], "impulse");
    let listed = lines.iter().find(|l| l["id"] == 2).unwrap();
    assert_eq!(listed["result"]["tools"], json!([]));
}

#[test]
fn mcp_calls_reach_the_app() {
    let app = FakeApp::start(|request| {
        Some(match request.command.as_str() {
            "mcp-tools" => ControlResponse {
                ok: true,
                message: None,
                data: Some(br#"{"disabled":["archive_task"]}"#.to_vec()),
            },
            _ => ControlResponse {
                ok: true,
                message: Some("trailhead: 1 workspace".into()),
                data: Some(br#"{"repository":"trailhead","workspaces":[]}"#.to_vec()),
            },
        })
    });
    let lines = mcp(
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "list_tasks", "arguments": {}}}),
        ],
        2,
        Some(&app.socket),
        app.dir.path(),
    );
    let listed = lines.iter().find(|l| l["id"] == 1).unwrap();
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"list_tasks") && !names.contains(&"archive_task"));
    let called = lines.iter().find(|l| l["id"] == 2).unwrap();
    assert_eq!(
        called["result"]["content"][0]["text"],
        "trailhead: 1 workspace"
    );
    assert_eq!(
        called["result"]["structuredContent"]["repository"],
        "trailhead"
    );
    let call = app
        .requests()
        .into_iter()
        .find(|r| r.command == "tasks")
        .unwrap();
    assert_eq!(call.argument("via"), Some("mcp"));
    assert_eq!(call.token.as_deref(), Some("PANE-1"));
}
