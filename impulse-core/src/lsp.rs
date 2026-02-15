use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex};
use url::Url;

type PendingRequests =
    Arc<TokioMutex<HashMap<i64, oneshot::Sender<Result<serde_json::Value, String>>>>>;

const START_RETRY_COOLDOWN: Duration = Duration::from_secs(15);
const IMPULSE_INSTALL_HINT: &str =
    "Run `impulse --install-lsp-servers` (or `cargo run -p impulse-linux -- --install-lsp-servers`) to install managed web LSP servers.";

const RECOMMENDED_WEB_LSP_PACKAGES: &[&str] = &[
    "typescript",
    "typescript-language-server",
    "intelephense",
    "vscode-langservers-extracted",
    "@tailwindcss/language-server",
    "@vue/language-server",
    "svelte-language-server",
    "graphql-language-service-cli",
    "emmet-ls",
    "yaml-language-server",
    "dockerfile-language-server-nodejs",
    "bash-language-server",
];

const MANAGED_NPM_SERVER_COMMANDS: &[&str] = &[
    "typescript-language-server",
    "intelephense",
    "vscode-html-language-server",
    "vscode-css-language-server",
    "vscode-json-language-server",
    "vscode-eslint-language-server",
    "tailwindcss-language-server",
    "vue-language-server",
    "svelteserver",
    "graphql-lsp",
    "emmet-ls",
    "yaml-language-server",
    "docker-langserver",
    "bash-language-server",
];

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: i64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcMessage {
    // Required for JSON-RPC protocol deserialization; not read directly by Rust code.
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<serde_json::Value>,
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

#[derive(Debug)]
pub enum LspEvent {
    Diagnostics {
        uri: String,
        version: Option<i32>,
        diagnostics: Vec<lsp_types::Diagnostic>,
    },
    Initialized {
        client_key: String,
        server_id: String,
    },
    ServerError {
        client_key: String,
        server_id: String,
        message: String,
    },
    ServerExited {
        client_key: String,
        server_id: String,
    },
}

fn parse_uri(s: &str) -> Result<lsp_types::Uri, String> {
    lsp_types::Uri::from_str(s).map_err(|e| e.to_string())
}

fn uri_to_file_path(uri: &str) -> Option<PathBuf> {
    Url::parse(uri).ok()?.to_file_path().ok()
}

fn path_to_file_uri(path: &Path) -> Option<String> {
    crate::util::file_path_to_uri(path)
}

fn workspace_folder_name(root_uri: &str) -> String {
    uri_to_file_path(root_uri)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "workspace".to_string())
}

pub fn managed_lsp_root_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("impulse").join("lsp"))
}

pub fn managed_lsp_bin_dir() -> Option<PathBuf> {
    managed_lsp_root_dir().map(|dir| dir.join("node_modules").join(".bin"))
}

pub fn managed_web_lsp_commands() -> &'static [&'static str] {
    MANAGED_NPM_SERVER_COMMANDS
}

