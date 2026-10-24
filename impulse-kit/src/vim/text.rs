//! Plain-text helpers for the Vim layer: characters, lines, word classes,
//! and the motions and text objects that only need the text. Offsets are
//! bytes into the text, always on character boundaries. Lines are split on
//! `\n` the way the editor counts them (a trailing line break makes an empty
//! last line), and a `\r` before the break isn't part of the line.

use std::ops::Range;

/// The offset after the character at `p` (the end of the text stays put).
pub(crate) fn next(t: &str, p: usize) -> usize {
    t[p..].chars().next().map_or(p, |c| p + c.len_utf8())
}

/// The offset of the character before `p` (the start stays put).
pub(crate) fn prev(t: &str, p: usize) -> usize {
    t[..p].chars().next_back().map_or(p, |c| p - c.len_utf8())
}

pub(crate) fn char_at(t: &str, p: usize) -> Option<char> {
    t.get(p..)?.chars().next()
}

pub(crate) fn line_start(t: &str, p: usize) -> usize {
    t[..p].rfind('\n').map_or(0, |i| i + 1)
}

/// The end of the line's content: before its `\n` (and a `\r` before that).
pub(crate) fn line_end(t: &str, p: usize) -> usize {
    let start = line_start(t, p);
    let end = t[p..].find('\n').map_or(t.len(), |i| p + i);
    if end > start && t[..end].ends_with('\r') {
        end - 1
    } else {
        end
    }
}

/// The start of the next line, if there is one.
pub(crate) fn next_line_start(t: &str, p: usize) -> Option<usize> {
    t[p..].find('\n').map(|i| p + i + 1)
}

/// The 0-based line of `p`.
pub(crate) fn line_of(t: &str, p: usize) -> usize {
    t.as_bytes()[..p].iter().filter(|&&b| b == b'\n').count()
}

pub(crate) fn line_count(t: &str) -> usize {
    t.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1
}

/// The start of 0-based line `n` (the last line's when there are fewer).
pub(crate) fn start_of_line(t: &str, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let mut found = 0;
    let mut last = 0;
    for (i, b) in t.bytes().enumerate() {
        if b == b'\n' {
            found += 1;
            last = i + 1;
            if found == n {
                return last;
            }
        }
    }
    last
}

/// The first character of the line that isn't a space or tab (the line's
/// end when it's blank).
pub(crate) fn first_non_blank(t: &str, p: usize) -> usize {
    let start = line_start(t, p);
    let end = line_end(t, p);
    t[start..end]
        .find(|c: char| c != ' ' && c != '\t')
        .map_or(end, |i| start + i)
}

/// The line's leading spaces and tabs.
pub(crate) fn indentation(t: &str, p: usize) -> &str {
    let start = line_start(t, p);
    &t[start..first_non_blank(t, p).max(start)]
}

/// The column of `p` on its line, in characters.
pub(crate) fn column(t: &str, p: usize) -> usize {
    t[line_start(t, p)..p].chars().count()
}

/// The offset of column `col` on the line starting at `start`, or the line's
/// end when it's shorter.
pub(crate) fn at_column(t: &str, start: usize, col: usize) -> usize {
    let end = line_end(t, start);
    t[start..end]
        .char_indices()
        .nth(col)
        .map_or(end, |(i, _)| start + i)
}

/// Where the cursor may rest in Normal mode: on a character, never past the
/// last one of a line that has some.
pub(crate) fn clamp_normal(t: &str, p: usize) -> usize {
    let p = p.min(t.len());
    let start = line_start(t, p);
    let end = line_end(t, p);
    if p >= end && end > start {
        prev(t, end)
    } else {
        p.min(end)
    }
}

/// The end of a range that takes in the character at `p` (inclusive
/// motions), without crossing into the line break.
pub(crate) fn inclusive_end(t: &str, p: usize) -> usize {
    if p < line_end(t, p) {
        next(t, p)
    } else {
        p
    }
}

