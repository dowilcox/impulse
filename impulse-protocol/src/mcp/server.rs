//! `impulse mcp`: a Model Context Protocol server on standard input and
//! output, one JSON-RPC message per line. Agents start it like any stdio
//! MCP server; each tool call becomes one control request to the Impulse
//! window the agent's terminal is in (see [`super::tools`]), so the app
//! decides and acts and this side keeps no state between calls.
//!
//! It answers both protocol eras. `initialize` starts a session in one of
//! the revisions up to 2025-11-25. In the stateless 2026-07-28 revision,
//! `server/discover` says what the server offers, and every request names
//! its revision in `_meta`: those get its result shape (`resultType`, cache
//! hints, the server's name in `_meta`). It offers tools only: no sampling
//! (Impulse never calls a model), resources, prompts or logging.
//!
//! Tool calls and tool lists wait on the app, so each runs on a thread of
//! its own and answers through `emit` when it's done; a call the agent
//! cancels hangs up its connection and gets no answer.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::tools::{self, McpContext, McpTool};
use crate::client::{ControlChannel, ControlClient, Failure};
use crate::control::{ControlRequest, ControlResponse, SOCKET_KEY, TOKEN_KEY};
use crate::json;

/// The revisions answered with `initialize`, newest first.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// The stateless revisions, named in each request's `_meta`.
pub const STATELESS_VERSIONS: [&str; 1] = ["2026-07-28"];

const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO_KEY: &str = "io.modelcontextprotocol/serverInfo";
/// How long a stateless client may keep the tool list (the settings can
/// turn tools off), in milliseconds.
pub const TOOLS_TTL: i64 = 60_000;
/// How long it may keep what `server/discover` says (fixed for the
/// process).
pub const DISCOVER_TTL: i64 = 3_600_000;

/// A new connection to the app, for each request.
pub type Connect = Arc<dyn Fn() -> Arc<dyn ControlChannel> + Send + Sync>;
/// Writes one message (a line, newline included) to the agent. Called from
/// several threads; it must write each line whole.
pub type Emit = Arc<dyn Fn(&[u8]) + Send + Sync>;

#[derive(Clone)]
pub struct McpServer {
    inner: Arc<Inner>,
}

struct Inner {
    context: McpContext,
    /// Whether the agent runs in an Impulse terminal (IMPULSE_SOCKET set).
    inside_impulse: bool,
    version: String,
    connect: Connect,
    emit: Emit,
    calls: Mutex<Calls>,
    /// Work under way in the background, for `finish`.
    pending: Mutex<usize>,
    pending_done: Condvar,
    progress_interval: Mutex<Duration>,
}

#[derive(Default)]
struct Calls {
    /// Tool calls under way, by request id (as JSON), for cancellation.
    in_flight: HashMap<String, Arc<dyn ControlChannel>>,
    cancelled: HashSet<String>,
}

/// Which revision a request is answered in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
    /// `initialize`'s session (2025-11-25 and before).
    Session,
    /// 2026-07-28: the request named it in `_meta`.
    Stateless,
}

impl McpServer {
    /// - `environment`: the server's environment (`IMPULSE_SOCKET`,
    ///   `IMPULSE_PANE_TOKEN`).
    /// - `connect`: a new connection to the app, for each request.
    /// - `emit`: writes one message to the agent.
    pub fn new(
        environment: &HashMap<String, String>,
        cwd: &str,
        version: &str,
        connect: Connect,
        emit: Emit,
    ) -> Self {
        let socket = environment
            .get(SOCKET_KEY)
            .map(String::as_str)
            .unwrap_or("");
        McpServer {
            inner: Arc::new(Inner {
                context: McpContext {
                    token: environment
                        .get(TOKEN_KEY)
                        .filter(|t| !t.is_empty())
                        .cloned(),
                    cwd: cwd.to_string(),
                },
                inside_impulse: !socket.is_empty(),
                version: version.to_string(),
                connect,
                emit,
                calls: Mutex::new(Calls::default()),
                pending: Mutex::new(0),
                pending_done: Condvar::new(),
                progress_interval: Mutex::new(Duration::from_secs(15)),
            }),
        }
    }

    /// The server for this process: the app's socket from the environment.
    pub fn standard(
        environment: &HashMap<String, String>,
        cwd: &str,
        version: &str,
        emit: Emit,
    ) -> Self {
        let socket = environment.get(SOCKET_KEY).cloned().unwrap_or_default();
        let connect: Connect = Arc::new(move || Arc::new(ControlClient::new(socket.clone())));
        McpServer::new(environment, cwd, version, connect, emit)
    }

