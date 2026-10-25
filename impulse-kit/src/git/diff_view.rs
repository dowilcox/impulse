//! Diff View (Monaco's diff editor in the Swift app): the file's index
//! version against the buffer. Side by side, the two editors scroll
//! together through `map_row`; inline, removed lines sit above the lines
//! that replaced them in one read-only document (`inline`). Both follow
//! the hunks of `editor_marks::line_hunks`, as the change marks do. Long
//! runs of unchanged lines in the inline document hide behind one row
//! (`hide_unchanged`, Monaco's `hideUnchangedRegions`).

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
    /// For each row, the buffer row it shows (`None` for a removed line
    /// or a hidden run's row).
    pub buffer_rows: Vec<Option<usize>>,
    /// Runs of unchanged lines shown as one row (`hide_unchanged`).
    pub hidden: Vec<HiddenLines>,
}

/// The unchanged lines one row of the inline document stands for.
#[derive(Debug, Clone, PartialEq)]
pub struct HiddenLines {
    /// The row standing in for them.
    pub row: u32,
    /// The buffer rows (0-based) it hides.
    pub buffer_rows: Range<usize>,
}

/// Monaco's `hideUnchangedRegions` as the Swift app sets it: three lines of
/// context stay beside each change, and a run hides only when at least
/// three lines would go.
pub const CONTEXT_LINES: usize = 3;
pub const MINIMUM_HIDDEN: usize = 3;

/// `diff` with each long run of unchanged lines replaced by one empty row
/// (`hidden`), except the runs overlapping `revealed` (buffer rows).
pub fn hide_unchanged(diff: InlineDiff, revealed: &[Range<usize>]) -> InlineDiff {
    let rows = diff.buffer_rows.len();
    let changed = |row: usize| {
        let row = row as u32;
        diff.removed
            .iter()
            .chain(&diff.added)
            .any(|r| r.contains(&row))
    };
    // The rows to hide, as ranges of the document's rows.
    let mut hide: Vec<Range<usize>> = Vec::new();
    let mut row = 0;
    while row < rows {
        if changed(row) {
            row += 1;
            continue;
        }
        let start = row;
        while row < rows && !changed(row) {
            row += 1;
        }
        let lead = if start == 0 { 0 } else { CONTEXT_LINES };
        let trail = if row == rows { 0 } else { CONTEXT_LINES };
        if row - start < lead + trail + MINIMUM_HIDDEN {
            continue;
        }
        let run = start + lead..row - trail;
        let (Some(Some(first)), Some(Some(last))) = (
            diff.buffer_rows.get(run.start),
            diff.buffer_rows.get(run.end - 1),
        ) else {
            continue;
        };
        let buffer = *first..*last + 1;
        if revealed
            .iter()
            .any(|r| r.start < buffer.end && buffer.start < r.end)
        {
            continue;
        }
        hide.push(run);
    }
    if hide.is_empty() {
        return diff;
    }
    let lines: Vec<&str> = diff.text.split('\n').collect();
    let mut out = InlineDiff::default();
    let mut text: Vec<&str> = Vec::new();
    // Each old row's new row (hidden rows map to their stand-in).
    let mut new_row = vec![0u32; rows + 1];
    let mut hide = hide.into_iter().peekable();
    let mut row = 0;
    while row < rows {
        if let Some(run) = hide.next_if(|r| r.start == row) {
            let stand_in = text.len() as u32;
            for r in run.clone() {
                new_row[r] = stand_in;
            }
            out.hidden.push(HiddenLines {
                row: stand_in,
                buffer_rows: diff.buffer_rows[run.start].unwrap_or(0)
                    ..diff.buffer_rows[run.end - 1].unwrap_or(0) + 1,
            });
            text.push("");
            out.buffer_rows.push(None);
            row = run.end;
            continue;
        }
        new_row[row] = text.len() as u32;
        text.push(lines.get(row).copied().unwrap_or(""));
        out.buffer_rows.push(diff.buffer_rows[row]);
        row += 1;
    }
    new_row[rows] = text.len() as u32;
    let remap = |ranges: &[Range<u32>]| -> Vec<Range<u32>> {
        ranges
            .iter()
            .map(|r| new_row[r.start as usize]..new_row[r.start as usize] + (r.end - r.start))
            .collect()
    };
    out.removed = remap(&diff.removed);
    out.added = remap(&diff.added);
    out.text = text.join("\n");
    out
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
    fn long_unchanged_runs_hide() {
        // Twenty lines; line 10 changed.
        let base: String = (1..=20).map(|n| format!("{n}\n")).collect();
        let new = base.replace("\n10\n", "\nten\n");
        let hunks = line_hunks(&base, &new);
        let diff = hide_unchanged(inline(&new, &hunks), &[]);
        // Lines 1-6 hide (no context needed at the top), 7-9 stay; the
        // change; 11-13 stay, and 14-20 with the final empty line hide.
        assert_eq!(
            diff.hidden,
            vec![
                HiddenLines {
                    row: 0,
                    buffer_rows: 0..6
                },
                HiddenLines {
                    row: 9,
                    buffer_rows: 13..21
                },
            ]
        );
        assert_eq!(diff.text, "\n7\n8\n9\n10\nten\n11\n12\n13\n");
        assert_eq!(diff.removed, vec![4..5]);
        assert_eq!(diff.added, vec![5..6]);
        assert_eq!(diff.buffer_rows[1], Some(6));
        assert_eq!(diff.buffer_rows[5], Some(9));
        assert_eq!(diff.buffer_rows[9], None);
        // A revealed run stays.
        let shown = hide_unchanged(inline(&new, &hunks), &[2..3]);
        assert_eq!(shown.hidden.len(), 1);
        assert_eq!(shown.hidden[0].buffer_rows, 13..21);
    }

    #[test]
    fn short_runs_stay() {
        let hunks = line_hunks(BASE, NEW);
        let diff = inline(NEW, &hunks);
        assert_eq!(hide_unchanged(inline(NEW, &hunks), &[]), diff);
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
