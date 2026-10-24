//! Commit history for the History surface, read with `git log` in pages
//! (the Swift `GitLog`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::history_query::HistoryQuery;
use crate::operations::{git, git_with, literal, Call, GitResult};
use crate::raw::unix_time;
use crate::scoped_diff::default_base_branch;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub date: SystemTime,
    pub subject: String,
    /// Decorations: "HEAD -> main", "origin/main", "tag: v1.0".
    pub refs: Vec<String>,
}

impl LogEntry {
    pub fn short_sha(&self) -> &str {
        &self.sha[..self.sha.len().min(7)]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LogScope {
    /// HEAD's history.
    Head,
    /// Every branch, remote branch and tag (not Impulse's private refs).
    All,
    /// One local branch's history.
    Branch(String),
}

impl LogScope {
    fn revisions(&self) -> Vec<String> {
        match self {
            LogScope::Head => vec!["HEAD".into()],
            LogScope::All => ["--branches", "--tags", "--remotes", "HEAD"]
                .map(String::from)
                .to_vec(),
            LogScope::Branch(name) => vec![format!("refs/heads/{name}")],
        }
    }
}

fn sixty_seconds() -> Call<'static> {
    Call {
        timeout: Duration::from_secs(60),
        ..Default::default()
    }
}

/// A page of history, newest first in topological order. `path` limits it
/// to commits touching that path (following renames for a single file);
/// `query` adds author and date limits and a path of its own.
pub fn entries(
    root: &str,
    scope: &LogScope,
    path: Option<&str>,
    query: &HistoryQuery,
    skip: usize,
    limit: usize,
) -> GitResult<Vec<LogEntry>> {
    let mut args: Vec<String> = [
        "log",
        "--topo-order",
        "--no-color",
        "--decorate=short",
        "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1e",
    ]
    .map(String::from)
    .to_vec();
    args.push(format!("--skip={skip}"));
    args.extend(["-n".to_string(), limit.max(1).to_string()]);
    args.extend(query.git_arguments());
    args.extend(scope.revisions());
    if let Some(path) = path {
        // --follow only works for a single file; on a folder it follows
        // whichever file git happens to pick.
        if *scope != LogScope::All && !is_directory(path, root) {
            args.insert(1, "--follow".into());
        }
        args.extend(["--".to_string(), path.to_string()]);
    } else if let Some(path) = &query.path {
        args.extend(["--".to_string(), path.clone()]);
    } else {
        // Revisions only, even when a file has the same name.
        args.push("--".into());
    }
    git_with(&args, root, sixty_seconds()).map(|r| parse(&r.stdout))
}

/// Each decorated commit's refs, as `entries` shows them, read from the ref
/// tips alone: when only refs moved, loaded history takes these instead of
/// being read again.
pub fn decorations(root: &str) -> GitResult<HashMap<String, Vec<String>>> {
    let result = git_with(
        &[
            "log",
            "--no-walk=unsorted",
            "--no-color",
            "--decorate=short",
            "--format=%H%x1f%D%x1e",
            "--all",
            "--",
        ],
        root,
        sixty_seconds(),
    )?;
    let mut refs = HashMap::new();
    for record in result.stdout.split('\u{1e}') {
        let fields: Vec<&str> = record.trim().split('\u{1f}').collect();
        if fields.len() >= 2 && !fields[1].is_empty() {
            refs.insert(fields[0].to_string(), parse_refs(fields[1]));
        }
    }
    Ok(refs)
}

/// The commits (and tag objects) `scope`'s history starts from, sorted.
/// While they're the same, so is the history `entries` reads, and only
/// decorations can have moved. `None` when they can't be read.
pub fn tips(root: &str, scope: &LogScope) -> Option<Vec<String>> {
    let mut args = vec!["rev-parse".to_string()];
    args.extend(scope.revisions());
    args.push("--".into());
    let result = git(&args, root).ok()?;
    let names: BTreeSet<String> = result
        .stdout
        .split('\n')
        .filter(|l| !l.is_empty() && *l != "--")
        .map(str::to_string)
        .collect();
    Some(names.into_iter().collect())
}

pub fn parse_refs(field: &str) -> Vec<String> {
    field
        .split(", ")
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .collect()
}

/// `path` is a folder: on disk, or (once deleted) in history; a pathspec
/// ending in "/" only matches folders.
fn is_directory(path: &str, root: &str) -> bool {
    if path.ends_with('/') || path.is_empty() || path == "." {
        return true;
    }
    if let Ok(meta) = std::fs::metadata(Path::new(root).join(path)) {
        return meta.is_dir();
    }
    git(
        &literal(&["rev-list", "-1", "HEAD", "--", &format!("{path}/")]),
        root,
    )
    .is_ok_and(|r| !r.stdout.trim().is_empty())
}

/// Everything about one commit, for its details header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitDetails {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub date: SystemTime,
    pub committer: String,
    pub committer_date: SystemTime,
    /// Subject and body.
    pub message: String,
}

