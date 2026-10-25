//! The editor's git layer, worked out on texts (the Swift app does it in
//! `web/editor.js`): the hunks between a file's git base (its index
//! version) and the live buffer, which the gutter marks and the peek show;
//! the edit that reverts one; merge conflict blocks and the edit that
//! resolves one.

use std::ops::Range;

/// One changed stretch of the buffer against the base. Lines are 1-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineHunk {
    pub old_start: usize,
    /// The base's lines it replaced (none for an addition).
    pub old_lines: Vec<String>,
    pub new_start: usize,
    /// How many buffer lines it covers (none for a deletion).
    pub new_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Added,
    Modified,
    /// Lines removed: marked on the line before them.
    Deleted,
}

impl LineHunk {
    pub fn kind(&self) -> MarkKind {
        if self.new_count == 0 {
            MarkKind::Deleted
        } else if self.old_lines.is_empty() {
            MarkKind::Added
        } else {
            MarkKind::Modified
        }
    }

    /// The buffer line its mark is on.
    pub fn anchor_line(&self) -> usize {
        if self.new_count == 0 {
            self.new_start.saturating_sub(1).max(1)
        } else {
            self.new_start
        }
    }

    /// The buffer lines it marks (a deletion marks its anchor line).
    pub fn marked_lines(&self) -> Range<usize> {
        if self.new_count == 0 {
            let line = self.anchor_line();
            line..line + 1
        } else {
            self.new_start..self.new_start + self.new_count
        }
    }

    /// The peek's title: "3 lines removed", "1 line added", "Changed 2
    /// lines (was 1)".
    pub fn title(&self) -> String {
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        let removed = self.old_lines.len();
        if self.new_count == 0 {
            format!("{removed} line{} removed", plural(removed))
        } else if removed == 0 {
            format!("{} line{} added", self.new_count, plural(self.new_count))
        } else {
            format!(
                "Changed {} line{} (was {removed})",
                self.new_count,
                plural(self.new_count)
            )
        }
    }
}

/// A text's lines as the editor diffs them: a final line break doesn't
/// start another line, and `\r\n` counts as one break.
pub fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// The hunk on buffer line `line`, if one marks it.
pub fn hunk_at_line(hunks: &[LineHunk], line: usize) -> Option<&LineHunk> {
    hunks.iter().find(|h| h.marked_lines().contains(&line))
}

/// The hunks between `base` and `current`: the common start and end are
/// skipped, the middle diffed with Myers (up to 1500 edits; past that the
/// whole middle is one hunk).
pub fn line_hunks(base: &str, current: &str) -> Vec<LineHunk> {
    let a = split_lines(base);
    let b = split_lines(current);
    let mut start = 0;
    while start < a.len() && start < b.len() && a[start] == b[start] {
        start += 1;
    }
    let (mut end_a, mut end_b) = (a.len(), b.len());
    while end_a > start && end_b > start && a[end_a - 1] == b[end_b - 1] {
        end_a -= 1;
        end_b -= 1;
    }
    let (mid_a, mid_b) = (&a[start..end_a], &b[start..end_b]);
    if mid_a.is_empty() && mid_b.is_empty() {
        return Vec::new();
    }
    let Some(ops) = myers(mid_a, mid_b) else {
        return vec![LineHunk {
            old_start: start + 1,
            old_lines: mid_a.iter().map(|l| l.to_string()).collect(),
            new_start: start + 1,
            new_count: mid_b.len(),
        }];
    };
    let mut hunks = Vec::new();
    let mut current: Option<LineHunk> = None;
    let (mut ai, mut bi) = (0, 0);
    for op in ops {
        if op == Op::Same {
            hunks.extend(current.take());
            ai += 1;
            bi += 1;
            continue;
        }
        let hunk = current.get_or_insert_with(|| LineHunk {
            old_start: start + ai + 1,
            old_lines: Vec::new(),
            new_start: start + bi + 1,
            new_count: 0,
        });
        if op == Op::Removed {
            hunk.old_lines.push(mid_a[ai].to_string());
            ai += 1;
        } else {
            hunk.new_count += 1;
            bi += 1;
        }
    }
    hunks.extend(current);
    hunks
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Same,
    Removed,
    Added,
}

/// Myers' shortest edit script, `None` past 1500 edits.
fn myers(a: &[&str], b: &[&str]) -> Option<Vec<Op>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = n + m;
    let limit = max.min(1500);
    let offset = max + 1;
    let mut v = vec![0isize; (2 * max + 3) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    for d in 0..=limit {
        trace.push(v[(offset - d - 1) as usize..(offset + d + 2) as usize].to_vec());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d
                || (k != d && v[(offset + k - 1) as usize] < v[(offset + k + 1) as usize])
            {
                v[(offset + k + 1) as usize]
            } else {
                v[(offset + k - 1) as usize] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[(offset + k) as usize] = x;
            if x >= n && y >= m {
                return Some(backtrack(&trace, n, m, d));
            }
            k += 2;
        }
    }
    None
}

