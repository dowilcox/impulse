# Warp-Inspired Impulse Improvements

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Capture the product and architecture lessons from Warp's open-source repo that fit Impulse, then implement them in Impulse with clean-room designs that match this codebase.

**Architecture:** Keep terminal parsing and command/session state in `impulse-terminal` when it is platform-neutral, shared shell integration and settings in `impulse-core`, and frontend-specific rendering and interaction in `impulse-linux` or `impulse-macos`.

**Licensing note:** Warp's UI framework crates are MIT, but the rest of the Warp repository is AGPL. Treat Warp as design research. Do not copy implementation code or script bodies from AGPL-licensed files into Impulse.

**Warp source areas reviewed:** terminal secret handling, quit-warning summaries, command-palette data sources, repo metadata updates, inline command history, shell parsing/completion, grid filtering, tab configs, worktree specs, settings-file error handling, settings offline-edit detection, path-drop behavior, and git subprocess PATH handling from <https://github.com/warpdotdev/warp>.

---

## Priority Roadmap

### 1. Command Blocks

**User value:** Terminal output becomes structured and navigable instead of one continuous scrollback.

**Fit:** Strong. Impulse already receives OSC 7 and OSC 133 events and now aligns command start to `OSC 133;C`.

**Clean-room design:**

- Track command block records in `impulse-terminal`: command text, cwd, start time, end time, exit code, and output line range.
- Keep block metadata separate from the terminal grid so rendering stays compatible with alacritty state.
- Expose block metadata through FFI and Linux Rust APIs.
- Add frontend actions: copy command, copy output, rerun command, jump to previous or next block, jump to failed block.

**Implementation tasks:**

- [x] Add `TerminalBlockId` and `TerminalCommandBlock` structs in `impulse-terminal`.
- [x] Extend shell integration with an Impulse-owned command-text payload emitted from preexec hooks.
- [x] Record block start/end when `CommandStart` and `CommandEnd` events are observed.
- [x] Track output line ranges in the backend without changing visible terminal output.
- [x] Expose block metadata over FFI for macOS and native Rust APIs for Linux.
- [x] Add focused scanner/backend tests for command text, cwd, exit code, and line range.
- [x] Add macOS and Linux UI affordances for copy, rerun, and block navigation.

### 2. Project Launch Configs

**User value:** One command opens a project exactly as needed: files, terminal tabs, working directories, and startup commands.

**Fit:** Strong. Impulse already owns editor tabs, terminal tabs, and project directory state.

**Clean-room design:**

- Support a project-local config file such as `.impulse/launch.json`.
- Model launch entries as editor tabs, terminal tabs, split layouts, cwd, and optional commands.
- Keep command execution explicit or gated behind a setting to avoid surprise side effects.

**Implementation tasks:**

- [ ] Define a versioned launch-config schema in `impulse-core`.
- [ ] Add parser and validation tests.
- [ ] Add command-palette entries for "Open Launch Config" and "Run Launch Config".
- [ ] Wire macOS and Linux frontends to create tabs/splits from the parsed config.
- [ ] Add confirmation UI for launch configs that execute commands.
- [ ] Add "Save Current Layout as Launch Config" by snapshotting editor/terminal tab layout into a generated config file.
- [ ] Open the generated launch config for review immediately after writing it.

### 3. Settings Schema and Validation

**User value:** Settings files become safer to hand-edit and easier to validate in UI.

**Fit:** Strong. `impulse-core/src/settings.rs` already centralizes defaults, migrations, and validation.

**Clean-room design:**

- Generate a JSON Schema for the shared `Settings` type.
- Validate defaults and sample settings against the schema in tests.
- Use the same metadata to drive future settings UI hints.

**Implementation tasks:**

- [x] Add schema generation for `Settings`, `CommandOnSave`, `CustomKeybinding`, and file-type overrides.
- [x] Add tests that validate `Settings::default_json()` against the generated schema.
- [x] Add `impulse settings schema` or a script target to write the schema artifact.
- [x] Surface schema validation errors in settings import/load paths.
- [x] Preserve broken settings files on disk instead of overwriting them with defaults.
- [x] Add a dismissible settings-error banner with an "Open settings file" action.
- [ ] Track a stable content hash for settings files so offline edits are not silently lost when future sync exists.

### 4. Terminal Block Search and Filtering

**User value:** Search the terminal by command, output, cwd, duration, or failure state.

**Fit:** Medium-high. This should build on Command Blocks instead of being implemented first.

**Clean-room design:**

