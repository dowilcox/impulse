//! Terminal regex search using alacritty_terminal's search engine.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::term::search::RegexSearch;
use alacritty_terminal::term::Term;
use serde::Serialize;

use crate::buffer::HighlightRange;

/// Result of a search operation, serialized as JSON for the FFI layer.
#[derive(Clone, Debug, Serialize)]
pub struct SearchResult {
    pub match_row: i32,
    pub match_start_col: i32,
    pub match_end_col: i32,
}

impl SearchResult {
    /// A "no match" sentinel value.
    pub fn no_match() -> Self {
        Self {
            match_row: -1,
            match_start_col: -1,
            match_end_col: -1,
        }
    }
}

/// How many matches a search has across the scrollback and which one is
/// current.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchStats {
    /// 1-based index of the current match among all matches (0: none yet).
    pub current: usize,
    pub total: usize,
    /// Counting stopped at the limit; there may be more.
    pub capped: bool,
    /// The pattern didn't compile.
    pub invalid: bool,
}

/// Wraps alacritty_terminal's RegexSearch to maintain search state across calls.
pub(crate) struct TerminalSearch {
    regex: Option<RegexSearch>,
    /// Last match position, used as the origin for next/prev navigation.
    last_match: Option<Point>,
    /// The pattern last given didn't compile.
    invalid: bool,
    /// The last search pattern, so we can avoid recompiling when the same
    /// pattern is provided again.
    last_pattern: String,
}

impl TerminalSearch {
    pub fn new() -> Self {
        Self {
            regex: None,
            last_match: None,
            invalid: false,
            last_pattern: String::new(),
        }
    }

    /// Compile a regex pattern and find the first match, searching forward
    /// from the top of the viewport.
    pub fn search<T>(&mut self, term: &Term<T>, pattern: &str) -> SearchResult {
        if pattern.is_empty() {
            self.clear();
            return SearchResult::no_match();
        }

        // Recompile only if the pattern changed.
        if pattern != self.last_pattern || self.regex.is_none() {
            match RegexSearch::new(pattern) {
                Ok(regex) => {
                    self.regex = Some(regex);
                    self.last_pattern = pattern.to_string();
                    self.last_match = None;
                    self.invalid = false;
                }
                Err(_) => {
                    self.clear();
                    self.invalid = true;
                    return SearchResult::no_match();
                }
            }
        }

        // Search forward from the top-left of the visible viewport. Alacritty
        // represents scrollback rows as negative line numbers when the display
        // is scrolled away from the bottom.
        let (origin, _, _) = Self::viewport_bounds(term);
        self.find(term, origin, Direction::Right)
    }

    /// Find the next match after the current one.
    pub fn search_next<T>(&mut self, term: &Term<T>) -> SearchResult {
        let regex = match self.regex.as_mut() {
            Some(r) => r,
            None => return SearchResult::no_match(),
        };

        let origin = match self.last_match {
            Some(pt) => {
                // Advance past the current match so we don't find it again.
                pt.add(term, alacritty_terminal::index::Boundary::None, 1)
            }
            None => {
                let (origin, _, _) = Self::viewport_bounds(term);
                origin
            }
        };

        match term.search_next(regex, origin, Direction::Right, Side::Left, None) {
            Some(m) => {
                let start = *m.start();
                let end = *m.end();
                self.last_match = Some(start);
                Self::result_from_match(term, start, end)
            }
            None => SearchResult::no_match(),
        }
    }

    /// Find the previous match before the current one.
    pub fn search_prev<T>(&mut self, term: &Term<T>) -> SearchResult {
        let regex = match self.regex.as_mut() {
            Some(r) => r,
            None => return SearchResult::no_match(),
        };

        let origin = match self.last_match {
            Some(pt) => {
                // Move back one cell so we don't find the current match again.
                pt.sub(term, alacritty_terminal::index::Boundary::None, 1)
            }
            None => {
                let (_, end, _) = Self::viewport_bounds(term);
                end
            }
        };

        match term.search_next(regex, origin, Direction::Left, Side::Left, None) {
            Some(m) => {
                let start = *m.start();
                let end = *m.end();
                self.last_match = Some(start);
                Self::result_from_match(term, start, end)
            }
            None => SearchResult::no_match(),
        }
    }

