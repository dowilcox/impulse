use crate::input::InputModeKind;
use aho_corasick::AhoCorasick;
use gpui::{Context, Window};
use regex::{Regex, RegexBuilder};
use ropey::Rope;
use std::{ops::Range, rc::Rc};

use super::{
    InputBaseState, Replace, RopeExt as _, Search, movement::MoveDirection, state::ScrollPadding,
};

/// Impulse: how a search query matches, for a custom search UI's toggles
/// (Monaco's find widget: Aa, ab and .*).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SearchOptions {
    /// Match case exactly. Otherwise ASCII letters match either case (and,
    /// with `regex`, every letter does).
    pub case_sensitive: bool,
    /// Only matches that start and end at a word boundary: the character
    /// beside each end, or the match's own character there, is not a word
    /// character (alphanumeric or `_`).
    pub whole_word: bool,
    /// The query is a regular expression (the `regex` crate's syntax, `^`
    /// and `$` at line ends). Empty matches are skipped.
    pub regex: bool,
}

/// Stateful, presentation-independent search engine used by text inputs.
#[derive(Debug, Clone)]
pub struct SearchMatcher {
    text: Rope,
    pub query: Option<AhoCorasick>,
    /// Impulse: the query as a regular expression, in place of `query`.
    regex: Option<Regex>,
    /// Impulse: keep only whole-word matches.
    whole_word: bool,
    matched_ranges: Rc<Vec<Range<usize>>>,
    current_match_ix: usize,
    replacing: bool,
}

/// One search over an input: the query, how the built-in panel shows it, and
/// its matches. Read it through [`InputBaseState::search_session`]; it is
/// written only through the input state's search methods, and it grows, so
/// build it with `Default` and do not destructure it exhaustively.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SearchSession {
    /// The built-in search panel is showing.
    pub open: bool,
    pub replace_mode: bool,
    pub case_insensitive: bool,
    pub query: String,
    pub replacement: String,
    pub anchor_offset: Option<usize>,
    pub matcher: SearchMatcher,
    /// Impulse: how the query matches. `case_insensitive` mirrors
    /// `!options.case_sensitive`.
    pub options: SearchOptions,
    /// Impulse: why the query matches nothing because it can't (an invalid
    /// regular expression), as a message to show.
    pub error: Option<String>,
    /// A search is in progress and its matches are highlighted: the panel is
    /// open, or a query was set without it and not closed since.
    active: bool,
}

impl Default for SearchSession {
    fn default() -> Self {
        Self {
            open: false,
            active: false,
            replace_mode: false,
            case_insensitive: true,
            query: String::new(),
            replacement: String::new(),
            anchor_offset: None,
            matcher: SearchMatcher::new(),
            options: SearchOptions::default(),
            error: None,
        }
    }
}

impl SearchSession {
    pub(crate) fn open(&mut self, replace_mode: bool, replaceable: bool) {
        self.open = true;
        self.active = true;
        self.replace_mode = replace_mode && replaceable;
    }

    /// Start a search without the built-in panel. A custom search UI drives
    /// the session through [`InputBaseState::set_search_query`], and the
    /// editor highlights the matches the same way it does for the panel.
    pub(crate) fn activate(&mut self) {
        self.active = true;
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.active = false;
    }

    /// Whether a search is in progress: the built-in panel is open, or a
    /// query was set without it and [`InputBaseState::close_search`] has not
    /// run since. Matches are highlighted while this holds.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Set the query with the case mode only, keeping whole-word and regex
    /// as they are.
    pub(crate) fn update_query(&mut self, query: impl Into<String>, case_insensitive: bool) {
        let options = SearchOptions {
            case_sensitive: !case_insensitive,
            ..self.options
        };
        let _ = self.update_query_with(query, options);
    }

    /// Impulse: set the query and how it matches. An invalid regular
    /// expression leaves no matches and its message in `error`.
    pub(crate) fn update_query_with(
        &mut self,
        query: impl Into<String>,
        options: SearchOptions,
    ) -> Result<(), String> {
        let query = query.into();
        if self.query != query || self.options != options {
            self.query = query;
            self.options = options;
            self.case_insensitive = !options.case_sensitive;
            self.error = self
                .matcher
                .update_query_with(&self.query, options)
                .err()
                .map(|error| error.to_string());
        }
        self.error.clone().map_or(Ok(()), Err)
    }
}