fn command_looks_like_path(command: &str) -> bool {
    command.contains(std::path::MAIN_SEPARATOR)
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match path.metadata() {
            Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111 != 0),
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn find_command_in_path(command: &str) -> Option<PathBuf> {
    if command_looks_like_path(command) {
        let path = PathBuf::from(command);
        return is_executable_file(&path).then_some(path);
    }

    let path_env: OsString = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        let candidate = dir.join(command);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn find_managed_command(command: &str) -> Option<PathBuf> {
    let managed = managed_lsp_bin_dir()?.join(command);
    is_executable_file(&managed).then_some(managed)
}

pub fn resolve_lsp_command_path(command: &str) -> Option<PathBuf> {
    find_command_in_path(command).or_else(|| find_managed_command(command))
}

fn is_managed_npm_server_command(command: &str) -> bool {
    MANAGED_NPM_SERVER_COMMANDS.contains(&command)
}

fn missing_command_message(server_id: &str, command: &str) -> String {
    if is_managed_npm_server_command(command) {
        format!(
            "LSP server '{}' requires '{}' but it is not installed. {}",
            server_id, command, IMPULSE_INSTALL_HINT
        )
    } else {
        format!(
            "LSP server '{}' requires '{}' but it is not in PATH. Install it or override `servers.{}` in lsp.json.",
            server_id, command, server_id
        )
    }
}

fn npm_is_available() -> bool {
    StdCommand::new("npm")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn install_managed_web_lsp_servers() -> Result<PathBuf, String> {
    if !npm_is_available() {
        return Err(
            "npm is required but was not found in PATH. Install Node.js + npm first.".to_string(),
        );
    }

    let root = managed_lsp_root_dir()
        .ok_or_else(|| "Unable to determine data directory for managed LSPs".to_string())?;
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create {}: {}", root.display(), e))?;

    let package_json = root.join("package.json");
    if !package_json.exists() {
        let package_doc = serde_json::json!({
            "name": "impulse-lsp-servers",
            "private": true,
            "description": "Managed web LSP dependencies for Impulse",
            "license": "UNLICENSED"
        });
        let content = serde_json::to_string_pretty(&package_doc)
            .map_err(|e| format!("Failed to serialize package.json: {}", e))?;
        fs::write(&package_json, content)
            .map_err(|e| format!("Failed to write {}: {}", package_json.display(), e))?;
    }

    let status = StdCommand::new("npm")
        .arg("install")
        .arg("--prefix")
        .arg(&root)
        .arg("--no-audit")
        .arg("--no-fund")
        .args(RECOMMENDED_WEB_LSP_PACKAGES)
        .status()
        .map_err(|e| format!("Failed to run npm install: {}", e))?;

    if !status.success() {
        return Err(format!(
            "npm install failed with status {} while installing managed LSP servers",
            status
        ));
    }

    managed_lsp_bin_dir().ok_or_else(|| {
        "Installation completed but managed bin directory could not be determined".to_string()
    })
}

#[derive(Debug, Clone)]
pub struct LspCommandStatus {
    pub command: String,
    pub resolved_path: Option<PathBuf>,
}

pub fn managed_web_lsp_status() -> Vec<LspCommandStatus> {
    MANAGED_NPM_SERVER_COMMANDS
        .iter()
        .map(|cmd| LspCommandStatus {
            command: (*cmd).to_string(),
            resolved_path: resolve_lsp_command_path(cmd),
        })
        .collect()
}

fn send_jsonrpc_result(
    sender: &mpsc::UnboundedSender<Vec<u8>>,
    id: serde_json::Value,
    result: serde_json::Value,
) {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    if let Ok(body) = serde_json::to_vec(&msg) {
        let _ = sender.send(body);
    }
}

fn send_jsonrpc_error(
    sender: &mpsc::UnboundedSender<Vec<u8>>,
    id: serde_json::Value,
    code: i64,
    message: &str,
) {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    });
    if let Ok(body) = serde_json::to_vec(&msg) {
        let _ = sender.send(body);
    }
}

pub struct LspClient {
    sender: mpsc::UnboundedSender<Vec<u8>>,
    pending: PendingRequests,
    next_id: Arc<TokioMutex<i64>>,
    pub capabilities: Arc<TokioMutex<Option<lsp_types::ServerCapabilities>>>,
    event_tx: mpsc::UnboundedSender<LspEvent>,
    client_key: String,
    server_id: String,
}

impl LspClient {
    pub async fn start(
        command: &str,
        args: &[String],
        root_uri: &str,
        server_id: &str,
        client_key: &str,
        event_tx: mpsc::UnboundedSender<LspEvent>,
    ) -> Result<Self, String> {
        log::info!(
            "LSP: starting server '{}' with args {:?} for server_id '{}', root_uri={}, key={}",
            command,
            args,
            server_id,
            root_uri,
            client_key
        );

        let mut child = TokioCommand::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start LSP server '{}': {}", command, e))?;

        let stdin = child.stdin.take().ok_or("Failed to get stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get stdout")?;
        let stderr = child.stderr.take();

        let (sender, receiver) = mpsc::unbounded_channel::<Vec<u8>>();
        let pending: PendingRequests = Arc::new(TokioMutex::new(HashMap::new()));
        let next_id = Arc::new(TokioMutex::new(1i64));

        tokio::spawn(Self::writer_task(stdin, receiver));

        let pending_clone = pending.clone();
        let event_tx_clone = event_tx.clone();
        let sender_clone = sender.clone();
        let client_key_reader = client_key.to_string();
        let server_id_reader = server_id.to_string();
        let root_uri_reader = root_uri.to_string();
        tokio::spawn(async move {
            Self::reader_task(
                stdout,
                pending_clone,
                sender_clone,
                event_tx_clone,
                &client_key_reader,
                &server_id_reader,
                &root_uri_reader,
            )
            .await;
        });

        if let Some(stderr) = stderr {
            let cmd_name = command.to_string();
            let key = client_key.to_string();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                log::warn!("LSP stderr [{}:{}]: {}", cmd_name, key, trimmed);
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        let event_tx_exit = event_tx.clone();
        let client_key_exit = client_key.to_string();
        let server_id_exit = server_id.to_string();
        let cmd_for_exit = command.to_string();
        let pending_exit = pending.clone();
        tokio::spawn(async move {
            let status = child.wait().await;
            log::warn!(
                "LSP server '{}' exited with status: {:?} (key={})",
                cmd_for_exit,
                status,
                client_key_exit
            );
            // Drain all pending requests so waiting callers get an error instead of hanging
            {
                let mut pending = pending_exit.lock().await;
                let count = pending.len();
                if count > 0 {
                    log::warn!(
                        "Draining {} pending LSP request(s) for crashed server '{}'",
                        count,
                        cmd_for_exit
                    );
                    for (_, tx) in pending.drain() {
                        let _ = tx.send(Err("LSP server exited unexpectedly".to_string()));
                    }
                }
            }
            let _ = event_tx_exit.send(LspEvent::ServerExited {
                client_key: client_key_exit,
                server_id: server_id_exit,
            });
        });

        let client = LspClient {
            sender,
            pending,
            next_id,
            capabilities: Arc::new(TokioMutex::new(None)),
            event_tx: event_tx.clone(),
            client_key: client_key.to_string(),
            server_id: server_id.to_string(),
        };

        client.initialize(root_uri).await?;
        log::info!(
            "LSP: server '{}' initialized successfully for key={}",
            command,
            client_key
        );

        Ok(client)
    }

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

    async fn reader_task(
        stdout: tokio::process::ChildStdout,
        pending: PendingRequests,
        sender: mpsc::UnboundedSender<Vec<u8>>,
        event_tx: mpsc::UnboundedSender<LspEvent>,
        client_key: &str,
        server_id: &str,
        root_uri: &str,
    ) {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut header_line = String::new();
            let mut content_length: usize = 0;
            loop {
                header_line.clear();
                match reader.read_line(&mut header_line).await {
                    Ok(0) => return,
                    Ok(_) => {
                        let trimmed = header_line.trim();
                        if trimmed.is_empty() {
                            break;
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

            // Reject absurdly large messages to prevent memory exhaustion (32 MB limit)
            const MAX_LSP_MESSAGE_SIZE: usize = 32 * 1024 * 1024;
            if content_length > MAX_LSP_MESSAGE_SIZE {
                log::warn!("LSP message too large ({} bytes), skipping", content_length);
                // Drain the oversized body to keep the stream in sync
                let mut remaining = content_length;
                let mut discard_buf = vec![0u8; 8192];
                while remaining > 0 {
                    let to_read = remaining.min(discard_buf.len());
                    match reader.read_exact(&mut discard_buf[..to_read]).await {
                        Ok(_) => remaining -= to_read,
                        Err(_) => return,
                    }
                }
                continue;
            }

            let mut body = vec![0u8; content_length];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }

            let msg: JsonRpcMessage = match serde_json::from_slice(&body) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("Failed to parse LSP message: {}", e);
                    continue;
                }
            };

            if let Some(id) = msg.id.clone() {
                if msg.method.is_none() {
                    if let Some(id_num) = id.as_i64() {
                        let mut pending = pending.lock().await;
                        if let Some(tx) = pending.remove(&id_num) {
                            if let Some(error) = msg.error {
                                let _ = tx.send(Err(format!(
                                    "LSP error {}: {}",
                                    error.code, error.message
                                )));
                            } else {
                                let _ = tx.send(Ok(msg.result.unwrap_or(serde_json::Value::Null)));
                            }
                        }
                    }
                    continue;
                }
            }

            if let Some(method) = &msg.method {
                if let Some(id) = msg.id {
                    Self::handle_server_request(method, id, msg.params, &sender, root_uri);
                    continue;
                }

                Self::handle_server_notification(
                    method, msg.params, &event_tx, client_key, server_id,
                );
            }
        }
    }

    fn handle_server_request(
        method: &str,
        id: serde_json::Value,
        params: Option<serde_json::Value>,
        sender: &mpsc::UnboundedSender<Vec<u8>>,
        root_uri: &str,
    ) {
        match method {
            "workspace/configuration" => {
                let count = params
                    .as_ref()
                    .and_then(|p| p.get("items"))
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or(0);
                let result = serde_json::Value::Array(vec![serde_json::Value::Null; count]);
                send_jsonrpc_result(sender, id, result);
            }
            "window/workDoneProgress/create" => {
                send_jsonrpc_result(sender, id, serde_json::Value::Null);
            }
            "workspace/workspaceFolders" => {
                let folder = serde_json::json!({
                    "uri": root_uri,
                    "name": workspace_folder_name(root_uri),
                });
                send_jsonrpc_result(sender, id, serde_json::Value::Array(vec![folder]));
            }
            "client/registerCapability" | "client/unregisterCapability" => {
                send_jsonrpc_result(sender, id, serde_json::Value::Null);
            }
            _ => {
                send_jsonrpc_error(sender, id, -32601, "Method not found");
            }
        }
    }

    fn handle_server_notification(
        method: &str,
        params: Option<serde_json::Value>,
        event_tx: &mpsc::UnboundedSender<LspEvent>,
        _client_key: &str,
        _server_id: &str,
    ) {
        match method {
            "textDocument/publishDiagnostics" => {
                if let Some(params) = params {
                    if let Ok(diag_params) =
                        serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params)
                    {
                        let _ = event_tx.send(LspEvent::Diagnostics {
                            uri: diag_params.uri.to_string(),
                            version: diag_params.version,
                            diagnostics: diag_params.diagnostics,
                        });
                    }
                }
            }
            "window/logMessage" | "window/showMessage" | "$/logTrace" | "$/progress" => {}
            _ => {
                log::debug!("Unhandled LSP notification: {}", method);
            }
        }
    }

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

        match tokio::time::timeout(Duration::from_secs(15), rx).await {
            Ok(result) => result.map_err(|_| "Request cancelled".to_string())?,
            Err(_) => {
                // Remove the pending request so the oneshot sender is dropped
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
                Err(format!("LSP request '{}' timed out after 15s", method))
            }
        }
    }

    pub fn notify<P: Serialize>(&self, method: &str, params: P) -> Result<(), String> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": serde_json::to_value(params).map_err(|e| e.to_string())?
        });
        let body = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
        self.sender.send(body).map_err(|e| e.to_string())
    }

    #[allow(deprecated)]
    async fn initialize(&self, root_uri: &str) -> Result<(), String> {
        let workspace_folders = Some(vec![lsp_types::WorkspaceFolder {
            uri: parse_uri(root_uri)?,
            name: workspace_folder_name(root_uri),
        }]);

        let params = lsp_types::InitializeParams {
            root_uri: Some(parse_uri(root_uri)?),
            workspace_folders,
            capabilities: lsp_types::ClientCapabilities {
                workspace: Some(lsp_types::WorkspaceClientCapabilities {
                    configuration: Some(true),
                    workspace_folders: Some(true),
                    ..Default::default()
                }),
                text_document: Some(lsp_types::TextDocumentClientCapabilities {
                    completion: Some(lsp_types::CompletionClientCapabilities {
                        completion_item: Some(lsp_types::CompletionItemCapability {
                            snippet_support: Some(true),
                            documentation_format: Some(vec![
                                lsp_types::MarkupKind::PlainText,
                                lsp_types::MarkupKind::Markdown,
                            ]),
                            insert_replace_support: Some(true),
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
                        version_support: Some(true),
                        ..Default::default()
                    }),
                    definition: Some(lsp_types::GotoCapability {
                        link_support: Some(true),
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

        if let Ok(init_result) = serde_json::from_value::<lsp_types::InitializeResult>(result) {
            *self.capabilities.lock().await = Some(init_result.capabilities);
        }

        self.notify("initialized", lsp_types::InitializedParams {})?;

        let _ = self.event_tx.send(LspEvent::Initialized {
            client_key: self.client_key.clone(),
            server_id: self.server_id.clone(),
        });

        Ok(())
    }

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
                    context: Some(lsp_types::CompletionContext {
                        trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                        trigger_character: None,
                    }),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            )
            .await?;

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

    pub async fn shutdown(&self) -> Result<(), String> {
        // Give the server 5 seconds to respond to shutdown
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            self.request("shutdown", serde_json::Value::Null),
        )
        .await;
        // Drain any remaining pending requests before sending exit
        {
            let mut pending = self.pending.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err("LSP server shutting down".to_string()));
            }
        }
        self.notify("exit", serde_json::Value::Null)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    pub servers: HashMap<String, LspServerConfig>,
    pub language_servers: HashMap<String, Vec<String>>,
    pub root_markers: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct LspConfigOverrides {
    servers: Option<HashMap<String, LspServerConfig>>,
    language_servers: Option<HashMap<String, Vec<String>>>,
    root_markers: Option<Vec<String>>,
}

impl LspConfig {
    pub fn load(fallback_root_uri: &str) -> Self {
        let mut cfg = Self::default();

        if let Some(global_path) = global_lsp_config_path() {
            cfg.apply_file(&global_path, /* trusted */ true);
        }

        if let Some(root_path) = uri_to_file_path(fallback_root_uri) {
            let project_config_paths = [
                root_path.join(".impulse").join("lsp.json"),
                root_path.join(".impulse-lsp.json"),
            ];
            for path in project_config_paths {
                // Project-local configs are untrusted: they cannot define new
                // server commands, only remap language->server associations and
                // root markers. This prevents malicious repos from executing
                // arbitrary binaries.
                cfg.apply_file(&path, /* trusted */ false);
            }
        }

        cfg
    }

    fn apply_file(&mut self, path: &Path, trusted: bool) {
        let contents = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return,
        };
        let overrides = match serde_json::from_str::<LspConfigOverrides>(&contents) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Invalid LSP config at {}: {}", path.display(), e);
                return;
            }
        };

        if let Some(servers) = overrides.servers {
            if trusted {
                // Global config may define arbitrary server commands
                self.servers.extend(servers);
            } else {
                // Project-local configs may only override args for servers
                // that already exist in the config, not add new commands.
                for (id, server_cfg) in servers {
                    if let Some(existing) = self.servers.get_mut(&id) {
                        // Allow overriding args but NOT the command itself
                        existing.args = server_cfg.args;
                    } else {
                        log::warn!(
                            "Project LSP config tried to add unknown server '{}' — ignoring (only global config can add servers)",
                            id
                        );
                    }
                }
            }
        }
        if let Some(language_servers) = overrides.language_servers {
            if trusted {
                self.language_servers.extend(language_servers);
            } else {
                // Only allow mapping to servers that already exist
                for (lang, server_ids) in language_servers {
                    let valid_ids: Vec<String> = server_ids
                        .into_iter()
                        .filter(|id| self.servers.contains_key(id))
                        .collect();
                    if !valid_ids.is_empty() {
                        self.language_servers.insert(lang, valid_ids);
                    }
                }
            }
        }
        if let Some(root_markers) = overrides.root_markers {
            if !root_markers.is_empty() {
                self.root_markers = root_markers;
            }
        }
    }
}

fn global_lsp_config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("impulse").join("lsp.json"));
    }

    std::env::var("HOME").ok().map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("impulse")
            .join("lsp.json")
    })
}

