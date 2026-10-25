//! Agents and tasks over the control socket (the Swift `TaskAwareness`'
//! `handleTasksCommand`, `AgentTaskRequests` and the project part of
//! `AgentWorkspaceCommands`): `impulse tasks` lists the repository's
//! workspaces, `show` one in detail and `wait` until its agent stops; `new`,
//! `finish` and `archive` go through the same steps as the user's own New
//! Task, Finish Task and Archive Task, and ask the user first
//! (`agent_task_creation` says when New Task may skip the sheet). Each
//! request waits for the outcome, so the agent hears what happened rather
//! than guessing. `impulse project` answers the project settings the user
//! trusts.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, AsyncApp, Context, Entity, EntityId, Window};
use impulse_kit::agents::known_agents::{self, AgentKind};
use impulse_kit::agents::state_machine::AgentState;
use impulse_kit::agents::task_requests::{self, Decision, Request, Setting};
use impulse_kit::git::task_registry::{self, TaskRecord};
use impulse_protocol::agent_tools::{
    Commit, Detected, ProjectScripts, ProjectSummary, TaskDetail, TaskListing, Workspace,
};
use impulse_protocol::control::{ControlRequest, ControlResponse, BRIEF_LIMIT};
use serde::Serialize;

use crate::agent_hooks::{agents_in_folder, build_listing, fill_agents};
use crate::settings_store::SettingsStore;
use crate::task_finish::{Status, TaskFinishView};
use crate::task_sheet::{AgentRequest, Outcome, TaskSheet};
use crate::tasks::{self, toast, ToastKind};
use crate::terminal_view::TerminalView;
use crate::workbench::Workbench;
use crate::workspace::{Kind, Surface};

type Reply = async_channel::Sender<ControlResponse>;

fn send(reply: &Reply, response: ControlResponse) {
    let _ = reply.try_send(response);
}

fn failure(reply: &Reply, message: impl Into<String>) {
    send(reply, ControlResponse::failure(message));
}

/// Text, with `value` as JSON data for the MCP server.
fn with_data<T: Serialize>(ok: bool, text: String, value: &T) -> ControlResponse {
    ControlResponse {
        ok,
        message: Some(text),
        data: serde_json::to_vec(value).ok(),
    }
}

fn name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn repo_root(path: &str) -> Option<String> {
    impulse_git::branches::repo_root(path).map(|r| r.trim_end_matches('/').to_string())
}

fn busy(state: AgentState) -> bool {
    matches!(state, AgentState::Working | AgentState::NeedsInput)
}

thread_local! {
    /// Windows with an agent's New Task sheet up: one request at a time.
    static ASKING: RefCell<HashSet<EntityId>> = RefCell::new(HashSet::new());
    /// Tasks being created without asking, by repository (main checkout),
    /// which the limit counts before they're in the registry.
    static CREATING: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

fn creating(main: &str, delta: i64) -> usize {
    CREATING.with(|c| {
        let mut c = c.borrow_mut();
        let count = c.entry(main.to_string()).or_default();
        *count = (*count as i64 + delta).max(0) as usize;
        *count
    })
}

/// The agent `id` names (by id or command), or why not.
fn agent_named(id: &str) -> Result<&'static AgentKind, String> {
    known_agents::built_in()
        .iter()
        .find(|k| k.id == id || k.names.iter().any(|n| n == id))
        .ok_or_else(|| {
            let known: Vec<&str> = known_agents::built_in()
                .iter()
                .map(|k| k.id.as_str())
                .collect();
            format!(
                "Impulse doesn't know an agent called {id}. It knows {}.",
                known.join(", ")
            )
        })
}

/// One workspace in detail (`impulse tasks show`, get_task). Off the main
/// thread.
pub fn task_detail(workspace: Workspace, cwd: &str, trusted: bool) -> TaskDetail {
    let mut detail = TaskDetail::new(workspace);
    let workspace = &detail.workspace;
    detail.uncommitted_files =
        tasks::uncommitted(impulse_git::snapshot(&workspace.path, false).as_ref())
            .into_iter()
            .collect();
    if workspace.is_main_checkout {
        return detail;
    }
    let Some(caller_root) = repo_root(cwd) else {
        return detail;
    };
    let main = tasks::main_checkout_root(&caller_root);
    let Some(record) =
        tasks::registry(&main).and_then(|r| r.record_for_path(&detail.workspace.path).cloned())
    else {
        return detail;
    };
    let branch = format!("refs/heads/{}", record.branch);
    if let Some(base) = record.base_ref() {
        use impulse_git::operations;
        if let Some(merge_base) = operations::merge_base(&base, &branch, &main) {
            let mut changed = operations::changed_paths(&merge_base, &branch, &main);
            changed.sort();
            detail.changed_files = changed;
        }
        detail.commits = operations::commits_ahead(&branch, &base, &main, TaskDetail::COMMIT_LIMIT)
            .into_iter()
            .map(|(id, subject)| Commit { id, subject })
            .collect();
        detail.more_commits =
            (detail.workspace.ahead.unwrap_or(0) - detail.commits.len() as i64).max(0);
        // A branch with no commits of its own isn't "merged".
        let head = impulse_git::scoped_diff::resolve_commit(&main, &branch);
        detail.merged = Some(
            head.is_some()
                && head != record.start
                && operations::is_merged(&branch, &base, &main, trusted),
        );
    }
    detail.slot = record.slot;
    if let (Some(slot), Some(config)) = (record.slot, tasks::project_config(&main)) {
        if !config.ports.is_empty() {
            detail.ports = impulse_kit::project::task_environment::ports(
                &config.ports,
                slot,
                config.port_offset,
            )
            .unwrap_or_default();
        }
    }
    if let Some(common) = impulse_git::branches::common_git_directory(&main) {
        detail.brief = task_requests::read_brief(&detail.workspace.name, &common);
    }
    detail
}

