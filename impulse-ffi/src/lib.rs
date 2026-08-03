//! C-compatible FFI wrappers around impulse-core and impulse-editor.
//!
//! All functions use C strings for input/output and JSON encoding for
//! complex types. Callers must free returned strings with `impulse_free_string`.
//!
//! All extern "C" functions are wrapped in `ffi_catch` to prevent Rust
//! panics from crossing the FFI boundary (which is undefined behavior).
//! Panic payloads are logged before returning the fallback value.
//!
//! Note: `extern "C"` functions cannot be marked `unsafe` since they are
//! called from C/Swift. Raw pointer dereferences inside `ffi_catch` are
//! guarded by null checks.
#![allow(clippy::not_unsafe_ptr_arg_deref)]
#![allow(private_interfaces)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

/// Run `f` inside `catch_unwind`, logging the panic payload before returning the
/// fallback value.
fn ffi_catch<T>(fallback: T, f: impl FnOnce() -> T + std::panic::UnwindSafe) -> T {
    match catch_unwind(f) {
        Ok(v) => v,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic payload".to_string()
            };
            log::error!("FFI panic caught: {}", msg);
            fallback
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_rust_str(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: Caller guarantees `ptr` is a valid, null-terminated C string
    // whose memory remains valid for the duration of this call.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(String::from)
}

fn to_c_string(s: &str) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => {
            log::warn!(
                "String contains interior NUL bytes, sanitizing ({} chars)",
                s.len()
            );
            let sanitized: String = s.chars().filter(|&c| c != '\0').collect();
            CString::new(sanitized).unwrap_or_default().into_raw()
        }
    }
}

// ---------------------------------------------------------------------------
// Memory management
// ---------------------------------------------------------------------------

/// Free a string previously returned by an `impulse_*` function.
#[no_mangle]
pub extern "C" fn impulse_free_string(s: *mut c_char) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if !s.is_null() {
                // SAFETY: `s` was previously returned by `CString::into_raw` from
                // one of the `impulse_*` functions, so it is valid to reclaim it.
                unsafe {
                    drop(CString::from_raw(s));
                }
            }
        }),
    );
}

// ---------------------------------------------------------------------------
// LSP management
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::OnceLock;

/// Maximum number of LSP events buffered in the bounded forwarding channel.
const LSP_EVENT_CHANNEL_CAPACITY: usize = 10_000;

/// Inner data for an LSP registry handle, stored in the global registry.
struct LspRegistryInner {
    registry: Arc<impulse_core::lsp::LspRegistry>,
    runtime: Arc<Runtime>,
    event_rx: parking_lot::Mutex<mpsc::Receiver<impulse_core::lsp::LspEvent>>,
    documents: parking_lot::Mutex<HashMap<String, String>>,
}

/// Global registry mapping handle addresses to their inner data.
/// This eliminates raw pointer dereference — we only use the pointer as an opaque key.
/// Uses `parking_lot::Mutex` to avoid mutex poisoning issues.
fn lsp_handle_registry() -> &'static parking_lot::Mutex<HashMap<usize, Arc<LspRegistryInner>>> {
    static REGISTRY: OnceLock<parking_lot::Mutex<HashMap<usize, Arc<LspRegistryInner>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

fn update_lsp_document_cache_for_notify(
    inner: &LspRegistryInner,
    method: &str,
    params: &serde_json::Value,
) {
    match method {
        "textDocument/didOpen" => {
            let Some(document) = params.get("textDocument") else {
                return;
            };
            let Some(uri) = document.get("uri").and_then(|value| value.as_str()) else {
                return;
            };
            let Some(text) = document.get("text").and_then(|value| value.as_str()) else {
                return;
            };
            inner
                .documents
                .lock()
                .insert(uri.to_string(), text.to_string());
        }
        "textDocument/didClose" => {
            let Some(uri) = params
                .get("textDocument")
                .and_then(|document| document.get("uri"))
                .and_then(|value| value.as_str())
            else {
                return;
            };
            inner.documents.lock().remove(uri);
        }
        _ => {}
    }
}

