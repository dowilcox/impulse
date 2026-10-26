//! Monaco's editor commands that GPUI Kit's editor doesn't have, on top of
//! `impulse_kit::line_edits`: Toggle Block Comment, Go to Bracket, Next
//! and Previous Problem, Expand and Shrink Selection, parameter hints,
//! Cursor Undo, cursors at line ends, Trim Trailing Whitespace, Home to the
//! first non-blank character, Expand Line Selection, and folding from the
//! keyboard (their keys are in `editor_keys`).

use std::ops::Range;

use gpui_kit::component::input::MoveHome;
use gpui_kit::{Context, EntityInputHandler as _, Focusable as _, Window};
use impulse_kit::{line_edits, linked_tags};

use crate::editor_keys::{
    CursorHome, CursorUndo, CursorsAtLineEnds, ExpandLineSelection, ExpandSelection, Fold, FoldAll,
    GoToBracket, NextProblem, PreviousProblem, ShowParameterHints, ShrinkSelection,
    ToggleBlockComment, TrimTrailingWhitespace, Unfold, UnfoldAll,
};
use crate::editor_tab::EditorTab;

/// How many earlier selections Cursor Undo keeps.
const CURSOR_HISTORY: usize = 64;

/// The selections Cursor Undo and Shrink Selection go back through.
#[derive(Default)]
pub(crate) struct SelectionHistory {
    /// Earlier selections, oldest first.
    past: Vec<Range<usize>>,
    /// Set while Cursor Undo moves the selection (that isn't a new step).
    restoring: bool,
    /// Expand Selection's steps: where each started, and where it got to.
    expanded: Vec<Range<usize>>,
    expanded_to: Option<Range<usize>>,
}

impl SelectionHistory {
    /// The selection moved from `previous`.
    pub(crate) fn moved(&mut self, previous: Range<usize>) {
        if std::mem::take(&mut self.restoring) {
            return;
        }
        if self.past.last() != Some(&previous) {
            self.past.push(previous);
            if self.past.len() > CURSOR_HISTORY {
                self.past.remove(0);
            }
        }
    }
}

/// Past this size, tags aren't linked (finding pairs reads the text).
const MAX_LINKED_TEXT: usize = 1024 * 1024;

/// The tag pair the cursor is in, as the text was when it was found.
#[derive(Default)]
pub(crate) struct LinkedTags {
    pair: Option<(Range<usize>, Range<usize>, String)>,
    /// The text's length when `pair` was found (an edit since isn't
    /// followed by a cursor move's search).
    len: usize,
    /// The partner's edit is under way.
    applying: bool,
}

