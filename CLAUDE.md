# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Impulse?

Impulse is a Mac-first terminal IDE: a terminal emulator with Warp-style command blocks combined with a Monaco-powered code editor in a tabbed interface. The app is Swift (AppKit + SwiftUI); the only remaining Rust is the terminal emulation core, reached through a small C FFI.

## Keeping this file current

Every feature change checks this file in the same commit. When you add, remove, rename or move a feature, target, directory, script, file format, setting mechanism, FFI function or key type, re-read the sections it touches (the architecture tree, Key patterns, Scripts, build commands) and fix whatever is now wrong or missing. Every type, file, function and flag named here must exist in the code, so grep this file for anything you rename or delete. Update `AGENTS.md` (the short version for other agents) when its facts change, and `docs/` when user-facing behavior changes.

## Architecture

```
impulse-macos/            Swift package (the app) — macOS 26+ (Tahoe)
  Sources/ImpulseApp      executable: AppKit/SwiftUI UI, terminal renderer,
                          Monaco WebViews, LSP/git/search wiring
  Sources/ImpulseKit      pure logic (Foundation-only): themes, previews,
                          palette, completion + shell parser + shell
                          quoting, editor buffer edits, file tree,
                          search results, close risk, glob, update checker,
                          pane LayoutTree, sidebar order + drag
                          reordering, agents (known agents, state
                          machine, hook events + installer, opencode
                          plugin, JSONC removal), command history DB,
                          worktree tasks + task registry, commit graph,
                          push server messages, terminal paths/hints/find
                          queries, project config, workspace trust, LSP
                          workspace edits + completion items, agents'
                          tool answers (TaskDetail, CommentList, ProblemList,
                          BlockList…), agent task policy + briefs, MCP
                          server registration
  Sources/ImpulseGit      git layer: libgit2 for reads (status, diff, blame,
                          log) + the git CLI for writes; safety snapshots;
                          gitignore-aware search
  Sources/ImpulseLSP      LSP client: server processes, JSON-RPC framing,
                          registry, document cache, managed npm installs
  Sources/ImpulseProtocol control-socket messages shared by app and CLI,
                          the socket client, and the MCP server (MCP/:
                          JSON-RPC over stdio, the tool catalog)
  Sources/ImpulseCLI      the `impulse` command-line tool (bundled in
                          Contents/Resources/bin): open, edit (waits for the
                          tab to close), review, split, tab, notify, status,
                          checkpoint, tasks (show/new/finish/archive/wait),
                          turns, comments, problems, blocks, unsaved,
                          project, hook, mcp
  Clibgit2/               module map for the vendored static libgit2
  CImpulseFFI/            C header for the Rust terminal FFI
  web/                    editor.html/js (Monaco glue)
impulse-terminal/         Rust: terminal emulation (alacritty_terminal),
                          OSC scanning (133 marks, 7 cwd, 6973 command
                          metadata, 9/777/99 notifications, 9;4 progress,
                          1337 attention, 21337 status), command blocks,
                          history
impulse-ffi/              Rust: C FFI over impulse-terminal (staticlib)
impulse-kit/              Rust (rust branch): ImpulseKit's logic for the
                          full-Rust migration (docs/plans/rust-migration.md):
                          glob, close risk, shell parser, themes, file URIs
                          and language ids revived from v0.29.0;
                          ChromePalette, TextFile, WorkspaceTrust, AppPaths,
                          Settings + SettingsCatalog (settings.json),
                          keybindings (Mac and Linux defaults), KeyEncoder
                          (key_encoder), Monaco's line commands, LayoutTree,
                          palette + fuzzy matching, file tree patches,
                          previews (comrak), Monaco/Markdown theme colors,
                          problems, review comments, update checker, shell
                          (types, quoting, login shell, integration),
                          completion, command history (SQLite), kitty
                          keyboard, terminal hints/paths/find,
                          ChildProcess, SessionState (session-state.json +
                          scrollback files), project config + settings file +
                          task environment + detector, task registry and
                          worktree tasks, commit graph, git helpers (the
                          editor's change marks, conflict blocks and blame
                          text: git/editor_marks.rs), agent
                          state machine, hook events + installers, MCP
                          installer, JSONC edits and task policy/briefs
                          ported; Vim mode's state machine (vim/: modes,
                          counts, motions, operators, text objects,
                          registers, `.`, search, `:` commands); tested
                          against the shared
                          golden fixtures
impulse-git/              Rust (rust branch): ImpulseGit: libgit2 (git2,
                          vendored 1.9.7, no network transports) for reads,
                          the git CLI for writes, safety snapshots, task
                          landing, change moves, operation abort, search,
                          watcher; checked against the Swift git fixtures
impulse-protocol/         Rust (rust branch): ImpulseProtocol: control-socket
                          messages (byte-compatible with the Swift app), the
                          socket client, the MCP server and tool catalog,
                          and the agents' tool answers (TaskListing, …)
impulse-cli/              Rust (rust branch): the `impulse` command-line tool
impulse-lsp/              Rust (rust branch): the language-server client
                          (no UI, any executor), LSPConfig (lsp.json, root
                          detection), TextEditApplier and WorkspaceEdit,
                          managed (npm-installed servers, the Swift
                          ManagedServers); its
                          e2e test runs rust-analyzer when installed
impulse-app/              Rust (rust branch): the app on GPUI + GPUI Kit for
                          macOS and Linux: workspaces (folder + Scratch)
                          with tabs of split panes (LayoutTree, draggable
                          dividers, zoom; preview tabs from a single click
                          in the file tree, pinned tabs, tabs dragged to
                          reorder, the tab context menu), commands as one RunCommand
                          action bound from impulse_kit::keybindings,
                          workbench chrome, the terminal (the login shell
                          with integration, shell_launch.rs),
                          its own painted grid, keys (the kitty keyboard
                          protocol once a program pushes its flags,
                          impulse_kit::kitty_keyboard), input methods,
                          selection, mouse reporting,
                          settings.json (SettingsStore, watched and merged),
                          session restore (session.rs: autosave every 15 s
                          when the structure changed, and at quit; a
                          restored terminal shows its old output down to
                          the "restored" rule before its first command),
                          Reopen Closed Tab and the close's Undo toast
                          bringing back a tab, pane (into the tab it left)
                          or workspace as it was, scrollback included
                          (session::session_tab / session_workspace,
                          Workbench::reopen), the
                          control socket for `impulse` (control_server.rs,
                          control_commands.rs; Linux keeps it in
                          $XDG_RUNTIME_DIR), Review (review_view.rs:
                          scopes, navigator, unified and split diffs with
                          word-level changes, stage/unstage/revert of files
                          and hunks, viewed marks, the j/k/n/p/s/u/x/v/c/o
                          keys; review comments in review_view/comments.rs:
                          the composer from a line's +, a hunk's Comment or
                          c, comments under their lines and outdated ones
                          on top, edit, delete, an agent's resolution with
                          Accept and Reopen, Copy as Prompt, kept by
                          impulse_kit::review_comments::CommentStore in the
                          data directory's review/<digest>.json), Problems
                          (problems_view.rs: ProblemStore with every
                          server's diagnostics, the tab by file with
                          severity toggles and a filter, the status bar's
                          counts), `impulse review|comments|problems`
                          (control_review.rs), History (history_view.rs: the lane graph,
                          ref chips, paging, the commit's details over its
                          Review list), command palette and quick open
                          (palette.rs: the Swift PaletteModel/PaletteView
                          modes by prefix, recents in
                          palette-recent-commands.json in the data
                          directory), Markdown and SVG preview (sanitized by
                          impulse_kit::svg_preview) in or beside the
                          editor, Settings and Keyboard Shortcuts tabs
                          (settings_view.rs, keybindings_view.rs; shortcut
                          overrides rebind live through
                          commands::rebind_on_change), git
                          (repository_state.rs: RepositoryState per
                          repository root via RepositoryStore, refreshed by
                          impulse_git's RepoWatcher and after every action,
                          operations serialized with safety snapshots, CLI
                          reads through RepositoryState::read_git, restricted
                          in untrusted folders; changes_panel.rs: the Changes
                          panel and commit composer, rows opening Review;
                          git_actions.rs: GitActions/GitPrompts and the git
                          commands; branch_switcher.rs; branch_manager.rs:
                          Manage Branches, with merge, rebase, delete
                          (Undo), rename and publish; file_icons.rs:
                          material icons from icon-mapping.json, served as
                          material/<file>.svg), the terminal input bar
                          (terminal_input_bar.rs over command_editor.rs:
                          chips, shell colors, ghost suggestions, Tab menu,
                          history from command_history.rs, which shares
                          history.sqlite3 and backs the palette's h:
                          mode, Import Shell History and Clear Command
                          History), find (terminal_find_bar.rs), and
                          block commands, hints and links
                          (terminal_blocks.rs, reached through
                          TerminalView::perform_block_command), the sticky
                          block header (Frame::paint_sticky_header), the
                          Search panel
                          (search_panel.rs; workbench_search.rs opens
                          results and runs Replace All with Undo, reloading
                          clean editors with EditorTab::reload_from_disk),
                          tasks (tasks.rs: registry, identity, trust, prep;
                          task_sheet.rs New Task; task_finish.rs Finish
                          Task; task_cleanup.rs Archive/Undo;
                          task_sidebar.rs repository groups, merged and
                          overlap chips), project settings
                          (project_setup.rs: the Project Setup tab,
                          Surface::ProjectSetup, over
                          project_setup_model.rs; project_actions.rs: the
                          palette's a: actions, Edit Project Actions;
                          dependency_prompts.rs: [on_change] and check
                          offers on RepositoryEvent::HeadMoved), the Quick
                          Terminal
                          (quick_terminal.rs: a pop-up
                          window over a TerminalView with no_bar, its
                          shortcut registered with global-hotkey),
                          Use Impulse as $EDITOR (EDITOR/VISUAL set to
                          `impulse edit` by control_server::terminal_env),
                          the file tree following the disk (tree_watch.rs:
                          every listed folder watched on its own, listed
                          again when its entries change),
                          the file tree's context menu
                          (file_tree_actions.rs: new file and folder, open
                          to the side, mention in an agent, history,
                          reveal, copy paths, git actions, rename, Trash
                          with Undo on Linux; drag
                          and drop moving project items and copying ones
                          from outside, Workbench::tree_drop),
                          close_flow.rs (asking before closing a pane,
                          tab or window, or quitting: a pinned tab, Save,
                          Don't Save or Cancel for each unsaved editor,
                          then running terminal work from
                          impulse_kit::close_risk),
                          update_check.rs (a newer release, once a day,
                          by impulse_kit::update_checker with curl, as
                          "Update x.y.z" in the status bar),
                          background fetch (repository_state.rs:
                          start_auto_fetch, git_auto_fetch_minutes,
                          trusted repositories with an upstream),
                          New File and Install Language Servers
                          (untitled.rs: an untitled EditorTab saved with
                          the system's save dialog, or a typed path where
                          Linux has no portal),
                          menus (menus.rs from impulse_kit::menus: the
                          macOS menu bar, the Linux titlebar menu button,
                          the fixed items' actions and keys),
                          workbench_workspaces.rs (recent folders in
                          recent-workspaces.json via
                          impulse_kit::recent_workspaces, Rename Workspace,
                          Workbench::text_prompt for one-field dialogs),
                          agents (agent_tracker.rs: AgentTracker in each
                          TerminalView, found from the foreground process by
                          agent_process.rs and from `impulse hook`, driving
                          AgentStateMachine and the attention dots;
                          agent_views.rs: tab glyphs, the inbox, Review Last
                          Agent Turn, Next Agent; agent_hooks.rs: hook,
                          status, notify, turns, blocks and review requests
                          and the hook replies; agent_checkpoints.rs: turns
                          under refs/impulse/checkpoints/, saved in the
                          session file as agent_turns (SavedTurn);
                          agent_integration.rs: the Agent Integration
                          dialog; agent_notify.rs: desktop notifications by
                          notify-send or osascript, a click on one showing
                          its terminal where notify-send takes --action;
                          agent_send.rs: text for agents as a paste, queued
                          while one is mid-turn (AgentTracker::pending_text),
                          Send Selection to Agent, the agent items of
                          Problems' Fix with Agent, Review's comments menu,
                          conflicts' Ask Agent and a block's Send to Agent;
                          agent_composer.rs: the composer over an agent's
                          TUI with @ mentions; agent_toolbelt.rs: the
                          toolbelt under its terminal, Review Turn and the
                          Turns menu with Restore; agent_tasks.rs: `impulse
                          tasks` (show, wait, and new, finish and archive as
                          the user's own flows, task_requests::decide
                          saying whether New Task asks) and `impulse
                          project`),
                          editor tabs (GPUI Kit's Editor, themed, file tree,
                          save through save_pipeline.rs: a "changed on
                          disk" question, the formatter and commands on
                          save in trusted folders with
                          impulse_kit::save_command; editor_watch.rs:
                          the open files' folders watched with notify, a
                          clean editor reloading what an agent or git
                          wrote; editor_git.rs: change marks against the
                          index painted over the gutter, the peek with
                          Revert, Stage, Diff and Review, inline blame,
                          merge conflicts tinted with Accept Current,
                          Incoming or Both, from
                          impulse_kit::git::editor_marks; Monaco's keys,
                          bracket pair colors (impulse_kit::bracket_pairs),
                          sticky scroll (impulse_kit::sticky_scroll),
                          the minimap (editor_minimap.rs on
                          impulse_kit::minimap),
                          word-based suggestions without a language
                          server (impulse_kit::word_completions, other
                          open files' words too per the setting);
                          occurrence and selection
                          highlights as range decorations from document
                          highlights or impulse_kit::occurrences, Monaco's
                          context menu, Go to Type Definition,
                          Implementations and Declaration through
                          DocumentLink::navigate, Change All Occurrences,
                          the Diff View (editor_diff.rs: the index
                          version read-only beside the buffer, scrolling
                          with it, or inline in one read-only document,
                          under a bar with the summary, ↑ ↓, the layout
                          switch and Done; impulse_kit::git::diff_view),
                          signature help over the caret with
                          impulse_lsp::signature; Vim mode in
                          editor_vim.rs over impulse_kit::vim, its keys
                          taken ahead of GPUI Kit's in the VimNormal
                          context, the mode in the status bar), workspace trust, language
                          servers (code actions, formatting, rename and
                          references with edits across files), the
                          headless snapshot mode of debug builds
                          (debug_snapshot.rs: the Swift DebugSnapshot's
                          --impulse-snapshot flags, actions and dump=
                          format, plus edit= and lsp= in place of
                          Monaco's js= (EditorTab::debug_edit and
                          debug_lsp); window images by ImageMagick's
                          import on X11, screencapture on macOS) that
                          scripts/smoke and scripts/docs drive,
                          --install-lsp-servers;
                          scripts/spike/linux-shot.sh screenshots it on Linux
vendor/                   Monaco, monaco-vim, fonts, highlight.js, lucide
                          (committed; refreshed by scripts/vendor-*.sh)
assets/                   file icons + icon-mapping.json (copied into the
                          app by build.sh), logo, DMG background
docs/                     user documentation, one Markdown page per area;
                          docs/images/ is generated by scripts/docs/capture.py;
                          docs/plans/ holds plans (not published)
website/                  impulse-terminal.app: homepage + demo, docs page
                          template (built by scripts/build-site.py)
scripts/                  build-libgit2.sh, vendor-*.sh, release.sh,
                          build-site.py, docs/ (screenshot capture),
                          smoke/ (release smoke checks)
CHANGELOG.md              user-facing changes per release
```

