//! History: a repository's commits with their lane graph, as the Swift
//! `HistorySurface` shows them. The header switches between the current
//! branch and all branches (or shows the branch or path it was opened on)
//! and filters: free text matches the loaded commits' subjects, authors,
//! SHAs and refs, while `author:`, `path:`, `since:` and `until:` read
//! history again (`impulse_git::history_query`), with presets in its menu.
//! Each row has its graph cell (curved lane lines, a dot, a ring for
//! merges), ↑ for commits not pushed yet and ↓ for ones not pulled, branch
//! and tag chips with their own menus, the fork point from the default
//! branch (older commits dimmed), the subject (HEAD's in bold), author,
//! date and short SHA, a page at a time; its menu checks out, branches,
//! tags, merges, rebases, cherry-picks, reverts, resets and compares.
//! Below, the selected commit's message and details over its diff (the
//! Review list, for that commit). Up and Down move the selection; refs
//! moving (a commit, a fetch, a tag) refresh what's loaded in place.

mod menu;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::{
    canvas, div, point, prelude::*, px, uniform_list, AnyElement, App, Bounds, ClipboardItem,
    Context, Entity, EventEmitter, FocusHandle, FontWeight, Hsla, KeyDownEvent, PathBuilder,
    Pixels, Subscription, Task, UniformListScrollHandle, Window,
};
use impulse_git::history_query::HistoryQuery;
use impulse_git::log::{CommitDetails, ForkPoint, LogEntry, LogScope};
use impulse_git::operations::ResetMode;
use impulse_git::types::DiffScope;
use impulse_kit::chrome::{metrics, ChromePalette, Rgba as KitRgba};
use impulse_kit::dates;
use impulse_kit::git::commit_graph::{GraphCommit, GraphRow, Layout as GraphLayout};
use impulse_kit::git::refs::RefDecoration;
use impulse_kit::theme::ResolvedTheme;

use crate::repository_state::RepositoryStore;
use crate::review_view::ReviewView;
use crate::workbench_chrome::{hsla, icon};

const PAGE: usize = 300;
const ROW_HEIGHT: f32 = 28.0;
const LANE_WIDTH: f32 = 13.0;
const MAX_LANES: usize = 8;
/// How many commits divergence and the fork point look through.
const SURROUNDINGS_LIMIT: usize = 2000;

/// What a commit's (or a ref chip's) menu asks the workbench to do.
#[derive(Clone, Debug, PartialEq)]
pub enum HistoryAction {
    Checkout,
    BranchHere,
    TagHere,
    CherryPick,
    Revert,
    Reset(ResetMode),
    /// Merge a branch, tag or commit into the current branch.
    Merge(String),
    /// Rebase the current branch onto a branch, tag or commit.
    RebaseOnto(String),
    Compare(DiffScope),
    SwitchToBranch(String),
    DeleteBranch(String),
    PushTag(String),
    DeleteTag(String),
    DeleteRemoteTag(String),
}

pub struct HistoryEvent {
    pub action: HistoryAction,
    pub entry: LogEntry,
}

/// What a page of history is read with.
#[derive(Clone, Debug, PartialEq)]
struct Request {
    scope: LogScope,
    path: Option<String>,
    query: HistoryQuery,
}

/// The current branch, the remotes (to tell `origin/x` from a local
/// `origin/x`), and where tags are pushed: for the menus.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryContext {
    pub branch: Option<String>,
    pub remotes: Vec<String>,
    pub tag_remote: Option<String>,
}

/// What the filter's text matches, lowercased once per entry.
struct SearchKey {
    subject: String,
    author: String,
    refs: Vec<String>,
}

impl SearchKey {
    fn new(entry: &LogEntry) -> Self {
        SearchKey {
            subject: entry.subject.to_lowercase(),
            author: entry.author.to_lowercase(),
            refs: entry.refs.iter().map(|r| r.to_lowercase()).collect(),
        }
    }

    fn matches(&self, text: &str, sha: &str) -> bool {
        self.subject.contains(text)
            || self.author.contains(text)
            || sha.starts_with(text)
            || self.refs.iter().any(|r| r.contains(text))
    }
}

