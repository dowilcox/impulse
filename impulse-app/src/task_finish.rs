//! Finish Task (the Swift `TaskFinish`): a tab that takes a task from done
//! to landed and cleaned up. It commits nothing itself; it checks the task
//! is committed, merges the base's new commits into it, runs the check
//! script in a terminal, merges the branch into its base and pushes (a
//! `--no-ff` merge in a throwaway worktree, `impulse_git::landing`) or
//! pushes it for review, updates the main checkout when that's safe, and
//! archives the task, deleting its branch if it hasn't moved. A step that
//! stops says why and what to do; Continue picks up from there.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{
    div, prelude::*, px, AnyElement, Context, Entity, FocusHandle, FontWeight, Subscription,
    WeakEntity, Window,
};
use impulse_git::landing::{self, Failure};
use impulse_git::operations::{self, PushOptions};
use impulse_git::{branches, ignored, scoped_diff, snapshot};
use impulse_kit::chrome::{metrics, ChromePalette, Rgba};
use impulse_kit::git::server_message::GitServerMessage;
use impulse_kit::project::config::{Landing, ProjectConfig};

use crate::task_cleanup::{ignored_files_warning, nested_repository_message, remove_task};
use crate::tasks::{self, confirm, toast, ToastKind};
use crate::terminal_view::TerminalViewEvent;
use crate::workbench::{surface_id, Workbench};
use crate::workbench_chrome::{hsla, icon};
use crate::workspace::{Kind, Surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Step {
    Commit,
    Sync,
    Check,
    Land,
    Update,
    CleanUp,
}

const STEPS: [Step; 6] = [
    Step::Commit,
    Step::Sync,
    Step::Check,
    Step::Land,
    Step::Update,
    Step::CleanUp,
];

impl Step {
    fn next(self) -> Option<Step> {
        STEPS
            .iter()
            .position(|s| *s == self)
            .and_then(|i| STEPS.get(i + 1).copied())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Running(String),
    Done(String),
    Skipped(String),
    Failed(String),
    /// Stopped on purpose, for the user (an agent's Finish stops before
    /// landing).
    Waiting(String),
}

/// What a stopped step offers besides Continue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remedy {
    ShowChanges,
    SkipCheck,
    PushForReview,
}

/// The Finish tab of one task.
pub struct TaskFinishView {
    pub palette: ChromePalette,
    workbench: WeakEntity<Workbench>,
    /// This view, for the buttons' handlers.
    this: WeakEntity<TaskFinishView>,
    focus: FocusHandle,
    /// The task's workspace.
    workspace_id: u64,
    /// The task's folder, its main checkout and branch.
    pub root: String,
    main_root: String,
    branch: String,
    /// The base branch (`main`) and its remote (`origin`; `None`: none).
    base: String,
    remote: Option<String>,
    check_script: Option<String>,
    /// The repository's saved answer; `None` until the first Finish asks.
    landing: Option<Landing>,
    chosen_landing: Landing,
    remembers_landing: bool,
    deletes_branch: bool,
    deletes_remote_branch: bool,
    /// The branch is on the remote.
    is_published: bool,
    /// Merged into its base already (on the server, after Push for Review).
    already_merged: bool,
    /// Why Finish can't run here (a worktree Impulse didn't make).
    unavailable: Option<String>,
    is_loading: bool,
    pub is_running: bool,
    statuses: HashMap<Step, Status>,
    /// Where Continue starts, and what else the stop offers.
    resume_at: Option<Step>,
    remedy: Option<Remedy>,
    /// What the server said on the push.
    server_message: Option<GitServerMessage>,
    /// The merge commit, for the summary.
    landed_commit: Option<String>,
    pushed_for_review: bool,
    /// The check's terminal, watched until its command ends.
    check: Option<(Subscription, Instant)>,
    /// An agent asked for this Finish (`impulse tasks finish`): it runs up
    /// to the land step (for a task already merged, the update) and waits
    /// there for the user. Who asked.
    pub(crate) stops_before_landing: Option<String>,
    /// The agent's terminal that asked, which the commit step doesn't wait
    /// for: it's mid-turn while it asks.
    pub(crate) requester_terminal: Option<gpui_kit::EntityId>,
}

impl TaskFinishView {
    fn name(&self) -> String {
        file_name(&self.root)
    }

    fn main_name(&self) -> String {
        file_name(&self.main_root)
    }

    fn base_ref(&self) -> String {
        match &self.remote {
            Some(remote) => format!("{remote}/{}", self.base),
            None => self.base.clone(),
        }
    }

    fn effective_landing(&self) -> Landing {
        self.landing.unwrap_or(self.chosen_landing)
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn title(&self) -> String {
        format!("Finish {}", self.name())
    }

    /// Each step's title, status and plan, for an agent's Finish.
    pub(crate) fn step_report(&self) -> Vec<(String, Status, String)> {
        STEPS
            .iter()
            .map(|step| (self.step_title(*step), self.status(*step), self.plan(*step)))
            .collect()
    }

    pub(crate) fn task_name(&self) -> String {
        self.name()
    }

    pub(crate) fn is_already_merged(&self) -> bool {
        self.already_merged
    }

    /// It stopped somewhere Continue picks up from.
    pub(crate) fn resumes(&self) -> bool {
        self.resume_at.is_some()
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub(crate) fn unavailable(&self) -> Option<String> {
        self.unavailable.clone()
    }

    /// The repository has its answer to how work lands.
    pub(crate) fn has_landing(&self) -> bool {
        self.landing.is_some()
    }

    pub(crate) fn main_name_text(&self) -> String {
        self.main_name()
    }

    fn status(&self, step: Step) -> Status {
        self.statuses.get(&step).cloned().unwrap_or(Status::Pending)
    }

    fn step_title(&self, step: Step) -> String {
        match step {
            Step::Commit => "Commit".into(),
            Step::Sync => format!("Sync with {}", self.base_ref()),
            Step::Check => "Check".into(),
            Step::Land if self.effective_landing() == Landing::Review => "Push for review".into(),
            Step::Land => match self.remote {
                None => format!("Merge into {}", self.base),
                Some(_) => format!("Merge into {} and push", self.base_ref()),
            },
            Step::Update => format!("Update {}", self.main_name()),
            Step::CleanUp => "Clean up".into(),
        }
    }

    /// What a step will do, before it runs.
    fn plan(&self, step: Step) -> String {
        let review = self.effective_landing() == Landing::Review && !self.already_merged;
        let (name, main, branch, base) = (self.name(), self.main_name(), &self.branch, &self.base);
        match step {
            Step::Commit => format!("Everything in {name} is committed, and no agent there is working."),
            Step::Sync => format!(
                "{}'s new commits are merged into {branch}, here in the task (never a rebase).",
                self.base_ref()
            ),
            Step::Check => match &self.check_script {
                Some(script) => format!("Runs {script} in a terminal tab; it has to pass."),
                None => "No check script, so this is skipped.".into(),
            },
            Step::Land if review => format!(
                "Pushes {branch} to {}. Your git host takes it from there.",
                self.remote.as_deref().unwrap_or("the remote")
            ),
            Step::Land => match self.remote {
                None => format!("A merge commit in {main}, when it's on {base} with nothing uncommitted."),
                Some(_) => format!(
                    "A merge commit, made in a throwaway folder and pushed as {base}. {main} isn't touched."
                ),
            },
            Step::Update if review => format!("After {branch} is merged."),
            Step::Update => match self.remote {
                None => "The merge is already there.".into(),
                Some(_) => format!("Fast-forwards {main} when it's on {base} with nothing uncommitted."),
            },
            Step::CleanUp if review => format!("After {branch} is merged, Impulse offers to clean up."),
            Step::CleanUp => "Archives the task (its archive script runs) and deletes its branch.".into(),
        }
    }

    fn finish_title(&self) -> &'static str {
        if self.already_merged {
            "Clean Up"
        } else if self.resume_at.is_some() {
            "Continue"
        } else if self.pushed_for_review {
            "Push Again"
        } else if self.effective_landing() == Landing::Review {
            "Push for Review"
        } else {
            "Finish"
        }
    }

    fn act<F>(
        &self,
        f: F,
    ) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static
    where
        F: Fn(&mut Workbench, Entity<TaskFinishView>, &mut Window, &mut Context<Workbench>)
            + 'static,
    {
        let workbench = self.workbench.clone();
        let view = self.this.clone();
        move |_, window, cx| {
            let Some(view) = view.upgrade() else { return };
            let _ = workbench.update(cx, |workbench, cx| f(workbench, view, window, cx));
        }
    }
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// MARK: Drawing

impl TaskFinishView {
    fn status_icon(&self, status: &Status) -> AnyElement {
        let p = &self.palette;
        let (name, color): (&str, Rgba) = match status {
            Status::Pending => ("circle", p.text_tertiary),
            Status::Running(_) => ("loader-circle", p.text_secondary),
            Status::Done(_) => ("circle-check", p.success),
            Status::Skipped(_) => ("minus", p.text_tertiary),
            Status::Failed(_) => ("circle-x", p.danger),
            Status::Waiting(_) => ("circle-alert", p.accent),
        };
        div()
            .w(px(16.))
            .h(px(16.))
            .child(icon(name, 13., color))
            .into_any_element()
    }

    fn step_row(&self, step: Step, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let status = self.status(step);
        let (text, color) = match &status {
            Status::Pending => (self.plan(step), p.text_tertiary),
            Status::Running(t) | Status::Done(t) => (t.clone(), p.text_secondary),
            Status::Skipped(t) => (t.clone(), p.text_tertiary),
            Status::Failed(t) => (t.clone(), p.danger),
            Status::Waiting(t) => (t.clone(), p.text),
        };
        let failed = matches!(status, Status::Failed(_));
        let stopped = (failed && !self.is_running) || (step == Step::Check && self.is_running);
        let mut buttons = div().flex().gap(px(8.)).pt(px(2.));
        let mut any = false;
        if stopped {
            if let Some(remedy) = self.remedy {
                any = true;
                let (label, icon_name) = match remedy {
                    Remedy::ShowChanges => ("Show Changes", Some("git-commit-horizontal")),
                    Remedy::SkipCheck => ("Skip Check", None),
                    Remedy::PushForReview => ("Push for Review Instead", Some("upload")),
                };
                let mut button = Button::new(("finish-remedy", step as usize))
                    .small()
                    .label(label)
                    .on_click(self.act(move |w, view, window, cx| {
                        w.apply_finish_remedy(remedy, view, window, cx)
                    }));
                if let Some(name) = icon_name {
                    button = button.icon(Icon::empty().path(format!("icons/{name}.svg")));
                }
                if remedy == Remedy::SkipCheck {
                    button = button.ghost();
                }
                buttons = buttons.child(button);
            }
        }
        if step == Step::Check && self.check_script.is_none() {
            any = true;
            let workspace = self.workspace_id;
            buttons = buttons.child(
                Button::new("finish-set-up-check")
                    .small()
                    .ghost()
                    .label("Set Up a Check…")
                    .on_click(self.act(move |w, _, window, cx| {
                        let index = w.workspaces.iter().position(|ws| ws.id == workspace);
                        w.open_project_setup(index, Some("scripts"), window, cx)
                    })),
            );
        }
        let _ = cx;
        div()
            .flex()
            .items_start()
            .gap(px(10.))
            .px(px(14.))
            .py(px(10.))
            .child(self.status_icon(&status))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.text))
                            .child(self.step_title(step)),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(hsla(color))
                            .whitespace_normal()
                            .child(text),
                    )
                    .when(any, |d| d.child(buttons)),
            )
            .into_any_element()
    }

    fn landing_choice(&self) -> AnyElement {
        let p = &self.palette;
        let mut column = div().flex().flex_col().gap(px(8.)).child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("How does work land in {}?", self.main_name())),
        );
        for landing in Landing::ALL {
            let chosen = self.chosen_landing == landing;
            let (title, detail) = match landing {
                Landing::Merge => (
                    format!("Merge into {} and push", self.base_ref()),
                    format!(
                        "Impulse merges the branch with a merge commit and pushes {}. Nobody reviews it on the server first.",
                        self.base
                    ),
                ),
                Landing::Review => (
                    "Push for review".to_string(),
                    "Impulse pushes the branch; you open a merge request on your git host and it's merged there."
                        .to_string(),
                ),
            };
            let running = self.is_running;
            column = column.child(
                div()
                    .id(("landing", landing as usize))
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .cursor_pointer()
                    .on_click(self.act(move |_, view, _, cx| {
                        if !running {
                            view.update(cx, |v, cx| {
                                v.chosen_landing = landing;
                                cx.notify();
                            });
                        }
                    }))
                    .child(icon(
                        if chosen { "circle-dot" } else { "circle" },
                        13.,
                        if chosen { p.accent } else { p.text_tertiary },
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(hsla(p.text_secondary))
                                    .whitespace_normal()
                                    .child(detail),
                            ),
                    ),
            );
        }
        column
            .child(
                Checkbox::new("finish-remember")
                    .label(format!(
                        "Remember for {} (on this computer; Project Setup can change it)",
                        self.main_name()
                    ))
                    .checked(self.remembers_landing)
                    .disabled(self.is_running)
                    .on_change(self.act_bool(|v, on| v.remembers_landing = on)),
            )
            .into_any_element()
    }

    fn act_bool(
        &self,
        set: impl Fn(&mut TaskFinishView, bool) + 'static,
    ) -> impl Fn(&bool, &mut Window, &mut gpui_kit::App) + 'static {
        let view = self.this.clone();
        move |checked, _, cx| {
            let _ = view.update(cx, |v, cx| {
                set(v, *checked);
                cx.notify();
            });
        }
    }

    fn branch_options(&self) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                Checkbox::new("finish-delete-branch")
                    .label(format!("Delete {} once it's merged", self.branch))
                    .checked(self.deletes_branch)
                    .disabled(self.is_running)
                    .on_change(self.act_bool(|v, on| v.deletes_branch = on)),
            )
            .when(self.is_published && self.remote.is_some(), |d| {
                d.child(
                    div().pl(px(18.)).child(
                        Checkbox::new("finish-delete-remote")
                            .label(format!(
                                "…and on {}",
                                self.remote.clone().unwrap_or_default()
                            ))
                            .checked(self.deletes_remote_branch)
                            .disabled(self.is_running || !self.deletes_branch)
                            .on_change(self.act_bool(|v, on| v.deletes_remote_branch = on)),
                    ),
                )
            })
            .into_any_element()
    }

    fn server_reply(&self, message: &GitServerMessage) -> AnyElement {
        let p = &self.palette;
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("The server said"),
            )
            .child(
                div()
                    .font_family("JetBrains Mono")
                    .text_size(px(11.))
                    .text_color(hsla(p.text_secondary))
                    .whitespace_normal()
                    .child(message.lines.join("\n")),
            )
            .children(message.link.clone().map(|link| {
                let caption = message
                    .link_caption
                    .clone()
                    .map(|c| format!("Open: {c}"))
                    .unwrap_or_else(|| "Open Link".into());
                Button::new("finish-open-link")
                    .primary()
                    .small()
                    .icon(Icon::empty().path("icons/external-link.svg"))
                    .label(caption)
                    .on_click(move |_, _, cx| cx.open_url(&link))
            }))
            .into_any_element()
    }
}

