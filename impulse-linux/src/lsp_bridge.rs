// SPDX-License-Identifier: GPL-3.0-only
//
// LSP management bridge QObject for QML. Uses a tokio runtime internally
// for async LSP operations.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, root_uri)]
        #[qproperty(bool, is_initialized)]
        #[qproperty(QString, pending_event_json)]
        type LspBridge = super::LspBridgeRust;

        #[qinvokable]
        fn initialize(self: Pin<&mut LspBridge>, root_uri: &QString);

        #[qinvokable]
        fn ensure_servers(self: Pin<&mut LspBridge>, language_id: &QString, file_uri: &QString)
            -> i32;

        #[qinvokable]
        fn request(
            self: Pin<&mut LspBridge>,
            language_id: &QString,
            file_uri: &QString,
            method: &QString,
            params_json: &QString,
        ) -> QString;

        #[qinvokable]
        fn notify(
            self: Pin<&mut LspBridge>,
            language_id: &QString,
            file_uri: &QString,
            method: &QString,
            params_json: &QString,
        );

        #[qinvokable]
        fn poll_event(self: Pin<&mut LspBridge>) -> QString;

        #[qinvokable]
        fn shutdown(self: Pin<&mut LspBridge>);

        #[qinvokable]
        fn check_server_status(self: &LspBridge) -> QString;

        #[qinvokable]
        fn install_servers(self: &LspBridge) -> QString;

        #[qsignal]
        fn diagnostics_received(
            self: Pin<&mut LspBridge>,
            uri: QString,
            diagnostics_json: QString,
        );

        #[qsignal]
        fn server_initialized(self: Pin<&mut LspBridge>, server_id: QString);

        #[qsignal]
        fn server_error(self: Pin<&mut LspBridge>, message: QString);
    }
}

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use std::pin::Pin;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

/// Maximum number of LSP events buffered in the channel.
const LSP_EVENT_CHANNEL_CAPACITY: usize = 10_000;

pub struct LspBridgeRust {
    root_uri: QString,
    is_initialized: bool,
    pending_event_json: QString,
    // Internal state
    runtime: Option<Arc<Runtime>>,
    registry: Option<Arc<impulse_core::lsp::LspRegistry>>,
    event_rx: Option<mpsc::Receiver<impulse_core::lsp::LspEvent>>,
}

impl Default for LspBridgeRust {
    fn default() -> Self {
        Self {
            root_uri: QString::default(),
            is_initialized: false,
            pending_event_json: QString::default(),
            runtime: None,
            registry: None,
            event_rx: None,
        }
    }
}

impl qobject::LspBridge {
    pub fn initialize(mut self: Pin<&mut Self>, root_uri: &QString) {
        let uri_str = root_uri.to_string();
        if uri_str.is_empty() {
            log::warn!("LspBridge::initialize called with empty root_uri");
            return;
        }

        // Create tokio runtime
        let runtime = match Runtime::new() {
            Ok(rt) => Arc::new(rt),
            Err(e) => {
                log::error!("Failed to create Tokio runtime for LSP: {}", e);
                let msg = QString::from(format!("Failed to create runtime: {}", e).as_str());
                self.as_mut().server_error(msg);
                return;
            }
        };

        // Create event channels
        let (event_tx, mut unbounded_rx) = mpsc::unbounded_channel();
        let registry = Arc::new(impulse_core::lsp::LspRegistry::new(
            uri_str.clone(),
            event_tx,
        ));

        // Bridge unbounded -> bounded channel
        let (bounded_tx, bounded_rx) = mpsc::channel(LSP_EVENT_CHANNEL_CAPACITY);
        runtime.spawn(async move {
            while let Some(event) = unbounded_rx.recv().await {
                match bounded_tx.try_send(event) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        log::warn!(
                            "LSP event channel full ({} capacity), dropping event",
                            LSP_EVENT_CHANNEL_CAPACITY
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                }
            }
        });

        self.as_mut().set_root_uri(root_uri.clone());
        self.as_mut().rust_mut().runtime = Some(runtime);
        self.as_mut().rust_mut().registry = Some(registry);
        self.as_mut().rust_mut().event_rx = Some(bounded_rx);
        self.as_mut().set_is_initialized(true);
    }

