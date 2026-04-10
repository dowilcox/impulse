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

            match impulse_core::git::get_git_branch(&path) {
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

/// Returns git status for files in a directory as a JSON object mapping
/// filenames to status codes (e.g. `{"file.rs": "M", "new.txt": "?"}`).
///
/// Status codes: "M" (modified), "A" (added), "?" (untracked), "D" (deleted),
/// "R" (renamed), "C" (conflicted).
///
/// Returns null on error (e.g. not a git repo).
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_git_status_for_directory(path: *const c_char) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let path = match to_rust_str(path) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            match impulse_core::filesystem::get_git_status_for_directory(&path) {
                Ok(status_map) => {
                    let json = match serde_json::to_string(&status_map) {
                        Ok(j) => j,
                        Err(e) => {
                            log::error!("JSON serialization failed: {}", e);
                            return std::ptr::null_mut();
                        }
                    };
                    to_c_string(&json)
                }
                Err(_) => std::ptr::null_mut(),
            }
        }),
    )
}

/// Batch-fetch git status for the entire repository in a single call.
///
/// Returns a JSON object mapping directory paths to inner objects mapping
/// filenames to status codes. Example:
/// `{"/path/to/dir": {"file.rs": "M", "new.txt": "?"}}`.
///
/// Parent directories receive the highest-priority status among descendants.
/// Returns null on error.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_get_all_git_statuses(path: *const c_char) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let path = match to_rust_str(path) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            match impulse_core::filesystem::get_all_git_statuses(&path) {
                Ok(status_map) => {
                    let json = match serde_json::to_string(&status_map) {
                        Ok(j) => j,
                        Err(e) => {
                            log::error!("JSON serialization failed: {}", e);
                            return std::ptr::null_mut();
                        }
                    };
                    to_c_string(&json)
                }
                Err(_) => std::ptr::null_mut(),
            }
        }),
    )
}

/// Read directory contents with git status enrichment as a JSON array.
///
/// Returns a JSON array of `FileEntry` objects, each with `name`, `path`,
/// `is_dir`, `is_symlink`, `size`, `modified`, and `git_status` fields.
/// Returns null on error.
/// The caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_read_directory_with_git_status(
    path: *const c_char,
    show_hidden: bool,
) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let path = match to_rust_str(path) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };

            match impulse_core::filesystem::read_directory_with_git_status(&path, show_hidden) {
                Ok(entries) => {
                    let json = match serde_json::to_string(&entries) {
                        Ok(j) => j,
                        Err(e) => {
                            log::error!("JSON serialization failed: {}", e);
                            return std::ptr::null_mut();
                        }
                    };
                    to_c_string(&json)
                }
                Err(_) => std::ptr::null_mut(),
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
                            let diff_status = match status {
                                impulse_core::git::DiffLineStatus::Added => {
                                    impulse_editor::protocol::DiffStatus::Added
                                }
                                impulse_core::git::DiffLineStatus::Modified => {
                                    impulse_editor::protocol::DiffStatus::Modified
                                }
                                impulse_core::git::DiffLineStatus::Unchanged => return None,
                            };
                            Some(impulse_editor::protocol::DiffDecoration {
                                line,
                                status: diff_status,
                            })
                        })
                        .collect();
                    for &line in &diff.deleted_lines {
                        markers.push(impulse_editor::protocol::DiffDecoration {
                            line,
                            status: impulse_editor::protocol::DiffStatus::Deleted,
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

// ---------------------------------------------------------------------------
// Markdown preview
// ---------------------------------------------------------------------------

/// Render markdown source to a full HTML document with themed CSS and highlight.js.
///
/// `source` — the markdown text to render.
/// `theme_json` — JSON-serialized `MarkdownThemeColors`.
/// `highlight_js_path` — absolute file:// path or URL to highlight.min.js.
///
/// Returns a newly allocated HTML string (caller must free with `impulse_free_string`),
/// or null on failure.
#[no_mangle]
pub extern "C" fn impulse_render_markdown_preview(
    source: *const c_char,
    theme_json: *const c_char,
    highlight_js_path: *const c_char,
) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let source = match to_rust_str(source) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };
            let theme_json = match to_rust_str(theme_json) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };
            let hljs_path = match to_rust_str(highlight_js_path) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };
            let theme: impulse_editor::markdown::MarkdownThemeColors =
                match serde_json::from_str(&theme_json) {
                    Ok(t) => t,
                    Err(e) => {
                        log::error!("Failed to parse MarkdownThemeColors: {}", e);
                        return std::ptr::null_mut();
                    }
                };
            let html = match impulse_editor::markdown::render_markdown_preview(
                &source, &theme, &hljs_path,
            ) {
                Some(h) => h,
                None => return std::ptr::null_mut(),
            };
            to_c_string(&html)
        }),
    )
}

