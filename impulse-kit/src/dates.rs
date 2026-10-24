//! Dates as the Swift app shows them: History's abbreviated relative dates
//! (`RelativeDateTimeFormatter` with `.abbreviated` units: "2d ago",
//! "1w ago", "3mo ago") and commit details' medium date with a short time
//! ("Oct 5, 2026 at 3:27 PM").

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};

/// How long ago `seconds` was, in its largest whole unit: "now", "12s ago",
/// "5m ago", "3h ago", "2d ago", "1w ago", "1mo ago", "2y ago". A date in
/// the future reads "in 5m".
pub fn abbreviated_relative(seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;
    let span = seconds.abs();
    if span < 1 {
        return "now".into();
    }
    let amount = match span {
        s if s < MINUTE => format!("{s}s"),
        s if s < HOUR => format!("{}m", s / MINUTE),
        s if s < DAY => format!("{}h", s / HOUR),
        s if s < WEEK => format!("{}d", s / DAY),
        s if s < MONTH => format!("{}w", s / WEEK),
        s if s < YEAR => format!("{}mo", s / MONTH),
        s => format!("{}y", s / YEAR),
    };
    if seconds < 0 {
        format!("in {amount}")
    } else {
        format!("{amount} ago")
    }
}

/// "Oct 5, 2026 at 3:27 PM" (`DateFormatter` with `.medium` date and
/// `.short` time, US English).
pub fn medium_date_time<Tz: TimeZone>(date: &DateTime<Tz>) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let hour = date.hour();
    let (twelve, meridiem) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!(
        "{} {}, {} at {}:{:02} {}",
        MONTHS[date.month0() as usize],
        date.day(),
        date.year(),
        twelve,
        date.minute(),
        meridiem
    )
}

/// `time` in the local time zone, as `medium_date_time` writes it.
pub fn local_medium_date_time(time: std::time::SystemTime) -> String {
    medium_date_time(&DateTime::<Local>::from(time))
}

/// Seconds from `time` until now (negative for a time still to come).
pub fn seconds_ago(time: std::time::SystemTime) -> i64 {
    match std::time::SystemTime::now().duration_since(time) {
        Ok(elapsed) => elapsed.as_secs() as i64,
        Err(ahead) => -(ahead.duration().as_secs() as i64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn relative_dates_take_the_largest_unit() {
        assert_eq!(abbreviated_relative(0), "now");
        assert_eq!(abbreviated_relative(12), "12s ago");
        assert_eq!(abbreviated_relative(5 * 60 + 9), "5m ago");
        assert_eq!(abbreviated_relative(3 * 3600), "3h ago");
        assert_eq!(abbreviated_relative(2 * 86_400 + 5), "2d ago");
        assert_eq!(abbreviated_relative(10 * 86_400), "1w ago");
        assert_eq!(abbreviated_relative(45 * 86_400), "1mo ago");
        assert_eq!(abbreviated_relative(800 * 86_400), "2y ago");
        assert_eq!(abbreviated_relative(-300), "in 5m");
    }

    #[test]
    fn medium_dates_read_like_the_swift_formatter() {
        let date = Utc.with_ymd_and_hms(2026, 10, 5, 15, 27, 0).unwrap();
        assert_eq!(medium_date_time(&date), "Oct 5, 2026 at 3:27 PM");
        let midnight = Utc.with_ymd_and_hms(2026, 1, 31, 0, 4, 0).unwrap();
        assert_eq!(medium_date_time(&midnight), "Jan 31, 2026 at 12:04 AM");
        let noon = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
        assert_eq!(medium_date_time(&noon), "Jun 1, 2026 at 12:00 PM");
    }
}
