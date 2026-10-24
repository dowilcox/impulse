//! Project search by file name and by contents (the original Rust's
//! `search.rs`, as the Swift `FileSearch` ported it). The walk is a plain
//! depth-first one that asks libgit2's ignore engine about each path: the
//! semantics git itself uses (nested .gitignore files, negations, global
//! excludes, info/exclude). Outside a git repository nothing is ignored (the
//! original `ignore` crate honored stray .gitignore files even there; that is
//! the one intended difference, kept from the Swift).
//!
//! Hidden entries, symbolic links, other filesystems and anything deeper than
//! `MAX_DEPTH` are skipped, as are files over 1 MB or with a NUL byte in the
//! first 8 KB when searching contents. Columns count characters.

use std::path::Path;

use git2::{Repository, RepositoryOpenFlags};
use serde::{Deserialize, Serialize};

const MAX_DEPTH: usize = 15;
const MAX_CONTENT_FILE_SIZE: u64 = 1_048_576;
const BINARY_SNIFF_BYTES: usize = 8192;
const MAX_LINE_CONTENT_CHARS: usize = 500;

/// The original Rust's `SearchResult`, keys and all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_end: Option<u32>,
    /// "file" or "content".
    pub match_type: String,
}

/// Files whose name contains `query`, ignoring case.
pub fn search_filenames(root: &str, query: &str, limit: usize) -> Vec<SearchResult> {
    let query = query.to_lowercase();
    let mut results = Vec::new();
    walk(root, &mut |path, name, is_directory| {
        if results.len() >= limit {
            return false;
        }
        if !is_directory && name.to_lowercase().contains(&query) {
            results.push(SearchResult {
                path: path.into(),
                name: name.into(),
                line_number: None,
                line_content: None,
                column_start: None,
                column_end: None,
                match_type: "file".into(),
            });
        }
        true
    });
    results
}

/// The lines of a searchable file: `None` when it's too large, unreadable or
/// binary. Lines that aren't UTF-8 are `None` (and skipped, as
/// `BufRead::lines` skipped them).
fn searchable_lines(path: &str) -> Option<Vec<Option<String>>> {
    if std::fs::metadata(path).ok()?.len() > MAX_CONTENT_FILE_SIZE {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    if data[..data.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
        return None;
    }
    Some(
        data.split(|&b| b == b'\n')
            .map(|line| {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                String::from_utf8(line.to_vec()).ok()
            })
            .collect(),
    )
}

/// Lines containing `query`, one result per match.
pub fn search_contents(
    root: &str,
    query: &str,
    limit: usize,
    case_sensitive: bool,
) -> Vec<SearchResult> {
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let match_length = query.chars().count();
    let mut results = Vec::new();
    walk(root, &mut |path, name, is_directory| {
        if results.len() >= limit {
            return false;
        }
        if is_directory || needle.is_empty() {
            return true;
        }
        let Some(lines) = searchable_lines(path) else {
            return true;
        };
        for (index, line) in lines.into_iter().enumerate() {
            if results.len() >= limit {
                break;
            }
            let Some(line) = line else {
                continue;
            };
            let haystack = if case_sensitive {
                line.clone()
            } else {
                line.to_lowercase()
            };
            let mut content: Option<String> = None;
            let mut search_from = 0;
            let mut previous_byte = 0;
            let mut previous_column = 0;
            while results.len() < limit {
                let Some(found) = haystack[search_from..].find(&needle) else {
                    break;
                };
                let start = search_from + found;
                // Character offsets, counted on from the previous match.
                let column = previous_column + haystack[previous_byte..start].chars().count();
                previous_byte = start;
                previous_column = column;
                let text = content
                    .get_or_insert_with(|| line.chars().take(MAX_LINE_CONTENT_CHARS).collect())
                    .clone();
                results.push(SearchResult {
                    path: path.into(),
                    name: name.into(),
                    line_number: Some(index as u32 + 1),
                    line_content: Some(text),
                    column_start: Some(column as u32),
                    column_end: Some((column + match_length) as u32),
                    match_type: "content".into(),
                });
                search_from = start + needle.len();
            }
        }
        true
    });
    results
}

/// Every file `search_contents` would report at least one match in, with no
/// limit (for replacing across the project, where the result list's cap
/// would leave files out).
pub fn files_containing(root: &str, query: &str, case_sensitive: bool) -> Vec<String> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let mut paths = Vec::new();
    walk(root, &mut |path, _, is_directory| {
        if is_directory {
            return true;
        }
        let Some(lines) = searchable_lines(path) else {
            return true;
        };
        let found = lines.into_iter().flatten().any(|line| {
            if case_sensitive {
                line.contains(&needle)
            } else {
                line.to_lowercase().contains(&needle)
            }
        });
        if found {
            paths.push(path.to_string());
        }
        true
    });
    paths
}