Dependency direction: ImpulseApp → {ImpulseKit, ImpulseGit, ImpulseLSP, ImpulseProtocol, CImpulseFFI}; ImpulseGit/ImpulseLSP → ImpulseKit; `impulse` (CLI) → ImpulseProtocol. The Rust workspace is `impulse-terminal` + `impulse-ffi` (plus, on the `rust` branch, `impulse-kit`, `impulse-git`, `impulse-lsp`, `impulse-protocol`, `impulse-cli` and `impulse-app`, which pins `gpui-kit` exactly and gets GPUI through it; the Swift app links neither), and the FFI surface is terminal-only (~38 functions; JSON strings for complex data, a binary buffer for grid snapshots, plain integers on hot paths — mode bits, the block-overlay cache key, the display offset, search stats — and `impulse_free_string` for cleanup). Command blocks are listed without their output; fetch one block by id when you need it.

### Why the split

- **ImpulseKit** is Foundation-only so its logic is headless-testable. No AppKit/WebKit imports there.
- **ImpulseGit** wraps a vendored static libgit2 1.9.7 built WITHOUT network transports for reads (status, diff/hunks with word-level spans, blame, log, ignore checks). Writes (stage, commit, push/pull, stash, branches, worktrees, conflict resolution) run the `git` CLI, and destructive ones take a safety snapshot under `refs/impulse/` first so they can be undone. Aborting a merge, cherry-pick or revert goes through `OperationAbort`, which falls back to a hard reset plus the work recorded when the operation started (`<git dir>/impulse-operation.json`) when git's own `--abort` refuses. No OpenSSL anywhere. New git APIs use the real `git` CLI as the test oracle.
- **ImpulseLSP** owns language-server processes with hand-rolled Content-Length framing and a poll-shaped facade (`pollEvent()` returns the same JSON envelopes the app has always decoded).
- **impulse-terminal (Rust)** stays because alacritty_terminal is a complete, battle-tested VT emulator. It owns the PTY, grid/scrollback, damage tracking, selection, scrollback search, and OSC 133/7/6973 command-block tracking. Swift owns all rendering (CoreText in `TerminalRenderer.swift`) and input encoding.
- `Bridge/ImpulseCore.swift` is the single seam: terminal calls go through the C FFI; everything else delegates to the Swift libraries. Keep new backend logic OUT of the bridge — put it in the right library target.

