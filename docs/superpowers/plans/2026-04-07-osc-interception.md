# OSC Byte-Stream Interception — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace alacritty's EventLoop with a custom PTY read thread that intercepts OSC 7 (CWD) and OSC 133 (command boundaries) before feeding bytes to Term, enabling instant CWD tracking and command boundary events.

**Architecture:** A new `OscScanner` observes the byte stream for OSC sequences. The existing `EventLoop` from alacritty_terminal is replaced by a custom read thread that reads from the PTY, scans with `OscScanner`, then calls `Processor::advance()` to feed bytes into `Term`. Input, resize, and shutdown are handled via a `BackendMsg` channel. New `TerminalEvent` variants flow through the existing FFI/JSON pipeline to Swift.

**Tech Stack:** Rust (`alacritty_terminal` 0.26, `crossbeam-channel`), Swift (AppKit)

**Spec:** `docs/superpowers/specs/2026-04-07-osc-interception-design.md`

---

## File Structure

### New Files

| File                                  | Responsibility                                                                                 |
| ------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `impulse-terminal/src/osc_scanner.rs` | Minimal byte-by-byte state machine that detects OSC 7 and OSC 133 sequences, emits `OscEvent`s |

### Modified Files

| File                                               | Change                                                           |
| -------------------------------------------------- | ---------------------------------------------------------------- |
| `impulse-terminal/src/event.rs`                    | Add 4 new `TerminalEvent` variants                               |
| `impulse-terminal/src/backend.rs`                  | Replace alacritty EventLoop with custom read thread + OscScanner |
| `impulse-terminal/src/lib.rs`                      | Add `mod osc_scanner`                                            |
| `impulse-macos/.../Terminal/TerminalBackend.swift` | Add new event cases to enum + JSON parsing                       |
| `impulse-macos/.../Terminal/TerminalTab.swift`     | Handle new events, reduce CWD poll to 5s fallback                |

---

## Task 1: OscScanner with Tests

**Files:**

- Create: `impulse-terminal/src/osc_scanner.rs`
- Modify: `impulse-terminal/src/lib.rs`

- [ ] **Step 1: Create osc_scanner module**

Create `impulse-terminal/src/osc_scanner.rs`:

