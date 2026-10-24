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