    pub fn ensure_servers(
        self: Pin<&mut Self>,
        language_id: &QString,
        file_uri: &QString,
    ) -> i32 {
        let lang = language_id.to_string();
        let uri = file_uri.to_string();

        let (runtime, registry) = match (self.rust().runtime.as_ref(), self.rust().registry.as_ref())
        {
            (Some(rt), Some(reg)) => (rt.clone(), reg.clone()),
            _ => {
                log::warn!("LspBridge::ensure_servers called before initialize");
                return -1;
            }
        };

        runtime.block_on(async {
            let clients = registry.get_clients(&lang, &uri).await;
            clients.len() as i32
        })
    }

    pub fn request(
        self: Pin<&mut Self>,
        language_id: &QString,
        file_uri: &QString,
        method: &QString,
        params_json: &QString,
    ) -> QString {
        let lang = language_id.to_string();
        let uri = file_uri.to_string();
        let method_str = method.to_string();
        let params_str = params_json.to_string();

        let params: Option<serde_json::Value> = if params_str.is_empty() {
            None
        } else {
            serde_json::from_str(&params_str).ok()
        };

        let (runtime, registry) = match (self.rust().runtime.as_ref(), self.rust().registry.as_ref())
        {
            (Some(rt), Some(reg)) => (rt.clone(), reg.clone()),
            _ => return QString::from("{\"error\":\"not initialized\"}"),
        };

        runtime.block_on(async {
            let clients = registry.get_clients(&lang, &uri).await;
            if let Some(client) = clients.first() {
                match client.request(&method_str, params).await {
                    Ok(value) => {
                        let json =
                            serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
                        QString::from(json.as_str())
                    }
                    Err(e) => {
                        let json = serde_json::json!({"error": e});
                        QString::from(json.to_string().as_str())
                    }
                }
            } else {
                QString::from("{\"error\":\"no LSP client available\"}")
            }
        })
    }

    pub fn notify(
        self: Pin<&mut Self>,
        language_id: &QString,
        file_uri: &QString,
        method: &QString,
        params_json: &QString,
    ) {
        let lang = language_id.to_string();
        let uri = file_uri.to_string();
        let method_str = method.to_string();
        let params_str = params_json.to_string();

        let params: Option<serde_json::Value> = if params_str.is_empty() {
            None
        } else {
            serde_json::from_str(&params_str).ok()
        };

        let (runtime, registry) = match (self.rust().runtime.as_ref(), self.rust().registry.as_ref())
        {
            (Some(rt), Some(reg)) => (rt.clone(), reg.clone()),
            _ => {
                log::warn!("LspBridge::notify called before initialize");
                return;
            }
        };

        runtime.block_on(async {
            let clients = registry.get_clients(&lang, &uri).await;
            if let Some(client) = clients.first() {
                if let Err(e) = client.notify(&method_str, params) {
                    log::warn!("LSP notify '{}' failed: {}", method_str, e);
                }
            }
        });
    }

    pub fn poll_event(mut self: Pin<&mut Self>) -> QString {
        // Extract the event first to avoid holding a mutable borrow on self
        // while also needing to emit signals.
        let mut rust = self.as_mut().rust_mut();
        let event_rx = match rust.event_rx.as_mut() {
            Some(rx) => rx,
            None => return QString::default(),
        };
        let event = match event_rx.try_recv() {
            Ok(ev) => ev,
            Err(mpsc::error::TryRecvError::Empty) => return QString::default(),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                log::warn!("LSP event channel disconnected");
                return QString::default();
            }
        };
        drop(rust);