fn backtrack(trace: &[Vec<isize>], n: isize, m: isize, final_d: isize) -> Vec<Op> {
    let (mut x, mut y) = (n, m);
    let mut ops = Vec::new();
    for d in (1..=final_d).rev() {
        let band = &trace[d as usize];
        let at = |k: isize| band[(k + d + 1) as usize];
        let k = x - y;
        let previous_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let previous_x = at(previous_k);
        let previous_y = previous_x - previous_k;
        while x > previous_x && y > previous_y {
            ops.push(Op::Same);
            x -= 1;
            y -= 1;
        }
        if x == previous_x {
            ops.push(Op::Added);
            y -= 1;
        } else {
            ops.push(Op::Removed);
            x -= 1;
        }
    }
    while x > 0 && y > 0 {
        ops.push(Op::Same);
        x -= 1;
        y -= 1;
    }
    ops.reverse();
    ops
}

/// The byte offset where 1-based `line` starts in `text` (the end for a
/// line past the last).
fn line_start(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line - 2)
        .map_or(text.len(), |(i, _)| i + 1)
}

/// Where 1-based `line` ends in `text`, before its line break.
fn line_end(text: &str, line: usize) -> usize {
    let start = line_start(text, line);
    text[start..].find('\n').map_or(text.len(), |i| start + i)
}

/// The edit that puts `hunk`'s base lines back in `text` (`\n` line
/// breaks): a byte range and its replacement.
pub fn revert_edit(text: &str, hunk: &LineHunk) -> (Range<usize>, String) {
    let line_count = split_lines(text).len().max(1);
    let old = hunk.old_lines.join("\n");
    if hunk.new_count == 0 {
        // Put the removed lines back before new_start.
        if hunk.new_start > line_count {
            let end = text.trim_end_matches('\n').len();
            return (end..end, format!("\n{old}"));
        }
        let at = line_start(text, hunk.new_start);
        return (at..at, format!("{old}\n"));
    }
    let (first, last) = (hunk.new_start, hunk.new_start + hunk.new_count - 1);
    if hunk.old_lines.is_empty() {
        // Remove the added lines, line breaks included.
        let range = if last < line_count {
            line_start(text, first)..line_start(text, last + 1)
        } else if first > 1 {
            line_end(text, first - 1)..line_end(text, last)
        } else {
            line_start(text, first)..line_end(text, last)
        };
        return (range, String::new());
    }
    (line_start(text, first)..line_end(text, last), old)
}

/// A merge conflict in the buffer. Lines are 1-based: `start` is the
/// `<<<<<<<` line, `base` the `|||||||` one (diff3) if any, `middle` the
/// `=======` one, `end` the `>>>>>>>` one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictBlock {
    pub start: usize,
    pub base: Option<usize>,
    pub middle: usize,
    pub end: usize,
    pub current_label: String,
    pub incoming_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Current,
    Incoming,
    Both,
}

/// Every whole conflict block in `text`.
pub fn find_conflicts(text: &str) -> Vec<ConflictBlock> {
    if !text.contains("<<<<<<<") {
        return Vec::new();
    }
    let lines = split_lines(text);
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("<<<<<<<") {
            i += 1;
            continue;
        }
        let (mut base, mut middle, mut end) = (None, None, None);
        for (j, line) in lines.iter().enumerate().skip(i + 1) {
            if line.starts_with("<<<<<<<") {
                break;
            }
            if line.starts_with("|||||||") && middle.is_none() && base.is_none() {
                base = Some(j);
            } else if line.starts_with("=======") && middle.is_none() {
                middle = Some(j);
            } else if line.starts_with(">>>>>>>") && middle.is_some() {
                end = Some(j);
                break;
            }
        }
        let (Some(middle), Some(end)) = (middle, end) else {
            i += 1;
            continue;
        };
        let label = |line: &str, fallback: &str| {
            let label = line.get(7..).unwrap_or("").trim();
            if label.is_empty() {
                fallback.to_string()
            } else {
                label.to_string()
            }
        };
        blocks.push(ConflictBlock {
            start: i + 1,
            base: base.map(|b| b + 1),
            middle: middle + 1,
            end: end + 1,
            current_label: label(lines[i], "current"),
            incoming_label: label(lines[end], "incoming"),
        });
        i = end + 1;
    }
    blocks
}

