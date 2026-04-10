# Changelog

All notable changes to Impulse are documented in this file.

## Unreleased

### macOS — Terminal backend migration and polish

**Breaking**

- **macOS minimum is now Tahoe (26.0).** Bumped from Sonoma (14). Required for modern AppKit APIs (`NSView.displayLink`) and to align the Swift deployment target with the Rust FFI build target, which eliminates linker warnings at build time.

**Terminal**

- Replaced SwiftTerm with an in-tree `impulse-terminal` backend wrapping `alacritty_terminal`, rendered via CoreText/CoreGraphics in `TerminalRenderer.swift`. Shell integration (OSC 133, OSC 7) is implemented via a custom PTY read thread and `OscScanner`.
- Custom NSView-based renderer with a `CADisplayLink`-driven refresh loop, run-based CoreText drawing, exact per-glyph cell positioning, and a binary grid snapshot buffer for zero-copy cell reads.
- Regex-based terminal search via alacritty's search engine.
- Live theme recoloring: changing theme applies to running terminals instantly.
- Split terminals no longer go black when a pane exits.

**Terminal settings that now actually take effect at runtime**

- Cursor shape (block/beam/underline)
- Cursor blink (with 0.5s timer, resets on keyboard input)
- `terminalBell` gates the bell beep
- `terminalBoldIsBright` substitutes palette 8-15 for bold ANSI 0-7
- `terminalScrollOnOutput` auto-follows output when not scrolled back
- Cursor color derived from the theme's foreground (previously hardcoded)
- Scrollback, shape, and blink are now set at creation and refreshed on any settings change

**Terminal input / selection**

- Mouse click/drag/release forwarded to the PTY as SGR or X10 escape sequences when the program enables mouse reporting (`CSI ? 1000 / 1002 / 1006`), including modifier flags. Restores tmux mouse mode, vim visual mouse selection, and clicks in TUI apps (fzf, lazygit).
- `NSTextInputClient` implementation for dead keys and IME composition. Marked text is drawn as an underlined overlay at the cursor position, and `firstRect(forCharacterRange:)` returns the cursor cell in screen coordinates so the IME candidate window anchors correctly.
- Right-click context menu with Copy / Paste / Select All / Clear.
- Wide-character (East Asian fullwidth) text now advances 2 columns correctly; subsequent characters on the same row stay aligned with the grid.

**Hyperlinks (OSC 8 + auto-detection)**

- OSC 8 hyperlinks tracked via a new `HYPERLINK` cell flag bit and resolved via `impulse_terminal_hyperlink_at(col, row)` FFI.
- Plain URL auto-detection scans each row with a `https?://…` regex on hover, trims trailing punctuation, and handles wide-char spacer cells.
- Hover shows a pointing-hand cursor and an underline across the detected range. Cmd+Click opens the URL in the default handler via `NSWorkspace`.

**Box-drawing characters**

- Added 50+ box-drawing characters rendered as primitives instead of font glyphs: eighth blocks (`▁▂▃▅▆▇` / `▉▊▋▍▎▏`), shade blocks (`░▒▓`), and all quadrant combinations (`▖▗▘▙▚▛▜▝▞▟`). Improves alignment for charts, progress bars, and TUI frames.

**File tree performance**

- Replaced the recursive SwiftUI `FileNodeView` with a flat `FlatFileRowView` rendered in a single `LazyVStack`. Only visible rows are materialized — previously all expanded nodes had live SwiftUI views, which caused major slowdowns at ~50+ expanded folders.
- `loadChildren()` moved off the main thread to eliminate UI hitches on expansion.
- Git status mutations on individual nodes no longer trigger re-renders of the entire expanded tree.
- Filesystem watcher changes now sync to the SwiftUI sidebar so externally created/renamed/deleted files appear immediately (previously only the hidden AppKit tree saw them until a manual refresh).

**Build / tooling**

- Swift package tools version bumped to 6.2 (required for `.macOS(.v26)`). Swift language mode pinned to `.v5` to avoid strict concurrency regressions on existing AppKit delegate code.
- `MACOSX_DEPLOYMENT_TARGET=26.0` exported in `impulse-macos/build.sh` before the Cargo FFI build, so Rust-compiled objects link cleanly against the Swift target with no "built for newer version" linker warnings.
- `LSMinimumSystemVersion` in the `.app` Info.plist bumped from 13.0 to 26.0.
- Fixed three clippy warnings: `if_same_then_else` in `theme.rs`, `redundant_guards` in `lsp.rs`, and `large_enum_variant` in `protocol.rs` (Boxed `EditorOptions` in `EditorCommand::UpdateSettings`).
- `CADisplayLink` replaces the deprecated `CVDisplayLink` API in the terminal renderer.
- Migrated to the flat `FlatTreeEntry` architecture for the SwiftUI file tree.

### Known limitations

- **Terminal scrollback size only takes effect on new terminals.** Changing `terminalScrollback` in settings does not resize the buffer of already-running terminals (alacritty allocates the scrollback ring at `Term::new()` and does not expose a runtime resize API). Restart the tab or open a new terminal to apply the new size.
- **Cursor shape override is unconditional.** The user's `terminalCursorShape` preference is applied at the renderer layer and overrides any ANSI `DECSCUSR` escape sequence from running programs. Vim users who rely on per-mode cursor shape switching in insert/normal modes will see the same shape throughout. A future release will track program overrides separately so user preference acts as the default rather than a hard override.
- **OSC 133 prompt/command events are captured but not wired to UI.** The backend emits `PromptStart`, `CommandStart`, and `CommandEnd` events (visible in the CWD tracking path), but jump-to-prompt keybindings and exit-code display are not yet implemented. Requires exposing alacritty's absolute scrollback-line indices through the FFI so prompt positions survive scrolling.
- **Settings migration is additive only.** Users upgrading from pre-0.20 releases will keep their old persisted values for `terminalScrollOnOutput` and `terminalBoldIsBright` (both `false` in the previous default). New installs get `true` for both. Flip them in Settings → Terminal if you want the new defaults on an existing install.

---

Prior releases were not tracked in a formal changelog. See the [git history](https://github.com/dowilcox/impulse/commits/main) and [GitHub Releases](https://github.com/dowilcox/impulse/releases) for earlier changes.