impl Render for TaskFinishView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let header = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(64.))
            .px(px(20.))
            .flex_none()
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title()),
                    )
                    .child(
                        div()
                            .font_family("JetBrains Mono")
                            .text_size(px(11.5))
                            .text_color(hsla(p.text_secondary))
                            .child(if self.branch.is_empty() {
                                " ".to_string()
                            } else {
                                format!("{} → {}", self.branch, self.base_ref())
                            }),
                    ),
            )
            .child(
                Button::new("finish-close")
                    .label("Close")
                    .on_click(self.act(|w, view, window, cx| {
                        let surface = Surface::TaskFinish(view);
                        w.close_surface(&surface, window, cx)
                    })),
            )
            .when(self.unavailable.is_none(), |d| {
                d.child(
                    Button::new("finish-run")
                        .primary()
                        .icon(Icon::empty().path("icons/git-merge.svg"))
                        .label(self.finish_title())
                        .disabled(self.is_running || self.is_loading)
                        .on_click(self.act(|w, view, window, cx| w.start_finish(view, window, cx))),
                )
            });
        // Swift's 720-point frame holds the 28-point margins too.
        let mut content = div().flex().flex_col().gap(px(18.)).max_w(px(664.));
        if self.is_loading {
            content = content.child(
                div()
                    .text_color(hsla(p.text_tertiary))
                    .child("Looking at the task…"),
            );
        } else if let Some(unavailable) = &self.unavailable {
            content = content.child(
                div()
                    .text_size(px(12.))
                    .text_color(hsla(p.text_secondary))
                    .whitespace_normal()
                    .child(unavailable.clone()),
            );
        } else {
            if self.landing.is_none() && !self.already_merged {
                content = content.child(self.landing_choice());
            }
            if !self.already_merged && self.effective_landing() == Landing::Merge {
                content = content.child(self.branch_options());
            }
            let mut steps = div()
                .flex()
                .flex_col()
                .rounded(px(metrics::RADIUS))
                .bg(hsla(p.panel))
                .border_1()
                .border_color(hsla(p.hairline));
            for (i, step) in STEPS.iter().enumerate() {
                if i > 0 {
                    steps = steps.child(div().h(px(1.)).bg(hsla(p.hairline)));
                }
                steps = steps.child(self.step_row(*step, cx));
            }
            content = content.child(steps);
            if let Some(message) = &self.server_message {
                content = content.child(self.server_reply(message));
            }
        }
        div()
            .id("task-finish")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.content))
            .text_color(hsla(p.text))
            .child(header)
            .child(
                div()
                    .id("task-finish-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(28.))
                    .py(px(20.))
                    .child(content),
            )
    }
}

