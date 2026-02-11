# Plan: Monaco Editor via WebView + macOS Skeleton

## Summary

Replace GtkSourceView5 with Monaco Editor embedded in a WebKitGTK 6.0 WebView.
Create a shared `impulse-editor` crate for the Monaco assets and communication protocol.
Scaffold a minimal `impulse-macos` crate.

## New Workspace Structure

```
impulse/
├── impulse-core/       # Unchanged - platform-agnostic backend
├── impulse-editor/     # NEW - shared Monaco assets + protocol types
├── impulse-linux/      # Modified - webkit6 WebView replaces GtkSourceView
└── impulse-macos/      # NEW - skeleton macOS app (not built on Linux)
```

## System Dependency

Install before building:
```bash
sudo pacman -S webkitgtk-6.0
```

---

## Phase 1: `impulse-editor/` Crate

A pure Rust crate (no platform deps) containing:

### `Cargo.toml`
- Dependencies: `serde`, `serde_json` only

### `src/protocol.rs` - Bidirectional Message Types

**EditorCommand** (Rust → Monaco JS):
- `OpenFile { file_path, content, language }` - load a file
- `SetTheme { colors }` - apply Tokyo Night (or other) theme
- `UpdateSettings { font_size, tab_size, insert_spaces, word_wrap, minimap_enabled, line_numbers }`
- `ApplyDiagnostics { uri, markers }` - set LSP diagnostic squigglies
- `ResolveCompletions { request_id, items }` - fulfill a completion request
- `ResolveHover { request_id, contents }` - fulfill a hover request
- `GoToPosition { line, column }` - navigate cursor
- `SetReadOnly { read_only }`

**EditorEvent** (Monaco JS → Rust):
- `Ready` - Monaco initialized, safe to send commands
- `ContentChanged { content, version }` - user edited text
- `CursorMoved { line, column }` - cursor position changed
- `SaveRequested` - Ctrl+S pressed in editor
- `CompletionRequested { request_id, line, character }` - needs LSP completions
- `HoverRequested { request_id, line, character }` - needs LSP hover
- `DefinitionRequested { line, character }` - go-to-definition
- `FocusChanged { focused }` - editor gained/lost focus

All types derive `Serialize, Deserialize, Debug, Clone`.

### `src/assets.rs` - Embedded Web Content

```rust
pub const EDITOR_HTML: &str = include_str!("../web/editor.html");
```

### `web/editor.html` - Monaco Host Page (~350 lines)

Single HTML file containing:

1. **Monaco loading** from jsDelivr CDN (AMD loader)
2. **Bridge JS** with platform-abstracted `sendToHost()`:
   - WebKitGTK: `window.webkit.messageHandlers.impulse.postMessage(json)`
   - macOS WKWebView: same API
3. **`window.impulseReceiveCommand(json)`** - Rust calls this to send commands
4. **Provider registrations**:
   - `CompletionItemProvider` → posts `CompletionRequested`, returns Promise resolved by `ResolveCompletions`
   - `HoverProvider` → posts `HoverRequested`, returns Promise resolved by `ResolveHover`
   - `DefinitionProvider` → posts `DefinitionRequested`
5. **Event listeners**:
   - `onDidChangeModelContent` → debounced `ContentChanged` (300ms)
   - `onDidChangeCursorPosition` → `CursorMoved`
   - Ctrl+S keybinding → `SaveRequested`
6. **Theme** - `defineTheme("impulse-dark", ...)` mapping from our ThemeColors

---

## Phase 2: `impulse-linux/` Modifications

### New dependency in `Cargo.toml`
```toml
impulse-editor = { path = "../impulse-editor" }
webkit6 = "0.5"
```

### New module: `src/editor_webview.rs`

**`MonacoEditorHandle`** struct (wrapped in `Rc<>` per GTK patterns):
- `webview: webkit6::WebView`
- `file_path: String`
- `cached_content: Rc<RefCell<String>>` - updated on every ContentChanged
- `is_modified: Rc<Cell<bool>>`
- `is_ready: Rc<Cell<bool>>`
- `language: RefCell<String>`
- `version: Rc<Cell<u32>>`