/// One step of an agent's Finish, as it hears it.
#[derive(Serialize)]
struct FinishStep {
    step: String,
    status: String,
    text: String,
}

/// Where an agent's Finish stopped, as data.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FinishAnswer {
    task: String,
    steps: Vec<FinishStep>,
    /// The step it stopped at, if it stopped before the end.
    stopped_at: Option<String>,
    waiting_for_user: bool,
    finished: bool,
}

/// Where an agent's Finish stopped, step by step: whether that's an ok
/// answer, its text, and the same as data.
fn finish_answer(
    task: &str,
    already_merged: bool,
    resumes: bool,
    report: Vec<(String, Status, String)>,
) -> (bool, String, FinishAnswer) {
    let mut steps = Vec::new();
    let mut stopped_at = None;
    let mut waiting = false;
    for (title, status, plan) in report {
        let (status, text) = match status {
            Status::Pending => ("pending", plan),
            Status::Running(text) => ("running", text),
            Status::Done(text) => ("done", text),
            Status::Skipped(text) => ("skipped", text),
            Status::Failed(text) => {
                stopped_at = stopped_at.or(Some(title.clone()));
                ("failed", text)
            }
            Status::Waiting(text) => {
                stopped_at = stopped_at.or(Some(title.clone()));
                waiting = true;
                ("waiting for the user", text)
            }
        };
        steps.push(FinishStep {
            step: title,
            status: status.into(),
            text,
        });
    }
    let finished = stopped_at.is_none() && !resumes;
    let lines: Vec<String> = steps
        .iter()
        .map(|s| format!("{}: {}. {}", s.step, s.status, s.text))
        .collect();
    let mut text = lines.join("\n");
    if waiting {
        text = format!(
            "{}\n\n{text}",
            if already_merged {
                "Already merged; the user cleans it up from the Finish tab."
            } else {
                "Ready to land; the user lands it from the Finish tab."
            }
        );
    } else if let Some(step) = &stopped_at {
        text =
            format!("Finish stopped at {step}. Fix that, then call finish_task again.\n\n{text}");
    } else {
        text = format!("Finished {task}.\n\n{text}");
    }
    let ok = stopped_at.is_none() || waiting;
    (
        ok,
        text,
        FinishAnswer {
            task: task.into(),
            steps,
            stopped_at,
            waiting_for_user: waiting,
            finished,
        },
    )
}

/// For agents whose command takes no first prompt: type the brief in (and
/// submit it) once the agent is up in `terminal`, after the setup script
/// has run, giving up after ten minutes.
pub(crate) fn type_brief_when_agent_starts(
    brief: String,
    terminal: Entity<TerminalView>,
    cx: &mut App,
) {
    let terminal = terminal.downgrade();
    cx.spawn(async move |cx: &mut AsyncApp| {
        for _ in 0..600 {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let ready = terminal.update(cx, |t, _| {
                t.agent.agent.is_some()
                    && matches!(t.agent.state(), Some(AgentState::Idle | AgentState::Done))
            });
            match ready {
                Err(_) => return,
                Ok(false) => continue,
                Ok(true) => {}
            }
            // Its prompt may still be drawing. Only into the agent: if it
            // quit meanwhile, the brief mustn't run as a shell command.
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = terminal.update(cx, |t, cx| {
                if t.agent.agent.is_some() {
                    t.paste_into_program(&brief, true, cx);
                }
            });
            return;
        }
    })
    .detach();
}

impl Workbench {
    /// Why the main checkout's files can't move now: an agent there is
    /// mid-turn.
    pub(crate) fn move_blocker(&self, root: &str, cx: &Context<Self>) -> Option<String> {
        let agent = self.working_agents_in(root, None, cx).into_iter().next()?;
        Some(format!(
            "{agent} is working in {}, so its files can't move until it finishes its turn.",
            name_of(root)
        ))
    }