// MARK: Running it

/// What loading the tab found.
struct Loaded {
    main: String,
    branch: String,
    base: Option<String>,
    remote: Option<String>,
    settings: Option<ProjectConfig>,
    merged: bool,
    published: bool,
    known: bool,
}

fn load(root: &str, trusted_main: bool) -> Loaded {
    let main = tasks::main_checkout_root(root);
    let record = tasks::record_for_path(root);
    let branch = operations::current_branch(root)
        .or_else(|| record.as_ref().map(|r| r.branch.clone()))
        .unwrap_or_default();
    let remotes = operations::remotes(&main);
    // Adopted tasks don't know their base: the remote's default branch.
    let mut base = record.as_ref().and_then(|r| r.base.clone());
    let mut remote = record.as_ref().and_then(|r| r.remote.clone());
    if base.is_none() {
        if let Some(fallback) = scoped_diff::default_base_branch(&main) {
            match remotes
                .iter()
                .find(|r| fallback.starts_with(&format!("{r}/")))
            {
                Some(found) => {
                    base = Some(fallback[found.len() + 1..].to_string());
                    remote = Some(found.clone());
                }
                None => base = Some(fallback),
            }
        }
    }
    let base_ref = match (&remote, &base) {
        (Some(r), Some(b)) => format!("{r}/{b}"),
        (_, b) => b.clone().unwrap_or_default(),
    };
    let tip = scoped_diff::resolve_commit(&main, &format!("refs/heads/{branch}"));
    let merged = !branch.is_empty()
        && base.is_some()
        && tip.is_some()
        && tip != record.as_ref().and_then(|r| r.start.clone())
        && operations::is_merged(
            &format!("refs/heads/{branch}"),
            &base_ref,
            &main,
            trusted_main,
        );
    let published = remote.as_ref().is_some_and(|r| {
        operations::resolve_ref(&format!("refs/remotes/{r}/{branch}"), &main).is_some()
    });
    Loaded {
        settings: tasks::project_config(root),
        main,
        branch,
        base,
        remote,
        merged,
        published,
        known: record.is_some(),
    }
}

