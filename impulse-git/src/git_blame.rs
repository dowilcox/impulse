//! Whole-file blame (the git CLI's porcelain format), and the "base" text an
//! editor diffs its buffer against (the index version).

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::operations::{git_with, Call};
use crate::raw::{canonical_path, open_repo, unix_time, workdir};
use crate::MAX_DIFF_CONTENT_SIZE;

/// Who last changed a line.
#[derive(Clone, Debug, PartialEq)]
pub struct BlameLine {
    pub sha: String,
    pub author: String,
    pub author_time: SystemTime,
    pub summary: String,
}

impl BlameLine {
    /// A line not committed yet (git reports the zero id).
    pub fn is_uncommitted(&self) -> bool {
        self.sha.chars().all(|c| c == '0')
    }
}

/// `git blame --porcelain` output by final line number.
pub fn parse_porcelain(text: &str) -> HashMap<usize, BlameLine> {
    #[derive(Clone)]
    struct CommitInfo {
        author: String,
        time: SystemTime,
        summary: String,
    }
    let empty = CommitInfo {
        author: String::new(),
        time: SystemTime::UNIX_EPOCH,
        summary: String::new(),
    };
    let mut commits: HashMap<String, CommitInfo> = HashMap::new();
    let mut result = HashMap::new();
    let mut current: Option<(String, usize)> = None;

    for raw_line in text.split('\n') {
        if raw_line.starts_with('\t') {
            // A content line ends the entry.
            if let Some((sha, line)) = current.take() {
                let info = commits.get(&sha).cloned().unwrap_or_else(|| empty.clone());
                result.insert(
                    line,
                    BlameLine {
                        sha,
                        author: info.author,
                        author_time: info.time,
                        summary: info.summary,
                    },
                );
            }
            continue;
        }
        let parts: Vec<&str> = raw_line.split(' ').filter(|p| !p.is_empty()).collect();
        if parts.len() >= 3
            && parts[0].len() >= 40
            && parts[0].chars().all(|c| c.is_ascii_hexdigit())
        {
            if let Ok(final_line) = parts[2].parse::<usize>() {
                commits
                    .entry(parts[0].to_string())
                    .or_insert_with(|| empty.clone());
                current = Some((parts[0].to_string(), final_line));
                continue;
            }
        }
        let Some((sha, _)) = &current else {
            continue;
        };
        let Some(info) = commits.get_mut(sha) else {
            continue;
        };
        if let Some(author) = raw_line.strip_prefix("author ") {
            info.author = author.to_string();
        } else if let Some(seconds) = raw_line.strip_prefix("author-time ") {
            if let Ok(seconds) = seconds.parse::<f64>() {
                info.time = unix_time(seconds);
            }
        } else if let Some(summary) = raw_line.strip_prefix("summary ") {
            info.summary = summary.to_string();
        }
    }
    result
}

/// Blame of the file as saved on disk, by line. Empty on failure
/// (untracked, binary).
pub fn blame(path: &str, root: &str) -> HashMap<usize, BlameLine> {
    match git_with(
        &["blame", "--porcelain", "--no-textconv", "--", path],
        root,
        Call {
            timeout: Duration::from_secs(30),
            ..Default::default()
        },
    ) {
        Ok(result) => parse_porcelain(&result.stdout),
        Err(_) => HashMap::new(),
    }
}

/// What the editor diffs its buffer against: the file's index version
/// (what's staged, or HEAD's when nothing is). "" for an untracked file
/// (every line is new); `None` when it isn't in a repository, is ignored, or
/// is binary.
pub fn base_content(absolute_path: &str) -> Option<String> {
    let repo = open_repo(absolute_path).ok()?;
    let workdir = workdir(&repo).ok()?;
    let prefix = if workdir.ends_with('/') {
        workdir
    } else {
        format!("{workdir}/")
    };
    let canonical = canonical_path(absolute_path).unwrap_or_else(|| absolute_path.to_string());
    let relative = canonical.strip_prefix(&prefix)?;

    let index = repo.index().ok()?;
    let Some(entry) = index.get_path(Path::new(relative), 0) else {
        // Not in the index: untracked (all new) unless ignored.
        if repo.is_path_ignored(relative).unwrap_or(false) {
            return None;
        }
        return Some(String::new());
    };
    let blob = repo.find_blob(entry.id).ok()?;
    if blob.is_binary() || blob.size() as u64 > MAX_DIFF_CONTENT_SIZE {
        return None;
    }
    String::from_utf8(blob.content().to_vec()).ok()
}
