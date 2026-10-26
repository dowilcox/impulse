//! The branch switcher (the Swift palette's `b:` mode): local branches
//! (the current one first, then most recently committed), then remote ones
//! without a local branch, filtered as you type; Enter or a click switches
//! (a remote branch gets a local tracking branch, as `git switch` makes
//! one), and a name no branch has offers "Create branch".

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    div, prelude::*, px, Context, Entity, FocusHandle, Focusable, KeyDownEvent, ScrollHandle,
    SharedString, Subscription, WeakEntity, Window,
};
use impulse_kit::chrome::{metrics, ChromePalette};

use crate::repository_state::RepositoryState;
use crate::workbench::Workbench;
use crate::workbench_chrome::{hsla, icon};

/// A row of the list.
#[derive(Clone)]
enum Row {
    Branch { name: String, remote: bool },
    Create(String),
}

pub struct BranchSwitcher {
    palette: ChromePalette,
    workbench: WeakEntity<Workbench>,
    repo: Entity<RepositoryState>,
    query: Entity<InputState>,
    current: Option<String>,
    /// Local branches then remote ones; `None` while loading.
    branches: Option<Vec<(String, bool)>>,
    selected: usize,
    /// The list's scroll, kept on the selection as Up and Down move it.
    scroll: ScrollHandle,
    _query_events: Subscription,
}

/// "origin/feature" is "feature" (git switch creates a tracking branch).
fn local_name(remote: &str) -> &str {
    remote.split_once('/').map_or(remote, |(_, rest)| rest)
}

impl BranchSwitcher {
    fn new(
        workbench: WeakEntity<Workbench>,
        repo: Entity<RepositoryState>,
        palette: ChromePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Switch to branch…"));
        let events = cx.subscribe_in(&query, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.selected = 0;
                this.scroll.scroll_to_item(0);
                cx.notify();
            }
        });
        let (root, trusted, current) = {
            let repo = repo.read(cx);
            (
                repo.root.clone(),
                repo.trusted,
                repo.snapshot.as_ref().and_then(|s| s.branch.clone()),
            )
        };
        let list = cx.background_spawn(async move {
            RepositoryState::read_git(
                &root,
                trusted,
                &[
                    "for-each-ref",
                    "--sort=-committerdate",
                    "--format=%(refname)",
                    "refs/heads",
                    "refs/remotes",
                ],
            )
        });
        let current_for_list = current.clone();
        cx.spawn(async move |this, cx| {
            let output = list.await.unwrap_or_default();
            let mut local = Vec::new();
            let mut remote = Vec::new();
            for line in output.lines() {
                if let Some(name) = line.strip_prefix("refs/heads/") {
                    local.push(name.to_string());
                } else if let Some(name) = line.strip_prefix("refs/remotes/") {
                    if !line.ends_with("/HEAD") {
                        remote.push(name.to_string());
                    }
                }
            }
            let current = current_for_list;
            let mut branches: Vec<(String, bool)> = local
                .iter()
                .filter(|b| Some(*b) == current.as_ref())
                .chain(local.iter().filter(|b| Some(*b) != current.as_ref()))
                .map(|b| (b.clone(), false))
                .collect();
            branches.extend(
                remote
                    .into_iter()
                    .filter(|r| !local.iter().any(|l| l == local_name(r)))
                    .map(|r| (r, true)),
            );
            let _ = this.update(cx, |this, cx| {
                this.branches = Some(branches);
                cx.notify();
            });
        })
        .detach();
        BranchSwitcher {
            palette,
            workbench,
            repo,
            query,
            current,
            branches: None,
            selected: 0,
            scroll: ScrollHandle::new(),
            _query_events: events,
        }
    }

    fn rows(&self, cx: &gpui_kit::App) -> Vec<Row> {
        let Some(branches) = &self.branches else {
            return Vec::new();
        };
        let term = self.query.read(cx).value().trim().to_string();
        let mut rows: Vec<Row> =
            impulse_kit::fuzzy::rank(branches, &term, 200, false, |(name, _)| name.as_str())
                .into_iter()
                .map(|((name, remote), _)| Row::Branch {
                    name: name.clone(),
                    remote: *remote,
                })
                .collect();
        if !term.is_empty()
            && !branches.iter().any(|(name, _)| *name == term)
            && impulse_git::types::is_valid_ref_name(&term)
        {
            rows.push(Row::Create(term));
        }
        rows
    }

    fn pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows(cx).get(index).cloned() else {
            return;
        };
        if let Row::Branch { name, .. } = &row {
            if Some(name) == self.current.as_ref() {
                window.close_dialog(cx);
                return;
            }
        }
        window.close_dialog(cx);
        let repo = self.repo.clone();
        let _ = self.workbench.update(cx, |workbench, cx| match row {
            Row::Branch { name, remote } => {
                let name = if remote {
                    local_name(&name).to_string()
                } else {
                    name
                };
                workbench.git_switch_branch(&repo, name, window, cx)
            }
            Row::Create(name) => workbench.git_create_branch(&repo, name, window, cx),
        });
    }

    fn move_selection(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.rows(cx).len();
        if count == 0 {
            return;
        }
        match event.keystroke.key.as_str() {
            "down" => self.selected = (self.selected + 1).min(count - 1),
            "up" => self.selected = self.selected.saturating_sub(1),
            _ => return,
        }
        self.scroll.scroll_to_item(self.selected);
        cx.stop_propagation();
        cx.notify();
    }
}

