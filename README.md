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

Impulse combines a terminal emulator with a Monaco-powered code editor in a modern tabbed interface. It's designed for developers who live in the terminal but want integrated editing, file navigation, and project awareness without leaving their workflow.

The app is native Swift (AppKit + SwiftUI); terminal emulation runs on a Rust core built on `alacritty_terminal`.

## Features

**Terminal**

- Terminal emulator with shell integration (bash, zsh, fish)
- Warp-style command blocks with exit status, duration, and jump-to-block navigation
- Command input bar with history ghost suggestions and path completion
- OSC 133/7 escape sequence support for prompt/command/CWD tracking
- Configurable scrollback, cursor shape, copy-on-select, and more

**Editor**

- Monaco editor for full-featured code editing
- Syntax highlighting for 80+ languages
- LSP integration with managed language server installation (completions, hover, go-to-definition, references, rename, code actions, formatting, signature help)
- Auto-detected indentation, configurable tab width and spaces/tabs
- Code folding, minimap, bracket pair colorization, indent guides
- Git diff gutter showing added/modified/deleted lines
- Review Changes tab with per-file diffs, word-level highlights, commit and discard
- Markdown preview with syntax-highlighted code blocks
- SVG preview with themed background
- Bundled JetBrains Mono font for editor and terminal

**Project Navigation**

- File sidebar with lazy-loaded directory tree
- File icons for 50+ languages and file types
- Git status coloring on filenames (added, modified, untracked, etc.)
- Project-wide file name and content search (gitignore-aware)
- Quick-open file picker (Cmd+P)

**Automation**

- Per-file-type indentation overrides (tab width, spaces/tabs)
- Commands-on-save with file pattern matching and optional file reload (for formatters)
- Custom keybindings that run shell commands

**Interface**

- Tabbed interface with command palette and pin tab support
- 19 built-in color themes (Kanagawa, Nord, Gruvbox, Tokyo Night, Catppuccin, Rose Pine, ...) plus user themes
- Settings UI with live-updating preferences for editor, terminal, appearance, automation, and keybindings
- Full keybinding visibility and customization UI — click any shortcut to rebind it
- Drag-and-drop file opening

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
# Swift (from impulse-macos/; needs full Xcode, not just CommandLineTools)
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test

# Rust terminal core
cargo test -p impulse-terminal
```

## Architecture

The app is a Swift package with a small Rust core for terminal emulation.

| Component          | Role                                                                                       |
| ------------------ | ------------------------------------------------------------------------------------------ |
| `ImpulseApp`       | The app: AppKit/SwiftUI UI, CoreText terminal renderer, Monaco WebViews                    |
| `ImpulseKit`       | Pure logic: themes, previews, command palette, input completion, file tree, settings logic |
| `ImpulseGit`       | Git layer on a vendored static libgit2 (status, diffs, blame, commit, search)              |
| `ImpulseLSP`       | LSP client: server processes, JSON-RPC framing, document sync, managed installs            |
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
