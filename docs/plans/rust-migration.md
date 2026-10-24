# Full Rust migration: one Impulse for macOS and Linux

Status: exploring, on the `rust` branch. Nothing here ships until the Rust
app passes the parity gate at the end.

## Goal

Replace all of the Swift, UI included, with Rust, so that **one codebase
builds for macOS and Linux and works exactly the same on both**, and looks
one to one with today's Mac app. Same layout, same chrome, same keyboard
model, same `settings.json`, same files on disk, same `impulse` CLI and
control socket, same agent integration.

"Exactly the same" sets the toolkit: Impulse has to draw its own UI, with
its own fonts, on both platforms. A toolkit that wraps native widgets (AppKit
on the Mac, GTK on Linux) would look and behave differently on each.

![The spike on Linux](images/rust-spike-linux.png)

*The spike (`impulse-app`) running on Linux: bash with Impulse's shell
integration, drawn by GPUI from the `impulse-terminal` grid, with command
blocks (the failed `ls` gets the red wash, stripe and `✗ 2` chip), and the
chrome derived from the Nord theme by the ported `ChromePalette`.*

## Where we start

| Target | Swift lines | Swift tests | Notes |
|---|---:|---:|---|
| ImpulseApp | 55,180 | 96 | AppKit + 55 SwiftUI files (~24k lines), WKWebView, CoreText |
| ImpulseKit | 13,371 | 446 | Foundation-only logic; much of it began as Rust |
| ImpulseGit | 6,468 | 156 | libgit2 reads, `git` CLI writes |
| ImpulseLSP | 3,122 | 68 | process + JSON-RPC framing |
| ImpulseProtocol | 1,503 | (in Kit) | control socket, MCP server |
| ImpulseCLI | 153 | | thin |
| impulse-terminal (Rust) | | 112 | stays; now builds and passes on Linux |
| `web/` (JS) | 2,497 | | Monaco glue; goes away (see Editor) |

Much of ImpulseKit, ImpulseGit and ImpulseLSP is a port of Rust that still
exists at tag `v0.29.0` (`impulse-core`, `impulse-editor`, which also ran on
Linux with GTK), and the golden fixtures were generated from that Rust. Those
modules are revived, then brought up to date with what the Swift gained since
(the Swift unit tests say what).

## Toolkit: GPUI

GPUI is the toolkit Zed is built with (Apache-2.0). It draws everything on
the GPU itself: Metal on macOS, Vulkan (wgpu) on Linux, X11 and Wayland.
Text is shaped by CoreText on the Mac and cosmic-text on Linux, with the
same fonts if we bundle them.

| Option | Same on Mac and Linux? | Notes | Verdict |
|---|---|---|---|
| **GPUI** | Yes: draws everything | Proven on exactly our workload: Zed's terminal is `alacritty_terminal` + GPUI, and its editor is a full code editor in GPUI. IME on both platforms. | **Chosen** |
| AppKit via `objc2` | No: Mac only | Was the earlier recommendation, before Linux was a goal | Rejected |
| Slint | Mostly | Declarative `.slint` language; has AccessKit accessibility; no proven terminal or code editor | Fallback |
| Tauri / webview | Yes | The UI becomes HTML; Linux WebKitGTK is slow and differs from WKWebView | Rejected |
| iced, egui, Floem, Xilem | Yes | Less proven for a terminal and editor at this scale; egui is immediate mode | Rejected |

How GPUI is used:

- Pinned to a Zed release tag from Zed's git repository (Zed publishes GPUI
  there; the crates.io `gpui` 0.2.2 is a year old and no longer builds against
  current `libc`). The workspace mirrors Zed's `[patch.crates-io]` entries
  for GPUI's dependencies, and the lockfile was seeded from Zed's, so GPUI's
  tree resolves the way Zed tests it. Upgrading GPUI means moving the tag and
  re-seeding.
- GPUI is pre-1.0 and changes between Zed releases. All Impulse views go
  through a small set of our own components (rows, chips, tabs, buttons,
  menus, the text field), so an API change is fixed in one place.

## What changes so the two platforms match

### Editor: Monaco goes

Monaco runs in a web view, and a web view can't be embedded the same way on
both platforms in a GPU-drawn window: wry's child web views work on macOS and
X11 but not Wayland, where it needs a GTK container. So the editor becomes
native GPUI, drawn like everything else:

- **Recommended: build on Zed's editor.** Zed's `editor`, `language`,
  `multi_buffer` and `vim` crates are GPL-3.0, compatible with Impulse's
  GPL-3.0-only. They bring multi-cursor, tree-sitter highlighting, LSP
  completion, hover and diagnostics, find and replace, and a Vim mode. The
  cost is the dependency weight (they pull in much of Zed's project model),
  so the first task is a spike that embeds Zed's editor in `impulse-app` and
  measures what comes with it.
- Fallback: an editor of our own on `ropey` + tree-sitter + GPUI text
  layout. Smaller, slower to reach Monaco's features.

The editor keeps today's behavior where it is Impulse's own (saving, format
on save, dirty marks, reload in place, the size limit, the LSP wiring, git
gutter, conflict actions), and Monaco's keybindings and colors (the theme's
syntax colors already exist per theme; `themeToMonaco` is the spec for how
they map). It will not be pixel-identical to Monaco: this is the one surface
where "one to one" means the same features, keys and colors, not the same
pixels.

