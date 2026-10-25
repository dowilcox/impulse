//! Sticky scroll (Monaco's `stickyScroll`, the `sticky_scroll` setting):
//! the lines that open the scopes enclosing the top of the view, kept at
//! the top while scrolling. Monaco takes scopes from its folding model;
//! here they come from indentation, which every language has.

/// Monaco's default most lines shown.
pub const MAX_LINES: usize = 5;

/// A line's indentation in columns (a tab counts as `tab_size`); `None`
/// for a blank line.
fn indent(line: &str, tab_size: usize) -> Option<usize> {
    if line.trim().is_empty() {
        return None;
    }
    Some(
        line.chars()
            .take_while(|c| c.is_whitespace())
            .map(|c| if c == '\t' { tab_size } else { 1 })
            .sum(),
    )
}

/// The rows (0-based) of the lines opening the scopes around row `top`,
/// outermost first, at most `max`. A line opens a scope when the next
/// non-blank line is indented deeper than it.
pub fn scope_rows(lines: &[&str], top: usize, tab_size: usize, max: usize) -> Vec<usize> {
    scope_rows_by(lines.len(), |row| lines[row], top, tab_size, max)
}

/// `scope_rows` over `count` lines read one at a time by `line`, so only
/// the few lines it looks at are taken from the document.
pub fn scope_rows_by<S: AsRef<str>>(
    count: usize,
    line: impl Fn(usize) -> S,
    top: usize,
    tab_size: usize,
    max: usize,
) -> Vec<usize> {
    if top == 0 || top >= count {
        return Vec::new();
    }
    let indent_at = |row: usize| indent(line(row).as_ref(), tab_size);
    // The indentation the top of the view is at (blank lines take the next
    // line's).
    let Some(mut depth) = (top..count).find_map(indent_at) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for row in (0..top).rev() {
        let Some(own) = indent_at(row) else {
            continue;
        };
        if own < depth {
            let opens = (row + 1..count)
                .find_map(indent_at)
                .is_some_and(|next| next > own);
            if opens {
                rows.push(row);
            }
            depth = own;
            if own == 0 {
                break;
            }
        }
    }
    rows.reverse();
    // The innermost scopes matter most when there are too many.
    if rows.len() > max {
        rows.drain(..rows.len() - max);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enclosing_scopes_by_indentation() {
        let text = "mod a {\n    fn b() {\n        let x = 1;\n\n        if x {\n            go();\n        }\n    }\n}\nfn c() {}";
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(scope_rows(&lines, 5, 4, MAX_LINES), vec![0, 1, 4]);
        assert_eq!(
            scope_rows(&lines, 3, 4, MAX_LINES),
            vec![0, 1],
            "a blank line takes the next one's depth"
        );
        assert_eq!(scope_rows(&lines, 2, 4, MAX_LINES), vec![0, 1]);
        assert!(
            scope_rows(&lines, 9, 4, MAX_LINES).is_empty(),
            "at the top level"
        );
        assert!(scope_rows(&lines, 0, 4, MAX_LINES).is_empty());
        assert_eq!(scope_rows(&lines, 5, 4, 2), vec![1, 4], "innermost kept");
        let tabs = ["fn a() {", "\tlet b;", "\tif b {", "\t\tc();"];
        assert_eq!(scope_rows(&tabs, 3, 4, MAX_LINES), vec![0, 2]);
    }
}