```rust
//! Minimal OSC 7/133 byte-stream scanner.
//!
//! Watches a byte stream for OSC escape sequences and emits events.
//! Does NOT modify or buffer the byte stream — all bytes pass through
//! unchanged. alacritty_terminal ignores OSC 7/133 harmlessly.

/// Events emitted by the OSC scanner.
#[derive(Clone, Debug, PartialEq)]
pub enum OscEvent {
    /// OSC 7: working directory changed.
    CwdChanged(String),
    /// OSC 133;A: shell prompt started.
    PromptStart,
    /// OSC 133;B: command execution started.
    CommandStart,
    /// OSC 133;D;{code}: command execution ended with exit code.
    CommandEnd(i32),
}

/// Scanner state machine.
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Normal,
    Escape,   // Saw ESC (0x1B), expecting ']' for OSC
    OscBody,  // Inside OSC payload, collecting until BEL or ST
}

/// Maximum OSC payload size before we reset (prevents unbounded growth).
const MAX_OSC_LEN: usize = 4096;

/// Scans a byte stream for OSC 7 and OSC 133 sequences.
pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
    events: Vec<OscEvent>,
}

impl OscScanner {
    pub fn new() -> Self {
        Self {
            state: State::Normal,
            buf: Vec::with_capacity(256),
            events: Vec::new(),
        }
    }

    /// Scan a chunk of bytes. Call `drain_events()` after to collect results.
    pub fn scan(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.state {
                State::Normal => {
                    if b == 0x1B {
                        self.state = State::Escape;
                    }
                }
                State::Escape => {
                    if b == b']' {
                        self.state = State::OscBody;
                        self.buf.clear();
                    } else {
                        // Not an OSC sequence, back to normal.
                        self.state = State::Normal;
                    }
                }
                State::OscBody => {
                    if b == 0x07 {
                        // BEL terminates the OSC sequence.
                        self.dispatch_osc();
                        self.state = State::Normal;
                    } else if b == 0x1B {
                        // Could be ST (ESC \). Peek: if next byte is '\',
                        // we'll handle it. For now, treat ESC as start of
                        // a new sequence and dispatch what we have if the
                        // buf looks like a valid OSC.
                        // Actually, ST is ESC + '\'. We need to handle the
                        // two-byte terminator. Switch to Escape state; if
                        // the next byte is '\' we dispatch, if ']' we start
                        // a new OSC, otherwise back to Normal.
                        self.state = State::Escape;
                        // Check if we had a valid OSC to dispatch.
                        // We'll dispatch on the assumption this ESC ends it.
                        self.dispatch_osc();
                    } else if self.buf.len() < MAX_OSC_LEN {
                        self.buf.push(b);
                    } else {
                        // Overflow, reset.
                        self.buf.clear();
                        self.state = State::Normal;
                    }
                }
            }
        }
    }

    /// Drain all events emitted since the last drain.
    pub fn drain_events(&mut self) -> Vec<OscEvent> {
        std::mem::take(&mut self.events)
    }

    fn dispatch_osc(&mut self) {
        if self.buf.is_empty() {
            return;
        }

        // Check for OSC 7 (CWD): "7;file://..."
        if self.buf.starts_with(b"7;") {
            if let Some(path) = Self::parse_osc7(&self.buf[2..]) {
                self.events.push(OscEvent::CwdChanged(path));
            }
        }
        // Check for OSC 133 (shell integration): "133;X" or "133;D;code"
        else if self.buf.starts_with(b"133;") && self.buf.len() >= 5 {
            match self.buf[4] {
                b'A' => self.events.push(OscEvent::PromptStart),
                b'B' => self.events.push(OscEvent::CommandStart),
                b'C' => {} // Ignored (redundant with B)
                b'D' => {
                    let code = if self.buf.len() > 6 && self.buf[5] == b';' {
                        std::str::from_utf8(&self.buf[6..])
                            .ok()
                            .and_then(|s| s.parse::<i32>().ok())
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    self.events.push(OscEvent::CommandEnd(code));
                }
                _ => {}
            }
        }

        self.buf.clear();
    }

    /// Parse OSC 7 payload: "file://hostname/path" → URL-decoded path.
    fn parse_osc7(payload: &[u8]) -> Option<String> {
        let s = std::str::from_utf8(payload).ok()?;

        // Strip "file://" prefix.
        let rest = s.strip_prefix("file://")?;

        // Skip hostname (everything up to the first '/').
        let path_start = rest.find('/')?;
        let encoded_path = &rest[path_start..];

        // URL-decode the path.
        let decoded = Self::url_decode(encoded_path)?;

        // Validate: must be absolute and non-empty.
        if decoded.starts_with('/') && !decoded.is_empty() {
            Some(decoded)
        } else {
            None
        }
    }

    /// Decode percent-encoded UTF-8 string.
    fn url_decode(input: &str) -> Option<String> {
        let mut bytes = Vec::with_capacity(input.len());
        let mut chars = input.bytes();
        while let Some(b) = chars.next() {
            if b == b'%' {
                let hi = chars.next()?;
                let lo = chars.next()?;
                let hex = [hi, lo];
                let val = u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?;
                bytes.push(val);
            } else {
                bytes.push(b);
            }
        }
        String::from_utf8(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_osc7_cwd() {
        let mut scanner = OscScanner::new();
        // ESC ] 7 ; file://hostname/Users/test BEL
        let seq = b"\x1b]7;file://myhost/Users/test\x07";
        scanner.scan(seq);
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CwdChanged("/Users/test".to_string())]);
    }

    #[test]
    fn test_osc7_url_encoded() {
        let mut scanner = OscScanner::new();
        let seq = b"\x1b]7;file://host/Users/my%20dir/foo%2Fbar\x07";
        scanner.scan(seq);
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CwdChanged("/Users/my dir/foo/bar".to_string())]);
    }

    #[test]
    fn test_osc133_prompt_start() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;A\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::PromptStart]);
    }

    #[test]
    fn test_osc133_command_start() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;B\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandStart]);
    }

    #[test]
    fn test_osc133_command_end_with_code() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;D;0\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandEnd(0)]);
    }

    #[test]
    fn test_osc133_command_end_nonzero() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;D;127\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandEnd(127)]);
    }

    #[test]
    fn test_osc133_c_ignored() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133;C\x07");
        let events = scanner.drain_events();
        assert!(events.is_empty());
    }

    #[test]
    fn test_mixed_bytes_and_osc() {
        let mut scanner = OscScanner::new();
        // Normal output, then OSC 133;A, then more output, then OSC 7
        let data = b"hello\x1b]133;A\x07world\x1b]7;file://h/tmp\x07";
        scanner.scan(data);
        let events = scanner.drain_events();
        assert_eq!(events, vec![
            OscEvent::PromptStart,
            OscEvent::CwdChanged("/tmp".to_string()),
        ]);
    }

    #[test]
    fn test_st_terminator() {
        let mut scanner = OscScanner::new();
        // OSC terminated by ST (ESC \) instead of BEL
        scanner.scan(b"\x1b]133;A\x1b\\");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::PromptStart]);
    }

    #[test]
    fn test_split_across_chunks() {
        let mut scanner = OscScanner::new();
        scanner.scan(b"\x1b]133");
        assert!(scanner.drain_events().is_empty());
        scanner.scan(b";B\x07");
        let events = scanner.drain_events();
        assert_eq!(events, vec![OscEvent::CommandStart]);
    }

    #[test]
    fn test_overflow_resets() {
        let mut scanner = OscScanner::new();
        // Start an OSC sequence then feed MAX_OSC_LEN+1 bytes
        scanner.scan(b"\x1b]");
        let overflow = vec![b'x'; MAX_OSC_LEN + 1];
        scanner.scan(&overflow);
        // Should have reset, no events
        assert!(scanner.drain_events().is_empty());
        // Scanner should be back to Normal, can parse new sequences
        scanner.scan(b"\x1b]133;A\x07");
        assert_eq!(scanner.drain_events(), vec![OscEvent::PromptStart]);
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

In `impulse-terminal/src/lib.rs`, add after `mod search;`:

```rust
pub mod osc_scanner;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p impulse-terminal`
Expected: All tests pass (3 existing buffer tests + 11 new OscScanner tests).

- [ ] **Step 4: Commit**

```bash
git add impulse-terminal/
git commit -m "Add OscScanner for OSC 7 and OSC 133 byte-stream detection"
```

---

## Task 2: Add New TerminalEvent Variants

**Files:**

- Modify: `impulse-terminal/src/event.rs`

- [ ] **Step 1: Add new variants to TerminalEvent**

In `impulse-terminal/src/event.rs`, add these variants before the closing `}`:

```rust
    /// Working directory changed (OSC 7).
    CwdChanged(String),
    /// Shell prompt started (OSC 133;A).
    PromptStart,
    /// Command execution started (OSC 133;B).
    CommandStart,
    /// Command execution ended with exit code (OSC 133;D).
    CommandEnd(i32),
