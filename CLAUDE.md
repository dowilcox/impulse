# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Impulse?

Impulse is a terminal IDE for macOS and Linux: a terminal emulator with Warp-style command blocks combined with a code editor, git review and history, and coding-agent support in a tabbed interface. It is all Rust: the app is built on GPUI and GPUI Kit (Metal on macOS, Vulkan on Linux) from one codebase that works the same on both platforms; terminal emulation is `alacritty_terminal`.

## Keeping this file current

Every feature change checks this file in the same commit. When you add, remove, rename or move a feature, target, directory, script, file format, setting mechanism, crate or key type, re-read the sections it touches (the architecture tree, Key patterns, Scripts, build commands) and fix whatever is now wrong or missing. Every type, file, function and flag named here must exist in the code, so grep this file for anything you rename or delete. Update `AGENTS.md` (the short version for other agents) when its facts change, and `docs/` when user-facing behavior changes.

## Architecture

```
impulse-terminal/         terminal emulation (alacritty_terminal),
                          OSC scanning (133 marks, 7 cwd, 6973 command
                          metadata, 9/777/99 notifications, 9;4 progress,
                          1337 attention, 21337 status), command blocks,
                          history; images by the kitty graphics protocol
                          (graphics/: APC commands, ImageStore with direct
                          placements anchored to rows like blocks and
                          virtual ones, Unicode placeholders, decoding off
                          the reader thread)
impulse-kit/              pure logic (no UI, testable on any OS), its
                          built-in files in resources/ (themes/,
                          shell-integration/, agents/opencode-plugin/),
                          compiled in: glob, close risk, shell parser, themes, file URIs
                          and language ids revived from v0.29.0;
                          ChromePalette, TextFile, WorkspaceTrust, AppPaths,
                          Settings + the settings catalog (settings.json,
                          settings::catalog's SettingItems; its
                          schema checked by settings_schema),
                          keybindings (Mac and Linux defaults), KeyEncoder
                          (key_encoder), Monaco's line commands, LayoutTree,
                          palette + fuzzy matching, file tree patches,
                          previews (comrak; markdown_blocks for the native
                          preview), Monaco/Markdown theme colors,
                          dates (History's and the palette's h: "2d ago",
                          and "Oct 5, 2026 at 3:27 PM"), tree_expansion (the file tree's open
                          folders per project, file-tree-expanded.json),
                          problems, review comments, update checker, shell
                          (types, quoting, login shell, integration),
                          completion, command history (SQLite), kitty
                          keyboard, terminal hints/paths/find,
                          ChildProcess, SessionState (session-state.json +
                          scrollback files), project config + settings file +
                          task environment + detector, task registry and
                          worktree tasks, commit graph, git helpers (the
                          editor's change marks and conflict blocks:
                          git/editor_marks.rs), agent
                          state machine, subagents (agents::subagents:
                          Claude Code's, from its hooks, and their
                          clashing edits), hook events + installers, MCP
                          installer, JSONC edits and task policy/briefs
                          ported; Vim mode's state machine (vim/: modes
                          including Visual Block in vim/block.rs,
                          counts, motions, operators, text objects,
                          registers, `.`, search as you type, marks,
                          macros, `:` commands); tested
                          against the golden fixtures in tests/fixtures/
impulse-git/              git: libgit2 (git2,
                          vendored 1.9.7, no network transports) for reads,
                          the git CLI for writes, safety snapshots, task
                          landing, change moves, operation abort, search,
                          watcher; checked against the golden fixtures in
                          tests/fixtures/
impulse-protocol/         control-socket messages (the same bytes the
                          Swift app spoke, so `impulse` tools and agents'
                          configs from before keep working), the
                          socket client, the MCP server and tool catalog,
                          and the agents' tool answers (TaskListing, …)
impulse-cli/              the `impulse` command-line tool (in the Mac
                          bundle's Contents/Resources/bin, beside the app in
                          the Linux packages): open, edit (waits for the
                          tab to close), review, split, tab, notify, status,
                          checkpoint, tasks, turns, comments, problems,
                          blocks, unsaved, project, hook, mcp
impulse-lsp/              the language-server client
                          (no UI, any executor), LSPConfig (lsp.json, root
                          detection), TextEditApplier and WorkspaceEdit,
                          managed (npm-installed servers); its
                          e2e test runs rust-analyzer when installed
impulse-app/              the app on GPUI + GPUI Kit for
                          macOS and Linux: workspaces (folder + Scratch)
                          with tabs of split panes (LayoutTree, draggable
                          dividers, zoom; preview tabs from a single click
                          in the file tree, pinned tabs, tabs dragged to
                          reorder, the tab context menu), commands as one RunCommand
                          action bound from impulse_kit::keybindings,
                          workbench chrome (the titlebar, workspace rows
                          with their chips, the status bar's items:
                          status_item, the sidebar's edge dragged:
                          SidebarDrag, the Workspaces section's handle:
                          Workbench::workspaces_handle), listening ports (port_scan.rs over
                          impulse_kit::listening_ports: /proc on Linux,
                          lsof on macOS), the terminal (the login shell
                          with integration, shell_launch.rs: its startup
                          files, and the fallback control socket, in a
                          fresh 0700 folder of the run's own),
                          its own painted grid (each glyph painted at its
                          cell, from one
                          shaped row: ShapedRow; underlines per cell
                          span; box-drawing and block
                          characters drawn as shapes, terminal_glyphs.rs;
                          the shell's blank padding between blocks collapsed
                          to a gap by RowLayout; images programs show,
                          terminal_images.rs: placeholder cells and direct
                          placements painted from GPU textures, and an
                          image path previewed while ⌘ is held),
                          keys (the kitty keyboard
                          protocol once a program pushes its flags,
                          impulse_kit::kitty_keyboard; Option as Meta by
                          the side held on the Mac), input methods,
                          selection, mouse reporting,
                          settings.json (SettingsStore, watched and merged;
                          checked against its schema in an editor tab by
                          settings_json.rs over impulse_kit::settings_schema;
                          Workbench::settings_banner while it doesn't
                          load), Increase Contrast and Reduce Motion
                          (display_options.rs: macOS's display options or
                          GNOME's high-contrast and animations keys,
                          followed while running), screen readers through
                          GPUI's AccessKit tree (roles and labels on
                          buttons, tabs, rows and tree items; the terminal's
                          screen text while assistive technology is
                          attached),
                          session restore (session.rs, with an agent's resume command
                          typed into its terminal's bar, each window's frame
                          in AppKit's coordinates (frame_string, saved_bounds)
                          and the front window: autosave every 15 s
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
                          scopes, unified and split diffs with word-level
                          changes and syntax colors (review_view/syntax.rs:
                          each side through GPUI Kit's tree-sitter
                          highlighter), stage/unstage/revert of files, hunks and
                          selected lines (review_view/selection.rs: gutter
                          click, Shift extends, ⌘C, Esc, the line menu),
                          viewed marks, the j/k/n/p/s/u/x/v/c/o/t keys and
                          ⌘Y ⇧⌘Y ⌥⌘Z; the scope and context menus in the
                          header, Edit Diff and Open on each file
                          (ReviewEvent::OpenDiff, ShowChanges handled by
                          Workbench::review_event); the navigator in
                          review_view/navigator.rs: filter, viewed bar,
                          files by folder; the snapshot's review-… actions
                          through ReviewView::debug_action; review comments
                          in review_view/comments.rs, spanning a selection:
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
                          ref chips, paging, Current branch / All branches,
                          the filter with author:, path:, since: and until:
                          and its presets, unpushed and unpulled marks, the
                          fork point, refs refreshed in place, the commit's
                          details over its Review list; history_view/menu.rs:
                          the commit's and each ref's menus, carried out by
                          history_actions.rs), right-click menus and
                          dropdowns (context_menu.rs: each described once,
                          MenuSpec; NSMenus through GPUI Kit's NativeMenu
                          on macOS, Ctrl-click opening a right-click menu
                          and a button's dropdown below it; PopupMenu
                          elsewhere), command palette and quick open
                          (palette.rs: modes by prefix, recents in
                          palette-recent-commands.json in the data
                          directory), Markdown preview (markdown_view.rs:
                          impulse_kit::markdown_blocks drawn with the
                          preview's styles, its text selectable as one
                          document through GPUI Kit's SelectableText, Run on shell blocks as
                          Workbench::run_from_preview) and SVG preview
                          (sanitized by impulse_kit::svg_preview) in or
                          beside the editor, Settings and Keyboard Shortcuts tabs
                          (settings_view.rs, keybindings_view.rs; shortcut
                          overrides rebind live through
                          commands::rebind_on_change), git
                          (repository_state.rs: RepositoryState per
                          repository root via RepositoryStore, refreshed by
                          impulse_git's RepoWatcher (RepositoryEvent::FilesChanged
                          for each change it sees, which a Review follows)
                          and after every action,
                          operations serialized with safety snapshots, CLI
                          reads through RepositoryState::read_git, restricted
                          in untrusted folders; changes_panel.rs: the Changes
                          panel (a GPUI list of ChangeItems) and commit composer, rows opening Review,
                          the list's keys (Show Changes focuses it, the
                          first row selected) and Move to New Task for one
                          file (TaskSheet::move_paths);
                          git_actions.rs: GitActions/GitPrompts and the git
                          commands (Switch Branch is the palette's b:); branch_manager.rs:
                          Manage Branches, with merge, rebase, delete
                          (Undo), rename and publish; file_icons.rs:
                          material icons from icon-mapping.json, served as
                          material/<file>.svg), the terminal input bar
                          (terminal_input_bar.rs over command_editor.rs:
                          SF Mono on macOS; chips, shell
                          colors, ghost suggestions, Tab menu,
                          history from command_history.rs, which shares
                          history.sqlite3 and backs the palette's h:
                          mode, Import Shell History and Clear Command
                          History), find (terminal_find_bar.rs over the find bar
                          terminals and editors share, find_bar.rs: FindBar and
                          its FindHost backend), and
                          block commands, the ⋯ and right-click menus
                          (terminal_blocks::BlockMenu), programs' progress
                          and status on tabs and workspace rows
                          (TerminalView::progress, SessionStatus), hints
                          and links (web and mailto
                          links open, file: ones in the editor, any other
                          scheme asks first: open_link)
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
                          one app per user (instance.rs: a second launch
                          on Linux hands its folder, files and commands to
                          the first over $XDG_RUNTIME_DIR/impulse/instance.sock;
                          macOS's open-URLs), Use Impulse as $EDITOR (EDITOR/VISUAL set to
                          `impulse edit` by control_server::terminal_env),
                          the file tree following the disk (tree_watch.rs:
                          every listed folder watched on its own, listed
                          again when its entries change; the open folders
                          remembered per project, Workspace::save_expanded),
                          the file tree's keys (Workbench::tree_key over
                          tree_focus and tree_selection) and context menu
                          (file_tree_actions.rs: new file and folder, open
                          to the side, mention in an agent, history,
                          reveal, copy paths, git actions, rename, Trash
                          with Undo on Linux; drag
                          and drop moving project items and copying ones
                          from outside, Workbench::tree_drop),
                          close_flow.rs (asking before closing a pane,
                          tab, workspace or window, or quitting: a pinned
                          tab, Save, Don't Save or Cancel for each unsaved
                          editor, then running terminal work from
                          impulse_kit::close_risk; quitting asks once about
                          every window's unsaved documents and running work;
                          closing the last window saves the session and quits),
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
                          the fixed items' actions and keys), the Linux
                          window frame (window_frame.rs: client-side
                          decorations, the titlebar's empty stretch moving
                          and zooming the window, minimize, maximize and
                          close at its right end, its edges and corners
                          resizing it: window_frame::resize_edges),
                          alert.rs (alert sheets in AppKit's NSAlert
                          layout: 260 wide, stacked pill buttons, an accessory, fields
                          and checkboxes; trust prompts, confirmations,
                          unsaved changes, git errors, name and tag
                          prompts), overlays.rs (every dialog opens there:
                          hung from the titlebar as a sheet, not dimming
                          the window on macOS, and debug builds report
                          where it is; the palette,
                          the agents inbox and toasts carry the same probe,
                          and overlays::frame_start forgets one closed
                          since the last frame), toasts.rs (bottom-center over the status bar,
                          up to three, ChromeButton actions),
                          workbench_workspaces.rs (recent folders in
                          recent-workspaces.json via
                          impulse_kit::recent_workspaces, Rename Workspace,
                          Scratch Workspace: Workbench::show_scratch,
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
                          notify-send, or on macOS GPUI's Notification
                          Center (show_system_notification; osascript only
                          outside an app bundle), a click on one showing
                          its terminal (on Linux where notify-send takes
                          --action), withdrawn when its attention clears
                          (on Linux where it takes --print-id;
                          agent_notify::clear), with
                          the sound `notification_sound` names
                          (notification_sound.rs: the compiled-in WAVs by
                          NSSound from memory on macOS, by paplay, pw-play
                          or canberra-gtk-play from a copy in
                          ~/.cache/impulse on Linux, the desktop's own
                          sound suppressed; a bell that beeped notifies
                          silently; choosing one in Settings plays it);
                          badge.rs: the terminals needing attention on the
                          Dock tile, or Unity's LauncherEntry by gdbus;
                          agent_send.rs: text for agents as a paste, queued
                          while one is mid-turn (AgentTracker::pending_text),
                          Send Selection to Agent, the agent items of
                          Problems' Fix with Agent, Review's comments menu,
                          conflicts' Ask Agent and a block's Send to Agent;
                          agent_composer.rs: the composer over an agent's
                          TUI with @ mentions; agent_toolbelt.rs: the
                          toolbelt under its terminal, Review Turn, the
                          Turns menu with Restore and the Subagents menu;
                          agent_tasks.rs: `impulse
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
                          Revert, Stage, Diff and Review, Show Commit
                          for This Line (blamed when asked),
                          merge conflicts tinted with Accept Current,
                          Incoming or Both on each marker line
                          (EditorTab::conflict_bars), from
                          impulse_kit::git::editor_marks; Monaco's keys
                          (editor_actions.rs: block comment, Go to
                          Bracket, F8, Expand/Shrink Selection, Cursor
                          Undo, trim trailing whitespace…, Home and ⇧Home
                          to each cursor's first non-blank (gpui-base's
                          smart_home), ⌘L Expand Line Selection, the
                          Mac's ⌃ text keys, keyboard folding (⌥⌘[ ⌥⌘]
                          ⌘K ⌘0 ⌘K ⌘J), and linked HTML
                          tag names over impulse_kit::linked_tags), font
                          settings listing families (fonts.rs), bracket pair colors (impulse_kit::bracket_pairs),
                          with its guides and the matching-bracket box
                          (editor_brackets.rs),
                          tree-sitter grammars for Monaco's languages, with
                          the vs-dark/vs rules the themes inherit for
                          JSON and HTML, Monaco's TypeScript keywords and
                          capitalized types, and its shell builtins in the
                          type color, in .sh files and sh/shell Markdown
                          fences (monaco-bash, monaco-markdown;
                          base_rules.rs), links underlined
                          and opened on Cmd/Ctrl+click (editor_links.rs),
                          sticky scroll (impulse_kit::sticky_scroll),
                          the minimap over the editor's right edge, beside the
                          scrollbar (editor_minimap.rs on impulse_kit::minimap,
                          room kept by Editor::scrollbar_gutter), find and
                          replace (editor_find.rs: the terminal's FindBar at the
                          top with Monaco's replace row, ⌥⌘F, F3 / ⇧F3, over
                          GPUI Kit's search with SearchOptions; its own panel
                          off with searchable(false)),
                          word-based suggestions without a language
                          server (impulse_kit::word_completions, other
                          open files' words too per the setting);
                          occurrence and selection
                          highlights as range decorations from document
                          highlights or impulse_kit::occurrences, Monaco's
                          context menu, Go to Type Definition,
                          Implementations and Declaration through
                          DocumentLink::navigate, Peek Definition and
                          the places Go to References and the Go to
                          commands find in a peek docked under the
                          editor (editor_peek.rs), Change All Occurrences,
                          the Diff View (editor_diff.rs: the index
                          version read-only beside the buffer, aligned by
                          hatched padding view zones (gpui-base's ViewZone)
                          with one scroll offset, or inline in one read-only document,
                          under a bar with the summary, ↑ ↓, the layout
                          switch and Done; impulse_kit::git::diff_view;
                          lines and changed words tinted under the text in
                          Monaco's colors (gpui-base's set_line_backgrounds
                          and range fills), impulse_git::buffer_diff::changed_words,
                          lines with nothing to line up with tinted whole;
                          Monaco's diff gutter (set_line_decorations_width,
                          − and + against the code) and overview ruler;
                          long unchanged runs on both sides hidden
                          (gpui-base's set_hidden_ranges) behind a "hidden
                          lines" view zone; the git peek and conflict actions
                          sit in view zones too),
                          signature help over the caret with
                          impulse_lsp::signature; inlay hints
                          (editor_inlay.rs: DocumentLink::inlay_hints for
                          the lines on screen and 30 around, again after
                          edits and scrolls, drawn by gpui-base's
                          set_inlay_hints; editor_inlay_hints' Ctrl+Alt
                          modes); completion snippets with tab stops
                          (gpui-base's insert_snippet, the tab's
                          set_snippet_variables); smooth_scrolling
                          (gpui-base's set_smooth_scrolling, Go to Line
                          through animate_scroll_offset); Vim mode in
                          editor_vim.rs over impulse_kit::vim, its keys
                          taken ahead of GPUI Kit's in the VimNormal
                          context, the mode in the status bar), workspace trust, language
                          servers (lsp_hub.rs: one per server and folder for
                          the whole app, its events passed to each workspace
                          using it; missing servers told once and
                          looked for again after 15 s, crashed ones restarted
                          after 2, 10, 30 or 120 s, showMessage as toasts,
                          $/progress in the status bar, shut down at quit,
                          restarted when lsp.json changes their command,
                          args or options (LspHub::reconfigure, watched by
                          Workbench::start_lsp_config_watch);
                          impulse-lsp's requests time out with
                          $/cancelRequest, documents sync by changed
                          ranges where the server takes them, and
                          completions reach GPUI Kit's editor as one
                          character-column edit each (a snippet's
                          additionalTextEdits kept apart),
                          impulse_lsp::completion::editor_completions) (code actions, formatting, rename in
                          Monaco's inline box (EditorTab's RenameBox) and
                          references with edits across files), the
                          headless snapshot mode of debug builds
                          (debug_snapshot.rs: the --impulse-snapshot
                          flags, actions and dump= format, edit= and
                          lsp= driving the editor (EditorTab::debug_edit
                          and debug_lsp); window images by ImageMagick's
                          import on X11, and on macOS rendered off screen
                          by GPUI's Window::render_to_image, which needs
                          impulse-app's `snapshot` feature (GPUI's
                          test-support, refused in release builds); there
                          windows open unshown and stay transparent and
                          click-through (crate::open_window, keep_hidden)
                          in an accessory app (no Dock icon, menu bar or
                          notifications), and the run draws their frames
                          itself, keep_drawing, as GPUI draws none for a
                          covered window; GPUI's pointer stays off them
                          except where an action clicks, keep_pointer, so
                          the user's real mouse never hovers in a shot)
                          that scripts/smoke and scripts/docs
                          drive, the
                          Component Gallery of debug builds
                          (component_gallery.rs: every built-in theme's
                          chrome in a window of its own),
                          --install-lsp-servers;
                          scripts/spike/linux-shot.sh screenshots it on Linux
