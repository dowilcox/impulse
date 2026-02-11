<p align="center">
  <img src="assets/impulse-logo.svg" width="120" alt="Impulse logo">
</p>

<h1 align="center">Impulse</h1>

<p align="center">
  A terminal-first development environment for Linux, built with Rust, GTK4, and libadwaita.
</p>

<p align="center">
  <a href="#features">Features</a> &bull;
  <a href="#installation">Installation</a> &bull;
  <a href="#building-from-source">Building from Source</a> &bull;
  <a href="#license">License</a>
</p>

---

Impulse combines a full VTE terminal emulator with a Monaco-powered code editor in a modern tabbed interface. It's designed for developers who live in the terminal but want integrated editing, file navigation, and project awareness without leaving their workflow.

## Features

**Terminal**
- Full VTE terminal with shell integration (bash, zsh, fish)
- Horizontal and vertical terminal splitting
- OSC 133/7 escape sequence support for prompt/command/CWD tracking
- Configurable scrollback, cursor shape, copy-on-select, and more

**Editor**
- Monaco editor embedded via WebKitGTK for full-featured code editing
- Syntax highlighting for 80+ languages
- LSP integration with managed language server installation
- Auto-detected indentation, configurable tab width and spaces/tabs
- Code folding, minimap, bracket pair colorization, indent guides
- Git diff gutter showing added/modified/deleted lines

**Project Navigation**
- File sidebar with lazy-loaded directory tree
- File icons for 50+ languages and file types
- Git status coloring on filenames (added, modified, untracked, etc.)
- Project-wide file name and content search

**Automation**
- Per-file-type settings overrides (tab width, spaces, format command)
- Format-on-save with configurable formatter per file type
- Custom commands-on-save with file pattern matching
- Custom keybindings

**Interface**
- Tabbed interface powered by libadwaita TabView
- Command palette for quick access to actions
- Six built-in color themes: Kanagawa, Nord, Gruvbox, Tokyo Night, Catppuccin Mocha, Rose Pine
- Settings UI with live-updating preferences
- Drag-and-drop file opening

## Installation

> Impulse is in active development. Packaged releases are not yet available.

### Building from Source

**System dependencies** (Arch/CachyOS):

```bash
sudo pacman -S gtk4 libadwaita vte4 gtksourceview5 webkit2gtk-4.1
```

**Build and run:**

```bash
git clone https://github.com/your-username/impulse.git
cd impulse
cargo build --release
cargo run -p impulse-linux --release
```

**Optional — install managed LSP servers** (for web language support):

```bash
./scripts/install-lsp-servers.sh
```

## Architecture

Impulse is a Rust workspace with three crates:

| Crate | Role |
|-------|------|
| `impulse-core` | Platform-agnostic backend: PTY management, shell integration, filesystem, search, git, LSP |
| `impulse-editor` | Monaco editor assets and WebView communication protocol |
| `impulse-linux` | GTK4/libadwaita frontend: window, tabs, terminal, sidebar, settings, themes |

Dependency direction is strictly one-way: `impulse-linux` depends on `impulse-core` and `impulse-editor`, never the reverse.

## License

[GPLv3](LICENSE)
