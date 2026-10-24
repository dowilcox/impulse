//! The sidebar's project search (the Swift `SidebarSearchBar` and
//! `SearchResultsList`): a field searching file contents as you type
//! (Return searches at once), Match Case, a Replace row whose matches show
//! struck through with the replacement after them, and Replace All, which
//! asks first, leaves files with unsaved edits alone, and can be undone
//! while the files still hold what it wrote.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    div, prelude::*, px, Context, Entity, EventEmitter, FontWeight, SharedString, StyledText,
    Subscription, Task, TextRun, Window,
};
use impulse_git::search::SearchResult;
use impulse_kit::chrome::{metrics, ChromePalette};
use impulse_kit::project_replace::{self, Segment};

use crate::workbench_chrome::{hsla, icon};

/// How many results the list shows (the Swift content search limit).
const LIMIT: usize = 500;

pub enum SearchPanelEvent {
    Open(PathBuf, Option<u32>),
    ReplaceAll {
        query: String,
        replacement: String,
        case_sensitive: bool,
    },
}

pub struct SearchPanel {
    pub root: String,
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    pub case_sensitive: bool,
    replace_visible: bool,
    pub results: Vec<SearchResult>,
    searching: bool,
    searched: String,
    palette: ChromePalette,
    _search: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SearchPanelEvent> for SearchPanel {}

impl SearchPanel {
    pub fn new(palette: ChromePalette, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search project…"));
        let replacement = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with…"));
        let subscriptions = vec![
            cx.subscribe(&query, |panel, _, event: &InputEvent, cx| match event {
                InputEvent::Change => panel.schedule(Duration::from_millis(250), cx),
                InputEvent::PressEnter { .. } => panel.schedule(Duration::ZERO, cx),
                _ => {}
            }),
            cx.subscribe(
                &replacement,
                |panel, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => panel.replace_all(cx),
                    _ => {}
                },
            ),
        ];
        SearchPanel {
            root: String::new(),
            query,
            replacement,
            case_sensitive: false,
            replace_visible: false,
            results: Vec::new(),
            searching: false,
            searched: String::new(),
            palette,
            _search: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_palette(&mut self, palette: ChromePalette, cx: &mut Context<Self>) {
        self.palette = palette;
        cx.notify();
    }

    /// Search this folder from now on (the active workspace's).
    pub fn set_root(&mut self, root: String, cx: &mut Context<Self>) {
        if root != self.root {
            self.root = root;
            self.schedule(Duration::ZERO, cx);
        }
    }

    /// Search for `text` now (the snapshot's `search=` action).
    pub fn search_for(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
        self.schedule(Duration::ZERO, cx);
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |q, cx| q.focus(window, cx));
    }

    fn query_text(&self, cx: &gpui_kit::App) -> String {
        self.query.read(cx).value().to_string()
    }

