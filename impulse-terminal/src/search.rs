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

/// Wraps alacritty_terminal's RegexSearch to maintain search state across calls.
pub(crate) struct TerminalSearch {
    regex: Option<RegexSearch>,
    /// Last match position, used as the origin for next/prev navigation.
    last_match: Option<Point>,
    /// The last search pattern, so we can avoid recompiling when the same
    /// pattern is provided again.
    last_pattern: String,
}

impl TerminalSearch {
    pub fn new() -> Self {
        Self {
            regex: None,
            last_match: None,
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
                }
                Err(_) => {
                    self.clear();
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

    /// Clear all search state.
    pub fn clear(&mut self) {
        self.regex = None;
        self.last_match = None;
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
        // Safety limit to prevent infinite loops on pathological patterns.
        let max_matches = num_lines * num_cols;
        let mut count = 0;

        loop {
            if count >= max_matches {
                break;
            }

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

                    // Advance cursor past this match.
                    cursor = m_end.add(term, alacritty_terminal::index::Boundary::Grid, 1);

                    // If we've gone past the viewport, stop.
                    if cursor.line > end.line
                        || (cursor.line == end.line && cursor.column > end.column)
                    {
                        break;
                    }

                    count += 1;
                }
                None => break,
            }
        }

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

    fn result_from_match<T>(term: &Term<T>, start: Point, end: Point) -> SearchResult {
        let display_offset = term.grid().display_offset() as i32;
        SearchResult {
            match_row: start.line.0 + display_offset,
            match_start_col: start.column.0 as i32,
            match_end_col: end.column.0 as i32,
        }
    }
}