/// Word classes: blank, word characters, punctuation. A WORD (`big`) is any
/// run of non-blanks.
pub(crate) fn class(c: char, big: bool) -> u8 {
    if c.is_whitespace() {
        0
    } else if big || c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

fn class_at(t: &str, p: usize, big: bool) -> u8 {
    char_at(t, p).map_or(0, |c| class(c, big))
}

/// An empty line's line break (Vim counts an empty line as a word).
fn is_empty_line(t: &str, p: usize) -> bool {
    char_at(t, p) == Some('\n') && (p == 0 || t.as_bytes()[p - 1] == b'\n')
}

/// `w`: the start of the next word.
pub(crate) fn word_forward(t: &str, p: usize, big: bool) -> usize {
    let n = t.len();
    if p >= n {
        return n;
    }
    let mut i = p;
    let c = class_at(t, i, big);
    if c != 0 {
        while i < n && class_at(t, i, big) == c {
            i = next(t, i);
        }
    } else if is_empty_line(t, i) {
        i = next(t, i);
        if is_empty_line(t, i) {
            return i;
        }
    }
    while i < n && class_at(t, i, big) == 0 {
        if is_empty_line(t, i) {
            return i;
        }
        i = next(t, i);
    }
    i
}

/// `b`: the start of this word or the one before.
pub(crate) fn word_backward(t: &str, p: usize, big: bool) -> usize {
    if p == 0 {
        return 0;
    }
    let mut i = prev(t, p);
    while i > 0 && class_at(t, i, big) == 0 {
        if is_empty_line(t, i) {
            return i;
        }
        i = prev(t, i);
    }
    let c = class_at(t, i, big);
    if c == 0 {
        return i;
    }
    while i > 0 && class_at(t, prev(t, i), big) == c {
        i = prev(t, i);
    }
    i
}

/// `e`: the end of this word or the next.
pub(crate) fn word_end(t: &str, p: usize, big: bool) -> usize {
    let n = t.len();
    let mut i = next(t, p);
    while i < n && class_at(t, i, big) == 0 {
        i = next(t, i);
    }
    if i >= n {
        return prev(t, n).max(p.min(n));
    }
    let c = class_at(t, i, big);
    while next(t, i) < n && class_at(t, next(t, i), big) == c {
        i = next(t, i);
    }
    i
}

/// `ge`: the end of the word before.
pub(crate) fn word_end_backward(t: &str, p: usize, big: bool) -> usize {
    if p == 0 {
        return 0;
    }
    let mut i = p;
    let c = class_at(t, i, big);
    if c != 0 {
        while i > 0 && class_at(t, i, big) == c {
            i = prev(t, i);
        }
    }
    while i > 0 && class_at(t, i, big) == 0 {
        if i != p && is_empty_line(t, i) {
            return i;
        }
        i = prev(t, i);
    }
    i
}

/// `f`, `t`, `F` and `T` on the cursor's line: the `count`th `ch`, and for
/// `till` the character before it (after it, going back).
pub(crate) fn find_in_line(
    t: &str,
    p: usize,
    ch: char,
    forward: bool,
    till: bool,
    count: usize,
    skip_adjacent: bool,
) -> Option<usize> {
    let start = line_start(t, p);
    let end = line_end(t, p);
    let mut at = p;
    // `;` after a `t` that already stopped before the character moves on.
    if till && skip_adjacent {
        at = if forward { next(t, at) } else { prev(t, at) };
    }
    for _ in 0..count {
        if forward {
            let from = next(t, at).min(end);
            at = from + t[from..end].find(ch)?;
        } else {
            at = start + t[start..at].rfind(ch)?;
        }
    }
    Some(if till {
        if forward {
            prev(t, at)
        } else {
            next(t, at)
        }
    } else {
        at
    })
}

const OPENERS: &str = "([{";
const CLOSERS: &str = ")]}";

/// `%`: the bracket matching the one under the cursor, or the first after it
/// on the line.
pub(crate) fn match_pair(t: &str, p: usize) -> Option<usize> {
    let end = line_end(t, p);
    let (at, c) = t[p..end]
        .char_indices()
        .find(|(_, c)| OPENERS.contains(*c) || CLOSERS.contains(*c))
        .map(|(i, c)| (p + i, c))?;
    if let Some(k) = OPENERS.find(c) {
        let close = CLOSERS.as_bytes()[k] as char;
        matching_close(t, next(t, at), c, close)
    } else {
        let k = CLOSERS.find(c)?;
        let open = OPENERS.as_bytes()[k] as char;
        matching_open(t, at, open, c)
    }
}

/// The `close` matching an `open` just before `from`.
fn matching_close(t: &str, from: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in t[from..].char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            if depth == 0 {
                return Some(from + i);
            }
            depth -= 1;
        }
    }
    None
}

