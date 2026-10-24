//! Visual Block mode (Ctrl-V): a rectangle of columns over a run of lines.
//! Its operators work line by line; `I` and `A` type on the first line and,
//! at Escape, put the same text on the others.

use std::ops::Range;

use super::text::*;
use super::{line_ending, Context, Mode, Op, Register, Vim, Work};

/// Insert mode started by `I` or `A` on a block: what Escape copies the
/// typed text to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BlockInsert {
    /// The block's other lines.
    pub lines: Range<usize>,
    /// The column typed at, in characters.
    pub column: usize,
    /// `A`: shorter lines are padded out to the column; `$A` types at each
    /// line's end.
    pub append: bool,
    pub to_end: bool,
}

impl Vim {
    /// The block's lines, its first column and the column after its last
    /// (`None`: to each line's end, after `$`).
    pub(super) fn block_bounds(&self, t: &str) -> (Range<usize>, usize, Option<usize>) {
        let len = t.len();
        let (a, h) = (self.anchor.min(len), self.head.min(len));
        let (la, lh) = (line_of(t, a), line_of(t, h));
        let (ca, ch) = (column(t, a), column(t, h));
        let lines = la.min(lh)..la.max(lh) + 1;
        let left = ca.min(ch);
        let right = if self.want_column == Some(usize::MAX) {
            None
        } else {
            Some(ca.max(ch) + 1)
        };
        (lines, left, right)
    }

    /// Each line's part of the block (empty where a line is too short), for
    /// the editor to draw.
    pub fn block_ranges(&self, t: &str) -> Vec<Range<usize>> {
        if self.mode != Mode::VisualBlock {
            return Vec::new();
        }
        let (lines, left, right) = self.block_bounds(t);
        lines
            .map(|line| {
                let start = start_of_line(t, line);
                let from = at_column(t, start, left);
                let to = match right {
                    Some(right) => at_column(t, start, right),
                    None => line_end(t, start),
                };
                from..to.max(from)
            })
            .collect()
    }

    /// Leave the block for Normal mode, the cursor at its top left.
    fn leave_block(&mut self, w: &mut Work) {
        let (lines, left, _) = self.block_bounds(&w.text);
        self.end_visual();
        self.mode = Mode::Normal;
        self.want_column = None;
        w.cursor = at_column(&w.text, start_of_line(&w.text, lines.start), left);
    }

    /// An operator on the block. `whole` (`D`, `C`, `X`, `Y`, `S`, `R`):
    /// each line to its end.
    pub(super) fn block_operate(
        &mut self,
        op: Op,
        whole: bool,
        register: Option<char>,
        w: &mut Work,
        cx: &mut Context,
    ) {
        if whole {
            self.want_column = Some(usize::MAX);
        }
        let ranges = self.block_ranges(&w.text);
        let (lines, left, right) = self.block_bounds(&w.text);
        match op {
            Op::Delete | Op::Yank | Op::Change => {
                let pieces: Vec<String> = ranges
                    .iter()
                    .map(|r| w.text[r.clone()].to_string())
                    .collect();
                self.store_block(register, pieces.join("\n"), op == Op::Yank, w);
                if op == Op::Yank {
                    self.leave_block(w);
                    return;
                }
                for range in ranges.iter().rev() {
                    w.text.replace_range(range.clone(), "");
                }
                self.leave_block(w);
                if op == Op::Change {
                    let line_count = lines.len();
                    self.start_insert(w, 1, None);
                    if let Some(session) = self.insert.as_mut() {
                        session.block = Some(BlockInsert {
                            lines: lines.start + 1..lines.start + line_count,
                            column: left,
                            append: false,
                            to_end: right.is_none(),
                        });
                    }
                }
            }
            Op::Lower | Op::Upper | Op::ToggleCase => {
                for range in ranges.iter().rev() {
                    let piece = &w.text[range.clone()];
                    let changed: String = piece
                        .chars()
                        .map(|c| match op {
                            Op::Lower => c.to_lowercase().collect::<String>(),
                            Op::Upper => c.to_uppercase().collect(),
                            _ if c.is_uppercase() => c.to_lowercase().collect(),
                            _ => c.to_uppercase().collect(),
                        })
                        .collect();
                    w.text.replace_range(range.clone(), &changed);
                }
                self.leave_block(w);
            }
            Op::Indent | Op::Outdent => {
                // Whole lines, as Visual Line mode shifts them.
                let start = start_of_line(&w.text, lines.start);
                let last = start_of_line(&w.text, lines.end - 1);
                let span = Span {
                    start,
                    end: next_line_start(&w.text, last).unwrap_or(w.text.len()),
                    linewise: true,
                };
                self.end_visual();
                self.mode = Mode::Normal;
                self.want_column = None;
                w.cursor = start;
                self.operate(op, span, register, 1, w, cx);
            }
        }
    }

