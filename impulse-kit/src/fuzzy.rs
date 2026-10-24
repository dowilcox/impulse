//! Fuzzy subsequence matching for the command palette and quick open (the
//! Swift `FuzzyMatcher`).
//!
//! A candidate matches when every query character appears in order. Scores
//! reward what people mean when they type a few letters:
//! - matches at word starts (after `/ _ - . space : \`, or a lower-to-upper
//!   camel hump) and at the very start;
//! - runs of consecutive matched characters;
//! - for paths, matches inside the last component (the file name);
//! - shorter candidates (less noise) as a tiebreak.
//!
//! Matching is case-insensitive (ASCII folding) unless the query contains
//! an uppercase letter ("smart case").
//!
//! Scoring runs over UTF-16 code units, as the Swift one does, so scores
//! and ranking are the same in both ports; the matched positions are
//! reported as byte offsets into the candidate, ready for slicing.

// Scoring weights.
const MATCH_BASE: i64 = 16;
const BOUNDARY_BONUS: i64 = 16;
const START_BONUS: i64 = 30;
const CONSECUTIVE_BONUS: i64 = 20;
const FILE_NAME_BONUS: i64 = 12;
const GAP_PENALTY: i64 = 1;
const MAX_STARTS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub score: i64,
    /// Byte offsets of the matched characters in the candidate, ascending.
    pub positions: Vec<usize>,
}

impl Match {
    fn empty() -> Self {
        Match {
            score: 0,
            positions: Vec::new(),
        }
    }
}

/// A query prepared once for matching many candidates.
struct Query {
    /// Its UTF-16 code units, lowercased unless it's case-sensitive.
    units: Vec<u16>,
    case_sensitive: bool,
}

impl Query {
    fn new(query: &str) -> Self {
        let case_sensitive = query.chars().any(char::is_uppercase);
        let units = query
            .encode_utf16()
            .map(|u| if case_sensitive { u } else { lower(u) })
            .collect();
        Query {
            units,
            case_sensitive,
        }
    }
}

/// Trim spaces and tabs (not line breaks), as the Swift `.whitespaces` set.
fn trim_horizontal(s: &str) -> &str {
    s.trim_matches(|c: char| {
        c.is_whitespace()
            && !matches!(
                c,
                '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
            )
    })
}

/// Score `candidate` against `query`, or `None` when it doesn't match.
/// `is_path` enables the file-name bonus for matches after the last `/`.
pub fn fuzzy_match(query: &str, candidate: &str, is_path: bool) -> Option<Match> {
    match_prepared(&Query::new(query), candidate, is_path)
}

fn match_prepared(query: &Query, candidate: &str, is_path: bool) -> Option<Match> {
    let qn = &query.units;
    if qn.is_empty() {
        return Some(Match::empty());
    }

    // Quick reject, before copying anything: the whole query must appear as
    // a subsequence (most candidates stop here).
    let mut qi = 0;
    for unit in candidate.encode_utf16() {
        let unit = if query.case_sensitive {
            unit
        } else {
            lower(unit)
        };
        if unit == qn[qi] {
            qi += 1;
            if qi == qn.len() {
                break;
            }
        }
    }
    if qi != qn.len() {
        return None;
    }

    let c: Vec<u16> = candidate.encode_utf16().collect();
    let cn: Vec<u16> = if query.case_sensitive {
        c.clone()
    } else {
        c.iter().map(|&u| lower(u)).collect()
    };
    let file_name_start = if is_path {
        c.iter().rposition(|&u| u == 0x2F).map_or(0, |i| i + 1)
    } else {
        0
    };

    // Try greedy alignments starting at each occurrence of the first query
    // character (bounded) and keep the best: cheap and close to optimal for
    // palette-sized inputs.
    let mut best: Option<(i64, Vec<usize>)> = None;
    let mut starts = 0;
    let mut start = 0;
    while start < cn.len() && starts < MAX_STARTS {
        if cn[start] != qn[0] {
            start += 1;
            continue;
        }
        starts += 1;
        if let Some(m) = align(qn, &cn, &c, start, file_name_start, is_path) {
            if best.as_ref().is_none_or(|b| m.0 > b.0) {
                best = Some(m);
            }
        }
        start += 1;
    }
    let (score, positions) = best?;
    // Prefer shorter candidates when everything else is equal.
    let score = score - (c.len() as i64 / 8).min(20);
    Some(Match {
        score,
        positions: byte_offsets(candidate, &positions),
    })
}

