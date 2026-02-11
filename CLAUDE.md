# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Impulse?

Impulse is a terminal-first development environment built with Rust. It combines a terminal emulator with a Monaco-powered code editor in a tabbed interface, with native frontends for Linux (GTK4/libadwaita) and macOS (AppKit/SwiftUI).

## Cross-Platform Development Rule

**IMPORTANT: When making any feature change, bug fix, or UI addition, the change MUST be implemented in BOTH the Linux (`impulse-linux`) and macOS (`impulse-macos`) frontends.** Shared logic belongs in `impulse-core` or `impulse-editor`. Platform-specific UI code goes in the respective frontend crate. If a feature cannot be implemented on one platform yet, add a TODO comment in that frontend's code referencing the feature and the other platform's implementation.

## Build & Development Commands

```bash
cargo build                        # Build all workspace members
cargo build -p impulse-core        # Build only the core library
cargo build -p impulse-editor      # Build only the editor crate
cargo build -p impulse-linux       # Build only the Linux frontend
cargo build -p impulse-macos       # Build only the macOS frontend
cargo run -p impulse-linux         # Run the Linux app
cargo run -p impulse-macos         # Run the macOS app
cargo check                        # Type-check without full compilation
cargo fmt                          # Format all code
cargo clippy                       # Lint
cargo test                         # Run tests
cargo test -p impulse-core         # Test only the core crate
```

## Architecture

The workspace has four crates. Dependency direction is strictly one-way: frontend crates (`impulse-linux`, `impulse-macos`) depend on `impulse-core` and `impulse-editor`, never the reverse. The two frontend crates are independent of each other.

### impulse-core (library, no GUI dependencies)

Platform-agnostic backend logic.

