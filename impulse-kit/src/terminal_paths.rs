//! File references in a line of terminal output (the Swift
//! `TerminalPathDetector`): compiler errors, stack traces, grep hits, plain
//! paths, so the terminal can open them in the editor. Candidates only: the
//! caller checks the file exists (relative to the terminal's directory)
//! before treating one as a link.
//!
//! Ranges are byte ranges in the scanned text (the Swift app's are UTF-16).

use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPathMatch {
    /// The whole reference, `:line:col` included.
    pub range: Range<usize>,
    pub path: String,
    /// 1-based, when the reference names a position.
    pub line: Option<usize>,
    pub column: Option<usize>,
}

/// Characters a path token is made of (no spaces, quotes or brackets).
const PATH_CHARS: &str = r"[A-Za-z0-9_.@+~/\-]";

/// `path:line[:col]`, `path(line[,col])` and bare paths that contain a slash
/// or end in an extension.
fn reference_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        let path = format!(r"(?<path>{PATH_CHARS}*(?:/{PATH_CHARS}+|\.[A-Za-z0-9]+){PATH_CHARS}*)");
        let position =
            r"(?::(?<line>\d+)(?::(?<col>\d+))?|\((?<pline>\d+)(?:,\s?(?<pcol>\d+))?\))?";
        Regex::new(&format!("{path}{position}")).expect("valid regex")
    })
}

/// Python tracebacks: `File "path", line N`.
fn python_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"File "(?<path>[^"]+)", line (?<line>\d+)"#).expect("valid regex")
    })
}

/// Every candidate reference in `text`, left to right.
pub fn matches(text: &str) -> Vec<TerminalPathMatch> {
    let mut results: Vec<TerminalPathMatch> = Vec::new();

    for captures in python_regex().captures_iter(text) {
        let whole = captures.get(0).expect("whole match");
        results.push(TerminalPathMatch {
            range: whole.range(),
            path: captures["path"].to_string(),
            line: number(captures.name("line")),
            column: None,
        });
    }

    for captures in reference_regex().captures_iter(text) {
        let whole = captures.get(0).expect("whole match");
        let full_path = captures.name("path").expect("path group").range();
        // Right after a colon is a URL's "//host/..." (the URL detector owns
        // those) or a key:value, not a path.
        if full_path.start > 0 && text.as_bytes()[full_path.start - 1] == b':' {
            continue;
        }
        // Trailing sentence punctuation isn't part of a path.
        let mut path_range = full_path.clone();
        while path_range.len() > 1
            && matches!(text.as_bytes()[path_range.end - 1], b'.' | b',' | b':')
        {
            path_range.end -= 1;
        }
        let path = &text[path_range.clone()];
        if !is_plausible(path) {
            continue;
        }

        let mut line = number(captures.name("line")).or(number(captures.name("pline")));
        let mut column = number(captures.name("col")).or(number(captures.name("pcol")));
        let mut end = whole.end();
        if path_range.len() < full_path.len() {
            // Punctuation was trimmed; the position (if any) went with it.
            line = None;
            column = None;
            end = path_range.end;
        }
        let range = path_range.start..end;
        if results.iter().any(|r| overlaps(&r.range, &range)) {
            continue;
        }
        results.push(TerminalPathMatch {
            range,
            path: path.to_string(),
            line,
            column,
        });
    }
    results.sort_by_key(|m| m.range.start);
    results
}

/// The candidate covering byte offset `offset`.
pub fn match_at(text: &str, offset: usize) -> Option<TerminalPathMatch> {
    matches(text)
        .into_iter()
        .find(|m| m.range.contains(&offset))
}

/// Resolve a candidate against a directory: absolute, `~`, or relative.
/// Absolute results have `.` and `..` worked out, as NSString's
/// `standardizingPath` does.
pub fn resolve(path: &str, directory: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        let home = dirs::home_dir().unwrap_or_default();
        let home = home.to_string_lossy();
        return format!("{home}{}", &path[1..]);
    }
    let joined = if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        Path::new(directory).join(path)
    };
    standardized(&joined)
}

