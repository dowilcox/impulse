//! The agent side of the control socket (the Swift `ControlCommands`'
//! `hook`, `status` and `notify`, `AgentWorkspaceCommands`' `turns` and
//! `blocks`, and `TaskAwareness`' hook replies): what an agent's hooks
//! report about it, `impulse status`, its turns, the terminals' command
//! blocks, and what Impulse answers Claude Code's hooks and the opencode
//! plugin with, letting agents see each other: a summary of the
//! repository's other workspaces when a session starts, a note when it
//! edits a file another workspace changes, a question before it merges a
//! branch a task is still working on, and dependency files that changed
//! under it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::{App, AppContext as _, Context, Entity, Window};
use impulse_kit::agents::hook_events::{self, Reply};
use impulse_kit::agents::state_machine::{AgentHook, AgentState};
use impulse_kit::git::task_overlap::Changes;
use impulse_kit::git::task_registry::{self, TaskRecord};
use impulse_kit::project::dependency_changes::{self, Move};
use impulse_protocol::agent_tools::{
    hook_reply, Block, BlockList, BlockOutput, TaskListing, Terminal, TurnList, Workspace,
};
use impulse_protocol::control::{ControlRequest, ControlResponse};

use crate::settings_store::SettingsStore;
use crate::terminal_view::TerminalView;
use crate::workbench::Workbench;
use crate::workspace::{Kind, Surface};

type Answer = std::rc::Rc<dyn Fn(ControlResponse)>;

fn repo_root(path: &str) -> Option<String> {
    impulse_git::branches::repo_root(path).map(|r| r.trim_end_matches('/').to_string())
}

fn name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// The agents in folder workspaces at `folder`, in every window.
pub fn agents_in_folder(folder: &str, cx: &App) -> Vec<(String, AgentState)> {
    agent_terminals_in_folder(folder, None, cx)
        .into_iter()
        .map(|(_, name, state)| (name, state))
        .collect()
}

impl Workbench {
    /// The names of the agents working in `folder`, in every window, but
    /// the one in the terminal `excluding`. From inside this window's
    /// update, which it reads from `self`.
    pub(crate) fn working_agents_in(
        &self,
        folder: &str,
        excluding: Option<gpui_kit::EntityId>,
        cx: &Context<Self>,
    ) -> Vec<String> {
        agent_terminals_in_folder(folder, Some((cx.entity_id(), self)), cx)
            .into_iter()
            .filter(|(id, _, state)| *state == AgentState::Working && Some(*id) != excluding)
            .map(|(_, name, _)| name)
            .collect()
    }
}

/// The agents in folder workspaces at `folder`, with their terminals.
/// `current`: a window being updated, which can't be read from the app.
fn agent_terminals_in_folder(
    folder: &str,
    current: Option<(gpui_kit::EntityId, &Workbench)>,
    cx: &App,
) -> Vec<(gpui_kit::EntityId, String, AgentState)> {
    let folder = task_registry::canonical(folder);
    let mut agents = Vec::new();
    for (workbench, _) in crate::session::workbenches(cx) {
        let workbench = match current {
            Some((id, current)) if id == workbench.entity_id() => current,
            _ => workbench.read(cx),
        };
        for workspace in &workbench.workspaces {
            if workspace.kind != Kind::Folder
                || task_registry::canonical(&workspace.root.to_string_lossy()) != folder
            {
                continue;
            }
            for terminal in workspace.all_surfaces().filter_map(Surface::terminal) {
                let tracker = &terminal.read(cx).agent;
                if let (Some(agent), Some(state)) = (&tracker.agent, tracker.state()) {
                    agents.push((terminal.entity_id(), agent.display_name.clone(), state));
                }
            }
        }
    }
    agents
}

fn busy(state: AgentState) -> bool {
    matches!(state, AgentState::Working | AgentState::NeedsInput)
}

fn describe(state: AgentState) -> &'static str {
    match state {
        AgentState::Working => "working",
        AgentState::NeedsInput => "needs input",
        AgentState::Done => "finished",
        _ => "idle",
    }
}

// MARK: The repository's workspaces

/// What each of the repository's workspaces changes: uncommitted files
/// and commits since its base (the Swift `OverlapMonitor.changes`), with
/// the tasks Impulse made. Off the main thread; empty without tasks.
fn workspace_changes(main: &str) -> (Vec<TaskRecord>, Vec<Changes>) {
    let tasks = crate::tasks::registry(main)
        .map(|r| r.tasks)
        .unwrap_or_default();
    let changes = crate::tasks::changes(main, &tasks);
    (tasks, changes)
}

/// `workspace_changes`' changes, read again at most every few seconds: an
/// agent's edits come in bursts, and each would otherwise run git in every
/// worktree of the repository.
fn recent_workspace_changes(main: &str) -> Vec<Changes> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    type Recent = HashMap<String, (Instant, Vec<Changes>)>;
    static RECENT: Mutex<Option<Recent>> = Mutex::new(None);
    const FRESH: Duration = Duration::from_secs(3);
    if let Some((at, changes)) = RECENT.lock().unwrap().get_or_insert_default().get(main) {
        if at.elapsed() < FRESH {
            return changes.clone();
        }
    }
    let (_, changes) = workspace_changes(main);
    RECENT
        .lock()
        .unwrap()
        .get_or_insert_default()
        .insert(main.to_string(), (Instant::now(), changes.clone()));
    changes
}

