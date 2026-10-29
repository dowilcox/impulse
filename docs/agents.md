# Agents

Impulse recognizes coding agents such as Claude Code and Codex when they run in its terminals, shows what each one is doing, and tells you when one needs you. It records a checkpoint around every agent turn so you can review exactly what the agent changed, restore files from before it, or send review comments back.

Impulse never calls an AI model itself. The agents are the command-line programs you already use; Impulse hosts them in its terminals and watches what they do.

## Supported agents

| Agent        | Recognized command |
| ------------ | ------------------ |
| Claude Code  | `claude`           |
| Codex        | `codex`            |
| Gemini CLI   | `gemini`           |
| Aider        | `aider`            |
| opencode     | `opencode`         |
| Amp          | `amp`              |
| Copilot CLI  | `copilot`          |
| Cursor Agent | `cursor-agent`     |
| Goose        | `goose`            |
| Qwen Code    | `qwen`             |
| Crush        | `crush`            |

Run the agent the way you normally do, in any Impulse terminal. Nothing needs to be configured for Impulse to notice it. [Hooks](#agent-hooks) make the status exact for Claude Code, Codex and opencode.

## How Impulse recognizes an agent

When a command starts in a terminal, Impulse looks at the terminal's foreground process shortly after it starts, then every two seconds while it runs. A process counts as an agent when:

- its executable or its first argument is one of the commands above (`claude`, `codex`, …), with or without a `.js`, `.mjs`, `.cjs`, `.py` or `.exe` extension (opencode's npm package installs it as `opencode.exe`, even on macOS);
- it's an interpreter (`node`, `bun`, `deno`, `python`, `ruby`, `uv`, `uvx`, `npx`, `pnpm`, `bunx`, or a shell such as `sh`, `bash`, `zsh`) running a script with one of those names, or a script from the agent's package (for example `node …/@anthropic-ai/claude-code/cli.js`);
- or it's one level down from the foreground process, so wrappers such as `caffeinate claude` work.

This relies on shell integration telling Impulse that a command is running (bash, zsh and fish; see [Terminal](terminal.md)). An agent that reports in through [hooks](#agent-hooks) or the `impulse status` command is tracked even without it.

Impulse stops tracking the agent when its command ends or another program takes over the terminal.

## Agent states

| State                | Glyph                         | Meaning                                                                           |
| -------------------- | ----------------------------- | --------------------------------------------------------------------------------- |
| **Working**          | A spinning ring               | The agent is taking a turn.                                                       |
| **Needs your input** | A bot with an orange dot      | The agent stopped mid-turn to ask you something: a permission prompt, a question. |
| **Finished**         | A bot with a green check mark | The agent finished a turn you haven't looked at yet.                              |
| **Idle**             | A dim bot                     | The agent is running and nothing is asked of you (or you've seen its result).     |
| **Exited**           | A dim bot                     | The agent's command ended. It leaves the agent list.                              |

The dot and the check mark use your theme's attention and success colors. They also differ in shape, so you can tell **Needs your input** from **Finished** without relying on color. A screen reader reads "needs attention" with the tab's title while the agent wants you.

### How the state is worked out

Without hooks, Impulse infers the state from what a terminal can see:

- Pressing Return in the agent (sending a prompt, answering a question) means **Working**.
- Output that keeps coming for about a second and a half (a spinner, streaming text) means **Working**. Output right after you type is treated as the echo of your typing.
- A spinner at the start of the terminal's title (braille dots, or Claude Code's ◐ and ◑) means **Working**, and 5 seconds after it stops means **Finished**, however much output follows.
- A progress report (OSC 9;4) means **Working** while it's shown, and **Finished** when it's cleared.
- A desktop notification from the agent (OSC 9, 777 or 99) that mentions permission, approval, allowing, confirming, "needs your input" or a question means **Needs your input**; any other notification means **Finished**.
- A bell while the agent is working means **Finished**.
- No output for 5 seconds while working means **Finished** (2 minutes for agents that report progress, since they can pause for longer).

With [hooks](#agent-hooks) installed, the agent tells Impulse directly and the guesses above are switched off (Return, permission-style notifications and a title spinner still count, as described below the table):

| Hook event                                                                        | State                                                                                                                                                                                                                                                                                                                       |
| --------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SessionStart` (Claude Code)                                                      | **Idle**                                                                                                                                                                                                                                                                                                                    |
| `UserPromptSubmit` (Claude Code)                                                  | **Working**                                                                                                                                                                                                                                                                                                                 |
| `PostToolUse` (Claude Code, shell commands and edits)                             | **Working** when it's the tool a permission request was waiting on, so a prompt answered with a number key shows it working again. Tools that finish at other times (in parallel subagents, or background tasks after the turn) change nothing.                                                                             |
| `Notification` (Claude Code)                                                      | **Needs your input** when it asks for permission or input; otherwise **Finished** if it was working. Claude Code's reminder that it's waiting, a minute after a turn, changes nothing.                                                                                                                                      |
| `Stop` (Claude Code)                                                              | **Finished**, unless subagents are still running (next row)                                                                                                                                                                                                                                                                 |
| `Stop` (Claude Code), subagents still running                                     | **Working**: the turn ended with subagents running in the background, or finished there but not yet reported to the agent, which takes another turn for each. The toolbelt says "Waiting on 2 subagents". Claude Code's reminder that it's waiting for input doesn't end it; 5 minutes without output or a hook event does. |
| `StopFailure` (Claude Code)                                                       | **Finished**: an API error (a rate limit, an overloaded server) ended the turn. The notification says what went wrong, such as "Claude Code stopped: API Error: Rate limit reached".                                                                                                                                        |
| Turn complete (Codex)                                                             | **Finished**                                                                                                                                                                                                                                                                                                                |
| `start` (opencode, when a terminal loads the plugin)                              | **Idle**                                                                                                                                                                                                                                                                                                                    |
| `session.execution.started` (opencode, a prompt)                                  | **Working**                                                                                                                                                                                                                                                                                                                 |
| `permission.asked`, `form.created` (opencode, a permission request or a question) | **Needs your input**                                                                                                                                                                                                                                                                                                        |
| `permission.replied`, `form.replied` or `form.cancelled` (opencode)               | **Working**: the request was answered (a rejected permission ends the turn, which opencode then reports).                                                                                                                                                                                                                   |
| `session.execution.succeeded` or `session.execution.interrupted` (opencode)       | **Finished**. A subagent's session (opencode's subagent tool) ending changes nothing.                                                                                                                                                                                                                                       |
| `session.execution.failed` (opencode)                                             | **Finished**: an error ended the turn, and the notification says what went wrong.                                                                                                                                                                                                                                           |

A turn you interrupt (Esc in Claude Code) sends no `Stop`, and neither does a question you dismiss. (opencode reports an interrupted turn as ended, so it shows **Finished**.) Claude Code puts a spinner (◐ ◑) at the start of the terminal's title while it works, and ✳ when it stops, whether the turn ended or it's asking you something. So when the spinner stops and no hook event follows within 8 seconds (Claude Code reports a permission prompt after 6), the turn was interrupted: the agent goes back to **Idle**, without a notification. When the spinner starts again after a question, because you answered it, the agent is **Working** again. An agent whose title has no spinner (or Claude Code with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE` set) goes back to **Idle** after 5 minutes without output or a hook event while **Working**, or 30 minutes while it **Needs your input**. A state reported with [`impulse status`](cli.md#impulse-status) stays until the next report.

## Where the status shows

- **The tab.** A terminal tab running an agent shows the agent's glyph instead of the terminal icon. Hover it for the agent's name and state.
- **The workspace row.** In the sidebar, a workspace shows a spinner while any of its agents work, and a bot icon with a count when agents are waiting for you (needs input or finished). Rows also show a badge with the number of tabs that need attention.
- **The Agents button.** While any agent runs in the window, the titlebar shows an Agents button with a spinner and count for working agents, and "N waiting" when some want you. Click it to open the agent list.
- **The toolbelt.** A bar under the focused agent's terminal shows its state and how long it has been in it. See [The toolbelt](#the-toolbelt).
- **Desktop notifications.** When Impulse is in the background and an agent needs input or finishes, you get a notification: "Claude Code needs your input" or "Claude Code finished", naming the workspace and tab. Clicking one brings the window forward and focuses that terminal, and they're removed once you look at the terminal. macOS asks for permission the first time. On Linux they're sent with `notify-send`; clicking one and removing it work where your `notify-send` supports that (libnotify 0.7.10 or later).
- **The Dock.** The Dock icon bounces once when an agent wants you while Impulse is in the background, and its badge counts the terminals, in all windows, that need attention. On Linux the window is marked urgent instead, and docks and task managers that read Unity's launcher count (KDE's, Dash to Dock, Plank) show the badge.

![The tab strip with three agent tabs: Claude Code working (spinner), Codex needing input (bot with an orange dot) and Gemini CLI finished (bot with a green check mark)](images/agents-tab-states.png)

### The agent list

![The Agents popover listing Codex (Needs your input) in the trailhead workspace, Claude Code (Finished, with a Review button) in fix-elevation, and another Claude Code (Working) in trailhead, with the "Agent hooks give exact status…" link at the bottom](images/agents-inbox.png)

Click the Agents button in the titlebar to see every agent in the window, most urgent first: needs input, then finished, then working, then idle. Each row shows the agent and its tab's title, then the workspace, the state, the message from the agent's last [`impulse status`](cli.md#impulse-status) if it gave one, and how long ago the state changed.

- Click a row to go to that agent's pane (its workspace, tab and pane are brought forward).
- Click **Review** to review the agent's last turn. The button appears on agents with a recorded turn: always once they've finished, otherwise when you hover the row.
- **Agent hooks give exact status…** at the bottom opens the [hooks sheet](#agent-hooks).

## Attention

When an agent needs input or finishes and you aren't looking at its terminal, Impulse flags the terminal as needing attention: the tab and its workspace row are marked, the Dock badge (on Linux, the launcher count) counts it, and you get the notification described above.

"Looking at it" means Impulse is the active app, the window is the key window and the terminal has keyboard focus. If you're looking at an agent when it finishes, Impulse treats the turn as seen: nothing is flagged and the agent goes straight back to **Idle**.

The flag clears, and a **Finished** agent becomes **Idle**, when you:

- select the agent's tab (in the tab strip, in the sidebar's tab list, or by switching to a workspace that shows that tab);
- focus its pane in a split;
- click into the terminal;
- click its input bar;
- jump to it with ⇧⌘U or by clicking its notification or its row in the agent list.

## Jump to the next agent that needs you

Press ⇧⌘U (**File ▸ Next Agent Needing You**, also in the command palette) to bring forward the next agent in this window that's waiting for you. Agents that need input come first, then finished ones, most recent first. Press it again to move on to the next one; it cycles past the agent you're already looking at. If none are waiting, Impulse says "No agents are waiting for you."

## The toolbelt

![The toolbelt under a Claude Code terminal: the agent glyph, "Claude Code", "Finished" with how long ago, and the Compose ⌘I, Review Turn and Turns buttons](images/agents-toolbelt.png)

While an agent runs in the focused terminal, a bar under the terminal shows:

- The agent's glyph and name, and its status: "Working · 42s", "Needs your input", "Finished 2m ago" or "Idle", followed by the message from its last [`impulse status`](cli.md#impulse-status), if any.
- **Compose** (⌘I): opens the [composer](#the-composer).
- **Review Turn**: reviews the agent's last turn. It's dimmed until a turn has been recorded.
- **Turns**: a menu of the agent's 12 most recent turns, each with **Review Turn 3 · 3:42 PM** and **Restore Files to Before Turn 3 · 3:42 PM…**. A turn still in progress is marked "(running)". See [Checkpoints and turns](#checkpoints-and-turns).
- **Subagents** (Claude Code with [hooks](#agent-hooks)): once the agent starts subagents, "Subagents · 2 running", or "Subagents · 5 done" once none run. Its menu lists each one by what it was asked to do, with its state and the last tool Impulse heard it use ("Fix the parser — running, Edit"). A finished one has **Copy Report** (its last message), and one working in a worktree of its own has **Open Worktree**. The list starts over when the agent starts subagents again after all of them finished.

While the agent waits for subagents it left running in the background, the status reads "Waiting on 2 subagents · 40s" instead of "Finished", and the agent list says "2 subagents running".

## The composer

Agent prompts in a terminal are awkward for long or multi-line messages. The composer (⌘I) gives you a real editor for them.

![The composer under a Claude Code terminal, headed "Message Claude Code", with a draft ending in "@src/ph" and the @ mention list above it showing src/photos.ts](images/agents-composer.png)

1. Focus the agent's terminal and press ⌘I (**File ▸ Compose Message to Agent**, **Compose Message to Agent** in the palette, or **Compose** on the toolbelt).
2. Type your message. Return starts a new line.
3. Type `@` and part of a file name to mention a file from the workspace. Choose with ↑ and ↓, then press Tab (or click) to insert its path, such as `@src/lib/units.ts`.
4. Press ⌘↩ to send. Impulse pastes the message into the agent's prompt and presses Return, then closes the composer and puts focus back in the terminal.

| Key    | In the composer                                                              |
| ------ | ---------------------------------------------------------------------------- |
| ⌘↩     | Send: paste into the agent and press Return                                  |
| ⌥⌘↩    | Paste into the agent without pressing Return (the composer stays open)       |
| Return | New line                                                                     |
| `@`    | Mention a file; ↑/↓ choose, Tab inserts                                      |
| ↑ / ↓  | In an empty composer, step through messages you've sent before (the last 50) |
| ⌃C     | Send an interrupt to the agent                                               |
| Esc    | Close the mention list, or close the composer and return to the terminal     |
| ⌘I     | Close the composer                                                           |

The composer keeps an unsent draft when you close it. It works over any program that has taken over the terminal, not only agents. In a terminal where the input bar is showing (no program has taken over), ⌘I focuses the input bar instead.

To have the composer open on its own when the focused agent asks for input, turn on **Open the composer when an agent needs input** (`agent_composer_auto_show`) in Settings ▸ Terminal ▸ Agents. It's off by default. See [Settings and themes](settings-and-themes.md).

## Send things to an agent

You can hand code, files and command output to an agent without copying and pasting.

| From            | How                                                                                                                          | What the agent receives                                                             |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| The editor      | Select code, then run **Send Selection to Agent** from the command palette                                                   | "In @src/forecast.ts, lines 12–18:" and the code in a fenced block                  |
| The file tree   | Right-click a file and choose **Mention in Agent**                                                                           | `@src/lib/cache.ts`                                                                 |
| A command block | Click the block's **Send to agent** button, right-click it and choose **Send to Agent**, or select blocks (⌘↑) and press ⇧⌘A | "I ran \`npm test\` in … (exit status 1). Output:" and the last 200 lines of output |
| Review          | Add comments, then choose **Send Comments to Claude Code · …** from the comments menu                                        | Your comments with their files and lines                                            |
| Problems        | Click **Fix with Agent** in the Problems panel and choose **Send to Claude Code · …**                                        | The problems the panel is showing                                                   |
| Merge conflicts | **Ask Agent** in the Changes panel                                                                                           | A request to resolve the conflicted files                                           |

The editor, file tree and command-block actions pick the agent for you: one in the active workspace that's waiting for you, else any agent in the active workspace, else any agent in the window (never the terminal the command block came from). If no agent is running in the window, Impulse says so. Review, Problems and conflicts list the running agents so you choose one.

Text is pasted into the agent's prompt without pressing Return, so you can read it and add to it first. A toast says "Sent to Claude Code. Review it in its prompt, then press Return." with a **Show** button that takes you there. If the agent is in the middle of a turn (working or waiting on a question), the text is queued instead ("Queued for Claude Code; it's sent when the current turn ends.") and pasted when the turn ends. If the agent exits first, the queued text is dropped rather than pasted into whatever runs next.

For the details of each source, see [Editor](editor.md), [Terminal](terminal.md), [Review](review.md) and [Git](git.md).

## Checkpoints and turns

A turn is one stretch of an agent's work: it starts when the agent goes to **Working** from **Idle** or **Finished**, and ends when it's **Finished**, **Idle** or exits. A question in the middle (**Needs your input**) doesn't split the turn.

At the start and end of every turn, Impulse records a checkpoint of the git repository the agent's terminal is in: every tracked file and every untracked file that isn't ignored, plus what's staged. Checkpoints don't touch your branch, staged changes or stash. Terminals outside a git repository get no checkpoints.

### Review the last turn

Press ⇧⌘I (**File ▸ Review Last Agent Turn**, or **Review Last Agent Turn** in the palette) to open Review on what the agent changed in its last turn. It uses the agent in the focused terminal, or else the most recent turn of any agent in the window's repository. The same review opens from:

- **Review Turn** on the toolbelt;
- **Review** in the agent list;
- **Last agent turn (Claude Code)** in Review's scope menu;
- `impulse review last-turn` in a terminal (see [Command-line tool](cli.md)).

A finished turn is shown as "Agent turn at 3:42 PM" (the start checkpoint against the end one). A turn still running is shown as "Agent turn since 3:42 PM", against the live files, and switches to the finished diff when the turn ends. The diff covers everything that changed in the repository during the turn, including edits you made yourself in that time.

If no turn has been recorded yet, Impulse says "No agent turns recorded yet in this repository."

### Review or restore an earlier turn

Open **Turns** on the toolbelt to pick any of the agent's 12 most recent turns:

- **Review Turn 3 · 3:42 PM** opens Review on that turn.
- **Restore Files to Before Turn 3 · 3:42 PM…** puts the repository's files back to how they were when that turn started.

Turns from an earlier day show the day too ("Turn 2 · Yesterday, 4:10 PM"). The toolbelt, and with it **Turns**, only shows while an agent runs in the terminal: after Impulse restarts, [resume the agent](#resume-after-a-restart) and the terminal's restored turns are in **Turns** again. ⇧⌘I reviews the last one straight away, agent running or not.

To restore:

1. Choose **Restore Files to Before Turn 3 · 3:42 PM…**.
2. Confirm. The dialog explains: "Every file in the repository goes back to how it was when Claude Code started that turn. Commits are kept, and you can undo this right after."
3. Impulse takes a safety snapshot of the current files, then restores. A toast says "Restored files to before turn 3" with **Undo**.

Restoring puts back the content (and staged state) of every file the checkpoint holds. Files created after the checkpoint are left where they are, ignored files aren't touched, and commits stay as they are.

### Send review comments back

In Review, add comments on the agent's lines, then open the comments menu in Review's header and choose **Send Comments to Claude Code · …**. The comments, anchored to their files and lines, go into the agent's prompt (queued if it's busy). See [Review](review.md).

### Where checkpoints are kept

Checkpoints are commits under `refs/impulse/checkpoints/` in your repository, one folder per terminal. They're outside `refs/heads` and `refs/tags`, so a normal `git push` never sends them. The first time in each session that a turn is recorded in a repository, and each time `impulse checkpoint` records one, Impulse deletes that repository's checkpoints older than 14 days, and any beyond the newest 200.

The list of turns (the **Turns** menu, **Review Turn** and ⇧⌘I) is saved with each terminal in your session. When Impulse restores the session, each terminal gets its latest 50 finished turns back, except turns whose checkpoints have since been pruned or are older than 14 days (they'd be pruned next); a turn that was still running when the session was saved isn't restored. When checkpoints are pruned while Impulse runs, the turns that used them leave **Turns** too. Without a restored session (**Restore session** off), the lists start empty, though the checkpoint refs are still in the repository.

You can also record a checkpoint yourself with `impulse checkpoint` (see [Command-line tool](cli.md#impulse-checkpoint)).

## Agent hooks

Without hooks, Impulse infers an agent's state from its output, which is usually right but can lag or misread a pause. Claude Code and Codex can call a program at key moments ("hooks"), and opencode loads plugins that can. Install Impulse's hooks (for opencode, its plugin) and the agent tells Impulse exactly when it starts working, needs you and finishes. Hooks also let Impulse [resume](#resume-after-a-restart) the agent's session after a restart.

The hooks call the `impulse` command-line tool through the `IMPULSE_CLI` environment variable, which only Impulse terminals set. In any other terminal, the hook does nothing and exits successfully, so it's safe to install them for all projects.

### Install hooks

![The Agent Integration sheet on Hooks, with Claude Code and "All projects" selected, "Not installed ~/.claude/settings.json", and a preview of the lines that will be added](images/agents-hooks-sheet.png)

1. Run **Install Agent Hooks…** from the command palette, or click **Agent hooks give exact status…** at the bottom of the agent list. The Agent Integration sheet opens on **Hooks** (its other half sets up [Impulse's MCP server](#impulses-mcp-server)).
2. Choose **Claude Code**, **Codex** or **opencode**.
3. For Claude Code, choose **All projects** or **This project only** (only available when the window is in a git repository). Codex's and opencode's are for all projects.
4. Check the status line ("Installed" or "Not installed", and the file that will change) and the preview of the change: added lines are marked `+`, removed lines `-`.
5. Click **Install Hooks**. Impulse shows "Installed Claude Code hooks. Restart running agents to pick them up."
6. Quit and restart any agents that were already running.

Before writing, Impulse saves the previous version of the file beside it as `<file>.impulse-backup`. If the file is a symlink (as with many dotfile managers), Impulse writes through the link.

If the file changed after the sheet showed its preview (a running agent rewrote its settings, say), **Install Hooks** writes nothing. The sheet makes the preview again from the file as it is now and says "The file changed since this preview, so nothing was written. Check the change again."

### What gets written

- **Claude Code, All projects:** `~/.claude/settings.json`.
- **Claude Code, This project only:** `.claude/settings.local.json` in the window's repository. In a [task](tasks.md) workspace that's the task's folder, which is removed when you archive the task; the sheet says so and names the main checkout. Install project hooks in the main checkout instead: new tasks copy `.claude/settings.local.json` from it (unless the repository's `.worktreeinclude` leaves it out; see [Which files are copied](tasks.md#which-files-are-copied)).

Impulse adds a hook for each of the `SessionStart`, `UserPromptSubmit`, `Notification`, `Stop` and `StopFailure` events, and for `PreToolUse` (shell commands, `"matcher": "Bash"`) and `PostToolUse` (edits and shell commands, `"matcher": "Edit|Write|MultiEdit|NotebookEdit|Bash"`), keeping your other settings and hooks. Each one looks like this:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "command": "[ -n \"$IMPULSE_CLI\" ] && \"$IMPULSE_CLI\" hook claude || true",
            "type": "command"
          }
        ]
      }
    ]
  }
}
```

Impulse rewrites the file pretty-printed with its keys sorted. Hooks that are already there aren't added twice. Hooks installed by an older version of Impulse show as "Out of date: install again to add the newer hooks"; **Install Hooks** then adds only the missing ones.

### What the hooks tell agents

In a repository with [tasks](tasks.md), Impulse's hooks for Claude Code and opencode also tell the agent about the other workspaces, which it otherwise can't see, and Claude Code's about its subagents that share a checkout. Each can be turned off in Settings ▸ Terminal ▸ Agents.

| When                                                                                                                                                                       | The agent is told                                                                                                                                                                                                                                                                                                                                                                                                                                               | Setting                   |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| A session starts                                                                                                                                                           | A short summary: which workspace it's in, the others with their agents, uncommitted files and the files they share with it, and to run `impulse tasks` before merging a branch or making sweeping edits.                                                                                                                                                                                                                                                        | `agent_hook_task_summary` |
| It edits a file that another workspace also changes                                                                                                                        | "Impulse: src/lib/units.ts is also changed in the task fix-elevation, where Claude Code is working…" Once per file.                                                                                                                                                                                                                                                                                                                                             | `agent_hook_shared_files` |
| Claude Code starts a subagent in its own checkout while other subagents that edit files are running there, or starting in the same message                                 | After the call: that they share the checkout and can overwrite each other's changes, and to give subagents that edit files their own worktree (`isolation: "worktree"`), which Impulse lists as [tasks](tasks.md#worktrees-claude-code-makes). Once per batch of subagents. The new subagent is told too, as it starts: to change only the files its task needs and read a file again right before editing it. Read-only subagents (Explore, Plan) don't count. | `agent_hook_subagents`    |
| A subagent edits a file that another subagent, or the agent that started it, changed while it was running, in the same checkout                                            | "Impulse: src/parser.rs was also edited by another subagent ("Fix the lexer") while you were working, in this same checkout…" and to read the file again before changing it. The agent itself, editing a file a subagent that's still running edited, is told to let it finish first.                                                                                                                                                                           | `agent_hook_subagents`    |
| It runs `git merge`, `git rebase` or `git pull` on a task's branch while that task is still being worked on (its agent working or waiting for input, or uncommitted files) | Nothing: Claude Code and opencode stop and ask you first, showing the same reason as Impulse's own [merge question](tasks.md#merge-by-hand).                                                                                                                                                                                                                                                                                                                    | `agent_hook_merge_guard`  |
| Its `git pull`, merge or checkout brought in a changed lock file or `Dockerfile`                                                                                           | Which files changed, and the commands from the project's [`[on_change]` rules](project-config.md#when-files-change) to run before testing. Until you've trusted the project settings, only the well-known dependency files it changed, and to reinstall dependencies.                                                                                                                                                                                           | `agent_hook_dependencies` |

Nothing is written into your repository or its instruction files (`CLAUDE.md`, `AGENTS.md`): it all goes through the hooks. opencode's plugin puts the summary beside opencode's own instructions (its system prompt) for the rest of the session, and a note after the output of the edit or shell command it's about, which the agent reads next. Codex only tells Impulse when a turn ends, so it gets none of this; it can still run `impulse tasks`.

**Codex:** `~/.codex/config.toml`. Impulse adds Codex's `notify` program at the top of the file:

```toml
# Tells Impulse when Codex finishes a turn (no-op outside Impulse).
notify = ["sh", "-c", "[ -n \"$IMPULSE_CLI\" ] && \"$IMPULSE_CLI\" hook codex \"$1\" || true", "impulse-hook"]
```

Codex allows only one `notify` program. If your `config.toml` already sets one, the sheet says "config.toml already sets notify; add Impulse's command by hand" and changes nothing.

**opencode:** a folder, `~/.config/opencode/plugins/impulse/` (under `$XDG_CONFIG_HOME/opencode` if you set `XDG_CONFIG_HOME`), with a plugin of Impulse's own for opencode 2. The preview shows its files. opencode runs one background service for all your terminals, started from whichever terminal ran `opencode` first, so the plugin has two halves:

- `tui.js` loads in each terminal running opencode, where it knows which Impulse pane it's in. It runs `impulse hook opencode <event>` through `$IMPULSE_CLI`, by its path rather than through a shell, so no alias or function can stand in for it, and does nothing outside Impulse terminals. It reports opencode starting, your prompts, permission requests and questions and their answers, and the end of each turn.
- `server.js` loads in opencode's service, where the hooks for the notes and questions above run. It never uses the service's own `IMPULSE_*` variables, which belong to whichever terminal started it: it asks the terminal showing the session (through `rpc.js`), and a session no Impulse terminal shows gets nothing. A shell command the agent runs gets the variables of the terminal its session is in, so `impulse` commands in it act for that pane.

A session belongs to the last terminal that showed it. The plugin also gives opencode [Impulse's tools](#impulses-mcp-server): each call runs in the session's own terminal, through an `impulse mcp` that terminal started.

Because the files are Impulse's, they're written without a backup and replaced as a whole when a newer Impulse changes them ("Out of date"). Installing also removes `plugins/impulse.js`, the single-file plugin older Impulses installed for opencode 1 (which opencode 2 can't load). A folder holding a file that isn't Impulse's is left alone: the sheet says to rename or remove it first.

### Remove hooks

Open the sheet again, choose the same agent (and, for Claude Code, the same scope), check the preview of the lines that will be removed, and click **Remove Hooks**. Impulse shows "Removed Impulse's Claude Code hooks." (or Codex, or opencode), and keeps the previous version as `<file>.impulse-backup`, as when installing.

- **Claude Code:** Impulse removes only its own hook entries, and drops events (and the `hooks` object) that end up empty.
- **Codex:** Impulse removes its `notify` line, the comment above it and the blank line it added after them. A `notify` program that isn't Impulse's is left alone. If Impulse's command was edited onto several lines, the sheet says to remove it from `config.toml` by hand.
- **opencode:** Impulse deletes its plugin's files, and the folder once it's empty.

## Resume after a restart

With hooks installed, Claude Code, Codex and opencode report their session id to Impulse. When Impulse saves your session and later restores it, each terminal that was running one of them comes back with the command that resumes the conversation already typed into its input bar:

```
claude --resume 4f1c2a9e-…
codex resume 7d0b…
opencode --session ses_4a9f…
```

Press Return to resume, or clear it to start fresh. This only applies to Claude Code, Codex and opencode, and only once the agent has sent at least one hook event with its session (for opencode, a prompt: its subagents' sessions don't count). See [Workspaces and tabs](workspaces-and-tabs.md) for session restore.

## The `impulse` command for agents

The `impulse` command-line tool, available in every Impulse terminal, has commands for agents and for scripts that wrap them. See [Command-line tool](cli.md) for the full reference.

| Command                                                        | Use                                                                                                                                                                                                    |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `impulse hook <claude\|codex\|opencode> [event]`               | What the installed hooks run. Reads the hook's JSON from standard input (Claude Code, opencode's plugin) or its last argument (Codex).                                                                 |
| `impulse status <working\|waiting\|done\|idle> [message…]`     | Report a state (and a message shown with it) from a program running in this pane: a wrapper around an agent without hooks, or your own long-running script.                                            |
| `impulse notify <title> [message…]`                            | Flag this pane and send a desktop notification.                                                                                                                                                        |
| `impulse checkpoint [message…]`                                | Snapshot the repository under `refs/impulse/checkpoints/manual/`.                                                                                                                                      |
| `impulse tasks [--json]`                                       | List the repository's workspaces: what each task's agent is doing, uncommitted files, and the files they share with this one. `impulse tasks wait <task>` waits until that task's agent stops working. |
| `impulse tasks new <title…>`                                   | Ask for a task, with an agent started on a brief. Impulse shows New Task first. See [Tasks agents start](tasks.md#tasks-agents-start).                                                                 |
| `impulse comments`, `problems`, `blocks`, `unsaved`, `project` | What Impulse knows about the workspace: review comments, language-server problems, other terminals' commands, unsaved files, the trusted project scripts.                                              |
| `impulse review last-turn`                                     | Open Review on the last agent turn.                                                                                                                                                                    |
| `impulse mcp`                                                  | All of the above as tools, for agents that load MCP servers. See [Impulse's MCP server](#impulses-mcp-server).                                                                                         |

## Impulse's MCP server

Hooks tell Impulse what an agent is doing. Impulse's MCP server works the other way: it gives the agent tools to ask Impulse about the work around it, and to start tasks. Any agent that loads [Model Context Protocol](https://modelcontextprotocol.io) servers can use it: Claude Code, Codex, Gemini CLI, Copilot CLI, Cursor Agent, Goose, Amp and others. opencode gets the same tools from Impulse's [plugin](#what-gets-written) instead.

The server is the `impulse mcp` command, which the agent starts itself. Each tool is one of the [`impulse` commands](cli.md#commands), so the agent gets the same answers as you do in a terminal. The server keeps nothing between calls and decides nothing: Impulse checks every call as it checks the command. Outside an Impulse terminal it starts but offers no tools, so registering it for all projects is safe.

### Install the server

1. Run **Install Impulse's MCP Server…** from the command palette. The Agent Integration sheet opens on **MCP Server**.
2. Choose **Claude Code**, **Codex**, **opencode** or **Other**.
3. For Claude Code, choose **All projects** or **This project only**.
4. Check the preview, then click **Install MCP Server**. Restart running agents to pick it up.

| Agent       | What Impulse does                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code | Runs `claude mcp add-json --scope user impulse …` (or `--scope local` for this project only), as the preview shows. Claude Code rewrites its own `~/.claude.json` while it runs, so Impulse never edits that file; it only reads it to show the status. Needs `claude` on your PATH.                                                                                                                                                                                                                                                                                                                                                                 |
| Codex       | Adds an `[mcp_servers.impulse]` table to the end of `~/.codex/config.toml`, keeping the previous file as `config.toml.impulse-backup`, as for its hook. It sets `tool_timeout_sec = 3600`: Codex gives up on a tool after 5 minutes, and `create_task` and `finish_task` wait for you.                                                                                                                                                                                                                                                                                                                                                               |
| opencode    | Nothing to install: Impulse's [opencode plugin](#what-gets-written) brings the tools. opencode 2 starts MCP servers in its background service, which every terminal shares, so a server there can't tell which pane is asking. If opencode's global config (`~/.config/opencode/opencode.json`, or the `opencode.jsonc` or `config.json` already there) still has the `impulse` entry older Impulses added under `mcp`, the sheet offers to remove it, keeping the previous file as `<file>.impulse-backup`. Only that entry goes (and an `mcp` object it leaves empty): your comments, trailing commas, key order and formatting stay as they were. |
| Other       | Shows the entry to add to the agent's own MCP settings, and **Copy** puts it on the clipboard.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |

The entry runs the bundled `impulse` through `$IMPULSE_CLI`, and forwards `IMPULSE_SOCKET`, `IMPULSE_PANE_TOKEN` and `IMPULSE_CLI`, which tell it which window and pane it's called from:

```json
{
  "command": "sh",
  "args": [
    "-c",
    "exec \"${IMPULSE_CLI:-/Applications/Impulse.app/Contents/Resources/bin/impulse}\" mcp"
  ],
  "env": {
    "IMPULSE_CLI": "${IMPULSE_CLI:-}",
    "IMPULSE_PANE_TOKEN": "${IMPULSE_PANE_TOKEN:-}",
    "IMPULSE_SOCKET": "${IMPULSE_SOCKET:-}"
  }
}
```

The path after `:-` is the `impulse` of the app that installed it (on Linux, the one beside `impulse-app`). If you move Impulse, the sheet says the server is out of date: install it again. Remove it from the same sheet; for Claude Code that runs `claude mcp remove`. An `impulse` server that isn't Impulse's is never replaced or removed.

### The tools

| Tool                              | What the agent gets                                                                                                                                                                                                                   | Same as                               |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------- |
| `list_tasks`                      | The repository's workspaces: branches, bases, agents and their state, how many files are uncommitted, files shared with the caller.                                                                                                   | `impulse tasks`                       |
| `get_task`                        | One task: also its changed files, commits ahead of its base, whether it's merged, its ports and its brief.                                                                                                                            | `impulse tasks show`                  |
| `wait_for_task`                   | Waits until a task's agent stops working, or until its timeout (50 seconds unless it asks for longer, at most 10 minutes), then says which.                                                                                           | `impulse tasks wait`                  |
| `create_task`                     | Asks for a new task, with an agent started on a brief. See [Tasks agents start](tasks.md#tasks-agents-start).                                                                                                                         | `impulse tasks new`                   |
| `finish_task`                     | Opens Finish for a task and runs it up to landing, which waits for you.                                                                                                                                                               | `impulse tasks finish`                |
| `archive_task`                    | Archive Task…, with its questions and Undo.                                                                                                                                                                                           | `impulse tasks archive`               |
| `list_review_comments`            | Your [review comments](review.md#comments) on the repository: file, lines, the code they were written on, their text, whether they're outdated or addressed.                                                                          | `impulse comments`                    |
| `resolve_review_comment`          | Marks a comment addressed with a note. You accept or reopen it in Review.                                                                                                                                                             | `impulse comments resolve`            |
| `get_diagnostics`                 | What the [Problems](editor.md#problems) tab shows for the workspace, errors first, at most 200, and which files are checked. It never starts a language server.                                                                       | `impulse problems`                    |
| `list_command_blocks`             | The workspace's terminals and their recent commands, with exit status and timing, without output.                                                                                                                                     | `impulse blocks`                      |
| `get_block_output`                | One command's output, its last 200 lines unless the agent asks for more (at most 2,000).                                                                                                                                              | `impulse blocks show`                 |
| `list_unsaved_files`              | Files open in the editor with edits you haven't saved, so the agent leaves them alone. Never their text.                                                                                                                              | `impulse unsaved`                     |
| `get_project_config`              | The [project settings'](project-config.md) check and setup scripts, actions and `[on_change]` rules, and a task's ports. Only from settings you've trusted; before that, only what Impulse found itself (lock files, a Compose file). | `impulse project`                     |
| `open_file`, `open_review`        | Opens a file or Review in your window, without taking focus from another app.                                                                                                                                                         | `impulse open`, `impulse review`      |
| `set_status`, `notify`            | For agents without Impulse's hooks: their state in the tab and agent list, and a notification. Where hooks report the agent's state, `set_status` changes nothing.                                                                    | `impulse status`, `impulse notify`    |
| `create_checkpoint`, `list_turns` | A snapshot of the repository, and the agent's recorded turns with their checkpoint refs. Restoring stays in Impulse.                                                                                                                  | `impulse checkpoint`, `impulse turns` |

The tools that start or change something (`create_task`, `finish_task`, `archive_task`, `resolve_review_comment`, `create_checkpoint`), and the two that read other terminals' output (`list_command_blocks`, `get_block_output`), only work when the server runs in the pane's own process tree, like `impulse split`; inside tmux the agent gets the rest. Tools that wait report progress every 15 seconds, and stop waiting when the agent cancels them.

Nothing an agent asks for gets past [workspace trust](getting-started.md) or the project file's trust, and Impulse still never calls a model: the server only answers the agent.

### Settings

In Settings ▸ Terminal ▸ Agents:

| Setting                            | Key                          | Values                             | Default     |
| ---------------------------------- | ---------------------------- | ---------------------------------- | ----------- |
| When an agent asks to start a task | `agent_task_creation`        | `ask`, `trusted_projects`, `off`   | `ask`       |
| Open tasks agents may start        | `agent_task_limit`           | 0 (no limit) to 50, per repository | 4           |
| Agents may read terminal output    | `agent_tool_terminal_output` | `workspace`, `window`, `off`       | `workspace` |

With `agent_task_creation` set to `off`, the server doesn't offer `create_task`; with `agent_tool_terminal_output` set to `off`, it doesn't offer the two block tools (and `impulse blocks` refuses too). Terminal output can hold secrets: `workspace` lets an agent read only the terminals of the workspace it runs in, and never the Quick Terminal.

## Run agents in parallel

Several agents in one checkout get in each other's way. Give each one its own task worktree: **New Task…** (⌥⌘N) creates a branch in its own folder, opens it as a workspace and can start the agent for you. See [Tasks](tasks.md), and [Running agents in parallel](tasks.md#running-agents-in-parallel) for habits that keep their work from colliding.

The same goes for subagents. Claude Code's run in the agent's own checkout unless the Agent call gives them a worktree of their own (`isolation: "worktree"`), and Impulse lists those worktrees as [tasks](tasks.md#worktrees-claude-code-makes), each in the sidebar while its subagent works. To ask for it, tell the agent something like "run subagents that edit files with worktree isolation", or put it in your `CLAUDE.md`. With [hooks](#agent-hooks), Impulse tells the agent when subagents share a checkout and when two of them edit the same file. Or have the agent start tasks instead, as the next section describes.

### Have agents start tasks instead of subagents

When you ask an agent to split work up ("do these in parallel", "use subagents"), it reaches for its own subagents: they run inside its session, where you can't watch, steer or review them one by one. An agent can start [Impulse tasks](tasks.md#tasks-agents-start) instead: each piece gets its own branch and folder, its own workspace in the sidebar, and its own agent working on a brief, which you can follow, talk to, review and finish like any task. Agents don't know to do this, so tell them, in the instructions they read at the start of every session:

| Agent       | Instructions file                                                                                                                     |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code | `CLAUDE.md` in the repository, or `~/.claude/CLAUDE.md` for every project. For one session only: `claude --append-system-prompt "…"`. |
| Codex       | `AGENTS.md` in the repository, or `~/.codex/AGENTS.md`                                                                                |
| opencode    | `AGENTS.md` in the repository, or `~/.config/opencode/AGENTS.md`                                                                      |
| Gemini CLI  | `GEMINI.md` in the repository, or `~/.gemini/GEMINI.md`                                                                               |
| Others      | Most read `AGENTS.md`; see your agent's documentation.                                                                                |

Something like this works for any of them; adjust it to how you like to work:

```markdown
## Parallel work in Impulse

When you run in Impulse (its MCP tools are available, or `$IMPULSE_SOCKET` is set) and I ask for work to be done in parallel or by subagents:

- Give each independent piece of work that changes files its own Impulse task instead of a subagent, with an agent working on a brief: `create_task` with `agent` set to `claude`, or `impulse tasks new "<title>" --agent claude --brief-file <file>` from the shell.
- Write each brief so it stands on its own, because the task's agent starts fresh and knows nothing of this conversation: the goal, the files and areas the task owns, what it must leave alone, and how to check the work.
- Run `list_tasks` first, and split the work by area, so tasks don't change the same files as each other or as work already under way.
- Keep subagents for work that only reads: searching, reviewing, planning.
- Impulse may ask me before it makes each task. If I decline one, do that piece yourself or ask me how to go on.
- Don't merge tasks yourself. Wait for them with `wait_for_task`, see what each changed with `get_task`, and report back; `finish_task` lands one, which still waits for me.

Outside Impulse, give subagents that edit files their own worktree (`isolation: "worktree"`).
```

"When you run in Impulse" keeps the instructions harmless in a repository others work in without Impulse, and the last line gives those agents something to do instead. Change `claude` to the agent you want the tasks to run (`codex`, `opencode`, …); you can still pick another on the New Task sheet.

What happens next is up to your settings ([Settings](#settings) above): each request opens New Task for you to change or cancel unless `agent_task_creation` lets agents start tasks in trusted projects, and requests past `agent_task_limit` (4 open tasks per repository) are refused until you finish or archive one. A task started this way can't start further tasks without asking you, whatever the setting. Agents without [Impulse's MCP server](#impulses-mcp-server) use the `impulse tasks` commands instead, which work from any shell the agent runs in its terminal.

## Troubleshooting

- **The agent isn't recognized.** Check that it's one of the [supported agents](#supported-agents) and that your shell is bash, zsh or fish with shell integration working. Agents launched through an unusual wrapper may not be recognized from the process; install [hooks](#agent-hooks) (Claude Code, Codex, opencode) or call `impulse status` from your wrapper.
- **The state is wrong or late.** Without hooks, Impulse guesses from output. Install hooks for Claude Code, Codex or opencode, then restart the agent.
- **No desktop notifications.** They're only shown when Impulse is in the background, and macOS must allow notifications for Impulse (System Settings ▸ Notifications). On Linux, Impulse sends them with `notify-send` (from libnotify), which must be installed.
- **Review Last Agent Turn finds nothing.** Turns are only recorded in a git repository, and after a restart they come back only for terminals restored with the session.
- **The hooks sheet shows an error.** The settings file isn't the JSON object Impulse expects (for example, a syntax error), Codex already has a `notify` program, Impulse's own `notify` line can't be found to remove, or opencode's `plugins/impulse` folder holds a file that isn't Impulse's. Fix the file, then open the sheet again.

## Related

- [Tasks](tasks.md): one worktree per agent, so agents work in parallel
- [Review](review.md): the Review surface, comments and sending them to agents
- [Terminal](terminal.md): shell integration, command blocks and the input bar
- [Command-line tool](cli.md): `impulse hook`, `status`, `notify`, `checkpoint`, `review` and `mcp`
- [Workspaces and tabs](workspaces-and-tabs.md): session restore
- [Keyboard shortcuts](keyboard-shortcuts.md)
