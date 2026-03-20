# Plan: Unified Terminal Backend with `alacritty_terminal`

**Status:** Phase 1, 2, & 3 functionally complete. SwiftTerm removed, macOS app builds and runs with impulse-terminal backend. Phase 4 (Linux) not started. Phase 5 (cleanup) partially done.

### Known Issues (macOS)

- **Scroll wheel in TUI apps:** Mouse scroll forwarding to TUI apps with mouse reporting (lazygit, htop, etc.) sends incorrect SGR encoding — scrolling behavior is broken in mouse-reporting TUI apps. Needs proper mouse button encoding matching alacritty's input handling.
- **Scroll in regular shell:** Works but may need sign tuning depending on natural scrolling settings. Uses `isScrolledBack` flag to prevent 60fps refresh from resetting viewport.
- **Box-drawing characters:** Programmatic rendering covers common chars (U+2500–U+259F) but some less common variants fall back to font glyphs which may have gaps.
- **Cmd+W on terminal tabs:** May beach ball — shutdown path needs investigation for proper PTY cleanup ordering.
- **CWD tracking:** Uses `proc_pidinfo` polling (1s interval) instead of OSC 7 parsing. Works but has latency. Future improvement: intercept OSC 7 in the PTY byte stream.
- **Performance:** Cell-by-cell rendering (needed for box-drawing) is slower than run-based rendering. Future optimization: hybrid approach (runs for regular text, per-cell for special chars).
- **IME input:** Not implemented — CJK input methods won't work. Needs NSTextInputClient conformance on TerminalRenderer.
  **Created:** 2026-03-19
  **Goal:** Replace VTE4 (Linux) and SwiftTerm (macOS) with a single Rust-based terminal emulation backend using `alacritty_terminal`, giving both platforms identical terminal behavior from shared code.

---

## Table of Contents