    /// `impulse tasks` and `impulse project`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_tasks_control(
        &mut self,
        request: &ControlRequest,
        terminal: Option<Entity<TerminalView>>,
        cwd: Option<String>,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let inside = "Run this inside an Impulse terminal.";
        let Some(cwd) = cwd else {
            return failure(&reply, inside);
        };
        let args = &request.arguments;
        if request.command == "project" {
            if terminal.is_none() {
                return failure(&reply, inside);
            }
            return self.project_summary(cwd, args.contains_key("json"), reply, cx);
        }
        if let Some(name) = args.get("wait").cloned() {
            let timeout = args.get("timeout").and_then(|t| t.parse::<f64>().ok());
            return self.wait_for_task(name, cwd, timeout, reply, cx);
        }
        if let Some(name) = args.get("show").cloned() {
            return self.show_task(name, cwd, args.contains_key("json"), reply, window, cx);
        }
        if args.contains_key("new") {
            return self.request_task_from_agent(request, cwd, terminal, reply, window, cx);
        }
        if let Some(name) = args.get("finish").cloned() {
            return self.finish_task_from_agent(name, cwd, terminal, reply, window, cx);
        }
        if let Some(name) = args.get("archive").cloned() {
            return self.archive_task_from_agent(name, cwd, reply, window, cx);
        }
        let json = args.contains_key("json");
        self.task_listing(cwd, window, cx, move |_, listing, _, _| match listing {
            Some(listing) => send(
                &reply,
                ControlResponse::answer(&listing, listing.text(), json),
            ),
            None => failure(&reply, "Not in a git repository."),
        });
    }

    /// The repository's workspaces as `impulse tasks` shows them, from the
    /// point of view of the workspace at `cwd`: worked out off the main
    /// thread, with the agents filled in on it.
    fn task_listing(
        &mut self,
        cwd: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Workbench, Option<TaskListing>, &mut Window, &mut Context<Workbench>)
            + 'static,
    ) {
        let work = cx.background_spawn(async move { build_listing(&cwd) });
        cx.spawn_in(window, async move |workbench, cx| {
            let mut listing = work.await;
            // Every window's agents, outside any window's update.
            if let Some(listing) = listing.as_mut() {
                let _ = cx.update(|_, cx| fill_agents(listing, cx));
            }
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                then(workbench, listing, window, cx)
            });
        })
        .detach();
    }

    /// `impulse tasks show <task>` (get_task).
    fn show_task(
        &mut self,
        name: String,
        cwd: String,
        json: bool,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let caller = cwd.clone();
        self.task_listing(cwd, window, cx, move |workbench, listing, _, cx| {
            let Some(listing) = listing else {
                return failure(&reply, "Not in a git repository.");
            };
            let Some(workspace) = listing
                .workspaces
                .into_iter()
                .find(|w| w.name == name || w.branch.as_deref() == Some(name.as_str()))
            else {
                return failure(&reply, format!("impulse tasks: no task named {name}"));
            };
            // Git reads in a folder that isn't trusted run no hooks or filters.
            let trusted = workbench.trust.is_trusted(&workspace.path);
            cx.background_spawn(async move {
                let detail = task_detail(workspace, &caller, trusted);
                send(
                    &reply,
                    ControlResponse::answer(&detail, detail.text(), json),
                );
            })
            .detach();
        });
    }

    /// Answer once the task's agents stop working (checked every 2
    /// seconds), or after `timeout` seconds (wait_for_task), or stop
    /// checking when the caller gives up.
    fn wait_for_task(
        &mut self,
        name: String,
        cwd: String,
        timeout: Option<f64>,
        reply: Reply,
        cx: &mut Context<Self>,
    ) {
        #[derive(Serialize)]
        struct WaitAnswer {
            task: String,
            /// False when it gave up waiting (its timeout).
            stopped: bool,
        }
        let lookup = {
            let name = name.clone();
            cx.background_spawn(async move {
                repo_root(&cwd)
                    .and_then(|root| tasks::registry(&root))
                    .and_then(|registry| registry.record_named(&name).cloned())
            })
        };
        cx.spawn(async move |_, cx| {
            let Some(task) = lookup.await else {
                return failure(&reply, format!("impulse tasks: no task named {name}"));
            };
            let answer = |stopped: bool, text: String| {
                let value = WaitAnswer {
                    task: name.clone(),
                    stopped,
                };
                send(&reply, with_data(true, text, &value));
            };
            let busy_now = |cx: &mut AsyncApp| {
                cx.update(|cx| agents_in_folder(&task.path, cx).iter().any(|(_, s)| busy(*s)))
            };
            if !busy_now(cx) {
                return answer(true, format!("{name}: no agent is working."));
            }
            let started = Instant::now();
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if reply.is_closed() {
                    return;
                }
                if !busy_now(cx) {
                    return answer(true, format!("{name}: its agent stopped working."));
                }
                if let Some(timeout) = timeout {
                    if started.elapsed().as_secs_f64() >= timeout {
                        return answer(
                            false,
                            format!(
                                "{name}: its agent is still working after {} seconds. Wait again, or check on it with get_task.",
                                timeout as i64
                            ),
                        );
                    }
                }
            }
        })
        .detach();
    }

    // MARK: New Task

    /// `impulse tasks new` / create_task.
    fn request_task_from_agent(
        &mut self,
        request: &ControlRequest,
        cwd: String,
        terminal: Option<Entity<TerminalView>>,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let args = &request.arguments;
        let title = args
            .get("new")
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
        let brief = args.get("brief").cloned().unwrap_or_default();
        if title.is_empty() {
            return failure(&reply, "A task needs a title.");
        }
        if brief.len() > BRIEF_LIMIT {
            return failure(&reply, "The brief holds more than 16 KB.");
        }
        let agent = match args
            .get("agent")
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
        {
            Some(id) => match agent_named(id) {
                Ok(kind) => Some(kind),
                Err(message) => return failure(&reply, message),
            },
            None => None,
        };
        let base = args.get("base").cloned().filter(|b| !b.is_empty());
        let moving = args.contains_key("move");
        let caller_workspace = terminal.as_ref().and_then(|t| {
            let id = t.entity_id();
            self.workspaces.iter().position(|w| {
                w.all_surfaces()
                    .filter_map(Surface::terminal)
                    .any(|s| s.entity_id() == id)
            })
        });
        let caller_workspace_root =
            caller_workspace.map(|i| self.workspaces[i].root.to_string_lossy().into_owned());
        let agent_name = terminal
            .as_ref()
            .and_then(|t| {
                t.read(cx)
                    .agent
                    .agent
                    .as_ref()
                    .map(|a| a.display_name.clone())
            })
            .unwrap_or_else(|| "An agent".into());
        struct Looked {
            caller_root: String,
            main: String,
            from_agent_task: bool,
            installed: bool,
            has_settings: bool,
            agent_tasks: usize,
        }
        let caller_cwd = cwd.clone();
        let work = cx.background_spawn(async move {
            let caller_root = repo_root(&cwd)?;
            let main = tasks::main_checkout_root(&caller_root);
            let registry = tasks::registry(&main);
            // From inside an agent's task: where the request says it runs,
            // or the caller's workspace (a `cd` doesn't leave it).
            let is_agents = |path: &str| {
                registry
                    .as_ref()
                    .and_then(|r| r.record_for_path(path))
                    .is_some_and(TaskRecord::is_from_agent)
            };
            let from_agent_task =
                is_agents(&caller_root) || caller_workspace_root.as_deref().is_some_and(is_agents);
            let installed = agent.is_none_or(|kind| {
                kind.names
                    .first()
                    .and_then(|n| impulse_kit::shell::login::which(n))
                    .is_some()
            });
            Some(Looked {
                has_settings: tasks::load_project_config(&main).is_some(),
                agent_tasks: registry.as_ref().map_or(0, |r| r.agent_task_count()),
                caller_root,
                main,
                from_agent_task,
                installed,
            })
        });
        let terminal_for_attention = terminal.clone();
        cx.spawn_in(window, async move |workbench, cx| {
            let Some(looked) = work.await else {
                return failure(&reply, "Not in a git repository.");
            };
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                if let (Some(agent), false) = (agent, looked.installed) {
                    return failure(
                        &reply,
                        format!("{} isn't installed here (its command isn't on PATH).", agent.display_name),
                    );
                }
                let main = looked.main.clone();
                let place = caller_workspace
                    .and_then(|i| workbench.workspaces.get(i))
                    .map(|w| w.name())
                    .unwrap_or_else(|| name_of(&looked.caller_root));
                let is_main =
                    task_registry::canonical(&looked.caller_root) == task_registry::canonical(&main);
                let requester = AgentRequest {
                    requested_by: format!(
                        "{agent_name} in {place}{}",
                        if is_main { " (main checkout)" } else { "" }
                    ),
                    from_path: looked.caller_root.clone(),
                };
                let settings = SettingsStore::get(cx).clone();
                let project_trusted =
                    !looked.has_settings || tasks::already_trusted_project_config(&main).is_some();
                let repository = task_registry::canonical(&main);
                let decision = task_requests::decide(
                    Setting::from_setting(&settings.agent_task_creation),
                    settings.agent_task_limit.max(0) as usize,
                    Request {
                        repository_trusted: workbench.trust.is_trusted(&main),
                        project_trusted,
                        agent_tasks: looked.agent_tasks + creating(&repository, 0),
                        from_agent_task: looked.from_agent_task,
                        moves_changes: moving,
                    },
                );
                let command = agent.and_then(|a| a.names.first().cloned()).unwrap_or_default();
                let (outcome, heard) = async_channel::bounded::<Outcome>(1);
                match decision {
                    Decision::Refuse(reason) => failure(&reply, reason),
                    Decision::Ask => {
                        // The sheet branches from the repository the request
                        // names: the caller's workspace when it's that one,
                        // else one open on it.
                        let in_repository = |w: &crate::workspace::Workspace| {
                            w.kind == Kind::Folder
                                && repo_root(&w.root.to_string_lossy())
                                    .map(|r| task_registry::canonical(&tasks::main_checkout_root(&r)))
                                    .as_deref()
                                    == Some(repository.as_str())
                        };
                        let index = caller_workspace
                            .filter(|i| in_repository(&workbench.workspaces[*i]))
                            .or_else(|| workbench.workspaces.iter().position(in_repository));
                        let Some(index) = index else {
                            return failure(
                                &reply,
                                format!(
                                    "{} isn't open in this Impulse window, so a task can't be started there. Ask the user to open it.",
                                    name_of(&main)
                                ),
                            );
                        };
                        let me = cx.entity().entity_id();
                        if !ASKING.with(|a| a.borrow_mut().insert(me)) {
                            return failure(
                                &reply,
                                "Another task request is waiting for the user. Try again once they've answered it.",
                            );
                        }
                        // The question is in this window: make sure it's seen.
                        if let Some(terminal) = &terminal_for_attention {
                            terminal.update(cx, |t, cx| t.request_attention(cx));
                        }
                        if !crate::agent_notify::app_in_front(cx) {
                            window.request_attention();
                        }
                        workbench.present_agent_task(
                            index, &title, &command, base.as_deref(), moving, requester, &brief, outcome,
                            window, cx,
                        );
                        cx.spawn_in(window, async move |workbench, cx| {
                            let heard = heard.recv().await.ok();
                            ASKING.with(|a| a.borrow_mut().remove(&me));
                            let answered = workbench.update_in(cx, |w, window, cx| {
                                w.reply_with_outcome(heard, true, caller_cwd, reply.clone(), window, cx)
                            });
                            if answered.is_err() {
                                failure(&reply, "The window closed.");
                            }
                        })
                        .detach();
                    }
                    Decision::Create => {
                        creating(&repository, 1);
                        workbench.create_task_without_asking(
                            &main, &title, &command, base.as_deref(), requester, &brief, outcome, window,
                            cx,
                        );
                        cx.spawn_in(window, async move |workbench, cx| {
                            let heard = heard.recv().await.ok();
                            creating(&repository, -1);
                            let answered = workbench.update_in(cx, |w, window, cx| {
                                w.reply_with_outcome(heard, false, caller_cwd, reply.clone(), window, cx)
                            });
                            if answered.is_err() {
                                failure(&reply, "The window closed.");
                            }
                        })
                        .detach();
                    }
                }
            });
        })
        .detach();
    }

    /// How an agent's request ended, as its answer: the new task as
    /// `impulse tasks show` describes it, seen from the caller's workspace.
    /// No outcome is the user's no only when they were `asked`.
    #[allow(clippy::too_many_arguments)]
    fn reply_with_outcome(
        &mut self,
        heard: Option<Outcome>,
        asked: bool,
        cwd: String,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (path, branch) = match heard {
            None if asked => return failure(&reply, "The user declined: no task was created."),
            None => return failure(&reply, "The task wasn't created."),
            Some(Outcome::Failed(message)) => {
                return failure(&reply, format!("The task wasn't created: {message}"))
            }
            Some(Outcome::Created { path, branch }) => (path, branch),
        };
        let wanted = task_registry::canonical(&path);
        let caller = cwd.clone();
        self.task_listing(cwd, window, cx, move |workbench, listing, _, cx| {
            let workspace =
                listing.and_then(|l| l.workspaces.into_iter().find(|w| w.path == wanted));
            let Some(workspace) = workspace else {
                return send(
                    &reply,
                    ControlResponse::ok_message(format!("Started task {branch} in {path}.")),
                );
            };
            let trusted = workbench.trust.is_trusted(&path);
            cx.background_spawn(async move {
                let detail = task_detail(workspace, &caller, trusted);
                let text = format!("Started task {branch}.\n\n{}", detail.text());
                send(&reply, with_data(true, text, &detail));
            })
            .detach();
        });
    }

    /// The New Task sheet for an agent's request, prefilled with who asked
    /// and the brief, from the repository of the workspace at `index`.
    #[allow(clippy::too_many_arguments)]
    fn present_agent_task(
        &mut self,
        index: usize,
        title: &str,
        command: &str,
        base: Option<&str>,
        moving: bool,
        requester: AgentRequest,
        brief: &str,
        outcome: async_channel::Sender<Outcome>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.task_repository(Some(index)) else {
            // The user was never asked.
            let _ = outcome.try_send(Outcome::Failed(
                "the workspace is no longer a repository's".into(),
            ));
            return;
        };
        let blocker = self.move_blocker(&root, cx);
        let (palette, workbench, trusted) = (
            self.palette.clone(),
            cx.entity().downgrade(),
            self.trust.is_trusted(&root),
        );
        let sheet = cx.new(|cx| {
            let mut sheet = TaskSheet::new(root, moving, palette, workbench, trusted, window, cx);
            sheet.agent_request = Some(requester);
            sheet.command = command.to_string();
            sheet.outcome = Some(outcome);
            sheet.move_blocker = blocker;
            sheet.prefill(title, base, brief, window, cx);
            sheet
        });
        self.show_task_sheet(sheet, window, cx);
    }

    /// `agent_task_creation = trusted_projects`: what New Task would make,
    /// without the sheet, then a toast with Undo.
    #[allow(clippy::too_many_arguments)]
    fn create_task_without_asking(
        &mut self,
        root: &str,
        title: &str,
        command: &str,
        base: Option<&str>,
        requester: AgentRequest,
        brief: &str,
        outcome: async_channel::Sender<Outcome>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (palette, workbench) = (self.palette.clone(), cx.entity().downgrade());
        let sheet = cx.new(|cx| {
            // It fetches nothing on its own: it isn't waited for.
            let mut sheet = TaskSheet::new(
                root.to_string(),
                false,
                palette,
                workbench,
                false,
                window,
                cx,
            );
            sheet.agent_request = Some(requester);
            sheet.command = command.to_string();
            sheet.outcome = Some(outcome);
            sheet.without_asking = true;
            sheet.prefill(title, base, brief, window, cx);
            sheet
        });
        // Once the branches and copies are looked up.
        cx.spawn_in(window, async move |workbench, cx| {
            for _ in 0..600 {
                if sheet.read_with(cx, |s, _| !s.is_loading()) {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                sheet.update(cx, |s, _| s.is_creating = true);
                workbench.create_task(sheet, window, cx)
            });
        })
        .detach();
    }

    /// The toast after a task was made without asking: who started it,
    /// with Undo.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn announce_agent_task(
        &mut self,
        who: String,
        root: &str,
        path: &str,
        branch: &str,
        start: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let undo: tasks::Run = {
            let (workbench, root, path, branch) = (
                cx.entity().downgrade(),
                root.to_string(),
                path.to_string(),
                branch.to_string(),
            );
            Rc::new(move |window, cx| {
                let _ = workbench.update(cx, |w, cx| {
                    w.undo_agent_task(&path, &branch, &root, start.clone(), window, cx)
                });
            })
        };
        toast(
            window,
            cx,
            ToastKind::Success,
            format!("{who} started task {branch}."),
            None,
            Some(("Undo", undo)),
        );
    }

    /// Undo of a task created without asking: close its workspace, archive
    /// its folder (whatever it holds is kept in a snapshot), and delete its
    /// branch when nothing was committed on it.
    fn undo_agent_task(
        &mut self,
        path: &str,
        branch: &str,
        root: &str,
        start: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.workspace_at(path) {
            let id = self.workspaces[index].id;
            if !self.close_workspace_for_task(id, window, cx) {
                return;
            }
        }
        let trust = self.trust.clone();
        let work = {
            let (path, branch, root) = (path.to_string(), branch.to_string(), root.to_string());
            cx.background_spawn(async move {
                let removed = crate::task_cleanup::remove_task(&path, &branch, true, None, &trust);
                // Only while it's still where it started: nothing was committed.
                let deleted = removed.result.is_ok()
                    && start.is_some_and(|start| {
                        impulse_git::operations::delete_branch_at(&branch, &start, &root).is_ok()
                    });
                (removed.result, deleted)
            })
        };
        let branch = branch.to_string();
        cx.spawn_in(window, async move |workbench, cx| {
            let (result, deleted) = work.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                workbench.refresh_task_states(cx);
                match result {
                    Err(error) => toast(
                        window,
                        cx,
                        ToastKind::Warning,
                        format!("Couldn't undo {branch}"),
                        Some(error),
                        None,
                    ),
                    Ok(_) if deleted => toast(
                        window,
                        cx,
                        ToastKind::Success,
                        format!("Removed task {branch}."),
                        None,
                        None,
                    ),
                    Ok(archived) => {
                        let restore = workbench.restore_action(vec![archived], cx);
                        toast(
                            window,
                            cx,
                            ToastKind::Success,
                            format!("Archived {branch}. The branch is kept."),
                            None,
                            Some(("Undo", restore)),
                        )
                    }
                }
            });
        })
        .detach();
    }

    // MARK: Finish and archive

    /// The task `name` (folder or branch) and its workspace in this
    /// window, opened if it isn't.
    fn agent_task_workspace(
        &mut self,
        name: String,
        cwd: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Workbench, Result<usize, String>, &mut Window, &mut Context<Workbench>)
            + 'static,
    ) {
        let lookup = {
            let name = name.clone();
            cx.background_spawn(async move {
                repo_root(&cwd)
                    .and_then(|root| tasks::registry(&root))
                    .and_then(|registry| registry.record_named(&name).cloned())
            })
        };
        cx.spawn_in(window, async move |workbench, cx| {
            let record = lookup.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                let Some(record) = record else {
                    return then(
                        workbench,
                        Err(format!("impulse tasks: no task named {name}")),
                        window,
                        cx,
                    );
                };
                if let Some(index) = workbench.workspace_at(&record.path) {
                    return then(workbench, Ok(index), window, cx);
                }
                if !Path::new(&record.path).exists() {
                    return then(
                        workbench,
                        Err(format!("{name}'s folder is gone.")),
                        window,
                        cx,
                    );
                }
                let index =
                    workbench.open_task_workspace(Path::new(&record.path), None, window, cx);
                then(workbench, Ok(index), window, cx)
            });
        })
        .detach();
    }

    /// `impulse tasks finish` / finish_task: Finish runs up to landing,
    /// then waits for the user. Answers when the run stops.
    fn finish_task_from_agent(
        &mut self,
        name: String,
        cwd: String,
        terminal: Option<Entity<TerminalView>>,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requester = terminal
            .as_ref()
            .and_then(|t| {
                t.read(cx)
                    .agent
                    .agent
                    .as_ref()
                    .map(|a| a.display_name.clone())
            })
            .unwrap_or_else(|| "An agent".into());
        let requester_terminal = terminal.as_ref().map(|t| t.entity_id());
        self.agent_task_workspace(
            name.clone(),
            cwd,
            window,
            cx,
            move |workbench, found, window, cx| {
                let index = match found {
                    Ok(index) => index,
                    Err(message) => return failure(&reply, message),
                };
                let root = workbench.workspaces[index]
                    .root
                    .to_string_lossy()
                    .into_owned();
                workbench.open_finish_task(Some(index), window, cx);
                let view = workbench.workspaces.get(index).and_then(|w| {
                    w.all_surfaces().find_map(|s| match s {
                        Surface::TaskFinish(view) if view.read(cx).root == root => {
                            Some(view.clone())
                        }
                        _ => None,
                    })
                });
                let Some(view) = view else {
                    return failure(&reply, format!("Finish Task couldn't open for {name}."));
                };
                if !crate::agent_notify::app_in_front(cx) {
                    window.request_attention();
                }
                // A run you started yourself goes on as you started it; the
                // agent hears where it stops.
                view.update(cx, |v, _| {
                    if !v.is_running {
                        v.stops_before_landing = Some(requester);
                        v.requester_terminal = requester_terminal;
                    }
                });
                workbench.answer_when_finish_stops(view, name, reply, window, cx);
            },
        );
    }

    /// Start the Finish once it has loaded (unless it runs already), then
    /// answer where it stops.
    fn answer_when_finish_stops(
        &mut self,
        view: Entity<TaskFinishView>,
        name: String,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = view.downgrade();
        cx.spawn_in(window, async move |workbench, cx| {
            let closed = || {
                ControlResponse::failure(format!(
                    "The Finish tab for {name} was closed before it stopped."
                ))
            };
            loop {
                match view.read_with(cx, |v, _| v.is_loading()) {
                    Err(_) => return send(&reply, closed()),
                    Ok(true) => {
                        cx.background_executor()
                            .timer(Duration::from_millis(200))
                            .await
                    }
                    Ok(false) => break,
                }
            }
            let Some(entity) = view.upgrade() else {
                return send(&reply, closed());
            };
            let problem = workbench.update_in(cx, |workbench, window, cx| {
                let (unavailable, running, landing, merged, main) = {
                    let v = entity.read(cx);
                    (
                        v.unavailable(),
                        v.is_running,
                        v.has_landing(),
                        v.is_already_merged(),
                        v.main_name_text(),
                    )
                };
                let problem = unavailable.or_else(|| {
                    (!running && !landing && !merged).then(|| {
                        format!(
                            "This is the first Finish in {main}, so the user has to choose how work lands there. The Finish tab is open for them."
                        )
                    })
                });
                if problem.is_some() {
                    entity.update(cx, |v, _| {
                        if !v.is_running {
                            v.stops_before_landing = None;
                            v.requester_terminal = None;
                        }
                    });
                } else if !running {
                    workbench.start_finish(entity.clone(), window, cx);
                }
                problem
            });
            match problem {
                Err(_) => return send(&reply, closed()),
                Ok(Some(problem)) => return failure(&reply, problem),
                Ok(None) => {}
            }
            loop {
                if reply.is_closed() {
                    return;
                }
                let stopped = view.read_with(cx, |v, _| {
                    (!v.is_running).then(|| {
                        finish_answer(&v.task_name(), v.is_already_merged(), v.resumes(), v.step_report())
                    })
                });
                match stopped {
                    Err(_) => return send(&reply, closed()),
                    Ok(Some((ok, text, answer))) => return send(&reply, with_data(ok, text, &answer)),
                    Ok(None) => {
                        cx.background_executor()
                            .timer(Duration::from_millis(300))
                            .await
                    }
                }
            }
        })
        .detach();
    }

    /// `impulse tasks archive` / archive_task: Archive Task, with its
    /// questions and Undo. Answers once it's archived, or the user said no.
    fn archive_task_from_agent(
        &mut self,
        name: String,
        cwd: String,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agent_task_workspace(name.clone(), cwd, window, cx, move |workbench, found, window, cx| {
            let index = match found {
                Ok(index) => index,
                Err(message) => return failure(&reply, message),
            };
            let id = workbench.workspaces[index].id;
            let root = workbench.workspaces[index].root.clone();
            if !crate::agent_notify::app_in_front(cx) {
                window.request_attention();
            }
            workbench.archive_task(index, window, cx);
            // Archive Task took it on (or said why not, in the window).
            if !crate::task_cleanup::is_archiving(id) {
                return failure(
                    &reply,
                    format!("{name} can't be archived now (Impulse says why in its window)."),
                );
            }
            // It asks first; once it's past the questions the workspace
            // closes and the folder goes once the archive script (up to two
            // minutes) has run.
            cx.spawn(async move |workbench, cx| {
                let mut closed_at: Option<Instant> = None;
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                    if reply.is_closed() {
                        return;
                    }
                    if !root.exists() {
                        return send(
                            &reply,
                            ControlResponse::ok_message(format!(
                                "Archived {name}. Its branch is kept, and the user can Undo."
                            )),
                        );
                    }
                    let open = workbench.read_with(cx, |w, _| w.workspaces.iter().any(|w| w.id == id));
                    let Ok(open) = open else {
                        return failure(&reply, format!("The window closed before {name} was archived."));
                    };
                    if crate::task_cleanup::is_archiving(id) {
                        continue;
                    }
                    if open {
                        // Declined, or stopped (a repository inside it, say).
                        return failure(
                            &reply,
                            format!(
                                "{name} wasn't archived: the user declined, or Impulse said why in its window."
                            ),
                        );
                    }
                    let since = *closed_at.get_or_insert_with(Instant::now);
                    if since.elapsed() > Duration::from_secs(300) {
                        return failure(
                            &reply,
                            format!(
                                "{name}'s workspace closed, but its folder is still there (Impulse says why in its window)."
                            ),
                        );
                    }
                }
            })
            .detach();
        });
    }

    // MARK: Project settings

    /// `impulse project` / get_project_config: the settings the user
    /// trusts, and what Impulse found in the repository itself.
    fn project_summary(&mut self, cwd: String, json: bool, reply: Reply, cx: &mut Context<Self>) {
        let work = cx.background_spawn(async move {
            let root = repo_root(&cwd).unwrap_or(cwd);
            let main = tasks::main_checkout_root(&root);
            let loaded = tasks::load_project_config(&root);
            let slot = tasks::record_for_path(&root).and_then(|r| r.slot);
            let exists = |name: &str| Path::new(&main).join(name).exists();
            let installs: Vec<(&str, &str)> = impulse_kit::project::detector::INSTALL_COMMANDS
                .iter()
                .filter(|(lock, _)| exists(lock))
                .copied()
                .collect();
            let detected = Detected {
                lock_files: installs.iter().map(|(lock, _)| lock.to_string()).collect(),
                install: installs
                    .iter()
                    .map(|(_, command)| command.to_string())
                    .collect(),
                compose_file: impulse_kit::project::compose_file::FILE_NAMES
                    .iter()
                    .find(|name| exists(name))
                    .map(|name| name.to_string()),
            };
            let trusted = tasks::already_trusted_project_config(&root).is_some();
            (root, main, loaded, slot, detected, trusted)
        });
        cx.spawn(async move |_, _| {
            let (_, main, loaded, slot, detected, trusted) = work.await;
            let relative = |path: &str| {
                path.strip_prefix(&format!("{main}/"))
                    .unwrap_or(path)
                    .to_string()
            };
            let summary = match loaded {
                None => ProjectSummary::new(
                    None,
                    true,
                    Vec::new(),
                    slot,
                    detected,
                    Some("No Impulse project settings (.impulse/project.toml) in this repository.".into()),
                ),
                Some(loaded) => {
                    let files: Vec<String> = loaded.sources().iter().map(|s| relative(&s.path)).collect();
                    match &loaded.config {
                        Err(_) => ProjectSummary::new(
                            None,
                            false,
                            files,
                            slot,
                            detected,
                            Some("The project settings don't parse, so nothing from them is listed.".into()),
                        ),
                        Ok(config) => {
                            let scripts = ProjectScripts {
                                check_script: config.check_script.clone(),
                                setup_script: config.setup_script.clone(),
                                archive_script: config.archive_script.clone(),
                                actions: config
                                    .actions
                                    .iter()
                                    .map(|a| (a.name.clone(), a.command.clone()))
                                    .collect(),
                                on_change: config.on_change.clone(),
                                env_file: config.env_file.clone(),
                                ports: config.ports.clone(),
                                port_offset: config.port_offset,
                            };
                            ProjectSummary::new(
                                Some(&scripts),
                                trusted,
                                files,
                                slot,
                                detected,
                                (!trusted).then(|| {
                                    "The user hasn't trusted these project settings yet, so their commands aren't listed: only what Impulse found itself.".to_string()
                                }),
                            )
                        }
                    }
                }
            };
            send(&reply, ControlResponse::answer(&summary, summary.text(), json));
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_are_named_by_id_or_command() {
        assert_eq!(agent_named("claude").map(|k| k.id.as_str()), Ok("claude"));
        let unknown = agent_named("hal").unwrap_err();
        assert!(unknown.starts_with("Impulse doesn't know an agent called hal. It knows claude, "));
    }

    #[test]
    fn a_finish_that_waits_says_the_user_lands_it() {
        let report = vec![
            (
                "Commit".to_string(),
                Status::Done("Everything is committed.".into()),
                String::new(),
            ),
            (
                "Merge into main".to_string(),
                Status::Waiting(
                    "Claude Code asked to finish fix. Click Continue to land it in main.".into(),
                ),
                String::new(),
            ),
            (
                "Clean up".to_string(),
                Status::Pending,
                "Archives the task.".into(),
            ),
        ];
        let (ok, text, answer) = finish_answer("fix", false, true, report);
        assert!(ok);
        assert!(text.starts_with(
            "Ready to land; the user lands it from the Finish tab.\n\nCommit: done. Everything is committed.\n"
        ));
        assert!(text.ends_with("Clean up: pending. Archives the task."));
        assert_eq!(answer.stopped_at.as_deref(), Some("Merge into main"));
        assert!(answer.waiting_for_user && !answer.finished);
        let json = serde_json::to_value(&answer).unwrap();
        assert_eq!(json["waitingForUser"], true);
        assert_eq!(json["steps"][1]["status"], "waiting for the user");
    }

    #[test]
    fn a_failed_finish_says_where_and_a_done_one_says_so() {
        let failed = vec![(
            "Check".to_string(),
            Status::Failed("Failed (exit status 1).".into()),
            String::new(),
        )];
        let (ok, text, _) = finish_answer("fix", false, true, failed);
        assert!(!ok);
        assert!(text.starts_with("Finish stopped at Check. Fix that, then call finish_task again."));
        let done = vec![(
            "Clean up".to_string(),
            Status::Done("Archived.".into()),
            String::new(),
        )];
        let (ok, text, answer) = finish_answer("fix", false, false, done);
        assert!(ok && answer.finished);
        assert!(text.starts_with("Finished fix.\n\n"));
    }
}
