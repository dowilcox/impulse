# macOS Terminal Dirty-Row Rendering + FileTreeView Retirement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** (1) Render only damaged terminal rows instead of the full grid every frame, using alacritty_terminal's damage API; (2) retire the hidden AppKit `FileTreeView` by moving its headless logic (watchers, git polling, expansion persistence, patch application) into a new `FileTreeDataController`.

**Architecture:** Workstream A threads alacritty's per-line damage through a new FFI call (`impulse_terminal_take_damage`) so the Swift renderer can invalidate only damaged row rects and clip its draw loops to `dirtyRect`. Workstream B extracts the ~800 lines of headless data/watcher logic out of the never-displayed `FileTreeView` into a plain `FileTreeDataController` object, replaces the NSOutlineView-driven expansion restore with a recursive node walk, and deletes the dead AppKit view (~900 lines of outline UI).

**Tech Stack:** Rust (alacritty_terminal 0.26, impulse-terminal, impulse-ffi), Swift (AppKit, CoreGraphics), C FFI.

**Out of scope (verified unnecessary during scouting):**

- _WindowModel split_ — `@Observable` already gives per-property dependency tracking; views reading `tabDisplayInfos` do not re-render on `cursorLine` changes. The original review finding assumed `ObservableObject` semantics.
- _Keybindings via responder chain_ — all 25 builtin actions already have menu items with key equivalents (MenuBuilder.swift); the NSEvent monitor only handles custom run-command bindings, mirroring the Linux capture-phase design. Only follow-up: verify the menu rebuilds when keybinding overrides change (Task B6).

---

## Workstream A: Terminal dirty-row rendering

### Task A1: Verify the alacritty damage API surface

**Files:** read-only investigation.

- [x] **Step 1:** Locate the vendored alacritty_terminal 0.26 source: `grep -rn "pub fn damage" ~/.cargo/registry/src/*/alacritty_terminal-0.26.0/src/term/mod.rs` and confirm:
  - `pub fn damage(&mut self) -> TermDamage<'_>` exists
  - `pub fn reset_damage(&mut self)` exists
  - `TermDamage::{Full, Partial(TermDamageIterator)}` with `LineDamageBounds { line, left, right }` (line is viewport-relative)
  - scrolling the display marks the term fully damaged (search `mark_fully_damaged`)
- [x] **Step 2:** In `/Users/dowilcox/Code/impulse/impulse-terminal/src/backend.rs`, list every method that mutates selection, search state, or scroll offset (these must force full damage since alacritty does not track them).

### Task A2: Rust — `take_damage` on TerminalBackend

**Files:**

- Modify: `/Users/dowilcox/Code/impulse/impulse-terminal/src/backend.rs`

- [x] **Step 1:** Add a `force_full_damage: AtomicBool` field (or `Mutex<bool>` alongside existing fields) to `TerminalBackend`, initialized true (first frame is full).
- [x] **Step 2:** Set it in every selection/search/scroll/resize/config mutation method identified in A1 Step 2.
- [x] **Step 3:** Add the method:

```rust
/// Damage result: None = full repaint required, Some(rows) = viewport rows
/// that changed since the last call. Resets alacritty's damage tracking.
pub fn take_damage(&self) -> Option<Vec<u16>> {
    let force_full = self.force_full_damage.swap(false, Ordering::Relaxed);
    let mut term = self.term.lock();
    let damage = term.damage();
    let result = match damage {
        TermDamage::Full => None,
        TermDamage::Partial(iter) => {
            let rows: Vec<u16> = iter.map(|bounds| bounds.line as u16).collect();
            Some(rows)
        }
    };
    term.reset_damage();
    if force_full { None } else { result }
}
```

(Adjust to the actual borrow rules: `damage()` borrows `term` mutably, so collect rows before calling `reset_damage()`.)

- [x] **Step 4:** Unit test in backend.rs (or a pure helper) asserting the force-full flag wins over partial damage and is cleared after one take.
- [x] **Step 5:** `cargo test -p impulse-terminal` — expect pass.

### Task A3: FFI — `impulse_terminal_take_damage`

**Files:**

- Modify: `/Users/dowilcox/Code/impulse/impulse-ffi/src/lib.rs` (next to the other terminal fns, ~line 1880)
- Modify: `/Users/dowilcox/Code/impulse/impulse-macos/CImpulseFFI/include/impulse_ffi.h`

- [x] **Step 1:** FFI function:

```rust
/// Returns -1 if a full repaint is required, otherwise the number of damaged
/// viewport rows written to `out_rows` (clamped to `cap`). Resets damage.
#[no_mangle]
pub extern "C" fn impulse_terminal_take_damage(
    handle: *mut TerminalHandle,
    out_rows: *mut u16,
    cap: usize,
) -> i64 {
    // null checks + ffi_catch like neighbours; on None return -1;
    // on Some(rows): if rows.len() > cap return -1 (degrade to full),
    // else copy rows into out_rows and return rows.len() as i64.
}
```