Methods:
- `send_command(&self, cmd)` - serializes to JSON and calls `evaluate_javascript()`
- `set_content()`, `get_content()` (returns cached), `apply_diagnostics()`,
  `resolve_completions()`, `resolve_hover()`, `go_to_position()`, `apply_settings()`, `set_theme()`

**`create_monaco_editor()`** function:
1. Create `gtk4::Box` container with `widget_name` = file_path (matches existing tab ID pattern)
2. Create `webkit6::UserContentManager`, register `"impulse"` message handler
3. Create `webkit6::WebView` with the content manager
4. Configure settings (enable JS, allow file access, disable unnecessary features)
5. Connect `script-message-received` signal → parse `EditorEvent` JSON → dispatch:
   - `Ready` → send `OpenFile` with content + `SetTheme` + `UpdateSettings`
   - `ContentChanged` → update cache, send `LspRequest::DidChange`
   - `CursorMoved` → update status bar
   - `SaveRequested` → write file, send `LspRequest::DidSave`
   - `CompletionRequested` → forward as `LspRequest::Completion`
   - `HoverRequested` → forward as `LspRequest::Hover`
   - `DefinitionRequested` → forward as `LspRequest::Definition`
6. Load HTML via `webview.load_html(EDITOR_HTML, Some("file:///"))`
7. Return `(container, handle)`

### Modify `src/editor.rs`

- `create_editor()` → delegates to `create_monaco_editor()`; return type changes from `(Box, sourceview5::Buffer)` to `(Box, Rc<MonacoEditorHandle>)`
- `get_editor_text()` → walks tree for `webkit6::WebView`, uses handle's cached content
- `get_editor_buffer()` → replaced by `get_editor_handle()` returning `Option<Rc<MonacoEditorHandle>>`
- `get_editor_view()` → replaced by `get_editor_webview()` returning `Option<webkit6::WebView>`
- `is_editor()` → unchanged (checks widget_name, already platform-agnostic)
- `get_editor_language()` → uses handle's cached language
- `get_editor_indent_info()` → returns default from handle
- `apply_settings()` → delegates to `handle.apply_settings()`
- Remove: bracket auto-close (Monaco built-in), GtkSourceView scheme install, git diff marks (future Monaco decoration)
- Keep: `detect_indentation()`, `is_image_file()`, `create_image_preview()`, `is_binary_file()`

### Modify `src/window.rs`

**Add** state:
- `monaco_handles: Rc<RefCell<HashMap<String, Rc<MonacoEditorHandle>>>>` - per-file handle map

**File activation** (currently creates GtkSourceView):
- Replace `editor::create_editor()` call with `create_monaco_editor()`
- Store handle in `monaco_handles`
- Remove: `buffer.connect_modified_changed()`, `buffer.connect_notify_local("cursor-position")`,
  LSP DidOpen/DidChange/auto-save signal wiring, multi-cursor key interception
  (all now handled inside `editor_webview.rs` via EditorEvent dispatch)

**LSP response handling**:
- Diagnostics: `handle.apply_diagnostics(diags)` instead of `apply_diagnostics(buf, view, diags)`
- Completions: `handle.resolve_completions(id, items)` instead of `show_completion_popup(view, buf, items)`
- Hover: `handle.resolve_hover(id, text)` instead of `show_hover_popover(view, buf, ...)`
- Definition: `handle.go_to_position(line, col)` instead of `buf.place_cursor() + view.scroll_to_iter()`

**Keybindings** - remove (Monaco handles natively):
- Ctrl+G (go-to-line), Ctrl+Space (completion), Ctrl+Shift+I (hover)
- Ctrl+F / Ctrl+H (editor find/replace)
- Multi-cursor shortcuts

**Keybindings** - keep:
- Ctrl+T/W/Tab (tab management), Ctrl+S (filesystem save), F12 (definition),
  Ctrl+Shift+B (sidebar), Ctrl+Shift+P (palette), Ctrl+=/- (font size → also UpdateSettings to Monaco)

