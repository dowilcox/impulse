//! Applying a server's `TextEdit`s (formatting, code actions, rename), as
//! the Swift `TextEditApplier` does: every range refers to the text before
//! any of them, inserts at one position keep their order, positions past
//! a line's or the text's end clamp to it, overlapping edits are refused,
//! and `\n`, `\r\n` and `\r` all end lines. Columns are characters (UTF-32)
//! or UTF-16 code units, whichever the server uses.

use lsp_types::{Position, TextDocumentContentChangeEvent, TextEdit};

/// Where each line starts, and where its content ends (before its break).
fn lines(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push((start, i));
                start = i + 1;
            }
            b'\r' => {
                out.push((start, i));
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push((start, text.len()));
    out
}

fn offset_in(text: &str, lines: &[(usize, usize)], position: Position, utf32: bool) -> usize {
    let Some(&(start, end)) = lines.get(position.line as usize) else {
        return text.len();
    };
    let mut units = 0u32;
    for (i, c) in text[start..end].char_indices() {
        if units >= position.character {
            return start + i;
        }
        units += if utf32 { 1 } else { c.len_utf16() as u32 };
    }
    end
}

/// The byte offset of `position` in `text`, clamped to the line's end and
/// to the text's end.
pub fn offset_of(text: &str, position: Position, utf32: bool) -> usize {
    offset_in(text, &lines(text), position, utf32)
}

/// `text` with `edits` applied, or `None` when two of them overlap.
pub fn apply_text_edits(text: &str, edits: &[TextEdit], utf32: bool) -> Option<String> {
    let lines = lines(text);
    let mut ranges: Vec<(usize, usize, usize)> = edits
        .iter()
        .enumerate()
        .map(|(index, edit)| {
            let start = offset_in(text, &lines, edit.range.start, utf32);
            let end = offset_in(text, &lines, edit.range.end, utf32).max(start);
            (start, end, index)
        })
        .collect();
    ranges.sort();
    if ranges.windows(2).any(|pair| pair[1].0 < pair[0].1) {
        return None;
    }
    let mut out = text.to_string();
    // Back to front so earlier offsets stay valid; inserts at one position
    // in reverse, so they end up in their original order.
    for &(start, end, index) in ranges.iter().rev() {
        out.replace_range(start..end, &edits[index].new_text);
    }
    Some(out)
}

/// The smallest byte range of `old` that, replaced by the returned text,
/// gives `new`: one edit, so the editor's undo takes all of it back and the
/// cursor stays where it was when it's outside the change.
pub fn changed_span<'a>(old: &str, new: &'a str) -> (std::ops::Range<usize>, &'a str) {
    let prefix = old
        .char_indices()
        .zip(new.chars())
        .find(|((_, a), b)| a != b)
        .map_or(old.len().min(new.len()), |((i, _), _)| i);
    let prefix = floor_boundary(new, prefix);
    let max_suffix = (old.len() - prefix).min(new.len() - prefix);
    let suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    let mut old_end = old.len() - suffix;
    let mut new_end = new.len() - suffix;
    // Keep both ends on character boundaries.
    while !old.is_char_boundary(old_end) || !new.is_char_boundary(new_end) {
        old_end += 1;
        new_end += 1;
    }
    (prefix..old_end, &new[prefix..new_end])
}

