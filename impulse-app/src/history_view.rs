//! History: a repository's commits with their lane graph, as the Swift
//! `HistorySurface` shows them: each row's graph cell (curved lane lines,
//! a dot, a ring for merges), branch and tag chips, the subject (HEAD's in
//! bold), author, relative date and short SHA, a page at a time; below,
//! the selected commit's message and details over its diff (the Review
//! list, for that commit). Up and Down move the selection.

use std::time::SystemTime;

use gpui_kit::{
    canvas, div, point, prelude::*, px, uniform_list, App, Bounds, Context, Entity, FocusHandle,
    FontWeight, Hsla, KeyDownEvent, PathBuilder, Pixels, UniformListScrollHandle, Window,
};
use impulse_git::log::{CommitDetails, LogEntry, LogScope};
use impulse_git::types::DiffScope;
use impulse_kit::chrome::{metrics, ChromePalette, Rgba as KitRgba};
use impulse_kit::git::commit_graph::{GraphCommit, GraphRow, Layout as GraphLayout};
use impulse_kit::git::refs::RefDecoration;
use impulse_kit::theme::ResolvedTheme;

use crate::review_view::ReviewView;
use crate::workbench_chrome::{hsla, icon};

const PAGE: usize = 300;
const ROW_HEIGHT: f32 = 28.0;
const LANE_WIDTH: f32 = 13.0;
const MAX_LANES: usize = 8;

pub struct HistoryView {
    pub root: String,
    /// The file or folder it follows (repository-relative), if any.
    pub path: Option<String>,
    entries: Vec<LogEntry>,
    rows: Vec<GraphRow>,
    graph: GraphLayout,
    remotes: Vec<String>,
    selected: Option<usize>,
    details: Option<CommitDetails>,
    review: Entity<ReviewView>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    palette: ChromePalette,
    loading: bool,
    exhausted: bool,
}

/// "3m", "2h", "5d", "4w", "8mo", "2y": the Swift abbreviated relative date.
fn relative(date: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(date)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (minute, hour, day) = (60, 3600, 86_400);
    match seconds {
        s if s < minute => "now".into(),
        s if s < hour => format!("{} min. ago", s / minute),
        s if s < day => format!("{} hr. ago", s / hour),
        s if s < 7 * day => format!("{} days ago", s / day),
        s if s < 30 * day => format!("{} wk. ago", s / (7 * day)),
        s if s < 365 * day => format!("{} mo. ago", s / (30 * day)),
        s => format!("{} yr. ago", s / (365 * day)),
    }
}

