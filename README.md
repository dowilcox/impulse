<p align="center">
  <img src="assets/impulse-logo.svg" width="120" alt="Impulse logo">
</p>

<h1 align="center">Impulse</h1>

<p align="center">
  A terminal-first development environment built with Rust.
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
  <img src="assets/screenshot.png" width="800" alt="Impulse screenshot">
</p>

Impulse combines a terminal emulator with a Monaco-powered code editor in a modern tabbed interface. It's designed for developers who live in the terminal but want integrated editing, file navigation, and project awareness without leaving their workflow.

## Features

**Terminal**
- Terminal emulator with shell integration (bash, zsh, fish)
- Horizontal and vertical terminal splitting
- OSC 133/7 escape sequence support for prompt/command/CWD tracking
- Configurable scrollback, cursor shape, copy-on-select, and more

**Editor**
- Monaco editor for full-featured code editing
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
- Per-file-type indentation overrides (tab width, spaces/tabs)
- Commands-on-save with file pattern matching and optional file reload (for formatters)
- Custom keybindings that run shell commands

**Interface**
- Tabbed interface with command palette
- Seven built-in color themes: Kanagawa, Nord, Gruvbox, Tokyo Night, Tokyo Night Storm, Catppuccin Mocha, Rose Pine
- Settings UI with live-updating preferences for editor, terminal, appearance, automation, and keybindings
- Full keybinding visibility and customization UI — click any shortcut to rebind it
- Drag-and-drop file opening

## Platform Support

| Platform | Status |
|----------|--------|
| Linux    | Available (GTK4 / libadwaita) |
| macOS    | In development |

## Installation

Download the latest package for your distro from [GitHub Releases](https://github.com/your-username/impulse/releases).

<details>
<summary><strong>Arch / CachyOS / Manjaro</strong></summary>

```bash
sudo pacman -U impulse-0.1.0-1-x86_64.pkg.tar.zst
```

</details>

<details>
<summary><strong>Debian / Ubuntu</strong></summary>

```bash
sudo dpkg -i impulse_0.1.0-1_amd64.deb
sudo apt install -f   # install any missing dependencies
```

</details>

<details>
<summary><strong>Fedora / RHEL / openSUSE</strong></summary>

```bash
sudo rpm -i impulse-0.1.0-1.x86_64.rpm
```

</details>

### Building from Source

Impulse requires [Rust](https://rustup.rs/) and platform-specific system libraries.

<details>
<summary><strong>Linux (Arch / CachyOS)</strong></summary>

```bash
sudo pacman -S gtk4 libadwaita vte4 gtksourceview5 webkit2gtk-4.1
```

```bash
git clone https://github.com/your-username/impulse.git
cd impulse
cargo build --release -p impulse-linux
cargo run --release -p impulse-linux
```

</details>

<details>
<summary><strong>Linux (Debian / Ubuntu)</strong></summary>

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libgtksourceview-5-dev libwebkitgtk-6.0-dev
```

```bash
git clone https://github.com/your-username/impulse.git
cd impulse
cargo build --release -p impulse-linux
cargo run --release -p impulse-linux
```

</details>

**Optional — install managed LSP servers** (for web language support):

```bash
./scripts/install-lsp-servers.sh
```

## Architecture

Impulse is a Rust workspace. Platform-agnostic logic lives in shared crates, with native frontends per platform.

| Crate | Role |
|-------|------|
| `impulse-core` | Platform-agnostic backend: PTY management, shell integration, filesystem, search, git, LSP |
| `impulse-editor` | Monaco editor assets and WebView communication protocol |
| `impulse-linux` | Linux frontend (GTK4 / libadwaita) |
| `impulse-macos` | macOS frontend (in development) |

Dependency direction is strictly one-way: frontends depend on `impulse-core` and `impulse-editor`, never the reverse.

## Releasing

The release script tags a version, builds a release binary, and produces distribution packages:

```bash
./scripts/release.sh 0.1.0           # tag + build + package locally
./scripts/release.sh 0.1.0 --push    # also push tag and create GitHub release
```

This produces the following in `dist/`:

| Format | Target |
|--------|--------|
| `.deb` | Debian, Ubuntu |
| `.rpm` | Fedora, RHEL, openSUSE |
| `.pkg.tar.zst` | Arch, CachyOS, Manjaro (requires `makepkg`) |
| `SHA256SUMS` | Checksums for all packages |

The script automatically bumps the version in all `Cargo.toml` files, creates an annotated git tag, and installs `cargo-deb` / `cargo-generate-rpm` if needed.

## License

[GPLv3](LICENSE)
