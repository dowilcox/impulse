//! C-compatible FFI wrappers around impulse-core and impulse-editor.
//!
//! All functions use C strings for input/output and JSON encoding for
//! complex types. Callers must free returned strings with `impulse_free_string`.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_rust_str(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok().map(String::from)
}

fn to_c_string(s: &str) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

// ---------------------------------------------------------------------------
// Memory management
// ---------------------------------------------------------------------------

/// Free a string previously returned by an `impulse_*` function.
#[no_mangle]
pub extern "C" fn impulse_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

// ---------------------------------------------------------------------------
// Monaco assets
// ---------------------------------------------------------------------------

/// Ensure Monaco editor files are extracted to the platform data directory.
///
/// Returns the extraction path on success or an error string on failure.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_ensure_monaco_extracted() -> *mut c_char {
    match impulse_editor::assets::ensure_monaco_extracted() {
        Ok(path) => to_c_string(&path.to_string_lossy()),
        Err(e) => to_c_string(&format!("ERROR:{}", e)),
    }
}

/// Return the embedded editor HTML content as a static string.
///
/// The returned pointer is valid for the lifetime of the process and must
/// NOT be freed.
#[no_mangle]
pub extern "C" fn impulse_get_editor_html() -> *const c_char {
    // Leak a CString once and return the same pointer on every call.
    static mut CACHED: *const c_char = std::ptr::null();
    unsafe {
        if CACHED.is_null() {
            let cs = CString::new(impulse_editor::assets::EDITOR_HTML).unwrap_or_default();
            CACHED = cs.into_raw();
        }
        CACHED
    }
}

// ---------------------------------------------------------------------------
// Shell integration
// ---------------------------------------------------------------------------

/// Return the shell integration script for the given shell name.
///
/// `shell` must be one of `"bash"`, `"zsh"`, or `"fish"`.
/// Returns null on invalid input.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_get_shell_integration_script(shell: *const c_char) -> *mut c_char {
    let shell_name = match to_rust_str(shell) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let shell_type = impulse_core::shell::detect_shell_type(&shell_name);
    let script = impulse_core::shell::get_integration_script(&shell_type);
    to_c_string(script)
}

/// Return the user's login shell path.
///
/// Falls back to `$SHELL`, then `/bin/bash`.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_get_user_login_shell() -> *mut c_char {
    to_c_string(&impulse_core::shell::get_default_shell_path())
}

/// Return the user's login shell name (e.g. "fish", "zsh", "bash").
///
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_get_user_login_shell_name() -> *mut c_char {
    to_c_string(&impulse_core::shell::get_default_shell_name())
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Search for files by name in `root` matching `query`.
///
/// Returns a JSON array of `SearchResult` objects.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_search_files(
    root: *const c_char,
    query: *const c_char,
) -> *mut c_char {
    let root = match to_rust_str(root) {
        Some(s) => s,
        None => return to_c_string("[]"),
    };
    let query = match to_rust_str(query) {
        Some(s) => s,
        None => return to_c_string("[]"),
    };

    match impulse_core::search::search_filenames(&root, &query, 200) {
        Ok(results) => {
            let json = serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string());
            to_c_string(&json)
        }
        Err(e) => to_c_string(&format!("[]{{\"error\":\"{}\"}}", e)),
    }
}

/// Search file contents in `root` for `query`.
///
/// Returns a JSON array of `SearchResult` objects.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_search_content(
    root: *const c_char,
    query: *const c_char,
    case_sensitive: bool,
) -> *mut c_char {
    let root = match to_rust_str(root) {
        Some(s) => s,
        None => return to_c_string("[]"),
    };
    let query = match to_rust_str(query) {
        Some(s) => s,
        None => return to_c_string("[]"),
    };

    match impulse_core::search::search_contents(&root, &query, 500, case_sensitive) {
        Ok(results) => {
            let json = serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string());
            to_c_string(&json)
        }
        Err(_) => to_c_string("[]"),
    }
}

// ---------------------------------------------------------------------------
// LSP management
// ---------------------------------------------------------------------------