**Editor search bar** (~270 lines):
- Remove entirely (Monaco has built-in Ctrl+F/Ctrl+H)
- Keep terminal search bar

**Tab switching**:
- Use `monaco_handles` to get cached language, cursor position, indent info for status bar

**Tab close**:
- Use `handle.is_modified()` and `handle.get_content()` instead of sourceview5 API

### Modules to remove/simplify

- **`multi_cursor.rs`** → delete (Monaco built-in multi-cursor)
- **`lsp_hover.rs`** → keep `hover_content_to_string()` and `marked_string_to_text()` (used in LSP bridge), remove `show_hover_popover()`
- **`lsp_completion.rs`** → keep `LspRequest`/`LspResponse` enums, `DiagnosticInfo`, `CompletionInfo`, `LspCompletionBridge`, `run_guarded_ui()`. Remove `apply_diagnostics()`, `show_completion_popup()`, `clear_diagnostics()`, `ensure_diagnostic_tags()`, GTK popover code

---

## Phase 3: `impulse-macos/` Skeleton

Minimal, non-functional scaffold showing the architecture.

### `Cargo.toml`
```toml
[package]
name = "impulse-macos"
version = "0.1.0"
edition = "2021"

[dependencies]
impulse-core = { path = "../impulse-core" }
impulse-editor = { path = "../impulse-editor" }
# macOS-specific deps (only compiles on macOS)
```

### Files
- `src/main.rs` - Entry point with placeholder app setup
- `src/app.rs` - Application lifecycle stubs
- `src/window.rs` - Window layout comments (sidebar, editor, terminal areas)
- `src/editor.rs` - Comments documenting future WKWebView + Monaco integration

### Workspace `Cargo.toml`
```toml
[workspace]
members = ["impulse-core", "impulse-editor", "impulse-linux"]
# On macOS, also add "impulse-macos"
```

`impulse-macos` is excluded on Linux since it requires macOS frameworks.

---

## Phase 4: Theme Integration

Create `ThemeColors → Monaco theme` conversion in `impulse-editor/src/protocol.rs`:
- Map `bg` → `editor.background`
- Map `fg` → `editor.foreground`
- Map `bg_highlight` → `editor.lineHighlightBackground`
- Map `selection` → `editor.selectionBackground`
- Map syntax colors (cyan, blue, green, etc.) → Monaco token colors for keywords, strings, types, etc.

---

## Implementation Order

| Step | What | Risk |
|------|------|------|
| 1 | Install `webkitgtk-6.0` system package | None |
| 2 | Create `impulse-editor/` crate (protocol.rs + assets.rs) | Low |
| 3 | Write `web/editor.html` (Monaco + bridge JS) | Medium |
| 4 | Create `editor_webview.rs` in impulse-linux | Medium |
| 5 | Modify `editor.rs` public API | Low |
| 6 | Modify `window.rs` (tab creation, LSP wiring, keybindings) | High |
| 7 | Remove dead code (multi_cursor.rs, old popover code) | Low |
| 8 | Create `impulse-macos/` skeleton | Low |
| 9 | Update workspace Cargo.toml | None |

---

## Key Risk: Focus Management

WebView widgets capture keyboard focus. App-level shortcuts (Ctrl+T, Ctrl+W) registered at `ShortcutScope::Global` should still work, but we need to verify. Monaco's `addCommand()` explicitly avoids handling app-level keys.

## Key Risk: Async Content

`get_editor_text()` was synchronous with GtkSourceView. With Monaco, content comes via `ContentChanged` events. We cache it on every change, so `get_content()` returns the cached copy (may lag by up to 300ms debounce window). For save operations, this is acceptable.

## Future Work (not in this plan)

- Bundle Monaco locally instead of CDN (offline support)
- Git diff decorations via Monaco decoration API
- Tree-sitter integration for enhanced highlighting
- macOS terminal emulator (SwiftTerm or custom PTY)
- macOS sidebar and tab management