impl Workbench {
    /// "Finish Task...": the Finish tab for the task workspace at `index`
    /// (else the active one), opened or shown.
    pub(crate) fn open_finish_task(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = index.unwrap_or(self.active);
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        if workspace.kind != Kind::Folder || !workspace.task.is_task {
            return toast(
                window,
                cx,
                ToastKind::Info,
                "Finish Task works in a task's workspace.".into(),
                None,
                None,
            );
        }
        let (root, id) = (workspace.root.to_string_lossy().into_owned(), workspace.id);
        if self.active != index {
            self.activate_workspace(index, window, cx);
        }
        let existing = self.workspaces[index].all_surfaces().find_map(|s| match s {
            Surface::TaskFinish(view) if view.read(cx).root == root => Some(view.clone()),
            _ => None,
        });
        let view = match existing {
            Some(view) => {
                let wanted = view.entity_id();
                self.reveal_tool(|s| surface_id(s) == wanted, window, cx);
                if view.read(cx).is_running {
                    return;
                }
                view
            }
            None => {
                let workbench = cx.entity().downgrade();
                let palette = self.palette.clone();
                let view = cx.new_view_finish(palette, workbench, id, root.clone());
                self.add_tab(index, Surface::TaskFinish(view.clone()), window, cx);
                view
            }
        };
        self.load_finish(view, window, cx);
    }

