//! Hints mode (the Swift `TerminalHints`): everything on screen you might
//! want to act on (URLs, file references, git SHAs, local ports), each
//! given a short label to type.
//!
//! Ranges are byte ranges in the scanned line (the Swift app's are UTF-16).

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

use crate::terminal_paths::{self, overlaps, TerminalPathMatch};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKind {
    Url,
    Path,
    Sha,
    Port,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalHintMatch {
    pub kind: HintKind,
    pub range: Range<usize>,
    /// The URL, path reference, SHA or `host:port` as it appears.
    pub text: String,
    /// For paths: the parsed reference (path, line, column).
    pub path: Option<TerminalPathMatch>,
}

impl TerminalHintMatch {
    /// What opening it means: the URL (ports become `http://localhost:N`).
    pub fn url(&self) -> Option<url::Url> {
        match self.kind {
            HintKind::Url => url::Url::parse(&self.text).ok(),
            HintKind::Port => {
                let port = self.text.rsplit(':').next()?;
                url::Url::parse(&format!("http://localhost:{port}")).ok()
            }
            HintKind::Path | HintKind::Sha => None,
        }
    }
}

fn url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"(?:https?|file)://[^\s<>"'`]+"#).expect("valid regex"))
}

/// Preceded by no word character, `/` or `.` (checked separately: the regex
/// crate has no lookbehind).
fn port_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\]):\d{2,5}\b")
            .expect("valid regex")
    })
}

/// Preceded by no word character, `/`, `.` or `-`, and followed by no word
/// character, `/` or `-` (checked separately).
fn sha_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"[0-9a-f]{7,40}").expect("valid regex"))
}

/// Every hint target in `line`, left to right, without overlaps (URLs win
/// over ports, ports over paths, paths over SHAs).
pub fn matches(line: &str) -> Vec<TerminalHintMatch> {
    let mut found: Vec<TerminalHintMatch> = Vec::new();
    let mut add = |hint: TerminalHintMatch| {
        if !hint.range.is_empty() && !found.iter().any(|f| overlaps(&f.range, &hint.range)) {
            found.push(hint);
        }
    };

    for url in url_regex().find_iter(line) {
        let mut end = url.end();
        // Sentence punctuation and an unbalanced closing bracket aren't part
        // of the URL.
        while end > url.start() {
            let last = line.as_bytes()[end - 1];
            if b".,;:!?'\"".contains(&last) {
                end -= 1;
            } else if let Some(open) = match last {
                b')' => Some(b'('),
                b']' => Some(b'['),
                b'}' => Some(b'{'),
                _ => None,
            } {
                let text = &line.as_bytes()[url.start()..end];
                let closing = text.iter().filter(|&&b| b == last).count();
                let opening = text.iter().filter(|&&b| b == open).count();
                if closing <= opening {
                    break;
                }
                end -= 1;
            } else {
                break;
            }
        }
        add(TerminalHintMatch {
            kind: HintKind::Url,
            range: url.start()..end,
            text: line[url.start()..end].to_string(),
            path: None,
        });
    }
    for port in find_in_context(port_regex(), line, |before, _| {
        !before.is_some_and(|c| is_word(c) || c == '/' || c == '.')
    }) {
        add(TerminalHintMatch {
            kind: HintKind::Port,
            text: line[port.clone()].to_string(),
            range: port,
            path: None,
        });
    }
    for reference in terminal_paths::matches(line) {
        add(TerminalHintMatch {
            kind: HintKind::Path,
            range: reference.range.clone(),
            text: line[reference.range.clone()].to_string(),
            path: Some(reference),
        });
    }
    for sha in find_in_context(sha_regex(), line, |before, after| {
        !before.is_some_and(|c| is_word(c) || matches!(c, '/' | '.' | '-'))
            && !after.is_some_and(|c| is_word(c) || matches!(c, '/' | '-'))
    }) {
        let text = &line[sha.clone()];
        // Hex that's all digits is a number; all letters is a word.
        if !text.chars().any(|c| c.is_ascii_digit()) || !text.chars().any(|c| c.is_alphabetic()) {
            continue;
        }
        add(TerminalHintMatch {
            kind: HintKind::Sha,
            text: text.to_string(),
            range: sha,
            path: None,
        });
    }
    found.sort_by_key(|f| f.range.start);
    found
}

