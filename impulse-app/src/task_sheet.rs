//! New Task (the Swift `TaskSheetModel`, `TaskSheetView` and
//! `createTask`): a title becomes a branch checked out in its own folder
//! beside the repository, from a base (what the main checkout's branch
//! tracks by default), with the untracked files a fresh checkout lacks
//! copied in, optionally the main checkout's uncommitted files moved in
//! (with Undo), the task's own values written once the project settings
//! are trusted, and its first terminal running the setup script and the
//! chosen agent.

use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::Select;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::{
    div, prelude::*, px, AnyElement, Context, Entity, FontWeight, Subscription, WeakEntity, Window,
};
use impulse_git::change_move::{self, Moved};
use impulse_git::{operations, scoped_diff, snapshot};
use impulse_kit::agents::task_requests;
use impulse_kit::chrome::{metrics, ChromePalette};
use impulse_kit::git::task_registry;
use impulse_kit::git::worktree_tasks;
use impulse_kit::project::config::ProjectConfig;
use impulse_kit::project::{detector, folder_clone, task_environment};

use crate::settings_view::{choice_select, Choice, ChoiceSelect};
use crate::tasks::{self, toast, ToastKind};
use crate::workbench::Workbench;
use crate::workbench_chrome::{abbreviate_home, hsla};

/// The agent that asked for a task (the Swift `AgentTaskRequest`): who,
/// and from which folder.
#[derive(Clone, Debug)]
pub(crate) struct AgentRequest {
    pub requested_by: String,
    pub from_path: String,
}

/// How an agent's request for a task ended; the sender dropped without a
/// word means the user cancelled.
pub(crate) enum Outcome {
    Created { path: String, branch: String },
    Failed(String),
}

