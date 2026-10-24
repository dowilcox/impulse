//! Bracket pair colors (Monaco's `bracketPairColorization`, on by default
//! in the Swift editor): each `()`, `[]` and `{}` pair takes the color of
//! its nesting depth, cycling through three, and a bracket with no partner
//! is marked as unexpected. Brackets inside strings and comments don't
//! count; those are found with a small lexer per language family rather
//! than the editor's tokens. The same pairs give Monaco's matching-bracket
//! box (`matching`) and its bracket pair guides (`guides`).

use std::ops::Range;

/// Monaco's default bracket colors (`editorBracketHighlight.foreground1`
/// to 3), dark themes then light ones, and the unexpected bracket's.
pub const DARK: [&str; 3] = ["#FFD700", "#DA70D6", "#179FFF"];
pub const LIGHT: [&str; 3] = ["#0431FA", "#319331", "#7B3814"];
pub const UNEXPECTED: &str = "#FF1212";

/// What a bracket's color is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// Nesting depth (0 outermost), which picks one of the three colors.
    Level(usize),
    /// Closes nothing, or is never closed.
    Unexpected,
}

/// How a language writes comments and strings, as far as brackets care.
#[derive(Clone, Copy, Debug, Default)]
struct Syntax {
    line_comment: Option<&'static str>,
    block_comment: Option<(&'static str, &'static str)>,
    /// `'` quotes a string (not in Rust, where it starts lifetimes too).
    single_quote: bool,
    backtick: bool,
}

fn syntax(language: &str) -> Syntax {
    let c_like = Syntax {
        line_comment: Some("//"),
        block_comment: Some(("/*", "*/")),
        single_quote: true,
        backtick: false,
    };
    match language {
        "rust" => Syntax {
            single_quote: false,
            ..c_like
        },
        "javascript" | "typescript" | "javascriptreact" | "typescriptreact" | "go" | "kotlin"
        | "swift" | "scala" | "dart" => Syntax {
            backtick: true,
            ..c_like
        },
        "c" | "cpp" | "csharp" | "java" | "php" | "objective-c" | "json" | "jsonc" => c_like,
        "css" | "scss" | "less" => Syntax {
            line_comment: None,
            ..c_like
        },
        "python" | "shellscript" | "ruby" | "toml" | "yaml" | "r" | "perl" | "dockerfile"
        | "makefile" | "elixir" => Syntax {
            line_comment: Some("#"),
            block_comment: None,
            single_quote: true,
            backtick: language == "shellscript",
        },
        "lua" | "sql" | "haskell" => Syntax {
            line_comment: Some("--"),
            block_comment: None,
            single_quote: true,
            backtick: false,
        },
        // Markdown, plain text and the rest: no comments or strings.
        _ => Syntax::default(),
    }
}

/// A bracket and its partner: their byte offsets and the pair's depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub open: usize,
    pub close: usize,
    pub depth: usize,
}

/// Every bracket in `text` outside strings and comments, with its color.
pub fn colorize(text: &str, language: &str) -> Vec<(Range<usize>, Color)> {
    scan(text, language).0
}

/// The matched pairs in `text`, by their opening bracket.
pub fn pairs(text: &str, language: &str) -> Vec<Pair> {
    scan(text, language).1
}