    /// Search again after `delay` (typing coalesces).
    pub fn schedule(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let query = self.query_text(cx);
        if query.trim().is_empty() || self.root.is_empty() {
            self.results.clear();
            self.searching = false;
            self.searched.clear();
            self._search = None;
            cx.notify();
            return;
        }
        let (root, case_sensitive) = (self.root.clone(), self.case_sensitive);
        self.searching = true;
        cx.notify();
        self._search = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let needle = query.clone();
            let results = cx
                .background_spawn(async move {
                    impulse_git::search::search_contents(&root, &needle, LIMIT, case_sensitive)
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.results = results;
                panel.searching = false;
                panel.searched = query;
                cx.notify();
            });
        }));
    }

    fn replace_all(&mut self, cx: &mut Context<Self>) {
        let query = self.query_text(cx);
        if query.is_empty() || !self.results.iter().any(|r| r.match_type == "content") {
            return;
        }
        cx.emit(SearchPanelEvent::ReplaceAll {
            query,
            replacement: self.replacement.read(cx).value().to_string(),
            case_sensitive: self.case_sensitive,
        });
    }

    /// The line with each match struck through and the replacement after it.
    fn preview(&self, line: &str, replacement: &str) -> StyledText {
        let p = &self.palette;
        let segments =
            project_replace::preview(line, &self.searched, replacement, self.case_sensitive);
        let mut text = String::new();
        let mut runs = Vec::new();
        let font = gpui_kit::font("JetBrains Mono");
        for segment in segments {
            let (part, color, strike) = match segment {
                Segment::Same(part) => (part, p.text_secondary, false),
                Segment::Removed(part) => (part, p.danger, true),
                Segment::Added(part) => (part, p.success, false),
            };
            text.push_str(&part);
            runs.push(TextRun {
                len: part.len(),
                font: font.clone(),
                color: hsla(color),
                background_color: None,
                underline: None,
                strikethrough: strike.then(|| gpui_kit::StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(hsla(color)),
                }),
            });
        }
        StyledText::new(text).with_runs(runs)
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.clone();
        let replacing = self.replace_visible;
        let replacement = self.replacement.read(cx).value().to_string();
        let has_content = self.results.iter().any(|r| r.match_type == "content");
        let toggle = |id: &'static str, on: bool| {
            div()
                .id(id)
                .px(px(6.))
                .py(px(2.))
                .rounded(px(4.))
                .font_family("JetBrains Mono")
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .text_color(hsla(if on { p.accent } else { p.text_secondary }))
                .when(on, |d| d.bg(hsla(p.accent.opacity(0.15))))
        };
        let mut list = div()
            .id("search-results")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if self.searching && self.results.is_empty() {
            list = list.child(
                div()
                    .p(px(12.))
                    .text_size(px(12.))
                    .text_color(hsla(p.text_tertiary))
                    .child("Searching…"),
            );
        } else if self.results.is_empty() && !self.searched.is_empty() {
            list = list.child(
                div()
                    .flex()
                    .justify_center()
                    .p(px(16.))
                    .text_size(px(12.))
                    .text_color(hsla(p.text_secondary))
                    .child("No results"),
            );
        } else if !self.results.is_empty() {
            list = list.child(
                div()
                    .px(px(10.))
                    .py(px(4.))
                    .text_size(px(11.))
                    .text_color(hsla(p.text_tertiary))
                    .child(format!("{} results", self.results.len())),
            );
            for (index, result) in self.results.iter().enumerate() {
                let path = PathBuf::from(&result.path);
                let line = result.line_number;
                let content = result.line_content.as_deref().map(str::trim).unwrap_or("");
                list = list.child(
                    div()
                        .id(("result", index))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .px(px(10.))
                        .py(px(5.))
                        .cursor_pointer()
                        .hover(|s| s.bg(hsla(p.hover)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SearchPanelEvent::Open(path.clone(), line))
                        }))
                        .child(
                            div()
                                .flex()
                                .text_size(px(12.))
                                .line_height(px(15.))
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .child(result.name.clone()),
                                )
                                .when_some(line, |d, line| {
                                    d.child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(hsla(p.text_secondary))
                                            .child(format!(":{line}")),
                                    )
                                }),
                        )
                        .when(!content.is_empty(), |d| {
                            d.child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(11.))
                                    .line_height(px(13.))
                                    .text_color(hsla(p.text_secondary))
                                    .child(if replacing {
                                        self.preview(content, &replacement).into_any_element()
                                    } else {
                                        SharedString::from(content.to_string()).into_any_element()
                                    }),
                            )
                        }),
                );
            }
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(10.))
                    .py(px(6.))
                    .child(
                        div()
                            .id("toggle-replace")
                            .w(px(10.))
                            .cursor_pointer()
                            .on_click(cx.listener(|panel, _, _, cx| {
                                panel.replace_visible = !panel.replace_visible;
                                cx.notify();
                            }))
                            .child(icon(
                                if replacing {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                metrics::ICON_SMALL,
                                p.text_secondary,
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.query).small().cleanable(true)),
                    )
                    .child(
                        toggle("match-case", self.case_sensitive)
                            .child("Aa")
                            .on_click(cx.listener(|panel, _, _, cx| {
                                panel.case_sensitive = !panel.case_sensitive;
                                panel.schedule(Duration::ZERO, cx);
                            })),
                    ),
            )
            .when(replacing, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(10.))
                        .pb(px(6.))
                        .child(div().w(px(10.)))
                        .child(div().flex_1().child(Input::new(&self.replacement).small()))
                        .child(
                            div()
                                .id("replace-all")
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .cursor_pointer()
                                .text_color(hsla(if has_content {
                                    p.accent
                                } else {
                                    p.text_tertiary
                                }))
                                .on_click(cx.listener(|panel, _, _, cx| panel.replace_all(cx)))
                                .child("Replace All"),
                        ),
                )
            })
            .child(list)
    }
}
