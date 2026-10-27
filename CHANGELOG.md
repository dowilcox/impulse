# Changelog

All notable changes to Impulse are documented in this file.

## Unreleased

### One Impulse for macOS and Linux

- Impulse is rewritten in Rust and runs on Linux again, as well as macOS: the same app on both, drawn on the GPU (Metal on macOS, Vulkan on Linux). It keeps your settings, sessions, trusted folders, command history and agent integrations, and the `impulse` command-line tool works as before.
- Linux releases come as a `.deb` for Debian and Ubuntu and a tarball for other distributions, for x86_64 and arm64. Building from source on Arch and its derivatives also makes a package for `pacman -U`.
- The editor is Impulse's own instead of Monaco, keeping Monaco's keys and behavior. Inline blame is gone; Show Commit for This Line still blames the cursor's line.

### Images in the terminal

- Programs can show pictures in the terminal: plots, `kitten icat`, yazi's previews, images in Neovim, an agent's preview of a pasted screenshot. Impulse speaks the kitty graphics protocol, with its Unicode placeholders, so images also work in tmux and over ssh. Turn it off with **Programs may show images** in Settings ▸ Terminal.
- Hold ⌘ (Ctrl on Linux) over an image's path in the output to preview it.

### Fixes

- In bash, command history, block headers and the palette's history now show the command as you typed it: a whole pipeline (`ls | wc -l`, not just `ls`), with aliases as written instead of expanded.
- Closing a terminal that wanted your attention (finishing a task whose agent had just notified you, say) clears it from the Dock badge and Notification Center instead of leaving the badge counting it.
- On a busy Mac, a git command that had already finished could look stuck, and one with a time limit could be reported as timed out and stopped.
- Impulse's MCP server answers tool lists and tool calls even when your Mac is busy; an agent's request could go unanswered.
- Git commands start faster: Impulse runs the developer tools' git directly instead of through `/usr/bin/git`, which starts it in turn. That's one program launch per command instead of two, which matters most where security software checks every launch.

## 0.36.0

**opencode 2.** opencode 2 runs one background service for all your
terminals and can't load the plugin Impulse installed for opencode 1.
Impulse's plugin is now a folder with a half in each terminal and a half
in the service, so every session still reports to its own tab, hears
about the other tasks, and gets Impulse's tools, even with several
terminals sharing one service. Open Agent Integration and install
opencode's hooks again; opencode 1 is no longer supported.

### Agents

- opencode's plugin installs as `~/.config/opencode/plugins/impulse/`, replacing opencode 1's `plugins/impulse.js`. It gives opencode Impulse's tools itself (`tools.impulse.…`): an MCP server would run in opencode's shared service, where it can't tell which terminal is asking. Agent Integration's MCP side offers to remove the `impulse` entry older Impulses added to `opencode.json`, leaving your comments and formatting as they were.
- opencode asks you before merging a branch a task is still working on, with Impulse's reason, as Claude Code does, instead of being stopped once. Shell commands opencode runs act for the pane their session is in.
- A Claude Code turn that an API error ends (a rate limit, an overloaded server) shows Finished, and the notification names the error. Install Claude Code's hooks again to get the new `StopFailure` hook.
- A Claude Code turn you interrupt with Esc goes back to Idle within seconds instead of staying Working, and approving a tool that reports nothing afterwards no longer leaves it on Needs Input.

### Workbench

- Drag a workspace up or down the sidebar to reorder it; a repository's header moves its whole group.
- In the editor, ⌘D (add the next occurrence), ⌥⌘↑/↓ (add a cursor above or below) and ⇧⌘↩ (insert a line above) stay Monaco's while an editor has focus, instead of splitting, moving between or zooming panes; those are still in View ▸ Panes and the palette.

## 0.35.0

**opencode joins Claude Code and Codex.** Agent Integration installs
Impulse's plugin for opencode, so its tab shows exactly when it's
working, needs you or has finished, and its session comes back after a
restart. In a repository with tasks it hears about the other workspaces,
like Claude Code: a summary when it starts, notes about shared files and
changed dependencies, and a stop before it merges a branch another task
is still working on. New Task can start it on a brief, and Impulse's MCP
server installs into opencode's own config.

### Agents

