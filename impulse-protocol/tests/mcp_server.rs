//! `impulse mcp` against a stand-in app (the Swift `MCPServerTests`).

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use impulse_protocol::control::{BRIEF_LIMIT, SOCKET_KEY, TOKEN_KEY};
use impulse_protocol::mcp::server::TOOLS_TTL;
use impulse_protocol::mcp::{tools, McpContext, McpServer, PROTOCOL_VERSIONS};
use impulse_protocol::{ControlChannel, ControlRequest, ControlResponse, Failure};
use serde_json::{json, Value};

type Answer = Arc<dyn Fn(&ControlRequest) -> Result<ControlResponse, Failure> + Send + Sync>;

/// Answers every request with `answer`, recording what it was sent; a
/// blocking one holds tool calls until it's cancelled.
struct FakeChannel {
    answer: Answer,
    log: Arc<Mutex<Vec<ControlRequest>>>,
    blocks: bool,
    released: (Mutex<bool>, Condvar),
}

impl ControlChannel for FakeChannel {
    fn send(&self, request: &ControlRequest) -> Result<ControlResponse, Failure> {
        self.log.lock().unwrap().push(request.clone());
        if self.blocks && request.command != "mcp-tools" {
            let (released, wake) = &self.released;
            let mut released = released.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
            return Err(Failure::NoAnswer);
        }
        (self.answer)(request)
    }

    fn cancel(&self) {
        let (released, wake) = &self.released;
        *released.lock().unwrap() = true;
        wake.notify_all();
    }
}

/// The messages the server wrote, parsed.
#[derive(Clone, Default)]
struct Output {
    lines: Arc<(Mutex<Vec<Value>>, Condvar)>,
}

impl Output {
    fn write(&self, data: &[u8]) {
        let (lines, arrived) = &*self.lines;
        lines
            .lock()
            .unwrap()
            .push(serde_json::from_slice(data).unwrap_or(Value::Null));
        arrived.notify_all();
    }

    /// At least `count` messages; fewer (after a generous wait) fails.
    fn wait_for(&self, count: usize) -> Vec<Value> {
        let (lines, arrived) = &*self.lines;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut current = lines.lock().unwrap();
        while current.len() < count {
            let now = Instant::now();
            assert!(
                now < deadline,
                "{} of {count} messages arrived",
                current.len()
            );
            current = arrived.wait_timeout(current, deadline - now).unwrap().0;
        }
        current.clone()
    }
}

struct Harness {
    server: McpServer,
    output: Output,
    log: Arc<Mutex<Vec<ControlRequest>>>,
    last: Arc<Mutex<Option<Arc<FakeChannel>>>>,
}

fn inside() -> HashMap<String, String> {
    HashMap::from([
        (SOCKET_KEY.to_string(), "/tmp/impulse.sock".to_string()),
        (TOKEN_KEY.to_string(), "PANE".to_string()),
    ])
}

fn fine() -> Answer {
    Arc::new(|_| Ok(ControlResponse::ok_message("fine")))
}

fn make_server(environment: HashMap<String, String>, blocks: bool, answer: Answer) -> Harness {
    let output = Output::default();
    let log = Arc::new(Mutex::new(Vec::new()));
    let last: Arc<Mutex<Option<Arc<FakeChannel>>>> = Arc::new(Mutex::new(None));
    let (channel_log, channel_last) = (Arc::clone(&log), Arc::clone(&last));
    let emitted = output.clone();
    let server = McpServer::new(
        &environment,
        "/repo/sub",
        "1.2.3",
        Arc::new(move || {
            let channel = Arc::new(FakeChannel {
                answer: Arc::clone(&answer),
                log: Arc::clone(&channel_log),
                blocks,
                released: (Mutex::new(false), Condvar::new()),
            });
            *channel_last.lock().unwrap() = Some(Arc::clone(&channel));
            channel
        }),
        Arc::new(move |line: &[u8]| emitted.write(line)),
    );
    Harness {
        server,
        output,
        log,
        last,
    }
}

impl Harness {
    fn send(&self, json: &str) {
        self.server.handle(json.as_bytes());
    }
}

fn by_id(lines: &[Value]) -> HashMap<i64, Value> {
    let mut map = HashMap::new();
    for line in lines {
        map.entry(line["id"].as_i64().unwrap_or(0))
            .or_insert_with(|| line.clone());
    }
    map
}