### Golden-fixture parity (important)

The Swift ports were verified against fixtures generated from the original Rust implementation, committed under `impulse-macos/Tests/ImpulseKitTests/Fixtures/` and `Tests/ImpulseGitTests/Fixtures/` (themes → Monaco JSON, git hunks/word-diff/blame/status, shell-parser tokenizations, file-tree patches, palette filtering, glob, close risk). Treat fixtures as the spec: never regenerate them from Swift output. If behavior changes intentionally, update the affected fixture explicitly in the same commit and say why.

## Build & Development Commands

```bash
# macOS app (canonical build — produces dist/Impulse.app)
./impulse-macos/build.sh             # release .app bundle
./impulse-macos/build.sh --dev       # "Impulse Dev.app" (separate bundle ID, runs side-by-side)
./impulse-macos/build.sh --dmg       # + disk image
./impulse-macos/build.sh --sign --notarize --dmg   # full release build

# Swift package directly — use the wrapper, which adds the flags the active
# toolchain needs (CLT Swift 6.4 lacks the SwiftUI macro plugin and puts
# Testing.framework off the default search path; swiftw borrows Xcode's macOS
# macro plugins and points tests at the CLT framework).
impulse-macos/swiftw build [-c release]
impulse-macos/swiftw test [--filter Name]
#   Tests are guarded with #if canImport(Testing); a run that prints no
#   "Test run with N tests" line means the Testing module wasn't found.

# Rust terminal core
cargo build -p impulse-ffi           # staticlib the Swift app links
cargo test -p impulse-terminal       # block/OSC/history tests
cargo test -p impulse-kit            # rust branch: ported logic vs the golden fixtures
cargo test -p impulse-git            # rust branch: git layer vs the Swift git fixtures
cargo test -p impulse-lsp            # rust branch: LSP client (+ rust-analyzer e2e)
cargo test -p impulse-protocol -p impulse-cli   # rust branch: control protocol, MCP, CLI
cargo run -p impulse-app -- --cwd <folder> --open <file>   # rust branch: the GPUI app
cargo fmt && cargo clippy

# One-time / occasional
./scripts/build-libgit2.sh           # vendored libgit2 (cached; build.sh runs it)
./scripts/vendor-monaco.sh           # refresh vendor/monaco
```

