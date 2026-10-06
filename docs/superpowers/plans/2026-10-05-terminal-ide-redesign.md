# Impulse Workbench: terminal IDE redesign plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to work through this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild Impulse as a dense, keyboard-first, agent-aware terminal IDE. It gets:

- custom chrome;
- workspaces (one per git worktree) and generalized splits;
- a real git layer with hunk/line staging;
- a live, multi-scope change-review surface whose comments flow back to CLI agents;
- a much stronger terminal input/blocks experience.

**Spec:** `docs/superpowers/specs/2026-10-05-terminal-ide-redesign-design.md`. Section references below (§n) point there. Approve or override the decisions table (§3) before starting M1.

**Architecture:**

- **ImpulseApp:** AppKit owns the workbench layout and focus. SwiftUI renders panels. A design system and an `OverlayHost` built on child panels supply all chrome.
- **ImpulseKit:** pure, tested logic (layout tree, keybinding resolution, fuzzy scoring, diff scopes, review anchoring, agent state machine, graph layout, project config).
- **ImpulseGit:** libgit2 handles reads; the `git` CLI handles writes through a single `GitCLI` runner.
- **Rust terminal core:** gains a few FFI calls (rendered text by row range, foreground pid, protocol additions) but is not replaced.

**Ground rules (from CLAUDE.md and memory):**

- Golden fixtures are the spec: never regenerate them from Swift output. Every intentional behavior change edits the fixture in the same commit and gives the reason. New git APIs use the real `git` CLI as the oracle.
- ImpulseKit stays Foundation-only. Keep backend logic out of `Bridge/ImpulseCore.swift`.
- Test GUI changes in **Impulse Dev.app** (`./impulse-macos/build.sh --dev`), not `/Applications/Impulse.app`. Don't drive the GUI with synthetic keystrokes. Use file-based diagnostics and user-driven repro.
- Every custom `NSView.draw` clips to bounds (macOS 26 shared-canvas dirtyRect trap).
- Run tests with `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test` from `impulse-macos/`, and `cargo test -p impulse-terminal`.
- Only `scripts/release.sh` releases. Each milestone below should be shippable as its own release.

---

## Milestone map

```
            ┌──────────── Track A: shell & layout ────────────┐
M0 ─────────┤ M1 Workbench chrome ──► M2 Workspaces & panes ──┼──► M6 Agents ──► M8 Depth & polish
Stabilize & │                                                 │      ▲
foundations ├──────────── Track B: git ───────────────────────┤      │
            │ M3 Git core ──────────► M4 Changes + Review v2 ─┼──────┤──► M7 History, conflicts, PRs
            ├──────────── Track C: terminal ──────────────────┤      │
            │ (M0 hub) ─────────────► M5 Terminal power ──────┘──────┘   (M5 per-pane input needs M2)
```

| Milestone | User-visible headline                                                              | Depends on     |
| --------- | ---------------------------------------------------------------------------------- | -------------- |
| M0        | Bug fixes, nothing else visible                                                    | none           |
| M1        | New look: titlebar tabs, docks, status bar, themed palette & settings              | M0             |
| M2        | Workspaces sidebar, splits, pane zoom, session restore v2                          | M1             |
| M3        | (Mostly invisible) staged/unstaged aware, watcher, CLI writes, oplog               | M0             |
| M4        | Changes panel, Review v2 with hunk/line staging, editor git peek/blame             | M1, M3         |
| M5        | Multi-line input editor, persistent history, blocks v2, links, notifications       | M0 hub, M2     |
| M6        | Agent status, inbox, `impulse` CLI, review→agent loop, checkpoints, worktree tasks | M2, M3, M4, M5 |
| M7        | History graph, conflicts UI, stash/branch mgmt, `gh` PRs                           | M3, M4         |
| M8        | Problems/outline, multi-file edits, multi-model editor, a11y & perf passes         | M2             |

---

## M0: Stabilize and lay foundations

**Why first:** the code survey found real bugs and structural debt that every later milestone would trip over. Nothing here changes the look.

### 0.1 Bug fixes (each is its own small commit)

- [x] **OSC 9;4 progress parsed as a notification.**
  - Problem: `impulse-terminal/src/osc_scanner.rs:186` treats any `9;` payload as a notification.
  - Fix: add `OscEvent::Progress { state, percent }` for `9;4;st;pct`, and keep `9;<text>` as a notification.
  - Tests: `9;4;1;50`, `9;4;0`, `9;4;3`, plain `9;hello`.
- [x] **Cursor style "bar" vs "beam".**
  - Problem: the settings popup offers `bar` (`Settings/SettingsWindow.swift:458`), but the terminal expects `beam` (`Terminal/TerminalTab.swift:876-880`).
  - Fix: normalize on load and accept both.
- [x] **Classic mode can't type.**
  - Problem: with `terminal_context_bar` off, the grid still refuses first responder at a prompt (`Terminal/TerminalRenderer.swift:209-213, 418, 2486-2493`).
  - Fix: make the grid accept focus whenever the input bar is disabled.
- [x] **Phantom ⌘R.**
  - Problem: the history button tooltip says "⌘R" (`SwiftUI/Views/TerminalContextBarView.swift:138`), but nothing binds it.
  - Fix: bind a `terminal.history` command to ⌃R in input focus and update the tooltip.
- [x] **Branch switch from an editor tab silently does nothing.**
  - Problem: `ContextChips.swift:78` → `MainWindow.swift:321` routes to `selectedTerminal`, which is nil on an editor tab.
  - Interim fix: run `git switch` via a background `Process` in the repo root and surface errors in a toast. M3 replaces this with `GitCLI`.
- [x] **Branch picker blocks the main thread.**
  - Problem: `BranchPickerView.swift:75-76` loads branches synchronously.
  - Fix: load async with a spinner row.
- [x] **File-tree discard runs on the main thread.**
  - Problem: it runs after `runModal` (`FileTreeListView.swift:276-296`).
  - Fix: move it to a background queue.
- [x] **Review discard doesn't reload editors.**
  - Problem: discarding from the review leaves open editor tabs stale.
  - Fix: post `.impulseReloadEditorFile` like the file-tree path does (`DiffReviewTab.swift:416`).
- [x] **Review sticky header never sticks.**
  - Problem: `web/review.html:98` sets `.review-section{overflow:hidden}`.
  - Fix: use `overflow: clip` on the body only, or move the header outside the clipped box.