fn apply_lsp_content_changes_to_string(
    content: &mut String,
    changes: &[lsp_types::TextDocumentContentChangeEvent],
) {
    for change in changes.iter().rev() {
        let Some(range) = change.range else {
            *content = change.text.clone();
            continue;
        };
        let start = lsp_position_to_byte_offset(content, range.start);
        let end = lsp_position_to_byte_offset(content, range.end);
        if start <= end && end <= content.len() {
            content.replace_range(start..end, &change.text);
        }
    }
}

fn lsp_position_to_byte_offset(content: &str, position: lsp_types::Position) -> usize {
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (byte_index, ch) in content.char_indices() {
        if line == position.line {
            break;
        }
        if ch == '\n' {
            line = line.saturating_add(1);
            line_start = byte_index + ch.len_utf8();
        }
    }
    if line != position.line {
        return content.len();
    }

    let mut utf16_units = 0u32;
    for (relative, ch) in content[line_start..].char_indices() {
        if ch == '\n' || utf16_units >= position.character {
            return line_start + relative;
        }
        utf16_units = utf16_units.saturating_add(ch.len_utf16() as u32);
        if utf16_units > position.character {
            return line_start + relative;
        }
    }
    content.len()
}

/// Look up a handle in the global registry and run `f` with the inner data.
/// Returns `default` if the handle is null or freed.
fn with_lsp_handle<T>(
    handle: *mut LspRegistryHandle,
    default: T,
    f: impl FnOnce(&LspRegistryInner) -> T,
) -> T {
    if handle.is_null() {
        return default;
    }
    let key = handle as usize;
    let guard = lsp_handle_registry().lock();
    match guard.get(&key) {
        Some(inner) => {
            let inner = Arc::clone(inner);
            drop(guard); // Release lock before calling f
            f(&inner)
        }
        None => {
            log::warn!("Attempted to use invalid or freed LSP registry handle");
            default
        }
    }
}

/// Opaque handle token for the C API. Never dereferenced — only used as a key.
pub struct LspRegistryHandle {
    _private: (),
}

/// Create a new LSP registry for the given workspace root URI.
///
/// Returns an opaque handle. The caller must free it with
/// `impulse_lsp_registry_free`.
#[no_mangle]
pub extern "C" fn impulse_lsp_registry_new(root_uri: *const c_char) -> *mut LspRegistryHandle {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let root_uri = match to_rust_str(root_uri) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            let runtime = match Runtime::new() {
                Ok(rt) => Arc::new(rt),
                Err(e) => {
                    log::error!("Failed to create Tokio runtime for LSP: {}", e);
                    return std::ptr::null_mut();
                }
            };

            let (event_tx, mut unbounded_rx) = mpsc::unbounded_channel();
            let registry = Arc::new(impulse_core::lsp::LspRegistry::new(root_uri, event_tx));

            // Create a bounded channel and spawn a forwarding task that bridges
            // the unbounded channel (required by LspRegistry) to a bounded one.
            // Events are dropped with a warning if the bounded channel is full.
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
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            // Receiver was dropped; stop forwarding.
                            break;
                        }
                    }
                }
            });

            let inner = Arc::new(LspRegistryInner {
                registry,
                runtime,
                event_rx: parking_lot::Mutex::new(bounded_rx),
                documents: parking_lot::Mutex::new(HashMap::new()),
            });

            // Allocate a stable address to use as an opaque handle key
            let handle = Box::into_raw(Box::new(LspRegistryHandle { _private: () }));
            lsp_handle_registry().lock().insert(handle as usize, inner);
            handle
        }),
    )
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
    ffi_catch(
        -1,
        AssertUnwindSafe(|| {
            let language_id = match to_rust_str(language_id) {
                Some(s) => s,
                None => return -1,
            };
            let file_uri = match to_rust_str(file_uri) {
                Some(s) => s,
                None => return -1,
            };

            with_lsp_handle(handle, -1, |inner| {
                inner.runtime.block_on(async {
                    let clients = inner.registry.get_clients(&language_id, &file_uri).await;
                    clients.len() as i32
                })
            })
        }),
    )
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
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
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
            let params: Option<serde_json::Value> =
                to_rust_str(params_json).and_then(|s| serde_json::from_str(&s).ok());

            with_lsp_handle(
                handle,
                to_c_string("{\"error\":\"invalid handle\"}"),
                |inner| {
                    inner.runtime.block_on(async {
                    let clients = inner.registry.get_clients(&language_id, &file_uri).await;
                    if let Some(client) = clients.first() {
                        match client.request(&method, params).await {
                            Ok(value) => {
                                let json = match serde_json::to_string(&value) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        log::error!("JSON serialization failed: {}", e);
                                        serde_json::json!({"error": format!("serialization failed: {}", e)})
                                            .to_string()
                                    }
                                };
                                to_c_string(&json)
                            }
                            Err(e) => {
                                let json = serde_json::json!({"error": e.to_string()});
                                to_c_string(&json.to_string())
                            }
                        }
                    } else {
                        to_c_string("{\"error\":\"no LSP client available\"}")
                    }
                })
                },
            )
        }),
    )
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
    ffi_catch(
        -1,
        AssertUnwindSafe(|| {
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

            with_lsp_handle(handle, -1, |inner| {
                update_lsp_document_cache_for_notify(inner, &method, &params);
                inner.runtime.block_on(async {
                    let clients = inner.registry.get_clients(&language_id, &file_uri).await;
                    if let Some(client) = clients.first() {
                        match client.notify(&method, params) {
                            Ok(()) => 0,
                            Err(_) => -1,
                        }
                    } else {
                        -1
                    }
                })
            })
        }),
    )
}