Note: `swift build` links `../target/release/libimpulse_ffi.a` — run `cargo build --release -p impulse-ffi` first on a fresh checkout (build.sh does all of this in order). SwiftPM does not track `.a` mtimes; build.sh has a relink hack for that.

## Key patterns

- **SwiftUI/AppKit bridge:** `@Observable WindowModel` is the single source of truth for UI state. AppKit (MainWindowController, TabManager) mutates it; SwiftUI observes it. Its tab snapshots (`TabDisplayInfo`, `WorkspaceInfo`, `AgentSummary`) are Equatable and assigned only when they change (`WindowModel.update`); frequent changes (titles, dirty marks) go through `TabManager.refreshSegmentLabels()`, which coalesces syncs; tab actions resolve a tab by its stable id when they run (`WindowModel.withTab`). SwiftUI→AppKit communication uses callback closures on WindowModel. The window chrome is drawn by `WorkbenchView` under a transparent titlebar; the window's `NSToolbar` is empty and only sizes the titlebar band (SwiftUI `.toolbar {}` does not work inside `NSHostingView`).
- **Window controller:** `MainWindowController` keeps its stored state and setup in `MainWindow.swift`; everything else is in `MainWindowController+<Area>.swift` extensions (Layout, Observers, Save, FindBar, TabClose, Session, Workspaces, WorkbenchActions, Preview, Palette, GitHost, LSP, Trust, ProjectSetup, Debug). Add code to the matching extension, not to `MainWindow.swift`.
- **Commands, settings, keys:** user-facing commands are `AppCommand`s in `App/CommandRegistry.swift` (palette and menus); every setting is described once in `Settings/SettingsCatalog.swift` (the Settings tab and the `settings.json` schema come from it); `SettingsStore` watches the resolved `settings.json` and merges edits made outside Impulse with its own (`Settings.merged`); shortcuts live in `Keybindings`. Debug-only UI actions for headless snapshots (the docs screenshots and the smoke checks) are in `MainWindowController+Debug.swift`; `DebugSnapshot` runs them one after another, and its invisible windows let clicks through and never stay the active app, so a run can't take keystrokes meant for another app.
- **Workbench:** a window has workspaces (folder or scratch; task worktrees live beside the repo at `<repo>.worktrees/<branch>`, and `ImpulseKit.TaskRegistry` lists the ones Impulse made, with base and slot, in `.git/impulse/tasks.json`; `TaskRegistryStore` reaches it from the app; `OverlapMonitor` works out where a repository's workspaces change the same files with `ImpulseKit.TaskOverlap`, shown as a chip on their sidebar rows, with `git merge-tree` conflict checks on demand, and which tasks are merged into their base, squash merges included (`GitOperations.isMerged`); archiving and its Undo, one task or all merged ones, are `Git/TaskCleanup.swift` (it refuses a task holding a nested repository, `GitOperations.nestedRepositories`, and revokes the task folder's trust); Finish Task, a tab per task that checks it's committed, syncs it, runs the check script in a terminal, lands and cleans up, is `Git/TaskFinish.swift`, landing through `ImpulseGit.TaskLanding`: a `--no-ff` merge in a throwaway worktree pushed as the base, never touching the main checkout, and deleting the landed branch only if it hasn't moved (`TaskLanding.deleteLandedBranch`: compare-and-swap locally, a lease on the remote); New Task can take the main checkout's uncommitted files along with `ImpulseGit.ChangeMove`: snapshot, files written into the task unstaged, then back to HEAD in the main checkout, with Undo), each with tabs; a tab is one surface or a split (`ImpulseKit.LayoutTree` of panes, laid out by `PaneLayoutView`). `TabManager` owns them; session restore v2 (`SessionState.swift`) saves workspaces, layouts and terminal scrollback, on quit, on window close and, so a crash loses little, from `AppDelegate`'s 15-second autosave whenever `SessionState.autosaveKey` changes (never while any window `isRestoringSession`). `SessionState.persist` writes off the main thread; scrollback files are named by content hash, and stale ones go only after the session JSON is written.
- **Agents:** `TerminalTab+Agent` detects CLI agents (Claude Code, Codex, opencode, …) from the foreground process and hook events (`ImpulseKit.AgentHookEvents` says what each agent's events mean: Claude Code's hook events, Codex's notify, and the opencode events Impulse's plugin sends); `AgentStateMachine` drives the inbox, tab dots and notifications. Each window listens on a control socket (`ControlServer`; every terminal gets `IMPULSE_SOCKET` / `IMPULSE_PANE_TOKEN`) for the `impulse` CLI. Commands that start processes or write files (split, tab, checkpoint, creating a file to edit, tasks new/finish/archive, comments resolve), and reading other terminals' output (blocks), also require the caller (`LOCAL_PEERPID`, `ControlCaller.descends(from:)`) to be the pane's shell or one of its descendants; the rest need only the token, and those that read the caller's workspace need it to name a pane. Agent turns are checkpointed under `refs/impulse/checkpoints/` for "review last turn". `impulse mcp` (`ImpulseProtocol.MCPServer`, tools in `MCPTools`) is a stdio MCP server agents start themselves: each tool call is one control request, the same as the CLI command it names, answered with text plus `ControlResponse.data` JSON (the types in ImpulseKit's `AgentToolResults.swift`; tools declare no `outputSchema`, since some answers are text only); it answers `initialize` (2025-11-25 and before) and the stateless 2026-07-28 revision (`server/discover`, and requests naming it in `_meta` get `resultType` and cache hints); outside Impulse it offers no tools, and the settings can turn tools off (`mcp-tools`). `App/AgentWorkspaceCommands.swift` answers turns, review comments (an agent marks one addressed, `ReviewComment.resolution`, which Review shows with Accept and Reopen), problems, command blocks (`agent_tool_terminal_output`), unsaved files and trusted project settings. `Git/AgentTaskRequests.swift` handles agents' `tasks new|finish|archive`: New Task prefilled with the requester and brief (`AgentTaskRequest`, provenance in `TaskRecord.createdBy`/`requestedBy`, the brief in `.git/impulse/tasks/<task>/brief.md`, `ImpulseKit.TaskBrief`), unless `ImpulseKit.AgentTaskPolicy` says to create it straight away (`agent_task_creation`, `agent_task_limit`); Finish runs up to landing and waits (`TaskFinishModel.stopsBeforeLanding`). The Agent Integration sheet (`App/AgentHooksSheet.swift`) installs hooks and the server (`ImpulseKit.AgentMCPInstaller`: `claude mcp add-json`, never editing `~/.claude.json`; a table in Codex's `config.toml`; for opencode only taking out the `mcp` entry older Impulses added, in place by `JSONCEdit` so comments and formatting stay, since the plugin brings the tools). opencode 2 runs one background service for every terminal, started with the environment of whichever terminal ran it first, so its plugin is a folder of Impulse's own with two halves (`ImpulseKit/Resources/Agents/opencode-plugin/`, installed as `<opencode config>/plugins/impulse/`, compared byte for byte for its status; installing removes opencode 1's `plugins/impulse.js`): `tui.js` runs in each terminal, as the pane, sending `$IMPULSE_CLI hook opencode <event>` with JSON on stdin (nothing outside Impulse) and running Impulse's tools through its own `impulse mcp`; `server.js` runs in the service, never reads `IMPULSE_*` from its own environment, and asks the terminal a session belongs to (the last to show it) over opencode's plugin RPC (`rpc.js`) for the summary, notes, merge question and tool calls; it registers Impulse's tools as opencode tools (MCP servers would run in the service) and gives each agent shell command its own pane's `IMPULSE_*`. Agents see each other through `App/TaskAwareness.swift`: `impulse tasks` (an `ImpulseKit.TaskListing`) and hook replies (`AgentHookReply`) for Claude Code and the opencode plugin: a session summary (opencode: added to the session's system prompt), notes after edits and shell commands about shared files and changed dependencies, and before a merge of a branch a task is still working on (`GitCommandInspector` reads the command) a PreToolUse "ask" for Claude Code, or for opencode a permission the plugin raises (`permission.assert`) with Impulse's reason. Impulse never calls an AI model itself.
- **Terminal:** event-driven. The Rust core calls a wake callback when output arrives; `TerminalRenderer` then runs `pollEvents()` → `takeDamage()` → invalidate rows → `draw` pulls a binary grid snapshot (16-byte header + 12-byte cells) parsed zero-copy by `GridBufferReader`. Command-block decorations come from `impulse_terminal_block_overlay`, cached by `impulse_terminal_block_overlay_key`. Input is the `CommandEditor` (an NSTextView with shell highlighting); completion is Swift (`ImpulseKit.InputCompletion`) fed by persistent history (`CommandHistoryDatabase`) and `impulse_terminal_recent_commands`. Text Impulse types into a terminal is quoted for that terminal's shell (`ImpulseKit.ShellQuoting`, `String.shellEscaped(for: TerminalTab.shellType)`; fish quotes differently), completions escape only the completed part (`CompletionCandidate.keptPrefix`: Impulse's own candidates are literal names, fish's are shell text), and input-bar commands go through `TerminalTab.commandText` (control characters stripped, a leading space kept). The Rust core drops OSC sequences over their size limit before vte sees them, and keeps alacritty's scrollback at the user's limit plus `SCROLLBACK_HEADROOM`, trimming it itself so block rows stay exact.
- **Shell integration:** `ImpulseKit/Resources/ShellIntegration/{bash,zsh,fish}.sh` emit OSC 133 (prompt/command marks), OSC 7 (cwd) and OSC 6973 (`Command=` the command text, `Names=` aliases/functions/builtins and `Path=` PATH, sent only on change). Every mark carries the terminal's nonce (`IMPULSE_SHELL_NONCE`); once one has arrived, marks without it are ignored, so printed output can't fake them. Keys: `KeyEncoder` encodes legacy sequences; when a program pushes kitty keyboard flags (mode bits 11–15) `ImpulseKit.KittyKeyboard` takes over. Option is Meta (Esc + key) only on the side(s) `terminal_option_as_meta` names (`KeyEncoder.OptionAsMeta`); otherwise it types what the layout composes.
- **Editor:** Monaco in WKWebView, loaded from the app bundle (`EditorAssets.monacoDirectory`; build.sh copies vendor/ + impulse-macos/web/ into `Sources/ImpulseApp/Resources/monaco` — copy again and rebuild after editing `web/`). `EditorWebViewPool` pre-warms one WebView. Swift keeps a copy of each buffer, applying Monaco's UTF-16 changes with `ImpulseKit.BufferEdits`; reloading the same file from disk is applied in place as one undoable edit, and files over `TabManager.maxEditorFileSize` don't open. Markdown preview renders with cmark-gfm in safe mode (raw HTML is elided by design — do not re-enable `CMARK_OPT_UNSAFE`).
- **Language servers:** LSP requests go through `enqueueLspRequest` (sent after queued didOpen/didChange, answered off the queue, never blocking a thread); it returns an `LSPRequestTicket` whose cancel sends `$/cancelRequest`. Notifications reach every server for the language; only documents a configured server serves are opened and mirrored. A server's project root never goes above the trusted folder (`LSPRegistry.trustedFolder`). Each editor tab is its own WebView, so Monaco can only edit its own file: workspace edits that reach further (rename, code actions, `workspace/applyEdit`) are applied in Swift by `Editor/LSPWorkspaceEdits.swift` on top of `ImpulseKit.WorkspaceEdit`; edits that create, move or delete files, or reach outside the server's root, ask first.
- **Review:** native, not web. `ReviewSurface` (header + `ReviewNavigatorView` + `ReviewDiffController`) builds flat rows with `ReviewRowBuilder` into one NSTableView: line rows are custom-drawn (`DiffLineCellView`, character-wrapped, heights from `ReviewMetrics`), headers/comments/composer are hosted SwiftUI (`ReviewRowViews.swift`), file headers float as group rows. Syntax colors come from highlight.js in JavaScriptCore (`SyntaxHighlighter`, old/new sides tokenized separately) mapped to the theme's syntax colors. History's lower half is the same surface. `ReviewDiffController` caches each file's rows and heights (`ReviewRowKey`) and replaces only the changed range; a Review refreshes only while it's in a window, and a fixed commit never does.
- **Git only:** Impulse talks to git (the `git` CLI and libgit2) and nothing else: no hosting platform's CLI or API (`gh`, `glab`, REST), so every feature works the same on any host. Host-facing bits stay generic: `RemoteWebURL` only rewrites a remote into `https://host/owner/repo`, and after a push the server's own `remote:` lines (`ImpulseKit.GitServerMessage`) are shown, links included. Anything new must work from git alone.
- **Workspace trust:** `ImpulseKit.WorkspaceTrust` (app instance `Trust.shared`, extension `MainWindowController+Trust`) lists the folders whose code Impulse may run on its own, and folders kept restricted inside a trusted one (`restrict`; tasks made from a remote branch, `TaskRecord.fromRemote`). Language servers (the registry's `isAllowed`), formatters/commands on save and background fetch check it; anything new that runs project code unprompted must too. Git reads in folders that aren't trusted run with `GitCLI.restrictedEnvironment` (no fsmonitor, hooks, filters or merge drivers; `GitOperations.isMerged` takes `trusted:`). Every git command runs with `-c log.showSignature=false` (`GitCLI.alwaysArguments`): showing signatures would run a repository's own `gpg.program` from `log`, `show` and `reflog`, and print among the output Impulse parses. Opening a folder asks (`TabManager.onFolderOpened`); restored workspaces don't.
- **Project config:** `ImpulseKit.ProjectConfig` reads a repository's `.impulse/project.toml` (palette actions, task setup/archive scripts, files to copy into new task worktrees; `.worktreeinclude` lists more), always the main checkout's, committed or not (`loadProjectConfig` goes through `mainCheckoutRoot`), so every task gets what was saved, and the local, never-committed `.git/impulse/project.toml`, which wins key by key (actions by name). Each file's commands run (and per-task values apply) only after the user trusts that exact file, and changing it asks again. Per-task values (`[worktrees.ports]` offset by the task's slot, `[worktrees.env]` with placeholders) are worked out by `ImpulseKit.TaskEnvironment` and written into the new task's env file; task terminals and scripts also get `IMPULSE_TASK`, `IMPULSE_TASK_SLOT` and `IMPULSE_REPO_ROOT` (`TaskRegistryStore.identity`). The Project Setup tab (`Settings/ProjectSetupSurface.swift`, `MainWindowController+ProjectSetup`) proposes settings from `ImpulseKit.ProjectDetector` (and `ComposeFile`, which also writes each task's Compose override) and saves them into `.impulse/project.toml` with `ProjectSettingsFile` (only its managed sections), trusting what it wrote; a local file Impulse wrote (`ProjectSettingsFile.isWrittenByImpulse`) is folded in and removed on save, one edited by hand is left alone. `[finish] land` (merge or review) is Finish Task's remembered answer, saved into the local file with `ProjectSettingsFile.setting`. When a workspace's HEAD moves (`GitRepositoryState` posts `.gitHeadMoved`), `Git/DependencyPrompts.swift` offers the `[on_change]` commands for the files that changed (`ImpulseKit.DependencyChanges`, which tells own commits from pulls and checkouts by the reflog) and, after a merge, the `check` script. Language servers are configured by `~/.config/impulse/lsp.json` plus per-project `.impulse/lsp.json`, which can only choose among servers that already exist (its `servers` are ignored).
- **Themes:** TOML files in `Sources/ImpulseKit/Resources/Themes/` (user themes in `~/Library/Application Support/impulse/themes`). `ThemeStore` resolves them; `themeToMonaco` / `themeToMarkdownColors` derive editor/preview themes. New built-in themes: add the TOML resource and its name to `ThemeStore.builtinThemeNames()`.
- **File tree:** `FileTreeDataController` (headless) owns watchers and data (diffed per batch, with an FSEvents fallback past 64 folders; expanded folders load off main; patches apply in one pass, `FileTreeDataController.applying`); patches computed by `ImpulseKit.FileTreePatcher` with git-status enrichment composed from `ImpulseGit.GitClient`; rendered by `FileTreeListView` (ScrollView + LazyVStack, NOT List/DisclosureGroup).
- **Version:** the top-level `VERSION` file is the single source of truth. build.sh stamps it into Info.plist; the app reads `CFBundleShortVersionString` (`AppVersion.current`). Never version-bump by hand — `scripts/release.sh` does it.
- **Error handling:** library targets return optionals/Results with descriptive messages; NSLog/os_log for non-fatal issues.

