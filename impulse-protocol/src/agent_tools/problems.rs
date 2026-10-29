//! The language servers' problems as agents see them (`impulse problems`,
//! `get_diagnostics`): one workspace's, worst first, at most
//! [`ProblemList::LIMIT`].
//!
//! [`Problem`], its counts and its grouping are the Swift ImpulseKit's
//! (`Problems.swift`), ported here until the Problems panel is.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error = 1,
    Warning = 2,
    Info = 3,
    Hint = 4,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "error" => Some(Severity::Error),
            "warning" => Some(Severity::Warning),
            "info" => Some(Severity::Info),
            "hint" => Some(Severity::Hint),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Problem {
    pub path: String,
    /// 1-based.
    pub line: i64,
    pub column: i64,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

impl Problem {
    pub fn new(path: &str, line: i64, column: i64, severity: Severity, message: &str) -> Self {
        Problem {
            path: path.to_string(),
            line,
            column,
            severity,
            message: message.to_string(),
            source: None,
            code: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub errors: i64,
    pub warnings: i64,
    pub others: i64,
}

impl Counts {
    pub fn of(problems: &[&Problem]) -> Self {
        let mut counts = Counts::default();
        for problem in problems {
            match problem.severity {
                Severity::Error => counts.errors += 1,
                Severity::Warning => counts.warnings += 1,
                Severity::Info | Severity::Hint => counts.others += 1,
            }
        }
        counts
    }
}

/// By file (files with errors first, then by path), each sorted by
/// position.
pub fn grouped<'a>(problems: &[&'a Problem]) -> Vec<(String, Vec<&'a Problem>)> {
    let mut by_path: BTreeMap<&str, Vec<&'a Problem>> = BTreeMap::new();
    for problem in problems {
        by_path.entry(&problem.path).or_default().push(problem);
    }
    let mut groups: Vec<(String, Vec<&'a Problem>)> = by_path
        .into_iter()
        .map(|(path, mut list)| {
            list.sort_by_key(|p| (p.line, p.column, p.severity));
            (path.to_string(), list)
        })
        .collect();
    let worst = |list: &[&Problem]| {
        list.iter()
            .map(|p| p.severity)
            .min()
            .unwrap_or(Severity::Hint)
    };
    groups.sort_by(|a, b| worst(&a.1).cmp(&worst(&b.1)).then_with(|| a.0.cmp(&b.0)));
    groups
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemList {
    pub problems: Vec<ProblemEntry>,
    /// Problems left out past the limit.
    pub truncated: i64,
    /// Everything reported in the workspace (not only what's listed).
    pub counts: Counts,
    /// Files whose diagnostics are current.
    pub checked_files: Vec<String>,
    /// Why there's nothing to report, when nothing ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemEntry {
    pub path: String,
    pub line: i64,
    pub column: i64,
    pub severity: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ProblemList {
    pub const LIMIT: usize = 200;

    /// Problems under `root` (relative paths), worst first, at most
    /// [`ProblemList::LIMIT`]. `path` keeps one file's; `minimum` keeps
    /// that severity and worse.
    pub fn new(
        all: &[Problem],
        root: &str,
        path: Option<&str>,
        minimum: Option<Severity>,
        checked_files: &[String],
        note: Option<String>,
    ) -> Self {
        let root_slash = format!("{root}/");
        let relative =
            |path: &str| -> String { path.strip_prefix(&root_slash).unwrap_or(path).to_string() };
        let inside: Vec<&Problem> = all
            .iter()
            .filter(|p| p.path == root || p.path.starts_with(&root_slash))
            .collect();
        let counts = Counts::of(&inside);
        let chosen: Vec<&Problem> = inside
            .into_iter()
            .filter(|p| path.is_none_or(|path| p.path == path))
            .filter(|p| minimum.is_none_or(|minimum| p.severity <= minimum))
            .collect();
        let mut sorted: Vec<&Problem> = grouped(&chosen)
            .into_iter()
            .flat_map(|(_, list)| list)
            .collect();
        sorted.sort_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.line.cmp(&b.line))
        });
        let problems = sorted
            .iter()
            .take(Self::LIMIT)
            .map(|p| ProblemEntry {
                path: relative(&p.path),
                line: p.line,
                column: p.column,
                severity: p.severity.label().to_string(),
                message: p.message.clone(),
                source: p.source.clone(),
                code: p.code.clone(),
            })
            .collect();
        let mut checked: Vec<String> = checked_files.iter().map(|f| relative(f)).collect();
        checked.sort();
        ProblemList {
            problems,
            truncated: sorted.len().saturating_sub(Self::LIMIT) as i64,
            counts,
            checked_files: checked,
            note,
        }
    }

    pub fn text(&self) -> String {
        if self.problems.is_empty() {
            if let Some(note) = &self.note {
                return note.clone();
            }
            let count = self.checked_files.len();
            return if count == 0 {
                "No language server has checked a file in this workspace yet.".to_string()
            } else {
                format!(
                    "No problems in the {count} file{} the language servers checked.",
                    if count == 1 { "" } else { "s" }
                )
            };
        }
        let plural = |n: i64| if n == 1 { "" } else { "s" };
        let mut out = format!(
            "{} error{}, {} warning{}\n",
            self.counts.errors,
            plural(self.counts.errors),
            self.counts.warnings,
            plural(self.counts.warnings)
        );
        for problem in &self.problems {
            let mut line = format!(
                "{}:{}:{} {}",
                problem.path, problem.line, problem.column, problem.severity
            );
            if let Some(source) = problem.source.as_deref().filter(|s| !s.is_empty()) {
                let code = problem
                    .code
                    .as_ref()
                    .map(|code| format!(" {code}"))
                    .unwrap_or_default();
                line += &format!(" [{source}{code}]");
            }
            out += &format!("{line}: {}\n", problem.message.replace('\n', " "));
        }
        if self.truncated > 0 {
            out += &format!("…and {} more.\n", self.truncated);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Severity; 4] = [
        Severity::Error,
        Severity::Warning,
        Severity::Info,
        Severity::Hint,
    ];

    fn list(all: &[Problem]) -> ProblemList {
        ProblemList::new(all, "/w", None, None, &[], None)
    }

    fn positions(list: &ProblemList) -> Vec<(&str, i64, i64, &str)> {
        list.problems
            .iter()
            .map(|p| (p.path.as_str(), p.line, p.column, p.severity.as_str()))
            .collect()
    }

    #[test]
    fn severity_labels_round_trip() {
        for severity in ALL {
            assert_eq!(Severity::from_label(severity.label()), Some(severity));
        }
        assert_eq!(Severity::from_label("fatal"), None);
        assert_eq!(Severity::from_label("Error"), None);
        assert_eq!(Severity::from_label(""), None);
    }

    #[test]
    fn severities_order_worst_first() {
        let mut shuffled = [
            Severity::Hint,
            Severity::Error,
            Severity::Info,
            Severity::Warning,
        ];
        shuffled.sort();
        assert_eq!(shuffled, ALL);
    }

    #[test]
    fn counts_put_info_and_hints_together() {
        let problems: Vec<Problem> = [
            Severity::Error,
            Severity::Error,
            Severity::Warning,
            Severity::Info,
            Severity::Hint,
        ]
        .into_iter()
        .map(|severity| Problem::new("/w/a.rs", 1, 1, severity, "m"))
        .collect();
        let refs: Vec<&Problem> = problems.iter().collect();
        assert_eq!(
            Counts::of(&refs),
            Counts {
                errors: 2,
                warnings: 1,
                others: 2
            }
        );
        assert_eq!(Counts::of(&[]), Counts::default());
    }

    #[test]
    fn grouped_puts_files_with_the_worst_problems_first() {
        let problems = [
            Problem::new("/w/c.rs", 1, 1, Severity::Warning, "w"),
            Problem::new("/w/a.rs", 5, 1, Severity::Hint, "h"),
            Problem::new("/w/b.rs", 2, 1, Severity::Warning, "w"),
            Problem::new("/w/a.rs", 9, 1, Severity::Error, "e"),
            Problem::new("/w/d.rs", 1, 1, Severity::Info, "i"),
        ];
        let refs: Vec<&Problem> = problems.iter().collect();
        let groups = grouped(&refs);
        let paths: Vec<&str> = groups.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, ["/w/a.rs", "/w/b.rs", "/w/c.rs", "/w/d.rs"]);
        let lines: Vec<i64> = groups[0].1.iter().map(|p| p.line).collect();
        assert_eq!(lines, [5, 9]);
        assert!(grouped(&[]).is_empty());
    }

    #[test]
    fn grouped_sorts_a_file_by_line_column_then_severity() {
        let problems = [
            Problem::new("/w/a.rs", 2, 7, Severity::Error, "later column"),
            Problem::new("/w/a.rs", 2, 3, Severity::Hint, "same place, hint"),
            Problem::new("/w/a.rs", 1, 9, Severity::Hint, "first line"),
            Problem::new("/w/a.rs", 2, 3, Severity::Warning, "same place, warning"),
        ];
        let refs: Vec<&Problem> = problems.iter().collect();
        let groups = grouped(&refs);
        assert_eq!(groups.len(), 1);
        let messages: Vec<&str> = groups[0].1.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "first line",
                "same place, warning",
                "same place, hint",
                "later column"
            ]
        );
    }

    #[test]
    fn list_is_worst_first_then_by_path_line_and_column() {
        let problems = [
            Problem::new("/w/b.rs", 4, 1, Severity::Hint, "h"),
            Problem::new("/w/b.rs", 2, 1, Severity::Error, "e"),
            Problem::new("/w/a.rs", 8, 1, Severity::Warning, "w"),
            Problem::new("/w/a.rs", 3, 1, Severity::Error, "e"),
            Problem::new("/w/a.rs", 3, 9, Severity::Warning, "w"),
            Problem::new("/w/a.rs", 3, 2, Severity::Warning, "w"),
            Problem::new("/w/c.rs", 1, 1, Severity::Info, "i"),
        ];
        assert_eq!(
            positions(&list(&problems)),
            [
                ("a.rs", 3, 1, "error"),
                ("b.rs", 2, 1, "error"),
                ("a.rs", 3, 2, "warning"),
                ("a.rs", 3, 9, "warning"),
                ("a.rs", 8, 1, "warning"),
                ("c.rs", 1, 1, "info"),
                ("b.rs", 4, 1, "hint"),
            ]
        );
    }

    #[test]
    fn list_keeps_only_the_root_and_makes_paths_relative() {
        let problems = [
            Problem::new("/w/src/a.rs", 1, 1, Severity::Error, "inside"),
            Problem::new("/wide/a.rs", 1, 1, Severity::Error, "sibling"),
            Problem::new("/elsewhere/a.rs", 1, 1, Severity::Error, "outside"),
            Problem::new("/w", 1, 1, Severity::Warning, "the root itself"),
        ];
        let list = list(&problems);
        // The root itself is kept, but its path stays absolute.
        assert_eq!(
            positions(&list),
            [("src/a.rs", 1, 1, "error"), ("/w", 1, 1, "warning")]
        );
        assert_eq!(list.counts.errors, 1);
    }

    #[test]
    fn filters_narrow_the_list_but_not_the_counts() {
        let problems = [
            Problem::new("/w/a.rs", 1, 1, Severity::Error, "e"),
            Problem::new("/w/a.rs", 2, 1, Severity::Warning, "w"),
            Problem::new("/w/a.rs", 3, 1, Severity::Info, "i"),
            Problem::new("/w/b.rs", 1, 1, Severity::Hint, "h"),
        ];
        let full = Counts {
            errors: 1,
            warnings: 1,
            others: 2,
        };
        let one_file = ProblemList::new(&problems, "/w", Some("/w/a.rs"), None, &[], None);
        assert_eq!(one_file.problems.len(), 3);
        assert_eq!(one_file.counts, full);
        let warnings_up =
            ProblemList::new(&problems, "/w", None, Some(Severity::Warning), &[], None);
        assert_eq!(
            positions(&warnings_up),
            [("a.rs", 1, 1, "error"), ("a.rs", 2, 1, "warning")]
        );
        assert_eq!(warnings_up.counts, full);
        // `path` is matched against the absolute path, so a relative one finds nothing.
        let relative = ProblemList::new(&problems, "/w", Some("a.rs"), None, &[], None);
        assert!(relative.problems.is_empty());
    }

    #[test]
    fn the_limit_cuts_the_list_and_reports_the_rest() {
        let problems: Vec<Problem> = (0..ProblemList::LIMIT as i64 + 3)
            .map(|line| Problem::new("/w/a.rs", line + 1, 1, Severity::Warning, "w"))
            .chain([Problem::new("/w/z.rs", 1, 1, Severity::Error, "e")])
            .collect();
        let cut = list(&problems);
        assert_eq!(cut.problems.len(), ProblemList::LIMIT);
        assert_eq!(cut.truncated, 4);
        assert_eq!(cut.problems[0].path, "z.rs");
        assert_eq!(
            cut.problems.last().unwrap().line,
            ProblemList::LIMIT as i64 - 1
        );
        assert_eq!(
            cut.counts,
            Counts {
                errors: 1,
                warnings: ProblemList::LIMIT as i64 + 3,
                others: 0
            }
        );
        let exactly: Vec<Problem> = (0..ProblemList::LIMIT as i64)
            .map(|line| Problem::new("/w/a.rs", line + 1, 1, Severity::Hint, "h"))
            .collect();
        let full = list(&exactly);
        assert_eq!(full.problems.len(), ProblemList::LIMIT);
        assert_eq!(full.truncated, 0);
        assert!(!full.text().contains("more."));
    }

    #[test]
    fn truncated_counts_only_what_the_filters_kept() {
        let problems: Vec<Problem> = (0..ProblemList::LIMIT as i64 + 2)
            .map(|line| Problem::new("/w/a.rs", line + 1, 1, Severity::Error, "e"))
            .chain((0..10).map(|line| Problem::new("/w/b.rs", line + 1, 1, Severity::Hint, "h")))
            .collect();
        let list = ProblemList::new(&problems, "/w", None, Some(Severity::Error), &[], None);
        assert_eq!(list.truncated, 2);
        assert_eq!(list.counts.others, 10);
    }

    #[test]
    fn checked_files_are_relative_and_sorted() {
        let checked = [
            "/w/z.rs".to_string(),
            "/w/src/a.rs".to_string(),
            "/elsewhere/b.rs".to_string(),
        ];
        let list = ProblemList::new(&[], "/w", None, None, &checked, None);
        // Files outside the root aren't dropped, and keep their absolute path.
        assert_eq!(list.checked_files, ["/elsewhere/b.rs", "src/a.rs", "z.rs"]);
    }

    #[test]
    fn nothing_to_report_says_why() {
        assert_eq!(
            list(&[]).text(),
            "No language server has checked a file in this workspace yet."
        );
        let one = ProblemList::new(&[], "/w", None, None, &["/w/a.rs".into()], None);
        assert_eq!(
            one.text(),
            "No problems in the 1 file the language servers checked."
        );
        let two = ProblemList::new(
            &[],
            "/w",
            None,
            None,
            &["/w/a.rs".into(), "/w/b.rs".into()],
            None,
        );
        assert_eq!(
            two.text(),
            "No problems in the 2 files the language servers checked."
        );
        let noted = ProblemList::new(
            &[Problem::new("/w/a.rs", 1, 1, Severity::Error, "e")],
            "/w",
            None,
            None,
            &[],
            Some("A note.".into()),
        );
        assert!(noted.text().starts_with("1 error, 0 warnings\n"));
        assert!(!noted.text().contains("A note."));
    }

    #[test]
    fn text_lists_each_problem_on_one_line() {
        let problems = [
            Problem {
                source: Some("rustc".into()),
                code: Some("E0308".into()),
                ..Problem::new("/w/a.rs", 3, 5, Severity::Error, "mismatched\ntypes")
            },
            Problem {
                source: Some("clippy".into()),
                ..Problem::new("/w/a.rs", 4, 1, Severity::Warning, "needless")
            },
            Problem {
                source: Some(String::new()),
                code: Some("x".into()),
                ..Problem::new("/w/b.rs", 1, 1, Severity::Warning, "empty source")
            },
            Problem {
                code: Some("y".into()),
                ..Problem::new("/w/c.rs", 1, 1, Severity::Warning, "code alone")
            },
        ];
        assert_eq!(
            list(&problems).text(),
            "1 error, 3 warnings\n\
             a.rs:3:5 error [rustc E0308]: mismatched types\n\
             a.rs:4:1 warning [clippy]: needless\n\
             b.rs:1:1 warning: empty source\n\
             c.rs:1:1 warning: code alone\n"
        );
    }

    #[test]
    fn serializes_in_camel_case_leaving_out_what_is_missing() {
        let problems = [
            Problem {
                source: Some("ts".into()),
                code: Some("2322".into()),
                ..Problem::new("/w/a.ts", 9, 2, Severity::Error, "type")
            },
            Problem::new("/w/b.ts", 3, 1, Severity::Hint, "unused"),
        ];
        let list = ProblemList::new(&problems, "/w", None, None, &["/w/a.ts".into()], None);
        let json = serde_json::to_value(&list).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "problems": [
                    {
                        "path": "a.ts",
                        "line": 9,
                        "column": 2,
                        "severity": "error",
                        "message": "type",
                        "source": "ts",
                        "code": "2322"
                    },
                    {
                        "path": "b.ts",
                        "line": 3,
                        "column": 1,
                        "severity": "hint",
                        "message": "unused"
                    }
                ],
                "truncated": 0,
                "counts": { "errors": 1, "warnings": 0, "others": 1 },
                "checkedFiles": ["a.ts"]
            })
        );
        assert_eq!(serde_json::from_value::<ProblemList>(json).unwrap(), list);
        let noted = ProblemList {
            note: Some("Not trusted.".into()),
            ..ProblemList::default()
        };
        let json = serde_json::to_value(&noted).unwrap();
        assert_eq!(json["note"], "Not trusted.");
        assert_eq!(serde_json::from_value::<ProblemList>(json).unwrap(), noted);
    }
}
