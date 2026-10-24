//! Command history shared by every terminal: the SQLite database of
//! commands run (`database`, the same file and schema as the Swift app's,
//! so both apps can share it) and readers for the shells' own history files
//! (`import`), so it can start with what the user already typed elsewhere.

pub mod database;
pub mod import;

pub use database::{CommandHistoryDatabase, HistoryEntry, HistoryFilter, HistoryHit};

/// How long ago something was, abbreviated as the palette shows it ("5 min.
/// ago", "3 days ago"), the way Apple's abbreviated relative date formatter
/// reads in English.
pub fn relative_time(seconds_ago: f64) -> String {
    let s = seconds_ago.max(0.0) as u64;
    let (n, one, many) = match s {
        0..=59 => (s, "sec.", "sec."),
        60..=3599 => (s / 60, "min.", "min."),
        3600..=86_399 => (s / 3600, "hr.", "hr."),
        86_400..=604_799 => (s / 86_400, "day", "days"),
        604_800..=2_629_799 => (s / 604_800, "wk.", "wk."),
        2_629_800..=31_557_599 => (s / 2_629_800, "mo.", "mo."),
        _ => (s / 31_557_600, "yr.", "yr."),
    };
    format!("{n} {} ago", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::relative_time;

    #[test]
    fn relative_times() {
        assert_eq!(relative_time(5.0), "5 sec. ago");
        assert_eq!(relative_time(-3.0), "0 sec. ago");
        assert_eq!(relative_time(125.0), "2 min. ago");
        assert_eq!(relative_time(3600.0), "1 hr. ago");
        assert_eq!(relative_time(86_400.0), "1 day ago");
        assert_eq!(relative_time(3.0 * 86_400.0), "3 days ago");
        assert_eq!(relative_time(15.0 * 86_400.0), "2 wk. ago");
        assert_eq!(relative_time(70.0 * 86_400.0), "2 mo. ago");
        assert_eq!(relative_time(800.0 * 86_400.0), "2 yr. ago");
    }
}