pub struct HistoryView {
    pub root: String,
    /// The file or folder it follows (repository-relative), if any.
    pub path: Option<String>,
    pub scope: LogScope,
    filter: Entity<InputState>,
    query: HistoryQuery,
    applied: Request,
    /// Bumped by every reload, so pages read for an older one are dropped.
    generation: u64,
    entries: Vec<LogEntry>,
    rows: Vec<GraphRow>,
    keys: Vec<SearchKey>,
    /// The entries the filter's text keeps, as indices.
    visible: Vec<usize>,
    graph: GraphLayout,
    max_lanes: usize,
    /// The commits the loaded history starts from (`log::tips`).
    tips: Option<Vec<String>>,
    selected: Option<String>,
    details: Option<CommitDetails>,
    expanded: bool,
    outgoing: HashSet<String>,
    incoming: HashSet<String>,
    fork: Option<ForkPoint>,
    compare_base: Option<LogEntry>,
    pub context: HistoryContext,
    review: Entity<ReviewView>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    palette: ChromePalette,
    loading: bool,
    exhausted: bool,
    error: Option<String>,
    /// A commit to select once its page is loaded (Show Commit for This
    /// Line).
    reveal: Option<String>,
    _query_reload: Option<Task<()>>,
    _refresh: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<HistoryEvent> for HistoryView {}

impl HistoryView {
    pub fn new(
        root: String,
        path: Option<String>,
        scope: LogScope,
        theme: &ResolvedTheme,
        palette: &ChromePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let review = cx.new(|cx| {
            ReviewView::new(
                root.clone(),
                DiffScope::Commit { sha: "HEAD".into() },
                theme,
                palette,
                cx,
            )
        });
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter, or author: path: since:"));
        let repository = RepositoryStore::state(&root, cx);
        let subscriptions = vec![
            cx.observe(&review, |_, _, cx| cx.notify()),
            cx.subscribe(&filter, |view, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    view.filter_changed(cx)
                }
            }),
            // Refs moved (a commit, a fetch, a tag): refresh what's loaded.
            cx.observe(&repository, |view, _, cx| view.schedule_refresh(cx)),
        ];
        let applied = Request {
            scope: scope.clone(),
            path: path.clone(),
            query: HistoryQuery::default(),
        };
        let mut view = HistoryView {
            root,
            path,
            scope,
            filter,
            query: HistoryQuery::default(),
            applied,
            generation: 0,
            entries: Vec::new(),
            rows: Vec::new(),
            keys: Vec::new(),
            visible: Vec::new(),
            graph: GraphLayout::default(),
            max_lanes: 1,
            tips: None,
            selected: None,
            details: None,
            expanded: false,
            outgoing: HashSet::new(),
            incoming: HashSet::new(),
            fork: None,
            compare_base: None,
            context: HistoryContext::default(),
            review,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            palette: palette.clone(),
            loading: false,
            exhausted: false,
            error: None,
            reveal: None,
            _query_reload: None,
            _refresh: None,
            _subscriptions: subscriptions,
        };
        view.reload(cx);
        view
    }

    /// "History · trailhead", or the followed file's name.
    pub fn title(&self) -> String {
        let name = |path: &str| {
            std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string())
        };
        match &self.path {
            Some(path) => format!("History · {}", name(path)),
            None if self.root.is_empty() => "History".into(),
            None => format!("History · {}", name(&self.root)),
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn review(&self) -> &Entity<ReviewView> {
        &self.review
    }

    pub fn set_theme(
        &mut self,
        theme: &ResolvedTheme,
        palette: &ChromePalette,
        cx: &mut Context<Self>,
    ) {
        self.palette = palette.clone();
        self.review
            .update(cx, |review, cx| review.set_theme(theme, palette, cx));
        cx.notify();
    }

    /// Show another scope's history, keeping the selected commit.
    pub fn set_scope(&mut self, scope: LogScope, cx: &mut Context<Self>) {
        self.scope = scope;
        self.reload(cx);
    }

    /// Type `text` into the filter (the snapshot's `history-filter=`).
    pub fn set_filter(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
        self.filter_changed(cx);
    }

    // MARK: Loading

    /// Read history from the start.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.entries.clear();
        self.rows.clear();
        self.keys.clear();
        self.visible.clear();
        self.graph = GraphLayout::default();
        self.max_lanes = 1;
        self.tips = None;
        self.loading = false;
        self.exhausted = false;
        self.applied = Request {
            scope: self.scope.clone(),
            path: self.path.clone(),
            query: self.query.server(),
        };
        self.load_page(cx);
        self.load_surroundings(cx);
        cx.notify();
    }

    /// The next page of history (the first when there's none yet).
    fn load_page(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.exhausted {
            return;
        }
        self.loading = true;
        let (root, request, skip, generation) = (
            self.root.clone(),
            self.applied.clone(),
            self.entries.len(),
            self.generation,
        );
        let task = cx.background_spawn(async move {
            // Read before the first page, so a ref moving meanwhile shows
            // next time.
            let tips = (skip == 0)
                .then(|| impulse_git::log::tips(&root, &request.scope))
                .flatten();
            let page = impulse_git::log::entries(
                &root,
                &request.scope,
                request.path.as_deref(),
                &request.query,
                skip,
                PAGE,
            );
            (tips, page)
        });
        cx.spawn(async move |this, cx| {
            let (tips, page) = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                match page {
                    Ok(page) => {
                        view.error = None;
                        view.exhausted = page.len() < PAGE;
                        if skip == 0 {
                            view.tips = tips;
                        }
                        view.append(page);
                        if view.reveal.is_some() {
                            view.reveal_pending(cx);
                        } else if view.selected.is_none() {
                            if let Some(first) = view.visible.first().copied() {
                                view.select(first, cx);
                            }
                        }
                    }
                    Err(error) => {
                        view.error = Some(error.to_string());
                        view.exhausted = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Add a page: only its rows are laid out and filtered.
    fn append(&mut self, page: Vec<LogEntry>) {
        let start = self.entries.len();
        let commits: Vec<GraphCommit> = page
            .iter()
            .map(|e| GraphCommit::new(e.sha.clone(), e.parents.clone()))
            .collect();
        let rows = self.graph.append(&commits);
        self.max_lanes = self
            .max_lanes
            .max(rows.iter().map(|r| r.width).max().unwrap_or(1))
            .min(MAX_LANES);
        self.rows.extend(rows);
        self.keys.extend(page.iter().map(SearchKey::new));
        self.entries.extend(page);
        let text = self.search_text();
        let kept: Vec<usize> = (start..self.entries.len())
            .filter(|&i| self.keeps(i, &text))
            .collect();
        self.visible.extend(kept);
    }

    /// Show `history` from scratch, its graph laid out again.
    fn replace(&mut self, history: Vec<LogEntry>) {
        self.entries.clear();
        self.rows.clear();
        self.keys.clear();
        self.visible.clear();
        self.graph = GraphLayout::default();
        self.max_lanes = 1;
        self.append(history);
    }

    fn search_text(&self) -> String {
        self.query.text.trim().to_lowercase()
    }

    fn keeps(&self, index: usize, text: &str) -> bool {
        text.is_empty() || self.keys[index].matches(text, &self.entries[index].sha)
    }

    fn update_visible(&mut self) {
        let text = self.search_text();
        self.visible = (0..self.entries.len())
            .filter(|&i| self.keeps(i, &text))
            .collect();
    }

    /// The graph only makes sense for unfiltered history.
    fn shows_graph(&self) -> bool {
        self.query == HistoryQuery::default()
    }

    /// On HEAD's history, from before the branch forked off the default
    /// branch.
    fn is_before_fork(&self, sha: &str) -> bool {
        self.scope == LogScope::Head
            && self
                .fork
                .as_ref()
                .is_some_and(|fork| !fork.branch_only.contains(sha))
    }

    /// Divergence from the upstream, the fork point, and the menus' context.
    fn load_surroundings(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let task = cx.background_spawn(async move {
            let divergence = impulse_git::log::divergence(&root, SURROUNDINGS_LIMIT);
            let fork = impulse_git::log::fork_point(&root, SURROUNDINGS_LIMIT);
            let context = HistoryContext {
                branch: impulse_git::operations::current_branch(&root),
                remotes: impulse_git::operations::remotes(&root),
                tag_remote: impulse_git::operations::default_remote(&root, None),
            };
            (divergence, fork, context)
        });
        cx.spawn(async move |this, cx| {
            let ((outgoing, incoming), fork, context) = task.await;
            let _ = this.update(cx, |view, cx| {
                view.outgoing = outgoing;
                view.incoming = incoming;
                view.fork = fork;
                view.context = context;
                cx.notify();
            });
        })
        .detach();
    }

    /// The filter's text changed: filter what's loaded now, and read history
    /// again once typing settles if its tokens changed.
    fn filter_changed(&mut self, cx: &mut Context<Self>) {
        self.query = HistoryQuery::parse(&self.filter.read(cx).value());
        self.update_visible();
        cx.notify();
        if self.query.server() == self.applied.query {
            self._query_reload = None;
            return;
        }
        self._query_reload = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.query.server() != view.applied.query {
                    view.reload(cx);
                }
            });
        }));
    }

    fn set_filter_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_filter(&text, window, cx);
    }

    /// The repository changed: once it settles, bring the loaded commits up
    /// to date in place.
    fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        self._refresh = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let _ = this.update(cx, |view, cx| view.refresh_in_place(cx));
        }));
    }

    /// When the history still starts from the same commits, only the
    /// decorations are read again; otherwise every loaded commit is. The
    /// selection and scroll position stay.
    fn refresh_in_place(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let count = self.entries.len().max(PAGE);
        let (root, request, loaded, generation) = (
            self.root.clone(),
            self.applied.clone(),
            self.tips.clone(),
            self.generation,
        );
        let task = cx.background_spawn(async move {
            let current = impulse_git::log::tips(&root, &request.scope);
            if current.is_some() && current == loaded {
                let decorations = impulse_git::log::decorations(&root).ok();
                (current, decorations, None)
            } else {
                let page = impulse_git::log::entries(
                    &root,
                    &request.scope,
                    request.path.as_deref(),
                    &request.query,
                    0,
                    count,
                )
                .ok();
                (current, None, page)
            }
        });
        cx.spawn(async move |this, cx| {
            let (current, decorations, page) = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                if let Some(decorations) = decorations {
                    view.tips = current;
                    view.apply_decorations(decorations, cx);
                } else if let Some(page) = page {
                    view.tips = current;
                    if page != view.entries {
                        view.exhausted = page.len() < count;
                        view.replace(page);
                        cx.notify();
                    }
                } else {
                    // Not read: the next refresh reads everything again.
                    view.tips = None;
                }
            });
        })
        .detach();
        self.load_surroundings(cx);
    }

    /// Put the refs from `decorations` on the loaded commits.
    fn apply_decorations(
        &mut self,
        decorations: HashMap<String, Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        for (entry, key) in self.entries.iter_mut().zip(self.keys.iter_mut()) {
            let refs = decorations.get(&entry.sha).cloned().unwrap_or_default();
            if refs != entry.refs {
                entry.refs = refs;
                *key = SearchKey::new(entry);
                changed = true;
            }
        }
        if changed {
            self.update_visible();
            cx.notify();
        }
    }

    // MARK: Selection

    /// Select commit `sha` (it may be abbreviated), loading pages until
    /// it's there (at most a few thousand commits back).
    pub fn reveal(&mut self, sha: String, cx: &mut Context<Self>) {
        self.reveal = Some(sha);
        self.reveal_pending(cx);
    }

    fn reveal_pending(&mut self, cx: &mut Context<Self>) {
        let Some(sha) = self.reveal.clone() else {
            return;
        };
        let found = self.visible.iter().copied().find(|&i| {
            let full = &self.entries[i].sha;
            full.starts_with(&sha) || sha.starts_with(full.as_str())
        });
        if let Some(index) = found {
            self.reveal = None;
            self.select(index, cx);
        } else if self.exhausted || self.entries.len() > 20 * PAGE {
            self.reveal = None;
            if self.selected.is_none() {
                if let Some(first) = self.visible.first().copied() {
                    self.select(first, cx);
                }
            }
        } else {
            self.load_page(cx);
        }
    }

    /// Show `sha` (a parent chip): select it and its changes.
    fn show(&mut self, sha: String, cx: &mut Context<Self>) {
        self.show_commit(sha.clone(), cx);
        self.reveal(sha, cx);
    }

    /// The details and changes of `sha`, whether or not it's loaded.
    fn show_commit(&mut self, sha: String, cx: &mut Context<Self>) {
        self.selected = Some(sha.clone());
        self.expanded = false;
        self.review.update(cx, |review, cx| {
            review.set_scope(DiffScope::Commit { sha: sha.clone() }, cx)
        });
        let root = self.root.clone();
        let task = cx.background_spawn(async move { impulse_git::log::details(&root, &sha) });
        cx.spawn(async move |this, cx| {
            let details = task.await;
            let _ = this.update(cx, |view, cx| {
                view.details = details;
                cx.notify();
            });
        })
        .detach();
    }

    /// Select entry `index`.
    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        let sha = entry.sha.clone();
        if let Some(row) = self.visible.iter().position(|&i| i == index) {
            self.scroll
                .scroll_to_item(row, gpui_kit::ScrollStrategy::Center);
            if row + 50 >= self.visible.len() {
                self.load_page(cx);
            }
        }
        self.show_commit(sha, cx);
        cx.notify();
    }

    fn selected_row(&self) -> Option<usize> {
        let sha = self.selected.as_deref()?;
        self.visible
            .iter()
            .position(|&i| self.entries[i].sha == sha)
    }

    /// Up and Down through the visible rows.
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta: i64 = match event.keystroke.key.as_str() {
            "down" => 1,
            "up" => -1,
            _ => return,
        };
        if self.visible.is_empty() {
            return;
        }
        let row = self.selected_row().map(|r| r as i64).unwrap_or(-1);
        let next = (row + delta).clamp(0, self.visible.len() as i64 - 1) as usize;
        self.select(self.visible[next], cx);
        cx.stop_propagation();
    }

    /// Copy `text` to the clipboard.
    fn copy(text: String, cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    // MARK: Drawing

    fn lane_colors(&self) -> Vec<KitRgba> {
        let p = &self.palette;
        vec![
            p.accent,
            p.success,
            p.warning,
            p.info,
            p.danger,
            p.git_renamed,
        ]
    }

    fn refs(&self, entry: &LogEntry) -> Vec<RefDecoration> {
        entry
            .refs
            .iter()
            .map(|r| RefDecoration::parse(r, &self.context.remotes))
            .collect()
    }

    fn ref_chip(
        &self,
        id: (&'static str, usize),
        decoration: RefDecoration,
        entry: &LogEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = &self.palette;
        let (color, name) = match &decoration {
            RefDecoration::Tag(_) => (p.warning, "tag"),
            RefDecoration::RemoteBranch { .. } => (p.text_secondary, "globe"),
            _ => (p.accent, "git-branch"),
        };
        let remote = matches!(decoration, RefDecoration::RemoteBranch { .. });
        let head = matches!(
            decoration,
            RefDecoration::Head { .. } | RefDecoration::DetachedHead
        );
        let label = decoration.label();
        let view = cx.entity();
        let entry = entry.clone();
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(3.))
            .px(px(5.))
            .h(px(17.))
            .rounded(px(4.))
            .bg(hsla(
                if remote { p.text_tertiary } else { color }.opacity(0.14),
            ))
            .text_size(px(10.5))
            .font_weight(if head {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
            .text_color(hsla(color))
            .child(icon(name, 9., color))
            .child(div().whitespace_nowrap().child(label))
            .context_menu(move |menu, _, cx| {
                let context = view.read(cx).context.clone();
                menu::ref_items(menu, &view, &decoration, &entry, &context)
            })
            .into_any_element()
    }

    fn row(
        &self,
        index: usize,
        lanes: usize,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = &self.palette;
        let entry = &self.entries[index];
        let selected = self.selected.as_deref() == Some(entry.sha.as_str());
        let head = entry.refs.iter().any(|r| r.starts_with("HEAD"));
        let row = self.rows.get(index).cloned();
        let colors = self.lane_colors();
        let before_fork = self.is_before_fork(&entry.sha);
        let fork_here =
            self.scope == LogScope::Head && self.fork.as_ref().is_some_and(|f| f.sha == entry.sha);
        let mut line = div()
            .id(("commit", index))
            .w_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(ROW_HEIGHT))
            .pl(px(if lanes > 0 { 2. } else { 10. }))
            .pr(px(10.))
            .when(selected, |d| d.bg(hsla(p.selection)))
            .when(selected && focused, |d| {
                d.border(px(1.5)).border_color(hsla(p.focus_ring))
            })
            .when(!selected, |d| d.hover(|s| s.bg(hsla(p.hover))))
            .on_click(cx.listener(move |view, _, window, cx| {
                window.focus(&view.focus, cx);
                view.select(index, cx)
            }))
            .when(lanes > 0, |d| {
                d.child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            if let Some(row) = &row {
                                paint_graph(row, lanes, &colors, bounds, window);
                            }
                        },
                    )
                    .flex_none()
                    .w(px(lanes as f32 * LANE_WIDTH + 6.))
                    .h(px(ROW_HEIGHT)),
                )
            });
        if self.outgoing.contains(&entry.sha) {
            line = line.child(icon("arrow-up", 10., p.accent));
        } else if self.incoming.contains(&entry.sha) {
            line = line.child(icon("arrow-down", 10., p.info));
        }
        if self
            .compare_base
            .as_ref()
            .is_some_and(|b| b.sha == entry.sha)
        {
            line = line.child(icon("git-compare", 11., p.warning));
        }
        for (n, decoration) in self.refs(entry).into_iter().enumerate() {
            line = line.child(self.ref_chip(("ref", index * 64 + n), decoration, entry, cx));
        }
        if fork_here {
            line = line.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(17.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(hsla(p.text_tertiary.opacity(0.5)))
                    .child(icon("git-fork", 10., p.text_secondary)),
            );
        }
        let view = cx.entity();
        let menu_entry = entry.clone();
        line.child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(hsla(if before_fork { p.text_tertiary } else { p.text }))
                .when(head, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(entry.subject.clone()),
        )
        .child(
            div()
                .flex_none()
                .max_w(px(110.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(11.))
                .text_color(hsla(p.text_tertiary))
                .child(entry.author.clone()),
        )
        .child(
            div()
                .flex_none()
                .w(px(58.))
                .flex()
                .justify_end()
                .text_size(px(11.))
                .text_color(hsla(p.text_tertiary))
                .child(dates::abbreviated_relative(dates::seconds_ago(entry.date))),
        )
        .child(
            div()
                .flex_none()
                .font_family("JetBrains Mono")
                .text_size(px(10.5))
                .text_color(hsla(p.text_tertiary))
                .child(entry.short_sha().to_string()),
        )
        .context_menu(move |menu, window, cx| {
            menu::commit_items(menu, &view, &menu_entry, window, cx)
        })
    }

    fn header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let mut header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(36.))
            .px(px(10.))
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(icon("history", 13., p.text_secondary));
        header = match &self.path {
            Some(path) => header.child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_family("JetBrains Mono")
                    .text_size(px(11.5))
                    .text_color(hsla(p.text))
                    .child(path.clone()),
            ),
            None => header.child(self.scope_switch(cx)),
        };
        let filtering = self.query.has_server_filters();
        let view = cx.entity();
        let empty = self.filter.read(cx).value().is_empty();
        header
            .child(div().flex_1().min_w(px(4.)))
            .child(
                crate::workbench_chrome::chrome_text_field(&self.filter, "search", p, window, cx)
                    .w(px(300.))
                    .flex_shrink(1.),
            )
            .child(
                gpui_kit::component::button::Button::new("history-presets")
                    .ghost()
                    .compact()
                    .tooltip("Filter presets")
                    .child(icon(
                        "list-filter",
                        13.,
                        if filtering {
                            p.accent
                        } else {
                            p.text_secondary
                        },
                    ))
                    .dropdown_menu(move |menu, _, _| {
                        let preset =
                            |label: &'static str, key: &'static str, value: &'static str| {
                                let view = view.clone();
                                PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                    view.update(cx, |view, cx| {
                                        let text = view.filter.read(cx).value().to_string();
                                        let next = HistoryQuery::setting(key, Some(value), &text);
                                        view.set_filter_text(next, window, cx)
                                    })
                                })
                            };
                        let mine = {
                            let view = view.clone();
                            PopupMenuItem::new("My Commits").on_click(move |_, window, cx| {
                                let root = view.read(cx).root.clone();
                                let Some(name) = impulse_git::log::user_name(&root) else {
                                    return;
                                };
                                view.update(cx, |view, cx| {
                                    let text = view.filter.read(cx).value().to_string();
                                    let next = HistoryQuery::setting("author", Some(&name), &text);
                                    view.set_filter_text(next, window, cx)
                                })
                            })
                        };
                        let clear = {
                            let view = view.clone();
                            PopupMenuItem::new("Clear Filter").disabled(empty).on_click(
                                move |_, window, cx| {
                                    view.update(cx, |view, cx| {
                                        view.set_filter_text(String::new(), window, cx)
                                    })
                                },
                            )
                        };
                        menu.item(mine)
                            .separator()
                            .item(preset("Last 7 Days", "since", "1w"))
                            .item(preset("Last 30 Days", "since", "30d"))
                            .item(preset("Last Year", "since", "1y"))
                            .separator()
                            .item(clear)
                    }),
            )
    }

    /// Current branch / All branches, plus the branch History was opened on
    /// (Manage Branches ▸ Show History).
    fn scope_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let mut options = vec![
            (LogScope::Head, "Current branch".to_string()),
            (LogScope::All, "All branches".to_string()),
        ];
        if let LogScope::Branch(name) = &self.scope {
            options.push((self.scope.clone(), name.clone()));
        }
        let mut switch = div()
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(metrics::RADIUS_SMALL + 2.))
            .bg(hsla(p.hover))
            .border_1()
            .border_color(hsla(p.hairline));
        for (n, (scope, label)) in options.into_iter().enumerate() {
            let selected = scope == self.scope;
            switch = switch.child(
                div()
                    .id(("history-scope", n))
                    .flex()
                    .items_center()
                    .px(px(10.))
                    .h(px(20.))
                    .rounded(px(metrics::RADIUS_SMALL))
                    .text_size(px(11.5))
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(hsla(if selected { p.text } else { p.text_secondary }))
                    .when(selected, |d| d.bg(hsla(p.raised)).shadow_xs())
                    .cursor_pointer()
                    .whitespace_nowrap()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if !matches!(scope, LogScope::Branch(_)) {
                            view.set_scope(scope.clone(), cx)
                        }
                    }))
                    .child(label),
            );
        }
        switch
    }

    fn sha_chip(
        &self,
        id: (&'static str, usize),
        sha: &str,
        name: &'static str,
        tooltip: &'static str,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let p = &self.palette;
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(3.))
            .px(px(5.))
            .h(px(18.))
            .rounded(px(4.))
            .bg(hsla(p.raised))
            .cursor_pointer()
            .text_color(hsla(p.text_secondary))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip).build(window, cx)
            })
            .child(icon(name, 9., p.text_secondary))
            .child(
                div()
                    .font_family("JetBrains Mono")
                    .text_size(px(10.5))
                    .child(sha.chars().take(7).collect::<String>()),
            )
    }

    /// The selected commit: message, who and when, SHA and parents.
    fn details_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.palette;
        let Some(details) = &self.details else {
            return div().into_any_element();
        };
        let subject = details.subject().to_string();
        let body = details.body();
        let lines = body.lines().count();
        let shown: String = if self.expanded || lines <= 4 {
            body.clone()
        } else {
            body.lines().take(4).collect::<Vec<_>>().join("\n")
        };
        let sha = details.sha.clone();
        let mut chips = div().flex().items_center().gap(px(6.)).child(
            self.sha_chip(("sha", 0), &details.sha, "copy", "Copy SHA")
                .on_click(move |_, _, cx| Self::copy(sha.clone(), cx)),
        );
        for (n, parent) in details.parents.iter().enumerate() {
            let parent = parent.clone();
            chips = chips.child(
                self.sha_chip(("parent", n), &parent, "arrow-down", "Show parent")
                    .on_click(cx.listener(move |view, _, _, cx| view.show(parent.clone(), cx))),
            );
        }
        div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(6.))
            .px(px(12.))
            .py(px(10.))
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(p.text))
                    .child(subject),
            )
            .when(!body.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(hsla(p.text_secondary))
                        .child(shown),
                )
                .when(lines > 4, |d| {
                    d.child(
                        div()
                            .id("show-more")
                            .text_size(px(11.))
                            .text_color(hsla(p.accent))
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.expanded = !view.expanded;
                                cx.notify();
                            }))
                            .child(if self.expanded {
                                "Show less"
                            } else {
                                "Show more"
                            }),
                    )
                })
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hsla(p.text))
                            .child(details.author.clone()),
                    )
                    .child(
                        div()
                            .text_color(hsla(p.text_tertiary))
                            .child(dates::local_medium_date_time(details.date)),
                    )
                    .when(details.committer != details.author, |d| {
                        d.child(
                            div()
                                .text_color(hsla(p.text_tertiary))
                                .child(format!("· committed by {}", details.committer)),
                        )
                    })
                    .child(div().flex_1().min_w(px(6.)))
                    .child(chips),
            )
            .into_any_element()
    }
}

