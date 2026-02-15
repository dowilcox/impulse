use serde::{Deserialize, Serialize};
use std::path::Path;

/// Status of a line relative to HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DiffLineStatus {
    Added,
    Modified,
    Unchanged,
}

/// Diff information for a file, mapping line numbers to their status.
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// Map of 1-based line numbers to their diff status. Only changed lines are included.
    pub changed_lines: std::collections::HashMap<u32, DiffLineStatus>,
    /// 1-based line numbers where pure-deletion hunks anchor (the line after the deletion).
    pub deleted_lines: Vec<u32>,
}

/// Blame information for a single line.
#[derive(Debug, Clone)]
pub struct BlameInfo {
    pub author: String,
    pub date: String,
    pub commit_hash: String,
    pub summary: String,
}

/// Get diff status for each line of a file (working tree vs HEAD).
/// Returns changed lines with their status.
pub fn get_file_diff(file_path: &str) -> Result<FileDiff, String> {
    let path = Path::new(file_path);
    let repo = git2::Repository::discover(path).map_err(|e| format!("Not a git repo: {}", e))?;

    let head = match repo.head() {
        Ok(head) => head,
        Err(_) => {
            // No HEAD (empty repo) -- all lines are added
            return Ok(FileDiff {
                changed_lines: std::collections::HashMap::new(),
                deleted_lines: Vec::new(),
            });
        }
    };
    let head_tree = head
        .peel_to_tree()
        .map_err(|e| format!("Failed to get HEAD tree: {}", e))?;

    let mut diff_opts = git2::DiffOptions::new();
    // Make file_path relative to repo root
    let repo_root = repo.workdir().ok_or("Bare repository")?;
    let rel_path = path
        .strip_prefix(repo_root)
        .map_err(|_| "File not in repo".to_string())?;
    diff_opts.pathspec(rel_path.to_string_lossy().as_ref());

    let diff = repo
        .diff_tree_to_workdir(Some(&head_tree), Some(&mut diff_opts))
        .map_err(|e| format!("Diff failed: {}", e))?;

    let mut changed_lines = std::collections::HashMap::new();

    diff.foreach(
        &mut |_, _| true,
        None,
        None,
        Some(&mut |_delta, _hunk, line| {
            let origin = line.origin();
            let new_lineno = line.new_lineno();
            match origin {
                '+' => {
                    if let Some(lineno) = new_lineno {
                        changed_lines.insert(lineno, DiffLineStatus::Added);
                    }
                }
                '-' => {
                    // Removed lines don't have new line numbers
                    // We handle modifications in the second pass below
                }
                _ => {}
            }
            true
        }),
    )
    .map_err(|e| format!("Diff iteration failed: {}", e))?;

    // Second pass: re-classify lines in hunks with both additions and deletions as Modified.
    // Also detect pure-deletion hunks (removed lines with no additions).
    // We track hunk transitions via the hunk header in the line callback to avoid
    // multiple mutable borrow issues with separate hunk_cb + line_cb closures.
    let mut hunk_added: Vec<u32> = Vec::new();
    let mut hunk_removed_count: u32 = 0;
    let mut last_hunk_header: Option<(u32, u32, u32, u32)> = None;
    let mut deleted_lines: Vec<u32> = Vec::new();

    let classify_hunk = |added: &mut Vec<u32>,
                         removed: &mut u32,
                         lines: &mut std::collections::HashMap<u32, DiffLineStatus>,
                         deleted: &mut Vec<u32>,
                         hunk_header: &Option<(u32, u32, u32, u32)>| {
        if !added.is_empty() && *removed > 0 {
            let modify_count = added.len().min(*removed as usize);
            for &lineno in added.iter().take(modify_count) {
                lines.insert(lineno, DiffLineStatus::Modified);
            }
        } else if added.is_empty() && *removed > 0 {
            // Pure deletion hunk — anchor at the new-file line where the deletion occurred
            if let Some((_, _, new_start, _)) = hunk_header {
                deleted.push(*new_start);
            }
        }
        added.clear();
        *removed = 0;
    };

    diff.foreach(
        &mut |_, _| true,
        None,
        None,
        Some(&mut |_delta, hunk, line| {
            // Detect hunk transitions by comparing hunk header values
            let current_hunk =
                hunk.map(|h| (h.old_start(), h.old_lines(), h.new_start(), h.new_lines()));
            if current_hunk != last_hunk_header {
                // New hunk - classify previous hunk's collected lines
                classify_hunk(
                    &mut hunk_added,
                    &mut hunk_removed_count,
                    &mut changed_lines,
                    &mut deleted_lines,
                    &last_hunk_header,
                );
                last_hunk_header = current_hunk;
            }

            match line.origin() {
                '+' => {
                    if let Some(lineno) = line.new_lineno() {
                        hunk_added.push(lineno);
                    }
                }
                '-' => {
                    hunk_removed_count += 1;
                }
                _ => {}
            }
            true
        }),
    )
    .map_err(|e| format!("Hunk analysis failed: {}", e))?;

    // Classify final hunk
    classify_hunk(
        &mut hunk_added,
        &mut hunk_removed_count,
        &mut changed_lines,
        &mut deleted_lines,
        &last_hunk_header,
    );

    Ok(FileDiff {
        changed_lines,
        deleted_lines,
    })
}