- [x] **Index watcher breaks on worktrees and submodules.**
  - Problem: `Sidebar/FileTreeDataController.swift:410-423` hardcodes `<root>/.git/index`.
  - Fix: resolve the gitdir via `git rev-parse --git-dir`, or via libgit2 `git_repository_path`. This must be fixed before M6 worktree tasks.
- [x] **Background polls write the index.**
  - Problem: `ImpulseGit/Status.swift:74,151` passes `GIT_STATUS_OPT_UPDATE_INDEX` on background polls, which contends with agents' `index.lock`.
  - Fix: drop the flag for background refreshes and keep it only for explicit user refresh. Re-run the parity fixtures; status output must not change.
- [x] **Session restore drops pinned state.** Re-apply pinned state, and fix the editor/terminal insertion-order drift (`MainWindow.swift:2643-2681`).
- [x] **Remove dead code:**
  - toolbar items `newFile/newFolder/refresh/collapseAll/toggleHidden` and `sidebarOnlyItems` (`MainWindow.swift:483-674`);
  - `WindowModel.commandPaletteVisible`;
  - `IconCache` `toolbar-*` SVGs;
  - the `.impulseThemeDidChange` posts that have no observers;
  - empty `windowDidResize`;
  - stale doc comments mentioning NSSplitView or tab segments.
- [x] **Stale repo docs.** `AGENTS.md` and `PLAN.md` describe the Rust/GTK era. Rewrite `AGENTS.md` to match CLAUDE.md, and delete `PLAN.md` (Linux plan).

### 0.2 Spikes (time-boxed, each ends in a short findings note under `docs/superpowers/specs/`)

- [x] **Spike A: titlebar chrome (§5.2).**
  - Build a throwaway window that tries option 1 (one full-width custom `NSToolbarItem` in an empty `.unifiedCompact` toolbar), then option 2 (content under a transparent titlebar with drag regions).
  - **Acceptance criteria:**
    - traffic lights stay vertically centered;
    - double-click honors `AppleActionOnDoubleClick`;
    - drag works on empty chrome but not on tabs;
    - full-screen enter/exit collapses the traffic-light inset;
    - Split View tiling, notch screens and menu-bar autohide all behave;
    - no Liquid Glass bezel on items;
    - VoiceOver reads the tab strip as tabs.
  - Pick one and record why.
- [x] **Spike B: workbench without NavigationSplitView.**
  - Build an AppKit `WorkbenchView` with left/right/bottom docks, hosting today's `SidebarView` and the content container.
  - Verify the terminal sizes correctly on first layout without the fake `didResizeNotification` (`ContentAreaRepresentable.swift:15-30`).
  - Verify that focus survives dock toggles.
- [ ] **Spike C: OverlayHost.**
  - Generalize `CompletionPanel` / `NonKeyPanel` into a host for themed child panels positioned over WKWebView and the grid.
  - Prove three cases: a popover anchored to a SwiftUI chip; a toast stack; a key-taking palette panel that returns focus correctly.
- [x] **Spike D: TerminalSessionHub.**
  - Pump `pollEvents()` for hidden terminals at 4 Hz, or on the PTY wakeup.
  - Measure CPU with 10 idle terminals.
  - Prove that a bell in a hidden tab reaches the sidebar dot in under 250 ms.

### 0.3 Foundations (refactors with no visible change)

- [x] **`SettingsStore`.**
  - A single `@Observable` instance replaces the hand-synced copies in `AppDelegate.settings`, `MainWindowController.settings`, `TabManager.settings` and `SettingsWindowController.settings`.
  - Keep the on-disk format, the per-key fault tolerance and the "don't overwrite a broken file" behavior (`Settings.swift:327-392, 617-664`).
- [ ] **`CommandRegistry` + `KeybindingResolver`.**
  - `ImpulseKit/Commands/` holds `CommandID`, the `WhenClause` parser/evaluator and `KeybindingResolver`. These are pure and tested.
  - `ImpulseApp/App/CommandRegistry.swift` holds handlers.
  - Migrate the 22 built-ins plus the 24 palette built-ins.
  - Generate menus from the registry (`UI/MenuBuilder.swift`).
  - Replace the NotificationCenter observers in `MainWindow.swift:1356-2050` with registry dispatch to the key window's controller. Keep the notification names as thin shims until callers are gone.
  - Tests: resolver precedence (user override > when-specific > default), conflict detection, migration of old override ids.
- [ ] **Split `MainWindow.swift` (2936 lines).** *(step one done: its sections now live in `MainWindowController+*.swift` extensions (layout, observers, save, find bar, tab close, session, workspaces, palette/git hosts, debug), leaving ~730 lines; extracting real controllers below is still to do)*
  - `WorkbenchWindowController` (window and chrome wiring);
  - `FileTreeCoordinator` (the four duplicated rebuild blocks become one);
  - `SessionController`;
  - `SavePipeline`;
  - `FindBarController`;
  - `ExternalCommandRunner`;
  - `CloseRiskController` (deduplicate with `AppDelegate.swift:285`).

  Remaining duplicates to fold in:
  - the New File/Folder dialogs (`MainWindow.swift:421-455` vs `:696-742`);
  - `TerminalTheme` construction (3×, `TabManager.swift:155-313`);
  - the drag-reorder plus context menu shared by `TabBarView` and `SidebarTabListView`.

- [x] **`GitCLI` runner (ImpulseGit).**
  - Resolves the git binary from login-shell `PATH`: extend `ImpulseKit/LoginShell.swift` to capture `PATH` once, asynchronously at launch.
  - Detects the missing-CLT shim.
  - Supports stdin, env, cwd, timeout, cancellation, streaming progress lines, and structured `GitCLIError`.
  - Maps common errors to plain English: no upstream, non-fast-forward, auth failure, identity not configured, index.lock present, hook failed (show hook output).
  - Tests with a temp repo.
- [ ] **`FocusCoordinator`.**
  - One place that decides input editor vs grid vs TUI vs WebView, and logs transitions (debug setting).
  - Wraps the current logic spread across `TerminalRenderer.swift:209-213, 418, 2486-2493`, `TerminalTab.swift:106-120`, `TabManager.swift:751-765` and `MainContentView.swift:60-67`.

**M0 definition of done:**

