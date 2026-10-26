//! Review's file list (the Swift `ReviewNavigatorView`): a filter field
//! (T or / puts the cursor there, Return goes to the first match), a bar of
//! how much is viewed, and the files under their folders, each with its
//! viewed checkbox, status, name, a dot when it changed since viewed, its
//! comment count and line stats. The file at the top of the diff is
//! highlighted and kept in view.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{div, prelude::*, px, Context, FontWeight, Window};

use super::{stat, status_letter, viewed_checkbox, ReviewView, NAVIGATOR_WIDTH};
use crate::workbench_chrome::hsla;

impl ReviewView {
    /// The filter field, made with the first render (it needs the window).
    fn nav_filter(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Entity<InputState> {
        if let Some(filter) = &self.nav_filter {
            return filter.clone();
        }
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter files (T)"));
        self._nav_filter_events = Some(cx.subscribe_in(
            &filter,
            window,
            |view, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => {
                    if let Some(&first) = view.visible_files(cx).first() {
                        view.reveal_file(first, cx);
                    }
                }
                _ => {}
            },
        ));
        self.nav_filter = Some(filter.clone());
        filter
    }

    /// T or /: the cursor into the filter.
    pub(super) fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.nav_filter(window, cx);
        filter.update(cx, |input, cx| input.focus(window, cx));
    }

    /// The files the filter lets through, by index.
    fn visible_files(&self, cx: &gpui_kit::App) -> Vec<usize> {
        let query = self
            .nav_filter
            .as_ref()
            .map(|f| f.read(cx).value().trim().to_lowercase())
            .unwrap_or_default();
        (0..self.files.len())
            .filter(|&i| {
                query.is_empty() || self.files[i].change.path.to_lowercase().contains(&query)
            })
            .collect()
    }

    pub(super) fn navigator(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let filter = self.nav_filter(window, cx);
        let p = self.palette.clone();
        // Swift's `model.current`: the file the diff is scrolled to.
        let current = self.top_file();
        let viewed = self.files.iter().filter(|f| f.viewed).count();
        let fraction = if self.files.is_empty() {
            0.
        } else {
            viewed as f32 / self.files.len() as f32
        };
        let mut list = div()
            .id("review-navigator")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.nav_scroll)
            .pb(px(12.));
        let mut last_dir: Option<String> = None;
        // Its rows are the list's children, folders among them.
        let mut child = 0;
        for index in self.visible_files(cx) {
            let file = &self.files[index];
            let path = std::path::Path::new(&file.change.path);
            let dir = path
                .parent()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            if last_dir.as_deref() != Some(dir.as_str()) {
                if !dir.is_empty() {
                    list = list.child(
                        div()
                            .flex_none()
                            .px(px(12.))
                            .pt(px(6.))
                            .pb(px(2.))
                            .text_size(px(10.5))
                            .text_color(hsla(p.text_tertiary))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis_start()
                            .child(dir.clone()),
                    );
                    child += 1;
                }
                last_dir = Some(dir);
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let selected = current == Some(index);
            if selected && self.nav_top != current {
                self.nav_scroll.scroll_to_item(child);
            }
            child += 1;
            let is_viewed = file.viewed;
            let lines = file
                .diff
                .as_ref()
                .filter(|d| !d.is_binary)
                .map(|d| (d.added, d.removed));
            list = list.child(
                div()
                    .id(("nav", index))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .h(px(24.))
                    .pl(px(8.))
                    .pr(px(6.))
                    .mx(px(6.))
                    .rounded(px(5.))
                    .when(selected, |d| d.bg(hsla(p.accent_soft)))
                    .when(!selected, |d| d.hover(|s| s.bg(hsla(p.hover))))
                    .on_click(cx.listener(move |view, _, _, cx| view.reveal_file(index, cx)))
                    .child(
                        viewed_checkbox(is_viewed, &p)
                            .id(("nav-viewed", index))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                cx.stop_propagation();
                                view.set_viewed(index, !is_viewed, cx);
                            })),
                    )
                    .child(status_letter(file.change.status, &p))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis_middle()
                            .text_size(px(12.))
                            .text_color(hsla(if is_viewed { p.text_tertiary } else { p.text }))
                            .child(name),
                    )
                    .when(file.changed_since_viewed, |d| {
                        d.child(div().size(px(6.)).rounded_full().bg(hsla(p.warning)))
                    })
                    .children(self.navigator_count(index))
                    .children(lines.map(|(added, removed)| stat(added, removed, &p))),
            );
        }
        if current.is_some() {
            self.nav_top = current;
        }
        div()
            .flex()
            .flex_col()
            .flex_none()
            // Swift's 250 navigator and its 1 pt divider beside it.
            .w(px(NAVIGATOR_WIDTH + 1.))
            .h_full()
            .bg(hsla(p.panel))
            .border_r_1()
            .border_color(hsla(p.hairline))
            .child(
                div()
                    .p(px(8.))
                    .child(crate::workbench_chrome::chrome_text_field(
                        &filter, "search", &p, window, cx,
                    )),
            )
            .child(
                div()
                    .mx(px(10.))
                    .mb(px(6.))
                    .h(px(3.))
                    .rounded_full()
                    .bg(hsla(p.hairline))
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .w(gpui_kit::relative(fraction))
                            .bg(hsla(p.git_added)),
                    ),
            )
            .child(list)
            .font_weight(FontWeight::NORMAL)
    }
}
