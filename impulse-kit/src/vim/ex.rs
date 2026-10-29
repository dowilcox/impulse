//! The `:` commands Impulse's Vim mode knows: line ranges, `:w`, `:q`,
//! `:wq`, `:x`, a line number, `:s`, `:d`, `:y`, `:noh`, `:u` and `:red`.

use super::text::{line_count, line_of};

/// What a `:` command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ex {
    /// Only a range: go to its last line (0-based).
    Goto(usize),
    Write,
    Quit,
    WriteQuit,
    /// `:x`: save when modified, then close.
    Exit,
    NoHighlight,
    Undo,
    Redo,
    Delete(LineRange),
    Yank(LineRange),
    Substitute {
        lines: LineRange,
        pattern: String,
        replacement: String,
        global: bool,
        ignore_case: bool,
    },
}

/// 0-based first and last line, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineRange {
    pub first: usize,
    pub last: usize,
}

/// Where a range starts from: the cursor's line, the last line, and the
/// last Visual selection's lines.
pub(crate) struct Lines {
    pub current: usize,
    pub last: usize,
    pub visual: Option<(usize, usize)>,
}

impl Lines {
    pub(crate) fn of(text: &str, cursor: usize, visual: Option<(usize, usize)>) -> Self {
        Lines {
            current: line_of(text, cursor),
            last: line_count(text) - 1,
            visual,
        }
    }
}

/// One address (`12`, `.`, `$`, `'<`, `'>`, with `+n`/`-n`), and the rest.
fn address<'a>(s: &'a str, lines: &Lines) -> Result<(Option<usize>, &'a str), String> {
    let s = s.trim_start();
    let (mut line, mut rest): (Option<i64>, &str) = if let Some(r) = s.strip_prefix('.') {
        (Some(lines.current as i64), r)
    } else if let Some(r) = s.strip_prefix('$') {
        (Some(lines.last as i64), r)
    } else if let Some(r) = s.strip_prefix("'<") {
        let (first, _) = lines.visual.ok_or("Mark not set")?;
        (Some(first as i64), r)
    } else if let Some(r) = s.strip_prefix("'>") {
        let (_, last) = lines.visual.ok_or("Mark not set")?;
        (Some(last as i64), r)
    } else {
        let digits = s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 {
            let n: i64 = s[..digits].parse().unwrap_or(i64::MAX / 2);
            (Some(n - 1), &s[digits..])
        } else {
            (None, s)
        }
    };
    // Offsets: `+3`, `-`, `.+1`.
    loop {
        let sign = match rest.chars().next() {
            Some('+') => 1,
            Some('-') => -1,
            _ => break,
        };
        rest = &rest[1..];
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let n: i64 = if digits == 0 {
            1
        } else {
            rest[..digits].parse().unwrap_or(0)
        };
        rest = &rest[digits..];
        line = Some(line.unwrap_or(lines.current as i64) + sign * n);
    }
    let line = line.map(|l| l.clamp(0, lines.last as i64) as usize);
    Ok((line, rest))
}

/// The range before a command, if it has one.
fn range<'a>(s: &'a str, lines: &Lines) -> Result<(Option<LineRange>, &'a str), String> {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('%') {
        return Ok((
            Some(LineRange {
                first: 0,
                last: lines.last,
            }),
            rest,
        ));
    }
    let (first, rest) = address(s, lines)?;
    let Some(first) = first else {
        return Ok((None, rest));
    };
    if let Some(rest) = rest.strip_prefix([',', ';']) {
        let (last, rest) = address(rest, lines)?;
        let last = last.unwrap_or(lines.current);
        let (first, last) = (first.min(last), first.max(last));
        return Ok((Some(LineRange { first, last }), rest));
    }
    Ok((Some(LineRange { first, last: first }), rest))
}

