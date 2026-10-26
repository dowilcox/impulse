//! Impulse: editor options a host sets on the state, named as Monaco names
//! them, so an editor can match Monaco's. Every one defaults to GPUI Kit's
//! own behavior.

use gpui::{Context, Hsla};
use instant::Duration;
#[cfg(not(target_family = "wasm"))]
use std::time::Instant;
#[cfg(target_family = "wasm")]
use web_time::Instant;

use super::{EditorMode, InputBaseState, InputModeKind, RopeExt as _};

/// The caret's shape (Monaco's `cursorStyle`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaretStyle {
    /// A 2px bar before the character.
    Line,
    /// A block over the character, which shows through in
    /// [`EditorColors::block_caret_text`].
    Block,
    /// A 2px bar under the character.
    Underline,
    /// A 1px bar before the character: GPUI Kit's caret.
    #[default]
    LineThin,
    /// A 1px frame around the character.
    BlockOutline,
    /// A 1px bar under the character.
    UnderlineThin,
}

/// How the caret blinks (Monaco's `cursorBlinking`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorBlinking {
    /// On and off every 500 ms: GPUI Kit's caret.
    #[default]
    Blink,
    /// Fades out and back in.
    Smooth,
    /// Fades out and back in, taking longer to fade.
    Phase,
    /// Shrinks to its middle and grows back.
    Expand,
    /// Never blinks.
    Solid,
}

/// Which whitespace gets a marker (Monaco's `renderWhitespace`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderWhitespace {
    /// No markers: GPUI Kit's default.
    #[default]
    None,
    /// Every space and tab.
    All,
    /// All but single spaces between words.
    Boundary,
    /// Only whitespace inside a selection.
    Selection,
    /// Only whitespace after a line's last other character.
    Trailing,
}

/// When typing an opening bracket inserts its closer (Monaco's
/// `autoClosingBrackets`). Quotes always follow the language's rule, as
/// Monaco's `autoClosingQuotes` does by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoCloseBrackets {
    /// Before whitespace, the end of the line, or a character the language's
    /// `auto_close_before` lists: GPUI Kit's rule.
    #[default]
    LanguageDefined,
    /// Only before whitespace or the end of the line.
    BeforeWhitespace,
    /// Before anything.
    Always,
}

/// When the gutter's fold chevrons show (Monaco's `showFoldingControls`).
/// A folded range's chevron always shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FoldingControls {
    /// While the pointer is over the gutter, and on the cursor's line: GPUI
    /// Kit's default.
    #[default]
    MouseoverAndCursorLine,
    /// Only while the pointer is over the gutter.
    Mouseover,
    /// Always.
    Always,
}

/// Colours a host sets beside the projected [`super::InputEditorStyle`]
/// (whose literal in `gpui-component` would have to name every new field).
/// `None` keeps GPUI Kit's colour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditorColors {
    /// Line numbers (Monaco's `editorLineNumber.foreground`). GPUI Kit's:
    /// the style's `muted_foreground`.
    pub line_number: Option<Hsla>,
    /// The cursor line's number (`editorLineNumber.activeForeground`). GPUI
    /// Kit's: the style's `foreground`.
    pub active_line_number: Option<Hsla>,
    /// Selections while the window is inactive
    /// (`editor.inactiveSelectionBackground`). GPUI Kit paints none then, and
    /// hides search matches too; with a colour both stay.
    pub inactive_selection: Option<Hsla>,
    /// The character under a [`CaretStyle::Block`] caret
    /// (`editorCursor.background`). Default: the style's `background`.
    pub block_caret_text: Option<Hsla>,
    /// Inlay hints' labels (`editorInlayHint.foreground`). Default: the
    /// style's `muted_foreground`.
    pub inlay_hint: Option<Hsla>,
    /// Behind inlay hints' labels (`editorInlayHint.background`). Default:
    /// none.
    pub inlay_hint_background: Option<Hsla>,
    /// A snippet's placeholders (`editor.snippetTabstopHighlightBackground`).
    /// Default: Monaco's dark one, a grey at 30%.
    pub snippet_tabstop: Option<Hsla>,
}