/// Discard working-tree changes for a single file, restoring it to the HEAD version.
/// For untracked files this is a no-op (returns Ok).
pub fn discard_file_changes(file_path: &str) -> Result<(), String> {
    let path = Path::new(file_path);
    let repo = git2::Repository::discover(path).map_err(|e| format!("Not a git repo: {}", e))?;
    let repo_root = repo.workdir().ok_or("Bare repository")?;
    let rel_path = path
        .strip_prefix(repo_root)
        .map_err(|_| "File not in repo".to_string())?;

    let mut checkout_opts = git2::build::CheckoutBuilder::new();
    checkout_opts.path(rel_path);
    checkout_opts.force();
    repo.checkout_head(Some(&mut checkout_opts))
        .map_err(|e| format!("Checkout failed: {}", e))
}

/// Get blame information for a specific line in a file.
/// line is 1-based.
pub fn get_line_blame(file_path: &str, line: u32) -> Result<BlameInfo, String> {
    let path = Path::new(file_path);
    let repo = git2::Repository::discover(path).map_err(|e| format!("Not a git repo: {}", e))?;

    let repo_root = repo.workdir().ok_or("Bare repository")?;
    let rel_path = path
        .strip_prefix(repo_root)
        .map_err(|_| "File not in repo".to_string())?;

    let mut blame_opts = git2::BlameOptions::new();
    let blame = repo
        .blame_file(rel_path, Some(&mut blame_opts))
        .map_err(|e| format!("Blame failed: {}", e))?;

    // git2 blame uses 1-based line indexing in get_line()
    let hunk = blame
        .get_line(line as usize)
        .ok_or_else(|| format!("No blame info for line {}", line))?;

    let sig = hunk.final_signature();
    let author = sig.name().unwrap_or("Unknown").to_string();

    // Format the time
    let time = sig.when();
    let timestamp = time.seconds();
    let date = format_timestamp(timestamp);

    let commit_hash = format!("{}", hunk.final_commit_id());
    let commit_hash_short = commit_hash[..7.min(commit_hash.len())].to_string();

    // Get commit summary
    let summary = match repo.find_commit(hunk.final_commit_id()) {
        Ok(commit) => commit.summary().unwrap_or("").to_string(),
        Err(_) => String::new(),
    };

    Ok(BlameInfo {
        author,
        date,
        commit_hash: commit_hash_short,
        summary,
    })
}

/// Format a unix timestamp into a human-readable date string.
fn format_timestamp(timestamp: i64) -> String {
    let secs_per_day = 86400i64;

    let days_since_epoch = timestamp / secs_per_day;

    // Calculate year, month, day from days since epoch (1970-01-01)
    let mut year = 1970i32;
    let mut remaining_days = days_since_epoch;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let days_in_months = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u32;
    for &days in &days_in_months {
        if remaining_days < days {
            break;
        }
        remaining_days -= days;
        month += 1;
    }
    let day = remaining_days + 1;

    format!("{}-{:02}-{:02}", year, month, day)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_leap_year_basic() {
        assert!(is_leap_year(2000)); // divisible by 400
        assert!(is_leap_year(2024)); // divisible by 4
        assert!(!is_leap_year(1900)); // divisible by 100 but not 400
        assert!(!is_leap_year(2023)); // not divisible by 4
    }

    #[test]
    fn format_timestamp_epoch() {
        assert_eq!(format_timestamp(0), "1970-01-01");
    }

    #[test]
    fn format_timestamp_known_date() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        assert_eq!(format_timestamp(1704067200), "2024-01-01");
    }

    #[test]
    fn format_timestamp_leap_day() {
        // 2024-02-29 00:00:00 UTC = 1709164800
        assert_eq!(format_timestamp(1709164800), "2024-02-29");
    }

    #[test]
    fn format_timestamp_end_of_year() {
        // 2023-12-31 00:00:00 UTC = 1703980800
        assert_eq!(format_timestamp(1703980800), "2023-12-31");
    }

    #[test]
    fn diff_line_status_serialization() {
        let json = serde_json::to_string(&DiffLineStatus::Added).unwrap();
        assert_eq!(json, "\"Added\"");

        let json = serde_json::to_string(&DiffLineStatus::Modified).unwrap();
        assert_eq!(json, "\"Modified\"");
    }
}
