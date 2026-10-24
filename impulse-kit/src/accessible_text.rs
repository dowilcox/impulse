//! Line and range math for exposing screen text to a screen reader as one
//! string of lines joined by `\n` (the Swift `AccessibleText`). Indexes are
//! UTF-16 offsets and lines are 0-based, as NSAccessibility's text-area
//! attributes count them.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleText {
    pub lines: Vec<String>,
    pub string: String,
    /// UTF-16 offset where each line starts.
    starts: Vec<usize>,
}

impl AccessibleText {
    /// The screen's lines, without trailing spaces, and without trailing
    /// blank lines (they're just empty screen).
    pub fn new<S: AsRef<str>>(lines: &[S]) -> Self {
        let mut trimmed: Vec<String> = lines
            .iter()
            .map(|line| line.as_ref().trim_end_matches(' ').to_string())
            .collect();
        while trimmed.len() > 1 && trimmed.last().is_some_and(String::is_empty) {
            trimmed.pop();
        }
        let mut starts = Vec::with_capacity(trimmed.len());
        let mut offset = 0;
        for line in &trimmed {
            starts.push(offset);
            offset += line.encode_utf16().count() + 1;
        }
        AccessibleText {
            string: trimmed.join("\n"),
            lines: trimmed,
            starts,
        }
    }

    /// The length in UTF-16 code units.
    pub fn len(&self) -> usize {
        self.string.encode_utf16().count()
    }

    pub fn is_empty(&self) -> bool {
        self.string.is_empty()
    }

    /// The line containing UTF-16 index `index` (the last line past the
    /// end); a line's newline belongs to it.
    pub fn line_for(&self, index: usize) -> usize {
        self.starts
            .partition_point(|&start| start <= index)
            .saturating_sub(1)
    }

    /// A line's UTF-16 range, without its newline.
    pub fn range_for_line(&self, line: usize) -> Option<Range<usize>> {
        let start = *self.starts.get(line)?;
        Some(start..start + self.lines[line].encode_utf16().count())
    }

    /// The text in a UTF-16 range, `None` when it reaches past the end.
    pub fn substring(&self, range: Range<usize>) -> Option<String> {
        let units: Vec<u16> = self.string.encode_utf16().collect();
        let slice = units.get(range)?;
        Some(String::from_utf16_lossy(slice))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_trimmed_lines() {
        let text = AccessibleText::new(&["$ ls   ", "a.txt  b.txt", "", "  ", ""]);
        assert_eq!(text.lines, ["$ ls", "a.txt  b.txt"]);
        assert_eq!(text.string, "$ ls\na.txt  b.txt");
        assert_eq!(text.len(), 17);
    }

    #[test]
    fn lines_and_ranges() {
        let text = AccessibleText::new(&["one", "", "three"]);
        assert_eq!(text.line_for(0), 0);
        assert_eq!(text.line_for(3), 0, "the newline belongs to its line");
        assert_eq!(text.line_for(4), 1);
        assert_eq!(text.line_for(5), 2);
        assert_eq!(text.line_for(99), 2);
        assert_eq!(text.range_for_line(2), Some(5..10));
        assert_eq!(text.range_for_line(7), None);
        assert_eq!(text.substring(5..10).as_deref(), Some("three"));
        assert_eq!(text.substring(8..13), None);
    }

    #[test]
    fn counts_utf16() {
        let text = AccessibleText::new(&["🚀 go", "é"]);
        assert_eq!(text.len(), 7);
        assert_eq!(text.range_for_line(1), Some(6..7));
        assert_eq!(text.substring(0..2).as_deref(), Some("🚀"));
    }

    #[test]
    fn empty_screen() {
        let text = AccessibleText::new::<&str>(&[]);
        assert!(text.string.is_empty());
        assert_eq!(text.line_for(0), 0);
    }
}