/// The repository's workspaces as `impulse tasks` shows them, from the
/// point of view of the workspace at `cwd`, agents not yet filled in. Off
/// the main thread; `None` outside a repository.
pub fn build_listing(cwd: &str) -> Option<TaskListing> {
    let caller_root = repo_root(cwd)?;
    let main = crate::tasks::main_checkout_root(&caller_root);
    let common = impulse_git::branches::common_git_directory(&main)?;
    let (tasks, changes) = workspace_changes(&main);
    let caller = task_registry::canonical(&caller_root);
    // The task's own env file (its ports and values) is no overlap.
    let env_file = crate::tasks::project_config(&main)
        .map(|c| c.env_file)
        .unwrap_or_default();
    let caller_files = changes
        .iter()
        .find(|c| c.path == caller)
        .map(|c| c.files.clone())
        .unwrap_or_default();
    let shared = |path: &str| -> Vec<String> {
        if path == caller {
            return Vec::new();
        }
        changes
            .iter()
            .find(|c| c.path == path)
            .map(|c| {
                c.files
                    .intersection(&caller_files)
                    .filter(|f| **f != env_file)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };
    let main_snapshot = impulse_git::snapshot(&main, false);
    let main_path = task_registry::canonical(&main);
    let tracked = main_snapshot.as_ref().is_some_and(|s| s.upstream.is_some());
    let mut workspaces = vec![Workspace {
        name: name_of(&main),
        path: main_path.clone(),
        branch: main_snapshot.as_ref().and_then(|s| s.branch.clone()),
        head: main_snapshot.as_ref().and_then(|s| s.head_oid.clone()),
        is_main_checkout: true,
        is_caller: main_path == caller,
        uncommitted: main_snapshot
            .as_ref()
            .map_or(0, |s| s.changed_file_count() as i64),
        ahead: tracked.then(|| main_snapshot.as_ref().map_or(0, |s| s.ahead as i64)),
        behind: tracked.then(|| main_snapshot.as_ref().map_or(0, |s| s.behind as i64)),
        shared_with_caller: shared(&main_path),
        ..Default::default()
    }];
    for task in &tasks {
        let path = task_registry::canonical(&task.path);
        let branch = format!("refs/heads/{}", task.branch);
        let counts = task
            .base_ref()
            .and_then(|base| impulse_git::operations::ahead_behind(&branch, &base, &main));
        let name = name_of(&task.path);
        let brief = impulse_kit::agents::task_requests::read_brief(&name, &common)
            .and_then(|b| impulse_kit::agents::task_requests::brief_summary(&b, 120));
        workspaces.push(Workspace {
            name,
            path: path.clone(),
            branch: Some(task.branch.clone()),
            base: task.base_ref(),
            head: impulse_git::scoped_diff::resolve_commit(&main, &branch),
            is_main_checkout: false,
            is_caller: path == caller,
            uncommitted: impulse_git::snapshot(&task.path, false)
                .map_or(0, |s| s.changed_file_count() as i64),
            ahead: counts.map(|(ahead, _)| ahead as i64),
            behind: counts.map(|(_, behind)| behind as i64),
            shared_with_caller: shared(&path),
            created_by: task.created_by.clone(),
            requested_by: task.requested_by.clone(),
            brief,
            ..Default::default()
        });
    }
    Some(TaskListing {
        repository: name_of(&main),
        workspaces,
    })
}

/// The listing's agents, from the windows' terminals.
pub fn fill_agents(listing: &mut TaskListing, cx: &App) {
    for workspace in &mut listing.workspaces {
        let agents = agents_in_folder(&workspace.path, cx);
        let agent = agents
            .iter()
            .find(|(_, state)| busy(*state))
            .or(agents.first());
        workspace.agent = agent.map(|(name, _)| name.clone());
        workspace.agent_state = agent.map(|(_, state)| describe(*state).to_string());
    }
}

// MARK: Hook replies

/// "src/page.tsx is also changed in the task interia-upgrade, where
/// Claude Code is working": the text, given the repository's workspaces'
/// changes and the agents in each.
pub fn shared_file_note(
    relative: &str,
    me: &str,
    changes: &[Changes],
    agents: impl Fn(&str) -> Vec<(String, AgentState)>,
) -> Option<String> {
    let others: Vec<&Changes> = changes
        .iter()
        .filter(|c| c.path != me && c.files.contains(relative))
        .collect();
    if others.is_empty() {
        return None;
    }
    let described: Vec<String> = others
        .iter()
        .map(|other| {
            // The main checkout comes first in the list.
            let is_main = changes.first().map(|c| &c.path) == Some(&other.path);
            let mut text = if is_main {
                format!("the main checkout ({})", other.name)
            } else {
                format!("the task {}", other.name)
            };
            if let Some((name, state)) = agents(&other.path).into_iter().find(|(_, s)| busy(*s)) {
                text += &format!(
                    ", where {name} is {}",
                    if state == AgentState::Working {
                        "working"
                    } else {
                        "waiting for input"
                    }
                );
            }
            text
        })
        .collect();
    Some(format!(
        "Impulse: {relative} is also changed in {}. Two workspaces changing the same file will conflict when merged: keep your changes there focused, or check with the user.",
        described.join(" and ")
    ))
}

/// Which dependency files came with a change, and what to run. The
/// project's `[on_change]` commands are named only from settings the user
/// trusts (`rules`): an untrusted file's text mustn't reach the agent as
/// Impulse's advice.
pub fn dependency_note(
    changed: &[String],
    rules: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    let matched = dependency_changes::matches(rules, changed);
    let known = dependency_changes::known_changed(changed);
    if matched.is_empty() && known.is_empty() {
        return None;
    }
    let files: BTreeSet<String> = matched
        .iter()
        .map(|(file, _)| file.clone())
        .chain(known)
        .collect();
    let files = files.into_iter().collect::<Vec<_>>().join(", ");
    if !matched.is_empty() {
        let commands: Vec<&str> = matched.iter().map(|(_, c)| c.as_str()).collect();
        return Some(format!(
            "Impulse: that changed {files}. Bring the dependencies up to date before running tests: {}",
            commands.join(" && ")
        ));
    }
    Some(format!(
        "Impulse: that changed {files}. Reinstall dependencies (or rebuild images) before running tests."
    ))
}

/// The files the agent's own git command changed when it moved HEAD (just
/// now). Off the main thread.
fn dependency_change(cwd: &str) -> Option<(String, Vec<String>)> {
    let root = repo_root(cwd)?;
    let head_move = impulse_git::operations::last_head_move(&root)?;
    let age = SystemTime::now()
        .duration_since(head_move.time)
        .unwrap_or_default();
    if age > Duration::from_secs(120)
        || dependency_changes::move_of(&head_move.subject) == Move::OwnCommit
    {
        return None;
    }
    let changed = impulse_git::operations::changed_paths(&head_move.from, &head_move.to, &root);
    Some((root, changed))
}

/// A task with `branch` checked out, still moving: what to ask before
/// merging it (`agents`: the busy agents in it).
pub(crate) fn merge_question(
    branch: &str,
    path: &str,
    commit: &str,
    uncommitted: usize,
    last_commit: Option<SystemTime>,
    agents: &[(String, AgentState)],
) -> Option<String> {
    if agents.is_empty() && uncommitted == 0 {
        return None;
    }
    let task = name_of(path);
    let mut what = match agents.first() {
        Some((name, state)) => format!(
            "{name} is {} in the {task} task",
            if *state == AgentState::Working {
                "working"
            } else {
                "waiting for input"
            }
        ),
        None => format!(
            "The {task} task has {uncommitted} uncommitted file{}",
            if uncommitted == 1 { "" } else { "s" }
        ),
    };
    if let Some(last) = last_commit {
        what += &format!(", and its last commit was {}", relative_time(last));
    }
    let short: String = commit.chars().take(7).collect();
    Some(format!(
        "{branch} is still being worked on: {what}. Merge {short} anyway? Anything committed to {branch} after it won't be included."
    ))
}

/// "3 minutes ago", as the Swift full-style relative formatter says it.
fn relative_time(date: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(date)
        .unwrap_or_default()
        .as_secs();
    let (count, unit) = match seconds {
        0..=59 => (seconds, "second"),
        60..=3599 => (seconds / 60, "minute"),
        3600..=86_399 => (seconds / 3600, "hour"),
        _ => (seconds / 86_400, "day"),
    };
    format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
}

/// The MCP tools the settings turn off.
pub fn disabled_mcp_tools(settings: &impulse_kit::settings::Settings) -> Vec<&'static str> {
    let mut off = Vec::new();
    if settings.agent_task_creation == "off" {
        off.push("create_task");
    }
    if settings.agent_tool_terminal_output == "off" {
        off.extend(["list_command_blocks", "get_block_output"]);
    }
    off
}

impl Workbench {
    /// The agent commands of the control socket; false for any other.
    pub(crate) fn handle_agent_control(
        &mut self,
        request: &ControlRequest,
        terminal: Option<&Entity<TerminalView>>,
        from_shell: bool,
        answer: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let args = &request.arguments;
        let inside = || ControlResponse::failure("Run this inside an Impulse terminal.");
        let json = args.contains_key("json");
        match request.command.as_str() {
            "notify" => {
                let Some(terminal) = terminal else {
                    answer(inside());
                    return true;
                };
                let message = args.get("message").cloned().unwrap_or_default();
                terminal.update(cx, |t, cx| {
                    t.request_attention(cx);
                    let title = args
                        .get("title")
                        .cloned()
                        .unwrap_or_else(|| t.display_title());
                    t.post_notification(title, message.clone(), false, true, cx);
                });
                let title = args
                    .get("title")
                    .cloned()
                    .unwrap_or_else(|| terminal.read(cx).display_title());
                self.set_notice(
                    if title.is_empty() {
                        message
                    } else {
                        format!("{title}: {message}")
                    },
                    cx,
                );
                answer(ControlResponse::ok());
            }
            "status" => {
                let Some(terminal) = terminal else {
                    answer(inside());
                    return true;
                };
                // Hooks are exact: an MCP status mustn't argue with them.
                if args.get("via").map(String::as_str) == Some("mcp")
                    && terminal.read(cx).agent.reports_by_hook
                {
                    answer(ControlResponse::ok_message(
                        "Impulse's hooks report this agent's state, so set_status changed nothing.",
                    ));
                    return true;
                }
                answer(status(request, terminal, cx));
            }
            "hook" => {
                // Not ours: ignored quietly.
                let Some(terminal) = terminal else {
                    answer(ControlResponse::ok());
                    return true;
                };
                let clash = apply_hook(request, terminal, cx);
                let agent = args.get("agent").map(String::as_str);
                if agent == Some("claude") {
                    self.follow_subagent_worktrees(terminal, cx);
                }
                let event = args.get("event").map(String::as_str).unwrap_or("");
                let tool = args.get("tool").map(String::as_str);
                let Some(reply) = hook_events::reply(agent, event, tool) else {
                    answer(ControlResponse::ok());
                    return true;
                };
                let mut request = request.clone();
                // From outside the pane's shell, the hook's folder isn't
                // where git runs: the pane's is.
                if !from_shell {
                    request.arguments.remove("hookCwd");
                }
                self.agent_hook_reply(reply, request, terminal.clone(), clash, answer, cx);
            }
            "turns" => {
                let Some(terminal) = terminal else {
                    answer(inside());
                    return true;
                };
                let limit = args
                    .get("limit")
                    .and_then(|l| l.parse().ok())
                    .unwrap_or(TurnList::DEFAULT_LIMIT);
                let list = crate::agent_checkpoints::turn_list(&terminal.read(cx).agent.id, limit);
                answer(ControlResponse::answer(&list, list.text(), json));
            }
            "blocks" => {
                if terminal.is_none() {
                    answer(inside());
                    return true;
                }
                if !from_shell {
                    answer(ControlResponse::failure(
                        "impulse blocks only works from the terminal's own shell, not inside tmux or screen.",
                    ));
                    return true;
                }
                answer(match self.command_blocks(request, terminal, cx) {
                    Ok(Ok(list)) => ControlResponse::answer(&list, list.text(), json),
                    Ok(Err(output)) => ControlResponse::answer(&output, output.text(), json),
                    Err(message) => ControlResponse::failure(message),
                });
            }
            // Review of the last agent turn; other scopes are
            // `control_review`'s.
            "review" if args.get("scope").map(String::as_str) == Some("last-turn") => {
                let turn = self.last_agent_turn(terminal, cx);
                if turn.is_none() && args.contains_key("via") {
                    answer(ControlResponse::failure(
                        "No agent turns recorded yet in this repository.",
                    ));
                    return true;
                }
                self.review_last_agent_turn(terminal.cloned(), window, cx);
                answer(ControlResponse::ok());
            }
            "mcp-tools" => {
                let off = disabled_mcp_tools(SettingsStore::get(cx));
                let mut response = ControlResponse::ok();
                response.data = Some(
                    serde_json::to_vec(&serde_json::json!({ "disabled": off })).unwrap_or_default(),
                );
                answer(response);
            }
            _ => return false,
        }
        true
    }

    /// What Impulse answers a Claude Code hook, or an event from its
    /// opencode plugin, with (nothing: an empty ok). `clash`: the note for
    /// an edit made beside a subagent editing the same file, or for one
    /// about to be, why it's stopped.
    fn agent_hook_reply(
        &mut self,
        reply: Reply,
        request: ControlRequest,
        terminal: Entity<TerminalView>,
        clash: Option<String>,
        answer: Answer,
        cx: &mut Context<Self>,
    ) {
        let args = request.arguments;
        let settings = SettingsStore::get(cx).clone();
        let cwd = args
            .get("hookCwd")
            .cloned()
            .unwrap_or_else(|| terminal.read(cx).cwd.clone());
        let opencode = args.get("agent").map(String::as_str) == Some("opencode");
        // Claude Code takes a note as JSON (a SessionStart hook's plain
        // output is context already); the opencode plugin, as its own JSON.
        let context = move |text: &str| {
            if opencode {
                hook_reply::opencode_context(text)
            } else {
                hook_reply::context(text, "PostToolUse")
            }
        };
        let reply_with = move |text: Option<String>| match text {
            Some(text) => answer(ControlResponse::ok_message(text)),
            None => answer(ControlResponse::ok()),
        };
        match reply {
            Reply::SessionSummary if settings.agent_hook_task_summary => {
                let task = cx.background_spawn(async move { build_listing(&cwd) });
                cx.spawn(async move |_, cx| {
                    let listing = task.await;
                    let text = cx.update(|cx| {
                        let mut listing = listing?;
                        fill_agents(&mut listing, cx);
                        listing.session_summary()
                    });
                    reply_with(text.map(|t| if opencode { context(&t) } else { t }));
                })
                .detach();
            }
            Reply::BeforeSubagent if settings.agent_hook_subagents => {
                let kind = args.get("toolSubagentType").cloned();
                let isolated = args.contains_key("toolIsolated");
                let note = terminal.update(cx, |t, _| {
                    t.agent.subagents.launch_note(kind.as_deref(), isolated)
                });
                // opencode's plugin adds it to the call's output.
                reply_with(note.map(|n| {
                    if opencode {
                        hook_reply::opencode_context(&n)
                    } else {
                        hook_reply::context(&n, "PreToolUse")
                    }
                }));
            }
            Reply::SubagentStarted if settings.agent_hook_subagents => {
                let note = args
                    .get("subagent")
                    .and_then(|id| terminal.read(cx).agent.subagents.start_note(id));
                reply_with(note.map(|n| {
                    if opencode {
                        hook_reply::opencode_context(&n)
                    } else {
                        hook_reply::context(&n, "SubagentStart")
                    }
                }));
            }
            Reply::BeforeEdit => {
                let clash = clash.filter(|_| settings.agent_hook_subagents);
                reply_with(clash.map(|n| {
                    if opencode {
                        hook_reply::opencode_deny(&n)
                    } else {
                        hook_reply::deny(&n)
                    }
                }));
            }
            Reply::EditedFile if !settings.agent_hook_shared_files => {
                let clash = clash.filter(|_| settings.agent_hook_subagents);
                reply_with(clash.map(|n| context(&n)));
            }
            Reply::EditedFile => {
                let clash = clash.filter(|_| settings.agent_hook_subagents);
                let Some(file) = args.get("toolFile").cloned() else {
                    return reply_with(clash.map(|n| context(&n)));
                };
                let pane = terminal.read(cx).cwd.clone();
                let task = cx.background_spawn(async move {
                    let dir = Path::new(&file).parent()?.to_string_lossy().into_owned();
                    let root = repo_root(&dir)?;
                    let main = crate::tasks::main_checkout_root(&root);
                    // Only the pane's own repository: a file named anywhere
                    // else mustn't have git run in a repository the user
                    // never opened (its fsmonitor, say).
                    let pane_main = crate::tasks::main_checkout_root(&repo_root(&pane)?);
                    if task_registry::canonical(&pane_main) != task_registry::canonical(&main) {
                        return None;
                    }
                    let changes = recent_workspace_changes(&main);
                    let me = task_registry::canonical(&root);
                    let relative = task_registry::canonical(&file)
                        .strip_prefix(&format!("{me}/"))?
                        .to_string();
                    Some((me, relative, changes))
                });
                cx.spawn(async move |_, cx| {
                    let found = task.await;
                    let text = cx.update(|cx| {
                        let (me, relative, changes) = found?;
                        let note = shared_file_note(&relative, &me, &changes, |path| {
                            agents_in_folder(path, cx)
                        })?;
                        // Once per file per terminal.
                        terminal
                            .update(cx, |t, _| t.agent.shared_file_notes.insert(relative))
                            .then_some(note)
                    });
                    let text = match (clash, text) {
                        (Some(clash), Some(text)) => Some(format!("{clash}\n{text}")),
                        (clash, text) => clash.or(text),
                    };
                    reply_with(text.map(|t| context(&t)));
                })
                .detach();
            }
            Reply::RanShell if settings.agent_hook_dependencies => {
                let moves = args
                    .get("toolCommand")
                    .is_some_and(|c| impulse_kit::git::command_inspector::moves_head(c));
                if !moves {
                    return reply_with(None);
                }
                let task = cx.background_spawn(async move {
                    let (root, changed) = dependency_change(&cwd)?;
                    // Commands go to the agent only from settings the user
                    // trusts.
                    let rules = crate::tasks::already_trusted_project_config(&root)
                        .map(|c| c.on_change)
                        .unwrap_or_default();
                    dependency_note(&changed, &rules)
                });
                cx.spawn(async move |_, _| reply_with(task.await.map(|t| context(&t))))
                    .detach();
            }
            Reply::BeforeShell if settings.agent_hook_merge_guard => {
                let Some(command) = args.get("toolCommand").cloned() else {
                    return reply_with(None);
                };
                let refs = impulse_kit::git::command_inspector::merged_refs(&command);
                if refs.is_empty() {
                    return reply_with(None);
                }
                let task = cx.background_spawn(async move { merge_candidates(&refs, &cwd) });
                cx.spawn(async move |_, cx| {
                    let candidates = task.await;
                    let reason = cx.update(|cx| {
                        candidates.into_iter().find_map(|c| {
                            let agents: Vec<_> = agents_in_folder(&c.path, cx)
                                .into_iter()
                                .filter(|(_, s)| busy(*s))
                                .collect();
                            merge_question(
                                &c.branch,
                                &c.path,
                                &c.commit,
                                c.uncommitted,
                                c.last_commit,
                                &agents,
                            )
                        })
                    });
                    reply_with(reason.map(|reason| {
                        if opencode {
                            hook_reply::opencode_ask(&reason)
                        } else {
                            hook_reply::ask(&reason)
                        }
                    }));
                })
                .detach();
            }
            _ => reply_with(None),
        }
    }

    // MARK: Command blocks

    /// `impulse blocks` (the window's or workspace's terminals and their
    /// latest commands) or `impulse blocks show <id>` (one command's
    /// output): `Ok(Ok)` a list, `Ok(Err)` an output.
    fn command_blocks(
        &self,
        request: &ControlRequest,
        caller: Option<&Entity<TerminalView>>,
        cx: &App,
    ) -> Result<Result<BlockList, BlockOutput>, String> {
        let scope = SettingsStore::get(cx).agent_tool_terminal_output.clone();
        if scope == "off" {
            return Err("Reading other terminals is turned off in Impulse's settings (agent_tool_terminal_output).".into());
        }
        let caller_id = caller.map(|c| c.entity_id());
        let terminals: Vec<Entity<TerminalView>> = if scope == "window" {
            self.workspaces
                .iter()
                .flat_map(|w| {
                    w.all_surfaces()
                        .filter_map(Surface::terminal)
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .collect()
        } else {
            let workspace = self.workspaces.iter().find(|w| {
                w.all_surfaces()
                    .filter_map(Surface::terminal)
                    .any(|t| Some(t.entity_id()) == caller_id)
            });
            let Some(workspace) = workspace else {
                return Err("This terminal isn't in a workspace, so there are no terminals beside it to read (agent_tool_terminal_output is workspace).".into());
            };
            workspace
                .all_surfaces()
                .filter_map(Surface::terminal)
                .cloned()
                .collect()
        };
        if let Some(id) = request.arguments.get("show") {
            let block = BlockList::parse(id).and_then(|(terminal, block)| {
                let terminal = terminals
                    .iter()
                    .find(|t| t.read(cx).agent.short_id() == terminal)?;
                terminal.read(cx).backend.command_block(block)
            });
            let Some(block) = block else {
                return Err(format!(
                    "No command block {id} in this workspace's terminals (list them with list_command_blocks)."
                ));
            };
            let lines = request
                .arguments
                .get("lines")
                .and_then(|l| l.parse().ok())
                .unwrap_or(BlockOutput::DEFAULT_LINES);
            return Ok(Err(BlockOutput::new(
                id,
                block.command,
                block.cwd,
                block.exit_code,
                block.ended_at_ms.is_none(),
                &block.output,
                lines,
            )));
        }
        let list = BlockList {
            terminals: terminals
                .iter()
                .map(|terminal| {
                    let view = terminal.read(cx);
                    let short = view.agent.short_id();
                    let summaries = view.backend.command_block_summaries();
                    let skip = summaries
                        .len()
                        .saturating_sub(BlockList::BLOCKS_PER_TERMINAL);
                    let blocks = summaries
                        .into_iter()
                        .skip(skip)
                        .map(|block| Block {
                            id: format!("{short}:{}", block.id.0),
                            command: block.command,
                            folder: block.cwd,
                            exit_code: block.exit_code,
                            started: UNIX_EPOCH + Duration::from_millis(block.started_at_ms),
                            duration_ms: block
                                .ended_at_ms
                                .map(|end| end.saturating_sub(block.started_at_ms)),
                            running: block.ended_at_ms.is_none(),
                        })
                        .collect();
                    Terminal {
                        id: short,
                        title: view.display_title(),
                        folder: (!view.cwd.is_empty()).then(|| view.cwd.clone()),
                        is_caller: Some(terminal.entity_id()) == caller_id,
                        agent: view.agent.agent.as_ref().map(|a| a.display_name.clone()),
                        blocks,
                    }
                })
                .collect(),
        };
        Ok(Ok(list))
    }
}

/// `impulse status`: the agent's state as the program running it reports.
fn status(
    request: &ControlRequest,
    terminal: &Entity<TerminalView>,
    cx: &mut App,
) -> ControlResponse {
    let args = &request.arguments;
    let state = match args.get("state").map(String::as_str) {
        Some("working") => AgentState::Working,
        Some("waiting") => AgentState::NeedsInput,
        Some("done") => AgentState::Done,
        _ => AgentState::Idle,
    };
    // Each report replaces the message (none clears it).
    let message = args.get("message").filter(|m| !m.is_empty()).cloned();
    let command = terminal.update(cx, |t, cx| {
        t.agent.status_message = message;
        t.agent_hook(AgentHook::Status(state), None, cx);
        cx.emit(crate::agent_tracker::AgentSignal::Changed);
        t.running_command().unwrap_or_default()
    });
    // Typed at a prompt, the report ends with this very command.
    let words: Vec<&str> = command.split(' ').filter(|w| !w.is_empty()).collect();
    let alone = args.get("via").map(String::as_str) != Some("mcp")
        && words.len() >= 2
        && (words[0] == "impulse" || words[0].ends_with("/impulse"))
        && words[1] == "status"
        && !command.contains([';', '&', '|']);
    if alone {
        ControlResponse::ok_message(
            "impulse status lasts only while the program that runs it does, so at a prompt it ends right away. Call it from a script or wrapper running in this pane.",
        )
    } else {
        ControlResponse::ok()
    }
}

/// The subagents a Stop says still run in the background (id, type,
/// description): `None` when it doesn't say.
fn background_subagents(args: &BTreeMap<String, String>) -> Option<Vec<(String, String, String)>> {
    let list: Vec<serde_json::Value> = serde_json::from_str(args.get("background")?).ok()?;
    let field = |task: &serde_json::Value, key: &str| {
        task.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Some(
        list.iter()
            .map(|task| {
                (
                    field(task, "id"),
                    field(task, "agentType"),
                    field(task, "description"),
                )
            })
            .collect(),
    )
}

/// What a hook says about the agent's state and its subagents. Returns the
/// note for an edit made at the same time as another agent's edit of the
/// same file in the checkout, or, before such an edit, why it's stopped.
fn apply_hook(
    request: &ControlRequest,
    terminal: &Entity<TerminalView>,
    cx: &mut App,
) -> Option<String> {
    let args = &request.arguments;
    let agent = args.get("agent").cloned();
    let event = args.get("event").map(String::as_str).unwrap_or("");
    let tool = args.get("tool").map(String::as_str);
    let background = background_subagents(args);
    let hook = hook_events::hook(
        agent.as_deref(),
        event,
        tool,
        args.get("message").map(String::as_str),
    );
    let session = args.get("session").filter(|s| !s.is_empty()).cloned();
    terminal.update(cx, |t, cx| {
        let still_running = background.as_ref().is_some_and(|b| !b.is_empty());
        let clash = matches!(agent.as_deref(), Some("claude" | "opencode"))
            .then(|| {
                follow_subagents(
                    args,
                    agent.as_deref(),
                    event,
                    tool,
                    background,
                    &t.cwd,
                    &mut t.agent.subagents,
                )
            })
            .flatten();
        // Subagents still running, or finished but not yet reported to the
        // agent, each bring it back for another turn.
        let hook = hook.map(|hook| match hook {
            AgentHook::Stopped if still_running || t.agent.subagents.awaiting_reports() > 0 => {
                AgentHook::AwaitingSubagents
            }
            hook => hook,
        });
        if let Some(hook) = hook {
            t.agent_hook(hook, agent.as_deref(), cx);
        }
        t.agent.reports_by_hook = true;
        if let (Some(agent), Some(session)) = (agent, session) {
            t.agent.session = Some((agent, session));
        }
        cx.notify();
        clash
    })
}

/// Claude Code's subagents, from one of its hook events, or opencode's,
/// from its plugin's events (named as Claude Code's,
/// `hook_events::subagent_event`); the note for a clashing edit, or before
/// one, why it's stopped. `pane`: the terminal's folder, which files are
/// named relative to when the hook doesn't say its own.
fn follow_subagents(
    args: &BTreeMap<String, String>,
    agent: Option<&str>,
    event: &str,
    tool: Option<&str>,
    background: Option<Vec<(String, String, String)>>,
    pane: &str,
    subagents: &mut impulse_kit::agents::subagents::Subagents,
) -> Option<String> {
    let now = std::time::Instant::now();
    let arg = |key: &str| args.get(key).map(String::as_str);
    let subagent = arg("subagent");
    let calls_agent = hook_events::starts_subagent(agent, tool);
    let event = hook_events::subagent_event(agent, event);
    match event {
        "SessionStart" => subagents.clear(),
        "UserPromptSubmit" => subagents.new_turn(),
        "Stop" => {
            subagents.new_turn();
            if let Some(running) = background {
                subagents.still_running(&running, now);
            }
        }
        "PreToolUse" if calls_agent => subagents.launching(
            arg("toolDescription"),
            arg("toolSubagentType"),
            args.contains_key("toolIsolated"),
        ),
        "PostToolUse" if calls_agent => {
            if let Some(id) = arg("launched") {
                subagents.launched(
                    id,
                    arg("toolDescription"),
                    args.contains_key("launchedAsync"),
                );
            }
        }
        "SubagentStart" => {
            if let Some(id) = subagent {
                subagents.started(id, arg("subagentType").unwrap_or(""), arg("hookCwd"), now);
            }
        }
        "SubagentStop" => {
            if let Some(id) = subagent {
                subagents.stopped(id, arg("message"), now);
            }
        }
        _ => {}
    }
    // Claude Code's turn that tells the agent, or opencode's plugin seeing
    // the result reach it.
    if let Some(ids) = arg("notified") {
        subagents.reported(&ids.split(',').collect::<Vec<_>>());
    }
    if let (Some(id), Some(tool), "PreToolUse" | "PostToolUse") = (subagent, tool, event) {
        subagents.used_tool(id, tool);
    }
    if let (Some(id), Some(cwd)) = (subagent, arg("hookCwd")) {
        subagents.worked_in(id, cwd, pane);
    }
    if !hook_events::edits_files(agent, tool) {
        return None;
    }
    let file = arg("toolFile")?;
    let folder = arg("hookCwd").unwrap_or(pane);
    let relative = file
        .strip_prefix(&format!("{}/", folder.trim_end_matches('/')))
        .unwrap_or(file);
    match event {
        "PreToolUse" => Some(subagents.before_edit(subagent, file)?.warning(relative)),
        "PostToolUse" => Some(subagents.edited(subagent, file, now)?.note(relative)),
        _ => None,
    }
}

/// A task branch a command would merge.
pub(crate) struct MergeCandidate {
    pub branch: String,
    pub path: String,
    pub commit: String,
    pub uncommitted: usize,
    pub last_commit: Option<SystemTime>,
}

/// The tasks (other than the caller's) whose branches `refs` name, with
/// what they have in progress. Off the main thread.
pub(crate) fn merge_candidates(refs: &[String], cwd: &str) -> Vec<MergeCandidate> {
    let Some(root) = repo_root(cwd) else {
        return Vec::new();
    };
    let Some(registry) = crate::tasks::registry(&root) else {
        return Vec::new();
    };
    let remotes = impulse_git::operations::remotes(&root);
    let me = task_registry::canonical(&root);
    refs.iter()
        .filter_map(|reference| {
            let branch = remotes
                .iter()
                .find_map(|remote| reference.strip_prefix(&format!("{remote}/")))
                .unwrap_or(reference);
            let record = registry.record_for_branch(branch)?;
            if task_registry::canonical(&record.path) == me {
                return None;
            }
            let commit =
                impulse_git::scoped_diff::resolve_commit(&root, &format!("refs/heads/{branch}"))?;
            Some(MergeCandidate {
                branch: branch.to_string(),
                path: record.path.clone(),
                uncommitted: impulse_git::snapshot(&record.path, false)
                    .map_or(0, |s| s.changed_file_count()),
                last_commit: impulse_git::operations::commit_date(&commit, &root),
                commit,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(path: &str, files: &[&str]) -> Changes {
        Changes::new(
            path,
            name_of(path),
            files.iter().map(|f| f.to_string()).collect(),
        )
    }

    #[test]
    fn shared_files_name_the_other_workspace_and_its_agent() {
        let list = vec![
            changes("/r/app", &["src/page.tsx"]),
            changes("/r/app.worktrees/upgrade", &["src/page.tsx", "x"]),
            changes("/r/app.worktrees/mine", &["src/page.tsx"]),
        ];
        let note = shared_file_note("src/page.tsx", "/r/app.worktrees/mine", &list, |path| {
            if path.ends_with("upgrade") {
                vec![("Claude Code".into(), AgentState::Working)]
            } else {
                Vec::new()
            }
        })
        .unwrap();
        assert_eq!(
            note,
            "Impulse: src/page.tsx is also changed in the main checkout (app) and the task upgrade, where Claude Code is working. Two workspaces changing the same file will conflict when merged: keep your changes there focused, or check with the user."
        );
        assert_eq!(
            shared_file_note("x", "/r/app.worktrees/upgrade", &list, |_| Vec::new()),
            None
        );
    }

    #[test]
    fn dependency_notes_name_the_files_and_commands() {
        let changed = vec!["package-lock.json".to_string(), "src/a.ts".to_string()];
        assert_eq!(
            dependency_note(&changed, &Default::default()).as_deref(),
            Some("Impulse: that changed package-lock.json. Reinstall dependencies (or rebuild images) before running tests.")
        );
        let rules = [("package-lock.json".to_string(), "npm ci".to_string())]
            .into_iter()
            .collect();
        assert_eq!(
            dependency_note(&changed, &rules).as_deref(),
            Some("Impulse: that changed package-lock.json. Bring the dependencies up to date before running tests: npm ci")
        );
        assert_eq!(
            dependency_note(&["src/a.ts".into()], &Default::default()),
            None
        );
    }

    #[test]
    fn merge_questions_say_why() {
        assert_eq!(
            merge_question("fix", "/r/app.worktrees/fix", "0123456789", 0, None, &[]),
            None
        );
        assert_eq!(
            merge_question(
                "fix",
                "/r/app.worktrees/fix",
                "0123456789",
                2,
                None,
                &[("Codex".into(), AgentState::NeedsInput)]
            )
            .as_deref(),
            Some("fix is still being worked on: Codex is waiting for input in the fix task. Merge 0123456 anyway? Anything committed to fix after it won't be included.")
        );
        assert_eq!(
            merge_question("fix", "/r/app.worktrees/fix", "abcdef0", 1, None, &[]).as_deref(),
            Some("fix is still being worked on: The fix task has 1 uncommitted file. Merge abcdef0 anyway? Anything committed to fix after it won't be included.")
        );
    }

    #[test]
    fn settings_turn_tools_off() {
        let mut settings = impulse_kit::settings::Settings::default();
        assert!(disabled_mcp_tools(&settings).is_empty());
        settings.agent_tool_terminal_output = "off".into();
        settings.agent_task_creation = "off".into();
        assert_eq!(
            disabled_mcp_tools(&settings),
            vec!["create_task", "list_command_blocks", "get_block_output"]
        );
    }

    #[test]
    fn relative_times_read_in_words() {
        let now = SystemTime::now();
        assert_eq!(relative_time(now - Duration::from_secs(1)), "1 second ago");
        assert_eq!(
            relative_time(now - Duration::from_secs(180)),
            "3 minutes ago"
        );
        assert_eq!(
            relative_time(now - Duration::from_secs(7200)),
            "2 hours ago"
        );
    }
}