/// One row's graph: lane lines curving into and out of the commit's dot,
/// a ring for a merge (`GraphCell`).
fn paint_graph(
    row: &GraphRow,
    lanes: usize,
    colors: &[KitRgba],
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let mid = bounds.size.height / 2.;
    let x = |lane: usize| bounds.origin.x + px(9. + lane as f32 * LANE_WIDTH);
    let top = bounds.origin.y;
    let color = |lane: usize| -> Hsla { hsla(colors[lane % colors.len()]) };
    let curve = |from: gpui_kit::Point<Pixels>,
                 to: gpui_kit::Point<Pixels>,
                 window: &mut Window,
                 c: Hsla| {
        let mut path = PathBuilder::stroke(px(1.6));
        path.move_to(from);
        let dy = to.y - from.y;
        path.cubic_bezier_to(
            to,
            point(from.x, from.y + dy * 0.6),
            point(to.x, from.y + dy * 0.6),
        );
        if let Ok(path) = path.build() {
            window.paint_path(path, c);
        }
    };
    for s in row.top.iter().filter(|s| s.from < lanes || s.to < lanes) {
        curve(
            point(x(s.from), top),
            point(x(s.to), top + mid),
            window,
            color(s.from),
        );
    }
    for s in row.bottom.iter().filter(|s| s.from < lanes || s.to < lanes) {
        curve(
            point(x(s.from), top + mid),
            point(x(s.to), top + bounds.size.height),
            window,
            color(s.to),
        );
    }
    let center = point(x(row.lane), top + mid);
    let dot = Bounds::new(
        center - point(px(4.), px(4.)),
        gpui_kit::size(px(8.), px(8.)),
    );
    if row.is_merge() {
        window.paint_quad(gpui_kit::quad(
            dot,
            px(4.),
            gpui_kit::transparent_black(),
            px(1.6),
            color(row.lane),
            Default::default(),
        ));
    } else {
        window.paint_quad(gpui_kit::fill(dot, color(row.lane)).corner_radii(px(4.)));
    }
}