impl<M: InputModeKind> InputBaseState<M> {
    /// Open the search session, or re-invoke it if it is already open.
    ///
    /// This is not idempotent: every call advances
    /// [`InputBaseState::search_activation_revision`], and the presentation
    /// layer answers that by re-focusing the search field and selecting its
    /// contents, the same as pressing the shortcut a second time. Call it from
    /// an action or another user gesture, never from a render pass or an
    /// observer that runs every frame — that would re-select the field under
    /// the user on every frame and make it impossible to type.
    pub fn open_search(&mut self, replace_mode: bool, cx: &mut Context<Self>) {
        if !self.searchable {
            return;
        }
        self.search_activation_revision = self.search_activation_revision.wrapping_add(1);
        self.search_session
            .open(replace_mode, self.is_replaceable());
        let selected = self.selected_text().to_string();
        let query = if selected.is_empty() {
            self.search_session.query.clone()
        } else {
            selected
        };
        let query_changed = query != self.search_session.query;
        // A retained query resumes its previous occurrence. Only a new query
        // is anchored to the current viewport.
        self.search_session.anchor_offset = if query_changed {
            self.last_layout
                .as_ref()
                .map(|layout| layout.visible_range_offset.start)
        } else {
            None
        };
        let case_insensitive = self.search_session.case_insensitive;
        self.search_session.update_query(query, case_insensitive);
        self.search_session.matcher.update(&self.text);
        if query_changed && let Some(anchor) = self.search_session.anchor_offset {
            self.search_session.matcher.update_cursor_by_offset(anchor);
        }
        cx.notify();
    }

    pub fn search_session(&self) -> &SearchSession {
        &self.search_session
    }

    /// A counter that advances every time [`InputBaseState::open_search`] runs,
    /// including while the session is already open.
    ///
    /// Re-invoking search leaves the session itself identical, so a presentation
    /// layer that decides what to rebuild by comparing session state cannot see
    /// the second request. Fold this into that comparison to notice it.
    pub fn search_activation_revision(&self) -> u64 {
        self.search_activation_revision
    }

    #[doc(hidden)]
    pub fn set_search_replace_mode(&mut self, replace_mode: bool, cx: &mut Context<Self>) {
        self.search_session.replace_mode = replace_mode && self.is_replaceable();
        cx.notify();
    }

    /// Returns true if the search panel can replace the matches.
    ///
    /// This is false when the input is not `replaceable`, or when it is
    /// `disabled` or `readonly`.
    pub fn is_replaceable(&self) -> bool {
        self.replaceable && self.is_editable()
    }

    /// Set the search query and highlight its matches.
    ///
    /// This is the entry point for a custom search UI: it needs neither
    /// `searchable` nor the built-in panel. Navigate the matches with
    /// [`InputBaseState::next_search_match`] and
    /// [`InputBaseState::previous_search_match`], read the count and the
    /// current index from [`InputBaseState::search_session`], and end the
    /// search with [`InputBaseState::close_search`].
    ///
    /// A changed query or case mode starts at the current viewport. Setting
    /// the same query and case mode again retains the current occurrence.
    pub fn set_search_query(
        &mut self,
        query: impl Into<String>,
        case_insensitive: bool,
        cx: &mut Context<Self>,
    ) {
        let options = SearchOptions {
            case_sensitive: !case_insensitive,
            ..self.search_session.options
        };
        let _ = self.set_search_query_with(query, options, cx);
    }

    /// Impulse: [`InputBaseState::set_search_query`] with whole-word and
    /// regular-expression matching. Returns how many matches there are, or
    /// the message of an invalid regular expression (also kept in
    /// [`SearchSession::error`]), which leaves no matches.
    ///
    /// A changed query or options start at the current viewport. Setting
    /// the same ones again retains the current occurrence.
    pub fn set_search_query_with(
        &mut self,
        query: impl Into<String>,
        options: SearchOptions,
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let query = query.into();
        let query_changed =
            query != self.search_session.query || options != self.search_session.options;
        self.search_session.anchor_offset = if query_changed {
            self.last_layout
                .as_ref()
                .map(|layout| layout.visible_range_offset.start)
        } else {
            None
        };
        self.search_session.activate();
        let result = self.search_session.update_query_with(query, options);
        self.search_session.matcher.update(&self.text);
        if query_changed && let Some(anchor) = self.search_session.anchor_offset {
            self.search_session.matcher.update_cursor_by_offset(anchor);
        }
        cx.notify();
        result.map(|()| self.search_session.matcher.len())
    }

