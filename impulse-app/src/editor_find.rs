//! Find and replace in an editor (Monaco's find widget in the Swift app):
//! the terminal's find bar (`find_bar`) at the top of the editor, with its
//! replace row, over GPUI Kit's editor search. GPUI Kit's own search panel
//! is off (`searchable(false)`), so ⌘F, Find Next and Find Previous reach
//! Impulse.
//!
//! As in Monaco: typing selects the nearest match from the cursor, Return
//! and F3 step from the selection, every match is highlighted with the
//! current one selected, ⌘F again (or ⌘F from the text) seeds the query
//! from a one-line selection or the word at the cursor, Escape in the
//! field or the text closes, Replace replaces the selected match (or first
//! moves to one) and Replace All every match as one undo step, and F3 /
//! ⇧F3 with the bar closed go to the last query's next or previous match.

use gpui_kit::component::input::RopeExt as _;
use gpui_kit::{AnyElement, Context, Window};
use impulse_kit::terminal_find::TerminalFindQuery;

use crate::editor_tab::EditorTab;
use crate::find_bar::{ChromeTheme, FindBar, FindHost};
use gpui_kit::base::input::SearchOptions;

impl FindHost for EditorTab {
    fn find_bar_mut(&mut self) -> Option<&mut FindBar> {
        self.find.as_mut()
    }

    fn find_changed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.run_find(cx);
    }

    fn find_step(&mut self, forward: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.step_find(forward, cx);
    }

    fn find_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_find(window, cx);
    }

    fn find_replace(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_find(all, window, cx);
    }
}

fn options(query: &TerminalFindQuery) -> SearchOptions {
    SearchOptions {
        case_sensitive: query.case_sensitive,
        whole_word: query.whole_word,
        regex: query.regex,
    }
}

impl EditorTab {
    /// Whether the buffer's editor is showing to search in (not the
    /// Markdown preview alone, nor the inline Diff View's document).
    fn find_available(&self) -> bool {
        self.preview != Some(false) && !self.diff.as_ref().is_some_and(|d| d.is_inline())
    }

