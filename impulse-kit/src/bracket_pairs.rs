//! Bracket pair colors (Monaco's `bracketPairColorization`, on by default
//! in the Swift editor): each `()`, `[]` and `{}` pair takes the color of
//! its nesting depth, cycling through three, and a bracket with no partner
//! is marked as unexpected. Brackets inside strings and comments don't
//! count; those are found with a small lexer per language family rather
//! than the editor's tokens.

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

/// Every bracket in `text` outside strings and comments, with its color.
pub fn colorize(text: &str, language: &str) -> Vec<(Range<usize>, Color)> {
    let syntax = syntax(language);
    let bytes = text.as_bytes();
    let mut out: Vec<(Range<usize>, Color)> = Vec::new();
    // Open brackets: their index in `out` and the closer they want.
    let mut open: Vec<(usize, u8)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        if let Some(marker) = syntax.line_comment.filter(|m| rest.starts_with(m)) {
            i += marker.len();
            i += text[i..].find('\n').unwrap_or(text.len() - i);
            continue;
        }
        if let Some((start, end)) = syntax.block_comment.filter(|(s, _)| rest.starts_with(s)) {
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
                Some(&(_, closer)) if closer == byte => {
                    open.pop();
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
    out
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
    fn strings_and_comments_hide_brackets() {
        let rust = "let s = \"(\"; // )\n/* { */ f('a', x)";
        assert_eq!(levels(rust, "rust").len(), 2, "only f( and )");
        let python = "x = ')'  # (\nf(1)";
        assert_eq!(levels(python, "python").len(), 2);
        let escaped = r#"s = "\"(" + (a)"#;
        assert_eq!(levels(escaped, "javascript").len(), 2);
        // Markdown has no strings: a quote doesn't hide anything.
        assert_eq!(levels("\"(\" text)", "markdown").len(), 2);
    }
}
