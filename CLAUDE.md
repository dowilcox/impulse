# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Impulse?

Impulse is a terminal-first development environment built with Rust. It combines a terminal emulator with a Monaco-powered code editor in a tabbed interface, with native frontends for Linux (GTK4/libadwaita) and macOS (AppKit/SwiftUI).

## Cross-Platform Development Rule

**IMPORTANT: When making any feature change, bug fix, or UI addition, the change MUST be implemented in BOTH the Linux (`impulse-linux`) and macOS (`impulse-macos`) frontends.** Shared logic belongs in `impulse-core` or `impulse-editor`. Platform-specific UI code goes in the respective frontend crate. If a feature cannot be implemented on one platform yet, add a TODO comment in that frontend's code referencing the feature and the other platform's implementation.

## Build & Development Commands

```bash
# Rust workspace (impulse-core, impulse-editor, impulse-linux, impulse-ffi)
cargo build                        # Build all workspace members
cargo build -p impulse-core        # Build only the core library
cargo build -p impulse-editor      # Build only the editor crate
cargo build -p impulse-linux       # Build only the Linux frontend
cargo build -p impulse-ffi         # Build only the FFI static library (for macOS)
cargo run -p impulse-linux         # Run the Linux app
cargo check                        # Type-check without full compilation
cargo fmt                          # Format all code
cargo clippy                       # Lint
cargo test                         # Run tests
cargo test -p impulse-core         # Test only the core crate

# macOS (Swift Package, built separately)
./impulse-macos/build.sh           # Build .app bundle (builds impulse-ffi + Swift app)
./impulse-macos/build.sh --dmg     # Build .app + .dmg disk image
./impulse-macos/build.sh --sign    # Build + codesign with Developer ID
./impulse-macos/build.sh --sign --notarize --dmg  # Full release build
```

## Architecture

The Cargo workspace has four Rust crates (`impulse-core`, `impulse-editor`, `impulse-linux`, `impulse-ffi`) plus one Swift package (`impulse-macos`). Dependency direction is strictly one-way: frontend code depends on `impulse-core` and `impulse-editor`, never the reverse. The macOS frontend links against `impulse-ffi` (a C-compatible static library wrapping `impulse-core` and `impulse-editor`). The two frontends are independent of each other.

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

### impulse-ffi (static library, C-compatible FFI)

C-compatible wrappers around `impulse-core` and `impulse-editor` for the macOS Swift frontend. Compiled as a static library (`libimpulse_ffi.a`). All functions use C strings for input/output and JSON encoding for complex types. Callers must free returned strings with `impulse_free_string`.

- **lib.rs** — `extern "C"` functions exposing filesystem, git, search, LSP, PTY, shell detection, and editor asset operations to Swift via the `CImpulseFFI` module.

### impulse-macos (Swift Package, macOS frontend)

The macOS frontend, built as a Swift Package (not a Cargo crate). Communicates with the Rust backend via `impulse-ffi` C FFI. Built with `./impulse-macos/build.sh`.

- **ImpulseApp.swift** — App entry point.
- **AppDelegate.swift** — NSApplication delegate, app lifecycle.
- **MainWindow.swift** — Main window setup, layout, and signal wiring.
- **TabManager.swift** — Tab management (custom tab bar mixing terminal and editor tabs).
- **Terminal/TerminalContainer.swift** — Terminal view with splitting support.
- **Terminal/TerminalTab.swift** — Terminal tab using SwiftTerm for terminal emulation.
- **Editor/EditorTab.swift** — Monaco editor tab via WKWebView.
- **Editor/EditorProtocol.swift** — Bidirectional JSON messaging with Monaco (mirrors `impulse-editor` protocol).
- **Editor/EditorWebViewPool.swift** — WebView pooling for editor instances.
- **Sidebar/FileTreeView.swift** — File tree with lazy-loaded directory expansion.
- **Sidebar/FileTreeNode.swift** — Tree node model for the file tree.
- **Sidebar/FileIcons.swift** — Maps file extensions to bundled SVG icons.
- **Sidebar/SearchPanel.swift** — Project-wide file and content search UI.
- **UI/CommandPalette.swift** — Command palette (equivalent to Linux Ctrl+Shift+P).
- **UI/CustomTabBar.swift** — Native tab bar widget.
- **UI/MenuBuilder.swift** — macOS menu bar construction.
- **UI/StatusBar.swift** — Status bar with CWD, git branch, cursor position, etc.
- **Settings/Settings.swift** — `Settings` struct (Codable), stored at `~/Library/Application Support/impulse/settings.json`.
- **Settings/SettingsFormSheet.swift** — Settings editor form.
- **Settings/SettingsWindow.swift** — Settings window controller.
- **Theme/Theme.swift** — Color theme constants matching the Linux themes.
- **Keybindings/Keybindings.swift** — Keybinding registry and handling.
- **Bridge/ImpulseCore.swift** — Swift wrapper calling `impulse-ffi` C functions.
- **CImpulseFFI/** — C header module (`impulse_ffi.h` + `module.modulemap`) for Swift-to-Rust bridging.

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
| C FFI wrappers for macOS Swift frontend | `impulse-ffi` |
| Window management, tab UI, native widgets | `impulse-linux` or `impulse-macos` |
| Terminal widget creation and configuration | `impulse-linux` or `impulse-macos` |
| Keybinding registration and UI | `impulse-linux` or `impulse-macos` |
| Theme/styling | `impulse-linux` or `impulse-macos` |

## Scripts

- **scripts/install-lsp-servers.sh** — Installs managed web LSP servers (typescript-language-server, etc.) to `~/.local/share/impulse/lsp/`. Invoked via `--install-lsp-servers` CLI flag.
- **scripts/vendor-monaco.sh** — Downloads and vendors Monaco Editor into `impulse-editor/vendor/monaco/`. Run once or when upgrading Monaco.
- **scripts/release.sh** — Cross-platform release script. Tags a release, builds, and produces distribution packages. Run on Linux for .deb/.rpm/.pkg.tar.zst, on macOS for .app/.dmg. Usage: `./scripts/release.sh 0.4.0 [--push] [--macos-only] [--linux-only]`.

## Project Directories

- **assets/** — App logo SVG (`impulse-logo.svg`), `.desktop` file, screenshots, and `icons/` subdirectory with file type SVG icons.
- **pkg/arch/** — PKGBUILD for Arch Linux packaging.
- **dist/** — Built distribution packages (.deb, .rpm, .pkg.tar.zst for Linux; .app, .dmg for macOS).

## System Dependencies

### Linux (GTK4 stack)

Building `impulse-linux` requires GTK4, libadwaita, VTE4, GtkSourceView5, and WebKitGTK development libraries. On Arch/CachyOS:

```bash
sudo pacman -S gtk4 libadwaita vte4 gtksourceview5 webkit2gtk-4.1
```

### macOS

Building `impulse-macos` requires Xcode command line tools and a Rust toolchain. The build script (`./impulse-macos/build.sh`) first compiles `impulse-ffi` as a static library via Cargo, then builds the Swift package via SwiftPM. AppKit and WKWebView are provided by the system frameworks. Terminal emulation uses the [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) library (declared in `Package.swift`). OpenSSL is vendored (statically linked) via `impulse-ffi` so Homebrew is not required.
