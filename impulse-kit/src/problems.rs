//! Diagnostics from every language server, across files (the Swift
//! `Problems`): the Problems panel's model (filter, group, count) and the
//! prompt it hands an agent.

use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error = 1,
    Warning = 2,
    Info = 3,
    Hint = 4,
}

impl Severity {
    pub const ALL: [Severity; 4] = [Self::Error, Self::Warning, Self::Info, Self::Hint];

    /// From an LSP `DiagnosticSeverity` number.
    pub fn from_lsp(value: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|s| *s as i64 == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Hint => "hint",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Problem {
    pub path: String,
    /// 1-based.
    pub line: usize,
    pub column: usize,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProblemCounts {
    pub errors: usize,
    pub warnings: usize,
    pub others: usize,
}

impl ProblemCounts {
    pub fn total(&self) -> usize {
        self.errors + self.warnings + self.others
    }
}

pub fn counts(problems: &[Problem]) -> ProblemCounts {
    let mut counts = ProblemCounts::default();
    for problem in problems {
        match problem.severity {
            Severity::Error => counts.errors += 1,
            Severity::Warning => counts.warnings += 1,
            Severity::Info | Severity::Hint => counts.others += 1,
        }
    }
    counts
}

/// Problems with one of `severities` whose message, source, code or path
/// contains `text` (case-insensitive).
pub fn filter(problems: &[Problem], severities: &HashSet<Severity>, text: &str) -> Vec<Problem> {
    let query = text.trim_matches([' ', '\t']).to_lowercase();
    let contains = |field: &str| field.to_lowercase().contains(&query);
    problems
        .iter()
        .filter(|p| severities.contains(&p.severity))
        .filter(|p| {
            query.is_empty()
                || contains(&p.message)
                || contains(&p.path)
                || p.source.as_deref().is_some_and(contains)
                || p.code.as_deref().is_some_and(contains)
        })
        .cloned()
        .collect()
}

/// By file (files with errors first, then by path), each sorted by
/// position.
pub fn grouped(problems: &[Problem]) -> Vec<(String, Vec<Problem>)> {
    let mut by_path: BTreeMap<&str, Vec<Problem>> = BTreeMap::new();
    for problem in problems {
        by_path
            .entry(problem.path.as_str())
            .or_default()
            .push(problem.clone());
    }
    let mut groups: Vec<(String, Vec<Problem>)> = by_path
        .into_iter()
        .map(|(path, mut list)| {
            list.sort_by_key(|p| (p.line, p.column, p.severity));
            (path.to_string(), list)
        })
        .collect();
    let worst = |list: &[Problem]| {
        list.iter()
            .map(|p| p.severity)
            .min()
            .unwrap_or(Severity::Hint)
    };
    groups.sort_by(|a, b| worst(&a.1).cmp(&worst(&b.1)).then_with(|| a.0.cmp(&b.0)));
    groups
}

/// A prompt asking an agent to fix these, paths relative to `root`, at
/// most `limit` listed.
pub fn prompt(problems: &[Problem], root: Option<&str>, limit: usize) -> String {
    if problems.is_empty() {
        return String::new();
    }
    let relative = |path: &str| -> String {
        root.and_then(|root| path.strip_prefix(root))
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(path)
            .to_string()
    };
    let shown: Vec<Problem> = grouped(problems)
        .into_iter()
        .flat_map(|(_, list)| list)
        .take(limit)
        .collect();
    let mut out = String::from("Please fix these problems reported by the language servers:\n\n");
    for problem in &shown {
        let mut line = format!(
            "- {}:{}:{} {}",
            relative(&problem.path),
            problem.line,
            problem.column,
            problem.severity.label()
        );
        if let Some(source) = problem.source.as_deref().filter(|s| !s.is_empty()) {
            let code = problem
                .code
                .as_deref()
                .map(|c| format!(" {c}"))
                .unwrap_or_default();
            line.push_str(&format!(" [{source}{code}]"));
        }
        out.push_str(&line);
        out.push_str(": ");
        out.push_str(&problem.message.replace('\n', " "));
        out.push('\n');
    }
    if problems.len() > shown.len() {
        out.push_str(&format!("\n…and {} more.\n", problems.len() - shown.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(
        path: &str,
        line: usize,
        column: usize,
        severity: Severity,
        message: &str,
        source: Option<&str>,
        code: Option<&str>,
    ) -> Problem {
        Problem {
            path: path.into(),
            line,
            column,
            severity,
            message: message.into(),
            source: source.map(String::from),
            code: code.map(String::from),
        }
    }

    fn problems() -> Vec<Problem> {
        vec![
            problem(
                "/r/b.swift",
                9,
                2,
                Severity::Warning,
                "unused variable 'x'",
                Some("sourcekit"),
                None,
            ),
            problem(
                "/r/a.swift",
                3,
                1,
                Severity::Info,
                "consider let",
                None,
                None,
            ),
            problem(
                "/r/b.swift",
                2,
                5,
                Severity::Error,
                "cannot find 'foo'",
                Some("sourcekit"),
                Some("E1"),
            ),
            problem(
                "/r/c.ts",
                1,
                1,
                Severity::Hint,
                "line one\nline two",
                None,
                None,
            ),
        ]
    }

    fn all() -> HashSet<Severity> {
        Severity::ALL.into_iter().collect()
    }

    #[test]
    fn counts_by_severity() {
        assert_eq!(
            counts(&problems()),
            ProblemCounts {
                errors: 1,
                warnings: 1,
                others: 2
            }
        );
        assert_eq!(counts(&[]).total(), 0);
    }

    #[test]
    fn filters_by_severity_and_text() {
        let problems = problems();
        let errors = HashSet::from([Severity::Error]);
        assert_eq!(
            filter(&problems, &errors, "")
                .iter()
                .map(|p| p.message.as_str())
                .collect::<Vec<_>>(),
            vec!["cannot find 'foo'"]
        );
        assert_eq!(filter(&problems, &all(), "SOURCEKIT").len(), 2);
        assert_eq!(filter(&problems, &all(), "c.ts").len(), 1);
        assert!(filter(&problems, &HashSet::from([Severity::Warning]), "foo").is_empty());
    }

    #[test]
    fn groups_worst_files_first_sorted_by_position() {
        let groups = grouped(&problems());
        assert_eq!(
            groups.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(),
            vec!["/r/b.swift", "/r/a.swift", "/r/c.ts"]
        );
        assert_eq!(
            groups[0].1.iter().map(|p| p.line).collect::<Vec<_>>(),
            vec![2, 9]
        );
    }

    #[test]
    fn prompt_is_relative_and_capped() {
        let problems = problems();
        let text = prompt(&problems, Some("/r"), 2);
        assert!(text.contains("- b.swift:2:5 error [sourcekit E1]: cannot find 'foo'"));
        assert!(text.contains("- b.swift:9:2 warning [sourcekit]: unused variable 'x'"));
        assert!(!text.contains("a.swift"), "capped at two");
        assert!(text.contains("…and 2 more."));
        assert!(prompt(&[], None, 100).is_empty());
        assert!(prompt(&problems[3..], None, 100).contains("line one line two"));
    }
}
