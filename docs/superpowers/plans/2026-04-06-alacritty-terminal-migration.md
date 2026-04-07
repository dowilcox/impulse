# Replace SwiftTerm with alacritty_terminal — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace SwiftTerm on macOS with a unified Rust terminal backend using `alacritty_terminal`, exposed via C FFI to a custom CoreText-based NSView renderer.

**Architecture:** New `impulse-terminal` Rust crate wraps `alacritty_terminal::Term` with a platform-agnostic API. `impulse-ffi` exposes 18 C functions. Swift frontend gets three new files: `TerminalBackend.swift` (FFI wrapper with binary buffer), `TerminalRenderer.swift` (CoreText run-based NSView), `KeyEncoder.swift` (keyboard input). `TerminalTab.swift` is rewritten. SwiftTerm dependency removed.

**Tech Stack:** Rust (`alacritty_terminal` 0.26.0, `crossbeam-channel`), Swift (CoreText, CVDisplayLink, AppKit), C FFI

**Spec:** `docs/superpowers/specs/2026-04-06-alacritty-terminal-migration-design.md`

---

## File Structure

### New Files

| File                                                | Responsibility                                                                                                               |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `impulse-terminal/Cargo.toml`                       | Crate manifest                                                                                                               |
| `impulse-terminal/src/lib.rs`                       | Public API re-exports                                                                                                        |
| `impulse-terminal/src/grid.rs`                      | Platform-agnostic types: `RgbColor`, `CellFlags`, `CursorShape`, `CursorState`, `TerminalMode`, `StyledCell`, `GridSnapshot` |
| `impulse-terminal/src/buffer.rs`                    | Binary buffer format: header + cell packing into `&mut [u8]`                                                                 |
| `impulse-terminal/src/config.rs`                    | `TerminalConfig` + translation to alacritty types                                                                            |
| `impulse-terminal/src/event.rs`                     | `TerminalEvent` enum                                                                                                         |
| `impulse-terminal/src/backend.rs`                   | `TerminalBackend`: owns `Term`, event loop, color resolution, selection, scroll, search                                      |
| `impulse-terminal/src/search.rs`                    | `TerminalSearch`: wraps alacritty `RegexSearch`                                                                              |
| `impulse-macos/.../Terminal/TerminalBackend.swift`  | Swift FFI wrapper + `GridBufferReader`                                                                                       |
| `impulse-macos/.../Terminal/TerminalRenderer.swift` | CoreText NSView: run-based drawing, input, refresh                                                                           |
| `impulse-macos/.../Terminal/KeyEncoder.swift`       | `NSEvent` → terminal escape sequence translation                                                                             |

### Modified Files

| File                                                          | Change                                                    |
| ------------------------------------------------------------- | --------------------------------------------------------- |
| `Cargo.toml:2`                                                | Add `impulse-terminal` to workspace members               |
| `impulse-ffi/Cargo.toml:11-23`                                | Add `impulse-terminal` + `crossbeam-channel` dependencies |
| `impulse-ffi/src/lib.rs`                                      | Add 18 terminal FFI functions (append after line 1350)    |
| `impulse-macos/CImpulseFFI/include/impulse_ffi.h:74-78`       | Add C declarations before `#endif`                        |
| `impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift`   | Add terminal bridge methods (append before closing `}`)   |
| `impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift` | Full rewrite (780→~350 lines)                             |
| `impulse-macos/Sources/ImpulseApp/MainWindow.swift:842-862`   | Wire search TODO stubs to backend                         |
| `impulse-macos/Sources/ImpulseApp/TabManager.swift:2`         | Remove `import SwiftTerm`                                 |
| `impulse-macos/Package.swift:10,20`                           | Remove SwiftTerm dependency                               |

---

## Task 1: Create `impulse-terminal` Crate with Grid Types

**Files:**

- Create: `impulse-terminal/Cargo.toml`
- Create: `impulse-terminal/src/lib.rs`
- Create: `impulse-terminal/src/grid.rs`
- Modify: `Cargo.toml:2`

- [ ] **Step 1: Create crate manifest**

Create `impulse-terminal/Cargo.toml`:

```toml
[package]
name = "impulse-terminal"
version = "0.19.1"
edition = "2021"
description = "Terminal emulation backend for Impulse, built on alacritty_terminal"
license = "GPL-3.0-only"

[dependencies]
alacritty_terminal = "0.26"
crossbeam-channel = "0.5"
serde = { workspace = true }
serde_json = { workspace = true }
log = { workspace = true }
```

- [ ] **Step 2: Create grid types**

Create `impulse-terminal/src/grid.rs`:

```rust
//! Platform-agnostic grid types for rendering.
//!
//! These types are the interface between the terminal backend and platform
//! renderers. They have no dependencies on alacritty_terminal, so frontends
//! never need to link against it.

use serde::{Deserialize, Serialize};

/// RGB color value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

bitflags::bitflags! {
    /// Cell attribute flags (transmitted as u16 in the binary buffer).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
    pub struct CellFlags: u16 {
        const BOLD              = 1 << 0;
        const ITALIC            = 1 << 1;
        const UNDERLINE         = 1 << 2;
        const STRIKETHROUGH     = 1 << 3;
        const DIM               = 1 << 4;
        const INVERSE           = 1 << 5;
        const HIDDEN            = 1 << 6;
        const WIDE_CHAR         = 1 << 7;
        const WIDE_CHAR_SPACER  = 1 << 8;
        const DOUBLE_UNDERLINE  = 1 << 9;
        const UNDERCURL         = 1 << 10;
        const DOTTED_UNDERLINE  = 1 << 11;
        const DASHED_UNDERLINE  = 1 << 12;
    }
}

/// Cursor shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum CursorShape {
    Block = 0,
    Beam = 1,
    Underline = 2,
    HollowBlock = 3,
    Hidden = 4,
}

/// Current cursor state for rendering.
#[derive(Clone, Debug, Serialize)]
pub struct CursorState {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShape,
    pub visible: bool,
}

bitflags::bitflags! {
    /// Terminal mode flags relevant to renderers and input handling.
    /// Transmitted as u16 in the binary buffer header.
    #[derive(Clone, Copy, Debug, Default, Serialize)]
    pub struct TerminalMode: u16 {
        const SHOW_CURSOR         = 1 << 0;
        const APP_CURSOR          = 1 << 1;
        const APP_KEYPAD          = 1 << 2;
        const MOUSE_REPORT_CLICK  = 1 << 3;
        const MOUSE_MOTION        = 1 << 4;
        const MOUSE_DRAG          = 1 << 5;
        const MOUSE_SGR           = 1 << 6;
        const BRACKETED_PASTE     = 1 << 7;
        const FOCUS_IN_OUT        = 1 << 8;
        const ALT_SCREEN          = 1 << 9;
        const LINE_WRAP           = 1 << 10;
    }
}
```

- [ ] **Step 3: Create lib.rs with re-exports**

Create `impulse-terminal/src/lib.rs`:

```rust
//! Terminal emulation backend for Impulse, built on alacritty_terminal.
//!
//! This crate provides a platform-agnostic terminal backend. Frontends only
//! need to render a grid of styled cells and forward input events.

mod grid;

pub use grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};
```

- [ ] **Step 4: Add bitflags dependency**

Update `impulse-terminal/Cargo.toml` — add `bitflags = "2"` to `[dependencies]`.

- [ ] **Step 5: Add crate to workspace**

In `Cargo.toml` (workspace root), change line 2:

```toml
members = ["impulse-core", "impulse-editor", "impulse-linux", "impulse-ffi", "impulse-terminal"]
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo build -p impulse-terminal`
Expected: Compiles with no errors.

- [ ] **Step 7: Commit**

```bash
git add impulse-terminal/ Cargo.toml Cargo.lock
git commit -m "Add impulse-terminal crate with platform-agnostic grid types"
```