- Add a block-aware index over terminal history.
- Keep full-text matching local and deterministic.
- Start with command/output text and exit status; defer semantic/AI search.

**Implementation tasks:**

- [ ] Add block query model in `impulse-terminal`.
- [ ] Implement command-text and output-range matching.
- [ ] Add filters for failed commands and long-running commands.
- [ ] Add frontend search UI entry points.

### 5. Shell-Aware Command Text Capture

**User value:** Tabs, notifications, history, and rerun actions can show the actual command instead of only knowing that a command started.

**Fit:** Strong, but it should be implemented as part of Command Blocks.

**Clean-room design:**

- Use an Impulse-owned OSC number or DCS payload for command text.
- Encode payloads safely so arbitrary command bytes cannot break the terminal parser.
- Preserve compatibility with standard OSC 133 prompt markers.

**Implementation tasks:**

- [ ] Choose and document an Impulse-private escape payload format.
- [ ] Add scanner support with size limits and malformed-payload tests.
- [ ] Emit command text from bash, zsh, and fish preexec hooks.
- [ ] Thread command text into block metadata and frontend notifications.

### 6. Session Restore

**User value:** Reopen a project with the same editor files, terminal cwd values, and layout.

**Fit:** Strong. Impulse already stores `open_files` and `last_directory`; this extends that idea to terminal/session layout.

**Clean-room design:**

- Persist local session layout in Impulse settings/state.
- Restore terminal tabs with cwd, not process state.
- Avoid restoring commands automatically unless the user explicitly chooses a launch config.

**Implementation tasks:**

- [ ] Add a versioned session-state model in `impulse-core`.
- [ ] Record editor tabs, terminal tabs, cwd, active tab, and split layout.
- [ ] Restore state at startup after project selection.
- [ ] Add settings to enable/disable session restore.

### 7. Settings Import

**User value:** Users can bring terminal look-and-feel from existing tools with less manual setup.

**Fit:** Medium. This is useful but less central than command/session features.

**Clean-room design:**

- Import common settings from Alacritty and iTerm-style config where mappings are clear.
- Keep import previewable and non-destructive.

**Implementation tasks:**

- [ ] Add Alacritty theme/settings parser for font, cursor, scrollback, and colors.
- [ ] Add iTerm color/theme import for macOS.
- [ ] Add import preview UI before applying settings.

### 8. Terminal Secret Redaction

**User value:** API keys, tokens, and passwords printed in terminal output are harder to expose accidentally in screenshots, copy operations, and saved history.

**Fit:** Strong. Impulse owns terminal grid processing in `impulse-terminal`, and redaction should be shared across frontends.

**Clean-room design:**

- Maintain redaction ranges as metadata beside the terminal grid, not as destructive edits to terminal contents.
- Start with opt-in detectors for common high-risk patterns: GitHub tokens, OpenAI-style keys, AWS keys, private-key headers, and obvious `KEY=value` secrets.
- Add explicit reveal/copy policy so users can choose whether copied output includes redacted plaintext.

**Implementation tasks:**

- [ ] Add a detector trait and default detector set in `impulse-terminal`.
- [ ] Scan dirty output ranges and store redaction spans with stable grid coordinates.
- [ ] Render redacted spans in macOS and Linux terminal views.
- [ ] Add settings for enabled detectors and copy behavior.
- [ ] Add tests for span updates across scrollback, wrapped lines, and partial dirty ranges.

### 9. Quit and Close Safety Warnings

**User value:** Closing Impulse should not silently kill long-running commands or discard unsaved editor changes.

**Fit:** Strong. macOS already has unsaved editor tab state, and terminal tabs already know command lifecycle timing.

**Clean-room design:**

- Summarize risk at close time: unsaved editor files, running terminal commands, long-running commands, and active background jobs where known.
- Keep warning text concise and action-oriented.
- Allow a setting to disable warnings for users who prefer fast close behavior.

**Implementation tasks:**

- [x] Add a shared close-risk summary model in `impulse-core`.
- [x] Thread terminal command-start timestamps and editor dirty state into the summary.
- [x] Add macOS window/app close confirmation.
- [x] Add Linux window close confirmation.
- [x] Add tests for summary wording and warning thresholds.

**Implementation note:** The first pass warns on macOS for unsaved editors, running terminal processes, and active command blocks. Linux warns for unsaved editors and active command blocks; process counts remain `0` until the GTK frontend exposes a platform-specific child-process count.

### 10. Shared Command Palette Registry and Recents