- **pty.rs** — `PtyManager` owns PTY sessions in an `Arc<Mutex<HashMap>>`. Each session spawns a reader thread that runs an `OscParser` to detect shell integration escape sequences (OSC 133 for command start/end, OSC 7 for CWD changes) and forwards `PtyMessage` events through a `PtyEventSender` trait.
- **shell.rs** — Detects the user's shell from `/etc/passwd`/`$SHELL`, injects integration scripts via temp rc files (bash `--rcfile`, zsh `ZDOTDIR` wrapper, fish `--init-command`). The `prepare_shell_spawn()` function is the main entry point.
- **filesystem.rs** — Directory listing sorted dirs-first with git status enrichment via `git status --porcelain`.
- **git.rs** — Git operations: branch detection, diff computation for gutter markers.
- **lsp.rs** — LSP client management: spawning language servers, JSON-RPC communication, managed web LSP installation/status.
- **search.rs** — File name and content search using the `ignore` crate for gitignore-aware walking.
- **shell_integration/*.sh** — Shell scripts emitting OSC 133 and OSC 7 escape sequences.

### impulse-editor (library, Monaco assets)

Bundles the vendored Monaco editor and defines the WebView communication protocol.

- **assets.rs** — Embeds the Monaco vendor directory and editor HTML via `include_dir!` / `include_str!`.
- **protocol.rs** — `EditorCommand` and `EditorEvent` enums for bidirectional JSON messaging between Rust and the Monaco WebView.

### impulse-linux (binary, GTK4 frontend)

- **main.rs** — `adw::Application` setup with app ID `dev.impulse.Impulse`, CLI flags (`--install-lsp-servers`, `--check-lsp-servers`).
- **window.rs** — The main window builder. Contains keybinding setup, tab management (libadwaita `TabView`), sidebar toggling, editor/terminal search, command palette, and signal wiring.
- **keybindings.rs** — Built-in keybinding registry, accel parsing, and override resolution.
- **terminal.rs** — Creates configured VTE terminals with shell integration and drag-and-drop support.
- **terminal_container.rs** — Wraps VTE terminals in a `gtk4::Box` and handles horizontal/vertical splitting via `gtk4::Paned`.
- **editor.rs** — GtkSourceView editor fallback with auto-detected language and indentation.
- **editor_webview.rs** — Monaco editor via WebKitGTK WebView. Handles bidirectional JSON messaging with the embedded Monaco instance.
- **sidebar.rs** — File tree with `Rc<RefCell<Vec<TreeNode>>>` for lazy-loaded directory expansion, plus a search panel.
- **file_icons.rs** — Maps file extensions to bundled SVG icons.
- **project_search.rs** — Project-wide file and content search UI.
- **lsp_completion.rs** / **lsp_hover.rs** — LSP autocomplete and hover info integration.
- **status_bar.rs** — `StatusBar` with labels for CWD, git branch, shell name, cursor position, language, encoding, and indentation.
- **settings.rs** — `Settings` struct serialized to `~/.config/impulse/settings.json`. Includes per-file-type overrides, commands-on-save, keybinding overrides, and custom keybindings.
- **settings_page.rs** — `adw::PreferencesWindow` with pages for Editor, Terminal, Appearance, Automation, and Keybindings.
- **theme.rs** — Color theme constants (Kanagawa, Nord, Gruvbox, Tokyo Night, Tokyo Night Storm, Catppuccin Mocha, Rose Pine) and CSS generation.

### impulse-macos (binary, macOS frontend)

The macOS frontend. Should implement feature parity with `impulse-linux` using native macOS frameworks.

- Uses AppKit/SwiftUI for the UI layer
- Uses WKWebView for the Monaco editor (equivalent to WebKitGTK WebView on Linux)
- Terminal emulation via macOS PTY APIs (equivalent to VTE on Linux)
- Shares `impulse-core` for all backend logic (PTY, filesystem, git, search, LSP, shell integration)
- Shares `impulse-editor` for Monaco assets and the WebView communication protocol
- Settings stored at `~/Library/Application Support/impulse/settings.json` (same schema as Linux)

### Key patterns

- **Shared state in GTK (Linux):** `Rc<RefCell<T>>` for mutable state shared across signal closures (single-threaded GTK main loop).
- **CSS styling (Linux):** All visual styling lives in `theme.rs` as a single formatted CSS string, applied via `add_css_class()`. No external CSS files.
- **Error handling:** Public APIs in `impulse-core` return `Result<T, String>`. Non-fatal errors use `log::warn!`.
- **Shell integration flow:** Shell scripts emit OSC escapes -> terminal emulator passes raw bytes -> `OscParser` in pty.rs strips and interprets them -> `PtyMessage` events sent to frontend via `PtyEventSender`. This flow is identical on both platforms.
- **Settings schema:** Both platforms use the same `Settings` struct and JSON format. The `settings.rs` module in each frontend should share the same data model (or it should be moved to `impulse-core` if divergence becomes a problem).

### What belongs where

| Logic | Crate |
|-------|-------|
| PTY management, shell detection, OSC parsing | `impulse-core` |
| Filesystem listing, git status, search | `impulse-core` |
| LSP client, JSON-RPC, server management | `impulse-core` |
| Monaco assets, editor HTML, WebView protocol | `impulse-editor` |
| Window management, tab UI, native widgets | `impulse-linux` or `impulse-macos` |
| Terminal widget creation and configuration | `impulse-linux` or `impulse-macos` |
| Keybinding registration and UI | `impulse-linux` or `impulse-macos` |
| Theme/styling | `impulse-linux` or `impulse-macos` |

## Scripts

- **scripts/install-lsp-servers.sh** — Installs managed web LSP servers (typescript-language-server, etc.) to `~/.local/share/impulse/lsp/`. Invoked via `--install-lsp-servers` CLI flag.
- **scripts/vendor-monaco.sh** — Downloads and vendors Monaco Editor into `impulse-editor/vendor/monaco/`. Run once or when upgrading Monaco.

## System Dependencies

### Linux (GTK4 stack)

Building `impulse-linux` requires GTK4, libadwaita, VTE4, GtkSourceView5, and WebKitGTK development libraries. On Arch/CachyOS:

```bash
sudo pacman -S gtk4 libadwaita vte4 gtksourceview5 webkit2gtk-4.1
```

### macOS

Building `impulse-macos` requires Xcode command line tools. AppKit, WKWebView, and PTY APIs are provided by the system frameworks.
