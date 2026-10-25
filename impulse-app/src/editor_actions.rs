//! Monaco's editor commands that GPUI Kit's editor doesn't have, on top of
//! `impulse_kit::line_edits`: Toggle Block Comment, Go to Bracket, Next
//! and Previous Problem, Expand and Shrink Selection, parameter hints,
//! Cursor Undo, cursors at line ends, Trim Trailing Whitespace (their keys
//! are in `editor_keys`).

use std::ops::Range;

use gpui_kit::{Context, Window};
use impulse_kit::line_edits;

use crate::editor_keys::{
    CursorUndo, CursorsAtLineEnds, ExpandSelection, GoToBracket, NextProblem, PreviousProblem,
    ShowParameterHints, ShrinkSelection, ToggleBlockComment, TrimTrailingWhitespace,
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

impl EditorTab {
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

    pub(crate) fn go_to_bracket(&mut self, _: &GoToBracket, _: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn next_problem(&mut self, _: &NextProblem, window: &mut Window, cx: &mut Context<Self>) {
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