impl Render for HistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let lanes = if self.shows_graph() {
            self.max_lanes
        } else {
            0
        };
        let focused = self.focus.is_focused(window);
        let count = self.visible.len();
        let view = cx.entity();
        let list: AnyElement = match &self.error {
            Some(error) => div()
                .p(px(16.))
                .text_size(px(12.))
                .text_color(hsla(p.danger))
                .child(error.clone())
                .into_any_element(),
            None => uniform_list("history", count, move |range, _, cx| {
                view.update(cx, |view, cx| {
                    // The last rows are on screen: read the next page.
                    if range.end + 20 >= view.visible.len() && !view.loading && !view.exhausted {
                        let this = cx.entity();
                        cx.defer(move |cx| this.update(cx, |view, cx| view.load_page(cx)));
                    }
                    range
                        .map(|row| {
                            let index = view.visible[row];
                            view.row(index, lanes, focused, cx).into_any_element()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .track_scroll(&self.scroll)
            .flex_1()
            .into_any_element(),
        };
        div()
            .id("history-view")
            .key_context("History")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.content))
            // The graph (with its header) gets 40% of the height.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .h(gpui_kit::relative(0.4))
                    .min_h(px(140.))
                    .border_b_1()
                    .border_color(hsla(p.hairline))
                    .child(self.header(window, cx))
                    .child(list),
            )
            .child(self.details_bar(cx))
            .child(div().flex_1().min_h_0().child(self.review.clone()))
    }
}

impl gpui_kit::Focusable for HistoryView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