Syntax colors in Review and History move from highlight.js (in
JavaScriptCore) to the same tree-sitter grammars the editor uses, so the
editor and Review finally color code the same way.

Markdown preview: rendered natively from cmark-gfm's AST (raw HTML is
already elided, so nothing needs a browser).

### Fonts: bundled, the same everywhere

The chrome uses SF Pro today, which Apple doesn't license for Linux. Both
platforms use the bundled Inter (already in `vendor/fonts`) for the chrome
and JetBrains Mono for the terminal and editor. On the Mac this is a small,
deliberate change from SF Pro, made so the two match. Glyph rasterization
still differs slightly (CoreText vs swash); layout and metrics match.

### Window chrome

The titlebar band is drawn by Impulse on both (as today on the Mac, under a
transparent titlebar). On macOS the traffic lights keep their 78pt inset; on
Linux the same band draws its own minimize, maximize and close buttons
(client-side decorations on Wayland and X11).

### Keyboard

One keymap, with `secondary` meaning ⌘ on macOS and Ctrl on Linux (GPUI's
own model). In a terminal, Ctrl belongs to the shell, so on Linux terminal
focus uses Ctrl+Shift for Impulse's shortcuts, as every Linux terminal does.
Option-as-Meta becomes Alt-as-Meta. The shortcut list in the docs shows each
platform's keys.

### Menus

macOS keeps the menu bar, built from `CommandRegistry`. Linux has no global
menu: a menu button in the titlebar opens the same menus. The command
palette has every command on both.

### Accessibility

This is the biggest gap. GPUI has no accessibility tree today (no AccessKit
at the pinned tag), and Impulse supports VoiceOver now (`docs/accessibility.md`):
terminal text, input fields, labels, agent announcements. The plan:

- Add an accessibility tree to our components with AccessKit, through its
  macOS (NSAccessibility) and Unix (AT-SPI, for Orca) adapters, attached to
  GPUI's window handle. Keep it general enough to upstream to GPUI.
- The cut-over gate includes a VoiceOver and Orca pass over everything
  `docs/accessibility.md` promises. Shipping without it would be a regression.
- Reduce Motion and Increase Contrast come from the platform (NSWorkspace on
  the Mac, the settings portal on Linux) into `ChromePalette::new`.

### Platform services

The few platform bits sit behind one `platform` module per OS, with the same
interface:

| Need | macOS | Linux |
|---|---|---|
| Foreground process, agent detection, port scan | libproc | `/proc` |
| Control-socket caller pid (`LOCAL_PEERPID`) | `LOCAL_PEERPID` | `SO_PEERCRED` |
| File watching (file tree, settings, git) | FSEvents (`notify`) | inotify (`notify`) |
| Notifications | UserNotifications | freedesktop notifications |
| Open/save panels | GPUI (NSOpenPanel) | GPUI (xdg portal) |
| Reveal in file manager | `open -R` | `org.freedesktop.FileManager1` |
| Login shell | `dscl` | `getent passwd` |
| Settings, themes, sessions | `~/Library/Application Support/impulse` (as today) | `$XDG_CONFIG_HOME/impulse`, `$XDG_DATA_HOME/impulse` |
| Packaging | `.app` + `.dmg`, signed and notarized | AppImage and `.deb` (Flatpak later) |

The terminal core needed two of these already: `arc4random_buf` (macOS) is
`getrandom` on Linux, and the shell's SIGHUP-then-SIGKILL shutdown now runs on
both (`impulse-terminal/src/backend.rs`).

## Dependency map

| Swift today | Rust |
|---|---|
| AppKit + SwiftUI | GPUI |
| CoreText renderer, `CVDisplayLink` | GPUI text system and frame loop |
| WKWebView + Monaco | native editor (see above) |
| JavaScriptCore + highlight.js | tree-sitter |
| Clibgit2 (vendored 1.9.7, no transports) | `git2` with `vendored-libgit2`, no default features (no https/ssh) |
| `git` CLI writes, restricted environment | `std::process::Command`, same arguments and environment |
| TOMLKit | `toml` (+ `toml_edit` for `ProjectSettingsFile`) |
| swift-cmark gfm | cmark-gfm C sources via `cc` (same parser, same output) |
| SQLite (history DB) | `rusqlite` (bundled), same schema and file |
| CryptoKit | `sha2` |
| `os` log | `log` + `oslog` on the Mac, journald/stderr on Linux |
| Swift Testing | `cargo test` |

## Layout

```
impulse-terminal/   terminal emulation (unchanged; macOS + Linux)
impulse-kit/        pure logic: themes, chrome palette, parsers, …   [started]
impulse-git/        git2 reads + git CLI writes
impulse-lsp/        language-server client
impulse-protocol/   control socket + MCP server
impulse-cli/        the `impulse` binary
impulse-platform/   the per-OS services above
impulse-app/        the GPUI app                                      [spike]
```

