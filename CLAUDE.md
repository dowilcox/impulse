# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Impulse?

Impulse is a Mac-first terminal IDE: a terminal emulator with Warp-style command blocks combined with a Monaco-powered code editor in a tabbed interface. The app is Swift (AppKit + SwiftUI); the only remaining Rust is the terminal emulation core, reached through a small C FFI.

## Architecture

```
impulse-macos/            Swift package (the app) — macOS 26+ (Tahoe)
  Sources/ImpulseApp      executable: AppKit/SwiftUI UI, terminal renderer,
                          Monaco WebViews, LSP/git/search wiring
  Sources/ImpulseKit      pure logic (Foundation-only): themes, previews,
                          palette, completion + shell parser, file tree,
                          search results, close risk, glob, update checker,
                          pane LayoutTree, agents (known agents, state
                          machine, hook installer), command history DB,
                          worktree tasks, commit graph, PR/gh parsing,
                          terminal paths/hints/find queries
  Sources/ImpulseGit      git layer: libgit2 for reads (status, diff, blame,
                          log) + the git CLI for writes; safety snapshots;
                          gitignore-aware search
  Sources/ImpulseLSP      LSP client: server processes, JSON-RPC framing,
                          registry, document cache, managed npm installs
  Sources/ImpulseProtocol control-socket messages shared by app and CLI
  Sources/ImpulseCLI      the `impulse` command-line tool (bundled in
                          Contents/Resources/bin): open, edit --wait, review,
                          split, tab, notify, status, checkpoint, hook
  Clibgit2/               module map for the vendored static libgit2
  CImpulseFFI/            C header for the Rust terminal FFI
  web/                    editor.html/js (Monaco glue)
impulse-terminal/         Rust: terminal emulation (alacritty_terminal),
                          OSC 133/7/6973 scanning, command blocks, history
impulse-ffi/              Rust: C FFI over impulse-terminal (staticlib)
vendor/                   Monaco editor, fonts, highlight.js (committed)
scripts/                  build-libgit2.sh, vendor-monaco.sh, release.sh
```

Dependency direction: ImpulseApp → {ImpulseKit, ImpulseGit, ImpulseLSP, ImpulseProtocol, CImpulseFFI}; ImpulseGit/ImpulseLSP → ImpulseKit; `impulse` (CLI) → ImpulseProtocol. The Rust workspace is `impulse-terminal` + `impulse-ffi` only, and the FFI surface is terminal-only (~37 functions; JSON strings for complex data, a binary buffer for grid snapshots, plain integers on hot paths — mode bits, the block-overlay cache key, search stats — and `impulse_free_string` for cleanup). Command blocks are listed without their output; fetch one block by id when you need it.

### Why the split

- **ImpulseKit** is Foundation-only so its logic is headless-testable. No AppKit/WebKit imports there.
- **ImpulseGit** wraps a vendored static libgit2 1.9.1 built WITHOUT network transports for reads (status, diff/hunks with word-level spans, blame, log, ignore checks). Writes (stage, commit, push/pull, stash, branches, worktrees, conflict resolution) run the `git` CLI, and destructive ones take a safety snapshot under `refs/impulse/` first so they can be undone. No OpenSSL anywhere. New git APIs use the real `git` CLI as the test oracle.
- **ImpulseLSP** owns language-server processes with hand-rolled Content-Length framing and a poll-shaped facade (`pollEvent()` returns the same JSON envelopes the app has always decoded).
- **impulse-terminal (Rust)** stays because alacritty_terminal is a complete, battle-tested VT emulator. It owns the PTY, grid/scrollback, damage tracking, selection, scrollback search, and OSC 133/7/6973 command-block tracking. Swift owns all rendering (CoreText in `TerminalRenderer.swift`) and input encoding.
- `Bridge/ImpulseCore.swift` is the single seam: terminal calls go through the C FFI; everything else delegates to the Swift libraries. Keep new backend logic OUT of the bridge — put it in the right library target.

### Golden-fixture parity (important)

The Swift ports were verified against fixtures generated from the original Rust implementation, committed under `impulse-macos/Tests/ImpulseKitTests/Fixtures/` and `Tests/ImpulseGitTests/Fixtures/` (themes → Monaco JSON, git hunks/word-diff/blame/status, shell-parser tokenizations, file-tree patches, palette filtering, glob, close risk). Treat fixtures as the spec: never regenerate them from Swift output. If behavior changes intentionally, update the affected fixture explicitly in the same commit and say why.

