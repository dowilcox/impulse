//! Who last changed one line (the original Rust's `get_line_blame` and
//! `format_timestamp`), read with libgit2. The JSON keys are the FFI's
//! camelCase ones, which the fixture pins.

use std::path::Path;

use git2::BlameOptions;
use serde::{Deserialize, Serialize};

use crate::raw::{lossy, open_repo, relative_path_components, workdir};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameInfo {
    pub author: String,
    /// "YYYY-MM-DD", in the commit's own time zone.
    pub date: String,
    /// The abbreviated (7-character) commit id.
    pub commit_hash: String,
    pub summary: String,
}

/// Blame for the 1-based `line` of `file_path`; `None` on error.
pub fn line_blame(file_path: &str, line: u32) -> Option<BlameInfo> {
    let repo = open_repo(file_path).ok()?;
    let workdir = workdir(&repo).ok()?;
    // "File not in repo"
    let rel = relative_path_components(file_path, &workdir).filter(|r| !r.is_empty())?;
    let rel = rel.join("/");

    let mut options = BlameOptions::new();
    let blame = repo.blame_file(Path::new(&rel), Some(&mut options)).ok()?;
    let hunk = blame.get_line(line as usize)?;

    // libgit2 gives every hunk of a committed file its commit's signature.
    let signature = hunk.final_signature();
    let author = lossy(signature.name_bytes());
    let when = signature.when();
    let date = format_timestamp(when.seconds(), when.offset_minutes());

    let commit_id = hunk.final_commit_id();
    let commit_hash = commit_id.to_string()[..7].to_string();
    let summary = repo
        .find_commit(commit_id)
        .ok()
        .and_then(|commit| commit.summary_bytes().map(lossy))
        .unwrap_or_default();
    Some(BlameInfo {
        author,
        date,
        commit_hash,
        summary,
    })
}

/// A unix time as "YYYY-MM-DD" at a time-zone offset in minutes; a time
/// before 1970 is "1970-01-01".
pub fn format_timestamp(timestamp: i64, tz_offset_minutes: i32) -> String {
    let local = timestamp + tz_offset_minutes as i64 * 60;
    if local < 0 {
        return "1970-01-01".into();
    }
    let mut remaining_days = local / 86_400;
    let mut year = 1970i32;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }
    let february = if is_leap_year(year) { 29 } else { 28 };
    let days_in_months = [31, february, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1;
    for days in days_in_months {
        if remaining_days < days {
            break;
        }
        remaining_days -= days;
        month += 1;
    }
    format!("{year}-{month:02}-{:02}", remaining_days + 1)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(format_timestamp(0, 0), "1970-01-01");
        assert_eq!(format_timestamp(1_704_067_200, 0), "2024-01-01");
        assert_eq!(format_timestamp(1_704_067_200, -60), "2023-12-31");
        assert_eq!(format_timestamp(1_709_164_800, 0), "2024-02-29");
        assert_eq!(format_timestamp(-5, 0), "1970-01-01");
    }
}