        // Now process the event without holding a borrow on event_rx
        let json = match &event {
            impulse_core::lsp::LspEvent::Diagnostics {
                uri,
                version,
                diagnostics,
            } => {
                let diag_json: Vec<serde_json::Value> = diagnostics
                    .iter()
                    .map(|d| {
                        serde_json::json!({
                            "severity": d.severity.map(|s| match s {
                                lsp_types::DiagnosticSeverity::ERROR => 1u8,
                                lsp_types::DiagnosticSeverity::WARNING => 2,
                                lsp_types::DiagnosticSeverity::INFORMATION => 3,
                                lsp_types::DiagnosticSeverity::HINT => 4,
                                _ => 1,
                            }).unwrap_or(1),
                            "message": d.message,
                            "range": {
                                "start": {
                                    "line": d.range.start.line,
                                    "character": d.range.start.character,
                                },
                                "end": {
                                    "line": d.range.end.line,
                                    "character": d.range.end.character,
                                }
                            },
                            "source": d.source,
                        })
                    })
                    .collect();

                let uri_qs = QString::from(uri.as_str());
                let diag_str =
                    serde_json::to_string(&diag_json).unwrap_or_else(|_| "[]".to_string());
                let diag_qs = QString::from(diag_str.as_str());
                self.as_mut().diagnostics_received(uri_qs, diag_qs);

                serde_json::json!({
                    "type": "Diagnostics",
                    "uri": uri,
                    "version": version,
                    "diagnostics": diag_json,
                })
            }
            impulse_core::lsp::LspEvent::Initialized {
                client_key,
                server_id,
            } => {
                let sid = QString::from(server_id.as_str());
                self.as_mut().server_initialized(sid);

                serde_json::json!({
                    "type": "Initialized",
                    "client_key": client_key,
                    "server_id": server_id,
                })
            }
            impulse_core::lsp::LspEvent::ServerError {
                client_key,
                server_id,
                message,
            } => {
                let msg = QString::from(message.as_str());
                self.as_mut().server_error(msg);

                serde_json::json!({
                    "type": "ServerError",
                    "client_key": client_key,
                    "server_id": server_id,
                    "message": message,
                })
            }
            impulse_core::lsp::LspEvent::ServerExited {
                client_key,
                server_id,
            } => {
                serde_json::json!({
                    "type": "ServerExited",
                    "client_key": client_key,
                    "server_id": server_id,
                })
            }
        };

        let result = json.to_string();
        self.as_mut()
            .set_pending_event_json(QString::from(result.as_str()));
        QString::from(result.as_str())
    }

    pub fn shutdown(mut self: Pin<&mut Self>) {
        if let (Some(runtime), Some(registry)) = (
            self.as_ref().rust().runtime.clone(),
            self.as_ref().rust().registry.clone(),
        ) {
            runtime.block_on(async {
                registry.shutdown_all().await;
            });
        }

        self.as_mut().rust_mut().registry = None;
        self.as_mut().rust_mut().runtime = None;
        self.as_mut().rust_mut().event_rx = None;
        self.as_mut().set_is_initialized(false);
    }

    pub fn check_server_status(&self) -> QString {
        let mut statuses = Vec::new();

        // Managed web LSP servers
        for status in impulse_core::lsp::managed_web_lsp_status() {
            statuses.push(serde_json::json!({
                "command": status.command,
                "type": "managed",
                "installed": status.resolved_path.is_some(),
                "path": status.resolved_path.map(|p| p.to_string_lossy().to_string()),
            }));
        }

        // System LSP servers
        for status in impulse_core::lsp::system_lsp_status() {
            statuses.push(serde_json::json!({
                "command": status.command,
                "type": "system",
                "installed": status.resolved_path.is_some(),
                "path": status.resolved_path.map(|p| p.to_string_lossy().to_string()),
            }));
        }

        let json = serde_json::to_string(&statuses).unwrap_or_else(|_| "[]".to_string());
        QString::from(json.as_str())
    }

    pub fn install_servers(&self) -> QString {
        match impulse_core::lsp::install_managed_web_lsp_servers() {
            Ok(path) => {
                let result = serde_json::json!({
                    "success": true,
                    "path": path.to_string_lossy().to_string(),
                });
                QString::from(result.to_string().as_str())
            }
            Err(e) => {
                let result = serde_json::json!({
                    "success": false,
                    "error": e,
                });
                QString::from(result.to_string().as_str())
            }
        }
    }
}