**User value:** One fast launcher can open commands, files, settings, launch configs, recent projects, and terminal actions.

**Fit:** Strong. Impulse has command palette UI, but it is frontend-heavy and should become shared capability over time.

**Clean-room design:**

- Move command identity, scoring metadata, and recent-item identity into `impulse-core`.
- Let each frontend supply UI rendering and platform-specific actions.
- Support synchronous sources first, then add async file/project sources.

**Implementation tasks:**

- [ ] Define `CommandPaletteItem`, stable item IDs, and `RecentCommandItem` in `impulse-core`.
- [ ] Port existing macOS palette commands onto the shared registry.
- [ ] Add Linux palette integration against the same registry.
- [ ] Add recents storage with deduping and stable identity across renamed labels.
- [ ] Add async file/project search sources after the synchronous registry is stable.

### 11. Incremental File Tree Store

**User value:** Large repos update quickly without rebuilding the entire visible tree on every filesystem or git change.

**Fit:** Strong. This extends the existing file-tree performance plan and helps both GTK and macOS.

**Clean-room design:**

- Keep a flattened tree plus stable node IDs in a shared model.
- Apply remove-before-add updates so moves are represented cleanly.
- Preserve expansion, selection, and scroll state across incremental updates.

**Implementation tasks:**

- [ ] Add an incremental file-tree update protocol in `impulse-core`.
- [ ] Convert watcher events into batched remove/update operations.
- [ ] Teach macOS and Linux sidebars to apply patches instead of full reloads.
- [ ] Add tests for rename, move, delete, nested directory replacement, and git-status refresh.

### 12. Rich Inline Command History

**User value:** Terminal history can show the right command for the current project, current directory, branch, exit status, and session.

**Fit:** Medium-high. This should build on Command Blocks so history has reliable metadata.

**Clean-room design:**

- Store command history as structured records: command, cwd, shell, exit code, start/end time, git branch, and session ID.
- Prefer current-session and current-cwd matches before global matches.
- Keep insertion deterministic and local; defer any AI/conversation integration.

**Implementation tasks:**

- [ ] Add a history store in `impulse-core` or `impulse-terminal` after Command Blocks land.
- [ ] Record completed block metadata into history.
- [ ] Add prefix and fuzzy matching.
- [ ] Add terminal input UI for selecting a history entry.
- [ ] Add rerun-from-history behavior with clear shell escaping rules.

### 13. Lightweight Shell Parser and Completion Context

**User value:** Completions, history, and rerun actions can understand command boundaries instead of treating input as plain text.

**Fit:** Medium. Useful, but a full shell parser is a large project and should start narrow.

**Clean-room design:**

- Parse only enough shell structure for command name, current argument span, quoting state, variable assignments, and redirection boundaries.
- Avoid executing commands or shell expansions in the first version.
- Expose a stable completion context that can power file, git branch, and launch-config completions.

**Implementation tasks:**

- [ ] Add a small parser module in `impulse-core` with bash/zsh/fish-compatible common cases.
- [ ] Add tests for quoting, escaping, pipes, redirects, env assignments, and unfinished input.
- [ ] Feed parser output into terminal history search and file completion.
- [ ] Add a future extension point for git branch and command-specific completions.

### 14. Terminal Output Filtering With Context

**User value:** Users can narrow a long terminal output to matching lines while keeping nearby context.

**Fit:** Medium-high after Command Blocks. Impulse already has terminal search primitives, but not filtered display.

**Clean-room design:**

- Keep filtering as a display projection over scrollback, not a mutation to the terminal grid.
- Support regex, case sensitivity, context lines, and invert-match.
- Build block-local filtering first, then whole-terminal filtering.

**Implementation tasks:**

- [ ] Add a filter query model in `impulse-terminal`.
- [ ] Translate between original row coordinates and displayed filtered rows.
- [ ] Add block-local filter actions in macOS and Linux.
- [ ] Add tests for context merging, invert-match, scrollback movement, and dirty-line refresh.

### 15. Worktree Factory

**User value:** Creating an isolated branch workspace should be a guided workflow, not a manual set of `git worktree` commands.

**Fit:** Medium-high. Impulse is already project-aware and has git status code.

**Clean-room design:**

- Add a modal or command-palette flow to select a repo and base branch.
- Generate a launch config that creates/opens the worktree and starts terminals in the new directory.
- Keep destructive cleanup explicit and previewable.

**Implementation tasks:**