impl Focusable for BranchSwitcher {
    fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
}

impl Render for BranchSwitcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let rows = self.rows(cx);
        let selected = self.selected.min(rows.len().saturating_sub(1));
        let mut list = div()
            .id("branch-list")
            .flex()
            .flex_col()
            .max_h(px(320.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        for (index, row) in rows.iter().enumerate() {
            let (glyph, title, trailing) = match row {
                Row::Branch { name, remote } => (
                    if *remote { "globe" } else { "git-branch" },
                    name.clone(),
                    if Some(name) == self.current.as_ref() {
                        Some("current")
                    } else if *remote {
                        Some("remote")
                    } else {
                        None
                    },
                ),
                Row::Create(name) => (
                    "git-branch-plus",
                    format!("Create branch \u{201c}{name}\u{201d}"),
                    None,
                ),
            };
            let subtitle = match (row, &self.current) {
                (Row::Create(_), Some(current)) => Some(format!("from {current}")),
                _ => None,
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("branch-{index}")))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(px(metrics::RADIUS))
                    .when(index == selected, |d| d.bg(hsla(p.selection)))
                    .when(index != selected, |d| d.hover(|s| s.bg(hsla(p.hover))))
                    .on_click(cx.listener(move |this, _, window, cx| this.pick(index, window, cx)))
                    .child(icon(glyph, metrics::ICON, p.text_secondary))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(13.))
                            .child(title),
                    )
                    .children(subtitle.map(|s| {
                        div()
                            .text_size(px(11.5))
                            .text_color(hsla(p.text_tertiary))
                            .child(s)
                    }))
                    .children(trailing.map(|t| {
                        div()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_tertiary))
                            .child(t)
                    })),
            );
        }
        let empty = match &self.branches {
            None => Some("Loading branches…"),
            Some(branches) if branches.is_empty() => Some("Not a git repository"),
            Some(_) if rows.is_empty() => Some("No matching branches"),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .capture_key_down(cx.listener(Self::move_selection))
            .child(Input::new(&self.query))
            .children(empty.map(|message| {
                div()
                    .px(px(8.))
                    .py(px(6.))
                    .text_color(hsla(p.text_tertiary))
                    .child(message)
            }))
            .child(list)
    }
}

impl Workbench {
    /// Switch Branch…: the branch list for `repo`, as a dialog.
    pub(crate) fn show_branch_switcher(
        &mut self,
        repo: Entity<RepositoryState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workbench = cx.entity().downgrade();
        let palette = self.palette.clone();
        let switcher = cx.new(|cx| BranchSwitcher::new(workbench, repo, palette, window, cx));
        let focused = std::cell::Cell::new(false);
        crate::overlays::open_dialog(window, cx, move |dialog, window, cx| {
            if !focused.replace(true) {
                let handle = switcher.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
            let picker = switcher.clone();
            dialog
                .title("Switch Branch")
                .w(px(480.))
                // Enter in the field confirms the dialog: switch to the
                // selected row (which closes it).
                .on_ok(move |_, window, cx| {
                    picker.update(cx, |this, cx| this.pick(this.selected, window, cx));
                    false
                })
                .child(switcher.clone())
        });
    }
}
