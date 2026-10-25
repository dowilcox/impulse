# Rust app: code review, October 2026

A review of the `rust` branch (at 99f4468) for three things: features the
Swift app has that the Rust app lacks or does differently, bugs a user
would hit, and performance. Seven reviewers read the code in parallel, one
per area (terminal, editor, git surfaces, workbench and app shell, agents
and tasks), plus one across the app for performance and one for the core
crates' correctness and safety. Nothing was changed. Items already in
[rust-remaining.md](rust-remaining.md) are left out.

Each finding was checked against the code on both sides. The ones marked
**confirmed** were also checked a second time, and the editor crash was
reproduced in the running app. Paths are under `impulse-app/src/` unless
another crate is named.

## Progress

Every finding has been worked through. Still open, each noted in place
below:
- W2's edge resizing needs a check on a real Linux desktop.
- Keyboard folding waits on GPUI Kit (E15).
- A Services menu item needs the bundle's Info.plist (W10).
- On macOS, `osascript` notifications can't be withdrawn (A4).
- The macOS build is type-checked (`scripts/check-macos.sh`) but not yet
  run on a Mac.

Fixed: E1, E6 (crashes on multibyte text), G1 with G2, G6 and G7
(Review's stage, unstage and revert go through the workbench's git
actions: confirmation, snapshot and Undo for reverts, errors as dialogs,
busy marks kept through refreshes), G5, W3, E4 (with the BOM and Diff
View parts of E11 and E7), E3, W16.

Security: S1 to S9, and the writer half of P4 (the other half, below) (each language server has
a writer thread, so the UI never waits on one). Automatic snapshots
(agent turns and checkpoints, the "reviewed" mark, overlap checks) use the
restricted environment in folders that aren't trusted
(`safety::create_with`, `working_tree_commit(root, trusted)`).

Redraws: P1 (terminals and editors are cached views in their panes, and a
terminal the window isn't showing redraws the workbench only when
`TerminalView::chrome_key` changes), P2 (the working ring steps twelve
times a second from a shared clock, `Workbench::start_spinner`, instead of
an animation asking for every frame) and P10 (its ticks no longer redraw
every pane). The workbench's own chrome still renders on each of them; P5
(the file tree) is what makes that cheap.

