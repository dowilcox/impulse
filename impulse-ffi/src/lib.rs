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
// Monaco assets
// ---------------------------------------------------------------------------

/// Ensure Monaco editor files are extracted to the platform data directory.
///
/// Returns the extraction path on success or an error string on failure.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_ensure_monaco_extracted() -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| match impulse_editor::assets::ensure_monaco_extracted() {
            Ok(path) => to_c_string(&path.to_string_lossy()),
            Err(e) => to_c_string(&format!("ERROR:{}", e)),
        }),
    )
}

/// Return the embedded editor HTML content as a static string.
///
/// The returned pointer is valid for the lifetime of the process and must
/// NOT be freed.
#[no_mangle]
pub extern "C" fn impulse_get_editor_html() -> *const c_char {
    ffi_catch(
        std::ptr::null(),
        AssertUnwindSafe(|| {
            static CACHED: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
            CACHED
                .get_or_init(|| {
                    CString::new(impulse_editor::assets::EDITOR_HTML).unwrap_or_else(|e| {
                        log::warn!("EDITOR_HTML contains NUL at byte {}", e.nul_position());
                        let html = impulse_editor::assets::EDITOR_HTML;
                        let sanitized: String = html.chars().filter(|&c| c != '\0').collect();
                        CString::new(sanitized).unwrap_or_default()
                    })
                })
                .as_ptr()
        }),
    )
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
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let shell_name = match to_rust_str(shell) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            let shell_type = impulse_core::shell::detect_shell_type(&shell_name);
            let script = impulse_core::shell::get_integration_script(&shell_type);
            to_c_string(script)
        }),
    )
}

/// Return the user's login shell path.
///
/// Falls back to `$SHELL`, then `/bin/bash`.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_get_user_login_shell() -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| to_c_string(&impulse_core::shell::get_default_shell_path())),
    )
}

/// Return the user's login shell name (e.g. "fish", "zsh", "bash").
///
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_get_user_login_shell_name() -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| to_c_string(&impulse_core::shell::get_default_shell_name())),
    )
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Search for files by name in `root` matching `query`.
///
/// Returns a JSON array of `SearchResult` objects.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_search_files(root: *const c_char, query: *const c_char) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let root = match to_rust_str(root) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };
            let query = match to_rust_str(query) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            match impulse_core::search::search_filenames(&root, &query, 200, None) {
                Ok(results) => {
                    let json = match serde_json::to_string(&results) {
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
        }),
    )
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
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let root = match to_rust_str(root) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };
            let query = match to_rust_str(query) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            match impulse_core::search::search_contents(&root, &query, 500, case_sensitive, None) {
                Ok(results) => {
                    let json = match serde_json::to_string(&results) {
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
        }),
    )
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
}

/// Global registry mapping handle addresses to their inner data.
/// This eliminates raw pointer dereference — we only use the pointer as an opaque key.
/// Uses `parking_lot::Mutex` to avoid mutex poisoning issues.
fn lsp_handle_registry() -> &'static parking_lot::Mutex<HashMap<usize, Arc<LspRegistryInner>>> {
    static REGISTRY: OnceLock<parking_lot::Mutex<HashMap<usize, Arc<LspRegistryInner>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
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

            with_lsp_handle(handle, to_c_string("{\"error\":\"invalid handle\"}"), |inner| {
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
            })
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
                    serde_json::json!({"error": format!("serialization failed: {}", e)})
                        .to_string()
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
    impulse_core::lsp::npm_is_available()
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
                    serde_json::json!({"error": format!("serialization failed: {}", e)})
                        .to_string()
                }
            };
            to_c_string(&result)
        }),
    )
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

/// Returns the current git branch name for the given directory path.
///
/// Returns null if not in a git repo or on error.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_git_branch(path: *const c_char) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let path = match to_rust_str(path) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            match impulse_core::filesystem::get_git_branch(&path) {
                Ok(Some(branch)) => to_c_string(&branch),
                Ok(None) | Err(_) => std::ptr::null_mut(),
            }
        }),
    )
}

/// Returns git blame info for a specific line in a file.
///
/// Returns a JSON object with `author`, `date`, `commitHash`, and `summary`
/// fields, or null on error.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_git_blame(file_path: *const c_char, line: u32) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let file_path = match to_rust_str(file_path) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            let result = impulse_core::util::run_with_timeout(
                std::time::Duration::from_secs(10),
                "git blame",
                move || {
                    impulse_core::git::get_line_blame(&file_path, line).map(|info| {
                        serde_json::json!({
                            "author": info.author,
                            "date": info.date,
                            "commitHash": info.commit_hash,
                            "summary": info.summary,
                        })
                        .to_string()
                    })
                },
            );
            match result {
                Ok(json) => to_c_string(&json),
                Err(_) => std::ptr::null_mut(),
            }
        }),
    )
}

/// Discard working-tree changes for a single file, restoring it to the HEAD version.
/// `workspace_root` is used to validate that the file is within the workspace.
///
/// Returns 0 on success or -1 on error.
#[no_mangle]
pub extern "C" fn impulse_git_discard_changes(
    file_path: *const c_char,
    workspace_root: *const c_char,
) -> i32 {
    ffi_catch(
        -1,
        AssertUnwindSafe(|| {
            let file_path = match to_rust_str(file_path) {
                Some(s) => s,
                None => return -1,
            };
            let workspace_root = match to_rust_str(workspace_root) {
                Some(s) => s,
                None => return -1,
            };

            match impulse_core::git::discard_file_changes(&file_path, &workspace_root) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        }),
    )
}

/// Computes diff markers for the given file path (comparing working copy to HEAD).
///
/// Returns a JSON array of objects with `"line"` (1-based u32) and `"status"`
/// (`"added"` / `"modified"` / `"deleted"`) fields.
/// Returns null on error.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_git_diff_markers(file_path: *const c_char) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let file_path = match to_rust_str(file_path) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            let result = impulse_core::util::run_with_timeout(
                std::time::Duration::from_secs(10),
                "git diff",
                move || {
                    let diff = impulse_core::git::get_file_diff(&file_path)?;
                    let mut markers: Vec<impulse_editor::protocol::DiffDecoration> = diff
                        .changed_lines
                        .iter()
                        .filter_map(|(&line, status)| {
                            let status_str = match status {
                                impulse_core::git::DiffLineStatus::Added => "added",
                                impulse_core::git::DiffLineStatus::Modified => "modified",
                                impulse_core::git::DiffLineStatus::Unchanged => return None,
                            };
                            Some(impulse_editor::protocol::DiffDecoration {
                                line,
                                status: status_str.to_string(),
                            })
                        })
                        .collect();
                    for &line in &diff.deleted_lines {
                        markers.push(impulse_editor::protocol::DiffDecoration {
                            line,
                            status: "deleted".to_string(),
                        });
                    }
                    serde_json::to_string(&markers)
                        .map_err(|e| format!("serialization failed: {}", e))
                },
            );
            match result {
                Ok(json) => to_c_string(&json),
                Err(_) => std::ptr::null_mut(),
            }
        }),
    )
}