/// Opaque handle wrapping an `LspRegistry` plus the tokio runtime it runs on.
pub struct LspRegistryHandle {
    registry: Arc<impulse_core::lsp::LspRegistry>,
    runtime: Arc<Runtime>,
    event_rx: std::sync::Mutex<mpsc::UnboundedReceiver<impulse_core::lsp::LspEvent>>,
}

/// Create a new LSP registry for the given workspace root URI.
///
/// Returns an opaque handle. The caller must free it with
/// `impulse_lsp_registry_free`.
#[no_mangle]
pub extern "C" fn impulse_lsp_registry_new(
    root_uri: *const c_char,
) -> *mut LspRegistryHandle {
    let root_uri = match to_rust_str(root_uri) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let runtime = match Runtime::new() {
        Ok(rt) => Arc::new(rt),
        Err(_) => return std::ptr::null_mut(),
    };

    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let registry = Arc::new(impulse_core::lsp::LspRegistry::new(root_uri, event_tx));

    Box::into_raw(Box::new(LspRegistryHandle {
        registry,
        runtime,
        event_rx: std::sync::Mutex::new(event_rx),
    }))
}

/// Ensure LSP servers are running for the given language and file.
///
/// `language_id` is the LSP language identifier (e.g. "typescript").
/// `file_uri` is the file URI (e.g. "file:///path/to/file.ts").
///
/// Returns the number of clients started/found, or -1 on error.
#[no_mangle]
pub extern "C" fn impulse_lsp_ensure_servers(
    handle: *mut LspRegistryHandle,
    language_id: *const c_char,
    file_uri: *const c_char,
) -> i32 {
    if handle.is_null() {
        return -1;
    }
    let handle = unsafe { &*handle };

    let language_id = match to_rust_str(language_id) {
        Some(s) => s,
        None => return -1,
    };
    let file_uri = match to_rust_str(file_uri) {
        Some(s) => s,
        None => return -1,
    };

    handle.runtime.block_on(async {
        let clients = handle
            .registry
            .get_clients(&language_id, &file_uri)
            .await;
        clients.len() as i32
    })
}

/// Send a JSON-RPC request to the first LSP server for the given language.
///
/// `method` is the LSP method name (e.g. "textDocument/completion").
/// `params_json` is the JSON-encoded params (or null for no params).
///
/// Returns a JSON string with the result or error. The caller must free it.
#[no_mangle]
pub extern "C" fn impulse_lsp_request(
    handle: *mut LspRegistryHandle,
    language_id: *const c_char,
    file_uri: *const c_char,
    method: *const c_char,
    params_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return to_c_string("{\"error\":\"null handle\"}");
    }
    let handle = unsafe { &*handle };

    let language_id = match to_rust_str(language_id) {
        Some(s) => s,
        None => return to_c_string("{\"error\":\"invalid language_id\"}"),
    };
    let file_uri = match to_rust_str(file_uri) {
        Some(s) => s,
        None => return to_c_string("{\"error\":\"invalid file_uri\"}"),
    };
    let method = match to_rust_str(method) {
        Some(s) => s,
        None => return to_c_string("{\"error\":\"invalid method\"}"),
    };
    let params: Option<serde_json::Value> = to_rust_str(params_json)
        .and_then(|s| serde_json::from_str(&s).ok());

    handle.runtime.block_on(async {
        let clients = handle.registry.get_clients(&language_id, &file_uri).await;
        if let Some(client) = clients.first() {
            match client.request(&method, params).await {
                Ok(value) => {
                    let json = serde_json::to_string(&value)
                        .unwrap_or_else(|_| "null".to_string());
                    to_c_string(&json)
                }
                Err(e) => to_c_string(&format!("{{\"error\":\"{}\"}}", e.replace('"', "\\\""))),
            }
        } else {
            to_c_string("{\"error\":\"no LSP client available\"}")
        }
    })
}