    /// End the search: hide the built-in panel and the match highlights. The
    /// query is kept so the next [`InputBaseState::open_search`] resumes it.
    pub fn close_search(&mut self, cx: &mut Context<Self>) {
        self.search_session.close();
        cx.notify();
    }

    pub fn next_search_match(&mut self, cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.sync_search_matcher();
        let range = self.search_session.matcher.next()?;
        // Match order does not describe viewport direction after a manual
        // scroll. Always allow search navigation to reveal the active match.
        self.scroll_to_with_padding(range.end, None, ScrollPadding::SurroundingLines, cx);
        Some(range)
    }

    pub fn previous_search_match(&mut self, cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.sync_search_matcher();
        let range = self.search_session.matcher.next_back()?;
        // Match order does not describe viewport direction after a manual
        // scroll. Always allow search navigation to reveal the active match.
        self.scroll_to_with_padding(range.start, None, ScrollPadding::SurroundingLines, cx);
        Some(range)
    }

    /// Impulse: make the match nearest `offset` in a direction the current
    /// one and reveal it, as Monaco's find widget moves from the cursor: going
    /// forward, the first match starting at or after `offset`; going back,
    /// the last one starting before it; wrapping around either way. `None`
    /// without matches. [`InputBaseState::next_search_match`] steps from the
    /// current match instead, wherever the cursor is.
    pub fn search_match_from(
        &mut self,
        offset: usize,
        forward: bool,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.sync_search_matcher();
        let matcher = &mut self.search_session.matcher;
        let ranges = matcher.matched_ranges();
        if ranges.is_empty() {
            return None;
        }
        let after = ranges.partition_point(|range| range.start < offset);
        let ix = match (forward, after) {
            (true, ix) if ix < ranges.len() => ix,
            (true, _) => 0,
            (false, 0) => ranges.len() - 1,
            (false, ix) => ix - 1,
        };
        matcher.set_current_match_index(ix);
        let range = ranges[ix].clone();
        let reveal = if forward { range.end } else { range.start };
        self.scroll_to_with_padding(reveal, None, ScrollPadding::SurroundingLines, cx);
        cx.notify();
        Some(range)
    }

    /// Replace the current match and move on to the next one. Returns whether
    /// there was a match to replace.
    pub fn replace_current_search_match(
        &mut self,
        replacement: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_replaceable() {
            return false;
        }
        self.sync_search_matcher();
        let matcher = &mut self.search_session.matcher;
        let Some(range) = matcher
            .matched_ranges()
            .get(matcher.current_match_index())
            .cloned()
        else {
            return false;
        };
        let next = matcher.peek().unwrap_or_else(|| range.clone());
        let direction = matcher
            .has_next_without_wrap()
            .then_some(MoveDirection::Down);
        if direction.is_none() {
            matcher.set_current_match_index(0);
        }
        matcher.begin_replacement();
        let range_utf16 = self.range_to_utf16(&range);
        self.scroll_to(next.end, direction, cx);
        self.replace_text_in_range_silent(Some(range_utf16), replacement, window, cx);
        true
    }

    /// Replace every match. Returns how many were replaced.
    pub fn replace_all_search_matches(
        &mut self,
        replacement: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        if !self.is_replaceable() {
            return 0;
        }
        self.sync_search_matcher();
        let ranges = self.search_session.matcher.matched_ranges();
        if ranges.is_empty() {
            return 0;
        }
        let mut text = self.text.clone();
        for range in ranges.iter().rev() {
            text.replace(range.clone(), replacement);
        }
        self.search_session.matcher.begin_replacement();
        let count = ranges.len();
        self.replace_text_in_range_silent(Some(0..self.text.len()), &text.to_string(), window, cx);
        self.scroll_to(0, Some(MoveDirection::Down), cx);
        count
    }