impl Default for LspConfig {
    fn default() -> Self {
        let mut servers = HashMap::new();
        servers.insert(
            "rust-analyzer".into(),
            LspServerConfig {
                command: "rust-analyzer".into(),
                args: vec![],
            },
        );
        servers.insert(
            "pyright".into(),
            LspServerConfig {
                command: "pyright-langserver".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "clangd".into(),
            LspServerConfig {
                command: "clangd".into(),
                args: vec![],
            },
        );
        servers.insert(
            "typescript-language-server".into(),
            LspServerConfig {
                command: "typescript-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "intelephense".into(),
            LspServerConfig {
                command: "intelephense".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "vscode-html-language-server".into(),
            LspServerConfig {
                command: "vscode-html-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "vscode-css-language-server".into(),
            LspServerConfig {
                command: "vscode-css-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "vscode-json-language-server".into(),
            LspServerConfig {
                command: "vscode-json-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "vscode-eslint-language-server".into(),
            LspServerConfig {
                command: "vscode-eslint-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "tailwindcss-language-server".into(),
            LspServerConfig {
                command: "tailwindcss-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "vue-language-server".into(),
            LspServerConfig {
                command: "vue-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "svelteserver".into(),
            LspServerConfig {
                command: "svelteserver".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "graphql-lsp".into(),
            LspServerConfig {
                command: "graphql-lsp".into(),
                args: vec!["server".into(), "-m".into(), "stream".into()],
            },
        );
        servers.insert(
            "emmet-ls".into(),
            LspServerConfig {
                command: "emmet-ls".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "yaml-language-server".into(),
            LspServerConfig {
                command: "yaml-language-server".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "docker-langserver".into(),
            LspServerConfig {
                command: "docker-langserver".into(),
                args: vec!["--stdio".into()],
            },
        );
        servers.insert(
            "bash-language-server".into(),
            LspServerConfig {
                command: "bash-language-server".into(),
                args: vec!["start".into()],
            },
        );

        let mut language_servers = HashMap::new();
        language_servers.insert("rust".into(), vec!["rust-analyzer".into()]);
        language_servers.insert("python".into(), vec!["pyright".into()]);
        language_servers.insert("c".into(), vec!["clangd".into()]);
        language_servers.insert("cpp".into(), vec!["clangd".into()]);
        language_servers.insert(
            "javascript".into(),
            vec![
                "typescript-language-server".into(),
                "vscode-eslint-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert(
            "javascriptreact".into(),
            vec![
                "typescript-language-server".into(),
                "vscode-eslint-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert(
            "typescript".into(),
            vec![
                "typescript-language-server".into(),
                "vscode-eslint-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert(
            "typescriptreact".into(),
            vec![
                "typescript-language-server".into(),
                "vscode-eslint-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert("php".into(), vec!["intelephense".into()]);
        language_servers.insert(
            "html".into(),
            vec![
                "vscode-html-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert(
            "css".into(),
            vec![
                "vscode-css-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert(
            "scss".into(),
            vec![
                "vscode-css-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert(
            "less".into(),
            vec![
                "vscode-css-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert("json".into(), vec!["vscode-json-language-server".into()]);
        language_servers.insert("jsonc".into(), vec!["vscode-json-language-server".into()]);
        language_servers.insert("yaml".into(), vec!["yaml-language-server".into()]);
        language_servers.insert(
            "vue".into(),
            vec![
                "vue-language-server".into(),
                "vscode-eslint-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert(
            "svelte".into(),
            vec![
                "svelteserver".into(),
                "vscode-eslint-language-server".into(),
                "tailwindcss-language-server".into(),
                "emmet-ls".into(),
            ],
        );
        language_servers.insert("graphql".into(), vec!["graphql-lsp".into()]);
        language_servers.insert("dockerfile".into(), vec!["docker-langserver".into()]);
        language_servers.insert("shellscript".into(), vec!["bash-language-server".into()]);

        let root_markers = vec![
            "Cargo.toml".to_string(),
            "package.json".to_string(),
            "tsconfig.json".to_string(),
            "jsconfig.json".to_string(),
            "pnpm-workspace.yaml".to_string(),
            "yarn.lock".to_string(),
            "package-lock.json".to_string(),
            "bun.lockb".to_string(),
            "turbo.json".to_string(),
            "nx.json".to_string(),
            "go.mod".to_string(),
            "pyproject.toml".to_string(),
            "setup.py".to_string(),
            "composer.json".to_string(),
            "Gemfile".to_string(),
            "deno.json".to_string(),
            "deno.jsonc".to_string(),
        ];

        LspConfig {
            servers,
            language_servers,
            root_markers,
        }
    }
}

pub struct LspRegistry {
    clients: Arc<TokioMutex<HashMap<String, Arc<LspClient>>>>,
    failed_until: Arc<TokioMutex<HashMap<String, Instant>>>,
    starting: Arc<TokioMutex<HashSet<String>>>,
    config: LspConfig,
    fallback_root_uri: String,
    event_tx: mpsc::UnboundedSender<LspEvent>,
}

fn detect_project_root(file_uri: &str, markers: &[String]) -> Option<String> {
    let path = uri_to_file_path(file_uri)?;
    let mut dir = path.parent()?;
    let mut best: Option<String> = None;

    loop {
        if dir.join(".git").exists() {
            return path_to_file_uri(dir);
        }

        if best.is_none() {
            for marker in markers {
                if dir.join(marker).exists() {
                    best = path_to_file_uri(dir);
                    break;
                }
            }
        }

        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => break,
        }
    }

    best
}

impl LspRegistry {
    pub fn new(root_uri: String, event_tx: mpsc::UnboundedSender<LspEvent>) -> Self {
        let config = LspConfig::load(&root_uri);
        Self {
            clients: Arc::new(TokioMutex::new(HashMap::new())),
            failed_until: Arc::new(TokioMutex::new(HashMap::new())),
            starting: Arc::new(TokioMutex::new(HashSet::new())),
            config,
            fallback_root_uri: root_uri,
            event_tx,
        }
    }

    fn resolve_server_ids(&self, language_id: &str) -> Vec<String> {
        if let Some(ids) = self.config.language_servers.get(language_id) {
            return ids.clone();
        }

        if self.config.servers.contains_key(language_id) {
            return vec![language_id.to_string()];
        }

        Vec::new()
    }

    fn detect_root_uri(&self, file_uri: &str) -> String {
        detect_project_root(file_uri, &self.config.root_markers)
            .unwrap_or_else(|| self.fallback_root_uri.clone())
    }

    fn client_key(server_id: &str, root_uri: &str) -> String {
        format!("{}@{}", server_id, root_uri)
    }

    async fn get_or_start_client(&self, server_id: &str, root_uri: &str) -> Option<Arc<LspClient>> {
        let client_key = Self::client_key(server_id, root_uri);

        loop {
            {
                let clients = self.clients.lock().await;
                if let Some(client) = clients.get(&client_key) {
                    return Some(client.clone());
                }
            }

            {
                let mut failed = self.failed_until.lock().await;
                if let Some(until) = failed.get(&client_key).copied() {
                    if Instant::now() < until {
                        return None;
                    }
                    failed.remove(&client_key);
                }
            }

            {
                let starting = self.starting.lock().await;
                if starting.contains(&client_key) {
                    drop(starting);
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    continue;
                }
            }

            self.starting.lock().await.insert(client_key.clone());
            break;
        }

        let result = self.start_server(server_id, root_uri).await;
        self.starting.lock().await.remove(&client_key);
        result
    }

    async fn start_server(&self, server_id: &str, root_uri: &str) -> Option<Arc<LspClient>> {
        let server_config = match self.config.servers.get(server_id) {
            Some(cfg) => cfg,
            None => {
                log::info!("No LSP server configured for server id: {}", server_id);
                return None;
            }
        };

        let client_key = Self::client_key(server_id, root_uri);
        let resolved_command = match resolve_lsp_command_path(&server_config.command) {
            Some(path) => path,
            None => {
                let message = missing_command_message(server_id, &server_config.command);
                log::warn!("LSP startup skipped for {}: {}", server_id, message);
                self.failed_until
                    .lock()
                    .await
                    .insert(client_key.clone(), Instant::now() + START_RETRY_COOLDOWN);
                let _ = self.event_tx.send(LspEvent::ServerError {
                    client_key,
                    server_id: server_id.to_string(),
                    message,
                });
                return None;
            }
        };
        let resolved_command = resolved_command.to_string_lossy().to_string();

        match LspClient::start(
            &resolved_command,
            &server_config.args,
            root_uri,
            server_id,
            &client_key,
            self.event_tx.clone(),
        )
        .await
        {
            Ok(client) => {
                let client = Arc::new(client);
                self.clients
                    .lock()
                    .await
                    .insert(client_key.clone(), client.clone());
                Some(client)
            }
            Err(e) => {
                log::warn!(
                    "Failed to start LSP server for '{}' (key='{}'): {}",
                    server_id,
                    client_key,
                    e
                );
                self.failed_until
                    .lock()
                    .await
                    .insert(client_key.clone(), Instant::now() + START_RETRY_COOLDOWN);
                let _ = self.event_tx.send(LspEvent::ServerError {
                    client_key,
                    server_id: server_id.to_string(),
                    message: e,
                });
                None
            }
        }
    }

    pub async fn get_clients(&self, language_id: &str, file_uri: &str) -> Vec<Arc<LspClient>> {
        let server_ids = self.resolve_server_ids(language_id);
        if server_ids.is_empty() {
            log::debug!("No LSP servers configured for language: {}", language_id);
            return Vec::new();
        }

        let root_uri = self.detect_root_uri(file_uri);
        let mut out = Vec::new();
        for server_id in server_ids {
            if let Some(client) = self.get_or_start_client(&server_id, &root_uri).await {
                out.push(client);
            }
        }
        out
    }

    pub async fn remove_client(&self, client_key: &str) {
        let mut clients = self.clients.lock().await;
        clients.remove(client_key);
    }

    pub async fn shutdown_all(&self) {
        let clients: Vec<Arc<LspClient>> = {
            let clients = self.clients.lock().await;
            clients.values().cloned().collect()
        };

        for client in clients {
            let _ = client.shutdown().await;
        }
    }
}