- Agent Integration has opencode for hooks (a plugin, `~/.config/opencode/plugins/impulse.js`) and for the MCP server (an `impulse` entry in its `opencode.json` or `opencode.jsonc`, edited in place so your comments and formatting stay). Its plugins can't ask you before a command runs, so a risky merge is stopped once, telling opencode why and to ask you, and goes ahead when it runs again.
- Codex's MCP entry gives Impulse's tools an hour: Codex gave up after 5 minutes while `create_task` waited for you to answer New Task, or `finish_task` for the check script. Install it again from Agent Integration.
- `list_turns` and `impulse turns` give the latest 10 turns (`--limit` for more) and say how many earlier ones they left out, instead of every turn the terminal ever recorded.

### Fixes

- A task started for an agent starts it on its brief even when your shell has a function or alias called `cat` (fish's `cat=bat`, say): the brief came out empty, and the agent started with no prompt.
- `create_task`'s answer no longer marks the new task as the asking agent's own workspace.
- Agent Integration is wider, so Claude Code's scope menu fits beside four agents instead of pushing the sheet's content aside.

## 0.34.0

**Agents can ask Impulse.** Impulse has an MCP server, `impulse mcp`,
that any agent loading Model Context Protocol servers can use: it lists
the repository's tasks and looks at one, waits for a task's agent, and
starts a new task with an agent working on a brief, after you approve it
in New Task. It also hands agents your review comments, the language
servers' problems, other terminals' commands and output, files you
haven't saved, and the trusted project scripts. Install it with
**Install Impulse's MCP Server…**; every tool is also an `impulse`
command.

### Agents

- New Task opens filled in when an agent asks (`impulse tasks new`, `create_task`), saying who asked and showing the brief; Claude Code and Codex start with the brief as their first prompt. **When an agent asks to start a task** can create them without asking in trusted projects, or not at all; **Open tasks agents may start** limits them to 4 per repository.
- Finish Task can be started by an agent (`impulse tasks finish`): it runs up to Land and waits for you. Archive Task… too (`impulse tasks archive`), with its questions and Undo.
- An agent can mark a review comment addressed with a note; Review shows it with **Accept** and **Reopen**.
- New commands: `impulse tasks show`, `turns`, `comments`, `problems`, `blocks`, `unsaved` and `project`.
- The Agent Hooks sheet is Agent Integration, with Hooks and MCP Server.
- **Agents may read terminal output** decides which terminals agents' tools see: their own workspace's (the default), the window's, or none.

### Terminal

- **Option as Meta** (Settings ▸ Terminal, off by default) makes Option send Esc + key, on the left, right or both sides. Off, Option types what your layout gives it (`@`, `[`, `{`, `|` on non-US layouts).
- Scrolling back through output is no longer pulled to the bottom by programs that redraw in place, command blocks stay exact once scrollback is full, and kitty keyboard, IME and mouse reporting got fixes.

### Workspaces

- Switch Workspace remembers every folder you open, not only the last 20. **Clear Recent Workspaces** (in the list and the palette) empties it, with Undo.

### Security

- History, and every other git read, no longer checks commit signatures. With `log.showSignature` on (in your config, or in a repository's own), git ran the signing program for every signed commit, so a repository you hadn't trusted could run a program of its choosing, and the program's output got mixed into what Impulse read.
- Git reads in folders you haven't trusted run without fsmonitor, hooks, filters or merge drivers. A task made from a remote branch stays restricted inside a trusted repository, and archiving a task takes back its folder's trust.
- Project settings you haven't trusted never reach agents: their `[on_change]` commands are only passed to Claude Code from a trusted `project.toml`. The trust question lists every command, and Project Setup only trusts what it wrote.
- Completion, paste and drag and drop quote names for the terminal's own shell (fish included), and the input bar drops control characters. Shell-integration marks need the terminal's own key once one has arrived, so printed output can't fake a command block, and oversized escape sequences can't use up memory.
- `impulse split`, `tab`, `checkpoint` and `edit` of a new file only work from the pane's own shell or a program it started.
- Hints never open `file://` links, the Markdown preview's Run button only runs a single visible command in a trusted folder, and a language server's edits that create, move or delete files, or reach outside the project, ask first.

### Fixes

- Moving changes into a task and archiving never delete a repository or submodule inside it. Undo of a move takes a snapshot first, and keeps the task when it can't put files back.
- A landed branch is only deleted if it hasn't moved; stash and uncommit act on what you saw; a failed Undo says so; archiving warns about ignored files it would remove.
- The editor reloads a file changed on disk in place, as one undoable edit, never takes a failed formatter's output, and notices changes made around opening and saving.
- Quitting while a session is being restored no longer wipes it, and edits to `settings.json` made outside Impulse are merged with Impulse's own.
- A very large port value no longer crashes Impulse at launch, and `impulse edit` on a pipe or device fails at once instead of hanging.
- Typing, terminal output, Review, History, the file tree and Quick Open do far less work, so Impulse stays responsive with many tabs and busy terminals.

## 0.33.1

**Project Setup shows what's saved, and saves what it shows.** A field
you empty stays empty after saving instead of filling back in with a
suggestion; what it found is offered beneath the field (**Use**,
**Add**). An entry that can't be saved as it is (a port that isn't a
number, a name given twice, a `{placeholder}` that doesn't exist) is
marked in red, and Save waits for it instead of dropping it.

### Project Setup

- Unsaved changes: the tab's title gets a `*`, ⌘S saves, and closing the tab, its window or Impulse asks first. Opening the tab again keeps your changes; opening it for another repository asks before discarding them.
- Saving keeps your place on the page instead of jumping back to the top.
- If `.impulse/project.toml` changed since the tab read it, Save asks: **Reload** or **Save Anyway**. It used to write the tab's older settings over the change.
- A `project.toml` that can't be read shows its error, with **Open File** and **Reload**, instead of looking like a project with no settings that Save would then overwrite.
- The note before saving names settings Impulse doesn't know in the sections it rewrites (`lint` under `[scripts]`), not only comments.
- A script pasted with Windows line endings no longer saves a `project.toml` that can't be read.
- A server every checkout shares on your Mac (MySQL on 3306, Redis on 6379, a mail catcher) keeps its port unless the Compose file publishes it. With Laravel's own `.env`, tasks were connecting to a database port nothing listened on.
- The suggested check script and actions use the project's package manager (`pnpm run lint`, `yarn test`, `bun run test`), not always npm.
- **Dump and load** starts the task's database, gives up after two minutes instead of waiting forever, and runs in fish as well as bash and zsh. A setup script with the earlier command gets the new one when Project Setup opens.
- Switching the database option back to **Nothing** no longer leaves a setup script starting with `&&`.
- Removing a port no longer crashes Impulse.

### Also

- The session is saved every 15 seconds while it changes, so a crash no longer loses the workspaces and tasks opened since launch.
- A History, Review or tool tab shows its own workspace's branch, changes and folder in the titlebar, Changes panel and status bar, not those of the workspace selected before it.
- In the Changes panel, **+** stages a file on the first click.
- Disabled buttons look disabled.

## 0.33.0

**Project settings live in the project.** Project Setup saves into
`.impulse/project.toml`, and every task reads it from the main checkout,
committed or not, so a change reaches the next task as soon as it's
saved. The "Save to: This Mac / The project" choice is gone. Settings an
earlier version saved on this Mac only (`.git/impulse/project.toml`) are
shown on the screen and move into the project on the next save; a file
there that you edited by hand is left alone, with a note naming what it
overrides. See
[Project configuration](https://www.impulse-terminal.app/docs/project-config/).

### Tasks

- Project Setup has one **Ports** section: the Compose file's fixed ports and container names, which **Move them in each task** moves and renames per task without changing the file, beside the ports in `.env`, each showing what task 1 gets.
- Project Setup no longer proposes moving a `.env` port that belongs to a Compose service or another machine (`DB_PORT` beside `DB_HOST=mysql`). Moving it broke the task's connection to its database.
- **Copy into tasks** and **Clone into tasks** take paths of your own, typed or picked. The database's data folder and its Compose service are fields to edit, offered whether or not a database was found.
- Finish Task updates the main checkout even when it has untracked files (an uncommitted `.impulse/project.toml`, say); only changes to tracked files hold it back. If the update would overwrite an untracked file, Finish names it.
- Finishing or archiving a task no longer adds a Scratch workspace beside the workspaces still open.

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
