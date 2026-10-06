# Changelog

All notable changes to Impulse are documented in this file.

## Unreleased

### Terminal IDE redesign

Impulse moves from an Apple-styled tabbed app to a terminal IDE workbench,
with deeper git integration and first-class support for coding agents
running in its terminals. Impulse itself never calls an AI service.

**Workbench**

- Themed workbench chrome: titlebar tabs, a workspaces sidebar, docks and a status bar, with Lucide icons.
- Workspaces (folders with their own tabs), split panes of any surface, and session restore of every window's workspaces, layouts and terminal output.
- A fuzzy command palette with modes: files, `>` commands, `:` line, `%` text, `@` / `#` symbols, `b:` branches, `t:` tabs, `w:` workspaces, `h:` history, `pr:` pull requests, `set:` settings, `a:` project actions.
- Settings and Keyboard Shortcuts as tabs, described by one catalog (which also validates `settings.json` in the editor).
- Quick terminal on a global shortcut (off by default).
- Closing a tab or pane can be undone with ⌘Z for ten seconds; toasts and sheets replace blocking alerts.

**Terminal**

- A multi-line command editor for input, with shell highlighting, history (Ctrl-R), completions for subcommands, options, branches, scripts and paths, and an underline on commands the shell can't run.
- Optional: ask fish for completions (`complete -C`) alongside the built-in ones.
- Command blocks: inline status, keyboard selection, bookmarks, a toolbar, and copy/send actions.
- Find with match counts and case/word/regex toggles; hints mode for URLs, paths, commit SHAs and ports in output.
- Persistent command history, desktop notifications for background terminals, listening ports per workspace, and iTerm2 session status (OSC 21337).
- The kitty keyboard protocol for programs that ask for it.
- Services: terminal selections work with the Services menu; "New Impulse Workspace Here" in Finder.

**Agents**

- Impulse notices coding agents in its terminals (Claude Code, Codex, Gemini CLI, Aider, opencode, Amp, Copilot CLI and more) and shows what they're doing.
- Each agent turn is checkpointed: review it, restore the files from before it, or send review comments back.
- A composer (⌘I) and toolbelt for agents; send code, files and command output to them.
- Task worktrees (`../<repo>.worktrees/<branch>`), agent hooks, session resume after a restart, and the `impulse` command-line tool.

**Git**

- A Changes panel with a commit composer and full keyboard control.
- A rebuilt, native review: navigator, scopes (unstaged, staged, branch, commit, range, stash, last agent turn, since last review), syntax-colored unified and split diffs, hunk and line staging, comments, and imported pull request threads.
- Live change marks in the editor with a peek, inline blame, and a side-by-side diff view you can edit.
- History with a commit graph, compare, filters (`author:`, `path:`, `since:`, `until:`) and fork-point dimming.
- Branch switching and management, undoable stash drop/pop, merge-conflict resolution in the editor (or by an agent), and GitHub pull requests through `gh`.
- Writes go through the git CLI; reads stay on libgit2.

**Editor and language servers**

- Problems panel, go to symbol in file or project, project-wide replace with a preview, and Markdown preview beside the editor with Run buttons on shell blocks.
- Language servers: cross-file renames and code actions applied across files with one Undo, commands and `codeAction/resolve`, `workspace/applyEdit`, document highlights, inlay hints (with a setting), type definition, implementation, server messages and progress. sourcekit-lsp serves Swift.
- Requests no longer queue behind each other, and every server for a language sees opened documents.

**Accessibility**

- VoiceOver can read terminal text; agent status changes are announced.
- Reduce Motion and Increase Contrast are honored; keyboard-focused lists show a focus ring.

**Fixes**

- Edit ▸ Undo/Redo now reach text fields and the window (they sent a selector nothing implemented).
- Monaco could only edit its own file, so renames that touched other files failed.

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