/// The options above, as the state holds them.
#[derive(Clone, Debug, Default)]
pub(crate) struct EditorOptions {
    pub(crate) caret_style: CaretStyle,
    pub(crate) render_whitespace: RenderWhitespace,
    pub(crate) auto_close_brackets: AutoCloseBrackets,
    pub(crate) folding_controls: FoldingControls,
    pub(crate) colors: EditorColors,
    /// Scroll a trackpad gesture along one axis only (VS Code's
    /// `scrollPredominantAxis`).
    pub(crate) scroll_predominant_axis: bool,
    /// The axis the current gesture is locked to (true for vertical), and
    /// when its last event came.
    pub(crate) gesture_axis: Option<(bool, Instant)>,
    /// Keep the I-beam while ⌥ is held, as Monaco does, instead of GPUI
    /// Kit's crosshair.
    pub(crate) hide_column_select_pointer: bool,
    /// When the scrollbars show; `None`: the theme's `scrollbar_mode`.
    pub(crate) scrollbar_mode: Option<crate::ScrollbarMode>,
    /// Animate scrolls the editor makes itself (Monaco's
    /// `smoothScrolling`); wheels and trackpads stay direct.
    pub(crate) smooth_scrolling: bool,
    /// The animated scroll under way.
    pub(crate) smooth_scroll: Option<SmoothScroll>,
    /// The gutter's room right of the line numbers, in place of GPUI Kit's
    /// margin and fold chevrons (Monaco's `lineDecorationsWidth` with
    /// `folding: false`); `None`: GPUI Kit's.
    pub(crate) line_decorations_width: Option<gpui::Pixels>,
    /// Whole-line backgrounds under the text: buffer lines (end exclusive)
    /// and their colors, painted in order.
    pub(crate) line_backgrounds: Vec<(std::ops::Range<usize>, Hsla)>,
}

/// How long an animated scroll takes (Monaco's `smoothScrollDuration`).
pub(crate) const SMOOTH_SCROLL_DURATION: Duration = Duration::from_millis(125);

/// An animated scroll: from where it started to where it goes, eased out.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SmoothScroll {
    pub(crate) from: gpui::Point<gpui::Pixels>,
    pub(crate) to: gpui::Point<gpui::Pixels>,
    pub(crate) start: Instant,
}

impl SmoothScroll {
    /// The offset at `now`, and whether it has arrived.
    pub(crate) fn at(&self, now: Instant) -> (gpui::Point<gpui::Pixels>, bool) {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed >= SMOOTH_SCROLL_DURATION {
            return (self.to, true);
        }
        let t = elapsed.as_secs_f32() / SMOOTH_SCROLL_DURATION.as_secs_f32();
        // Monaco's ease out cubic.
        let eased = 1. - (1. - t).powi(3);
        (self.from + (self.to - self.from) * eased, false)
    }

    /// The offset now, and whether it has arrived.
    pub(crate) fn now(&self) -> (gpui::Point<gpui::Pixels>, bool) {
        self.at(Instant::now())
    }
}

/// A pause without events that ends a gesture, as `overscroll`'s
/// `GESTURE_GAP`: momentum and some precise devices send no phases.
const GESTURE_GAP: Duration = Duration::from_millis(250);

