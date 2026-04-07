# Design: OSC Byte-Stream Interception (OSC 7 + OSC 133)

**Date:** 2026-04-07
**Scope:** Replace alacritty's EventLoop with custom PTY read thread, add OSC 7 (instant CWD) and OSC 133 (command boundaries) interception.
**Status:** Approved

---

## 1. Motivation

The current terminal backend uses alacritty_terminal's `EventLoop` to read PTY output and feed bytes to `Term`. This works but offers no hook to intercept bytes before processing. OSC 7 (CWD change) and OSC 133 (command boundaries) sequences emitted by shell integration scripts are silently ignored by alacritty since it doesn't recognize them.

Currently, CWD tracking uses `proc_pidinfo` polling at 1s intervals — functional but laggy. Command boundaries aren't tracked at all, blocking future features like scroll-to-prompt, command duration display, and click-to-select output.

## 2. Key Decisions

| Decision                   | Choice                                                 | Rationale                                                                                                                                               |
| -------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Byte interception approach | Custom PTY read thread replacing alacritty's EventLoop | Only way to see raw bytes before Term processes them. Same approach Zed uses.                                                                           |
| OSC parser                 | New minimal scanner in impulse-terminal                | Existing OscParser in impulse-core is designed for a different flow (output buffering, command timing). We just need to observe bytes, not modify them. |
| Byte stripping             | Don't strip OSC sequences                              | Alacritty ignores OSC 7/133 harmlessly. Simpler to pass all bytes through unchanged.                                                                    |
| Command boundary usage     | Emit events only, defer grid marking                   | Grid row tagging is a larger change to the binary buffer format and renderer. Events are the foundation; visual features come in a follow-up.           |
| CWD fallback               | Keep proc_pidinfo polling at 5s                        | Fallback for shells that don't emit OSC 7. Increased from 1s since OSC 7 handles the primary case.                                                      |

## 3. Architecture

### Current flow (being replaced):

```
PTY fd → alacritty EventLoop thread → Term::advance(bytes) → EventProxy events
```

### New flow:

```
PTY fd → our read thread → OscScanner observes bytes → emit OscEvents
                         → Processor::advance(&mut term, bytes) → EventProxy events
         ← BackendMsg channel (Input/Resize/Shutdown) ← main thread
```

## 4. OscScanner (`impulse-terminal/src/osc_scanner.rs`)

Minimal byte-by-byte state machine that watches for OSC 7 and OSC 133 sequences. Does NOT buffer or modify the byte stream — just observes and emits events.

### State machine:

- `Normal` — scanning bytes, watching for ESC (0x1B)
- `Escape` — saw ESC, expecting `]` (0x5D) to enter OSC mode
- `OscBody` — accumulating OSC payload bytes until BEL (0x07) or ST (ESC + `\`)

### Events emitted:

```rust
pub enum OscEvent {
    CwdChanged(String),    // OSC 7;file://hostname/path (URL-decoded)
    PromptStart,           // OSC 133;A
    CommandStart,          // OSC 133;B
    CommandEnd(i32),       // OSC 133;D;{exit_code}
}
```

### Parsing rules:

- `7;file://...` → URL-decode path, validate starts with `/`, emit `CwdChanged`
- `133;A` → emit `PromptStart`
- `133;B` → emit `CommandStart`
- `133;C` → ignored (redundant with B for our purposes)
- `133;D;{code}` → parse exit code, emit `CommandEnd`
- Anything else → ignored
- Max buffer: 4KB, reset on overflow to prevent unbounded growth

## 5. Custom PTY Read Thread (`backend.rs` changes)

Replace `alacritty_terminal::event_loop::EventLoop` with a custom thread.

### Thread responsibilities:

1. Read bytes from PTY fd in a loop
2. Pass bytes through `OscScanner`, drain emitted `OscEvent`s into the event channel as `TerminalEvent` variants
3. Lock `FairMutex<Term>`, call `Processor::advance(&mut term, &buf[..n])` to update terminal state
4. Send `TerminalEvent::Wakeup` after processing to trigger redraw
5. Listen on `crossbeam_channel` for commands:
   - `BackendMsg::Input(Vec<u8>)` — write bytes to PTY
   - `BackendMsg::Resize { cols, rows, cell_width, cell_height }` — resize PTY + Term
   - `BackendMsg::Shutdown` — exit loop, close PTY

### What gets removed:

- `alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg}` imports and usage
- `_pty_thread: Option<JoinHandle<(EventLoop<...>, State)>>` field
- `EventLoopSender` field

### What gets added:

- `cmd_tx: Sender<BackendMsg>` — send commands to read thread
- `_read_thread: Option<JoinHandle<()>>` — the custom read thread handle
- `OscScanner` instance (owned by the read thread)

### Public API unchanged:

- `write()`, `resize()`, `shutdown()` — now send `BackendMsg` instead of `Msg`
- `poll_events()` — unchanged, reads from same event channel
- `write_grid_to_buffer()` — unchanged
- All other methods — unchanged

## 6. New TerminalEvent Variants (`event.rs`)

```rust
pub enum TerminalEvent {
    // ... existing variants ...
    CwdChanged(String),
    PromptStart,
    CommandStart,
    CommandEnd(i32),
}
```

These serialize as JSON via serde and flow through the existing FFI `impulse_terminal_poll_events` → Swift `pollEvents()` pipeline with no FFI changes needed.

## 7. Swift Frontend Changes

### TerminalBackendEvent (TerminalBackend.swift):

Add cases:

```swift
case cwdChanged(String)
case promptStart
case commandStart
case commandEnd(Int32)
```

Update `pollEvents()` JSON parsing to handle the new variants.

### TerminalTab.swift event handler:

- `.cwdChanged(path)` — update `currentWorkingDirectory`, post `terminalCwdChanged` notification. Direct replacement for poll-based tracking.
- `.promptStart` / `.commandStart` / `.commandEnd(code)` — log for now. Store last command exit code for future status bar display.
- CWD poll timer interval: change from 1s to 5s (kept as fallback for shells without OSC 7).

## 8. File Changes

### New files:

| File                                  | Purpose                                            |
| ------------------------------------- | -------------------------------------------------- |
| `impulse-terminal/src/osc_scanner.rs` | Minimal OSC 7/133 byte-stream scanner (~120 lines) |

### Modified files:

| File                                               | Change                                                                        |
| -------------------------------------------------- | ----------------------------------------------------------------------------- |
| `impulse-terminal/src/backend.rs`                  | Replace alacritty EventLoop with custom PTY read thread, integrate OscScanner |
| `impulse-terminal/src/event.rs`                    | Add CwdChanged, PromptStart, CommandStart, CommandEnd variants                |
| `impulse-terminal/src/lib.rs`                      | Add `mod osc_scanner`                                                         |
| `impulse-macos/.../Terminal/TerminalTab.swift`     | Handle new events, reduce CWD poll to 5s fallback                             |
| `impulse-macos/.../Terminal/TerminalBackend.swift` | Add new event cases + JSON parsing                                            |

### Unchanged:

- `impulse-ffi/` — serde serialization handles new enum variants automatically
- `impulse-macos/CImpulseFFI/` — no new FFI functions
- `impulse-macos/.../Bridge/ImpulseCore.swift` — no changes

### Removed dependencies:

- `alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg}`

### New dependencies:

- `alacritty_terminal::vte::ansi::Processor` — direct use for feeding bytes to Term

## 9. Deferred Work

| Item                                                  | Why                                                   |
| ----------------------------------------------------- | ----------------------------------------------------- |
| Grid row semantic marking (prompt/input/output zones) | Requires binary buffer format changes + renderer work |
| Command duration in status bar                        | Needs CommandStart/CommandEnd timestamp tracking + UI |
| Scroll-to-previous-prompt                             | Needs prompt row tracking in grid                     |
| Strip OSC 7/133 from byte stream                      | Not needed since alacritty ignores them               |
