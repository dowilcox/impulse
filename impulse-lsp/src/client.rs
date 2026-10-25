//! One language server: its process, the initialize handshake, requests
//! answered as futures (any executor can await them) and what the server
//! sends on its own (diagnostics) as events on a channel.

use std::collections::HashMap;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use futures::channel::oneshot;
use lsp_types::notification::Notification;
use lsp_types::request::Request;
use lsp_types::{
    ClientCapabilities, CompletionClientCapabilities, CompletionItemCapability,
    GeneralClientCapabilities, GotoCapability, HoverClientCapabilities, InitializeParams,
    InitializeResult, InitializedParams, MarkupKind, PositionEncodingKind,
    PublishDiagnosticsClientCapabilities, ServerCapabilities, TextDocumentClientCapabilities,
    TextDocumentSyncClientCapabilities, Uri, WorkspaceFolder,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::framing::{read_message, write_message};

/// What a server sends without being asked.
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// From the server named first (its id in the config).
    Diagnostics(String, lsp_types::PublishDiagnosticsParams),
    /// `window/showMessage` (or a `window/showMessageRequest`, which has no
    /// buttons here): the server's id, the message type (1 error, 2
    /// warning, 3 info, 4 log) and the text.
    ShowMessage {
        server: String,
        kind: i64,
        text: String,
    },
    /// `$/progress` work: `kind` is `begin`, `report` or `end`; the token
    /// tells pieces of work apart.
    Progress {
        server: String,
        token: String,
        kind: String,
        title: Option<String>,
        message: Option<String>,
        percentage: Option<u32>,
    },
    /// A server couldn't start (its command isn't installed): what to tell
    /// the user.
    Unavailable { server: String, message: String },
    /// The process ended; `expected` after `shutdown`.
    Exited {
        server: String,
        root: PathBuf,
        expected: bool,
    },
    /// `workspace/applyEdit`: an edit the server asks the client to make
    /// (after a code action's command, say), with the server's label for
    /// it, its project root and whether its positions count characters.
    ApplyEdit {
        edit: lsp_types::WorkspaceEdit,
        label: Option<String>,
        root: std::path::PathBuf,
        utf32: bool,
    },
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

pub struct LspClient {
    pub name: String,
    pub root: PathBuf,
    child: Mutex<Child>,
    /// Messages for the server, written in order by the client's writer
    /// thread, so a sender never waits on a server that isn't reading (the
    /// Swift client's write queue).
    outbox: mpsc::Sender<Vec<u8>>,
    next_id: AtomicI64,
    pending: Pending,
    capabilities: Mutex<ServerCapabilities>,
    /// UTF-32 (columns in characters) when the server agreed to it,
    /// otherwise LSP's default UTF-16.
    encoding: Mutex<PositionEncodingKind>,
    /// `shutdown` was asked for: the exit that follows isn't a crash.
    retired: Arc<std::sync::atomic::AtomicBool>,
    /// Requests' deadlines, watched by the client's timer thread.
    deadlines: Arc<Deadlines>,
}

/// When each waiting request is given up on: (deadline, request id).
#[derive(Default)]
struct Deadlines {
    due: Mutex<Vec<(std::time::Instant, i64)>>,
    changed: std::sync::Condvar,
}

/// How long a request may wait for its answer (the Swift client's): 5
/// seconds for what the editor waits on as you type or point, 10 for
/// formatting and code actions, 30 for the initialize handshake, 5 for
/// shutdown and 15 for the rest.
fn timeout_for(method: &str) -> std::time::Duration {
    let seconds = match method {
        "initialize" => 30,
        "shutdown" => 5,
        "textDocument/completion"
        | "textDocument/hover"
        | "textDocument/signatureHelp"
        | "textDocument/definition"
        | "textDocument/documentHighlight"
        | "textDocument/inlayHint"
        | "textDocument/prepareRename" => 5,
        "textDocument/formatting"
        | "textDocument/rangeFormatting"
        | "textDocument/codeAction"
        | "codeAction/resolve" => 10,
        _ => 15,
    };
    std::time::Duration::from_secs(seconds)
}

pub fn path_uri(path: &Path) -> Option<Uri> {
    impulse_kit::uri::file_path_to_uri(path).and_then(|s| Uri::from_str(&s).ok())
}

impl LspClient {
    /// Start `command` with `args` in `root` and complete the initialize
    /// handshake. Events go to `events`.
    pub async fn start(
        name: &str,
        command: &Path,
        args: &[String],
        initialization_options: Option<Value>,
        root: &Path,
        events: async_channel::Sender<ServerEvent>,
    ) -> Result<Arc<LspClient>, String> {
        let mut child = Command::new(command)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // A server's log goes nowhere unless IMPULSE_LSP_STDERR is set, for
            // debugging.
            .stderr(if std::env::var_os("IMPULSE_LSP_STDERR").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .spawn()
            .map_err(|e| format!("{}: {e}", command.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;

        let (outbox, messages) = mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name(format!("lsp-{name}-writer"))
            .spawn(move || {
                let mut stdin = BufWriter::new(stdin);
                for body in messages {
                    if write_message(&mut stdin, &body).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let client = Arc::new(LspClient {
            name: name.to_string(),
            root: root.to_path_buf(),
            child: Mutex::new(child),
            outbox,
            next_id: AtomicI64::new(1),
            pending: Arc::default(),
            capabilities: Mutex::default(),
            encoding: Mutex::new(PositionEncodingKind::UTF16),
            retired: Arc::default(),
            deadlines: Arc::default(),
        });
        watch_deadlines(&client);

        // The reader thread answers pending requests and the server's own
        // requests, and forwards notifications.
        let weak = Arc::downgrade(&client);
        let pending = client.pending.clone();
        let server = name.to_string();
        let (retired, exited_root) = (client.retired.clone(), root.to_path_buf());
        std::thread::Builder::new()
            .name(format!("lsp-{name}"))
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                while let Ok(Some(body)) = read_message(&mut reader) {
                    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
                        continue;
                    };
                    dispatch(&server, message, &pending, &weak, &events);
                }
                for (_, sender) in pending.lock().unwrap().drain() {
                    let _ = sender.send(Err("server exited".into()));
                }
                let _ = events.send_blocking(ServerEvent::Exited {
                    server,
                    root: exited_root,
                    expected: retired.load(Ordering::SeqCst),
                });
            })
            .map_err(|e| e.to_string())?;

        let root_uri = path_uri(root).ok_or("root isn't a path")?;
        #[allow(deprecated)]
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.clone()),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri,
                name: root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            }]),
            capabilities: client_capabilities(),
            initialization_options,
            ..Default::default()
        };
        let result: InitializeResult = client
            .request::<lsp_types::request::Initialize>(params)
            .await?;
        *client.encoding.lock().unwrap() = result
            .capabilities
            .position_encoding
            .clone()
            .unwrap_or(PositionEncodingKind::UTF16);
        *client.capabilities.lock().unwrap() = result.capabilities;
        client.notify::<lsp_types::notification::Initialized>(InitializedParams {})?;
        // Many servers (typescript-language-server among them) take their
        // main settings pushed rather than asked for.
        client.notify::<lsp_types::notification::DidChangeConfiguration>(
            lsp_types::DidChangeConfigurationParams {
                settings: crate::config::default_workspace_settings(),
            },
        )?;
        Ok(client)
    }

    pub fn capabilities(&self) -> ServerCapabilities {
        self.capabilities.lock().unwrap().clone()
    }

    /// Whether the server takes changed ranges rather than whole texts.
    pub fn incremental_sync(&self) -> bool {
        use lsp_types::{TextDocumentSyncCapability, TextDocumentSyncKind};
        let kind = match &self.capabilities.lock().unwrap().text_document_sync {
            Some(TextDocumentSyncCapability::Kind(kind)) => Some(*kind),
            Some(TextDocumentSyncCapability::Options(options)) => options.change,
            None => None,
        };
        kind == Some(TextDocumentSyncKind::INCREMENTAL)
    }

    /// Whether positions are in characters (UTF-32) rather than UTF-16.
    pub fn uses_utf32(&self) -> bool {
        *self.encoding.lock().unwrap() == PositionEncodingKind::UTF32
    }

    pub async fn request<R: Request>(&self, params: R::Params) -> Result<R::Result, String>
    where
        R::Result: DeserializeOwned,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, sender);
        self.deadlines
            .due
            .lock()
            .unwrap()
            .push((std::time::Instant::now() + timeout_for(R::METHOD), id));
        self.deadlines.changed.notify_one();
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": R::METHOD, "params": params });
        if let Err(error) = self.send(&message) {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        let value = receiver
            .await
            .map_err(|_| "request dropped".to_string())??;
        serde_json::from_value(value).map_err(|e| format!("{}: {e}", R::METHOD))
    }

    pub fn notify<N: Notification>(&self, params: N::Params) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": N::METHOD, "params": params }))
    }

    fn send(&self, message: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(message).map_err(|e| e.to_string())?;
        self.outbox
            .send(body)
            .map_err(|_| "the server's input is closed".to_string())
    }

    /// Whether it was told to stop (`shutdown` or `kill`).
    pub fn is_retired(&self) -> bool {
        self.retired.load(Ordering::SeqCst)
    }

    /// Stop the process now (a shutdown that took too long).
    pub fn kill(&self) {
        self.retired.store(true, Ordering::SeqCst);
        let _ = self.child.lock().unwrap().kill();
    }

    /// Ask the server to exit, then make sure it has.
    pub async fn shutdown(&self) {
        self.retired.store(true, Ordering::SeqCst);
        let _ = self.request::<lsp_types::request::Shutdown>(()).await;
        let _ = self.notify::<lsp_types::notification::Exit>(());
        let _ = self.child.lock().unwrap().kill();
    }
}