## Build & Development Commands

```bash
# macOS app (canonical build — produces dist/Impulse.app)
./impulse-macos/build.sh             # release .app bundle
./impulse-macos/build.sh --dev       # "Impulse Dev.app" (separate bundle ID, runs side-by-side)
./impulse-macos/build.sh --dmg       # + disk image
./impulse-macos/build.sh --sign --notarize --dmg   # full release build

# Swift package directly — use the wrapper, which adds the flags the active
# toolchain needs (CLT Swift 6.4 lacks the SwiftUI macro plugin and puts
# Testing.framework off the default search path; swiftw borrows Xcode's macOS
# macro plugins and points tests at the CLT framework).
impulse-macos/swiftw build [-c release]
impulse-macos/swiftw test [--filter Name]
#   Tests are guarded with #if canImport(Testing); a run that prints no
#   "Test run with N tests" line means the Testing module wasn't found.

# Rust terminal core
cargo build -p impulse-ffi           # staticlib the Swift app links
cargo test -p impulse-terminal       # block/OSC/history tests
cargo fmt && cargo clippy

# One-time / occasional
./scripts/build-libgit2.sh           # vendored libgit2 (cached; build.sh runs it)
./scripts/vendor-monaco.sh           # refresh vendor/monaco
```

Note: `swift build` links `../target/release/libimpulse_ffi.a` — run `cargo build --release -p impulse-ffi` first on a fresh checkout (build.sh does all of this in order). SwiftPM does not track `.a` mtimes; build.sh has a relink hack for that.

## Key patterns

