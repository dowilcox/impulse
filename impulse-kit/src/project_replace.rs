//! Project-wide replace (the Swift `ProjectReplace`): the same literal,
//! per-line matching as project search, applied to whole files, plus the
//! before/after preview of a line.
//!
//! Case-insensitive matching compares characters by their lowercase forms,
//! one character against one, as project search lowercases both sides.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Same(String),
    Removed(String),
    Added(String),
}

fn chars_equal(a: char, b: char, case_sensitive: bool) -> bool {
    a == b || (!case_sensitive && a.to_lowercase().eq(b.to_lowercase()))
}

/// The byte range of the first match of `query` in `text` at or after
/// `from`.
fn find(text: &str, query: &str, from: usize, case_sensitive: bool) -> Option<(usize, usize)> {
    if case_sensitive {
        return text[from..]
            .find(query)
            .map(|i| (from + i, from + i + query.len()));
    }
    for (offset, _) in text[from..].char_indices() {
        let start = from + offset;
        let mut rest = text[start..].char_indices();
        let mut end = start;
        let matched = query.chars().all(|q| match rest.next() {
            Some((i, c)) if chars_equal(c, q, false) => {
                end = start + i + c.len_utf8();
                true
            }
            _ => false,
        });
        if matched {
            return Some((start, end));
        }
    }
    None
}

/// Replace every occurrence of `query` in `text`, with how many there were.
/// Like search, matches never span lines, so a query containing a line
/// break replaces nothing.
pub fn replace(
    text: &str,
    query: &str,
    replacement: &str,
    case_sensitive: bool,
) -> (String, usize) {
    if query.is_empty() || query.contains(['\n', '\r']) {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    let mut cursor = 0;
    while let Some((start, end)) = find(text, query, cursor, case_sensitive) {
        out.push_str(&text[cursor..start]);
        out.push_str(replacement);
        count += 1;
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    (out, count)
}

/// A line with each match shown removed and the replacement added.
pub fn preview(line: &str, query: &str, replacement: &str, case_sensitive: bool) -> Vec<Segment> {
    if query.is_empty() {
        return vec![Segment::Same(line.to_string())];
    }
    let mut segments = Vec::new();
    let mut cursor = 0;
    while let Some((start, end)) = find(line, query, cursor, case_sensitive) {
        if cursor < start {
            segments.push(Segment::Same(line[cursor..start].to_string()));
        }
        segments.push(Segment::Removed(line[start..end].to_string()));
        if !replacement.is_empty() {
            segments.push(Segment::Added(replacement.to_string()));
        }
        cursor = end;
    }
    if cursor < line.len() {
        segments.push(Segment::Same(line[cursor..].to_string()));
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use Segment::*;

    #[test]
    fn replaces_every_occurrence_keeping_line_endings() {
        let text = "let Foo = foo()\r\nfoo.bar\n";
        assert_eq!(
            replace(text, "foo", "baz", false),
            ("let baz = baz()\r\nbaz.bar\n".to_string(), 3)
        );
        assert_eq!(
            replace(text, "foo", "baz", true),
            ("let Foo = baz()\r\nbaz.bar\n".to_string(), 2)
        );
    }

    #[test]
    fn empty_and_multiline_queries_replace_nothing() {
        assert_eq!(replace("abc", "", "x", false).1, 0);
        assert_eq!(replace("a\nb", "a\nb", "x", false).0, "a\nb");
    }

    #[test]
    fn replacement_can_contain_the_query() {
        assert_eq!(replace("aa", "a", "aa", true), ("aaaa".to_string(), 2));
    }

    #[test]
    fn non_ascii_matches_keep_their_own_text() {
        assert_eq!(
            replace("Größe und GRÖSSE", "größe", "x", false),
            ("x und GRÖSSE".to_string(), 1)
        );
        assert_eq!(
            preview("ÄÖ äö", "äö", "", false),
            vec![Removed("ÄÖ".into()), Same(" ".into()), Removed("äö".into())]
        );
    }

    #[test]
    fn preview_marks_removed_and_added() {
        assert_eq!(
            preview("x = Foo(foo)", "foo", "bar", false),
            vec![
                Same("x = ".into()),
                Removed("Foo".into()),
                Added("bar".into()),
                Same("(".into()),
                Removed("foo".into()),
                Added("bar".into()),
                Same(")".into()),
            ]
        );
        assert_eq!(
            preview("remove me", " me", "", true),
            vec![Same("remove".into()), Removed(" me".into())]
        );
    }
}
