# Project configuration

A repository can tell Impulse about itself in `.impulse/project.toml`: commands to run from the command palette (project actions), scripts that set up and clean up task worktrees, and extra files to copy into new tasks. A second file, `.worktreeinclude`, lists the untracked files that new tasks need. Commands from `project.toml` only run after you trust the file.

Both files live at the root of a git repository and are meant to be committed, so everyone who works on the project (and every task worktree) gets them. To keep the settings to yourself instead, put them in `.git/impulse/project.toml`, which is never committed; see [Settings for this Mac only](#settings-for-this-mac-only).

## Project Setup

The easiest way to write these settings is **Project Setup**, a tab that looks through the repository, proposes what its tasks need, and saves your choices. Open it with:

- **File ▸ Project Setup…**, or **Project Setup…** in the command palette (⇧⌘P),
- **Project Setup…** in a workspace row's context menu in the sidebar,
- **Set Up This Project for Tasks…** in the New Task sheet, shown when the project has no task settings yet (with the lock files it found, such as "This repository has composer.lock and package-lock.json"),
- **Edit Project Actions**, which opens it at its Actions.

It shows the settings already saved, and fills what's missing from what it finds:

| Section              | What it proposes                                                                                                                                                                                                                                                                                                   |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Docker Compose       | Warnings about what keeps two checkouts' stacks from running at once: `container_name`, and host ports written as plain numbers (`"8000:8000"`). Tick **Handle in tasks** to deal with them per task (below), or **Show** to open the Compose file.                                                                |
| Copy into tasks      | Ignored files in the main checkout; `.env` files come ticked. See [Copying files into tasks](#copying-files-into-tasks).                                                                                                                                                                                           |
| Clone into tasks     | Ignored dependency and build folders (`vendor`, `node_modules`, `public/build`, …), with their sizes. A folder Compose hides from the containers with an anonymous volume, and `node_modules` when setup runs `npm ci`, start unticked, saying why. See [Cloning folders into tasks](#cloning-folders-into-tasks). |
| Database             | When the Compose file runs a database: clone its data folder, dump and load it, or start empty with migrations and seed data. See [A database for each task](#a-database-for-each-task).                                                                                                                           |
| Ports                | Ports read from variables in the Compose file (`${APP_PORT:-8000}`) and `_PORT` values in `.env`. See [Ports and values for each task](#ports-and-values-for-each-task).                                                                                                                                           |
| Values for each task | A database name, when tasks share one database server, and an app URL on a task's port.                                                                                                                                                                                                                            |
| Scripts              | **Setup** from the lock files (`npm ci`, `composer install`, …) and `docker compose up -d`; **Check** from type-check, lint and test scripts; **Archive** `docker compose down -v`.                                                                                                                                |
| Actions              | The repository's `package.json` and `composer.json` scripts and Makefile targets, unticked until you tick them.                                                                                                                                                                                                    |
| Finishing tasks      | How **Finish Task…** lands a task: ask the first time (the default), **Merge and push**, or **Push for review**. See [Finish a task](tasks.md#finish-a-task).                                                                                                                                                      |
| When files change    | A rule for each lock file it finds (`composer.lock` → `composer install`), and for a `Dockerfile` beside a Compose file. See [When files change](#when-files-change).                                                                                                                                              |

Everything is editable. **Save to** chooses where the settings go: **This Mac** (`.git/impulse/project.toml`, never committed; see [Settings for this Mac only](#settings-for-this-mac-only)) or **The project** (`.impulse/project.toml`, to commit for everyone). In a committed file, Saving rewrites only the sections the tab manages and keeps the rest of the file as written; it says so first if comments in those sections would be dropped. Saving trusts exactly what it wrote, so the settings work in the next task without a prompt. **Open File** opens the file in the editor.

**Try in a New Task** saves, then opens New Task, so you can see the settings work in a throwaway task (archive it afterwards). **Re-apply Values to N Tasks** saves, then writes the current ports and values into the env files of tasks that already exist, since a task's values are otherwise only written when it's created.

### Docker Compose in tasks

A Compose file with `container_name` or fixed host ports can only run one copy of its stack. Fixing the file means a change to commit; **Handle in tasks** (`compose_override = true` under `[worktrees]`) avoids that. Each new task gets an override file in `.git/impulse/tasks/<task>/compose.override.yml` that renames its containers (`pulseboard-app` becomes `pulseboard-app-fix-elevation`) and moves its fixed host ports by its slot (`"8000:8000"` becomes `"8100:8000"` in slot 1). The task's `.env` gets `COMPOSE_FILE` naming the project's Compose file, its own override file if it has one, and the task's override, so `docker compose` in the task folder uses them from any terminal. Archiving the task deletes the override. It needs Docker Compose 2.24 or later.

### Writing the file by hand

The settings are plain TOML, and you can write them yourself: create `.impulse/project.toml` (or `.git/impulse/project.toml`) and open it in the editor, or use **Open File** in Project Setup. The [Reference](#reference) lists every key.

## An example

The trailhead project's `.impulse/project.toml`:

```toml
[[actions]]
name = "dev"
command = "npm run dev"
open = "right"

[[actions]]
name = "test"
command = "npm test"

[[actions]]
name = "lint"
command = "npm run lint"

[scripts]
setup = "npm ci"

[worktrees]
copy = ["config/*.local.json"]
```

With it, ⌃⌘R lists `dev`, `test` and `lint`; `dev` opens the dev server in a split to the right. Every new task runs `npm ci` in its first terminal, and gets the local config files copied in alongside the `.env` from `.worktreeinclude`.

## Reference

Every key is optional. Keys Impulse doesn't know are ignored.

### `[[actions]]`

Each `[[actions]]` table is one project action.

| Key       | Type   | Required | Meaning                                                                                                                                                                               |
| --------- | ------ | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`    | string | Yes      | What the action is called in the palette. Matched when you type after `a:`.                                                                                                           |
| `command` | string | Yes      | The shell command to run.                                                                                                                                                             |
| `cwd`     | string | No       | The folder to run in, relative to the repository root, for example `"web"`. Default: the repository root.                                                                             |
| `open`    | string | No       | Where the action's terminal opens: `"tab"` (a new tab, the default), `"right"` (a split to the right of the current tab) or `"down"` (a split below it). Any other value opens a tab. |

Actions with an empty `name` or `command` are left out.

### `[scripts]`

| Key       | Type   | Meaning                                                                                                                                                                                                                                                                                                                 |
| --------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `setup`   | string | Runs in a new task's first terminal, before the agent you chose. See [Setup script](#setup-script).                                                                                                                                                                                                                     |
| `archive` | string | Runs in a task's folder before **Archive Task…** removes it. See [Archive script](#archive-script).                                                                                                                                                                                                                     |
| `check`   | string | Checks that a task's work is ready (type checks, tests), for example `npm run typecheck && npm test`. **Finish Task…** runs it before landing a task (see [Finish a task](tasks.md#finish-a-task)), and Impulse offers to run it after a pull or merge brings in changes (see [When files change](#when-files-change)). |

An empty string means no script.

### `[worktrees]`

| Key                | Type             | Meaning                                                                                                                                                                                  |
| ------------------ | ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `copy`             | array of strings | Untracked files to copy into new tasks, in addition to `.worktreeinclude` (or its defaults). Same patterns as [`.worktreeinclude`](#worktreeinclude).                                    |
| `env_file`         | string           | The dotenv file each new task's own values are written into, relative to the repository root. Default `.env`. See [Ports and values for each task](#ports-and-values-for-each-task).     |
| `port_offset`      | integer          | What each task adds to every port in `[worktrees.ports]`, times its slot. Default `100`.                                                                                                 |
| `clone`            | array of strings | Folders cloned into new tasks from the main checkout, such as `vendor` or `public/build`. See [Cloning folders into tasks](#cloning-folders-into-tasks).                                 |
| `compose_override` | boolean          | Give each new task a Compose override that renames its containers and moves its fixed ports. Default `false`. See [Docker Compose in tasks](#docker-compose-in-tasks).                   |
| `overlap_ignore`   | array of strings | Files that don't count when two workspaces change the same files: names, paths or patterns. See [When workspaces change the same files](tasks.md#when-workspaces-change-the-same-files). |

### `[worktrees.ports]`

The main checkout's ports, one per line, named as they are in the env file: `APP_PORT = 8000`. Each task gets each port plus its slot × `port_offset`. See [Ports and values for each task](#ports-and-values-for-each-task).

### `[worktrees.env]`

Other values that have to differ in each task, one per line: `DB_DATABASE = "trailhead_{task_}"`. They can use the placeholders `{task}`, `{task_}`, `{slot}` and the port names. See [Ports and values for each task](#ports-and-values-for-each-task).

### `[on_change]`

What to run when a file changes in a pull, merge or checkout, one rule per line: `"composer.lock" = "composer install"`. A name matches that file in any folder; a path (`"web/package-lock.json"`) or a pattern (`"*.gemspec"`) works too. Quote names with dots in them. See [When files change](#when-files-change).

### `[worktrees.database]`

| Key       | Type   | Meaning                                                                                                                                                    |
| --------- | ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `clone`   | string | A database's data folder, cloned into each new task so it starts with the main checkout's data. See [A database for each task](#a-database-for-each-task). |
| `service` | string | The Compose service that writes that folder. It's stopped in the main checkout while the folder is cloned, then started again.                             |

### `[finish]`

| Key    | Type   | Meaning                                                                                                                                                                                                                                             |
| ------ | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `land` | string | How **Finish Task…** lands a task: `"merge"` (merge into the base with a merge commit and push it) or `"review"` (push the branch for review). Unset, the first Finish asks and saves the answer here. See [Finish a task](tasks.md#finish-a-task). |

### When the file has a mistake

If the file isn't valid TOML, or a key has the wrong type (for example `copy = ".env"` instead of `copy = [".env"]`), Impulse ignores the project's settings until it's fixed, including the other file when there are two. When you open the project actions list, a warning toast names the file (`.impulse/project.toml:` or `.git/impulse/project.toml:`) followed by the error. Fix the file and open the list again.

## Settings for this Mac only

When a project's settings shouldn't be committed (the other people on the project don't use Impulse, or the setup only suits your Mac), put them in `.git/impulse/project.toml` instead. It takes the same keys as `.impulse/project.toml`.

- It's inside the repository's `.git` folder, so git never sees it: it can't be committed or pushed, and a clone doesn't bring one with it.
- Every task uses it straight away, because all of a repository's worktrees share that folder. You don't commit it first, as you would `.impulse/project.toml` (see [Setup script](#setup-script)).
- Its commands need trusting like the committed file's (see [Trusting the project file](#trusting-the-project-file)), and separately: trusting one file doesn't trust the other.

Create it from a terminal in the main checkout (in a task, `.git` is a file; `git rev-parse --git-common-dir` prints the folder to use), then open it in Impulse:

```sh
mkdir -p .git/impulse && touch .git/impulse/project.toml
impulse open .git/impulse/project.toml
```

The trailhead project keeps its Docker setup there:

```toml
[scripts]
setup = "docker compose up -d && npm ci"
archive = "docker compose down -v"

[[actions]]
name = "test"
command = "docker compose exec web npm test"
```

### When both files exist

Impulse reads both, and `.git/impulse/project.toml` wins key by key:

| In `.git/impulse/project.toml`         | Result                                                                          |
| -------------------------------------- | ------------------------------------------------------------------------------- |
| A key the committed file also sets     | The local value is used.                                                        |
| A key it doesn't set                   | The committed file's value is used.                                             |
| An empty script (`setup = ""`)         | The committed file's script is turned off.                                      |
| `[worktrees] copy`                     | Replaces the committed file's list.                                             |
| `[worktrees.ports]`, `[worktrees.env]` | Merged name by name: a local value replaces the committed one of the same name. |
| An action with the same `name`         | Replaces the committed file's action; the others from both files stay.          |

## Trusting the project file

A `project.toml` is part of the repository, so anyone who can commit to it can put commands in it. Impulse never runs those commands until you've trusted that exact file. The same goes for `.git/impulse/project.toml`, which is trusted on its own: when both files have commands you haven't trusted, one prompt lists both files' commands. A file that sets [values for each task](#ports-and-values-for-each-task) asks too, naming them ("It sets API_PORT, DATABASE_NAME in each new task's env file.").

The first time something would run a command from the file (an action, a task's setup script or a task's archive script), Impulse asks:

![The trust prompt "Trust .impulse/project.toml in trailhead?" listing npm run dev, npm test, npm run lint and npm ci, with Cancel and Trust and Run buttons](images/project-config-trust-prompt.png)

> **Trust .impulse/project.toml in trailhead?**
>
> It can run these commands on your Mac: _(each command in the file, one per line, up to eight, then "…and N more")_
>
> You'll be asked again if the file changes.

- **Trust and Run** remembers your answer and runs the command.
- **Cancel** runs nothing from the file. Impulse asks again next time.

What you trust is the file's exact content (a SHA-256 hash of it) in that repository. Any change to the file, even a comment, makes Impulse ask again, so you see new commands before they run. Task worktrees share their repository's trust: a task whose copy of the file is identical to the one you trusted doesn't ask again. The exception is the setup script of a pull request checked out as a task, which asks every time (see [Setup script](#setup-script)).

Some things don't need trust: listing the actions in the palette, and copying the files named in `[worktrees] copy`. A file with no actions and no scripts never asks.

Trusting `project.toml` is separate from [workspace trust](getting-started.md), which controls language servers, formatters on save and background fetch: trusting one doesn't trust the other. Taking trust back covers both, though:

- **Restrict This Folder** (command palette) also forgets the trusted `project.toml` of the folder's repository and of every repository inside the folder.
- **Forget Trusted Folders** also forgets every trusted `project.toml`.

After either, Impulse asks again before running a command from the file. Editing the file asks again too.

## Project actions

Project actions are the commands you run over and over in a project (dev server, tests, linting), one keystroke away instead of retyped in a terminal.

![The command palette in a: mode listing the dev, test and lint actions with their commands, dev marked "split", and "Edit project actions…" at the bottom](images/project-config-actions-palette.png)

1. Press ⌃⌘R (**View ▸ Run Project Action…**, or **Run Project Action…** in the palette). The palette opens with the `a:` prefix; typing `a:` in the palette does the same.
2. Type part of the action's name to filter the list. Each row shows the action's name and its command; actions that open in a split are marked "split".
3. Press Return to run the selected action.

The action runs in a new terminal: a new tab, or a split to the right of or below the current tab, depending on `open`. The terminal starts in the repository root (or `cwd`) and the command is typed into its shell, so the output appears as a command block and the terminal stays open when the command finishes. Run it again from the terminal, or re-run the block.

Actions come from the active workspace's repository. In a [task](tasks.md) workspace, they come from the task's own copy of the file and run in the task's folder, so `npm run dev` in a task serves the task's code.

The last row of the list is **Add project actions…** when there are no actions, or **Edit project actions…** when there are. Both open [Project Setup](#project-setup) at its Actions.

## Setup script

`[scripts] setup` runs when you create a task with **New Task…**, to get a fresh worktree ready to use: installing dependencies, building, generating local config.

- It runs in the task's first terminal, in the task folder, where you can watch it. Its output is a command block like any other.
- If you chose an agent under **Start**, the agent runs after it, as one command joined with `&&` (for example `npm ci && claude`). If setup fails, the agent doesn't start; fix the problem in that terminal and start the agent yourself.
- Impulse reads the script from the new task's own copy of `.impulse/project.toml`, which is whatever is committed on the base branch. Commit changes to the file before relying on them in new tasks, or put the script in [`.git/impulse/project.toml`](#settings-for-this-mac-only), which every new task sees as soon as you save it.
- If you haven't trusted the file, Impulse asks first. If you cancel, the script doesn't run, and the task's terminal still opens (and starts the agent, if you chose one).
- **New Task from Branch…** on a remote branch reads it from the branch's own copy of the file and always asks before running it, even if you trusted the file: the branch can change what the script runs (a `package.json` script, for example) without changing the file. The answer isn't remembered, and **Don't Run** opens the task without it.

## Archive script

`[scripts] archive` runs when you choose **Archive Task…**, before the task's folder is removed: for example to stop the task's containers (`docker compose down`) or drop a database it created.

- It runs in the task folder with `/bin/sh -c` and your login shell's `PATH`, without a terminal. Its output isn't shown.
- If it exits with an error, or runs for more than two minutes (it's stopped then), a toast says "The archive script failed; archiving anyway." and the task is archived regardless.
- It needs the file to be trusted, like any command from it. If you cancel the trust prompt, the task is archived without running it.

## Ports and values for each task

Two checkouts of a project that each run a dev server, or a Docker stack, collide when both want the same ports or database. The project settings can name what has to differ, and each new task gets its own values, written into its copy of `.env` when it's created:

```toml
[worktrees]
port_offset = 100          # task n adds n × 100 to every port (the default)

[worktrees.ports]          # the main checkout's ports
WEB_PORT = 5173
API_PORT = 8080

[worktrees.env]            # may use {task}, {task_}, {slot} and the port names
DATABASE_NAME = "trailhead_{task_}"
API_URL = "http://localhost:{API_PORT}"
```

With that, a task named `fix-elevation` in slot 1 gets `WEB_PORT=5273`, `API_PORT=8180`, `DATABASE_NAME=trailhead_fix_elevation` and `API_URL=http://localhost:8180`.

- **Slots.** Each task Impulse creates gets a slot, a small number that's unique among the repository's tasks and stays with the task until it's archived; the main checkout is slot 0 and keeps its ports. A new task takes the lowest slot whose ports are all free on your Mac. The New Task sheet's **Values** line shows what the task will get.
- **The file.** The task's `.env` starts as a copy of the main checkout's (with your secrets and other settings), even when `.worktreeinclude` doesn't list it, and only these keys are changed: in place where the file already sets them, otherwise added at the end under a `# Impulse task fix-elevation` comment. `env_file` names a different file. If the main checkout has none, the task's file holds only these values. Because the values are in the file, everything that reads it gets them: your app, `docker compose` from any terminal, Vite.
- **Written once.** The values are written when the task is created. After that the file is yours to edit; changing the settings later doesn't change existing tasks.
- **Placeholders.** `{task}` is the task's folder name (`fix-elevation`, fine for host names and Compose project names), `{task_}` the same with dashes as underscores (`fix_elevation`, for database names), `{slot}` the slot number, and `{API_PORT}` (any name from `[worktrees.ports]`) that port for the task.
- **Trust.** The values are only written from settings you've trusted, like scripts (see [Trusting the project file](#trusting-the-project-file)).

Every terminal in a task also gets `IMPULSE_TASK` (the task's folder name), `IMPULSE_TASK_SLOT` and `IMPULSE_REPO_ROOT` (the main checkout) in its environment, and so do setup and archive scripts and project actions. Terminals in the main checkout get `IMPULSE_REPO_ROOT` only.

For a Docker project, read the ports in `docker-compose.yml` from the file (`"${API_PORT:-8080}:8080"`) and don't set `container_name`, so each task's stack is separate; Compose names a stack after its folder. Then `setup = "docker compose up -d"` starts the task's own stack and `archive = "docker compose down -v"` removes it.

## Cloning folders into tasks

A new task is a fresh checkout, so dependency folders and build output (`vendor`, `node_modules`, `public/build`) aren't there, and rebuilding them takes time. `[worktrees] clone` lists folders to clone from the main checkout instead:

```toml
[worktrees]
clone = ["vendor", "public/build"]
```

Cloning uses APFS clones: a folder of any size comes over at once and takes no extra disk space until something in it changes, and changing it doesn't change the main checkout's. If the task already has the folder (it holds a tracked file, say), it gets the entries it lacks. A folder the main checkout doesn't have is skipped. The New Task sheet's **Clones** line lists them. Cloning needs no trust: it only copies your own files.

A clone is the main checkout's dependencies, which can differ from what the task's base needs, so keep a setup script to bring them in line; it only has to fix the difference. `npm ci` deletes `node_modules` before installing, so with `node_modules` cloned use `npm install` instead.

### A database for each task

If the project's database keeps its data in a folder of the project (a Compose bind mount such as `./docker/data/mysql:/var/lib/mysql`), each task can start with a copy of the main checkout's data:

```toml
[worktrees.database]
clone = "docker/data/mysql"
service = "mysql"
```

A running database's files can't be copied safely, so Impulse stops that Compose service in the main checkout (`docker compose stop mysql`), clones the folder, and starts it again: a second or two. If the service isn't running, nothing is stopped. Because these run `docker compose`, they need the settings to be trusted, and the trust prompt lists them. The clone is also a safety net: a task can try a change that rewrites the data, such as a database upgrade, on its own copy.

When the data lives in a Docker volume instead, have the setup script load it (a dump from the main checkout's database, or migrations and seed data).

## When files change

When a checkout's commit changes under it and a dependency file comes along (a lock file, a `Dockerfile`), the installed dependencies are stale, and tests fail for the wrong reason. Impulse notices when a workspace moves to another commit, from Impulse or from a terminal (an agent's `git pull`, say), and offers to bring it up to date:

```toml
[on_change]
"composer.lock" = "composer install"
"package-lock.json" = "npm ci"
"Dockerfile" = "docker compose build && docker compose up -d --renew-anon-volumes"
```

- **The toast.** It names the files that changed ("composer.lock and package-lock.json changed") and the commands, with **Update Dependencies**, which runs them in a new terminal tab of that workspace. When a known dependency file changed but no rule names it, the button is **Run Setup** and runs the setup script.
- **After a pull or merge** that brought in changes, the toast also has **Run Check**, which runs the [`check` script](#scripts): a merge without conflicts can still break code that relied on something the other side changed.
- **Not for your own commits.** Committing, amending or cherry-picking moves the commit too, but you already have what you committed, so nothing is offered. A checkout or reset gets **Update Dependencies** without **Run Check**.
- **Trust.** The commands come from the project settings, so they run once you've trusted them, like scripts.

[Project Setup](#project-setup) fills in a rule for each lock file it finds. For a `Dockerfile`, its rule also renews the containers' anonymous volumes (`--renew-anon-volumes`): Compose keeps them when it recreates a container, so the old dependencies would otherwise stay.

## Copying files into tasks

A new task worktree contains only the files git tracks. Untracked files your project needs, such as `.env`, are copied in from the repository when the task is created. Two sources decide which:

- `.worktreeinclude` at the repository root, or, when that file doesn't exist, the defaults `.env`, `.env.local` and `.claude/settings.local.json` (Claude Code's project-only settings, so new tasks keep the project's [agent hooks](agents.md#agent-hooks));
- plus `[worktrees] copy` in `.impulse/project.toml`.

The New Task sheet's **Copies** line lists exactly which files will be copied. See [Tasks](tasks.md#which-files-are-copied).

### `.worktreeinclude`

A plain text file at the repository root with one pattern per line. The trailhead project's:

```
# Local settings a fresh checkout doesn't have
.env
```

The rules:

- Each line is one pattern, a path relative to the repository root. A leading `/` is allowed and means the same thing.
- Blank lines are ignored. Lines starting with `#` are comments. A `#` later in a line is part of the pattern, so don't put comments after a pattern.
- Spaces at the start and end of a line are trimmed.
- The last part of a pattern may use shell wildcards: `*` (any characters, including a leading dot), `?` (one character) and `[…]` (one of a set). Wildcards in folder names, and `**`, don't work.
- Only files are copied. A pattern that names a folder copies nothing, and folders aren't searched recursively.
- Patterns that start with `~` or contain `..` are ignored, so nothing outside the repository can be copied.
- A file is only copied when it exists in the repository and doesn't already exist in the new task.
- When `.worktreeinclude` exists, the defaults (`.env`, `.env.local`, `.claude/settings.local.json`) no longer apply: list them if you want them. An empty `.worktreeinclude` turns copying off (apart from `[worktrees] copy`).

| Pattern               | Copies                                               |
| --------------------- | ---------------------------------------------------- |
| `.env`                | `.env`                                               |
| `.env*`               | `.env`, `.env.local`, `.env.test`, … at the root     |
| `config/*.local.json` | `config/dev.local.json`, `config/test.local.json`, … |
| `/certs/dev.pem`      | `certs/dev.pem`                                      |
| `node_modules`        | Nothing (it's a folder; use a setup script)          |
| `../shared/.env`      | Nothing (outside the repository)                     |

### `[worktrees] copy`

`copy` in `.impulse/project.toml` takes the same patterns as `.worktreeinclude`, as a TOML array, with one difference: write them without a leading `/`. They're added to the `.worktreeinclude` patterns, or to the defaults when there's no `.worktreeinclude`.

```toml
[worktrees]
copy = [".env", "config/*.local.json"]
```

Use whichever file suits your project: `.worktreeinclude` keeps the list in a file of its own, `copy` keeps everything about tasks in `project.toml`. **New Task…** and **New Task from Branch…** both use the two together.

## Related

- [Tasks](tasks.md): New Task…, setup, copies and archiving
- [Command palette](command-palette.md): the `a:` prefix and the other modes
- [Getting started](getting-started.md): workspace trust
- [Keyboard shortcuts](keyboard-shortcuts.md)