    /// Start of the current match, if any.
    pub fn current_match(&self) -> Option<Point> {
        self.last_match
    }

    /// Count matches across the whole scrollback (up to `limit`) and find
    /// the current one's position among them.
    pub fn stats<T>(&mut self, term: &Term<T>, limit: usize) -> SearchStats {
        let mut stats = SearchStats {
            invalid: self.invalid,
            ..Default::default()
        };
        let last_match = self.last_match;
        let Some(regex) = self.regex.as_mut() else {
            return stats;
        };
        let top = Point::new(Line(-(term.grid().history_size() as i32)), Column(0));
        let end = Point::new(
            Line(term.screen_lines() as i32 - 1),
            Column(term.columns().saturating_sub(1)),
        );
        let mut cursor = top;
        while let Some(m) = term.regex_search_right(regex, cursor, end) {
            stats.total += 1;
            if Some(*m.start()) == last_match {
                stats.current = stats.total;
            }
            if stats.total >= limit {
                stats.capped = true;
                break;
            }
            let next = m
                .end()
                .add(term, alacritty_terminal::index::Boundary::Grid, 1);
            // Stop at the end of the grid (add can't move past it) or past it.
            if next <= *m.end() || next > end {
                break;
            }
            cursor = next;
        }
        stats
    }

    /// Clear all search state.
    pub fn clear(&mut self) {
        self.regex = None;
        self.last_match = None;
        self.invalid = false;
        self.last_pattern.clear();
    }

    /// Return all match ranges visible in the current viewport for the grid
    /// snapshot buffer. These ranges drive the amber highlight rendering.
    pub fn visible_matches<T>(&mut self, term: &Term<T>) -> Vec<HighlightRange> {
        let regex = match self.regex.as_mut() {
            Some(r) => r,
            None => return Vec::new(),
        };

        let num_lines = term.screen_lines();
        let num_cols = term.columns();
        let mut ranges = Vec::new();

        // Search through the entire visible viewport.
        let (start, end, display_offset) = Self::viewport_bounds(term);

        // Use regex_search_right to iterate through all matches in the viewport.
        let mut cursor = start;

        // The snapshot header counts ranges in a u16.
        while ranges.len() < u16::MAX as usize {
            match term.regex_search_right(regex, cursor, end) {
                Some(m) => {
                    let m_start = *m.start();
                    let m_end = *m.end();

                    // Only include matches whose lines are in [0, num_lines).
                    let viewport_start_row = m_start.line.0 + display_offset;
                    if viewport_start_row >= 0 && (viewport_start_row as usize) < num_lines {
                        // A match may span multiple lines; emit one range per line.
                        let start_row = (m_start.line.0 + display_offset).max(0) as usize;
                        let end_row =
                            ((m_end.line.0 + display_offset).max(0) as usize).min(num_lines - 1);
                        for row in start_row..=end_row {
                            let grid_line = row as i32 - display_offset;
                            let sc = if grid_line == m_start.line.0 {
                                m_start.column.0
                            } else {
                                0
                            };
                            let ec = if grid_line == m_end.line.0 {
                                m_end.column.0
                            } else {
                                num_cols - 1
                            };
                            ranges.push(HighlightRange {
                                row: row as u16,
                                start_col: sc as u16,
                                end_col: ec as u16,
                            });
                        }
                    }

                    // Advance past this match. At the grid's last cell `add`
                    // can't move forward (it clamps or wraps), which would find
                    // the same match again.
                    let next = m_end.add(term, alacritty_terminal::index::Boundary::Grid, 1);
                    if next <= m_end || next > end {
                        break;
                    }
                    cursor = next;
                }
                None => break,
            }
        }

        ranges.truncate(u16::MAX as usize);
        ranges
    }

    /// Internal helper: perform a search from the given origin in the given
    /// direction, updating `last_match`.
    fn find<T>(&mut self, term: &Term<T>, origin: Point, direction: Direction) -> SearchResult {
        let regex = match self.regex.as_mut() {
            Some(r) => r,
            None => return SearchResult::no_match(),
        };

        match term.search_next(regex, origin, direction, Side::Left, None) {
            Some(m) => {
                let start = *m.start();
                let end = *m.end();
                self.last_match = Some(start);
                Self::result_from_match(term, start, end)
            }
            None => SearchResult::no_match(),
        }
    }

