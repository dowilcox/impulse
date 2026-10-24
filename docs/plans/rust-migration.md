# Full Rust migration (exploration)

Status: exploring, on the `rust` branch. Nothing here ships until the
Rust app passes the parity gate at the end.

Goal: replace every line of Swift with Rust, UI included, while the app
looks and behaves one to one with the Swift app: same pixels, same
keybindings, same files on disk, same `impulse` CLI and control socket, same
`settings.json`, same session restore files. A user upgrading must not be able
to tell, except that it is faster to build on any OS and the logic is testable
on Linux.

## Where we start

| Target | Swift lines | Swift tests | Notes |
|---|---:|---:|---|
| ImpulseApp | 55,180 | 96 | AppKit + 55 SwiftUI files (~24k lines), WKWebView, CoreText |
| ImpulseKit | 13,371 | 446 | Foundation-only logic; most of it began as Rust |
| ImpulseGit | 6,468 | 156 | libgit2 reads, `git` CLI writes |
| ImpulseLSP | 3,122 | 68 | process + JSON-RPC framing |
| ImpulseProtocol | 1,503 | (in Kit) | control socket, MCP server |
| ImpulseCLI | 153 | | thin |
| impulse-terminal (Rust) | | cargo | stays as is |
| `web/` (JS) | 2,497 | | Monaco glue, stays as is |

About 80k lines of Swift. Much of ImpulseKit, ImpulseGit and ImpulseLSP is a
port of Rust that still exists at tag `v0.29.0` (`impulse-core`,
`impulse-editor`), and the golden fixtures were generated from that Rust, so
those modules can be revived rather than rewritten, then brought up to date
with what the Swift gained since (the Swift unit tests say what).

## The decision that sets everything else: the UI toolkit

"One to one" rules out anything that draws its own widgets or its own text
input. Impulse leans on AppKit for things a custom toolkit would have to fake:
`NSTextView` (the `CommandEditor`, with IME, dictation, services, spell check),
`NSTableView` (Review, with group rows and row heights), `WKWebView` (Monaco),
`NSOpenPanel`/`NSSavePanel`, the menu bar, `NSAccessibility`, window
tabbing and the transparent titlebar.