    /// Keep the matches in step with an edit. A closed search skips the scan:
    /// it copies and searches the whole document, and nothing reads the
    /// matches until the search is resumed or navigated, which sync first.
    pub(super) fn update_search(&mut self, _cx: &mut gpui::App) {
        if !self.search_session.is_active() {
            return;
        }
        self.sync_search_matcher();
    }

    /// Recompute the matches if the text changed since the last scan.
    fn sync_search_matcher(&mut self) {
        self.search_session.matcher.update(&self.text);
    }

    /// An input that is not `searchable` leaves the shortcut to its
    /// ancestors, so a custom search UI can take it.
    pub(super) fn on_action_search(&mut self, _: &Search, _: &mut Window, cx: &mut Context<Self>) {
        if !self.searchable {
            cx.propagate();
            return;
        }
        self.open_search(false, cx);
    }

    pub(super) fn on_action_replace(
        &mut self,
        _: &Replace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.searchable {
            cx.propagate();
            return;
        }
        self.open_search(true, cx);
    }
}

impl Default for SearchMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchMatcher {
    pub fn new() -> Self {
        Self {
            text: "".into(),
            query: None,
            regex: None,
            whole_word: false,
            matched_ranges: Rc::new(Vec::new()),
            current_match_ix: 0,
            replacing: false,
        }
    }

    /// Update the source text and recompute matches.
    pub fn update(&mut self, text: &Rope) {
        if self.text.eq(text) {
            self.replacing = false;
            return;
        }
        self.text = text.clone();
        self.update_matches();
    }

    pub fn update_query(&mut self, query: &str, case_insensitive: bool) {
        let options = SearchOptions {
            case_sensitive: !case_insensitive,
            ..SearchOptions::default()
        };
        let _ = self.update_query_with(query, options);
    }

    /// Impulse: set the query and how it matches, and recompute the
    /// matches. An invalid regular expression is returned, not a panic, and
    /// leaves no matches.
    pub fn update_query_with(
        &mut self,
        query: &str,
        options: SearchOptions,
    ) -> Result<(), regex::Error> {
        self.query = None;
        self.regex = None;
        self.whole_word = options.whole_word;
        let mut result = Ok(());
        if query.is_empty() {
            // Nothing matches an empty query.
        } else if options.regex {
            match RegexBuilder::new(query)
                .case_insensitive(!options.case_sensitive)
                .multi_line(true)
                .build()
            {
                Ok(regex) => self.regex = Some(regex),
                Err(error) => result = Err(error),
            }
        } else {
            self.query = Some(
                AhoCorasick::builder()
                    .ascii_case_insensitive(!options.case_sensitive)
                    .build([query])
                    .expect("failed to build input search query"),
            );
        }
        self.update_matches();
        result
    }

    pub fn matched_ranges(&self) -> Rc<Vec<Range<usize>>> {
        self.matched_ranges.clone()
    }

    pub fn current_match_index(&self) -> usize {
        self.current_match_ix
    }

    /// The index of the current match into [`SearchMatcher::matched_ranges`],
    /// `None` while there is no match.
    pub fn current(&self) -> Option<usize> {
        (!self.is_empty()).then_some(self.current_match_ix)
    }

    pub fn len(&self) -> usize {
        self.matched_ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.matched_ranges.is_empty()
    }

    /// `2/5`: the current match and the total, `0/0` without matches.
    pub fn label(&self) -> String {
        match self.current() {
            Some(ix) => format!("{}/{}", ix + 1, self.len()),
            None => "0/0".into(),
        }
    }

    fn peek(&self) -> Option<Range<usize>> {
        self.next_index()
            .and_then(|ix| self.matched_ranges.get(ix).cloned())
    }

    fn has_next_without_wrap(&self) -> bool {
        self.current_match_ix < self.matched_ranges.len().saturating_sub(1)
    }

    pub fn update_cursor_by_offset(&mut self, offset: usize) {
        for (ix, range) in self.matched_ranges.iter().enumerate() {
            self.current_match_ix = ix;
            if range.contains(&offset) || range.end >= offset {
                return;
            }
        }
    }

