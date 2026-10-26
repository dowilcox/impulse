//! Impulse: inlay hints, as Monaco's `inlayHints` draws them: labels a host
//! puts between a line's characters (a parameter's name, a variable's
//! type), drawn in the line in their own colour without being part of the
//! text. The text after one moves right; the caret, selections and clicks
//! work in the text's offsets, so the caret steps over a hint and a click on
//! one lands at its offset. The element lays them out
//! (`element.rs`, `display_map/inline_line.rs`), and soft wrapping counts
//! their widths through the wrap map's inline metrics.

use std::ops::Range;

use gpui::{Context, Font, Hsla, Pixels, ShapedLine, SharedString, Window, px};

use super::{EditorMode, InputBaseState};

/// One hint: its label and where it goes, a byte offset in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlayHint {
    offset: usize,
    label: SharedString,
    padding_left: bool,
    padding_right: bool,
}

impl InlayHint {
    /// A hint drawn before the character at `offset` (after the line's
    /// last one when `offset` is its end).
    pub fn new(offset: usize, label: impl Into<SharedString>) -> Self {
        Self {
            offset,
            label: label.into(),
            padding_left: false,
            padding_right: false,
        }
    }

    /// Room before the label (LSP's `paddingLeft`).
    pub fn padding_left(mut self, padding: bool) -> Self {
        self.padding_left = padding;
        self
    }

    /// Room after the label (LSP's `paddingRight`).
    pub fn padding_right(mut self, padding: bool) -> Self {
        self.padding_right = padding;
        self
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn label(&self) -> &SharedString {
        &self.label
    }
}

/// A hint as the editor keeps it.
#[derive(Clone, Debug)]
pub(crate) struct PlacedHint {
    pub(crate) hint: InlayHint,
    /// It follows the word before it (a type after a name), so text typed
    /// at its offset goes before it; otherwise it stays before the text
    /// after it (a parameter's name before an argument), as Monaco anchors
    /// a hint to the word it touches.
    after_word: bool,
    /// Its label shaped, the room before it and its whole width, for the
    /// element's font (`InlayHints::shape`).
    pub(crate) shaped: Option<(ShapedLine, Pixels, Pixels)>,
}

/// The editor's hints, sorted by offset.
#[derive(Debug, Default)]
pub(crate) struct InlayHints {
    pub(crate) hints: Vec<PlacedHint>,
    pub(crate) hidden: bool,
    /// Bumped whenever the hints, their offsets or their widths change.
    pub(crate) revision: u64,
    /// The revision the wrap map's inline metrics were last given.
    pub(crate) synced: Option<u64>,
    /// What the labels were shaped with.
    shaped_with: Option<(Font, Pixels, Hsla)>,
}

impl InlayHints {
    /// The hints to draw: none while hidden.
    pub(crate) fn shown(&self) -> &[PlacedHint] {
        if self.hidden { &[] } else { &self.hints }
    }

    /// The shown hints with offsets in `range` (both ends included).
    pub(crate) fn in_range(&self, range: Range<usize>) -> &[PlacedHint] {
        let hints = self.shown();
        let first = hints.partition_point(|h| h.hint.offset < range.start);
        let last = hints.partition_point(|h| h.hint.offset <= range.end);
        &hints[first..last.max(first)]
    }

    /// Move the hints with an edit replacing `range` with `new_len` bytes. A
    /// hint inside the replaced text goes.
    pub(crate) fn adjust_for_edit(&mut self, range: &Range<usize>, new_len: usize) {
        if self.hints.is_empty() {
            return;
        }
        let delta = new_len as isize - range.len() as isize;
        let before = self.hints.len();
        let mut moved = false;
        self.hints.retain_mut(|placed| {
            let offset = placed.hint.offset;
            if offset > range.start && offset < range.end {
                return false;
            }
            let shifts = offset > range.end
                || (offset == range.end && (range.start < range.end || placed.after_word));
            if shifts && delta != 0 {
                placed.hint.offset = offset.saturating_add_signed(delta);
                moved = true;
            }
            true
        });
        if moved || self.hints.len() != before {
            self.revision += 1;
        }
    }

