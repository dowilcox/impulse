# Parallel tasks: lessons and plan

Plan, proposed and decided 8 Oct 2026; each proposal records what was decided. Not user documentation: this folder isn't published to the website (`scripts/build-site.py` only builds `docs/*.md`).

Two Claude Code sessions worked on PulseBoard at once: one did a framework upgrade in a task (`pulseboard.worktrees/interia-upgrade`), and the other did a UX overhaul with 156 uncommitted files in the main checkout. Both finished, but most of the effort went into the setup around the worktree, not into the code. The full write-up is [Git Worktrees with Claude Code: Lessons from the PulseBoard Upgrade](https://claude.ai/artifact/94WxPQf4DsGvgns3gN1Wuq).

The short version: **a task isolates files, but not the environment, and not what the agents intend to do.** The two sessions collided only at merge time, because each one could only see its own folder. Impulse is the one program that sees every workspace, every branch and every agent's state at once, and it doesn't use that view yet.

## Status

Implemented 8 Oct 2026, every proposal, in the order below (commits `1fd53ac` to `3ffc43c`; user docs in `docs/tasks.md`, `docs/project-config.md`, `docs/agents.md`, `docs/cli.md` and `docs/git.md`). Where the build differs from the decisions:

- **Finish is a tab, not a sheet** (4). A sheet blocks the window, and the check's terminal output and conflict resolution both need it. The tab lists the steps; one that stops says why and offers what helps (Show Changes, Skip Check, Push for Review Instead), and Continue picks up from there.
- **"Main checkout is behind" lives on its row** (4): ↓3 on the main checkout's row, click to pull, rather than in the group header, which disappears once the task is archived and the main checkout is the only workspace left.
- **The throwaway merge folder skips post-checkout hooks** (4); merge and push hooks run as usual. A server refusal (a protected branch) is told apart from a moved base and offers Push for Review instead.
- **Finish's toast says Restore Task, not Undo** (4): it brings back the folder and the local branch; the pushed merge stays.
- **The overlap chip is on the workspace rows** (2), not the group header: one chip per workspace, with its own list.
- **Archive Merged Tasks… is also on the row menu** (12), and a task whose upstream is gone gets a hover hint, not a mark.
- **Moving changes picks one file or all** (11): the Changes panel has single selection, so its context menu moves one file and its ⋯ menu moves everything.

Still open: an agent starting Finish (`impulse tasks finish`, with Impulse asking you to approve the land step), which was decided to come after the tab.

## What went wrong, and what Impulse does today

| What went wrong                                                                                               | Impulse today                                                                | Proposal                                                                                                     |
| ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| The main checkout had 156 uncommitted files from another session; 17 of them overlapped the upgrade's         | Each workspace's changes are shown on their own; nothing compares workspaces | [2](#2-show-where-workspaces-overlap), [11](#11-move-uncommitted-changes-to-a-new-task)                      |
| The task had no `vendor/`, `node_modules/` or `public/build`; 131 tests failed on the missing Vite manifest   | `.worktreeinclude` copies files only; the setup script is optional           | [7](#7-clone-dependency-folders), [8](#8-suggest-a-setup-script)                                             |
| The dev stack couldn't run the task: fixed container names, fixed ports 8000/9080, one bind-mounted folder    | A task has no identity of its own beyond its folder                          | [1](#1-give-each-task-its-own-environment)                                                                   |
| The other session's Chrome DevTools MCP browser held the shared profile                                       | Outside Impulse                                                              | [13](#13-docs-running-agents-in-parallel)                                                                    |
| The main checkout was too dirty to merge into, so the merge was built in the task and pushed to `origin/main` | **Merge into main** runs in the main checkout                                | [4](#4-finish-task)                                                                                          |
| CI on `main` had been red since 14 Sep and nobody noticed                                                     | Nothing; CI isn't part of git                                                | Out of scope ([15](#15-git-only-no-hosting-platform-integrations)); [4](#4-finish-task)'s check runs locally |
| `interia-upgrade` moved three times while the main checkout was merging it; `main` now has three merges of it | No warning; agents can't see each other                                      | [3](#3-let-agents-see-each-other-impulse-tasks), [6](#6-warn-before-merging-a-branch-thats-still-moving)     |
| Git flagged 9 conflicts, but 25 files were broken; only `npm ci` + `tsc` found the other 16                   | Nothing runs after a merge                                                   | [4](#4-finish-task), [9](#9-offer-setup-when-dependencies-change)                                            |
| `git merge --abort` refused once conflicted files had been edited                                             | The banner's **Abort** runs `git merge --abort`                              | [5](#5-abort-that-works-after-edits)                                                                         |
| Five merged `fix/*` worktrees were still lying around, nested inside the repo under `.worktrees/`             | Archiving is one task at a time                                              | [12](#12-clean-up-finished-tasks)                                                                            |

Out of scope: the MySQL 8.4 data-directory upgrade and Claude Code pausing at irreversible steps. Both went as they should.

Already right, and worth keeping: task folders go beside the repo (`<repo>.worktrees/<branch>`), never inside it; `.env` is copied in; the setup script runs before the agent; Impulse's own **Merge** takes a safety snapshot first.

### A bug found while writing this (fixed)

A task started from a remote branch tracks it. With **From** set to `origin/main`, `git worktree add -b fix-x <path> origin/main` sets `fix-x`'s upstream to `origin/main` (git's default `branch.autoSetupMerge`). **Git ▸ Push** then sees an upstream (`GitActions.swift:382`) and runs a plain `git push`, which fails ("The upstream branch of your current branch does not match the name of your current branch"), and with `push.default=upstream` would push the task to `main`. Fixed in 47a2f23: `GitOperations.addWorktree` now passes `--no-track` when it creates a branch, which proposal 10 relies on.

## Proposals

Sizes are rough: **S** is a day or two, **M** about a week, **L** more.

### 1. Give each task its own environment

**Size M.** Fixes: fixed ports, shared database. Discussed 8 Oct; decisions below.

**Problem.** Docker Compose already names a project after its folder, so a task's stack is separate from the main checkout's by default. What collides is everything the project fixes by hand: `container_name`, host ports and the database name. A task has no number or name that a project can build unique values from.

**Proposal.** Each task gets a slot, a small number that is unique among the repo's tasks and kept for the task's lifetime (the main checkout is `0`). The project's settings name the ports and values that have to differ per task, and Impulse writes the task's values into the task's copy of `.env` when it creates the task. The settings live in the local, never-committed `.git/impulse/project.toml` that the [setup screen](#14-project-setup-screen) writes, or in a committed `.impulse/project.toml`:

```toml
[worktrees]
env_file = ".env"          # default; copied from the main checkout, then updated
port_offset = 100          # default; task n adds n × 100 to every port

[worktrees.ports]          # the main checkout's ports
APP_PORT = 8000
VITE_PORT = 5173

[worktrees.env]            # may use {task}, {task_}, {slot} and the port names
DB_DATABASE = "pulseboard_{task_}"
APP_URL = "http://localhost:{APP_PORT}"
```

For `interia-upgrade` in slot 1, the task's `.env` is the main checkout's `.env` with these lines changed: `APP_PORT=8100`, `VITE_PORT=5273`, `DB_DATABASE=pulseboard_interia_upgrade`, `APP_URL=http://localhost:8100`.

- **The file is the source of truth.** Laravel in its container, `docker compose` from any terminal, and Vite all read `.env`, so the values work outside Impulse too. A key that's already in the file is changed in place; a missing one is added at the end under a `# Impulse task interia-upgrade` comment. Everything else in the file, secrets and working values included, stays as copied.
- **The env file is always copied.** `env_file` is copied from the main checkout even when no `.worktreeinclude` pattern names it. If the main checkout has none, the task's file holds only these keys.
- **Written once.** Values are written when the task is created; editing the task's `.env` afterwards is yours. If `project.toml` changes later, existing tasks keep their values (re-applying them is a [setup screen](#14-project-setup-screen) action).
- **Slots.** A new task takes the lowest free slot whose ports are all free: not held by another task of the repo, and nothing listening on them (`PortScanner`). Archiving frees the slot; Undo takes it back if it's still free. Slots are stored in `.git/impulse/tasks.json`, which every worktree shares and which is never committed.
- **Terminal variables.** Only identity goes into the terminal environment, not the values above (a shell variable would silently override an edited `.env` in Compose and Laravel): `IMPULSE_TASK` (`interia-upgrade`, unset in the main checkout), `IMPULSE_TASK_SLOT` and `IMPULSE_REPO_ROOT`. Setup, archive and check scripts and project actions get them too.
- **Placeholders.** `{task}` is the folder name (`interia-upgrade`, fine in hostnames and Compose project names); `{task_}` has dashes as underscores (`interia_upgrade`, for database names); `{slot}` is the slot number; `{APP_PORT}` and the other port names are the task's ports.
- **Trust.** These keys apply only from a trusted file, like the scripts. Saving from the setup screen trusts the local file; a committed `project.toml` asks as it does today. The docs should say so.
- **Visible.** The New Task sheet's preview gains a line for them ("Ports: APP_PORT 8100, VITE_PORT 5273 · .env: DB_DATABASE, APP_URL"), and the task row's hover shows its slot and ports.

With `docker-compose.yml` reading `${APP_PORT:-8000}` and no `container_name`, each task runs its own stack on its own ports and database, and an archive script of `docker compose down -v` (run in the task folder, so Compose picks the task's project) tears it down.

**Decided.** Values go into the file, not the terminal environment. Per-task offsets (`port_offset`, default 100) on named ports, rather than searching for a free port per name, so a task's ports are predictable and stay clear of neighboring ports. The task's `.env` starts from the main checkout's `.env`, not `.env.example`, because the example lacks working values. Only worktrees Impulse creates (**New Task…** and **Check Out Pull Request as Task…**) get a slot and values; worktrees made any other way are left alone. A task's entry in `.git/impulse/tasks.json` is what marks it as Impulse's.

**Where.** Slot allocation, placeholder expansion and the `.env` line editing in ImpulseKit (`ProjectConfig` gains `envFile`, `portOffset`, `ports` and `worktreeEnv`), with tests. Writing the file in `createTask` after the copies (`TaskWorkflows.swift:249`). Terminal variables where the environment is built (`TerminalTab.spawnShell`, `TerminalTab.swift:1321`) and where the archive script runs (`removeTaskWorktree`, `TaskWorkflows.swift:488`).

### 2. Show where workspaces overlap

**Size M.** Fixes: finding the 17-file overlap at merge time instead of on day one. Discussed 8 Oct; decisions below.

**Problem.** Two workspaces editing the same files is the real risk, and git only reports it when you merge. Both sessions were rewriting the same 9 pages for days.

**Proposal.** Impulse works out which files each of a repository's workspaces changes, and shows where two of them change the same file.

- **Which workspaces.** The main checkout, plus every task Impulse created (from `.git/impulse/tasks.json`), whether or not it's open in the sidebar: a closed task can still have an agent running in another terminal. Worktrees made any other way are left out, as in proposal 1.
- **What a workspace changes.** For a task: the files changed on its branch since it left its base, plus its uncommitted files. `tasks.json` records the base each task was created from, so "since it left its base" is the merge base of the task and that branch, not a guess. For the main checkout: its uncommitted files plus its commits not yet pushed. Uncommitted files are already tracked for the sidebar's diff stats.
- **Every file counts.** No default ignore list: `composer.lock` or `package-lock.json` changed on both sides is one of the worst conflicts to untangle. Files that are only noise in a project can be ignored on the [setup screen](#14-project-setup-screen) (`[worktrees] overlap_ignore`).
- **Where it shows.** The repo group's header in the sidebar shows a chip ("17 shared files"). The rows' hover lists the files each shares. Clicking the chip opens a popover with each overlapping pair (`pulseboard ↔ interia-upgrade: 17 files`); a file opens in either workspace from there.
- **One notification per new pair.** When two workspaces that didn't overlap start to ("interia-upgrade and pulseboard now change the same 3 files"), Impulse notifies once, through the same path as agent notifications. More shared files between the same pair update the chip but don't notify again. A setting turns the notification off. Agents create overlap while you're looking elsewhere, so the chip alone could go unseen.
- **Conflicts on demand.** The popover's **Check for Conflicts** runs `git merge-tree --write-tree --name-only A B` for that pair, which merges two commits in memory without touching any folder (git 2.38 or later; macOS ships 2.54). For uncommitted work, A is a temporary commit of the working tree, made the way `SafetySnapshots` makes one, without a ref. Files that would conflict are marked in the list.

**Decided.** Shared files in the background, conflicts only when asked. In this session 17 files were shared, git flagged 9 conflicts and 25 files were broken, so shared files were the earliest and most honest warning; predicting conflicts in the background would also write a temporary commit on every recompute while agents edit. Overlap by folder or area ("both in `Pages/Teams/`") is left out of the first version as too noisy.

**Where.** Changed-file sets with libgit2 in ImpulseGit; pairwise overlap in ImpulseKit, with tests; `merge-tree` through the git CLI (the existing pattern for anything libgit2 doesn't do). Recomputed on repository-watcher events, debounced, for every workspace in the repository's group. Shown in `WorkspacesSection.swift`.

### 3. Let agents see each other: `impulse tasks`

**Size S to M.** Fixes: merging a moving branch; agents making sweeping edits without knowing someone else is in the same files.

**Problem.** Agents ran most of the git commands in this session from their terminals, so warnings in Impulse's UI would never have reached them. Each agent knows only its own folder.

**Proposal.** A new CLI command (`impulse status` already reports agent state, so this is `tasks`):

```
impulse tasks [--json]
```

It lists the repository's workspaces, the main checkout and every task Impulse created (as in proposal 2): folder, branch, base, head commit, agent and its state (`idle`, `working`, `needsInput`, `done` from `AgentStateMachine`), uncommitted file count, commits ahead of and behind the base, and the files each one shares with the caller's workspace (from proposal 2). `impulse tasks wait <task>` blocks until that task's agent is idle, so a session can wait for a branch to stop moving before merging it.

Agents learn about it through the hooks Impulse already installs, not through a `CLAUDE.md` / `AGENTS.md` snippet, which would have to be committed:

- **Session start.** Impulse's Claude Code `SessionStart` hook (`impulse hook claude`) prints a short summary, which Claude Code adds to the agent's context: the task it's in and its base, the repository's other workspaces and what their agents are doing, and "run `impulse tasks` before merging a branch or making sweeping edits". Anyone with hooks installed gets it with no change.
- **Edits to shared files.** A new `PostToolUse` hook, matched to Claude Code's file-editing tools, stays silent unless the edited file is also changed in another workspace (proposal 2's overlap set). Then it adds one line to the agent's context: "`Pages/Teams/Show.tsx` is also changed in task interia-upgrade, where Claude Code is working." This adds an event to **Install Agent Hooks…**, so existing installs need reinstalling; the sheet's status line should say when installed hooks are out of date.
- **Codex** only has an end-of-turn `notify`, so it gets the command and the docs, without the automatic context.

**Decided.** Both hooks are on by default, each with a setting to turn it off: the summary is a few lines, only inside Impulse terminals, and the edit-time line only appears when two workspaces really touch the same file. A merge guard for agents (a `PreToolUse` hook that makes Claude Code ask before merging a branch that's still moving) belongs with [proposal 6](#6-warn-before-merging-a-branch-thats-still-moving).

**Where.** A new message in ImpulseProtocol, a handler in `ControlServer`, the command in ImpulseCLI, and the docs in `docs/cli.md`. The hook output in `impulse hook claude`; the new event in the agent hook installer (`ImpulseKit` agents), with the docs in `docs/agents.md`.

### 4. Finish Task

**Size L.** Fixes: merging needing a clean main checkout; broken code passing a clean merge; leftover worktrees and branches. Discussed 8 Oct; decisions below.

**Problem.** The documented way to finish is **Merge into main** from the main checkout, which is exactly what 156 uncommitted files blocked. Everything after it (updating the main checkout, checks, cleanup) is manual.

**Proposal.** **Finish Task…** on the task row and in the palette opens one sheet that goes through the steps and shows where it is:

1. **Commit.** Stops if the task has uncommitted files, with a button to the Changes panel.
2. **Sync.** Fetches, then merges `origin/<base>` into the task, in the task's folder. Never a rebase, which would rewrite commits that may already be pushed. Conflicts are resolved there with the existing conflict tools; the operation banner's **Ask Agent** hands them to the task's own agent, which has the branch's context.
3. **Check.** Runs the `check` script (see [proposal 14](#14-project-setup-screen)) in a terminal tab in the task, for example `npm ci && npx tsc --noEmit && php artisan test`. Impulse reads pass or fail from the command block's exit status, which shell integration already reports. A failure stops here. With no check script, the step is skipped, with a link to the setup screen.
4. **Land.** Either merge directly, or push the branch for review on whatever hosts the repository (Impulse uses only git, [proposal 15](#15-git-only-no-hosting-platform-integrations)). The first Finish in a repository asks which, and the answer is saved in the project's settings (on this Mac unless you've chosen to share them), where the setup screen can change it.
   - **Direct merge:** in a throwaway worktree on `origin/<base>` (`git worktree add --detach`), merge the branch with `--no-ff`, push `HEAD:<base>`, then remove the throwaway worktree. That's the "leave the main checkout alone" route from the write-up, done for you; the main checkout is never touched, dirty or not. If the push is rejected because `origin/<base>` moved since the sync, Finish fetches, merges and pushes once more before stopping.
   - **No remote:** the merge happens in the main checkout when it's clean and on the base. Otherwise Finish refuses and says why: moving the base branch under uncommitted files would make them look like they undo the merge.
   - **Push for Review:** push the branch and show the server's reply. Git servers send `remote:` lines back on a push, and hosts such as GitLab include a link to create a merge request; Impulse shows those lines with their links clickable, without knowing what the host is. The sheet stops here; once the branch shows up as merged after a fetch ([proposal 12](#12-clean-up-finished-tasks)), Impulse offers to finish the cleanup.
5. **Update the main checkout.** If it's clean and on the base, fast-forward it. If not, the group header says so ("main checkout: 3 behind origin/main, 156 uncommitted") with a **Pull** button for later.
6. **Clean up.** Run the archive script (`docker compose down -v`), free the task's slot and delete its Compose override (proposal 1), archive the task, and delete the branch locally and on the remote.

**Decided.** A direct merge leaves a `--no-ff` merge commit, one per task, so `main`'s first-parent history reads as one entry per task. Fast-forward and squash aren't offered in v1. Landing never looks at CI, which isn't part of git (proposal 15); the check step is the local stand-in. Rebase isn't offered for sync. Letting an agent start Finish (`impulse tasks finish`, with the sheet asking you to approve the land step) comes after the sheet works.

**Where.** A new `Git/TaskFinish.swift` beside `TaskWorkflows.swift`; `check` in `ProjectConfig`; an `AppCommand` in `CommandRegistry.swift`.

### 5. Abort that works after edits

**Size S to M.** Fixes: the escape hatch closing mid-merge. Discussed 8 Oct; decisions below.

**Problem.** The operation banner's **Abort** runs `git merge --abort` (`GitOperations.perform`, `GitOperations.swift:661`), which refuses once a conflicted file has been edited: "Entry … not uptodate. Cannot merge." The only ways out are then finishing the merge or `git reset --hard`. And today's confirmation warns "Changes made during the operation will be lost", because nothing keeps them.

**Proposal.** **Abort** for a merge, cherry-pick or revert becomes one step that always works and can always be undone:

1. Take a safety snapshot of the whole working tree, so nothing is lost.
2. Run `git merge --abort` (or the cherry-pick/revert equivalent).
3. If git refuses, fall back on its own, without asking again: `git reset --hard HEAD` (during an unfinished merge, cherry-pick or revert, `HEAD` is still the commit from before it), then put back the uncommitted work from before the operation (below).
4. Toast with **Undo**, which restores the snapshot. The confirmation before Abort says the current state is kept for Undo instead of "will be lost".

**Putting back the work from before the merge.** It has to be exactly that work, not edits made while resolving. Edits in files the merge didn't bring in look the same as earlier work: in this session, the 44 `component={Link}` fixes were in the overhaul's own files, which the merge never touched, and a loose rule would have kept them as if they predated the merge.

- **Merges Impulse starts** already take a snapshot first (`GitActions.merge`); restore from it.
- **Merges started in a terminal** (an agent's): when the repository watcher sees `MERGE_HEAD` (or `CHERRY_PICK_HEAD`, `REVERT_HEAD`) appear, Impulse records the dirty files and their contents at that moment, before anyone has edited anything, under `.git/impulse/`. Restore those.
- **Neither record** (Impulse wasn't running): restore the files that were dirty and that the incoming side didn't change (`git diff --name-only $(git merge-base HEAD MERGE_HEAD) MERGE_HEAD`; git refuses to start a merge when uncommitted changes touch one of those). The toast then says that edits made during the merge were kept as uncommitted changes.

`git rebase --abort` already works reliably, so rebase keeps today's behavior.

**Decided.** One step with Undo, no second prompt before falling back. The watcher records the dirty files when an operation starts in a terminal.

**Where.** The fallback beside `GitOperations.perform` in ImpulseGit, tested against the real `git` CLI (including the "not uptodate" case); the recording in the repository watcher (`RepoWatcher.swift` already watches for `MERGE_HEAD`); the banner's Abort in `GitActions`.

### 6. Warn before merging a branch that's still moving

**Size S.** Fixes: three merges of the same branch. Discussed 8 Oct; decisions below.

**Proposal.** When **Merge into …** (Manage Branches, History, `GitActions.merge`) targets a branch that's checked out in a task Impulse created, and that task is still moving (its agent is working or needs input, or it has uncommitted files), ask first:

> **interia-upgrade is still being worked on**
>
> Claude Code is working in the interia-upgrade task, and its last commit was 2 minutes ago. Merge c5aa814 anyway?

Merge by commit, not branch name, so what's merged is what the sheet showed. The last commit's age is shown but doesn't trigger the question on its own: you may just have committed by hand.

**For agents.** The three merges in this session were run by an agent in a terminal, so the sheet alone wouldn't have stopped them. A Claude Code `PreToolUse` hook on shell commands looks for `git merge`, `git rebase` or `git pull` naming a task's branch (or `origin/<that branch>`); when that task is still moving, the hook makes Claude Code ask you first, with the same reason as the sheet. Commands are split with Impulse's shell parser (`ImpulseKit/Completion/ShellParser.swift`), so `git -C ../x merge interia-upgrade` and `&&` chains are caught. It's installed with the other hooks; the reinstall from proposal 3 covers it.

**Decided.** Triggers: agent working or needing input, or uncommitted files; commit age is information only. The agent hook is on by default, with a setting to turn it off, like the hooks in proposal 3. No "moved since it was merged" indicator on the task row: Finish Task (proposal 4) prevents the situation instead.

**Where.** The check in `GitActions.merge` and the merge actions of Manage Branches and History; the task state from `tasks.json` and the agent state machine; the hook in `impulse hook claude` and the hook installer.

### 7. Clone dependency folders

**Size S.** Fixes: a task starting without `public/build` (and `vendor/` for tools on the Mac). Discussed 8 Oct; decisions below.

**Problem.** `.worktreeinclude` patterns that name a folder copy nothing (`WorktreeTasks.matchingFiles`, `WorktreeTasks.swift:63`), so dependencies and build output have to be rebuilt from scratch in every task.

**Proposal.** A `clone` list of folders in the project's settings, set by the [setup screen](#14-project-setup-screen)'s "Clone into tasks" row:

```toml
[worktrees]
clone = ["vendor", "public/build"]
```

- **How.** `clonefile(2)` clones each folder tree in one call: a copy-on-write clone that takes no time and no extra disk space until something in it changes. Tasks sit beside the repository, so they're on the same APFS volume; elsewhere it falls back to `FileManager.copyItem`, the call that copies files today (`TaskWorkflows.swift:384`). A folder missing from the main checkout is skipped. The New Task sheet's preview lists them ("Clones: vendor/, public/build/").
- **Setup still runs.** A clone holds the main checkout's dependencies, which can differ from the task's base; setup only has to fix the difference.
- **Docker.** PulseBoard's `docker-compose.yml` mounts `vendor/` and `node_modules/` as anonymous volumes, so its containers never see the host's copies: cloning those only helps tools on the Mac. The clone that would have prevented the 131 test failures is `public/build`, which is bind-mounted.
- **What the setup screen pre-ticks.** Detected folders that would help, with their sizes ("212 MB, no extra disk until changed"). Folders Compose mounts as anonymous volumes start unticked, saying why. So does `node_modules` when setup runs `npm ci`, which deletes it first; the screen suggests `npm install` instead.

**Decided.** A separate `clone` key, set by the setup screen; `.worktreeinclude` stays files only, and copying a 400 MB folder is always a deliberate choice.

**Where.** Cloning in `createTask` beside `copyFiles` (`TaskWorkflows.swift`); the key in `ProjectConfig`; detection (sizes, Compose anonymous volumes, `npm ci`) in the setup screen's `ProjectDetector`.

### 8. Suggest a setup script

**Size S. Folded into [proposal 14](#14-project-setup-screen) on 8 Oct:** the setup screen's Setup row uses this table, and the New Task sheet's suggestion becomes its link to the screen.

**Proposal.** When there's no setup script, the New Task sheet looks for lock files at the repository root and suggests one: "No setup script. This repo has `composer.lock` and `package-lock.json`." with a link to the setup screen, which fills in `composer install && npm ci` for you to edit (PulseBoard runs composer in its container, for example).

| Lock file           | Suggests                         |
| ------------------- | -------------------------------- |
| `package-lock.json` | `npm ci`                         |
| `pnpm-lock.yaml`    | `pnpm install --frozen-lockfile` |
| `yarn.lock`         | `yarn install --immutable`       |
| `bun.lock`          | `bun install --frozen-lockfile`  |
| `composer.lock`     | `composer install`               |
| `Gemfile.lock`      | `bundle install`                 |
| `uv.lock`           | `uv sync`                        |
| `poetry.lock`       | `poetry install`                 |

The table lives in ImpulseKit, with tests.

### 9. Offer setup when dependencies change

**Size S to M.** Fixes: broken code that merged cleanly going unnoticed; stale dependencies after a merge. Discussed 8 Oct; decisions below.

**Proposal.** When a workspace's `HEAD` moves (a pull, merge, rebase or checkout, from Impulse or from a terminal; the repository watcher sees both), Impulse compares the old and new commits and runs the changed files through rules from the project's settings:

```toml
[on_change]
"composer.lock" = "docker compose exec app composer install"
"package-lock.json" = "npm ci"
"Dockerfile" = "docker compose build app && docker compose up -d --renew-anon-volumes app"
```

- **Rules, not the whole setup.** Setup is written for a new task (start the stack, install, build); rerunning all of it because `composer.lock` changed is heavy and sometimes wrong. The [setup screen](#14-project-setup-screen) fills the rules in from what it detects (the lock-file table from proposal 8; for a `Dockerfile`, the write-up's lesson that anonymous volumes outlive a rebuild). When a known dependency file changes and no rule matches, the toast falls back to **Run setup**.
- **The toast.** A checkout gets the dependency step: "composer.lock changed. **Update dependencies**". A merge or pull that brings in changes also gets **Run check** (proposal 4's check script): "composer.lock and package-lock.json changed. **Update dependencies** · **Run check**". Both run in a terminal tab in that workspace. Here, that would have pointed straight at the `npm ci` and `tsc` run that found the 44 broken `component={Link}` uses.
- **The agent hears it too.** When an agent's own `git merge` or `git pull` moves `HEAD` and dependency files changed, the hook from proposal 3 adds one line to its context naming the files and the commands from the rules, so it doesn't run tests against stale dependencies.
- **Not a trigger:** `.impulse/project.toml` (a change there asks for trust again, and its rules come from the new version), and the local settings, which live in `.git` and never move with `HEAD`.

**Decided.** Per-file rules filled in by the setup screen, with **Run setup** as the fallback; **Run check** offered after merges and pulls; the agent told through the hook.

**Where.** Matching changed files to rules in ImpulseKit, with tests; `HEAD` moves from the repository watcher; the rules (`[on_change]`) in `ProjectConfig`; the agent line in `impulse hook claude`.

### 10. Branch from a fresh base

**Size S.** Fixes: tasks starting from a stale `main`. Discussed 8 Oct; decisions below. (The red CI on `main` is out of scope: CI isn't part of git, proposal 15.)

**Problem.** **From** defaults to the branch checked out in the main checkout (`TaskWorkflows.swift:173`). Here that was a local `main` that was behind `origin/main`.

**Proposal.**

- **From defaults to the remote branch.** When the main checkout's branch has an upstream, **From** is `origin/<branch>`; the plain local branch only when there's no upstream. If the local branch has commits that aren't pushed, a note says so: "main has 2 unpushed commits that aren't included; type main to include them." The `--no-track` fix above makes branching from `origin/main` safe.
- **The base is a branch, not a commit.** `.git/impulse/tasks.json` records the task's base as `main` on `origin`, so overlap (proposal 2) and Finish (proposal 4) compare against the live base, not the commit the task started from.
- **Fetching never blocks.** In a trusted repository the sheet fetches when it opens (`GitRepositoryState.fetchQuietly`) and shows "Fetching origin…"; **Create Task** works meanwhile. When the fetch finishes, **From** refreshes, unless you've edited it. In an untrusted repository, where Impulse doesn't fetch on its own, a small **Fetch** button sits beside **From**.

**Decided.** **From** is always `origin/<branch>` when there's an upstream, with the unpushed-commits note; the fetch never blocks. CI status in the sheet and on the group header was agreed, then dropped when Impulse went git only (proposal 15).

**Where.** The sheet in `TaskWorkflows.swift` (`TaskSheetModel`, `presentNewTaskSheet`); the base in `tasks.json`.

### 11. Move uncommitted changes to a new task

**Size M.** Fixes: the root cause, feature work piling up in the main checkout. Discussed 8 Oct; decisions below.

**Proposal.** Move the main checkout's uncommitted work into a new task:

1. Take a safety snapshot of the uncommitted work.
2. Create the task from the current commit (the changes were made against it).
3. Restore the snapshot into the task, the same way archive Undo does (`git restore --overlay --source=<snapshot> --worktree -- .`).
4. Clean the main checkout.
5. Show a toast with **Undo**.

- **Entry points.** **Move Changes to New Task…** in the Changes panel, for everything or only the selected files (with 156 files, some may belong to other work); the same on the main checkout's row menu; and in the New Task sheet, when the main checkout has uncommitted files, a checkbox "Move pulseboard's 156 uncommitted files into this task". The checkbox is disabled, with the reason, when **From** isn't the current commit.
- **What moves.** Modified, staged and untracked files that aren't ignored; they arrive unstaged. Ignored files such as `.env` stay put (the task gets its own `.env` from proposal 1). Cleaning the main checkout removes only the untracked files that moved, never other untracked or ignored files.
- **Agents don't move.** While an agent in the main checkout is working, the move is refused until it's idle: its files would vanish mid-turn. Afterwards the old agent is still in the main checkout and can't follow its files; the sheet's **Start** launches a fresh one in the task, and you close the old one.
- **No standing nudge.** Proposal 2 already speaks up at the moment that matters, when the main checkout's changes start overlapping a task; its popover offers **Move to New Task…** when the main checkout is one side of the overlap. A dirty main checkout that collides with nothing isn't flagged.

**Decided.** The three entry points, partial moves included; refused while an agent there is working, with a fresh agent started in the task; no standing nudge or "integration only" mode, the overlap popover offers the move instead.

**Where.** The flow beside `createTask` in `TaskWorkflows.swift`, using `SafetySnapshots` for the snapshot and restore; the entry points in the Changes panel, the workspace row menu, the New Task sheet and the overlap popover.

### 12. Clean up finished tasks

**Size S.** Fixes: finished tasks piling up. Discussed 8 Oct; decisions below.

**Proposal.**

- **Only Impulse's tasks.** As decided in proposal 1, worktrees Impulse didn't create are left alone. The five `fix/*` worktrees from this session (nested in `pulseboard/.worktrees/`) are those; they're removed with git, and the parallel-agents docs (proposal 13) show how. No **Move Beside Repository**: Impulse never nests worktrees itself.
- **What counts as merged,** using git only:
  - the task's branch is contained in its base, the same `git branch --merged` check Manage Branches uses (`GitOperations.swift:289`): Finish's `--no-ff` merges, and merge requests merged with a merge commit;
  - or merging the branch into the base would change nothing: `git merge-tree --write-tree <base> <branch>` gives back the base's own tree. That catches squash merges, which many GitLab projects use and which never look merged to the first check. It can miss a branch whose lines the base has since changed again.
  - A remote branch deleted after merging (`[gone]` after a fetch with `--prune`) is shown as a hint, not counted as merged.
- **A "merged" badge.** When a fetch shows a task's branch is merged, its row in the sidebar gets a small "merged" badge, and **Archive Task…** moves to the top of its menu. No notification.
- **Archive Merged Tasks…** on the repo group's context menu lists the merged tasks from `.git/impulse/tasks.json`, open or not, with their uncommitted file counts, and archives the ones you tick. Each runs its archive script (stopping its Docker stack) and frees its slot. Deleting their branches, locally and on the remote, is an option.
- **One Undo for the batch.** "Archived 5 tasks" with an **Undo** that brings all of them back.

**Decided.** Only Impulse's tasks, no nested-worktree move; "merged" means contained in the base or absorbed by it (squash merges), checked with git alone; the "merged" badge; one Undo for a batch.

**Where.** The list and batch archive beside `archiveTask` in `TaskWorkflows.swift`; the badge in `WorkspacesSection.swift`; the merged and absorbed checks in ImpulseGit, tested against the real `git` CLI with a squash merge among the cases.

### 13. Docs: running agents in parallel

**Size S.** Discussed 8 Oct; decisions below.

**Proposal.** A "Running agents in parallel" section in [Tasks](../tasks.md), written now, with the practices that need no new feature:

- Keep the main checkout for integrating, and do feature work in tasks.
- Split sessions by area, not just by branch: an upgrade that touches every page runs alone, or lands first.
- Commit small and often in every workspace.
- One browser per agent: the Chrome DevTools MCP shares one profile unless it's started with `--isolated`.
- After merging an upgrade, reinstall dependencies and type-check before anything else; a clean merge can still break code.
- Removing worktrees Impulse didn't create: `git worktree list`, `git worktree remove <path>`, `git worktree prune`.

The lessons are rewritten in general terms with the docs' `trailhead` example project, not PulseBoard. The short "Run agents in parallel" section in [Agents](../agents.md) keeps pointing to Tasks.

**Decided.** The practices section ships in phase 1. Every other proposal documents itself in the commit that ships it, as the repository's `CLAUDE.md` already requires: per-task environments and the setup screen, Finish Task, `impulse tasks` and the hooks, and so on.

### 14. Project setup screen

**Size M to L.** Raised and discussed 8 Oct; decisions below. Takes in proposal 8.

**Problem.** Proposals 1 and 7 add keys to `project.toml`, and getting them right means knowing the project's ports, ignored files, dependency folders and scripts. Writing the file by hand is the step most people won't take, especially for a quick task. And committing Impulse settings to a repository isn't always wanted: teammates may not use Impulse.

**Proposal.** A **Project Setup** tab that looks at the repository, proposes values for everything a task needs, and saves your choices on this Mac only, or in the project for everyone who clones it.

```
Project Setup — pulseboard                       Save to: [This Mac ▾]  [Save]
───────────────────────────────────────────────────────────────────────────────
⚠ docker-compose.yml sets container_name on app, mysql, redis   [Show] [Handle in tasks]
⚠ docker-compose.yml publishes "8000:8000" as a fixed port      [Show] [Handle in tasks]

Copy into tasks    ☑ .env   ☐ .env.testing   ☐ storage/oauth-private.key
Clone into tasks   ☑ vendor  212 MB   ☑ public/build  3 MB   ☐ node_modules  480 MB
Ports  (+100/task) APP_PORT 8000 · VITE_PORT 5173 · FORWARD_DB_PORT 3306   [+]
Per-task .env      DB_DATABASE = pulseboard_{task_}                         [+]
Setup    [docker compose up -d && docker compose exec app composer install && npm ci]
Check    [npx tsc --noEmit && npm run test:run]
Archive  [docker compose down -v]

Open tasks: interia-upgrade (slot 1)  [Re-apply .env values]
                                                    [Try in a New Task]
```

**Where the settings live.** **Save to** chooses; **This Mac** is the default.

- **This Mac: `.git/impulse/project.toml`.** For quick tasks and for repositories whose other contributors don't use Impulse. Inside the repository's git folder, so:
  - It can't be committed or pushed. A file in the working tree would be one `git add -A` away, and agents run that all the time.
  - Every task sees it at once, because all worktrees share the git folder. That also removes today's gotcha that an uncommitted `project.toml` never reaches new tasks.
  - It can't arrive with a clone, so a repository can't plant commands in it. Saving from the screen trusts exactly what it saved; if anything else edits the file (an agent, say), Impulse asks before running commands from it, as it does for `project.toml`.
  - The screen owns the whole file, so writing it back has no comments or layout to preserve (TOMLKit drops comments when it writes). Editing it by hand still works; the screen says so before overwriting a hand-edited file.
- **The project: `.impulse/project.toml`, committed.** For repositories where everyone should get the same task setup (your own projects, Impulse itself). The screen rewrites only the sections it manages, so comments and anything else in the file stay as written. It's read from each task's own checkout, as today, so it reaches new tasks once committed. Saving trusts the content the screen wrote; for everyone else it asks as it does today.

Where both files set a key, the local one wins, so a shared setup can be adjusted for one Mac.

**Compose without editing the Compose file.** Fixing `container_name` and fixed ports in `docker-compose.yml` would leave a change to commit, or sitting in the main checkout. Instead, **Handle in tasks** makes Impulse generate, for each new task, a Compose override in `.git/impulse/tasks/<task>/compose.override.yml` that renames the containers (`pulseboard-app` becomes `pulseboard-interia-upgrade-app`) and replaces the published ports with the task's (`ports: !override ["8100:8000"]`, Compose 2.24 or later). The task's `.env` gets `COMPOSE_FILE=docker-compose.yml:<that override>`, which Compose reads, so `docker compose up` in the task folder uses it from any terminal. Nothing tracked changes, and archiving deletes the override. **Show** opens the line, for anyone who'd rather fix the project itself.

What it detects:

| Section          | Detected from                                                                                                         |
| ---------------- | --------------------------------------------------------------------------------------------------------------------- |
| Files to copy    | Ignored files that exist in the main checkout (`.env`, `.env.local`, `config/*.local.json`)                           |
| Folders to clone | Ignored dependency and build folders that exist (`vendor`, `node_modules`, `public/build`), with sizes                |
| Ports            | `ports:` in `docker-compose.yml` (`${APP_PORT:-8000}:8000`), and `_PORT` keys in `.env`                               |
| Per-task values  | `.env` keys that usually have to differ (`DB_DATABASE`, `APP_URL`)                                                    |
| Setup            | Lock files ([proposal 8's table](#8-suggest-a-setup-script)), plus `docker compose up -d` when there's a Compose file |
| Archive          | `docker compose down -v` when there's a Compose file                                                                  |
| Check            | `package.json` and `composer.json` scripts such as `typecheck`, `test` and `lint`                                     |
| Actions          | The same scripts, and Makefile targets                                                                                |

It warns about what blocks parallel tasks: `container_name:` in the Compose file, and host ports written as plain numbers (`"8000:8000"`). Both get **Handle in tasks**, above.

Also on the screen:

- **Try in a New Task** makes a throwaway task with these settings, runs setup and shows the result, so a wrong setup script shows up before a real task depends on it.
- **Re-apply .env values** for open tasks, since proposal 1 writes them only when a task is created.
- **Actions.** An Actions section lists the actions from both files; committed ones are labelled with their file, and new ones are saved to the **Save to** location. **Edit Project Actions** opens this section instead of creating a committed file. **Open File** shows the raw file.
- **The database.** With a Compose stack per task, each task's database starts empty. A Database row offers whichever of these fits the project:
  - **Clone the data folder.** When Compose mounts a project folder at the database's data path (PulseBoard keeps its data in `docker/data`), the folder is cloned like proposal 7's folders: instant, and no extra disk until written. A running database's files can't be copied safely, so Impulse stops the main checkout's database service first (`docker compose stop mysql`, a second or two) and starts it again after; if the main stack isn't running, no stop is needed. The clone is also a safety net: a task can try an irreversible upgrade (MySQL 8.0 to 8.4) on its own copy.
  - **Dump and load,** for data in a Docker named volume or on a server: a `mysqldump` (or `pg_dump`) from the main checkout's stack piped into the task's, written into the setup script.
  - **Empty:** migrations and seed data (`php artisan migrate --seed`, or the framework's equivalent), written into the setup script.

**Decided.** A tab like Settings (`SettingsSurface`), not a sheet: there's a lot on it, and you'll want the Compose file or a terminal open beside it. It opens from the palette (**Project Setup…**), from a link in the New Task sheet when the project has no task settings, and from a workspace row's context menu; never on its own when a repository opens. Settings are saved on this Mac by default (`.git/impulse/project.toml`), or in the committed `.impulse/project.toml` when you choose **The project**; the local file wins where both set a key. (First decided as local only, then both, 8 Oct.) Actions are managed on the screen in either location. The Compose file is handled per task by an override, not edited. The database is cloned with its data folder when it has one (stopping the main checkout's database for the copy), otherwise dumped and loaded or started empty through the setup script. Proposal 8 folds in here.

**Where.** Detection in ImpulseKit (a `ProjectDetector` over a folder: ignored files, Compose services, ports and data mounts, lock files, package scripts), tested against small fixture projects. Reading the local file beside the committed one in `ProjectConfig`. The tab in `ImpulseApp/Settings/` next to `SettingsSurface`. The data-folder clone in `createTask`, with proposal 7's cloning.

### 15. Git only: no hosting-platform integrations

**Size M.** Decided 8 Oct: a rule for the whole app, not only tasks.

**The rule.** Impulse talks to git and nothing else: the `git` CLI and libgit2, no hosting platform's CLI or API (`gh`, `glab`, REST). Any host works the same, including your self-hosted GitLab, because Impulse never needs to know which one it is. (Agent integrations such as the Claude Code and Codex hooks aren't affected; supporting agents is what Impulse is for.)

**Removed.** Everything that only works through GitHub's `gh`:

| Feature                                                                              | Code                                                                                                       |
| ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| The titlebar's pull request chip, its checks, and the "checks finished" notification | `PullRequestMonitor.swift`, `ChromeBarView.swift`, `Notifications.swift`, `GitRepositoryState.pullRequest` |
| **Open or Create Pull Request**, **Create Draft Pull Request**                       | `MainWindowController+Workspaces.swift`, `CommandRegistry.swift`                                           |
| **Check Out Pull Request as Task…** and the palette's `pr:` mode                     | `TaskWorkflows.swift`, `MainWindowController+Palette.swift`, `PaletteModel.swift`                          |
| Importing a pull request's review threads into Review                                | `ReviewSurface.swift`                                                                                      |
| The pull request parsing and its tests                                               | `ImpulseKit/Git/PullRequest.swift`, `PullRequestTests.swift`                                               |

Also removed: the per-host web links (**Open Branch in Browser**, History's **Open Commit on …** and its tag equivalent), which encode each host's URL layout (`/commit/…` on GitHub, `/-/commit/…` on GitLab) in `RemoteWebURL` (`ImpulseKit/Git/GitRefs.swift`).

**Kept or added, git only:**

- **Open Repository in Browser** stays, as a plain rewrite of the remote into `https://host/owner/repo` with no knowledge of the host. **Copy Remote URL** is added.
- **New Task from Branch…** replaces checking out a pull request: an existing local or remote branch opened as a task (a colleague's branch you're reviewing, say), with the same copies, setup and per-task environment as New Task. A branch on someone's fork needs its remote added first (`git remote add`).
- **The server's reply after a push.** Git servers send `remote:` lines on a push, and GitLab's include "To create a merge request for X, visit: https://…". Impulse shows those lines after any push, with links clickable. It's git's own channel, so it works on every host. Finish Task's **Push for Review** (proposal 4) relies on it.
- **Merged without asking a host** (proposal 12): contained in the base, or absorbed by it (`git merge-tree`), which catches squash merges.

**Changes to the plan.** Proposal 4 lands by direct merge or **Push for Review** and never looks at CI; proposal 10 drops CI status; proposal 12 detects merges with git alone.

**Also to do.** The docs (9 pages mention pull requests: `git.md`, `tasks.md`, `review.md`, `command-palette.md`, `keyboard-shortcuts.md`, `project-config.md`, `getting-started.md` and the docs' own `README.md`), the repository `README.md` and the website's homepage; a "Removed" section in the next release's changelog; the `gitPullRequest` icon if nothing else uses it (`scripts/vendor-lucide.sh`); and checking that a saved custom shortcut for a removed command (`pull_request`, `create_draft_pr`, `checkout_pr`) is ignored, not an error.

## Order

Decided 8 Oct. Two pieces come first, because most proposals build on them:

- **The task registry:** `.git/impulse/tasks.json`, listing the worktrees Impulse created, each with its base branch (`main` on `origin`) and slot. Read by 1, 2, 3, 6, 10 and 12.
- **Local settings:** `.git/impulse/project.toml`, read beside the committed `.impulse/project.toml`, the local file winning key by key. Used by 1, 4, 7, 9 and 14.

| Phase | #   | Proposal                                                         | Size   | Needs                                  |
| ----- | --- | ---------------------------------------------------------------- | ------ | -------------------------------------- |
| 1     | —   | Task registry and local settings                                 | S to M |                                        |
| 1     | 5   | Abort that works after edits                                     | S to M |                                        |
| 1     | 6   | Warn before merging a moving branch (the sheet)                  | S      | registry                               |
| 1     | 10  | Branch from a fresh base                                         | S      | registry                               |
| 1     | 13  | Docs: running agents in parallel                                 | S      |                                        |
| 1     | 15  | Git only: remove host integrations                               | M      |                                        |
| 2     | 1   | Per-task environment                                             | M      | registry, local settings               |
| 2     | 7   | Clone dependency folders                                         | S      | local settings                         |
| 2     | 14  | Project setup screen (takes in 8)                                | M to L | 1, 7                                   |
| 2     | 9   | Offer setup when dependencies change                             | S to M | 14 for the rules                       |
| 3     | 2   | Show where workspaces overlap                                    | M      | registry                               |
| 3     | 3   | `impulse tasks` and the agent hooks, with 6's guard and 9's line | S to M | 2                                      |
| 4     | 12  | Clean up finished tasks                                          | S      | registry                               |
| 4     | 4   | Finish Task                                                      | L      | 10, 12's merged check, 15's push reply |
| 4     | 11  | Move uncommitted changes to a new task                           | M      |                                        |

1. **Foundations and quick wins.** The registry and local settings, then 5, 6's sheet, 10, 13 and 15, which are independent and mostly small. 15 goes early: it removes shipped features, and Finish's **Push for Review** relies on its push reply. (The `--no-track` fix is already done.)
2. **Task environment.** 1, 7, then the setup screen (14), then 9, whose rules the screen fills in. This makes a task runnable, which is what blocked this session most.
3. **Awareness.** 2, then 3. Every hook change ships here together (3's session summary and edit heads-up, 6's merge guard, 9's dependency line), so hooks are reinstalled once; the hook sheet says when installed hooks are out of date.
4. **Finishing.** 12, then 4, which uses its merged check; 11 alongside.

## Decisions that cut across everything

- **The main checkout.** Impulse doesn't nag about a dirty main checkout. It speaks up when the main checkout's changes overlap a task (2), and offers to move them into a task (11).
- **Only Impulse's tasks.** Worktrees Impulse didn't create are left alone everywhere: no slots, overlap, warnings or cleanup.
- **Where state lives.** `.git/impulse/`: the task registry, local settings, per-task Compose overrides, the records Abort uses. Shared by every worktree, never committed, and it survives restarts and windows.
- **Settings.** On this Mac by default, or committed for everyone (14); the local file wins.
- **Telling agents.** Through the hooks Impulse installs, each on by default with a setting to turn it off: the session-start summary, the edit-time heads-up, the merge guard and the dependency line. Nothing is written into committed instruction files.
- **Git only.** No hosting platform's CLI or API (15); everything works on any host.
- **Undo first.** Anything destructive takes a safety snapshot and offers Undo (Abort in 5, moving changes in 11, batch archiving in 12), as Impulse's git actions already do.
- **Tests.** The new logic (slots and placeholders, `.env` editing, lock-file and Compose detection, overlap sets, change rules, the abort restore set, merged-or-absorbed) belongs in ImpulseKit or ImpulseGit with unit tests. New git behavior is tested against the real `git` CLI, as the other ImpulseGit APIs are.