## Scripts

**Always use the existing scripts for their intended tasks — do not replicate their steps manually.**

- **scripts/release.sh <version> [--push]** — the ONLY way to release: writes VERSION, syncs crate versions, commits, tags, builds signed+notarized .app/.dmg, checksums, and (with --push) pushes and creates the GitHub release. Never run `gh release create`, `git tag`, or manual version edits.
- **scripts/build-libgit2.sh** — pinned libgit2 static build into `impulse-macos/.libgit2/` (checksum-verified; idempotent).
- **scripts/vendor-monaco.sh** — refreshes `vendor/monaco`.
- **scripts/vendor-monaco-vim.sh** — refreshes `vendor/monaco-vim` (the editor's optional Vim mode; pinned version and checksum).
- **scripts/vendor-highlight.sh** — refreshes `vendor/highlight` (highlight.js for Markdown preview and review/history syntax colors; pinned version and checksum).
- **scripts/vendor-lucide.sh** — regenerates `LucideIcons.swift` from the pinned lucide-static icons (add a name to its list for a new icon).
- **scripts/generate-icon-mapping.py <upstream-json> <output-json>** — regenerates `assets/icons/material/icon-mapping.json` (the file tree's icons, read by `FileIcons.swift`) from upstream material-icon-theme.
- **impulse-macos/build.sh** — builds the .app (libgit2 → impulse-ffi → asset copy → SwiftPM → bundle → optional sign/notarize/dmg).
- **scripts/build-site.py [--serve] [--check]** — builds impulse-terminal.app into `website/_site/` (homepage from `website/`, docs pages from `docs/*.md` with the sidebar following `docs/README.md`, changelog, the demo's themes from the theme TOMLs with the Monaco colors `themeToMonaco` gives them (ported to Python and checked against the `themes/*.monaco.json` fixtures), and the demo's icons from `vendor/lucide` and `assets/icons/material`, picked the way `FileIcons.swift` picks them) and checks every docs link, anchor and image. `.github/workflows/website.yml` runs it and deploys to GitHub Pages on pushes to main; `--serve` rebuilds on every change. Needs `pip install -r website/requirements.txt`. See `website/README.md`.
- **scripts/docs/capture.py [name-prefix…]** — regenerates the user docs' screenshots (`docs/images/`) from the dev build's headless snapshot mode, inside a mock project and home built by `scripts/docs/make_demo.py`; shots are defined in `scripts/docs/shots.py`. Build with `build.sh --dev` first. With `IMPULSE_SNAPSHOT_APP` set to the Rust app's binary (the default on Linux: `target/debug/impulse-app`, after `cargo build -p impulse-app -p impulse-cli`, with `DISPLAY` on an X server such as Xvfb) it shoots the Rust app instead, composing with `scripts/docs/imagetool.py` (Pillow) where there's no `imagetool.swift`.
- **scripts/spike/linux-shot.sh <out.png> [impulse-app args…]** (rust branch) — screenshots the GPUI spike under Xvfb with software Vulkan (needs Xvfb, xdotool, ImageMagick, mesa-vulkan-drivers); `cargo build -p impulse-app` first.
- **scripts/smoke/run.py [name-prefix…]** — smoke checks for a release, in the same headless mode and demo home: each scenario in `scripts/smoke/scenarios.py` drives the dev build through real use (edits and saves, commands in zsh, bash and fish, the `impulse` CLI, agent hooks, a fresh managed language-server install with trust, tasks from creation to Finish, Agent Integration installing the MCP server, opencode's plugin replacing opencode 1's plugin and MCP entry, its two halves driven by a stand-in opencode (`scripts/smoke/opencode-agent.mjs`, which needs `node`), the MCP server asking for a task and an agent finishing its own, session restore) and checks the state its actions dump (`dump=`) plus the files and git repositories they changed. Build with `build.sh --dev` first; results and window images are left in `target/smoke/<scenario>/`. `IMPULSE_SNAPSHOT_APP` runs the scenarios against the Rust app, as for `capture.py`; the editor and lsp scenarios then drive its editor with `edit=` and `lsp=` instead of `js=` (`edit_action`, `LSP_STEPS_RUST`).

## History

Impulse was originally a Rust workspace with a GTK4 Linux frontend and most backend logic in Rust crates (`impulse-core`, `impulse-editor`) behind an 88-function FFI. It was rewritten Mac-first in Swift in 2026; the last Rust-era release (including the Linux app) is tag `v0.29.0`. If you need old behavior for reference, read the Rust sources at that tag — the fixtures under Tests/\*/Fixtures were generated from it.