/// Send a textDocument/didChange notification with capability-aware incremental fallback.
///
/// `changes_json` should encode an array of LSP TextDocumentContentChangeEvent
/// objects. Servers that did not advertise incremental sync receive `full_text`
/// as a full-document change instead.
#[no_mangle]
pub extern "C" fn impulse_lsp_did_change(
    handle: *mut LspRegistryHandle,
    language_id: *const c_char,
    file_uri: *const c_char,
    version: i32,
    full_text: *const c_char,
    changes_json: *const c_char,
) -> i32 {
    ffi_catch(
        -1,
        AssertUnwindSafe(|| {
            let language_id = match to_rust_str(language_id) {
                Some(s) => s,
                None => return -1,
            };
            let file_uri = match to_rust_str(file_uri) {
                Some(s) => s,
                None => return -1,
            };
            let full_text = to_rust_str(full_text);
            let changes = to_rust_str(changes_json)
                .and_then(|json| {
                    serde_json::from_str::<Vec<lsp_types::TextDocumentContentChangeEvent>>(&json)
                        .ok()
                })
                .unwrap_or_default();

            with_lsp_handle(handle, -1, |inner| {
                inner.runtime.block_on(async {
                    let clients = inner.registry.get_clients(&language_id, &file_uri).await;
                    let mut documents = inner.documents.lock();
                    let document = documents.entry(file_uri.clone()).or_default();
                    if let Some(full_text) = full_text {
                        *document = full_text;
                    } else {
                        apply_lsp_content_changes_to_string(document, &changes);
                    }
                    let mut ok = false;
                    for client in clients {
                        ok |= client
                            .did_change_with_changes(&file_uri, version, document, changes.clone())
                            .is_ok();
                    }
                    if ok {
                        0
                    } else {
                        -1
                    }
                })
            })
        }),
    )
}

/// Poll for LSP events (diagnostics, server lifecycle).
///
/// Returns a JSON string describing the event, or null if no events are pending.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_lsp_poll_event(handle: *mut LspRegistryHandle) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            with_lsp_handle(handle, std::ptr::null_mut(), |inner| {
                let mut rx = inner.event_rx.lock();

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
            })
        }),
    )
}

/// Shut down all LSP servers managed by this registry.
#[no_mangle]
pub extern "C" fn impulse_lsp_shutdown_all(handle: *mut LspRegistryHandle) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            with_lsp_handle(handle, (), |inner| {
                inner.runtime.block_on(async {
                    inner.registry.shutdown_all().await;
                });
            });
        }),
    );
}