vendor/                   fonts, lucide, highlight.js (the website's)
                          (committed; refreshed by scripts/vendor-*.sh);
                          gpui-kit/ (GPUI Kit's gpui-base and
                          gpui-component, patched in place through
                          [patch.crates-io]; its README lists the patches)
assets/                   file icons + icon-mapping.json (read at run time,
                          copied into the packages), logo, DMG background;
                          icons/impulse/, icons Impulse draws
                          in Lucide's style where Lucide has none
                          (flag-checkered, symbol-variable), compiled into the app
                          (resources.rs OWN_ICONS) and loaded as icons/<name>.svg;
                          sounds/{ack,two-tone,chime}.wav, the notification sounds
                          (made by scripts/sounds/make-notification-sound.py,
                          compiled in by notification_sound.rs);
                          macos/: Info.plist.in (the Mac bundle's
                          Info.plist, filled in by package-macos.sh) and
                          Impulse.entitlements
docs/                     user documentation, one Markdown page per area;
                          docs/images/ is generated by scripts/docs/capture.py;
                          docs/plans/ holds plans (not published)
website/                  impulse-terminal.app: homepage + demo, docs page
                          template (built by scripts/build-site.py)
scripts/                  build.sh, release.sh, package-linux.sh and
                          package-macos.sh (the packages), vendor-*.sh,
                          build-site.py, docs/ (screenshot capture),
                          smoke/ (release smoke checks), check-macos.sh
                          (type-check the app for macOS from Linux),
                          sounds/ (the notification sounds), spike/
CHANGELOG.md              user-facing changes per release
```

Dependency direction: `impulse-app` → {`impulse-kit`, `impulse-git`, `impulse-lsp`, `impulse-protocol`, `impulse-terminal`}; `impulse-git`/`impulse-lsp` → `impulse-kit`; `impulse-cli` → `impulse-protocol`. `impulse-app` pins `gpui-kit` exactly and gets GPUI through it, with GPUI Kit's `gpui-base` and `gpui-component` from `vendor/gpui-kit/` (`[patch.crates-io]`). Command blocks are listed without their output; fetch one block by id when you need it.

### Why the crates

- **impulse-kit** has no UI and no platform frameworks, so its logic is headless-testable on any OS. No GPUI there.
- **impulse-git** uses libgit2 (`git2`, vendored 1.9.7, built WITHOUT network transports) for reads (status, diff/hunks with word-level spans, blame, log, ignore checks). Writes (stage, commit, push/pull, stash, branches, worktrees, conflict resolution) run the `git` CLI, and destructive ones take a safety snapshot under `refs/impulse/` first so they can be undone. Aborting a merge, cherry-pick or revert falls back to a hard reset plus the work recorded when the operation started (`<git dir>/impulse-operation.json`) when git's own `--abort` refuses. No OpenSSL anywhere. New git APIs use the real `git` CLI as the test oracle.
- **impulse-lsp** owns language-server processes with hand-rolled Content-Length framing, no UI and any executor.
- **impulse-terminal** wraps alacritty_terminal, a complete, battle-tested VT emulator: the PTY, grid/scrollback, damage tracking, selection, scrollback search, and OSC 133/7/6973 command-block tracking. `impulse-app` owns all rendering and input encoding.
- **impulse-protocol** is shared by the app and the CLI, so the control socket's messages have one definition.

### Golden-fixture parity (important)

Behavior is checked against fixtures generated from the original Rust implementation (tag `v0.29.0`), the spec the Swift app was also held to: `impulse-kit/tests/fixtures/` and `impulse-git/tests/fixtures/` (themes → Monaco JSON, git hunks/word-diff/blame/status, shell-parser tokenizations, file-tree patches, palette filtering, glob, close risk). Treat fixtures as the spec: never regenerate them from the code's output. If behavior changes intentionally, update the affected fixture explicitly in the same commit and say why.

## Build & Development Commands

```bash
./scripts/build.sh --dev             # debug build: target/debug/impulse-app (+ the CLI); on the Mac with the snapshot feature
./scripts/build.sh                   # release packages in dist/ (package-linux.sh or package-macos.sh)
./scripts/build.sh --sign --notarize --dmg   # macOS: signed, notarized dist/Impulse.app + .dmg

cargo run -p impulse-app -- --cwd <folder> --open <file>   # the app
cargo build --profile dev-app -p impulse-app -p impulse-cli   # optimized, still the dev data folder (target/dev-app/): the app to try out
cargo build -p impulse-app -p impulse-cli --features impulse-app/snapshot   # Mac: the build capture.py/run.py snapshot (window images)

cargo test --workspace
cargo test -p impulse-terminal       # block/OSC/history tests
cargo test -p impulse-kit            # logic vs the golden fixtures
cargo test -p impulse-git            # git layer vs the golden fixtures and real git
cargo test -p impulse-lsp            # LSP client (+ rust-analyzer e2e)
cargo test -p impulse-protocol -p impulse-cli   # control protocol, MCP, CLI
scripts/check-macos.sh               # cargo check for macOS from Linux
cargo fmt && cargo clippy
```

A debug build (debug assertions on) keeps its own settings, session, socket and data under `impulse-dev`, so it runs beside an installed release.

`.github/workflows/tests.yml` runs `cargo fmt --all --check` and `cargo test --workspace --locked` on Linux and macOS for pushes to main and for pull requests: keep the workspace formatted with `cargo fmt`, not `rustfmt` by hand (it takes each crate's edition from Cargo.toml).

## Key patterns

- **Window, workspaces, tabs, panes:** each window is one `Workbench` entity, opened by `open_main_window` (every window goes through `crate::open_window`, so snapshots can keep it off screen). It holds `Vec<Workspace>` (`workspace::Kind::Folder` or `Scratch`), each with its own `Vec<Tab>`; a `Tab` is a `LayoutTree<PaneId>` plus `panes: HashMap<PaneId, Surface>`, with `focused`, `zoomed`, `preview` and `pinned`. `Surface` is the closed list of what a pane shows; a new kind of pane is a new variant. Drawing is in `workbench_chrome.rs` (sizes from `impulse_kit::chrome::metrics`, colors from `ChromePalette`) and `workbench_panes.rs`. Views are GPUI entities that `cx.notify()` themselves and report up through events the workbench subscribes to (`TerminalViewEvent`, `EditorTabChanged`, `ReviewEvent`, `RepositoryEvent`); app-wide state is a GPUI `Global` (`SettingsStore`, `RepositoryStore`, `CommandHistory`, `Toasts`, `IncreaseContrast`). Git, disk and process work runs in `cx.background_spawn`, and its result comes back through `update` on the main thread. Closing goes through `close_flow.rs` (pinned tabs, unsaved editors, then running work from `impulse_kit::close_risk`); what closed is kept as a `SessionTab`/`SessionWorkspace` for `Workbench::reopen` and the Undo toast. Every dialog opens through `overlays::open_dialog` (alerts through `alert::Alert::show`), never GPUI Kit's directly, so debug builds can report where it is.
- **Commands, keys, settings:** there is one action, `commands::RunCommand { id }`; the palette, the menus (`menus.rs` over `impulse_kit::menus`) and shortcuts all send it, and `Workbench::run_command` matches on the id. `commands::catalog` lists them (`impulse_kit::command_palette::builtin_items` plus the `WORKBENCH`/`PANES`/`BLOCKS` tables). Shortcuts come from `impulse_kit::keybindings::bindings(overrides, Platform::current())`: `commands::bind_keys` binds them tagged `COMMAND_KEYS`, and `commands::rebind_on_change` swaps them in place when `keybinding_overrides` or `custom_keybindings` change. Key contexts are `Terminal`, `CommandEditor` and `ImpulseEditor`; on Linux plain Ctrl+letter chords go to the shell in a terminal (`keybindings::is_shell_chord`), reaching their commands with Shift added (`keybindings::terminal_aliases`); the editor keeps its editing keys (`editor_keeps`) and Vim outranks both (`editor_vim::command_overrides`). Every setting is one line in `impulse_kit::settings`'s `settings!` list (field, settings.json key, default) plus a `settings::catalog()` `SettingItem`, which feeds the Settings tab and `settings::json_schema()` (checked in the editor by `settings_json.rs`). `SettingsStore` (`get`, `update`) watches `settings.json` and merges edits made outside Impulse with its own (`Settings::merged`). Impulse's files live in `impulse_kit::paths::data_dir` (`~/Library/Application Support/impulse` on macOS, `$XDG_DATA_HOME/impulse` on Linux; debug builds use `impulse-dev`), and `IMPULSE_NO_PERSISTENCE` turns saving off.
- **Session restore:** `session.rs` over `impulse_kit::session` (`SessionState` → `SessionWindow` → `SessionWorkspace` → `SessionTab` → `SessionSurface`) writes `session-state.json` at quit and, so a crash loses little, from `session::init`'s 15-second autosave whenever `SessionState::autosave_key` changes (structure only, not titles or frames). `SessionState::persist` writes the scrollback files, named by content (`scrollback_file_name`), and removes stale ones only after the JSON is written. Window frames are stored in AppKit's coordinates (`frame_string`, `session::saved_bounds`) so the file means the same on both platforms. Nothing is restored when `restore_session` is off or the launch named a folder, command or file (`--cwd`, `--run`, `--open`).
- **Terminal:** `impulse_terminal::TerminalBackend` owns the PTY, grid, scrollback, damage, selection, search and OSC 133/7/6973 block tracking; `TerminalView` owns painting and input. Output is event-driven: `set_wakeup_callback` pings a channel, and `TerminalView::drain` on the main thread runs `poll_events` → `take_damage` → `cx.notify()`. Painting is one `canvas` building a `Frame` from `TerminalView::snapshot`: `write_grid_to_buffer` fills a binary grid (`FIXED_HEADER_SIZE` header, `CELL_STRIDE` cells) parsed by `Grid::read`, re-read only after damage or a scroll/resize; the block overlay is cached by `block_overlay_key`. Rows are shaped once and each glyph placed at its own cell (`Frame::paint_text`); box-drawing characters are shapes (`terminal_glyphs::paint`), and `RowLayout` collapses blank padding between blocks. The input bar stands in for the prompt, which isn't drawn. The scanner also picks out kitty graphics APCs and the CSI sequences that clear images (erase display, the alternate screen, `ESC c`) and answers `CSI 16 t` and primary device attributes (`DEVICE_ATTRIBUTES` in `backend.rs`, kept from alacritty, whose VT102 answer stalls `kitten icat`); `graphics::ImageStore` keeps the images, read by `TerminalBackend::image_layer` and `image`, placeholder cells arrive as `EXTRA_IMAGE_CELL` extras, and `TerminalView::fit` tells the PTY the cell's size in device pixels, which programs size images by. The core drops any OSC over its limit (`osc_scanner`'s `MAX_OSC_LEN`) before vte sees it, and keeps alacritty's scrollback at the user's limit plus `SCROLLBACK_HEADROOM`, trimming it itself so block rows stay exact. Keys go through `impulse_kit::key_encoder::encode`, or `impulse_kit::kitty_keyboard` once a program pushes kitty flags (`TerminalMode::KITTY_*`, bits 11–15). Alt is Meta per `terminal_option_as_meta` on macOS (`key_encoder::alt_is_meta_held`) and always Meta on Linux. Text Impulse types into a terminal is quoted for that terminal's shell (`impulse_kit::shell::quoting::quoted`/`escaped` with `TerminalView::shell`; fish quotes differently) and has its control characters stripped (`quoting::without_control_characters`). Completions escape only the completed part (`CompletionCandidate::kept_prefix`). The input bar is a `CommandEditor` (`command_editor.rs`) fed by `impulse_kit::completion` and `command_history::CommandHistory` (`history.sqlite3`). A dropped `TerminalView` hangs up its shell with `TerminalBackend::close` without waiting for it; while the shell exits its output is still read (`stop_child`: on macOS a shell that writes as it exits, as fish does, can't finish exiting otherwise); at quit `impulse_terminal::wait_for_closed_terminals` waits for them all at once, so quitting costs the slowest shell, not their sum.
- **Shell integration:** `impulse-kit/resources/shell-integration/{bash,zsh,fish}.sh` (compiled in, `impulse_kit::shell::integration::script`) are loaded by `shell_launch.rs` (bash `--rcfile`, zsh a `ZDOTDIR`, fish `--init-command`). They emit OSC 133 marks, OSC 7 (cwd) and OSC 6973 (`Command=`; `Names=` and `Path=` only on change). Every mark carries the terminal's nonce (`IMPULSE_SHELL_NONCE`, `OscScanner::with_nonce`); once one has arrived, marks without it are ignored, so printed output can't fake them.
- **Editor:** GPUI Kit's `Editor`/`EditorState`. Its `gpui-base` and `gpui-component` crates are vendored in `vendor/gpui-kit/` and swapped in with `[patch.crates-io]`; gaps that can only be fixed inside GPUI Kit are patched there, each marked `// Impulse:` and listed in `vendor/gpui-kit/README.md`. One `EditorTab` per file, loaded and saved by `impulse_kit::text_file` (UTF-8 only, BOM kept, writes through symlinks; over `LARGE_FILE_SIZE` read-only, over `MAX_EDITOR_FILE_SIZE` not opened, and `Workbench::offer_pager` pages it in a terminal instead). Saving is `save_pipeline.rs`: it asks if the file changed on disk, and runs formatters and commands on save only in trusted folders. `editor_watch.rs` reloads clean editors with `EditorTab::reload_from_disk`, one undoable edit. Monaco, as the Swift app shipped it, is the reference: its keys (`editor_keys.rs`, bound in `ImpulseEditor > Input`), its token colors (`kit_theme.rs`, `base_rules.rs`), Vim mode (`editor_vim.rs` over `impulse_kit::vim`, in the `VimNormal` context). Language features plug into GPUI Kit's providers: `lsp_hub::DocumentLink` implements `CompletionProvider`, `HoverProvider`, `DefinitionProvider` and `CodeActionProvider`, and editors with no server get `WordCompletions`.
- **Language servers:** a workspace's `LspHub` exists only once its folder is trusted (`Workbench::enable_lsp`). The servers themselves are app-wide (a thread-local registry keyed by server id and project root), shared by every workspace and window that needs them, and stopped once none does. `LspHub::servers_for` starts only servers that `lsp.json` names and that are installed, in the file's project root (`impulse_lsp::config::detect_project_root`), never above the trusted folder (`WorkspaceTrust::trusting_folder`). The global `lsp.json` (`$XDG_CONFIG_HOME/impulse/lsp.json`, else `~/.config/impulse/lsp.json`) may define servers; a project's `.impulse/lsp.json` may only choose among existing ones. `LspClient::request` is a future with a per-method timeout (`timeout_for`); a timed-out request fails and the server gets `$/cancelRequest`. Each open document has one `DocumentLink` per server, shared by every editor of the file in any window, and `DocumentLink::sync` runs before each request so it's never answered against old text. Edits reaching other files (rename, code actions, `workspace/applyEdit`) are applied by `workspace_edits.rs` (`PendingEdit` over `impulse_lsp::workspace_edit::WorkspaceEdit`): open editors take their part as one undo step, other files are rewritten on disk, and creating, moving or deleting files, or editing outside `user_edit_boundary`, asks first. Crashed servers come back after `RESTART_DELAYS`; `LspHub::reconfigure` restarts those whose `lsp.json` entry changed.
- **Review and History:** native GPUI. `ReviewView` is one GPUI `list` (`ListState`) of `Row`s. `rebuild_rows` splices only the changed range, so the list stays where it was scrolled. Diffs are read in the background with `impulse_git::scoped_diff` (`changed_files_in_scope`, `file_diff`), and syntax colors come from GPUI Kit's tree-sitter `SyntaxHighlighter`, old and new sides separately (`review_view/syntax.rs`). Live scopes are re-read only when the repository's watcher has fired and the view is drawn again (`refresh_if_stale`); a fixed commit never is. Comments are `impulse_kit::review_comments::CommentStore` files in the data directory (`review/<digest>.json`), never in the repository. `HistoryView` shows the selected commit's diff in an embedded `ReviewView`. Markdown preview (`markdown_view.rs`) draws `impulse_kit::markdown_blocks`, parsed by comrak with raw HTML left out; don't turn it back on (`markdown_preview` sets `render.r#unsafe = false`).
- **Git only:** Impulse talks to git (libgit2 through `git2` for reads, the `git` CLI through `impulse_git::cli` for writes) and nothing else: no hosting platform's CLI or API (`gh`, `glab`, REST), so every feature works the same on any host. Host-facing bits stay generic: `impulse_kit::git::refs::RemoteWebUrl` only rewrites a remote into `https://host/owner/repo`, and after a push the server's own `remote:` lines are shown (`GitServerMessage::parse`), links included. Anything new must work from git alone. Each repository root has one `RepositoryState` (`RepositoryStore::state`), refreshed by `impulse_git::RepoWatcher` and after every action; writes go through `RepositoryState::run`, one at a time per repository, destructive ones after a safety snapshot under `refs/impulse/` (`impulse_git::safety`; with `require_snapshot`, not at all if it fails). Aborting a merge, cherry-pick or revert is `impulse_git::operation_abort::abort`, which falls back to the work recorded at the start (`<git dir>/impulse-operation.json`) when git's own `--abort` refuses. New git APIs use the real `git` as the test oracle.
- **Workspace trust:** `impulse_kit::trust::WorkspaceTrust` (one `Arc` shared by every window, `Workbench::trust`, saved in `trusted-folders.json`) lists the folders whose code Impulse may run on its own, plus folders kept restricted inside a trusted one (`restrict`: tasks made from someone else's branch, `TaskRecord::from_remote`). Language servers, formatters and commands on save, background fetch (`start_auto_fetch`) and project-config commands check it; anything new that runs project code unprompted must too. libgit2 reads never run repository-configured programs. Git CLI reads in untrusted folders go through `RepositoryState::read_git` with `cli::restricted_environment` (no fsmonitor, hooks, filters or merge drivers), and `operations::is_merged` and `ignored::nested_repositories` take `trusted`. Every git command starts with `cli::ALWAYS_ARGUMENTS` (`-c log.showSignature=false`): showing signatures would run a repository's own `gpg.program` and mix its output into what Impulse parses. `Workbench::request_trust` runs whenever a workspace is shown: servers start if it's trusted, otherwise `present_trust_prompt` asks once (`Workspace::declined`). The `workspace_trust` setting off, or `--trust`, skips asking.
- **Tasks:** a branch checked out at `<repo>.worktrees/<branch>` (`impulse_kit::git::worktree_tasks`), opened as a workspace; `.git/impulse/tasks.json` (`task_registry::TaskRecord`) lists the ones Impulse made, and `TaskRegistry::reconcile` adopts Claude Code's own worktrees (`<repo>/.claude/worktrees/<name>`, `claude --worktree` and subagents with worktree isolation) as tasks started for it. Overlaps come from `impulse_kit::git::task_overlap`, with `operations::predict_conflicts` (`git merge-tree`) on demand; `operations::is_merged` counts squash merges. Archiving (`task_cleanup.rs`) refuses a task holding a nested repository and revokes the task folder's trust. Finish Task (`task_finish.rs`) lands with `impulse_git::landing::merge_and_push`: a `--no-ff` merge in a throwaway worktree, pushed as the base, never touching the main checkout (`merge_locally` without a remote). `landing::delete_landed_branch` deletes the branch only if it hasn't moved. New Task can carry the main checkout's uncommitted files along (`impulse_git::change_move::move_changes`, with `change_move::undo`). Task terminals and scripts get `IMPULSE_TASK`, `IMPULSE_TASK_SLOT` and `IMPULSE_REPO_ROOT` (`tasks::identity`).
- **Project config:** `impulse_kit::project::config::ProjectConfig::load` reads the main checkout's `.impulse/project.toml`, committed or not (`tasks::load_project_config` goes through `main_checkout_root`, so every task gets what was saved). The local, never-committed `.git/impulse/project.toml` is layered on top and wins key by key (actions by name). A file's commands run only after the user trusts that exact content (`ProjectTrust`, by digest, in `project-trust.json`), and editing it asks again. Per-task values (`[worktrees.ports]` offset by the slot, `[worktrees.env]` placeholders) come from `project::task_environment`. Project Setup (`project_setup.rs`) proposes settings from `project::detector::suggest` and `project::compose_file::ComposeFile` and saves only its managed sections (`project::settings_file`); a local file Impulse wrote (`settings_file::is_written_by_impulse`) is folded in and removed, one edited by hand is left alone. On `RepositoryEvent::HeadMoved`, `dependency_prompts.rs` offers the `[on_change]` commands for the changed files (`project::dependency_changes`, which reads the reflog) and, after a merge, the check script.
- **Agents and the control socket:** each `TerminalView`'s `AgentTracker` finds a CLI agent from the foreground process (`agent_process.rs`) and `impulse hook` events (`impulse_kit::agents::hook_events`), driving `AgentStateMachine` (inbox, tab dots, notifications, badge). The app has one control socket (`control_server.rs`: the data directory on macOS, `$XDG_RUNTIME_DIR` on Linux; 0600, peer uid checked); every terminal gets `IMPULSE_SOCKET`, `IMPULSE_PANE_TOKEN` and `IMPULSE_CLI` (`control_server::terminal_env`), and the pane's window answers (`Workbench::handle_control`). Requests that start processes or write files (split, tab, checkpoint, tasks new/finish/archive, creating a file for `impulse edit`, comments resolve) and `blocks` also require the caller to be the pane's shell or a descendant (`control_server::descends`, the peer pid from `SO_PEERCRED` on Linux or `LOCAL_PEERPID` on macOS); otherwise work runs in the pane's folder, not the caller's. Turns are checkpointed under `refs/impulse/checkpoints/`. `impulse mcp` is `impulse_protocol::mcp::McpServer`: each tool call is one control request, answered with text plus `ControlResponse` data typed in `impulse_protocol::agent_tools`, with no `outputSchema`; it answers `initialize` (`PROTOCOL_VERSIONS`) and the stateless 2026-07-28 revision (`server/discover`), offers no tools outside Impulse, and settings can turn tools off (`agent_hooks::disabled_mcp_tools`). The Agent Integration dialog (`agent_integration.rs`) installs through `impulse_kit::agents::{hook_installer, mcp_installer}`: `claude mcp add-json` (never editing `~/.claude.json`), a table in Codex's `config.toml`, and for opencode the plugin folder from `impulse-kit/resources/agents/opencode-plugin/`. Agents' `tasks new|finish|archive` go through the user's own flows (`agent_tasks.rs`, `task_requests::decide` with `agent_task_creation`/`agent_task_limit`, the brief at `task_requests::brief_path`). Agents see each other through hook replies (`agent_hooks.rs`, `impulse_protocol::agent_tools::hook_reply`): a session summary, notes about shared files and changed dependencies, and a question before merging a branch a task is still on (`impulse_kit::git::command_inspector::merged_refs`). Claude Code's subagents are followed per terminal (`AgentTracker::subagents`, `impulse_kit::agents::subagents::Subagents`, fed by `agent_hooks::follow_subagents` from SubagentStart/SubagentStop and the Agent tool's events): the toolbelt's Subagents menu, a Stop whose `background_tasks` still holds subagents keeping the agent Working (`AgentHook::AwaitingSubagents`), and notes (`agent_hook_subagents`) when subagents share a checkout or edit the same file at once (`Subagents::launch_note`, `start_note`, `edited`). Impulse never calls an AI model itself.
- **Platforms:** one codebase. Platform code sits behind `cfg(target_os = …)` and must still type-check for macOS from Linux (`scripts/check-macos.sh`). macOS vs Linux: window frame (AppKit's, with `app_owns_titlebar_drag`, vs client-side decorations in `window_frame.rs`), menus (menu bar vs the titlebar's menu button, `menus.rs`), notifications (`agent_notify::post`: Notification Center, `osascript` outside a bundle, vs `notify-send`), badge (`badge::update`: Dock tile vs Unity's LauncherEntry by `gdbus`), display options (`display_options.rs`: accessibility settings vs GNOME's via `gsettings`). On Linux a second launch hands its request to the running app (`instance.rs`). Bundled icons and fonts are found by `resources.rs` (`$IMPULSE_RESOURCES`, `../share/impulse`, a bundle's `../Resources`, else the source tree) and served to GPUI as `icons/<name>.svg` (lucide) and `material/<file>.svg`.
- **Headless snapshots:** debug builds take `--impulse-snapshot <dir>` (`debug_snapshot.rs`): they open the window, run actions one after another (`dump=` writes state for the smoke checks; views add their own, such as `ReviewView::debug_action`, `EditorTab::debug_edit` and `debug_lsp`), write `window-<n>.png` and `snapshot.log` (with `overlay` lines from the dialogs' probes), and quit without saving the session or settings. Images come from ImageMagick's `import -window` on X11, and on macOS from GPUI's `Window::render_to_image` (the `snapshot` feature), with windows kept hidden. A new UI feature that the docs or smoke checks should reach needs an action there.
- **Themes:** TOML files in `impulse-kit/resources/themes/`, compiled in by `BUILTIN_THEMES` in `impulse_kit::theme` (a new built-in theme is the TOML plus its entry there). User themes live in `~/Library/Application Support/impulse/themes` on macOS and `$XDG_CONFIG_HOME/impulse/themes` on Linux. `theme::get_theme` resolves one into a `ResolvedTheme`; the chrome draws from `ChromePalette::new(theme, increase_contrast)`, GPUI Kit's own components from `kit_theme::apply`, and syntax colors map as `impulse_kit::monaco_theme::theme_to_monaco` (checked against the fixtures) does.
- **Version:** the top-level `VERSION` file is the single source of truth. `scripts/release.sh` writes it into every workspace crate's version, and the binaries report `env!("CARGO_PKG_VERSION")`. Never version-bump by hand.
- **Error handling:** library crates return `Result`s with descriptive messages (`impulse_git::GitResult`/`OperationError`, `Result<_, String>` elsewhere) and never show UI. The app logs non-fatal issues with `log` (`env_logger`) and shows failures the user must see as toasts (`toasts::show`) or alerts.

## Scripts

**Always use the existing scripts for their intended tasks — do not replicate their steps manually.**

- **scripts/release.sh <version> [--push]** — the ONLY way to release. Without `--push` it builds, on any branch and from a clean tree, exactly what a release would ship at `<version>`, then puts the version files back: on macOS the signed, notarized `.dmg` (`scripts/package-macos.sh --sign --notarize --dmg`), on Linux the tarball and `.deb` (`scripts/package-linux.sh`), plus `dist/SHA256SUMS`. `--push` runs only on macOS, on `main` in step with `origin/main`: after the build it commits the bump of `VERSION`, `Cargo.lock` and every workspace crate's version (the binaries report `CARGO_PKG_VERSION`), tags, pushes, and creates the GitHub release with the disk image; publishing it runs `.github/workflows/release-linux.yml`, which builds the Linux packages on x86_64 and arm64 inside Ubuntu 22.04 (so they need glibc 2.35, not the runner's), the x86_64 Arch package from the same binaries in an Arch container, attaches them and adds their lines to `SHA256SUMS` (run it by hand with a tag to rebuild them, or with no tag to build the branch without publishing, as a dry run). Never run `gh release create`, `git tag`, or manual version edits.
- **scripts/vendor-highlight.sh** — refreshes `vendor/highlight` (highlight.js for the website's code blocks and the demo's Review colors; pinned version and checksum).
- **scripts/vendor-lucide.sh** — downloads the pinned lucide-static icons into `vendor/lucide/` (add a name to its list for a new icon). An icon Lucide doesn't have goes in `assets/icons/impulse/` instead (listed in `OWN_ICONS` in `impulse-app/src/resources.rs`), out of this script's way.
- **scripts/sounds/make-notification-sound.py [output folder]** — synthesizes the notification sounds into `assets/sounds/`: `ack.wav` (the default: two upward chirps and a settled blip), `two-tone.wav` (a chirp up answered by one down) and `chime.wav` (two soft notes rising a fifth); 16-bit mono, peak -3 dBFS at most and no louder than the chime; standard library only, the same bytes every run. `impulse-app/src/notification_sound.rs` compiles them in (`Sound`) and sets the playback volume (`VOLUME`); rebuild the app after regenerating.
- **scripts/generate-icon-mapping.py <upstream-json> <output-json>** — regenerates `assets/icons/material/icon-mapping.json` (the file tree's icons, read by `impulse-app/src/file_icons.rs`) from upstream material-icon-theme.
- **scripts/build.sh [--dev] [--sign] [--notarize] [--dmg]** — builds Impulse for this platform: `--dev` the debug `impulse-app` and `impulse` CLI that capture.py and the smoke checks run (`cargo build -p impulse-app -p impulse-cli`, on the Mac with `--features impulse-app/snapshot`); no flags the release packages, `scripts/package-linux.sh` on Linux and `scripts/package-macos.sh` (with `--sign`, `--notarize` and `--dmg` passed on) on macOS.
- **scripts/build-site.py [--serve] [--check]** — builds impulse-terminal.app into `website/_site/` (homepage from `website/`, docs pages from `docs/*.md` with the sidebar following `docs/README.md`, changelog, the demo's themes from `impulse-kit/resources/themes/` with the Monaco colors `impulse_kit::monaco_theme::theme_to_monaco` gives them (ported to Python and checked against the `impulse-kit/tests/fixtures/themes/*.monaco.json` fixtures), and the demo's icons from `vendor/lucide` and `assets/icons/material`, picked the way `file_icons.rs` picks them) and checks every docs link, anchor and image. `.github/workflows/website.yml` runs it and deploys to GitHub Pages on pushes to main; `--serve` rebuilds on every change. Needs `pip install -r website/requirements.txt`. See `website/README.md`.
- **scripts/docs/capture.py [name-prefix…]** — regenerates the user docs' screenshots (`docs/images/`) from the debug build's headless snapshot mode, inside a mock project and home built by `scripts/docs/make_demo.py`; shots are defined in `scripts/docs/shots.py`. Build with `scripts/build.sh --dev` first (`target/debug/impulse-app`, or the binary in `IMPULSE_SNAPSHOT_APP`); on Linux it needs `DISPLAY` on an X server such as Xvfb. Shots are composed by `scripts/docs/imagetool.swift` on the Mac and its Pillow port `scripts/docs/imagetool.py` elsewhere, cropping to dialogs, the palette, the agents inbox and toasts from the rectangle the app reports (`overlay` lines in `snapshot.log`), as the app draws them inside its window; on the Mac the main window gets the inactive window buttons GPUI's off-screen images lack, from `scripts/docs/traffic-lights@2x.png`.
- **scripts/spike/linux-shot.sh <out.png> [impulse-app args…]** — screenshots the app under Xvfb with software Vulkan (needs Xvfb, xdotool, ImageMagick, mesa-vulkan-drivers); `cargo build -p impulse-app` first.
- **scripts/package-linux.sh [--no-build]** — builds `impulse-app` and the `impulse` CLI in release mode and packages them for Linux into `dist/`: a tarball (`bin/`, and `share/impulse/` with the lucide and material icons and the bundled fonts the app reads at run time, plus a desktop entry and icon) and, with `dpkg-deb`, a `.deb` of the same tree under `/usr`, and with `makepkg` (Arch and its derivatives) a `.pkg.tar.zst` of it for `pacman -U`; both depend on the newest glibc symbol version the binaries use (read with `objdump`), so a too-old system refuses them. The app finds those files through `impulse-app/src/resources.rs` (`$IMPULSE_RESOURCES`, `../share/impulse` or a bundle's `../Resources` beside the executable, else the source tree).
- **scripts/package-macos.sh [--sign] [--notarize] [--dmg] [--no-build]** — builds `impulse-app` and the `impulse` CLI in release mode and bundles them into `dist/Impulse.app` (bundle ID `dev.impulse.Impulse`, the Swift app's, so it installs over it and keeps its settings): the app in `Contents/MacOS/Impulse`, the CLI in `Contents/Resources/bin/impulse` (found by `control_server::cli_path`, and where `impulse` reads the bundle's version), the icons and fonts in `Contents/Resources/` as `package-linux.sh` lays out `share/impulse/`, the Info.plist from `assets/macos/Info.plist.in`, signed with `assets/macos/Impulse.entitlements`, notarized and stapled, and with `--dmg` `dist/Impulse-<version>.dmg`. Signing and notarizing take the environment variables its header lists. `release.sh` runs it (through `build.sh` or directly); by hand it's for trying a release bundle.
- **scripts/check-macos.sh [cargo check args…]** — type-checks `impulse-app` for `aarch64-apple-darwin` from Linux, so code behind `cfg(target_os = "macos")` compiles before anyone builds on a Mac: stand-in `cc`/`ar` make the files C build scripts expect (nothing is linked) and an empty shader file stands in for GPUI's Mac-only shader step. Needs `rustup target add aarch64-apple-darwin`.
- **scripts/smoke/run.py [name-prefix…]** — smoke checks for a release, in the same headless mode and demo home: each scenario in `scripts/smoke/scenarios.py` drives the debug build through real use (edits and saves, commands in zsh, bash and fish, the `impulse` CLI, agent hooks, a fresh managed language-server install with trust, tasks from creation to Finish, Agent Integration installing the MCP server, opencode's plugin replacing opencode 1's plugin and MCP entry, its two halves driven by a stand-in opencode (`scripts/smoke/opencode-agent.mjs`, which needs `node`), the MCP server asking for a task and an agent finishing its own, session restore) and checks the state its actions dump (`dump=`) plus the files and git repositories they changed; the editor and lsp scenarios drive the editor with `edit=` and `lsp=`. Build with `scripts/build.sh --dev` first; results and window images are left in `target/smoke/<scenario>/`.

## History

Impulse was originally a Rust workspace with a GTK4 Linux frontend and most backend logic in Rust crates (`impulse-core`, `impulse-editor`) behind an 88-function FFI; the last release of that app (including the Linux app) is tag `v0.29.0`, and the golden fixtures were generated from it. It was then rewritten Mac-first in Swift (AppKit + SwiftUI, Monaco in WKWebView, a Rust terminal core behind a C FFI) through 0.36, and rewritten again in Rust on GPUI for macOS and Linux on the `rust` branch, which replaced the Swift app in October 2026. For the Swift app's behavior, read `impulse-macos/` at the commit before its removal (`git log --diff-filter=D -- impulse-macos/Package.swift`).
