# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Impulse?

Impulse is a terminal-first development environment built with Rust. It combines a terminal emulator with a Monaco-powered code editor in a tabbed interface, with native frontends for Linux (GTK4/libadwaita) and macOS (AppKit/SwiftUI).

## Cross-Platform Development Rule

**IMPORTANT: When making any feature change, bug fix, or UI addition, the change MUST be implemented in BOTH the Linux (`impulse-linux`) and macOS (`impulse-macos`) frontends.** Shared logic belongs in `impulse-core` or `impulse-editor`. Platform-specific UI code goes in the respective frontend crate. If a feature cannot be implemented on one platform yet, add a TODO comment in that frontend's code referencing the feature and the other platform's implementation.

## Build & Development Commands

**Note:** `cargo build` (all workspace members) only works on Linux where GTK4 libraries are available. On macOS, build specific crates or use the macOS build script.

```bash
# Rust workspace (impulse-core, impulse-editor, impulse-linux, impulse-ffi)
cargo build                        # Build all workspace members (Linux only — needs GTK4)
cargo build -p impulse-core        # Build only the core library (cross-platform)
cargo build -p impulse-editor      # Build only the editor crate (cross-platform)
cargo build -p impulse-linux       # Build only the Linux frontend (Linux only)
cargo build -p impulse-ffi         # Build only the FFI static library (cross-platform)
cargo run -p impulse-linux         # Run the Linux app (Linux only)
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

### Platform-aware build verification

When verifying builds, use the right commands for the current platform:

- **On macOS:** Build cross-platform crates with `cargo build -p impulse-core -p impulse-editor -p impulse-ffi`, run tests with `cargo test -p impulse-core -p impulse-editor -p impulse-ffi`, and build the macOS app with `./impulse-macos/build.sh`. Do NOT attempt `cargo build -p impulse-linux` or `cargo check -p impulse-linux` — it will fail due to missing GTK4 system libraries.
- **On Linux:** `cargo build` works for all Cargo workspace members. The macOS frontend (`impulse-macos`) cannot be built on Linux.

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
- **shell_integration/\*.sh** — Shell scripts emitting OSC 133 and OSC 7 escape sequences.

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

| Logic                                        | Crate                              |
| -------------------------------------------- | ---------------------------------- |
| PTY management, shell detection, OSC parsing | `impulse-core`                     |
| Filesystem listing, git status, search       | `impulse-core`                     |
| LSP client, JSON-RPC, server management      | `impulse-core`                     |
| Monaco assets, editor HTML, WebView protocol | `impulse-editor`                   |
| C FFI wrappers for macOS Swift frontend      | `impulse-ffi`                      |
| Window management, tab UI, native widgets    | `impulse-linux` or `impulse-macos` |
| Terminal widget creation and configuration   | `impulse-linux` or `impulse-macos` |
| Keybinding registration and UI               | `impulse-linux` or `impulse-macos` |
| Theme/styling                                | `impulse-linux` or `impulse-macos` |

## Scripts

**IMPORTANT: Always use the existing scripts for their intended tasks. Do NOT manually replicate what a script does with ad-hoc commands.**

- **scripts/release.sh** — The **only** way to create releases. Handles version bumping, git tagging, building, packaging, checksum generation, and GitHub release creation. See the "Release Process" section below for details.
- **scripts/install-lsp-servers.sh** — Installs managed web LSP servers (typescript-language-server, etc.) to `~/.local/share/impulse/lsp/`. Invoked via `--install-lsp-servers` CLI flag.
- **scripts/vendor-monaco.sh** — Downloads and vendors Monaco Editor into `impulse-editor/vendor/monaco/`. Run once or when upgrading Monaco.
- **impulse-macos/build.sh** — Builds the macOS `.app` bundle. Handles compiling `impulse-ffi`, copying Monaco assets, building Swift, creating the `.app` bundle, and optionally signing/notarizing/creating a `.dmg`. Called by `scripts/release.sh` during macOS releases — do NOT replicate its steps manually.

## Release Process

**CRITICAL: All releases MUST go through `scripts/release.sh`. Never manually run `gh release create`, `gh release upload`, `git tag`, or version-bump Cargo.toml files. The release script handles all of this correctly and consistently.**

### What the release script does

Every invocation of `scripts/release.sh <version>` performs these steps:

1. Bumps the version in all four `Cargo.toml` files and updates `Cargo.lock`
2. Commits the version bump and creates an annotated git tag (`vX.Y.Z`) — skipped if the tag already exists, or if `--macos-only`/`--linux-only` is passed
3. **Cleans `dist/`** to remove stale artifacts from previous releases
4. Builds platform-appropriate packages (Linux: `.deb`/`.rpm`/`.pkg.tar.zst`, macOS: signed+notarized `.app`/`.dmg`)
5. Generates SHA256 checksums for all artifacts in `dist/`
6. **Only with `--push`:** pushes the commit + tag to GitHub, then creates the GitHub release (or uploads to it if it already exists) with everything in `dist/`

**Important:** `--push` is additive — it does the full build first, then pushes. If the GitHub release already exists (e.g., created from another machine), it uploads artifacts with `--clobber` instead of failing.

### Flags

| Flag                  | Version bump & tag | Builds           | Pushes to GitHub |
| --------------------- | ------------------ | ---------------- | ---------------- |
| _(none)_              | Yes                | Current platform | No               |
| `--push`              | Yes                | Current platform | Yes              |
| `--macos-only`        | No                 | macOS only       | No               |
| `--linux-only`        | No                 | Linux only       | No               |
| `--macos-only --push` | No                 | macOS only       | Yes              |
| `--linux-only --push` | No                 | Linux only       | Yes              |

### Single-platform release (simplest)

If you only need to release from one machine:

```bash
./scripts/release.sh 0.8.0 --push
```

This bumps versions, tags, builds for the current platform, pushes, and creates the GitHub release in one step.

### Cross-platform release

Releases need artifacts from both macOS and Linux. The first platform creates the tag and GitHub release; the second platform builds its artifacts and uploads them to the existing release.

**Starting from macOS (recommended — signing/notarization is slow):**

```bash
# 1. On macOS — bump version, tag, build + sign + notarize, push + create release:
./scripts/release.sh 0.8.0 --push

# 2. On Linux — pull the tag, build Linux packages, upload to existing release:
git pull origin main
./scripts/release.sh 0.8.0 --linux-only --push
```

**Starting from Linux:**

```bash
# 1. On Linux — bump version, tag, build Linux packages, push + create release:
./scripts/release.sh 0.8.0 --push

# 2. On macOS — pull the tag, build + sign + notarize, upload to existing release:
git pull origin main
./scripts/release.sh 0.8.0 --macos-only --push
```

### What NOT to do for releases

- Do NOT run `gh release create` or `gh release upload` directly — use `./scripts/release.sh <version> --push`
- Do NOT manually edit version numbers in `Cargo.toml` files — the release script handles this
- Do NOT manually create git tags — the release script creates annotated tags
- Do NOT manually run `./impulse-macos/build.sh --sign --notarize --dmg` for releases — `scripts/release.sh` calls it with the correct flags
- Do NOT manually compute or upload checksums — the release script generates `SHA256SUMS`

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