/// The `open` matching a `close` at `before` (or the unmatched one before
/// `before`).
fn matching_open(t: &str, before: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in t[..before].char_indices().rev() {
        if c == close {
            depth += 1;
        } else if c == open {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
        }
    }
    None
}

fn line_is_empty(t: &str, start: usize) -> bool {
    line_end(t, start) == start
}

/// `}`: the next empty line after a paragraph, or the end of the text.
pub(crate) fn paragraph_forward(t: &str, p: usize) -> usize {
    let mut start = line_start(t, p);
    while line_is_empty(t, start) {
        match next_line_start(t, start) {
            Some(s) => start = s,
            None => return t.len(),
        }
    }
    loop {
        match next_line_start(t, start) {
            Some(s) if line_is_empty(t, s) => return s,
            Some(s) => start = s,
            None => return t.len(),
        }
    }
}

/// `{`: the empty line before a paragraph, or the start of the text.
pub(crate) fn paragraph_backward(t: &str, p: usize) -> usize {
    let mut start = line_start(t, p);
    while start > 0 && line_is_empty(t, start) {
        start = line_start(t, start - 1);
    }
    while start > 0 {
        let above = line_start(t, start - 1);
        if line_is_empty(t, above) {
            return above;
        }
        start = above;
    }
    0
}

/// A span a text object or an operator covers; `linewise` spans are whole
/// lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub linewise: bool,
}

impl Span {
    pub(crate) fn chars(range: Range<usize>) -> Self {
        Span {
            start: range.start,
            end: range.end,
            linewise: false,
        }
    }
}

/// `iw`, `aw`, `iW`, `aW`: runs of one class on the cursor's line.
pub(crate) fn word_object(t: &str, p: usize, big: bool, around: bool, count: usize) -> Span {
    let line_s = line_start(t, p);
    let line_e = line_end(t, p);
    if line_s == line_e {
        return Span::chars(p..p);
    }
    let p = p.min(prev(t, line_e));
    let class_of = |i: usize| class_at(t, i, big);
    // The run around `p`.
    let run = |at: usize| -> Range<usize> {
        let c = class_of(at);
        let mut s = at;
        while s > line_s && class_of(prev(t, s)) == c {
            s = prev(t, s);
        }
        let mut e = next(t, at);
        while e < line_e && class_of(e) == c {
            e = next(t, e);
        }
        s..e
    };
    let first = run(p);
    let mut start = first.start;
    let mut end = first.end;
    let on_blank = class_of(p) == 0;
    if around {
        // The word with the blanks after it, or the blanks before it when
        // there are none after (or the blanks with the word after them).
        let mut words = 0;
        let mut took_trailing = false;
        if on_blank {
            if end < line_e {
                end = run(end).end;
            }
            words += 1;
        } else {
            if end < line_e && class_of(end) == 0 {
                end = run(end).end;
                took_trailing = true;
            }
            words += 1;
        }
        while words < count && end < line_e {
            end = run(end).end;
            if end < line_e && class_of(end) == 0 {
                end = run(end).end;
            }
            words += 1;
        }
        if !on_blank && !took_trailing && start > line_s && class_of(prev(t, start)) == 0 {
            start = run(prev(t, start)).start;
            // Not the line's indentation.
            if start == line_s {
                start = first.start;
            }
        }
    } else {
        for _ in 1..count {
            if end >= line_e {
                break;
            }
            end = run(end).end;
        }
    }
    Span::chars(start..end)
}