    fn load_finish(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = view.read(cx).root.clone();
        view.update(cx, |v, cx| {
            v.is_loading = true;
            cx.notify();
        });
        let main = tasks::main_checkout_root(&root);
        let trusted = self.trust.is_trusted(&main);
        let work = cx
            .background_executor()
            .spawn(async move { load(&root, trusted) });
        cx.spawn_in(window, async move |_, cx| {
            let found = work.await;
            view.update(cx, |v, cx| {
                v.main_root = found.main;
                v.branch = found.branch;
                v.base = found.base.clone().unwrap_or_default();
                v.remote = found.remote;
                v.check_script = found.settings.as_ref().and_then(|s| s.check_script.clone());
                v.landing = found.settings.as_ref().and_then(|s| s.landing);
                v.is_published = found.published;
                v.already_merged = found.merged;
                v.statuses.clear();
                v.resume_at = None;
                v.remedy = None;
                v.server_message = None;
                v.unavailable = None;
                if !found.known {
                    v.unavailable = Some(
                        "Impulse didn't start this worktree, so it doesn't finish it. Merge its branch \
                         yourself, then remove it with git worktree remove."
                            .into(),
                    );
                } else if v.branch.is_empty() || found.base.is_none() {
                    v.unavailable = Some(
                        "Finish needs the task on a branch, and a branch to land it in. Neither could be \
                         worked out here."
                            .into(),
                    );
                } else if found.merged {
                    let text = format!("Already merged into {}.", v.base_ref());
                    for step in [Step::Commit, Step::Sync, Step::Check, Step::Land] {
                        v.statuses.insert(step, Status::Done(text.clone()));
                    }
                }
                v.is_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn start_finish(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (merged, landing, chosen, remembers, main, resume) = {
            let v = view.read(cx);
            (
                v.already_merged,
                v.landing,
                v.chosen_landing,
                v.remembers_landing,
                v.main_root.clone(),
                v.resume_at,
            )
        };
        if merged {
            view.update(cx, |v, _| {
                v.chosen_landing = Landing::Merge;
                v.landing = Some(v.landing.unwrap_or(Landing::Merge));
            });
            return self.run_finish(view, Step::Update, window, cx);
        }
        if landing.is_none() {
            view.update(cx, |v, _| v.landing = Some(chosen));
            if remembers {
                save_landing(chosen, &main, cx);
            }
        }
        self.run_finish(view, resume.unwrap_or(Step::Commit), window, cx);
    }

    fn run_finish(
        &mut self,
        view: Entity<TaskFinishView>,
        first: Step,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        view.update(cx, |v, cx| {
            v.is_running = true;
            v.resume_at = None;
            v.remedy = None;
            for step in STEPS.iter().filter(|s| **s >= first) {
                v.statuses.insert(*step, Status::Pending);
            }
            cx.notify();
        });
        self.advance_finish(view, Some(first), window, cx);
    }

    /// Mark `step` with `status` and run the next one.
    fn proceed(
        &mut self,
        view: Entity<TaskFinishView>,
        step: Step,
        status: Status,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        view.update(cx, |v, cx| {
            v.statuses.insert(step, status);
            cx.notify();
        });
        self.advance_finish(view, step.next(), window, cx);
    }

    /// Stop at `step`: Continue starts again at `resume`.
    fn stop(
        view: &Entity<TaskFinishView>,
        step: Step,
        text: String,
        resume: Step,
        remedy: Option<Remedy>,
        cx: &mut gpui_kit::App,
    ) {
        view.update(cx, |v, cx| {
            v.statuses.insert(step, Status::Failed(text));
            v.resume_at = Some(resume);
            v.remedy = remedy;
            v.is_running = false;
            v.check = None;
            cx.notify();
        });
    }

    fn set_status(
        view: &Entity<TaskFinishView>,
        step: Step,
        status: Status,
        cx: &mut gpui_kit::App,
    ) {
        view.update(cx, |v, cx| {
            v.statuses.insert(step, status);
            cx.notify();
        });
    }

    /// Run `step`, then the next, until one stops or they're done.
    fn advance_finish(
        &mut self,
        view: Entity<TaskFinishView>,
        step: Option<Step>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(step) = step else {
            view.update(cx, |v, cx| {
                v.is_running = false;
                cx.notify();
            });
            return;
        };
        // An agent's Finish: landing, and for a task that's already merged
        // updating the main checkout and archiving, are the user's to start.
        let requester = view.update(cx, |v, _| {
            (step >= Step::Land)
                .then(|| v.stops_before_landing.take())
                .flatten()
        });
        if let Some(requester) = requester {
            return view.update(cx, |v, cx| {
                let text = if v.already_merged {
                    format!(
                        "{requester} asked to finish {}, which is already merged into {}. Click Clean Up to update {} and archive it.",
                        v.name(),
                        v.base_ref(),
                        v.main_name()
                    )
                } else {
                    format!(
                        "{requester} asked to finish {}. Click Continue to {}.",
                        v.name(),
                        if v.effective_landing() == Landing::Review {
                            "push it for review".to_string()
                        } else {
                            format!("land it in {}", v.base_ref())
                        }
                    )
                };
                v.statuses.insert(step, Status::Waiting(text));
                v.resume_at = Some(step);
                v.remedy = None;
                v.is_running = false;
                cx.notify();
            });
        }
        match step {
            Step::Commit => self.finish_commit(view, window, cx),
            Step::Sync => self.finish_sync(view, window, cx),
            Step::Check => self.finish_check(view, window, cx),
            Step::Land => self.finish_land(view, window, cx),
            Step::Update => self.finish_update(view, window, cx),
            Step::CleanUp => self.finish_clean_up(view, window, cx),
        }
    }

    /// Run `work` off the main thread, then `then` on the workbench.
    fn finish_off_main<T: Send + 'static>(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
        work: impl FnOnce() -> T + Send + 'static,
        then: impl FnOnce(&mut Workbench, Entity<TaskFinishView>, T, &mut Window, &mut Context<Workbench>)
            + 'static,
    ) {
        let task = cx.background_executor().spawn(async move { work() });
        cx.spawn_in(window, async move |workbench, cx| {
            let value = task.await;
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                then(workbench, view, value, window, cx)
            });
        })
        .detach();
    }

    fn finish_commit(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (root, name) = (view.read(cx).root.clone(), view.read(cx).name());
        Self::set_status(
            &view,
            Step::Commit,
            Status::Running(format!("Looking at {name}…")),
            cx,
        );
        // The agent that asked for this run is mid-turn: only others count.
        let requester = view.update(cx, |v, _| v.requester_terminal.take());
        if let Some(agent) = self.working_agents_in(&root, requester, cx).first() {
            return Self::stop(
                &view,
                Step::Commit,
                format!("{agent} is working in this task. Let it finish its turn, then Continue."),
                Step::Commit,
                None,
                cx,
            );
        }
        self.finish_off_main(view, window, cx, move || snapshot::snapshot(&root, false), |w, view, snapshot, window, cx| {
            let Some(snapshot) = snapshot else {
                return Self::stop(&view, Step::Commit, "The task's folder couldn't be read.".into(), Step::Commit, None, cx);
            };
            if snapshot.operation.is_some() || !snapshot.conflicted.is_empty() {
                return Self::stop(
                    &view,
                    Step::Commit,
                    "A merge (or rebase) is under way in the task. Finish it in the Changes panel, then Continue.".into(),
                    Step::Commit,
                    Some(Remedy::ShowChanges),
                    cx,
                );
            }
            let count = snapshot.changed_file_count();
            if count > 0 {
                return Self::stop(
                    &view,
                    Step::Commit,
                    format!(
                        "{count} uncommitted file{}. Commit (or stash) {}, then Continue.",
                        if count == 1 { "" } else { "s" },
                        if count == 1 { "it" } else { "them" }
                    ),
                    Step::Commit,
                    Some(Remedy::ShowChanges),
                    cx,
                );
            }
            w.proceed(view, Step::Commit, Status::Done("Everything is committed.".into()), window, cx);
        });
    }

    fn finish_sync(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (root, remote, target, branch) = {
            let v = view.read(cx);
            (
                v.root.clone(),
                v.remote.clone(),
                v.base_ref(),
                v.branch.clone(),
            )
        };
        let running = match &remote {
            Some(r) => format!("Fetching {r}…"),
            None => format!("Merging {target}…"),
        };
        Self::set_status(&view, Step::Sync, Status::Running(running), cx);
        enum Synced {
            FetchFailed(String, String),
            UpToDate,
            Merged(usize),
            Conflicts(usize),
            Failed(String),
        }
        let work = {
            let target = target.clone();
            move || {
                if let Some(remote) = &remote {
                    if let Err(error) = operations::fetch(
                        false,
                        Some(remote),
                        &root,
                        Duration::from_secs(300),
                        None,
                    ) {
                        return Synced::FetchFailed(remote.clone(), error.message());
                    }
                }
                if landing::is_ancestor(&target, "HEAD", &root) {
                    return Synced::UpToDate;
                }
                let count = operations::commit_count("HEAD", &target, &root).unwrap_or(0);
                let merged = operations::merge(&target, false, &root);
                let conflicted = snapshot::snapshot(&root, false)
                    .map(|s| s.conflicted.len())
                    .unwrap_or(0);
                match merged {
                    Ok(()) => Synced::Merged(count),
                    Err(_) if conflicted > 0 => Synced::Conflicts(conflicted),
                    Err(error) => Synced::Failed(error.message()),
                }
            }
        };
        self.finish_off_main(view, window, cx, work, move |w, view, synced, window, cx| match synced {
            Synced::FetchFailed(remote, error) => {
                Self::stop(&view, Step::Sync, format!("Couldn't fetch {remote}: {error}"), Step::Sync, None, cx)
            }
            Synced::UpToDate => w.proceed(view, Step::Sync, Status::Done(format!("Up to date with {target}.")), window, cx),
            Synced::Merged(count) => w.proceed(
                view,
                Step::Sync,
                Status::Done(format!(
                    "Merged {count} new commit{} from {target} into {branch}.",
                    if count == 1 { "" } else { "s" }
                )),
                window,
                cx,
            ),
            Synced::Conflicts(conflicted) => Self::stop(
                &view,
                Step::Sync,
                format!(
                    "Merging {target} into {branch} conflicts in {conflicted} file{}. Resolve them in the Changes panel and commit the merge (or ask the task's agent to), then Continue.",
                    if conflicted == 1 { "" } else { "s" }
                ),
                Step::Commit,
                Some(Remedy::ShowChanges),
                cx,
            ),
            Synced::Failed(error) => {
                Self::stop(&view, Step::Sync, format!("Couldn't merge {target}: {error}"), Step::Sync, None, cx)
            }
        });
    }

    fn finish_check(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let no_script =
            "No check script. Set one up so Finish can tell the merged code still works.";
        if view.read(cx).check_script.is_none() {
            return self.proceed(
                view,
                Step::Check,
                Status::Skipped(no_script.into()),
                window,
                cx,
            );
        }
        Self::set_status(
            &view,
            Step::Check,
            Status::Running("Waiting for the project settings to be trusted…".into()),
            cx,
        );
        let root = view.read(cx).root.clone();
        self.trust_project_config(
            &root.clone(),
            window,
            cx,
            Box::new(move |w, config, window, cx| {
                let Some(config) = config else {
                    return Self::stop(
                        &view,
                        Step::Check,
                        "The project settings weren't trusted, so the check didn't run.".into(),
                        Step::Check,
                        Some(Remedy::SkipCheck),
                        cx,
                    );
                };
                // The trusted file's script, not the one read when the tab
                // opened: the file can change in between.
                view.update(cx, |v, _| v.check_script = config.check_script.clone());
                let Some(script) = config.check_script else {
                    return w.proceed(
                        view,
                        Step::Check,
                        Status::Skipped(no_script.into()),
                        window,
                        cx,
                    );
                };
                let id = view.read(cx).workspace_id;
                let Some(index) = w.workspaces.iter().position(|ws| ws.id == id) else {
                    return Self::stop(
                        &view,
                        Step::Check,
                        "The task's workspace was closed.".into(),
                        Step::Check,
                        None,
                        cx,
                    );
                };
                w.activate_workspace(index, window, cx);
                let terminal = w.new_terminal(
                    Path::new(&root).to_path_buf(),
                    vec![script.clone()],
                    window,
                    cx,
                );
                let Surface::Terminal(entity) = terminal.clone() else {
                    return;
                };
                w.add_tab(index, terminal, window, cx);
                let watched = view.clone();
                let subscription = cx.subscribe_in(
                    &entity,
                    window,
                    move |w, _, event: &TerminalViewEvent, window, cx| {
                        let code = match event {
                            TerminalViewEvent::CommandEnded(code) => Some(code.unwrap_or(1)),
                            TerminalViewEvent::Exited => None,
                            _ => return,
                        };
                        w.end_check(watched.clone(), code, window, cx);
                    },
                );
                view.update(cx, |v, cx| {
                    v.statuses.insert(
                        Step::Check,
                        Status::Running(format!("Running {script} in a terminal tab…")),
                    );
                    v.remedy = Some(Remedy::SkipCheck);
                    v.check = Some((subscription, Instant::now()));
                    cx.notify();
                });
            }),
        );
    }

    /// The check's command ended (`None`: its terminal closed first).
    fn end_check(
        &mut self,
        view: Entity<TaskFinishView>,
        code: Option<i32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, started)) = view.update(cx, |v, _| v.check.take()) else {
            return;
        };
        view.update(cx, |v, _| v.remedy = None);
        self.reveal_finish(&view, window, cx);
        match code {
            Some(0) => {
                let seconds = started.elapsed().as_secs_f64().round() as u64;
                let took = if seconds < 60 {
                    format!("{seconds}s")
                } else {
                    format!("{}m {}s", seconds / 60, seconds % 60)
                };
                self.proceed(view, Step::Check, Status::Done(format!("Passed in {took}.")), window, cx)
            }
            Some(code) => Self::stop(
                &view,
                Step::Check,
                format!("Failed (exit status {code}). See its terminal tab; fix it, commit, then Continue."),
                Step::Commit,
                None,
                cx,
            ),
            None => Self::stop(
                &view,
                Step::Check,
                "Its terminal closed before the check finished.".into(),
                Step::Check,
                Some(Remedy::SkipCheck),
                cx,
            ),
        }
    }