---

## Task 2: Binary Buffer Format

**Files:**

- Create: `impulse-terminal/src/buffer.rs`
- Modify: `impulse-terminal/src/lib.rs`

- [ ] **Step 1: Create buffer module**

Create `impulse-terminal/src/buffer.rs`:

```rust
//! Binary buffer format for efficient grid snapshot transport across FFI.
//!
//! Layout:
//!   Header (variable size):
//!     [0..2)   cols (u16 LE)
//!     [2..4)   lines (u16 LE)
//!     [4..6)   cursor row (u16 LE)
//!     [6..8)   cursor col (u16 LE)
//!     [8]      cursor shape (u8)
//!     [9]      cursor visible (u8: 0/1)
//!     [10..12) mode flags (u16 LE)
//!     [12..14) selection range count N (u16 LE)
//!     [14..16) search match range count M (u16 LE)
//!     [16 .. 16+N*6)  selection ranges (row u16 + start_col u16 + end_col u16 each)
//!     [16+N*6 .. 16+N*6+M*6)  search match ranges (same format)
//!   Cell data (row-major, 12 bytes per cell):
//!     [0..4)   character (u32 LE, UTF-32 codepoint)
//!     [4..7)   fg RGB
//!     [7..10)  bg RGB
//!     [10..12) flags (u16 LE, CellFlags)

use crate::grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};

/// Bytes per cell in the binary buffer.
pub const CELL_STRIDE: usize = 12;

/// Fixed header size (before variable-length selection/search ranges).
pub const FIXED_HEADER_SIZE: usize = 16;

/// Bytes per range entry (row u16 + start_col u16 + end_col u16).
pub const RANGE_ENTRY_SIZE: usize = 6;

/// A range highlight (selection or search match).
#[derive(Clone, Copy, Debug)]
pub struct HighlightRange {
    pub row: u16,
    pub start_col: u16,
    pub end_col: u16,
}

/// Calculate the buffer size needed for a grid of the given dimensions.
pub fn buffer_size(cols: u16, lines: u16, selection_count: u16, search_count: u16) -> usize {
    FIXED_HEADER_SIZE
        + (selection_count as usize + search_count as usize) * RANGE_ENTRY_SIZE
        + (cols as usize * lines as usize * CELL_STRIDE)
}

/// Write the grid header into the buffer. Returns the offset where cell data begins.
pub fn write_header(
    buf: &mut [u8],
    cols: u16,
    lines: u16,
    cursor: &CursorState,
    mode: TerminalMode,
    selection_ranges: &[HighlightRange],
    search_ranges: &[HighlightRange],
) -> usize {
    let sel_count = selection_ranges.len() as u16;
    let search_count = search_ranges.len() as u16;

    buf[0..2].copy_from_slice(&cols.to_le_bytes());
    buf[2..4].copy_from_slice(&lines.to_le_bytes());
    buf[4..6].copy_from_slice(&(cursor.row as u16).to_le_bytes());
    buf[6..8].copy_from_slice(&(cursor.col as u16).to_le_bytes());
    buf[8] = cursor.shape as u8;
    buf[9] = cursor.visible as u8;
    buf[10..12].copy_from_slice(&mode.bits().to_le_bytes());
    buf[12..14].copy_from_slice(&sel_count.to_le_bytes());
    buf[14..16].copy_from_slice(&search_count.to_le_bytes());

    let mut offset = FIXED_HEADER_SIZE;
    for range in selection_ranges {
        buf[offset..offset + 2].copy_from_slice(&range.row.to_le_bytes());
        buf[offset + 2..offset + 4].copy_from_slice(&range.start_col.to_le_bytes());
        buf[offset + 4..offset + 6].copy_from_slice(&range.end_col.to_le_bytes());
        offset += RANGE_ENTRY_SIZE;
    }
    for range in search_ranges {
        buf[offset..offset + 2].copy_from_slice(&range.row.to_le_bytes());
        buf[offset + 2..offset + 4].copy_from_slice(&range.start_col.to_le_bytes());
        buf[offset + 4..offset + 6].copy_from_slice(&range.end_col.to_le_bytes());
        offset += RANGE_ENTRY_SIZE;
    }
    offset
}

/// Write a single cell into the buffer at the given offset.
#[inline]
pub fn write_cell(buf: &mut [u8], offset: usize, ch: char, fg: RgbColor, bg: RgbColor, flags: CellFlags) {
    let cp = ch as u32;
    buf[offset..offset + 4].copy_from_slice(&cp.to_le_bytes());
    buf[offset + 4] = fg.r;
    buf[offset + 5] = fg.g;
    buf[offset + 6] = fg.b;
    buf[offset + 7] = bg.r;
    buf[offset + 8] = bg.g;
    buf[offset + 9] = bg.b;
    buf[offset + 10..offset + 12].copy_from_slice(&flags.bits().to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_size() {
        assert_eq!(buffer_size(80, 24, 0, 0), FIXED_HEADER_SIZE + 80 * 24 * CELL_STRIDE);
        assert_eq!(buffer_size(80, 24, 2, 1), FIXED_HEADER_SIZE + 3 * RANGE_ENTRY_SIZE + 80 * 24 * CELL_STRIDE);
    }

    #[test]
    fn test_write_header_roundtrip() {
        let cols: u16 = 80;
        let lines: u16 = 24;
        let cursor = CursorState { row: 5, col: 10, shape: CursorShape::Beam, visible: true };
        let mode = TerminalMode::SHOW_CURSOR | TerminalMode::APP_CURSOR;
        let sel = vec![HighlightRange { row: 3, start_col: 5, end_col: 20 }];

        let buf_size = buffer_size(cols, lines, sel.len() as u16, 0);
        let mut buf = vec![0u8; buf_size];
        let cell_offset = write_header(&mut buf, cols, lines, &cursor, mode, &sel, &[]);

        // Read back header
        assert_eq!(u16::from_le_bytes([buf[0], buf[1]]), 80);
        assert_eq!(u16::from_le_bytes([buf[2], buf[3]]), 24);
        assert_eq!(u16::from_le_bytes([buf[4], buf[5]]), 5); // cursor row
        assert_eq!(u16::from_le_bytes([buf[6], buf[7]]), 10); // cursor col
        assert_eq!(buf[8], CursorShape::Beam as u8);
        assert_eq!(buf[9], 1); // visible
        assert_eq!(u16::from_le_bytes([buf[12], buf[13]]), 1); // 1 selection range
        assert_eq!(u16::from_le_bytes([buf[14], buf[15]]), 0); // 0 search ranges

        // Selection range
        assert_eq!(u16::from_le_bytes([buf[16], buf[17]]), 3); // row
        assert_eq!(u16::from_le_bytes([buf[18], buf[19]]), 5); // start_col
        assert_eq!(u16::from_le_bytes([buf[20], buf[21]]), 20); // end_col

        assert_eq!(cell_offset, FIXED_HEADER_SIZE + RANGE_ENTRY_SIZE);
    }

    #[test]
    fn test_write_cell_roundtrip() {
        let mut buf = [0u8; CELL_STRIDE];
        let fg = RgbColor::new(255, 128, 0);
        let bg = RgbColor::new(0, 0, 30);
        let flags = CellFlags::BOLD | CellFlags::ITALIC;

        write_cell(&mut buf, 0, 'A', fg, bg, flags);

        assert_eq!(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), 'A' as u32);
        assert_eq!(buf[4], 255); // fg.r
        assert_eq!(buf[5], 128); // fg.g
        assert_eq!(buf[6], 0);   // fg.b
        assert_eq!(buf[7], 0);   // bg.r
        assert_eq!(buf[8], 0);   // bg.g
        assert_eq!(buf[9], 30);  // bg.b
        assert_eq!(u16::from_le_bytes([buf[10], buf[11]]), (CellFlags::BOLD | CellFlags::ITALIC).bits());
    }
}
```