- **SwiftUI/AppKit bridge:** `@Observable WindowModel` is the single source of truth for UI state. AppKit (MainWindowController, TabManager) mutates it; SwiftUI observes it. SwiftUI→AppKit communication uses callback closures on WindowModel. The window chrome is drawn by `WorkbenchView` under a transparent titlebar; the window's `NSToolbar` is empty and only sizes the titlebar band (SwiftUI `.toolbar {}` does not work inside `NSHostingView`).
- **Window controller:** `MainWindowController` keeps its stored state and setup in `MainWindow.swift`; everything else is in `MainWindowController+<Area>.swift` extensions (Layout, Observers, Save, FindBar, TabClose, Session, Workspaces, Preview, Palette, GitHost, LSP, Debug). Add code to the matching extension, not to `MainWindow.swift`.
- **Commands, settings, keys:** user-facing commands are `AppCommand`s in `App/CommandRegistry.swift` (palette and menus); every setting is described once in `Settings/SettingsCatalog.swift` (the Settings tab and the `settings.json` schema come from it); shortcuts live in `Keybindings`. Debug-only UI actions for headless snapshots are in `MainWindowController+Debug.swift`.
- **Workbench:** a window has workspaces (folder or scratch; task worktrees live beside the repo at `<repo>.worktrees/<branch>`), each with tabs; a tab is one surface or a split (`ImpulseKit.LayoutTree` of panes, laid out by `PaneLayoutView`). `TabManager` owns them; session restore v2 (`SessionState.swift`) saves workspaces, layouts and terminal scrollback.
- **Agents:** `TerminalTab+Agent` detects CLI agents (Claude Code, Codex, …) from the foreground process and hook events; `AgentStateMachine` drives the inbox, tab dots and notifications. Each window listens on a control socket (`ControlServer`; every terminal gets `IMPULSE_SOCKET` / `IMPULSE_PANE_TOKEN`) for the `impulse` CLI. Agent turns are checkpointed under `refs/impulse/checkpoints/` for "review last turn". Impulse never calls an AI model itself.
- **Terminal:** event-driven. The Rust core calls a wake callback when output arrives; `TerminalRenderer` then runs `pollEvents()` → `takeDamage()` → invalidate rows → `draw` pulls a binary grid snapshot (16-byte header + 12-byte cells) parsed zero-copy by `GridBufferReader`. Command-block decorations come from `impulse_terminal_block_overlay`, cached by `impulse_terminal_block_overlay_key`. Input is the `CommandEditor` (an NSTextView with shell highlighting); completion is Swift (`ImpulseKit.InputCompletion`) fed by persistent history (`CommandHistoryDatabase`) and `impulse_terminal_recent_commands`.
- **Shell integration:** `ImpulseKit/Resources/ShellIntegration/{bash,zsh,fish}.sh` emit OSC 133 (prompt/command marks), OSC 7 (cwd) and OSC 6973 (`Command=` the command text, `Names=` aliases/functions/builtins and `Path=` PATH, sent only on change). Keys: `KeyEncoder` encodes legacy sequences; when a program pushes kitty keyboard flags (mode bits 11–15) `ImpulseKit.KittyKeyboard` takes over.
- **Editor:** Monaco in WKWebView, loaded from the app bundle (`EditorAssets.monacoDirectory`; build.sh copies vendor/ + impulse-macos/web/ into `Sources/ImpulseApp/Resources/monaco` — copy again and rebuild after editing `web/`). `EditorWebViewPool` pre-warms one WebView. Markdown preview renders with cmark-gfm in safe mode (raw HTML is elided by design — do not re-enable `CMARK_OPT_UNSAFE`).
- **Language servers:** LSP requests go through `enqueueLspRequest` (sent after queued didOpen/didChange, answered off the queue); notifications reach every server for the language. Each editor tab is its own WebView, so Monaco can only edit its own file: workspace edits that reach further (rename, code actions, `workspace/applyEdit`) are applied in Swift by `Editor/LSPWorkspaceEdits.swift` on top of `ImpulseKit.WorkspaceEdit`.
- **Review:** native, not web. `ReviewSurface` (header + `ReviewNavigatorView` + `ReviewDiffController`) builds flat rows with `ReviewRowBuilder` into one NSTableView: line rows are custom-drawn (`DiffLineCellView`, character-wrapped, heights from `ReviewMetrics`), headers/comments/composer are hosted SwiftUI (`ReviewRowViews.swift`), file headers float as group rows. Syntax colors come from highlight.js in JavaScriptCore (`SyntaxHighlighter`, old/new sides tokenized separately) mapped to the theme's syntax colors. History's lower half is the same surface.
- **Themes:** TOML files in `Sources/ImpulseKit/Resources/Themes/` (user themes in `~/Library/Application Support/impulse/themes`). `ThemeStore` resolves them; `themeToMonaco` / `themeToMarkdownColors` derive editor/preview themes. New built-in themes: add the TOML resource and its name to `ThemeStore.builtinThemeNames()`.
- **File tree:** `FileTreeDataController` (headless) owns watchers and data; patches computed by `ImpulseKit.FileTreePatcher` with git-status enrichment composed from `ImpulseGit.GitClient`; rendered by `FileTreeListView` (ScrollView + LazyVStack, NOT List/DisclosureGroup).
- **Version:** the top-level `VERSION` file is the single source of truth. build.sh stamps it into Info.plist; the app reads `CFBundleShortVersionString` (`AppVersion.current`). Never version-bump by hand — `scripts/release.sh` does it.
- **Error handling:** library targets return optionals/Results with descriptive messages; NSLog/os_log for non-fatal issues.

## Scripts

**Always use the existing scripts for their intended tasks — do not replicate their steps manually.**

- **scripts/release.sh <version> [--push]** — the ONLY way to release: writes VERSION, syncs crate versions, commits, tags, builds signed+notarized .app/.dmg, checksums, and (with --push) pushes and creates the GitHub release. Never run `gh release create`, `git tag`, or manual version edits.
- **scripts/build-libgit2.sh** — pinned libgit2 static build into `impulse-macos/.libgit2/` (checksum-verified; idempotent).
- **scripts/vendor-monaco.sh** — refreshes `vendor/monaco`.
- **scripts/vendor-monaco-vim.sh** — refreshes `vendor/monaco-vim` (the editor's optional Vim mode; pinned version and checksum).
- **impulse-macos/build.sh** — builds the .app (libgit2 → impulse-ffi → asset copy → SwiftPM → bundle → optional sign/notarize/dmg).

## History

Impulse was originally a Rust workspace with a GTK4 Linux frontend and most backend logic in Rust crates (`impulse-core`, `impulse-editor`) behind an 88-function FFI. It was rewritten Mac-first in Swift in 2026; the last Rust-era release (including the Linux app) is tag `v0.29.0`. If you need old behavior for reference, read the Rust sources at that tag — the fixtures under Tests/\*/Fixtures were generated from it.