/// What the sheet knows and what the user chose.
pub struct TaskSheet {
    palette: ChromePalette,
    workbench: WeakEntity<Workbench>,
    pub repo_root: String,
    title: Entity<InputState>,
    base: Entity<InputState>,
    /// "Just a terminal" or an agent found on PATH, once they're looked up.
    agent: Option<ChoiceSelect>,
    taken: HashSet<String>,
    /// Files `.worktreeinclude` (or the defaults) would copy.
    copies: Vec<String>,
    /// Taken branches, copies and agents are still being looked up.
    is_loading: bool,
    pub is_creating: bool,
    error: Option<String>,
    /// Under From: unpushed commits the default base leaves out.
    base_note: Option<String>,
    /// The remote From's default comes from, when there is one.
    remote: Option<String>,
    is_fetching: bool,
    fetch_error: Option<String>,
    /// The repository is trusted, so the sheet fetches on its own;
    /// otherwise From has a Fetch button.
    fetches_on_its_own: bool,
    /// From as the sheet filled it in; once the user types, it's left alone.
    default_base: Option<String>,
    /// The main checkout's uncommitted files, which can move into the task.
    movable_count: usize,
    moves_changes: bool,
    /// What From can say for the commit the changes were made on.
    head_names: HashSet<String>,
    /// The main checkout's branch (or HEAD), for "Start from main".
    head_name: String,
    settings: Option<ProjectConfig>,
    slot: Option<i64>,
    /// Every slot's ports are in use or past 65535.
    no_free_slot: bool,
    /// Folders that will be cloned in.
    clones: Vec<String>,
    /// Lock files in the repository, when it has no task settings yet.
    lock_files: Vec<String>,
    /// The title field takes the focus the first time the dialog is built.
    pub initial_focus: bool,
    /// The agent that asked for the task, when one did.
    pub(crate) agent_request: Option<AgentRequest>,
    /// What the task is for, kept with it: the agent's first prompt.
    brief: Entity<TextareaState>,
    /// Start chooses this command once the agents are looked up.
    pub(crate) command: String,
    /// Who hears how an agent's request ended.
    pub(crate) outcome: Option<async_channel::Sender<Outcome>>,
    /// Made without the sheet (`agent_task_creation = trusted_projects`):
    /// its toast offers Undo.
    pub(crate) without_asking: bool,
    /// An agent working in the main checkout: its files can't move mid-turn.
    pub(crate) move_blocker: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// What the sheet's background lookups found.
struct Lookups {
    base: Option<(String, Option<String>)>,
    remote: Option<String>,
    taken: HashSet<String>,
    copies: Vec<String>,
    agents: Vec<(String, String)>,
    settings: Option<ProjectConfig>,
    slot: Option<i64>,
    no_free_slot: bool,
    clones: Vec<String>,
    lock_files: Vec<String>,
    movable: usize,
    head_names: HashSet<String>,
    head_name: String,
    main_branch: Option<String>,
}

/// What From can say for HEAD: "HEAD", the branch, the commit id, and
/// remote-tracking branches at the same commit (`origin/main`). Off the
/// main thread.
pub(crate) fn head_names(root: &str) -> (HashSet<String>, String) {
    let Some(head) = scoped_diff::resolve_commit(root, "HEAD") else {
        return (HashSet::new(), "HEAD".into());
    };
    let branch = operations::current_branch(root);
    let mut names: HashSet<String> = ["HEAD".to_string(), head.clone(), head[..7].to_string()]
        .into_iter()
        .collect();
    names.extend(branch.clone());
    names.extend(scoped_diff::remote_branches_at(&head, root));
    (names, branch.unwrap_or_else(|| "HEAD".into()))
}

fn look_up(root: &str) -> Lookups {
    let main = snapshot::snapshot(root, true);
    let base = main
        .as_ref()
        .map(|s| worktree_tasks::default_base(s.branch.as_deref(), s.upstream.as_deref(), s.ahead));
    let remote = main
        .as_ref()
        .and_then(|s| s.upstream.clone())
        .and_then(|upstream| {
            operations::remotes(root)
                .into_iter()
                .find(|r| upstream.starts_with(&format!("{r}/")))
        });
    let taken = operations::branches(root).0.into_iter().collect();
    let copies = tasks::task_copies(root);
    let agents = impulse_kit::agents::known_agents::built_in()
        .iter()
        .filter_map(|kind| {
            let name = kind.names.first()?;
            impulse_kit::shell::login::which(name)?;
            Some((kind.display_name.clone(), name.clone()))
        })
        .collect();
    let settings = tasks::project_config(root);
    let registry = settings
        .as_ref()
        .filter(|s| s.has_task_values())
        .and_then(|_| tasks::registry(root));
    let slot = registry
        .as_ref()
        .and_then(|r| r.next_slot(|slot| tasks::ports_are_free(settings.as_ref(), slot)));
    let no_free_slot = registry.is_some() && slot.is_none();
    let clones = settings
        .as_ref()
        .map(|s| {
            s.worktree_clone
                .iter()
                .chain(s.database_folder.iter())
                .filter(|f| Path::new(root).join(f).exists())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let lock_files = detector::INSTALL_COMMANDS
        .iter()
        .map(|(lock, _)| lock.to_string())
        .filter(|lock| Path::new(root).join(lock).exists())
        .collect();
    let (head_names, head_name) = head_names(root);
    Lookups {
        base,
        remote,
        taken,
        copies,
        agents,
        settings,
        slot,
        no_free_slot,
        clones,
        lock_files,
        movable: main.as_ref().map(|s| s.changed_file_count()).unwrap_or(0),
        head_names,
        head_name,
        main_branch: main.and_then(|s| s.branch),
    }
}

impl TaskSheet {
    pub(crate) fn new(
        repo_root: String,
        moving: bool,
        palette: ChromePalette,
        workbench: WeakEntity<Workbench>,
        trusted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title =
            cx.new(|cx| InputState::new(window, cx).placeholder("e.g. Fix the flaky login test"));
        let base = cx.new(|cx| InputState::new(window, cx).placeholder("base branch"));
        let brief = cx.new(|cx| TextareaState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &title,
                window,
                |sheet, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => sheet.create(window, cx),
                    _ => cx.notify(),
                },
            ),
            cx.subscribe_in(&base, window, |_, _, _: &InputEvent, _, cx| cx.notify()),
        ];
        let mut sheet = TaskSheet {
            palette,
            workbench,
            repo_root: repo_root.clone(),
            title,
            base,
            agent: None,
            taken: HashSet::new(),
            copies: Vec::new(),
            is_loading: true,
            is_creating: false,
            error: None,
            base_note: None,
            remote: None,
            is_fetching: false,
            fetch_error: None,
            fetches_on_its_own: trusted,
            default_base: None,
            movable_count: 0,
            moves_changes: moving,
            head_names: HashSet::new(),
            head_name: "HEAD".into(),
            settings: None,
            slot: None,
            no_free_slot: false,
            clones: Vec::new(),
            lock_files: Vec::new(),
            initial_focus: true,
            agent_request: None,
            brief,
            command: String::new(),
            outcome: None,
            without_asking: false,
            move_blocker: None,
            _subscriptions: subscriptions,
        };
        sheet.load(window, cx);
        sheet
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.repo_root.clone();
        let work = cx
            .background_executor()
            .spawn(async move { look_up(&root) });
        cx.spawn_in(window, async move |sheet, cx| {
            let found = work.await;
            let _ = sheet.update_in(cx, |sheet, window, cx| sheet.loaded(found, window, cx));
        })
        .detach();
    }

    fn loaded(&mut self, found: Lookups, window: &mut Window, cx: &mut Context<Self>) {
        self.movable_count = found.movable;
        self.head_names = found.head_names;
        self.head_name = found.head_name;
        if self.can_replace_base(cx) {
            // Moving changes starts where they were made: the main
            // checkout's branch. Otherwise what it tracks.
            let (base, note) = if self.moves_changes {
                (found.main_branch.unwrap_or_else(|| "HEAD".into()), None)
            } else {
                found.base.unwrap_or_else(|| ("HEAD".into(), None))
            };
            self.base
                .update(cx, |input, cx| input.set_value(base.clone(), window, cx));
            self.default_base = Some(base);
            self.base_note = note;
        }
        self.settings = found.settings;
        self.slot = found.slot;
        self.no_free_slot = found.no_free_slot;
        self.clones = found.clones;
        self.lock_files = found.lock_files;
        self.remote = found.remote;
        self.taken = found.taken;
        self.copies = found.copies;
        let mut choices = vec![Choice::new("", "Just a terminal")];
        choices.extend(
            found
                .agents
                .into_iter()
                .map(|(name, command)| Choice::new(command, name)),
        );
        let command = self.command.clone();
        self.agent = Some(choice_select(choices, &command, false, window, cx));
        self.is_loading = false;
        // Start from what's on the remote now; creating doesn't wait.
        if self.fetches_on_its_own {
            self.fetch(window, cx);
        }
        cx.notify();
    }

    /// Fetch the remote From's default comes from (a failure shows under
    /// From; the task can be created anyway).
    fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(remote) = self.remote.clone() else {
            return;
        };
        if self.is_fetching {
            return;
        }
        self.is_fetching = true;
        self.fetch_error = None;
        let root = self.repo_root.clone();
        let work = cx.background_executor().spawn(async move {
            let result = operations::fetch(false, None, &root, Duration::from_secs(60), None);
            (result, head_names(&root).0)
        });
        cx.spawn_in(window, async move |sheet, cx| {
            let (result, names) = work.await;
            let _ = sheet.update(cx, |sheet, cx| {
                sheet.is_fetching = false;
                sheet.head_names = names;
                if let Err(error) = result {
                    let message = error.message();
                    let reason = message.lines().next().unwrap_or("");
                    sheet.fetch_error = Some(if reason.is_empty() {
                        format!("Couldn't fetch {remote}")
                    } else {
                        format!("Couldn't fetch {remote}: {reason}")
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// An agent's request: its title, base, brief and agent filled in.
    pub(crate) fn prefill(
        &mut self,
        title: &str,
        base: Option<&str>,
        brief: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = title.to_string();
        self.title
            .update(cx, |input, cx| input.set_value(title, window, cx));
        if let Some(base) = base.filter(|b| !b.is_empty()) {
            let base = base.to_string();
            self.base
                .update(cx, |input, cx| input.set_value(base, window, cx));
        }
        let brief = brief.to_string();
        self.brief
            .update(cx, |input, cx| input.set_value(brief, window, cx));
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.is_loading
    }

    fn brief_text(&self, cx: &gpui_kit::App) -> String {
        self.brief.read(cx).value().trim().to_string()
    }

    fn title_text(&self, cx: &gpui_kit::App) -> String {
        self.title.read(cx).value().trim().to_string()
    }

    pub fn base_text(&self, cx: &gpui_kit::App) -> String {
        self.base.read(cx).value().trim().to_string()
    }

    /// From still holds what the sheet put there (or nothing).
    fn can_replace_base(&self, cx: &gpui_kit::App) -> bool {
        match &self.default_base {
            Some(default) => &self.base_text(cx) == default,
            None => self.base_text(cx).is_empty(),
        }
    }

    pub fn branch(&self, cx: &gpui_kit::App) -> String {
        worktree_tasks::branch_name(&self.title_text(cx), &self.taken)
    }

    pub fn path(&self, cx: &gpui_kit::App) -> String {
        worktree_tasks::worktree_path(&self.repo_root, &self.branch(cx))
    }

    fn main_name(&self) -> String {
        Path::new(&self.repo_root)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn offers_move(&self) -> bool {
        self.movable_count > 0
    }

    /// Why the files can't move now; `None` when they can.
    fn move_unavailable(&self, cx: &gpui_kit::App) -> Option<String> {
        if let Some(blocker) = &self.move_blocker {
            return Some(blocker.clone());
        }
        let base = self.base_text(cx);
        (!base.is_empty() && !self.head_names.contains(&base)).then(|| {
            format!(
                "The changes were made on {}'s current commit, so From has to be that commit.",
                self.main_name()
            )
        })
    }

    /// The project has nothing set up for tasks yet.
    fn needs_setup(&self) -> bool {
        match &self.settings {
            None => true,
            Some(s) => {
                s.setup_script.is_none()
                    && s.worktree_clone.is_empty()
                    && !s.has_task_values()
                    && s.database_folder.is_none()
                    && !s.compose_override
            }
        }
    }

    /// "APP_PORT 8100 · VITE_PORT 5273 · .env: APP_URL, DB_DATABASE".
    fn values_preview(&self) -> Option<String> {
        let settings = self.settings.as_ref().filter(|s| s.has_task_values())?;
        let ports = task_environment::ports(&settings.ports, self.slot?, settings.port_offset)?;
        let mut parts: Vec<String> = ports.iter().map(|(k, v)| format!("{k} {v}")).collect();
        if !settings.worktree_env.is_empty() {
            parts.push(format!(
                "{}: {}",
                settings.env_file,
                settings
                    .worktree_env
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Some(parts.join(" · "))
    }

    /// Not before the lookups finish: the branch name must be checked
    /// against the taken ones, and the copies known.
    pub fn can_create(&self, cx: &gpui_kit::App) -> bool {
        !self.is_loading && !self.is_creating && !self.title_text(cx).is_empty()
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_create(cx) {
            return;
        }
        self.is_creating = true;
        self.error = None;
        cx.notify();
        // After this update: creating reads the sheet.
        let (sheet, workbench) = (cx.entity(), self.workbench.clone());
        window.defer(cx, move |window, cx| {
            let _ = workbench.update(cx, |workbench, cx| workbench.create_task(sheet, window, cx));
        });
    }

    /// For the headless snapshots (`debug_snapshot`): fill in the title
    /// and From, as typing does.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_fill(
        &mut self,
        title: &str,
        base: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = title.to_string();
        self.title
            .update(cx, |input, cx| input.set_value(title, window, cx));
        if let Some(base) = base.filter(|b| !b.is_empty()) {
            let base = base.to_string();
            self.default_base = None;
            self.base
                .update(cx, |input, cx| input.set_value(base, window, cx));
        }
        cx.notify();
    }

    /// What the sheet holds, as the snapshots' dumps show it.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_state(&self, cx: &gpui_kit::App) -> serde_json::Value {
        serde_json::json!({
            "title": self.title_text(cx), "base": self.base_text(cx),
            "command": self.agent.as_ref().and_then(|a| a.read(cx).selected_value().cloned()),
            "brief": self.brief_text(cx),
            "requestedBy": self.agent_request.as_ref().map(|r| r.requested_by.clone()),
            "branch": self.branch(cx), "error": self.error,
        })
    }

    fn field(&self, label: &'static str, content: impl IntoElement) -> impl IntoElement {
        let p = &self.palette;
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .w(px(44.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .text_size(px(12.))
                    .text_color(hsla(p.text_secondary))
                    .child(label),
            )
            .child(content)
    }

    fn detail(&self, label: &'static str, value: String) -> impl IntoElement {
        let p = &self.palette;
        div()
            .flex()
            .gap(px(8.))
            .child(
                div()
                    .w(px(44.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .text_size(px(11.))
                    .text_color(hsla(p.text_tertiary))
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family("JetBrains Mono")
                    .text_size(px(11.))
                    .text_color(hsla(p.text_secondary))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(value),
            )
    }

    fn note(&self, text: String, color: impulse_kit::chrome::Rgba) -> AnyElement {
        div()
            .pl(px(54.))
            .text_size(px(11.))
            .text_color(hsla(color))
            .whitespace_normal()
            .child(text)
            .into_any_element()
    }
}

impl Render for TaskSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let titled = !self.title_text(cx).is_empty();
        let base_row = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().w(px(220.)).child(Input::new(&self.base).small()))
            .when(self.is_fetching, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(hsla(p.text_tertiary))
                        .child(format!(
                            "Fetching {}…",
                            self.remote.clone().unwrap_or_default()
                        )),
                )
            })
            .when(
                !self.is_fetching && !self.fetches_on_its_own && self.remote.is_some(),
                |d| {
                    d.child(
                        Button::new("task-fetch")
                            .label("Fetch")
                            .small()
                            .on_click(cx.listener(|sheet, _, window, cx| sheet.fetch(window, cx))),
                    )
                },
            );
        let start = match &self.agent {
            Some(state) => div()
                .w(px(220.))
                .child(Select::new(state).small().menu_width(px(220.)))
                .into_any_element(),
            None => div()
                .text_size(px(12.))
                .text_color(hsla(p.text_tertiary))
                .child("Just a terminal")
                .into_any_element(),
        };
        let unavailable = self.move_unavailable(cx);
        let moving = self.moves_changes && unavailable.is_none();
        let main_name = self.main_name();
        let move_label = format!(
            "Move {main_name}'s {} uncommitted file{} into this task",
            self.movable_count,
            if self.movable_count == 1 { "" } else { "s" }
        );
        let copies = if self.is_loading {
            "…".to_string()
        } else if self.copies.is_empty() {
            "nothing (add patterns to .worktreeinclude)".to_string()
        } else {
            self.copies.join(", ")
        };
        let creating = self.is_creating;
        let can_create = self.can_create(cx);
        let head = self.head_name.clone();
        let _ = window;
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .text_color(hsla(p.text))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("New Task"),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_secondary))
                    .whitespace_normal()
                    .child(
                        "A branch checked out in its own folder, opened as a workspace: work on it \
                         (or let an agent) without touching your current checkout.",
                    ),
            )
            .children(self.agent_request.as_ref().map(|request| {
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .child(div().pt(px(2.)).child(crate::workbench_chrome::icon("bot", 11., p.accent)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .whitespace_normal()
                            .child(format!(
                                "{} wants to start this task. Check it, change anything, then create it or cancel.",
                                request.requested_by
                            )),
                    )
            }))
            .child(self.field("Task", div().flex_1().child(Input::new(&self.title).small())))
            .child(self.field("From", base_row))
            .children(
                self.fetch_error
                    .clone()
                    .or(self.base_note.clone())
                    .map(|note| self.note(note, p.text_tertiary)),
            )
            .child(self.field("Start", start))
            .when(
                self.agent_request.is_some() || !self.brief_text(cx).is_empty(),
                |d| {
                    let agent_chosen = self
                        .agent
                        .as_ref()
                        .and_then(|a| a.read(cx).selected_value().cloned())
                        .is_some_and(|c| !c.is_empty());
                    d.child(
                        div()
                            .pl(px(54.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(hsla(p.text_tertiary))
                                    .child(if agent_chosen {
                                        "Brief (the agent's first prompt)"
                                    } else {
                                        "Brief (kept with the task; no agent starts)"
                                    }),
                            )
                            .child(
                                div()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(11.))
                                    .child(Textarea::new(&self.brief).h(px(110.))),
                            ),
                    )
                },
            )
            .when(self.offers_move(), |d| {
                d.child(
                    div()
                        .pl(px(54.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            Checkbox::new("task-move")
                                .label(move_label)
                                .checked(moving)
                                .disabled(unavailable.is_some())
                                .on_change(cx.listener(|sheet, checked: &bool, _, cx| {
                                    sheet.moves_changes = *checked;
                                    cx.notify();
                                })),
                        )
                        .children(unavailable.clone().map(|reason| {
                            let blocked = self.move_blocker.is_some();
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .text_size(px(11.))
                                .text_color(hsla(p.text_tertiary))
                                .child(div().whitespace_normal().child(reason))
                                .when(!blocked, |d| d.child(
                                    // A plain button, as the Swift sheet's.
                                    div()
                                        .id("task-start-from-head")
                                        .self_start()
                                        .cursor_pointer()
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(hsla(p.text))
                                        .hover(|s| s.opacity(0.8))
                                        .child(format!("Start from {head}"))
                                        .on_click(cx.listener(move |sheet, _, window, cx| {
                                            let head = sheet.head_name.clone();
                                            sheet.base.update(cx, |input, cx| {
                                                input.set_value(head, window, cx)
                                            });
                                            sheet.moves_changes = true;
                                            cx.notify();
                                        })),
                                ))
                        }))
                        .when(moving, |d| {
                            d.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(hsla(p.text_tertiary))
                                    .whitespace_normal()
                                    .child(format!(
                                        "They arrive unstaged, and {main_name} goes back to its last \
                                         commit for them. Ignored files such as .env stay."
                                    )),
                            )
                        }),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(10.))
                    .rounded(px(metrics::RADIUS))
                    .bg(hsla(p.raised))
                    .child(self.detail(
                        "Branch",
                        if titled { self.branch(cx) } else { "-".into() },
                    ))
                    .child(self.detail(
                        "Folder",
                        if titled {
                            abbreviate_home(&self.path(cx))
                        } else {
                            "-".into()
                        },
                    ))
                    .child(self.detail("Copies", copies))
                    .when(!self.clones.is_empty(), |d| {
                        d.child(self.detail(
                            "Clones",
                            self.clones.iter().map(|c| format!("{c}/")).collect::<Vec<_>>().join(", "),
                        ))
                    })
                    .children(self.values_preview().map(|v| self.detail("Values", v))),
            )
            .when(self.no_free_slot, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(hsla(p.warning))
                        .whitespace_normal()
                        .child(format!(
                            "No free ports for a new task: every slot up to {} has a port in use or past \
                             65535. This task gets no ports or values of its own.",
                            task_registry::MAX_SLOT
                        )),
                )
            })
            .when(!self.is_loading && self.needs_setup(), |d| {
                let (workbench, root) = (self.workbench.clone(), self.repo_root.clone());
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(hsla(p.text_tertiary))
                                .child(if self.lock_files.is_empty() {
                                    "No setup for tasks yet.".to_string()
                                } else {
                                    format!(
                                        "No setup script. This repository has {}.",
                                        self.lock_files.join(" and ")
                                    )
                                }),
                        )
                        .child(
                            Button::new("task-set-up")
                                .ghost()
                                .xsmall()
                                .label("Set Up This Project for Tasks…")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let root = root.clone();
                                    let _ = workbench.update(cx, |w, cx| {
                                        w.open_project_setup_at(root, None, window, cx)
                                    });
                                }),
                        ),
                )
            })
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(11.5))
                    .text_color(hsla(p.danger))
                    .whitespace_normal()
                    .child(error)
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("task-cancel")
                            .label("Cancel")
                            .disabled(creating)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("task-create")
                            .primary()
                            .icon(gpui_kit::component::Icon::empty().path("icons/git-branch-plus.svg"))
                            .label(if creating { "Creating…" } else { "Create Task" })
                            .disabled(!can_create)
                            .on_click(cx.listener(|sheet, _, window, cx| sheet.create(window, cx))),
                    ),
            )
    }
}

