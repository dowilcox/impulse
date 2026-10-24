//! Diff View (Toggle Diff View; Monaco's diff editor in the Swift app): the
//! file's index version against the buffer, under a bar with the file, the
//! summary, ↑ ↓ through the changes, Inline or Side by Side, and Done.
//! Side by side, the index version sits read-only on the left and the two
//! sides scroll together (`impulse_kit::git::diff_view::map_row`), so a
//! change lines up with what it replaced. Inline, the removed lines sit
//! above the lines that replaced them in one read-only document (GPUI Kit's
//! editor has no view zones to put them into the editable one). Changed
//! lines are tinted on both, following edits.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, Position, TabSize};
use gpui_kit::component::{ActiveTheme as _, RopeExt as _, Sizable as _};
use gpui_kit::{
    canvas, div, fill, point, prelude::*, px, AnyElement, Bounds, Context, Entity, FontWeight,
    Hsla, Pixels, Window,
};
use impulse_kit::git::diff_view;
use impulse_kit::git::editor_marks::{line_hunks, LineHunk};
use impulse_kit::text_file::LineEnding;

use crate::editor_tab::EditorTab;

/// How long typing rests before the Diff View marks again.
pub(crate) const DIFF_DELAY: Duration = Duration::from_millis(200);

/// Rows (0-based, end exclusive) tinted across the editor, and the tint.
type Tints = Vec<(std::ops::Range<u32>, Hsla)>;

/// The Diff View's other editor and its marks.
pub(crate) struct DiffView {
    base: Entity<EditorState>,
    base_text: String,
    base_tints: Tints,
    own_tints: Tints,
    hunks: Vec<LineHunk>,
    /// Inline: the combined document, rebuilt as the buffer changes.
    inline: Option<InlineView>,
    /// The last scroll position the sync set on either side, so a side
    /// that can't scroll that far isn't asked again every frame.
    synced: Rc<Cell<Option<f32>>>,
    _refresh: Option<gpui_kit::Task<()>>,
}

struct InlineView {
    state: Entity<EditorState>,
    tints: Tints,
    rows: diff_view::InlineDiff,
    /// The text to show at the next render (setting it needs the window).
    pending: Option<String>,
}

