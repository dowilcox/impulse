# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Impulse?

Impulse is a terminal-first development environment built with Rust. It combines a terminal emulator with a Monaco-powered code editor in a tabbed interface, with native frontends for Linux (Qt Quick Controls 2 via CXX-Qt) and macOS (AppKit/SwiftUI).

## Cross-Platform Development Rule

**IMPORTANT: When making any feature change, bug fix, or UI addition, the change MUST be implemented in BOTH the Linux (`impulse-linux`) and macOS (`impulse-macos`) frontends.** Shared logic belongs in `impulse-core` or `impulse-editor`. Platform-specific UI code goes in the respective frontend crate. If a feature cannot be implemented on one platform yet, add a TODO comment in that frontend's code referencing the feature and the other platform's implementation.

## Build & Development Commands

**Note:** `cargo build` (all workspace members) only works on Linux where Qt6 libraries are available. On macOS, build specific crates or use the macOS build script.

```bash
# Rust workspace (impulse-core, impulse-editor, impulse-linux, impulse-ffi)
cargo build                        # Build all workspace members (Linux only — needs Qt6)
cargo build -p impulse-core        # Build only the core library (cross-platform)
cargo build -p impulse-editor      # Build only the editor crate (cross-platform)
cargo build -p impulse-linux       # Build only the Linux frontend (Linux only)
cargo build -p impulse-ffi         # Build only the FFI static library (cross-platform)
cargo run -p impulse-linux         # Run the Linux app (Linux only)
cargo run -p impulse-linux -- --dev  # Run in dev mode (uses separate app ID + config)
cargo check                        # Type-check without full compilation
cargo fmt                          # Format all code
cargo clippy                       # Lint
cargo test                         # Run tests
cargo test -p impulse-core         # Test only the core crate

# macOS (Swift Package, built separately)
./impulse-macos/build.sh           # Build .app bundle (builds impulse-ffi + Swift app)
./impulse-macos/build.sh --dev     # Build dev variant (separate bundle ID, runs side-by-side)
./impulse-macos/build.sh --dmg     # Build .app + .dmg disk image
./impulse-macos/build.sh --sign    # Build + codesign with Developer ID
./impulse-macos/build.sh --sign --notarize --dmg  # Full release build
```

### Platform-aware build verification

When verifying builds, use the right commands for the current platform:

- **On macOS:** Build cross-platform crates with `cargo build -p impulse-core -p impulse-editor -p impulse-ffi`, run tests with `cargo test -p impulse-core -p impulse-editor -p impulse-ffi`, and build the macOS app with `./impulse-macos/build.sh`. Do NOT attempt `cargo build -p impulse-linux` or `cargo check -p impulse-linux` — it will fail due to missing Qt6 system libraries.
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
- **util.rs** — Shared utilities: `language_from_uri()` for language ID detection, `file_path_to_uri()` / `uri_to_file_path()` conversions, file pattern matching for settings overrides.
- **shell_integration/\*.sh** — Shell scripts emitting OSC 133 and OSC 7 escape sequences.

### impulse-editor (library, Monaco assets)

Bundles the vendored Monaco editor and defines the WebView communication protocol.

- **assets.rs** — Embeds the Monaco vendor directory and editor HTML via `include_dir!` / `include_str!`.
- **protocol.rs** — `EditorCommand` and `EditorEvent` enums for bidirectional JSON messaging between Rust and the Monaco WebView.
- **css.rs** — CSS color sanitizer validating `#hex`, `rgb()`, and `rgba()` color formats with fallbacks for theme customization.
- **markdown.rs** — Markdown preview renderer using `pulldown_cmark` with themed HTML output and highlight.js syntax highlighting.
- **svg.rs** — SVG preview renderer embedding SVG sources in themed HTML documents with centered layout.

### impulse-linux (binary, Qt Quick Controls 2 + CXX-Qt frontend)