impl EditorOptions {
    /// `delta` with the axis it doesn't mostly move on zeroed: per event for
    /// a mouse wheel, per gesture for a trackpad (decided on its first
    /// movement, kept through its momentum).
    pub(crate) fn predominant_axis(
        &mut self,
        delta: gpui::Point<gpui::Pixels>,
        precise: bool,
        phase: gpui::TouchPhase,
        now: Instant,
    ) -> gpui::Point<gpui::Pixels> {
        if !self.scroll_predominant_axis {
            return delta;
        }
        let paused = self
            .gesture_axis
            .is_some_and(|(_, last)| now.saturating_duration_since(last) > GESTURE_GAP);
        if !precise || phase == gpui::TouchPhase::Started || paused {
            self.gesture_axis = None;
        }
        let vertical = match self.gesture_axis {
            Some((vertical, _)) => vertical,
            None if delta.x == gpui::px(0.) && delta.y == gpui::px(0.) => return delta,
            None => delta.y.abs() >= delta.x.abs(),
        };
        if precise {
            self.gesture_axis = Some((vertical, now));
        }
        if vertical {
            gpui::point(gpui::px(0.), delta.y)
        } else {
            gpui::point(delta.x, gpui::px(0.))
        }
    }
}

impl<M: InputModeKind> InputBaseState<M> {
    /// Impulse: set the caret's shape.
    pub fn set_caret_style(&mut self, style: CaretStyle, cx: &mut Context<Self>) {
        self.editor_options.caret_style = style;
        cx.notify();
    }

    /// Impulse: set how the caret blinks.
    pub fn set_cursor_blinking(&mut self, blinking: CursorBlinking, cx: &mut Context<Self>) {
        self.blink_cursor
            .update(cx, |cursor, cx| cursor.set_blinking(blinking, cx));
    }

    /// Impulse: set colours the projected style doesn't carry.
    pub fn set_editor_colors(&mut self, colors: EditorColors, cx: &mut Context<Self>) {
        if self.editor_options.colors != colors {
            self.editor_options.colors = colors;
            cx.notify();
        }
    }

    /// Impulse: scroll a trackpad gesture along one axis only.
    pub fn set_scroll_predominant_axis(&mut self, predominant: bool) {
        self.editor_options.scroll_predominant_axis = predominant;
        self.editor_options.gesture_axis = None;
    }

    /// Impulse: keep the I-beam while ⌥ is held, instead of the crosshair
    /// (⌥-drag still selects a column).
    pub fn set_hide_column_select_pointer(&mut self, hide: bool, cx: &mut Context<Self>) {
        self.editor_options.hide_column_select_pointer = hide;
        cx.notify();
    }

    /// Impulse: when the editor's scrollbars show, whatever the theme's
    /// `scrollbar_mode` (Monaco's `scrollbar.vertical: "visible"` is
    /// [`crate::ScrollbarMode::Always`]); `None` follows the theme.
    pub fn set_scrollbar_mode(
        &mut self,
        mode: Option<crate::ScrollbarMode>,
        cx: &mut Context<Self>,
    ) {
        if self.editor_options.scrollbar_mode != mode {
            self.editor_options.scrollbar_mode = mode;
            cx.notify();
        }
    }

    /// Impulse: animate the scrolls the editor makes itself (keys that move
    /// the cursor off screen, revealing a line, [`Self::animate_scroll_offset`])
    /// over 125 ms, as Monaco's `smoothScrolling`. Wheels and trackpads stay
    /// direct.
    pub fn set_smooth_scrolling(&mut self, smooth: bool) {
        self.editor_options.smooth_scrolling = smooth;
        if !smooth {
            self.editor_options.smooth_scroll = None;
        }
    }

    /// Impulse: scroll to `offset` as [`Self::set_scroll_offset`] does,
    /// animated when [`Self::set_smooth_scrolling`] is on.
    pub fn animate_scroll_offset(
        &mut self,
        offset: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.set_scroll_target(offset, cx);
    }