- [ ] Add git branch listing and default-branch detection helpers in `impulse-core`.
- [ ] Add a worktree launch-config generator with filename/branch sanitization.
- [ ] Add UI for repo, base branch, and generated branch/worktree name.
- [ ] Add an optional cleanup command that can be attached as a launch-config close hook.

### 16. Launch Config Close Hooks

**User value:** Temporary workspaces can clean up background services, docker compose stacks, or ephemeral worktrees when a tab closes.

**Fit:** Medium. Valuable once launch configs exist, but it must be guarded carefully.

**Clean-room design:**

- Support optional `on_close` commands in launch configs.
- Resolve close behavior from the live tab instance, not from stale persisted snapshots.
- Run once, show failures, and never hide destructive effects.

**Implementation tasks:**

- [ ] Extend launch-config schema with optional close hooks.
- [ ] Require confirmation for destructive-looking close hooks.
- [ ] Execute close hooks best-effort when a launch-config tab closes.
- [ ] Show persistent failure UI when cleanup fails.
- [ ] Add tests for single-fire behavior and snapshot restore interactions.

### 17. Per-Tab and Directory Theme Overrides

**User value:** Projects and launch configs can carry visual identity without changing the global theme.

**Fit:** Medium. Useful for context switching, especially with multiple projects open.

**Clean-room design:**

- Add optional theme references to launch config tabs/windows.
- Add directory-pattern theme rules in settings.
- Resolve theme per tab from explicit tab override, launch config, directory match, then global default.

**Implementation tasks:**

- [ ] Add theme lookup and validation helpers in `impulse-core`.
- [ ] Add optional per-tab theme fields to launch/session state.
- [ ] Apply theme overrides in macOS and Linux render paths.
- [ ] Add settings UI and schema support for directory theme rules.

### 18. Terminal Path Drop Normalization

**User value:** Dragging files into terminal input should insert paths in the form the active shell can actually use.

**Fit:** Medium. Impulse already has terminal drag/drop on macOS and file tree drag/drop in both frontends.

**Clean-room design:**

- Centralize path normalization by shell/session type.
- Preserve raw host paths for contexts that need host filesystem access, such as image attachment handling.
- Apply shell escaping after path normalization.

**Implementation tasks:**

- [ ] Add shared path normalization helpers in `impulse-core`.
- [ ] Audit macOS terminal drag/drop and paste-image fallback.
- [ ] Add Linux terminal drag/drop parity if missing.
- [ ] Add tests for spaces, quotes, home paths, POSIX shells, and future WSL/Git Bash support.

### 19. Interactive PATH for Git and Tooling Commands

**User value:** GUI-launched Impulse should find the same `git`, `gh`, language tools, and hooks that the user's interactive shell finds.

**Fit:** Strong, especially on macOS where Dock-launched apps inherit a minimal environment.

**Clean-room design:**

- Capture the user's interactive shell PATH once per app session with a timeout.
- Use it for git operations, LSP/tool discovery, and launch-config helper commands where appropriate.
- Fall back to inherited PATH if capture fails.

**Implementation tasks:**

- [ ] Add shared interactive PATH capture and caching in `impulse-core`.
- [ ] Thread captured PATH into git helper commands.
- [ ] Use the same PATH for LSP/tool discovery where safe.
- [ ] Add logging for capture failures without blocking app startup.

### 20. Local Diagnostic Log Pane

**User value:** Debugging Impulse itself becomes easier without attaching a debugger or hunting logs.

**Fit:** Low-medium. Useful for development and support, but not core workflow.

**Clean-room design:**

- Add a bounded in-memory log stream for app diagnostics.
- Include optional file-tail sources for app, LSP, update, and terminal backend logs.
- Keep the pane local-only and out of session restore by default.

**Implementation tasks:**

- [ ] Add a bounded diagnostic event channel in `impulse-core`.
- [ ] Feed selected app/LSP/update errors into the channel.
- [ ] Add a hidden command-palette action to open the diagnostic pane.
- [ ] Add filtering by source and severity.

---

## First Implementation Recommendation

Build **Command Blocks** first. It is the enabling layer for block search, command history, rerun, long-command notifications, and richer terminal UI. Keep the first version narrow:

- record block metadata,
- capture command text,
- expose copy/rerun actions,
- add tests around OSC parsing and block lifecycle.

After Command Blocks, prioritize the safety stack: **Terminal Secret Redaction**, **Quit and Close Safety Warnings**, and **Settings Error Banner**. Those are high-value features with limited dependency on larger launch-config or worktree work.