/// The client's timer thread: a request still unanswered at its deadline
/// fails with "timed out", and the server is told to stop working on it
/// (`$/cancelRequest`). Ends with the client.
fn watch_deadlines(client: &Arc<LspClient>) {
    let deadlines = client.deadlines.clone();
    let pending = client.pending.clone();
    let weak = Arc::downgrade(client);
    let _ = std::thread::Builder::new()
        .name(format!("lsp-{}-timeouts", client.name))
        .spawn(move || loop {
            let expired: Vec<i64> = {
                let mut due = deadlines.due.lock().unwrap();
                let now = std::time::Instant::now();
                let next = due.iter().map(|(at, _)| *at).min();
                let wait = next
                    .map(|at| at.saturating_duration_since(now))
                    .unwrap_or(std::time::Duration::from_secs(1))
                    .min(std::time::Duration::from_secs(1));
                if wait > std::time::Duration::ZERO {
                    due = deadlines.changed.wait_timeout(due, wait).unwrap().0;
                }
                let now = std::time::Instant::now();
                let (gone, kept): (Vec<_>, Vec<_>) =
                    due.drain(..).partition(|(at, _)| *at <= now);
                *due = kept;
                gone.into_iter().map(|(_, id)| id).collect()
            };
            let Some(client) = weak.upgrade() else {
                return;
            };
            for id in expired {
                let Some(sender) = pending.lock().unwrap().remove(&id) else {
                    continue;
                };
                let _ = sender.send(Err("timed out".into()));
                let _ = client.send(
                    &json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": id } }),
                );
            }
        });
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn dispatch(
    server: &str,
    message: Value,
    pending: &Pending,
    client: &std::sync::Weak<LspClient>,
    events: &async_channel::Sender<ServerEvent>,
) {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str);
    match (id, method) {
        // A response to one of ours.
        (Some(id), None) => {
            let Some(id) = id.as_i64() else { return };
            let Some(sender) = pending.lock().unwrap().remove(&id) else {
                return;
            };
            let result = match message.get("error") {
                Some(error) => Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string()),
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = sender.send(result);
        }
        // A request from the server: answer what a client must, so servers
        // that wait on the answer (configuration, progress) carry on.
        (Some(id), Some(method)) => {
            let result = match method {
                "workspace/configuration" => Value::Array(
                    message
                        .pointer("/params/items")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|item| {
                            let section = item.get("section").and_then(Value::as_str);
                            crate::config::workspace_configuration_for_section(
                                section.unwrap_or_default(),
                            )
                        })
                        .collect(),
                ),
                // Made by the app (asking first where it must); a server
                // that waits on the answer carries on.
                "workspace/applyEdit" => {
                    let params: Option<lsp_types::ApplyWorkspaceEditParams> = message
                        .get("params")
                        .cloned()
                        .and_then(|p| serde_json::from_value(p).ok());
                    match (params, client.upgrade()) {
                        (Some(params), Some(client)) => {
                            let _ = events.send_blocking(ServerEvent::ApplyEdit {
                                edit: params.edit,
                                label: params.label,
                                root: client.root.clone(),
                                utf32: client.uses_utf32(),
                            });
                            json!({ "applied": true })
                        }
                        _ => json!({ "applied": false }),
                    }
                }
                "workspace/workspaceFolders" => match client.upgrade() {
                    Some(client) => json!([{
                        "uri": path_uri(&client.root).map(|u| u.to_string()),
                        "name": client.root.file_name().map(|n| n.to_string_lossy().into_owned()),
                    }]),
                    None => Value::Null,
                },
                // No buttons: shown like a message, answered "dismissed".
                "window/showMessageRequest" => {
                    if let Some(event) = show_message(server, message.get("params")) {
                        let _ = events.send_blocking(event);
                    }
                    Value::Null
                }
                _ => Value::Null,
            };
            if let Some(client) = client.upgrade() {
                let _ = client.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
        }
        (None, Some(method)) => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let event = match method {
                "textDocument/publishDiagnostics" => serde_json::from_value(params)
                    .ok()
                    .map(|params| ServerEvent::Diagnostics(server.to_string(), params)),
                "window/showMessage" => show_message(server, Some(&params)),
                "$/progress" => progress(server, &params),
                "window/logMessage" => {
                    if let Some(text) = params.get("message").and_then(Value::as_str) {
                        log::debug!("{server}: {text}");
                    }
                    None
                }
                _ => None,
            };
            if let Some(event) = event {
                let _ = events.send_blocking(event);
            }
        }
        (None, None) => {}
    }
}