/// Check whether a file path has a markdown extension.
#[no_mangle]
pub extern "C" fn impulse_is_markdown_file(path: *const c_char) -> bool {
    ffi_catch(
        false,
        AssertUnwindSafe(|| {
            let path = match to_rust_str(path) {
                Some(s) => s,
                None => return false,
            };
            impulse_editor::markdown::is_markdown_file(&path)
        }),
    )
}

/// Render an SVG source string to a themed HTML preview document.
///
/// Returns a newly allocated HTML string (caller must free with `impulse_free_string`),
/// or null on failure.
#[no_mangle]
pub extern "C" fn impulse_render_svg_preview(
    source: *const c_char,
    bg_color: *const c_char,
) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let source = match to_rust_str(source) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };
            let bg_color = match to_rust_str(bg_color) {
                Some(s) => s,
                None => return std::ptr::null_mut(),
            };
            let html = match impulse_editor::svg::render_svg_preview(&source, &bg_color) {
                Some(h) => h,
                None => return std::ptr::null_mut(),
            };
            to_c_string(&html)
        }),
    )
}

/// Check whether a file path has an SVG extension.
#[no_mangle]
pub extern "C" fn impulse_is_svg_file(path: *const c_char) -> bool {
    ffi_catch(
        false,
        AssertUnwindSafe(|| {
            let path = match to_rust_str(path) {
                Some(s) => s,
                None => return false,
            };
            impulse_editor::svg::is_svg_file(&path)
        }),
    )
}

/// Check whether a file path is a previewable type (markdown or SVG).
#[no_mangle]
pub extern "C" fn impulse_is_previewable_file(path: *const c_char) -> bool {
    ffi_catch(
        false,
        AssertUnwindSafe(|| {
            let path = match to_rust_str(path) {
                Some(s) => s,
                None => return false,
            };
            impulse_editor::is_previewable_file(&path)
        }),
    )
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Return default settings as a JSON string.
#[no_mangle]
pub extern "C" fn impulse_settings_default_json() -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| to_c_string(&impulse_core::settings::Settings::default_json())),
    )
}

/// Parse, migrate, and validate a raw settings JSON string.
/// Returns the cleaned JSON. If the input is null or invalid, returns default settings.
#[no_mangle]
pub extern "C" fn impulse_settings_load_json(json: *const c_char) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let raw = to_rust_str(json).unwrap_or_default();
            let settings = impulse_core::settings::Settings::from_json(&raw).unwrap_or_default();
            let result = settings
                .to_json()
                .unwrap_or_else(|_| impulse_core::settings::Settings::default_json());
            to_c_string(&result)
        }),
    )
}

/// Validate/clamp a settings JSON string and return the cleaned version.
#[no_mangle]
pub extern "C" fn impulse_settings_validate_json(json: *const c_char) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            let raw = match to_rust_str(json) {
                Some(s) => s,
                None => return to_c_string(&impulse_core::settings::Settings::default_json()),
            };
            let mut settings: impulse_core::settings::Settings =
                serde_json::from_str(&raw).unwrap_or_default();
            settings.validate();
            let result = settings
                .to_json()
                .unwrap_or_else(|_| impulse_core::settings::Settings::default_json());
            to_c_string(&result)
        }),
    )
}

/// Check for a newer version on GitHub Releases.
///
/// Returns a JSON string `{"version":"X.Y.Z","url":"..."}` if an update is
/// available, an empty string if up-to-date or checked recently, or an
/// `"ERROR:..."` string on failure. Caller must free with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_check_for_update() -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| match impulse_core::update::check_for_update() {
            Ok(Some(info)) => {
                let json = serde_json::json!({
                    "version": info.version,
                    "current_version": info.current_version,
                    "url": info.url,
                });
                to_c_string(&json.to_string())
            }
            Ok(None) => to_c_string(""),
            Err(e) => to_c_string(&format!("ERROR:{}", e)),
        }),
    )
}

/// Return the current application version string.
///
/// The returned pointer is valid for the lifetime of the process and must
/// NOT be freed.
#[no_mangle]
pub extern "C" fn impulse_get_version() -> *const c_char {
    ffi_catch(
        std::ptr::null(),
        AssertUnwindSafe(|| {
            static CACHED: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
            CACHED
                .get_or_init(|| {
                    CString::new(impulse_core::update::CURRENT_VERSION).unwrap_or_default()
                })
                .as_ptr()
        }),
    )
}