/// What creating a task did, for the toasts.
struct Created {
    slot: Option<i64>,
    moved: Option<Moved>,
    move_failure: Option<String>,
    /// Where the brief was saved, when there's one.
    brief_path: Option<String>,
    /// The commit it started at (Undo deletes the branch only from there).
    start: Option<String>,
}

/// The login shell, as task commands are written for it.
fn login_shell_type() -> task_requests::ShellType {
    let shell = impulse_kit::shell::login::default_shell_path();
    match impulse_kit::shell::ShellType::detect(&shell) {
        impulse_kit::shell::ShellType::Zsh => task_requests::ShellType::Zsh,
        impulse_kit::shell::ShellType::Fish => task_requests::ShellType::Fish,
        impulse_kit::shell::ShellType::Bash => task_requests::ShellType::Bash,
    }
}

impl Workbench {
    /// "New Task...": ask for a title, a base and what to start, then
    /// create it from the workspace at `index`'s repository (else the
    /// active one's). `moving`: "Move Changes to New Task...", the main
    /// checkout's uncommitted files go along.
    pub(crate) fn present_new_task(
        &mut self,
        index: Option<usize>,
        moving: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.task_repository(index) else {
            return toast(
                window,
                cx,
                ToastKind::Info,
                "Open a folder in a git repository to start a task.".into(),
                None,
                None,
            );
        };
        let (palette, workbench, trusted) = (
            self.palette.clone(),
            cx.entity().downgrade(),
            self.trust.is_trusted(&root),
        );
        let blocker = self.move_blocker(&root, cx);
        let sheet = cx.new(|cx| {
            let mut sheet = TaskSheet::new(root, moving, palette, workbench, trusted, window, cx);
            sheet.move_blocker = blocker;
            sheet
        });
        self.show_task_sheet(sheet, window, cx);
    }