- [ ] **Step 2: Add buffer module to lib.rs**

In `impulse-terminal/src/lib.rs`, add:

```rust
mod buffer;

pub use buffer::{
    buffer_size, write_cell, write_header, HighlightRange, CELL_STRIDE, FIXED_HEADER_SIZE,
    RANGE_ENTRY_SIZE,
};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p impulse-terminal`
Expected: All 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add impulse-terminal/
git commit -m "Add binary buffer format for grid snapshot transport"
```

---

## Task 3: Config, Event, and Color Types

**Files:**

- Create: `impulse-terminal/src/config.rs`
- Create: `impulse-terminal/src/event.rs`
- Modify: `impulse-terminal/src/lib.rs`

- [ ] **Step 1: Create config module**

Create `impulse-terminal/src/config.rs`:

```rust
//! Terminal configuration and translation to alacritty types.

use std::collections::HashMap;
use std::path::PathBuf;

use alacritty_terminal::tty::{Options as PtyOptions, Shell};
use alacritty_terminal::term::Config as AlacrittyConfig;
use alacritty_terminal::vte::ansi::{CursorShape as AlacCursorShape, CursorStyle as AlacCursorStyle};
use serde::Deserialize;

use crate::grid::{CursorShape, RgbColor};

/// Terminal configuration provided by the frontend (deserialized from JSON).
#[derive(Deserialize)]
pub struct TerminalConfig {
    pub scrollback_lines: usize,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub shell_path: String,
    pub shell_args: Vec<String>,
    pub working_directory: Option<String>,
    pub env_vars: HashMap<String, String>,
    pub colors: TerminalColors,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: 10_000,
            cursor_shape: CursorShape::Block,
            cursor_blink: true,
            shell_path: String::new(),
            shell_args: Vec::new(),
            working_directory: None,
            env_vars: HashMap::new(),
            colors: TerminalColors::default(),
        }
    }
}

/// Terminal color palette.
#[derive(Deserialize)]
pub struct TerminalColors {
    pub foreground: RgbColor,
    pub background: RgbColor,
    /// 16-color ANSI palette (indices 0-15).
    pub palette: [RgbColor; 16],
}

impl Default for TerminalColors {
    fn default() -> Self {
        Self {
            foreground: RgbColor::new(220, 215, 186),
            background: RgbColor::new(31, 31, 40),
            palette: [
                RgbColor::new(0, 0, 0),
                RgbColor::new(205, 49, 49),
                RgbColor::new(13, 188, 121),
                RgbColor::new(229, 229, 16),
                RgbColor::new(36, 114, 200),
                RgbColor::new(188, 63, 188),
                RgbColor::new(17, 168, 205),
                RgbColor::new(229, 229, 229),
                RgbColor::new(102, 102, 102),
                RgbColor::new(241, 76, 76),
                RgbColor::new(35, 209, 139),
                RgbColor::new(245, 245, 67),
                RgbColor::new(59, 142, 234),
                RgbColor::new(214, 112, 214),
                RgbColor::new(41, 184, 219),
                RgbColor::new(229, 229, 229),
            ],
        }
    }
}

impl TerminalConfig {
    /// Convert to alacritty's term Config.
    pub(crate) fn to_alacritty_config(&self) -> AlacrittyConfig {
        AlacrittyConfig {
            scrolling_history: self.scrollback_lines,
            default_cursor_style: AlacCursorStyle {
                shape: match self.cursor_shape {
                    CursorShape::Block => AlacCursorShape::Block,
                    CursorShape::Beam => AlacCursorShape::Beam,
                    CursorShape::Underline => AlacCursorShape::Underline,
                    CursorShape::HollowBlock => AlacCursorShape::HollowBlock,
                    CursorShape::Hidden => AlacCursorShape::Hidden,
                },
                blinking: self.cursor_blink,
            },
            ..Default::default()
        }
    }

    /// Convert to alacritty's PTY Options.
    pub(crate) fn to_pty_options(&self) -> PtyOptions {
        let shell = if self.shell_path.is_empty() {
            None
        } else {
            Some(Shell::new(self.shell_path.clone(), self.shell_args.clone()))
        };
        PtyOptions {
            shell,
            working_directory: self.working_directory.as_ref().map(PathBuf::from),
            drain_on_exit: false,
            env: self.env_vars.clone(),
        }
    }
}
```

- [ ] **Step 2: Create event module**

Create `impulse-terminal/src/event.rs`:

```rust
//! Terminal events emitted to the frontend.

use serde::Serialize;

/// Events emitted by the terminal backend.
/// Frontends poll these via `TerminalBackend::poll_events()`.
#[derive(Clone, Debug, Serialize)]
pub enum TerminalEvent {
    /// Terminal output changed — frontend should re-render.
    Wakeup,
    /// Terminal title changed (OSC 0/2).
    TitleChanged(String),
    /// Title was reset to default.
    ResetTitle,
    /// Bell character received.
    Bell,
    /// Child process exited.
    ChildExited(i32),
    /// Request to store text in the clipboard (OSC 52).
    ClipboardStore(String),
    /// Request to read text from the clipboard (OSC 52).
    ClipboardLoad,
    /// Cursor blinking state has changed.
    CursorBlinkingChange,
    /// Terminal requested exit.
    Exit,
}
```

- [ ] **Step 3: Update lib.rs**

Replace `impulse-terminal/src/lib.rs` with:

```rust
//! Terminal emulation backend for Impulse, built on alacritty_terminal.
//!
//! This crate provides a platform-agnostic terminal backend. Frontends only
//! need to render a grid of styled cells and forward input events.

mod buffer;
mod config;
mod event;
mod grid;

pub use buffer::{
    buffer_size, write_cell, write_header, HighlightRange, CELL_STRIDE, FIXED_HEADER_SIZE,
    RANGE_ENTRY_SIZE,
};
pub use config::{TerminalColors, TerminalConfig};
pub use event::TerminalEvent;
pub use grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p impulse-terminal`
Expected: Compiles with no errors.

- [ ] **Step 5: Commit**

```bash
git add impulse-terminal/
git commit -m "Add config, event, and color types for terminal backend"
```

---

## Task 4: Terminal Backend — Core

**Files:**

- Create: `impulse-terminal/src/backend.rs`
- Modify: `impulse-terminal/src/lib.rs`

This is the largest single file. It owns `Term<EventProxy>`, alacritty's PTY event loop, and the color resolution logic.

- [ ] **Step 1: Create backend module**

Create `impulse-terminal/src/backend.rs`:

```rust
//! Terminal backend — owns the alacritty_terminal::Term and PTY event loop.

use std::borrow::Cow;
use std::sync::Arc;
use std::thread::JoinHandle;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags as AlacFlags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AlacColor, CursorShape as AlacCursorShape, NamedColor};
use crossbeam_channel::{Receiver, Sender};

use crate::buffer::{self, HighlightRange};
use crate::config::TerminalConfig;
use crate::event::TerminalEvent;
use crate::grid::{CellFlags, CursorShape, CursorState, RgbColor, TerminalMode};