/// A `window/showMessage`'s event.
fn show_message(server: &str, params: Option<&Value>) -> Option<ServerEvent> {
    let params = params?;
    Some(ServerEvent::ShowMessage {
        server: server.to_string(),
        kind: params.get("type").and_then(Value::as_i64).unwrap_or(3),
        text: params.get("message")?.as_str()?.to_string(),
    })
}

/// A `$/progress` notification's work-done event.
fn progress(server: &str, params: &Value) -> Option<ServerEvent> {
    let token = match params.get("token")? {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let value = params.get("value")?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    Some(ServerEvent::Progress {
        server: server.to_string(),
        token,
        kind: text("kind")?,
        title: text("title"),
        message: text("message"),
        percentage: value
            .get("percentage")
            .and_then(Value::as_u64)
            .map(|p| p.min(100) as u32),
    })
}

fn client_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        general: Some(GeneralClientCapabilities {
            position_encodings: Some(vec![
                PositionEncodingKind::UTF32,
                PositionEncodingKind::UTF16,
            ]),
            ..Default::default()
        }),
        text_document: Some(TextDocumentClientCapabilities {
            synchronization: Some(TextDocumentSyncClientCapabilities {
                did_save: Some(true),
                ..Default::default()
            }),
            completion: Some(CompletionClientCapabilities {
                completion_item: Some(CompletionItemCapability {
                    snippet_support: Some(false),
                    documentation_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            hover: Some(HoverClientCapabilities {
                content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                ..Default::default()
            }),
            definition: Some(GotoCapability {
                link_support: Some(true),
                ..Default::default()
            }),
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities::default()),
            code_action: Some(lsp_types::CodeActionClientCapabilities {
                // Actions as literals (rust-analyzer only sends assists then),
                // with their edits filled in when one is picked.
                code_action_literal_support: Some(lsp_types::CodeActionLiteralSupport {
                    code_action_kind: lsp_types::CodeActionKindLiteralSupport {
                        value_set: [
                            "",
                            "quickfix",
                            "refactor",
                            "refactor.extract",
                            "refactor.inline",
                            "refactor.rewrite",
                            "source",
                            "source.organizeImports",
                        ]
                        .iter()
                        .map(|kind| kind.to_string())
                        .collect(),
                    },
                }),
                data_support: Some(true),
                resolve_support: Some(lsp_types::CodeActionCapabilityResolveSupport {
                    properties: vec!["edit".to_string()],
                }),
                ..Default::default()
            }),
            formatting: Some(lsp_types::DocumentFormattingClientCapabilities::default()),
            declaration: Some(GotoCapability {
                link_support: Some(true),
                ..Default::default()
            }),
            type_definition: Some(GotoCapability {
                link_support: Some(true),
                ..Default::default()
            }),
            implementation: Some(GotoCapability {
                link_support: Some(true),
                ..Default::default()
            }),
            references: Some(Default::default()),
            rename: Some(lsp_types::RenameClientCapabilities {
                prepare_support: Some(true),
                ..Default::default()
            }),
            signature_help: Some(lsp_types::SignatureHelpClientCapabilities {
                signature_information: Some(lsp_types::SignatureInformationSettings {
                    documentation_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                    parameter_information: Some(lsp_types::ParameterInformationSettings {
                        label_offset_support: Some(true),
                    }),
                    active_parameter_support: Some(true),
                }),
                ..Default::default()
            }),
            document_highlight: Some(Default::default()),
            ..Default::default()
        }),
        // Long-running work (indexing) shown in the status bar.
        window: Some(lsp_types::WindowClientCapabilities {
            work_done_progress: Some(true),
            ..Default::default()
        }),
        // Edits across files, with files created, moved and deleted
        // (`impulse-app`'s workspace edits), as the Swift client offers.
        workspace: Some(lsp_types::WorkspaceClientCapabilities {
            apply_edit: Some(true),
            workspace_edit: Some(lsp_types::WorkspaceEditClientCapabilities {
                document_changes: Some(true),
                resource_operations: Some(vec![
                    lsp_types::ResourceOperationKind::Create,
                    lsp_types::ResourceOperationKind::Rename,
                    lsp_types::ResourceOperationKind::Delete,
                ]),
                failure_handling: Some(lsp_types::FailureHandlingKind::Abort),
                ..Default::default()
            }),
            configuration: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}
