//! The sidebar's Changes panel (the Swift `ChangesPanelView` and
//! `CommitComposer`): the branch with ahead and behind, pull, push and
//! fetch; the operation in progress with Continue, Skip and Abort; the
//! Conflicts, Staged, Changes, Untracked and Stashes sections with the
//! status letter of each file in git's colors (`ChromePalette::git_*`);
//! staging, unstaging and discarding a file or all of them; the list's
//! keys (arrows, Space to stage or unstage, Return for Review, secondary+
//! Return for the file, alt+Return for the Diff View, Delete to discard,
//! Escape back to the pane); and the commit
//! message composer, whose button commits (and pushes, with
//! `git_commit_and_push`; secondary+shift+Enter does the other one).
//!
//! `GitPanel` is the window's side of it: which repository each workspace
//! is in (`sync_git`, run on every render, so Scratch follows its tab),
//! the states it holds, and the panel's own view state.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::Icon;
use gpui_kit::{
    actions, div, img, prelude::*, px, AnyElement, App, ClipboardItem, Context, ElementId, Entity,
    FocusHandle, FontWeight, KeyBinding, KeyDownEvent, ListAlignment, ListState, SharedString,
    Subscription, WeakEntity, Window,
};
use impulse_git::operations::{self, CommitOptions, OperationAction, StashEntry};
use impulse_git::{ChangeStatus, FileChange, RepoSnapshot};
use impulse_kit::chrome::{metrics, Rgba};

use crate::context_menu::{DropdownMenuButton as _, MenuSpec, RightClickMenu as _};
use crate::repository_state::{RepositoryEvent, RepositoryState, RepositoryStore};
use crate::settings_store::SettingsStore;
use crate::workbench::Workbench;
use crate::workbench_chrome::{hsla, icon, ButtonKind};
use crate::workspace::Kind;

actions!(changes, [CommitDefault, CommitOther]);

const CONTEXT: &str = "CommitMessage";

/// Secondary+Enter commits (and pushes, with the setting);
/// secondary+shift+Enter does the other one.
pub fn bind_keys(cx: &mut App) {
    let c = Some("CommitMessage > Input");
    cx.bind_keys([
        KeyBinding::new("secondary-enter", CommitDefault, c),
        KeyBinding::new("secondary-shift-enter", CommitOther, c),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Section {
    Conflicted,
    Staged,
    Unstaged,
    Untracked,
}

/// The sections in the order listed.
const SECTIONS: [Section; 4] = [
    Section::Conflicted,
    Section::Staged,
    Section::Unstaged,
    Section::Untracked,
];

impl Section {
    fn changes(self, snapshot: &RepoSnapshot) -> &[FileChange] {
        match self {
            Section::Conflicted => &snapshot.conflicted,
            Section::Staged => &snapshot.staged,
            Section::Unstaged => &snapshot.unstaged,
            Section::Untracked => &snapshot.untracked,
        }
    }

    /// The working copy differs from the index (what the Diff View shows).
    fn has_working_copy_diff(self) -> bool {
        matches!(self, Section::Unstaged | Section::Untracked)
    }

    /// The Review a change in this section opens in.
    fn review_scope(self) -> impulse_git::types::DiffScope {
        use impulse_git::types::DiffScope;
        match self {
            Section::Staged => DiffScope::Staged,
            Section::Conflicted => DiffScope::Uncommitted,
            Section::Unstaged | Section::Untracked => DiffScope::Unstaged,
        }
    }
}

/// A file row as listed: its selection key, its change and section, and
/// its index among the list's children (headers included).
struct ListedRow {
    key: String,
    change: FileChange,
    section: Section,
    child: usize,
}

/// An item of the Changes list, as `GitPanel::list_state` counts them.
#[derive(Clone, PartialEq)]
enum ChangeItem {
    Header(Section),
    /// A file: its index in the section, and its path.
    Row(Section, usize, String),
    Stashes,
    Stash(usize),
}

fn row_key(section: Section, path: &str) -> String {
    format!("{section:?}-{path}")
}

struct Held {
    state: Entity<RepositoryState>,
    _observing: Subscription,
    _events: Subscription,
}

pub(crate) struct GitPanel {
    /// The active workspace's repository.
    pub active: Option<Entity<RepositoryState>>,
    held: HashMap<String, Held>,
    pub committing: bool,
    message: Entity<TextareaState>,
    amend: bool,
    sign_off: bool,
    skip_hooks: bool,
    collapsed: HashSet<Section>,
    stashes_expanded: bool,
    stashes: Vec<StashEntry>,
    /// The repository root and stash count the list was read for.
    stashes_for: Option<(String, usize)>,
    /// Recently used commit messages per repository, newest first (↑ in an
    /// empty composer), kept between launches.
    history: HashMap<String, Vec<String>>,
    history_index: Option<usize>,
    recalled: String,
    selection: Option<String>,
    /// The file list's keyboard focus (Swift's `listFocused`).
    list_focus: FocusHandle,
    /// Only the rows on screen are built (Swift's List).
    list_state: ListState,
    list_items: RefCell<Vec<ChangeItem>>,
    _message_events: Subscription,
}

impl GitPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Workbench>) -> Self {
        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .placeholder("Commit message")
        });
        let events = cx.subscribe_in(
            &message,
            window,
            |workbench: &mut Workbench, state, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    // Editing a recalled message makes it the user's own.
                    let git = &mut workbench.git;
                    if git.history_index.is_some() && state.read(cx).value() != git.recalled {
                        git.history_index = None;
                    }
                    cx.notify();
                }
            },
        );
        GitPanel {
            active: None,
            held: HashMap::new(),
            committing: false,
            message,
            amend: false,
            sign_off: false,
            skip_hooks: false,
            collapsed: HashSet::new(),
            stashes_expanded: false,
            stashes: Vec::new(),
            stashes_for: None,
            history: load_messages(),
            history_index: None,
            recalled: String::new(),
            selection: None,
            list_focus: cx.focus_handle(),
            list_state: ListState::new(0, ListAlignment::Top, px(200.)),
            list_items: RefCell::new(Vec::new()),
            _message_events: events,
        }
    }

    pub fn record_message(&mut self, root: &str, message: &str) {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return;
        }
        let list = self.history.entry(root.to_string()).or_default();
        list.retain(|m| m != trimmed);
        list.insert(0, trimmed.to_string());
        list.truncate(25);
        save_messages(&self.history);
    }

    /// The status letter and color of a path in the file tree.
    pub fn tree_status(
        &self,
        path: &std::path::Path,
        palette: &impulse_kit::chrome::ChromePalette,
        ignored: Rgba,
        cx: &App,
    ) -> Option<(&'static str, Rgba)> {
        let repo = self.active.as_ref()?.read(cx);
        Some(match repo.status_of(path)? {
            "M" => ("M", palette.git_modified),
            "A" => ("A", palette.git_added),
            "?" => ("?", palette.git_added),
            "D" => ("D", palette.git_deleted),
            "R" => ("R", palette.git_renamed),
            "C" => ("C", palette.git_conflict),
            // Ignored: no letter, the name in the theme's ignored color.
            "I" => ("", ignored),
            _ => return None,
        })
    }
}