/// UTF-16 offsets (ascending) as byte offsets of the characters they're in.
fn byte_offsets(text: &str, units: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(units.len());
    let mut wanted = units.iter().peekable();
    let mut unit = 0;
    for (byte, ch) in text.char_indices() {
        let end = unit + ch.len_utf16();
        while let Some(&&u) = wanted.peek() {
            if u >= end {
                break;
            }
            out.push(byte);
            wanted.next();
        }
        unit = end;
        if wanted.peek().is_none() {
            break;
        }
    }
    out
}

fn align(
    q: &[u16],
    c: &[u16],
    original: &[u16],
    start: usize,
    file_name_start: usize,
    is_path: bool,
) -> Option<(i64, Vec<usize>)> {
    let mut positions = Vec::with_capacity(q.len());
    let mut score = 0;
    let mut qi = 0;
    let mut ci = start;
    let mut previous: isize = -2;
    while qi < q.len() && ci < c.len() {
        if c[ci] != q[qi] {
            ci += 1;
            continue;
        }
        // Prefer a later word-boundary occurrence of this character when it
        // scores better than taking this one and the rest still fits after
        // it (e.g. "tb" -> the "B" in "TabBar", not the "b").
        let mut pick = ci;
        let here = step_score(original, ci, previous, file_name_start, is_path);
        if ci == 0 || !is_boundary(original, ci) {
            let mut j = ci + 1;
            while j < c.len() {
                if c[j] == q[qi] && is_boundary(original, j) && is_subsequence(q, qi + 1, c, j) {
                    if step_score(original, j, previous, file_name_start, is_path) > here {
                        pick = j;
                    }
                    break;
                }
                j += 1;
            }
        }
        score += step_score(original, pick, previous, file_name_start, is_path);
        positions.push(pick);
        previous = pick as isize;
        qi += 1;
        ci = pick + 1;
    }
    (qi == q.len()).then_some((score, positions))
}

/// Score for matching the character at `i` given the previous match.
fn step_score(
    original: &[u16],
    i: usize,
    previous: isize,
    file_name_start: usize,
    is_path: bool,
) -> i64 {
    let mut s = MATCH_BASE;
    if i == 0 {
        s += START_BONUS;
    } else if is_path && i == file_name_start {
        // First letter of the file name: a strong word start.
        s += BOUNDARY_BONUS + 8;
    } else if is_boundary(original, i) {
        s += BOUNDARY_BONUS;
    }
    let i = i as isize;
    if previous == i - 1 {
        s += CONSECUTIVE_BONUS;
    } else if previous >= 0 {
        s -= ((i - previous - 1).min(10) as i64) * GAP_PENALTY;
    }
    if is_path && i as usize >= file_name_start {
        s += FILE_NAME_BONUS;
    }
    s
}

/// Whether `q[qi..]` is a subsequence of `c[after + 1..]`.
fn is_subsequence(q: &[u16], qi: usize, c: &[u16], after: usize) -> bool {
    let mut k = qi;
    let mut j = after + 1;
    while k < q.len() && j < c.len() {
        if c[j] == q[k] {
            k += 1;
        }
        j += 1;
    }
    k == q.len()
}

fn is_boundary(c: &[u16], i: usize) -> bool {
    let prev = c[i - 1];
    match prev {
        // / _ - . space : \
        0x2F | 0x5F | 0x2D | 0x2E | 0x20 | 0x3A | 0x5C => true,
        // camelCase hump: lowercase followed by uppercase.
        _ => is_lower_ascii(prev) && is_upper_ascii(c[i]),
    }
}

