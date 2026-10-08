# Changelog

All notable changes to Impulse are documented in this file.

## 0.32.0

**Running tasks and agents in parallel.** Tasks now get an environment of
their own, see each other, and finish in one step. Two agents can work on
one repository without sharing ports, databases or containers; Impulse
says when their work starts to overlap, tells the agents too, and lands
each task with a merge commit (or pushes it for review) without touching
the main checkout. [Tasks](https://www.impulse-terminal.app/docs/tasks/)
covers all of it, including a section on running agents in parallel.

**Git only.** Impulse no longer talks to GitHub. The titlebar's pull
request chip, the pull request commands, the `pr:` palette mode and
importing review threads from a pull request are gone; everything works
the same on any git host. After a push, the server's own message is
shown, with its link (GitLab's "create a merge request", say) as **Open
Link**. **New Task from Branch…** (the palette's `task:` mode) opens any
branch, local or remote, as a task, and **Open Repository in Browser** and
**Copy Remote URL** replace the per-host links.

**Documentation** is online at
[impulse-terminal.app](https://www.impulse-terminal.app/docs/), with a new
homepage.

### Tasks

- New Task starts from the remote branch (`origin/main`): fetched in the background in trusted folders, with a **Fetch** button otherwise, and a note when your local branch has commits that aren't pushed. A new task's branch no longer tracks its base, so its first push publishes it under its own name.
- Impulse keeps a list of the tasks it made in `.git/impulse/tasks.json`, with each one's base and slot. Worktrees you made yourself are left alone.
- **Project Setup** (File ▸ Project Setup…, or from the New Task sheet) looks through a repository and proposes what its tasks need: files to copy, folders to clone, ports and values, a database, Compose changes, setup, check and archive scripts, and rules for when files change. Save it for this Mac only (`.git/impulse/project.toml`, the default) or for everyone (`.impulse/project.toml`); the local file wins.
- Each task can get ports and values of its own: `[worktrees.ports]`, moved by 100 × the task's slot, and `[worktrees.env]`, with placeholders such as `{task}` and `{slot}`, written into its copy of `.env`. Task terminals and scripts get `IMPULSE_TASK`, `IMPULSE_TASK_SLOT` and `IMPULSE_REPO_ROOT`.
- Dependency folders (`vendor`, `node_modules`) and a database's data folder can be cloned into new tasks, instantly on APFS, with the database's Compose service stopped for a moment. A Compose project can give each task an override that renames its containers and moves its fixed ports.
- When a pull, merge or checkout changes a lock file, or another file named in `[on_change]`, Impulse offers to run its command (`composer install`), and after a merge, the `check` script.
- Workspaces of one repository that change the same files show a warning with the count. Its list names the files, opens them, and checks which would conflict, without touching either folder. A notification says so when two workspaces start to overlap (`task_overlap_notify`).
- **Finish Task…** takes a task from done to landed, in a tab that shows each step: committed, synced with its base (a merge, never a rebase), checked with the `check` script in a terminal, then merged with a merge commit and pushed, or pushed for review. The merge is made in a throwaway folder, so the main checkout isn't touched; it's fast-forwarded afterwards when it's clean, and the task is archived with its branch. The first Finish asks how work lands in the repository and remembers it (`[finish] land`).
- Tasks whose work has landed, squash merges included, are marked "merged". **Archive Merged Tasks…** archives them together, optionally deleting their branches here and on the remote, with one Undo. A task pushed for review gets a **Clean Up…** offer once it's merged.
- **Move Changes to New Task…** moves the main checkout's uncommitted files, or one file from the Changes panel, into a new task, and the main checkout goes back to its last commit for them. Undo puts them back.
- Merging a branch that a task is still working on asks first, naming the agent and the branch's last commit, and merges that commit.
- A workspace row shows ↓ and a count when its branch is behind its upstream; click it to pull.

### Agents

- `impulse tasks` lists the repository's workspaces from the caller's point of view: branches, bases with ahead/behind counts, agents and their state, uncommitted files, and the files each shares with the caller (`--json` for scripts). `impulse tasks wait <task>` waits until that task's agent stops working.
- Claude Code hooks now carry what Impulse knows: a summary of the other workspaces when a session starts, a note when an edited file is also changed in another workspace, a note when a pull or merge brought in changed dependency files, and a question before merging a branch a task is still working on. Each has a setting (`agent_hook_task_summary`, `agent_hook_shared_files`, `agent_hook_dependencies`, `agent_hook_merge_guard`). The Agent Hooks sheet says when installed hooks are out of date: install them again to get these.

### Git

- Abort works after you've edited files during a merge, cherry-pick or revert: when git's own abort refuses, Impulse puts the branch back and restores the work you had before, with Undo.

## 0.31.0

**Documentation.** `docs/` now explains every part of Impulse, with
screenshots: getting started, workspaces and tabs, tasks, the terminal,
agents, git, review, history, the editor, the command palette, project
configuration, the `impulse` CLI, settings and themes, keyboard shortcuts
and accessibility. Help ▸ Impulse Help opens it. The screenshots are
generated from a mock project by `scripts/docs/capture.py`.

### Workbench

- ⌘P (**Go to File…**) opens the palette's file mode from the menu and the palette alike.
- New setting `sidebar_tabs` lists the active workspace's tabs in the sidebar instead of the titlebar. It replaces `tab_bar_position`, which did nothing.
- The sidebar is shown and the previous session restored by default (`sidebar_visible`, `restore_session`).
- The workspaces section resizes by dragging the hairline under it; double-click goes back to fitting the rows.
- Workspaces can be reordered with **Move Up** / **Move Down** in a row's context menu, and the order (Scratch included) comes back with the session.
- Closing a workspace is one undoable step: Undo, ⌘Z or ⇧⌘T reopen it with its tabs. Each "Closed …" notice's Undo reopens its own item.
- Every command has one name in the menus, the palette and Keyboard Shortcuts, and the Git menu commands, Select Blocks and Last Failed Block can be given shortcuts.
- Shortcut hints use the Mac order (⇧⌘P) and menu titles use "…" throughout.
- The New Task, Branches and Agent Hooks sheets fit their content and use the theme's colors.

### Terminal

- Clicking into a terminal, or its input bar, clears its attention mark.
- Typing in the terminal at a prompt goes to the input bar instead of the shell's hidden line.
- While a command runs, the input bar sends what you type as is, and Return on its own answers "press Enter" prompts.
- ⌃C and other control keys reach a running program even right after selecting command blocks.
- → at the end of the line accepts the whole suggestion; ⌥→ accepts one word.
- zsh: history goes to your own `.zsh_history` again (it was written to a temporary file and lost when the tab closed), and a `ZDOTDIR` set in `.zshenv` no longer turns off shell integration.
- The input bar follows the terminal font size, and a program's OSC 21337 status color shows on its tab.

### Editor

- Tab width and indentation from Settings ▸ Automation ▸ File types now apply.
- Commands and formatters on save run with your login shell's `PATH`, accept a `{file}` placeholder, and report failures; quitting waits for them.
- Save & Close and Save when quitting run the whole save (formatter, commands on save, language servers).
- Impulse's shortcuts, such as ⌘D and ⌘G, work while the editor has focus.
- Vim mode understands `:w`, `:q`, `:wq` and `:x`.
- A project's `.impulse/lsp.json` applies to that project and reloads without a restart.
- Vue and Svelte files are highlighted, `.fish` files aren't sent to the bash language server, and `sourcekit-lsp` is listed among the system servers.
- Replace All in the project covers every match, not only the first 500.
- The change peek's Stage saves the file first, as its tooltip says.

### Git, Review and History

- Review can show more or less context: the `review_context_lines` setting and a Context menu in the header (changed lines only up to the whole file).
- Agent turns survive a relaunch: Review Last Agent Turn and Review's "Last agent turn" still work.
- Cherry-Pick, Revert, Keep Current and Take Incoming can be undone.
- Manage Branches ▸ Show History shows that branch.
- Review's comment count, Send and Copy cover the files on screen; Delete All Comments in Repository says how many are elsewhere.
- Folder history lists every commit under the folder.
- Publishing a branch and pushing tags use the branch's remote, named in the menus.
- ⌘↩ and ⇧⌘↩ commit (and push) from the commit message field.
- Old safety snapshots under `refs/impulse/oplog/` are pruned (two weeks, newest 200).
- The diff font follows the editor's font size.

### Tasks and agents

- New Task… started from a task's row branches from the main checkout and puts the folder beside it.
- Check Out Pull Request as Task… copies the same files as New Task… and asks before running the pull request's setup script.
- Archive Task… warns about ignored files it will delete, and its Undo lasts 15 seconds.
- Codex hooks can be removed from the Agent Hooks sheet, and project hooks (`.claude/settings.local.json`) are copied into new tasks.
- A finished agent shows a check mark, so it's told apart from one that needs input without relying on color.
- `impulse status` messages show in the agent list and toolbelt, `impulse split` no longer refuses to run, and the CLI rejects unknown options.
- Forget Trusted Folders and Restrict This Folder also forget trust in `.impulse/project.toml`.

### Settings and themes

- Run in Terminal shortcuts run the command line as typed (pipes, `&&`) in the focused terminal.
- Editing `color_scheme` in `settings.json` applies at once, user theme files with capital letters load, a theme file with a mistake says what's wrong, and the Theme menu shows each theme's name.
- Font zoom (⌘= / ⌘- / ⌘0) applies to every window.
- Keyboard Shortcuts warns about clashes with fixed shortcuts and explains why a key can't be used.
- Out-of-range numbers in `settings.json` are clamped the same way the Settings tab limits them.

## 0.30.0

**Impulse is now a native Mac app.** It was rewritten in Swift (AppKit +
SwiftUI); only the terminal emulation core is still Rust. **Linux is no
longer supported**: the GTK4 app's last packages are on the 0.28.0 GitHub
release, and 0.29.0 is the last version whose source includes it. Release
builds are for Apple silicon (macOS 26 or later).

**Workspace trust.** Opening a folder asks whether to trust it. Until you
do, Impulse runs none of its code on its own: no language servers (some run
a project's build scripts and tools), no formatters or commands on save, no
background fetch. A Restricted item in the status bar brings the question
back; the palette can trust, restrict or forget folders, and a setting
turns the question off. Folders from the saved session and recent
workspaces are trusted on the first launch.

### Terminal IDE redesign

Impulse moves from an Apple-styled tabbed app to a terminal IDE workbench,
with deeper git integration and first-class support for coding agents
running in its terminals. Impulse itself never calls an AI service.

**Workbench**

- Themed workbench chrome: titlebar tabs, a workspaces sidebar, docks and a status bar, with Lucide icons.
- Workspaces (folders with their own tabs), split panes of any surface, and session restore of every window's workspaces, layouts and terminal output.
- A fuzzy command palette with modes: files, `>` commands, `:` line, `%` text, `@` / `#` symbols, `b:` branches, `t:` tabs, `w:` workspaces, `h:` history, `pr:` pull requests, `set:` settings, `a:` project actions.
- Settings and Keyboard Shortcuts as tabs, described by one catalog (which also validates `settings.json` in the editor).
- Quick terminal on a global shortcut (off by default).
- Closing a tab or pane can be undone with ⌘Z for ten seconds; toasts and sheets replace blocking alerts.
- Sidebar panel shortcuts work like VS Code's: ⇧⌘E Files (new), ⇧⌘F Search, ⌃⇧G Changes; pressed again while the panel has the keyboard, they go back to the active tab.
- Tooltips show after half a second.

**Terminal**

- A multi-line command editor for input, with shell highlighting, history (Ctrl-R), completions for subcommands, options, branches, scripts and paths, and an underline on commands the shell can't run.
- Optional: ask fish for completions (`complete -C`) alongside the built-in ones.
- Command blocks: inline status, keyboard selection, bookmarks, a toolbar, and copy/send actions.
- Find with match counts and case/word/regex toggles; hints mode for URLs, paths, commit SHAs and ports in output.
- Persistent command history, desktop notifications for background terminals, listening ports per workspace, and iTerm2 session status (OSC 21337).
- The kitty keyboard protocol for programs that ask for it.
- Services: terminal selections work with the Services menu; "New Impulse Workspace Here" in Finder.

**Agents**

- Impulse notices coding agents in its terminals (Claude Code, Codex, Gemini CLI, Aider, opencode, Amp, Copilot CLI and more) and shows what they're doing.
- Each agent turn is checkpointed: review it, restore the files from before it, or send review comments back.
- A composer (⌘I) and toolbelt for agents; send code, files and command output to them.
- Task worktrees (`../<repo>.worktrees/<branch>`), agent hooks, session resume after a restart, and the `impulse` command-line tool.

**Git**

- A Changes panel with a commit composer and full keyboard control.
- A rebuilt, native review: navigator, scopes (unstaged, staged, branch, commit, range, stash, last agent turn, since last review), syntax-colored unified and split diffs, hunk and line staging, comments, and imported pull request threads.
- Live change marks in the editor with a peek, inline blame, and a side-by-side diff view you can edit.
- History with a commit graph, compare, filters (`author:`, `path:`, `since:`, `until:`) and fork-point dimming.
- Branch switching and management, undoable stash drop/pop, merge-conflict resolution in the editor (or by an agent), and GitHub pull requests through `gh`.
- A Git menu and palette commands: fetch (one or all remotes), pull with a chosen strategy, push, force push with lease, stash/pop, and undo last commit.
- Tags: create lightweight or annotated tags from History or at HEAD (the next version is suggested), optionally pushed on creation; push, delete locally or on the remote from a tag's menu.
- Merge and rebase from History and the branch manager, with Undo; open commits, tags and branches on GitHub, GitLab, Bitbucket, Codeberg or Azure DevOps.
- Git settings: pull strategy, `--follow-tags`, push new tags, and background fetch.
- Writes go through the git CLI; reads stay on libgit2.

**Editor and language servers**

- Problems panel, go to symbol in file or project, project-wide replace with a preview, and Markdown preview beside the editor with Run buttons on shell blocks.
- Language servers: cross-file renames and code actions applied across files with one Undo, commands and `codeAction/resolve`, `workspace/applyEdit`, document highlights, inlay hints (with a setting), type definition, implementation, server messages and progress. sourcekit-lsp serves Swift.
- Requests no longer queue behind each other, and every server for a language sees opened documents.
- Servers start in the background, get the files opened before they were up, and come back after a crash (with a growing delay); a file open in two windows is one document to them.
- Monaco 0.57.

**Accessibility**

- VoiceOver can read terminal text; agent status changes are announced.
- Reduce Motion and Increase Contrast are honored; keyboard-focused lists show a focus ring.

**Fixes**

- Edit ▸ Undo/Redo now reach text fields and the window (they sent a selector nothing implemented).
- Monaco could only edit its own file, so renames that touched other files failed.
- Editor files are never lost to the disk: files that aren't UTF-8 stay closed instead of opening empty, byte-order marks are kept, saving through a symlink writes its target, and a file changed on disk (by an agent, git or another app) is never silently overwritten or replaced — saving asks, and unsaved edits are kept with a notice.
- Pasted text can't end a bracketed paste early and run commands; pasting or dropping on a terminal whose input bar owns input fills the input bar.
- Live output keeps drawing after scrolling; a synchronized update left open by a program that died no longer freezes the terminal.
- Shell integration: bash records the commands you run (not its PROMPT_COMMAND hooks) and reads your login profile; folders and commands with non-ASCII names are tracked correctly; terminals get a UTF-8 locale.
- Markdown preview links and terminal OSC 8 links can no longer open arbitrary apps or pages that run commands.
- Git: reverting a hunk applies to the right block, partial staging refuses non-UTF-8 files instead of corrupting them, file names are never treated as globs, commit messages keep lines starting with #, and destructive actions don't run without their safety snapshot.
- Language servers, node and npm are found when Impulse is started from the Dock.
- Command block separators stay on their prompts when a resize rewraps long lines (opening an editor beside a terminal, for one).
- Clicking a short commit hash in terminal output selects that commit in History.
- A narrow terminal pane drops whole context chips instead of cutting the last one in half.
- Diagnostics were drawn a line and a column late.
- Workspace edits from a language server are refused when the file changed after the server computed them, instead of landing in the wrong places.
- Text a program prints can no longer pose as a command: command text needs the terminal's shell-integration nonce to reach blocks, Rerun, completions or history.
- Terminals stay responsive under floods of output, and shells no longer look like they run in Alacritty.
- Typing in the input bar and Tab completion don't block the window on the filesystem, git or fish.
- Git: timeouts stop hooks and ssh along with git, and long local operations (an LFS checkout) aren't cut off; background fetch no longer holds up staging and committing; stash review shows the untracked files a stash carries; agent-turn snapshots no longer take the index lock.
- History doesn't mix branches when switching scope mid-load; an agent turn that ends quickly still gets its end snapshot; the Branch Manager's confirmations, errors and Undo show on top of it.
- gh calls time out, and the login PATH includes what `.zshrc` / `config.fish` add.
- libgit2 1.9.7 (security fixes); highlight.js 11.12.
- Review and History: the file list's Viewed checkbox works, and a line's hover no longer sticks while scrolling.
- History's commit list has the keyboard when it opens, so ↑/↓ work right away.

## 0.29.0

The last release with the Linux (GTK4) app.

### macOS — Terminal backend migration and polish

**Breaking**

- **macOS minimum is now Tahoe (26.0).** Bumped from Sonoma (14). Required for modern AppKit APIs (`NSView.displayLink`) and to align the Swift deployment target with the Rust FFI build target, which eliminates linker warnings at build time.

**Terminal**

- Replaced SwiftTerm with an in-tree `impulse-terminal` backend wrapping `alacritty_terminal`, rendered via CoreText/CoreGraphics in `TerminalRenderer.swift`. Shell integration (OSC 133, OSC 7) is implemented via a custom PTY read thread and `OscScanner`.
- Custom NSView-based renderer with a `CADisplayLink`-driven refresh loop, run-based CoreText drawing, exact per-glyph cell positioning, and a binary grid snapshot buffer for zero-copy cell reads.
- Regex-based terminal search via alacritty's search engine.
- Live theme recoloring: changing theme applies to running terminals instantly.
- Split terminals no longer go black when a pane exits.

**Terminal settings that now actually take effect at runtime**

- Cursor shape (block/beam/underline)
- Cursor blink (with 0.5s timer, resets on keyboard input)
- `terminalBell` gates the bell beep
- `terminalBoldIsBright` substitutes palette 8-15 for bold ANSI 0-7
- `terminalScrollOnOutput` auto-follows output when not scrolled back
- Cursor color derived from the theme's foreground (previously hardcoded)
- Scrollback, shape, and blink are now set at creation and refreshed on any settings change

**Terminal input / selection**

- Mouse click/drag/release forwarded to the PTY as SGR or X10 escape sequences when the program enables mouse reporting (`CSI ? 1000 / 1002 / 1006`), including modifier flags. Restores tmux mouse mode, vim visual mouse selection, and clicks in TUI apps (fzf, lazygit).
- `NSTextInputClient` implementation for dead keys and IME composition. Marked text is drawn as an underlined overlay at the cursor position, and `firstRect(forCharacterRange:)` returns the cursor cell in screen coordinates so the IME candidate window anchors correctly.
- Right-click context menu with Copy / Paste / Select All / Clear.
- Wide-character (East Asian fullwidth) text now advances 2 columns correctly; subsequent characters on the same row stay aligned with the grid.

**Hyperlinks (OSC 8 + auto-detection)**

- OSC 8 hyperlinks tracked via a new `HYPERLINK` cell flag bit and resolved via `impulse_terminal_hyperlink_at(col, row)` FFI.
- Plain URL auto-detection scans each row with a `https?://…` regex on hover, trims trailing punctuation, and handles wide-char spacer cells.
- Hover shows a pointing-hand cursor and an underline across the detected range. Cmd+Click opens the URL in the default handler via `NSWorkspace`.

**Box-drawing characters**

- Added 50+ box-drawing characters rendered as primitives instead of font glyphs: eighth blocks (`▁▂▃▅▆▇` / `▉▊▋▍▎▏`), shade blocks (`░▒▓`), and all quadrant combinations (`▖▗▘▙▚▛▜▝▞▟`). Improves alignment for charts, progress bars, and TUI frames.

**File tree performance**

- Replaced the recursive SwiftUI `FileNodeView` with a flat `FlatFileRowView` rendered in a single `LazyVStack`. Only visible rows are materialized — previously all expanded nodes had live SwiftUI views, which caused major slowdowns at ~50+ expanded folders.
- `loadChildren()` moved off the main thread to eliminate UI hitches on expansion.
- Git status mutations on individual nodes no longer trigger re-renders of the entire expanded tree.
- Filesystem watcher changes now sync to the SwiftUI sidebar so externally created/renamed/deleted files appear immediately (previously only the hidden AppKit tree saw them until a manual refresh).

**Build / tooling**

- Swift package tools version bumped to 6.2 (required for `.macOS(.v26)`). Swift language mode pinned to `.v5` to avoid strict concurrency regressions on existing AppKit delegate code.
- `MACOSX_DEPLOYMENT_TARGET=26.0` exported in `impulse-macos/build.sh` before the Cargo FFI build, so Rust-compiled objects link cleanly against the Swift target with no "built for newer version" linker warnings.
- `LSMinimumSystemVersion` in the `.app` Info.plist bumped from 13.0 to 26.0.
- Fixed three clippy warnings: `if_same_then_else` in `theme.rs`, `redundant_guards` in `lsp.rs`, and `large_enum_variant` in `protocol.rs` (Boxed `EditorOptions` in `EditorCommand::UpdateSettings`).
- `CADisplayLink` replaces the deprecated `CVDisplayLink` API in the terminal renderer.
- Migrated to the flat `FlatTreeEntry` architecture for the SwiftUI file tree.

### Known limitations (in 0.29.0)

- **Terminal scrollback size only takes effect on new terminals.** Changing `terminalScrollback` in settings does not resize the buffer of already-running terminals (alacritty allocates the scrollback ring at `Term::new()` and does not expose a runtime resize API). Restart the tab or open a new terminal to apply the new size.
- **Cursor shape override is unconditional.** The user's `terminalCursorShape` preference is applied at the renderer layer and overrides any ANSI `DECSCUSR` escape sequence from running programs. Vim users who rely on per-mode cursor shape switching in insert/normal modes will see the same shape throughout. A future release will track program overrides separately so user preference acts as the default rather than a hard override.
- **OSC 133 prompt/command events are captured but not wired to UI.** The backend emits `PromptStart`, `CommandStart`, and `CommandEnd` events (visible in the CWD tracking path), but jump-to-prompt keybindings and exit-code display are not yet implemented. Requires exposing alacritty's absolute scrollback-line indices through the FFI so prompt positions survive scrolling.
- **Settings migration is additive only.** Users upgrading from pre-0.20 releases will keep their old persisted values for `terminalScrollOnOutput` and `terminalBoldIsBright` (both `false` in the previous default). New installs get `true` for both. Flip them in Settings → Terminal if you want the new defaults on an existing install.

---

Prior releases were not tracked in a formal changelog. See the [git history](https://github.com/dowilcox/impulse/commits/main) and [GitHub Releases](https://github.com/dowilcox/impulse/releases) for earlier changes.