// ---------------------------------------------------------------------------
// Event proxy — bridges alacritty events to our channel
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct EventProxy {
    event_tx: Sender<TerminalEvent>,
    pty_write_tx: Sender<String>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacEvent) {
        match event {
            AlacEvent::PtyWrite(text) => { let _ = self.pty_write_tx.send(text); }
            AlacEvent::Wakeup => { let _ = self.event_tx.send(TerminalEvent::Wakeup); }
            AlacEvent::Title(title) => { let _ = self.event_tx.send(TerminalEvent::TitleChanged(title)); }
            AlacEvent::ResetTitle => { let _ = self.event_tx.send(TerminalEvent::ResetTitle); }
            AlacEvent::Bell => { let _ = self.event_tx.send(TerminalEvent::Bell); }
            AlacEvent::Exit => { let _ = self.event_tx.send(TerminalEvent::Exit); }
            AlacEvent::ChildExit(code) => { let _ = self.event_tx.send(TerminalEvent::ChildExited(code)); }
            AlacEvent::ClipboardStore(_, text) => { let _ = self.event_tx.send(TerminalEvent::ClipboardStore(text)); }
            AlacEvent::ClipboardLoad(_, _) => { let _ = self.event_tx.send(TerminalEvent::ClipboardLoad); }
            AlacEvent::CursorBlinkingChange => { let _ = self.event_tx.send(TerminalEvent::CursorBlinkingChange); }
            AlacEvent::ColorRequest(_, _)
            | AlacEvent::TextAreaSizeRequest(_)
            | AlacEvent::MouseCursorDirty => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal size helper
// ---------------------------------------------------------------------------

struct TermSize {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize { self.screen_lines }
    fn screen_lines(&self) -> usize { self.screen_lines }
    fn columns(&self) -> usize { self.columns }
}

// ---------------------------------------------------------------------------
// Color resolution
// ---------------------------------------------------------------------------

struct ConfiguredColors {
    foreground: RgbColor,
    background: RgbColor,
    palette: [RgbColor; 269],
}

impl ConfiguredColors {
    fn from_config(config: &TerminalConfig) -> Self {
        let mut palette = [RgbColor::new(0, 0, 0); 269];

        // 16 ANSI colors from config.
        for (i, c) in config.colors.palette.iter().enumerate() {
            palette[i] = *c;
        }

        // 6x6x6 color cube (indices 16-231).
        for i in 16u16..232 {
            let idx = i - 16;
            let r = (idx / 36) as u8;
            let g = ((idx % 36) / 6) as u8;
            let b = (idx % 6) as u8;
            let to_val = |v: u8| if v == 0 { 0u8 } else { 55 + 40 * v };
            palette[i as usize] = RgbColor::new(to_val(r), to_val(g), to_val(b));
        }

        // Grayscale ramp (indices 232-255).
        for i in 232u16..256 {
            let val = (8 + 10 * (i - 232)) as u8;
            palette[i as usize] = RgbColor::new(val, val, val);
        }

        // Named colors.
        palette[NamedColor::Foreground as usize] = config.colors.foreground;
        palette[NamedColor::Background as usize] = config.colors.background;
        palette[NamedColor::Cursor as usize] = config.colors.foreground;
        palette[NamedColor::BrightForeground as usize] = config.colors.foreground;
        palette[NamedColor::DimForeground as usize] = config.colors.foreground;

        // Dim colors.
        palette[NamedColor::DimBlack as usize] = RgbColor::new(0, 0, 0);
        for i in 1..8usize {
            let base = config.colors.palette[i];
            palette[NamedColor::DimBlack as usize + i] =
                RgbColor::new(base.r * 3 / 4, base.g * 3 / 4, base.b * 3 / 4);
        }

        Self {
            foreground: config.colors.foreground,
            background: config.colors.background,
            palette,
        }
    }

    fn resolve(&self, color: AlacColor, term_colors: &alacritty_terminal::term::color::Colors) -> RgbColor {
        match color {
            AlacColor::Spec(rgb) => RgbColor::new(rgb.r, rgb.g, rgb.b),
            AlacColor::Named(named) => {
                if let Some(rgb) = term_colors[named] {
                    RgbColor::new(rgb.r, rgb.g, rgb.b)
                } else {
                    self.palette[named as usize]
                }
            }
            AlacColor::Indexed(idx) => {
                if let Some(rgb) = term_colors[idx as usize] {
                    RgbColor::new(rgb.r, rgb.g, rgb.b)
                } else {
                    self.palette[idx as usize]
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Selection kind
// ---------------------------------------------------------------------------

/// Selection kind for `start_selection()`.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum SelectionKind {
    Simple = 0,
    Block = 1,
    Semantic = 2,
    Lines = 3,
}

impl SelectionKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Block,
            2 => Self::Semantic,
            3 => Self::Lines,
            _ => Self::Simple,
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalBackend
// ---------------------------------------------------------------------------

/// The main terminal backend. One instance per terminal tab/split.
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
}

impl TerminalBackend {
    /// Create a new terminal backend and spawn a shell process.
    pub fn new(
        config: TerminalConfig,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<Self, String> {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (pty_write_tx, pty_write_rx) = crossbeam_channel::unbounded();
        let proxy = EventProxy { event_tx, pty_write_tx };

        let alac_config = config.to_alacritty_config();
        let pty_options = config.to_pty_options();
        let colors = ConfiguredColors::from_config(&config);

        let size = TermSize { columns: cols as usize, screen_lines: rows as usize };
        let term = Term::new(alac_config, &size, proxy.clone());
        let term = Arc::new(FairMutex::new(term));

        let window_size = WindowSize { num_lines: rows, num_cols: cols, cell_width, cell_height };
        let pty = tty::new(&pty_options, window_size, 0)
            .map_err(|e| format!("Failed to create PTY: {e}"))?;
        let child_pid = pty.child().id();

        let event_loop = EventLoop::new(Arc::clone(&term), proxy, pty, pty_options.drain_on_exit, false)
            .map_err(|e| format!("Failed to create event loop: {e}"))?;
        let event_loop_sender = event_loop.channel();
        let pty_thread = event_loop.spawn();

        Ok(Self {
            term,
            event_loop_sender,
            event_rx,
            pty_write_rx,
            _pty_thread: Some(pty_thread),
            cols,
            rows,
            colors,
            child_pid,
        })
    }

    /// Send input bytes to the PTY.
    pub fn write(&self, data: &[u8]) {
        self.drain_pty_writes();
        if !data.is_empty() {
            let _ = self.event_loop_sender.send(Msg::Input(Cow::Owned(data.to_vec())));
        }
    }

    /// Resize the terminal grid and PTY.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        if cols == self.cols && rows == self.rows { return; }
        self.cols = cols;
        self.rows = rows;
        let size = TermSize { columns: cols as usize, screen_lines: rows as usize };
        self.term.lock().resize(size);
        let ws = WindowSize { num_lines: rows, num_cols: cols, cell_width, cell_height };
        let _ = self.event_loop_sender.send(Msg::Resize(ws));
    }

    /// Poll for terminal events (non-blocking).
    pub fn poll_events(&self) -> Vec<TerminalEvent> {
        self.drain_pty_writes();
        let mut events = Vec::new();
        while let Ok(ev) = self.event_rx.try_recv() {
            events.push(ev);
        }
        events
    }

    /// Write the visible grid into a pre-allocated binary buffer.
    /// Returns the number of bytes written.
    pub fn write_grid_to_buffer(&self, buf: &mut [u8]) -> usize {
        let term = self.term.lock();
        let content = term.renderable_content();
        let term_colors = content.colors;
        let mode = content.mode;
        let cursor = content.cursor;
        let num_cols = term.columns();
        let num_lines = term.screen_lines();

        // Build mode flags.
        let mut mode_flags = TerminalMode::empty();
        if mode.contains(TermMode::SHOW_CURSOR) { mode_flags |= TerminalMode::SHOW_CURSOR; }
        if mode.contains(TermMode::APP_CURSOR) { mode_flags |= TerminalMode::APP_CURSOR; }
        if mode.contains(TermMode::APP_KEYPAD) { mode_flags |= TerminalMode::APP_KEYPAD; }
        if mode.contains(TermMode::MOUSE_REPORT_CLICK) { mode_flags |= TerminalMode::MOUSE_REPORT_CLICK; }
        if mode.contains(TermMode::MOUSE_MOTION) { mode_flags |= TerminalMode::MOUSE_MOTION; }
        if mode.contains(TermMode::MOUSE_DRAG) { mode_flags |= TerminalMode::MOUSE_DRAG; }
        if mode.contains(TermMode::SGR_MOUSE) { mode_flags |= TerminalMode::MOUSE_SGR; }
        if mode.contains(TermMode::BRACKETED_PASTE) { mode_flags |= TerminalMode::BRACKETED_PASTE; }
        if mode.contains(TermMode::FOCUS_IN_OUT) { mode_flags |= TerminalMode::FOCUS_IN_OUT; }
        if mode.contains(TermMode::ALT_SCREEN) { mode_flags |= TerminalMode::ALT_SCREEN; }
        if mode.contains(TermMode::LINE_WRAP) { mode_flags |= TerminalMode::LINE_WRAP; }

        // Cursor state.
        let cursor_visible = mode.contains(TermMode::SHOW_CURSOR)
            && cursor.point.line.0 >= 0
            && (cursor.point.line.0 as usize) < num_lines;
        let cursor_state = CursorState {
            row: cursor.point.line.0.max(0) as usize,
            col: cursor.point.column.0,
            shape: match cursor.shape {
                AlacCursorShape::Block => CursorShape::Block,
                AlacCursorShape::Beam => CursorShape::Beam,
                AlacCursorShape::Underline => CursorShape::Underline,
                AlacCursorShape::HollowBlock => CursorShape::HollowBlock,
                AlacCursorShape::Hidden => CursorShape::Hidden,
            },
            visible: cursor_visible,
        };

        // Selection ranges.
        let mut selection_ranges = Vec::new();
        if let Some(sel) = &content.selection {
            let start_line = sel.start.line.0.max(0) as usize;
            let end_line = (sel.end.line.0.max(0) as usize).min(num_lines.saturating_sub(1));
            for row in start_line..=end_line {
                let sc = if row == start_line { sel.start.column.0 } else { 0 };
                let ec = if row == end_line { sel.end.column.0 } else { num_cols.saturating_sub(1) };
                selection_ranges.push(HighlightRange {
                    row: row as u16,
                    start_col: sc as u16,
                    end_col: ec as u16,
                });
            }
        }

        // TODO: search match ranges will be added in Task 8.
        let search_ranges: Vec<HighlightRange> = Vec::new();

        let required = buffer::buffer_size(
            num_cols as u16, num_lines as u16,
            selection_ranges.len() as u16, search_ranges.len() as u16,
        );
        if buf.len() < required { return 0; }

        // Write header.
        let cell_offset = buffer::write_header(
            buf,
            num_cols as u16, num_lines as u16,
            &cursor_state, mode_flags,
            &selection_ranges, &search_ranges,
        );

        // Initialize all cells to space with default colors.
        let default_fg = self.colors.foreground;
        let default_bg = self.colors.background;
        for i in 0..(num_cols * num_lines) {
            buffer::write_cell(
                buf,
                cell_offset + i * buffer::CELL_STRIDE,
                ' ', default_fg, default_bg, CellFlags::empty(),
            );
        }

        // Fill from display iterator.
        for indexed in content.display_iter {
            let row = indexed.point.line.0 as usize;
            let col = indexed.point.column.0;
            if row < num_lines && col < num_cols {
                let offset = cell_offset + (row * num_cols + col) * buffer::CELL_STRIDE;
                let fg = self.colors.resolve(indexed.cell.fg, term_colors);
                let bg = self.colors.resolve(indexed.cell.bg, term_colors);
                let flags = convert_flags(indexed.cell.flags);
                buffer::write_cell(buf, offset, indexed.cell.c, fg, bg, flags);
            }
        }

        required
    }

    /// Calculate the buffer size needed for a grid snapshot.
    pub fn grid_buffer_size(&self) -> usize {
        // Worst case: max possible selection/search ranges = num_lines.
        // In practice we use 0 for the size calculation since the actual
        // counts aren't known until snapshot time. The caller should
        // over-allocate slightly.
        let term = self.term.lock();
        let lines = term.screen_lines() as u16;
        let cols = term.columns() as u16;
        // Allow up to lines*2 ranges for selection + search.
        buffer::buffer_size(cols, lines, lines, lines)
    }

    /// Start a text selection.
    pub fn start_selection(&self, col: usize, row: usize, kind: SelectionKind) {
        let mut term = self.term.lock();
        let point = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(row as i32),
            alacritty_terminal::index::Column(col),
        );
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Block => SelectionType::Block,
            SelectionKind::Semantic => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
        };
        term.selection = Some(Selection::new(ty, point, alacritty_terminal::index::Side::Left));
    }

    /// Update the current selection endpoint.
    pub fn update_selection(&self, col: usize, row: usize) {
        let mut term = self.term.lock();
        if let Some(ref mut sel) = term.selection {
            let point = alacritty_terminal::index::Point::new(
                alacritty_terminal::index::Line(row as i32),
                alacritty_terminal::index::Column(col),
            );
            sel.update(point, alacritty_terminal::index::Side::Right);
        }
    }

    /// Clear the current selection.
    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    /// Get the selected text.
    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string()
    }

    /// Scroll the viewport. Positive = up (towards history), negative = down.
    pub fn scroll(&self, delta: i32) {
        self.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
    }

    /// Scroll the viewport to the bottom.
    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Bottom);
    }

    /// Get the current terminal mode flags.
    pub fn mode(&self) -> TerminalMode {
        let mode = *self.term.lock().mode();
        let mut flags = TerminalMode::empty();
        if mode.contains(TermMode::SHOW_CURSOR) { flags |= TerminalMode::SHOW_CURSOR; }
        if mode.contains(TermMode::APP_CURSOR) { flags |= TerminalMode::APP_CURSOR; }
        if mode.contains(TermMode::APP_KEYPAD) { flags |= TerminalMode::APP_KEYPAD; }
        if mode.contains(TermMode::MOUSE_REPORT_CLICK) { flags |= TerminalMode::MOUSE_REPORT_CLICK; }
        if mode.contains(TermMode::MOUSE_MOTION) { flags |= TerminalMode::MOUSE_MOTION; }
        if mode.contains(TermMode::MOUSE_DRAG) { flags |= TerminalMode::MOUSE_DRAG; }
        if mode.contains(TermMode::SGR_MOUSE) { flags |= TerminalMode::MOUSE_SGR; }
        if mode.contains(TermMode::BRACKETED_PASTE) { flags |= TerminalMode::BRACKETED_PASTE; }
        if mode.contains(TermMode::FOCUS_IN_OUT) { flags |= TerminalMode::FOCUS_IN_OUT; }
        if mode.contains(TermMode::ALT_SCREEN) { flags |= TerminalMode::ALT_SCREEN; }
        if mode.contains(TermMode::LINE_WRAP) { flags |= TerminalMode::LINE_WRAP; }
        flags
    }

    /// Get the current terminal dimensions.
    pub fn dimensions(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// Get the PID of the child shell process.
    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    /// Notify the terminal about focus change.
    pub fn set_focus(&self, focused: bool) {
        self.term.lock().is_focused = focused;
    }

    /// Shut down the terminal.
    pub fn shutdown(&self) {
        let _ = self.event_loop_sender.send(Msg::Shutdown);
    }

    /// Access the term lock (for search module).
    pub(crate) fn term(&self) -> &Arc<FairMutex<Term<EventProxy>>> {
        &self.term
    }

    fn drain_pty_writes(&self) {
        while let Ok(text) = self.pty_write_rx.try_recv() {
            let _ = self.event_loop_sender.send(Msg::Input(Cow::Owned(text.into_bytes())));
        }
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Convert alacritty cell flags to our CellFlags.
fn convert_flags(flags: AlacFlags) -> CellFlags {
    let mut result = CellFlags::empty();
    if flags.contains(AlacFlags::BOLD) { result |= CellFlags::BOLD; }
    if flags.contains(AlacFlags::ITALIC) { result |= CellFlags::ITALIC; }
    if flags.contains(AlacFlags::UNDERLINE) { result |= CellFlags::UNDERLINE; }
    if flags.contains(AlacFlags::STRIKEOUT) { result |= CellFlags::STRIKETHROUGH; }
    if flags.contains(AlacFlags::DIM) { result |= CellFlags::DIM; }
    if flags.contains(AlacFlags::INVERSE) { result |= CellFlags::INVERSE; }
    if flags.contains(AlacFlags::HIDDEN) { result |= CellFlags::HIDDEN; }
    if flags.contains(AlacFlags::WIDE_CHAR) { result |= CellFlags::WIDE_CHAR; }
    if flags.contains(AlacFlags::WIDE_CHAR_SPACER) { result |= CellFlags::WIDE_CHAR_SPACER; }
    if flags.contains(AlacFlags::DOUBLE_UNDERLINE) { result |= CellFlags::DOUBLE_UNDERLINE; }
    if flags.contains(AlacFlags::UNDERCURL) { result |= CellFlags::UNDERCURL; }
    if flags.contains(AlacFlags::DOTTED_UNDERLINE) { result |= CellFlags::DOTTED_UNDERLINE; }
    if flags.contains(AlacFlags::DASHED_UNDERLINE) { result |= CellFlags::DASHED_UNDERLINE; }
    result
}
```

- [ ] **Step 2: Update lib.rs**

Add to `impulse-terminal/src/lib.rs`:

```rust
mod backend;

pub use backend::{SelectionKind, TerminalBackend};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p impulse-terminal`
Expected: Compiles with no errors. There may be warnings about unused fields — that's fine at this stage.

- [ ] **Step 4: Run all tests**

Run: `cargo test -p impulse-terminal`
Expected: All buffer tests still pass.

- [ ] **Step 5: Commit**

```bash
git add impulse-terminal/
git commit -m "Add terminal backend wrapping alacritty_terminal with PTY event loop"
```

---

## Task 5: FFI Layer

**Files:**

- Modify: `impulse-ffi/Cargo.toml:11-23`
- Modify: `impulse-ffi/src/lib.rs` (append after line 1350)
- Modify: `impulse-macos/CImpulseFFI/include/impulse_ffi.h:74-78`

- [ ] **Step 1: Add impulse-terminal dependency to impulse-ffi**

In `impulse-ffi/Cargo.toml`, add after the `impulse-editor` line:

```toml
impulse-terminal = { path = "../impulse-terminal" }
crossbeam-channel = "0.5"
```

- [ ] **Step 2: Add terminal FFI functions to lib.rs**

Append to the end of `impulse-ffi/src/lib.rs`:

```rust
// ---------------------------------------------------------------------------
// Terminal backend API
// ---------------------------------------------------------------------------

use impulse_terminal::{SelectionKind, TerminalBackend};

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
        // Reallocate snapshot buffer for new dimensions.
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
pub extern "C" fn impulse_terminal_child_pid(handle: *mut TerminalHandle) -> u32 {
    ffi_catch(0, AssertUnwindSafe(|| {
        if handle.is_null() { return 0; }
        let h = unsafe { &*handle };
        h.backend.child_pid()
    }))
}

// Search FFI functions — stubbed for now, implemented in Task 8.

#[no_mangle]
pub extern "C" fn impulse_terminal_search(handle: *mut TerminalHandle, _pattern: *const c_char) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return to_c_string("{}"); }
        // TODO: Implement in Task 8 after search module is added.
        to_c_string("{}")
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_search_next(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return to_c_string("{}"); }
        to_c_string("{}")
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_search_prev(handle: *mut TerminalHandle) -> *mut c_char {
    ffi_catch(std::ptr::null_mut(), AssertUnwindSafe(|| {
        if handle.is_null() { return to_c_string("{}"); }
        to_c_string("{}")
    }))
}

#[no_mangle]
pub extern "C" fn impulse_terminal_search_clear(handle: *mut TerminalHandle) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        // TODO: Implement in Task 8.
    }))
}
```

- [ ] **Step 3: Add C declarations to impulse_ffi.h**

In `impulse-macos/CImpulseFFI/include/impulse_ffi.h`, add before the `#endif`:

```c
// Terminal backend API
void *impulse_terminal_create(const char *config_json, unsigned short cols, unsigned short rows, unsigned short cell_width, unsigned short cell_height);
void impulse_terminal_destroy(void *handle);
void impulse_terminal_write(void *handle, const unsigned char *data, unsigned long len);
void impulse_terminal_resize(void *handle, unsigned short cols, unsigned short rows, unsigned short cell_width, unsigned short cell_height);
unsigned long impulse_terminal_grid_snapshot(void *handle, unsigned char *out_buf, unsigned long buf_len);
unsigned long impulse_terminal_grid_snapshot_size(void *handle);
char *impulse_terminal_poll_events(void *handle);
void impulse_terminal_start_selection(void *handle, unsigned short col, unsigned short row, unsigned char kind);
void impulse_terminal_update_selection(void *handle, unsigned short col, unsigned short row);
void impulse_terminal_clear_selection(void *handle);
char *impulse_terminal_selected_text(void *handle);
void impulse_terminal_scroll(void *handle, int delta);
char *impulse_terminal_mode(void *handle);
void impulse_terminal_set_focus(void *handle, _Bool focused);
unsigned int impulse_terminal_child_pid(void *handle);
char *impulse_terminal_search(void *handle, const char *pattern);
char *impulse_terminal_search_next(void *handle);
char *impulse_terminal_search_prev(void *handle);
void impulse_terminal_search_clear(void *handle);
```

