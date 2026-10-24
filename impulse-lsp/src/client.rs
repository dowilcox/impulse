//! One language server: its process, the initialize handshake, requests
//! answered as futures (any executor can await them) and what the server
//! sends on its own (diagnostics) as events on a channel.

use std::collections::HashMap;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, Ordering};
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
    /// `window/logMessage` and `window/showMessage`.
    Message(String),
    /// The process ended.
    Exited,
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

pub struct LspClient {
    pub name: String,
    pub root: PathBuf,
    child: Mutex<Child>,
    stdin: Mutex<BufWriter<ChildStdin>>,
    next_id: AtomicI64,
    pending: Pending,
    capabilities: Mutex<ServerCapabilities>,
    /// UTF-32 (columns in characters) when the server agreed to it,
    /// otherwise LSP's default UTF-16.
    encoding: Mutex<PositionEncodingKind>,
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

        let client = Arc::new(LspClient {
            name: name.to_string(),
            root: root.to_path_buf(),
            child: Mutex::new(child),
            stdin: Mutex::new(BufWriter::new(stdin)),
            next_id: AtomicI64::new(1),
            pending: Arc::default(),
            capabilities: Mutex::default(),
            encoding: Mutex::new(PositionEncodingKind::UTF16),
        });

        // The reader thread answers pending requests and the server's own
        // requests, and forwards notifications.
        let weak = Arc::downgrade(&client);
        let pending = client.pending.clone();
        let server = name.to_string();
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
                let _ = events.send_blocking(ServerEvent::Exited);
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
        let mut stdin = self.stdin.lock().unwrap();
        write_message(&mut *stdin, &body).map_err(|e| e.to_string())
    }

    /// Ask the server to exit, then make sure it has.
    pub async fn shutdown(&self) {
        let _ = self.request::<lsp_types::request::Shutdown>(()).await;
        let _ = self.notify::<lsp_types::notification::Exit>(());
        let _ = self.child.lock().unwrap().kill();
    }
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
                "window/logMessage" | "window/showMessage" => params
                    .get("message")
                    .and_then(Value::as_str)
                    .map(|m| ServerEvent::Message(m.to_string())),
                _ => None,
            };
            if let Some(event) = event {
                let _ = events.send_blocking(event);
            }
        }
        (None, None) => {}
    }
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
            ..Default::default()
        }),
        ..Default::default()
    }
}