    /// Time between progress notifications while a tool waits (15 s).
    pub fn set_progress_interval(&self, interval: Duration) {
        *self.inner.progress_interval.lock().unwrap() = interval;
    }

    /// Handle one line from the agent. Answers go out through `emit`, tool
    /// calls once they finish (on another thread).
    pub fn handle(&self, line: &[u8]) {
        let trimmed: Vec<u8> = line
            .iter()
            .copied()
            .filter(|b| *b != b'\n' && *b != b'\r')
            .collect();
        if trimmed.is_empty() {
            return;
        }
        let inner = &self.inner;
        let Ok(message) = serde_json::from_slice::<Value>(&trimmed) else {
            return inner.send_error(-32700, "Parse error", &Value::Null, None);
        };
        let Value::Object(object) = message else {
            // Batches were dropped from the protocol in 2025-06-18.
            return inner.send_error(
                -32600,
                "Invalid request: send one message per line, not a batch.",
                &Value::Null,
                None,
            );
        };
        let id = object.get("id");
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            // A response to a request of ours (there are none), or nonsense.
            if id.is_none() || object.contains_key("result") || object.contains_key("error") {
                return;
            }
            return inner.send_error(-32600, "Invalid request", id.unwrap_or(&Value::Null), None);
        };
        let empty = Map::new();
        let params = object
            .get("params")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        // A notification: nothing to answer.
        let Some(id) = id.filter(|id| !id.is_null()).cloned() else {
            if method == "notifications/cancelled" {
                if let Some(target) = params.get("requestId") {
                    inner.cancel(target);
                }
            }
            return;
        };
        // A stateless request names its revision; one that doesn't is in
        // the session `initialize` started.
        let meta = params.get("_meta").and_then(Value::as_object);
        let era = match meta
            .and_then(|meta| meta.get(VERSION_KEY))
            .and_then(Value::as_str)
        {
            Some(requested) => {
                if !STATELESS_VERSIONS.contains(&requested) {
                    return inner.send_error(
                        -32022,
                        &format!("Unsupported protocol version: {requested}"),
                        &id,
                        Some(json!({"supported": STATELESS_VERSIONS, "requested": requested})),
                    );
                }
                if !meta
                    .and_then(|meta| meta.get(CAPABILITIES_KEY))
                    .is_some_and(Value::is_object)
                {
                    return inner.send_error(
                        -32602,
                        &format!("Invalid params: _meta needs {CAPABILITIES_KEY}."),
                        &id,
                        None,
                    );
                }
                Era::Stateless
            }
            None => Era::Session,
        };
        match method {
            "initialize" => {
                let requested = params.get("protocolVersion").and_then(Value::as_str);
                let version = requested
                    .filter(|v| PROTOCOL_VERSIONS.contains(v))
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                let mut result = inner.description();
                result.insert("serverInfo".into(), inner.server_info());
                result.insert("protocolVersion".into(), json!(version));
                inner.send_result(Value::Object(result), &id, Era::Session);
            }
            "server/discover" => {
                let mut result = inner.description();
                result.insert("supportedVersions".into(), json!(STATELESS_VERSIONS));
                result.insert("ttlMs".into(), json!(DISCOVER_TTL));
                result.insert("cacheScope".into(), json!("private"));
                inner.send_result(Value::Object(result), &id, Era::Stateless);
            }
            "ping" => inner.send_result(json!({}), &id, era),
            "tools/list" => {
                let worker = Arc::clone(inner);
                Inner::in_background(inner, move || {
                    let tools: Vec<Value> =
                        worker.enabled_tools().iter().map(|t| t.listing()).collect();
                    let mut result = Map::new();
                    result.insert("tools".into(), Value::Array(tools));
                    if era == Era::Stateless {
                        result.insert("ttlMs".into(), json!(TOOLS_TTL));
                        result.insert("cacheScope".into(), json!("private"));
                    }
                    worker.send_result(Value::Object(result), &id, era);
                });
            }
            "tools/call" => Inner::call(inner, params, id, era),
            _ => inner.send_error(-32601, &format!("Method not found: {method}"), &id, None),
        }
    }

    /// Hang up every call under way (the agent closed standard input).
    pub fn cancel_all(&self) {
        let channels: Vec<Arc<dyn ControlChannel>> = {
            let mut calls = self.inner.calls.lock().unwrap();
            let keys: Vec<String> = calls.in_flight.keys().cloned().collect();
            calls.cancelled.extend(keys);
            calls.in_flight.values().cloned().collect()
        };
        for channel in channels {
            channel.cancel();
        }
    }

    /// The agent closed standard input: hang up the calls under way (they
    /// get no answer), and give what's left (a tool list being made) up to
    /// `timeout` to go out.
    pub fn finish(&self, timeout: Duration) {
        self.cancel_all();
        let deadline = Instant::now() + timeout;
        let mut pending = self.inner.pending.lock().unwrap();
        while *pending > 0 {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            pending = self
                .inner
                .pending_done
                .wait_timeout(pending, deadline - now)
                .unwrap()
                .0;
        }
    }

    /// A control response as a tool result: its text, and its JSON as
    /// structured content.
    pub fn tool_result(response: &ControlResponse) -> Value {
        let mut text = response.message.clone().unwrap_or_default();
        if text.is_empty() {
            text = if response.ok {
                "Done.".to_string()
            } else {
                "Impulse couldn't do that.".to_string()
            };
        }
        let mut result = Self::text_result(&text, !response.ok);
        if let Some(Value::Object(structured)) = response.data_json() {
            result["structuredContent"] = Value::Object(structured);
        }
        result
    }

    pub fn text_result(text: &str, is_error: bool) -> Value {
        json!({"content": [{"type": "text", "text": text}], "isError": is_error})
    }
}