`impulse-ffi`, the Swift package and `impulse-macos/web/` are deleted at
cut-over.

## Phases

The shipped Swift app keeps working through every phase.

### 1. Logic crates, with parity

Port ImpulseKit, ImpulseGit, ImpulseLSP and ImpulseProtocol. For each module:

1. Revive the `v0.29.0` Rust where it exists; port the Swift where it doesn't.
2. Pass the golden fixtures, read in place from `impulse-macos/Tests/*/Fixtures/`
   (never copied, never regenerated).
3. Port that module's Swift unit tests, which encode everything added since the
   Rust era. A Swift test without a Rust twin means the port isn't done.
4. Git: the `git` CLI stays the test oracle.

Modules with no fixture get one first: a small Swift test that dumps inputs
and outputs as JSON, generated once and frozen like the others.

All of this runs in Linux CI.

### 2. The CLI

`impulse-cli` speaks the same control-socket JSON, so the Rust `impulse` (and
`impulse mcp`) can replace the Swift one inside today's app bundle, proven by
the smoke scenarios that use the CLI and the MCP server.

### 3. The app shell and terminal

Window, titlebar, tabs and panes (`LayoutTree`), workspaces, the left dock,
the status bar, the control server, settings and keybindings, session restore
(same JSON and scrollback files, so a Mac user's session comes back in the
Rust app). The terminal view ported from `TerminalRenderer.swift` decoration
by decoration (selection, search, hyperlinks, sticky headers, block toolbar),
the input bar with completion, Kitty keyboard, and the accessibility tree for
the terminal. Plus the headless snapshot mode (`DebugSnapshot`) with its
`dump=` format, because that is how parity gets measured. GPUI only renders
offscreen on the Mac today (`render_to_image`, Metal); Linux needs the same on
wgpu, which is a small addition to contribute upstream.

Exit: a terminal-only Impulse on both platforms that passes the terminal smoke
scenarios on both.

### 4. Surfaces

Editor (the Zed-editor spike first), file tree, palette, search, Review and
History, git panels, tasks (New, Finish, landing, cleanup), agents (hooks,
inbox, Agent Integration, the opencode plugin), settings, project setup,
notifications, toasts.

### 5. Parity gate and cut-over

- `scripts/smoke/run.py` runs every scenario against the Swift app and against
  the Rust app on macOS and on Linux; the dumps must match.
- `scripts/docs/capture.py` renders every docs shot from the Rust app on both
  platforms and from the Swift app; Mac against Linux must match within a
  small pixel threshold (rasterization), Rust against Swift is reviewed by eye
  (fonts change from SF Pro to Inter).
- Manual pass on both: IME (Japanese, Chinese), VoiceOver and Orca,
  trackpad and touchpad gestures, HiDPI and fractional scaling, full screen,
  several displays.

When the gate is green, `build.sh` builds the Rust app, Linux packages are
added to `scripts/release.sh`, the Swift targets are deleted, and CLAUDE.md,
AGENTS.md and `docs/` are rewritten for both platforms.

## Done so far

- `impulse-terminal` builds and passes all its tests on Linux, including the
  tests that run a real shell in a PTY.
- `impulse-kit`: `glob`, `close_risk`, `shell_parser` and `theme` revived from
  `v0.29.0`, and `chrome` (`ChromePalette` and `Metrics`) ported from the
  Swift. They pass `glob.json`, `close_risk.json`, `shell_parser.json` and all
  19 `themes/*.theme.json`, plus the Rust twins of the Swift unit tests.
- `impulse-app`: the GPUI spike in the screenshot. Titlebar, left dock, a live
  bash with shell integration and command blocks, context chips, the input bar
  and the status bar, from any built-in theme (`--theme`). Builds on Linux;
  not yet built on a Mac.
- `scripts/spike/linux-shot.sh` screenshots it under Xvfb with software
  Vulkan, so the app can be checked on Linux CI without a GPU.

## Risks

- **Accessibility** (above): real work, and a regression if skipped.
- **The editor**: Zed's editor may bring more of Zed than we want; the
  fallback is months of editor work.
- **GPUI churn**: pre-1.0, released only as Zed's repository. Pinning plus our
  own component layer contains it.
- **SwiftUI to GPUI by hand**: GPUI's layout is flexbox (Taffy), close to how
  the SwiftUI views stack, but spacing, truncation, hover and focus states have
  to be matched one view at a time. The pixel diff is the safety net.
- **This reverses the 2026 rewrite**, which left Rust and Linux for the Mac.
  What's different now: one toolkit draws both platforms, so there is no
  second frontend to keep in step (the GTK app was a separate frontend).
- **Size**: logic ports run about 1:1 in lines; the views are smaller in GPUI's
  builder style than in AppKit, larger than SwiftUI. Expect 80k to 100k lines
  of Rust, less if Zed's editor is used.
