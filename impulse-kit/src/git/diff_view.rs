//! Diff View (Monaco's diff editor in the Swift app): the file's index
//! version against the buffer. Side by side, the two editors scroll
//! together through `map_row`; inline, removed lines sit above the lines
//! that replaced them in one read-only document (`inline`). Both follow
//! the hunks of `editor_marks::line_hunks`, as the change marks do.

use std::ops::Range;

use super::editor_marks::LineHunk;

/// The row (0-based) on the other side matching `row`: from the buffer to
/// the base when `from_new`, else back. Inside a change it is the
/// matching line of the other side's lines, or where they would be.
pub fn map_row(hunks: &[LineHunk], row: usize, from_new: bool) -> usize {
    let mut delta: isize = 0;
    for hunk in hunks {
        let (old_start, old_len) = (hunk.old_start.saturating_sub(1), hunk.old_lines.len());
        let (new_start, new_len) = (hunk.new_start.saturating_sub(1), hunk.new_count);
        let (this_start, this_len, other_start, other_len) = if from_new {
            (new_start, new_len, old_start, old_len)
        } else {
            (old_start, old_len, new_start, new_len)
        };
        if row < this_start {
            break;
        }
        if row < this_start + this_len {
            return other_start + (row - this_start).min(other_len.saturating_sub(1));
        }
        delta += other_len as isize - this_len as isize;
    }
    (row as isize + delta).max(0) as usize
}

/// The bar's summary: "No changes", or "2 changes  +5 −3".
pub fn summary(hunks: &[LineHunk]) -> String {
    if hunks.is_empty() {
        return "No changes".to_string();
    }
    let added: usize = hunks.iter().map(|h| h.new_count).sum();
    let removed: usize = hunks.iter().map(|h| h.old_lines.len()).sum();
    let noun = if hunks.len() == 1 {
        "change"
    } else {
        "changes"
    };
    format!("{} {noun}  +{added} \u{2212}{removed}", hunks.len())
}

/// The buffer row (0-based) of the next change after `row` (or before it
/// when not `forward`), wrapping around as Monaco's `goToDiff` does.
pub fn next_change(hunks: &[LineHunk], row: usize, forward: bool) -> Option<usize> {
    let rows: Vec<usize> = hunks
        .iter()
        .map(|h| h.new_start.saturating_sub(1))
        .collect();
    if forward {
        rows.iter().find(|&&r| r > row).or(rows.first()).copied()
    } else {
        rows.iter()
            .rev()
            .find(|&&r| r < row)
            .or(rows.last())
            .copied()
    }
}

/// The inline layout: the buffer with each change's removed lines above
/// the lines that replaced them.
#[derive(Debug, Default, PartialEq)]
pub struct InlineDiff {
    pub text: String,
    /// Rows holding removed lines, and rows holding added ones.
    pub removed: Vec<Range<u32>>,
    pub added: Vec<Range<u32>>,
    /// For each row, the buffer row it shows (`None` for a removed line).
    pub buffer_rows: Vec<Option<usize>>,
}

pub fn inline(current: &str, hunks: &[LineHunk]) -> InlineDiff {
    let lines: Vec<&str> = current.split('\n').collect();
    let mut out = InlineDiff::default();
    let mut text: Vec<&str> = Vec::new();
    let mut hunks = hunks.iter().peekable();
    for row in 0..=lines.len() {
        while let Some(hunk) = hunks.next_if(|h| h.new_start.saturating_sub(1) <= row) {
            if !hunk.old_lines.is_empty() {
                let start = text.len() as u32;
                for line in &hunk.old_lines {
                    text.push(line);
                    out.buffer_rows.push(None);
                }
                out.removed.push(start..text.len() as u32);
            }
            if hunk.new_count > 0 {
                let start = text.len() as u32;
                out.added.push(start..start + hunk.new_count as u32);
            }
        }
        if let Some(line) = lines.get(row) {
            text.push(line);
            out.buffer_rows.push(Some(row));
        }
    }
    out.text = text.join("\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::editor_marks::line_hunks;

    const BASE: &str = "a\nb\nc\nd\ne\n";
    // b changed to B and B2, d removed, f added at the end.
    const NEW: &str = "a\nB\nB2\nc\ne\nf\n";

    #[test]
    fn rows_map_across_changes() {
        let hunks = line_hunks(BASE, NEW);
        // Before any change, unchanged.
        assert_eq!(map_row(&hunks, 0, true), 0);
        // Inside the change: B to b, B2 to b (the base has one line).
        assert_eq!(map_row(&hunks, 1, true), 1);
        assert_eq!(map_row(&hunks, 2, true), 1);
        // c moved down a line in the buffer.
        assert_eq!(map_row(&hunks, 3, true), 2);
        assert_eq!(map_row(&hunks, 2, false), 3);
        // e: after the deleted d.
        assert_eq!(map_row(&hunks, 4, true), 4);
        assert_eq!(map_row(&hunks, 4, false), 4);
        // The removed d maps to where it would be.
        assert_eq!(map_row(&hunks, 3, false), 4);
    }

    #[test]
    fn summary_and_navigation() {
        let hunks = line_hunks(BASE, NEW);
        assert_eq!(summary(&hunks), "3 changes  +3 \u{2212}2");
        assert_eq!(summary(&[]), "No changes");
        assert_eq!(next_change(&hunks, 0, true), Some(1));
        assert_eq!(next_change(&hunks, 1, true), Some(4));
        assert_eq!(next_change(&hunks, 5, true), Some(1), "wraps");
        assert_eq!(next_change(&hunks, 1, false), Some(5), "wraps back");
        assert_eq!(next_change(&[], 1, true), None);
    }

    #[test]
    fn inline_puts_removed_lines_above() {
        let hunks = line_hunks(BASE, NEW);
        let diff = inline(NEW, &hunks);
        assert_eq!(diff.text, "a\nb\nB\nB2\nc\nd\ne\nf\n");
        assert_eq!(diff.removed, vec![1..2, 5..6]);
        assert_eq!(diff.added, vec![2..4, 7..8]);
        assert_eq!(
            diff.buffer_rows,
            vec![
                Some(0),
                None,
                Some(1),
                Some(2),
                Some(3),
                None,
                Some(4),
                Some(5),
                Some(6)
            ]
        );
    }
}