    /// `r` on the block: each of its characters becomes `c`.
    pub(super) fn block_replace(&mut self, c: char, w: &mut Work) {
        for range in self.block_ranges(&w.text).into_iter().rev() {
            let count = w.text[range.clone()].chars().count();
            let replacement: String = std::iter::repeat_n(c, count).collect();
            w.text.replace_range(range, &replacement);
        }
        self.leave_block(w);
    }

    /// `I` (before the block) or `A` (after it): type on the first line.
    pub(super) fn block_insert(&mut self, append: bool, w: &mut Work) {
        let (lines, left, right) = self.block_bounds(&w.text);
        let column = if append { right.unwrap_or(0) } else { left };
        let to_end = append && right.is_none();
        self.end_visual();
        self.mode = Mode::Normal;
        self.want_column = None;
        let start = start_of_line(&w.text, lines.start);
        w.cursor = if to_end {
            line_end(&w.text, start)
        } else {
            let at = at_column(&w.text, start, column);
            // `A` pads a short first line out to the block's edge.
            let short = column.saturating_sub(w.text[start..at].chars().count());
            if append && short > 0 {
                w.text.insert_str(at, &" ".repeat(short));
                at + short
            } else {
                at
            }
        };
        self.start_insert(w, 1, None);
        if let Some(session) = self.insert.as_mut() {
            session.block = Some(BlockInsert {
                lines: lines.start + 1..lines.end,
                column,
                append,
                to_end,
            });
        }
    }

    /// `O`: the other corner on the same line.
    pub(super) fn swap_block_columns(&mut self, t: &str) {
        let (ca, ch) = (column(t, self.anchor), column(t, self.head));
        self.anchor = at_column(t, line_start(t, self.anchor), ch);
        self.head = at_column(t, line_start(t, self.head), ca);
    }

    /// Escape after `I`, `A` or `c` on a block: what was typed on the first
    /// line goes on the others too (when it's one line of plain typing).
    pub(super) fn finish_block_insert(&self, block: &BlockInsert, typed: &str, w: &mut Work) {
        if typed.is_empty() || typed.contains('\n') {
            return;
        }
        for line in block.lines.clone() {
            if line >= line_count(&w.text) {
                break;
            }
            let start = start_of_line(&w.text, line);
            let end = line_end(&w.text, start);
            let length = w.text[start..end].chars().count();
            let at = if block.to_end {
                end
            } else if length < block.column {
                if !block.append {
                    // `I` and `c` leave lines that end before the block alone.
                    continue;
                }
                w.text.insert_str(end, &" ".repeat(block.column - length));
                line_end(&w.text, start)
            } else {
                at_column(&w.text, start, block.column)
            };
            w.text.insert_str(at, typed);
        }
    }

    /// A yanked or deleted block, line by line.
    fn store_block(&mut self, register: Option<char>, text: String, yank: bool, w: &mut Work) {
        self.store(register, text, false, yank, w);
        for name in [register.unwrap_or('"'), '"', '0', '-'] {
            if let Some(reg) = self.registers.get_mut(&name) {
                reg.block = true;
            }
        }
    }

    /// `p` or `P` of a block: its lines at the cursor's column on the lines
    /// from the cursor's down, padding short lines and adding lines at the
    /// end as needed.
    pub(super) fn put_block(&mut self, reg: &Register, before: bool, w: &mut Work) {
        let t = &w.text;
        let line = line_of(t, w.cursor);
        let mut col = column(t, w.cursor);
        if !before && w.cursor < line_end(t, w.cursor) {
            col += 1;
        }
        for (i, piece) in reg.text.split('\n').enumerate() {
            let target = line + i;
            while target >= line_count(&w.text) {
                let ending = line_ending(&w.text).to_string();
                w.text.push_str(&ending);
            }
            let start = start_of_line(&w.text, target);
            let end = line_end(&w.text, start);
            let length = w.text[start..end].chars().count();
            if length < col {
                w.text.insert_str(end, &" ".repeat(col - length));
            }
            let at = at_column(&w.text, start, col);
            w.text.insert_str(at, piece);
        }
        let start = start_of_line(&w.text, line);
        w.cursor = at_column(&w.text, start, col);
    }
}