/// The matches of `regex` whose surroundings `fits` accepts (the character
/// before and after), trying again one character later after a match that
/// doesn't fit, as a lookaround would.
fn find_in_context(
    regex: &Regex,
    text: &str,
    fits: impl Fn(Option<char>, Option<char>) -> bool,
) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut position = 0;
    while let Some(found) = regex.find_at(text, position) {
        let before = text[..found.start()].chars().next_back();
        let after = text[found.end()..].chars().next();
        if fits(before, after) {
            out.push(found.range());
            position = found.end();
        } else {
            let step = text[found.start()..]
                .chars()
                .next()
                .map_or(1, char::len_utf8);
            position = found.start() + step;
        }
        if position >= text.len() {
            break;
        }
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `count` prefix-free labels from home-row keys: single letters when they
/// suffice, otherwise two letters each.
pub fn labels(count: usize) -> Vec<String> {
    labels_from(count, &['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l'])
}

/// `labels` from another alphabet.
pub fn labels_from(count: usize, alphabet: &[char]) -> Vec<String> {
    if count <= alphabet.len() {
        return alphabet[..count].iter().map(|c| c.to_string()).collect();
    }
    alphabet
        .iter()
        .flat_map(|&first| {
            alphabet
                .iter()
                .map(move |&second| format!("{first}{second}"))
        })
        .take(count)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_each_kind_without_overlaps() {
        let line =
            "see https://example.com/a(b). at src/app.ts:12:4 on localhost:5173 from 1a2b3c4d";
        let hints = matches(line);
        let kinds: Vec<HintKind> = hints.iter().map(|h| h.kind).collect();
        assert_eq!(
            kinds,
            [HintKind::Url, HintKind::Path, HintKind::Port, HintKind::Sha]
        );
        assert_eq!(
            hints[0].text, "https://example.com/a(b)",
            "balanced parens stay, the period goes"
        );
        assert_eq!(hints[1].text, "src/app.ts:12:4");
        assert_eq!(hints[1].path.as_ref().and_then(|p| p.line), Some(12));
        assert_eq!(hints[2].text, "localhost:5173");
        assert_eq!(
            hints[2].url().map(String::from),
            Some("http://localhost:5173/".to_string())
        );
        assert_eq!(hints[3].text, "1a2b3c4d");
    }

    #[test]
    fn urls_own_their_host_and_port() {
        let hints = matches("open http://localhost:3000/docs now");
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].kind, HintKind::Url);
        assert_eq!(hints[0].text, "http://localhost:3000/docs");
    }

    #[test]
    fn sha_needs_digits_and_letters() {
        let texts: Vec<String> = matches("1234567 deadbeef cafe123 abc1234def")
            .into_iter()
            .map(|h| h.text)
            .collect();
        assert_eq!(texts, ["cafe123", "abc1234def"]);
    }

    #[test]
    fn lookarounds_hold() {
        // Inside a longer word, and over 40 characters, isn't a SHA.
        assert!(matches("x1a2b3c4d").is_empty());
        assert!(matches("1a2b3c4d-x").is_empty());
        assert!(matches(&"ab12".repeat(11)).is_empty());
        // A port right after a word isn't one.
        assert!(matches("xlocalhost:80").is_empty());
        assert_eq!(matches("(localhost:80)")[0].text, "localhost:80");
    }

    #[test]
    fn labels_are_prefix_free() {
        assert_eq!(labels(3), ["a", "s", "d"]);
        let many = labels(12);
        assert_eq!(many.len(), 12);
        assert!(many.iter().all(|l| l.len() == 2));
        let distinct: std::collections::HashSet<_> = many.iter().collect();
        assert_eq!(distinct.len(), 12);
        assert!(labels(0).is_empty());
    }
}