```

The full enum becomes:

```rust
#[derive(Clone, Debug, Serialize)]
pub enum TerminalEvent {
    Wakeup,
    TitleChanged(String),
    ResetTitle,
    Bell,
    ChildExited(i32),
    ClipboardStore(String),
    ClipboardLoad,
    CursorBlinkingChange,
    Exit,
    CwdChanged(String),
    PromptStart,
    CommandStart,
    CommandEnd(i32),
    /// Internal: Term sends PtyWrite for device query responses (e.g., DA1).
    /// Filtered out in poll_events() and forwarded back to the PTY as input.
    /// Not visible to Swift — serialized but never reaches the frontend.
    PtyWrite(String),
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p impulse-terminal`
Expected: Compiles. Warnings about unused variants are fine — they'll be used in Task 3.

- [ ] **Step 3: Commit**

```bash
git add impulse-terminal/src/event.rs
git commit -m "Add CwdChanged, PromptStart, CommandStart, CommandEnd terminal events"
```

---

## Task 3: Replace EventLoop with Custom PTY Read Thread

**Files:**

- Modify: `impulse-terminal/src/backend.rs`

This is the largest task. The implementing agent must read the current `backend.rs` first, then make these changes:

- [ ] **Step 1: Update imports**

Replace the imports at the top of `backend.rs`. Remove:

```rust
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
```

Add:

```rust
use std::io::Read;
use alacritty_terminal::vte::ansi::Processor;
```

Keep all other existing imports.

- [ ] **Step 2: Remove pty_write_tx from EventProxy**

The `EventProxy` struct currently has a `pty_write_tx` field for `PtyWrite` events. Since we own the read thread now, we handle PtyWrite directly. Change `EventProxy`:

```rust
#[derive(Clone)]
struct EventProxy {
    event_tx: Sender<TerminalEvent>,
}
```

Update its `EventListener::send_event` impl — remove the `PtyWrite` arm and send PtyWrite text directly to the event channel as input:

```rust
impl EventListener for EventProxy {
    fn send_event(&self, event: AlacEvent) {
        match event {
            AlacEvent::PtyWrite(text) => {
                // PtyWrite events are responses to queries (e.g., DA1).
                // Forward the text as input bytes to be written to the PTY.
                // We send it as a special event that the read thread will pick up.
                let _ = self.event_tx.send(TerminalEvent::PtyWrite(text));
            }
            AlacEvent::Wakeup => { let _ = self.event_tx.send(TerminalEvent::Wakeup); }
            AlacEvent::Title(title) => { let _ = self.event_tx.send(TerminalEvent::TitleChanged(title)); }
            AlacEvent::ResetTitle => { let _ = self.event_tx.send(TerminalEvent::ResetTitle); }
            AlacEvent::Bell => { let _ = self.event_tx.send(TerminalEvent::Bell); }
            AlacEvent::Exit => { let _ = self.event_tx.send(TerminalEvent::Exit); }
            AlacEvent::ChildExit(status) => {
                let code = status.code().unwrap_or(-1);
                let _ = self.event_tx.send(TerminalEvent::ChildExited(code));
            }
            AlacEvent::ClipboardStore(_, text) => { let _ = self.event_tx.send(TerminalEvent::ClipboardStore(text)); }
            AlacEvent::ClipboardLoad(_, _) => { let _ = self.event_tx.send(TerminalEvent::ClipboardLoad); }
            AlacEvent::CursorBlinkingChange => { let _ = self.event_tx.send(TerminalEvent::CursorBlinkingChange); }
            AlacEvent::ColorRequest(_, _)
            | AlacEvent::TextAreaSizeRequest(_)
            | AlacEvent::MouseCursorDirty => {}
        }
    }
}
```

Also add `PtyWrite(String)` to `TerminalEvent` in `event.rs` (this is an internal variant, serialized but filtered out in `poll_events`).

- [ ] **Step 3: Add BackendMsg enum**

Add after the `SelectionKind` definition:

```rust
/// Messages sent from the main thread to the PTY read thread.
enum BackendMsg {
    /// Write input bytes to the PTY.
    Input(Vec<u8>),
    /// Resize the PTY and terminal grid.
    Resize { cols: u16, rows: u16, cell_width: u16, cell_height: u16 },
    /// Shut down the read thread.
    Shutdown,
}
```

- [ ] **Step 4: Rewrite TerminalBackend struct fields**

Replace:

```rust
pub struct TerminalBackend {
    term: Arc<FairMutex<Term<EventProxy>>>,
    event_loop_sender: EventLoopSender,
    event_rx: Receiver<TerminalEvent>,
    pty_write_rx: Receiver<String>,
    _pty_thread: Option<JoinHandle<(EventLoop<tty::Pty, EventProxy>, alacritty_terminal::event_loop::State)>>,
    cols: u16,
    rows: u16,
    colors: ConfiguredColors,
    child_pid: u32,
    search: TerminalSearch,
}
```

With:

```rust
pub struct TerminalBackend {
    term: Arc<FairMutex<Term<EventProxy>>>,
    cmd_tx: Sender<BackendMsg>,
    event_rx: Receiver<TerminalEvent>,
    _read_thread: Option<JoinHandle<()>>,
    cols: u16,
    rows: u16,
    colors: ConfiguredColors,
    child_pid: u32,
    search: TerminalSearch,
}
```

- [ ] **Step 5: Rewrite TerminalBackend::new()**

Replace the constructor. The key change: instead of `EventLoop::new().spawn()`, spawn our own thread.

```rust
impl TerminalBackend {
    pub fn new(
        config: TerminalConfig,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<Self, String> {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<BackendMsg>();
        let proxy = EventProxy { event_tx: event_tx.clone() };

        let alac_config = config.to_alacritty_config();
        let pty_options = config.to_pty_options();
        let colors = ConfiguredColors::from_config(&config);

        let size = TermSize { columns: cols as usize, screen_lines: rows as usize };
        let term = Term::new(alac_config, &size, proxy);
        let term = Arc::new(FairMutex::new(term));

        let window_size = WindowSize { num_lines: rows, num_cols: cols, cell_width, cell_height };
        let mut pty = tty::new(&pty_options, window_size, 0)
            .map_err(|e| format!("Failed to create PTY: {e}"))?;
        let child_pid = pty.child().id();

        // Spawn custom read thread.
        let term_clone = Arc::clone(&term);
        let read_thread = std::thread::Builder::new()
            .name("impulse-pty-reader".into())
            .spawn(move || {
                Self::read_loop(pty, term_clone, event_tx, cmd_rx);
            })
            .map_err(|e| format!("Failed to spawn read thread: {e}"))?;

        Ok(Self {
            term,
            cmd_tx,
            event_rx,
            _read_thread: Some(read_thread),
            cols,
            rows,
            colors,
            child_pid,
            search: TerminalSearch::new(),
        })
    }
```

- [ ] **Step 6: Add the read_loop function**

Add as a private associated function on `TerminalBackend`:

```rust
    /// PTY read loop. Runs on a dedicated thread.
    fn read_loop(
        mut pty: tty::Pty,
        term: Arc<FairMutex<Term<EventProxy>>>,
        event_tx: Sender<TerminalEvent>,
        cmd_rx: Receiver<BackendMsg>,
    ) {
        let mut buf = [0u8; 0x10000]; // 64KB read buffer
        let mut processor = Processor::new();
        let mut scanner = crate::osc_scanner::OscScanner::new();
        let reader = pty.file().try_clone().expect("Failed to clone PTY fd");
        let mut reader = std::io::BufReader::new(reader);

        loop {
            // Check for commands (non-blocking).
            while let Ok(msg) = cmd_rx.try_recv() {
                match msg {
                    BackendMsg::Input(data) => {
                        use std::io::Write;
                        let _ = pty.file().write_all(&data);
                    }
                    BackendMsg::Resize { cols, rows, cell_width, cell_height } => {
                        let ws = WindowSize { num_lines: rows, num_cols: cols, cell_width, cell_height };
                        pty.on_resize(ws);
                        let size = TermSize { columns: cols as usize, screen_lines: rows as usize };
                        term.lock().resize(size);
                    }
                    BackendMsg::Shutdown => return,
                }
            }

            // Read from PTY (blocking with short timeout via non-blocking + sleep).
            match reader.read(&mut buf) {
                Ok(0) => {
                    // EOF — child process exited.
                    let _ = event_tx.send(TerminalEvent::Exit);
                    return;
                }
                Ok(n) => {
                    // Scan for OSC sequences.
                    scanner.scan(&buf[..n]);
                    for osc_event in scanner.drain_events() {
                        match osc_event {
                            crate::osc_scanner::OscEvent::CwdChanged(path) => {
                                let _ = event_tx.send(TerminalEvent::CwdChanged(path));
                            }
                            crate::osc_scanner::OscEvent::PromptStart => {
                                let _ = event_tx.send(TerminalEvent::PromptStart);
                            }
                            crate::osc_scanner::OscEvent::CommandStart => {
                                let _ = event_tx.send(TerminalEvent::CommandStart);
                            }
                            crate::osc_scanner::OscEvent::CommandEnd(code) => {
                                let _ = event_tx.send(TerminalEvent::CommandEnd(code));
                            }
                        }
                    }

                    // Feed bytes to alacritty's terminal state machine.
                    {
                        let mut term = term.lock();
                        processor.advance(&mut *term, &buf[..n]);
                    }

                    // Notify frontend that content changed.
                    let _ = event_tx.send(TerminalEvent::Wakeup);

                    // Check for PtyWrite events (responses to device queries).
                    // These are sent by Term via EventProxy and need to be
                    // written back to the PTY.
                    // We drain them from the event channel here.
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No data available, check commands and retry.
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                Err(_) => {
                    // PTY read error — child likely exited.
                    let _ = event_tx.send(TerminalEvent::Exit);
                    return;
                }
            }
        }
    }
```

**Important note for the implementer:** The `pty.file()` returns `&File`. We need to clone the fd for the reader while keeping the original for writes. Use `pty.file().try_clone()` to get a separate `File` handle for reading. The PTY write happens via the original `pty` (through `BackendMsg::Input`). The `pty.on_resize()` needs `&mut self` so the pty must be owned by the read thread.

Also: `Processor::new()` is from `alacritty_terminal::vte::ansi::Processor`. Check the exact import path — it may be re-exported differently in 0.26.

- [ ] **Step 7: Update write(), resize(), shutdown(), drain_pty_writes()**

Replace these methods:

```rust
    pub fn write(&self, data: &[u8]) {
        if !data.is_empty() {
            let _ = self.cmd_tx.send(BackendMsg::Input(data.to_vec()));
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        if cols == self.cols && rows == self.rows { return; }
        self.cols = cols;
        self.rows = rows;
        let _ = self.cmd_tx.send(BackendMsg::Resize { cols, rows, cell_width, cell_height });
    }

    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(BackendMsg::Shutdown);
    }
```

Remove `drain_pty_writes()` entirely — it's no longer needed.

Update `poll_events()` to filter out the internal `PtyWrite` variant:

```rust
    pub fn poll_events(&self) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = self.event_rx.try_recv() {
            match &ev {
                TerminalEvent::PtyWrite(text) => {
                    // PtyWrite responses need to go back to the PTY.
                    let _ = self.cmd_tx.send(BackendMsg::Input(text.as_bytes().to_vec()));
                }
                _ => events.push(ev),
            }
        }
        events
    }
```

- [ ] **Step 8: Verify it compiles**

Run: `cargo build -p impulse-terminal`
Expected: Compiles. If the `Processor` import path is wrong, check the alacritty_terminal docs and adapt.

- [ ] **Step 9: Build the full FFI**

Run: `cargo build -p impulse-ffi`
Expected: Compiles.

- [ ] **Step 10: Run tests**

Run: `cargo test -p impulse-terminal`
Expected: All tests pass (buffer tests + OscScanner tests).

- [ ] **Step 11: Commit**

```bash
git add impulse-terminal/
git commit -m "Replace alacritty EventLoop with custom PTY read thread and OscScanner"
```

---

## Task 4: Swift Event Handling

**Files:**

- Modify: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift`
- Modify: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift`

- [ ] **Step 1: Add new event cases to TerminalBackendEvent**

In `TerminalBackend.swift`, add to the `TerminalBackendEvent` enum (after `case exit`):

```swift
    case cwdChanged(String)
    case promptStart
    case commandStart
    case commandEnd(Int32)
```

- [ ] **Step 2: Update pollEvents() JSON parsing**

In the `pollEvents()` method of `TerminalBackend`, add handling for the new variants.

In the string-case section (where `"Wakeup"`, `"Bell"`, etc. are matched), add:

```swift
case "PromptStart": events.append(.promptStart)
case "CommandStart": events.append(.commandStart)
```

In the dict-case section (where `"TitleChanged"`, `"ChildExited"`, etc. are matched), add:

```swift
else if let path = dict["CwdChanged"] as? String {
    events.append(.cwdChanged(path))
} else if let code = dict["CommandEnd"] as? Int {
    events.append(.commandEnd(Int32(code)))
}
```

- [ ] **Step 3: Handle new events in TerminalTab**

In `TerminalTab.swift`, find the `handleBackendEvent` method (or the `renderer.onEvent` callback). Add handling for the new events:

```swift
case .cwdChanged(let path):
    currentWorkingDirectory = path
    NotificationCenter.default.post(
        name: .terminalCwdChanged,
        object: self,
        userInfo: ["directory": path]
    )

case .promptStart:
    break // Future: scroll-to-prompt navigation

case .commandStart:
    break // Future: command timing

case .commandEnd(_):
    break // Future: exit code display in status bar
```

- [ ] **Step 4: Reduce CWD poll interval to 5s**

In `TerminalTab.swift`, find `startCwdPolling()` and change the timer interval from `1.0` to `5.0`:

```swift
cwdPollTimer = Timer.scheduledTimer(withTimeInterval: 5.0, repeats: true) { ...
```

- [ ] **Step 5: Build the app**

Run: `./impulse-macos/build.sh`
Expected: Build succeeds.

- [ ] **Step 6: Commit**

```bash
git add impulse-macos/
git commit -m "Handle OSC 7 and OSC 133 events in Swift frontend"
```

---

## Task 5: Build, Smoke Test, and Fix Issues

**Files:**

- Potentially any file from Tasks 1-4

- [ ] **Step 1: Full build**

Run: `cargo build -p impulse-core -p impulse-editor -p impulse-ffi -p impulse-terminal && cargo test -p impulse-terminal && ./impulse-macos/build.sh`
Expected: All pass.

- [ ] **Step 2: Build and launch dev app**

Run: `./impulse-macos/build.sh --dev && open "dist/Impulse Dev.app"`

- [ ] **Step 3: Smoke test terminal basics**

Verify these still work after the EventLoop replacement:

- Terminal opens with shell prompt
- Typing works, arrow keys work
- Ctrl+C sends interrupt
- Colors render (`ls --color`)
- Scrollback works
- Copy/paste (Cmd+C/V)
- Terminal splits
- Tab close (no beach ball)
- TUI apps (vim, htop)

- [ ] **Step 4: Test CWD tracking**

In the terminal:

```bash
cd /tmp
cd ~/Code
cd /
```

Watch the status bar — CWD should update instantly (within the same frame) instead of with a 1s delay. If it updates but with a slight delay, the OSC 7 interception is working but there may be a buffering issue.

- [ ] **Step 5: Verify OSC 133 events**

The command boundary events aren't visible in the UI yet, but you can verify they work by checking the console log. Run a command and look for log output confirming PromptStart/CommandStart/CommandEnd events are being received. If no logging exists, add temporary `os_log` calls in the Swift event handler and remove them after verification.

- [ ] **Step 6: Fix any issues**

Common issues to watch for:

- **PTY read blocking:** If the terminal is unresponsive, the read loop may be blocking. Check that `pty.file().try_clone()` works and the reader is non-blocking or uses appropriate timeouts.
- **PtyWrite responses:** If device query responses (like cursor position reports) don't work, the PtyWrite event forwarding in `poll_events()` may not be triggering. Verify the flow: Term sends PtyWrite → EventProxy → event channel → poll_events drains and sends BackendMsg::Input → read thread writes to PTY.
- **Resize not working:** If terminal doesn't resize, verify BackendMsg::Resize reaches the read thread and both `pty.on_resize()` and `term.resize()` are called.

- [ ] **Step 7: Commit fixes**

```bash
git add -A
git commit -m "Fix issues found during OSC interception smoke testing"
```