Uses CXX-Qt 0.8 to bridge Rust and Qt/QML. The UI is defined in QML files under `qml/`, with Rust `#[cxx_qt::bridge]` modules exposing QObject models to QML. A C++ helpers layer in `cpp/` provides any Qt functionality not yet covered by CXX-Qt bindings. The build is entirely cargo-driven via `cxx-qt-build` in `build.rs`.

- **build.rs** — CXX-Qt build script using `CxxQtBuilder::new_qml_module()`. Registers QML files, Rust bridge source files, links Qt modules (QuickControls2, WebEngineQuick), and compiles C++ helpers.
- **main.rs** — Qt application setup with `QGuiApplication`, QML engine initialization, CLI flags (`--install-lsp-servers`, `--check-lsp-servers`).
- **window_model.rs** — `#[cxx_qt::bridge]` module exposing `WindowModel` QObject with properties and invokables for tab management, sidebar state, and application-level actions.
- **theme_bridge.rs** — `#[cxx_qt::bridge]` module exposing theme colors and palette data to QML.
- **file_tree_model.rs** — `#[cxx_qt::bridge]` module providing a tree model for the sidebar file browser with lazy-loading and git status enrichment.
- **editor_bridge.rs** — `#[cxx_qt::bridge]` module bridging the Monaco WebView editor (via Qt WebEngine) with Rust-side editor logic.
- **lsp_bridge.rs** — `#[cxx_qt::bridge]` module exposing LSP client operations (completions, hover, diagnostics) to QML.
- **search_model.rs** — `#[cxx_qt::bridge]` module for project-wide file and content search.
- **settings_model.rs** — `#[cxx_qt::bridge]` module exposing `Settings` struct to the QML settings UI.
- **keybindings.rs** — Built-in keybinding registry, accel parsing, and override resolution.
- **helpers.rs** — Rust-side utility functions.
- **qml/** — QML UI files: `Main.qml` (root window), `Sidebar.qml`, `FileTreeView.qml`, `FileNodeDelegate.qml`, `TabBar.qml`, `ContentArea.qml`, `TerminalView.qml`, `EditorView.qml`, `StatusBar.qml`, `SearchPanel.qml`, `QuickOpenDialog.qml`, `CommandPalette.qml`, `GoToLineDialog.qml`, `SettingsWindow.qml`.
- **cpp/** — C++ helper code (`helpers.cpp`/`helpers.h`) for Qt functionality not exposed through CXX-Qt bindings.

### impulse-ffi (static library, C-compatible FFI)

C-compatible wrappers around `impulse-core` and `impulse-editor` for the macOS Swift frontend. Compiled as a static library (`libimpulse_ffi.a`). All functions use C strings for input/output and JSON encoding for complex types. Callers must free returned strings with `impulse_free_string`.

- **lib.rs** — `extern "C"` functions exposing filesystem, git, search, LSP, PTY, shell detection, and editor asset operations to Swift via the `CImpulseFFI` module.

### impulse-macos (Swift Package, macOS frontend)

The macOS frontend, built as a Swift Package (not a Cargo crate). Requires **macOS 26+ (Tahoe)**. Communicates with the Rust backend via `impulse-ffi` C FFI. Built with `./impulse-macos/build.sh`.

**Architecture:** The window uses a hybrid AppKit/SwiftUI approach. `MainWindowController` (AppKit) owns the window, NSToolbar, and tab content lifecycle. A single `NSHostingView` fills the window content area with a SwiftUI `NavigationSplitView` that renders the sidebar, tab bar, and status bar. Terminal and editor views remain AppKit (`NSView`) and are embedded via `NSViewRepresentable`. The `@Observable WindowModel` class is the bridge — AppKit mutates it, SwiftUI observes it.

#### App lifecycle & wiring

- **ImpulseApp.swift** — App entry point.
- **AppState.swift** — Global static flags (`isDev`) set once at startup.
- **AppDelegate.swift** — NSApplication delegate, app lifecycle.
- **MainWindow.swift** — `MainWindowController`: window setup, `NSToolbarDelegate` (sidebar toggle, new file/folder, refresh, collapse, hidden files, new tab, search — placed in titlebar like Apple apps using `.sidebarTrackingSeparator`), `NSHostingView` creation, `WindowModel` callback wiring, status bar syncing, file tree syncing. Uses `titlebarAppearsTransparent = true` and `titlebarSeparatorStyle = .none` for seamless toolbar/tab bar integration.
- **MainWindowController+LSP.swift** — LSP integration extension: background polling of LSP events (diagnostics, completions), batched processing, and main-thread dispatch.
- **TabManager.swift** — Tab management: tab creation/selection/close/reorder, content view lifecycle, `syncToWindowModel()` pushes tab info and `activeFilePath` to `WindowModel`.
- **Notifications.swift** — Centralized `NSNotification.Name` constants for theme/settings changes, tab management events, and search operations.
- **ResourceBundle.swift** — Bundle resource locator handling both packaged `.app` and development contexts for SwiftPM resources.
- **ShellEscape.swift** — String extension for shell-escaping arguments.

#### SwiftUI views (all visual chrome)

- **SwiftUI/Models/WindowModel.swift** — `@Observable` state class shared between AppKit and SwiftUI. Contains tab display info, sidebar state, file tree nodes, status bar fields, theme, icon cache, active file path, and callback closures for SwiftUI→AppKit communication.
- **SwiftUI/Views/MainContentView.swift** — Root SwiftUI view: `NavigationSplitView` with sidebar + detail (tab bar, content area, status bar).
- **SwiftUI/Views/SidebarView.swift** — Switches between `FileTreeListView` and `SearchPanelView` based on search state.
- **SwiftUI/Views/FileTreeListView.swift** — Recursive file tree using `ScrollView` + `LazyVStack` (not `List`, to avoid NSOutlineView/DisclosureGroup click conflicts). Manual chevron expand/collapse, themed SVG icons via `IconCache`, git status colored file names and badges, hover highlighting, active file highlighting, context menus (new file, new folder, rename, delete, reveal in Finder, copy path).
- **SwiftUI/Views/TabBarView.swift** — Finder-style tab bar: full-width pill tabs, hidden with one tab, hover-reveal close buttons, drag-drop reordering via `DropDelegate`.
- **SwiftUI/Views/StatusBarView.swift** — Bottom status bar: shell name, git branch, CWD, blame info, cursor position, language, encoding, indent, preview toggle.
- **SwiftUI/Views/SearchPanelView.swift** — Search results display with case-sensitive toggle, result count, debounced search with generation counter to prevent stale results.
- **SwiftUI/Representables/ContentAreaRepresentable.swift** — `NSViewRepresentable` wrapping `TabManager.contentView` in a `ContentContainer` that syncs frames and posts resize notifications for SwiftTerm sizing.

#### AppKit components (terminal, editor, data loading)

- **Terminal/TerminalContainer.swift** — Terminal view with splitting support.
- **Terminal/TerminalTab.swift** — Terminal tab using SwiftTerm for terminal emulation.
- **Editor/EditorTab.swift** — Monaco editor tab via WKWebView.
- **Editor/EditorProtocol.swift** — Bidirectional JSON messaging with Monaco (mirrors `impulse-editor` protocol).
- **Editor/EditorWebViewPool.swift** — WebView pooling for editor instances.

#### Sidebar data (AppKit, kept for data loading — not rendered)

- **Sidebar/FileTreeView.swift** — NSOutlineView-based file tree. Kept alive for `refreshTree()`, `showNameInputAlert()`, `rootNodes` data loading, but not displayed (SwiftUI `FileTreeListView` renders the tree).
- **Sidebar/FileTreeNode.swift** — `@Observable` tree node model. Lazy-loads children via `FileManager`, supports git status enrichment. `.DS_Store` files are always filtered out.
- **Sidebar/FileIcons.swift** — `IconCache` class: loads SVG icons from bundle, recolors with theme colors, caches as `NSImage`. Used by both `FileTreeListView` and `TabManager` for file/folder/toolbar icons.
- **Sidebar/SearchPanel.swift** — AppKit search panel (kept for `setRootPath()` data, not rendered).

#### Other AppKit UI

- **UI/CommandPalette.swift** — Command palette (equivalent to Linux Ctrl+Shift+P).
- **UI/MenuBuilder.swift** — macOS menu bar construction.
- **UI/StatusBar.swift** — AppKit status bar (receives updates alongside `WindowModel` for compatibility; will be removed when fully migrated).
- **Settings/Settings.swift** — `Settings` struct (Codable), stored at `~/Library/Application Support/impulse/settings.json`.
- **Settings/SettingsFormSheet.swift** — Settings editor form.
- **Settings/SettingsWindow.swift** — Settings window controller.
- **Theme/Theme.swift** — Color theme constants matching the Linux themes. Includes `bgSurface`, `border`, `accent` fields for the SwiftUI UI.
- **Keybindings/Keybindings.swift** — Keybinding registry and handling.
- **Bridge/ImpulseCore.swift** — Swift wrapper calling `impulse-ffi` C functions.
- **CImpulseFFI/** — C header module (`impulse_ffi.h` + `module.modulemap`) for Swift-to-Rust bridging.

### Key patterns

- **CXX-Qt bridge pattern (Linux):** Each Rust `#[cxx_qt::bridge]` module defines a QObject with properties, signals, and invokables that QML binds to. Rust owns the business logic; QML owns the presentation. State flows from Rust QObjects to QML via property bindings and signals.
- **QML styling (Linux):** Visual styling is driven by theme properties exposed from `theme_bridge.rs` as QObject properties. QML files reference these properties for colors, fonts, and spacing. No external CSS files.
- **SwiftUI/AppKit bridge (macOS):** `@Observable WindowModel` is the single source of truth for UI state. AppKit code (MainWindowController, TabManager) mutates it; SwiftUI views observe it for automatic re-rendering. Communication from SwiftUI back to AppKit uses callback closures on WindowModel (e.g., `onTabSelected`, `onOpenFile`, `onRefreshTree`). The NSToolbar uses `NSToolbarDelegate` with `.sidebarTrackingSeparator` to place items in the correct column. SwiftUI's `.toolbar {}` and `.searchable()` modifiers do NOT work inside `NSHostingView` — all toolbar items must be native `NSToolbarItem`.
- **File tree (macOS):** Uses `ScrollView` + `LazyVStack` with recursive `FileNodeView`, NOT `List` + `DisclosureGroup` (which has known click-handling conflicts with NSOutlineView). `FileTreeNode` is `@Observable` so expand/collapse and git status changes trigger SwiftUI re-renders. The old AppKit `FileTreeView` is kept alive (hidden) for its `showNameInputAlert()` dialog and data-loading methods but will be fully removed once those are migrated to SwiftUI.
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

### Linux (Qt6 stack)

Building `impulse-linux` requires Qt6 base, declarative (QML), WebEngine, and Quick Controls 2 libraries. CXX-Qt handles the Qt build integration via `build.rs`. On Arch/CachyOS:

```bash
sudo pacman -S qt6-base qt6-declarative qt6-webengine qt6-quickcontrols2 cmake
```

### macOS

Requires **macOS 26 (Tahoe) or later**. Building `impulse-macos` requires Xcode command line tools and a Rust toolchain. The build script (`./impulse-macos/build.sh`) first compiles `impulse-ffi` as a static library via Cargo, then builds the Swift package via SwiftPM. AppKit, SwiftUI, and WKWebView are provided by the system frameworks. Terminal emulation uses an in-tree backend (`impulse-terminal`) wrapping `alacritty_terminal`, rendered via CoreText + CoreGraphics in `TerminalRenderer.swift`. OpenSSL is vendored (statically linked) via `impulse-ffi` so Homebrew is not required.