Keys: E2 (the editor keeps Monaco's ⌘D, ⌥⌘↑/↓ and ⇧⌘↩, and the terminal
commands' keys, over the app's shortcuts while it has the keyboard, as
Swift's `EditorWebView.appCommandWins` does: `commands::editor_priority`)
and T17 (Shift with PageUp, PageDown, Home and End go to a program that
owns the screen, and scroll the scrollback at the shell).

Agents: A1 (`impulse unsaved` answers for the caller's workspace, the
Quick Terminal's for the window's editors under its folder, with paths
relative to it and `changedOnDisk` read from the file), A2 (a resolved
review comment is credited to the agent the pane runs), A3 (`open --wait`
on an image opens it and answers at once; a background `open_file` while
Impulse isn't in front flags the pane), A4 (desktop notifications and the
Dock's bounce wait for Impulse itself to be out of front, not just the
window, `agent_notify::app_in_front`; on Linux a terminal's notifications
are withdrawn when its attention clears and only its latest ten stay up,
by the id `notify-send --print-id` gives; `osascript` gives none, so on the Mac they stay), A5 (each window works out its
task states on its own, a refresh asked for meanwhile following the one
running), A6 (an agent hears "declined" only when the user was asked) and
A7 (the Quick Terminal's hooks with no window open get a window to answer
them from).

Core: every app git read goes through `cli::run` (a timeout and no
prompts; `command_history.rs`, the input bar's completions and
`workspace::git_branch`); blame never runs a textconv driver (`git blame`
does by default: `--no-textconv`, which also keeps its lines the file's
own), and the restricted environment empties any configured
`diff.external` and `diff.<driver>.textconv` or `.command`, so git
refuses instead of running them (Swift has the same gap); the MCP
launcher holds a CLI path with a `}` in a variable first (others keep the
documented form, so existing installs still match); `npm install` gives
up after 10 minutes and `npm --version` after 30 s, stopping everything
npm started, and the initialize handshake after 30 s and shutdown after
5, as Swift's; the control socket reads a request by its deadline in all
and serves at most 64 clients at once; and OSC 7's folder check runs on a
helper thread, waited for 500 ms, one at a time per terminal, so a mount
that stopped answering can't stall the terminal's output.

Terminal: T1 (the grid snapshot's extras are read: combining marks shape
with their cell's character, and SGR 58 underline colors are drawn), T2
(⌘C with nothing selected does nothing), T4 (pasting files or an image
into the input bar inserts their paths, `CommandEditorEvent::PasteOther`),
T6 (hovered and hinted paths are looked up on a background thread,
`FileChecks`, the hover and the hints asked again once known), T3 (password
mode's start and end empty the input bar, its undo history included,
`CommandEditor::reset`), T18 (mouse reports carry their button and
modifiers, a release pairs with its press, drags are reported once per
cell, wheel reports cap at 20, the middle button reports or pastes the
primary selection on Linux), T8 (input-method text is drawn at the
cursor even where the program hides it), T9 (OSC 52 store and load ask
the settings as each request comes), T10 (the branch chip opens the
branch switcher), T11 (on the Mac "left" and "right" read the side held
from AppKit's device flags, `key_encoder::alt_is_meta_held`; Linux keeps
Alt as Meta), T12 (Copy and Paste in terminals and their input bars are
bound from the settings' shortcuts), T13 (a shell that never sends OSC 7
has its folder read every 5 s, as Swift's `startCwdPolling`), T16 (a
terminal takes its window's handle each frame; the Quick Terminal opens
on the front window's display) and T19 (Option-click selects under mouse
reporting, Select All takes the screen, Ctrl+Right takes the
suggestion's next word on Linux, the block toolbar's tooltips, Clear's
comment, and no Ctrl+Shift+K Clear on Linux, where it's Bookmark Block).

Editor: E5 (settings.json keeps its schema's completions when settings
change), E7 (the Diff View's base follows staging, commits and
checkouts, `EditorTab::diff_base_changed`), E8 (the git overlay takes
clicks only where its hitbox is hovered, so menus and palettes over the
editor keep theirs), E9 (a file open in several editors, in any window,
is one document per server: one didOpen, versions that only go up, and
didClose with the last editor, `SharedDocument` in `lsp_hub.rs`), E13
(every completion reaches the editor as one edit in character columns:
`insertText` replaces what's typed, `additionalTextEdits` such as
auto-imports are folded in, `impulse_lsp::completion::editor_completions`),
E14 (code actions are asked with the diagnostics the range touches), E10
(Vim reads Ctrl-C, which leaves Insert and Visual, and its other keys
ahead of the app's commands, so Ctrl-F pages on Linux,
`editor_vim::command_overrides`), E15 (Select All Occurrences ⇧⌘L,
Toggle Block Comment ⇧⌥A (Ctrl+Shift+A on Linux), Go to Bracket ⇧⌘\,
Next and Previous Problem F8 and ⇧F8, Expand and Shrink Selection
(⌃⇧⌘→/←, Shift+Alt+→/← on Linux), parameter hints ⇧⌘Space, Cursor Undo
⌘U, cursors at line ends ⇧⌥I, Trim Trailing Whitespace ⌘K ⌘X and F1 for
the palette, in `editor_actions.rs` over `impulse_kit::line_edits`; Go to
Next and Previous Merge Conflict in the context menu while the file has
conflicts. Folding from the keyboard waits on GPUI Kit, whose folds only
a gutter click sets), and the rest of E11 (didSave goes out after the
formatter, with the formatted text; clicking the inline blame shows the
line's commit; font settings may list families, the first installed
used, `fonts.rs`; Format Document sends only Monaco's options; a lone CR
ends a line; definitions in the same document come back in character
columns; and linked editing of HTML and XML tag names, as Swift's Monaco
has it on, `impulse_kit::linked_tags`).

Windows: W1 (closing the last window saves the session and quits) and W2
(the Linux frame's edges and corners resize the window; to check on a
real desktop, since Xvfb's windows are decorated by the server), W4
(Scratch's tree keeps the folder it lists with the workspace's others,
so its folders expand; see P5), W5 (restored editors go back to their
cursor's line and column), W6 (closing the active tab goes back to the
tab that opened it, else the one to its right; closing a workspace goes
to the one used before it, `Workbench::recent_workspaces`) and W7 (each
window has its own sidebar visibility and width, saved with it; the
settings give new windows theirs, and changing them there applies to
every window), W8 (Restrict and Forget Trusted Folders stop language
servers in every window), W9 (the titlebar's tab strip scrolls and keeps
the active tab in view) and W10 (one app per user on Linux: a second
launch hands its folder, files and commands to the running one over
`$XDG_RUNTIME_DIR/impulse/instance.sock` and exits, `instance.rs`; paths
on the command line open, and the desktop entry passes `%F`; on macOS
files the system asks to open come in through GPUI's open-URLs handler.
A Services menu item still needs the bundle's Info.plist), W11 (Linux
leaves GNOME's Ctrl+Alt(+Shift) arrows alone: Focus and Resize Pane
have no keys there by default; Next Agent is Ctrl+Alt+U, clear of IBus's
Ctrl+Shift+U; Close Window shows Alt+F4), W12 (a tab of another
workspace clicked in the sidebar brings that workspace up as itself)
and W13 (`impulse` requests without a pane go to the front window; the
front window's size is kept for new windows, and the files open in
editors are kept in `open_files` and opened again when there's no
session; status-bar notices clear after 8 s; Save As while closing or
quitting carries on once the file is written; a reopened tab doesn't land
among pinned ones).

Performance: P3 (a repository notifies only when a refresh changed
something, `RepositoryState::generation`; editors re-read only then, and
blame again only when HEAD or the file on disk moved), P5 (the file
tree is a `uniform_list` building only the rows on screen; Scratch lists
a new folder once), P11 (the Changes panel is a `list` over
`ChangeItem`s, spliced where they changed; the Problems counts are kept
in the store) and P14 (saving no longer lists the tree again), P4 (servers that sync
incrementally get the changed range, `edits::content_change`, never
splitting a `\r\n`), P6 (word suggestions scan in the background, over
shared copies of the open documents' text), P7 (the preview's code
blocks are highlighted once per text and theme), P8 (a Review reads the
working tree again when the repository's watcher saw a change,
`RepositoryEvent::FilesChanged`, and only once it's drawn, so not while
hidden; its default scope comes from the repository's last refresh;
History refreshes on a new `generation`, not progress lines), P9 (a
terminal reads its grid again only after damage or a change of scroll
place or size, and the block overlay only when `block_overlay_key`
changes; blinks and hovers reuse them), P12 (an editor's change marks,
conflicts, links and bracket pairs are worked out in the background and
painted if the text hasn't moved on; the word diff keeps only each
step's live diagonals, and the Diff View skips word marks on lines past
`MAX_WORD_DIFF_LINE_LEN`, as Review does), P13 (task states refresh
1.5 s after a held repository's watcher sees a change,
`Workbench::schedule_task_refresh`, with a one-minute fallback instead of
a five-second poll; an agent's edit hooks share one reading of the
workspaces' changes for three seconds) and P15 (the render's watch sets
are compared by a key before any set is built, `tree_watch::paths_key`;
settings.json is read and hashed only when its stat changed; on Linux one
`gsettings monitor` per key replaces two processes every five seconds;
autosave writes the session in the background, an older save never
overwriting a newer; Replace All reads and writes files in the
background; switching workspaces takes the branch from the repository's
state; Review's scope menu reads the "reviewed" snapshot with the diffs;
sticky scroll copies out only the lines it looks at; Vim copies the
document once per key, not three times; word suggestions copy ropes;
hovers reuse the terminal's cached grid). Git: G3 (waiting
for a refresh always waits for one that starts after the call) and G4
(refreshes take the repository's operation lock), G8 (merging a task's
branch that's still moving asks first and merges the commit it has, as
Swift's `TaskActivity`), G9 (a stash opens in Review from its row) and
G10 (a partly aborted operation is a warning that stays; commit messages
are kept between launches in `commit-messages.json`; a file's History
shows that file in each commit; Review follows `review_context_lines`
and "Last commit" follows HEAD; a manual fetch restarts the background
fetch's interval; merging a commit shows its short SHA; Review's Copy
Path copies the repository-relative path; Revert buttons are red).

## What to fix first

1. **Crashes and lost work**: the editor crash on non-ASCII typing (E1),
   Review's Revert with no snapshot or Undo (G1), `--open` wiping the saved
   session (W3), and the reload that makes the next edit not count (E4).
2. **Security**: shell startup files in shared `/tmp` (S1), and the LSP
   reader's missing size cap (S3).
3. **Whole-window redraws**: every terminal's output, the agent spinner and
   every caret blink rebuild the entire window (P1, P2). Fixing this with
   cached views makes most other performance items much cheaper.
4. **Key conflicts**: app shortcuts beating the editor's own keys (E2) and
   the terminal's Shift+PageUp family never reaching programs (T17).
5. **The last window**: closing it doesn't quit and can't be undone from
   the Dock (W1), and Linux windows can't be resized from their edges (W2).

## Crashes and data loss

| | Finding | Where | Notes |
|---|---|---|---|
| E1 | **Typing some non-ASCII text crashes the app.** `typed_for_signature` slices the rope 4 bytes before the cursor, which panics inside a multibyte character ("Größe", "ééx"). Runs on every edit, with or without a language server. **Confirmed by reproducing it.** | `editor_tab.rs:1941` | Take the last character with `char_at` or a clipped offset. |
| G1 | **Review's Revert throws work away** (file, hunk or selected lines): no safety snapshot, no Undo, no confirmation, and not serialized with other git operations. An untracked file goes to the Trash without asking. **Confirmed.** | `review_view.rs:711-792` | Swift goes through `GitActions.discard` / `apply` with a snapshot and Undo (`ReviewSurface.swift:810-846`). |
| W3 | **Launching with `--open` or `--cwd` skips session restore, and the next quit overwrites the saved session** with only that window. **Confirmed.** | `main.rs:281` | Swift restores the session and opens the files on top (`AppDelegate.swift:150-165`). |
| E4 | **A reload that changes nothing leaves `reloading` set**, so the next edit isn't marked dirty, isn't auto-saved and can be lost on close. Triggered by `touch`, `git stash && git stash pop`, or an agent rewriting identical bytes. | `editor_tab.rs:1112-1126`, `:218-222` | `replace_buffer` (`:1474-1483`) does it right. |
| E3 | **Format Document applies stale edits**: the reply is applied to whatever the buffer holds by then, with no version check, so typing while it formats corrupts the buffer. | `editor_tab.rs:707-736`, `lsp_hub.rs:957-985` | |
| E6 | The minimap cuts long lines at 480 bytes and can panic inside a multibyte character (minimap is off by default). | `editor_minimap.rs:140-141` | |
| G5 | Pressing v in Review after a refresh error indexes an emptied file list and panics. | `review_view.rs:446-449`, `:871-874` | |
| W16 | Replace All only checks this window's editors for unsaved edits, so it can rewrite a file that has unsaved edits in another window. | `workbench_search.rs:101` | |
| W1 | **Closing the last window doesn't quit.** On Linux the process stays with no window and keeps the control socket; on macOS the Dock icon opens nothing. Closing a window doesn't save the session either. **Confirmed.** | `close_flow.rs:552`; no `on_window_closed` anywhere | Swift quits after the last window and saves in `windowWillClose`. |

## Security

| | Finding | Where | Notes |
|---|---|---|---|
| S1 | **High on shared Linux machines: zsh and bash startup files go to predictable names in `/tmp`** (`/tmp/impulse-zsh-<pid>-<n>`), and an existing directory or file there is reused. Another local user can pre-create it and get code run in your shell. **Confirmed.** | `shell_launch.rs:22-30`, `:72-107` | Use a fresh 0700 directory that must not exist (`tempfile`) or `$XDG_RUNTIME_DIR`. Swift uses the per-user temp folder plus a UUID. |
| S2 | LSP workspace edits resolve `..` before symlinks: `<root>/link/../x` counts as inside the project, so no question is asked, but the write lands outside through the link. | `impulse-lsp/src/workspace_edit.rs:264-288` | Let `trust::normalize` canonicalize instead. |
| S3 | **The LSP reader has no size limits**: it allocates whatever `Content-Length` says (a huge value aborts the app) and reads header lines without a cap. **Confirmed.** | `impulse-lsp/src/framing.rs:15-41` | Swift caps a message at 32 MiB and header lines and counts. |
| S4 | Automatic agent-turn checkpoints run `git add --all` with the full environment in folders that aren't trusted, so a repository's fsmonitor and clean filters run unprompted. The Swift app does the same. | `agent_checkpoints.rs:115,147` over `impulse-git/src/safety.rs:103-171` | Pass trust down and use the restricted environment, or skip checkpoints there. |
| S5 | Files copied into a new task follow symlinks: an untracked `.env -> ~/.ssh/id_ed25519` becomes a real file holding the key inside the task folder. Swift copies links as links. | `tasks.rs:1119-1130`, `impulse-kit/src/git/worktree_tasks.rs:156-160` | |
| S6 | URL hints open `file://` targets with the system opener, which can launch a program; the click path (`open_link`) already has the safe rules. | `terminal_blocks.rs:975-985` | |
| S7 | Low: `notify-send` gets the title with no `--`, so a program-printed title like `--action=...` is read as an option. | `agent_notify.rs:79-87` | |
| S8 | Low: an empty entry in the trust list trusts every folder (same in Swift). The control socket falls back to `/tmp/impulse-<pid>.sock` without `XDG_RUNTIME_DIR`. `open` only rejects the literal `/dev` prefix, not paths that resolve there (`/proc/self/fd/N`). A hook's `toolFile` runs git in any repository it names. | `impulse-kit/src/trust.rs:192-240`; `control_server.rs:97-123`; `control_commands.rs:130`; `agent_hooks.rs:567-579` | |
| S9 | Low: trust-list saves can lose a concurrent restriction (state read under one lock, written outside it, through a fixed temp name), and `write_private_file` makes its temp file before setting 0600. | `impulse-kit/src/trust.rs:169-186`; `impulse-kit/src/settings.rs:583-601` | |

## Performance

Ranked by how much a user would feel it. Costs are estimates for realistic
sizes.

| | Finding | Where | Fix |
|---|---|---|---|
| P1 | **Any terminal's output, even in a hidden tab, rebuilds the whole window every frame.** The Workbench observes every terminal and editor, and no view is `.cached()`, so a build log or dev server keeps the sidebar, tree, status bar, every visible terminal and every editor overlay rendering at 60-120 Hz. **Confirmed.** | `workbench.rs:474`, `:670` | Subscribe to the events that change chrome; render terminals, editors and the sidebar as cached views. |
| P2 | **A working agent's glyph redraws the whole window every frame** (a repeating animation requests a frame each time), for as long as the agent works. | `agent_views.rs:101-113` | A slow timer in a small cached view, paused while inactive. |
| P3 | **Every open editor re-runs `git blame` on every repository notification** (each save, each agent edit, each progress line). **Confirmed.** | `editor_git.rs:141-144` → `refresh_git` | Notify only on real snapshot changes; blame lazily, cached by HEAD and file stamp. |
| P4 | **Language servers get the full document on every keystroke, written synchronously on the UI thread** while holding the stdin lock; a 400 KB file fills the pipe, and a server that's busy writing can freeze typing. **Confirmed.** | `lsp_hub.rs:421-437`, `impulse-lsp/src/registry.rs:48-56`, `client.rs:268-272` | Incremental changes, and a writer thread per server (Swift has a write queue). |
| P5 | The file tree isn't virtualized and rebuilds every row, with closures and context menus, every frame; in Scratch it also lists the folder from disk during render. | `workbench_chrome.rs:1381-1520`, `:1386` | `uniform_list`, cached rows, menus built on open. |
| P6 | Word completions copy and scan every same-language open document on the main thread on each typed word character (about 5-15 ms on a 10k-line file). | `editor_tab.rs:2165-2210`, `impulse-kit/src/word_completions.rs` | A background word index. |
| P7 | The Markdown preview compiles highlight queries and re-highlights every code block on every frame. | `markdown_view.rs:421-470` | Cache highlighted blocks and one highlighter per language. |
| P8 | An open Review re-diffs every changed file every 2 s, visible or not; opening one blocks the UI on up to two full working-tree diffs; History re-runs 6-8 git processes on every repository notification. | `review_view.rs:323-333`, `:468-512`; `workbench_review.rs:119`; `history_view.rs:192` | Drive from the watcher, re-diff only what changed, pause while hidden. |
| P9 | Terminal frames always take a full grid snapshot, rebuild the block overlay and reshape every row, with no damage check; `block_overlay_key` exists and is never used. | `terminal_view.rs:2790-2850`, `:1891-2160` | Cache by damage and overlay key; cache shaped rows. |
| P10 | The input bar's 8 Hz spinner and the caret blinks rebuild the whole window (P1), for hours under a dev server. | `terminal_input_bar.rs:775-800`, `terminal_view.rs:529-552` | Follows from P1. |
| P11 | The Changes panel renders and clones every row each frame; the Problems status counts by cloning every diagnostic each frame. | `changes_panel.rs:1183-1232`; `problems_view.rs:53-62` | `uniform_list`; keep counts in the store. |
| P12 | Editor git marks (full-text line diff, bracket, link and conflict scans) run on the main thread after each typing pause; the word diff keeps O(D·(n+m)) memory and the Diff View doesn't cap line length (tens of MB for one long changed line). | `editor_git.rs:148-247`; `impulse-git/src/word_diff.rs:105-150`, `buffer_diff.rs` | Background work; banded or linear-space diff; cap line length. |
| P13 | The task monitor polls every 5 s per window (status of the main checkout and every task); every agent edit hook recomputes all workspace changes. | `tasks.rs:592-604`; `agent_hooks.rs:567-579` | Drive from watchers and the cache, as Swift's OverlapMonitor does. |
| P14 | Every save re-lists every folder in the tree synchronously, though the tree watcher already handles changes. | `workbench.rs:1219-1224` | Drop the reload on save. |
| P15 | Smaller ones: `Workbench::render` rebuilds three watch sets every frame; settings.json is read and hashed every second; on Linux two `gsettings` processes start every 5 s; session save, Replace All and some git reads (`rev-parse` on workspace switch, Review's scope menu) run on the main thread; sticky scroll allocates every line above the viewport; Vim copies the document per key; hover re-snapshots the grid per cell. | various | |

## Feature gaps and bugs by area

### Terminal

| | Finding | Where |
|---|---|---|
| T1 | **Combining characters and underline colours are never drawn**: the grid snapshot's extras are ignored, so decomposed names (`ls` on macOS) lose accents, emoji sequences break, and neovim's coloured underlines use the text colour. **Confirmed.** | `terminal_view.rs:161-200` |
| T2 | **Cmd+C with nothing selected sends ^C** to the program (interrupting an agent); Swift does nothing. **Confirmed.** | `terminal_view.rs:1402` |
| T3 | **Half-typed text in the input bar isn't cleared when password mode starts or ends**, so part of a password can show in plain text, run as a command and land in history. **Confirmed.** | `terminal_view.rs:788`, `terminal_input_bar.rs:678-681` |
| T17 | Shift+PageUp/PageDown/Home/End are bound in the Terminal context and never reach programs (vim, less, editors). | `terminal_view.rs:78-81` |
| T18 | Mouse reporting: releases always report button 0, drags aren't de-duplicated by cell or tagged with their button, modifier bits are never sent, presses and releases aren't paired, wheel reports aren't capped. The middle button is ignored entirely (no reports, no Linux PRIMARY paste). | `terminal_view.rs:1196-1221`, `:2756-2783` |
| T4 | Paste in the input bar takes text only (no files or images, which Swift inserts as paths). | `command_editor.rs:502-506` |
| T6 | Hovering or hinting a path stats it on the main thread; a path on a dead network mount freezes the UI. | `terminal_blocks.rs:111` |
| T8 | Input-method composition isn't drawn when the program hides the cursor (Claude Code and most TUIs). | `terminal_view.rs:2214-2236` |
| T9 | OSC 52 clipboard settings apply only to terminals opened after the change. | `terminal_view.rs:837-847` |
| T10 | The input bar's branch chip isn't clickable (Swift opens the branch switcher). | `terminal_input_bar.rs:958-1003` |
| T11 | Option as Meta: "left" or "right" makes both Option keys Meta on macOS; on Linux the setting is ignored and Alt is always Meta. | `impulse-kit/src/key_encoder.rs:10-12` |
| T12 | Rebinding Copy and Paste doesn't affect terminals. | `commands.rs:785-795`, `terminal_view.rs:85-98` |
| T13 | No working-directory fallback for shells without OSC 7 (Swift polls the shell's directory). | `terminal_view.rs:759` |
| T16 | The Quick Terminal keeps its first window's handle, so after the first hide its cursor never blinks and attention requests go to a dead window; it opens on the primary display. | `terminal_view.rs:597`, `quick_terminal.rs:220` |
| T19 | Smaller: Option-click doesn't force selection under mouse reporting; Select All takes the whole scrollback (Swift: the screen); Ctrl+Right doesn't accept the suggestion's next word on Linux; the toolbar's tooltips are unused; Clear only sends ^L (its comment says it clears scrollback); Ctrl+Shift+K may clear instead of bookmarking on Linux. | various |

### Editor

| | Finding | Where |
|---|---|---|
| E2 | **App shortcuts beat the editor's own keys**: commands are bound with no context and registered last, so on macOS ⌘D splits the pane instead of Add Next Occurrence, ⌘I opens the composer, ⇧⌘K bookmarks, ⇧⌘↩ zooms, ⌥⌘↑/↓ move focus. Swift reserves these for Monaco (`EditorWebView.swift:18-52`). **Confirmed.** | `commands.rs:745-760`, `editor_keys.rs:41-56` |
| E5 | settings.json loses its schema completions after any settings change (`apply_settings` replaces the provider). | `editor_tab.rs:356-367` |
| E7 | The Diff View's base never refreshes after staging or committing, and a reload leaves its tints stale. | `editor_diff.rs:89-104` |
| E8 | The git overlay's window-wide mouse listener steals clicks from palettes and menus drawn over the editor's sticky lines or change bars. | `editor_git.rs:617-650` |
| E9 | The same file open in two windows confuses its language server: duplicate opens, versions going backwards, and closing one tab closes the document for the other. | `lsp_hub.rs:408`, `editor_tab.rs:1496-1500` |
| E13 | Completions drop `additionalTextEdits` (no auto-imports), and `insertText`-only items don't replace the typed prefix ("pri" + "println" gives "priprintln"). | `lsp_hub.rs:716-742` |
| E14 | Code actions are requested with no diagnostics, so diagnostic quick fixes don't show. | `lsp_hub.rs:827` |
| E15 | Missing Monaco keys and actions: fold and unfold, Select All Occurrences, block comment, Go to Bracket, Next Problem (F8), expand/shrink selection, parameter hints, cursor undo, cursors at line ends, trim trailing whitespace, F1; Next/Previous Merge Conflict exist but aren't bound or listed. | `editor_keys.rs:38-77` |
| E10 | Vim mode on Linux: Ctrl-C never leaves Insert or Visual (taken by Copy), and Ctrl-F opens Find instead of paging. | `editor_vim.rs:31-56` |
| E11 | Smaller: a reload doesn't pick up a changed BOM; didSave goes out before the formatter; no linked editing (HTML tags); clicking inline blame does nothing; `font_family` lists ("Fira Code, Menlo") aren't supported; Format Document sends two extra options; some LSP and schema ranges mix UTF-16 and character columns; a lone CR isn't a line break. | various |

### Git surfaces

| | Finding | Where |
|---|---|---|
| G2 | Review's stage, unstage and revert errors are wiped by the refresh that follows, so failures look like nothing happened. | `review_view.rs:785-788`, `:554` |
| G3 | An operation's "after refresh" can get a snapshot read before it finished (fetch says "nothing new" when commits arrived; Commit right after Stage can say nothing is staged). | `repository_state.rs:158-165`, `:270-279` |
| G4 | Snapshot refreshes run alongside git operations (Swift shares one queue), so multi-step operations like Undo are seen half done and can fire `HeadMoved` prompts. | `repository_state.rs:96-118` |
| G6 | Review's whole-file Unstage fails in repositories with no commits and leaves a rename's old path staged. | `review_view.rs:748-749` |
| G7 | Review's busy guard is cleared by the background refresh, so a second click can apply the same hunk twice. | `review_view.rs:542` |
| G8 | Your own merge of a branch a task is still working on doesn't ask first (only agents get the merge guard). | `branch_manager.rs:461-489` |
| G9 | A stash can't be opened in Review. | `changes_panel.rs:1588-1675` |
| G10 | Smaller: the "only partly aborted" notice is a success toast that times out (Swift keeps a warning up); commit message history isn't saved; History of a file doesn't focus it; Review doesn't follow `review_context_lines` or a new HEAD for "Last commit"; a manual fetch doesn't reset the auto-fetch timer; Merge into shows a 40-character SHA; Copy Path copies the absolute path; Revert buttons aren't red. | various |

### Workbench and app shell

| | Finding | Where |
|---|---|---|
| W2 | **Linux windows can't be resized from their edges**: client-side decorations with no resize borders, and the titlebar's only drag area shrinks to nothing with many tabs. **Confirmed.** | `window_frame.rs`, `main.rs:356` |
| W4 | **Scratch's file tree can't expand folders** once the terminal changes directory, and lists the folder from disk on every render. **Confirmed.** | `workbench_chrome.rs:1384-1395` |
| W5 | Restored editors lose their cursor line and column. | `session.rs:492-504` |
| W6 | Closing the active tab always selects the left neighbour (Swift returns to the tab that opened it, else the right one); closing a workspace picks index-1, not the last used. | `workbench.rs:1202-1205`, `:1640` |
| W7 | Sidebar visibility and width are shared by all windows. | `workbench.rs:1303`, `:1327` |
| W8 | Restrict and Forget Trusted Folders only update the window they run in; other windows keep their language servers. | `workbench_trust.rs:379` |
| W9 | The titlebar tab strip doesn't scroll or keep the selected tab in view. | `workbench_chrome.rs:638` |
| W10 | No single instance or open-files plumbing: a second launch on Linux runs a second app (both write the session), the desktop entry has no `%F`, macOS has no open-URLs handler or Services item. | `control_server.rs:115-119`, `scripts/package-linux.sh:72` |
| W11 | Linux defaults collide with the desktop: Ctrl+Shift+U (IBus Unicode), Ctrl+Alt+Left/Right and Ctrl+Alt+Shift+Left/Right (GNOME workspaces); Close Window has no shortcut. | `impulse-kit/src/keybindings.rs` |
| W12 | Clicking a tab under another workspace in the sidebar skips workspace activation (no trust prompt, no language servers, no branch refresh). | `workbench_chrome.rs:1296`, `:550` |
| W13 | Smaller: `impulse` requests without a pane go to the first window, not the front one; window size and `open_files` aren't saved; some Swift alerts became status-bar notices that linger; saving an untitled editor during quit ends the quit silently; a reopened tab can land among pinned tabs. | various |

Settings the Rust app doesn't act on, beyond those in rust-remaining.md:
`open_files` (never read or written), `window_width` and `window_height`
(read, never saved), and `last_directory` (written, never read; minor).
Every command in the catalog has a handler, and the Mac default shortcuts,
menus and palette modes match Swift.

### Agents, tasks and the control socket

Caller checks, trust gates, task landing, compare-and-swap branch deletes,
the protocol and the MCP catalog all match Swift.

| | Finding | Where |
|---|---|---|
| A1 | `impulse unsaved` / `list_unsaved_files` answers for the window's active workspace instead of the caller's, with absolute paths, and `changedOnDisk` is always false. | `control_commands.rs:246-258` |
| A2 | A resolved review comment is always credited to "An agent", not the agent's name. | `control_review.rs:168` |
| A3 | `open --wait` on an image fails; MCP `open_file` doesn't flag the pane when the app isn't in front. | `control_commands.rs:182-195` |
| A4 | Desktop notifications post when this window is inactive (so also while another Impulse window is in front) and aren't cleared when attention clears. | `agent_views.rs:199-203` |
| A5 | One global busy flag for the task monitor drops other windows' refreshes. | `tasks.rs:381`, `:618-627` |
| A6 | An agent can be told "The user declined" when the sheet never opened. | `agent_tasks.rs:205-206`, `:249-251` |
| A7 | Unverified: with no window open, the Quick Terminal's hooks get "Impulse isn't ready yet". | `control_commands.rs:20-33` |

### Core crates

Invariants hold: every git command goes through `cli::run` with
`log.showSignature=false`, libgit2 has no network transports, the
restricted environment matches Swift, shell quoting matches scalar for
scalar, and every golden fixture is exercised by a Rust test. Beyond the
security items above:

- A few app git reads bypass `cli::run` (no timeout, no
  `GIT_TERMINAL_PROMPT=0`): `workspace.rs:452`, `terminal_input_bar.rs:1224`.
- The restricted environment doesn't turn off `diff.external` or textconv
  drivers (unverified whether blame honours them; Swift has the same gap):
  `impulse-git/src/cli.rs:286-341`.
- The MCP launcher breaks on a `}` in the CLI path:
  `impulse-kit/src/agents/mcp_installer.rs:41-51`.
- No timeouts on managed `npm install` or the LSP initialize request:
  `impulse-lsp/src/managed.rs:180-190`, `client.rs:103`.
- The control socket's 5 s deadline is per read, not per request, with no
  connection cap: `control_server.rs:171`.
- OSC 7 stats a program-supplied path on the terminal's reader thread:
  `impulse-terminal/src/osc_scanner.rs:657-660`.