    /// Scroll to `target` (already clamped), through an animation when the
    /// option is on and it moves.
    pub(crate) fn set_scroll_target(
        &mut self,
        target: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let now = Instant::now();
        let current = match self.editor_options.smooth_scroll {
            Some(scroll) => scroll.at(now).0,
            None => self
                .deferred_scroll_offset
                .unwrap_or(self.scroll_handle.offset()),
        };
        if !self.editor_options.smooth_scrolling || cx.reduce_motion() || current == target {
            self.editor_options.smooth_scroll = None;
            self.deferred_scroll_offset = Some(target);
        } else if self
            .editor_options
            .smooth_scroll
            .is_none_or(|scroll| scroll.to != target)
        {
            self.editor_options.smooth_scroll = Some(SmoothScroll {
                from: current,
                to: target,
                start: now,
            });
            self.deferred_scroll_offset = Some(current);
        }
        cx.notify();
    }

    /// Impulse: which whitespace gets a marker. [`Self::set_show_whitespaces`]
    /// sets `All` or `None`.
    pub fn set_render_whitespace(&mut self, mode: RenderWhitespace, cx: &mut Context<Self>) {
        self.editor_options.render_whitespace = mode;
        self.show_whitespaces = mode != RenderWhitespace::None;
        cx.notify();
    }
}

impl InputBaseState<EditorMode> {
    /// Impulse: when typing an opening bracket inserts its closer.
    pub fn set_auto_close_brackets(&mut self, mode: AutoCloseBrackets, cx: &mut Context<Self>) {
        self.editor_options.auto_close_brackets = mode;
        cx.notify();
    }

    /// Impulse: when the gutter's fold chevrons show.
    pub fn set_folding_controls(&mut self, controls: FoldingControls, cx: &mut Context<Self>) {
        self.editor_options.folding_controls = controls;
        cx.notify();
    }

    /// Impulse: the gutter's room right of the line numbers, as Monaco's
    /// `lineDecorationsWidth` with `folding: false` (its diff editor's): the
    /// numbers end `width` before the text, and no fold chevrons show (folds
    /// stay). `None` (the default): GPUI Kit's margin and chevron lane.
    pub fn set_line_decorations_width(
        &mut self,
        width: Option<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.editor_options.line_decorations_width != width {
            self.editor_options.line_decorations_width = width;
            cx.notify();
        }
    }

    /// Impulse: whole-line backgrounds (Monaco's `isWholeLine` decorations
    /// with a background, as its diff editor tints changed lines): buffer
    /// lines (0-based, end exclusive) and their colors, painted in order
    /// under the text (and the selection), from the text column's left edge
    /// across the editor, each line's wrapped rows included and the view
    /// zones under it not. They don't follow edits: the host sets them again.
    pub fn set_line_backgrounds(
        &mut self,
        backgrounds: Vec<(std::ops::Range<usize>, Hsla)>,
        cx: &mut Context<Self>,
    ) {
        if self.editor_options.line_backgrounds != backgrounds {
            self.editor_options.line_backgrounds = backgrounds;
            cx.notify();
        }
    }

    /// Impulse: how long the pointer rests on a symbol before its hover is
    /// asked for. GPUI Kit's: 150 ms.
    pub fn set_hover_delay(&mut self, delay: Duration) {
        self.extras.lsp.hover_delay = delay;
    }

    /// Impulse: Monaco's Home, for every cursor: to its line's first
    /// non-blank character, or to the line's start when it's there already
    /// (or the line is blank); on a soft-wrapped line's later rows, to the
    /// row's start first. With `select`, each selection extends to there
    /// instead (Shift+Home). GPUI Kit's `MoveHome` goes to the start.
    pub fn smart_home(&mut self, select: bool, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if select {
            self.undo_manager.break_transaction_coalescing();
            self.select_all_cursors_to(
                |s, sel| s.smart_home_target(sel.cursor_offset(), s.line_end_affinity_for(sel)),
                cx,
            );
        } else {
            self.move_all_cursors(
                |s, sel| {
                    let offset =
                        s.smart_home_target(sel.cursor_offset(), s.line_end_affinity_for(sel));
                    (offset, s.preferred_column_for(offset), false)
                },
                Some(super::movement::MoveDirection::Up),
                window,
                cx,
            );
        }
    }