    /// Find (⌘F): from the text, open the bar or bring the keyboard back
    /// to it, seeded from the selection; from the bar's field, close it.
    pub fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .find
            .as_ref()
            .is_some_and(|f| f.field_focused(window, cx))
        {
            return self.close_find(window, cx);
        }
        self.open_find(false, window, cx);
    }

    /// Show the bar (with the replace row when `replace`) and give its
    /// field the keyboard, the query seeded from a one-line selection or
    /// the word at the cursor (Monaco's `seedSearchStringFromSelection`).
    pub fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find_available() {
            return;
        }
        let seed = self.find_seed(cx);
        let Some(chrome) = cx.try_global::<ChromeTheme>().cloned() else {
            return;
        };
        let opened = self.find.is_none();
        if opened {
            let mut query = self.last_find.clone();
            if let Some(seed) = &seed {
                query.text = seed.clone();
            }
            let find = FindBar::new(&chrome.theme, "Find", query, !self.read_only, window, cx);
            self.find = Some(find);
        }
        let Some(find) = self.find.as_mut() else {
            return;
        };
        if let Some(seed) = seed.filter(|s| !opened && *s != find.query.text) {
            find.set_query_text(&seed, cx);
        }
        // ⌥⌘F with a query goes to the replacement, as in Monaco.
        let to_replace = replace && !find.query.text.is_empty();
        if replace {
            find.show_replace(true, to_replace, window, cx);
        }
        if !to_replace {
            if opened {
                find.focus(window, cx);
            } else {
                find.focus_selected(window, cx);
            }
        }
        self.run_find(cx);
    }

    /// ⌥⌘F: show the replace row, or hide it when its field has the
    /// keyboard.
    pub(crate) fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return self.open_find(false, window, cx);
        }
        if let Some(find) = self.find.as_mut() {
            if find.replace_focused(window, cx) {
                find.show_replace(false, false, window, cx);
                cx.notify();
                return;
            }
        }
        self.open_find(true, window, cx);
    }

    /// What ⌘F seeds the query with: the selection if it's on one line,
    /// else the word at the cursor.
    fn find_seed(&self, cx: &gpui_kit::App) -> Option<String> {
        let state = self.state.read(cx);
        let range = state.selected_range();
        let text = state.text();
        if !range.is_empty() {
            let selected = text.slice(range).to_string();
            return (!selected.contains('\n')).then_some(selected);
        }
        let point = text.offset_to_point(range.start);
        let line_start = text.line_start_offset(point.row);
        let line = text.slice_line(point.row).to_string();
        let word = impulse_kit::occurrences::word_at(&line, range.start - line_start)?;
        (!word.is_empty()).then(|| line[word].to_string())
    }

    /// Hide the bar and the highlights, keeping the query for next time,
    /// and give the text the keyboard.
    pub(crate) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = self.find.take() {
            self.last_find = find.query;
        }
        self.state.update(cx, |state, cx| state.close_search(cx));
        self.focus(window, cx);
        cx.notify();
    }

    /// Search for the bar's query, selecting the nearest match from the
    /// selection's start, as Monaco's find-as-you-type does.
    fn run_find(&mut self, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_ref() else {
            return;
        };
        let (text, options) = (find.query.text.clone(), options(&find.query));
        let result = self.state.update(cx, |state, cx| {
            state.set_search_query_with(text, options, cx)
        });
        if matches!(result, Ok(n) if n > 0) {
            self.state.update(cx, |state, cx| {
                let from = state.selected_range().start;
                if let Some(range) = state.search_match_from(from, true, cx) {
                    state.set_selected_range(range, cx);
                }
            });
        }
        self.update_find_stats(cx);
    }

    /// Return, Shift+Return, the arrows, F3 and ⇧F3 with the bar open:
    /// the match after the selection, or before it.
    fn step_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.find.as_ref().is_none_or(|f| f.query.text.is_empty()) {
            return;
        }
        self.state.update(cx, |state, cx| {
            let selected = state.selected_range();
            let from = if forward {
                selected.end
            } else {
                selected.start
            };
            if let Some(range) = state.search_match_from(from, forward, cx) {
                state.set_selected_range(range, cx);
            }
        });
        self.update_find_stats(cx);
    }

    /// Find Next and Find Previous (F3, ⇧F3): step through the bar's
    /// matches, or with it closed select the last query's next or previous
    /// match without showing it (opening it when there's no query yet).
    pub(crate) fn find_next(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_some() {
            return self.step_find(forward, cx);
        }
        if !self.find_available() {
            return;
        }
        let mut query = self.last_find.clone();
        if query.text.is_empty() {
            match self.find_seed(cx) {
                Some(seed) => query.text = seed,
                None => return self.open_find(false, window, cx),
            }
        }
        let options = options(&query);
        self.state.update(cx, |state, cx| {
            if state
                .set_search_query_with(query.text.clone(), options, cx)
                .is_ok()
            {
                let selected = state.selected_range();
                let from = if forward {
                    selected.end
                } else {
                    selected.start
                };
                if let Some(range) = state.search_match_from(from, forward, cx) {
                    state.set_selected_range(range, cx);
                }
            }
            state.close_search(cx);
        });
        self.last_find = query;
    }

    /// Replace: the selected match with the replacement, then on to the
    /// next one; with no match selected, first only go to the next one (as
    /// Monaco does). Replace All: every match, as one undo step.
    fn replace_find(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(find) = self.find.as_ref() else {
            return;
        };
        if self.read_only || find.query.text.is_empty() {
            return;
        }
        let replacement = find.replacement(cx);
        if all {
            self.state.update(cx, |state, cx| {
                state.replace_all_search_matches(&replacement, window, cx)
            });
            return self.update_find_stats(cx);
        }
        let on_match = {
            let state = self.state.read(cx);
            let selected = state.selected_range();
            let ranges = state.search_session().matcher.matched_ranges();
            ranges
                .binary_search_by_key(&selected.start, |r| r.start)
                .ok()
                .is_some_and(|ix| ranges[ix] == selected)
        };
        if !on_match {
            return self.step_find(true, cx);
        }
        self.state.update(cx, |state, cx| {
            let start = state.selected_range().start;
            // The selected match is the one GPUI Kit replaces.
            state.search_match_from(start, true, cx);
            if state.replace_current_search_match(&replacement, window, cx) {
                // On from the end of the replacement, so a replacement that
                // matches again isn't replaced again.
                if let Some(next) = state.search_match_from(start + replacement.len(), true, cx) {
                    state.set_selected_range(next, cx);
                }
            }
        });
        self.update_find_stats(cx);
    }

    /// The text changed under an open bar: GPUI Kit's search found the
    /// matches again with the edit (an active search follows every one),
    /// so count them again.
    pub(crate) fn find_text_changed(&mut self, cx: &mut Context<Self>) {
        if self.find.is_some() {
            self.update_find_stats(cx);
        }
    }

    /// The bar's count, and which match is current: the one the selection
    /// is on, as Monaco counts (none after an edit moves away from it).
    fn update_find_stats(&mut self, cx: &mut Context<Self>) {
        let stats = {
            let state = self.state.read(cx);
            let session = state.search_session();
            let ranges = session.matcher.matched_ranges();
            let selected = state.selected_range();
            let at = ranges.partition_point(|r| r.start < selected.start);
            let current = ranges
                .get(at)
                .filter(|r| **r == selected)
                .map_or(0, |_| at + 1);
            impulse_terminal::SearchStats {
                current,
                total: ranges.len(),
                capped: false,
                invalid: session.error.is_some(),
            }
        };
        if let Some(find) = self.find.as_mut() {
            find.stats = stats;
        }
        cx.notify();
    }

    /// The bar over the editor, when it's open.
    pub(crate) fn find_element(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let chrome = cx.try_global::<ChromeTheme>()?.palette.clone();
        Some(find.render(&chrome, cx))
    }

    /// The theme changed: the fields take its colors.
    pub(crate) fn find_retheme(&self, cx: &mut gpui_kit::App) {
        let (Some(find), Some(chrome)) = (self.find.as_ref(), cx.try_global::<ChromeTheme>())
        else {
            return;
        };
        let colors = crate::command_editor::EditorColors::new(&chrome.theme);
        find.set_colors(colors, cx);
    }

    /// Escape in the text: closes the bar (as Monaco's does). Whether it
    /// did.
    pub(crate) fn find_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match &self.find {
            Some(find) if !find.contains_focus(window, cx) => {
                self.close_find(window, cx);
                true
            }
            _ => false,
        }
    }

    /// Find `text` (the snapshot's `edit=` `find TEXT`), opening the bar.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_find(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            self.open_find(false, window, cx);
        }
        if let Some(find) = self.find.as_mut() {
            find.set_query_text(text, cx);
        }
        self.run_find(cx);
    }
}