impl Inner {
    fn server_info(&self) -> Value {
        json!({"name": "impulse", "title": "Impulse", "version": self.version})
    }

    /// What `initialize` and `server/discover` share.
    fn description(&self) -> Map<String, Value> {
        let mut description = Map::new();
        description.insert(
            "capabilities".into(),
            json!({"tools": {"listChanged": false}}),
        );
        description.insert("instructions".into(), json!(self.instructions()));
        description
    }

    fn instructions(&self) -> &'static str {
        if !self.inside_impulse {
            return "This agent isn't running in an Impulse terminal, so Impulse has no tools for it. Start the agent in an Impulse terminal to use them.";
        }
        "Impulse is the terminal IDE you're running in. Its tools describe the repository you're in: its tasks \
         (branches in their own folders, often with another agent working in them), the user's review comments, \
         the language servers' problems, the commands and output in the user's other terminals, files with \
         unsaved edits, and the project's scripts. Check list_tasks before merging a branch or making sweeping \
         edits. create_task, finish_task and archive_task ask the user in Impulse before anything happens."
    }

    /// The tools to offer: none outside Impulse; otherwise all but the ones
    /// the user's settings turn off (an older app doesn't say: all).
    fn enabled_tools(&self) -> Vec<&'static McpTool> {
        if !self.inside_impulse {
            return Vec::new();
        }
        let mut request = ControlRequest::new("mcp-tools");
        request.token = self.context.token.clone();
        request.cwd = Some(self.context.cwd.clone());
        request.arguments.insert("via".into(), "mcp".into());
        let disabled: Option<HashSet<String>> = match (self.connect)().send(&request) {
            Ok(response) if response.ok => response
                .data_json()
                .and_then(|data| data.get("disabled").and_then(Value::as_array).cloned())
                .map(|names| {
                    names
                        .iter()
                        .filter_map(|name| name.as_str().map(String::from))
                        .collect()
                }),
            _ => None,
        };
        let off = disabled.unwrap_or_default();
        tools::all()
            .iter()
            .filter(|tool| !off.contains(tool.name))
            .collect()
    }

    fn call(inner: &Arc<Inner>, params: &Map<String, Value>, id: Value, era: Era) {
        let name = params.get("name").and_then(Value::as_str);
        let Some(tool) = name.and_then(tools::named) else {
            return inner.send_error(
                -32602,
                &format!("Unknown tool: {}", name.unwrap_or("(none)")),
                &id,
                None,
            );
        };
        if !inner.inside_impulse {
            return inner.send_result(
                McpServer::text_result(
                    "Not inside an Impulse terminal: Impulse's tools only work there.",
                    true,
                ),
                &id,
                era,
            );
        }
        let empty = Map::new();
        let arguments = params
            .get("arguments")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let request = match tool.request(arguments, &inner.context) {
            Ok(request) => request,
            Err(error) => {
                return inner.send_result(McpServer::text_result(&error.0, true), &id, era)
            }
        };
        let key = Self::key(&id);
        let channel = (inner.connect)();
        inner
            .calls
            .lock()
            .unwrap()
            .in_flight
            .insert(key.clone(), Arc::clone(&channel));
        let progress = if tool.waits {
            params
                .get("_meta")
                .and_then(|meta| meta.get("progressToken"))
                .cloned()
        } else {
            None
        };
        let worker = Arc::clone(inner);
        Inner::in_background(inner, move || {
            let ticker = progress.map(|token| worker.start_progress(token, key.clone()));
            let outcome = channel.send(&request);
            // No progress after the answer (or once cancelled).
            if let Some(ticker) = ticker {
                ticker.stop();
            }
            let was_cancelled = {
                let mut calls = worker.calls.lock().unwrap();
                calls.in_flight.remove(&key);
                calls.cancelled.remove(&key)
            };
            // A cancelled request gets no answer.
            if was_cancelled {
                return;
            }
            let result = match outcome {
                Ok(response) => McpServer::tool_result(&response),
                Err(Failure::Unreachable) => {
                    McpServer::text_result("Couldn't reach Impulse (is it still running?).", true)
                }
                Err(Failure::NoAnswer) => McpServer::text_result("Impulse didn't answer.", true),
            };
            worker.send_result(result, &id, era);
        });
    }

    /// Run work that waits on the app (a tool call can wait minutes) on a
    /// thread of its own, counted for `finish`.
    fn in_background(inner: &Arc<Inner>, work: impl FnOnce() + Send + 'static) {
        *inner.pending.lock().unwrap() += 1;
        let tracker = Arc::clone(inner);
        thread::spawn(move || {
            work();
            let mut pending = tracker.pending.lock().unwrap();
            *pending -= 1;
            tracker.pending_done.notify_all();
        });
    }

    fn cancel(&self, request_id: &Value) {
        let key = Self::key(request_id);
        let channel = {
            let mut calls = self.calls.lock().unwrap();
            let channel = calls.in_flight.get(&key).cloned();
            if channel.is_some() {
                calls.cancelled.insert(key);
            }
            channel
        };
        if let Some(channel) = channel {
            channel.cancel();
        }
    }

    fn is_cancelled(&self, key: &str) -> bool {
        self.calls.lock().unwrap().cancelled.contains(key)
    }

    /// Progress notifications while a tool waits, so the agent (and its
    /// timeout) can tell it's alive.
    fn start_progress(self: &Arc<Self>, token: Value, key: String) -> Ticker {
        let interval = *self.progress_interval.lock().unwrap();
        let started = Instant::now();
        let server = Arc::downgrade(self);
        Ticker::start(interval, move || {
            let Some(server) = server.upgrade() else {
                return;
            };
            if server.is_cancelled(&key) {
                return;
            }
            let elapsed = started.elapsed().as_secs();
            let message = json!({
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": {
                    "progressToken": token,
                    "progress": elapsed,
                    "message": format!("Waiting ({elapsed} s)"),
                },
            });
            (server.emit)(&json::line(&message));
        })
    }

    /// A result, in the shape of the request's revision: a stateless one
    /// says it's complete, and who answered.
    fn send_result(&self, result: Value, id: &Value, era: Era) {
        let mut result = result;
        if era == Era::Stateless {
            if let Value::Object(fields) = &mut result {
                fields.insert("resultType".into(), json!("complete"));
                let meta = fields.entry("_meta").or_insert_with(|| json!({}));
                if !meta.is_object() {
                    *meta = json!({});
                }
                meta[SERVER_INFO_KEY] = self.server_info();
            }
        }
        (self.emit)(&json::line(
            &json!({"jsonrpc": "2.0", "id": id, "result": result}),
        ));
    }

    fn send_error(&self, code: i64, message: &str, id: &Value, data: Option<Value>) {
        let mut error = json!({"code": code, "message": message});
        if let Some(data) = data {
            error["data"] = data;
        }
        (self.emit)(&json::line(
            &json!({"jsonrpc": "2.0", "id": id, "error": error}),
        ));
    }

    fn key(id: &Value) -> String {
        json::write(id, json::Style::COMPACT)
    }
}

/// Calls `tick` every `interval` on a thread of its own until stopped.
struct Ticker {
    state: Arc<(Mutex<bool>, Condvar)>,
}

impl Ticker {
    fn start(interval: Duration, tick: impl Fn() + Send + 'static) -> Self {
        let state = Arc::new((Mutex::new(false), Condvar::new()));
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            let (stopped, wake) = &*shared;
            let mut stopped = stopped.lock().unwrap();
            let mut next = Instant::now() + interval;
            loop {
                let now = Instant::now();
                if !*stopped && now < next {
                    stopped = wake.wait_timeout(stopped, next - now).unwrap().0;
                    continue;
                }
                if *stopped {
                    return;
                }
                // Ticks while holding the lock, so `stop` waits for one
                // under way.
                tick();
                next += interval;
            }
        });
        Ticker { state }
    }

    /// Once this returns, no more ticks go out.
    fn stop(self) {
        let (stopped, wake) = &*self.state;
        *stopped.lock().unwrap() = true;
        wake.notify_all();
    }
}