/// Send an LSP notification (no response expected).
///
/// `method` is the LSP method name (e.g. "textDocument/didOpen").
/// `params_json` is the JSON-encoded params.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn impulse_lsp_notify(
    handle: *mut LspRegistryHandle,
    language_id: *const c_char,
    file_uri: *const c_char,
    method: *const c_char,
    params_json: *const c_char,
) -> i32 {
    if handle.is_null() {
        return -1;
    }
    let handle = unsafe { &*handle };

    let language_id = match to_rust_str(language_id) {
        Some(s) => s,
        None => return -1,
    };
    let file_uri = match to_rust_str(file_uri) {
        Some(s) => s,
        None => return -1,
    };
    let method = match to_rust_str(method) {
        Some(s) => s,
        None => return -1,
    };
    let params: serde_json::Value = to_rust_str(params_json)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null);

    handle.runtime.block_on(async {
        let clients = handle.registry.get_clients(&language_id, &file_uri).await;
        if let Some(client) = clients.first() {
            match client.notify(&method, params) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        } else {
            -1
        }
    })
}

/// Poll for LSP events (diagnostics, server lifecycle).
///
/// Returns a JSON string describing the event, or null if no events are pending.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_lsp_poll_event(handle: *mut LspRegistryHandle) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let handle = unsafe { &*handle };

    let mut rx = match handle.event_rx.lock() {
        Ok(rx) => rx,
        Err(_) => return std::ptr::null_mut(),
    };

    match rx.try_recv() {
        Ok(event) => {
            let json = match event {
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
                                "startLine": d.range.start.line,
                                "startColumn": d.range.start.character,
                                "endLine": d.range.end.line,
                                "endColumn": d.range.end.character,
                                "message": d.message,
                                "source": d.source,
                            })
                        })
                        .collect();
                    serde_json::json!({
                        "type": "diagnostics",
                        "uri": uri,
                        "version": version,
                        "diagnostics": diag_json,
                    })
                }
                impulse_core::lsp::LspEvent::Initialized {
                    client_key,
                    server_id,
                } => {
                    serde_json::json!({
                        "type": "initialized",
                        "clientKey": client_key,
                        "serverId": server_id,
                    })
                }
                impulse_core::lsp::LspEvent::ServerError {
                    client_key,
                    server_id,
                    message,
                } => {
                    serde_json::json!({
                        "type": "serverError",
                        "clientKey": client_key,
                        "serverId": server_id,
                        "message": message,
                    })
                }
                impulse_core::lsp::LspEvent::ServerExited {
                    client_key,
                    server_id,
                } => {
                    serde_json::json!({
                        "type": "serverExited",
                        "clientKey": client_key,
                        "serverId": server_id,
                    })
                }
            };
            to_c_string(&json.to_string())
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Shut down all LSP servers managed by this registry.
#[no_mangle]
pub extern "C" fn impulse_lsp_shutdown_all(handle: *mut LspRegistryHandle) {
    if handle.is_null() {
        return;
    }
    let handle = unsafe { &*handle };
    handle.runtime.block_on(async {
        handle.registry.shutdown_all().await;
    });
}

/// Free an LSP registry handle. Shuts down all servers first.
#[no_mangle]
pub extern "C" fn impulse_lsp_registry_free(handle: *mut LspRegistryHandle) {
    if !handle.is_null() {
        let handle = unsafe { Box::from_raw(handle) };
        handle.runtime.block_on(async {
            handle.registry.shutdown_all().await;
        });
    }
}

// ---------------------------------------------------------------------------
// Managed LSP server installation
// ---------------------------------------------------------------------------

/// Check the installation status of managed web LSP servers.
///
/// Returns a JSON array of objects with `command` and `installed` fields.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_lsp_check_status() -> *mut c_char {
    let statuses = impulse_core::lsp::managed_web_lsp_status();
    let json: Vec<serde_json::Value> = statuses
        .iter()
        .map(|s| {
            serde_json::json!({
                "command": s.command,
                "installed": s.resolved_path.is_some(),
                "resolvedPath": s.resolved_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            })
        })
        .collect();
    to_c_string(&serde_json::to_string(&json).unwrap_or_else(|_| "[]".to_string()))
}

/// Install managed web LSP servers.
///
/// Returns the installation root path on success, or an error string prefixed
/// with "ERROR:" on failure.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_lsp_install() -> *mut c_char {
    match impulse_core::lsp::install_managed_web_lsp_servers() {
        Ok(path) => to_c_string(&path.to_string_lossy()),
        Err(e) => to_c_string(&format!("ERROR:{}", e)),
    }
}