/// Free an LSP registry handle. Shuts down all servers first.
#[no_mangle]
pub extern "C" fn impulse_lsp_registry_free(handle: *mut LspRegistryHandle) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let key = handle as usize;
            // Remove from registry — the Arc<Inner> keeps data alive if another
            // thread is currently using it via with_lsp_handle.
            let inner = {
                let mut reg = lsp_handle_registry().lock();
                reg.remove(&key)
            };
            if let Some(inner) = inner {
                inner.runtime.block_on(async {
                    inner.registry.shutdown_all().await;
                });
            } else {
                log::warn!("impulse_lsp_registry_free called on already-freed handle");
                return; // Don't double-free
            }
            // Free the opaque handle allocation
            // SAFETY: `handle` was allocated by `Box::into_raw` in `impulse_lsp_registry_new`.
            // The registry removal above ensures this only happens once per handle.
            unsafe {
                drop(Box::from_raw(handle));
            }
        }),
    );
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
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
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
            let result = match serde_json::to_string(&json) {
                Ok(j) => j,
                Err(e) => {
                    log::error!("JSON serialization failed: {}", e);
                    serde_json::json!({"error": format!("serialization failed: {}", e)}).to_string()
                }
            };
            to_c_string(&result)
        }),
    )
}

/// Install managed web LSP servers.
///
/// Returns the installation root path on success, or an error string prefixed
/// with "ERROR:" on failure.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_lsp_install() -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(
            || match impulse_core::lsp::install_managed_web_lsp_servers() {
                Ok(path) => to_c_string(&path.to_string_lossy()),
                Err(e) => to_c_string(&format!("ERROR:{}", e)),
            },
        ),
    )
}

/// Check whether npm is available on the system PATH.
#[no_mangle]
pub extern "C" fn impulse_npm_is_available() -> bool {
    ffi_catch(false, AssertUnwindSafe(impulse_core::lsp::npm_is_available))
}

/// Check the installation status of system (non-managed) LSP servers.
///
/// Returns a JSON array of objects with `command`, `installed`, and
/// `resolvedPath` fields.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_system_lsp_status() -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let statuses = impulse_core::lsp::system_lsp_status();
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
            let result = match serde_json::to_string(&json) {
                Ok(j) => j,
                Err(e) => {
                    log::error!("JSON serialization failed: {}", e);
                    serde_json::json!({"error": format!("serialization failed: {}", e)}).to_string()
                }
            };
            to_c_string(&result)
        }),
    )
}

// ---------------------------------------------------------------------------
// Terminal backend API
// ---------------------------------------------------------------------------

use impulse_terminal::{SelectionKind, TerminalBackend};

/// Opaque handle passed across FFI — never constructed by external code.
struct TerminalHandle {
    backend: TerminalBackend,
}