/// The LSP position of byte `offset` in `text` (`\n`, `\r\n` and `\r` end
/// lines; columns in characters or UTF-16 code units).
pub fn position_of(text: &str, offset: usize, utf32: bool) -> Position {
    let bytes = text.as_bytes();
    let (mut line, mut start, mut i) = (0u32, 0usize, 0usize);
    while i < offset {
        match bytes[i] {
            b'\n' => {
                line += 1;
                start = i + 1;
            }
            b'\r' => {
                if bytes.get(i + 1) == Some(&b'\n') && i + 1 < offset {
                    i += 1;
                }
                line += 1;
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let column = text[start..offset]
        .chars()
        .map(|c| if utf32 { 1 } else { c.len_utf16() as u32 })
        .sum();
    Position::new(line, column)
}

/// The change from `old` to `new` as one incremental `didChange` event:
/// the smallest range, widened so it never splits a `\r\n`.
pub fn content_change(old: &str, new: &str, utf32: bool) -> TextDocumentContentChangeEvent {
    let (range, _) = changed_span(old, new);
    let suffix = old.len() - range.end;
    let (mut start, mut old_end) = (range.start, range.end);
    if start > 0 && old.as_bytes()[start - 1] == b'\r' {
        start -= 1;
    }
    if old_end < old.len() && old.as_bytes()[old_end] == b'\n' && old_end > 0 {
        old_end += 1;
    }
    let new_end = new.len() - (old.len() - old_end).min(suffix);
    TextDocumentContentChangeEvent {
        range: Some(lsp_types::Range::new(
            position_of(old, start, utf32),
            position_of(old, old_end, utf32),
        )),
        range_length: None,
        text: new[start..new_end].to_string(),
    }
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Range;

    fn edit(sl: u32, sc: u32, el: u32, ec: u32, text: &str) -> TextEdit {
        TextEdit {
            range: Range::new(Position::new(sl, sc), Position::new(el, ec)),
            new_text: text.into(),
        }
    }

    // The Swift `WorkspaceEditTests` for `TextEditApplier`, in UTF-16.

    #[test]
    fn applies_against_the_original_text() {
        let text = "let a = 1\nlet b = a\nprint(a)\n";
        let edits = [
            edit(0, 4, 0, 5, "count"),
            edit(1, 8, 1, 9, "count"),
            edit(2, 6, 2, 7, "count"),
        ];
        let want = "let count = 1\nlet b = count\nprint(count)\n";
        assert_eq!(apply_text_edits(text, &edits, false).as_deref(), Some(want));
        let reversed: Vec<_> = edits.iter().rev().cloned().collect();
        assert_eq!(
            apply_text_edits(text, &reversed, false).as_deref(),
            Some(want)
        );
    }

    #[test]
    fn inserts_at_one_position_keep_their_order() {
        let edits = [
            edit(0, 0, 0, 0, "import A\n"),
            edit(0, 0, 0, 0, "import B\n"),
        ];
        assert_eq!(
            apply_text_edits("code\n", &edits, false).as_deref(),
            Some("import A\nimport B\ncode\n")
        );
    }

    #[test]
    fn columns_are_utf16_and_clamp() {
        // "é" is one UTF-16 unit, "😀" two.
        assert_eq!(
            apply_text_edits("😀éz", &[edit(0, 2, 0, 3, "X")], false).as_deref(),
            Some("😀Xz")
        );
        assert_eq!(
            apply_text_edits("ab\ncd", &[edit(0, 99, 0, 99, "!")], false).as_deref(),
            Some("ab!\ncd"),
            "past the line end"
        );
        assert_eq!(
            apply_text_edits("ab", &[edit(9, 0, 9, 0, "!")], false).as_deref(),
            Some("ab!"),
            "past the document end"
        );
        assert_eq!(
            apply_text_edits("ab\r\ncd", &[edit(0, 1, 1, 1, "")], false).as_deref(),
            Some("ad"),
            "CRLF line breaks"
        );
    }

    #[test]
    fn overlapping_edits_are_rejected() {
        assert_eq!(
            apply_text_edits(
                "abcdef",
                &[edit(0, 0, 0, 3, "x"), edit(0, 2, 0, 4, "y")],
                false
            ),
            None
        );
        assert_eq!(
            apply_text_edits(
                "abcdef",
                &[edit(0, 0, 0, 2, "x"), edit(0, 2, 0, 4, "y")],
                false
            )
            .as_deref(),
            Some("xyef")
        );
    }

    #[test]
    fn utf32_columns_are_characters() {
        assert_eq!(
            apply_text_edits("😀éz", &[edit(0, 1, 0, 2, "X")], true).as_deref(),
            Some("😀Xz")
        );
        assert_eq!(offset_of("a𝄞b\n", Position::new(0, 99), true), "a𝄞b".len());
    }

    #[test]
    fn formatting_edits() {
        let text = "fn main(){\nlet x=1;\n}\n";
        let edits = [
            edit(0, 9, 0, 9, " "),
            edit(1, 0, 1, 0, "    "),
            edit(1, 5, 1, 6, " = "),
        ];
        assert_eq!(
            apply_text_edits(text, &edits, true).as_deref(),
            Some("fn main() {\n    let x = 1;\n}\n")
        );
    }

    /// Applying the incremental change to the old text gives the new one.
    fn check_change(old: &str, new: &str) {
        let change = content_change(old, new, false);
        let edit = TextEdit {
            range: change.range.unwrap(),
            new_text: change.text,
        };
        assert_eq!(
            apply_text_edits(old, &[edit], false).as_deref(),
            Some(new),
            "{old:?} -> {new:?}"
        );
    }

    #[test]
    fn incremental_changes_rebuild_the_text() {
        check_change("", "a");
        check_change("abc", "abc");
        check_change("let x = 1;\nlet y = 2;\n", "let x = 1;\nlet yy = 2;\n");
        check_change("one\ntwo\nthree", "one\nthree");
        check_change("a\r\nb", "a\r\nxb");
        check_change("a\r\nb", "a\rb");
        check_change("a\rb", "a\r\nb");
        check_change("a\nb", "a\r\nb");
        check_change("😀 x", "😀 yx");
        check_change("é\r\n\r\nz", "é\r\nz");
        // Every pair of short texts over line breaks and wide characters.
        let parts = ["a", "\r", "\n", "é", "😀"];
        let mut texts = vec![String::new()];
        for _ in 0..3 {
            let longer: Vec<String> = texts
                .iter()
                .filter(|t| t.chars().count() == texts.last().unwrap().chars().count())
                .flat_map(|t| parts.iter().map(move |p| format!("{t}{p}")))
                .collect();
            texts.extend(longer);
        }
        for old in &texts {
            for new in &texts {
                check_change(old, new);
            }
        }
        assert_eq!(position_of("a\r\nb😀c", 8, false), Position::new(1, 3));
        assert_eq!(position_of("a\r\nb😀c", 8, true), Position::new(1, 2));
    }

    #[test]
    fn changed_span_is_minimal() {
        let old = "fn main(){\nlet x=1;\n}\n";
        let new = "fn main() {\n    let x = 1;\n}\n";
        let (range, replacement) = changed_span(old, new);
        let mut patched = old.to_string();
        patched.replace_range(range.clone(), replacement);
        assert_eq!(patched, new);
        assert_eq!(range.start, "fn main()".len());
        assert_eq!(changed_span("same", "same"), (4..4, ""));
        let (range, replacement) = changed_span("héllo", "hállo");
        assert_eq!((&"héllo"[range], replacement), ("é", "á"));
    }
}
