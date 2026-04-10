# Terminal Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clean up the alacritty-terminal migration branch — add live theme recoloring, expose `scroll_to_bottom` properly through FFI, fix all clippy warnings, and fix the CWD timer comment — preparing the branch for merge into main.

**Architecture:** Five independent tasks touching four layers: Rust backend (`impulse-terminal`), FFI bridge (`impulse-ffi`), Swift bridge (`ImpulseCore.swift` + `TerminalBackend.swift`), and Swift frontend (`TerminalTab.swift`, `TerminalRenderer.swift`). Each task is self-contained and can be committed independently.

**Tech Stack:** Rust (alacritty_terminal, serde, bitflags), C FFI, Swift/AppKit

---

### Task 1: Live Theme Recoloring

Add a `set_colors()` method to the Rust backend, expose it through FFI, wire it through Swift, and call it from `TerminalTab.applyTheme()` so running terminals update colors immediately on theme change.

**Files:**

- Modify: `impulse-terminal/src/backend.rs:191-201` (add `set_colors` method)
- Modify: `impulse-ffi/src/lib.rs:1352-1396` (add `impulse_terminal_set_colors` FFI function)
- Modify: `impulse-macos/CImpulseFFI/include/impulse_ffi.h:98` (add C declaration)
- Modify: `impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift` (add Swift bridge)
- Modify: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift:339-345` (add `setColors` method)
- Modify: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift:386-393` (call `setColors` in `applyTheme`)

- [ ] **Step 1: Add `set_colors` to `TerminalBackend` in Rust**

In `impulse-terminal/src/backend.rs`, add this method to the `impl TerminalBackend` block, after the existing `scroll_to_bottom()` method (around line 555):

```rust
    /// Update the terminal's color palette at runtime (for live theme changes).
    pub fn set_colors(&mut self, config: &TerminalConfig) {
        self.colors = ConfiguredColors::from_config(config);
    }
```

- [ ] **Step 2: Add `impulse_terminal_set_colors` FFI function**

In `impulse-ffi/src/lib.rs`, add this function after the `impulse_terminal_set_focus` function (after line 1538):

```rust
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
```

- [ ] **Step 3: Add C header declaration**

In `impulse-macos/CImpulseFFI/include/impulse_ffi.h`, add before the closing `#endif`:

```c
void impulse_terminal_set_colors(void *handle, const char *config_json);
```

- [ ] **Step 4: Add Swift bridge function**

In `impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift`, add inside the `ImpulseCore` enum after the `terminalSearchClear` function:

```swift
    /// Updates the terminal's color palette at runtime.
    static func terminalSetColors(handle: OpaquePointer, configJson: String) {
        configJson.withCString { ptr in
            impulse_terminal_set_colors(UnsafeMutableRawPointer(handle), ptr)
        }
    }
```

- [ ] **Step 5: Add `setColors` to Swift `TerminalBackend`**

In `impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift`, add after the `searchClear()` method (around line 403):

```swift
    // MARK: - Colors

    func setColors(config: TerminalBackendConfig) {
        guard let handle, !isShutdown else { return }
        guard let data = try? JSONEncoder().encode(config),
              let json = String(data: data, encoding: .utf8) else { return }
        ImpulseCore.terminalSetColors(handle: handle, configJson: json)
    }
```

- [ ] **Step 6: Wire `applyTheme` to call `setColors`**

In `impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift`, replace the `applyTheme` method body (lines 386-393):

```swift
    func applyTheme(theme: TerminalTheme) {
        currentTheme = theme
        // Build a config with the new colors and push it to the backend.
        let settings = currentSettings ?? TerminalSettings()
        var config = TerminalBackendConfig.from(
            settings: settings,
            theme: theme,
            shellPath: "",
            shellArgs: [],
            environment: [:],
            workingDirectory: nil
        )
        backend?.setColors(config: config)
        renderer.needsDisplay = true
    }
```

- [ ] **Step 7: Build and verify**

Run: `cargo build -p impulse-terminal -p impulse-ffi && ./impulse-macos/build.sh`

Expected: Clean build, no errors.

- [ ] **Step 8: Commit**

```bash
git add impulse-terminal/src/backend.rs impulse-ffi/src/lib.rs impulse-macos/CImpulseFFI/include/impulse_ffi.h impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift
git commit -m "Add live theme recoloring for running terminals"
```

---

### Task 2: Expose `scroll_to_bottom` Through FFI

Replace the fragile `scroll(delta: -999_999)` workaround in `TerminalRenderer.swift` with a proper `scrollToBottom()` call that uses alacritty's `Scroll::Bottom`.

**Files:**

- Modify: `impulse-ffi/src/lib.rs:1510-1516` (add `impulse_terminal_scroll_to_bottom`)
- Modify: `impulse-macos/CImpulseFFI/include/impulse_ffi.h` (add C declaration)
- Modify: `impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift` (add Swift bridge)
- Modify: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift:340-343` (add `scrollToBottom`)
- Modify: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalRenderer.swift:786-789` (use `scrollToBottom`)

- [ ] **Step 1: Add `impulse_terminal_scroll_to_bottom` FFI function**

In `impulse-ffi/src/lib.rs`, add after the `impulse_terminal_scroll` function (after line 1516):

```rust
#[no_mangle]
pub extern "C" fn impulse_terminal_scroll_to_bottom(handle: *mut TerminalHandle) {
    ffi_catch((), AssertUnwindSafe(|| {
        if handle.is_null() { return; }
        let h = unsafe { &*handle };
        h.backend.scroll_to_bottom();
    }))
}
```

- [ ] **Step 2: Add C header declaration**