impl EditorTab {
    /// Toggle Diff View, in the layout it last had. An error message when
    /// the file isn't in a repository.
    pub fn toggle_diff_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.diff.is_some() {
            self.close_diff_view(window, cx);
            return Ok(());
        }
        // The file's git base, as the change marks have it: its index
        // version (what's staged, else HEAD's).
        // A file git doesn't track yet is all new.
        let base_text = impulse_git::base_content(&self.path.to_string_lossy())
            .map(|b| LineEnding::normalize(&b))
            .or_else(|| self.git.root.is_some().then(String::new))
            .ok_or_else(|| "This file isn't in a git repository.".to_string())?;
        if self.preview == Some(false) {
            self.preview = None;
        }
        self.git.peek = None;
        let base = self.read_only_state(base_text.clone(), window, cx);
        self.diff = Some(DiffView {
            base,
            base_text,
            base_tints: Vec::new(),
            own_tints: Vec::new(),
            hunks: Vec::new(),
            inline: None,
            synced: Rc::default(),
            _refresh: None,
        });
        if self.diff_inline {
            self.set_diff_inline(true, window, cx);
        }
        self.refresh_diff(Duration::ZERO, cx);
        cx.notify();
        Ok(())
    }

    /// Done: back to the editor, the cursor where it was.
    fn close_diff_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.diff = None;
        let focus = self.state.clone();
        focus.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// An editor like the buffer's, read-only, holding `text`.
    fn read_only_state(
        &self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorState> {
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let grammar = crate::editor_tab::grammar_for(&self.language).to_string();
        let (tab_size, use_spaces) = (self.tab_size, self.use_spaces);
        cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language(grammar)
                .line_number(settings.show_line_numbers)
                .folding(settings.folding)
                .soft_wrap(settings.word_wrap)
                .tab_size(TabSize {
                    tab_size,
                    hard_tabs: !use_spaces,
                })
                .default_value(text);
            state.set_readonly(true, cx);
            state
        })
    }

    /// Switch between side by side and inline (remembered for the tab).
    fn set_diff_inline(&mut self, inline: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.diff_inline = inline;
        let Some(diff) = self.diff.as_mut() else {
            return;
        };
        if !inline {
            diff.inline = None;
            let focus = self.state.clone();
            focus.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        if diff.inline.is_some() {
            return;
        }
        let text = self.state.read(cx).value().to_string();
        let rows = diff_view::inline(&text, &diff.hunks);
        // The cursor's line, where it now is.
        let cursor = self.state.read(cx).cursor_position().line as usize;
        let at = rows.buffer_rows.iter().position(|r| *r == Some(cursor));
        let shown = rows.text.clone();
        let state = self.read_only_state(shown, window, cx);
        if let Some(diff) = self.diff.as_mut() {
            diff.inline = Some(InlineView {
                state: state.clone(),
                tints: Vec::new(),
                rows,
                pending: None,
            });
        }
        match at {
            Some(row) => center(&state, row, window, cx),
            None => state.update(cx, |state, cx| state.focus(window, cx)),
        }
        self.refresh_diff(Duration::ZERO, cx);
        cx.notify();
    }

    /// Mark the changed lines again, after `delay`.
    pub(crate) fn refresh_diff(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.as_mut() else {
            return;
        };
        let old = diff.base_text.clone();
        let new = self.state.read(cx).value().to_string();
        diff._refresh = Some(cx.spawn(async move |tab, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let (hunks, removed, added, inline) = cx
                .background_executor()
                .spawn(async move {
                    let hunks = line_hunks(&old, &new);
                    let rows = |start: usize, len: usize| {
                        let start = start.saturating_sub(1) as u32;
                        start..start + len as u32
                    };
                    let removed: Vec<_> = hunks
                        .iter()
                        .filter(|h| !h.old_lines.is_empty())
                        .map(|h| rows(h.old_start, h.old_lines.len()))
                        .collect();
                    let added: Vec<_> = hunks
                        .iter()
                        .filter(|h| h.new_count > 0)
                        .map(|h| rows(h.new_start, h.new_count))
                        .collect();
                    let inline = diff_view::inline(&new, &hunks);
                    (hunks, removed, added, inline)
                })
                .await;
            let _ = tab.update(cx, |tab, cx| {
                let Some(diff) = tab.diff.as_mut() else {
                    return;
                };
                let theme = cx.theme();
                let (red, green) = (theme.danger.opacity(0.22), theme.success.opacity(0.18));
                diff.hunks = hunks;
                diff.base_tints = tints(removed, red);
                diff.own_tints = tints(added, green);
                if let Some(view) = diff.inline.as_mut() {
                    if view.rows.text != inline.text {
                        view.pending = Some(inline.text.clone());
                    }
                    let mut all = tints(inline.removed.clone(), red);
                    all.extend(tints(inline.added.clone(), green));
                    view.tints = all;
                    view.rows = inline;
                }
                cx.notify();
            });
        }));
    }

    /// The inline document's new text, once there's a window to set it in.
    pub(crate) fn apply_inline_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.diff.as_mut().and_then(|d| d.inline.as_mut()) else {
            return;
        };
        if let Some(text) = view.pending.take() {
            view.state
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }

    /// ↑ ↓: the previous or next change, wrapping around.
    fn diff_go(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.as_ref() else {
            return;
        };
        if let Some(view) = &diff.inline {
            let row = view.state.read(cx).cursor_position().line as usize;
            let mut starts: Vec<usize> = view
                .rows
                .removed
                .iter()
                .chain(&view.rows.added)
                .map(|r| r.start as usize)
                .collect();
            starts.sort_unstable();
            starts.dedup();
            // A change is its removed lines and the added ones under them.
            starts.retain(|&s| {
                !view.rows.added.iter().any(|a| a.start as usize == s)
                    || !view.rows.removed.iter().any(|r| r.end as usize == s)
            });
            let target = if forward {
                starts.iter().find(|&&s| s > row).or(starts.first())
            } else {
                starts.iter().rev().find(|&&s| s < row).or(starts.last())
            };
            if let Some(&target) = target {
                center(&view.state.clone(), target, window, cx);
            }
            return;
        }
        let row = self.state.read(cx).cursor_position().line as usize;
        if let Some(target) = diff_view::next_change(&diff.hunks, row, forward) {
            center(&self.state.clone(), target, window, cx);
        }
    }

    /// The bar and the two sides (or the inline document).
    pub(crate) fn diff_element(&self, editor: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let Some(diff) = self.diff.as_ref() else {
            return editor;
        };
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let styled = |state: &Entity<EditorState>| {
            let mut editor = Editor::new(state)
                .size_full()
                .font_family(settings.font_family.clone())
                .text_size(px(settings.font_size as f32))
                .bordered(false);
            if settings.editor_line_height > 0 {
                editor = editor.line_height(px(settings.editor_line_height as f32));
            }
            editor.font_features(if settings.font_ligatures {
                gpui_kit::FontFeatures::default()
            } else {
                gpui_kit::FontFeatures::disable_ligatures()
            })
        };
        let border = cx.theme().border;
        let background = cx
            .try_global::<crate::kit_theme::EditorBackdrop>()
            .map(|b| b.background)
            .unwrap_or(cx.theme().background);
        let body = match &diff.inline {
            Some(view) => div()
                .flex_1()
                .min_h_0()
                .child(tinted(
                    styled(&view.state),
                    &view.state,
                    &view.tints,
                    background,
                ))
                .into_any_element(),
            None => {
                let sync = scroll_sync(
                    self.state.clone(),
                    diff.base.clone(),
                    diff.hunks.clone(),
                    diff.synced.clone(),
                );
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .flex()
                    .child(div().flex_1().min_w_0().h_full().child(tinted(
                        styled(&diff.base),
                        &diff.base,
                        &diff.base_tints,
                        background,
                    )))
                    .child(div().w(px(1.)).h_full().bg(border))
                    .child(div().flex_1().min_w_0().h_full().child(tinted(
                        editor,
                        &self.state,
                        &diff.own_tints,
                        background,
                    )))
                    .child(sync)
                    .into_any_element()
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.diff_bar(&diff.hunks, diff.inline.is_some(), cx))
            .child(body)
            .into_any_element()
    }

    fn diff_bar(&self, hunks: &[LineHunk], inline: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let button = |id: &'static str, label: &'static str| Button::new(id).label(label).xsmall();
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.title.clone());
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(4.))
            .text_size(px(12.))
            .bg(theme.secondary)
            .border_b_1()
            .border_color(theme.border)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("Index \u{2194} Working copy"),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .text_color(theme.foreground.opacity(0.8))
                    .child(diff_view::summary(hunks)),
            )
            .child(
                button("diff-previous", "↑")
                    .ghost()
                    .tooltip("Previous change")
                    .on_click(cx.listener(|tab, _, window, cx| tab.diff_go(false, window, cx))),
            )
            .child(
                button("diff-next", "↓")
                    .ghost()
                    .tooltip("Next change")
                    .on_click(cx.listener(|tab, _, window, cx| tab.diff_go(true, window, cx))),
            )
            .child(
                button(
                    "diff-layout",
                    if inline { "Side by Side" } else { "Inline" },
                )
                .tooltip("Switch between side by side and inline")
                .on_click(
                    cx.listener(move |tab, _, window, cx| tab.set_diff_inline(!inline, window, cx)),
                ),
            )
            .child(
                button("diff-done", "Done")
                    .tooltip("Back to the editor")
                    .on_click(cx.listener(|tab, _, window, cx| tab.close_diff_view(window, cx))),
            )
            .into_any_element()
    }
}