1. [Motivation](#1-motivation)
2. [Architecture Overview](#2-architecture-overview)
3. [Phase 1 — New `impulse-terminal` Crate](#3-phase-1--new-impulse-terminal-crate)
4. [Phase 2 — FFI Exposure for macOS](#4-phase-2--ffi-exposure-for-macos)
5. [Phase 3 — macOS Frontend Integration](#5-phase-3--macos-frontend-integration)
6. [Phase 4 — Linux Frontend Integration](#6-phase-4--linux-frontend-integration)
7. [Phase 5 — Remove Old Dependencies](#7-phase-5--remove-old-dependencies)
8. [Rendering Strategy](#8-rendering-strategy)
9. [Feature Parity Checklist](#9-feature-parity-checklist)
10. [Risk Register](#10-risk-register)
11. [Reference Material](#11-reference-material)

---

## 1. Motivation

Currently Impulse uses two completely separate terminal implementations:

|                     | Linux                         | macOS                                        |
| ------------------- | ----------------------------- | -------------------------------------------- |
| **Library**         | VTE4 (C library, GTK4 widget) | SwiftTerm (Swift package)                    |
| **PTY management**  | VTE handles internally        | SwiftTerm `LocalProcessTerminalView` handles |
| **Rendering**       | VTE renders via GTK/Cairo     | SwiftTerm renders via NSView/CoreText        |
| **Escape handling** | VTE's built-in parser         | SwiftTerm's built-in parser                  |

**Problems this creates:**

- Terminal behavior diverges between platforms (escape sequence support, edge cases, bugs)
- Every terminal feature must be implemented twice with different APIs
- Shell integration (OSC parsing) is partially duplicated — `impulse-core` has its own `OscParser` that runs alongside VTE/SwiftTerm's parsers
- Bug fixes in one platform don't transfer to the other
- No path to GPU-accelerated rendering (VTE is Cairo-based, SwiftTerm is CoreText-based)

**What the migration achieves:**

- Single terminal emulation backend in Rust, shared across both platforms
- Consistent escape sequence handling, scrollback behavior, and selection model
- Shell integration moves into the terminal backend (no more parallel OSC parsing)
- Foundation for future GPU-accelerated rendering (wgpu)
- Each frontend only needs to: render a grid of styled cells and forward input

---

## 2. Architecture Overview

### Current Architecture

```
┌─────────────────┐     ┌──────────────────┐
│  impulse-linux   │     │  impulse-macos    │
│  ┌─────────────┐│     │  ┌──────────────┐ │
│  │ VTE4 Widget ││     │  │  SwiftTerm    │ │
│  │ (PTY+Parse  ││     │  │ (PTY+Parse   │ │
│  │  +Render)   ││     │  │  +Render)    │ │
│  └─────────────┘│     │  └──────────────┘ │
│         │       │     │         │         │
│  impulse-core   │     │    impulse-ffi    │
│  (OscParser,    │     │    impulse-core   │
│   shell.rs)     │     │    (OscParser,    │
└─────────────────┘     │     shell.rs)     │
                        └──────────────────┘
```

### Target Architecture

```
┌──────────────────┐     ┌──────────────────┐
│  impulse-linux    │     │  impulse-macos    │
│  ┌──────────────┐│     │  ┌──────────────┐ │
│  │ GTK4 Renderer││     │  │ NSView/Metal │ │
│  │ (DrawingArea ││     │  │  Renderer    │ │
│  │  or GLArea)  ││     │  │              │ │
│  └──────┬───────┘│     │  └──────┬───────┘ │
│         │        │     │         │         │
│  ┌──────┴───────┐│     │    impulse-ffi   │
│  │impulse-term- ││     │  ┌──────┴───────┐ │
│  │   inal       ││     │  │impulse-term- │ │
│  │ (alacritty_  ││     │  │   inal       │ │
│  │  terminal +  ││     │  │ (via C FFI)  │ │
│  │  PTY + OSC)  ││     │  └──────────────┘ │
│  └──────────────┘│     └──────────────────┘
└──────────────────┘

         impulse-terminal (new crate)
         ├── TerminalBackend (owns Term + EventLoop)
         ├── PTY spawning (replaces portable-pty in impulse-core)
         ├── Shell integration (reuses impulse-core::shell)
         ├── Grid state snapshot for rendering
         └── Input routing
```

### Key Design Decisions

1. **New `impulse-terminal` crate** — sits between `impulse-core` and the frontends. Owns the `alacritty_terminal::Term` instance, PTY event loop, and provides a clean API for frontends.

2. **Frontends only render** — each frontend receives a grid of `StyledCell` structs (character + fg/bg color + attributes) and draws them. No terminal emulation logic in frontend code.

3. **Shell integration consolidation** — the current `OscParser` in `impulse-core/pty.rs` becomes unnecessary because `alacritty_terminal` handles escape sequences. We hook into alacritty's event system for OSC 7/133 events, or add custom handling for sequences alacritty doesn't natively support.

4. **PTY management moves to `impulse-terminal`** — `alacritty_terminal` has its own PTY module and event loop. The `PtyManager` in `impulse-core` can be simplified or adapted to delegate to alacritty's PTY handling.

5. **Platform rendering is phase-appropriate** — start with CPU rendering (GTK4 `DrawingArea` / `NSView` `draw()`) for correctness, then optionally upgrade to GPU (wgpu) later.

---

## 3. Phase 1 — New `impulse-terminal` Crate

**Goal:** Create a new workspace crate that wraps `alacritty_terminal` and provides a high-level API for terminal emulation, independent of any GUI toolkit.

### 3.1 Create Crate Structure

```
impulse-terminal/
├── Cargo.toml
└── src/
    ├── lib.rs           # Public API
    ├── backend.rs       # TerminalBackend: owns Term + event loop
    ├── config.rs        # Map impulse settings → alacritty Config
    ├── event.rs         # TerminalEvent enum (frontend-facing events)
    ├── grid.rs          # Grid snapshot types for rendering
    ├── input.rs         # Keyboard/mouse input translation
    ├── pty.rs           # PTY spawning using alacritty's tty module
    └── shell.rs         # Shell integration (delegates to impulse-core::shell)
```

**Cargo.toml:**

```toml
[package]
name = "impulse-terminal"
version = "0.15.2"
edition = "2021"
description = "Terminal emulation backend for Impulse, built on alacritty_terminal"
license = "GPL-3.0-only"

[dependencies]
impulse-core = { path = "../impulse-core" }
alacritty_terminal = "0.25"
log = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
parking_lot = { workspace = true }
```

Add to workspace `Cargo.toml`:

```toml
members = ["impulse-core", "impulse-editor", "impulse-terminal", "impulse-linux", "impulse-ffi"]
```

### 3.2 `TerminalBackend` — Core Abstraction

This is the main type frontends interact with. One instance per terminal tab/split.

```rust
// backend.rs

use alacritty_terminal::term::Term;
use alacritty_terminal::event_loop::{EventLoop, Notifier};
use alacritty_terminal::grid::Dimensions;
use parking_lot::FairMutex;
use std::sync::Arc;

pub struct TerminalBackend {
    /// The terminal state (grid, cursor, scrollback, etc.)
    term: Arc<FairMutex<Term<TerminalEventProxy>>>,
    /// Channel to send input to the PTY
    notifier: Notifier,
    /// Receiver for events from the terminal (title change, bell, CWD, etc.)
    event_rx: crossbeam_channel::Receiver<TerminalEvent>,
    /// Terminal dimensions in cells
    cols: u16,
    rows: u16,
}

impl TerminalBackend {
    /// Create a new terminal and spawn a shell process.
    pub fn new(config: TerminalConfig, cols: u16, rows: u16) -> Result<Self, String>;

    /// Send keyboard input bytes to the PTY.
    pub fn write(&self, data: &[u8]);

    /// Resize the terminal grid and PTY.
    pub fn resize(&self, cols: u16, rows: u16);

    /// Get a snapshot of the visible grid for rendering.
    /// Returns a Vec<Vec<StyledCell>> representing rows × cols.
    pub fn grid_snapshot(&self) -> GridSnapshot;

    /// Get cursor position and style.
    pub fn cursor(&self) -> CursorState;

    /// Get current selection (if any) as cell ranges.
    pub fn selection(&self) -> Option<SelectionRange>;

    /// Poll for terminal events (non-blocking).
    /// Returns events like TitleChanged, Bell, CwdChanged, ChildExited.
    pub fn poll_events(&self) -> Vec<TerminalEvent>;

    /// Start a text selection at the given grid position.
    pub fn start_selection(&self, point: GridPoint, kind: SelectionKind);

    /// Update selection to the given grid position.
    pub fn update_selection(&self, point: GridPoint);

    /// Clear selection.
    pub fn clear_selection(&self);

    /// Get selected text as a string.
    pub fn selected_text(&self) -> Option<String>;

    /// Scroll the viewport (positive = up, negative = down).
    pub fn scroll(&self, delta: i32);

    /// Get the terminal's current title (set by OSC 0/2).
    pub fn title(&self) -> String;

    /// Get the terminal's current working directory (from OSC 7).
    pub fn cwd(&self) -> Option<String>;

    /// Shutdown the terminal and kill the child process.
    pub fn shutdown(&mut self);
}
```

### 3.3 `TerminalEvent` — Frontend-Facing Events

```rust
// event.rs

pub enum TerminalEvent {
    /// Terminal output changed, frontend should re-render.
    Wakeup,
    /// Terminal title changed (OSC 0/2).
    TitleChanged(String),
    /// Bell character received.
    Bell,
    /// Working directory changed (OSC 7).
    CwdChanged(String),
    /// Child process exited.
    ChildExited(Option<i32>),
    /// Clipboard store request (OSC 52).
    ClipboardStore(String),
    /// Clipboard load request (OSC 52).
    ClipboardLoad,
    /// Shell command started (OSC 133;B).
    CommandStart { block_id: String, command: String },
    /// Shell command ended (OSC 133;D).
    CommandEnd { block_id: String, exit_code: i32, duration_ms: u64 },
    /// Color request (OSC 10/11/12).
    ColorRequest(ColorRequestKind),
}
```

### 3.4 `GridSnapshot` — Rendering Data

```rust
// grid.rs

/// A complete snapshot of the visible terminal grid, ready for rendering.
pub struct GridSnapshot {
    pub rows: Vec<Vec<StyledCell>>,
    pub cursor: CursorState,
    pub selection: Option<SelectionRange>,
    pub cols: usize,
    pub lines: usize,
}

/// A single cell in the terminal grid.
#[derive(Clone)]
pub struct StyledCell {
    pub character: char,
    pub fg: RgbColor,
    pub bg: RgbColor,
    pub flags: CellFlags,
}

bitflags::bitflags! {
    pub struct CellFlags: u16 {
        const BOLD          = 0b0000_0001;
        const ITALIC        = 0b0000_0010;
        const UNDERLINE     = 0b0000_0100;
        const STRIKETHROUGH = 0b0000_1000;
        const DIM           = 0b0001_0000;
        const INVERSE       = 0b0010_0000;
        const HIDDEN        = 0b0100_0000;
        const WIDE_CHAR     = 0b1000_0000;
        const WIDE_SPACER   = 0b0000_0001_0000_0000;
        const HYPERLINK     = 0b0000_0010_0000_0000;
    }
}

#[derive(Clone, Copy)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub struct CursorState {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShape,
    pub visible: bool,
}

pub enum CursorShape {
    Block,
    Beam,
    Underline,
}
```

### 3.5 `TerminalConfig` — Settings Translation

```rust
// config.rs

/// Impulse terminal settings, translated to alacritty_terminal::Config internally.
pub struct TerminalConfig {
    pub scrollback_lines: usize,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub shell_path: String,
    pub shell_args: Vec<String>,
    pub working_directory: Option<String>,
    pub env_vars: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
    /// The 16-color ANSI palette + foreground + background.
    pub colors: TerminalColors,
}

pub struct TerminalColors {
    pub foreground: RgbColor,
    pub background: RgbColor,
    pub palette: [RgbColor; 16],
}
```

### 3.6 Shell Integration Consolidation

`alacritty_terminal` handles standard escape sequences (colors, cursor movement, alternate screen, etc.) but does **not** handle shell integration sequences (OSC 133 for command boundaries). Two options:

**Option A (Recommended): Keep `impulse-core`'s OSC 133 handling as a post-processor.**

- Hook into alacritty's `EventListener` to receive raw output events
- Run the existing `OscParser` (from `impulse-core/pty.rs`) on output before it reaches alacritty, stripping OSC 133 sequences and emitting `CommandStart`/`CommandEnd` events
- This preserves the current battle-tested shell integration code

**Option B: Patch alacritty_terminal to forward unrecognized OSC sequences.**

- More invasive, requires maintaining a fork
- Not recommended unless Option A proves insufficient

**OSC 7 (CWD tracking):** `alacritty_terminal` may already handle this — check if `Event::Title` or a custom event is emitted. If not, use the same post-processing approach as OSC 133.

### 3.7 Implementation Steps

1. Create `impulse-terminal/` directory and `Cargo.toml`
2. Add to workspace members
3. Implement `TerminalConfig` → `alacritty_terminal::Config` translation
4. Implement `TerminalEventProxy` (the `EventListener` trait)
5. Implement `TerminalBackend::new()` — create `Term`, spawn PTY via alacritty's `tty` module, start `EventLoop`
6. Implement `grid_snapshot()` — iterate `term.renderable_content()`, map to `StyledCell`
7. Implement input routing (`write()`, `resize()`)
8. Implement selection API
9. Implement event polling
10. Implement shell integration (OSC 133/7 handling)
11. Write unit tests for config translation, grid snapshot, event mapping
12. Verify with `cargo build -p impulse-terminal` and `cargo test -p impulse-terminal`

### 3.8 Handling alacritty_terminal's PTY vs impulse-core's PTY

Currently `impulse-core` uses `portable-pty` for PTY management. `alacritty_terminal` has its own `tty` module. **We should use alacritty's PTY handling** for terminals managed by `impulse-terminal`, because:

- alacritty's `EventLoop` is designed to work with its own PTY types
- Avoids bridging two PTY abstractions
- alacritty's PTY code is well-tested

The `PtyManager` in `impulse-core/pty.rs` can be:

- **Kept** for non-terminal PTY use cases (if any exist)
- **Simplified** to delegate to `impulse-terminal` for terminal sessions
- **Eventually removed** if all PTY usage goes through `impulse-terminal`

The shell detection and integration script injection in `impulse-core/shell.rs` remains useful — `impulse-terminal` will call `build_shell_command()` to get the properly configured `CommandBuilder` with integration scripts, then translate that into alacritty's spawn format.

---

## 4. Phase 2 — FFI Exposure for macOS

**Goal:** Expose `impulse-terminal` APIs through `impulse-ffi` so the macOS Swift frontend can use them.

### 4.1 FFI Functions to Add

```c
// Terminal lifecycle
void* impulse_terminal_create(const char* config_json);
void impulse_terminal_destroy(void* handle);

// Input
void impulse_terminal_write(void* handle, const uint8_t* data, size_t len);
void impulse_terminal_resize(void* handle, uint16_t cols, uint16_t rows);

// Rendering
char* impulse_terminal_grid_snapshot(void* handle);  // Returns JSON
char* impulse_terminal_cursor(void* handle);          // Returns JSON

// Selection
void impulse_terminal_start_selection(void* handle, uint16_t col, uint16_t row, const char* kind);
void impulse_terminal_update_selection(void* handle, uint16_t col, uint16_t row);
void impulse_terminal_clear_selection(void* handle);
char* impulse_terminal_selected_text(void* handle);

// Scrolling
void impulse_terminal_scroll(void* handle, int32_t delta);

// Events
char* impulse_terminal_poll_events(void* handle);  // Returns JSON array

// State queries
char* impulse_terminal_title(void* handle);
char* impulse_terminal_cwd(void* handle);
```

### 4.2 JSON Encoding Strategy

Complex types (grid snapshot, events) are encoded as JSON and returned as C strings. The Swift side decodes them using `Codable`. This matches the existing FFI pattern in `impulse-ffi`.

**Grid snapshot JSON format (optimized):**

```json
{
  "cols": 80,
  "lines": 24,
  "cursor": { "row": 5, "col": 10, "shape": "block", "visible": true },
  "cells": [
    { "r": 0, "c": 0, "ch": "h", "fg": [220, 215, 186], "bg": [31, 31, 40], "fl": 0 },
    ...
  ],
  "selection": { "start": [0, 5], "end": [20, 5] }
}
```

**Performance consideration:** Serializing the full grid as JSON every frame could be expensive at high refresh rates. Optimizations:

- Only send **changed cells** using alacritty's damage tracking (`term.damage()`)
- Use a binary format instead of JSON for the grid (MessagePack or custom)
- Send full snapshots infrequently, damage-based deltas for most frames

### 4.3 Implementation Steps

1. Add `impulse-terminal` as a dependency of `impulse-ffi`
2. Implement `extern "C"` wrapper functions
3. Add JSON serialization for `GridSnapshot`, `TerminalEvent`, `CursorState`
4. Update `impulse_ffi.h` header with new function declarations
5. Update `module.modulemap`
6. Verify with `cargo build -p impulse-ffi`

---

## 5. Phase 3 — macOS Frontend Integration

**Goal:** Replace SwiftTerm with the new `impulse-terminal` backend on macOS. The macOS frontend renders the terminal grid using native AppKit drawing.

### 5.1 New Swift Components

**`TerminalBackend.swift`** — Swift wrapper around FFI terminal functions:

```swift
class TerminalBackend {
    private var handle: UnsafeMutableRawPointer

    init(config: TerminalConfig) throws
    func write(_ data: Data)
    func resize(cols: UInt16, rows: UInt16)
    func gridSnapshot() -> GridSnapshot
    func pollEvents() -> [TerminalEvent]
    func startSelection(col: UInt16, row: UInt16, kind: SelectionKind)
    func updateSelection(col: UInt16, row: UInt16)
    func clearSelection()
    func selectedText() -> String?
    func scroll(delta: Int32)
    var title: String { get }
    var cwd: String? { get }
    func shutdown()
}
```

**`TerminalRenderer.swift`** — NSView subclass that renders the grid:

```swift
class TerminalRenderer: NSView {
    var backend: TerminalBackend
    var fontMetrics: FontMetrics  // cell width, cell height, baseline offset

    // Phase 1: CPU rendering via CoreText
    override func draw(_ dirtyRect: NSRect) {
        let snapshot = backend.gridSnapshot()
        // Draw cell backgrounds as filled rects
        // Draw text using CTLine/CTRun for each row
        // Draw cursor overlay
        // Draw selection highlight
    }

    // Input forwarding
    override func keyDown(with event: NSEvent) {
        // Translate NSEvent → terminal input bytes
        // Handle modifier keys (Ctrl, Alt, Shift)
        backend.write(translatedBytes)
    }

    override func mouseDown(with event: NSEvent) {
        // Convert pixel coords → grid coords using fontMetrics
        // Start selection or forward mouse event
    }

    // Refresh timer
    func startRefreshLoop() {
        // CVDisplayLink or Timer at ~60fps
        // Poll events, trigger redraw on Wakeup
    }
}
```

### 5.2 Font Metrics Calculation

Critical for correct rendering — must calculate exact cell dimensions:

```swift
struct FontMetrics {
    let cellWidth: CGFloat    // Width of a single character cell
    let cellHeight: CGFloat   // Height including line spacing
    let baseline: CGFloat     // Y offset from cell top to text baseline
    let descent: CGFloat      // Below baseline
    let font: CTFont

    init(fontFamily: String, fontSize: CGFloat) {
        let font = CTFontCreateWithName(fontFamily as CFString, fontSize, nil)
        // Use "W" or "M" to measure monospace cell width
        // cellHeight = ascent + descent + leading
    }
}
```

### 5.3 Keyboard Input Translation

Map macOS key events to terminal escape sequences:

| macOS Event       | Terminal Bytes                                   |
| ----------------- | ------------------------------------------------ |
| Regular character | UTF-8 bytes                                      |
| Enter             | `\r` (0x0D)                                      |
| Backspace         | `\x7F`                                           |
| Tab               | `\t` (0x09)                                      |
| Arrow keys        | `\x1B[A/B/C/D` (or application mode equivalents) |
| Ctrl+C            | `\x03`                                           |
| Ctrl+letter       | `letter - 0x60`                                  |
| Alt+letter        | `\x1B` + letter (meta encoding)                  |
| Shift+Enter       | `\x1B[13;2u` (CSI u)                             |
| Fn keys           | `\x1BOP` through `\x1B[24~`                      |
| Home/End          | `\x1B[H` / `\x1B[F`                              |
| Page Up/Down      | `\x1B[5~` / `\x1B[6~`                            |

**Note:** `alacritty_terminal` may provide input translation helpers. Check the `term::input` module.

### 5.4 Mouse Event Translation

For mouse-reporting modes (used by vim, tmux, etc.):

```swift
// Convert NSView coordinates to terminal grid coordinates
func gridPoint(from event: NSEvent) -> (col: Int, row: Int) {
    let point = convert(event.locationInWindow, from: nil)
    let col = Int(point.x / fontMetrics.cellWidth)
    let row = Int((bounds.height - point.y) / fontMetrics.cellHeight)
    return (col, row)
}
```

Mouse reporting modes to support:

- Normal tracking (click only)
- Button tracking (press + release)
- Any-event tracking (move + press + release)
- SGR extended coordinates (for grids > 223 columns)

### 5.5 What Changes in Existing macOS Code

| File                        | Change                                                                                                                                                                                      |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TerminalTab.swift`         | Replace `SwiftTerm.LocalProcessTerminalView` with `TerminalRenderer` + `TerminalBackend`. Remove all SwiftTerm delegate methods. Shell spawning moves to Rust via `TerminalBackend.init()`. |
| `TerminalContainer.swift`   | Replace `ImpulseTerminalView` references with `TerminalRenderer`. Split logic (NSSplitView) stays the same.                                                                                 |
| `Package.swift`             | Remove `SwiftTerm` dependency.                                                                                                                                                              |
| `ImpulseCore.swift`         | Add Swift wrappers for new FFI terminal functions.                                                                                                                                          |
| `CImpulseFFI/impulse_ffi.h` | Add new terminal function declarations.                                                                                                                                                     |

### 5.6 Drag-and-Drop

Reimplement on `TerminalRenderer`:

```swift
// Register for drag types in init
registerForDraggedTypes([.fileURL, .string])

override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
    // Extract file URLs or text
    // Shell-escape paths
    // Send via backend.write() with bracketed paste if appropriate
}
```

### 5.7 Copy-on-Select

```swift
// In mouseUp handler:
if let text = backend.selectedText(), !text.isEmpty {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(text, forType: .string)
}
```

### 5.8 Image Paste Fallback

Same logic as current implementation — detect image-only clipboard, save to temp PNG, paste path.

### 5.9 Implementation Steps

1. Create `TerminalBackend.swift` (Swift FFI wrapper)
2. Create `FontMetrics` calculation
3. Create `TerminalRenderer.swift` (NSView subclass) with basic cell rendering
4. Implement keyboard input translation (keyDown → bytes)
5. Implement mouse input forwarding
6. Implement selection (click-drag → start/update/clear selection)
7. Implement scrolling (scroll wheel → `scroll()`)
8. Wire up event polling (Wakeup → setNeedsDisplay, TitleChanged, CwdChanged, ChildExited)
9. Update `TerminalTab.swift` to use new renderer instead of SwiftTerm
10. Implement drag-and-drop
11. Implement copy-on-select and clipboard (Cmd+C/Cmd+V)
12. Implement Shift+Enter special handling
13. Test: basic shell interaction, vim/tmux, colors, scrollback, splits
14. Remove SwiftTerm from `Package.swift`

---

## 6. Phase 4 — Linux Frontend Integration

**Goal:** Replace VTE4 with the new `impulse-terminal` backend on Linux. The Linux frontend renders the terminal grid using GTK4 drawing.

### 6.1 Rendering Approach — GTK4 `DrawingArea`

Replace `vte4::Terminal` with a `gtk4::DrawingArea` that renders the terminal grid:

```rust
// linux_renderer.rs

use gtk4::prelude::*;
use gtk4::{DrawingArea, cairo};
use impulse_terminal::{TerminalBackend, GridSnapshot, StyledCell};

pub struct TerminalWidget {
    pub drawing_area: DrawingArea,
    pub backend: Rc<RefCell<TerminalBackend>>,
    font_metrics: Rc<RefCell<FontMetrics>>,
}

struct FontMetrics {
    cell_width: f64,
    cell_height: f64,
    baseline: f64,
    font_desc: pango::FontDescription,
}
```

### 6.2 Drawing with Cairo + Pango

```rust
drawing_area.set_draw_func(move |_widget, cr, width, height| {
    let backend = backend.borrow();
    let snapshot = backend.grid_snapshot();
    let metrics = font_metrics.borrow();

    // 1. Fill background
    cr.set_source_rgb(bg.r, bg.g, bg.b);
    cr.paint().ok();

    // 2. Draw cell backgrounds (only non-default backgrounds)
    for (row_idx, row) in snapshot.rows.iter().enumerate() {
        for (col_idx, cell) in row.iter().enumerate() {
            if cell.bg != default_bg {
                let x = col_idx as f64 * metrics.cell_width;
                let y = row_idx as f64 * metrics.cell_height;
                cr.set_source_rgb(cell.bg.r, cell.bg.g, cell.bg.b);
                cr.rectangle(x, y, metrics.cell_width, metrics.cell_height);
                cr.fill().ok();
            }
        }
    }

    // 3. Draw text row by row using Pango
    for (row_idx, row) in snapshot.rows.iter().enumerate() {
        let y = row_idx as f64 * metrics.cell_height + metrics.baseline;
        // Build attributed text for the row (grouping consecutive same-color chars)
        // Use PangoLayout for each color run
        for run in color_runs(row) {
            let layout = pangocairo::create_layout(cr);
            layout.set_font_description(Some(&metrics.font_desc));
            layout.set_text(&run.text);
            cr.move_to(run.start_col as f64 * metrics.cell_width, y);
            cr.set_source_rgb(run.fg.r, run.fg.g, run.fg.b);
            pangocairo::show_layout(cr, &layout);
        }
    }

    // 4. Draw cursor
    let cursor = &snapshot.cursor;
    if cursor.visible {
        let x = cursor.col as f64 * metrics.cell_width;
        let y = cursor.row as f64 * metrics.cell_height;
        // Draw block/beam/underline cursor
    }

    // 5. Draw selection highlight (semi-transparent overlay)
    if let Some(sel) = &snapshot.selection {
        // Iterate selected cells, draw highlight rectangles
    }
});
```

### 6.3 Input Handling

**Keyboard:** Use a `gtk4::EventControllerKey` on the DrawingArea:

```rust
let key_controller = gtk4::EventControllerKey::new();
key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, modifiers| {
    let bytes = translate_key_to_terminal_bytes(keyval, modifiers);
    backend.borrow().write(&bytes);
    glib::Propagation::Stop
});
drawing_area.add_controller(key_controller);
```

**Key translation table** — same mappings as macOS (Section 5.3) but using GDK key constants:

```rust
fn translate_key_to_terminal_bytes(keyval: gdk4::Key, mods: gdk4::ModifierType) -> Vec<u8> {
    match keyval {
        gdk4::Key::Return if mods.contains(gdk4::ModifierType::SHIFT_MASK) => {
            b"\x1b[13;2u".to_vec()  // Shift+Enter CSI u
        }
        gdk4::Key::Return => vec![0x0D],
        gdk4::Key::BackSpace => vec![0x7F],
        gdk4::Key::Tab => vec![0x09],
        gdk4::Key::Up => b"\x1b[A".to_vec(),
        gdk4::Key::Down => b"\x1b[B".to_vec(),
        gdk4::Key::Right => b"\x1b[C".to_vec(),
        gdk4::Key::Left => b"\x1b[D".to_vec(),
        // ... etc
        _ => {
            if mods.contains(gdk4::ModifierType::CONTROL_MASK) {
                // Ctrl+letter → control code
                if let Some(c) = keyval.to_unicode() {
                    if c.is_ascii_lowercase() {
                        return vec![(c as u8) - 0x60];
                    }
                }
            }
            // Regular character
            if let Some(c) = keyval.to_unicode() {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            } else {
                vec![]
            }
        }
    }
}
```

**Mouse:** Use `gtk4::GestureClick` and `gtk4::EventControllerMotion`:

```rust
// Click handler for selection start
let click = gtk4::GestureClick::new();
click.connect_pressed(move |_gesture, _n_press, x, y| {
    let col = (x / cell_width) as u16;
    let row = (y / cell_height) as u16;
    backend.borrow().start_selection(GridPoint { col, row }, SelectionKind::Simple);
});
drawing_area.add_controller(click);

// Motion handler for selection update during drag
let motion = gtk4::EventControllerMotion::new();
motion.connect_motion(move |_ctrl, x, y| {
    if selecting {
        let col = (x / cell_width) as u16;
        let row = (y / cell_height) as u16;
        backend.borrow().update_selection(GridPoint { col, row });
    }
});
drawing_area.add_controller(motion);
```

**Scroll:**

```rust
let scroll_ctrl = gtk4::EventControllerScroll::new(
    gtk4::EventControllerScrollFlags::VERTICAL
);
scroll_ctrl.connect_scroll(move |_ctrl, _dx, dy| {
    let lines = (dy * 3.0) as i32;  // 3 lines per scroll notch
    backend.borrow().scroll(lines);
    glib::Propagation::Stop
});
drawing_area.add_controller(scroll_ctrl);
```

### 6.4 Refresh Loop

```rust
// Poll terminal events at ~60fps using glib timeout
glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
    let events = backend.borrow().poll_events();
    for event in events {
        match event {
            TerminalEvent::Wakeup => drawing_area.queue_draw(),
            TerminalEvent::TitleChanged(title) => { /* update tab title */ },
            TerminalEvent::CwdChanged(path) => { /* update status bar, sidebar */ },
            TerminalEvent::ChildExited(code) => { /* close tab or remove split */ },
            TerminalEvent::Bell => { /* play bell if enabled */ },
            _ => {}
        }
    }
    glib::ControlFlow::Continue
});
```

### 6.5 Font Metrics Calculation (Pango)

```rust
fn calculate_font_metrics(
    widget: &gtk4::Widget,
    font_family: &str,
    font_size: i32,
) -> FontMetrics {
    let font_desc = pango::FontDescription::from_string(
        &format!("{} {}", font_family, font_size)
    );
    let pango_ctx = widget.pango_context();
    let metrics = pango_ctx.metrics(Some(&font_desc), None);

    let cell_width = metrics.approximate_char_width() as f64 / pango::SCALE as f64;
    let ascent = metrics.ascent() as f64 / pango::SCALE as f64;
    let descent = metrics.descent() as f64 / pango::SCALE as f64;
    let cell_height = ascent + descent;

    FontMetrics {
        cell_width,
        cell_height,
        baseline: ascent,
        font_desc,
    }
}
```

### 6.6 What Changes in Existing Linux Code

| File                         | Change                                                                                                                                                                                 |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `terminal.rs`                | Replace `create_terminal()` → create `TerminalWidget`. Remove all VTE API calls. Shell spawning moves to `TerminalBackend::new()`. Keep drag-and-drop but rewire to `backend.write()`. |
| `terminal_container.rs`      | Replace `vte4::Terminal` references with `TerminalWidget`. `Paned` split logic stays the same but wraps `DrawingArea` instead of VTE widget. Update `find_focused_terminal()` etc.     |
| `window/tab_management.rs`   | Remove VTE signal connections (`connect_current_directory_uri_notify`, `connect_child_exited`, `connect_selection_changed`). Replace with event polling from `TerminalBackend`.        |
| `window/keybinding_setup.rs` | Terminal-specific keybindings (Ctrl+V paste, etc.) may need adjustment since input routing changes.                                                                                    |
| `Cargo.toml`                 | Remove `vte4` dependency. Add `impulse-terminal`.                                                                                                                                      |
| `settings.rs`                | No change (settings struct stays the same).                                                                                                                                            |
| `theme.rs`                   | Remove VTE-specific CSS. Add color mappings for the new renderer.                                                                                                                      |

### 6.7 GTK4 `DrawingArea` Considerations

- **Focus:** `DrawingArea` needs `set_focusable(true)` and `set_can_focus(true)` to receive keyboard events.
- **IME support:** May need `gtk4::IMContextSimple` or `gtk4::IMMulticontext` for input method support (CJK, etc.). VTE handled this automatically.
- **Accessibility:** VTE provided screen reader support via ATK/AT-SPI. With a custom `DrawingArea`, accessibility is lost unless we implement `AccessibleText` interface. This is a known regression to address post-migration.
- **Minimum size:** Set `set_content_width()` and `set_content_height()` based on initial grid size × font metrics.

### 6.8 Implementation Steps

1. Add `impulse-terminal` dependency to `impulse-linux/Cargo.toml`
2. Create `linux_terminal_renderer.rs` with `TerminalWidget` struct
3. Implement Cairo+Pango rendering in `set_draw_func`
4. Implement font metrics calculation
5. Implement keyboard input controller + key translation
6. Implement mouse input (click, drag, motion for selection)
7. Implement scroll controller
8. Implement refresh loop (glib timeout + event polling)
9. Wire `TerminalWidget` into `terminal.rs` (replace `create_terminal()`)
10. Update `terminal_container.rs` for split management with new widget
11. Update `tab_management.rs` to use event polling instead of VTE signals
12. Implement drag-and-drop on `DrawingArea`
13. Implement clipboard (Ctrl+C/Ctrl+V via GDK clipboard API)
14. Implement copy-on-select
15. Test: basic shell interaction, vim/tmux, colors, scrollback, splits, resize
16. Remove `vte4` from `Cargo.toml`

---

## 7. Phase 5 — Remove Old Dependencies

### 7.1 Cleanup Checklist

- [ ] Remove `vte4` from `impulse-linux/Cargo.toml`
- [ ] Remove `SwiftTerm` from `impulse-macos/Package.swift`
- [ ] Remove `portable-pty` from `impulse-core/Cargo.toml` (if no longer used)
- [ ] Remove or simplify `impulse-core/src/pty.rs` (the `PtyManager` and `OscParser`)
- [ ] Update `impulse-linux` system dependency metadata (deb `depends`, rpm `requires`) to remove `libvte-2.91-gtk4-0` / `vte4`
- [ ] Update CLAUDE.md architecture section
- [ ] Update README system requirements
- [ ] Run `cargo clippy` and fix warnings
- [ ] Run full test suite

### 7.2 What to Keep in `impulse-core`

- `shell.rs` — Shell detection and integration script injection (still needed by `impulse-terminal`)
- `shell_integration/*.sh` — Shell integration scripts (still needed)
- `pty.rs` — **Evaluate:** If `impulse-terminal` fully replaces PTY management, the `PtyManager` can be removed. The `OscParser` may still be needed if alacritty doesn't handle OSC 133. Keep `PtyMessage` and `PtyEventSender` as they may be useful abstractions.

---

## 8. Rendering Strategy

### 8.1 Phase 1: CPU Rendering (Ship First)

| Platform | Renderer            | Drawing API             |
| -------- | ------------------- | ----------------------- |
| Linux    | `gtk4::DrawingArea` | Cairo + Pango           |
| macOS    | `NSView` subclass   | CoreGraphics + CoreText |

**Pros:** Simpler, integrates naturally with each toolkit, no surface embedding complexity.
**Cons:** Slower than GPU for high-throughput output, no ligature-optimized pipeline.

This is sufficient for shipping and may be "good enough" permanently for many use cases.

### 8.2 Phase 2: GPU Rendering (Future, Optional)

| Platform | Surface                             | Renderer                     |
| -------- | ----------------------------------- | ---------------------------- |
| Linux    | GTK4 `GLArea` or subsurface overlay | wgpu (Vulkan/OpenGL backend) |
| macOS    | `NSView` + `CAMetalLayer`           | wgpu (Metal backend)         |

**Text rendering stack:** `glyphon` (wraps `cosmic-text` + `etagere` + wgpu)

**When to pursue this:**

- After CPU rendering is stable and feature-complete
- If users report performance issues with high-throughput output
- If we want to add visual features that benefit from GPU (smooth scrolling, transparency, custom shaders)

**The GTK4 + wgpu embedding problem** (see Risk Register) makes this harder on Linux. The macOS path is straightforward.

---

## 9. Feature Parity Checklist

Every feature currently supported must work after migration. Check off as implemented.

### Terminal Emulation

- [ ] Basic text input/output
- [ ] ANSI color (16 colors + 256 colors + 24-bit truecolor)
- [ ] Cursor styles (block, beam, underline)
- [ ] Cursor blink
- [ ] Bold, italic, underline, strikethrough, dim, inverse, hidden
- [ ] Wide characters (CJK)
- [ ] Alternate screen buffer (vim, less, etc.)
- [ ] Scrollback buffer (configurable size)
- [ ] Tab stops
- [ ] Line wrapping
- [ ] Terminal resize (SIGWINCH)

### Input

- [ ] Regular character input (including Unicode/emoji)
- [ ] Modifier keys (Ctrl, Alt/Meta, Shift)
- [ ] Arrow keys, Home, End, Page Up/Down, Insert, Delete
- [ ] Function keys (F1-F12)
- [ ] Shift+Enter → `\x1B[13;2u`
- [ ] Ctrl+V / Cmd+V paste
- [ ] Bracketed paste mode
- [ ] Mouse reporting (normal, button, any-event, SGR extended)
- [ ] Mouse scroll wheel
- [ ] IME input (CJK input methods)

### Selection & Clipboard

- [ ] Click-and-drag text selection
- [ ] Double-click word selection
- [ ] Triple-click line selection
- [ ] Copy selected text (Ctrl+C / Cmd+C)
- [ ] Copy-on-select option
- [ ] Paste from clipboard
- [ ] Image paste fallback (save temp PNG, paste path)

### Shell Integration

- [ ] OSC 7 (CWD tracking) → status bar + sidebar update
- [ ] OSC 133 (command boundaries) → CommandStart/CommandEnd events
- [ ] Shell integration script injection (bash, zsh, fish)

### Terminal Features

- [ ] Audible bell
- [ ] OSC 0/2 (window/tab title)
- [ ] OSC 8 (hyperlinks) — if supported by alacritty_terminal
- [ ] OSC 52 (clipboard via escape sequence)
- [ ] Scrollback search (optional — currently not exposed in Impulse UI)

### Container & Splitting

- [ ] Horizontal split
- [ ] Vertical split
- [ ] Nested splits
- [ ] Focus navigation between splits
- [ ] New split inherits CWD from focused terminal
- [ ] Close split (remove from tree, rebalance)

### Drag & Drop

- [ ] File drop → shell-escaped paths pasted
- [ ] Text drop → text pasted
- [ ] Respect bracketed paste mode during drops

### Platform-Specific

- [ ] **Linux:** GTK4 clipboard integration
- [ ] **Linux:** System font fallback
- [ ] **macOS:** Cmd+C/Cmd+V keyboard shortcuts
- [ ] **macOS:** Scroll wheel forwarding for TUI apps
- [ ] **macOS:** Process termination (SIGHUP → SIGTERM → SIGKILL escalation)

---

## 10. Risk Register

### High Risk

**R1: GTK4 `DrawingArea` keyboard input edge cases**

- IME support, dead keys, compose sequences may not work correctly without VTE's input handling
- **Mitigation:** Test with IME early. May need `IMMulticontext` integration. Look at how other GTK4 apps (e.g., GNOME Text Editor) handle input on `DrawingArea`.

**R2: Performance of CPU rendering at high throughput**

- `cat large_file` or rapid build output could be slow with Cairo+Pango per-row rendering
- **Mitigation:** Use alacritty's damage tracking to only redraw changed regions. Batch rows into a single Pango layout where possible. Profile early.

**R3: `alacritty_terminal` API stability**

- Pre-1.0 crate, breaking changes on minor bumps. Zed and Lapce use forks.
- **Mitigation:** Pin to exact version in `Cargo.toml` (e.g., `=0.25.1`). Consider vendoring if breakage becomes frequent. Keep the wrapper layer (`impulse-terminal`) thin enough to adapt.

### Medium Risk

**R4: Mouse reporting mode compatibility**

- TUI apps (vim, tmux, htop) rely on precise mouse reporting. Incorrect coordinate translation or missing modes will break them.
- **Mitigation:** Test with vim, tmux, htop, mc, and other mouse-aware TUI apps. Refer to alacritty's own mouse handling code.

**R5: Accessibility regression on Linux**

- VTE4 provides AT-SPI accessibility (screen reader support). Custom `DrawingArea` does not.
- **Mitigation:** Document as known regression. Implement `AccessibleText` in a follow-up phase.

**R6: Grid snapshot performance over FFI (macOS)**

- Serializing full grid as JSON every frame could be expensive.
- **Mitigation:** Use damage tracking for incremental updates. Profile and optimize JSON encoding. Consider binary format if needed.

### Low Risk

**R7: Escape sequence coverage gaps**

- `alacritty_terminal` may not support every sequence VTE/SwiftTerm did.
- **Mitigation:** `alacritty_terminal` is mature and widely used. Edge cases can be patched or worked around.

**R8: Shell integration script conflicts**

- Shell integration scripts may interact differently with alacritty's parser vs VTE/SwiftTerm.
- **Mitigation:** Test all three shells (bash, zsh, fish). The scripts emit standard OSC sequences that should work with any parser.

---

## 11. Reference Material

### Projects to Study

| Project            | What to Learn                                                      | Repo                                      |
| ------------------ | ------------------------------------------------------------------ | ----------------------------------------- |
| **Zed** (terminal) | How they wrap `alacritty_terminal`, event handling, grid rendering | `zed-industries/zed` → `crates/terminal/` |
| **COSMIC Term**    | Using `alacritty_terminal` with `cosmic-text` rendering            | `pop-os/cosmic-term`                      |
| **iced_term**      | Simple `alacritty_terminal` integration in an Iced widget          | `ppalermo/iced_term`                      |
| **egui_term**      | Simple `alacritty_terminal` integration in egui                    | `junkdog/egui_term`                       |
| **Alacritty**      | Input handling, mouse events, config, rendering                    | `alacritty/alacritty`                     |

### Key alacritty_terminal Types

```rust
// Core terminal state
alacritty_terminal::term::Term<T: EventListener>
alacritty_terminal::term::RenderableContent
alacritty_terminal::term::RenderableCell

// Grid
alacritty_terminal::grid::Grid<T>
alacritty_terminal::grid::Dimensions

// Events
alacritty_terminal::event::Event
alacritty_terminal::event::EventListener

// PTY
alacritty_terminal::tty::EventedReadWrite
alacritty_terminal::event_loop::EventLoop
alacritty_terminal::event_loop::Notifier

// Config
alacritty_terminal::term::Config

// Selection
alacritty_terminal::selection::Selection
alacritty_terminal::selection::SelectionType
```

### Useful crates

| Crate                | Purpose                                            |
| -------------------- | -------------------------------------------------- |
| `alacritty_terminal` | Terminal emulation (grid, parser, PTY, event loop) |
| `bitflags`           | Cell attribute flags                               |
| `crossbeam-channel`  | Event channel between PTY thread and frontend      |
| `parking_lot`        | `FairMutex` for `Term` (same as COSMIC Term uses)  |

---

## Implementation Order Summary

```
Phase 1: impulse-terminal crate        ← Do first, can test standalone
Phase 2: FFI exposure                   ← Needed before macOS work
Phase 3: macOS integration              ← Replace SwiftTerm
Phase 4: Linux integration              ← Replace VTE4
Phase 5: Cleanup                        ← Remove old deps
```

Phases 3 and 4 are independent and can be done in parallel by different developers or on different machines. Phase 1 and 2 must be done first.
