# MCP server: plan

Plan, proposed 19 Oct 2026 and extended the same day beyond tasks; built the same day, every phase, with the open questions answered by their recommendations except where the status below says otherwise. Not user documentation: this folder isn't published to the website (`scripts/build-site.py` only builds `docs/*.md`). For how it works now, see [Impulse's MCP server](../agents.md#impulses-mcp-server), [Tasks agents start](../tasks.md#tasks-agents-start) and the [command-line tool](../cli.md#impulse-tasks-show-new-finish-and-archive).

## Status

Built on 19 Oct 2026, phases 0 to 4, in one change. Where the build differs from the plan below:

- **Requests stay flat strings.** `ControlRequest` didn't get an `input` field: every tool's arguments fit its string `arguments` (the brief included). `ControlResponse` did get `data`.
- **The brief is read from its file** (open question 2). Claude Code and Codex start with `claude -- "$(cat …/brief.md)"` (fish: `(cat … | string collect)`; the `--` keeps a brief that starts with a dash from reading as an option), so the brief is neither typed into the shell nor kept in its history; other agents get it typed in once they're up, and submitted.
- **No mark on the sidebar row.** The robot icon there already means an agent waiting for you, so an agent-made task says "started for Claude Code in trailhead" in the row's hover instead.
- **Finish stops before landing, not before running.** `finish_task` runs commit, sync and check by itself and stops at Land with a new waiting state ("Claude Code asked to finish fix-elevation. Click Continue to land it."). The first Finish in a repository still needs you to choose how work lands, so the agent's waits for that. For a task that's already merged, it stops before updating the main checkout and archiving instead.
- **`archive_task` was built** (open question 6), behind Archive Task…'s own questions.
- **The cap is its own setting,** `agent_task_limit` (4, 0 for no limit), beside `agent_task_creation` and `agent_tool_terminal_output`.
- **The sheet is Agent Integration,** with Hooks and MCP Server halves; **Install Impulse's MCP Server…** in the palette opens the second. For agents other than Claude Code and Codex it shows the entry to copy.
- **The `SessionStart` summary always names `list_tasks`,** registered or not: Impulse can't tell cheaply whether the agent has the server.
- **Tools declare no `outputSchema`.** Answers carry their JSON as `structuredContent` when the app sends it, but some are text only ("Done.", or anything from an older app), and with a schema Claude Code rejects a result without structured content.
- **Both protocol eras, checked against Claude Code 2.1.303 and Codex 0.145.** Claude Code probes with `server/discover` and then sends requests naming 2026-07-28 in `_meta`, which get that revision's shape (`resultType`, `ttlMs` and `cacheScope`, the server's name in `_meta`; an unknown revision gets `-32022`). Codex uses `initialize` (2025-06-18), and starts servers with a cleared environment, so its table forwards `IMPULSE_SOCKET`, `IMPULSE_PANE_TOKEN` and `IMPULSE_CLI` with `env_vars`.
- **Reading other terminals needs the pane's own process tree,** like creating tasks: `blocks` (`list_command_blocks`, `get_block_output`) doesn't work with the token alone, and none of the workspace tools answer a request that names no pane.
- **Progress** goes out every 15 seconds while a tool waits, when the client asked for it.

Impulse already tells agents about each other's work. The `impulse tasks` command lists a repository's workspaces, and Claude Code hooks push notes into a session (a summary at start, shared files, a question before merging a moving branch). Two things are missing:

- **Agents can read, but not act.** An agent that has planned three independent changes can't start three tasks. It can only ask you to press ⌥⌘N three times and paste a brief into each.
- **What an agent can read depends on hooks and on parsing text.** Codex gets no hook replies. Other agents only get `impulse tasks` if they think to run it, and then read its output as prose or JSON from a shell command.