/// Parse a command line (without its `:`). Errors are messages for the
/// status bar.
pub(crate) fn parse(line: &str, lines: &Lines) -> Result<Ex, String> {
    let (range, rest) = range(line, lines)?;
    let rest = rest.trim();
    let here = LineRange {
        first: lines.current,
        last: lines.current,
    };
    let name_len = rest.len()
        - rest
            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .len();
    let (name, args) = rest.split_at(name_len);
    // `:s/a/b/`, `:s#a#b#`: a substitute's delimiter follows its name
    // straight away.
    for prefix in ["substitute", "su", "s"] {
        if let Some(after) = rest.strip_prefix(prefix) {
            if after.chars().next().is_some_and(is_delimiter) {
                return substitute(after, range.unwrap_or(here));
            }
        }
    }
    let bang = args.trim_start().starts_with('!');
    match name {
        "" if args.is_empty() || bang => match range {
            Some(r) => Ok(Ex::Goto(r.last)),
            None => Err(String::new()),
        },
        "w" | "write" => Ok(Ex::Write),
        "q" | "quit" | "clo" | "close" => Ok(Ex::Quit),
        "wq" => Ok(Ex::WriteQuit),
        "x" | "xit" | "exi" | "exit" => Ok(Ex::Exit),
        "noh" | "nohl" | "nohlsearch" => Ok(Ex::NoHighlight),
        "u" | "un" | "undo" => Ok(Ex::Undo),
        "red" | "redo" => Ok(Ex::Redo),
        "d" | "de" | "del" | "delete" => Ok(Ex::Delete(range.unwrap_or(here))),
        "y" | "ya" | "yank" => Ok(Ex::Yank(range.unwrap_or(here))),
        _ => Err(format!("Not an editor command: {rest}")),
    }
}

fn is_delimiter(c: char) -> bool {
    !c.is_alphanumeric() && !c.is_whitespace() && c != '"' && c != '|' && c != '\\'
}