/// `i"`, `a"` (and `'`, `` ` ``): the quoted string on the line around the
/// cursor, or the first one after it.
pub(crate) fn quote_object(t: &str, p: usize, quote: char, around: bool) -> Option<Span> {
    let line_s = line_start(t, p);
    let line_e = line_end(t, p);
    let line = &t[line_s..line_e];
    let mut quotes = Vec::new();
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            quotes.push(line_s + i);
        }
    }
    let pair = quotes
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .find(|pair| pair[0] <= p && p <= pair[1])
        .or_else(|| {
            quotes
                .chunks(2)
                .filter(|pair| pair.len() == 2)
                .find(|pair| pair[0] > p)
        })?;
    let (open, close) = (pair[0], pair[1]);
    if !around {
        return Some(Span::chars(next(t, open)..close));
    }
    let mut end = next(t, close);
    let trailing = t[end..line_e]
        .find(|c: char| c != ' ' && c != '\t')
        .map_or(line_e, |i| end + i);
    let mut start = open;
    if trailing > end {
        end = trailing;
    } else {
        let before = t[line_s..open].trim_end_matches([' ', '\t']).len() + line_s;
        if before > line_s {
            start = before;
        }
    }
    Some(Span::chars(start..end))
}

/// `i(`, `a(` and the other brackets: the `count`th pair around the cursor.
/// An inner block whose brackets end and start their lines is the lines in
/// between.
pub(crate) fn bracket_object(
    t: &str,
    p: usize,
    open: char,
    close: char,
    around: bool,
    count: usize,
) -> Option<Span> {
    let mut from = p;
    let mut pair = None;
    for _ in 0..count.max(1) {
        // On the opening bracket itself, that pair; otherwise (on the closing
        // one too) the unmatched opening bracket before.
        let open_at = if pair.is_none() && char_at(t, from) == Some(open) {
            from
        } else {
            matching_open(t, from, open, close)?
        };
        let close_at = matching_close(t, next(t, open_at), open, close)?;
        pair = Some((open_at, close_at));
        from = open_at;
    }
    let (open_at, close_at) = pair?;
    if around {
        return Some(Span::chars(open_at..next(t, close_at)));
    }
    let inner_start = next(t, open_at);
    let after_open = &t[inner_start..line_end(t, inner_start)];
    let close_line = line_start(t, close_at);
    let before_close = &t[close_line..close_at];
    if after_open.trim().is_empty()
        && before_close.trim().is_empty()
        && close_line > inner_start
        && line_of(t, close_at) > line_of(t, open_at) + 1
    {
        let first = next_line_start(t, inner_start)?;
        return Some(Span {
            start: first,
            end: close_line,
            linewise: true,
        });
    }
    Some(Span::chars(inner_start..close_at))
}

/// `ip`, `ap`: the paragraph (or run of empty lines) around the cursor, and
/// for `ap` the empty lines after it (before it at the end of the text).
pub(crate) fn paragraph_object(t: &str, p: usize, around: bool, count: usize) -> Span {
    let empty = |start: usize| line_is_empty(t, start);
    // The run of lines like the one at `start`: (first line start, start of
    // the line after the run, if any).
    let run = |start: usize| -> (usize, Option<usize>) {
        let kind = empty(start);
        let mut first = start;
        while first > 0 {
            let above = line_start(t, first - 1);
            if empty(above) != kind {
                break;
            }
            first = above;
        }
        let mut last = start;
        loop {
            match next_line_start(t, last) {
                Some(s) if empty(s) == kind => last = s,
                Some(s) => return (first, Some(s)),
                None => return (first, None),
            }
        }
    };
    let (first, mut after) = run(line_start(t, p));
    let mut runs = 1;
    let mut took_blank = empty(first);
    let wanted = if around { count * 2 } else { count };
    while runs < wanted {
        let Some(s) = after else { break };
        took_blank |= empty(s);
        after = run(s).1;
        runs += 1;
    }
    let mut start = first;
    let end = after.unwrap_or(t.len());
    // `ap` at the end of the text takes the empty lines before instead.
    if around && !took_blank && start > 0 {
        let mut s = start;
        while s > 0 && empty(line_start(t, s - 1)) {
            s = line_start(t, s - 1);
        }
        start = s;
    }
    Span {
        start,
        end,
        linewise: true,
    }
}