impl EditorTab {
    /// Monaco's Fold: the innermost open range at the cursor.
    pub(crate) fn fold(&mut self, _: &Fold, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.fold_at_cursor(cx);
        });
    }

    /// Monaco's Unfold: the innermost folded range at the cursor.
    pub(crate) fn unfold(&mut self, _: &Unfold, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.unfold_at_cursor(cx);
        });
    }

    pub(crate) fn fold_all(&mut self, _: &FoldAll, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.fold_all(cx));
    }

    pub(crate) fn unfold_all(&mut self, _: &UnfoldAll, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.unfold_all(cx));
    }

    fn links_tags(&self) -> bool {
        linked_tags::applies(&self.language) && !self.read_only
    }

    /// The cursor moved: the tag pair it's in now, if any.
    pub(crate) fn find_linked_tag(&mut self, cx: &mut Context<Self>) {
        if !self.links_tags() {
            return;
        }
        let state = self.state.read(cx);
        let len = state.text().len();
        // Right after an edit `follow_linked_tag` looks again.
        if len != self.linked.len && self.linked.pair.is_some() {
            return;
        }
        let selection = state.selected_range();
        self.linked.len = len;
        self.linked.pair = None;
        if !selection.is_empty() || len > MAX_LINKED_TEXT {
            return;
        }
        let text = state.value().to_string();
        self.linked.pair =
            linked_tags::linked_names(&text, selection.start).map(|(here, there)| {
                let name = text[here.clone()].to_string();
                (here, there, name)
            });
    }

    /// The text changed: a tag name the cursor was in takes its partner
    /// along.
    pub(crate) fn follow_linked_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The partner's own edit: the pair was moved along already.
        if std::mem::take(&mut self.linked.applying) {
            self.linked.len = self.state.read(cx).text().len();
            return;
        }
        let Some((here, there, name)) = self.linked.pair.take() else {
            self.linked.len = self.state.read(cx).text().len();
            return;
        };
        let (text, cursor) = {
            let state = self.state.read(cx);
            (state.value().to_string(), state.selected_range())
        };
        let delta = text.len() as isize - self.linked.len as isize;
        self.linked.len = text.len();
        let end = here.end as isize + delta;
        let fits = end >= here.start as isize
            && (end as usize) <= text.len()
            && cursor.is_empty()
            && cursor.start >= here.start
            && cursor.start as isize <= end;
        if !fits || !text.is_char_boundary(here.start) || !text.is_char_boundary(end as usize) {
            return self.find_linked_tag(cx);
        }
        let renamed = &text[here.start..end as usize];
        let after = there.start > here.start;
        let shift = |at: usize| {
            if after {
                (at as isize + delta) as usize
            } else {
                at
            }
        };
        let partner = shift(there.start)..shift(there.end);
        let partner_holds = text.get(partner.clone()) == Some(name.as_str());
        if !partner_holds || !linked_tags::is_name(renamed) || renamed == name {
            if renamed != name {
                return self.find_linked_tag(cx);
            }
            self.linked.pair = Some((here.start..end as usize, partner, name));
            return;
        }
        let renamed = renamed.to_string();
        let utf16 = |byte: usize| text[..byte].encode_utf16().count();
        // The cursor stays in the name being typed.
        let moved = renamed.len() as isize - name.len() as isize;
        let keep = if after {
            cursor.start
        } else {
            (cursor.start as isize + moved) as usize
        };
        // The pair as it will be, kept even while the name is empty.
        let here = if after {
            here.start..end as usize
        } else {
            (here.start as isize + moved) as usize..(end + moved) as usize
        };
        let there = partner.start..partner.start + renamed.len();
        self.linked.pair = Some((here, there, renamed.clone()));
        self.linked.applying = true;
        let range = utf16(partner.start)..utf16(partner.end);
        self.state.update(cx, |state, cx| {
            state.replace_text_in_range(Some(range), &renamed, window, cx);
            state.set_selected_range(keep..keep, cx);
        });
    }

    pub(crate) fn toggle_block_comment(
        &mut self,
        _: &ToggleBlockComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tokens) = line_edits::block_comment(&self.language) else {
            return;
        };
        self.line_edit(window, cx, |text, selection| {
            Some(line_edits::toggle_block_comment(text, selection, tokens))
        });
    }

    /// The bracket pairs found in the text (outside strings and comments).
    fn pairs(&self) -> Vec<(usize, usize)> {
        self.bracket_pairs()
            .iter()
            .map(|pair| (pair.open, pair.close))
            .collect()
    }

    pub(crate) fn go_to_bracket(
        &mut self,
        _: &GoToBracket,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pairs = self.pairs();
        let cursor = self.state.read(cx).cursor();
        if let Some(to) = line_edits::matching_bracket(&pairs, cursor) {
            self.state
                .update(cx, |state, cx| state.set_selected_range(to..to, cx));
        }
    }

    pub(crate) fn expand_selection(
        &mut self,
        _: &ExpandSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text, selection) = {
            let state = self.state.read(cx);
            (state.value().to_string(), state.selected_range())
        };
        let pairs = self.pairs();
        let wider = line_edits::expand_selection(&text, selection.clone(), &pairs);
        if wider == selection {
            return;
        }
        // A fresh start unless this carries on from the last expansion.
        if self.selections.expanded_to.as_ref() != Some(&selection) {
            self.selections.expanded.clear();
        }
        self.selections.expanded.push(selection);
        self.selections.expanded_to = Some(wider.clone());
        self.state
            .update(cx, |state, cx| state.set_selected_range(wider, cx));
    }

    pub(crate) fn shrink_selection(
        &mut self,
        _: &ShrinkSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.read(cx).selected_range();
        if self.selections.expanded_to.as_ref() != Some(&selection) {
            return;
        }
        let Some(narrower) = self.selections.expanded.pop() else {
            return;
        };
        self.selections.expanded_to = Some(narrower.clone());
        self.state
            .update(cx, |state, cx| state.set_selected_range(narrower, cx));
    }

    pub(crate) fn cursor_undo(&mut self, _: &CursorUndo, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.state.read(cx).text().len();
        let Some(previous) = self.selections.past.pop() else {
            return;
        };
        let previous = previous.start.min(len)..previous.end.min(len);
        self.selections.restoring = true;
        self.state
            .update(cx, |state, cx| state.set_selected_range(previous, cx));
    }

    pub(crate) fn cursors_at_line_ends(
        &mut self,
        _: &CursorsAtLineEnds,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text, selection) = {
            let state = self.state.read(cx);
            (state.value().to_string(), state.selected_range())
        };
        let ends = line_edits::line_ends(&text, selection);
        self.state.update(cx, |state, cx| {
            let mut ends = ends.into_iter();
            if let Some(first) = ends.next() {
                state.set_selected_range(first..first, cx);
            }
            for end in ends {
                state.add_selection(end..end, cx);
            }
        });
    }

    pub(crate) fn trim_trailing_whitespace(
        &mut self,
        _: &TrimTrailingWhitespace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_edit(window, cx, |text, selection| {
            line_edits::trim_trailing_whitespace(text, selection.start)
        });
    }

    pub(crate) fn show_parameter_hints(
        &mut self,
        _: &ShowParameterHints,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_signature(cx);
    }

    pub(crate) fn next_problem(
        &mut self,
        _: &NextProblem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_problem(true, window, cx);
    }

    pub(crate) fn previous_problem(
        &mut self,
        _: &PreviousProblem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_problem(false, window, cx);
    }

    /// F8 and ⇧F8: the next (or previous) problem in this file, around the
    /// end, with its message shown.
    fn go_to_problem(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.state.read(cx).value().to_string();
        let cursor = self.state.read(cx).cursor();
        let mut starts: Vec<usize> = self
            .problem_positions()
            .into_iter()
            .map(|(position, utf32)| impulse_lsp::edits::offset_of(&text, position, utf32))
            .collect();
        starts.sort_unstable();
        starts.dedup();
        let target = if forward {
            starts
                .iter()
                .find(|&&s| s > cursor)
                .or(starts.first())
                .copied()
        } else {
            starts
                .iter()
                .rev()
                .find(|&&s| s < cursor)
                .or(starts.last())
                .copied()
        };
        if let Some(at) = target {
            self.state.update(cx, |state, cx| {
                state.set_selected_range(at..at, cx);
                state.focus(window, cx);
            });
        }
    }
}

