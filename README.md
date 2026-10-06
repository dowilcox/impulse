<p align="center">
  <img src="assets/impulse-logo.svg" width="120" alt="Impulse logo">
</p>

<h1 align="center">Impulse</h1>

<p align="center">
  A terminal-first development environment for the Mac.
</p>

<p align="center">
  <a href="#features">Features</a> &bull;
  <a href="#installation">Installation</a> &bull;
  <a href="#building-from-source">Building from Source</a> &bull;
  <a href="#architecture">Architecture</a> &bull;
  <a href="#license">License</a>
</p>

---

<p align="center">
  <img src="assets/screenshot-mac.png" width="800" alt="Impulse on macOS">
</p>

Impulse is a terminal IDE: a fast terminal with Warp-style command blocks, a Monaco code editor, deep git integration, and first-class support for the coding agents you run in your terminals. It's built for developers who live in the terminal but want editing, review and project awareness next to it.

The app is native Swift (AppKit + SwiftUI); terminal emulation runs on a Rust core built on `alacritty_terminal`. Impulse never calls an AI service itself; it hosts the agent CLIs you already use.

## Features

**Workbench**

- Workspaces (folders with their own tabs) in a sidebar, titlebar tabs, and split panes of any surface
- Session restore of every window's workspaces, layouts and terminal output
- Fuzzy command palette with modes: files, `>` commands, `:` line, `%` text, `@`/`#` symbols, `b:` branches, `h:` history, `pr:` pull requests, `a:` project actions, and more
- Project actions from `.impulse/project.toml`; the `impulse` command-line tool
- Quick terminal on a global shortcut; ⌘Z brings back a tab you just closed
- 19 built-in themes plus user themes; Settings and Keyboard Shortcuts as searchable tabs

**Terminal**

- Shell integration for bash, zsh and fish; command blocks with status, duration, selection, bookmarks and actions
- A multi-line input editor with shell highlighting, history (Ctrl-R), completions for subcommands, options, branches and scripts, and an underline on commands the shell can't run (optionally with fish's own completions)
- Find with match counts and regex; hints mode to open URLs, paths, SHAs and ports from the keyboard
- Persistent command history, background notifications, listening ports, the kitty keyboard protocol, OSC 8 links and iTerm2 session status

**Coding agents**

- Recognizes Claude Code, Codex, Gemini CLI, Aider, opencode, Amp, Copilot CLI and others in its terminals, and shows when each is working or waiting for you
- Checkpoints every agent turn so you can review it, restore files from before it, or send review comments back
- A composer (⌘I), a toolbelt, and "send to agent" for code, files and command output
- Task worktrees for parallel work, agent hooks, and session resume after a restart

**Git**

- Changes panel with a commit composer and keyboard control
- Review with scopes (unstaged, staged, branch, commit, range, stash, last agent turn, since your last review), hunk and line staging, comments, and pull request threads
- Live change marks, inline blame, and an editable side-by-side diff view in the editor
- History with a commit graph, compare, author/path/date filters and fork-point dimming
- Branch switching and management, undoable stashes, merge-conflict resolution, and GitHub pull requests through `gh`
- Fetch, pull (fast-forward, rebase or merge), push and force push with lease from the Git menu or palette, with optional background fetch
- Tags from History or at HEAD (lightweight or annotated, optionally pushed right away); merge, rebase and "open on GitHub/GitLab" from History

**Editor**

- Monaco with language servers: completions, hover, definitions, references, rename across files, code actions, inlay hints, highlights, formatting and diagnostics in a Problems panel
- Go to symbol in a file or the project, project-wide find and replace with a preview
- Markdown and SVG preview beside the editor, with Run buttons on shell code blocks
- Bundled JetBrains Mono for editor and terminal

**Accessibility**

- VoiceOver can read terminal output; agent status changes are announced
- Reduce Motion and Increase Contrast are honored

## Installation

Download `Impulse-X.Y.Z.dmg` from [GitHub Releases](https://github.com/dowilcox/impulse/releases), open it, and drag **Impulse.app** to your **Applications** folder.

Requires **macOS 26 (Tahoe)** or later.

> Looking for the old Linux (GTK4) app? It shipped through `v0.29.0` — grab those packages from the corresponding release. Impulse is Mac-first as of the Swift rewrite.

## Building from Source

Requires [Rust](https://rustup.rs/), Xcode (for the Swift toolchain), and CMake (`brew install cmake`, for the vendored libgit2).

```bash
git clone https://github.com/dowilcox/impulse.git
cd impulse
./impulse-macos/build.sh                           # produces dist/Impulse.app
./impulse-macos/build.sh --dev                     # produces dist/Impulse Dev.app (separate bundle ID)
./impulse-macos/build.sh --dmg                     # also creates dist/Impulse-X.Y.Z.dmg
./impulse-macos/build.sh --sign                    # build + codesign with Developer ID
./impulse-macos/build.sh --sign --notarize --dmg   # build + sign + notarize + .dmg
```

The build script handles all steps automatically: building the vendored libgit2, the Rust terminal FFI library, copying Monaco editor assets, compiling the Swift app, and assembling the `.app` bundle. Code signing auto-detects your Developer ID from the keychain, or you can set `IMPULSE_SIGN_IDENTITY` explicitly.

The `--dev` flag builds with bundle ID `dev.impulse.Impulse.Devel`, allowing the dev build to run side-by-side with an installed release.

To run the built app:

```bash
open dist/Impulse.app
```

**Optional — install managed LSP servers** (for web language support):

```bash
./scripts/install-lsp-servers.sh
```

## Testing

```bash
# Swift (the wrapper adds the flags the active toolchain needs)
impulse-macos/swiftw test

# Rust terminal core
cargo test -p impulse-terminal
```

## Architecture

The app is a Swift package with a small Rust core for terminal emulation.

| Component          | Role                                                                                       |
| ------------------ | ------------------------------------------------------------------------------------------ |
| `ImpulseApp`       | The app: AppKit/SwiftUI UI, CoreText terminal renderer, Monaco WebViews                    |
| `ImpulseKit`       | Pure logic: themes, previews, palette, completion, layouts, agents, git models, LSP edits  |
| `ImpulseGit`       | Git layer: reads on a vendored static libgit2, writes through the git CLI                  |
| `ImpulseLSP`       | LSP client: server processes, JSON-RPC framing, document sync, managed installs            |
| `ImpulseProtocol`  | Control-socket messages shared by the app and the `impulse` CLI                            |
| `ImpulseCLI`       | The `impulse` command-line tool: open, edit, review, split, notify from an Impulse terminal |
| `impulse-terminal` | Rust: terminal emulation (`alacritty_terminal`), OSC parsing, command blocks, history      |
| `impulse-ffi`      | Rust: C FFI static library exposing the terminal core to Swift                             |

Much of the Swift logic is verified against golden fixtures generated from the original Rust implementation (see `impulse-macos/Tests/*/Fixtures`).

## Releasing

```bash
./scripts/release.sh 0.30.0          # bump VERSION, tag, build signed+notarized .app/.dmg
./scripts/release.sh 0.30.0 --push   # …then push and create the GitHub release
```

The script writes the top-level `VERSION` file (the single source of truth), syncs crate versions, commits, tags, builds via `impulse-macos/build.sh --dmg --sign --notarize`, generates `SHA256SUMS`, and uploads everything in `dist/`.

## License

[GPLv3](LICENSE)