    /// Impulse: every selection, as byte ranges (start before end), in the
    /// order they were made; the active one is
    /// [`Self::selected_range`].
    pub fn selected_ranges(&self) -> Vec<std::ops::Range<usize>> {
        self.selections.iter().map(|sel| (*sel).into()).collect()
    }

    /// Impulse: fold the innermost unfolded range holding the cursor's line
    /// (Monaco's Fold). Returns whether one folded.
    pub fn fold_at_cursor(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.mode.is_folding() {
            return false;
        }
        let line = self.text.offset_to_point(self.cursor()).row;
        let innermost = self
            .display_map
            .fold_candidates()
            .iter()
            .filter(|range| range.start_line <= line && line < range.end_line)
            .filter(|range| !self.display_map.is_folded_at(range.start_line))
            .max_by_key(|range| range.start_line)
            .copied();
        let Some(range) = innermost else {
            return false;
        };
        self.display_map.set_folded(range.start_line, true);
        self.reveal_cursor_after_fold(cx);
        cx.notify();
        true
    }

    /// Impulse: unfold the innermost folded range holding the cursor's line
    /// (Monaco's Unfold). Returns whether one opened.
    pub fn unfold_at_cursor(&mut self, cx: &mut Context<Self>) -> bool {
        let line = self.text.offset_to_point(self.cursor()).row;
        let innermost = self
            .display_map
            .folded_ranges()
            .iter()
            .filter(|range| range.start_line <= line && line < range.end_line)
            .max_by_key(|range| range.start_line)
            .copied();
        let Some(range) = innermost else {
            return false;
        };
        self.display_map.set_folded(range.start_line, false);
        cx.notify();
        true
    }

    /// Impulse: fold every range there is (Monaco's Fold All).
    pub fn fold_all(&mut self, cx: &mut Context<Self>) {
        if !self.mode.is_folding() {
            return;
        }
        let starts: Vec<usize> = self
            .display_map
            .fold_candidates()
            .iter()
            .map(|range| range.start_line)
            .collect();
        for start in starts {
            self.display_map.set_folded(start, true);
        }
        self.reveal_cursor_after_fold(cx);
        cx.notify();
    }