/// `/pattern/replacement/flags` (any delimiter), split on unescaped
/// delimiters; an escaped delimiter is the character itself.
fn substitute(s: &str, lines: LineRange) -> Result<Ex, String> {
    let mut chars = s.chars();
    let delimiter = chars.next().ok_or("Missing pattern")?;
    let mut parts = vec![String::new()];
    let mut escaped = false;
    for c in chars {
        if escaped {
            if c != delimiter {
                parts.last_mut().unwrap().push('\\');
            }
            parts.last_mut().unwrap().push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delimiter && parts.len() < 3 {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    if escaped {
        parts.last_mut().unwrap().push('\\');
    }
    let pattern = parts[0].clone();
    let replacement = parts.get(1).cloned().unwrap_or_default();
    let flags = parts.get(2).cloned().unwrap_or_default();
    Ok(Ex::Substitute {
        lines,
        pattern,
        replacement,
        global: flags.contains('g'),
        ignore_case: flags.contains('i'),
    })
}

/// A `:s` replacement for one match: `&` and `\0` the match, `\1`…`\9` its
/// groups, `\r` and `\n` a line break, `\&` and `\\` the characters.
pub(crate) fn expand_replacement(replacement: &str, captures: &regex::Captures) -> String {
    let mut out = String::new();
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => out.push_str(captures.get(0).map_or("", |m| m.as_str())),
            '\\' => match chars.next() {
                Some(d @ '0'..='9') => {
                    let group = d as usize - '0' as usize;
                    out.push_str(captures.get(group).map_or("", |m| m.as_str()));
                }
                Some('r' | 'n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten lines, the cursor on the third, no Visual selection yet.
    fn lines() -> Lines {
        Lines {
            current: 2,
            last: 9,
            visual: None,
        }
    }

    fn ex(line: &str) -> Result<Ex, String> {
        parse(line, &lines())
    }

    fn span(first: usize, last: usize) -> LineRange {
        LineRange { first, last }
    }

    fn sub(lines: LineRange, pattern: &str, replacement: &str, flags: &str) -> Ex {
        Ex::Substitute {
            lines,
            pattern: pattern.into(),
            replacement: replacement.into(),
            global: flags.contains('g'),
            ignore_case: flags.contains('i'),
        }
    }

    fn not_a_command(rest: &str) -> Result<Ex, String> {
        Err(format!("Not an editor command: {rest}"))
    }

    #[test]
    fn addresses() {
        assert_eq!(ex("5"), Ok(Ex::Goto(4)));
        assert_eq!(ex("1"), Ok(Ex::Goto(0)));
        assert_eq!(ex("0"), Ok(Ex::Goto(0)));
        assert_eq!(ex("."), Ok(Ex::Goto(2)));
        assert_eq!(ex("$"), Ok(Ex::Goto(9)));
        assert_eq!(ex("  7  "), Ok(Ex::Goto(6)));
    }

    #[test]
    fn visual_marks() {
        let visual = Lines {
            visual: Some((3, 5)),
            ..lines()
        };
        assert_eq!(parse("'<", &visual), Ok(Ex::Goto(3)));
        assert_eq!(parse("'>", &visual), Ok(Ex::Goto(5)));
        assert_eq!(parse("'<,'>d", &visual), Ok(Ex::Delete(span(3, 5))));
        assert_eq!(parse("'>+1", &visual), Ok(Ex::Goto(6)));
        let unset = Err("Mark not set".to_string());
        assert_eq!(ex("'<"), unset);
        assert_eq!(ex("'>"), unset);
        assert_eq!(ex("'<,'>d"), unset);
        assert_eq!(ex("1,'>d"), unset);
    }

    #[test]
    fn offsets() {
        assert_eq!(ex("+3"), Ok(Ex::Goto(5)));
        assert_eq!(ex("-2"), Ok(Ex::Goto(0)));
        assert_eq!(ex("+"), Ok(Ex::Goto(3)));
        assert_eq!(ex("-"), Ok(Ex::Goto(1)));
        assert_eq!(ex("++"), Ok(Ex::Goto(4)));
        assert_eq!(ex("+2-1"), Ok(Ex::Goto(3)));
        assert_eq!(ex(".+1"), Ok(Ex::Goto(3)));
        assert_eq!(ex("$-1"), Ok(Ex::Goto(8)));
        assert_eq!(ex("4+2"), Ok(Ex::Goto(5)));
        assert_eq!(ex("5-1-1"), Ok(Ex::Goto(2)));
    }

    #[test]
    fn addresses_clamp_to_the_buffer() {
        assert_eq!(ex("99"), Ok(Ex::Goto(9)));
        assert_eq!(ex("$+1"), Ok(Ex::Goto(9)));
        assert_eq!(ex("-5"), Ok(Ex::Goto(0)));
        assert_eq!(ex("1-3"), Ok(Ex::Goto(0)));
        assert_eq!(ex("99999999999999999999"), Ok(Ex::Goto(9)));
        // Suspicious: an offset too big to parse counts as 0 instead of
        // clamping like a line number does.
        assert_eq!(ex("+99999999999999999999"), Ok(Ex::Goto(2)));
    }

    #[test]
    fn percent_is_every_line() {
        assert_eq!(ex("%"), Ok(Ex::Goto(9)));
        assert_eq!(ex("%d"), Ok(Ex::Delete(span(0, 9))));
        assert_eq!(ex(" % y"), Ok(Ex::Yank(span(0, 9))));
    }

    #[test]
    fn ranges() {
        assert_eq!(ex("2,4d"), Ok(Ex::Delete(span(1, 3))));
        assert_eq!(ex("2, 4 d"), Ok(Ex::Delete(span(1, 3))));
        assert_eq!(ex(".,$y"), Ok(Ex::Yank(span(2, 9))));
        assert_eq!(ex("2,4"), Ok(Ex::Goto(3)));
        // A missing second address is the cursor's line.
        assert_eq!(ex("6,d"), Ok(Ex::Delete(span(2, 5))));
        // Backwards ranges are swapped without asking.
        assert_eq!(ex("4,2d"), Ok(Ex::Delete(span(1, 3))));
        assert_eq!(ex("2,99d"), Ok(Ex::Delete(span(1, 9))));
        assert_eq!(ex("2;4d"), Ok(Ex::Delete(span(1, 3))));
        // Suspicious: after `;` Vim counts an offset from the first
        // address (lines 5 and 6); here it is from the cursor, like `,`.
        assert_eq!(ex("5;+1d"), Ok(Ex::Delete(span(3, 4))));
        // Suspicious: Vim reads a missing first address as `.`.
        assert_eq!(ex(",4d"), not_a_command(",4d"));
    }

    #[test]
    fn commands() {
        let cases = [
            ("w", Ex::Write),
            ("write", Ex::Write),
            ("q", Ex::Quit),
            ("q!", Ex::Quit),
            ("quit", Ex::Quit),
            ("clo", Ex::Quit),
            ("close", Ex::Quit),
            ("wq", Ex::WriteQuit),
            ("x", Ex::Exit),
            ("xit", Ex::Exit),
            ("exi", Ex::Exit),
            ("exit", Ex::Exit),
            ("noh", Ex::NoHighlight),
            ("nohl", Ex::NoHighlight),
            ("nohlsearch", Ex::NoHighlight),
            ("u", Ex::Undo),
            ("un", Ex::Undo),
            ("undo", Ex::Undo),
            ("red", Ex::Redo),
            ("redo", Ex::Redo),
            ("  w  ", Ex::Write),
            // A range before a command that takes none is dropped.
            ("2,3w", Ex::Write),
            ("%u", Ex::Undo),
        ];
        for (line, command) in cases {
            assert_eq!(ex(line), Ok(command), "{line}");
        }
    }

    #[test]
    fn delete_and_yank() {
        assert_eq!(ex("d"), Ok(Ex::Delete(span(2, 2))));
        assert_eq!(ex("de"), Ok(Ex::Delete(span(2, 2))));
        assert_eq!(ex("del"), Ok(Ex::Delete(span(2, 2))));
        assert_eq!(ex("delete"), Ok(Ex::Delete(span(2, 2))));
        assert_eq!(ex("5d"), Ok(Ex::Delete(span(4, 4))));
        assert_eq!(ex("y"), Ok(Ex::Yank(span(2, 2))));
        assert_eq!(ex("ya"), Ok(Ex::Yank(span(2, 2))));
        assert_eq!(ex("yank"), Ok(Ex::Yank(span(2, 2))));
        assert_eq!(ex("2,3y"), Ok(Ex::Yank(span(1, 2))));
    }

    #[test]
    fn arguments_are_ignored() {
        // Suspicious: `:w` with a file name saves the current file, and a
        // register or count after `:d` or `:y` is dropped.
        assert_eq!(ex("w other.txt"), Ok(Ex::Write));
        assert_eq!(ex("d a"), Ok(Ex::Delete(span(2, 2))));
        assert_eq!(ex("y 3"), Ok(Ex::Yank(span(2, 2))));
    }

    #[test]
    fn unknown_commands() {
        assert_eq!(ex("frobnicate"), not_a_command("frobnicate"));
        assert_eq!(ex("2frob"), not_a_command("frob"));
        assert_eq!(ex("wfoo"), not_a_command("wfoo"));
        assert_eq!(ex("W"), not_a_command("W"));
        assert_eq!(ex("'a"), not_a_command("'a"));
        // An empty line is no error worth showing.
        assert_eq!(ex(""), Err(String::new()));
        assert_eq!(ex("   "), Err(String::new()));
    }

    #[test]
    fn substitute() {
        let here = span(2, 2);
        assert_eq!(ex("s/a/b/"), Ok(sub(here, "a", "b", "")));
        assert_eq!(ex("su/a/b/"), Ok(sub(here, "a", "b", "")));
        assert_eq!(ex("substitute/a/b/"), Ok(sub(here, "a", "b", "")));
        assert_eq!(ex("s/a b/c d/"), Ok(sub(here, "a b", "c d", "")));
        assert_eq!(ex("s/a//"), Ok(sub(here, "a", "", "")));
        // An empty pattern is the last search, filled in by the caller.
        assert_eq!(ex("s//b/"), Ok(sub(here, "", "b", "")));
        assert_eq!(ex("%s/a/b/"), Ok(sub(span(0, 9), "a", "b", "")));
        assert_eq!(ex("2,4s/a/b/"), Ok(sub(span(1, 3), "a", "b", "")));
        let visual = Lines {
            visual: Some((3, 5)),
            ..lines()
        };
        assert_eq!(
            parse("'<,'>s/a/b/", &visual),
            Ok(sub(span(3, 5), "a", "b", ""))
        );
    }

    #[test]
    fn substitute_flags() {
        let here = span(2, 2);
        assert_eq!(ex("s/a/b/g"), Ok(sub(here, "a", "b", "g")));
        assert_eq!(ex("s/a/b/i"), Ok(sub(here, "a", "b", "i")));
        assert_eq!(ex("s/a/b/gi"), Ok(sub(here, "a", "b", "gi")));
        // `I` is Vim's "don't ignore case", the default here.
        assert_eq!(ex("s/a/b/I"), Ok(sub(here, "a", "b", "")));
        assert_eq!(ex("s/a/b/gI"), Ok(sub(here, "a", "b", "g")));
        // Flags without a closing delimiter after them are part of the
        // replacement.
        assert_eq!(ex("s/a/bg"), Ok(sub(here, "a", "bg", "")));
    }

    #[test]
    fn substitute_delimiters() {
        let here = span(2, 2);
        assert_eq!(ex("s#a#/#g"), Ok(sub(here, "a", "/", "g")));
        assert_eq!(ex("s,a,b,"), Ok(sub(here, "a", "b", "")));
        assert_eq!(ex(r"s/a\/b/c\/d/"), Ok(sub(here, "a/b", "c/d", "")));
        // Other escapes reach the regex and the replacement as written.
        assert_eq!(
            ex(r"s/\(a\)\./\1\&/"),
            Ok(sub(here, r"\(a\)\.", r"\1\&", ""))
        );
        assert_eq!(ex(r"s/a\"), Ok(sub(here, r"a\", "", "")));
        // A delimiter can't be a letter, digit, space, `"`, `|` or `\`.
        assert_eq!(ex("sxaxbx"), not_a_command("sxaxbx"));
        assert_eq!(ex("s1a1b1"), not_a_command("s1a1b1"));
        assert_eq!(ex("s a b"), not_a_command("s a b"));
        assert_eq!(ex("s|a|b|"), not_a_command("s|a|b|"));
        assert_eq!(ex(r#"s"a"b""#), not_a_command(r#"s"a"b""#));
        assert_eq!(ex(r"s\a\b\"), not_a_command(r"s\a\b\"));
    }

    #[test]
    fn substitute_missing_delimiters() {
        let here = span(2, 2);
        assert_eq!(ex("s/a/b"), Ok(sub(here, "a", "b", "")));
        assert_eq!(ex("s/a"), Ok(sub(here, "a", "", "")));
        assert_eq!(ex("s/"), Ok(sub(here, "", "", "")));
        // Suspicious: Vim's `:s` alone repeats the last substitute, and it
        // takes every abbreviation from `:su` to `:substitute`.
        assert_eq!(ex("s"), not_a_command("s"));
        assert_eq!(ex("sub/a/b/"), not_a_command("sub/a/b/"));
    }
}
