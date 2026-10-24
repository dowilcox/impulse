//! Monaco's line commands for the editor, as plain text operations: toggle
//! line comment, move and copy lines, delete lines, insert a line above or
//! below, and add the next occurrence. Each works on the text and the
//! selection (byte offsets) and returns one replacement plus the selection
//! after it, so the editor applies it as one undo step.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineEdit {
    /// The bytes of the original text to replace…
    pub range: Range<usize>,
    /// …with this.
    pub text: String,
    /// The selection afterwards, in the new text.
    pub selection: Range<usize>,
}

/// The line-comment prefix for a language id (`language_from_uri`), if the
/// language has line comments.
pub fn line_comment(language: &str) -> Option<&'static str> {
    Some(match language {
        "rust" | "c" | "cpp" | "javascript" | "javascriptreact" | "typescript"
        | "typescriptreact" | "go" | "java" | "swift" | "php" | "scss" | "less" | "jsonc"
        | "zig" | "kotlin" | "scala" | "csharp" | "dart" | "groovy" | "graphql" => "//",
        "python" | "shellscript" | "yaml" | "toml" | "ruby" | "dockerfile" | "makefile"
        | "cmake" | "perl" | "r" | "elixir" | "nix" | "fish" => "#",
        "lua" | "sql" | "haskell" => "--",
        _ => return None,
    })
}

/// The whole lines the selection touches: from the start of its first line
/// to the end of its last line, line break included when there is one. A
/// selection ending at the very start of a line doesn't take that line.
fn line_span(text: &str, selection: &Range<usize>) -> Range<usize> {
    let start = text[..selection.start].rfind('\n').map_or(0, |i| i + 1);
    let last = if selection.end > selection.start && text[..selection.end].ends_with('\n') {
        selection.end - 1
    } else {
        selection.end
    };
    let end = text[last..].find('\n').map_or(text.len(), |i| last + i + 1);
    start..end.max(start)
}

/// Toggle line comments on the selected lines: comment them all unless
/// every non-blank one is already commented. The prefix goes at the
/// shallowest indentation, as Monaco puts it.
pub fn toggle_comment(text: &str, selection: Range<usize>, prefix: &str) -> LineEdit {
    let span = line_span(text, &selection);
    let block = &text[span.clone()];
    let lines: Vec<&str> = block.split_inclusive('\n').collect();
    let content = |line: &str| line.trim_end_matches(['\n', '\r']).to_string();
    let non_blank: Vec<String> = lines
        .iter()
        .map(|l| content(l))
        .filter(|l| !l.trim().is_empty())
        .collect();
    let uncomment =
        !non_blank.is_empty() && non_blank.iter().all(|l| l.trim_start().starts_with(prefix));
    let indent = non_blank
        .iter()
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out = String::new();
    let mut delta_first = 0i64;
    let mut delta_total = 0i64;
    for (i, line) in lines.iter().enumerate() {
        let body = content(line);
        let ending = &line[body.len()..];
        let new_body = if body.trim().is_empty() {
            body.clone()
        } else if uncomment {
            let at = body.len() - body.trim_start().len();
            let rest = &body[at + prefix.len()..];
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            format!("{}{}", &body[..at], rest)
        } else {
            format!("{}{prefix} {}", &body[..indent], &body[indent..])
        };
        let delta = new_body.len() as i64 - body.len() as i64;
        if i == 0 {
            delta_first = delta;
        }
        delta_total += delta;
        out.push_str(&new_body);
        out.push_str(ending);
    }
    let shift = |offset: usize, delta: i64| (offset as i64 + delta).max(span.start as i64) as usize;
    let selection = if selection.is_empty() {
        let cursor = shift(selection.start, delta_first);
        cursor..cursor
    } else {
        shift(selection.start, delta_first)..shift(selection.end, delta_total)
    };
    LineEdit {
        range: span,
        text: out,
        selection,
    }
}

/// Move the selected lines up or down one line (⌥↑ / ⌥↓). `None` at the
/// top or bottom.
pub fn move_lines(text: &str, selection: Range<usize>, up: bool) -> Option<LineEdit> {
    let span = line_span(text, &selection);
    let mut block = text[span.clone()].to_string();
    if up {
        if span.start == 0 {
            return None;
        }
        let prev_start = text[..span.start - 1].rfind('\n').map_or(0, |i| i + 1);
        let mut previous = text[prev_start..span.start].to_string();
        // The last line has no break: borrow the previous line's.
        if !block.ends_with('\n') {
            block.push('\n');
            previous.pop();
        }
        let moved = span.start - prev_start;
        Some(LineEdit {
            range: prev_start..span.end,
            text: block + &previous,
            selection: selection.start - moved..selection.end - moved,
        })
    } else {
        if span.end >= text.len() {
            return None;
        }
        let next_end = text[span.end..]
            .find('\n')
            .map_or(text.len(), |i| span.end + i + 1);
        let mut next = text[span.end..next_end].to_string();
        if !next.ends_with('\n') {
            next.push('\n');
            block.pop();
        }
        let moved = next.len();
        Some(LineEdit {
            range: span.start..next_end,
            text: next + &block,
            selection: selection.start + moved..selection.end + moved,
        })
    }
}