/// Both at once: the brackets with their colors, and the pairs.
pub fn scan(text: &str, language: &str) -> (Vec<(Range<usize>, Color)>, Vec<Pair>) {
    let syntax = syntax(language);
    let bytes = text.as_bytes();
    let mut out: Vec<(Range<usize>, Color)> = Vec::new();
    let mut pairs: Vec<Pair> = Vec::new();
    // Open brackets: their index in `out` and the closer they want.
    let mut open: Vec<(usize, u8)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Bytes, not `&str`: `i` steps a byte at a time, through the middle
        // of multi-byte characters. Markers are ASCII, so a match starts
        // and ends on character boundaries.
        let rest = &bytes[i..];
        if let Some(marker) = syntax
            .line_comment
            .filter(|m| rest.starts_with(m.as_bytes()))
        {
            i += marker.len();
            i += text[i..].find('\n').unwrap_or(text.len() - i);
            continue;
        }
        if let Some((start, end)) = syntax
            .block_comment
            .filter(|(s, _)| rest.starts_with(s.as_bytes()))
        {
            i += start.len();
            i += text[i..]
                .find(end)
                .map_or(text.len() - i, |e| e + end.len());
            continue;
        }
        let byte = bytes[i];
        let quote = match byte {
            b'"' => true,
            b'\'' => syntax.single_quote,
            b'`' => syntax.backtick,
            _ => false,
        };
        if quote && (syntax.line_comment.is_some() || syntax.block_comment.is_some()) {
            // To the closing quote, past escapes; a string a line break
            // ends (unless backticked) stops there.
            i += 1;
            while i < bytes.len() && bytes[i] != byte {
                if bytes[i] == b'\\' {
                    i += 1;
                } else if bytes[i] == b'\n' && byte != b'`' {
                    break;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        match byte {
            b'(' | b'[' | b'{' => {
                let closer = match byte {
                    b'(' => b')',
                    b'[' => b']',
                    _ => b'}',
                };
                open.push((out.len(), closer));
                out.push((i..i + 1, Color::Level(open.len() - 1)));
            }
            b')' | b']' | b'}' => match open.last() {
                Some(&(index, closer)) if closer == byte => {
                    open.pop();
                    pairs.push(Pair {
                        open: out[index].0.start,
                        close: i,
                        depth: open.len(),
                    });
                    out.push((i..i + 1, Color::Level(open.len())));
                }
                _ => out.push((i..i + 1, Color::Unexpected)),
            },
            _ => {}
        }
        i += 1;
    }
    for (index, _) in open {
        out[index].1 = Color::Unexpected;
    }
    pairs.sort_by_key(|pair| pair.open);
    (out, pairs)
}

/// The pair Monaco boxes (`matchBrackets`) with the cursor at `offset`:
/// the one with a bracket just before the cursor, else just after it.
pub fn matching(pairs: &[Pair], offset: usize) -> Option<Pair> {
    let touches = |at: usize| {
        pairs
            .iter()
            .find(|pair| pair.open == at || pair.close == at)
            .copied()
    };
    offset
        .checked_sub(1)
        .and_then(touches)
        .or_else(|| touches(offset))
}

/// A bracket pair guide: a vertical line down `column` (in visible
/// columns) over the lines between a pair's brackets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guide {
    /// Zero-based lines, the brackets' own excluded.
    pub lines: Range<usize>,
    pub column: usize,
    /// Its pair; the `active` one is drawn at full strength.
    pub pair: Pair,
}

/// The innermost pair around the cursor at `offset`, whose guide Monaco
/// draws at full strength.
pub fn active(pairs: &[Pair], offset: usize) -> Option<Pair> {
    pairs
        .iter()
        .filter(|pair| pair.open < offset && offset <= pair.close)
        .max_by_key(|pair| pair.open)
        .copied()
}

/// Monaco's bracket pair guides (`guides.bracketPairs`) for pairs that span
/// lines: each runs down the lesser of its opening line's indentation and
/// its closing bracket's column.
pub fn guides(text: &str, pairs: &[Pair], tab_size: usize) -> Vec<Guide> {
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_of = |offset: usize| starts.partition_point(|&s| s <= offset) - 1;
    let tab = tab_size.max(1);
    let visible = |from: usize, to: usize| {
        text[from..to].chars().fold(0, |column, c| {
            if c == '\t' {
                column + tab - column % tab
            } else {
                column + 1
            }
        })
    };
    pairs
        .iter()
        .filter_map(|pair| {
            let (open_line, close_line) = (line_of(pair.open), line_of(pair.close));
            if close_line <= open_line + 1 {
                return None;
            }
            let start = starts[open_line];
            let indent_end = start
                + text[start..]
                    .find(|c: char| c != ' ' && c != '\t')
                    .unwrap_or(0);
            let indent = visible(start, indent_end.min(pair.open));
            let close_column = visible(starts[close_line], pair.close);
            Some(Guide {
                lines: open_line + 1..close_line,
                column: indent.min(close_column),
                pair: *pair,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(text: &str, language: &str) -> Vec<(char, Color)> {
        colorize(text, language)
            .into_iter()
            .map(|(range, color)| (text[range].chars().next().unwrap(), color))
            .collect()
    }

    #[test]
    fn pairs_take_their_depth() {
        use Color::*;
        assert_eq!(
            levels("f(a[0], {b})", "rust"),
            vec![
                ('(', Level(0)),
                ('[', Level(1)),
                (']', Level(1)),
                ('{', Level(1)),
                ('}', Level(1)),
                (')', Level(0)),
            ]
        );
        assert_eq!(
            levels("(]", "rust"),
            vec![('(', Unexpected), (']', Unexpected)]
        );
        assert_eq!(levels(")", "plaintext"), vec![(')', Unexpected)]);
    }

    #[test]
    fn matching_takes_the_bracket_before_the_cursor_first() {
        let text = "f(a)(b)";
        let found = pairs(text, "rust");
        assert_eq!(matching(&found, 0), None);
        assert_eq!(matching(&found, 1).map(|p| (p.open, p.close)), Some((1, 3)));
        // Between `)` and `(`: the closing bracket before the cursor.
        assert_eq!(matching(&found, 4).map(|p| (p.open, p.close)), Some((1, 3)));
        assert_eq!(matching(&found, 7).map(|p| (p.open, p.close)), Some((4, 6)));
        // Away from any bracket.
        assert_eq!(matching(&pairs("f(abc)", "rust"), 3), None);
    }

    #[test]
    fn guides_run_between_the_brackets_lines() {
        let text = "{\n  \"a\": 1,\n  \"b\": {\n    \"c\": 2\n  }\n}";
        let found = pairs(text, "json");
        let lines: Vec<(Range<usize>, usize, usize)> = guides(text, &found, 2)
            .into_iter()
            .map(|g| (g.lines, g.column, g.pair.depth))
            .collect();
        assert_eq!(lines, vec![(1..5, 0, 0), (3..4, 2, 1)]);
        // Before the outer `{` nothing is active; inside the inner pair, it is.
        assert_eq!(active(&found, 0), None);
        let inner = active(&found, text.find("\"c\"").unwrap()).unwrap();
        assert_eq!(inner.depth, 1);
        // A pair on one line, or on two adjacent lines, has no guide.
        let short = "f(a)\n{\n}";
        assert!(guides(short, &pairs(short, "rust"), 4).is_empty());
    }

    #[test]
    fn strings_and_comments_hide_brackets() {
        let rust = "let s = \"(\"; // )\n/* { */ f('a', x)";
        assert_eq!(levels(rust, "rust").len(), 2, "only f( and )");
        let python = "x = ')'  # (\nf(1)";
        assert_eq!(levels(python, "python").len(), 2);
        let escaped = r#"s = "\"(" + (a)"#;
        assert_eq!(levels(escaped, "javascript").len(), 2);
        // Multi-byte text around brackets and markers.
        let unicode = "let s = \"é(\"; // ü)\nf(ä, \"→\") /* ö */";
        assert_eq!(levels(unicode, "rust").len(), 2);
        assert_eq!(levels("é(ü)→", "plaintext").len(), 2);
        // Markdown has no strings: a quote doesn't hide anything.
        assert_eq!(levels("\"(\" text)", "markdown").len(), 2);
    }
}