    /// Bring a task's Finish tab to the front.
    fn reveal_finish(
        &mut self,
        view: &Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let wanted = view.entity_id();
        let found = self
            .workspaces
            .iter()
            .enumerate()
            .find_map(|(w, workspace)| {
                workspace.tabs.iter().enumerate().find_map(|(t, tab)| {
                    tab.pane_of(|s| surface_id(s) == wanted)
                        .map(|pane| (w, t, pane))
                })
            });
        if let Some((w, t, pane)) = found {
            self.active = w;
            self.workspaces[w].active = t;
            self.workspaces[w].tabs[t].focused = pane;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn finish_land(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (root, main, branch, base, remote, target, review, published, main_name) = {
            let v = view.read(cx);
            (
                v.root.clone(),
                v.main_root.clone(),
                v.branch.clone(),
                v.base.clone(),
                v.remote.clone(),
                v.base_ref(),
                v.effective_landing() == Landing::Review,
                v.is_published,
                v.main_name(),
            )
        };
        if review {
            let Some(remote) = remote.or_else(|| operations::default_remote(&root, Some(&branch)))
            else {
                return Self::stop(
                    &view,
                    Step::Land,
                    "Push for Review needs a remote, and this repository has none.".into(),
                    Step::Land,
                    None,
                    cx,
                );
            };
            Self::set_status(
                &view,
                Step::Land,
                Status::Running(format!("Pushing {branch} to {remote}…")),
                cx,
            );
            let work = {
                let (root, remote, branch) = (root.clone(), remote.clone(), branch.clone());
                move || {
                    let mut lines: Vec<String> = Vec::new();
                    let mut collect = |line: &str| lines.push(line.to_string());
                    let options = PushOptions {
                        set_upstream: !published,
                        remote: remote.clone(),
                        branch: (!published).then(|| branch.clone()),
                        ..Default::default()
                    };
                    let pushed = operations::push(&options, &root, Some(&mut collect));
                    if pushed.is_ok() {
                        tasks::mark_pushed_for_review(&root, &main);
                    }
                    (pushed.map_err(|e| e.message()), lines)
                }
            };
            return self.finish_off_main(view, window, cx, work, move |_, view, (pushed, lines), _, cx| {
                view.update(cx, |v, _| v.server_message = GitServerMessage::parse(&lines));
                match pushed {
                    Ok(()) => view.update(cx, |v, cx| {
                        v.is_published = true;
                        v.pushed_for_review = true;
                        v.statuses.insert(
                            Step::Land,
                            Status::Done(format!("Pushed {branch} to {remote}. Open a merge request for it on your git host.")),
                        );
                        v.statuses.insert(Step::Update, Status::Skipped(format!("After {branch} is merged into {target}.")));
                        v.statuses.insert(Step::CleanUp, Status::Skipped(format!("Once {branch} is merged, Impulse offers to clean up.")));
                        v.is_running = false;
                        cx.notify();
                    }),
                    Err(error) => Self::stop(&view, Step::Land, format!("Couldn't push: {error}"), Step::Land, None, cx),
                }
            });
        }
        Self::set_status(
            &view,
            Step::Land,
            Status::Running(match remote {
                None => format!("Merging into {base}…"),
                Some(_) => format!("Merging into {target} and pushing…"),
            }),
            cx,
        );
        let work = {
            let (branch, base, remote, main) =
                (branch.clone(), base.clone(), remote.clone(), main.clone());
            move || match &remote {
                Some(remote) => landing::merge_and_push(&branch, &base, remote, &main, None),
                None => landing::merge_locally(&branch, &base, &main),
            }
        };
        self.finish_off_main(view, window, cx, work, move |w, view, result, window, cx| match result {
            Ok(landed) => {
                let short: String = landed.commit.chars().take(7).collect();
                let moved = if landed.retried {
                    format!(" {target} had moved meanwhile, so its new commits were merged in first.")
                } else {
                    String::new()
                };
                view.update(cx, |v, _| {
                    v.landed_commit = Some(landed.commit.clone());
                    v.server_message = GitServerMessage::parse(&landed.output);
                });
                let text = match remote {
                    None => format!("Merged into {base} as {short}."),
                    Some(_) => format!("Merged as {short} and pushed to {target}.{moved}"),
                };
                w.proceed(view, Step::Land, Status::Done(text), window, cx)
            }
            Err(Failure::NothingToLand) => w.proceed(
                view,
                Step::Land,
                Status::Done(format!("Nothing to merge: {target} already has {branch}'s commits.")),
                window,
                cx,
            ),
            Err(Failure::Conflicts(files)) => Self::stop(
                &view,
                Step::Land,
                format!(
                    "{target} changed since the sync, and merging now conflicts in {}. Nothing was pushed. Continue syncs again.",
                    if files.len() == 1 { files[0].clone() } else { format!("{} files", files.len()) }
                ),
                Step::Sync,
                None,
                cx,
            ),
            Err(Failure::Refused(output)) => {
                let lines: Vec<&str> = output.split('\n').collect();
                view.update(cx, |v, _| v.server_message = GitServerMessage::parse(&lines));
                Self::stop(
                    &view,
                    Step::Land,
                    format!("The server turned the push to {base} down (a protected branch, perhaps). Nothing was pushed."),
                    Step::Land,
                    Some(Remedy::PushForReview),
                    cx,
                )
            }
            Err(Failure::Untracked(files)) => Self::stop(
                &view,
                Step::Land,
                format!(
                    "{main_name} has untracked {}, which {branch} adds. Without a remote, Finish merges in {main_name}: move {} aside there, then Continue.",
                    landing::naming(&files),
                    if files.len() == 1 { "it" } else { "them" }
                ),
                Step::Land,
                None,
                cx,
            ),
            Err(failure) => {
                let hint = if remote.is_none() {
                    format!(" Without a remote, Finish merges in {main_name}: commit or stash there, switch it to {base}, then Continue.")
                } else {
                    String::new()
                };
                Self::stop(&view, Step::Land, failure.message() + &hint, Step::Land, None, cx)
            }
        });
    }

    fn finish_update(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (main, base, remote, name) = {
            let v = view.read(cx);
            (
                v.main_root.clone(),
                v.base.clone(),
                v.remote.clone(),
                v.main_name(),
            )
        };
        let Some(remote) = remote else {
            return self.proceed(
                view,
                Step::Update,
                Status::Done(format!("The merge is in {name}.")),
                window,
                cx,
            );
        };
        let target = format!("{remote}/{base}");
        Self::set_status(
            &view,
            Step::Update,
            Status::Running(format!("Updating {name}…")),
            cx,
        );
        let work = move || {
            let current = operations::current_branch(&main);
            // Untracked files don't hold it back: git refuses to overwrite one.
            let dirty = snapshot::snapshot(&main, false)
                .map(|s| s.tracked_change_count())
                .unwrap_or(0);
            if current.as_deref() != Some(base.as_str()) {
                return Status::Skipped(format!(
                    "{name} is on {}, so it was left alone.",
                    current.as_deref().unwrap_or("a detached commit")
                ));
            }
            if dirty > 0 {
                return Status::Skipped(format!(
                    "{name} has {dirty} uncommitted file{}, so it wasn't updated. Pull when you're ready.",
                    if dirty == 1 { "" } else { "s" }
                ));
            }
            match operations::fast_forward(&target, &main) {
                Ok(()) => Status::Done(format!("Fast-forwarded {name} to {target}.")),
                Err(error) => {
                    let untracked = landing::untracked_in_the_way(error.output().unwrap_or(""));
                    Status::Skipped(if untracked.is_empty() {
                        format!("{name} has commits of its own on {base}, so it wasn't updated. Pull when you're ready.")
                    } else {
                        format!(
                            "{name} has untracked {}, which {target} now tracks, so it wasn't updated. Move {} aside, then pull.",
                            landing::naming(&untracked),
                            if untracked.len() == 1 { "it" } else { "them" }
                        )
                    })
                }
            }
        };
        self.finish_off_main(view, window, cx, work, |w, view, status, window, cx| {
            w.proceed(view, Step::Update, status, window, cx)
        });
    }

    fn finish_clean_up(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = view.read(cx).workspace_id;
        if !self.workspaces.iter().any(|w| w.id == id) {
            return Self::stop(
                &view,
                Step::CleanUp,
                "The task's workspace was closed.".into(),
                Step::CleanUp,
                None,
                cx,
            );
        }
        let (root, name) = (view.read(cx).root.clone(), view.read(cx).name());
        let trusted = self.trust.is_trusted(&root);
        Self::set_status(
            &view,
            Step::CleanUp,
            Status::Running(format!("Looking at what's left in {name}…")),
            cx,
        );
        // What removing the folder would delete for good: a repository of
        // its own stops it; ignored files are asked about.
        let work = move || {
            (
                ignored::nested_repositories(&root, trusted),
                ignored::ignored_entry_list(&root, trusted),
            )
        };
        self.finish_off_main(view, window, cx, work, move |w, view, (nested, ignored), window, cx| {
            if !nested.as_ref().is_some_and(|n| n.is_empty()) {
                return Self::stop(
                    &view,
                    Step::CleanUp,
                    format!("{} Then Continue.", nested_repository_message(nested.as_deref())),
                    Step::CleanUp,
                    None,
                    cx,
                );
            }
            let Some(warning) = ignored_files_warning(ignored.as_deref(), "Restore Task") else {
                return w.archive_finished_task(view, window, cx);
            };
            let workbench = cx.entity().downgrade();
            confirm(
                window,
                cx,
                format!("Archive {name}?"),
                format!("The task's folder is removed. {warning}"),
                "Archive",
                "Cancel",
                true,
                move |yes, window, cx| {
                    if !yes {
                        return Self::stop(
                            &view,
                            Step::CleanUp,
                            "The task wasn't archived, so its ignored files are still there. Continue when you're ready.".into(),
                            Step::CleanUp,
                            None,
                            cx,
                        );
                    }
                    let _ = workbench.update(cx, |w, cx| w.archive_finished_task(view, window, cx));
                },
            );
        });
    }

    /// Close the task's workspace, archive the task, and delete its branch
    /// (checked again first: it has to be merged still).
    fn archive_finished_task(
        &mut self,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (id, root, main, branch, remote, target, delete_branch, delete_remote, landed) = {
            let v = view.read(cx);
            (
                v.workspace_id,
                v.root.clone(),
                v.main_root.clone(),
                v.branch.clone(),
                v.remote.clone(),
                v.base_ref(),
                v.deletes_branch,
                v.deletes_branch && v.deletes_remote_branch && v.is_published,
                v.landed_commit.clone(),
            )
        };
        if !self.workspaces.iter().any(|w| w.id == id) {
            return Self::stop(
                &view,
                Step::CleanUp,
                "The task's workspace was closed.".into(),
                Step::CleanUp,
                None,
                cx,
            );
        }
        let script = tasks::already_trusted_project_config(&root).and_then(|c| c.archive_script);
        let root2 = root.clone();
        self.confirm_archive_script(
            script,
            &root2,
            &branch.clone(),
            window,
            cx,
            Box::new(move |w, script, window, cx| {
                Self::set_status(&view, Step::CleanUp, Status::Running("Closing the task's workspace…".into()), cx);
                // The Finish tab goes with the workspace, so the outcome is a toast.
                if !w.close_workspace_for_task(id, window, cx) {
                    return Self::stop(
                        &view,
                        Step::CleanUp,
                        "The workspace stayed open, so the task wasn't archived. Continue when you're ready.".into(),
                        Step::CleanUp,
                        None,
                        cx,
                    );
                }
                let trust = w.trust.clone();
                let work = {
                    let (root, branch, main, target, landed) = (root.clone(), branch.clone(), main.clone(), target.clone(), landed.clone());
                    move || {
                        let removed = remove_task(&root, &branch, false, script.as_deref(), &trust);
                        let mut kept = Vec::new();
                        let result = removed.result.map(|mut task| {
                            if delete_branch {
                                let targets: Vec<&str> = landed.iter().map(String::as_str).chain([target.as_str()]).collect();
                                let cleanup = landing::delete_landed_branch(
                                    &branch,
                                    &targets,
                                    if delete_remote { remote.as_deref() } else { None },
                                    &main,
                                    trust.is_trusted(&main),
                                );
                                task.deleted_branch_commit = cleanup.deleted_commit;
                                kept = cleanup.kept;
                            }
                            task
                        });
                        (result, removed.script_failed, kept)
                    }
                };
                let task = cx.background_executor().spawn(async move { work() });
                cx.spawn_in(window, async move |workbench, cx| {
                    let (result, script_failed, kept) = task.await;
                    let _ = workbench.update_in(cx, |w, window, cx| {
                        if script_failed {
                            toast(window, cx, ToastKind::Warning, "The archive script failed; archiving anyway.".into(), None, None);
                        }
                        match result {
                            Err(error) => toast(window, cx, ToastKind::Warning, format!("Couldn't archive {branch}"), Some(error), None),
                            Ok(task) => {
                                w.refresh_task_states(cx);
                                let what = if task.deleted_branch_commit.is_some() {
                                    "The task is archived and its branch deleted."
                                } else {
                                    "The task is archived; its branch is kept."
                                };
                                let landed = landed
                                    .map(|c| format!(" as {}", c.chars().take(7).collect::<String>()))
                                    .unwrap_or_default();
                                let detail = std::iter::once(what.to_string()).chain(kept.iter().cloned()).collect::<Vec<_>>().join(" ");
                                let restore = w.restore_action(vec![task], cx);
                                toast(
                                    window,
                                    cx,
                                    if kept.is_empty() { ToastKind::Success } else { ToastKind::Warning },
                                    format!("Finished {branch}: merged into {target}{landed}."),
                                    Some(detail),
                                    Some(("Restore Task", restore)),
                                );
                            }
                        }
                    });
                })
                .detach();
            }),
        );
    }

    fn apply_finish_remedy(
        &mut self,
        remedy: Remedy,
        view: Entity<TaskFinishView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match remedy {
            Remedy::ShowChanges => {
                let id = view.read(cx).workspace_id;
                if let Some(index) = self.workspaces.iter().position(|w| w.id == id) {
                    self.activate_workspace(index, window, cx);
                }
                self.run_command("show_changes", window, cx);
            }
            Remedy::SkipCheck => {
                view.update(cx, |v, cx| {
                    v.check = None;
                    v.remedy = None;
                    v.statuses
                        .insert(Step::Check, Status::Skipped("Skipped.".into()));
                    v.is_running = true;
                    v.resume_at = None;
                    cx.notify();
                });
                self.advance_finish(view, Some(Step::Land), window, cx);
            }
            Remedy::PushForReview => {
                view.update(cx, |v, _| {
                    v.landing = Some(Landing::Review);
                    v.chosen_landing = Landing::Review;
                    v.server_message = None;
                });
                self.run_finish(view, Step::Land, window, cx);
            }
        }
    }
}

/// Save Finish's answer in the repository's local settings. The file stays
/// trusted if it was: a landing choice runs nothing.
fn save_landing(landing: Landing, root: &str, cx: &mut Context<Workbench>) {
    let root = root.to_string();
    cx.background_executor()
        .spawn(async move {
            let Some(common) = branches::common_git_directory(&root) else {
                return;
            };
            let path = ProjectConfig::local_path(&common);
            let was_trusted = match tasks::load_project_config(&root) {
                Some(loaded) => match &loaded.local {
                    Some(local) => tasks::project_trust().is_trusted(
                        &tasks::trust_key(&local.path, &loaded, &root),
                        &local.digest,
                    ),
                    None => true,
                },
                None => true,
            };
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            if let Some(parent) = Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let text = impulse_kit::project::settings_file::setting(landing, &existing);
            if std::fs::write(&path, text).is_err() || !was_trusted {
                return;
            }
            if let Some(after) = tasks::load_project_config(&root) {
                if let Some(local) = &after.local {
                    let mut trust = tasks::project_trust();
                    trust.trust(&tasks::trust_key(&local.path, &after, &root), &local.digest);
                    tasks::save_project_trust(&trust);
                }
            }
        })
        .detach();
}

#[cfg(debug_assertions)]
impl Workbench {
    /// For the headless snapshots (`debug_snapshot`): press Finish (or
    /// Continue) on every Finish tab, `merge` or `review` answering the
    /// landing question first.
    pub(crate) fn debug_finish(
        &mut self,
        landing: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let views: Vec<Entity<TaskFinishView>> = self
            .workspaces
            .iter()
            .flat_map(|w| w.all_surfaces())
            .filter_map(|s| match s {
                Surface::TaskFinish(view) => Some(view.clone()),
                _ => None,
            })
            .collect();
        for view in views {
            match landing {
                Some("merge") => view.update(cx, |v, _| v.chosen_landing = Landing::Merge),
                Some("review") => view.update(cx, |v, _| v.chosen_landing = Landing::Review),
                _ => {}
            }
            self.start_finish(view, window, cx);
        }
    }
}

/// Making the view (kept apart so `open_finish_task` reads simply).
trait NewFinishView {
    fn new_view_finish(
        &mut self,
        palette: ChromePalette,
        workbench: WeakEntity<Workbench>,
        workspace_id: u64,
        root: String,
    ) -> Entity<TaskFinishView>;
}

impl NewFinishView for Context<'_, Workbench> {
    fn new_view_finish(
        &mut self,
        palette: ChromePalette,
        workbench: WeakEntity<Workbench>,
        workspace_id: u64,
        root: String,
    ) -> Entity<TaskFinishView> {
        self.new(|cx| TaskFinishView {
            palette,
            workbench,
            this: cx.entity().downgrade(),
            focus: cx.focus_handle(),
            workspace_id,
            root,
            main_root: String::new(),
            branch: String::new(),
            base: String::new(),
            remote: None,
            check_script: None,
            landing: None,
            chosen_landing: Landing::Merge,
            remembers_landing: true,
            deletes_branch: true,
            deletes_remote_branch: true,
            is_published: false,
            already_merged: false,
            unavailable: None,
            is_loading: true,
            is_running: false,
            statuses: HashMap::new(),
            resume_at: None,
            remedy: None,
            server_message: None,
            landed_commit: None,
            pushed_for_review: false,
            check: None,
            stops_before_landing: None,
            requester_terminal: None,
        })
    }
}
