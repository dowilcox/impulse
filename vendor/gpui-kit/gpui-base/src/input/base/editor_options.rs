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

    /// Impulse: how long the pointer rests on a symbol before its hover is
    /// asked for. GPUI Kit's: 150 ms.
    pub fn set_hover_delay(&mut self, delay: Duration) {
        self.extras.lsp.hover_delay = delay;
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