#[no_mangle]
pub extern "C" fn impulse_terminal_create(
    config_json: *const c_char,
    cols: u16,
    rows: u16,
    cell_width: u16,
    cell_height: u16,
) -> *mut TerminalHandle {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let json = to_rust_str(config_json).unwrap_or_default();
            let config: impulse_terminal::TerminalConfig = match serde_json::from_str(&json) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to parse terminal config: {e}");
                    return std::ptr::null_mut();
                }
            };
            match TerminalBackend::new(config, cols, rows, cell_width, cell_height) {
                Ok(backend) => {
                    let handle = TerminalHandle { backend };
                    Box::into_raw(Box::new(handle))
                }
                Err(e) => {
                    log::error!("Failed to create terminal: {e}");
                    std::ptr::null_mut()
                }
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_destroy(handle: *mut TerminalHandle) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if !handle.is_null() {
                let h = unsafe { Box::from_raw(handle) };
                h.backend.shutdown();
                drop(h);
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_write(handle: *mut TerminalHandle, data: *const u8, len: usize) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() || data.is_null() || len == 0 {
                return;
            }
            let h = unsafe { &*handle };
            let bytes = unsafe { std::slice::from_raw_parts(data, len) };
            h.backend.write(bytes);
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_resize(
    handle: *mut TerminalHandle,
    cols: u16,
    rows: u16,
    cell_width: u16,
    cell_height: u16,
) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &mut *handle };
            h.backend.resize(cols, rows, cell_width, cell_height);
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_grid_snapshot(
    handle: *mut TerminalHandle,
    out_buf: *mut u8,
    buf_len: usize,
) -> usize {
    ffi_catch(
        0,
        AssertUnwindSafe(|| {
            if handle.is_null() || out_buf.is_null() {
                return 0;
            }
            let h = unsafe { &*handle };
            let required = h.backend.grid_buffer_size();
            if buf_len < required {
                return 0;
            }
            let out = unsafe { std::slice::from_raw_parts_mut(out_buf, buf_len) };
            h.backend.write_grid_to_buffer(out)
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_grid_snapshot_size(handle: *mut TerminalHandle) -> usize {
    ffi_catch(
        0,
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return 0;
            }
            let h = unsafe { &*handle };
            h.backend.grid_buffer_size()
        }),
    )
}

/// Take the terminal damage accumulated since the last call.
///
/// Returns -1 when the entire viewport must be repainted, otherwise the
/// number of damaged viewport row indices written to `out_rows` (at most
/// `cap`). When the damaged row count exceeds `cap`, degrades to -1.
/// Resets the backend's damage tracking either way.
#[no_mangle]
pub extern "C" fn impulse_terminal_take_damage(
    handle: *mut TerminalHandle,
    out_rows: *mut u16,
    cap: usize,
) -> i64 {
    ffi_catch(
        -1,
        AssertUnwindSafe(|| {
            if handle.is_null() || out_rows.is_null() {
                return -1;
            }
            let h = unsafe { &*handle };
            match h.backend.take_damage() {
                None => -1,
                Some(rows) => {
                    if rows.len() > cap {
                        return -1;
                    }
                    let out = unsafe { std::slice::from_raw_parts_mut(out_rows, cap) };
                    out[..rows.len()].copy_from_slice(&rows);
                    rows.len() as i64
                }
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_poll_events(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return std::ptr::null_mut();
            }
            let h = unsafe { &*handle };
            let events = h.backend.poll_events();
            if events.is_empty() {
                return std::ptr::null_mut();
            }
            match serde_json::to_string(&events) {
                Ok(json) => to_c_string(&json),
                Err(_) => std::ptr::null_mut(),
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_command_blocks(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return to_c_string("[]");
            }
            let h = unsafe { &*handle };
            match serde_json::to_string(&h.backend.command_blocks()) {
                Ok(json) => to_c_string(&json),
                Err(_) => to_c_string("[]"),
            }
        }),
    )
}

/// Viewport-mapped command block regions plus the live prompt region as JSON
/// (see `impulse_terminal::BlockOverlay`). Used for Warp-style block
/// decorations. Caller frees with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_terminal_block_overlay(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return std::ptr::null_mut();
            }
            let h = unsafe { &*handle };
            match serde_json::to_string(&h.backend.block_overlay()) {
                Ok(json) => to_c_string(&json),
                Err(_) => std::ptr::null_mut(),
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_command_block_flags(handle: *mut TerminalHandle) -> u32 {
    ffi_catch(
        0,
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return 0;
            }
            let h = unsafe { &*handle };
            let flags = h.backend.command_block_flags();
            (flags.has_command as u32)
                | ((flags.has_output as u32) << 1)
                | ((flags.has_failed as u32) << 2)
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_command_history_search(
    handle: *mut TerminalHandle,
    query_json: *const c_char,
) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return to_c_string("[]");
            }
            let query = to_rust_str(query_json)
                .and_then(|json| {
                    serde_json::from_str::<impulse_terminal::CommandHistoryQuery>(&json).ok()
                })
                .unwrap_or_default();
            let h = unsafe { &*handle };
            match serde_json::to_string(&h.backend.search_command_history(&query)) {
                Ok(json) => to_c_string(&json),
                Err(_) => to_c_string("[]"),
            }
        }),
    )
}