fn lower(u: u16) -> u16 {
    if is_upper_ascii(u) {
        u + 32
    } else {
        u
    }
}

fn is_upper_ascii(u: u16) -> bool {
    (0x41..=0x5A).contains(&u)
}

fn is_lower_ascii(u: u16) -> bool {
    (0x61..=0x7A).contains(&u)
}

/// Rank `items` by fuzzy score against `query` (best first), keeping at
/// most `limit`. Items that don't match are dropped; an empty query keeps
/// the input order.
pub fn rank<'a, T>(
    items: &'a [T],
    query: &str,
    limit: usize,
    is_path: bool,
    text: impl Fn(&T) -> &str,
) -> Vec<(&'a T, Match)> {
    let trimmed = trim_horizontal(query);
    if trimmed.is_empty() {
        return items
            .iter()
            .take(limit)
            .map(|item| (item, Match::empty()))
            .collect();
    }
    best(
        matches(items, trimmed, is_path, || false, text).unwrap_or_default(),
        limit,
    )
}

/// Every item that matches `query` (trimmed), in input order, or `None`
/// when `should_stop` returned true (asked every few hundred items, so a
/// superseded search over a big list ends early). An empty query matches
/// everything. With `best`, this is `rank` in two steps: the matches of a
/// query are all a longer query that starts with it can match, so they can
/// be searched again instead of the whole list.
pub fn matches<'a, T>(
    items: &'a [T],
    query: &str,
    is_path: bool,
    mut should_stop: impl FnMut() -> bool,
    text: impl Fn(&T) -> &str,
) -> Option<Vec<(&'a T, Match)>> {
    let prepared = Query::new(trim_horizontal(query));
    let mut found = Vec::with_capacity(items.len().min(4096));
    for (order, item) in items.iter().enumerate() {
        if order % 512 == 0 && should_stop() {
            return None;
        }
        if let Some(m) = match_prepared(&prepared, text(item), is_path) {
            found.push((item, m));
        }
    }
    Some(found)
}

