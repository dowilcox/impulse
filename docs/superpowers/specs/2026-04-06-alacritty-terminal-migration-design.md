# Design: Replace SwiftTerm with alacritty_terminal on macOS

**Date:** 2026-04-06
**Scope:** macOS frontend only. The Rust crate is cross-platform; Linux adoption is a separate spec.
**Status:** Approved

---

## 1. Motivation

SwiftTerm is the current terminal emulator on macOS. It works but has fundamental limitations:

1. **Owns its own PTY** — impulse-core's PtyManager sits unused on macOS
2. **Monolithic design** — parsing, state, rendering, and PTY are inseparable
3. **5 NSEvent monitor workarounds** — arrow key modifier bug, scroll wheel forwarding, alt screen redraw, paste newline handling, Shift+Enter
4. **Terminal search is a stub** — never implemented in SwiftTerm's SearchService
5. **SwiftUI integration friction** — PTY sizing breaks inside NSHostingView
6. **Single maintainer** — sole dependency on one developer's spare-time project

A prior migration attempt (branch `alacritty-terminal`, commit `28c9254`) reached macOS Phase 3 (SwiftTerm fully removed, app building and running) but was never merged. It proved the architecture works. The issues that stalled it were:

- Cell-by-cell CoreText rendering was slow
- JSON grid snapshot serialization was a per-frame bottleneck
- SGR mouse encoding was incorrect, breaking TUI app scrolling
- IME input was not implemented
- Cmd+W caused beach balls due to PTY shutdown ordering

This design addresses all of those issues.

## 2. Key Decisions

| Decision       | Choice                     | Rationale                                                                                     |
| -------------- | -------------------------- | --------------------------------------------------------------------------------------------- |
| Rendering      | CoreText run-based         | Proven, simpler than Metal, fast enough. Group same-styled cells into attributed string runs. |
| PTY ownership  | Alacritty's event loop     | Simpler than wiring impulse-core's PtyManager. Proven in old branch.                          |
| Grid transport | Flat binary buffer via FFI | Eliminates JSON encode/decode per frame. Fixed 12 bytes/cell.                                 |
| Scope          | macOS only                 | Linux is a separate toolkit/renderer. Rust crate is cross-platform by design.                 |
| IME            | Deferred                   | Non-trivial NSTextInputClient conformance. Not blocking for primary use case.                 |

## 3. Architecture

```
                    ┌──────────────────────────────────────────┐
                    │         impulse-macos (Swift)             │
                    │                                          │
                    │  TerminalTab                             │
                    │    ├── TerminalBackend (FFI wrapper)     │
                    │    │     └── binary buffer (reusable)    │
                    │    └── TerminalRenderer (NSView)         │
                    │          ├── CoreText run-based drawing  │
                    │          ├── KeyEncoder (input)          │
                    │          └── CVDisplayLink refresh       │
                    └──────────────┬───────────────────────────┘
                                   │ C FFI (18 functions)
                    ┌──────────────┴───────────────────────────┐
                    │         impulse-ffi (Rust, static lib)    │
                    └──────────────┬───────────────────────────┘
                                   │
                    ┌──────────────┴───────────────────────────┐
                    │         impulse-terminal (Rust crate)     │
                    │                                          │
                    │  TerminalBackend                         │
                    │    ├── Term<EventProxy> (alacritty)      │
                    │    ├── EventLoop + PTY (alacritty)       │
                    │    ├── Binary buffer packing             │
                    │    ├── RegexSearch (alacritty)           │
                    │    └── crossbeam event channel           │
                    └──────────────────────────────────────────┘
```

## 4. Rust Crate: `impulse-terminal`

New workspace member. Depends on `alacritty_terminal`, `crossbeam-channel`, `serde`.

### 4.1 Modules

**`backend.rs`** — `TerminalBackend` struct.

Owns:

- `Arc<FairMutex<Term<EventProxy>>>` — alacritty's terminal state
- `EventLoopSender` — sends input/resize/shutdown to PTY event loop
- `Receiver<TerminalEvent>` — receives terminal events via crossbeam channel
- `Receiver<String>` — receives PtyWrite events for forwarding
- `ConfiguredColors` — pre-computed 269-color palette for resolving named/indexed colors

Public methods:

- `new(config, cols, rows, cell_width, cell_height) -> Result<Self, String>`
- `write(data: &[u8])` — send input bytes to PTY
- `resize(cols, rows, cell_width, cell_height)` — resize grid + PTY
- `scroll(delta: i32)` / `scroll_to_bottom()`
- `start_selection(col, row, kind)` / `update_selection(col, row)` / `clear_selection()` / `selected_text() -> Option<String>`
- `poll_events() -> Vec<TerminalEvent>`
- `mode() -> TerminalMode`
- `set_focus(focused: bool)`
- `child_pid() -> u32`
- `shutdown()`
- `write_grid_to_buffer(buf: &mut [u8]) -> usize` — pack grid into binary buffer
- `grid_buffer_size() -> usize` — required buffer size for current dimensions
- `search(pattern: &str) -> SearchResult` / `search_next()` / `search_prev()` / `search_clear()`

**`buffer.rs`** — Binary buffer format.

Cell layout (12 bytes, fixed stride):

```
Offset  Size  Field
0       4     character (UTF-32 codepoint, little-endian)
4       1     fg red
5       1     fg green
6       1     fg blue
7       1     bg red
8       1     bg green
9       1     bg blue
10      2     flags (CellFlags bitfield, little-endian)
```

Buffer layout:

```
Offset  Size              Field
0       2                 cols (u16 LE)
2       2                 lines (u16 LE)
4       2                 cursor row (u16 LE)
6       2                 cursor col (u16 LE)
8       1                 cursor shape (u8: 0=Block,1=Beam,2=Underline,3=HollowBlock,4=Hidden)
9       1                 cursor visible (u8: 0/1)
10      2                 mode flags (u16 LE bitfield: bit0=show_cursor, bit1=app_cursor,
                                    bit2=app_keypad, bit3=mouse_report_click, bit4=mouse_motion,
                                    bit5=mouse_drag, bit6=mouse_sgr, bit7=bracketed_paste,
                                    bit8=focus_in_out, bit9=alt_screen, bit10=line_wrap)
12      2                 selection range count (u16 LE)
14      2                 search match range count (u16 LE)
16      N*6               selection ranges (N entries: row u16 + start_col u16 + end_col u16)
16+N*6  M*6               search match ranges (M entries: same format)
HEADER  cols*lines*12     cell data (row-major order)
```

**`config.rs`** — `TerminalConfig` struct (Deserialize). Translates to alacritty's `Config` and `PtyOptions`.

Fields: `scrollback_lines`, `cursor_shape`, `cursor_blink`, `shell_path`, `shell_args`, `working_directory`, `env_vars`, `colors` (foreground, background, 16-color palette).

**`event.rs`** — `TerminalEvent` enum (Serialize).

Variants: `Wakeup`, `TitleChanged(String)`, `ResetTitle`, `Bell`, `ChildExited(i32)`, `ClipboardStore(String)`, `ClipboardLoad`, `CursorBlinkingChange`, `Exit`.

**`grid.rs`** — Platform-agnostic grid types. No alacritty types in public API.

Types: `RgbColor`, `CellFlags` (bitfield), `StyledCell`, `CursorShape`, `CursorState`, `TerminalMode`, `GridSnapshot`.

**`search.rs`** — Wraps alacritty's `RegexSearch`.

`SearchResult`: `{ match_row, match_start_col, match_end_col, total_matches, current_match_index }`.

**`lib.rs`** — Re-exports all public types and `TerminalBackend`.

### 4.2 Color Resolution

Same approach as old branch. `ConfiguredColors` pre-computes a 269-entry palette:

- Indices 0-15: ANSI colors from config
- Indices 16-231: 6x6x6 color cube
- Indices 232-255: grayscale ramp
- Named colors (foreground, background, cursor, dim variants)

Resolves `alacritty_terminal::vte::ansi::Color` to `RgbColor`, checking terminal color overrides first (apps can change colors at runtime).

## 5. FFI Layer: `impulse-ffi`

18 `extern "C"` functions added to `impulse-ffi/src/lib.rs`.

### 5.1 Function Signatures