In `impulse-macos/CImpulseFFI/include/impulse_ffi.h`, add after the `impulse_terminal_scroll` line:

```c
void impulse_terminal_scroll_to_bottom(void *handle);
```

- [ ] **Step 3: Add Swift bridge function**

In `impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift`, add after the `terminalScroll` function:

```swift
    /// Scrolls the terminal viewport to the bottom (most recent output).
    static func terminalScrollToBottom(handle: OpaquePointer) {
        impulse_terminal_scroll_to_bottom(UnsafeMutableRawPointer(handle))
    }
```

- [ ] **Step 4: Add `scrollToBottom` to Swift `TerminalBackend`**

In `impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift`, add after the `scroll(delta:)` method:

```swift
    func scrollToBottom() {
        guard let handle, !isShutdown else { return }
        ImpulseCore.terminalScrollToBottom(handle: handle)
    }
```

- [ ] **Step 5: Replace workaround in `TerminalRenderer`**

In `impulse-macos/Sources/ImpulseApp/Terminal/TerminalRenderer.swift`, replace line 788:

```swift
// Before:
                backend.scroll(delta: -999_999)
// After:
                backend.scrollToBottom()
```

- [ ] **Step 6: Build and verify**

Run: `cargo build -p impulse-ffi && ./impulse-macos/build.sh`

Expected: Clean build, no errors.

- [ ] **Step 7: Commit**

```bash
git add impulse-ffi/src/lib.rs impulse-macos/CImpulseFFI/include/impulse_ffi.h impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift impulse-macos/Sources/ImpulseApp/Terminal/TerminalRenderer.swift
git commit -m "Expose scroll_to_bottom through FFI, replace scroll(-999999) workaround"
```

---

### Task 3: Fix Clippy Warnings — `TerminalHandle` Visibility

All 19 clippy warnings in `impulse-ffi` are the same: `TerminalHandle` is private but used in `pub extern "C"` function signatures. Fix by adding `#[allow(private_interfaces)]` to the module section, since the struct is intentionally opaque — it's only ever passed as a raw pointer across FFI and never constructed or accessed by external Rust code.

**Files:**

- Modify: `impulse-ffi/src/lib.rs:1352-1358`

- [ ] **Step 1: Add `#[allow(private_interfaces)]` attribute**

In `impulse-ffi/src/lib.rs`, add the allow attribute above the `TerminalHandle` struct (before line 1358). The comment section starting at line 1352 already groups the terminal API — add the attribute right before the struct:

```rust
// ---------------------------------------------------------------------------
// Terminal backend API
// ---------------------------------------------------------------------------

use impulse_terminal::{SelectionKind, TerminalBackend};

/// Opaque handle passed across FFI — never constructed by external code.
#[allow(private_interfaces)]
struct TerminalHandle {
    backend: TerminalBackend,
    /// Pre-allocated buffer for grid snapshots.
    snapshot_buf: Vec<u8>,
}
```

Note: `#[allow(private_interfaces)]` goes on the struct, not on the functions. But clippy 1.xx emits the warning on the functions. The correct fix is to silence at module/crate level if the struct-level attribute doesn't suppress all 19 warnings. If the struct-level allow doesn't work, add `#![allow(private_interfaces)]` at the top of `lib.rs` instead.

- [ ] **Step 2: Fix `OscScanner` missing `Default` impl**

In `impulse-terminal/src/osc_scanner.rs`, add a `Default` impl after the `impl OscScanner` block (after the closing `}` of the impl, around line 130):

```rust
impl Default for OscScanner {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 3: Run clippy and verify warnings are gone**

Run: `cargo clippy -p impulse-terminal -p impulse-ffi 2>&1 | grep warning`

Expected: No warnings from `impulse-terminal` or `impulse-ffi` (there may still be warnings from `impulse-core` and `impulse-editor` — those are pre-existing and out of scope).

- [ ] **Step 4: Commit**

```bash
git add impulse-ffi/src/lib.rs impulse-terminal/src/osc_scanner.rs
git commit -m "Fix clippy warnings: TerminalHandle visibility, OscScanner Default"
```

---

### Task 4: Fix CWD Poll Timer Comment

The comment on line 526 of `TerminalTab.swift` says "1 second interval" but the actual timer on line 558 uses `5.0` seconds.

**Files:**

- Modify: `impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift:526`

- [ ] **Step 1: Fix the comment**

In `impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift`, change line 526:

```swift
// Before:
            // Start CWD polling timer (1 second interval).
// After:
            // Start CWD polling timer (5 second interval).
```

- [ ] **Step 2: Build and verify**

Run: `./impulse-macos/build.sh`

Expected: Clean build.

- [ ] **Step 3: Commit**

```bash
git add impulse-macos/Sources/ImpulseApp/Terminal/TerminalTab.swift
git commit -m "Fix incorrect CWD poll timer comment (5s, not 1s)"
```

---

### Task 5: Merge into Main

Merge the `alacritty-terminal-v2` branch into `main`.

- [ ] **Step 1: Run full test suite**

Run: `cargo test -p impulse-core -p impulse-editor -p impulse-ffi -p impulse-terminal`

Expected: All tests pass.

- [ ] **Step 2: Run clippy on all cross-platform crates**

Run: `cargo clippy -p impulse-terminal -p impulse-ffi 2>&1 | grep "warning:"`

Expected: No warnings from these crates.

- [ ] **Step 3: Build macOS app**

Run: `./impulse-macos/build.sh`

Expected: Clean build, `dist/Impulse.app` created.

- [ ] **Step 4: Merge into main**

```bash
git checkout main
git merge alacritty-terminal-v2
```

Expected: Clean merge (no conflicts since this branch was created from main).