fn status_color(status: ChangeStatus, p: &impulse_kit::chrome::ChromePalette) -> Rgba {
    match status {
        ChangeStatus::Added | ChangeStatus::Untracked => p.git_added,
        ChangeStatus::Deleted => p.git_deleted,
        ChangeStatus::Renamed => p.git_renamed,
        ChangeStatus::Conflicted => p.git_conflict,
        _ => p.git_modified,
    }
}

fn branch_label(snapshot: Option<&RepoSnapshot>) -> String {
    let Some(snapshot) = snapshot else {
        return "…".into();
    };
    if let Some(branch) = &snapshot.branch {
        return branch.clone();
    }
    match &snapshot.head_oid {
        Some(head) => format!("detached at {}", &head[..head.len().min(7)]),
        None => "no branch".into(),
    }
}

type Act = Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>;

/// Recent commit messages per repository, kept between launches (Swift's
/// `CommitMessageHistory`): `commit-messages.json` in the data directory.
fn messages_file() -> std::path::PathBuf {
    impulse_kit::paths::data_dir(cfg!(debug_assertions)).join("commit-messages.json")
}

fn load_messages() -> HashMap<String, Vec<String>> {
    std::fs::read(messages_file())
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

fn save_messages(history: &HashMap<String, Vec<String>>) {
    if std::env::var_os("IMPULSE_NO_PERSISTENCE").is_some() {
        return;
    }
    if let Ok(data) = serde_json::to_vec_pretty(history) {
        let _ = impulse_kit::settings::write_private_file(&messages_file(), &data);
    }
}

/// A menu item's handler that runs `act` on the workbench.
fn on_workbench(
    workbench: &WeakEntity<Workbench>,
    act: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let workbench = workbench.clone();
    move |window, cx| {
        let _ = workbench.update(cx, |workbench, cx| act(workbench, window, cx));
    }
}

impl Workbench {
    // MARK: Repositories

    /// Hold the repository of every folder workspace and of the active one
    /// (Scratch's follows its tab), and note which is active.
    pub(crate) fn sync_git(&mut self, cx: &mut Context<Self>) {
        let mut dirs = Vec::new();
        for (index, workspace) in self.workspaces.iter().enumerate() {
            if index == self.active {
                dirs.push((index, self.tree_root(cx)));
            } else if workspace.kind == Kind::Folder {
                dirs.push((index, workspace.root.clone()));
            }
        }
        let roots: Vec<(usize, Option<String>)> = dirs
            .into_iter()
            .map(|(index, dir)| (index, RepositoryStore::root_for(&dir, cx)))
            .collect();
        let wanted: HashSet<String> = roots.iter().filter_map(|(_, r)| r.clone()).collect();
        self.git.held.retain(|root, _| wanted.contains(root));
        for root in &wanted {
            if self.git.held.contains_key(root) {
                continue;
            }
            let state = RepositoryStore::state(root, cx);
            let observing = cx.observe(&state, |workbench, _, cx| {
                workbench.sync_branches(cx);
                cx.notify();
            });
            let events = cx.subscribe(&state, |workbench, state, event, cx| match event {
                // Files changed with HEAD: list the tree's folders again, and
                // offer to bring dependencies up to date.
                RepositoryEvent::HeadMoved { from, to } => {
                    workbench.reload_tree(cx);
                    let root = state.read(cx).root.clone();
                    workbench.offer_dependency_steps(root, from.clone(), to.clone(), cx);
                }
                RepositoryEvent::FilesChanged => workbench.schedule_task_refresh(cx),
            });
            self.git.held.insert(
                root.clone(),
                Held {
                    state,
                    _observing: observing,
                    _events: events,
                },
            );
        }
        let ask = SettingsStore::get(cx).ask_to_trust_folders;
        for (root, held) in &self.git.held {
            let trusted = !ask || self.trust.is_trusted(root);
            if held.state.read(cx).trusted != trusted {
                held.state.update(cx, |state, cx| {
                    state.trusted = trusted;
                    cx.notify();
                });
            }
        }
        self.git.active = roots
            .iter()
            .find(|(index, _)| *index == self.active)
            .and_then(|(_, root)| root.as_ref())
            .and_then(|root| self.git.held.get(root))
            .map(|held| held.state.clone());
        self.load_stashes_if_needed(cx);
    }

    /// Folder workspaces show their repository's branch (the titlebar, the
    /// status bar, the sidebar rows), updated when HEAD moves.
    pub(crate) fn sync_branches(&mut self, cx: &mut Context<Self>) {
        for index in 0..self.workspaces.len() {
            if self.workspaces[index].kind != Kind::Folder {
                continue;
            }
            let root = self.workspaces[index].root.clone();
            let Some(repo_root) = RepositoryStore::root_for(&root, cx) else {
                continue;
            };
            let Some(held) = self.git.held.get(&repo_root) else {
                continue;
            };
            let state = held.state.read(cx);
            if state.loaded {
                self.workspaces[index].branch = state.branch_label().unwrap_or_default();
            }
        }
    }

    /// `↑2 ↓1` beside the titlebar's branch.
    pub(crate) fn branch_sync_text(&self, cx: &App) -> Option<String> {
        let snapshot = self.git.active.as_ref()?.read(cx).snapshot.as_ref()?;
        let parts: Vec<String> = [
            (snapshot.ahead > 0).then(|| format!("↑{}", snapshot.ahead)),
            (snapshot.behind > 0).then(|| format!("↓{}", snapshot.behind)),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    fn load_stashes_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.git.stashes_expanded {
            return;
        }
        let Some(repo) = &self.git.active else {
            return;
        };
        let repo = repo.read(cx);
        let Some(count) = repo.snapshot.as_ref().map(|s| s.stash_count) else {
            return;
        };
        let key = (repo.root.clone(), count);
        if self.git.stashes_for.as_ref() == Some(&key) {
            return;
        }
        self.git.stashes_for = Some(key);
        let root = repo.root.clone();
        let list = cx.background_spawn(async move { operations::stash_list(&root) });
        cx.spawn(async move |workbench, cx| {
            let list = list.await;
            let _ = workbench.update(cx, |workbench, cx| {
                workbench.git.stashes = list;
                cx.notify();
            });
        })
        .detach();
    }

    // MARK: Commit

    fn pushes_by_default(&self, cx: &App) -> bool {
        SettingsStore::get(cx).git_commit_and_push && !self.git.amend
    }

    fn commit_from_composer(
        &mut self,
        then_push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git.committing {
            return;
        }
        let Some(repo) = self.git.active.clone() else {
            return;
        };
        let message = self.git.message.read(cx).value().to_string();
        let options = CommitOptions {
            amend: self.git.amend,
            sign_off: self.git.sign_off,
            no_verify: self.git.skip_hooks,
            ..Default::default()
        };
        self.git_commit(
            &repo,
            message,
            options,
            then_push,
            window,
            cx,
            |workbench, committed, window, cx| {
                if committed {
                    let git = &mut workbench.git;
                    git.message
                        .update(cx, |message, cx| message.set_value("", window, cx));
                    git.amend = false;
                    git.history_index = None;
                    workbench.update_message_placeholder(window, cx);
                }
            },
        );
    }

    fn commit_default(&mut self, _: &CommitDefault, window: &mut Window, cx: &mut Context<Self>) {
        let push = self.pushes_by_default(cx);
        self.commit_from_composer(push, window, cx);
    }

    fn commit_other(&mut self, _: &CommitOther, window: &mut Window, cx: &mut Context<Self>) {
        let push = !self.pushes_by_default(cx);
        self.commit_from_composer(push, window, cx);
    }

    fn update_message_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = if self.git.amend {
            "Amend message (empty keeps the current one)"
        } else {
            "Commit message"
        };
        self.git.message.update(cx, |message, cx| {
            message.set_placeholder(placeholder, window, cx)
        });
    }

    fn toggle_amend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.git.amend = !self.git.amend;
        self.update_message_placeholder(window, cx);
        cx.notify();
        // Amend starts from the last commit's message.
        let (true, Some(repo)) = (self.git.amend, self.git.active.clone()) else {
            return;
        };
        if !self.git.message.read(cx).value().is_empty() {
            return;
        }
        let (root, trusted) = {
            let repo = repo.read(cx);
            (repo.root.clone(), repo.trusted)
        };
        let previous = cx.background_spawn(async move {
            RepositoryState::read_git(&root, trusted, &["log", "-1", "--format=%B"])
                .map(|m| m.trim().to_string())
        });
        cx.spawn_in(window, async move |workbench, cx| {
            let Some(previous) = previous.await else {
                return;
            };
            let _ = workbench.update_in(cx, |workbench, window, cx| {
                let message = workbench.git.message.clone();
                if message.read(cx).value().is_empty() {
                    message.update(cx, |m, cx| m.set_value(previous, window, cx));
                }
            });
        })
        .detach();
    }

    /// ↑ in an empty composer brings back earlier messages; ↓ goes forward.
    fn recall_message(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = &self.git.active else {
            return;
        };
        let root = repo.read(cx).root.clone();
        let history = self.git.history.get(&root).cloned().unwrap_or_default();
        if history.is_empty() {
            return;
        }
        let next = self.git.history_index.map_or(-1, |i| i as isize) + step;
        let value = if next < 0 {
            self.git.history_index = None;
            String::new()
        } else if (next as usize) < history.len() {
            self.git.history_index = Some(next as usize);
            history[next as usize].clone()
        } else {
            return;
        };
        self.git.recalled = value.clone();
        self.git
            .message
            .update(cx, |message, cx| message.set_value(value, window, cx));
        cx.notify();
    }

    // MARK: Drawing

    #[allow(clippy::too_many_arguments)]
    fn chrome_button(
        &self,
        id: impl Into<ElementId>,
        title: String,
        icon_name: Option<&str>,
        kind: ButtonKind,
        disabled: bool,
        cx: &mut Context<Self>,
        act: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
    ) -> AnyElement {
        crate::workbench_chrome::chrome_button(id, title, icon_name, kind, disabled, &self.palette)
            .when(!disabled, |d| {
                d.on_click(cx.listener(move |workbench, _, window, cx| {
                    cx.stop_propagation();
                    act(workbench, window, cx)
                }))
            })
            .into_any_element()
    }

    fn icon_button(
        &self,
        id: impl Into<ElementId>,
        name: &str,
        size: f32,
        icon_size: f32,
        cx: &mut Context<Self>,
        act: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
    ) -> AnyElement {
        let p = &self.palette;
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(size))
            .h(px(size))
            .rounded(px(metrics::RADIUS_SMALL + 1.))
            .hover(|s| s.bg(hsla(p.hover)))
            .on_click(cx.listener(move |workbench, _, window, cx| {
                cx.stop_propagation();
                act(workbench, window, cx)
            }))
            .child(icon(name, icon_size, p.text_secondary))
            .into_any_element()
    }

    fn empty_state(&self, name: &str, title: &str, message: String) -> AnyElement {
        let p = &self.palette;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .p(px(16.))
            .child(icon(name, 22., p.text_tertiary))
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text_secondary))
                    .child(title.to_string()),
            )
            .child(
                div()
                    .max_w(px(220.))
                    .text_center()
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_tertiary))
                    .child(message),
            )
            .into_any_element()
    }

    /// The panel, below the sidebar's panel selector.
    pub(crate) fn changes_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(repo) = self.git.active.clone() else {
            return self.empty_state(
                "git-branch",
                "Not a git repository",
                "Open a folder inside a git repository to see its changes.".into(),
            );
        };
        let p = &self.palette;
        let state = repo.read(cx);
        let snapshot = state.snapshot.clone();
        let activity = state.activity.clone();
        let detail = state.activity_detail.clone();
        let focused = self.git.list_focus.is_focused(window);
        let mut list = div()
            .id("changes-list")
            .track_focus(&self.git.list_focus)
            .on_key_down(cx.listener(Self::changes_key))
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0();
        let clean = snapshot
            .as_ref()
            .is_some_and(|s| !s.has_changes() && s.operation.is_none());
        if let Some(snapshot) = &snapshot {
            let items = self.change_items(snapshot);
            self.sync_change_list(&items);
            let (items, snapshot, repo) = (Rc::new(items), Rc::new(snapshot.clone()), repo.clone());
            list = list
                .child(
                    gpui_kit::list(
                        self.git.list_state.clone(),
                        cx.processor(move |this, index: usize, _, cx| {
                            this.change_item(&repo, &snapshot, &items[index], focused, cx)
                        }),
                    )
                    .flex_1()
                    .pb(px(8.)),
                )
                .vertical_scrollbar(&self.git.list_state);
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.changes_header(&repo, snapshot.as_ref(), cx))
            .children(
                snapshot
                    .as_ref()
                    .and_then(|s| s.operation.clone())
                    .map(|op| {
                        let conflicts = snapshot.as_ref().is_some_and(|s| !s.conflicted.is_empty());
                        self.operation_banner(&repo, op, conflicts, cx)
                    }),
            )
            .children(activity.map(|activity| {
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .h(px(24.))
                    .px(px(12.))
                    .child(Spinner::new().color(hsla(p.accent)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_secondary))
                            .child(detail.unwrap_or(activity)),
                    )
            }))
            .child(div().h(px(1.)).flex_none().bg(hsla(p.hairline)))
            .child(if clean {
                let message = snapshot
                    .as_ref()
                    .and_then(|s| s.branch.clone())
                    .map(|b| format!("Working tree clean on {b}."))
                    .unwrap_or_else(|| "Working tree clean.".into());
                self.empty_state("circle-check", "No changes", message)
            } else {
                list.into_any_element()
            })
            .child(div().h(px(1.)).flex_none().bg(hsla(p.hairline)))
            .child(self.commit_composer(snapshot.as_ref(), cx))
            .into_any_element()
    }

    fn changes_header(
        &self,
        repo: &Entity<RepositoryState>,
        snapshot: Option<&RepoSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &self.palette;
        let workbench = cx.entity().downgrade();
        let menu_repo = repo.clone();
        let has_head = snapshot.is_some_and(|s| s.head_oid.is_some());
        let root = repo.read(cx).root.clone();
        // Only the main checkout's changes can go to a task (Swift's
        // `canMoveChangesToTask`).
        let can_move = crate::tasks::main_checkout_root(&root) == root
            && snapshot.is_some_and(|s| s.changed_file_count() > 0);
        let review_root = root.clone();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(34.))
            .px(px(8.))
            .child(
                div()
                    .id("changes-branch")
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap(px(5.))
                    .px(px(6.))
                    .h(px(24.))
                    .rounded(px(metrics::RADIUS))
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(|workbench, _, window, cx| {
                        workbench.run_command("switch_branch", window, cx)
                    }))
                    .child(icon("git-branch", 13., p.text_secondary))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(branch_label(snapshot)),
                    )
                    .child(icon("chevron-down", 10., p.text_tertiary)),
            )
            .child(div().flex_1().min_w(px(4.)))
            .children(snapshot.map(|s| self.sync_indicator(repo, s, cx)))
            .child(
                Button::new("changes-more")
                    .ghost()
                    .compact()
                    .icon(Icon::empty().path("icons/ellipsis.svg"))
                    .dropdown(move |_, _| {
                        let w = &workbench;
                        let repo = menu_repo.clone();
                        let (r1, r2, r3, r4) = (repo.clone(), repo.clone(), repo.clone(), repo);
                        let command = |id: &'static str| {
                            on_workbench(w, move |wb, window, cx| wb.run_command(id, window, cx))
                        };
                        MenuSpec::new()
                            .item("Fetch", command("git_fetch"))
                            .item("Fetch All Remotes", command("git_fetch_all"))
                            .item("Pull", command("git_pull"))
                            .item("Pull (Rebase)", command("git_pull_rebase"))
                            .item("Push", command("git_push"))
                            .item("Force Push (With Lease)…", command("git_force_push"))
                            .separator()
                            .item("Create Tag…", command("git_create_tag"))
                            .item("Push All Tags", command("git_push_tags"))
                            .separator()
                            .item(
                                "Stage All Changes",
                                on_workbench(w, move |wb, window, cx| {
                                    wb.git_stage_all(&r1, window, cx)
                                }),
                            )
                            .item(
                                "Unstage All Changes",
                                on_workbench(w, move |wb, window, cx| {
                                    wb.git_unstage_all(&r2, window, cx)
                                }),
                            )
                            .separator()
                            .item(
                                "Stash All Changes",
                                on_workbench(w, move |wb, window, cx| {
                                    wb.git_stash_all(&r3, window, cx)
                                }),
                            )
                            .item_enabled(
                                "Move All Changes to New Task…",
                                can_move,
                                on_workbench(w, |wb, window, cx| {
                                    wb.present_new_task(None, true, window, cx)
                                }),
                            )
                            .item("Pop Latest Stash", command("git_pop_stash"))
                            .item_enabled("Undo Last Commit", has_head, command("git_undo_commit"))
                            .separator()
                            .item("Review Uncommitted Changes", {
                                let root = review_root.clone();
                                on_workbench(w, move |wb, window, cx| {
                                    wb.open_review_of(
                                        root.clone(),
                                        Some(impulse_git::types::DiffScope::Uncommitted),
                                        window,
                                        cx,
                                    )
                                })
                            })
                            .item("Refresh", move |_, cx| {
                                r4.update(cx, |repo, cx| repo.refresh(cx))
                            })
                    }),
            )
            .into_any_element()
    }

    fn sync_indicator(
        &self,
        repo: &Entity<RepositoryState>,
        snapshot: &RepoSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if snapshot.upstream.is_none() && snapshot.branch.is_some() && !snapshot.is_unborn {
            let repo = repo.clone();
            return self.chrome_button(
                "changes-publish",
                "Publish".into(),
                Some("upload"),
                ButtonKind::Ghost,
                false,
                cx,
                move |wb, window, cx| wb.git_push(&repo, false, window, cx),
            );
        }
        if snapshot.ahead > 0 || snapshot.behind > 0 {
            let (pull_repo, push_repo) = (repo.clone(), repo.clone());
            return div()
                .flex()
                .gap(px(2.))
                .when(snapshot.behind > 0, |d| {
                    d.child(self.chrome_button(
                        "changes-pull",
                        snapshot.behind.to_string(),
                        Some("arrow-down"),
                        ButtonKind::Ghost,
                        false,
                        cx,
                        move |wb, window, cx| wb.git_pull(&pull_repo, None, window, cx),
                    ))
                })
                .when(snapshot.ahead > 0, |d| {
                    d.child(self.chrome_button(
                        "changes-push",
                        snapshot.ahead.to_string(),
                        Some("arrow-up"),
                        ButtonKind::Ghost,
                        false,
                        cx,
                        move |wb, window, cx| wb.git_push(&push_repo, false, window, cx),
                    ))
                })
                .into_any_element();
        }
        let repo = repo.clone();
        self.icon_button(
            "changes-fetch",
            "refresh-cw",
            22.,
            12.,
            cx,
            move |wb, window, cx| wb.git_fetch(&repo, false, window, cx),
        )
    }

    fn operation_banner(
        &self,
        repo: &Entity<RepositoryState>,
        operation: impulse_git::RepoOperation,
        has_conflicts: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &self.palette;
        let can_skip = operation.can_skip();
        let action = |id: &'static str,
                      title: &str,
                      kind,
                      action: OperationAction,
                      cx: &mut Context<Self>| {
            let (repo, operation) = (repo.clone(), operation.clone());
            self.chrome_button(
                id,
                title.into(),
                None,
                kind,
                false,
                cx,
                move |wb, window, cx| wb.git_perform(&repo, action, operation.clone(), window, cx),
            )
        };
        let continue_button = action(
            "op-continue",
            "Continue",
            ButtonKind::Secondary,
            OperationAction::Continue,
            cx,
        );
        let skip_button = can_skip.then(|| {
            action(
                "op-skip",
                "Skip",
                ButtonKind::Ghost,
                OperationAction::Skip,
                cx,
            )
        });
        let abort_button = action(
            "op-abort",
            "Abort",
            ButtonKind::Danger,
            OperationAction::Abort,
            cx,
        );
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(6.))
            .bg(hsla(p.warning.opacity(0.12)))
            .child(icon("triangle-alert", 13., p.warning))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(operation.title()),
            )
            .child(div().flex_1())
            .when(has_conflicts, |d| {
                let repo = repo.clone();
                // The label when the sidebar has room for it (as Swift's
                // ViewThatFits), else only the icon so Abort stays in view.
                let wide = self.sidebar_width(cx) >= 320.;
                d.child(
                    Button::new("op-ask-agent")
                        .ghost()
                        .compact()
                        .icon(Icon::empty().path("icons/bot.svg"))
                        .when(wide, |b| b.label("Ask Agent"))
                        .tooltip("Ask Agent: ask an agent to resolve the conflicts")
                        .dropdown(move |window, cx| {
                            crate::agent_send::conflict_menu_spec(
                                "Send to",
                                repo.clone(),
                                Vec::new(),
                                window,
                                cx,
                            )
                        }),
                )
            })
            .child(continue_button)
            .children(skip_button)
            .child(abort_button)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn section_header(
        &self,
        id: &'static str,
        title: String,
        count: Option<usize>,
        expanded: bool,
        toggle: impl Fn(&mut Workbench) + 'static,
        trailing: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &self.palette;
        div()
            .id(id)
            .flex()
            .flex_none()
            // A list item is laid out on its own: without this it would
            // shrink to its content.
            .w_full()
            .items_center()
            .gap(px(6.))
            .h(px(26.))
            .px(px(12.))
            .on_click(cx.listener(move |workbench, _, _, cx| {
                toggle(workbench);
                workbench.sync_git(cx);
                cx.notify();
            }))
            .child(icon(
                if expanded {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                11.,
                p.text_tertiary,
            ))
            .child(
                div()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text_tertiary))
                    .child(crate::workbench_chrome::tracked(&title.to_uppercase())),
            )
            .children(count.map(|count| {
                div()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text_tertiary))
                    .child(count.to_string())
            }))
            .child(div().flex_1())
            .children(trailing)
            .into_any_element()
    }

    /// The file rows in the order shown (collapsed sections left out), as
    /// `sections` lays them out.
    fn listed_rows(&self, snapshot: &RepoSnapshot) -> Vec<ListedRow> {
        let mut rows = Vec::new();
        let mut child = 0;
        let groups = [
            (Section::Conflicted, &snapshot.conflicted),
            (Section::Staged, &snapshot.staged),
            (Section::Unstaged, &snapshot.unstaged),
            (Section::Untracked, &snapshot.untracked),
        ];
        for (section, changes) in groups {
            if changes.is_empty() {
                continue;
            }
            child += 1;
            if self.git.collapsed.contains(&section) {
                continue;
            }
            for change in changes {
                rows.push(ListedRow {
                    key: row_key(section, &change.path),
                    change: change.clone(),
                    section,
                    child,
                });
                child += 1;
            }
        }
        rows
    }

    fn active_snapshot(&self, cx: &App) -> Option<(Entity<RepositoryState>, RepoSnapshot)> {
        let repo = self.git.active.clone()?;
        let snapshot = repo.read(cx).snapshot.clone()?;
        Some((repo, snapshot))
    }

    /// The file list has the keyboard.
    pub(crate) fn changes_list_focused(&self, window: &Window, cx: &App) -> bool {
        self.git.list_focus.contains_focused(window, cx)
    }

    /// Keyboard focus to the file list, with the first row selected if
    /// none is (Show Changes).
    pub(crate) fn focus_changes_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.git.list_focus, cx);
        let Some((_, snapshot)) = self.active_snapshot(cx) else {
            return;
        };
        let rows = self.listed_rows(&snapshot);
        let current = self.git.selection.as_deref();
        if !rows.iter().any(|r| Some(r.key.as_str()) == current) {
            self.git.selection = rows.first().map(|r| r.key.clone());
        }
        cx.notify();
    }

    /// The list's keys, as the Swift panel's `handleKey`.
    fn changes_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((repo, snapshot)) = self.active_snapshot(cx) else {
            return;
        };
        let rows = self.listed_rows(&snapshot);
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|r| Some(r.key.as_str()) == self.git.selection.as_deref());
        let modifiers = event.keystroke.modifiers;
        let select = |panel: &mut Self, i: isize| {
            let row = &rows[i.clamp(0, rows.len() as isize - 1) as usize];
            panel.git.selection = Some(row.key.clone());
            panel.git.list_state.scroll_to_reveal_item(row.child);
        };
        match event.keystroke.key.as_str() {
            "up" => select(self, index.map_or(rows.len() as isize, |i| i as isize) - 1),
            "down" => select(self, index.map_or(-1, |i| i as isize) + 1),
            "space" => {
                let Some(row) = index.map(|i| &rows[i]) else {
                    return;
                };
                let one = vec![row.change.clone()];
                match row.section {
                    Section::Staged => {
                        self.git_unstage(&repo, &one, window, cx);
                        self.git.selection = Some(row_key(Section::Unstaged, &row.change.path));
                    }
                    Section::Unstaged | Section::Untracked => {
                        self.git_stage(&repo, &one, window, cx);
                        self.git.selection = Some(row_key(Section::Staged, &row.change.path));
                    }
                    Section::Conflicted => self.git_mark_resolved(&repo, &one, window, cx),
                }
            }
            "enter" => {
                let Some(row) = index.map(|i| &rows[i]) else {
                    return;
                };
                let absolute = std::path::Path::new(&repo.read(cx).root).join(&row.change.path);
                if modifiers.secondary() {
                    self.open_file(&absolute, window, cx);
                } else if modifiers.alt
                    && row.section.has_working_copy_diff()
                    && row.change.status != ChangeStatus::Deleted
                {
                    self.open_diff_editor(&absolute, window, cx);
                } else {
                    self.open_review_at(row.section.review_scope(), &absolute, window, cx);
                }
            }
            "backspace" | "delete" => {
                let Some(row) = index.map(|i| &rows[i]) else {
                    return;
                };
                if row.section == Section::Conflicted {
                    return;
                }
                let staged = row.section == Section::Staged;
                self.git_discard(&repo, vec![row.change.clone()], staged, window, cx);
            }
            "escape" => self.focus_active(window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// The list's items in order (collapsed sections' rows left out).
    fn change_items(&self, snapshot: &RepoSnapshot) -> Vec<ChangeItem> {
        let mut items = Vec::new();
        for section in SECTIONS {
            let changes = section.changes(snapshot);
            if changes.is_empty() {
                continue;
            }
            items.push(ChangeItem::Header(section));
            if !self.git.collapsed.contains(&section) {
                for (index, change) in changes.iter().enumerate() {
                    items.push(ChangeItem::Row(section, index, change.path.clone()));
                }
            }
        }
        if snapshot.stash_count > 0 {
            items.push(ChangeItem::Stashes);
            if self.git.stashes_expanded {
                items.extend((0..self.git.stashes.len()).map(ChangeItem::Stash));
            }
        }
        items
    }

    /// Tell the list which items changed since the last frame, keeping
    /// its scroll place and the heights it measured for the rest.
    fn sync_change_list(&self, items: &[ChangeItem]) {
        let mut listed = self.git.list_items.borrow_mut();
        if *listed == items {
            return;
        }
        let prefix = listed.iter().zip(items).take_while(|(a, b)| a == b).count();
        let suffix = listed[prefix..]
            .iter()
            .rev()
            .zip(items[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        self.git
            .list_state
            .splice(prefix..listed.len() - suffix, items.len() - prefix - suffix);
        *listed = items.to_vec();
    }

    /// One item of the list: a section's header, a file or a stash.
    fn change_item(
        &self,
        repo: &Entity<RepositoryState>,
        snapshot: &RepoSnapshot,
        item: &ChangeItem,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match item {
            ChangeItem::Header(section) => {
                let section = *section;
                let changes = section.changes(snapshot);
                let expanded = !self.git.collapsed.contains(&section);
                let (id, title, count) = match section {
                    Section::Conflicted => (
                        "sec-conflicts",
                        "Conflicts".to_string(),
                        Some(changes.len()),
                    ),
                    Section::Staged => ("sec-staged", "Staged".to_string(), Some(changes.len())),
                    Section::Unstaged => {
                        ("sec-changes", "Changes".to_string(), Some(changes.len()))
                    }
                    Section::Untracked if snapshot.untracked_truncated => (
                        "sec-untracked",
                        format!("Untracked (first {})", changes.len()),
                        None,
                    ),
                    Section::Untracked => (
                        "sec-untracked",
                        "Untracked".to_string(),
                        Some(changes.len()),
                    ),
                };
                let trailing: Vec<AnyElement> = match section {
                    Section::Conflicted => Vec::new(),
                    Section::Staged => {
                        let repo = repo.clone();
                        vec![self.icon_button(
                            "unstage-all",
                            "minus",
                            20.,
                            12.,
                            cx,
                            move |wb, window, cx| wb.git_unstage_all(&repo, window, cx),
                        )]
                    }
                    Section::Unstaged => {
                        let (r1, r2) = (repo.clone(), repo.clone());
                        let (c1, c2) = (changes.to_vec(), changes.to_vec());
                        vec![
                            self.icon_button(
                                "discard-all",
                                "undo-2",
                                20.,
                                12.,
                                cx,
                                move |wb, window, cx| {
                                    wb.git_discard(&r1, c1.clone(), false, window, cx)
                                },
                            ),
                            self.icon_button(
                                "stage-all",
                                "plus",
                                20.,
                                12.,
                                cx,
                                move |wb, window, cx| wb.git_stage(&r2, &c2, window, cx),
                            ),
                        ]
                    }
                    Section::Untracked => {
                        let (repo, changes) = (repo.clone(), changes.to_vec());
                        vec![self.icon_button(
                            "stage-untracked",
                            "plus",
                            20.,
                            12.,
                            cx,
                            move |wb, window, cx| wb.git_stage(&repo, &changes, window, cx),
                        )]
                    }
                };
                self.section_header(
                    id,
                    title,
                    count,
                    expanded,
                    move |wb| {
                        if !wb.git.collapsed.remove(&section) {
                            wb.git.collapsed.insert(section);
                        }
                    },
                    trailing,
                    cx,
                )
            }
            ChangeItem::Row(section, index, _) => {
                let change = &section.changes(snapshot)[*index];
                self.change_row(repo, change, *section, focused, cx)
            }
            ChangeItem::Stashes => {
                let expanded = self.git.stashes_expanded;
                self.section_header(
                    "sec-stashes",
                    "Stashes".into(),
                    Some(snapshot.stash_count),
                    expanded,
                    |wb| wb.git.stashes_expanded = !wb.git.stashes_expanded,
                    Vec::new(),
                    cx,
                )
            }
            ChangeItem::Stash(index) => match self.git.stashes.get(*index) {
                Some(entry) => self.stash_row(repo, entry.clone(), cx),
                None => div().into_any_element(),
            },
        }
    }

    fn change_row(
        &self,
        repo: &Entity<RepositoryState>,
        change: &FileChange,
        section: Section,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &self.palette;
        let key = row_key(section, &change.path);
        let group: SharedString = format!("row-{key}").into();
        let selected = self.git.selection.as_deref() == Some(key.as_str());
        let (dir, name) = match change.path.rsplit_once('/') {
            Some((dir, name)) => (dir.to_string(), name.to_string()),
            None => (String::new(), change.path.clone()),
        };
        let deleted = change.status == ChangeStatus::Deleted;
        let root = repo.read(cx).root.clone();
        let absolute = std::path::Path::new(&root).join(&change.path);
        let stat: Option<AnyElement> = match (change.added, change.removed) {
            (Some(added), Some(removed)) => Some(
                div()
                    .flex()
                    .gap(px(4.))
                    .font_family("JetBrains Mono")
                    .text_size(px(11.))
                    .when(added > 0 || removed == 0, |d| {
                        d.child(
                            div()
                                .text_color(hsla(p.git_added))
                                .child(format!("+{added}")),
                        )
                    })
                    .when(removed > 0, |d| {
                        d.child(
                            div()
                                .text_color(hsla(p.git_deleted))
                                .child(format!("\u{2212}{removed}")),
                        )
                    })
                    .into_any_element(),
            ),
            _ if change.is_binary => Some(
                div()
                    .font_family("JetBrains Mono")
                    .text_size(px(10.))
                    .text_color(hsla(p.text_tertiary))
                    .child("bin")
                    .into_any_element(),
            ),
            _ => None,
        };
        let one = vec![change.clone()];
        let mut actions: Vec<AnyElement> = Vec::new();
        {
            let path = absolute.clone();
            actions.push(self.icon_button(
                SharedString::from(format!("open-{key}")),
                "file-code",
                20.,
                12.,
                cx,
                move |wb, window, cx| wb.open_file(&path, window, cx),
            ));
        }
        let button = |suffix: &str, name: &str, act: Act, cx: &mut Context<Self>| {
            self.icon_button(
                SharedString::from(format!("{suffix}-{key}")),
                name,
                20.,
                12.,
                cx,
                move |wb, window, cx| act(wb, window, cx),
            )
        };
        let (r, c) = (repo.clone(), one.clone());
        match section {
            Section::Staged => actions.push(button(
                "unstage",
                "minus",
                Rc::new(move |wb, window, cx| wb.git_unstage(&r, &c, window, cx)),
                cx,
            )),
            Section::Unstaged | Section::Untracked => {
                let (r2, c2) = (r.clone(), c.clone());
                actions.push(button(
                    "discard",
                    "undo-2",
                    Rc::new(move |wb, window, cx| wb.git_discard(&r, c.clone(), false, window, cx)),
                    cx,
                ));
                actions.push(button(
                    "stage",
                    "plus",
                    Rc::new(move |wb, window, cx| wb.git_stage(&r2, &c2, window, cx)),
                    cx,
                ));
            }
            Section::Conflicted => {
                let (r2, c2, r3, c3) = (r.clone(), c.clone(), r.clone(), c.clone());
                actions.push(button(
                    "ours",
                    "arrow-up",
                    Rc::new(move |wb, window, cx| wb.git_resolve(&r, &c, true, window, cx)),
                    cx,
                ));
                actions.push(button(
                    "theirs",
                    "arrow-down",
                    Rc::new(move |wb, window, cx| wb.git_resolve(&r2, &c2, false, window, cx)),
                    cx,
                ));
                actions.push(button(
                    "resolved",
                    "check",
                    Rc::new(move |wb, window, cx| wb.git_mark_resolved(&r3, &c3, window, cx)),
                    cx,
                ));
            }
        }
        let workbench = cx.entity().downgrade();
        let menu_repo = repo.clone();
        let menu_change = change.clone();
        let menu_path = absolute.clone();
        let click_key = key.clone();
        let click_path = absolute.clone();
        div()
            .id(SharedString::from(format!("row-{key}")))
            .flex_none()
            .px(px(6.))
            .child(
                div()
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(metrics::ROW_HEIGHT))
                    .pl(px(8.))
                    .pr(px(4.))
                    .rounded(px(metrics::RADIUS_SMALL + 1.))
                    .when(selected, |d| d.bg(hsla(p.selection)))
                    // Swift's ring is a stroke over the row, not a border
                    // that moves its content.
                    .when(selected && focused, |d| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded(px(metrics::RADIUS_SMALL + 1.))
                                .border(px(1.5))
                                .border_color(hsla(p.focus_ring)),
                        )
                    })
                    .when(!selected, |d| d.hover(|s| s.bg(hsla(p.hover))))
                    .child(
                        img(crate::file_icons::icon_path(&name, false, false))
                            .size(px(14.))
                            .flex_none(),
                    )
                    .child(
                        div()
                            .flex_none()
                            .max_w_full()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(hsla(if deleted { p.text_secondary } else { p.text }))
                            .when(deleted, |d| d.line_through())
                            .child(name),
                    )
                    .when(!dir.is_empty(), |d| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(11.))
                                .text_color(hsla(p.text_tertiary))
                                .child(dir),
                        )
                    })
                    .child(div().flex_1().min_w(px(4.)))
                    .children(stat.map(|stat| {
                        div()
                            .flex_none()
                            .group_hover(group.clone(), |s| s.invisible())
                            .child(stat)
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .absolute()
                            .right(px(24.))
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
                            // The row's hover color, opaque, over what it covers
                            // (the sidebar's chrome color).
                            .bg(hsla(p.chrome.mix(p.hover, p.hover.a)))
                            .children(actions),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(12.))
                            .font_family("JetBrains Mono")
                            .text_size(px(11.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(hsla(status_color(change.status, p)))
                            .child(change.status.letter()),
                    )
                    .relative(),
            )
            .on_click(
                cx.listener(move |workbench, event: &gpui_kit::ClickEvent, window, cx| {
                    // A double click opens the file; one click selects the row
                    // and opens the change in Review, as in the Swift app.
                    if event.click_count() >= 2 {
                        if !deleted {
                            workbench.open_file(&click_path, window, cx);
                        }
                    } else {
                        workbench.git.selection = Some(click_key.clone());
                        window.focus(&workbench.git.list_focus, cx);
                        workbench.open_review_at(section.review_scope(), &click_path, window, cx);
                    }
                    cx.notify();
                }),
            )
            .right_click_menu(move |window, cx| {
                let w = &workbench;
                let (repo, change, path) =
                    (menu_repo.clone(), menu_change.clone(), menu_path.clone());
                let one = vec![change.clone()];
                let (review_path, diff_path, open_path) =
                    (path.clone(), path.clone(), path.clone());
                let mut menu = MenuSpec::new().item(
                    "Open Changes",
                    on_workbench(w, move |wb, window, cx| {
                        wb.open_review_at(section.review_scope(), &review_path, window, cx)
                    }),
                );
                if section.has_working_copy_diff() && change.status != ChangeStatus::Deleted {
                    menu = menu.item(
                        "Open in Diff Editor",
                        on_workbench(w, move |wb, window, cx| {
                            wb.open_diff_editor(&diff_path, window, cx)
                        }),
                    );
                }
                let mut menu = menu
                    .item(
                        "Open File",
                        on_workbench(w, move |wb, window, cx| {
                            wb.open_file(&open_path, window, cx)
                        }),
                    )
                    .separator();
                let (r1, r2, r3, r4) = (repo.clone(), repo.clone(), repo.clone(), repo.clone());
                let (c1, c2, c3, c4) = (one.clone(), one.clone(), one.clone(), one.clone());
                menu = match section {
                    Section::Staged => menu
                        .item(
                            "Unstage",
                            on_workbench(w, move |wb, window, cx| {
                                wb.git_unstage(&r1, &c1, window, cx)
                            }),
                        )
                        .item(
                            "Discard Staged and Unstaged Changes…",
                            on_workbench(w, move |wb, window, cx| {
                                wb.git_discard(&r2, c2.clone(), true, window, cx)
                            }),
                        ),
                    Section::Unstaged | Section::Untracked => menu
                        .item(
                            "Stage",
                            on_workbench(w, move |wb, window, cx| {
                                wb.git_stage(&r1, &c1, window, cx)
                            }),
                        )
                        .item(
                            "Discard Changes…",
                            on_workbench(w, move |wb, window, cx| {
                                wb.git_discard(&r2, c2.clone(), false, window, cx)
                            }),
                        ),
                    Section::Conflicted => {
                        let resolve_path = path.clone();
                        menu.item(
                            "Open to Resolve",
                            on_workbench(w, move |wb, window, cx| {
                                wb.open_file(&resolve_path, window, cx)
                            }),
                        )
                        .item(
                            "Keep Current (HEAD)",
                            on_workbench(w, move |wb, window, cx| {
                                wb.git_resolve(&r1, &c1, true, window, cx)
                            }),
                        )
                        .item(
                            "Take Incoming",
                            on_workbench(w, move |wb, window, cx| {
                                wb.git_resolve(&r3, &c3, false, window, cx)
                            }),
                        )
                        .item(
                            "Mark Resolved",
                            on_workbench(w, move |wb, window, cx| {
                                wb.git_mark_resolved(&r4, &c4, window, cx)
                            }),
                        )
                        .submenu(
                            "Ask Agent to Resolve",
                            crate::agent_send::conflict_menu_spec(
                                "",
                                repo.clone(),
                                vec![change.path.clone()],
                                window,
                                cx,
                            ),
                        )
                    }
                };
                let root = repo.read(cx).root.clone();
                if section != Section::Conflicted && crate::tasks::main_checkout_root(&root) == root
                {
                    let paths: Vec<String> = std::iter::once(change.path.clone())
                        .chain(change.old_path.clone())
                        .collect();
                    menu = menu.item(
                        "Move to New Task…",
                        on_workbench(w, move |wb, window, cx| {
                            wb.present_moving_task(None, true, Some(paths.clone()), window, cx)
                        }),
                    );
                }
                let relative = change.path.clone();
                let reveal_path = path.clone();
                menu.separator()
                    .item("Copy Path", move |_, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(relative.clone()))
                    })
                    .item(crate::file_tree_actions::REVEAL, move |_, cx| {
                        crate::file_tree_actions::reveal(&reveal_path, cx)
                    })
            })
            .into_any_element()
    }

    fn stash_row(
        &self,
        repo: &Entity<RepositoryState>,
        entry: StashEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &self.palette;
        let group: SharedString = format!("stash-{}", entry.index).into();
        let (r1, r2, r3) = (repo.clone(), repo.clone(), repo.clone());
        let (index, commit) = (entry.index, entry.commit.clone());
        let (c1, c2, c3) = (commit.clone(), commit.clone(), commit);
        let actions = vec![
            self.icon_button(
                SharedString::from(format!("stash-apply-{index}")),
                "play",
                20.,
                11.,
                cx,
                move |wb, window, cx| {
                    wb.git_apply_stash(&r1, index, Some(c1.clone()), false, window, cx)
                },
            ),
            self.icon_button(
                SharedString::from(format!("stash-pop-{index}")),
                "arrow-up",
                20.,
                11.,
                cx,
                move |wb, window, cx| {
                    wb.git_apply_stash(&r2, index, Some(c2.clone()), true, window, cx)
                },
            ),
            self.icon_button(
                SharedString::from(format!("stash-drop-{index}")),
                "trash-2",
                20.,
                11.,
                cx,
                move |wb, window, cx| wb.git_drop_stash(&r3, index, Some(c3.clone()), window, cx),
            ),
        ];
        // A click shows the stash in Review (its changes against where it
        // was made).
        let root = repo.read(cx).root.clone();
        div()
            .flex_none()
            .px(px(6.))
            .child(
                div()
                    .id(SharedString::from(format!("stash-row-{index}")))
                    .group(group.clone())
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(metrics::ROW_HEIGHT))
                    .pl(px(8.))
                    .pr(px(4.))
                    .rounded(px(metrics::RADIUS_SMALL + 1.))
                    .hover(|s| s.bg(hsla(p.hover)))
                    .on_click(cx.listener(move |wb, _, window, cx| {
                        wb.open_review_of(
                            root.clone(),
                            Some(impulse_git::types::DiffScope::Stash { index }),
                            window,
                            cx,
                        )
                    }))
                    .child(icon("package", 12., p.text_tertiary))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .child(entry.message.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .group_hover(group.clone(), |s| s.invisible())
                            .font_family("JetBrains Mono")
                            .text_size(px(10.))
                            .text_color(hsla(p.text_tertiary))
                            .child(format!("stash@{{{index}}}")),
                    )
                    .child(
                        div()
                            .flex()
                            .absolute()
                            .right(px(4.))
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .children(actions),
                    ),
            )
            .into_any_element()
    }

    fn toggle_chip(
        &self,
        id: &'static str,
        title: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        act: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
    ) -> AnyElement {
        let p = &self.palette;
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(4.))
            .on_click(cx.listener(move |workbench, _, window, cx| act(workbench, window, cx)))
            .child(icon(
                if on { "circle-check" } else { "circle" },
                12.,
                if on { p.accent } else { p.text_tertiary },
            ))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(hsla(p.text_secondary))
                    .child(title),
            )
            .into_any_element()
    }

    fn commit_composer(
        &self,
        snapshot: Option<&RepoSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &self.palette;
        let value = self.git.message.read(cx).value().to_string();
        let subject = value.split('\n').next().unwrap_or("").chars().count();
        let push = self.pushes_by_default(cx);
        let staged = snapshot.map(|s| s.staged.len()).unwrap_or(0);
        let title = if self.git.committing {
            if push {
                "Committing & Pushing…".to_string()
            } else {
                "Committing…".to_string()
            }
        } else if self.git.amend {
            "Amend".to_string()
        } else {
            let files = match staged {
                0 => String::new(),
                1 => " 1 file".to_string(),
                n => format!(" {n} files"),
            };
            if push {
                format!("Commit{files} & Push")
            } else {
                format!("Commit{files}")
            }
        };
        let commit_button = self.chrome_button(
            "commit",
            title,
            Some(if push { "arrow-up" } else { "check" }),
            ButtonKind::Primary,
            self.git.committing,
            cx,
            move |wb, window, cx| wb.commit_from_composer(push, window, cx),
        );
        let workbench = cx.entity().downgrade();
        let key_hint = if cfg!(target_os = "macos") {
            "⌘↩"
        } else {
            "Ctrl+↩"
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(6.))
            .p(px(10.))
            .child(
                div()
                    .id("commit-message")
                    .key_context(CONTEXT)
                    .on_action(cx.listener(Self::commit_default))
                    .on_action(cx.listener(Self::commit_other))
                    .capture_key_down(cx.listener(
                        |workbench, event: &gpui_kit::KeyDownEvent, window, cx| {
                            let keystroke = &event.keystroke;
                            if keystroke.modifiers.modified() {
                                return;
                            }
                            let empty = workbench.git.message.read(cx).value().is_empty();
                            let recalling = workbench.git.history_index.is_some();
                            match keystroke.key.as_str() {
                                "up" if empty || recalling => {
                                    workbench.recall_message(1, window, cx);
                                    cx.stop_propagation();
                                }
                                "down" if recalling => {
                                    workbench.recall_message(-1, window, cx);
                                    cx.stop_propagation();
                                }
                                _ => {}
                            }
                        },
                    ))
                    .relative()
                    // The Textarea keeps insets of its own: with them, the
                    // text and the box's height are the Swift TextEditor's.
                    .pl(px(0.))
                    .pr(px(6.))
                    .pt(px(0.))
                    .pb(px(3.))
                    .rounded(px(metrics::RADIUS))
                    .bg(hsla(p.content))
                    .border_1()
                    .border_color(hsla(p.hairline_strong))
                    .text_size(px(12.))
                    .child(
                        // The Swift TextEditor's minimum height.
                        Textarea::new(&self.git.message)
                            .appearance(false)
                            .min_h(px(40.)),
                    )
                    .when(subject > 0, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(0.))
                                .right(px(0.))
                                .p(px(5.))
                                .font_family("JetBrains Mono")
                                .text_size(px(10.))
                                .text_color(hsla(if subject > 72 {
                                    p.danger
                                } else if subject > 50 {
                                    p.warning
                                } else {
                                    p.text_tertiary
                                }))
                                .child(subject.to_string()),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(self.toggle_chip(
                        "amend",
                        "Amend",
                        self.git.amend,
                        cx,
                        |wb, window, cx| wb.toggle_amend(window, cx),
                    ))
                    .child(self.toggle_chip(
                        "sign-off",
                        "Sign-off",
                        self.git.sign_off,
                        cx,
                        |wb, _, cx| {
                            wb.git.sign_off = !wb.git.sign_off;
                            cx.notify();
                        },
                    ))
                    .child(self.toggle_chip(
                        "skip-hooks",
                        "Skip hooks",
                        self.git.skip_hooks,
                        cx,
                        |wb, _, cx| {
                            wb.git.skip_hooks = !wb.git.skip_hooks;
                            cx.notify();
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(commit_button)
                    .child(
                        Button::new("commit-more")
                            .ghost()
                            .compact()
                            .icon(Icon::empty().path("icons/chevron-down.svg"))
                            .dropdown(move |_, _| {
                                let w = &workbench;
                                MenuSpec::new()
                                    .item(
                                        "Commit",
                                        on_workbench(w, |wb, window, cx| {
                                            wb.commit_from_composer(false, window, cx)
                                        }),
                                    )
                                    .item(
                                        "Commit & Push",
                                        on_workbench(w, |wb, window, cx| {
                                            wb.commit_from_composer(true, window, cx)
                                        }),
                                    )
                                    .separator()
                                    .item(
                                        "Amend Last Commit",
                                        on_workbench(w, |wb, window, cx| {
                                            if !wb.git.amend {
                                                wb.git.amend = true;
                                                wb.update_message_placeholder(window, cx);
                                            }
                                            wb.commit_from_composer(false, window, cx)
                                        }),
                                    )
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(16.))
                            .px(px(4.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(hsla(p.hairline_strong))
                            .text_size(px(10.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.text_tertiary))
                            .child(key_hint),
                    ),
            )
            .into_any_element()
    }
}