/// Return up to `limit` recent command strings from the terminal's block
/// history (newest first) as a JSON string array. Used by the Swift-side
/// input completion. Caller frees with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_terminal_recent_commands(
    handle: *mut TerminalHandle,
    limit: usize,
) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return to_c_string("[]");
            }
            let h = unsafe { &*handle };
            match serde_json::to_string(&h.backend.recent_command_strings(limit)) {
                Ok(json) => to_c_string(&json),
                Err(_) => to_c_string("[]"),
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_rerun_command(
    handle: *mut TerminalHandle,
    command: *const c_char,
) -> bool {
    ffi_catch(
        false,
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return false;
            }
            let Some(command) = to_rust_str(command) else {
                return false;
            };
            let h = unsafe { &*handle };
            h.backend.rerun_command(&command)
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_start_selection(
    handle: *mut TerminalHandle,
    col: u16,
    row: u16,
    kind: u8,
) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &*handle };
            h.backend
                .start_selection(col as usize, row as usize, SelectionKind::from_u8(kind));
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_update_selection(
    handle: *mut TerminalHandle,
    col: u16,
    row: u16,
) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &*handle };
            h.backend.update_selection(col as usize, row as usize);
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_clear_selection(handle: *mut TerminalHandle) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &*handle };
            h.backend.clear_selection();
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_selected_text(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return std::ptr::null_mut();
            }
            let h = unsafe { &*handle };
            match h.backend.selected_text() {
                Some(text) => to_c_string(&text),
                None => std::ptr::null_mut(),
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_scroll(handle: *mut TerminalHandle, delta: i32) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &*handle };
            h.backend.scroll(delta);
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_scroll_to_bottom(handle: *mut TerminalHandle) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &*handle };
            h.backend.scroll_to_bottom();
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_scroll_to_command_block(
    handle: *mut TerminalHandle,
    block_id: u64,
) -> bool {
    ffi_catch(
        false,
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return false;
            }
            let h = unsafe { &*handle };
            h.backend
                .scroll_to_command_block(impulse_terminal::TerminalBlockId(block_id))
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_mode(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return to_c_string("{}");
            }
            let h = unsafe { &*handle };
            let mode = h.backend.mode();
            let json_mode = serde_json::json!({ "bits": mode.bits() });
            match serde_json::to_string(&json_mode) {
                Ok(json) => to_c_string(&json),
                Err(_) => to_c_string("{}"),
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_set_focus(handle: *mut TerminalHandle, focused: bool) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &*handle };
            h.backend.set_focus(focused);
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_set_colors(
    handle: *mut TerminalHandle,
    config_json: *const c_char,
) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &mut *handle };
            let json = to_rust_str(config_json).unwrap_or_default();
            if let Ok(config) = serde_json::from_str::<impulse_terminal::TerminalConfig>(&json) {
                h.backend.set_colors(&config);
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_child_pid(handle: *mut TerminalHandle) -> u32 {
    ffi_catch(
        0,
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return 0;
            }
            let h = unsafe { &*handle };
            h.backend.child_pid()
        }),
    )
}

/// Return the OSC 8 hyperlink URI at the given grid cell, or NULL if none.
/// Caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_terminal_hyperlink_at(
    handle: *mut TerminalHandle,
    col: u32,
    row: u32,
) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return std::ptr::null_mut();
            }
            let h = unsafe { &*handle };
            match h.backend.hyperlink_at(col as usize, row as usize) {
                Some(uri) => to_c_string(&uri),
                None => std::ptr::null_mut(),
            }
        }),
    )
}

// Search FFI functions.

#[no_mangle]
pub extern "C" fn impulse_terminal_search(
    handle: *mut TerminalHandle,
    pattern: *const c_char,
) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return to_c_string("{}");
            }
            let h = unsafe { &*handle };
            let pat = to_rust_str(pattern).unwrap_or_default();
            let result = h.backend.search(&pat);
            match serde_json::to_string(&result) {
                Ok(json) => to_c_string(&json),
                Err(_) => to_c_string("{}"),
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_search_next(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return to_c_string("{}");
            }
            let h = unsafe { &*handle };
            let result = h.backend.search_next();
            match serde_json::to_string(&result) {
                Ok(json) => to_c_string(&json),
                Err(_) => to_c_string("{}"),
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_search_prev(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return to_c_string("{}");
            }
            let h = unsafe { &*handle };
            let result = h.backend.search_prev();
            match serde_json::to_string(&result) {
                Ok(json) => to_c_string(&json),
                Err(_) => to_c_string("{}"),
            }
        }),
    )
}

#[no_mangle]
pub extern "C" fn impulse_terminal_search_clear(handle: *mut TerminalHandle) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &*handle };
            h.backend.search_clear();
        }),
    )
}