    /// Shape every label not shaped yet for `font` at `font_size` in
    /// `color`; a change of any of them shapes all again.
    pub(crate) fn shape(&mut self, font: &Font, font_size: Pixels, color: Hsla, window: &Window) {
        let key = (font.clone(), font_size, color);
        if self.shaped_with.as_ref() != Some(&key) {
            self.shaped_with = Some(key);
            for placed in &mut self.hints {
                placed.shaped = None;
            }
            self.revision += 1;
        }
        // Monaco's room beside a label: a third of the font size.
        let padding = px((f32::from(font_size) / 3.).floor());
        for placed in self.hints.iter_mut().filter(|p| p.shaped.is_none()) {
            let label = placed.hint.label.clone();
            let run = gpui::TextRun {
                len: label.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window
                .text_system()
                .shape_line(label, font_size, &[run], None);
            let left = if placed.hint.padding_left {
                padding
            } else {
                px(0.)
            };
            let right = if placed.hint.padding_right {
                padding
            } else {
                px(0.)
            };
            let width = left + shaped.width + right;
            placed.shaped = Some((shaped, left, width));
        }
    }

    /// The shown hints' widths at their offsets, as the wrap map's inline
    /// metrics take them (empty ranges).
    pub(crate) fn metrics(&self) -> std::rc::Rc<[(Range<usize>, Pixels)]> {
        self.shown()
            .iter()
            .filter_map(|p| {
                let (_, _, width) = p.shaped.as_ref()?;
                Some((p.hint.offset..p.hint.offset, *width))
            })
            .collect()
    }
}

/// Whether the character before `offset` in `text` is a word's.
fn follows_word(text: &ropey::Rope, offset: usize) -> bool {
    use crate::input::RopeExt as _;
    if offset == 0 {
        return false;
    }
    let start = text.clip_offset(offset.saturating_sub(4), sum_tree::Bias::Left);
    text.slice(start..offset)
        .to_string()
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

impl InputBaseState<EditorMode> {
    /// Impulse: draw these inlay hints, in place of those drawn before.
    /// Offsets past the end or inside a character are dropped; labels are
    /// one line (line breaks become spaces).
    pub fn set_inlay_hints(&mut self, hints: Vec<InlayHint>, cx: &mut Context<Self>) {
        use crate::input::RopeExt as _;
        let len = self.text.len();
        let mut placed: Vec<PlacedHint> = hints
            .into_iter()
            .filter(|hint| {
                hint.offset <= len
                    && self.text.clip_offset(hint.offset, sum_tree::Bias::Left) == hint.offset
                    && !hint.label.is_empty()
            })
            .map(|mut hint| {
                if hint.label.contains(['\n', '\r']) {
                    hint.label = hint.label.replace(['\n', '\r'], " ").into();
                }
                PlacedHint {
                    after_word: follows_word(&self.text, hint.offset),
                    hint,
                    shaped: None,
                }
            })
            .collect();
        placed.sort_by_key(|p| p.hint.offset);
        // Labels already shaped are kept.
        for p in &mut placed {
            let same = |old: &&PlacedHint| {
                old.hint.label == p.hint.label
                    && old.hint.padding_left == p.hint.padding_left
                    && old.hint.padding_right == p.hint.padding_right
            };
            if let Some(old) = self.inlay_hints.hints.iter().find(same) {
                p.shaped = old.shaped.clone();
            }
        }
        self.inlay_hints.hints = placed;
        self.inlay_hints.revision += 1;
        cx.notify();
    }

    /// Impulse: the inlay hints, by offset, as they now are (they move with
    /// edits).
    pub fn inlay_hints(&self) -> Vec<InlayHint> {
        self.inlay_hints
            .hints
            .iter()
            .map(|p| p.hint.clone())
            .collect()
    }

    /// Impulse: show or hide the inlay hints, keeping them (Monaco's
    /// `onUnlessPressed` and `offUnlessPressed`).
    pub fn set_inlay_hints_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.inlay_hints.hidden == !visible {
            return;
        }
        self.inlay_hints.hidden = !visible;
        self.inlay_hints.revision += 1;
        cx.notify();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{InlayHint, InlayHints, PlacedHint};

    fn hints(at: &[(usize, bool)]) -> InlayHints {
        InlayHints {
            hints: at
                .iter()
                .map(|&(offset, after_word)| PlacedHint {
                    hint: InlayHint::new(offset, "x"),
                    after_word,
                    shaped: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn offsets(hints: &InlayHints) -> Vec<usize> {
        hints.hints.iter().map(|p| p.hint.offset).collect()
    }

    #[test]
    fn hints_move_with_edits() {
        // A type after a name moves on with text typed at it; a parameter's
        // name before an argument stays before what's typed there.
        let mut h = hints(&[(5, true), (10, false), (20, false)]);
        h.adjust_for_edit(&(5..5), 2);
        h.adjust_for_edit(&(12..12), 1);
        assert_eq!(offsets(&h), vec![7, 12, 23]);
        // Text before a hint deleted up to it pulls it back; a hint inside
        // replaced text goes.
        h.adjust_for_edit(&(10..12), 0);
        assert_eq!(offsets(&h), vec![7, 10, 21]);
        h.adjust_for_edit(&(6..15), 3);
        assert_eq!(offsets(&h), vec![15]);
        let revision = h.revision;
        h.adjust_for_edit(&(0..0), 0);
        assert_eq!(h.revision, revision);
    }

    struct Root(gpui::Entity<crate::input::EditorState>);

    impl gpui::Render for Root {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{ParentElement as _, Styled as _};
            gpui::div().size_full().child(self.0.clone())
        }
    }

    /// An editor holding `text`, in a window, laid out once.
    pub(crate) fn editor(
        text: &str,
        cx: &mut gpui::TestAppContext,
    ) -> (
        gpui::Entity<crate::input::EditorState>,
        gpui::VisualTestContext,
    ) {
        use gpui::AppContext as _;
        let text = text.to_string();
        let mut state = None;
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                cx.set_global(crate::theme::Theme::default());
                crate::input::init(cx);
                let editor =
                    cx.new(|cx| crate::input::EditorState::new(window, cx).default_value(text));
                state = Some(editor.clone());
                cx.new(|_| Root(editor))
            })
            .unwrap()
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (state.unwrap(), cx)
    }

    fn draw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    #[gpui::test]
    fn hints_push_the_text_after_them_and_take_clicks(cx: &mut gpui::TestAppContext) {
        use crate::input::{MoveRight, Position};
        use gpui::{point, px};

        let (state, mut cx) = editor("let x = 1;\nfoo(bar);", cx);
        let left = |cx: &mut gpui::VisualTestContext, offset: usize| {
            cx.update(|_, cx| {
                state
                    .read(cx)
                    .range_to_bounds(&(offset..offset))
                    .unwrap()
                    .left()
            })
        };
        let (four, five, six, bar) = (
            left(&mut cx, 4),
            left(&mut cx, 5),
            left(&mut cx, 6),
            left(&mut cx, 15),
        );
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.set_inlay_hints(
                    vec![
                        InlayHint::new(5, ": number"),
                        InlayHint::new(15, "x:").padding_right(true),
                    ],
                    cx,
                );
            })
        });
        draw(&mut cx);
        let (width, second) = cx.update(|_, cx| {
            let hints = &state.read(cx).inlay_hints.hints;
            (
                hints[0].shaped.as_ref().unwrap().2,
                hints[1].shaped.as_ref().unwrap().2,
            )
        });
        assert!(width > px(0.));
        // The text before a hint stays; the caret at its offset and the text
        // after it move right by its width.
        assert_eq!(left(&mut cx, 4), four);
        assert_eq!(left(&mut cx, 5), five + width);
        assert_eq!(left(&mut cx, 6), six + width);
        // The room after a label counts too.
        assert!(
            second
                > cx.update(|_, cx| state.read(cx).inlay_hints.hints[1]
                    .shaped
                    .as_ref()
                    .unwrap()
                    .0
                    .width)
        );
        assert_eq!(left(&mut cx, 15), bar + second);