/// `path` without empty and `.` components, with `..` taken off the folder
/// before it in absolute paths, and without a trailing slash.
fn standardized(path: &Path) -> String {
    let absolute = path.is_absolute();
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
            Component::ParentDir if absolute => {
                parts.pop();
            }
            Component::ParentDir => parts.push("..".to_string()),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

pub(crate) fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

fn number(capture: Option<regex::Match>) -> Option<usize> {
    capture
        .filter(|m| !m.is_empty())
        .and_then(|m| m.as_str().parse().ok())
}

fn is_plausible(path: &str) -> bool {
    if path.chars().count() < 2 || !path.chars().any(char::is_alphabetic) {
        return false;
    }
    // Version numbers and decimals ("1.2.3", "v2.0", "0.5s") aren't files.
    if !path.contains('/') {
        let stem = path.split('.').next().unwrap_or_default();
        let digits = stem.strip_prefix('v').unwrap_or(stem);
        let starts_with_number = path.chars().next().is_some_and(char::is_numeric);
        if starts_with_number || (!digits.is_empty() && digits.chars().all(char::is_numeric)) {
            return false;
        }
    }
    !path.chars().all(|c| matches!(c, '.' | '/' | '~' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(text: &str) -> TerminalPathMatch {
        let all = matches(text);
        assert_eq!(all.len(), 1, "{text}: {all:?}");
        all.into_iter().next().unwrap()
    }

    #[test]
    fn compiler_style_locations() {
        let rust = only("  --> src/main.rs:12:5");
        assert_eq!(rust.path, "src/main.rs");
        assert_eq!(rust.line, Some(12));
        assert_eq!(rust.column, Some(5));
        assert_eq!(rust.range, 6..22);

        let swift = only("/Users/me/app/Thing.swift:40:9: error: nope");
        assert_eq!(swift.path, "/Users/me/app/Thing.swift");
        assert_eq!(swift.line, Some(40));
        assert_eq!(swift.column, Some(9));

        let grep = only("lib/util.go:7:    return nil");
        assert_eq!(grep.path, "lib/util.go");
        assert_eq!(grep.line, Some(7));
        assert_eq!(grep.column, None);
    }

    #[test]
    fn parenthesized_positions_and_stack_traces() {
        let ts = only("src/app.ts(3,14): error TS2322");
        assert_eq!(ts.path, "src/app.ts");
        assert_eq!(ts.line, Some(3));
        assert_eq!(ts.column, Some(14));

        let node = only("    at run (/srv/app/index.js:10:15)");
        assert_eq!(node.path, "/srv/app/index.js");
        assert_eq!(node.line, Some(10));
        assert_eq!(node.column, Some(15));
    }

    #[test]
    fn python_tracebacks() {
        let python = only(r#"  File "/opt/tool/main.py", line 88, in <module>"#);
        assert_eq!(python.path, "/opt/tool/main.py");
        assert_eq!(python.line, Some(88));
    }

    #[test]
    fn bare_paths_and_punctuation() {
        assert_eq!(only("see docs/README.md.").path, "docs/README.md");
        assert_eq!(only("edit ~/notes/todo.txt now").path, "~/notes/todo.txt");
        assert_eq!(only("cd ../other/dir").path, "../other/dir");
        assert_eq!(only("modified:   Package.swift").path, "Package.swift");
    }

    #[test]
    fn ignores_urls_versions_and_noise() {
        assert!(matches("open https://example.com/a/b.html").is_empty());
        assert!(matches("v1.2.3 took 0.5s").is_empty());
        assert!(matches("... / -- ./").is_empty());
        assert!(matches("plain words only").is_empty());
    }

    #[test]
    fn ranges_are_byte_offsets() {
        let found = only("→ src/main.rs:3");
        assert_eq!(found.range, 4..17);
        assert_eq!(&"→ src/main.rs:3"[found.range], "src/main.rs:3");
    }

    #[test]
    fn lookup_by_offset_and_resolution() {
        let text = "a.rs:1 and b/c.rs:2:3";
        assert_eq!(match_at(text, 0).map(|m| m.path), Some("a.rs".into()));
        assert_eq!(match_at(text, 12).map(|m| m.path), Some("b/c.rs".into()));
        assert_eq!(match_at(text, 8), None);

        assert_eq!(resolve("src/x.rs", "/repo"), "/repo/src/x.rs");
        assert_eq!(resolve("../y", "/repo/sub"), "/repo/y");
        assert_eq!(resolve("/abs/z", "/repo"), "/abs/z");
        let home = dirs::home_dir().unwrap();
        assert_eq!(resolve("~/q", "/repo"), format!("{}/q", home.display()));
    }
}
