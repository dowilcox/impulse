//! Applying a server's `TextEdit`s (formatting, code actions). Every edit's
//! range refers to the text before any of them, so they're applied from the
//! end of the document back; positions are in characters (UTF-32) or UTF-16
//! code units, whichever the server uses.

use lsp_types::{Position, TextEdit};

/// The byte offset of `position` in `text`, clamped to the line's end and
/// to the text's end.
pub fn offset_of(text: &str, position: Position, utf32: bool) -> usize {
    let mut line_start = 0;
    for _ in 0..position.line {
        match text[line_start..].find('\n') {
            Some(i) => line_start += i + 1,
            None => return text.len(),
        }
    }
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);
    let line = &text[line_start..line_end];
    let mut units = 0u32;
    for (i, c) in line.char_indices() {
        if units >= position.character {
            return line_start + i;
        }
        units += if utf32 { 1 } else { c.len_utf16() as u32 };
    }
    line_end
}

/// `text` with `edits` applied.
pub fn apply_text_edits(text: &str, edits: &[TextEdit], utf32: bool) -> String {
    let mut spans: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|edit| {
            let start = offset_of(text, edit.range.start, utf32);
            let end = offset_of(text, edit.range.end, utf32).max(start);
            (start, end, edit.new_text.as_str())
        })
        .collect();
    // From the end back, so earlier offsets stay valid; edits at the same
    // place keep the server's order.
    spans.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = text.to_string();
    for (start, end, new_text) in spans {
        out.replace_range(start..end, new_text);
    }
    out
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

    #[test]
    fn edits_refer_to_the_original_text() {
        let text = "fn main(){\nlet x=1;\n}\n";
        // As rustfmt would send them: each against the original.
        let edits = [
            edit(0, 9, 0, 9, " "),
            edit(1, 0, 1, 0, "    "),
            edit(1, 5, 1, 6, " = "),
        ];
        assert_eq!(
            apply_text_edits(text, &edits, true),
            "fn main() {\n    let x = 1;\n}\n"
        );
        // The order they arrive in doesn't matter.
        let reversed: Vec<_> = edits.iter().rev().cloned().collect();
        assert_eq!(
            apply_text_edits(text, &reversed, true),
            "fn main() {\n    let x = 1;\n}\n"
        );
    }

    #[test]
    fn utf16_and_utf32_columns() {
        // '𝄞' is one character but two UTF-16 units.
        let text = "a𝄞b\n";
        assert_eq!(offset_of(text, Position::new(0, 2), true), "a𝄞".len());
        assert_eq!(offset_of(text, Position::new(0, 3), false), "a𝄞".len());
        assert_eq!(
            apply_text_edits(text, &[edit(0, 3, 0, 4, "c")], false),
            "a𝄞c\n"
        );
        // Past the end of a line or the text: clamped.
        assert_eq!(offset_of(text, Position::new(0, 99), true), "a𝄞b".len());
        assert_eq!(offset_of(text, Position::new(9, 0), true), text.len());
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