#[test]
fn initialize_negotiates_the_version() {
    let h = make_server(inside(), false, fine());
    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#);
    h.send(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#);
    let lines = h.output.wait_for(2);
    assert_eq!(lines[0]["id"], 1);
    assert_eq!(lines[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(lines[0]["result"]["serverInfo"]["version"], "1.2.3");
    assert!(lines[0]["result"]["capabilities"].get("tools").is_some());
    assert!(lines[0]["result"].get("resultType").is_none());
    assert_eq!(lines[1]["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
}

/// `_meta` naming the 2026-07-28 revision, as a stateless client sends.
const STATELESS: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;

#[test]
fn discover_describes_the_stateless_revision() {
    let h = make_server(inside(), false, fine());
    h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":"d","method":"server/discover","params":{{{STATELESS}}}}}"#
    ));
    let result = &h.output.wait_for(1)[0]["result"];
    assert_eq!(result["supportedVersions"], json!(["2026-07-28"]));
    assert_eq!(result["resultType"], "complete");
    assert!(result["capabilities"].get("tools").is_some());
    assert!(result["ttlMs"].is_i64());
    assert_eq!(result["cacheScope"], "private");
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "impulse"
    );
    assert!(result["instructions"].is_string());
    // Not initialize's fields.
    assert!(result.get("protocolVersion").is_none());
    assert!(result.get("serverInfo").is_none());
}

#[test]
fn stateless_requests_get_their_revisions_shape() {
    let h = make_server(inside(), false, fine());
    h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{{STATELESS}}}}}"#
    ));
    h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"list_tasks","arguments":{{}},{STATELESS}}}}}"#
    ));
    h.send(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2099-01-01","io.modelcontextprotocol/clientCapabilities":{}}}}"#);
    h.send(r#"{"jsonrpc":"2.0","id":4,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#);
    // A session's requests keep the older shape.
    h.send(r#"{"jsonrpc":"2.0","id":5,"method":"tools/list"}"#);
    let lines = by_id(&h.output.wait_for(5));
    assert_eq!(lines[&1]["result"]["resultType"], "complete");
    assert_eq!(lines[&1]["result"]["ttlMs"], TOOLS_TTL);
    assert_eq!(lines[&1]["result"]["cacheScope"], "private");
    assert_eq!(lines[&2]["result"]["resultType"], "complete");
    assert_eq!(lines[&2]["result"]["isError"], false);
    assert_eq!(
        lines[&2]["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
        "1.2.3"
    );
    assert_eq!(lines[&3]["error"]["code"], -32022);
    assert_eq!(lines[&3]["error"]["data"]["requested"], "2099-01-01");
    assert_eq!(
        lines[&3]["error"]["data"]["supported"],
        json!(["2026-07-28"])
    );
    assert_eq!(lines[&4]["error"]["code"], -32602);
    assert!(lines[&5]["result"].get("tools").is_some());
    assert!(lines[&5]["result"].get("resultType").is_none());
    assert!(lines[&5]["result"].get("ttlMs").is_none());
}

#[test]
fn no_output_schema() {
    // With one, clients reject a successful result without structured
    // content, and "Done." has none.
    for tool in tools::all() {
        assert!(
            tool.listing().get("outputSchema").is_none(),
            "{}",
            tool.name
        );
    }
    let result = McpServer::tool_result(&ControlResponse::ok());
    assert!(result.get("structuredContent").is_none());
    assert_eq!(result["isError"], false);
    assert_eq!(result["content"][0]["text"], "Done.");
}

#[test]
fn notifications_get_no_answer_and_unknown_methods_an_error() {
    let h = make_server(inside(), false, fine());
    h.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    h.send(r#"{"jsonrpc":"2.0","id":7,"method":"resources/list"}"#);
    h.send("not json");
    h.send(r#"[{"jsonrpc":"2.0","id":8,"method":"ping"}]"#);
    h.send(r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#);
    let lines = h.output.wait_for(4);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0]["error"]["code"], -32601);
    assert_eq!(lines[1]["error"]["code"], -32700);
    assert_eq!(lines[2]["error"]["code"], -32600);
    assert_eq!(lines[3]["result"], json!({}));
}

#[test]
fn tools_list_honors_the_apps_settings() {
    let h = make_server(
        inside(),
        false,
        Arc::new(|request| {
            if request.command != "mcp-tools" {
                return Ok(ControlResponse::ok());
            }
            Ok(ControlResponse {
                ok: true,
                message: None,
                data: Some(br#"{"disabled":["create_task"]}"#.to_vec()),
            })
        }),
    );
    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
    let lines = h.output.wait_for(1);
    let tools = lines[0]["result"]["tools"].as_array().expect("no tools");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"list_tasks"));
    assert!(names.contains(&"get_diagnostics"));
    assert!(!names.contains(&"create_task"));
    assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
    let asked = h.log.lock().unwrap()[0].clone();
    assert_eq!(asked.command, "mcp-tools");
    assert_eq!(asked.argument("via"), Some("mcp"));
}

#[test]
fn outside_impulse_there_are_no_tools() {
    let h = make_server(HashMap::new(), false, fine());
    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
    h.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_tasks"}}"#);
    h.send(r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{}}"#);
    let lines = by_id(&h.output.wait_for(3));
    assert_eq!(lines[&1]["result"]["tools"], json!([]));
    assert_eq!(lines[&2]["result"]["isError"], true);
    assert!(lines[&3]["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("isn't running in an Impulse terminal"));
    assert!(h.log.lock().unwrap().is_empty());
}

#[test]
fn a_call_becomes_a_control_request() {
    let h = make_server(
        inside(),
        false,
        Arc::new(|_| {
            Ok(ControlResponse {
                ok: true,
                message: Some("trailhead: 2 workspaces".into()),
                data: Some(br#"{"repository":"trailhead"}"#.to_vec()),
            })
        }),
    );
    h.send(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_task","arguments":{"task":"fix-elevation"}}}"#);
    let lines = h.output.wait_for(1);
    let result = &lines[0]["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(result["structuredContent"]["repository"], "trailhead");
    assert_eq!(result["content"][0]["text"], "trailhead: 2 workspaces");
    let request = h.log.lock().unwrap()[0].clone();
    assert_eq!(request.command, "tasks");
    assert_eq!(request.argument("show"), Some("fix-elevation"));
    assert_eq!(request.argument("via"), Some("mcp"));
    assert_eq!(request.token.as_deref(), Some("PANE"));
    assert_eq!(request.cwd.as_deref(), Some("/repo/sub"));
}

#[test]
fn arguments_are_checked() {
    let h = make_server(inside(), false, fine());
    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_task","arguments":{}}}"#);
    h.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"no_such_tool"}}"#);
    let brief = "x".repeat(BRIEF_LIMIT + 1);
    h.send(&format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"create_task","arguments":{{"title":"t","brief":"{brief}"}}}}}}"#
    ));
    let lines = by_id(&h.output.wait_for(3));
    assert_eq!(lines[&1]["result"]["isError"], true);
    assert_eq!(lines[&2]["error"]["code"], -32602);
    assert_eq!(lines[&3]["result"]["isError"], true);
    assert!(h.log.lock().unwrap().is_empty());
}

#[test]
fn tool_arguments_map_to_the_commands_arguments() {
    let context = McpContext {
        token: Some("T".into()),
        cwd: "/repo".into(),
    };
    let request = |name: &str, arguments: Value| -> Option<ControlRequest> {
        let arguments = arguments.as_object().unwrap().clone();
        tools::named(name)
            .unwrap()
            .request(&arguments, &context)
            .ok()
    };
    let open = request("open_file", json!({"path": "src/a.ts", "line": 12})).unwrap();
    assert_eq!(open.command, "open");
    assert_eq!(open.argument("path"), Some("/repo/src/a.ts"));
    assert_eq!(open.argument("line"), Some("12"));
    assert_eq!(open.argument("background"), Some("1"));
    let wait = request(
        "wait_for_task",
        json!({"task": "x", "timeout_seconds": 100_000}),
    )
    .unwrap();
    assert!(wait.wait);
    assert_eq!(
        wait.argument("timeout"),
        Some(tools::MAX_WAIT.to_string().as_str())
    );
    let create = request(
        "create_task",
        json!({"title": "Fix it", "agent": "claude", "brief": "Do it", "move_changes": true}),
    )
    .unwrap();
    assert_eq!(create.argument("new"), Some("Fix it"));
    assert_eq!(create.argument("agent"), Some("claude"));
    assert_eq!(create.argument("brief"), Some("Do it"));
    assert_eq!(create.argument("move"), Some("1"));
    assert!(create.wait);
    assert_eq!(request("set_status", json!({"state": "busy"})), None);
    assert_eq!(
        request(
            "get_block_output",
            json!({"block": "ab12:7", "lines": 5000})
        )
        .unwrap()
        .argument("lines"),
        Some("2000")
    );
    assert_eq!(
        request("list_review_comments", json!({"include_outdated": false}))
            .unwrap()
            .argument("current"),
        Some("1")
    );
    assert_eq!(request("resolve_review_comment", json!({"id": "c1"})), None);
    // Every tool has a schema that's an object.
    for tool in tools::all() {
        assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
    }
    let mut names: Vec<&str> = tools::all().iter().map(|t| t.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), tools::all().len());
}

