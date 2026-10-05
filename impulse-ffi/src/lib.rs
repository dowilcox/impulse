//! C-compatible FFI wrappers around the impulse-terminal backend.
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

use std::ffi::c_void;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

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
            match serde_json::to_string(&h.backend.command_block_summaries()) {
                Ok(json) => to_c_string(&json),
                Err(_) => to_c_string("[]"),
            }
        }),
    )
}

/// One command block, with its captured output, as JSON; null when there's
/// no such block. Caller frees with `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_terminal_command_block(
    handle: *mut TerminalHandle,
    id: u64,
) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return std::ptr::null_mut();
            }
            let h = unsafe { &*handle };
            match h
                .backend
                .command_block(id)
                .map(|block| serde_json::to_string(&block))
            {
                Some(Ok(json)) => to_c_string(&json),
                _ => std::ptr::null_mut(),
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

/// Changes whenever `impulse_terminal_block_overlay` could return something
/// different; 0 for a null handle.
#[no_mangle]
pub extern "C" fn impulse_terminal_block_overlay_key(handle: *mut TerminalHandle) -> u64 {
    ffi_catch(
        0,
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return 0;
            }
            let h = unsafe { &*handle };
            h.backend.block_overlay_key()
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

/// Called from the PTY reader thread when the terminal has events to poll.
pub type ImpulseWakeupFn = extern "C" fn(context: *mut c_void);

/// Register (or, with a null `callback`, remove) the wakeup callback. It runs
/// on a background thread, at most once between `impulse_terminal_poll_events`
/// calls; `context` is passed back unchanged. After this returns, the previous
/// callback is not running and won't be called again.
#[no_mangle]
pub extern "C" fn impulse_terminal_set_wakeup_callback(
    handle: *mut TerminalHandle,
    callback: Option<ImpulseWakeupFn>,
    context: *mut c_void,
) {
    ffi_catch(
        (),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return;
            }
            let h = unsafe { &*handle };
            let context = context as usize;
            h.backend.set_wakeup_callback(callback.map(
                |callback| -> Box<dyn Fn() + Send + Sync> {
                    Box::new(move || callback(context as *mut c_void))
                },
            ));
        }),
    )
}

/// About the last `max_rows` rows of output as text (with SGR colors when
/// `with_sgr`), for restoring scrollback in a later session. Free with
/// `impulse_free_string`.
#[no_mangle]
pub extern "C" fn impulse_terminal_transcript(
    handle: *mut TerminalHandle,
    max_rows: u32,
    with_sgr: bool,
) -> *mut c_char {
    ffi_catch(
        std::ptr::null_mut(),
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return std::ptr::null_mut();
            }
            let h = unsafe { &*handle };
            to_c_string(&h.backend.transcript(max_rows as usize, with_sgr))
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
pub extern "C" fn impulse_terminal_mode(handle: *mut TerminalHandle) -> u32 {
    ffi_catch(
        0,
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return 0;
            }
            let h = unsafe { &*handle };
            u32::from(h.backend.mode().bits())
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

/// The foreground process group of the terminal's PTY (the shell, or the
/// program it runs), or -1 when unknown or closed.
#[no_mangle]
pub extern "C" fn impulse_terminal_foreground_pid(handle: *mut TerminalHandle) -> i32 {
    ffi_catch(
        -1,
        AssertUnwindSafe(|| {
            if handle.is_null() {
                return -1;
            }
            let h = unsafe { &*handle };
            h.backend.foreground_pid().unwrap_or(-1)
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