/// Depth-first walk of `root`, in name order, skipping hidden entries,
/// symbolic links, other filesystems, ignored paths (inside a repository)
/// and anything deeper than `MAX_DEPTH`. `visit` gets the path, the name and
/// whether it's a folder, and returns false to stop the walk.
pub(crate) fn walk(root: &str, visit: &mut dyn FnMut(&str, &str, bool) -> bool) {
    // Canonical once (realpath), so every path below matches libgit2's
    // canonical working directory; symlinked folders are never entered, so
    // children stay canonical.
    let root = crate::raw::canonical_path(root).unwrap_or_else(|| root.to_string());
    let Ok(meta) = std::fs::metadata(&root) else {
        return;
    };
    if !meta.is_dir() {
        return;
    }
    let ignore = IgnoreChecker::new(&root);
    let device = device_of(&meta);
    let mut stopped = false;
    descend(Path::new(&root), 0, device, &ignore, visit, &mut stopped);
}

fn descend(
    dir: &Path,
    depth: usize,
    device: Option<u64>,
    ignore: &IgnoreChecker,
    visit: &mut dyn FnMut(&str, &str, bool) -> bool,
    stopped: &mut bool,
) {
    if *stopped || depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        if *stopped {
            return;
        }
        if name.starts_with('.') {
            continue;
        }
        let path = dir.join(&name);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let is_directory = meta.is_dir();
        if is_directory && device.is_some() && device_of(&meta) != device {
            continue;
        }
        let path = path.to_string_lossy().into_owned();
        if ignore.is_ignored(&path, is_directory) {
            continue;
        }
        if !visit(&path, &name, is_directory) {
            *stopped = true;
            return;
        }
        if is_directory {
            descend(Path::new(&path), depth + 1, device, ignore, visit, stopped);
        }
    }
}

#[cfg(unix)]
fn device_of(meta: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.dev())
}

#[cfg(not(unix))]
fn device_of(_meta: &std::fs::Metadata) -> Option<u64> {
    None
}

/// libgit2's ignore engine for a search root; outside a repository nothing
/// is ignored.
struct IgnoreChecker {
    repo: Option<(Repository, String)>,
}

impl IgnoreChecker {
    fn new(root: &str) -> Self {
        let repo = Repository::open_ext(
            root,
            RepositoryOpenFlags::empty(),
            std::iter::empty::<&str>(),
        )
        .ok()
        .and_then(|repo| {
            let workdir = crate::raw::trim_trailing_slash(&repo.workdir()?.to_string_lossy());
            Some((repo, workdir))
        });
        IgnoreChecker { repo }
    }

    /// `path` must be canonical (derived from the canonical walk root).
    fn is_ignored(&self, path: &str, is_directory: bool) -> bool {
        let Some((repo, workdir)) = &self.repo else {
            return false;
        };
        let Some(rel) = path.strip_prefix(&format!("{workdir}/")) else {
            return false;
        };
        // libgit2 matches folder patterns (`build/`) only for a path marked
        // as a folder.
        let rel = if is_directory {
            format!("{rel}/")
        } else {
            rel.to_string()
        };
        repo.is_path_ignored(rel).unwrap_or(false)
    }
}