impl CommitDetails {
    pub fn subject(&self) -> &str {
        self.message.split('\n').next().unwrap_or("")
    }

    /// The message after the subject line.
    pub fn body(&self) -> String {
        self.message
            .split('\n')
            .skip(1)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }
}

pub fn details(root: &str, sha: &str) -> Option<CommitDetails> {
    let result = git(
        &[
            "show",
            "-s",
            "--no-color",
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ct%x1f%B",
            sha,
        ],
        root,
    )
    .ok()?;
    let fields: Vec<&str> = result.stdout.split('\u{1f}').collect();
    if fields.len() < 8 {
        return None;
    }
    Some(CommitDetails {
        sha: fields[0].into(),
        parents: fields[1]
            .split(' ')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
        author: fields[2].into(),
        email: fields[3].into(),
        date: unix_time(fields[4].parse().unwrap_or(0.0)),
        committer: fields[5].into(),
        committer_date: unix_time(fields[6].parse().unwrap_or(0.0)),
        message: fields[7..].join("\u{1f}").trim().to_string(),
    })
}

/// Commits on HEAD that its upstream doesn't have yet (outgoing) and the
/// other way round (incoming). Both empty without an upstream.
pub fn divergence(root: &str, limit: usize) -> (HashSet<String>, HashSet<String>) {
    let Ok(result) = git(
        &[
            "rev-list",
            "--left-right",
            &format!("--max-count={limit}"),
            "HEAD...@{upstream}",
        ],
        root,
    ) else {
        return (HashSet::new(), HashSet::new());
    };
    let mut outgoing = HashSet::new();
    let mut incoming = HashSet::new();
    for line in result.stdout.split('\n') {
        if let Some(sha) = line.strip_prefix('<') {
            outgoing.insert(sha.to_string());
        } else if let Some(sha) = line.strip_prefix('>') {
            incoming.insert(sha.to_string());
        }
    }
    (outgoing, incoming)
}

/// `user.name` for this repository ("My Commits").
pub fn user_name(root: &str) -> Option<String> {
    let result = git(&["config", "user.name"], root).ok()?;
    Some(result.stdout.trim().to_string()).filter(|s| !s.is_empty())
}

/// Where the current branch left the default branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkPoint {
    /// The default branch ("origin/main").
    pub base: String,
    /// The merge base of HEAD and `base`.
    pub sha: String,
    /// Commits on HEAD that `base` doesn't have.
    pub branch_only: HashSet<String>,
}

/// HEAD's fork point from the default branch; `None` on the default branch
/// itself, without one, or when the branch is too long to list.
pub fn fork_point(root: &str, limit: usize) -> Option<ForkPoint> {
    let base = default_base_branch(root)?;
    let head = git(&["rev-parse", "--abbrev-ref", "HEAD"], root).ok()?;
    let branch = head.stdout.trim();
    if branch == base || base.ends_with(&format!("/{branch}")) {
        return None;
    }
    let merge_base = git(&["merge-base", "HEAD", &base], root).ok()?;
    let list = git(
        &[
            "rev-list",
            &format!("--max-count={}", limit + 1),
            "HEAD",
            &format!("^{base}"),
        ],
        root,
    )
    .ok()?;
    let sha = merge_base.stdout.trim().to_string();
    let commits: Vec<String> = list
        .stdout
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if sha.is_empty() || commits.len() > limit {
        return None;
    }
    Some(ForkPoint {
        base,
        sha,
        branch_only: commits.into_iter().collect(),
    })
}

fn parse(output: &str) -> Vec<LogEntry> {
    output
        .split('\u{1e}')
        .filter_map(|record| {
            let fields: Vec<&str> = record.trim().split('\u{1f}').collect();
            if fields.len() < 7 || fields[0].len() < 7 {
                return None;
            }
            Some(LogEntry {
                sha: fields[0].into(),
                parents: fields[1]
                    .split(' ')
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect(),
                author: fields[2].into(),
                email: fields[3].into(),
                date: unix_time(fields[4].parse().unwrap_or(0.0)),
                subject: fields[5].into(),
                refs: parse_refs(fields[6]),
            })
        })
        .collect()
}
