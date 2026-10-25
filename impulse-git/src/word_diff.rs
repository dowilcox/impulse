//! Intra-line word diffs (the original Rust's `assign_word_spans` and
//! `word_spans`, which used `similar::TextDiff::from_words`): lines are split
//! into alternating runs of whitespace and non-whitespace, the runs are
//! diffed with Myers' algorithm, and the changed runs become UTF-16 ranges on
//! each side, adjacent ones merged. UTF-16, because the ranges index
//! JavaScript strings in the web renderer.

use crate::hunks::{DiffLine, DiffLineKind, WordSpan};
use crate::MAX_WORD_DIFF_LINE_LEN;

/// Word spans for paired removed and added lines within a hunk: a run of
/// removed lines directly followed by added lines is paired index for index.
pub fn assign_word_spans(lines: &mut [DiffLine]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != DiffLineKind::Removed {
            i += 1;
            continue;
        }
        let removed_start = i;
        while i < lines.len() && lines[i].kind == DiffLineKind::Removed {
            i += 1;
        }
        let removed_end = i;
        let added_start = i;
        while i < lines.len() && lines[i].kind == DiffLineKind::Added {
            i += 1;
        }
        let added_end = i;

        let pairs = (removed_end - removed_start).min(added_end - added_start);
        for k in 0..pairs {
            let old_index = removed_start + k;
            let new_index = added_start + k;
            let old = &lines[old_index].content;
            let new = &lines[new_index].content;
            if utf16_len(old) + utf16_len(new) > MAX_WORD_DIFF_LINE_LEN {
                continue;
            }
            let (old_spans, new_spans) = word_spans(old, new);
            lines[old_index].spans = old_spans;
            lines[new_index].spans = new_spans;
        }
    }
}

/// The changed UTF-16 ranges of two lines, old side then new side.
pub fn word_spans(old: &str, new: &str) -> (Vec<WordSpan>, Vec<WordSpan>) {
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    let (old_changed, new_changed) = changed_flags(&old_tokens, &new_tokens);
    (
        spans(&old_tokens, &old_changed),
        spans(&new_tokens, &new_changed),
    )
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Alternating runs of whitespace and non-whitespace characters (Unicode
/// `White_Space`, which `char::is_whitespace` is).
pub fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut current: Option<bool> = None;
    for (index, c) in text.char_indices() {
        let is_whitespace = c.is_whitespace();
        if current.is_some_and(|w| w != is_whitespace) {
            tokens.push(&text[start..index]);
            start = index;
        }
        current = Some(is_whitespace);
    }
    if current.is_some() {
        tokens.push(&text[start..]);
    }
    tokens
}

/// Changed tokens as UTF-16 spans; a span that starts where the previous
/// one ended extends it.
fn spans(tokens: &[&str], changed: &[bool]) -> Vec<WordSpan> {
    let mut result: Vec<WordSpan> = Vec::new();
    let mut offset = 0u32;
    for (token, &is_changed) in tokens.iter().zip(changed) {
        let length = utf16_len(token) as u32;
        if is_changed && length > 0 {
            match result.last_mut() {
                Some(last) if last.end == offset => last.end = offset + length,
                _ => result.push(WordSpan {
                    start: offset,
                    end: offset + length,
                }),
            }
        }
        offset += length;
    }
    result
}

/// Myers' diff (greedy O(ND), then backtracking) over two token sequences:
/// per token, whether it was deleted (old) or inserted (new).
pub fn changed_flags(old: &[&str], new: &[&str]) -> (Vec<bool>, Vec<bool>) {
    let n = old.len();
    let m = new.len();
    let mut old_changed = vec![false; n];
    let mut new_changed = vec![false; m];
    if n == 0 || m == 0 {
        old_changed.iter_mut().for_each(|c| *c = true);
        new_changed.iter_mut().for_each(|c| *c = true);
        return (old_changed, new_changed);
    }

    let max_d = (n + m) as isize;
    let offset = max_d;
    let at = |k: isize| (offset + k) as usize;
    let mut v = vec![0isize; 2 * (n + m) + 1];
    // Before step d only diagonals -(d-1)..=(d-1) have been written (the
    // rest are still 0), so each step keeps just that window: O(D²)
    // memory, not O(D·(n+m)).
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut end_d = 0;

    'outer: for d in 0..=max_d {
        let reach = (d - 1).max(0);
        trace.push(v[at(-reach)..=at(reach)].to_vec());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) {
                v[at(k + 1)]
            } else {
                v[at(k - 1)] + 1
            };
            let mut y = x - k;
            while (x as usize) < n
                && (y as usize) < m
                && y >= 0
                && old[x as usize] == new[y as usize]
            {
                x += 1;
                y += 1;
            }
            v[at(k)] = x;
            if x as usize >= n && y >= 0 && y as usize >= m {
                end_d = d;
                break 'outer;
            }
            k += 2;
        }
    }

    // Back from (n, m) to (0, 0), marking deletions and insertions.
    let mut x = n as isize;
    let mut y = m as isize;
    let mut d = end_d;
    while d > 0 {
        let window = &trace[d as usize];
        let reach = (d - 1).max(0);
        let previous = |k: isize| {
            if k.abs() <= reach {
                window[(k + reach) as usize]
            } else {
                0
            }
        };
        let k = x - y;
        let previous_k = if k == -d || (k != d && previous(k - 1) < previous(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let previous_x = previous(previous_k);
        let previous_y = previous_x - previous_k;
        // Back through the snake (equal tokens).
        while x > previous_x && y > previous_y {
            x -= 1;
            y -= 1;
        }
        if previous_k == k + 1 {
            // Down: new[previous_y] was inserted.
            y -= 1;
            new_changed[y as usize] = true;
        } else {
            // Right: old[previous_x] was deleted.
            x -= 1;
            old_changed[x as usize] = true;
        }
        d -= 1;
    }
    (old_changed, new_changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_cover_the_changed_words() {
        let (old, new) = word_spans(
            "let message = \"hello world\";",
            "let message = \"hello swift world\";",
        );
        assert!(old.is_empty());
        assert_eq!(new, [WordSpan { start: 21, end: 27 }]);
        // UTF-16 offsets: the emoji is two code units.
        let (old, new) = word_spans("a 😀 b", "a 😀 c");
        assert_eq!(old, [WordSpan { start: 5, end: 6 }]);
        assert_eq!(new, [WordSpan { start: 5, end: 6 }]);
        assert_eq!(tokenize("  a\tb "), ["  ", "a", "\t", "b", " "]);
    }
}
