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