#[test]
fn failures_are_tool_errors() {
    let h = make_server(inside(), false, Arc::new(|_| Err(Failure::Unreachable)));
    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tasks"}}"#);
    assert_eq!(h.output.wait_for(1)[0]["result"]["isError"], true);
    let refusing = make_server(
        inside(),
        false,
        Arc::new(|_| Ok(ControlResponse::failure("Not in a git repository."))),
    );
    refusing
        .send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_tasks"}}"#);
    assert_eq!(refusing.output.wait_for(1)[0]["result"]["isError"], true);
}

#[test]
fn finish_lets_a_tool_list_go_out() {
    let h = make_server(inside(), false, fine());
    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
    h.server.finish(Duration::from_secs(2));
    let lines = h.output.wait_for(1);
    assert_eq!(lines[0]["id"], 1);
}

#[test]
fn a_cancelled_call_hangs_up_and_gets_no_answer() {
    let h = make_server(inside(), true, fine());
    h.send(r#"{"jsonrpc":"2.0","id":"w","method":"tools/call","params":{"name":"wait_for_task","arguments":{"task":"x"}}}"#);
    // Wait for the call to reach the app.
    for _ in 0..100 {
        if !h.log.lock().unwrap().is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    h.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"w"}}"#);
    h.send(r#"{"jsonrpc":"2.0","id":"p","method":"ping"}"#);
    h.output.wait_for(1);
    thread::sleep(Duration::from_millis(100));
    let lines = h.output.wait_for(1);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["id"], "p");
    assert!(h.last.lock().unwrap().is_some());
}