impl EditorTab {
    /// Home and ⌘←: to the line's first non-blank character, then from
    /// there to column 0 (Monaco's `cursorHome`). Where that's the line's
    /// start anyway, and on a wrapped line's later rows, GPUI Kit's own
    /// move does it for every cursor; otherwise only the active cursor
    /// moves (GPUI Kit doesn't let others be read or moved one by one).
    pub(crate) fn cursor_home(
        &mut self,
        _: &CursorHome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (target, line_start, wrapped) = {
            let state = self.state.read(cx);
            let text = state.value();
            let cursor = state.cursor();
            let line_start = line_start(&text, cursor);
            let top = |offset: usize| state.range_to_bounds(&(offset..offset)).map(|b| b.top());
            let wrapped = matches!((top(line_start), top(cursor)), (Some(a), Some(b)) if a != b);
            (home_target(&text, cursor), line_start, wrapped)
        };
        if target == line_start || wrapped {
            let focus = self.state.focus_handle(cx);
            focus.dispatch_action(&MoveHome, window, cx);
            return;
        }
        self.state
            .update(cx, |state, cx| state.set_selected_range(target..target, cx));
    }

    /// ⌘L: select the cursor's line, line end included; again, the next
    /// line too (Monaco's `expandLineSelection`).
    pub(crate) fn expand_line_selection(
        &mut self,
        _: &ExpandLineSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text, selection) = {
            let state = self.state.read(cx);
            (state.value().to_string(), state.selected_range())
        };
        let lines = line_selection(&text, selection);
        self.state
            .update(cx, |state, cx| state.set_selected_range(lines, cx));
    }
}

/// Where the line holding byte `offset` starts.
fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |i| i + 1)
}

/// Where Home takes a cursor at `cursor`: the line's first non-blank
/// character, or its start when it's there already (or the line is
/// blank).
fn home_target(text: &str, cursor: usize) -> usize {
    let start = line_start(text, cursor);
    let line = text[start..].split('\n').next().unwrap_or("");
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let first = if indent == line.len() {
        start
    } else {
        start + indent
    };
    if cursor == first {
        start
    } else {
        first
    }
}

/// `selection` grown to whole lines: from its first line's start to the
/// start of the line after its last (the end of the text on the last
/// line). A selection ending at a line's start takes that line as well, so
/// each press adds one.
fn line_selection(text: &str, selection: Range<usize>) -> Range<usize> {
    let start = line_start(text, selection.start);
    let end = text[selection.end..]
        .find('\n')
        .map_or(text.len(), |i| selection.end + i + 1);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_goes_to_the_first_non_blank_then_column_zero() {
        let text = "fn a() {\n    let x = 1;\n\n  \n}";
        // From inside the line, to its first non-blank character.
        assert_eq!(home_target(text, 20), 13);
        // From there, to the line's start, and back.
        assert_eq!(home_target(text, 13), 9);
        assert_eq!(home_target(text, 9), 13);
        // Inside the indent, to the first non-blank character.
        assert_eq!(home_target(text, 11), 13);
        // Unindented and blank lines start at their first character.
        assert_eq!(home_target(text, 4), 0);
        assert_eq!(home_target(text, 24), 24);
        assert_eq!(home_target(text, 26), 25);
    }

    #[test]
    fn each_line_selection_takes_one_more_line() {
        let text = "one\ntwo\nthree";
        assert_eq!(line_selection(text, 1..1), 0..4);
        assert_eq!(line_selection(text, 0..4), 0..8);
        assert_eq!(line_selection(text, 0..8), 0..13);
        // The last line ends at the end of the text.
        assert_eq!(line_selection(text, 10..10), 8..13);
        assert_eq!(line_selection(text, 8..13), 8..13);
        // A selection inside a line, from its line's start.
        assert_eq!(line_selection(text, 5..10), 4..13);
    }
}