/// Check whether a file path matches a glob-style pattern.
#[no_mangle]
pub extern "C" fn impulse_matches_file_pattern(
    path: *const c_char,
    pattern: *const c_char,
) -> bool {
    ffi_catch(
        false,
        AssertUnwindSafe(|| {
            let path = match to_rust_str(path) {
                Some(s) => s,
                None => return false,
            };
            let pattern = match to_rust_str(pattern) {
                Some(s) => s,
                None => return false,
            };
            impulse_core::util::matches_file_pattern(&path, &pattern)
        }),
    )
}

// ---------------------------------------------------------------------------
// Theme API
// ---------------------------------------------------------------------------

/// Return a JSON array of all available theme names (built-in + user).
#[no_mangle]
pub extern "C" fn impulse_available_themes() -> *mut c_char {
    ffi_catch(
        to_c_string("[]"),
        AssertUnwindSafe(|| {
            let names = impulse_core::theme::available_themes();
            let json = serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string());
            to_c_string(&json)
        }),
    )
}

/// Return the display name for a theme ID.
#[no_mangle]
pub extern "C" fn impulse_theme_display_name(id: *const c_char) -> *mut c_char {
    ffi_catch(
        to_c_string(""),
        AssertUnwindSafe(|| {
            let id = match to_rust_str(id) {
                Some(s) => s,
                None => return to_c_string(""),
            };
            to_c_string(&impulse_core::theme::theme_display_name(&id))
        }),
    )
}

/// Resolve a theme by name and return the full `ResolvedTheme` as JSON.
#[no_mangle]
pub extern "C" fn impulse_get_theme(name: *const c_char) -> *mut c_char {
    ffi_catch(
        to_c_string("{}"),
        AssertUnwindSafe(|| {
            let name = match to_rust_str(name) {
                Some(s) => s,
                None => "nord".to_string(),
            };
            let theme = impulse_core::theme::get_theme(&name);
            to_c_string(&impulse_core::theme::theme_to_json(&theme))
        }),
    )
}

/// Resolve a theme by name and return the `MonacoThemeDefinition` as JSON.
#[no_mangle]
pub extern "C" fn impulse_get_monaco_theme(name: *const c_char) -> *mut c_char {
    ffi_catch(
        to_c_string("{}"),
        AssertUnwindSafe(|| {
            let name = match to_rust_str(name) {
                Some(s) => s,
                None => "nord".to_string(),
            };
            let theme = impulse_core::theme::get_theme(&name);
            let monaco = impulse_editor::protocol::theme_to_monaco(&theme);
            let json = serde_json::to_string(&monaco).unwrap_or_else(|_| "{}".to_string());
            to_c_string(&json)
        }),
    )
}