- All bugs fixed with tests where testable.
- Spikes A–D have findings notes and chosen approaches.
- No visible UI change except the bug fixes.
- All existing fixtures green.

---

## M1: Workbench shell (the new look)

### 1.1 Design system

- [x] **Tokens in `ImpulseKit/Themes/UITokens.swift`:**
  - spacing, radii, row heights per density, font roles;
  - `ThemeSchema` gains an optional `[ui]` table (§5.3) with derivation from the palette and semantic seeds;
  - `surface_style` is folded into it.
  - Tests: every built-in theme derives every UI token; the existing `ThemeParityTests` stay unchanged and green.
- [x] **Components in `Sources/ImpulseApp/DesignSystem/`:**
  - `IconButton`, `Chip`, `SplitButton`, `SegmentedTabs`;
  - `ListRow` (hover actions plus a ⌘K row action menu), `SectionHeader`;
  - `Badge`, `StatusDot`, `ProgressRing`, `KeyHint`;
  - `ThemedTextField`, `ThemedTextEditor`;
  - `Toast` (with Undo), `ConfirmInline`, `EmptyState`, `PaneHeader`.
- [x] **Icons:**
  - Vendor a Lucide subset (ISC) as template PDFs or SVGs with a `scripts/vendor-icons.sh`.
  - Add `Icon` enum mapping.
  - Material file icons stay for the tree.
- [x] **Component Gallery:** a debug-only window (Debug menu in Dev builds) showing every component × every built-in theme × light/dark. *("Component Gallery" in the palette of Dev builds; filter dark/light, toggle Increase Contrast; `gallery=light+contrast` for snapshots)*
- [x] **Accessibility baseline:** focus rings, `accessibilityLabel` on all icon-only buttons, Reduce Motion / Contrast / Transparency honored. (icon buttons labeled; Reduce Motion honored; Increase Contrast firms up the chrome palette live; the chrome has no translucent materials for Reduce Transparency; keyboard-focused lists ring their selected row)

### 1.2 Window and layout

- [x] Implement the chosen titlebar approach from Spike A: `ChromeBarView` with traffic-light inset, dock toggles, breadcrumb placeholder, tab strip, ＋ menu, palette button, inbox bell placeholder, diff pill, right-dock toggle (§5.1).
- [x] **Tab strip:**
  - compact pills, attention dot, progress ring (from 0.1 OSC 9;4), pin section, close on hover;
  - double-click to rename, color from the ANSI palette;
  - drag to reorder;
  - AXTabGroup.
  - Replaces both `TabBarView` and `SidebarTabListView`. Vertical tabs return in M2 as a workspace-row expansion.
- [x] **`WorkbenchView` from Spike B:**
  - left, right and bottom docks with persisted size and visibility;
  - `NavigationSplitView` removed from `MainContentView.swift`;
  - `currentSidebarWidth()` hack (`MainWindow.swift:2820-2843`) deleted.
- [x] **Left dock tool panel** with `SegmentedTabs`: Files (current tree), Search (current `SearchPanelView`), Changes (placeholder until M4).
- [x] **Status bar** (always visible, §5.1): branch, problems placeholder, agent summary placeholder, ports placeholder, LSP dot, editor info. The `TerminalContextBarView` / `StatusBarView` swap in `MainContentView.swift:58-67` goes away. The terminal input stays at the bottom of the terminal tab until M2 moves it into panes.
- [x] **`window.allowsAutomaticWindowTabbing = false`.** Delete the window background workaround "so the titlebar blends" (`MainWindow.swift:1070`), because the chrome is now drawn.
- [x] **Replace the `runModal` NSAlerts with themed `ConfirmInline` or sheets:** Go to Line, Rename, Discard, Trash, binary file, LSP install result. (quitting with unsaved work stays app-modal)

### 1.3 Command palette v2

- [x] **`ImpulseKit/FuzzyScorer.swift`:** a subsequence scorer with boundary and camel bonuses, consecutive-match bonus and path-segment awareness. Tests include file-path ranking cases.
- [x] **Palette UI on `OverlayHost`:**
  - themed (fixes the dead `applyTheme`, `CommandPalette.swift:588`);
  - prefix modes `>`, `:`, `%`, `b:`, `w:`, `t:`, `h:`;
  - `@` and `#` arrive in M8;
  - recents persisted to Application Support.
- [x] **⌘P quick open** is a real fuzzy file finder over the workspace file index, gitignore-aware via existing `FileSearch`. Today it aliases project search (`MainWindow.swift:1610-1615`).
- [x] **Switch over** from `CommandPalette.filterItems`. Update or retire `Phase1ParityTests` palette fixtures in the same commit, with the reason: "palette moves to fuzzy scoring; old substring scorer retired".

### 1.4 Settings and keybindings surfaces

- [x] **Settings as a tab surface** (SwiftUI, themed): (palette deep link is the `set:` mode)
  - search, categories, modified markers, reset-to-default per key;
  - deep links from the palette (`settings: <query>`).
  - Retire `SettingsWindow.swift` (1863 lines) and `SettingsFormSheet.swift`.
- [x] **JSON Schema for `settings.json`**, generated from `Settings` metadata. "Open settings.json" opens in Monaco with schema validation.
- [x] **Keybindings surface:** search by command or key, record shortcut, conflict warnings, `when` context column, custom command bindings (today's `custom_keybindings`). (no `when` column: shortcuts aren't context-scoped yet)
- [x] Expose the settings that exist without UI (§10.3).

**M1 definition of done:**

- The new chrome is the default.
- No NavigationSplitView, NSToolbar glass items, HUD palette, or `runModal` alerts remain.
- Every theme renders the chrome legibly (WCAG AA on text tokens; extend the existing contrast audit).
- Component Gallery reviewed.
- Release notes explain the change.

---

## M2: Workspaces, tabs and panes

### 2.1 Models (pure parts in ImpulseKit)

- [x] **`ImpulseKit/Layout/LayoutTree.swift`:** split/leaf tree with ratios, plus split, close, move, swap, zoom, equalize and neighbor-in-direction. Codable. Tests cover every operation and serialization round-trips.
- [x] **`WorkspaceModel` (ImpulseApp):**
  - id, root URL, repo ref, display name, tabs, active tab, dock state, env (port block later), agent sessions;
  - `AppModel` → `WindowModel` → `[WorkspaceModel]`;
  - rewrites `WindowModel.swift` (267 lines) around this hierarchy.