```rust
// Lifecycle
impulse_terminal_create(config_json: *const c_char, cols: u16, rows: u16, cell_width: u16, cell_height: u16) -> *mut TerminalHandle
impulse_terminal_destroy(handle: *mut TerminalHandle)

// I/O
impulse_terminal_write(handle: *mut TerminalHandle, data: *const u8, len: usize)
impulse_terminal_resize(handle: *mut TerminalHandle, cols: u16, rows: u16, cell_width: u16, cell_height: u16)

// Grid snapshot (binary)
impulse_terminal_grid_snapshot(handle: *mut TerminalHandle, out_buf: *mut u8, buf_len: usize) -> usize
impulse_terminal_grid_snapshot_size(handle: *mut TerminalHandle) -> usize

// Events
impulse_terminal_poll_events(handle: *mut TerminalHandle) -> *mut c_char  // JSON

// Selection
impulse_terminal_start_selection(handle: *mut TerminalHandle, col: u16, row: u16, kind: u8)
impulse_terminal_update_selection(handle: *mut TerminalHandle, col: u16, row: u16)
impulse_terminal_clear_selection(handle: *mut TerminalHandle)
impulse_terminal_selected_text(handle: *mut TerminalHandle) -> *mut c_char

// Search
impulse_terminal_search(handle: *mut TerminalHandle, pattern: *const c_char) -> *mut c_char  // JSON
impulse_terminal_search_next(handle: *mut TerminalHandle) -> *mut c_char
impulse_terminal_search_prev(handle: *mut TerminalHandle) -> *mut c_char
impulse_terminal_search_clear(handle: *mut TerminalHandle)

// State
impulse_terminal_mode(handle: *mut TerminalHandle) -> *mut c_char  // JSON
impulse_terminal_set_focus(handle: *mut TerminalHandle, focused: bool)
impulse_terminal_child_pid(handle: *mut TerminalHandle) -> u32
impulse_terminal_scroll(handle: *mut TerminalHandle, delta: i32)
```

### 5.2 TerminalHandle

Opaque wrapper around `Box<TerminalBackend>`. Created via `Box::into_raw`, freed via `Box::from_raw` in destroy. All functions guard against null handles.

## 6. Swift Frontend

### 6.1 TerminalBackend.swift

Swift wrapper around FFI. One instance per terminal tab/split.

Key design: pre-allocates a reusable `UnsafeMutableRawPointer` buffer for grid snapshots. Sized on creation and resized on `resize()`. The renderer reads directly from this buffer every frame — zero per-frame allocation.

`GridBufferReader` struct provides typed access:

- `cols`, `lines`, `cursor`, `modeFlags` read from header
- `cell(row:col:)` computes offset and returns character, fg, bg, flags inline
- `selectionRanges` / `searchMatchRanges` iterate the variable-length header section

### 6.2 TerminalRenderer.swift

`NSView` subclass. `isFlipped = true`. Renders the terminal grid via CoreText.

**Run-based drawing algorithm:**

For each row:

1. Scan cells left to right
2. Accumulate consecutive cells with matching fg color + bold + italic into a `currentRun` string
3. When style changes or a box-drawing character is encountered:
   - Flush `currentRun`: create `NSAttributedString` with the accumulated characters and style, create `CTLine`, position at `(padding + startCol * cellWidth, rowY + ascent)`, call `CTLineDraw`
   - Start new run with the new style
4. Box-drawing characters (U+2500-U+259F) are drawn programmatically with `CGContext` path operations for pixel-perfect cell connections. Common subset (~30 chars): light/heavy lines, corners, T-junctions, crosses, rounded corners, block elements. Others fall back to font glyph.
5. After all rows, draw cursor overlay

**Background drawing:** Separate pass before text. Scan each row for spans of non-default background color and batch `CGContext.fill()` per span (not per cell).

**Selection and search highlights:** Drawn as semi-transparent filled rectangles between background and text passes. Selection: blue (`rgba(0.3, 0.5, 0.8, 0.3)`). Search matches: amber (`rgba(0.8, 0.6, 0.2, 0.3)`). Current search match: brighter amber (`rgba(0.9, 0.7, 0.1, 0.5)`).

**Refresh:** `CVDisplayLink` callback on display thread. Calls `backend.pollEvents()`, checks for `Wakeup`, sets `needsDisplay = true` on main thread only when content changed. Skips viewport-to-bottom snap when `isScrolledBack` is true.

**Input:**

- `keyDown(with:)` → Cmd shortcuts (copy/paste) handled first, then `KeyEncoder.encode(event, mode)` → `backend.write(bytes)`
- Mouse events via standard NSView responder methods (no NSEvent monitors)
- `scrollWheel(with:)` → accumulate deltas, forward as scroll or SGR mouse events depending on terminal mode

### 6.3 KeyEncoder.swift

Stateless struct. `encode(event: NSEvent, mode: TerminalModeFlags) -> [UInt8]`.

Handles:

- Special keys: arrows (respects app cursor mode), function keys F1-F12, home/end/pgup/pgdn, backspace, tab, escape, forward delete
- Shift+Enter: `\e[13;2u` (CSI u)
- Ctrl+letter: control codes (0x01-0x1A), Ctrl+[/]/\ for ESC/GS/FS
- Option+key: ESC prefix (meta encoding)
- Regular characters: UTF-8 bytes from `event.characters`
- Cmd+ combinations: returns empty (handled by menu/responder chain)

### 6.4 TerminalTab.swift

Rewrite. Owns `TerminalRenderer` + `TerminalBackend`. No SwiftTerm dependency.

Responsibilities:

- Shell spawning with integration script injection (fish/zsh/bash)
- Environment setup (TERM, COLORTERM, dangerous var filtering)
- Backend event dispatch (title, bell, clipboard, exit → NSNotifications)
- Paste handling (newline stripping, CRLF normalization, bracketed paste, image fallback)
- Copy / copy-on-select
- Drag and drop (file URL → shell-escaped path)
- CWD polling via `proc_pidinfo` (1s timer)
- Process termination lifecycle

Estimated ~350 lines (down from 780).

### 6.5 TerminalContainer.swift

Unchanged. Manages `NSSplitView` of `TerminalTab` instances. No SwiftTerm dependency.

## 7. Search Integration

Fills the stub search in `MainWindow.swift`.

**Rust side:** `TerminalBackend` stores an `Option<RegexSearch>`. `search()` compiles the pattern, finds the first match, populates `search_match_ranges` on subsequent `write_grid_to_buffer()` calls. `search_next()`/`search_prev()` advance through matches.

**Swift side:** `TerminalBackend.swift` wraps search FFI calls. `TerminalRenderer` draws match highlights from the binary buffer's search match ranges.

**UI wiring:** `MainWindow.swift` methods `termSearchFieldChanged()`, `termSearchNext()`, `termSearchPrev()` call through to `TerminalBackend.search()`, `.searchNext()`, `.searchPrev()`.

## 8. File Changes

### New Files

| File                                                | Purpose                             |
| --------------------------------------------------- | ----------------------------------- |
| `impulse-terminal/Cargo.toml`                       | Crate manifest                      |
| `impulse-terminal/src/lib.rs`                       | Re-exports                          |
| `impulse-terminal/src/backend.rs`                   | Terminal backend wrapping alacritty |
| `impulse-terminal/src/buffer.rs`                    | Binary buffer packing               |
| `impulse-terminal/src/config.rs`                    | Config types and translation        |
| `impulse-terminal/src/event.rs`                     | Event types                         |
| `impulse-terminal/src/grid.rs`                      | Platform-agnostic grid types        |
| `impulse-terminal/src/search.rs`                    | Regex search wrapper                |
| `impulse-macos/.../Terminal/TerminalBackend.swift`  | Swift FFI wrapper                   |
| `impulse-macos/.../Terminal/TerminalRenderer.swift` | CoreText NSView renderer            |
| `impulse-macos/.../Terminal/KeyEncoder.swift`       | Keyboard input encoding             |

### Modified Files

| File                                                          | Change                                      |
| ------------------------------------------------------------- | ------------------------------------------- |
| `Cargo.toml`                                                  | Add `impulse-terminal` to workspace members |
| `impulse-ffi/Cargo.toml`                                      | Add `impulse-terminal` dependency           |
| `impulse-ffi/src/lib.rs`                                      | Add 18 terminal FFI functions               |
| `impulse-macos/CImpulseFFI/include/impulse_ffi.h`             | Add C declarations                          |
| `impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift`   | Add Swift bridge functions                  |
| `impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift` | Rewrite without SwiftTerm                   |
| `impulse-macos/Sources/ImpulseApp/MainWindow.swift`           | Wire search toolbar                         |
| `impulse-macos/Sources/ImpulseApp/TabManager.swift`           | Remove SwiftTerm import                     |
| `impulse-macos/Package.swift`                                 | Remove SwiftTerm dependency                 |

### Unchanged

- `impulse-macos/.../Terminal/TerminalContainer.swift`
- `impulse-core/` (not involved)
- `impulse-editor/` (not involved)
- `impulse-linux/` (out of scope)

## 9. Deferred Work

| Item                       | Why                                      | Impact                                 |
| -------------------------- | ---------------------------------------- | -------------------------------------- |
| IME / NSTextInputClient    | ~10 method protocol, not blocking        | CJK input and emoji picker won't work  |
| OSC 7 CWD tracking         | Requires byte-stream interception        | 1s polling latency instead of instant  |
| OSC 133 command boundaries | Same interception issue                  | No command markers for future features |
| Metal GPU rendering        | CoreText is fast enough                  | CPU-only rendering                     |
| Linux frontend             | Different toolkit, separate spec         | Linux stays on VTE4                    |
| Sixel / image protocol     | alacritty_terminal doesn't support it    | No inline images (same as SwiftTerm)   |
| Ligatures                  | Cell alignment issues in monospace grids | Same limitation as most terminals      |

## 10. Regressions to Monitor

- Terminal split divider theming
- `--dev` mode flag propagation to shell environment
- Keybinding commands targeting terminal tabs
- Copy-on-select behavior
- Drag and drop file paths
- Terminal title updates in tab bar