/// The `limit` best of `matches` (in input order), best first; equal scores
/// keep their input order.
pub fn best<T>(mut matches: Vec<(T, Match)>, limit: usize) -> Vec<(T, Match)> {
    // A stable sort keeps input order among equal scores.
    matches.sort_by_key(|m| std::cmp::Reverse(m.1.score));
    matches.truncate(limit);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank_strs<'a>(items: &'a [&'a str], query: &str, is_path: bool) -> Vec<&'a str> {
        rank(items, query, usize::MAX, is_path, |s| s)
            .into_iter()
            .map(|(s, _)| *s)
            .collect()
    }

    #[test]
    fn requires_in_order_subsequence() {
        assert!(fuzzy_match("tmn", "TabManager.swift", false).is_some());
        assert!(fuzzy_match("nmt", "TabManager.swift", false).is_none());
        assert!(fuzzy_match("xyz", "abc", false).is_none());
        assert_eq!(fuzzy_match("", "anything", false).unwrap().score, 0);
    }

    #[test]
    fn reports_matched_positions() {
        assert_eq!(
            fuzzy_match("tm", "TabManager", false).unwrap().positions,
            vec![0, 3]
        );
        // Byte offsets past multi-byte characters.
        assert_eq!(
            fuzzy_match("tm", "é👍TabManager", false).unwrap().positions,
            vec![6, 9]
        );
    }

    #[test]
    fn smart_case() {
        assert!(fuzzy_match("tab", "TabManager", false).is_some());
        assert!(fuzzy_match("Tab", "tabmanager", false).is_none());
    }

    #[test]
    fn prefers_word_boundaries_and_consecutive_runs() {
        let ranked = rank_strs(
            &["text/bar.md", "the_big_ref.swift", "TabBar.swift"],
            "tb",
            true,
        );
        assert_eq!(ranked[0], "TabBar.swift");

        let consecutive = fuzzy_match("main", "MainWindow.swift", false).unwrap();
        let scattered = fuzzy_match("main", "my_awful_init.swift", false).unwrap();
        assert!(consecutive.score > scattered.score);
    }

    #[test]
    fn path_matches_favor_the_file_name() {
        let ranked = rank_strs(
            &[
                "impulse-macos/Sources/ImpulseApp/Workbench/ChromeBarView.swift",
                "impulse-macos/Sources/ImpulseApp/Workbench/WorkbenchView.swift",
            ],
            "workbenchview",
            true,
        );
        assert!(ranked[0].ends_with("WorkbenchView.swift"));
    }

    #[test]
    fn rank_keeps_order_for_empty_query_and_respects_limit() {
        let items = ["b", "a", "c"];
        assert_eq!(rank_strs(&items, "", false), vec!["b", "a", "c"]);
        assert_eq!(rank(&items, "", 2, false, |s| s).len(), 2);
    }

    #[test]
    fn command_titles() {
        let titles = [
            "Toggle Sidebar",
            "Toggle Right Panel",
            "Go to Line",
            "New Terminal Tab",
        ];
        assert_eq!(rank_strs(&titles, "ntt", false)[0], "New Terminal Tab");
        // Spaces are literal characters in the query; "tog side" still matches.
        assert_eq!(rank_strs(&titles, "tog side", false)[0], "Toggle Sidebar");
    }

    const PATHS: [&str; 10] = [
        "impulse-macos/Sources/ImpulseApp/TabManager.swift",
        "impulse-macos/Sources/ImpulseApp/Workbench/TitlebarTabStrip.swift",
        "impulse-macos/Sources/ImpulseApp/Palette/PaletteModel.swift",
        "impulse-macos/Sources/ImpulseKit/FuzzyMatcher.swift",
        "impulse-macos/Tests/ImpulseKitTests/FuzzyMatcherTests.swift",
        "docs/command-palette.md",
        "README.md",
        "src/tab_bar.rs",
        "src/TabBar/index.ts",
        "Makefile",
    ];

    #[test]
    fn matches_then_best_is_rank() {
        for query in [
            "t", "tab", "tb", "Tab", "pm", "fzm", "md", " tab ", "", "zzz",
        ] {
            let ranked = rank(&PATHS, query, 4, true, |s| s);
            let matched = matches(&PATHS, query, true, || false, |s| s).unwrap_or_default();
            let best = best(matched, 4);
            assert_eq!(best, ranked, "{query}");
        }
    }

    #[test]
    fn narrowing_from_a_shorter_query_ranks_the_same() {
        // Quick open searches the last query's matches when the query only
        // grew; that must give what searching everything gives.
        for (shorter, longer) in [
            ("t", "tab"),
            ("ta", "taB"),
            ("s", "src/t"),
            ("", "pal"),
            ("m", "md"),
        ] {
            let narrowed: Vec<&str> = matches(&PATHS, shorter, true, || false, |s| s)
                .unwrap_or_default()
                .into_iter()
                .map(|(s, _)| *s)
                .collect();
            let from_narrowed: Vec<(&str, Match)> = best(
                matches(&narrowed, longer, true, || false, |s| s)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(s, m)| (*s, m))
                    .collect(),
                usize::MAX,
            );
            let from_all: Vec<(&str, Match)> = rank(&PATHS, longer, usize::MAX, true, |s| s)
                .into_iter()
                .map(|(s, m)| (*s, m))
                .collect();
            assert_eq!(from_narrowed, from_all, "{shorter} -> {longer}");
        }
    }

    #[test]
    fn matching_stops_when_asked() {
        let many = vec!["Sources/ImpulseApp/TabManager.swift"; 5000];
        let mut asked = 0;
        let stopped = matches(
            &many,
            "tab",
            false,
            || {
                asked += 1;
                asked > 2
            },
            |s| s,
        );
        assert!(stopped.is_none());
        assert_eq!(asked, 3);
        assert_eq!(
            matches(&many, "tab", false, || false, |s| s).map(|m| m.len()),
            Some(5000)
        );
    }
}