fn remotes(root: &str) -> Vec<String> {
    std::process::Command::new("git")
        .args(["remote"])
        .current_dir(root)
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

impl HistoryView {
    pub fn new(
        root: String,
        path: Option<String>,
        theme: &ResolvedTheme,
        palette: &ChromePalette,
        cx: &mut Context<Self>,
    ) -> Self {
        let review = cx.new(|cx| {
            let mut review = ReviewView::new(
                root.clone(),
                DiffScope::Commit { sha: "HEAD".into() },
                theme,
                palette,
                cx,
            );
            review.embedded = true;
            review
        });
        cx.observe(&review, |_, _, cx| cx.notify()).detach();
        let mut view = HistoryView {
            remotes: remotes(&root),
            root,
            path,
            entries: Vec::new(),
            rows: Vec::new(),
            graph: GraphLayout::default(),
            selected: None,
            details: None,
            review,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            palette: palette.clone(),
            loading: false,
            exhausted: false,
        };
        view.load_page(cx);
        view
    }

    pub fn title(&self) -> String {
        match &self.path {
            Some(path) => format!(
                "History · {}",
                std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone())
            ),
            None => "History".into(),
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

    /// The next page of history (the first when there's none yet).
    fn load_page(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.exhausted {
            return;
        }
        self.loading = true;
        let (root, path, skip) = (self.root.clone(), self.path.clone(), self.entries.len());
        let task = cx.background_spawn(async move {
            impulse_git::log::entries(
                &root,
                &LogScope::Head,
                path.as_deref(),
                &Default::default(),
                skip,
                PAGE,
            )
        });
        cx.spawn(async move |this, cx| {
            let page = task.await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                let page = page.unwrap_or_default();
                if page.len() < PAGE {
                    view.exhausted = true;
                }
                let commits: Vec<GraphCommit> = page
                    .iter()
                    .map(|e| GraphCommit::new(e.sha.clone(), e.parents.clone()))
                    .collect();
                view.rows.extend(view.graph.append(&commits));
                view.entries.extend(page);
                if view.selected.is_none() && !view.entries.is_empty() {
                    view.select(0, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        self.selected = Some(index);
        self.scroll
            .scroll_to_item(index, gpui_kit::ScrollStrategy::Center);
        let (root, sha) = (self.root.clone(), entry.sha.clone());
        self.review.update(cx, |review, cx| {
            review.set_scope(DiffScope::Commit { sha: sha.clone() }, cx)
        });
        let task = cx.background_spawn(async move { impulse_git::log::details(&root, &sha) });
        cx.spawn(async move |this, cx| {
            let details = task.await;
            let _ = this.update(cx, |view, cx| {
                view.details = details;
                cx.notify();
            });
        })
        .detach();
        if index + 50 >= self.entries.len() {
            self.load_page(cx);
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = match event.keystroke.key.as_str() {
            "down" => 1,
            "up" => -1,
            _ => return,
        };
        let next = self
            .selected
            .map(|s| (s as i64 + delta).clamp(0, self.entries.len() as i64 - 1) as usize)
            .unwrap_or(0);
        self.select(next, cx);
        cx.stop_propagation();
    }

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

    fn row(&self, index: usize, lanes: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let p = &self.palette;
        let entry = &self.entries[index];
        let selected = self.selected == Some(index);
        let head = entry.refs.iter().any(|r| r.starts_with("HEAD"));
        let row = self.rows.get(index).cloned();
        let colors = self.lane_colors();
        let mut chips = div().flex().gap(px(4.)).flex_none();
        for text in &entry.refs {
            let decoration = RefDecoration::parse(text, &self.remotes);
            let (color, name) = match &decoration {
                RefDecoration::Head { .. } | RefDecoration::DetachedHead => {
                    (p.accent, "git-branch")
                }
                RefDecoration::Tag(_) => (p.warning, "tag"),
                RefDecoration::RemoteBranch { .. } => (p.info, "globe"),
                _ => (p.success, "git-branch"),
            };
            chips = chips.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .px(px(5.))
                    .h(px(17.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(hsla(color.opacity(0.5)))
                    .bg(hsla(color.opacity(0.12)))
                    .text_size(px(10.5))
                    .text_color(hsla(color))
                    .child(icon(name, 10., color))
                    .child(decoration.label()),
            );
        }
        div()
            .id(("commit", index))
            .w_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(ROW_HEIGHT))
            .pl(px(if lanes > 0 { 2. } else { 10. }))
            .pr(px(10.))
            .when(selected, |d| d.bg(hsla(p.selection)))
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
            })
            .child(chips)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.))
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
                    .w(px(70.))
                    .flex()
                    .justify_end()
                    .text_size(px(11.))
                    .text_color(hsla(p.text_tertiary))
                    .child(relative(entry.date)),
            )
            .child(
                div()
                    .flex_none()
                    .font_family("JetBrains Mono")
                    .text_size(px(10.5))
                    .text_color(hsla(p.text_tertiary))
                    .child(entry.short_sha().to_string()),
            )
    }

    fn details_bar(&self) -> impl IntoElement {
        let p = &self.palette;
        let Some(details) = &self.details else {
            return div().into_any_element();
        };
        let mut message = details.message.trim().lines();
        let subject = message.next().unwrap_or_default().to_string();
        let body: String = message.collect::<Vec<_>>().join("\n").trim().to_string();
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .px(px(14.))
            .py(px(10.))
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.hairline))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(subject),
            )
            .when(!body.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(hsla(p.text_secondary))
                        .child(body),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.5))
                    .text_color(hsla(p.text_secondary))
                    .child(format!("{} <{}>", details.author, details.email))
                    .child(
                        div()
                            .text_color(hsla(p.text_tertiary))
                            .child(relative(details.date)),
                    )
                    .when(details.committer != details.author, |d| {
                        d.child(
                            div()
                                .text_color(hsla(p.text_tertiary))
                                .child(format!("· committed by {}", details.committer)),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .px(px(6.))
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .rounded(px(metrics::RADIUS_SMALL))
                            .bg(hsla(p.raised))
                            .font_family("JetBrains Mono")
                            .text_size(px(10.5))
                            .child(details.sha.chars().take(10).collect::<String>()),
                    ),
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let lanes = self
            .rows
            .iter()
            .map(|r| r.width)
            .max()
            .unwrap_or(0)
            .min(MAX_LANES);
        let count = self.entries.len();
        let view = cx.entity();
        let list = uniform_list("history", count, move |range, _, cx| {
            view.update(cx, |view, cx| {
                range
                    .map(|i| view.row(i, lanes, cx).into_any_element())
                    .collect::<Vec<_>>()
            })
        })
        .track_scroll(&self.scroll)
        .flex_1();
        div()
            .id("history-view")
            .key_context("History")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.content))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(36.))
                    .px(px(10.))
                    .bg(hsla(p.panel))
                    .border_b_1()
                    .border_color(hsla(p.hairline))
                    .child(icon("history", 13., p.text))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title()),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(hsla(p.text_secondary))
                            .child(format!(
                                "{count}{} commit{}",
                                if self.exhausted { "" } else { "+" },
                                if count == 1 { "" } else { "s" }
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .h(gpui_kit::relative(0.42))
                    .border_b_1()
                    .border_color(hsla(p.hairline))
                    .child(list),
            )
            .child(self.details_bar())
            .child(div().flex_1().min_h_0().child(self.review.clone()))
    }
}

impl gpui_kit::Focusable for HistoryView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