- [ ] **`Surface` protocol** (§4), with `TerminalSurface`, `EditorSurface`, `ReviewSurface` (wraps today's `DiffReviewTab` until M4), `ImagePreviewSurface`, `SettingsSurface`.
- [x] **Retire the parallel arrays in `TabManager.swift`** (`tabs`, `pinnedTabs`, `tabUniqueIds`, `tabCloseReturnIds`, `openFilePaths` plus `editorTabsByPath`) in favor of a model-driven `TabModel` with one surface index.

### 2.2 UI

- [x] **Workspaces section** at the top of the left dock:
  - grouped by repo, resizable (reuse `VerticalResizeHandle`);
  - each row shows status dot, name, branch, ±count, attention badge;
  - expanding a row lists its tabs and panes (vertical-tabs mode);
  - hover card with full metadata.
  - Context menu: rename, color, reveal in Finder, open in new window, close workspace.
- [x] **Opening a folder** (⌘O, drag onto Dock icon, `application(openFiles:)`) creates or focuses its workspace. Linked worktrees of an open repo appear automatically under that repo; listing is via `git_worktree_list`, and creation comes in M6.
- [x] **Scratch workspace** for `~`.
- [x] **Breadcrumb in the titlebar:** repo › workspace and branch, opening the workspace/branch switcher. Branch actions are wired fully in M3/M4.
- [x] **Panes:**
  - ⌘D / ⌘⇧D split; ⌥⌘arrows focus; ⌘[ ⌘] cycle; ⌘⇧↩ zoom; equalize;
  - drag the pane header to move it, or onto the tab strip to pop it out;
  - inactive-pane dimming setting;
  - accent focus marker.
- [x] **Move the terminal input into each terminal pane** (`TerminalPaneView` = grid + chips row + input). `TerminalContextBarView`'s chip logic moves to the pane, and `WindowModel`'s window-level input fields become per-pane state. Preserve `inputBarFocusToken`, `terminalDirectInteraction`, `passwordInputActive` and completion anchoring through `FocusCoordinator`.
- [ ] **Editor panes:** *(preview tabs done: a single click in the file tree shows the file in an italic tab the next click reuses, kept by double-click, edit, pin or split; stacked editors with a mini tab header per pane still to do)*
  - stack multiple editor surfaces with a mini tab header (preview-mode italic tab for single-click opens, Nova/VS Code style);
  - opening a file targets the focused tab's editor pane, or creates one to the right (setting: right split / new tab).
- [x] **⌘Z restores** a closed pane or tab within 10 s (keep surface state alive; PTYs are not kept alive). After that, "Reopen closed tab" stays (existing `closedTabs` stack).
- [x] **`TerminalSessionHub`** from Spike D wired for all terminals. Hidden tabs keep delivering events (§6.1).

### 2.3 Session restore v2

- [x] **`SessionState` version 2** stores workspaces, tabs, `LayoutTree`, surfaces, dock state, and editor cursor/scroll/folds. Migrate from v1, including the dormant `panes` / `paneLayout` fields.
- [x] **Restore all windows**, not only the `activeWindow` as today (`AppDelegate.swift:74`).
- [x] **Scrollback restore** for the last 2,000 rendered lines per terminal.
  - New FFI `impulse_terminal_rendered_text(term, start_row, end_row, with_sgr)`.
  - Inject into the new terminal before the shell starts, under a dimmed "Restored" separator.
  - Setting, default on.

**M2 definition of done:**

- Splits with any surface work.
- Workspaces persist across relaunch.
- Background tabs deliver notifications.
- 10 idle terminals stay under 0.5% CPU (§11).
- Focus tests cover the input ↔ grid ↔ TUI ↔ WebView transitions.

---

## M3: Git core (Track B; can start right after M0)

All of this lives in `Sources/ImpulseGit/`. Each task adds scenario-repo tests that use the `git` CLI as the oracle.

- [x] **Scenario repos** (`Tests/ImpulseGitTests/ScenarioRepo.swift` variants): *(in `GitScenarioTests.swift`, checked against `git status --porcelain -z`; a non-UTF-8 path can't exist on APFS, so Unicode and spaces stand in)*
  - staged-only; mixed staged and unstaged in the same file;
  - rename plus edit; binary; too-large;
  - merge conflict; rebase conflict (stopped at step 2/3);
  - linked worktree; submodule;
  - detached HEAD; unborn HEAD;
  - `.gitignore`d files; CRLF; non-UTF-8 path.

  The existing scenario and fixtures stay untouched.

- [x] **`GitRepository`:** one per repo root, with a serial queue and a cached `git_repository*`. Add public invalidation of `RepoCache`. Handle `git_repository_commondir` for worktrees.
- [x] **`RepoSnapshot`:**
  - branch, HEAD oid, upstream, ahead/behind (`git_graph_ahead_behind`);
  - operation state and step, read from gitdir files: `MERGE_HEAD`, `rebase-merge/msgnum` + `end`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `BISECT_LOG`;
  - `staged[]`, `unstaged[]`, `untracked[]` and `conflicted[]`, each with +/− counts, computed lazily per file;
  - stash count;
  - worktrees.

  No `UPDATE_INDEX`. Oracle: `git status --porcelain=v2 --branch -z` and `git rev-list --left-right --count`.

- [x] **`DiffScope`** (ImpulseKit) and `GitRepository.diff(scope:pathspec:options:)`:
  - covers unstaged, staged, uncommitted, branch(merge-base), commit, range, stash, and checkpoint (M6);
  - pathspec-limited;
  - options: context lines, ignore whitespace, ignore EOL;
  - stable hunk ids.
  - Keep the existing `changedFiles` / `fileHunks` / `diffMarkers` APIs and their fixtures. They become thin wrappers or stay as-is for parity until callers move.
  - Oracle: `git diff [--cached] --numstat -z` and `git diff -U<n>`.
- [x] **`RepoWatcher`:**
  - FSEvents stream on the worktree root plus the gitdir and commondir (HEAD, refs, index, packed-refs, operation files);
  - debounced; classifies changes as worktree vs index vs refs;
  - replaces the DispatchSource index watcher plus the 10 s poll in `FileTreeDataController.swift:399-487` and the per-expand whole-repo status (`WindowModel.swift:257`).
- [x] **`PatchBuilder`** (pure):
  - inputs: hunks plus a line selection; output: a unified patch, forward or reverse;
  - handles no-newline-at-EOF, context recount, and partial selections inside mixed hunks.
  - Tests round-trip through `git apply --check --cached`.
- [x] **Mutations through `GitCLI`:**
  - staging: stage/unstage path(s), stage/unstage patch (`git apply --cached [-R]`), revert patch in the workdir (`git apply -R`);
  - commit (`git commit -F -` with amend, sign-off, `--no-verify`, `--allow-empty` guard); uncommit (`reset --soft HEAD^`);
  - branches: create, switch, rename, delete, set upstream;
  - stash: save (message, include untracked, keep index), apply, pop, drop;
  - remote: fetch, pull (ff-only or rebase), push / publish / force-with-lease;
  - worktree add/remove/prune;
  - merge / rebase / cherry-pick / revert / reset, plus continue/skip/abort.
- [x] **Retire libgit2 `commitAll` and `discardPath` / `discardFileChanges`.** Discard of a tracked file becomes `git restore`; untracked files go to the Trash via the app layer. Update `GitParityTests` for `commitAll` and discard in the same commit, explaining the CLI move (hooks, signing, LFS).
- [x] **Oplog snapshots** (`Snapshots.swift`):
  - `snapshot(reason:)` writes `refs/impulse/oplog/<ts>-<reason>` (index tree plus worktree-including-untracked tree, built in a temp `GIT_INDEX_FILE`);
  - `listSnapshots`, `restore(snapshot)`;
  - retention pruning.
  - Tests: restore after a discard reproduces the exact pre-discard workdir and index.
- [x] **Blame:** whole-file blame cached by blob id; `lineBlame` becomes a lookup. Keep its fixture.
- [x] **Log and graph data:** a revwalk with topo+time order, parents and ref decorations, paged. Oracle: `git log --format=%H%x00%P%x00%D --topo-order`. (read with the git CLI in `GitLog`, paged, tested against real repositories)
- [x] **App-side `GitRepositoryState`** (`@Observable`, one per repo, shared across windows):
  - fed by snapshot plus watcher;
  - replaces the branch cache (`MainWindow.swift:2600`), the 15 s branch TTL (`TabManager.swift:978`), `refreshReviewSummary` (`MainWindow.swift:982`) and the file-tree git polling.
  - The file tree, chips, status bar and breadcrumb all read from it.

**M3 definition of done:**

- No full-repo diff work on the main thread.
- Background refresh never writes the index.
- Snapshot refresh is under 150 ms on a 50k-file repo (§11).
- All mutations have oracle tests.
- Commits run hooks and signing.

---

## M4: Changes panel, Review v2, editor git

### 4.1 Changes panel (§7.2)

- [x] SwiftUI panel in the left dock Changes tab, showing:
  - branch switcher button;
  - sync split button (state-morphing: Commit → Push → Publish → Create PR → PR #n; the PR states activate in M7);
  - operation banner;
  - sections Conflicts / Staged / Changes / Untracked / Stashes / Worktrees / Recent commits;
  - list or tree toggle; multi-select; hover actions.
- [x] **Panel-focus keys:** `space`, `⏎`, `⌫`, `⌘⌫`, `⌘↩`, `⌘⇧↩`, `⇧Esc`. All are registered as commands with a `gitPanelFocus` context. *(Done in the panel itself: ↑/↓ move, space stages/unstages (or marks a conflict resolved), ⏎ opens the diff, ⌘⏎ opens the file, ⌫ discards, Esc returns to the terminal; Show Changes focuses the list. ⌘↩/⌘⇧↩ stay with the commit composer.)*
- [x] **Commit composer:**
  - subject/body with 50/72 guides; `commit.template`;
  - message history (↑ in an empty field);
  - amend (prefills the previous message), sign-off, skip-hooks;
  - "Commit tracked changes?" confirmation when nothing is staged;
  - Uncommit after commit;
  - hook output shown on failure.
- [x] **Undo:** ⌘Z in git focus restores the latest oplog snapshot. Toasts with Undo after discard and revert.
- [x] **Diff pill** (titlebar) and the input chip show the live `±files +add −del` for the workspace.

### 4.2 Review surface v2 (§7.3)

- [x] **Protocol v2** (`Editor/ReviewProtocol.swift` and `web/review.js`):
  - `Render{generation, scope, files[{id, path, oldPath, status, added, removed, binary, contentHash, staged?}]}`;
  - `UpdateFile`, `RemoveFile`, `SetHunks{fileId, hunks[{id, …}], contentHash}`;
  - `SetViewed`, `SetComments`, `SetLayout{unified|split, whitespace, context}`.
  - JS → Swift:
    - `RequestDiff`, `ExpandContext{fileId, hunkId, direction, lines}`;
    - `StageHunk` / `UnstageHunk` / `RevertHunk{fileId, hunkId}`;
    - `StageLines` / `RevertLines{fileId, hunkId, lineIds[]}`;
    - `ToggleViewed`, `AddComment` / `EditComment` / `DeleteComment`;
    - `OpenInEditor{path, line}`, `SendComments`, `Refresh` (today defined but never sent).
  - Drop the duplicate Codable structs and the JSON round-trip in `Bridge/ImpulseCore.swift:124, 345-430`; encode ImpulseGit types directly.
- [x] **Navigator pane:** file tree with status, +/−, Viewed checkbox (auto-reset on content-hash change, "changed since viewed" badge), comment count, filter (`T`), progress count.
- [x] **Body:**
  - sticky headers;
  - row-level virtualization inside large files;
  - per-side tokenization using full old and new text (fixes state bleed, `review.js:477-513`);
  - split view;
  - whitespace toggle;
  - expandable context;
  - keep expansion and scroll across updates;
  - an "Updated — show" chip when the visible file changes.
- [x] **Actions:** hunk and line-selection stage/unstage/revert, file stage/unstage/discard, all/none; Undo toasts.
- [x] **Keys (review focus):** `j`/`k`, `n`/`p`, `N`, `⌘Y`, `⌘⇧Y`, `⌘⌥Z`, `v`, `c`, `o`, `s`, `⌘↩`.
- [x] **Scope selector:** Uncommitted / Unstaged / Staged / vs default branch (auto-detect `origin/HEAD` → main/master) / vs branch… / last commit / commit range / stash. "Last agent turn" and "Since my last review" arrive in M6.
- [x] **Comments:**
  - anchored by (scope, path, side, range, snippet hash);
  - stored at `~/Library/Application Support/impulse/review/<repo-hash>/<branch>.json`;
  - an Outdated section;
  - "Copy as prompt" and "Export markdown" (agent delivery arrives in M6).
  - The anchoring model lives in ImpulseKit with tests.
- [ ] **Placement:** docked in the right dock (live) or opened as a tab or pane; ⌘⇧G toggles. Today's tab is one per repo root, isn't persisted and can't be reopened; v2 is persisted and per workspace. *(Review and History tabs now persist with the session, scope included, and come back with Reopen Closed Tab / ⌘Z; the right-dock placement is still to do)*
- [x] **"Open in diff editor":** Monaco `createDiffEditor` single-file surface, side-by-side or inline, `hideUnchangedRegions`, modified side editable and saving to disk. *(A diff view inside the editor tab, index ↔ working copy, sharing the live model: Toggle Diff View, the peek's Diff button, Changes panel ⌥⏎ / context menu, the review's Edit Diff.)*

### 4.3 Editor git integration (§7.4)

- [x] **Gutter markers from the live buffer.** Send the base blob (index or HEAD per setting) once per file. Compute the line diff in JS on edit (debounced), or in Swift from buffer deltas. Today markers come from disk and go stale while typing.
- [x] **Clickable markers** open a peek diff view zone with Stage / Revert / Next / Prev.
- [x] **Inline current-line blame** (ghost text, setting) and a blame gutter toggle. Click goes to the commit (History, M7). Until then, show the commit summary in a hover card.
- [x] **Branch switcher popover** (⌃⌘B, breadcrumb, chip):
  - fuzzy; recent first; local/remote grouped;
  - create branch;
  - dirty-tree choices (stash & switch / carry / new worktree / cancel).
  - Replaces `BranchPickerView`.

**M4 definition of done:**

- A full stage/revert/commit loop is possible without a terminal.
- Review of 200 files / 20k lines meets §11.
- Every destructive action is undoable.
- Review survives live updates without losing place.

---

## M5: Terminal power (Track C)

### 5.1 Input editor v2 (§6.2)

- [x] **`CommandEditorView`:** a TextKit 2 `NSTextView` subclass.
  - Multi-line (⇧⏎); paste preserves newlines.
  - Ghost text via a temporary attribute or overlay.
  - IME and dictation; Emacs bindings; undo.
  - Replaces the SwiftUI `TextField` in `TerminalContextBarView.swift:249-321`.
  - Keeps password `SecureField` behavior via ECHO detection.
- [x] **Highlighting** from `ShellParser` tokens. Unknown-command dashed underline, resolved against a `PATH` / alias / function cache captured from the shell through an OSC 6973 extension emitted at prompt time, size-capped.
- [x] **Completion engine:** (specs are hand-written in `CompletionSpecs.swift` for the common tools rather than vendored from withfig; generators for git refs, paths, package.json scripts, Make/just targets, ssh hosts; the menu stays prefix-matched)
  - `scripts/vendor-completion-specs.sh` exports a curated JSON subset of withfig/autocomplete (MIT) into `ImpulseKit/Resources/CompletionSpecs/`.
  - A spec interpreter for subcommands, options and args.
  - Native generators: git refs, files, package.json scripts, Makefile and justfile targets, ssh hosts.
  - The fuzzy menu runs on `OverlayHost` with descriptions.
  - Keep `InputCompletionTests`; add spec tests.
  - Move `onInputSuggestion` off the main thread, where today it pulls 500 history entries per keystroke.
- [x] **Native shell completion bridge** (opt-in): fish `complete -C`, bash `compgen` (with bash-completion if present), zsh zpty capture (research). Results merge with spec results. *(fish only, behind "Ask the shell for completions": bash's compgen adds nothing over the built-in specs outside an interactive shell, and zsh capture needs a pty session; left for later)*
- [x] **Persistent history** (SQLite via the system `libsqlite3`):
  - shared across tabs;
  - metadata: cwd, exit, duration, branch (populate `git_branch`, always nil today at `backend.rs:568`), timestamp;
  - space-prefixed commands are skipped;
  - optional import of zsh/bash/fish history.
  - Rust `history.rs` stays the per-session ranking source, or moves to Swift. Decide in the task and record the decision.
- [x] **⌃R history panel:** fuzzy, with filters for cwd, repo, failed and today. Replaces `TerminalHistoryPicker`. Has an option to defer to the shell's own ⌃R binding (atuin/fzf).
- [x] **Classic mode** (D6) as a first-class setting: "Input: Impulse editor | Shell prompt". Wire OSC 133;B so prompt and command are separable. (the Input bar setting in Settings › Terminal turns the Impulse editor off)

### 5.2 Blocks v2 (§6.3)

- [x] **Native block header rows** with inline chips: exit, duration, cwd when changed, branch. Hover toolbar: copy, rerun, filter, bookmark, send to agent (M6), overflow. (exit/duration chips on the prompt row; cwd and branch come from the shell's own prompt; toolbar has copy, run again, send to agent and a menu with bookmark)
- [x] **Block selection:** ⌘↑, ↑/↓, ⇧-extend, ⌘-click; copy selection; Esc back to input.
- [ ] **Fold/collapse output**; "last N lines" mode. (not started: the renderer draws only the viewport rows, so folding needs virtualized rendering first)
- [ ] **Block filter** (⌥⇧F): regex, case, invert, context lines. Rendering filters rows in the overlay layer and is non-destructive. (not started: same virtualization dependency as folding)
- [x] **Find v2:** (block scope not done)
  - match count, case/regex/whole-word toggles, block scope;
  - themed bar in the pane header instead of the AppKit `NSSearchField` bar (`MainWindow.swift:793-980`);
  - search results stop being discarded (`TerminalBackend.swift:681-684`).
- [x] **Bookmarks** (⌘⇧K) with scrollbar markers and ⌥↑/↓. (ribbon on the block; next/previous are menu commands, unbound by default; no scrollbar to mark)
- [x] **Rendered-text block output** via the FFI from 2.3. `command_blocks()` stops serializing all output on every call: add `impulse_terminal_command_block(id)` and a metadata-only list.
- [x] **Performance:**
  - `block_overlay()` gets a generation counter, so it is cached until blocks or viewport change, instead of JSON every frame and every mouse move (`TerminalRenderer.swift:2361-2380`);
  - `mode()` is cached;
  - `gridPoint()` stops taking a full snapshot per mouse event (`TerminalRenderer.swift:2793-2802`).

### 5.3 Links, hints, protocols, notifications

- [x] **Path detection** (`file:line:col` and common stack-trace formats) on hover, opening in the Impulse editor. OSC 8 `file://` routes to the editor.
- [x] **Hints mode** (⌘⇧Space): labels for URLs, paths, SHAs and ports; open, copy or insert.
- [x] **Protocols:**
  - OSC 9;4 progress UI (tab ring, sidebar);
  - OSC 21337 status;
  - OSC 99 notifications.
  - Tests in `osc_scanner.rs`.
- [x] **Grapheme clusters** (zero-width and combining characters) and styled or colored underlines: snapshot format v2 with a side-table. Requires matching FFI and Swift `GridBufferReader` changes behind a version byte in the header. *(an extras table after the cells carries zero-width characters and SGR 58 underline colors; double, curly, dotted and dashed underlines draw; emoji with U+FE0F shrink into their cell. ZWJ sequences still split, since alacritty gives the joined emoji their own cells)*
- [x] **Kitty keyboard protocol** (progressive enhancement flags) in `KeyEncoder.swift`. *(alacritty_terminal tracks the flags; they ride in bits 11–15 of the mode word; the encoder lives in ImpulseKit with tests; releases via keyUp. Typing with a real keyboard in fish/Neovim still needs a manual check.)*
- [x] **`UNUserNotificationCenter`** (§6.6):
  - request authorization on first need;
  - thread per workspace; click focuses the pane;
  - remove delivered notifications on focus;
  - Dock badge count.
  - Wire long-command-finished, bell, OSC 9/777/99.

**M5 definition of done:**

- Multi-line editing with highlighting and spec completions.
- History persists across relaunch.
- Blocks selectable, filterable and foldable.
- Paths open in the editor.
- Notifications are native and land on the right pane.
- Keystroke latency meets §11.

---

## M6: Agent workflows

- [x] **Foreground process FFI.** `impulse_terminal_foreground_pid` (via `tcgetpgrp` on the PTY master). Swift resolves the path and argv (`proc_pidpath`, `KERN_PROCARGS2`).
- [x] **`ImpulseKit/Agents/`:**
  - `KnownAgents` table (user-extendable in settings);
  - `AgentStateMachine` (idle/working/needsInput/done/error) driven by typed events, with debouncing and timeouts.
  - Tests are event-sequence fixtures.
- [x] **Agent panes** (§8.3):
  - agent icon in the tab, sidebar row and pane header;
  - footer toolbelt (status, compose, diff, checkpoints) replacing the input editor while the TUI runs.
- [x] **`impulse` CLI and socket** (§8.6):
  - new executable target `impulse-cli`, bundled at `Contents/MacOS/impulse` by `build.sh`;
  - `SocketServer` in the app; per-pane tokens; env injection in the shell integration scripts;
  - commands: `open`, `edit --wait`, `split`, `tab new`, `notify`, `status`, `checkpoint`, `review`, `task`, `send`;
  - "Install command line tool…" action, with consent.
  - Tests: protocol encode/decode (ImpulseKit), token checks.
- [x] **Hook installers** with a diff preview and explicit confirmation:
  - Claude Code (`UserPromptSubmit`, `Notification`, `Stop`, `SessionStart`), user-level or project-local;
  - Codex `notify`;
  - uninstall.
  - Record agent session ids for **Resume** on restore.
- [x] **Opt-in `$VISUAL` / `$EDITOR`** = `impulse edit --wait` inside Impulse terminals. The tab closes, then the CLI returns.
- [x] **Composer overlay (⌘I):** (image paste saves a PNG and inserts its path)
  - `CommandEditorView` above the TUI;
  - @file mentions from the workspace index; image paste; history;
  - send via bracketed paste (Enter optional);
  - auto-show on idle/needs-input (setting).
- [x] **Inbox (§8.7):** titlebar bell popover, ⌘⇧U next needs-input, ⌘⇧I open, status-bar summary, snooze, filters. Sidebar rows and tabs show status dots with AX labels.
- [x] **Checkpoints:**
  - created on `UserPromptSubmit`/`Stop` hooks, `impulse checkpoint`, or manually;
  - stored in `refs/impulse/checkpoints/<workspace-id>/<n>` (reuses oplog snapshot code);
  - pane footer shows the count; timeline popover.
  - Review scopes: **Last agent turn** and **checkpoint n→m**.
  - Restore a checkpoint (oplog snapshot first).
- [x] **"Since my last review" scope.** Record the content hashes when the user sends comments or marks everything viewed.
- [x] **Review → agent delivery:**
  - target picker (agent panes in this workspace, last-used first);
  - queued delivery while the agent is "working";
  - send to composer instead (setting).
  - Same path for blocks, editor selections, diagnostics and files ("Send to agent").
- [x] **`.impulse/project.toml`** (ImpulseKit parser plus JSON Schema): worktree location and copy list, scripts, actions, layouts with params. Per-repo trust prompt before running any command from it. Palette `a:` mode and an actions menu. (no schema or layouts yet; worktree location stays the sibling folder)
- [x] **Worktree tasks:** New task (branch name generator, worktree add, copy list / `.worktreeinclude`, setup scripts, open the layout with the agent command); Archive (dirty/unpushed check, archive scripts, worktree remove, restorable list).
- [x] **Ports:** a process-tree walk per workspace (`proc_pidinfo` socket info) on a 2 s timer while visible. Chips in the status bar and workspace row. Optional `IMPULSE_PORT` block per workspace.

**M6 definition of done:** this end-to-end scenario works without leaving Impulse:

1. Start a task.
2. The agent works.
3. Get notified.
4. Review the last turn.
5. Stage half and comment on the rest.
6. Send to the agent.
7. Re-review "since last review".
8. Commit and push.
9. Archive.

---

## M7: History, conflicts, branches, PRs

- [x] **History surface:** (lane graph, ref chips, incoming/outgoing markers, paging; the filter field takes free text plus `author:` `path:` `since:` `until:` tokens that go to `git log`, with presets; commits from before the fork point off the default branch are dimmed and the fork point is marked)
  - lane graph (`ImpulseKit/Git/GraphLayout.swift`, tested against `git log --graph` shapes);
  - ref chips; incoming/outgoing markers; fork-point dimming;
  - search and filters (message, author, SHA, path, date); paging.
- [x] **Commit details:** metadata plus a read-only multi-file diff (review renderer).
- [x] **Actions:** checkout, branch here, cherry-pick, revert, reset soft/mixed/hard (with oplog), copy SHA, compare with the working tree, compare two commits.
- [x] **File history** from the tree or editor (path-filtered History). Blame click-through.
- [x] **Conflicts:**
  - operation banner (merge/rebase/cherry-pick/revert with step) with Continue/Skip/Abort;
  - conflict list;
  - inline Accept current / incoming / both in Monaco; counter with next/previous;
  - Mark resolved (stage).
  - "Ask agent to resolve" sends the conflict hunks to an agent pane.
- [x] **Stash management:** view as multi-diff, apply/pop/drop (undoable), partial stash from selection (later).
- [x] **Branch management:** rename, delete with merged/stale badges, upstream, publish.
- [x] **`gh` integration (optional, detected):**
  - PR chip: state, draft, review decision, checks;
  - checks polling with backoff;
  - Create PR (draft, title and body from commits);
  - open in browser;
  - notification when checks finish;
  - `gh pr checkout` into a new worktree.
- [x] **Import PR review threads** (`gh api graphql` `reviewThreads`) into the Review surface as comments with "View on GitHub" and "Send to agent".
- [ ] **Later (separate plan):** interactive rebase editor, 3-way merge editor with "apply non-conflicting", operation history timeline UI.

---

## M8: Editor depth, accessibility, performance, docs

- [ ] **Multi-model editor panes:** one WKWebView per pane, with view-state save/restore. `EditorWebViewPool` pre-warms per pane. Measure against §11.
- [x] **Problems panel** (bottom dock): keep diagnostics for all URIs; filters; status-bar counts; send to agent. (a tool tab rather than a bottom dock)
- [x] **Outline** (right dock) and breadcrumbs from `documentSymbol`. Palette `@` symbols and `#` workspace symbols. (no breadcrumbs yet)
- [x] **LSP coverage:** *(requests now wait for answers off the serial queue, which keeps only the send order; notifications go to every server for the language; sourcekit-lsp added for Swift; inlay hints have a setting)*
  - `workspace/applyEdit` and multi-file `WorkspaceEdit` applied in Swift with one undo group;
  - command-only code actions via `executeCommand`;
  - document highlight, inlay hints, type definition/implementation.
  - `showMessage` and `$/progress` go to toasts and the status bar.
  - Move LSP off the single global serial `lspQueue` (`AppDelegate.swift:24`) to per-server queues.
- [x] **Project-wide find & replace** with preview.
- [x] **Optional vim mode** (monaco-vim, vendored). *(monaco-vim 0.4.4 via `scripts/vendor-monaco-vim.sh`, loaded on demand behind "Vim keybindings"; it was built against an older Monaco, so it needs some real typing to confirm nothing's off)*
- [x] **Side-by-side Markdown preview**, with "Run in terminal" buttons on shell code blocks.
- [x] **Quick terminal:** a global-hotkey dropdown `NSPanel` (Carbon `RegisterEventHotKey`, `.canJoinAllSpaces`, `.fullScreenAuxiliary`) bound to a workspace. (starts in the front window's folder; off by default; the hotkey itself needs a manual check)
- [ ] **Accessibility pass:** (partly: terminal text area and agent announcements done; VoiceOver walkthrough and Full Keyboard Access need a person at the Mac)
  - VoiceOver walkthrough of every surface;
  - terminal AXTextArea (value, selected range, line-for-index);
  - announcements for agent status changes;
  - Full Keyboard Access.
- [ ] **Performance pass:** measure every §11 budget and fix regressions. Spike a Metal glyph-atlas renderer with ligatures, as a separate plan if pursued. *(git budgets measured with `GitPerformanceTests` (set `IMPULSE_PERF_REPO`) on a 50k-file repo with 200 changed files / 20k lines: warm snapshot 132 ms after caching per-file line counts (was 170 ms), review file list 179 ms and list + first 5 diffs 250 ms, debug build; launch, idle CPU, keystroke latency and memory need the app running on screen)*
- [x] **Services:** the terminal implements `NSServicesMenuRequestor`; add a Finder service "New Impulse Workspace Here". (verified the bundle's Info.plist; the service itself needs a manual check in Finder)
- [x] **Docs:** *(CLAUDE.md, README feature list and CHANGELOG updated; superseded items in the May plan marked; README screenshots still to retake from a real project)*
  - update `CLAUDE.md`: architecture tree, workbench/surface model, CommandRegistry, GitCLI write path, `impulse` CLI, revised NSToolbar note from Spike A;
  - update `README.md` with screenshots and the feature list;
  - update `CHANGELOG.md`;
  - retire superseded items in `docs/superpowers/plans/2026-05-04-warp-inspired-improvements.md`: launch configs → `project.toml`, block search → M5, worktrees → M6.

---

## Cross-cutting checklist (every milestone)

- [ ] `swift build`, `DEVELOPER_DIR=… swift test`, `cargo test -p impulse-terminal`, `cargo clippy`. Revert `cargo fmt` collateral in unrelated files.
- [ ] Every intentional fixture change is in the same commit, with the reason in the message.
- [ ] All built-in themes checked in the Component Gallery. Contrast audit green.
- [ ] Manual QA script run in Impulse Dev.app. Record findings in the milestone's notes.
- [ ] Measure §11 budgets touched by the milestone.
- [ ] Release notes draft for the milestone.

## Decisions (settled 2026-10-05)

1. **D2/D3:** workspaces in one window, sidebar grouped by repo, titlebar tabs for the active workspace. Vertical tabs are a setting.
2. **D5:** no AI features in the app at all. No command templates.
3. **D7:** Lucide icons in the chrome; Material file icons stay in the tree.
4. **UI font:** SF Pro Text (default), family configurable.
5. **Worktree location:** `../<repo>.worktrees/<branch>`.
6. **Hooks scope:** project-local `.claude/settings.local.json` by default, user-level as an option (decide finally in M6).