/// The edit that resolves `block` in `text` (`\n` line breaks) by keeping
/// one side or both, current first.
pub fn resolve_edit(
    text: &str,
    block: &ConflictBlock,
    choice: Resolution,
) -> (Range<usize>, String) {
    let lines = split_lines(text);
    let current_end = block.base.unwrap_or(block.middle) - 1;
    let current = &lines[block.start..current_end];
    let incoming = &lines[block.middle..block.end - 1];
    let kept: Vec<&str> = match choice {
        Resolution::Current => current.to_vec(),
        Resolution::Incoming => incoming.to_vec(),
        Resolution::Both => current.iter().chain(incoming).copied().collect(),
    };
    let start = line_start(text, block.start);
    if block.end < lines.len() {
        let replacement = if kept.is_empty() {
            String::new()
        } else {
            format!("{}\n", kept.join("\n"))
        };
        (start..line_start(text, block.end + 1), replacement)
    } else {
        (start..line_end(text, block.end), kept.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, (range, replacement): (Range<usize>, String)) -> String {
        format!(
            "{}{}{}",
            &text[..range.start],
            replacement,
            &text[range.end..]
        )
    }

    #[test]
    fn hunks_mark_added_changed_and_removed_lines() {
        let base = "a\nb\nc\nd\ne\n";
        let current = "a\nB\nc\ne\nf\ng\n";
        let hunks = line_hunks(base, current);
        assert_eq!(hunks.len(), 3);
        assert_eq!(
            (hunks[0].kind(), hunks[0].new_start, hunks[0].new_count),
            (MarkKind::Modified, 2, 1)
        );
        assert_eq!(hunks[0].old_lines, vec!["b"]);
        assert_eq!(
            (hunks[1].kind(), hunks[1].anchor_line()),
            (MarkKind::Deleted, 3)
        );
        assert_eq!(hunks[1].old_lines, vec!["d"]);
        assert_eq!(
            (hunks[2].kind(), hunks[2].new_start, hunks[2].new_count),
            (MarkKind::Added, 5, 2)
        );
        assert_eq!(hunks[0].title(), "Changed 1 line (was 1)");
        assert_eq!(hunks[1].title(), "1 line removed");
        assert_eq!(hunks[2].title(), "2 lines added");
        assert!(line_hunks(base, base).is_empty());
        assert_eq!(hunk_at_line(&hunks, 6).map(|h| h.new_start), Some(5));
        assert!(hunk_at_line(&hunks, 1).is_none());
        // An untracked file: every line is new.
        let all = line_hunks("", "x\ny\n");
        assert_eq!((all[0].kind(), all[0].new_count), (MarkKind::Added, 2));
    }

    #[test]
    fn reverting_a_hunk_restores_the_base() {
        let base = "a\nb\nc\nd\ne\n";
        let current = "a\nB\nc\ne\nf\ng\n";
        for hunk in line_hunks(base, current) {
            let reverted = apply(current, revert_edit(current, &hunk));
            // Only that hunk goes back.
            assert_eq!(line_hunks(base, &reverted).len(), 2, "{reverted:?}");
        }
        let mut text = current.to_string();
        while let Some(hunk) = line_hunks(base, &text).into_iter().next() {
            text = apply(&text, revert_edit(&text, &hunk));
        }
        assert_eq!(text, base);
        // Lines removed at the very end, and added at the very start.
        let removed_last = line_hunks("a\nb\n", "a\n");
        assert_eq!(apply("a\n", revert_edit("a\n", &removed_last[0])), "a\nb\n");
        let added_first = line_hunks("b\n", "a\nb\n");
        assert_eq!(
            apply("a\nb\n", revert_edit("a\nb\n", &added_first[0])),
            "b\n"
        );
    }

    #[test]
    fn conflicts_are_found_and_resolved() {
        let text = "top\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feature\nmiddle\n<<<<<<< HEAD\na\n||||||| base\nb\n=======\nc\n>>>>>>> other\n";
        let blocks = find_conflicts(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            (
                blocks[0].start,
                blocks[0].middle,
                blocks[0].end,
                blocks[0].base
            ),
            (2, 4, 6, None)
        );
        assert_eq!(
            (
                blocks[0].current_label.as_str(),
                blocks[0].incoming_label.as_str()
            ),
            ("HEAD", "feature")
        );
        assert_eq!(blocks[1].base, Some(10));
        let current = apply(text, resolve_edit(text, &blocks[0], Resolution::Current));
        assert!(current.starts_with("top\nours\nmiddle\n"), "{current:?}");
        let incoming = apply(text, resolve_edit(text, &blocks[0], Resolution::Incoming));
        assert!(
            incoming.starts_with("top\ntheirs\nmiddle\n"),
            "{incoming:?}"
        );
        let both = apply(text, resolve_edit(text, &blocks[0], Resolution::Both));
        assert!(both.starts_with("top\nours\ntheirs\nmiddle\n"), "{both:?}");
        // The last block, at the end of the file: diff3's base is dropped.
        let last = apply(text, resolve_edit(text, &blocks[1], Resolution::Both));
        assert!(last.ends_with("middle\na\nc\n"), "{last:?}");
        assert!(find_conflicts("<<<<<<< unclosed\nx\n").is_empty());
    }
}
