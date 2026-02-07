# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Impulse?

Impulse is a terminal-first development environment for Linux, built with Rust, GTK4, and libadwaita. It combines a VTE terminal emulator with a GtkSourceView code editor in a tabbed interface, inspired by Warp.

## Build & Development Commands

```bash
cargo build                        # Build all workspace members
cargo build -p impulse-core        # Build only the core library
cargo build -p impulse-linux       # Build only the GTK4 frontend
cargo run -p impulse-linux         # Run the application
cargo check                        # Type-check without full compilation
cargo fmt                          # Format all code
cargo clippy                       # Lint
cargo test                         # Run tests
cargo test -p impulse-core         # Test only the core crate
```

## Architecture

The workspace has two crates with a strict dependency direction: `impulse-linux` depends on `impulse-core`, never the reverse.

### impulse-core (library, no GUI dependencies)

Platform-agnostic backend logic. Designed to be reusable if other frontends are added.

- **pty.rs** — `PtyManager` owns PTY sessions in an `Arc<Mutex<HashMap>>`. Each session spawns a reader thread that runs an `OscParser` to detect shell integration escape sequences (OSC 133 for command start/end, OSC 7 for CWD changes) and forwards `PtyMessage` events through a `PtyEventSender` trait that frontends implement.
- **shell.rs** — Detects the user's shell from `/etc/passwd`/`$SHELL`, injects integration scripts via temp rc files (bash `--rcfile`, zsh `ZDOTDIR` wrapper, fish `--init-command`). The `prepare_shell_spawn()` function is the main entry point.
- **filesystem.rs** — Directory listing sorted dirs-first with git status enrichment via `git status --porcelain`.
- **search.rs** — File name and content search using the `ignore` crate for gitignore-aware walking.
- **shell_integration/*.sh** — Shell scripts emitting OSC 133 (A=prompt start, B=command start, C=output start, D=command end) and OSC 7 (CWD) escape sequences.

### impulse-linux (binary, GTK4 frontend)

- **main.rs** — `adw::Application` setup with app ID `dev.impulse.Impulse`, forces dark color scheme.
- **window.rs** — The main window builder (~900+ lines). Contains all keybinding setup, tab management (libadwaita `TabView`), sidebar toggling, editor/terminal search, command palette, and signal wiring. This is the central integration point.
- **terminal.rs** — Creates configured VTE terminals with Tokyo Night colors, shell integration, and drag-and-drop support. Calls `impulse_core::shell::prepare_shell_spawn()` to get the shell command, then uses VTE's `spawn_async` to run it.
- **terminal_container.rs** — Wraps VTE terminals in a `gtk4::Box` and handles horizontal/vertical splitting via `gtk4::Paned`.
- **editor.rs** — Creates GtkSourceView editors with auto-detected language and indentation.
- **sidebar.rs** — File tree with `Rc<RefCell<Vec<TreeNode>>>` for lazy-loaded directory expansion, plus a search panel. Exposes `SidebarState` for cross-component communication.
- **status_bar.rs** — `StatusBar` struct with labels for CWD, git branch, shell name, cursor position, language, encoding, and indentation info.
- **settings.rs** — `Settings` struct serialized to `~/.config/impulse/settings.json`. Loaded on startup, saved on quit.
- **theme.rs** — `TokyoNight` color constants and a `load_css()` function that generates and loads GTK CSS for the entire app.

### Key patterns

- **Shared state in GTK:** `Rc<RefCell<T>>` for mutable state shared across signal closures (single-threaded GTK main loop, no `Arc/Mutex` needed on the frontend side).
- **CSS styling:** All visual styling lives in `theme.rs` as a single formatted CSS string, applied via `add_css_class()` on widgets. No external CSS files.
- **Error handling:** Public APIs in `impulse-core` return `Result<T, String>`. Non-fatal errors use `log::warn!`.
- **Shell integration flow:** Shell scripts emit OSC escapes -> VTE passes raw bytes -> `OscParser` in pty.rs strips and interprets them -> `PtyMessage` events sent to frontend via the `PtyEventSender` trait.

## System Dependencies (GTK4 stack)

Building requires GTK4, libadwaita, VTE4, and GtkSourceView5 development libraries. On Arch/CachyOS:

```bash
# These are the underlying C libraries needed by the Rust bindings
# gtk4, libadwaita, vte4 (vte-2.91-gtk4), gtksourceview5
```