- [x] **Step 2:** Header decl: `int64_t impulse_terminal_take_damage(TerminalHandle *handle, uint16_t *out_rows, size_t cap);`
- [x] **Step 3:** `cargo build -p impulse-ffi` — expect pass.

### Task A4: Swift — TerminalBackend.takeDamage()

**Files:**

- Modify: `/Users/dowilcox/Code/impulse/impulse-macos/Sources/ImpulseApp/Terminal/TerminalBackend.swift` (next to gridSnapshot, ~line 354)
- Modify: `/Users/dowilcox/Code/impulse/impulse-macos/Sources/ImpulseApp/Bridge/ImpulseCore.swift` (terminal wrapper section)

- [x] **Step 1:** ImpulseCore wrapper calling the C function with an `UnsafeMutablePointer<UInt16>` buffer.
- [x] **Step 2:** Backend API:

```swift
enum TerminalDamage {
    case full
    case rows([Int])
}

func takeDamage() -> TerminalDamage {
    guard let handle, !isShutdown else { return .full }
    // reuse a persistent UInt16 buffer sized to `rows` (realloc on resize)
    let count = ImpulseCore.terminalTakeDamage(handle: handle, buffer: damageBuffer, cap: damageBufferCap)
    if count < 0 { return .full }
    return .rows((0..<Int(count)).map { Int(damageBuffer[$0]) })
}
```

- [x] **Step 3:** Build the package: `cd impulse-macos && swift build` (or full build.sh) — expect pass.

### Task A5: Renderer — invalidate damaged rows, clip draw to dirtyRect

**Files:**

- Modify: `/Users/dowilcox/Code/impulse/impulse-macos/Sources/ImpulseApp/Terminal/TerminalRenderer.swift`

- [x] **Step 1:** Add row-rect helpers (pure, testable):

```swift
func rectForRow(_ row: Int) -> NSRect   // padding + row*cellHeight, full width
func rowRange(intersecting rect: NSRect, lines: Int) -> Range<Int>
```

- [x] **Step 2:** In `tick()` (line ~281): on wakeup, replace `needsDisplay = true` with:

```swift
switch backend.takeDamage() {
case .full: needsDisplay = true
case .rows(let rows):
    guard !rows.isEmpty else { break }
    for row in rows { setNeedsDisplay(rectForRow(row)) }
}
```

(keep the existing scrollOnOutput/isScrolledBack logic; scrolled-back state forces `.full` from the backend anyway).

- [x] **Step 3:** In `draw(_:)` (line ~306): compute `let drawRows = rowRange(intersecting: dirtyRect, lines: lines)` and:
  - background fill: `context.fill(dirtyRect)` instead of `bounds`
  - `drawCellBackgrounds` row loop: iterate `drawRows`
  - selection/search highlight loops: skip ranges whose row ∉ drawRows
  - text + decoration row loop: iterate `drawRows`
  - cursor: draw only if `grid.cursorRow ∈ drawRows` (alacritty damages old+new cursor lines, so blink/move invalidation is covered)
  - cursor blink timer: invalidate `rectForRow(lastCursorRow)` instead of full view
  - hover-link underline + IME overlay: leave conditional on intersection with dirtyRect (or force-full when active — choose simplest correct)
- [x] **Step 4:** Add a Swift unit test for `rowRange(intersecting:)` math in `impulse-macos/Tests/ImpulseAppTests/`.
- [x] **Step 5:** Full build via `./impulse-macos/build.sh`; run `cargo test -p impulse-terminal`; expect pass.

---

## Workstream B: Retire the hidden FileTreeView

### Task B1: NameInputDialog helper

**Files:**

- Create: `/Users/dowilcox/Code/impulse/impulse-macos/Sources/ImpulseApp/UI/NameInputDialog.swift`

- [x] **Step 1:** Port `showNameInputAlert` (FileTreeView.swift lines 737–767) verbatim into:

```swift
enum NameInputDialog {
  static func show(
    title: String, message: String, placeholder: String,
    defaultValue: String, completion: @escaping (String) -> Void
  ) { /* ported NSAlert + NSTextField + stem pre-selection */ }
}
```

### Task B2: FileTreeDataController (headless port)

**Files:**

- Create: `/Users/dowilcox/Code/impulse/impulse-macos/Sources/ImpulseApp/Sidebar/FileTreeDataController.swift`

Port these regions of FileTreeView.swift, dropping every `outlineView`/`scrollView` reference:

| Ported member                                                                                                           | Source lines       | Changes                                                                                       |
| ----------------------------------------------------------------------------------------------------------------------- | ------------------ | --------------------------------------------------------------------------------------------- |
| properties (rootNodes, rootPath, showHidden, onTreeRefreshed, watcher fields, git fields, node index, coalescing flags) | 100–180            | drop UI-only fields                                                                           |
| `updateTree(nodes:rootPath:skipGitRefresh:)`                                                                            | ~330–365           | replace `restoreExpandedPaths()` (outline) with `applyExpansionState` (Step 2); no reloadData |
| `refreshGitStatus()`                                                                                                    | 383–409            | replace `reloadVisibleRows()` with no-op (SwiftUI rows observe nodes directly)                |
| `collapseAll()`                                                                                                         | ~412–425           | walk nodes setting `isExpanded = false`; stop subdir watchers; persist                        |
| `persistCurrentExpandedPaths()`                                                                                         | 437–441            | unchanged                                                                                     |
| expansion persistence                                                                                                   | 769–822            | `collectExpandedPaths` walks `node.isExpanded` (not outline)                                  |
| FS watchers (root/subdir)                                                                                               | 824–943            | unchanged                                                                                     |
| .git/index watcher                                                                                                      | 945–1027           | unchanged                                                                                     |
| git status timer + app-active observers                                                                                 | 258–273, 1029–1094 | unchanged                                                                                     |
| `handleFileSystemEvent` → `refreshTreePatches` → patch application                                                      | 1096–1269          | drop `outlineView.reloadItem` calls; keep `onTreeRefreshed?(rootNodes)`                       |
| node index                                                                                                              | 1279–1307          | unchanged                                                                                     |
| `refreshTree()`                                                                                                         | 439–519            | expansion restore via `applyExpansionState`                                                   |

- [x] **Step 2:** New headless expansion restore (replaces outlineView.expandItem → delegate → loadChildren):

```swift
/// Re-apply a saved set of expanded paths to a freshly built tree,
/// loading children for each expanded directory (the old code relied on
/// NSOutlineView's expandItem delegate to do this).
private func applyExpansionState(_ paths: Set<String>, to nodes: [FileTreeNode]) {
  for node in nodes where node.isDirectory && paths.contains(node.path) {
    if !node.isLoaded {
      node.loadChildren()   // verify actual signature on FileTreeNode
    }
    node.isExpanded = true
    if let children = node.children {
      applyExpansionState(paths, to: children)
    }
  }
}
```

- [x] **Step 3:** Build (`swift build`) — expect pass with the controller unused.

### Task B3: Switch MainWindow to the controller

**Files:**

- Modify: `/Users/dowilcox/Code/impulse/impulse-macos/Sources/ImpulseApp/MainWindow.swift`

- [x] **Step 1:** Replace property (line 66) + init (line 220) with `private let fileTreeData = FileTreeDataController()`.
- [x] **Step 2:** Mechanical substitutions at every callsite found in scouting (lines 270, 280–281, 361–362, 370–372, 376, 394–395, 409, 426, 700, 722, 1514, 2063–2064, 2392, 2441, 2458–2471, 2786): `fileTreeView.X` → `fileTreeData.X`; `fileTreeView.showNameInputAlert(...)` → `NameInputDialog.show(...)`.
- [x] **Step 3:** Build — expect pass.

### Task B4: Delete FileTreeView.swift

- [x] **Step 1:** `grep -rn "FileTreeView\|HoverRowView\|PointerOutlineView" impulse-macos/Sources/` — expect zero remaining references (FileTreeListView is a different symbol; verify no prefix-collision in the grep).
- [x] **Step 2:** Delete `/Users/dowilcox/Code/impulse/impulse-macos/Sources/ImpulseApp/Sidebar/FileTreeView.swift`.
- [x] **Step 3:** Full `./impulse-macos/build.sh` — expect pass.

### Task B5: Update CLAUDE.md

- [x] Remove/replace the stale claims: "FileTreeView.swift — NSOutlineView-based file tree. Kept alive for…", "The old AppKit FileTreeView is kept alive (hidden)…", and the SearchPanel.swift bullet (file doesn't exist). Describe `FileTreeDataController` instead.

### Task B6: Menu rebuild on keybinding override change (small)

- [x] **Step 1:** `grep -n "rebuildMainMenu" impulse-macos/Sources/ImpulseApp/AppDelegate.swift` — check whether the settings-change observer triggers a rebuild.
- [x] **Step 2:** If not, add `rebuildMainMenu()` to the `.impulseSettingsDidChange` observer in AppDelegate so overridden shortcuts show on menu items immediately.

---

## Verification (whole plan)

- [x] `cargo test -p impulse-core -p impulse-editor -p impulse-ffi` and `cargo test -p impulse-terminal`
- [x] `swift test` in impulse-macos — tests compile; the runner cannot execute on this machine (Command Line Tools only, no `xctest`), so they are build-verified only
- [x] `./impulse-macos/build.sh` final build
- [ ] Manual smoke items for the user: terminal output under `yes | head -c 1M`, scrollback, selection highlight, search highlight, cursor blink; sidebar expansion persists across root switches; new file/folder dialogs; git badges update after commit from terminal.