/// The cursor to the start of `row`, scrolled to the middle of the view
/// (Monaco's `revealPositionInCenter`).
pub(crate) fn center(
    state: &Entity<EditorState>,
    row: usize,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    state.update(cx, |state, cx| {
        state.set_cursor_position(Position::new(row as u32, 0), window, cx);
        let (Some(rows), Some(lh)) = (state.visible_row_range(), state.line_height()) else {
            return;
        };
        let top = row.saturating_sub(rows.len() / 2);
        let x = state.scroll_offset().x;
        state.set_scroll_offset(point(x, -lh * top as f32), cx);
    });
}

fn tints(rows: Vec<std::ops::Range<u32>>, color: Hsla) -> Tints {
    rows.into_iter()
        .filter(|r| !r.is_empty())
        .map(|r| (r, color))
        .collect()
}

/// `editor` with `tints` painted over its rows across its whole width, as
/// Monaco's whole-line decorations are (a range decoration stops at the
/// end of the text).
fn tinted(
    editor: impl IntoElement,
    state: &Entity<EditorState>,
    tints: &Tints,
    background: Hsla,
) -> AnyElement {
    let (state, tints) = (state.clone(), tints.clone());
    div()
        .relative()
        .size_full()
        // The theme leaves code editors' background to us.
        .bg(background)
        .child(editor)
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let state = state.read(cx);
                    let Some(visible) = state.visible_row_range() else {
                        return;
                    };
                    let text = state.text();
                    let at = |row: usize| {
                        (row < text.lines_len())
                            .then(|| text.line_start_offset(row))
                            .and_then(|o| state.range_to_bounds(&(o..o)))
                    };
                    for (rows, color) in &tints {
                        let first = (rows.start as usize).max(visible.start);
                        let last = (rows.end as usize).min(visible.end);
                        if first >= last {
                            continue;
                        }
                        if let (Some(top), Some(bottom)) = (at(first), at(last - 1)) {
                            window.paint_quad(fill(
                                Bounds::from_corners(
                                    point(top.origin.x - px(4.), top.top()),
                                    point(bounds.right(), bottom.bottom()),
                                ),
                                *color,
                            ));
                        }
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .into_any_element()
}

/// Keeps the two sides at matching lines: the side under the pointer leads
/// (the buffer when the pointer is elsewhere, as when typing).
fn scroll_sync(
    own: Entity<EditorState>,
    base: Entity<EditorState>,
    hunks: Vec<LineHunk>,
    synced: Rc<Cell<Option<f32>>>,
) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let (Some(own_lh), Some(base_lh)) =
                (own.read(cx).line_height(), base.read(cx).line_height())
            else {
                return;
            };
            let top = |state: &Entity<EditorState>, lh: Pixels| {
                let y = -f32::from(state.read(cx).scroll_offset().y);
                (y / f32::from(lh)).max(0.)
            };
            let mouse = window.mouse_position();
            let base_leads =
                bounds.contains(&mouse) && mouse.x < bounds.left() + bounds.size.width / 2.;
            let (leader, follower, from_new, follower_lh) = if base_leads {
                (top(&base, base_lh), &own, false, own_lh)
            } else {
                (top(&own, own_lh), &base, true, base_lh)
            };
            let row = leader.floor();
            let mapped = diff_view::map_row(&hunks, row as usize, from_new) as f32;
            let target = (mapped + leader - row) * f32::from(follower_lh);
            let current = -f32::from(follower.read(cx).scroll_offset().y);
            if (target - current).abs() < 0.5 || synced.get() == Some(target) {
                return;
            }
            synced.set(Some(target));
            follower.update(cx, |state, cx| {
                let x = state.scroll_offset().x;
                state.set_scroll_offset(point(x, px(-target)), cx);
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}