    /// Impulse: open every fold (Monaco's Unfold All). The ranges can still
    /// be folded again.
    pub fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.display_map.clear_folds();
        cx.notify();
    }

    /// A fold that hides the cursor's line moves the cursor to the end of the
    /// outermost such fold's first line, as Monaco does.
    fn reveal_cursor_after_fold(&mut self, cx: &mut Context<Self>) {
        let line = self.text.offset_to_point(self.cursor()).row;
        let outermost = self
            .display_map
            .folded_ranges()
            .iter()
            .filter(|range| line > range.start_line && line < range.end_line)
            .map(|range| range.start_line)
            .min();
        if let Some(start) = outermost {
            let offset = self.text.line_end_offset(start);
            self.move_to(offset, None, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use gpui::AppContext as _;
    use gpui::{TouchPhase, point, px};
    use instant::Duration;

    use super::{EditorOptions, Instant};

    #[test]
    fn a_gesture_keeps_the_axis_it_started_on() {
        let mut options = EditorOptions {
            scroll_predominant_axis: true,
            ..Default::default()
        };
        let now = Instant::now();
        let started =
            options.predominant_axis(point(px(1.), px(-8.)), true, TouchPhase::Started, now);
        assert_eq!(started, point(px(0.), px(-8.)));
        // Drift that's mostly sideways doesn't move the text sideways.
        let drift = options.predominant_axis(point(px(5.), px(-2.)), true, TouchPhase::Moved, now);
        assert_eq!(drift, point(px(0.), px(-2.)));
        // Momentum (no phase, reported as moved) stays on the axis too.
        let momentum =
            options.predominant_axis(point(px(3.), px(0.)), true, TouchPhase::Moved, now);
        assert_eq!(momentum, point(px(0.), px(0.)));
        // The next gesture decides again.
        let sideways =
            options.predominant_axis(point(px(6.), px(1.)), true, TouchPhase::Started, now);
        assert_eq!(sideways, point(px(6.), px(0.)));
        // So does a stream without phases, after a pause.
        let later = now + Duration::from_millis(300);
        let down = options.predominant_axis(point(px(1.), px(4.)), true, TouchPhase::Moved, later);
        assert_eq!(down, point(px(0.), px(4.)));
    }

    #[test]
    fn a_wheel_decides_each_event() {
        let mut options = EditorOptions {
            scroll_predominant_axis: true,
            ..Default::default()
        };
        let now = Instant::now();
        let down = options.predominant_axis(point(px(1.), px(3.)), false, TouchPhase::Moved, now);
        assert_eq!(down, point(px(0.), px(3.)));
        let right = options.predominant_axis(point(px(3.), px(0.)), false, TouchPhase::Moved, now);
        assert_eq!(right, point(px(3.), px(0.)));
    }

    #[test]
    fn both_axes_move_without_the_option() {
        let mut options = EditorOptions::default();
        let delta = point(px(2.), px(3.));
        assert_eq!(
            options.predominant_axis(delta, true, TouchPhase::Moved, Instant::now()),
            delta
        );
    }

    #[gpui::test]
    fn folding_from_the_keyboard(cx: &mut gpui::TestAppContext) {
        use crate::input::{EditorState, FoldRange, Position};

        cx.update(crate::init);
        let (state, cx) = cx.add_window_view(|window, cx| EditorState::new(window, cx));
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_value("a\nb\nc\nd\ne\nf\ng\nh", window, cx);
                state.apply_highlighter_fold_candidates(
                    vec![FoldRange::new(0, 6), FoldRange::new(2, 5)],
                    cx,
                );
                state.set_cursor_position(Position::new(3, 0), window, cx);
            });
        });
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                // The innermost range first, then the one around it, which
                // hides the cursor's line and so takes the cursor to its
                // first line's end.
                assert!(state.fold_at_cursor(cx));
                assert!(state.display_map.is_folded_at(2));
                assert!(!state.display_map.is_folded_at(0));
                assert!(state.fold_at_cursor(cx));
                assert!(state.display_map.is_folded_at(0));
                assert_eq!(state.cursor_position(), Position::new(0, 1));
                // Unfold opens the innermost fold at the cursor.
                assert!(state.unfold_at_cursor(cx));
                assert!(!state.display_map.is_folded_at(0));
                assert!(state.display_map.is_folded_at(2));
                state.unfold_all(cx);
                assert!(state.display_map.folded_ranges().is_empty());
                state.fold_all(cx);
                assert_eq!(state.display_map.folded_ranges().len(), 2);
            });
        });
    }

    #[gpui::test]
    fn home_takes_every_cursor_to_its_first_non_blank(cx: &mut gpui::TestAppContext) {
        use crate::input::inlay_hints::tests::editor;

        let (state, mut cx) = editor("  foo\n    bar\nbaz\n   ", cx);
        let cursors = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| {
                let mut ranges = state.read(cx).selected_ranges();
                ranges.sort_by_key(|r| r.start);
                ranges
            })
        };
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.set_selected_range(4..4, cx);
                state.add_selection(12..12, cx);
                state.add_selection(16..16, cx);
                state.add_selection(20..20, cx);
            })
        });
        let home = |cx: &mut gpui::VisualTestContext, select: bool| {
            cx.update(|window, cx| {
                state.update(cx, |state, cx| state.smart_home(select, window, cx))
            })
        };
        home(&mut cx, false);
        // A blank line's cursor goes to its start.
        assert_eq!(cursors(&mut cx), vec![2..2, 10..10, 14..14, 18..18]);
        home(&mut cx, false);
        assert_eq!(cursors(&mut cx), vec![0..0, 6..6, 14..14, 18..18]);
        home(&mut cx, false);
        assert_eq!(cursors(&mut cx), vec![2..2, 10..10, 14..14, 18..18]);

        // Shift+Home selects to there.
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.set_selected_range(4..4, cx);
                state.add_selection(12..12, cx);
            })
        });
        home(&mut cx, true);
        assert_eq!(cursors(&mut cx), vec![2..4, 10..12]);
    }

    #[gpui::test]
    fn scrolls_the_editor_makes_animate(cx: &mut gpui::TestAppContext) {
        use crate::input::{Position, inlay_hints::tests::editor};

        let text = (0..400)
            .map(|ix| ix.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (state, mut cx) = editor(&text, cx);
        let draw =
            |cx: &mut gpui::VisualTestContext| cx.update(|window, cx| window.draw(cx).clear(cx));
        let offset =
            |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| state.read(cx).scroll_offset().y);
        // Off: a jump.
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_cursor_position(Position::new(300, 0), window, cx)
            })
        });
        draw(&mut cx);
        let target = offset(&mut cx);
        assert!(target < px(0.));
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_cursor_position(Position::new(0, 0), window, cx)
            })
        });
        draw(&mut cx);
        assert_eq!(offset(&mut cx), px(0.));

        // On: from where it is to where it goes over 125 ms.
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_smooth_scrolling(true);
                state.set_cursor_position(Position::new(300, 0), window, cx);
                assert!(state.editor_options.smooth_scroll.is_some());
            })
        });
        draw(&mut cx);
        let early = offset(&mut cx);
        assert!(early > target && early <= px(0.), "{early:?}");
        std::thread::sleep(std::time::Duration::from_millis(150));
        draw(&mut cx);
        assert_eq!(offset(&mut cx), target);
        assert!(cx.update(|_, cx| state.read(cx).editor_options.smooth_scroll.is_none()));

        // Setting the offset directly is a jump.
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.animate_scroll_offset(gpui::point(px(0.), px(0.)), cx);
                assert!(state.editor_options.smooth_scroll.is_some());
                state.set_scroll_offset(gpui::point(px(0.), px(-40.)), cx);
                assert!(state.editor_options.smooth_scroll.is_none());
            })
        });
        draw(&mut cx);
        assert_eq!(offset(&mut cx), px(-40.));
    }

    #[gpui::test]
    fn brackets_close_by_the_hosts_rule(cx: &mut gpui::TestAppContext) {
        use super::AutoCloseBrackets;
        use crate::input::{EditorState, Position};
        use gpui::EntityInputHandler as _;

        cx.update(crate::init);
        let (state, cx) = cx.add_window_view(|window, cx| EditorState::new(window, cx));
        let mut typed = |mode, text: &str, open: &str| -> String {
            cx.update(|window, cx| {
                state.update(cx, |state, cx| {
                    state.set_auto_close_brackets(mode, cx);
                    state.set_value(text.to_string(), window, cx);
                    state.set_cursor_position(Position::new(0, 0), window, cx);
                    state.replace_text_in_range(None, open, window, cx);
                    state.text().to_string()
                })
            })
        };
        assert_eq!(typed(AutoCloseBrackets::LanguageDefined, "word", "("), "(word");
        assert_eq!(typed(AutoCloseBrackets::LanguageDefined, ";", "("), "();");
        assert_eq!(typed(AutoCloseBrackets::BeforeWhitespace, ";", "("), "(;");
        assert_eq!(typed(AutoCloseBrackets::BeforeWhitespace, " x", "("), "() x");
        assert_eq!(typed(AutoCloseBrackets::Always, "word", "("), "()word");
        // Quotes keep the language's rule.
        assert_eq!(typed(AutoCloseBrackets::Always, "word", "\""), "\"word");
    }
}