    fn viewport_bounds<T>(term: &Term<T>) -> (Point, Point, i32) {
        let display_offset = term.grid().display_offset() as i32;
        let top_line = Line(-display_offset);
        let bottom_line = Line(term.screen_lines() as i32 - 1 - display_offset);
        let last_col = Column(term.columns().saturating_sub(1));
        (
            Point::new(top_line, Column(0)),
            Point::new(bottom_line, last_col),
            display_offset,
        )
    }

    pub(crate) fn result_from_match<T>(term: &Term<T>, start: Point, end: Point) -> SearchResult {
        let display_offset = term.grid().display_offset() as i32;
        SearchResult {
            match_row: start.line.0 + display_offset,
            match_start_col: start.column.0 as i32,
            match_end_col: end.column.0 as i32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    fn term_with(columns: usize, lines: usize, output: &[u8]) -> Term<VoidListener> {
        let size = TermSize::new(columns, lines);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let mut processor: Processor = Processor::new();
        processor.advance(&mut term, output);
        term
    }

    #[test]
    fn counts_matches_across_scrollback_and_tracks_the_current_one() {
        // Five lines on a three-line screen: two scroll into history.
        let term = term_with(20, 3, b"foo 1\r\nbar\r\nfoo 2\r\nfoo 3\r\nend");
        let mut search = TerminalSearch::new();
        search.search(&term, "foo");
        let stats = search.stats(&term, 100);
        assert_eq!(stats.total, 3);
        assert_eq!(
            stats.current, 2,
            "the first match from the viewport's top is foo 2"
        );
        search.search_prev(&term);
        assert_eq!(search.stats(&term, 100).current, 1);

        let capped = search.stats(&term, 2);
        assert!(capped.capped);
        assert_eq!(capped.total, 2);
    }

    /// The find bar's patterns (see ImpulseKit's TerminalFindQuery): explicit
    /// case flags and ASCII word boundaries.
    #[test]
    fn find_bar_patterns_compile_and_match() {
        let term = term_with(30, 3, b"Error error\r\nid idx kid\r\n");
        let mut search = TerminalSearch::new();
        let count = |search: &mut TerminalSearch, pattern: &str| {
            search.search(&term, pattern);
            let stats = search.stats(&term, 100);
            assert!(!stats.invalid, "{pattern} should compile");
            stats.total
        };
        assert_eq!(count(&mut search, "(?-i)Error"), 1);
        assert_eq!(count(&mut search, "(?i)Error"), 2);
        assert_eq!(count(&mut search, "(?i)(?-u:\\b)(?:id)(?-u:\\b)"), 1);
        assert_eq!(count(&mut search, "(?i)id"), 3);
    }

    #[test]
    fn a_match_in_the_last_cell_is_found_once() {
        // Fill the screen so the last match ends in the bottom-right cell.
        let term = term_with(4, 2, b"ab a\r\nb ab");
        let mut search = TerminalSearch::new();
        search.search(&term, "b");
        assert_eq!(search.visible_matches(&term).len(), 3);
        assert_eq!(search.stats(&term, 100).total, 3);
        search.search(&term, "ab");
        assert_eq!(search.visible_matches(&term).len(), 2);
        assert_eq!(search.stats(&term, 100).total, 2);
        // Matches everything: one range per cell at most.
        search.search(&term, ".");
        assert_eq!(search.visible_matches(&term).len(), 8);
    }

    #[test]
    fn invalid_patterns_are_reported() {
        let term = term_with(20, 3, b"text");
        let mut search = TerminalSearch::new();
        search.search(&term, "(unclosed");
        let stats = search.stats(&term, 100);
        assert!(stats.invalid);
        assert_eq!(stats.total, 0);
        search.search(&term, "text");
        assert!(!search.stats(&term, 100).invalid);
        assert_eq!(search.stats(&term, 100).total, 1);
    }
}