    /// Preserve the current logical match while a replacement mutates text.
    fn begin_replacement(&mut self) {
        self.replacing = true;
    }

    fn set_current_match_index(&mut self, index: usize) {
        self.current_match_ix = index.min(self.matched_ranges.len().saturating_sub(1));
    }

    fn next_index(&self) -> Option<usize> {
        if self.is_empty() {
            None
        } else if self.has_next_without_wrap() {
            Some(self.current_match_ix + 1)
        } else {
            Some(0)
        }
    }

    fn update_matches(&mut self) {
        let mut ranges = Vec::new();
        if let Some(query) = &self.query {
            let text = self.text.to_string();
            ranges.extend(
                query
                    .stream_find_iter(text.as_bytes())
                    .map(|result| result.expect("input search match").range()),
            );
            if self.whole_word {
                ranges.retain(|range| is_whole_word(&text, range));
            }
        } else if let Some(regex) = &self.regex {
            // Impulse: regular-expression matches.
            let text = self.text.to_string();
            ranges.extend(
                regex
                    .find_iter(&text)
                    .filter(|found| !found.is_empty())
                    .map(|found| found.range()),
            );
            if self.whole_word {
                ranges.retain(|range| is_whole_word(&text, range));
            }
        }
        self.matched_ranges = Rc::new(ranges);
        if !self.replacing || self.is_empty() {
            self.current_match_ix = 0;
        } else {
            self.current_match_ix = self.current_match_ix.min(self.len() - 1);
        }
        self.replacing = false;
    }
}

/// Impulse: whether `range` of `text` is bounded by word boundaries, the way
/// Monaco's whole-word search decides it: at each end, the character outside
/// or the match's own character there is not a word character.
fn is_whole_word(text: &str, range: &Range<usize>) -> bool {
    fn is_word(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }
    let matched = &text[range.clone()];
    let starts = text[..range.start].chars().next_back().is_none_or(|c| !is_word(c))
        || matched.chars().next().is_some_and(|c| !is_word(c));
    let ends = text[range.end..].chars().next().is_none_or(|c| !is_word(c))
        || matched.chars().next_back().is_some_and(|c| !is_word(c));
    starts && ends
}

impl Iterator for SearchMatcher {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Self::Item> {
        let ix = self.next_index()?;
        self.current_match_ix = ix;
        self.matched_ranges.get(ix).cloned()
    }
}

impl DoubleEndedIterator for SearchMatcher {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.is_empty() {
            return None;
        }
        if self.current_match_ix == 0 {
            self.current_match_ix = self.len();
        }
        self.current_match_ix -= 1;
        self.matched_ranges.get(self.current_match_ix).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_navigates_and_preserves_replacement_position() {
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from("foo FOO foo"));
        matcher.update_query("foo", true);
        assert_eq!(&*matcher.matched_ranges(), &[0..3, 4..7, 8..11]);
        assert_eq!(matcher.next(), Some(4..7));
        assert_eq!(matcher.next_back(), Some(0..3));

