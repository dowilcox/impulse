use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex};

type PendingRequests =
    Arc<TokioMutex<HashMap<i64, oneshot::Sender<Result<serde_json::Value, String>>>>>;

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 Transport Types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str, // always "2.0"
    id: i64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<i64>,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
    method: Option<String>,
    params: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ---------------------------------------------------------------------------
// LspEvent – sent from the LSP backend to the frontend
// ---------------------------------------------------------------------------

/// Events sent from the LSP backend to the frontend.
#[derive(Debug)]
pub enum LspEvent {
    Diagnostics {
        uri: String,
        diagnostics: Vec<lsp_types::Diagnostic>,
    },
    Initialized {
        language_id: String,
    },
    ServerError {
        language_id: String,
        message: String,
    },
    ServerExited {
        language_id: String,
    },
}

// ---------------------------------------------------------------------------
// Helper: parse a string into an lsp_types::Uri
// ---------------------------------------------------------------------------

fn parse_uri(s: &str) -> Result<lsp_types::Uri, String> {
    lsp_types::Uri::from_str(s).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// LspClient – a client connected to a single language server process
// ---------------------------------------------------------------------------

/// A client connected to a single language server process.
pub struct LspClient {
    /// Channel to send outgoing requests/notifications to the writer task.
    sender: mpsc::UnboundedSender<Vec<u8>>,
    /// Pending request handlers, keyed by request ID.
    pending: PendingRequests,
    /// Next request ID counter.
    next_id: Arc<TokioMutex<i64>>,
    /// Server capabilities (set after initialization).
    pub capabilities: Arc<TokioMutex<Option<lsp_types::ServerCapabilities>>>,
    /// Channel for sending events to the frontend.
    event_tx: mpsc::UnboundedSender<LspEvent>,
    /// Language ID this client serves.
    language_id: String,
}

impl LspClient {
    /// Spawn a language server process and connect to it over stdio.
    pub async fn start(
        command: &str,
        args: &[String],
        root_uri: &str,
        language_id: &str,
        event_tx: mpsc::UnboundedSender<LspEvent>,
    ) -> Result<Self, String> {
        let mut child = TokioCommand::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start LSP server '{}': {}", command, e))?;

        let stdin = child.stdin.take().ok_or("Failed to get stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get stdout")?;

        let (sender, receiver) = mpsc::unbounded_channel::<Vec<u8>>();
        let pending: PendingRequests = Arc::new(TokioMutex::new(HashMap::new()));
        let next_id = Arc::new(TokioMutex::new(1i64));

        // Spawn writer task
        tokio::spawn(Self::writer_task(stdin, receiver));

        // Spawn reader task
        let pending_clone = pending.clone();
        let event_tx_clone = event_tx.clone();
        let lang_id = language_id.to_string();
        tokio::spawn(async move {
            Self::reader_task(stdout, pending_clone, event_tx_clone, &lang_id).await;
        });

        // Spawn a task to detect server exit
        let event_tx_exit = event_tx.clone();
        let lang_exit = language_id.to_string();
        tokio::spawn(async move {
            let _ = child.wait().await;
            let _ = event_tx_exit.send(LspEvent::ServerExited {
                language_id: lang_exit,
            });
        });

        let client = LspClient {
            sender,
            pending,
            next_id,
            capabilities: Arc::new(TokioMutex::new(None)),
            event_tx: event_tx.clone(),
            language_id: language_id.to_string(),
        };

        // Perform initialization handshake
        client.initialize(root_uri).await?;

        Ok(client)
    }

    // -- internal tasks -----------------------------------------------------

    /// Writer task: reads messages from channel and writes to stdin with
    /// `Content-Length` headers.
    async fn writer_task(
        mut stdin: tokio::process::ChildStdin,
        mut receiver: mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        while let Some(msg) = receiver.recv().await {
            let header = format!("Content-Length: {}\r\n\r\n", msg.len());
            if stdin.write_all(header.as_bytes()).await.is_err() {
                break;
            }
            if stdin.write_all(&msg).await.is_err() {
                break;
            }
            if stdin.flush().await.is_err() {
                break;
            }
        }
    }

    /// Reader task: reads JSON-RPC messages from stdout and dispatches them.
    async fn reader_task(
        stdout: tokio::process::ChildStdout,
        pending: PendingRequests,
        event_tx: mpsc::UnboundedSender<LspEvent>,
        language_id: &str,
    ) {
        let mut reader = BufReader::new(stdout);
        loop {
            // Read headers until the blank line separator.
            let mut header_line = String::new();
            let mut content_length: usize = 0;
            loop {
                header_line.clear();
                match reader.read_line(&mut header_line).await {
                    Ok(0) => return, // EOF
                    Ok(_) => {
                        let trimmed = header_line.trim();
                        if trimmed.is_empty() {
                            break; // End of headers
                        }
                        if let Some(len_str) = trimmed.strip_prefix("Content-Length: ") {
                            if let Ok(len) = len_str.parse::<usize>() {
                                content_length = len;
                            }
                        }
                    }
                    Err(_) => return,
                }
            }

            if content_length == 0 {
                continue;
            }

            // Read the message body.
            let mut body = vec![0u8; content_length];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }

            // Parse JSON-RPC message.
            let msg: JsonRpcResponse = match serde_json::from_slice(&body) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("Failed to parse LSP message: {}", e);
                    continue;
                }
            };

            // If it has an id and no method, it is a response to one of our
            // requests.
            if let Some(id) = msg.id {
                if msg.method.is_none() {
                    let mut pending = pending.lock().await;
                    if let Some(tx) = pending.remove(&id) {
                        if let Some(error) = msg.error {
                            let _ = tx
                                .send(Err(format!("LSP error {}: {}", error.code, error.message)));
                        } else {
                            let _ = tx.send(Ok(msg.result.unwrap_or(serde_json::Value::Null)));
                        }
                    }
                    continue;
                }
            }

            // If it has a method, it is a notification or server-to-client
            // request.
            if let Some(method) = &msg.method {
                Self::handle_server_notification(method, msg.params, &event_tx, language_id);
            }
        }
    }

    fn handle_server_notification(
        method: &str,
        params: Option<serde_json::Value>,
        event_tx: &mpsc::UnboundedSender<LspEvent>,
        _language_id: &str,
    ) {
        match method {
            "textDocument/publishDiagnostics" => {
                if let Some(params) = params {
                    if let Ok(diag_params) =
                        serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params)
                    {
                        let _ = event_tx.send(LspEvent::Diagnostics {
                            uri: diag_params.uri.to_string(),
                            diagnostics: diag_params.diagnostics,
                        });
                    }
                }
            }
            _ => {
                log::debug!("Unhandled LSP notification: {}", method);
            }
        }
    }

    // -- public API ---------------------------------------------------------

    /// Send a JSON-RPC request and wait for the response.
    pub async fn request<P: Serialize>(
        &self,
        method: &str,
        params: P,
    ) -> Result<serde_json::Value, String> {
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            id
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params: Some(serde_json::to_value(params).map_err(|e| e.to_string())?),
        };

        let body = serde_json::to_vec(&request).map_err(|e| e.to_string())?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        self.sender.send(body).map_err(|e| e.to_string())?;

        rx.await.map_err(|_| "Request cancelled".to_string())?
    }

    /// Send a JSON-RPC notification (no response expected).
    pub fn notify<P: Serialize>(&self, method: &str, params: P) -> Result<(), String> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": serde_json::to_value(params).map_err(|e| e.to_string())?
        });
        let body = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
        self.sender.send(body).map_err(|e| e.to_string())
    }

    // -- initialization -----------------------------------------------------

    /// Perform the LSP initialization handshake.
    #[allow(deprecated)] // root_uri is deprecated in favour of workspace_folders
    async fn initialize(&self, root_uri: &str) -> Result<(), String> {
        let params = lsp_types::InitializeParams {
            root_uri: Some(parse_uri(root_uri)?),
            capabilities: lsp_types::ClientCapabilities {
                text_document: Some(lsp_types::TextDocumentClientCapabilities {
                    completion: Some(lsp_types::CompletionClientCapabilities {
                        completion_item: Some(lsp_types::CompletionItemCapability {
                            snippet_support: Some(false),
                            documentation_format: Some(vec![lsp_types::MarkupKind::PlainText]),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    hover: Some(lsp_types::HoverClientCapabilities {
                        content_format: Some(vec![
                            lsp_types::MarkupKind::PlainText,
                            lsp_types::MarkupKind::Markdown,
                        ]),
                        ..Default::default()
                    }),
                    publish_diagnostics: Some(lsp_types::PublishDiagnosticsClientCapabilities {
                        related_information: Some(true),
                        ..Default::default()
                    }),
                    definition: Some(lsp_types::GotoCapability {
                        link_support: Some(false),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            client_info: Some(lsp_types::ClientInfo {
                name: "Impulse".to_string(),
                version: Some("0.1.0".to_string()),
            }),
            ..Default::default()
        };

        let result = self.request("initialize", params).await?;

        // Parse server capabilities.
        if let Ok(init_result) = serde_json::from_value::<lsp_types::InitializeResult>(result) {
            *self.capabilities.lock().await = Some(init_result.capabilities);
        }

        // Send "initialized" notification.
        self.notify("initialized", lsp_types::InitializedParams {})?;

        let _ = self.event_tx.send(LspEvent::Initialized {
            language_id: self.language_id.clone(),
        });

        Ok(())
    }

    // -- document lifecycle methods -----------------------------------------

    pub fn did_open(
        &self,
        uri: &str,
        language_id: &str,
        version: i32,
        text: &str,
    ) -> Result<(), String> {
        self.notify(
            "textDocument/didOpen",
            lsp_types::DidOpenTextDocumentParams {
                text_document: lsp_types::TextDocumentItem {
                    uri: parse_uri(uri)?,
                    language_id: language_id.to_string(),
                    version,
                    text: text.to_string(),
                },
            },
        )
    }

    pub fn did_change(&self, uri: &str, version: i32, text: &str) -> Result<(), String> {
        self.notify(
            "textDocument/didChange",
            lsp_types::DidChangeTextDocumentParams {
                text_document: lsp_types::VersionedTextDocumentIdentifier {
                    uri: parse_uri(uri)?,
                    version,
                },
                content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: text.to_string(),
                }],
            },
        )
    }

    pub fn did_save(&self, uri: &str) -> Result<(), String> {
        self.notify(
            "textDocument/didSave",
            lsp_types::DidSaveTextDocumentParams {
                text_document: lsp_types::TextDocumentIdentifier {
                    uri: parse_uri(uri)?,
                },
                text: None,
            },
        )
    }

    pub fn did_close(&self, uri: &str) -> Result<(), String> {
        self.notify(
            "textDocument/didClose",
            lsp_types::DidCloseTextDocumentParams {
                text_document: lsp_types::TextDocumentIdentifier {
                    uri: parse_uri(uri)?,
                },
            },
        )
    }

    // -- feature requests ---------------------------------------------------

    pub async fn completion(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::CompletionItem>, String> {
        let result = self
            .request(
                "textDocument/completion",
                lsp_types::CompletionParams {
                    text_document_position: lsp_types::TextDocumentPositionParams {
                        text_document: lsp_types::TextDocumentIdentifier {
                            uri: parse_uri(uri)?,
                        },
                        position: lsp_types::Position { line, character },
                    },
                    context: None,
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            )
            .await?;

        // CompletionResponse can be either an array or a CompletionList.
        if let Ok(list) = serde_json::from_value::<lsp_types::CompletionResponse>(result) {
            match list {
                lsp_types::CompletionResponse::Array(items) => Ok(items),
                lsp_types::CompletionResponse::List(list) => Ok(list.items),
            }
        } else {
            Ok(vec![])
        }
    }

    pub async fn hover(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::Hover>, String> {
        let result = self
            .request(
                "textDocument/hover",
                lsp_types::HoverParams {
                    text_document_position_params: lsp_types::TextDocumentPositionParams {
                        text_document: lsp_types::TextDocumentIdentifier {
                            uri: parse_uri(uri)?,
                        },
                        position: lsp_types::Position { line, character },
                    },
                    work_done_progress_params: Default::default(),
                },
            )
            .await?;

        if result.is_null() {
            Ok(None)
        } else {
            serde_json::from_value(result).map_err(|e| e.to_string())
        }
    }

    pub async fn definition(
        &self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::GotoDefinitionResponse>, String> {
        let result = self
            .request(
                "textDocument/definition",
                lsp_types::GotoDefinitionParams {
                    text_document_position_params: lsp_types::TextDocumentPositionParams {
                        text_document: lsp_types::TextDocumentIdentifier {
                            uri: parse_uri(uri)?,
                        },
                        position: lsp_types::Position { line, character },
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            )
            .await?;

        if result.is_null() {
            Ok(None)
        } else {
            serde_json::from_value(result).map_err(|e| e.to_string())
        }
    }

    /// Shutdown the language server gracefully.
    pub async fn shutdown(&self) -> Result<(), String> {
        let _ = self.request("shutdown", serde_json::Value::Null).await;
        self.notify("exit", serde_json::Value::Null)
    }
}

// ---------------------------------------------------------------------------
// LspConfig – maps language IDs to their LSP server configurations
// ---------------------------------------------------------------------------

/// Configuration for a language server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    pub command: String,
    pub args: Vec<String>,
}

/// Maps language IDs to their LSP server configurations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    pub servers: HashMap<String, LspServerConfig>,
}

impl Default for LspConfig {
    fn default() -> Self {
        let mut servers = HashMap::new();
        servers.insert(
            "rust".into(),
            LspServerConfig {
                command: "rust-analyzer".into(),
                args: vec![],
            },
        );
        servers.insert(
            "python".into(),
            LspServerConfig {
                command: "pyright-langserver".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "c".into(),
            LspServerConfig {
                command: "clangd".into(),
                args: vec![],
            },
        );
        servers.insert(
            "cpp".into(),
            LspServerConfig {
                command: "clangd".into(),
                args: vec![],
            },
        );
        servers.insert(
            "javascript".into(),
            LspServerConfig {
                command: "typescript-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "typescript".into(),
            LspServerConfig {
                command: "typescript-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "php".into(),
            LspServerConfig {
                command: "intelephense".into(),
                args: vec!["--stdio".into()],
            },
        );
        LspConfig { servers }
    }
}

// ---------------------------------------------------------------------------
// LspRegistry – manages multiple LSP client instances, one per language
// ---------------------------------------------------------------------------

/// Manages multiple LSP client instances, one per language.
pub struct LspRegistry {
    clients: Arc<TokioMutex<HashMap<String, Arc<LspClient>>>>,
    config: LspConfig,
    root_uri: String,
    event_tx: mpsc::UnboundedSender<LspEvent>,
}

impl LspRegistry {
    pub fn new(root_uri: String, event_tx: mpsc::UnboundedSender<LspEvent>) -> Self {
        Self {
            clients: Arc::new(TokioMutex::new(HashMap::new())),
            config: LspConfig::default(),
            root_uri,
            event_tx,
        }
    }

    /// Get or start an LSP client for the given language ID.
    pub async fn get_client(&self, language_id: &str) -> Option<Arc<LspClient>> {
        // Check if already running.
        {
            let clients = self.clients.lock().await;
            if let Some(client) = clients.get(language_id) {
                return Some(client.clone());
            }
        }

        // Try to start a new server.
        let server_config = match self.config.servers.get(language_id) {
            Some(cfg) => cfg,
            None => {
                log::info!("No LSP server configured for language: {}", language_id);
                return None;
            }
        };

        match LspClient::start(
            &server_config.command,
            &server_config.args,
            &self.root_uri,
            language_id,
            self.event_tx.clone(),
        )
        .await
        {
            Ok(client) => {
                let client = Arc::new(client);
                let mut clients = self.clients.lock().await;
                clients.insert(language_id.to_string(), client.clone());
                Some(client)
            }
            Err(e) => {
                log::warn!("Failed to start LSP server for '{}': {}", language_id, e);
                let _ = self.event_tx.send(LspEvent::ServerError {
                    language_id: language_id.to_string(),
                    message: e,
                });
                None
            }
        }
    }

    /// Remove a client (e.g., after server exits).
    pub async fn remove_client(&self, language_id: &str) {
        let mut clients = self.clients.lock().await;
        clients.remove(language_id);
    }

    /// Shutdown all running servers.
    pub async fn shutdown_all(&self) {
        let clients = self.clients.lock().await;
        for (_, client) in clients.iter() {
            let _ = client.shutdown().await;
        }
    }
}