/// Copy the selected lines above or below themselves (⇧⌥↑ / ⇧⌥↓); the
/// selection goes with the copy below, or stays on the copy above.
pub fn copy_lines(text: &str, selection: Range<usize>, up: bool) -> LineEdit {
    let span = line_span(text, &selection);
    let mut block = text[span.clone()].to_string();
    if !block.ends_with('\n') {
        // The last line: the copy needs a break between the two.
        block.insert(0, '\n');
        let at = span.end;
        let shift = if up { 0 } else { block.len() };
        return LineEdit {
            range: at..at,
            text: block,
            selection: selection.start + shift..selection.end + shift,
        };
    }
    let shift = if up { 0 } else { block.len() };
    LineEdit {
        range: span.start..span.start,
        text: block,
        selection: selection.start + shift..selection.end + shift,
    }
}

/// Delete the selected lines (⇧⌘K); the cursor goes to the start of what
/// follows.
pub fn delete_lines(text: &str, selection: Range<usize>) -> LineEdit {
    let mut span = line_span(text, &selection);
    // The last line: take the break before it instead.
    if span.end == text.len() && !text[span.clone()].ends_with('\n') && span.start > 0 {
        span.start -= 1;
    }
    let cursor = span.start.min(text.len() - (span.end - span.start));
    LineEdit {
        range: span,
        text: String::new(),
        selection: cursor..cursor,
    }
}