| Option | One to one? | Verdict |
|---|---|---|
| **AppKit from Rust via `objc2`** (`objc2-app-kit`, `objc2-foundation`, `objc2-web-kit`, `objc2-core-text`, `objc2-quartz-core`, `objc2-user-notifications`) | Yes: the same AppKit classes the Swift app uses, so the same pixels, text input and accessibility | **Recommended** |
| GPUI (Zed's toolkit) | No: custom text input and widgets; no embedded `WKWebView`, so Monaco would have to go | Rejected |
| Tauri / webview UI | No: the whole chrome becomes HTML | Rejected |
| egui / iced / Slint / Xilem | No: immediate or custom-drawn, non-native text and menus | Rejected |

The cost of `objc2`: SwiftUI cannot be called from Rust. The ~24k lines of
SwiftUI views (`WorkbenchView`, sidebar sections, palette, settings, agent
views, Review headers and composer, chips, toasts) become AppKit views.
Most of Impulse's chrome is already custom-drawn from `ChromePalette` and
the theme, not stock SwiftUI controls, so these become `NSView` subclasses
(`define_class!`) drawing with the same metrics, fonts and colors. SwiftUI
animations (`.animation`, `.transition`) become `NSAnimationContext` /
Core Animation with the same curves and durations.

`@Observable WindowModel` becomes a plain Rust struct owned by the window
controller, with change notification by explicit dirty flags per snapshot
(`TabDisplayInfo`, `WorkspaceInfo`, `AgentSummary` are already Equatable and
assigned only on change, so the diffing model carries over directly).

## Target layout

```
Cargo.toml                workspace
impulse-terminal/         unchanged
impulse-kit/              ImpulseKit (pure logic, any OS)        [started]
impulse-git/              ImpulseGit (git2 vendored, no network features; git CLI writes)
impulse-lsp/              ImpulseLSP
impulse-protocol/         ImpulseProtocol (control socket, MCP)
impulse-cli/              the `impulse` binary
impulse-app/              the macOS app (objc2), the only macOS-only crate
impulse-macos/web/        Monaco glue, kept byte for byte
impulse-ffi/              deleted at cut-over (the app links impulse-terminal directly)
```

Everything except `impulse-app` builds and tests on Linux, which is a real
gain: CI for 85% of the code no longer needs a Mac.

## Dependency map

| Swift today | Rust |
|---|---|
| Clibgit2 (vendored 1.9.7, no transports) | `git2` with `vendored-libgit2`, `default-features = false` (no https/ssh), pinned to the same libgit2 |
| `git` CLI writes, `GitCLI.restrictedEnvironment`, `-c log.showSignature=false` | `std::process::Command`, same arguments and environment |
| TOMLKit | `toml` (+ `toml_edit` where `ProjectSettingsFile` edits in place) |
| swift-cmark gfm (safe mode) | the same cmark-gfm C sources through `cc`, so preview HTML stays byte identical (`comrak` would differ in output) |
| SQLite (history DB) | `rusqlite` (bundled), same schema and file |
| JavaScriptCore + highlight.js (Review colors) | `objc2-java-script-core`, same highlight.js, so tokens match |
| CryptoKit | `sha2` |
| FSEvents / DispatchSource | `objc2-core-services` FSEvents (or `notify` with its fsevent backend) and `kqueue` |
| UserNotifications | `objc2-user-notifications` |
| Carbon (input source, layout) | TIS calls through `objc2` / `core-foundation` |
| `os` log | `oslog` crate behind `log` |
| CoreText renderer, `CVDisplayLink` | `objc2-core-text`, `objc2-core-graphics`, `CADisplayLink`/`CVDisplayLink` |
| Swift Testing | `cargo test` |

## Phases

Each phase leaves the shipped Swift app untouched and working.

### Phase 1: logic crates, with parity (any OS)

Port ImpulseKit, ImpulseGit, ImpulseLSP and ImpulseProtocol. For each module:

1. Revive the `v0.29.0` Rust where it exists; port the Swift where it doesn't.
2. Pass the existing golden fixtures (read in place from
   `impulse-macos/Tests/*/Fixtures/`, never copied, never regenerated).
3. Port that module's Swift unit tests, which encode everything added since
   the Rust era. A Swift test with no Rust twin means the port is not done.
4. Git: keep the `git` CLI as the test oracle, as today.

Started: `impulse-kit` holds `glob`, `close_risk` and `shell_parser`,
revived from `v0.29.0`; they pass `glob.json`, `close_risk.json` and
`shell_parser.json` and the Rust unit tests that mirror the Swift ones
(`cargo test -p impulse-kit`).

Modules with no fixture today get one before they are ported: a small
Swift test that dumps inputs and outputs as JSON. On this branch the Swift
app is the spec, so for these the fixture comes from Swift, generated once
and then frozen like the others.

### Phase 2: the CLI (first shippable piece)

`impulse-cli` speaks the same control-socket JSON as `ImpulseProtocol`, so a
Rust `impulse` binary (and `impulse mcp`) can ship inside the Swift app's
bundle with no UI change. The smoke scenarios that use the CLI and the MCP
server prove it. This is the first real users of the Rust crates.

### Phase 3: the app shell in Rust

`impulse-app`: `NSApplication` delegate, menus from `CommandRegistry`,
`MainWindowController` and its extensions, the transparent titlebar with the
empty toolbar, `WorkbenchView`'s chrome, tabs, `LayoutTree` panes, the control
server, session restore v2 (same JSON, same scrollback files), settings and
keybindings. Then the terminal: `TerminalRenderer` (4.2k lines of CoreText)
ported call for call, `GridBufferReader` replaced by reading
`impulse-terminal`'s grid directly, `KeyEncoder` / `KittyKeyboard`,
`CommandEditor` as an `NSTextView` subclass.

Exit: a terminal-only Impulse that passes the terminal smoke scenarios.

### Phase 4: surfaces

In order of how much they are used: editor (WKWebView, `web/` unchanged,
`EditorWebViewPool`), file tree, palette, search, Review and History
(`NSTableView`, `DiffLineCellView`), git panels and tasks (Finish, landing,
cleanup), agents (hooks, inbox, Agent Integration sheet, opencode plugin
unchanged), settings tab, project setup, notifications, toasts.

### Phase 5: parity gate and cut-over

The headless snapshot mode (`MainWindowController+Debug`, `DebugSnapshot`)
is ported first in Phase 3, keeping its `dump=` format, because it is how
"one to one" gets measured:

- `scripts/smoke/run.py` runs every scenario against both builds; dumps
  must match.
- `scripts/docs/capture.py` renders every docs shot from both builds; a
  pixel diff over a small threshold fails the gate.
- Manual pass: IME (Japanese, Chinese), VoiceOver, dictation, trackpad
  gestures, full-screen, multiple displays, Stage Manager.

When the gate is green: `build.sh` builds the Rust app, the Swift targets and
`impulse-ffi` are deleted, CLAUDE.md and AGENTS.md are rewritten for the new
layout.

## Risks

- **SwiftUI to AppKit by hand** is the largest and least mechanical part:
  layout that SwiftUI did implicitly (stack spacing, text truncation, hover
  states, focus rings) has to be matched exactly. The pixel diff is the
  safety net.
- **`objc2` ergonomics**: subclassing with `define_class!`, delegates, blocks,
  `MainThreadMarker`, and retain cycles between Rust closures and Objective-C
  objects. Settle the patterns in Phase 3 with the terminal before the bulk
  of the views.
- **New macOS APIs land in Swift first.** Anything Apple adds (as with
  Tahoe's chrome) arrives in `objc2` bindings after a delay, generated from
  the SDK headers; Swift-only APIs never arrive.
- **Build**: no SwiftPM; `build.sh` keeps its job (libgit2 is handled by
  `git2`'s build script) and bundles a cargo binary. Signing and notarization
  are unchanged.
- **This reverses the 2026 rewrite.** That rewrite moved off Rust for a
  reason; the gains here are one language, Linux CI for the logic, and the
  terminal core in the same process without an FFI. If the UI port proves too
  costly, Phase 1 and 2 still stand on their own: a Rust core with a thin
  Swift UI over one FFI is the fallback.

## Size

Logic ports run close to 1:1 in lines. Declarative SwiftUI written as
imperative AppKit grows, often 1.5 to 2 times. Expect roughly 90k to 110k
lines of Rust for the whole app.