- [ ] **Step 4: Verify Rust builds**

Run: `cargo build -p impulse-ffi`
Expected: Compiles with no errors.

- [ ] **Step 5: Commit**

```bash
git add impulse-ffi/ impulse-macos/CImpulseFFI/ Cargo.lock
git commit -m "Add 18 terminal FFI functions bridging impulse-terminal to Swift"
```

---

## Task 6: Swift TerminalBackend + GridBufferReader

**Files:**

- Create: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift`
- Modify: `impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift`

- [ ] **Step 1: Add terminal bridge functions to ImpulseCore.swift**

Append before the closing `}` of the `ImpulseCore` class in `Bridge/ImpulseCore.swift`:

```swift
    // MARK: - Terminal Backend

    static func terminalCreate(configJson: String, cols: UInt16, rows: UInt16, cellWidth: UInt16, cellHeight: UInt16) -> OpaquePointer? {
        return configJson.withCString { ptr in
            impulse_terminal_create(ptr, cols, rows, cellWidth, cellHeight)
        }.flatMap { OpaquePointer($0) }
    }

    static func terminalDestroy(handle: OpaquePointer) {
        impulse_terminal_destroy(UnsafeMutableRawPointer(handle))
    }

    static func terminalWrite(handle: OpaquePointer, data: Data) {
        data.withUnsafeBytes { rawBuf in
            guard let ptr = rawBuf.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return }
            impulse_terminal_write(UnsafeMutableRawPointer(handle), ptr, rawBuf.count)
        }
    }

    static func terminalResize(handle: OpaquePointer, cols: UInt16, rows: UInt16, cellWidth: UInt16, cellHeight: UInt16) {
        impulse_terminal_resize(UnsafeMutableRawPointer(handle), cols, rows, cellWidth, cellHeight)
    }

    static func terminalGridSnapshot(handle: OpaquePointer, buffer: UnsafeMutablePointer<UInt8>, bufferSize: Int) -> Int {
        return impulse_terminal_grid_snapshot(UnsafeMutableRawPointer(handle), buffer, bufferSize)
    }

    static func terminalGridSnapshotSize(handle: OpaquePointer) -> Int {
        return impulse_terminal_grid_snapshot_size(UnsafeMutableRawPointer(handle))
    }

    static func terminalPollEvents(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_poll_events(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    static func terminalStartSelection(handle: OpaquePointer, col: UInt16, row: UInt16, kind: UInt8) {
        impulse_terminal_start_selection(UnsafeMutableRawPointer(handle), col, row, kind)
    }

    static func terminalUpdateSelection(handle: OpaquePointer, col: UInt16, row: UInt16) {
        impulse_terminal_update_selection(UnsafeMutableRawPointer(handle), col, row)
    }

    static func terminalClearSelection(handle: OpaquePointer) {
        impulse_terminal_clear_selection(UnsafeMutableRawPointer(handle))
    }

    static func terminalSelectedText(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_selected_text(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    static func terminalScroll(handle: OpaquePointer, delta: Int32) {
        impulse_terminal_scroll(UnsafeMutableRawPointer(handle), delta)
    }

    static func terminalMode(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_mode(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    static func terminalSetFocus(handle: OpaquePointer, focused: Bool) {
        impulse_terminal_set_focus(UnsafeMutableRawPointer(handle), focused)
    }

    static func terminalChildPid(handle: OpaquePointer) -> UInt32 {
        return impulse_terminal_child_pid(UnsafeMutableRawPointer(handle))
    }

    static func terminalSearch(handle: OpaquePointer, pattern: String) -> String? {
        return pattern.withCString { ptr in
            guard let result = impulse_terminal_search(UnsafeMutableRawPointer(handle), ptr) else { return nil }
            return consumeCString(result)
        }
    }

    static func terminalSearchNext(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_search_next(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    static func terminalSearchPrev(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_search_prev(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    static func terminalSearchClear(handle: OpaquePointer) {
        impulse_terminal_search_clear(UnsafeMutableRawPointer(handle))
    }
```

- [ ] **Step 2: Create TerminalBackend.swift**

Create `impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift`. This is a large file (~300 lines) containing `TerminalBackend` (FFI wrapper), `GridBufferReader` (binary buffer accessor), config types, and event types. The implementing agent should write this file following the spec in section 6.1 of `docs/superpowers/specs/2026-04-06-alacritty-terminal-migration-design.md`. Key requirements:

- `TerminalBackend` class holds `OpaquePointer?` handle and a reusable `UnsafeMutablePointer<UInt8>` buffer
- Buffer allocated via `UnsafeMutablePointer<UInt8>.allocate(capacity:)` on init, reallocated on resize, freed in deinit
- `gridSnapshot() -> GridBufferReader?` calls FFI, returns reader wrapping the buffer pointer
- `pollEvents() -> [TerminalBackendEvent]` parses JSON (same pattern as old branch)
- `GridBufferReader` struct reads header fields from fixed offsets and provides `cell(row:col:)` accessor
- `TerminalBackendConfig` is `Codable` for JSON serialization to the FFI
- `TerminalModeFlags` is `Codable` for JSON deserialization from `mode()` calls
- All methods guard against `isShutdown` state

- [ ] **Step 3: Verify Swift builds**

Run: `./impulse-macos/build.sh`
Expected: Build succeeds (SwiftTerm is still present at this point — we haven't removed it yet).

- [ ] **Step 4: Commit**

```bash
git add impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift
git commit -m "Add Swift terminal backend wrapper with binary grid buffer reader"
```

---

## Task 7: Swift TerminalRenderer + KeyEncoder

**Files:**

- Create: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalRenderer.swift`
- Create: `impulse-macos/Sources/ImpulseApp/Terminal/KeyEncoder.swift`

These are the two largest new Swift files. The implementing agent should write them following sections 6.2 and 6.3 of the spec.

- [ ] **Step 1: Create KeyEncoder.swift**

Create `impulse-macos/Sources/ImpulseApp/Terminal/KeyEncoder.swift` (~150 lines). Requirements:

- Stateless struct with `static func encode(event: NSEvent, mode: TerminalModeFlags) -> [UInt8]`
- Cmd+ combinations return empty (handled by menu system)
- Shift+Enter → `[0x1B, 0x5B, 0x31, 0x33, 0x3B, 0x32, 0x75]` (CSI u)
- Arrow keys (keyCodes 123-126) respect `mode.appCursor` for OA/OB/OC/OD vs [A/[B/[C/[D
- Function keys F1-F12 (keyCodes 122,120,99,118,96,97,98,100,101,109,103,111)
- Home (115), End (119), PageUp (116), PageDown (121), Delete (117)
- Backspace (51) → 0x7F, Tab (48) → 0x09, Escape (53) → 0x1B, Return (36,76) → 0x0D
- Ctrl+letter → control codes 0x01-0x1A; Ctrl+[ → ESC, Ctrl+] → GS, Ctrl+\ → FS
- Option+key → ESC prefix + character UTF-8
- Regular characters → `event.characters` UTF-8 bytes

- [ ] **Step 2: Create TerminalRenderer.swift**

Create `impulse-macos/Sources/ImpulseApp/Terminal/TerminalRenderer.swift` (~750 lines). Requirements:

- `NSView` subclass, `isFlipped = true`, `acceptsFirstResponder = true`
- `TerminalFontMetrics` struct: CTFont, cellWidth, cellHeight, ascent, descent, leading
- `var backend: TerminalBackend?` — set by TerminalTab
- **Run-based `draw(_:)`:** Fill background → background spans → selection/search highlights → text runs → box drawing → cursor. Text runs group consecutive cells with same fg+bold+italic into one `CTLineDraw` call
- **Box drawing:** Programmatic `CGContext` paths for U+2500-U+259F common subset (~30 chars). Others fall back to font glyph
- **Refresh:** `CVDisplayLink` (not Timer). Callback polls events, only sets `needsDisplay` on wakeup. `startRefreshLoop()` / `stopRefreshLoop()` methods
- **`keyDown(with:)`:** Cmd+C/V handled first, then `KeyEncoder.encode()` → `backend.write()`
- **Mouse:** `mouseDown`/`mouseDragged`/`mouseUp` for selection. `scrollWheel` accumulates deltas, forwards as scroll or SGR mouse events
- **`gridPoint(from:)`** helper converts mouse coordinates to grid col/row
- `resizeToFit()` called from `setFrameSize()`, calculates grid dimensions and calls `backend.resize()`
- `updateFont(family:size:)` rebuilds `TerminalFontMetrics` and triggers resize
- Callbacks: `onEvent`, `onPaste`, `onCopy` closures (set by TerminalTab)

- [ ] **Step 3: Verify Swift builds**

Run: `./impulse-macos/build.sh`
Expected: Build succeeds.

- [ ] **Step 4: Commit**

```bash
git add impulse-macos/Sources/ImpulseApp/Terminal/KeyEncoder.swift impulse-macos/Sources/ImpulseApp/Terminal/TerminalRenderer.swift
git commit -m "Add CoreText terminal renderer and keyboard input encoder"
```

---

## Task 8: Rewrite TerminalTab + Remove SwiftTerm

**Files:**

- Modify: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift` (full rewrite)
- Modify: `impulse-macos/Sources/ImpulseApp/TabManager.swift:2` (remove import)
- Modify: `impulse-macos/Package.swift:10,20` (remove dependency)

This is the swap — SwiftTerm gets removed and the new backend takes over.

- [ ] **Step 1: Rewrite TerminalTab.swift**

Fully rewrite `impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift` (~350 lines). The implementing agent should follow section 6.4 of the spec. Key requirements:

- `class TerminalTab: NSView` — no SwiftTerm import, no `LocalProcessTerminalViewDelegate`
- Owns `TerminalRenderer` (subview, pinned to edges) and `TerminalBackend` (created in `spawnShell()`)
- `spawnShell()`: detect shell via `ImpulseCore.getUserLoginShell()`, inject integration scripts via `ImpulseCore.getShellIntegrationScript()`, build environment (TERM=xterm-256color, TERM*PROGRAM=Impulse, COLORTERM=truecolor, filter DYLD*_/LD\__ vars), create `TerminalBackendConfig`, create `TerminalBackend`, wire to renderer, start refresh loop
- `configureTerminal(settings:theme:)`: update font, copy-on-select
- Event handling via `renderer.onEvent` callback: title → NSNotification, bell → NSSound.beep(), clipboard, exit
- `pasteFromClipboard()`: strip trailing newlines, normalize CRLF, bracketed paste, image fallback
- `copySelection()`: `backend.selectedText()` → NSPasteboard
- Copy-on-select: optional NSEvent monitor for mouseUp
- Drag and drop: register `.fileURL`, shell-escape paths, bracketed paste
- CWD polling: 1s Timer using `proc_pidinfo` / `PROC_PIDVNODEPATHINFO`
- `terminateProcess()`: stop refresh loop, shutdown backend
- `focus()`: `window?.makeFirstResponder(renderer)`
- `sendCommand(_:)`: append newline, write bytes

- [ ] **Step 2: Remove SwiftTerm import from TabManager.swift**

In `impulse-macos/Sources/ImpulseApp/TabManager.swift`, remove line 2:

```swift
import SwiftTerm
```

- [ ] **Step 3: Remove SwiftTerm from Package.swift**

In `impulse-macos/Package.swift`:

Remove line 10:

```swift
        .package(url: "https://github.com/migueldeicaza/SwiftTerm.git", from: "1.11.2"),
```

Remove `"SwiftTerm",` from line 20 in the dependencies array.

- [ ] **Step 4: Build the full app**

Run: `./impulse-macos/build.sh`
Expected: Build succeeds with SwiftTerm fully removed. There will likely be compiler warnings but no errors.

- [ ] **Step 5: Manual smoke test**

Run: `open dist/Impulse.app`

Verify:

- Terminal tab opens and shows a shell prompt
- Typing works (characters appear)
- Arrow keys work (history navigation, cursor movement)
- Ctrl+C works (sends interrupt)
- Tab opens with correct title
- Multiple tabs work
- Terminal split works
- Scrollback works (scroll up with trackpad, type to snap back)
- Copy/paste works (Cmd+C/V)

- [ ] **Step 6: Commit**

```bash
git add impulse-macos/
git commit -m "Replace SwiftTerm with alacritty_terminal backend on macOS"
```

---

## Task 9: Search Integration

**Files:**

- Create: `impulse-terminal/src/search.rs`
- Modify: `impulse-terminal/src/backend.rs`
- Modify: `impulse-terminal/src/lib.rs`
- Modify: `impulse-ffi/src/lib.rs` (replace search stubs)
- Modify: `impulse-macos/Sources/ImpulseApp/MainWindow.swift:842-862`

- [ ] **Step 1: Create search module**

Create `impulse-terminal/src/search.rs`. The implementing agent should:

- Add a `TerminalSearch` struct that holds `Option<RegexSearch>` and current match state
- Expose `search(term, pattern)`, `search_next(term)`, `search_prev(term)`, `clear()` methods
- Use `alacritty_terminal::term::search::RegexSearch` for pattern compilation
- Use `Term::search_next()` / `Term::search_prev()` for navigation (from `alacritty_terminal::grid::Dimensions` trait)
- Return `SearchResult { match_row, match_start_col, match_end_col, total_matches, current_match_index }` serializable as JSON
- Collect all visible match ranges for inclusion in the grid snapshot buffer

- [ ] **Step 2: Integrate search into backend**

In `impulse-terminal/src/backend.rs`:

- Add `search: TerminalSearch` field to `TerminalBackend`
- Add public methods: `search()`, `search_next()`, `search_prev()`, `search_clear()`
- In `write_grid_to_buffer()`, replace the empty `search_ranges` with actual matches from `self.search.visible_matches()`

- [ ] **Step 3: Update lib.rs exports**

Add to `impulse-terminal/src/lib.rs`:

```rust
mod search;
pub use search::SearchResult;
```

- [ ] **Step 4: Replace FFI search stubs**

In `impulse-ffi/src/lib.rs`, replace the 4 search stub functions with real implementations that call `h.backend.search()`, `.search_next()`, `.search_prev()`, `.search_clear()`.

- [ ] **Step 5: Wire MainWindow.swift search methods**

In `impulse-macos/Sources/ImpulseApp/MainWindow.swift`, replace the TODO stubs at lines 842-862:

```swift
@objc private func termSearchFieldChanged(_ sender: NSSearchField) {
    guard let container = tabManager.selectedTerminal,
          let terminal = container.activeTerminal else { return }
    let query = sender.stringValue
    if query.isEmpty {
        terminal.searchClear()
    } else {
        terminal.search(query)
    }
}

@objc private func termSearchNext(_ sender: Any?) {
    guard let container = tabManager.selectedTerminal,
          let terminal = container.activeTerminal else { return }
    terminal.searchNext()
}

@objc private func termSearchPrev(_ sender: Any?) {
    guard let container = tabManager.selectedTerminal,
          let terminal = container.activeTerminal else { return }
    terminal.searchPrev()
}
```

Add corresponding `search()`, `searchNext()`, `searchPrev()`, `searchClear()` methods to `TerminalTab.swift` that delegate to `backend`.

- [ ] **Step 6: Build and test**

Run: `cargo test -p impulse-terminal && ./impulse-macos/build.sh`
Expected: Tests pass, app builds.

- [ ] **Step 7: Manual test search**

Run the app. Open a terminal tab. Run a command that produces output (e.g., `ls -la`). Open the search bar (Cmd+F or toolbar button). Type a search term. Verify:

- Matches are highlighted in amber
- Next/prev navigation works
- Clearing the search field removes highlights

- [ ] **Step 8: Commit**

```bash
git add impulse-terminal/ impulse-ffi/ impulse-macos/
git commit -m "Add terminal regex search using alacritty_terminal's search engine"
```

---

## Task 10: Final Verification and Cleanup

**Files:**

- No new files
- Potential minor fixes across any file

- [ ] **Step 1: Full build verification**

Run: `cargo build -p impulse-core -p impulse-editor -p impulse-ffi -p impulse-terminal && cargo test -p impulse-terminal && ./impulse-macos/build.sh`
Expected: All builds pass, all tests pass.

- [ ] **Step 2: Verify SwiftTerm is fully removed**

Run: `grep -r "SwiftTerm" impulse-macos/ --include="*.swift"`
Expected: No matches.

Run: `grep -r "SwiftTerm" impulse-macos/Package.swift`
Expected: No matches.

- [ ] **Step 3: Comprehensive smoke test**

Run the app and verify:

- Terminal opens with shell prompt
- Basic typing and command execution
- Arrow keys (up for history, left/right for cursor)
- Ctrl+C interrupt
- Tab completion
- Colors (run `ls --color` or similar)
- Scrollback (scroll up with trackpad, text output snaps back)
- Copy/paste (Cmd+C/V, copy-on-select if enabled)
- Terminal splits (Cmd+D or equivalent)
- Multiple terminal tabs
- TUI apps: `htop`, `vim`, or `nano` — verify they render correctly and keyboard works
- Search (Cmd+F, type query, next/prev)
- Drag and drop a file from Finder into terminal
- Resize window — terminal should resize smoothly
- Theme changes — terminal colors should update
- Close tab — no beach ball or hang

- [ ] **Step 4: Fix any issues found in smoke testing**

Address any regressions discovered. Common areas:

- Mouse wheel scroll direction
- Special key sequences
- Color rendering
- CWD tracking
- Tab title updates

- [ ] **Step 5: Commit any fixes**

```bash
git add -A
git commit -m "Fix post-migration issues found during smoke testing"
```

- [ ] **Step 6: Update memory**

Update the project memory file at `/Users/dowilcox/.claude/projects/-Users-dowilcox-Code-impulse/memory/project_terminal_migration.md` to reflect the completed macOS migration status and remove stale references to the old branch.
