# Design: Impulse Workbench — the terminal IDE redesign

**Date:** 2026-10-05
**Scope:** Full redesign of the macOS app: window chrome, layout model, terminal, git, change review, agent workflows, editor depth.
**Status:** Approved 2026-10-05. Decisions: D2/D3 workspaces sidebar + titlebar tabs; D5 no AI features at all; D7 Lucide icons; worktrees default to `../<repo>.worktrees/<branch>`; UI font stays SF Pro Text.
**Companion plan:** `docs/superpowers/plans/2026-10-05-terminal-ide-redesign.md`

---

## 1. Why

Impulse is a terminal emulator with an editor bolted on, wearing native macOS chrome. It now competes in a category that changed shape in 2025–2026:

- **Warp** became an "agentic development environment". It has vertical tabs with per-tab branch, diff and agent status. Its docked code-review pane sends inline comments to any running CLI agent. It also has a rich-input overlay for TUI agents, tab configs, and git commit/push/PR from the review pane.
- **cmux, Conductor, Superset, Claude Code Desktop and the Codex app** converged on one model:
  - A vertical sidebar of **workspaces** (one git worktree each), showing branch, PR, ports and agent status.
  - Terminals and diff panes inside each workspace.
  - Layered notifications.
  - A diff review whose comments flow back to the agent.
- **Ghostty, kitty, iTerm2, Zellij and Wave** all added vertical tabs, progress reporting (OSC 9;4), agent/session status (iTerm2 OSC 21337) and command-finished notifications.
- **Zed, VS Code, GitHub, Codex and Tower** converged on git review that is:
  - one scrolling multi-file diff;
  - hunk- and line-level staging;
  - scoped (unstaged / staged / branch / commit / last agent turn);
  - undoable (snapshot refs, ⌘Z).

Impulse already has building blocks most of these products had to build first:

- OSC 133 command blocks.
- A Warp-model input bar.
- libgit2 with word-level diff spans.
- Monaco, an LSP client, a review renderer and a command palette.

What it lacks:

- **A layout model.** There are no splits (they were removed), no workspaces and no docks.
- **A real git layer.** There is no staged/unstaged split, no hunk staging and no history.
- **Any awareness of the agents its users run.**
- **A chrome that can grow.** NavigationSplitView, glass toolbar buttons, HUD palettes and NSAlert dialogs make up most of the "Apple-app" look. They also cause many of the focus and layout hacks found in the code survey.

**Positioning.** _Impulse is the terminal-first workbench for working with coding agents._

- It is local-first, needs no account and has no telemetry.
- It hosts the CLI agents people already use (Claude Code, Codex, Gemini CLI, OpenCode, Aider, …).
- It shows what they are doing, and makes reviewing and landing their changes fast and safe.

It is not another chat UI. Warp's backlash, from users who said "I thought it used to be a terminal?", is the cautionary tale.

## 2. Design principles

1. **The terminal is the center.** Everything else (files, git, review, editor) orbits the terminal pane. A new window opens to a working shell in under 500 ms.
2. **Git is a workflow, not a tab.** Repo state is always visible: branch, ahead/behind, dirty count, operation in progress. Review → stage → commit → push → PR is one continuous path. Every destructive git action is undoable.
3. **Agent-aware, agent-agnostic.** Impulse detects and hosts CLI agents and gives them status, notifications, checkpoints, and a review loop that sends comments back. It does not ship its own LLM, keys or account, and nothing in Impulse calls an AI tool itself.
4. **Dense, keyboard-first, custom chrome.** Flat themed surfaces, hairline borders, compact rows, inline key hints, a command palette for everything. **Native where it matters:**
   - menu bar;
   - traffic lights and full screen;
   - text input and IME;
   - Services;
   - accessibility;
   - notifications.
5. **One model, many views.** A single observable source of truth per concept feeds every surface that shows it: one `GitRepositoryState` per repo, one `SettingsStore`, one `CommandRegistry`. No more hand-synced copies or per-view refresh triggers.
6. **Disruptive behavior is opt-in.** The redesign changes the default layout once, at a major version. After that, new agent-related behavior ships off-by-default or behind a visible toggle. There is no natural-language detection hijacking the shell input.
7. **Performance budgets are features.** See §11.

## 3. Decisions (recommendations; approve or override)