/// Resolve a theme by name and return the `MarkdownThemeColors` as JSON.
#[no_mangle]
pub extern "C" fn impulse_get_markdown_theme(name: *const c_char) -> *mut c_char {
    ffi_catch(
        to_c_string("{}"),
        AssertUnwindSafe(|| {
            let name = match to_rust_str(name) {
                Some(s) => s,
                None => "nord".to_string(),
            };
            let theme = impulse_core::theme::get_theme(&name);
            let md_colors = impulse_editor::markdown::theme_to_markdown_colors(&theme);
            let json = serde_json::to_string(&md_colors).unwrap_or_else(|_| "{}".to_string());
            to_c_string(&json)
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
    /// Pre-allocated buffer for grid snapshots.
    snapshot_buf: Vec<u8>,
}

#[no_mangle]
pub extern "C" fn impulse_terminal_create(
    config_json: *const c_char,
    cols: u16,
    rows: u16,
    cell_width: u16,
    cell_height: u16,
) -> *mut TerminalHandle {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
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
                let buf_size = backend.grid_buffer_size();
                let handle = TerminalHandle {
                    backend,
                    snapshot_buf: vec![0u8; buf_size],
                };
                Box::into_raw(Box::new(handle))
            }
            Err(e) => {
                log::error!("Failed to create terminal: {e}");
                std::ptr::null_mut()
            }
        }
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_destroy(handle: *mut TerminalHandle) {
    ffi_catch((), AssertUnwindSafe(|| {
        if !handle.is_null() {
            let h = unsafe { Box::from_raw(handle) };
            h.backend.shutdown();
            drop(h);
        }
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_write(handle: *mut TerminalHandle, data: *const u8, len: usize) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() || data.is_null() || len == 0 { return; }
        let h = unsafe { &*handle };
        let bytes = unsafe { std::slice::from_raw_parts(data, len) };
        h.backend.write(bytes);
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_resize(
    handle: *mut TerminalHandle, cols: u16, rows: u16, cell_width: u16, cell_height: u16,
) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &mut *handle };
        h.backend.resize(cols, rows, cell_width, cell_height);
        let new_size = h.backend.grid_buffer_size();
        h.snapshot_buf.resize(new_size, 0);
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_grid_snapshot(
    handle: *mut TerminalHandle, out_buf: *mut u8, buf_len: usize,
) -> usize {
    ffi_catch(0, AssertUnwindSafe(|| {
        if handle.is_null() || out_buf.is_null() { return 0; }
        let h = unsafe { &mut *handle };
        let written = h.backend.write_grid_to_buffer(&mut h.snapshot_buf);
        if written == 0 || written > buf_len { return 0; }
        unsafe { std::ptr::copy_nonoverlapping(h.snapshot_buf.as_ptr(), out_buf, written); }
        written
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_grid_snapshot_size(handle: *mut TerminalHandle) -> usize {
    ffi_catch(0, AssertUnwindSafe(|| {
        if handle.is_null() { return 0; }
        let h = unsafe { &*handle };
        h.backend.grid_buffer_size()
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_poll_events(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return to_c_string("[]"); }
        let h = unsafe { &*handle };
        let events = h.backend.poll_events();
        match serde_json::to_string(&events) {
            Ok(json) => to_c_string(&json),
            Err(_) => to_c_string("[]"),
        }
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_start_selection(
    handle: *mut TerminalHandle, col: u16, row: u16, kind: u8,
) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &*handle };
        h.backend.start_selection(col as usize, row as usize, SelectionKind::from_u8(kind));
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_update_selection(handle: *mut TerminalHandle, col: u16, row: u16) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &*handle };
        h.backend.update_selection(col as usize, row as usize);
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_clear_selection(handle: *mut TerminalHandle) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &*handle };
        h.backend.clear_selection();
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_selected_text(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return std::ptr::null_mut(); }
        let h = unsafe { &*handle };
        match h.backend.selected_text() {
            Some(text) => to_c_string(&text),
            None => std::ptr::null_mut(),
        }
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_scroll(handle: *mut TerminalHandle, delta: i32) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &*handle };
        h.backend.scroll(delta);
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_scroll_to_bottom(handle: *mut TerminalHandle) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &*handle };
        h.backend.scroll_to_bottom();
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_mode(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return to_c_string("{}"); }
        let h = unsafe { &*handle };
        let mode = h.backend.mode();
        match serde_json::to_string(&mode) {
            Ok(json) => to_c_string(&json),
            Err(_) => to_c_string("{}"),
        }
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_set_focus(handle: *mut TerminalHandle, focused: bool) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &*handle };
        h.backend.set_focus(focused);
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_set_colors(handle: *mut TerminalHandle, config_json: *const c_char) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &mut *handle };
        let json = to_rust_str(config_json).unwrap_or_default();
        if let Ok(config) = serde_json::from_str::<impulse_terminal::TerminalConfig>(&json) {
            h.backend.set_colors(&config);
        }
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_child_pid(handle: *mut TerminalHandle) -> u32 {
    ffi_catch(0, AssertUnwindSafe(|| {
        if handle.is_null() { return 0; }
        let h = unsafe { &*handle };
        h.backend.child_pid()
    }))
}

/// Return the OSC 8 hyperlink URI at the given grid cell, or NULL if none.
/// Caller must free the returned string with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_terminal_hyperlink_at(
    handle: *mut TerminalHandle,
    col: u32,
    row: u32,
) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return std::ptr::null_mut(); }
        let h = unsafe { &*handle };
        match h.backend.hyperlink_at(col as usize, row as usize) {
            Some(uri) => to_c_string(&uri),
            None => std::ptr::null_mut(),
        }
    }))
}

// Search FFI functions.

#[no_mangle]
pub extern "C" fn impulse_terminal_search(handle: *mut TerminalHandle, pattern: *const c_char) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return to_c_string("{}"); }
        let h = unsafe { &mut *handle };
        let pat = to_rust_str(pattern).unwrap_or_default();
        let result = h.backend.search(&pat);
        match serde_json::to_string(&result) {
            Ok(json) => to_c_string(&json),
            Err(_) => to_c_string("{}"),
        }
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_search_next(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return to_c_string("{}"); }
        let h = unsafe { &mut *handle };
        let result = h.backend.search_next();
        match serde_json::to_string(&result) {
            Ok(json) => to_c_string(&json),
            Err(_) => to_c_string("{}"),
        }
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_search_prev(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return to_c_string("{}"); }
        let h = unsafe { &mut *handle };
        let result = h.backend.search_prev();
        match serde_json::to_string(&result) {
            Ok(json) => to_c_string(&json),
            Err(_) => to_c_string("{}"),
        }
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_search_clear(handle: *mut TerminalHandle) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &mut *handle };
        h.backend.search_clear();
    }))
}