        matcher.set_current_match_index(2);
        matcher.begin_replacement();
        matcher.update(&Rope::from("foo FOO bar"));
        assert_eq!(matcher.current_match_index(), 1);
    }

    #[test]
    fn whole_word_keeps_only_bounded_matches() {
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from("foo foo_bar barfoo foo.x (foo) é foo"));
        let options = SearchOptions {
            whole_word: true,
            ..SearchOptions::default()
        };
        matcher.update_query_with("foo", options).unwrap();
        assert_eq!(&*matcher.matched_ranges(), &[0..3, 19..22, 26..29, 34..37]);
        // A match whose own edge is not a word character needs no boundary.
        matcher.update_query_with(".x", options).unwrap();
        assert_eq!(&*matcher.matched_ranges(), &[22..24]);
    }

    #[test]
    fn regex_matches_and_reports_invalid_patterns() {
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from("let a1 = 1;\nlet B22 = 2;"));
        let options = SearchOptions {
            regex: true,
            ..SearchOptions::default()
        };
        matcher.update_query_with(r"[a-z]\d+", options).unwrap();
        assert_eq!(&*matcher.matched_ranges(), &[4..6, 16..19]);
        let sensitive = SearchOptions {
            case_sensitive: true,
            ..options
        };
        matcher.update_query_with(r"[a-z]\d+", sensitive).unwrap();
        assert_eq!(&*matcher.matched_ranges(), &[4..6]);
        // Line anchors, and no empty matches.
        matcher.update_query_with("^let", options).unwrap();
        assert_eq!(matcher.len(), 2);
        matcher.update_query_with("x*", options).unwrap();
        assert!(matcher.is_empty());
        // An invalid pattern is an error and leaves no matches.
        assert!(matcher.update_query_with("(let", options).is_err());
        assert!(matcher.is_empty());
        assert_eq!(matcher.label(), "0/0");
    }

    #[test]
    fn session_keeps_the_error_until_the_query_changes() {
        let mut session = SearchSession::default();
        session.matcher.update(&Rope::from("abc"));
        let options = SearchOptions {
            regex: true,
            ..SearchOptions::default()
        };
        assert!(session.update_query_with("[", options).is_err());
        assert!(session.error.is_some());
        assert!(session.update_query_with("[", options).is_err());
        assert_eq!(session.update_query_with("[ab]", options), Ok(()));
        assert!(session.error.is_none());
        assert_eq!(session.matcher.len(), 2);
        // The panel's case toggle keeps whole-word and regex.
        session.update_query("[AB]", false);
        assert!(session.options.regex);
        assert_eq!(session.matcher.len(), 0);
    }

    #[test]
    fn next_wraps_to_start() {
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from(".....aaaaa.....aaaaa.....aaaaa"));
        matcher.update_query("aaaaa", false);
        matcher.set_current_match_index(2);
        assert_eq!(matcher.next(), Some(5..10));
    }

    #[test]
    fn a_query_set_without_the_panel_keeps_the_session_active_until_closed() {
        let mut session = SearchSession::default();
        assert!(!session.is_active());

        session.open(false, true);
        assert!(session.is_active());
        session.close();
        assert!(!session.is_active());

        // A custom search UI never opens the panel; setting a query is what
        // turns the match highlights on, and closing turns them off again.
        session.activate();
        assert!(session.is_active());
        assert!(!session.open);
        session.close();
        assert!(!session.is_active());
    }

    #[test]
    fn identical_query_keeps_the_current_match() {
        let mut session = SearchSession::default();
        session.update_query("foo", true);
        session.matcher.update(&Rope::from("foo bar foo baz foo"));
        session.matcher.update_cursor_by_offset(12);
        assert_eq!(session.matcher.current_match_index(), 2);

        // Reopening Find and the styled search panel's initial query echo both
        // update the session with the same query. Neither should reset the
        // previously active occurrence.
        session.update_query("foo", true);

        assert_eq!(session.matcher.current_match_index(), 2);
        assert_eq!(session.matcher.label(), "3/3");
    }

    #[test]
    fn replacement_keeps_current_match_index_on_next_match() {
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from("foo foo foo"));
        matcher.update_query("foo", true);
        assert_eq!(matcher.label(), "1/3");

        assert!(matcher.has_next_without_wrap());
        matcher.begin_replacement();
        matcher.update(&Rope::from("bar foo foo"));
        assert_eq!(matcher.current_match_index(), 0);
        assert_eq!(matcher.matched_ranges()[0], 4..7);
        assert_eq!(matcher.label(), "1/2");

        matcher.set_current_match_index(1);
        assert!(!matcher.has_next_without_wrap());
        matcher.set_current_match_index(0);
        matcher.begin_replacement();
        matcher.update(&Rope::from("bar foo bar"));
        assert_eq!(matcher.current_match_index(), 0);
        assert_eq!(matcher.matched_ranges()[0], 4..7);
        assert_eq!(matcher.label(), "1/1");
    }

    #[test]
    fn update_matches_clamps_current_match_index_while_replacing() {
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from("foo foo foo"));
        matcher.update_query("foo", true);
        matcher.set_current_match_index(2);
        matcher.begin_replacement();

        matcher.update(&Rope::from("foo xoo foo"));

        assert_eq!(matcher.len(), 2);
        assert_eq!(matcher.current_match_index(), 1);
        assert_eq!(matcher.label(), "2/2");
    }
}