| #   | Decision                             | Recommendation                                                                                                                                                                                                                                                                | Alternatives considered                                                                                                                                                                                                                   |
| --- | ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | UI technology                        | **AppKit-owned workbench layout + SwiftUI panels + an in-house design system.** Terminal stays CoreText (Metal later), editor stays Monaco.                                                                                                                                   | (a) Custom GPU UI framework like Warp/Zed: years of work, and both have weak accessibility (Warp #11160, Zed AccessKit still experimental). (b) Whole UI in one WKWebView like Raycast 2.0: fights the native terminal for focus and IME. |
| D2  | Window model                         | **A window holds many workspaces.** The sidebar groups them by repo, and each workspace has its own tabs.                                                                                                                                                                     | A window per project (VS Code). Simpler, but it loses the cross-project "who needs me" view that agent users want.                                                                                                                        |
| D3  | Tab placement                        | **Workspace sidebar on the left, the active workspace's tabs as a strip in the titlebar.** Vertical tabs-in-sidebar is a setting.                                                                                                                                             | Vertical tabs only (Warp 2026): polarizing; the default flip caused Warp #9160.                                                                                                                                                           |
| D4  | Git write path                       | **libgit2 reads; the `git` CLI writes.** Reads cover status, diff, blame, log, graph and refs. Writes cover commit, checkout/switch, stash, merge/rebase/cherry-pick/revert/reset, worktree add/remove, fetch/pull/push, and applying staging patches (`git apply --cached`). | All-libgit2: skips hooks, GPG/SSH signing and LFS filters, and needs network transports and credential helpers.                                                                                                                           |
| D5  | AI                                   | **No AI features in the app (decided 2026-10-05).** Impulse only hosts CLI agents in terminals; nothing in Impulse calls an AI tool. An ACP client is a later option.                                                                                           | A built-in agent, which needs accounts, keys and churn.                                                                                                                                                                                   |
| D6  | Input model                          | **Keep the Warp-style input editor as default, rebuilt as a multi-line editor. Make "Classic (shell-native prompt)" a first-class mode that works.**                                                                                                                          | Input editor only. Classic mode is broken today: the grid refuses focus when the context bar is off.                                                                                                                                      |
| D7  | Icons                                | **One stroke icon set for chrome (Lucide, ISC license)** rendered as template images. Material file icons stay for the file tree.                                                                                                                                             | SF Symbols, which carry most of the "Apple app" feel.                                                                                                                                                                                     |
| D8  | Splits                               | **Bring splits back, generalized.** A tab is a split tree of panes. A pane hosts one surface, or a stack of editor surfaces with a mini tab header.                                                                                                                           | Terminal-only splits (removed in 2764560 for being half-built).                                                                                                                                                                           |
| D9  | Overlays above terminal and WebViews | **One `OverlayHost` built on child `NSPanel`s** for palettes, completions, popovers, toasts and the inbox.                                                                                                                                                                    | SwiftUI overlays, which cannot draw above AppKit-hosted WKWebView or the grid.                                                                                                                                                            |
| D10 | Review renderer                      | **Evolve `review.js`** (custom stacked renderer) into the multi-file review. Use Monaco's `createDiffEditor` only for an editable single-file "deep dive".                                                                                                                    | Pierre diffs (Shiki-based); keep it as a reference. Monaco's experimental multi-file diff editor is not a public API.                                                                                                                     |

## 4. Information architecture

```
App
└─ Window (WorkbenchWindowController)
   ├─ Workspaces (sidebar, grouped by repo)
   │   └─ Workspace  = { root dir (git worktree or plain folder), repo ref, tabs, dock state, env }
   │       └─ Tab    = split tree (LayoutTree) of Panes
   │           └─ Pane = one Surface, or a stack of editor Surfaces (mini tab header)
   │               └─ Surface: Terminal | Editor | Review | History | Commit | Preview(md/svg/img)
   │                           | Browser (later) | Settings | Keybindings
   ├─ Left dock:   [Workspaces section] + [Files | Changes | Search | (History)] tool panel
   ├─ Right dock:  Review (live, docked) | Inbox | Outline
   ├─ Bottom dock: Problems | Ports & Processes | Output (LSP / git command log)
   └─ Status bar
```

- **Workspace.** The unit of context. It is usually a git worktree: the main checkout is a workspace, and each linked worktree is another. Opening a folder creates or focuses its workspace. A home-directory "Scratch" workspace exists for ad-hoc shells. A workspace owns:
  - its file-tree root;
  - its git state;
  - its review scope;
  - its port block;
  - its agent sessions.

  This replaces today's "file-tree root follows the active tab's cwd" behavior, though a terminal that `cd`s elsewhere still shows its own cwd in chips.

- **Tab.** A named layout. Double-click renames it. It has a color and can be pinned (pins already exist). Tabs can join tab groups later.
- **Pane.** A leaf of the split tree. It can be focused, zoomed (⌘⇧↩), moved by dragging its header, and closed. Closing is undoable for 10 seconds (⌘Z).
- **Surface.** A protocol that any content implements:
  - `focus()`, `title`, `icon`, `status`;
  - `closeRisk`;
  - `restoreState` / `snapshotState`;
  - `contextKeys` (used by keybinding "when" clauses);
  - `commands`, which feed the palette.

## 5. Window chrome and layout

### 5.1 Main window

```
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│ ● ● ●  ◧ │ impulse › review-v2  ⎇ ↑2 │ ❯ claude ◐ │ ❯ zsh │ ◇ Hunks.swift │ ＋ │   ⌘P  🔔2  ±3 +63 −2 ◨ │ 38pt
├────────────────┬────────────────────────────────────────────────────────────────┬──────────────┤
│ WORKSPACES   ＋│ ▏❯ claude                                    ◐ working  ⋯ │ ❯ zsh        │ REVIEW  ▾    │
│ ▾ impulse      │ ▏                                                           │              │ 3 files      │
│   ● main     ±0│ ▏ (Claude Code TUI)                                         │ ❯ npm run dev│ +63 −2       │
│   ◐ review-v2 ±3│ ▏                                                          │   ready :5173│ ▾ M review.js│
│   ⏸ osc-fix  ✓ │ ▏                                                           │              │   …          │
│ ▸ dotfiles     │ ▏                                                           │──────────────│              │
│────────────────│ ▏◆ Claude Code · needs input · ⌘I compose · ±+63 −2 · ⟲ 4  │ ~/…/impulse ⎇│              │
│ Files Changes ⌕│                                                             │ ❯ ▌          │              │
│ ▾ Sources      │                                                             │              │              │
├────────────────┴────────────────────────────────────────────────────────────────┴──────────────┤
│ ⎇ review-v2 ↑2↓0 │ ✕0 ⚠3 │ ◐ 1 working · 1 needs input │ ⚡ :5173 │ LSP ● sourcekit │ Ln 12, Col 4 │ 24pt
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

**Titlebar (38pt, one row).** From left to right:

- Traffic lights.
- Left-dock toggle.
- Workspace breadcrumb: `repo › workspace`, then branch with ahead/behind. Clicking opens the workspace/branch switcher.
- The active workspace's tab strip (compact pills, attention dots, progress ring, close on hover; drag to reorder).
- `＋` with a menu: new terminal, new agent session, new worktree task, tab config.
- Spacer, which is a window drag region.
- Palette button.
- Inbox bell with a count.
- Diff-stats pill, which opens the review.
- Right-dock toggle.

**Status bar (24pt, always visible, one per window).**

- Workspace branch and sync state.
- Problems counts, which open the Problems dock.
- Agent summary, e.g. "1 working · 1 needs input", which opens the Inbox.
- Listening ports.
- LSP status dot.
- Editor cursor, language, encoding and indent (only when an editor is focused).
- Update pill.

The status bar is no longer swapped with the terminal input bar. The input moves into each terminal pane (§6.2).

### 5.2 Building it on AppKit (macOS 26)

- **Window.** Keep a real titled `NSWindow`:
  - `.fullSizeContentView`, a transparent titlebar, hidden title;
  - `allowsAutomaticWindowTabbing = false`, because Impulse owns tabs;
  - a full menu bar.
- **Titlebar band** (the first Phase 0 spike, with acceptance criteria in the plan). Prototype in this order:
  1. An empty `NSToolbar` with `.unifiedCompact` to get vertically centered traffic lights at ~38pt, with the chrome view hosted as **one full-width custom-view `NSToolbarItem`**. This keeps the CLAUDE.md rule that toolbar content is native `NSToolbarItem`s.
  2. If (1) fights item sizing or Tahoe glass, draw the chrome as content under the transparent titlebar, with explicit drag regions (`mouseDownCanMoveWindow`) and a traffic-light inset that collapses in full screen.

  Never move the standard window buttons by hand. Keep double-click honoring `AppleActionOnDoubleClick` (existing `ImpulseWindow` logic).

- **Workbench layout.** Replace `NavigationSplitView` with an AppKit `WorkbenchView`: left dock | center | right dock, over a bottom dock and the status bar. Docks are non-sidebar split items with Impulse-drawn backgrounds, so there is no automatic floating glass sidebar. Dock sizes and visibility live in the workspace dock state. This removes these hacks:
  - `currentSidebarWidth()` walking SwiftUI's private `NSSplitView`;
  - the deferred-focus races in NavigationSplitView;
  - the fake `NSWindow.didResizeNotification` in `ContentContainer`.
- **SwiftUI** renders panel contents (lists, the git panel, headers) inside `NSHostingView`s that the workbench owns. The AppKit layer owns geometry and focus.
- **Overlays.** `OverlayHost` generalizes today's `CompletionPanel` / `NonKeyPanel`. It hosts themed child `NSPanel`s for:
  - command palette;
  - completion menus;
  - popovers (branch switcher, diff base menu);
  - toasts with Undo;
  - the inbox;
  - hover cards.

  Panels are non-key unless they need text input, so key-window routing keeps working.

- **Dialogs.** Replace `runModal` `NSAlert`s with themed in-window sheets or inline confirmations. Affected: Go to Line, Rename, Discard, Trash, binary file, LSP install result.
- **Clip every custom draw to bounds.** This is the macOS 26 shared-canvas dirtyRect trap (see memory note `macos-shared-canvas-dirtyrect`).

### 5.3 Design system ("Impulse UI")

- **Tokens** live in `ImpulseKit`, so they are testable, and are resolved per theme:
  - spacing on a 4pt grid;
  - radii of 4, 6 and 8;
  - row heights of 22 (compact), 26 (default) and 30 (comfortable);
  - UI font size 12 or 13 (default SF Pro Text; the family is configurable);
  - metadata in the monospace font at 11.
- **Theme schema.** The TOML gains an optional `[ui]` table:
  - `chrome_bg`, `panel_bg`, `panel_header_bg`, `hairline`;
  - `accent`, `accent_fg`, `focus_ring`, `tab_active_bg`, `tab_inactive_fg`;
  - `badge_*`, `status_working`, `status_attention`, `status_done`, `status_error`;
  - `density`.

  Each derives from the existing palette and semantic seeds when omitted, Warp-style: background, foreground, accent and ANSI produce everything. `surface_style` ("flat"/"card") is folded in.

  Adding optional keys must not change `themeToMonaco` / `themeToMarkdownColors` output. The existing theme fixtures must stay green with no edits. If a derived chrome color is later fed into Monaco, update that fixture deliberately in the same commit.

- **Components** live in `Sources/ImpulseApp/DesignSystem/`:
  - `IconButton`, `Chip`, `SplitButton`, `SegmentedTabs`;
  - `ListRow` (with hover actions and a ⌘K action menu per row, Raycast-style), `SectionHeader` (with count and actions);
  - `Badge`, `StatusDot`, `ProgressRing`, `KeyHint`;
  - `ThemedTextField`, `ThemedTextEditor`, `Toast`;
  - `ConfirmInline`, `EmptyState`, `PaneHeader`.

  A debug-only **Component Gallery** window renders every component in every theme for visual review.

- **Visual language:**
  - flat surfaces, no vibrancy or glass on primary chrome (glass only optional on floating overlays);
  - 1px hairlines;
  - a sidebar a few notches dimmer than content (Linear);
  - the focused pane marked by a 2px accent corner or edge (Warp);
  - optional dimming of inactive panes;
  - inline key hints;
  - semantic colors kept separate from the accent.
- **Accessibility is a requirement:**
  - The tab strip exposes AXTabGroup/AXRadioButton.
  - Sidebar rows are AXOutline rows whose labels include status text, e.g. "Claude: needs input".
  - State changes post `.announcementRequested`.
  - Full Keyboard Access works with focus rings.
  - Reduce Motion, Reduce Transparency and Increase Contrast are honored.
  - The terminal exposes an AXTextArea (value, selected range, line for index), the gap Warp is criticized for.

## 6. Terminal

### 6.1 Panes and lifecycle

- **Splits:**
  - ⌘D splits right, ⌘⇧D splits down.
  - ⌥⌘arrows focus the neighboring pane; ⌘[ and ⌘] cycle panes.
  - ⌘⇧↩ zooms a pane.
  - Drag the pane header to rearrange, or drop it on the tab strip to pop it out.
  - Equalize splits.
  - ⌘Z undoes closing a pane or tab (Ghostty).
- **`TerminalSessionHub`.** Event polling is decoupled from view visibility. Today hidden tabs are `removeFromSuperview`d and stop polling, so bells, OSC 9 and command-finished events from background tabs wait until you look at them. Instead:
  - every live terminal is pumped at a low rate (or woken by the PTY reader's coalesced wakeup);
  - only visible panes render;
  - the hub owns per-terminal metadata: cwd, foreground process, agent state, progress, last block.
- **Idle cost.** Per-terminal 30–60 Hz timers are replaced by wakeup-driven redraws through `NSView.displayLink` on damage. Target: ~0% CPU with 10 idle terminals.

### 6.2 Input editor v2 (per pane)

```
│ ~/Code/impulse  ⎇ review-v2 ↑2  ±3 +63 −2  ⬢ node 22                                  │  chips
│ ❯ git commit -m "Stage hunks via git apply --cached" ▌                                 │  editor
│   ⏎ run · ⇧⏎ newline · ⌃R history · ⌘↑ select block · ⌘I compose for agent            │  hint (toggle)
```

- **Placement.** It lives inside each terminal pane instead of in a window-level bar, which is required once splits exist. The chips row shows:
  - cwd, with a dropdown of recent directories;
  - branch (switcher popover);
  - diff stats (opens review, carries the diff-base menu);
  - runtime version (node/python/ruby, when detected);
  - last exit/duration.
- **Editor.** A TextKit 2 `NSTextView` subclass (`CommandEditorView`), not a SwiftUI `TextField`:
  - multi-line, with ⇧⏎ for a new line and paste of multi-line commands;
  - mouse selection and Emacs keybindings for free;
  - native IME and dictation;
  - undo;
  - vi mode later.
- **Syntax highlighting** from `ImpulseKit.ShellParser` tokens:
  - commands, flags, strings, variables, pipes and redirects;
  - a dashed red underline for unknown commands (resolved against `PATH`, aliases and functions captured from the shell).
- **Completions:**
  - Spec-based: a curated JSON export of the MIT-licensed withfig/autocomplete specs, vendored by a `scripts/vendor-completion-specs.sh` in the same style as `vendor-monaco.sh`.
  - Impulse-native generators: git branches, remotes, tags and SHAs; files; npm/pnpm scripts; make and just targets; ssh hosts; docker containers.
  - An opt-in **native-shell bridge**: fish `complete -C`, bash `compgen`, zsh via a zpty capture as a research item.
  - The fuzzy menu shows descriptions and runs in `OverlayHost`.
- **History:**
  - Persistent and shared across tabs (SQLite, `~/Library/Application Support/impulse/history.sqlite`).
  - Each entry records command, cwd, exit code, duration, branch and timestamp. Commands starting with a space are not recorded.
  - Optional one-time import of zsh, bash and fish history.
  - ↑ gives prefix-filtered recall.
  - **⌃R** opens a themed history panel with fuzzy search and filters (this cwd / this repo / failed / today). It replaces `TerminalHistoryPicker` and fixes the phantom ⌘R hint. If the user's shell binds ⌃R to atuin or fzf, there is an option to defer to it.
- **Classic mode (D6).** The shell's own prompt and line editor. The grid takes focus, prompt suppression is off, and blocks still work from OSC 133. This is the escape hatch for zsh-vi, atuin and heavy zle users.

### 6.3 Blocks v2

- **Native block header row.** Command text plus inline chips:
  - ✓/✗ exit;
  - duration;
  - cwd when it differs from the previous block;
  - branch.

  The renderer's doc comment promises inline chips that are never drawn today. A hover toolbar on the header offers copy, rerun, filter, bookmark, "Send to agent" and the overflow menu.

- **Selection model (Warp).**
  - ⌘↑ selects the last block, then ↑/↓ move between blocks.
  - ⇧ extends the selection, ⌘-click toggles a block.
  - ⌘C on a selection copies its commands and outputs.
  - Esc returns to the input.
- **Fold/collapse** a block's output, with a "show last N lines" mode for noisy builds.
- **Block filter** (⌥⇧F): regex/case/invert plus an N-context-lines field. It is non-destructive.
- **Find** (⌘F) can be scoped to the selected block. It shows a match count and has case, regex and whole-word toggles.
- **Bookmarks** (⌘⇧K), shown as scrollbar markers with hover previews. ⌥↑/↓ jumps between them.
- **Send to agent.** Pastes the block as fenced context (command, exit code and output, trimmed) into a chosen agent pane (§8.4).
- **Open output in editor** as a scratch buffer.
- **Rendered-text block output.** A new FFI returns the **rendered** grid text for a row range, replacing the raw escape-stripped byte capture. The old capture concatenates spinner frames. Keep the byte capture only as a fallback for evicted rows.
- **Persistence.** Block metadata and rendered output for the last N blocks are restored with the session (§6.6).

### 6.4 Links, paths, hints

- `file:line:col`, `path(line,col)`, and stack-trace formats for Rust, Swift, TS, Python and Go are detected on hover. ⌘-click opens them in Impulse's editor at that line.
- OSC 8 `file://` links route to the editor instead of the default app.
- **Hints mode** (⌘⇧Space): keyboard labels over URLs, paths, SHAs and ports. Choose one to open, copy, or insert into the input (WezTerm QuickSelect / kitty hints).

### 6.5 Protocol coverage

- **Fix the existing bug:** OSC `9;4;…` (ConEmu progress) is currently parsed as a notification (`impulse-terminal/src/osc_scanner.rs:186`). Handle it as **progress** (state + percent) and show it as a ring on the tab and pane, plus a sidebar indicator.
- **OSC 21337** (iTerm2 session status): a status string and color drive the pane and agent status.
- **OSC 99** (kitty rich notifications) in addition to OSC 9 and 777.
- **OSC 133;B** marks prompt end, so prompt text and command text can be separated in classic mode.
- **Rendering fidelity:**
  - zero-width and combining characters (grapheme clusters);
  - undercurl, double, dotted and dashed underlines, and underline color.

  These need snapshot-format changes over FFI: a new cell-flag version and side-table for extra codepoints, versioned so the header changes atomically.

- **Kitty keyboard protocol**, so TUI agents get unambiguous modifiers.
- **Later:**
  - Image protocols (kitty graphics / iTerm2 inline). alacritty_terminal lacks them; a spike will evaluate a scanner-side implementation against Rio's `librio`, but there is no core swap without a separate approval.
  - A Metal glyph-atlas renderer with ligatures.

### 6.6 Notifications and attention

**Layers**, from least to most intrusive:

1. A ring on the pane.
2. A dot or progress indicator on the tab and the workspace row.
3. An entry in the Inbox.
4. A **`UNUserNotificationCenter`** notification, but only when the window is unfocused or the pane is not visible.
5. A Dock badge with the count of sessions needing input.

**Delivery details:**

- Clicking a notification focuses the exact pane (`userInfo` = pane id, `threadIdentifier` = workspace).
- Delivered notifications are removed when the pane gains focus.

**Sources:**

- Bell.
- OSC 9 / 777 / 99 / 9;4 / 21337.
- Long-running command finished while unfocused (setting, default 30s; exists today but only bounces the Dock).
- The agent hooks of §8.2.

### 6.7 Session restore v2

- **Restored:**
  - workspaces, tabs, split trees and pane surfaces;
  - cwd, titles and colors;
  - editor cursor, scroll and folds;
  - the last ~2,000 rendered lines of scrollback per terminal, fed back into the new terminal before the shell starts, under a dimmed "restored" separator;
  - block metadata;
  - agent session IDs collected by hooks, so an agent pane shows **Resume** (`claude --resume <id>`, `codex resume <id>`).
- Running processes are not restored. A live-process daemon (tmux-style) is explicitly out of scope; users who need it can run tmux or zellij inside Impulse.
- Session schema `version = 2`, with a migration from v1 (`SessionState.swift`).

## 7. Git

### 7.1 Architecture

```
ImpulseGit (library)
  GitRepository (one per repo, serial queue, cached git_repository*)
    ├─ snapshot() -> RepoSnapshot            libgit2 read path
    ├─ diff(scope:, pathspec:, options:)     libgit2 read path
    ├─ blame / log / graph / refs / stash list / worktree list
    └─ mutations -> GitCLI                   git CLI write path (hooks, signing, LFS, credentials)
  GitCLI (Process runner: resolved git path, login-shell PATH, env, stdin, timeout, cancel,
          progress parsing, plain-English error mapping)
  PatchBuilder (pure: hunks/line selections -> unified patch text; forward and reverse)
  RepoWatcher (FSEvents on worktree + gitdir + commondir; replaces DispatchSource on a hardcoded .git/index and the 10s poll)
  Snapshots (private refs: refs/impulse/oplog/*, refs/impulse/checkpoints/*)

ImpulseApp
  GitRepositoryState (@Observable, one per repo, shared across windows)
    branch, upstream, ahead/behind, operation (merge/rebase/cherry-pick/revert/bisect + step),
    staged[], unstaged[], untracked[], conflicted[], stashes, worktrees, PR (optional), lastError
  → feeds: titlebar breadcrumb, status bar, workspace rows, file tree badges, input chips,
           Changes panel, Review surface, editor gutters.
```

**`RepoSnapshot`:**

- Uses libgit2 status with **separate index and workdir flags**. Today staged and unstaged collapse into one letter.
- Rename detection is on.
- **No `GIT_STATUS_OPT_UPDATE_INDEX` in background refreshes.** It writes `.git/index` and contends with `index.lock` held by the user's or an agent's git commands.
- Untracked directories are collapsed in the list view, then expanded lazily.

**Diff scopes** (`DiffScope`, a pure enum in ImpulseKit):

- `unstaged` (index → workdir)
- `staged` (HEAD → index)
- `uncommitted` (HEAD → workdir)
- `branch(base)` (merge-base(base, HEAD) → workdir)
- `commit(sha)`
- `range(a, b)`
- `stash(n)`
- `checkpoint(from, to)`
- `sinceLastReview`

All diffs are **pathspec-limited per file**. Today each file expanded in review recomputes the whole-repo diff. Each hunk gets a stable id (a hash of its header plus content) for actions and reviewed state. Options: context lines, ignore whitespace, ignore EOL.

**Staging.**

- `PatchBuilder` turns a hunk or a line selection into a patch.
- Stage: `git apply --cached`. Unstage: `git apply --cached -R`. Revert in the workdir: `git apply -R`. All run through `GitCLI`, so filters and attributes match the user's git.
- Each revert first writes an oplog snapshot (§7.6).

**Commit** uses `git commit -F -` with amend, sign-off and skip-hooks as options. Hooks, `commit.template` and signing all apply. The libgit2 `commitAll` is retired. Its parity fixtures are updated deliberately, with the reason in the commit message ("commit moves to git CLI so hooks/signing run").

**Git binary resolution.**

- PATH is captured from the login shell (extending `ImpulseKit.LoginShell`).
- Detect when `/usr/bin/git` is the Command Line Tools shim without CLT installed, and show a one-time explanatory banner. Never trigger the CLT install dialog unexpectedly.

**Remote.**

- `fetch --prune --progress`, with optional auto-fetch every N minutes.
- `pull --ff-only` by default, with a rebase option.
- `push -u` and publish; force only as `--force-with-lease`, with confirmation.
- Progress shows in a toast. Auth failures map to "Run in terminal" with the exact command pre-filled.

**Operation state.** Merge, rebase (step n/m), cherry-pick, revert and bisect are read from the gitdir and drive a banner with Continue / Skip / Abort.

### 7.2 Changes panel (left dock, ⌃⇧G)

```
┌ CHANGES ───────────────────────────────── ⋯ ┐
│ ⎇ review-v2 ▾            ↑2 ↓0   [ Push ]   │  branch switcher · sync split-button
│ ┌ ⚠ Rebasing 3/7 ──── Continue · Skip · Abort│  only during an operation
│ ▾ CONFLICTS 1                               │
│   C  Sources/ImpulseGit/Hunks.swift   ⇄ ✓   │  open merge view · mark resolved
│ ▾ STAGED 2                        − all     │
│   M  web/review.js         +12 −3   −  ⋯    │
│   A  web/review.css        +88      −  ⋯    │
│ ▾ CHANGES 5                       + all  ↺  │
│   M  Sources/…/DiffReviewTab.swift +40 −2 + ↺│
│ ▾ UNTRACKED 1                     + all     │
│ ▸ STASHES 2                                 │
│ ▸ WORKTREES 3                               │
│ ─────────────────────────────────────────── │
│ ┌ Summary                              38/50│
│ │ Body…                                     │
│ └───────────────────────────────────────────│
│ ☐ Amend  ☐ Sign-off  ☐ Skip hooks           │
│ [ Commit ▾ ]  ⌘↩   (Commit & Push · Commit & PR)
│ Last: "Fix OSC 9;4 parsing" · Uncommit       │
└─────────────────────────────────────────────┘
```

**Sections.** Conflicts → Staged → Changes → Untracked, then Stashes, Worktrees, and recent commits (5).

- List or tree toggle.
- Multi-select.
- Hover actions per row: stage/unstage, discard, open file, open diff.

**Keys (panel focus only, never in the terminal):**

| Key                     | Action                                                                   |
| ----------------------- | ------------------------------------------------------------------------ |
| `space`                 | stage/unstage                                                            |
| `⏎`                     | open diff                                                                |
| `⌫`                     | discard (prompt for a file, no prompt for a hunk because it is undoable) |
| `⌘⌫`                    | discard without prompt                                                   |
| `⌘↩`                    | commit                                                                   |
| `⌘⇧↩`                   | amend                                                                    |
| `⇧Esc`                  | expand the message editor                                                |
| `↑` in an empty message | previous messages                                                        |

**Commit composer:**

- Subject/body split with 50/72 guides and a wrap toggle; honors `commit.template`.
- Message history.
- Optional Conventional Commits helper.
- If nothing is staged, a confirm offers to commit all tracked changes (Zed behavior). Untracked files are never added silently.
- After a commit, **Uncommit** (`reset --soft HEAD^`) appears.

**State-morphing primary button (Warp):** Commit → Push (n) → Publish branch → Create PR → PR #123 ↗. Its chevron holds the other actions.

**Untracked discard** moves files to the Trash (`NSWorkspace.recycle`), never `unlink`.

### 7.3 Review surface (⌘⇧G; docked right or full tab)

```
┌ REVIEW  [Uncommitted ▾ vs HEAD]  [Unified|Split]  [␣ ws]  [≡ 3 ctx]   7/12 viewed   💬 3  [Send to ◆ claude ▾] ┐
├ FILES ── filter (T) ────┬──────────────────────────────────────────────────────────────────────────────────────┤
│ ▾ web                   │ ▾ M web/review.js                            +120 −34   ☐ Viewed   Stage file   ⋯  │ sticky
│   ☑ M review.js    +120 │   @@ -232,18 +232,41 @@ function render(files)          ⌘Y Stage · ⌘⌥Z Revert · 💬  │
│   ☐ A review.css    +88 │    232  232    const sections = new Map()                                           │
│ ▾ Sources/ImpulseGit    │    233       − container.innerHTML = ''                                             │
│   ☐ M Hunks.swift   +40 │         233  + patchSections(container, files, { keepScroll: true })                │
│   ● changed since view  │   ┃ 💬 You: keep expansion state across refreshes        Edit · Delete            │
│ STAGED 2 · UNSTAGED 8   │   ⋯ 18 unchanged lines ⋯                                                         │
│                         │ ▸ A web/review.css                                     +88 −0   ☐ Viewed         │
└─────────────────────────┴──────────────────────────────────────────────────────────────────────────────────────┘
```

**Scope selector** (fuzzy searchable). Uncommitted (default) / Unstaged / Staged / vs `main` (merge base, auto-detected) / vs any branch / Last commit / commit range / stash / **Last agent turn** / **Since my last review** / checkpoint n→m. The input diff chip opens the same menu.

**Navigator:**

- File tree with status, +/−, a comment badge and a **Viewed** checkbox.
- Viewed resets automatically when the file's content hash changes, and the file is badged "changed since viewed".
- A progress count; filter with `T`.

**Body.** One continuous scroll of file cards.

- **Sticky file headers.** Today's CSS cannot stick, because `overflow:hidden` is on the section.
- Collapse/expand per file and for all.
- Unchanged regions fold to 3 context lines with expanders.
- Word-level spans (existing).
- **Unified or split** (split renders aligned two-column rows in the same renderer).
- Whitespace toggle.
- **Row-level virtualization** inside huge files; today a single huge file renders every row.

**Syntax highlighting fix.** Tokenize the old and new sides separately, using full old-blob and new-file text so tokenizer state is correct. Today every hunk line is fed into one throwaway model, so state bleeds across hunks and between the old and new sides.

**Actions.** Per hunk, per line selection (drag in the gutter), per file and for all: Stage, Unstage, Revert. Each revert shows an **Undo** toast.

**Keys (review focus):**

| Key       | Action                 |
| --------- | ---------------------- |
| `j` / `k` | next / previous hunk   |
| `n` / `p` | next / previous file   |
| `N`       | next unviewed file     |
| `⌘Y`      | stage hunk and advance |
| `⌘⇧Y`     | unstage and advance    |
| `⌘⌥Z`     | revert hunk            |
| `v`       | toggle viewed          |
| `c`       | comment                |
| `o`       | open in editor at line |
| `⌘↩`      | send comments          |
| `s`       | toggle split           |

`⌘Y` and `⌘⌥Z` follow Zed's convention, so agent-review keep/reject uses the same muscle memory.

**Comments.** Click or drag line numbers, then `c`.

- The composer is markdown-lite. Comments are anchored by (scope, path, side, line range, snippet hash).
- They are stored locally per repo and branch: `~/Library/Application Support/impulse/review/<repo-hash>/<branch>.json`, never in the repo.
- When the anchor no longer matches, a comment moves to an **Outdated** section.
- **Send to agent** formats all pending comments as one markdown prompt (path:line-range, snippet, comment), then delivers it to the chosen agent pane (§8.4). Copy-as-prompt and export are also available.
- Later: import GitHub PR review threads via `gh api graphql` (§7.7).

**Deep dive.** "Open in diff editor" opens Monaco's `createDiffEditor` for one file: side-by-side or inline, `hideUnchangedRegions`, and the modified side editable and saved to disk.

**Live updates.** The surface subscribes to `GitRepositoryState`. The protocol becomes incremental:

- `Render{files, generation}`, `UpdateFile`, `RemoveFile`, `SetHunks{fileId, hunks, contentHash}`.
- Expansion, scroll and viewed state survive refreshes. Today every refresh tears all sections down.
- If the file under the cursor changes, a "Updated — show" chip appears instead of yanking the scroll position.

**Discard and revert propagate** to open editor tabs, which reload. That works today only from the file tree.

### 7.4 Editor integration

- Gutter markers are computed **against the live buffer**, not the disk. Today they go stale while typing. Each marker reflects index or HEAD per setting.
- Clicking a marker opens an inline **peek diff** (Monaco view zone) with Stage hunk / Revert hunk / Next / Previous.
- **Inline current-line blame** as end-of-line ghost text (author, relative date, summary), plus a full blame gutter toggle. It uses a whole-file blame cached by blob id; today `lineBlame` re-blames the whole file per call and nothing calls it.
- **File history** opens a History surface filtered to the path. **Open changes** opens the review scoped to the file.
- **Conflict markers.** Inline Accept current / Accept incoming / Accept both actions, a conflict counter with next/previous, and **Mark resolved** (stages the file).

### 7.5 History surface

- A commit graph with lanes. The layout algorithm is pure, lives in ImpulseKit, and is fixture-tested.
- Columns: subject, ref chips (branches, tags, HEAD, upstream), author, relative date and SHA.
- Incoming/outgoing markers against upstream. Commits before the branch's fork point are greyed out (Tower).
- Search and filter by message, author, SHA, path and date.
- **Commit details pane:** metadata plus a multi-file diff, reusing the review renderer read-only.
- **Actions:**
  - checkout (detached, with a warning);
  - create branch here;
  - cherry-pick;
  - revert;
  - reset soft/mixed/hard (hard needs confirmation and gets an oplog snapshot first);
  - copy SHA;
  - compare with the working tree;
  - compare two selected commits.
- **Later:** interactive rebase editor (reorder/squash/fixup/reword/drop), and "find base commit for fixup".

### 7.6 Safety: the operation log

- **Snapshot before every destructive action:**
  - discard (file or hunk);
  - checkout over a dirty tree;
  - reset;
  - stash drop;
  - branch delete;
  - rebase start.

  The snapshot is written to `refs/impulse/oplog/<ts>`. It holds the index tree plus a worktree tree including untracked, non-ignored files, built in a temporary index (`GIT_INDEX_FILE=… git add -A && git write-tree && git commit-tree`), the same semantics as `git stash create -u`.

- **⌘Z in git focus** undoes the last git operation. The **Operation History** list (Changes panel ⋯ menu) offers Restore per entry.
- **Retention:** last 200 entries or 14 days, pruned on launch. These refs are never pushed (outside `refs/heads` and `refs/tags`). Document that `git push --mirror` would include them.

### 7.7 Branches, stashes, worktrees, PRs

- **Branch switcher** (popover from the breadcrumb, the chip or `⌃⌘B`):
  - Fuzzy search; recent first; local and remote grouped.
  - Badges: worktree, PR, merged, stale.
  - Create from any ref.
  - Switching over a dirty tree offers stash & switch / carry changes / open in a new worktree / cancel.
  - Rename, delete (with merged check) and set upstream.
- **Stashes:** save with a message, include untracked, keep index. Each can be applied, popped or dropped; drop is undoable via the oplog. View a stash as a multi-file diff.
- **Worktrees:**
  - The list shows path, branch, dirty count and locked status.
  - Add from a branch, a new branch or a PR (`gh pr checkout` into a new worktree). Location: setting, default `../<repo>.worktrees/<branch>`.
  - Remove, with dirty/unpushed checks; prune.
  - Opening a worktree creates a workspace.
  - Fix the watcher: resolve `git_repository_commondir` and the per-worktree gitdir instead of hardcoding `<root>/.git/index`.
- **PRs (only if `gh` is installed and authenticated):**
  - A per-branch PR chip: number, state, draft, review decision and checks rollup, from `gh pr view --json …` and `gh pr checks --json …`, polled with backoff.
  - Create PR (`gh pr create`, draft option, title and body from commits).
  - Open in browser. A notification fires when checks finish.
  - Later: import inline review threads into the Review surface.

## 8. Agent workflows

### 8.1 Detection

- **Foreground process.** Detect the foreground process of each PTY. This needs a new FFI: the foreground process-group pid via `tcgetpgrp` on the master fd, plus `proc_pidpath` and argv through `KERN_PROCARGS2` for node-wrapped CLIs.
- **Known agents table** in ImpulseKit, user-extendable: `claude`, `codex`, `gemini`, `opencode`, `aider`, `amp`, `cursor-agent`, `copilot`, `goose`, `droid`, `crush`.
- A pane running a known agent becomes an **agent pane**:
  - an agent icon in the tab and sidebar row;
  - an agent footer instead of the input editor (§8.3);
  - status tracking.

### 8.2 Status signals (layered)

1. Generic, always available:
   - bell;
   - OSC 9 / 777 / 99, including Codex `tui.notifications`;
   - OSC 9;4 progress (Claude Code sends this to terminals it recognizes);
   - OSC 21337 status;
   - OSC 133 prompt/command boundaries for shells;
   - foreground-process exit.
2. **Hooks (best fidelity).** An `impulse` CLI that talks to the app socket (§8.6):
   - Claude Code: a one-click **"Install Impulse hooks"** action. It shows the exact JSON diff and asks for confirmation before writing `~/.claude/settings.json` (or the project's `.claude/settings.local.json`). Hooks: `UserPromptSubmit` → working + checkpoint; `Notification` (`permission_prompt` / `idle_prompt`) → needs input; `Stop` → done + checkpoint; `SessionStart` → record session id for Resume.
   - Codex: the `notify` program config.
   - Others: OSC 9 or nothing.
3. **State machine** in ImpulseKit (pure and tested): `idle → working → needsInput → working → done | error`, with timeouts and debouncing. It is unit-testable from event sequences.

### 8.3 Agent pane UI

- **Footer:** `◆ Claude Code · needs input · ⌘I Compose · ±+63 −2 Review · ⟲ 4 checkpoints · ⋯`.
- **Composer (⌘I):** a multi-line `CommandEditorView` overlay above the TUI with:
  - @file mentions from the workspace file index;
  - image paste (a temp PNG path, as today);
  - prompt history;
  - send via bracketed paste.
- It can auto-show when the agent becomes idle or needs input (setting).
- **Opt-in `$VISUAL` integration.** Impulse terminals can export `VISUAL="impulse edit --wait"`. Then Claude Code's and Codex's own ⌃G ("edit prompt in external editor"), and `git commit` from the shell, open an Impulse editor tab, and the CLI waits until the tab is closed. This is robust and needs no PTY tricks.

### 8.4 The review loop

1. Agent works → the pane is "working" and the diff pill counts up live.
2. Agent stops → "done" plus a notification. The Inbox entry offers **Review last turn**.
3. Review opens on the `Last agent turn` scope (checkpoint n-1 → workdir). The user stages, reverts and comments.
4. **Send to agent** delivers the comment batch to the agent pane, or to the composer for editing first (setting).
   - Delivery is a bracketed paste. It does not press Enter by default (setting).
   - If the target agent is "working", the batch is queued and delivered when it becomes idle.
5. Next turn → the `Since my last review` scope shows only what changed after the review.

Blocks, editor selections, diagnostics (Problems panel) and files all have **Send to agent** using the same delivery path. This is the "context harvesting" set from `.serena/memories/future-ideas/ai-workflow-features.md`.

### 8.5 Worktree tasks, ports and project config

- **New task** (＋ menu, or `impulse task new "<name>"`):
  1. Creates a branch with an auto-generated name and its worktree.
  2. Copies the files listed in `.worktreeinclude`, or in `[worktree].copy` in `.impulse/project.toml`, such as `.env*`.
  3. Runs `setup` scripts.
  4. Opens a workspace with the configured agent command.
- **Archive** checks for unpushed or dirty work, runs `archive` scripts, and removes the worktree. Archived workspaces are listed and can be restored from their branch.
- **`.impulse/project.toml`** (versioned schema, parsed in ImpulseKit, validated; the earlier "launch config" item in the 2026-05-04 plan merges into this):
  - `[worktree]` location and copy list;
  - `[scripts]` setup/run/archive;
  - `[[actions]]` named commands shown as buttons and palette entries;
  - `[[layouts]]` tab configs: a pane tree with terminal/agent/editor/review leaves, cwd, commands, and params of type `text`, `branch` or `repo`.

  Commands from a project file run only after a per-repo trust prompt that shows the commands.

- **Ports.**
  - Listening TCP ports are detected per workspace by walking each pane's process tree (`proc_pidinfo` / `PROC_PIDFDSOCKETINFO`, no `lsof`) and shown as chips in the status bar and workspace row.
  - Clicking opens the port in the default browser, or later in a Preview/Browser pane.
  - Optional per-workspace port block: `IMPULSE_PORT` plus 9 more (the Conductor model).

### 8.6 `impulse` CLI and local socket

- **Socket.** A Unix domain socket at `~/Library/Application Support/impulse/run/impulse.sock` (directory 0700, socket 0600). It speaks newline-delimited JSON requests and responses with a protocol version.
- **CLI.** A small Swift executable target bundled at `Impulse.app/Contents/MacOS/impulse`. Impulse terminals get it on `PATH` automatically. "Install command line tool" symlinks it into `~/.local/bin` or `/usr/local/bin`, with consent.
- **Commands:**
  - `open <path[:line[:col]]>`
  - `edit --wait <path>`
  - `split [-v] [cmd]`
  - `tab new`
  - `notify [--title] <body>`
  - `status <working|needs-input|done|error> [--detail]`
  - `checkpoint [--label]`
  - `review [--scope]`
  - `task new|archive`
  - `send <pane> <text>`
- **Environment.** Each shell gets `IMPULSE_SOCKET`, `IMPULSE_PANE_ID` and `IMPULSE_WORKSPACE_ID`, plus a per-pane token that is required for status, notify and checkpoint calls, so panes cannot spoof each other.
- **No screen-reading API in v1.** If added later, it needs a setting and a per-call indicator.

### 8.7 Inbox

- **Titlebar bell popover:**
  - Sessions sorted by needs input → error → working → done.
  - Each entry shows its latest notification line, the time, and Jump / Review last turn / Snooze.
- Filters: All / Unread / Errors.
- **⌘⇧U** jumps to the next pane that needs input (cmux).
- The status-bar agent summary opens the same list.

### 8.8 Explicitly not doing (v1)

- A built-in model or chat.
- Accounts, telemetry or cloud sync.
- Natural-language detection in the shell input.
- An agent "orchestration chat".
- Best-of-N fan-out. Later; worktree tasks make it possible.
- Docker-isolated agents.
- An ACP client. Revisit after v1, because it would let structured agents render natively.
- The Claude Code IDE-protocol server (`~/.claude/ide/*.lock` + MCP over WebSocket). It is reverse-engineered and could change. Revisit as an experiment.

## 9. Editor

- **Panes.** Editor surfaces stack within a pane, with a mini tab header. One **WKWebView per editor pane** holds multiple Monaco models and saves and restores view state when switching. Today there is one WebView per file, so memory grows linearly with open tabs. `EditorWebViewPool` pre-warms per pane.
- **Problems panel** (bottom dock):
  - Diagnostics for all files the servers report, not only open ones (today non-open diagnostics are dropped).
  - Grouped by file; filter by severity.
  - Counts in the status bar.
  - Send to agent.
- **Outline** (right dock) and **breadcrumbs** (pane header) from `textDocument/documentSymbol`. **Workspace symbols** in the palette (`#`). Document highlight, inlay hints, type definition and implementation, and code lens as follow-ups.
- **`workspace/applyEdit` and multi-file `WorkspaceEdit`** are applied by Swift across open and closed files, with an undo group. This fixes cross-file rename and code actions, which likely fail today. Command-only code actions execute via `workspace/executeCommand`.
- **Project-wide find & replace** with a preview. **Quick Open** (⌘P) becomes a real fuzzy file finder; today it aliases project search.
- **Optional vim mode** (monaco-vim, MIT).
- **Side-by-side Markdown preview** as a split rather than an in-place toggle. "Run" buttons on shell code blocks send to the focused terminal (Warp's Markdown viewer).
- **LSP status** in the status bar, with restart/stop/logs. Logs go to the bottom dock's Output. Move LSP off the single app-wide serial queue so a slow hover can't block completions in other windows.

## 10. Commands, keybindings, settings

### 10.1 Command system

- **`CommandRegistry`** in ImpulseApp, with pure parts in ImpulseKit. Each command has:
  - a typed id, title, category and icon;
  - a `when` context expression (`terminalFocus`, `inputFocus`, `editorFocus`, `reviewFocus`, `gitPanelFocus`, `agentPane`, …);
  - a handler.
- Menus, the palette, keybindings, toolbar buttons and the socket API all dispatch through it.
- It replaces about 45 NotificationCenter observers in `MainWindow.swift` that gate on `isKeyWindow`. `KeybindingResolver` (pure) is unit-tested with context sets.
- **Palette v2** (themed, `OverlayHost`), prefix modes:
  - (none) files, then commands;
  - `>` commands;
  - `@` symbols in file;
  - `#` workspace symbols;
  - `:` go to line;
  - `%` text search;
  - `b:` branches;
  - `w:` workspaces;
  - `t:` tabs and panes;
  - `h:` history;
  - `a:` actions from `project.toml`.

  A new **fuzzy scorer** in ImpulseKit with its own tests. The existing `CommandPalette.filterItems` fixtures stay until the palette switches over; then the fixture is retired or updated in that commit with the reason. Recents persist across launches.

### 10.2 Default keymap (draft)

| Area                           | Keys                                                                                                                                          |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Palette / quick open / symbols | ⌘⇧P · ⌘P · ⌘⇧O                                                                                                                                |
| Docks                          | ⌘B left · ⌥⌘B right · ⌘J bottom · ⌘⇧E files · ⌃⇧G changes · ⌘⇧F search                                                                        |
| Panes                          | ⌘D / ⌘⇧D split · ⌥⌘←→↑↓ focus · ⌘[ ⌘] cycle · ⌘⇧↩ zoom · ⌘W close · ⌘Z (after close) restore                                                  |
| Tabs / workspaces              | ⌘T · ⌘1–9 tab · ⌃Tab · ⌃⌘[ ⌃⌘] workspace · ⌃⌘B branch switcher · ⌃⌘N new worktree task                                                        |
| Terminal                       | ⌘↑ select block · ⌥⇧F filter block · ⌘⇧K bookmark · ⌥↑↓ bookmarks · ⌃R history · ⌘⇧Space hints · ⌘K clear · ⌘I compose (agent pane) |
| Git / review                   | ⌘⇧G review · ⌘Y stage-advance · ⌘⇧Y unstage-advance · ⌘⌥Z revert hunk · ⌘↩ commit/send · ⌘⇧↩ amend                                            |
| Attention                      | ⌘⇧U next needs-input · ⌘⇧I inbox                                                                                                              |

User overrides keep working, and settings migration maps old command ids to new ones. The old `⌘⇧B` sidebar toggle stays as a secondary binding for one release so muscle memory has time to move. `⌘⇧M` keeps toggling Markdown preview, which now opens as a side-by-side split. Multi-key chords are avoided throughout.

### 10.3 Settings

- **One `@Observable SettingsStore`.** Today there are four hand-synced copies: AppDelegate, MainWindowController, TabManager and SettingsWindowController.
- **Settings becomes a themed surface** opened as a tab:
  - search;
  - categories;
  - "modified" markers;
  - each setting deep-linkable from the palette;
  - "Open settings.json", backed by a JSON Schema so Monaco validates it.
- **The keybindings editor** is also a surface: search by command or by key, conflict detection, `when` contexts.
- **Expose keys that exist without UI:** minimum contrast, OSC 52 read/write, cursor surrounding lines, selection/occurrence highlight, word-based suggestions.
- Fix the `"bar"` vs `"beam"` cursor mismatch (`SettingsWindow.swift:458` vs `TerminalTab.swift:876-880`).

## 11. Performance budgets (acceptance criteria)

| Budget                                                 | Target                                                                     |
| ------------------------------------------------------ | -------------------------------------------------------------------------- |
| Cold launch to interactive terminal                    | < 500 ms (M-series)                                                        |
| Idle CPU, 10 terminals (2 visible), no output          | < 0.5%                                                                     |
| Keystroke-to-glyph in terminal                         | < 8 ms p95                                                                 |
| Git snapshot refresh, 50k-file repo, warm              | < 150 ms, off main thread; UI never blocks on git                          |
| Review open, 200 files / 20k changed lines             | first paint < 300 ms; scroll at 120 Hz without dropped frames on ProMotion |
| Memory per extra editor file in an existing pane       | < 5 MB (multi-model)                                                       |
| Background terminal event latency (bell/OSC 9 → badge) | < 250 ms                                                                   |

## 12. Testing strategy

- **Pure logic → ImpulseKit tests** (Swift Testing; run with `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test`):
  - LayoutTree operations;
  - KeybindingResolver;
  - FuzzyScorer;
  - DiffScope;
  - comment anchoring and outdated detection;
  - viewed-state hashing;
  - the agent state machine;
  - commit-graph lane layout;
  - `project.toml` parsing and validation;
  - theme `[ui]` derivation.
- **ImpulseGit:**
  - **New scenario repos:** staged-only, mixed staged+unstaged, renames, binary, conflicts (merge and rebase), linked worktree, submodule, detached HEAD, unborn HEAD, ignored files, CRLF.
  - **Use the real `git` CLI as the oracle** for new APIs: `status --porcelain=v2`, `diff --numstat`, `diff --cached`, `rev-list --left-right --count`, `log --graph --format`. This follows the fixture rule ("never regenerate from Swift output") without the deleted Rust generator.
  - `PatchBuilder` is tested by round-trip: build a patch, `git apply --check`, apply it, and compare the index with an expected `git diff --cached`.
- **Existing golden fixtures stay the spec.** Every intentional change, such as `commitAll` moving to the CLI, edits the fixture in the same commit with the reason.
- **App-level:**
  - Expand `ImpulseAppTests` for focus routing (input ↔ grid ↔ TUI) and session migration.
  - A debug Component Gallery for visual review.
  - Manual QA scripts per milestone, run in **Impulse Dev.app**, not the release app (memory note `impulse-macos-dev-vs-release-app`).
  - Avoid synthetic keystroke automation (memory note `claude-no-synthetic-input-while-working`).
- **Rust:** `cargo test -p impulse-terminal` for scanner additions (9;4 progress, 21337, 99, 133;B) and the rendered-text and foreground-pid FFI.

## 13. Risks

| Risk                                                                                | Mitigation                                                                                                                    |
| ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Titlebar/toolbar behavior on Tahoe (glass, item sizing, full screen; Ghostty #9597) | Phase 0 spike with explicit acceptance tests (full screen, Split View, notch, RTL, double-click, drag) before any chrome work |
| Focus regressions (input ↔ grid ↔ TUI ↔ WKWebView)                                  | Centralize in a `FocusCoordinator` with logging, and keep the existing tokens; dedicated tests                                |
| Scope: this is a year-sized plan                                                    | Milestones ship independently; Track B (git core) and Track C (terminal) run parallel to chrome; each milestone is releasable |
| git CLI semantics vs libgit2 reads (e.g. racy status after CLI writes)              | Every CLI mutation triggers a snapshot refresh; RepoWatcher debounces; tests compare against the CLI oracle                   |
| Writing to users' agent configs                                                     | Always show a diff and confirm; support uninstall; prefer project-local files                                                 |
| Socket/CLI security                                                                 | Same-user permissions, per-pane tokens, no remote listeners, no screen reading in v1                                          |
| Users who liked the native look                                                     | One-time default change at a major version; density and font settings; the classic input mode stays; release notes            |
| Memory (Warp's 16–130 GB horror stories)                                            | Hard caps on block output, scrollback restore and review rendering; memory budgets in §11 checked per milestone               |

## 14. What stays the same

- The Rust terminal core (alacritty_terminal + OSC scanner + blocks), behind the C FFI. It is extended, not replaced.
- Monaco, vendored via `scripts/vendor-monaco.sh`. cmark-gfm safe mode (no `CMARK_OPT_UNSAFE`).
- The target structure: ImpulseKit stays Foundation-only; ImpulseGit and ImpulseLSP are libraries; `Bridge/ImpulseCore.swift` stays thin.
- The theme TOML format, extended. User themes keep working.
- `scripts/release.sh` remains the only release path, and `VERSION` remains the single source of truth.