/// Insert an empty line below (⌘↩) or above (⇧⌘↩) the cursor's line,
/// indented like it, and put the cursor there.
pub fn insert_line(text: &str, selection: Range<usize>, above: bool) -> LineEdit {
    let line_start = text[..selection.start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[selection.end..]
        .find('\n')
        .map_or(text.len(), |i| selection.end + i);
    let line = &text[line_start..line_end];
    let indent = &line[..line.len() - line.trim_start().len()];
    if above {
        let cursor = line_start + indent.len();
        LineEdit {
            range: line_start..line_start,
            text: format!("{indent}\n"),
            selection: cursor..cursor,
        }
    } else {
        let cursor = line_end + 1 + indent.len();
        LineEdit {
            range: line_end..line_end,
            text: format!("\n{indent}"),
            selection: cursor..cursor,
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The word at `offset`, if there is one.
pub fn word_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(offset, |(i, _)| i);
    let end = text[offset..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(text.len(), |(i, _)| offset + i);
    (start < end).then_some(start..end)
}

/// Add Next Occurrence (⌘D): with nothing selected, the word at the cursor;
/// otherwise the next place the selected text appears after `after` (where
/// the last one added ends), wrapping around, as a whole word when the
/// selection is one (as Monaco matches after ⌘D picked a word). `None` when
/// there's nothing more to add.
pub fn next_occurrence(text: &str, selection: Range<usize>, after: usize) -> Option<Range<usize>> {
    if selection.is_empty() {
        return word_at(text, selection.start);
    }
    let needle = &text[selection.clone()];
    let whole_word = word_at(text, selection.start) == Some(selection.clone());
    let fits = |start: usize| {
        let end = start + needle.len();
        !whole_word
            || (!text[..start].chars().next_back().is_some_and(is_word)
                && !text[end..].chars().next().is_some_and(is_word))
    };
    let find_from = |from: usize, to: usize| {
        text[from..to]
            .match_indices(needle)
            .map(|(i, _)| from + i)
            .find(|&start| fits(start))
    };
    find_from(after.min(text.len()), text.len())
        .or_else(|| find_from(0, after.min(text.len())))
        .map(|start| start..start + needle.len())
        .filter(|found| *found != selection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, edit: &LineEdit) -> String {
        let mut out = text.to_string();
        out.replace_range(edit.range.clone(), &edit.text);
        out
    }

    #[test]
    fn comments_and_uncomments_at_the_shallowest_indent() {
        let text = "fn main() {\n    let a = 1;\n        let b = 2;\n\n}\n";
        let start = text.find("    let a").unwrap();
        let end = text.find("2;").unwrap();
        let commented = toggle_comment(text, start..end, "//");
        let after = apply(text, &commented);
        assert_eq!(
            after,
            "fn main() {\n    // let a = 1;\n    //     let b = 2;\n\n}\n"
        );
        let again = toggle_comment(&after, commented.selection.clone(), "//");
        assert_eq!(apply(&after, &again), text);
    }

    #[test]
    fn a_cursor_comments_its_line_and_moves_with_it() {
        let text = "a\nlet x = 1;\nb";
        let cursor = text.find("x").unwrap();
        let edit = toggle_comment(text, cursor..cursor, "#");
        assert_eq!(apply(text, &edit), "a\n# let x = 1;\nb");
        assert_eq!(edit.selection, cursor + 2..cursor + 2);
    }

    #[test]
    fn moves_lines_up_and_down() {
        let text = "one\ntwo\nthree";
        let at = text.find("two").unwrap();
        let up = move_lines(text, at..at, true).unwrap();
        assert_eq!(apply(text, &up), "two\none\nthree");
        assert_eq!(up.selection, 0..0);
        let down = move_lines(text, at..at, false).unwrap();
        assert_eq!(apply(text, &down), "one\nthree\ntwo");
        assert!(move_lines(text, 0..0, true).is_none());
        let last = text.len();
        assert!(move_lines(text, last..last, false).is_none());
        // The last line up, which has no line break.
        let three = text.find("three").unwrap();
        let edit = move_lines(text, three..three, true).unwrap();
        assert_eq!(apply(text, &edit), "one\nthree\ntwo");
    }

    #[test]
    fn copies_lines() {
        let text = "one\ntwo\n";
        let at = text.find("one").unwrap();
        let down = copy_lines(text, at..at + 1, false);
        assert_eq!(apply(text, &down), "one\none\ntwo\n");
        assert_eq!(
            down.selection,
            4..5,
            "the selection goes with the copy below"
        );
        let up = copy_lines(text, at..at + 1, true);
        assert_eq!(apply(text, &up), "one\none\ntwo\n");
        assert_eq!(up.selection, 0..1);
        // The last line, without a break.
        let edit = copy_lines("a\nb", 2..2, false);
        assert_eq!(apply("a\nb", &edit), "a\nb\nb");
    }

    #[test]
    fn deletes_lines() {
        assert_eq!(
            apply("one\ntwo\nthree", &delete_lines("one\ntwo\nthree", 5..5)),
            "one\nthree"
        );
        let last = delete_lines("one\ntwo", 5..5);
        assert_eq!(apply("one\ntwo", &last), "one");
        assert_eq!(last.selection, 3..3);
        assert_eq!(apply("only", &delete_lines("only", 1..1)), "");
    }

    #[test]
    fn inserts_lines_with_the_indentation() {
        let text = "fn f() {\n    body();\n}";
        let at = text.find("body").unwrap();
        let below = insert_line(text, at..at, false);
        assert_eq!(apply(text, &below), "fn f() {\n    body();\n    \n}");
        assert_eq!(below.selection.start, text.find("();").unwrap() + 3 + 1 + 4);
        let above = insert_line(text, at..at, true);
        assert_eq!(apply(text, &above), "fn f() {\n    \n    body();\n}");
        assert_eq!(above.selection, 13..13);
    }

    #[test]
    fn next_occurrence_selects_the_word_then_finds_the_next() {
        let text = "fn main() { let a = b + a + abc; }";
        let first = text.find("a =").unwrap();
        let second = text.find("+ a").unwrap() + 2;
        assert_eq!(
            next_occurrence(text, first..first, first),
            Some(first..first + 1)
        );
        // A whole word: not the `a` in `main` or `abc`.
        assert_eq!(
            next_occurrence(text, first..first + 1, first + 1),
            Some(second..second + 1)
        );
        // Past the last one it wraps around, and finds only the first again.
        assert_eq!(next_occurrence(text, first..first + 1, second + 1), None);
        // A selection that isn't a whole word matches anywhere.
        assert_eq!(
            next_occurrence("ab abc", 0..1, 1),
            Some(3..4),
            "`a` selected by hand inside `ab`"
        );
        assert_eq!(next_occurrence("x", 0..0, 0), Some(0..1));
        assert_eq!(next_occurrence("  ", 1..1, 1), None);
    }

    #[test]
    fn comment_prefixes() {
        assert_eq!(line_comment("rust"), Some("//"));
        assert_eq!(line_comment("python"), Some("#"));
        assert_eq!(line_comment("lua"), Some("--"));
        assert_eq!(line_comment("markdown"), None);
    }
}