        // A click on a hint lands at its offset.
        let line = cx.update(|_, cx| state.read(cx).range_to_bounds(&(0..0)).unwrap());
        cx.update(|_, cx| {
            let state = state.read(cx);
            let middle = point(five + width / 2., line.center().y);
            assert_eq!(state.index_for_mouse_position(middle).0, 5);
            let after = point(five + width + (six - five) / 2. + px(1.), line.center().y);
            assert_eq!(state.index_for_mouse_position(after).0, 6);
        });

        // The caret steps over a hint: it's not text.
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_cursor_position(Position::new(0, 4), window, cx);
                state.right(&MoveRight, window, cx);
                assert_eq!(state.cursor(), 5);
                state.right(&MoveRight, window, cx);
                assert_eq!(state.cursor(), 6);
                // Typed after the name, the type hint moves on with it.
                state.set_cursor_position(Position::new(0, 5), window, cx);
                state.replace_text_in_range_silent(None, "y", window, cx);
                assert_eq!(state.inlay_hints()[0].offset(), 6);
                assert_eq!(state.text().to_string(), "let xy = 1;\nfoo(bar);");
            })
        });
        draw(&mut cx);
        // A selection over a hint covers it.
        cx.update(|_, cx| {
            let state = state.read(cx);
            let bounds = state.range_to_bounds(&(4..7)).unwrap();
            assert!(bounds.size.width > width);
        });

        // Hidden, the text goes back.
        cx.update(|_, cx| state.update(cx, |state, cx| state.set_inlay_hints_visible(false, cx)));
        draw(&mut cx);
        assert_eq!(left(&mut cx, 4), four);
        assert_eq!(left(&mut cx, 16), bar);
        assert_eq!(cx.update(|_, cx| state.read(cx).inlay_hints().len()), 2);
    }

    #[gpui::test]
    fn wrapping_counts_hints(cx: &mut gpui::TestAppContext) {
        use gpui::px;

        let text = "word ".repeat(60);
        let (state, mut cx) = editor(&text, cx);
        cx.update(|window, cx| state.update(cx, |state, cx| state.set_soft_wrap(true, window, cx)));
        draw(&mut cx);
        let rows = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| state.read(cx).display_map.wrap_row_count())
        };
        let before = rows(&mut cx);
        assert!(before > 1);
        // Long hints after every word make the line wrap more.
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                let hints = (0..60)
                    .map(|ix| InlayHint::new(ix * 5 + 4, ": a_long_type_name"))
                    .collect();
                state.set_inlay_hints(hints, cx);
            })
        });
        draw(&mut cx);
        draw(&mut cx);
        assert!(rows(&mut cx) > before);
        // Every row still fits.
        cx.update(|_, cx| {
            let state = state.read(cx);
            let layout = state.last_layout.as_ref().unwrap();
            let wrap = layout.wrap_width.unwrap();
            for line in layout.lines.iter() {
                for row in &line.wrapped_lines {
                    assert!(row.width <= wrap + px(1.), "{:?} > {:?}", row.width, wrap);
                }
            }
        });
    }

    #[test]
    fn hints_in_a_range_include_both_ends() {
        let h = hints(&[(0, false), (4, false), (8, false), (12, false)]);
        let found: Vec<usize> = h.in_range(4..8).iter().map(|p| p.hint.offset).collect();
        assert_eq!(found, vec![4, 8]);
        let mut hidden = h;
        hidden.hidden = true;
        assert!(hidden.in_range(0..12).is_empty());
    }
}