    /// Show a New Task sheet made elsewhere (an agent's request).
    pub(crate) fn show_task_sheet(
        &mut self,
        sheet: Entity<TaskSheet>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(debug_assertions)]
        crate::debug_snapshot::note_task_sheet(&sheet);
        window.open_dialog(cx, move |dialog, window, cx| {
            if sheet.update(cx, |s, _| std::mem::replace(&mut s.initial_focus, false)) {
                let title = sheet.read(cx).title.clone();
                title.update(cx, |input, cx| input.focus(window, cx));
            }
            let creating = sheet.read(cx).is_creating;
            let on_enter = sheet.clone();
            dialog
                .w(px(480.))
                .close_button(false)
                .keyboard(!creating)
                .overlay_closable(false)
                // Return creates the task (the sheet closes once it's made).
                .on_ok(move |_, window, cx| {
                    on_enter.update(cx, |sheet, cx| sheet.create(window, cx));
                    false
                })
                .child(sheet.clone())
        });
    }

    /// Create the task the sheet describes.
    pub(crate) fn create_task(
        &mut self,
        sheet: Entity<TaskSheet>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (root, branch, path, base, copies, command, moving) = {
            let s = sheet.read(cx);
            (
                s.repo_root.clone(),
                s.branch(cx),
                s.path(cx),
                s.base_text(cx),
                s.copies.clone(),
                s.agent
                    .as_ref()
                    .and_then(|a| a.read(cx).selected_value().cloned())
                    .unwrap_or_else(|| s.command.clone()),
                s.moves_changes && s.offers_move() && s.move_unavailable(cx).is_none(),
            )
        };
        let (brief, request, without_asking) = {
            let s = sheet.read(cx);
            (s.brief_text(cx), s.agent_request.clone(), s.without_asking)
        };
        let outcome = sheet.update(cx, |s, _| s.outcome.take());
        // An agent may have started working there since the sheet opened.
        if let Some(blocker) = moving.then(|| self.move_blocker(&root, cx)).flatten() {
            sheet.update(cx, |s, cx| {
                s.is_creating = false;
                s.error = Some(blocker);
                s.outcome = outcome;
                cx.notify();
            });
            return;
        }
        let work = {
            let (root, branch, path, base) =
                (root.clone(), branch.clone(), path.clone(), base.clone());
            let (brief, request) = (brief.clone(), request.clone());
            cx.background_executor().spawn(async move {
                let failure = if moving
                    && scoped_diff::resolve_commit(&root, if base.is_empty() { "HEAD" } else { &base })
                        != scoped_diff::resolve_commit(&root, "HEAD")
                {
                    Some(format!(
                        "From isn't {}'s current commit anymore, so the changes can't move into it.",
                        Path::new(&root).file_name().unwrap_or_default().to_string_lossy()
                    ))
                } else if Path::new(&path).exists() {
                    Some(format!("{} already exists.", abbreviate_home(&path)))
                } else {
                    if let Some(parent) = Path::new(&path).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    operations::add_worktree(
                        &path,
                        &branch,
                        true,
                        (!base.is_empty()).then_some(base.as_str()),
                        false,
                        &root,
                    )
                    .err()
                    .map(|e| e.message())
                };
                if let Some(failure) = failure {
                    return Err(failure);
                }
                // Read (not yet trusted) only to pick a slot whose ports are free.
                let config = tasks::project_config(&path);
                let mut files = copies.clone();
                files.extend(tasks::env_file_copy(config.as_ref(), &root, &copies));
                tasks::copy_files(&files, &root, &path);
                for folder in config.as_ref().map(|c| c.worktree_clone.clone()).unwrap_or_default() {
                    folder_clone::clone(&folder, &root, &path);
                }
                let record = tasks::record_created(
                    &path,
                    &branch,
                    &base,
                    &root,
                    false,
                    match request {
                        Some(request) => tasks::Provenance {
                            created_by: "agent",
                            requested_by: Some(request.requested_by),
                            requested_from: Some(request.from_path),
                        },
                        None => tasks::Provenance::default(),
                    },
                    |slot| tasks::ports_are_free(config.as_ref(), slot),
                );
                // Beside the task's other files, outside its folder.
                let brief_path = (!brief.is_empty())
                    .then(|| impulse_git::branches::common_git_directory(&root))
                    .flatten()
                    .and_then(|common| {
                        let name = Path::new(&path).file_name()?.to_string_lossy().into_owned();
                        task_requests::write_brief(&brief, &name, &common).ok()?;
                        Some(task_requests::brief_path(&name, &common))
                    });
                let (mut moved, mut move_failure) = (None, None);
                if moving {
                    match change_move::move_changes(None, &root, &path) {
                        Ok(result) => moved = Some(result),
                        Err(error) => move_failure = Some(error.message()),
                    }
                }
                Ok(Created {
                    slot: record.as_ref().and_then(|r| r.slot),
                    moved,
                    move_failure,
                    brief_path,
                    start: record.and_then(|r| r.start),
                })
            })
        };
        cx.spawn_in(window, async move |workbench, cx| {
            let result = work.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                let created = match result {
                    Ok(created) => created,
                    Err(failure) => {
                        // Without the sheet, this is the only word of it.
                        if without_asking {
                            if let Some(outcome) = &outcome {
                                let _ = outcome.try_send(Outcome::Failed(failure.clone()));
                            }
                        }
                        sheet.update(cx, |s, cx| {
                            s.is_creating = false;
                            s.error = Some(failure);
                            if !without_asking {
                                s.outcome = outcome;
                            }
                            cx.notify();
                        });
                        return;
                    }
                };
                if !without_asking {
                    window.close_dialog(cx);
                }
                // The same repository: a task folder is as trusted as it is.
                if workbench.trust.is_trusted(&root) {
                    workbench.trust.trust(&path);
                }
                let (root2, path2) = (root.clone(), path.clone());
                workbench.trust_project_config(
                    &path.clone(),
                    window,
                    cx,
                    Box::new(move |workbench, config, window, cx| {
                        let setup = config.as_ref().and_then(|c| c.setup_script.clone());
                        workbench.after_preparing(
                            config,
                            root2,
                            path2,
                            created.slot,
                            window,
                            cx,
                            Box::new(move |workbench, _, window, cx| {
                                // The agent starts on its brief: as its
                                // first prompt where its command takes one,
                                // else typed in once it's up.
                                let agent = impulse_kit::agents::known_agents::built_in()
                                    .iter()
                                    .find(|k| k.names.first() == Some(&command));
                                let mut start = command.clone();
                                let mut type_brief = false;
                                if let (Some(agent), Some(brief_path)) =
                                    (agent, &created.brief_path)
                                {
                                    match task_requests::command(
                                        agent,
                                        Some(brief_path),
                                        login_shell_type(),
                                    ) {
                                        Some(with_brief) => start = with_brief,
                                        None => type_brief = true,
                                    }
                                }
                                let first: Vec<String> =
                                    [setup, (!start.is_empty()).then_some(start)]
                                        .into_iter()
                                        .flatten()
                                        .collect();
                                let first = (!first.is_empty()).then(|| first.join(" && "));
                                let index = workbench.open_task_workspace(
                                    Path::new(&path),
                                    first,
                                    window,
                                    cx,
                                );
                                if type_brief {
                                    let terminal = workbench.workspaces[index]
                                        .all_surfaces()
                                        .find_map(|s| s.terminal().cloned());
                                    if let Some(terminal) = terminal {
                                        crate::agent_tasks::type_brief_when_agent_starts(
                                            brief.clone(),
                                            terminal,
                                            cx,
                                        );
                                    }
                                }
                                if let Some(outcome) = &outcome {
                                    let _ = outcome.try_send(Outcome::Created {
                                        path: path.clone(),
                                        branch: branch.clone(),
                                    });
                                }
                                if without_asking {
                                    let who = request
                                        .as_ref()
                                        .map(|r| r.requested_by.clone())
                                        .unwrap_or_else(|| "An agent".into());
                                    workbench.announce_agent_task(
                                        who,
                                        &root,
                                        &path,
                                        &branch,
                                        created.start.clone(),
                                        window,
                                        cx,
                                    );
                                } else {
                                    workbench
                                        .announce_task(&root, &path, &branch, created, window, cx);
                                }
                            }),
                        );
                    }),
                );
            });
        })
        .detach();
    }

    /// The toasts after a task opened: started (with Undo when files
    /// moved), and what a move left behind.
    #[allow(clippy::too_many_arguments)]
    fn announce_task(
        &mut self,
        root: &str,
        path: &str,
        branch: &str,
        created: Created,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let main = Path::new(root)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match &created.moved {
            Some(moved) => {
                let count = moved.count();
                let undo: tasks::Run = {
                    let (workbench, moved, root, path, branch) = (
                        cx.entity().downgrade(),
                        moved.clone(),
                        root.to_string(),
                        path.to_string(),
                        branch.to_string(),
                    );
                    Rc::new(move |window, cx| {
                        let _ = workbench.update(cx, |workbench, cx| {
                            workbench.undo_move(moved.clone(), &root, &path, &branch, window, cx)
                        });
                    })
                };
                toast(
                    window,
                    cx,
                    ToastKind::Success,
                    format!(
                        "Started task {branch} with {count} file{} from {main}.",
                        if count == 1 { "" } else { "s" }
                    ),
                    None,
                    Some(("Undo", undo)),
                );
            }
            None => toast(
                window,
                cx,
                ToastKind::Success,
                format!("Started task {branch}."),
                None,
                None,
            ),
        }
        if let Some(failure) = created.move_failure {
            toast(
                window,
                cx,
                ToastKind::Warning,
                format!("The changes stayed in {main}."),
                Some(failure),
                None,
            );
        }
        if let Some((message, detail)) = created.moved.as_ref().and_then(|m| move_note(m, &main)) {
            toast(window, cx, ToastKind::Warning, message, Some(detail), None);
        }
    }

    /// Undo a move: the files go back to the main checkout, and the new task
    /// is archived (what it holds by then kept in a snapshot) and its branch
    /// deleted when nothing was committed on it. When the files can't go
    /// back, or some changed in the main checkout since, the task stays.
    pub(crate) fn undo_move(
        &mut self,
        moved: Moved,
        root: &str,
        path: &str,
        branch: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.workspace_at(path) {
            let id = self.workspaces[index].id;
            if !self.close_workspace_for_task(id, window, cx) {
                return;
            }
        }
        let main = Path::new(root)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let trust = self.trust.clone();
        let work = {
            let (root, path, branch) = (root.to_string(), path.to_string(), branch.to_string());
            cx.background_executor().spawn(async move {
                // Snapshotted first, so nothing in the main checkout is
                // overwritten unrecorded.
                impulse_git::safety::create("undo move", &root, impulse_git::safety::OPLOG_PREFIX)
                    .map_err(|e| e.message())?;
                let left = change_move::undo(&moved, &root).map_err(|e| e.message())?;
                if !left.is_empty() {
                    return Ok(Err(left));
                }
                let removed = crate::task_cleanup::remove_task(&path, &branch, true, None, &trust);
                if removed.result.is_ok() {
                    let _ = operations::delete_branch(&branch, false, &root);
                }
                Ok(Ok(removed.result.map(|_| ())))
            })
        };
        let (path, branch) = (path.to_string(), branch.to_string());
        cx.spawn_in(window, async move |workbench, cx| {
            let result = work.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| match result {
                Err(error) => toast(
                    window,
                    cx,
                    ToastKind::Warning,
                    format!("Couldn't put the changes back, so {branch} was kept"),
                    Some(error),
                    None,
                ),
                Ok(Err(left)) => {
                    // The task still has its version of those files.
                    workbench.open_task_workspace(Path::new(&path), None, window, cx);
                    let one = left.len() == 1;
                    toast(
                        window,
                        cx,
                        ToastKind::Warning,
                        format!("The changes are back in {main}, but {branch} was kept."),
                        Some(format!(
                            "{} changed in {main} since the move, so {} left as {}. {branch} has the moved version.",
                            impulse_git::landing::naming(&left),
                            if one { "it was" } else { "they were" },
                            if one { "it is" } else { "they are" },
                        )),
                        None,
                    );
                }
                Ok(Ok(Err(error))) => toast(
                    window,
                    cx,
                    ToastKind::Warning,
                    format!("The changes are back, but {branch} couldn't be removed"),
                    Some(error),
                    None,
                ),
                Ok(Ok(Ok(()))) => {
                    workbench.refresh_task_states(cx);
                    toast(
                        window,
                        cx,
                        ToastKind::Success,
                        format!("The changes are back in {main}; {branch} is gone."),
                        None,
                        None,
                    )
                }
            });
        })
        .detach();
    }
}

/// What a move left behind, for a toast: files edited while it ran, and
/// folders holding a repository of their own. `None` when it took
/// everything.
fn move_note(moved: &Moved, main: &str) -> Option<(String, String)> {
    let mut parts = Vec::new();
    let edited = &moved.edited_during;
    if !edited.is_empty() {
        let one = edited.len() == 1;
        parts.push(format!(
            "{} {} edited during the move: {main} keeps the {}, and the task has {} as before.",
            impulse_git::landing::naming(edited),
            if one { "was" } else { "were" },
            if one { "edit" } else { "edits" },
            if one { "it" } else { "them" },
        ));
    }
    let nested = &moved.nested_repositories;
    if !nested.is_empty() {
        parts.push(if nested.len() == 1 {
            format!(
                "{} holds a git repository of its own, so it stays in {main}.",
                nested[0]
            )
        } else {
            format!(
                "{} folders hold git repositories of their own ({}), so they stay in {main}.",
                nested.len(),
                nested
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
    }
    (!parts.is_empty()).then(|| (format!("Some changes stayed in {main}."), parts.join(" ")))
}