The [Model Context Protocol](https://modelcontextprotocol.io) (MCP) is how every agent Impulse supports takes tools from outside: Claude Code, Codex, Gemini CLI, Copilot CLI, Cursor Agent, Goose, opencode and Amp all load MCP servers. An Impulse MCP server would give each of them the same typed tools: list the tasks, look at one, wait for one, and **create one**, with the agent's brief and an agent already started in it.

Tasks come first, but they aren't the only thing Impulse knows that an agent's shell can't easily find out. Impulse already has three ways to hand an agent something by typing it into its prompt: **Send Comments to …** in [Review](../review.md#sending-comments-to-an-agent), **Send to agent** on a [command block](../terminal.md#sending-a-block-to-an-agent) and **Fix with Agent** in [Problems](../editor.md#problems). Each becomes a tool the agent can call when it needs it, as structured data, while you can still send. [Beyond tasks](#beyond-tasks) covers those and a few smaller tools.

## Goals

1. Any MCP-capable agent running in an Impulse terminal can list the repository's tasks and read one task's state as structured data.
2. An agent can ask Impulse to create a task, optionally starting an agent there with a brief, through the same steps New Task… takes (copies, slot and values, trust, setup script).
3. You stay in charge: by default nothing is created until you approve it in Impulse, and nothing an agent asks for gets past workspace trust or the project file's trust.
4. One behavior, two doors: every MCP tool is a control-socket command the `impulse` CLI can also send, so agents without MCP (and your scripts) get the same thing.
5. An agent can read what Impulse knows about the work in front of it: your review comments, the language servers' problems, the output of commands in the panes beside it, the files you haven't saved, and the project's trusted scripts. It can also hand its work back to you (open a file or a review) and report its state when it has no hooks.

Not goals: Impulse calling a model (it [never does](#what-stays-the-same)), messages typed into another agent's terminal, anything that needs a hosting platform, an MCP server reachable from outside Impulse terminals, and anything listed under [Left out on purpose](#left-out-on-purpose).

## What exists to build on

| Piece                                                                                                           | What it gives the MCP server                                                                                                                                                                                                      |
| --------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`ControlServer`](../../impulse-macos/Sources/ImpulseApp/App/ControlServer.swift)                                  | A per-user Unix socket (0600, peer uid checked), one JSON request line and one response line per connection, waiting requests that notice a hang-up, and the caller's pid (`LOCAL_PEERPID`) with `ControlCaller.descends(from:)`. |
| [`ControlProtocol`](../../impulse-macos/Sources/ImpulseProtocol/ControlProtocol.swift)                             | `ControlRequest` (command, pane token, cwd, string arguments, `wait`) and `ControlResponse` (ok, message), shared by the app and the CLI. Foundation-only, so the MCP layer can live beside it and be tested headless.            |
| [`ControlCommands.swift`](../../impulse-macos/Sources/ImpulseApp/App/ControlCommands.swift)                        | The dispatch, and the rule that matters most here: commands that start processes or write files need the caller to be the pane's shell or one of its descendants, not just to hold the token.                                     |
| [`TaskAwareness.swift`](../../impulse-macos/Sources/ImpulseApp/App/TaskAwareness.swift)                            | `taskListing(cwd:)` builds the listing off the main thread and fills in agents on it; `waitForTask` already waits for a task's agent to stop and gives up when the caller hangs up.                                               |
| [`TaskListing`](../../impulse-macos/Sources/ImpulseKit/Agents/TaskListing.swift)                                   | A `Codable` model of the workspaces (branch, base, head, agent and state, uncommitted count, ahead/behind, files shared with the caller) with `json()` and `text()`. It becomes the tool's structured output almost as is.        |
| [`TaskWorkflows.swift`](../../impulse-macos/Sources/ImpulseApp/Git/TaskWorkflows.swift)                            | `presentNewTaskSheet(title:command:base:movingChanges:)` already prefills the sheet; `createTask` does the work off the main thread; `debugCreateTask` shows the sheet can be skipped when something else has decided.            |
| [`TaskRegistry`](../../impulse-macos/Sources/ImpulseKit/Git/TaskRegistry.swift)                                    | `TaskRecord` in `.git/impulse/tasks.json`: path, branch, base, slot, start commit, `fromRemote`. Where a task's provenance (made by an agent, from which workspace, with which brief) would go.                                   |
| [`AgentHookInstaller`](../../impulse-macos/Sources/ImpulseKit/Agents/AgentHookInstaller.swift) and the Hooks sheet | Already edits `~/.claude/settings.json`, `.claude/settings.local.json` and `~/.codex/config.toml` with a preview, a backup, a changed-since-preview check and Remove. Registering an MCP server is the same kind of edit.         |
| [`KnownAgents`](../../impulse-macos/Sources/ImpulseKit/Agents/KnownAgents.swift)                                   | Stable agent ids (`claude`, `codex`, `gemini`, …): the set an MCP request may name, instead of an arbitrary command line.                                                                                                         |
| [`ShellQuoting`](../../impulse-macos/Sources/ImpulseKit/Completion/ShellQuoting.swift)                             | Quoting for the new terminal's shell, so a brief passed to `claude "…"` can't break out of its argument.                                                                                                                          |
| [`ReviewComment`](../../impulse-macos/Sources/ImpulseKit/Review/ReviewComments.swift) and `ReviewCommentStore`     | Comments per repository in Impulse's data folder: path, side, line range, the anchored snippet and the text. `ReviewCommentAnchoring` already tells which are outdated.                                                           |
| [`Problems`](../../impulse-macos/Sources/ImpulseKit/Problems.swift)                                                | The diagnostics behind the Problems tab, with counts, filtering, grouping and the **Fix with Agent** prompt (`Problems.prompt`).                                                                                                  |
| `impulse_terminal_command_blocks` / `impulse_terminal_command_block` ([FFI](../../impulse-macos/CImpulseFFI))      | A terminal's command blocks listed without their output, and one block (with output) fetched by id: the shape a tool listing blocks, then reading one, needs.                                                                     |
| [`AgentCheckpoints`](../../impulse-macos/Sources/ImpulseApp/Git/AgentCheckpoints.swift)                            | Each agent turn's start and end snapshots per terminal (`turns(terminalID:)`), under `refs/impulse/checkpoints/`.                                                                                                                 |
| [`EditorTab`](../../impulse-macos/Sources/ImpulseApp/Editor/EditorTab.swift)                                       | `isModified` for each open file: which files have edits you haven't saved.                                                                                                                                                        |
| [`ProjectConfig`](../../impulse-macos/Sources/ImpulseKit/Project/ProjectConfig.swift) and project-file trust       | The check and setup scripts, palette actions, `[on_change]` rules and per-task values, and whether you've trusted the exact file they come from.                                                                                  |

## Shape of the server

### A stdio server inside the `impulse` CLI

```
agent (Claude Code, Codex, …)
  │  stdio: MCP JSON-RPC, one message per line
  ▼
impulse mcp                         ← bundled CLI, started by the agent
  │  Unix socket: one ControlRequest per tool call
  ▼
Impulse.app  ControlServer → handleControl → TaskAwareness / TaskWorkflows
```

`impulse mcp` is a new subcommand of the CLI that's already on PATH in every Impulse terminal (`$IMPULSE_CLI`). The agent starts it as a stdio MCP server; it answers `initialize` and `tools/list` itself, and turns each `tools/call` into one control request to the window, exactly as `impulse tasks` does. The app stays the only thing that decides and acts.

Why not the alternatives:

- **An HTTP MCP server in the app.** A listening TCP port is reachable by anything on the Mac (browsers included, so it needs auth and Origin checks), it can't tell which pane is calling, and it means one more server to keep alive. The Unix socket already has the peer uid and pid checks the security model relies on.
- **A separate MCP binary or a Node package.** One more thing to install, sign, version and keep in step. The CLI is already bundled, signed and versioned with the app.
- **MCP over the control socket directly.** The socket's one-line-per-connection protocol is simple and well tested; MCP's long-lived bidirectional session doesn't fit it, and the CLI is a natural adapter.

The JSON-RPC and MCP handling (framing, `initialize`, the tool catalog, mapping tool arguments to a `ControlRequest` and a `ControlResponse` back to a tool result) goes in a new `ImpulseProtocol/MCP` folder, Foundation-only and tested beside `ControlProtocolTests` in ImpulseKitTests. `ImpulseCLI/main.swift` only wires stdin and stdout to it.

### Protocol version

Today's clients speak the 2025-11-25 revision (an `initialize` handshake, then requests). The 2026-07-28 revision makes the protocol stateless: no handshake, a `server/discover` method, and state carried in tool arguments rather than in a session. The server should answer both: `initialize` for current clients, `server/discover` for newer ones. The design suits the stateless revision anyway, because nothing is kept between calls: who is calling comes from the environment and the socket, and a task is named by its folder or branch in every call. Use tools only; leave out sampling (Impulse doesn't call models), roots and logging (deprecated in 2026-07-28), and don't depend on elicitation (support differs between clients, and approval belongs in Impulse's own window; see [Approval](#approval)).

### Who is calling

A tool call is answered for the workspace and pane the agent runs in, like every `impulse` command. Two signals, both of which exist today:

1. **The environment.** The agent passes `IMPULSE_SOCKET` and `IMPULSE_PANE_TOKEN` to `impulse mcp`. Codex only forwards variables it's told to (`env_vars`), so the registration names them; for Claude Code the registration sets them with `${IMPULSE_SOCKET:-}` and `${IMPULSE_PANE_TOKEN:-}` rather than relying on inheritance.
2. **The process tree.** `impulse mcp` is a child of the agent, which is a child of the pane's shell, so `ControlCaller.descends(from:)` holds and the write tools get the same protection `split`, `tab` and `checkpoint` have: holding the token isn't enough, the caller must be the pane's shell or started from it. An agent in tmux gets the read tools only, as `impulse split` does today.

Outside an Impulse terminal (`IMPULSE_SOCKET` unset), `impulse mcp` still starts and answers, but lists no tools and says why in its `instructions`. That's the MCP counterpart of `impulse hook` exiting 0 outside Impulse: registering the server for all projects is safe, and an agent run in another terminal isn't shown tools that would only fail.

## Tools

Every tool returns `structuredContent` with an `outputSchema`, plus the same in text (`TaskListing.text()`) for clients that only read text. Annotations say which tools are read-only. Names are the agent-facing API, so they're short and stable; they appear to Claude Code as `mcp__impulse__list_tasks` and so on.

"Caller" in the table is the rule from [Who is calling](#who-is-calling): **token** tools only need the pane's token, as `open` and `tasks` do today; **tree** tools also need the caller in the pane's process tree, as `split` and `checkpoint` do.

| Tool                     | Control command         | Kind                             | Caller | Phase |
| ------------------------ | ----------------------- | -------------------------------- | ------ | ----- |
| `list_tasks`             | `tasks` (`--json`)      | read-only                        | token  | 1     |
| `get_task`               | `tasks show <task>`     | read-only                        | token  | 1     |
| `wait_for_task`          | `tasks wait <task>`     | read-only, long-running          | token  | 1     |
| `open_file`              | `open`                  | shows you something              | token  | 1     |
| `open_review`            | `review`                | shows you something              | token  | 1     |
| `set_status`             | `status`                | changes the pane's agent state   | token  | 1     |
| `notify`                 | `notify`                | flags the pane, notifies         | token  | 1     |
| `create_checkpoint`      | `checkpoint`            | writes a ref                     | tree   | 1     |
| `list_turns`             | `turns`                 | read-only                        | token  | 1     |
| `list_review_comments`   | `comments`              | read-only                        | token  | 2     |
| `resolve_review_comment` | `comments resolve <id>` | marks a comment, you accept      | tree   | 2     |
| `get_diagnostics`        | `problems`              | read-only                        | token  | 2     |
| `list_command_blocks`    | `blocks`                | read-only, behind a setting      | token  | 2     |
| `get_block_output`       | `blocks show <id>`      | read-only, behind a setting      | token  | 2     |
| `list_unsaved_files`     | `unsaved`               | read-only                        | token  | 2     |
| `get_project_config`     | `project`               | read-only, trusted settings only | token  | 2     |
| `create_task`            | `tasks new`             | creates, asks                    | tree   | 3     |
| `finish_task`            | `tasks finish <task>`   | lands, asks                      | tree   | 4     |
| `archive_task`           | `tasks archive <task>`  | removes a folder, asks           | tree   | 4     |

### `list_tasks`

No arguments. The repository's workspaces from the caller's point of view: exactly `TaskListing`, so the CLI's `--json` and the tool can't drift apart. Each workspace gains two fields from phase 3's provenance: `createdBy` (`user` or `agent`) and `brief` (its first line).

### `get_task`

`{ "task": "fix-elevation" }`, a folder name or branch, as `impulse tasks wait` takes. One workspace's entry from the listing plus what an agent needs before merging it or building on it: the files it changes (relative to its base, and uncommitted), its commits ahead of its base (id and subject, at most 50), whether it's merged into its base (`GitOperations.isMerged`, squash merges included), its slot and port values when the project gives tasks values, and its full brief. Read in the main checkout with `trusted:` as the listing does, so it works in restricted repositories without running their hooks or filters.

### `wait_for_task`

`{ "task": "fix-elevation", "timeout_seconds": 600 }`. `waitForTask` as it is, with a bound: it answers when the task's agent stops working or when the timeout passes (at most the client's tool timeout; Codex defaults to 60 seconds, Claude Code to much longer), saying which, so the agent can call again. It sends progress notifications while it waits when the client asked for them. Cancelling the call hangs up the socket, which already stops the wait.

### `create_task`

```json
{
  "title": "Fix elevation units on the trail page",
  "base": "origin/main",
  "agent": "claude",
  "brief": "Trail elevations show metres when the profile says feet. Fix …",
  "move_changes": false
}
```

| Field          | Meaning                                                                                                                                                                                                                       |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `title`        | Required. Becomes the branch and folder name the way the sheet makes them (`fix-elevation-units-on-the-trail-page`, shortened, made unique against taken branches).                                                           |
| `base`         | Optional. Defaults to what the sheet would pick (`WorktreeTasks.defaultBase`): the main checkout's upstream, with the same note about unpushed commits.                                                                       |
| `agent`        | Optional. A `KnownAgents` id found on PATH. Never a command line: an agent can't use this tool to run an arbitrary command in a new terminal. Omitted: the task opens with a plain terminal (after the setup script, if any). |
| `brief`        | Optional. Saved as `.git/impulse/tasks/<task>/brief.md` (beside the Compose override, outside the task's folder) and passed to the agent as its first prompt, quoted for the terminal's shell. Up to 16 KB.                   |
| `move_changes` | Optional, default false. Take the main checkout's uncommitted files along (`ChangeMove`), with the same blockers (an agent mid-turn in the main checkout can't have its files moved; it's usually the caller).                |

The answer is the new task's `get_task` entry, or why it wasn't created: you declined, a sheet for another request is already open, the folder exists, the base doesn't resolve, the limit on agent-made tasks is reached. The call waits while the approval sheet is open (it's a waiting request, like `impulse edit`), so the agent learns the outcome rather than guessing.

How the brief reaches the new agent: `claude '<brief>'` and `codex '<brief>'` start an interactive session with that first prompt. For agents whose CLI takes no prompt argument, the terminal starts the agent and the brief is typed into its input once the agent is detected, the way the composer already sends text to an agent. The brief also stays readable through `get_task`, so the new agent (or you) can read it again later.

The work itself is `createTask`, unchanged, called with a `TaskSheetModel` filled from the request: the same copies, cloned folders, slot, values, trust carry-over, project-file trust prompt and setup script, in the same order. An agent can't get a task that New Task… wouldn't have made.

### `finish_task` and `archive_task` (phase 4)

The open item from [Parallel tasks](parallel-tasks.md) ("an agent starting Finish, with Impulse asking you to approve the land step"). `finish_task` opens the Finish tab for the task and runs its steps; the land step always stops for your approval, and the tool answers with the step it stopped at and why, so the agent can fix a failing check and call again. `archive_task` asks the same questions Archive Task… asks and keeps its Undo. Neither ever runs without a sheet: both push, remove folders or delete branches.

## Beyond tasks

The rule for choosing these: expose what Impulse knows and the agent's shell can't easily find out, scoped to the caller's workspace, and nothing that lets one agent reach into another's terminal. Every one reads the caller's workspace (its repository, its panes, its open files) unless it says otherwise, and paths in answers are relative to the workspace's folder.

### `list_review_comments` and `resolve_review_comment` (phase 2)

Comments live in Impulse's data folder, never in the repository, so today an agent sees them only when you send them. `list_review_comments` returns the repository's comments from `ReviewCommentStore`:

```json
{
  "comments": [
    {
      "id": "6F1C…",
      "location": "src/lib/cache.ts:16-18",
      "side": "new",
      "snippet": "    if (!entry || entry.expires <= this.now()) {\n …",
      "text": "Expired entries count as misses here. Is that what the metrics should report?",
      "outdated": false,
      "resolution": null
    }
  ]
}
```

Arguments: `path` (only that file's) and `include_outdated` (default true, since an outdated comment is often one the agent already acted on). `outdated` comes from `ReviewCommentAnchoring`, as Review shows it.

`resolve_review_comment` takes `{ "id": "6F1C…", "note": "Expired entries now count separately as evictions." }` and marks the comment **addressed** with the agent's note. It never deletes: Review shows the comment with "Addressed by Claude Code" and the note, and **Accept** (deletes it) or **Reopen** (clears the mark). `ReviewComment` gains an optional `resolution` (who, when, note), so stored comments read as before. "Address my review comments" then works without sending anything, and you check each answer where you wrote the comment.

### `get_diagnostics` (phase 2)

What the Problems tab shows, from the language servers already running: `{ "path": "src/forecast.ts", "severity": "error" }`, both optional. Each problem has its path, line, column, severity, message, source and code, as `Problems.prompt` lists them for **Fix with Agent**. At most 200, errors first, with a `truncated` count; the counts (`ProblemCounts`) always cover everything.

It never starts a server or opens a document: servers run only in trusted folders, and only for documents a configured server serves. When none is running for the workspace, the answer says why ("Language servers don't run in this folder because it isn't trusted", or "No language server is configured for TypeScript") rather than an empty list that reads as "no problems". Each answer names the files whose diagnostics are current (open in the editor, or reported by a server that checks the whole project), so the agent knows an absent file is unknown, not clean.

### `list_command_blocks` and `get_block_output` (phase 2)

How an agent sees the failing dev server or test watcher in the pane beside it. `list_command_blocks` lists the terminals in the caller's workspace (id, title, folder, whether an agent runs there), each with its recent blocks from `impulse_terminal_command_blocks`: id, command, folder, exit status, start time and duration, and whether it's still running. No output, as the FFI already lists them. `get_block_output` takes `{ "block": 1042, "lines": 200 }` and returns that block's output from `impulse_terminal_command_block`, the last 200 lines by default (at most 2,000), as **Send to agent** does.

Terminal output holds whatever was printed, secrets included, so a setting, `agent_tool_terminal_output`, decides which terminals these tools see: `workspace` (the default: the caller's workspace), `window`, or `off` (the tools aren't listed). The Quick Terminal is never included, and neither is a block from a program on the alternate screen (an editor, `less`), which has no output to give. A terminal's bookmarks and block flags aren't exposed.

### `list_unsaved_files` (phase 2)

The files in the caller's workspace that are open in the editor with edits you haven't saved (`EditorTab.isModified`), each with whether the file also changed on disk since. An agent that rewrites one of them makes the editor reload a file you were in the middle of changing; with this, it can leave that file alone or ask you to save first. Paths only, never the unsaved text: what you haven't saved is yours until you save it.

### `get_project_config` (phase 2)

How this repository is checked and set up, from `.impulse/project.toml` and the local `.git/impulse/project.toml` as Impulse merges them: the check and setup scripts, palette actions (name and command), `[on_change]` rules, the env file's name (never its contents), and for a task its slot and own values. **Only from a file you've trusted**, by the same rule the dependency hook follows: an untrusted file's commands mustn't reach the agent as Impulse's advice. Before you've trusted it, the answer says so and lists only what Impulse detected on its own (`ProjectDetector`: the package manager, lock files, a Compose file).

### `open_file` and `open_review` (phase 1)

`impulse open` and `impulse review`, for handing work back to you: `{ "path": "src/forecast.ts", "line": 14 }` opens the file at that line in the caller's window, and `{ "scope": "last-turn" }` (or `uncommitted`, `staged`, `unstaged`) opens Review, so an agent can end with "I'm done, here's the review". Unlike the CLI, neither brings Impulse to the front when you're in another app: the tab opens and the pane is flagged for attention, as when an agent finishes. The tool doesn't create files (`impulse edit` of a new file is a tree command); a path that doesn't exist is an error.

### `set_status` and `notify` (phase 1)

`impulse status` and `impulse notify`, for agents without Impulse's hooks (Gemini CLI, opencode, Goose and the rest): `set_status` takes `working`, `waiting`, `done` or `idle` with a message, and `notify` a title and message, which flag the pane and send a desktop notification. The status lasts while `impulse mcp` runs, which is the agent's session. For a pane whose agent sends hook events, hooks win: they're exact, and `set_status` there changes nothing (the answer says so), so an agent can't hold its tab on "working" against what its hooks report. The tool descriptions say to use them only when the agent has no Impulse integration.

### `create_checkpoint` and `list_turns` (phase 1)

`create_checkpoint` is `impulse checkpoint`: a snapshot of the repository under `refs/impulse/checkpoints/manual/`, with a message, before a risky step. It answers with the ref. `list_turns` lists the caller's pane's agent turns from `AgentCheckpoints`: when each started and ended, and its start and end refs, so the agent can `git diff <start> <end>` to see what an earlier turn changed. Restoring a checkpoint or turn stays in Impulse's UI, behind its usual confirmation.

### Left out on purpose

- **Sending messages to another task's agent, or typing into any other pane.** Typing into another agent's terminal lets one agent prompt another with whatever it read, a direct path for prompt injection across tasks. The brief at creation, which you approve, is the only text one agent hands another. Revisit only with a design that shows you the message first.
- **Running commands in a task.** Agents already have a shell; `impulse split` and `impulse tab` cover the pane's own workspace.
- **Changing workspace trust, settings or keybindings.** Trust is the line every other rule leans on; an agent asking to move it is the case it exists for.
- **Command history.** `CommandHistoryDatabase` is a private record of everything you've typed, in every project.
- **Destructive git.** Reverting, discarding, restoring a checkpoint, deleting a branch or stash: never without a sheet in Impulse, so not as tools. Agents that need git have git, and Impulse's safety snapshots still cover what Impulse itself does.
- **Language-server edits.** Rename and code actions reach across files, and Impulse asks before applying the ones that create, move or delete files or leave the server's root. Agents edit files themselves; read-only lookups (definition, references, hover) could come later if diagnostics prove useful.
- **The unsaved text of open files, and the editor's selection.** See [`list_unsaved_files`](#list_unsaved_files-phase-2) and [Open questions](#open-questions).
- **Resources and subscriptions.** A `impulse://tasks` resource that updates as tasks change would suit clients that subscribe, but few agents act on resource updates mid-turn today, and the hooks already push the notes that matter. Reconsider once tools are in use.

## Approval

Creating a task starts processes (a setup script, an agent that spends your money and edits files), writes into the repository's `.git` and can move your uncommitted work. So the default is **ask every time**:

1. `create_task` opens the New Task sheet in the caller's window, prefilled from the request, with a line naming who asked: "Claude Code in trailhead (main checkout) wants to start this task." The brief is shown, scrollable, under **Start**.
2. You can edit anything (title, base, agent, brief) and click **Create Task**, or **Cancel**. The tool answers with what was created, including your edits, or "The user declined."
3. While one request's sheet is open, a second gets "Another task request is waiting for the user." rather than a queue of sheets.

A setting, `agent_task_creation`, offers three levels, in Settings ▸ Terminal ▸ Agents beside the hook settings:

| Value              | Behavior                                                                                                                                                 |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ask` (default)    | As above.                                                                                                                                                |
| `trusted_projects` | In a [trusted](../getting-started.md) repository whose project file you've trusted, create without the sheet and show a toast with **Undo**; otherwise ask. |
| `off`              | `create_task` isn't listed; the read tools stay.                                                                                                         |

Whatever the setting, these always hold:

- **Trust isn't bypassed.** An untrusted repository's task folder starts restricted, and the project file's setup script and per-task values need the project file to be trusted, as for New Task…. A request never trusts anything.
- **A cap on agent-made tasks.** At most four open tasks made by agents per repository (a setting), so a looping agent can't fill the disk with worktrees and the machine with agents.
- **No chains without you.** A request from an agent working in a task that an agent made always asks, whatever the setting.
- **Moving changes always asks.** It changes your main checkout.
- **Undo.** The toast's Undo archives the new task (its folder snapshotted, the branch deleted when nothing was committed on it), as Undo of a move does today.

## Registering the server

The Agent Hooks sheet becomes **Agent Integration…**: the hooks as today, plus an **MCP server** row per agent with the same preview, backup, changed-since-preview check and Remove. The entry runs the bundled CLI through `$IMPULSE_CLI` with a fallback to the app's own path, so it also starts (with no tools) outside Impulse:

```json
{
  "mcpServers": {
    "impulse": {
      "command": "sh",
      "args": [
        "-c",
        "exec \"${IMPULSE_CLI:-/Applications/Impulse.app/Contents/Resources/bin/impulse}\" mcp"
      ],
      "env": {
        "IMPULSE_SOCKET": "${IMPULSE_SOCKET:-}",
        "IMPULSE_PANE_TOKEN": "${IMPULSE_PANE_TOKEN:-}",
        "IMPULSE_CLI": "${IMPULSE_CLI:-}"
      }
    }
  }
}
```

The fallback path is the running app's bundle at install time; the sheet shows "Out of date" when the app has moved since, as it does for hooks from an older version.

- **Claude Code, all projects:** user scope. Claude Code keeps user-scope servers in `~/.claude.json`, a large file it rewrites itself while running. Impulse runs `claude mcp add --scope user impulse -- sh -c '…'` (and `claude mcp remove`) when `claude` is on PATH, rather than editing that file; without it, the sheet shows the command to run.
- **Claude Code, this project only:** local scope, through `claude mcp add --scope local`, never `.mcp.json`: a committed file would give every clone of the repository a server that only works in Impulse, and Claude Code asks each user to approve project servers anyway.
- **Codex:** an `[mcp_servers.impulse]` table in `~/.codex/config.toml`, with `env_vars = ["IMPULSE_SOCKET", "IMPULSE_PANE_TOKEN", "IMPULSE_CLI"]`, written by `AgentHookInstaller` beside its `notify` line.
- **Other agents:** the sheet shows the snippet for their config to copy (Gemini CLI, opencode, Goose and the rest each have their own file), since Impulse can't test edits to every one.

Hooks and the server complement each other: hooks push what an agent should know at the moment it matters, the server lets it ask and act. The `SessionStart` summary gains one line when the server is registered: "Use the impulse tools to look at a task or start a new one."

## Control protocol changes

`ControlRequest.arguments` is `[String: String]` and `ControlResponse` carries one `message` string. Both get one optional field, so old CLIs and old apps keep working:

- `ControlRequest.input: Data?`: the tool's JSON arguments, as given, for commands whose input isn't flat strings (`tasks new`'s brief can hold anything).
- `ControlResponse.data: Data?`: structured JSON for the tool result; `message` keeps the text.

New control commands, also reachable from the CLI so agents without MCP and your scripts get them too:

```
impulse tasks show <task> [--json]
impulse turns [--json]
impulse comments [<path>] [--json]
impulse comments resolve <id> [note…]
impulse problems [<path>] [--errors] [--json]
impulse blocks [--json]
impulse blocks show <id> [--lines <n>]
impulse unsaved [--json]
impulse project [--json]
impulse tasks new <title> [--from <base>] [--agent <id>] [--brief-file <path> | --brief -] [--move-changes]
impulse tasks finish <task>       (phase 4)
impulse tasks archive <task>      (phase 4)
```

`open`, `review`, `status`, `notify` and `checkpoint` exist already. `comments resolve`, `tasks new`, `finish` and `archive` join `split`, `tab` and `checkpoint` in needing the caller in the pane's process tree. `blocks` follows `agent_tool_terminal_output` from the CLI too, so the setting can't be sidestepped by running the command instead of calling the tool.

## Provenance

`TaskRecord` gains optional fields, so existing `tasks.json` files read as before:

- `createdBy`: `"user"` or `"agent"`.
- `requestedBy`: for an agent-made task, the agent's id and the workspace it was in (`claude`, `trailhead`).
- `brief`: whether `.git/impulse/tasks/<task>/brief.md` exists.

The sidebar row shows a small mark on agent-made tasks with the requester in its tooltip; `list_tasks` and `impulse tasks` show both fields; archiving keeps the brief with the snapshot so Undo brings it back.

## Order

Sizes as in the other plans: **S** is a day or two, **M** about a week.

| Phase | What                                                                                                                                                                                                                                                                                                       | Size   | Needs |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ----- |
| 0     | `ControlRequest.input` / `ControlResponse.data`; `impulse tasks show`, `impulse turns`                                                                                                                                                                                                                     | S      |       |
| 1     | `impulse mcp` (JSON-RPC over stdio, `initialize` and `server/discover`, no tools outside Impulse), `list_tasks`, `get_task`, `wait_for_task`; registration in the sheet. The thin wrappers over commands that exist: `open_file`, `open_review`, `set_status`, `notify`, `create_checkpoint`, `list_turns` | M      | 0     |
| 2     | Workspace context: `list_review_comments` and `resolve_review_comment` (with Review's **Accept** and **Reopen**), `get_diagnostics`, `list_command_blocks` and `get_block_output` with `agent_tool_terminal_output`, `list_unsaved_files`, `get_project_config`, and their CLI commands                    | M      | 1     |
| 3     | `impulse tasks new` and `create_task`: the prefilled sheet, briefs, provenance, `agent_task_creation`, the cap and Undo                                                                                                                                                                                    | M      | 1     |
| 4     | `finish_task` and `archive_task` behind their sheets                                                                                                                                                                                                                                                       | S to M | 3     |

Phase 1 is useful by itself: Codex and other agents without Impulse's hook replies get the same picture Claude Code gets, as typed data, and agents without hooks get tab states and notifications. Phases 2 and 3 only share phase 1's server and can go in either order; 2 comes first here because its tools are read-only (apart from marking a comment) and its data already exists.

## Testing

- **ImpulseKitTests, beside `ControlProtocolTests`:** JSON-RPC framing (partial lines, batches, notifications without ids, malformed input answered with JSON-RPC errors, never a crash), `initialize` and `server/discover` answers, the tool catalog and its schemas, argument validation (unknown agent ids, briefs over the limit, a missing title), and the mapping to and from `ControlRequest` / `ControlResponse`.
- **ImpulseKitTests, task registry:** `TaskRecord` decoding with and without the new fields; `TaskListing` with provenance.
- **ImpulseKitTests, review comments:** `ReviewComment` decoding with and without `resolution`; resolving, reopening and accepting.
- **App:** `create_task` against a mock window: ask, decline, a second request while one waits, the cap, the process-tree check, and that a request never trusts a folder or a project file. For the phase 2 tools: `get_diagnostics` in an untrusted folder (an explanation, not an empty list), `get_project_config` before and after trusting the file (no commands before), `list_command_blocks` under each `agent_tool_terminal_output` value and never the Quick Terminal, `list_unsaved_files` returning paths only, `set_status` ignored for a pane with hooks, and `open_file` not activating the app.
- **Smoke:** a scenario in `scripts/smoke/scenarios.py` that pipes MCP messages into `impulse mcp` from a pane of the demo repository (with a debug action standing in for clicking **Create Task**) and checks the new worktree, its `tasks.json` entry, its brief and the first terminal's command. A second one runs a failing `npm test` in one pane, then from another reads the block's output, review comments the scenario adds through Review, and resolves one, checking the dump.
- **Against real clients:** Claude Code and Codex each list the tools, call `list_tasks`, and create a task with a brief, before each phase ships.

## What stays the same

- **Impulse never calls a model.** The server only answers agents; it doesn't sample, summarize or plan.
- **Git only.** Every tool works from git, the task registry and what Impulse already holds (comments, diagnostics, terminals), on any host.
- **Trust.** No tool starts a language server, runs a project command or reads an untrusted project file's commands; the tools only report what trusted parts of Impulse already have.
- **The app is the authority.** The CLI adapter holds no state and makes no decisions; each call is checked in the app as `impulse` commands are today.
- **Hooks stay.** Nothing they do moves to the server.

## Open questions

Each has a recommendation; none is decided.

1. **Default approval.** `ask` (recommended) or `trusted_projects`. Asking costs one click per task and makes the first experience predictable; the setting is there for people who run many agents.
2. **Brief delivery.** As the first prompt on the command line (recommended for Claude Code and Codex), or always typed in once the agent is up. The command line is reliable but leaves the brief in the shell's history; typing it in avoids that but depends on detecting the agent's prompt.
3. **Where the brief lives.** `.git/impulse/tasks/<task>/brief.md` (recommended: outside the work tree, never committed by accident), or a file in the task folder that the agent can see and edit.
4. **The cap.** Four open agent-made tasks per repository (recommended), per Mac, or none.
5. **Claude Code registration.** Through `claude mcp add` (recommended), or editing `~/.claude.json` with the installer's usual safeguards.
6. **Phase 4 at all.** `finish_task` was asked for in Parallel tasks; `archive_task` might be enough as a Finish step. Recommended: build `finish_task`, leave `archive_task` out unless asked.
7. **Terminal output default.** `agent_tool_terminal_output` as `workspace` (recommended: the panes an agent's own work runs in, and the reason the tool exists), `off` until you turn it on, or `window`.
8. **Resolving comments.** Marked addressed for you to accept (recommended), or deleted outright by the agent. Deleting is simpler but loses the check that each comment was really dealt with.
9. **Fresh diagnostics.** Report only what servers already have (recommended for phase 2), or let `get_diagnostics` open a file that isn't open (a `didOpen` with no tab) so its server checks it. The second gives better answers in trusted folders but starts work in a server on the agent's say-so.
10. **The editor's selection.** A `get_selection` tool (the file and lines you have selected, as Claude Code's IDE integrations offer) would let "fix this" mean what you're looking at. Recommended: later, after phase 2 shows how the read tools are used, and with a setting, since it reveals where you are as you work.

## Also to do

- **User docs.** A new section in [Agents](../agents.md) (the server, the tools, approval, registering it), `impulse tasks show` and `new` in [Command-line tool](../cli.md#impulse-tasks), agent-made tasks and their mark in [Tasks](../tasks.md#tasks-in-the-sidebar), the settings in [Settings and themes](../settings-and-themes.md), addressed comments with **Accept** and **Reopen** in [Review › Comments](../review.md#comments), the tools' view of Problems in [Editor](../editor.md#problems) and of command blocks in [Terminal](../terminal.md#command-blocks). When phase 1 ships, this page becomes the record of what was decided, as `docs/plans/parallel-tasks.md` is.
- **CLAUDE.md and AGENTS.md.** `impulse mcp` in the CLI's command list in the architecture tree; the MCP adapter in ImpulseProtocol's description; the Agents key pattern (`create_task` through `createTask`, the approval rule, provenance in `TaskRecord`).
- **CHANGELOG.md.** Each phase under the release it ships in.