// Beyond the Swift tests.

#[test]
fn a_waiting_call_reports_progress_until_answered() {
    let h = make_server(
        inside(),
        false,
        Arc::new(|_| {
            thread::sleep(Duration::from_millis(1300));
            Ok(ControlResponse::ok_message("stopped"))
        }),
    );
    h.server.set_progress_interval(Duration::from_millis(500));
    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"wait_for_task","arguments":{"task":"x"},"_meta":{"progressToken":"tok"}}}"#);
    // A tool that doesn't wait reports none, token or not.
    h.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_tasks","_meta":{"progressToken":"other"}}}"#);
    let deadline = Instant::now() + Duration::from_secs(20);
    let lines = loop {
        let lines = h.output.wait_for(1);
        if lines.iter().filter(|l| l.get("id").is_some()).count() == 2 || Instant::now() > deadline
        {
            break lines;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let progress: Vec<&Value> = lines
        .iter()
        .filter(|l| l["method"] == "notifications/progress")
        .collect();
    assert!(!progress.is_empty());
    assert!(progress
        .iter()
        .all(|p| p["params"]["progressToken"] == "tok"));
    assert!(progress[0]["params"]["message"]
        .as_str()
        .unwrap()
        .starts_with("Waiting ("));
    // None after the answer.
    let answered = lines.iter().position(|l| l["id"] == 1).unwrap();
    thread::sleep(Duration::from_millis(700));
    let after = h.output.wait_for(1);
    assert!(after[answered + 1..]
        .iter()
        .all(|l| l["method"] != "notifications/progress"));
}

#[test]
fn messages_are_written_as_sorted_lines() {
    let written = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = Arc::clone(&written);
    let server = McpServer::new(
        &inside(),
        "/r",
        "1",
        Arc::new(|| -> Arc<dyn ControlChannel> { unreachable!() }),
        Arc::new(move |line: &[u8]| sink.lock().unwrap().extend_from_slice(line)),
    );
    server.handle(br#"{"jsonrpc":"2.0","id":"a/b","method":"ping"}"#);
    assert_eq!(
        String::from_utf8(written.lock().unwrap().clone()).unwrap(),
        "{\"id\":\"a/b\",\"jsonrpc\":\"2.0\",\"result\":{}}\n"
    );
}
