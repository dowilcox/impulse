# Pi integration: what's left

Status, 8 Nov 2026. Not user documentation: this folder isn't published to the website (`scripts/build-site.py` only builds `docs/*.md`). The user-facing description is in [Agents](../agents.md#what-gets-written).

## What was built

Pi (`@earendil-works/pi-coding-agent` 1.5, the CLI `pi`) is a supported agent at Claude Code's level:

- Recognized from its process (`known_agents.rs`: either npm package name, or a binary called `pi`), resumed with `pi --session <id>`, and started by New Task with the brief after `--` (`task_requests.rs`).
- One extension of Impulse's own, `impulse-kit/resources/agents/pi-extension/impulse.js`, installed by Agent Integration into `$PI_CODING_AGENT_DIR/extensions/` (default `~/.pi/agent/extensions/`). It reports Pi's events to `impulse hook pi <event>` under Claude Code's names (`SessionStart`, `UserPromptSubmit`, `Stop`, `StopFailure`, `PreToolUse`/`PostToolUse` for `bash`, `edit`, `write`), plus `prompt.start`/`prompt.end` (a dialog waiting mid-turn) and `ProjectTrust`.
- It reads Impulse's replies as the opencode plugin does (`{"context"}`, `{"ask"}`, and `{"trusted"}` for trust): the other tasks' summary as the `impulse` section of Pi's system prompt, notes after edits and shell commands, and the merge question in `ctx.ui.confirm`.
- Impulse's tools are registered as Pi's own (`impulse_<name>`), from an `impulse mcp` the extension starts in the pane. Pi needs no MCP entry.
- Pi's `project_trust` is answered from Impulse's trust (`WorkspaceTrust::verdict`): yes for trusted folders, no for restricted ones, nothing otherwise or with `workspace_trust` off. Setting: `agent_hook_project_trust`.
- A Pi started by the pane's Pi (`IMPULSE_PI_PID` set by another process) gets the tools and reports nothing.
- An older Impulse's extension is pointed out once a run (`Workbench::check_pi_extension`).

## Verified here (Linux, cloud container)

- `cargo test --workspace`: all pass. `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets`: no warnings in the files this work touched.
- `scripts/check-macos.sh`: the app type-checks for `aarch64-apple-darwin`.
- `scripts/build-site.py --check`: all docs links and anchors resolve.
- Smoke checks under Xvfb (`scripts/smoke/run.py`), including the new `pi` scenario: Agent Integration replacing an older extension, the out-of-date note, states (done, needs input during the merge question, done after an error), resume, the trust answer, the summary section with tools renamed, the shared-file note, Impulse's tools in the pane, the merge question approved, and a nested Pi reporting nothing. Results of the full run are below.
- The extension was checked by hand against Pi 1.5.0's published type declarations (`dist/core/extensions/types.d.ts`): every event it handles exists with the fields it reads; `project_trust` returns `{trusted}`; `edit` and `write` take `path`; the session manager has `getSessionId`/`getSessionFile`; the bash tool spreads `process.env` into commands at call time (so `IMPULSE_PI_PID` reaches nested Pis).

Full smoke run (8 Nov 2026, Linux, Xvfb with software Vulkan, zsh 5.9, bash, fish 3.7, node 22): every scenario passed, 542 checks: editor 30, terminal-zsh 115, terminal-bash 108, terminal-fish 108, fish-check 11, images 4, scratch 8, agents 39, lsp 15, tasks 14, agent-integration 11, opencode 32, pi 25, mcp 19, session 3.

## Real Pi run (8 Nov 2026)

Pi 1.5.0 (`@earendil-works/pi-coding-agent`) in this branch's dev build under Xvfb, the extension loaded with `pi -e`, the model Gemma 4 E4B served locally by Hob's `llama-server` (no cloud account). Items marked verified below come from this run. Found: Esc mid-turn showed Finished (since changed to Idle, below), and the summary said "(or Impulse's impulse_list_tasks tool, if you have its MCP server)" in Pi, where the tools are always there (since reworded for every agent: "Run `impulse tasks` or Impulse's list_tasks tool before merging…").

## Needs another environment

### A real Pi (any machine where Pi can be installed)

Installing Pi from npm wasn't allowed in the container, so the extension has only run under the stand-in (`scripts/smoke/pi-agent.mjs`), which calls it as Pi's documentation says Pi does. Before release, with a real Pi and a model:

- [ ] Agent Integration ▸ Hooks ▸ Pi ▸ Install Extension, then `pi` in an Impulse terminal: Pi loads it without errors (`/extensions` or the startup list shows it); outside Impulse it does nothing.
- [x] A prompt: the tab shows Working, then Finished, with a notification. A retried or compacted run stays one turn. **Verified 8 Nov** (Working, then Finished; no notification checked: the run was under Xvfb without D-Bus).
- [x] Esc mid-turn: Pi sends `agent_settled` with `aborted`, which the extension reports as `Stop`, so the agent shows Finished (opencode does the same; Claude Code shows Idle). Decide whether aborted should be Idle instead (a new event name mapped to `AgentHook::Status(Idle)` or similar). **Verified 8 Nov**: it showed Finished; **decided** to match Claude Code: the extension now sends `Interrupted` when `agent_settled` is `aborted`, which `hook_events` maps to `AgentHook::Interrupted`, back to Idle with no notification (smoke: `pi aborted`).
- [x] An error (bad API key, rate limit): Finished, with the provider's message in the notification. **Verified 8 Nov** (a bad OpenAI key: Finished, the 401 in the transcript).
- [x] `git merge <branch of a busy task>` asked of Pi: Pi's confirm dialog shows Impulse's reason; the tab shows Needs your input while it's open; declining blocks the command with the reason. **Verified 8 Nov**: the dialog read "Impulse asks before this merge — busy is still being worked on: … 1 uncommitted file …", the tab showed Needs your input, Yes ran it. Declining not tried.
- [ ] Impulse's tools: Pi lists `impulse_list_tasks` and the rest; `impulse_create_task` opens New Task in Impulse and waits; Esc while it waits cancels the call (`notifications/cancelled`). **Partly verified 8 Nov**: `impulse_list_tasks` called and answered; `impulse_create_task` not tried.
- [ ] The tools' JSON schemas are accepted by Pi's validator and by each provider used (Anthropic, OpenAI, Google); Impulse's `inputSchema`s are plain JSON Schema, which Pi documents as fine, but provider quirks (e.g. `additionalProperties`) only show with real calls. **Partly verified 8 Nov**: accepted by Pi and by llama.cpp's OpenAI-compatible server (Gemma 4 E4B through Hob's `llama-server`); not yet by Anthropic, OpenAI or Google.
- [x] The summary section appears in Pi's system prompt (`/session` or an HTML export) once per session, not repeated each turn. **Verified 8 Nov**: the model quoted the `<impulse>` section. Once-per-session not checked.
- [ ] Notes after an edit of a file another task changes, and after a `git pull` that changes a lock file.
- [ ] Trust: a project with `.pi/extensions` in a folder trusted in Impulse loads without Pi's trust prompt; in a task made from a remote branch (restricted) Pi doesn't load it; elsewhere Pi asks as usual. With `agent_hook_project_trust` off, Pi always asks.
- [ ] `/reload`, `/new`, `/resume`, `/fork`: the tools come back, and Impulse's resume command follows the new session (restart Impulse and check the terminal resumes the right one).
- [x] `pi -p "…"` run by Pi's own bash tool, and a subagent extension's child Pis: they don't change the pane's state. **`pi -p` verified 8 Nov**: the nested Pi had Impulse's tools and the pane kept the outer Pi's state.
- [ ] Recognition: Pi installed with npm (both package names), with bun, and as the release binary, through `nvm`/`volta` shims.
- [x] `PI_CODING_AGENT_DIR` set in the shell profile but not in Impulse's own environment: **fixed** (Nov 2026). The login shell Impulse already starts for its `PATH` reports it too (`shell::login::LOGIN_VARIABLES`, `login_variable`), and Agent Integration and the out-of-date check read it through `hook_installer::agents_environment`.
- [x] Older Pi (`@mariozechner/pi-coding-agent` 0.x): decided against. 0.69 has no `agent_settled` (no `Stop`), `project_trust`, `ui_prompt_*` or `systemPromptOptions.sections` (the `before_agent_start` handler would throw), so Pi 1.5 is the documented minimum ([Agents](../agents.md#what-gets-written)).

### macOS

`check-macos.sh` only type-checks. On a Mac:

- [ ] `scripts/build.sh --dev`, then `python3 scripts/smoke/run.py pi opencode agents agent-integration` (needs `node`).
- [ ] The Agent Integration sheet in SF Pro: the MCP Server side shows five agents at 88 points each beside Claude Code's 170-point scope pop-up (fits on Linux with a few points to spare); check "Claude Code" isn't clipped.
- [ ] Regenerate `docs/images/agents-hooks-sheet.png` (`python3 scripts/docs/capture.py agents-hooks-sheet`): it shows three agents, the sheet now has four on Hooks. The shots are made on the Mac (window buttons, SF Pro), so it wasn't regenerated on Linux.
- [ ] Regenerate `docs/images/tasks-sidebar-group.png` and `tasks-sidebar-collapsed.png` (`python3 scripts/docs/capture.py tasks-sidebar`): the repository header moved 2 points and got a hover tint, and the collapsed shot was first made on Linux at 1x.
- [ ] A desktop notification for a finished and a failed Pi turn, and the Dock badge while Pi needs input.

### CI

- [ ] `.github/workflows/tests.yml` (fmt and tests on Linux and macOS) runs on the pull request.

## Possible follow-ups

- Subagents: Pi has none built in. Its example `subagent` extension starts child `pi` processes, which the extension treats as nested (tools, no reports). Following them as Impulse follows Claude Code's would need events from that extension.
- Sending text to Pi: Impulse pastes into Pi's editor as for other agents. The extension could take messages from Impulse directly (`pi.sendUserMessage`) if the app had a way to push to it.
- A footer status in Pi (`ctx.ui.setStatus`) naming the Impulse task the pane is in.
