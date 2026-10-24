//! JSON written the way the Swift app's Foundation encoders write it, so the
//! two apps' messages match byte for byte. Keys are sorted as Foundation's
//! `.sortedKeys` sorts them: runs of digits by value, case ignored, and
//! keys that still tie by their characters. The control socket escapes `/`
//! as `\/` (`JSONEncoder`'s default); the MCP server and the agents' answers
//! leave slashes alone, as their Swift encoders are told to. Pretty output
//! is Foundation's too: two-space indents, ` : ` after a key, and empty
//! collections on two lines.
//!
//! Also the accessors the MCP server reads tool arguments with, which are
//! as lenient as the Swift `JSONValue`'s, and ISO 8601 dates.

use std::cmp::Ordering;

use serde::Serialize;
use serde_json::Value;

/// How to write a value.
#[derive(Clone, Copy, Debug, Default)]
pub struct Style {
    /// `/` as `\/`, as the control socket's encoder writes it.
    pub escape_slashes: bool,
    pub pretty: bool,
}

impl Style {
    /// The control socket's lines.
    pub const CONTROL: Style = Style {
        escape_slashes: true,
        pretty: false,
    };
    /// MCP messages and the agents' answers as data.
    pub const COMPACT: Style = Style {
        escape_slashes: false,
        pretty: false,
    };
    /// The agents' answers as text (`--json`).
    pub const PRETTY: Style = Style {
        escape_slashes: false,
        pretty: true,
    };
}

/// `value` as text in `style`.
pub fn write(value: &Value, style: Style) -> String {
    let mut out = String::new();
    write_value(value, style, 0, &mut out);
    out
}

/// Any serializable value in `style`; `{}` if it can't be serialized (a map
/// with keys that aren't strings), as the Swift encoders fall back.
pub fn encode<T: Serialize + ?Sized>(value: &T, style: Style) -> String {
    match serde_json::to_value(value) {
        Ok(value) => write(&value, style),
        Err(_) => "{}".to_string(),
    }
}

/// One MCP message: compact, sorted, newline included.
pub fn line(value: &Value) -> Vec<u8> {
    let mut out = write(value, Style::COMPACT).into_bytes();
    out.push(b'\n');
    out
}

fn write_value(value: &Value, style: Style, depth: usize, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => write_string(text, style, out),
        Value::Array(items) => {
            out.push('[');
            if style.pretty {
                out.push('\n');
            }
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                    if style.pretty {
                        out.push('\n');
                    }
                }
                if style.pretty {
                    indent(depth + 1, out);
                }
                write_value(item, style, depth + 1, out);
            }
            if style.pretty {
                out.push('\n');
                indent(depth, out);
            }
            out.push(']');
        }
        Value::Object(object) => {
            let mut entries: Vec<(&String, &Value)> = object.iter().collect();
            entries.sort_by(|a, b| compare_keys(a.0, b.0));
            out.push('{');
            if style.pretty {
                out.push('\n');
            }
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                    if style.pretty {
                        out.push('\n');
                    }
                }
                if style.pretty {
                    indent(depth + 1, out);
                }
                write_string(key, style, out);
                out.push_str(if style.pretty { " : " } else { ":" });
                write_value(item, style, depth + 1, out);
            }
            if style.pretty {
                out.push('\n');
                indent(depth, out);
            }
            out.push('}');
        }
    }
}

fn indent(depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_string(text: &str, style: Style, out: &mut String) {
    let quoted = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    if style.escape_slashes {
        // No escape sequence contains a slash, so this only touches the
        // text's own.
        out.push_str(&quoted.replace('/', "\\/"));
    } else {
        out.push_str(&quoted);
    }
}

/// Foundation's key order for `.sortedKeys` (`[.numeric, .caseInsensitive,
/// .forcedOrdering]`).
pub fn compare_keys(a: &str, b: &str) -> Ordering {
    let left: Vec<char> = a.chars().collect();
    let right: Vec<char> = b.chars().collect();
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        if left[i].is_ascii_digit() && right[j].is_ascii_digit() {
            let start_i = i;
            while i < left.len() && left[i].is_ascii_digit() {
                i += 1;
            }
            let start_j = j;
            while j < right.len() && right[j].is_ascii_digit() {
                j += 1;
            }
            let order = compare_numbers(&left[start_i..i], &right[start_j..j]);
            if order != Ordering::Equal {
                return order;
            }
            continue;
        }
        let order = fold(left[i]).cmp(&fold(right[j]));
        if order != Ordering::Equal {
            return order;
        }
        i += 1;
        j += 1;
    }
    let order = (left.len() - i).cmp(&(right.len() - j));
    if order != Ordering::Equal {
        return order;
    }
    // Equal but for case or leading zeros: still an order.
    a.cmp(b)
}

fn compare_numbers(a: &[char], b: &[char]) -> Ordering {
    let trim = |digits: &[char]| -> Vec<char> {
        let first = digits
            .iter()
            .position(|c| *c != '0')
            .unwrap_or(digits.len());
        digits[first..].to_vec()
    };
    let (a, b) = (trim(a), trim(b));
    a.len().cmp(&b.len()).then_with(|| a.cmp(&b))
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

// MARK: Reading tool arguments

/// A string value.
pub fn string(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

/// A whole number, also when it came as `3.0` or `"3"`.
pub fn int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                return Some(int);
            }
            let float = number.as_f64()?;
            (float == float.round() && float.abs() < 1e15).then_some(float as i64)
        }
        Value::String(text) => parse_int(text),
        _ => None,
    }
}

/// A boolean, also when it came as `"true"` or `"false"`.
pub fn bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::String(text) if text == "true" => Some(true),
        Value::String(text) if text == "false" => Some(false),
        _ => None,
    }
}

/// A number as Swift's `Int(String)` reads it: an optional sign, then
/// digits only.
pub fn parse_int(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

// MARK: Dates

/// ISO 8601 dates in UTC, whole seconds (`2026-10-01T00:00:00Z`), as the
/// Swift encoders' `.iso8601` strategy writes them. For
/// `#[serde(with = "json::iso8601")]`.
pub mod iso8601 {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn format(time: SystemTime) -> String {
        let seconds = match time.duration_since(UNIX_EPOCH) {
            Ok(after) => after.as_secs() as i64,
            Err(before) => {
                let before = before.duration();
                -(before.as_secs() as i64) - i64::from(before.subsec_nanos() > 0)
            }
        };
        let days = seconds.div_euclid(86_400);
        let time = seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            time / 3600,
            time / 60 % 60,
            time % 60
        )
    }

    /// `YYYY-MM-DDTHH:MM:SS` with `Z` or an offset (`+02:00`).
    pub fn parse(text: &str) -> Option<SystemTime> {
        let bytes = text.as_bytes();
        if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        if !matches!(bytes[10], b'T' | b't') || bytes[13] != b':' || bytes[16] != b':' {
            return None;
        }
        let number = |range: std::ops::Range<usize>| -> Option<i64> {
            let part = text.get(range)?;
            part.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| part.parse().ok())?
        };
        let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
        let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
            return None;
        }
        if second > 60 {
            return None;
        }
        let offset = match &text[19..] {
            "Z" | "z" => 0,
            zone if zone.len() == 6 && zone.as_bytes()[3] == b':' => {
                let sign = match zone.as_bytes()[0] {
                    b'+' => 1,
                    b'-' => -1,
                    _ => return None,
                };
                sign * (number(20..22)? * 3600 + number(23..25)? * 60)
            }
            _ => return None,
        };
        let seconds =
            days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
                - offset;
        Some(if seconds >= 0 {
            UNIX_EPOCH + Duration::from_secs(seconds as u64)
        } else {
            UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs())
        })
    }

    // Howard Hinnant's algorithms for the proleptic Gregorian calendar.
    fn civil_from_days(days: i64) -> (i64, i64, i64) {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        (year, month, day)
    }

    fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
        let year = if month <= 2 { year - 1 } else { year };
        let era = year.div_euclid(400);
        let yoe = year.rem_euclid(400);
        let mp = if month > 2 { month - 3 } else { month + 9 };
        let doy = (153 * mp + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn serialize<S: Serializer>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format(*time))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "Expected date string to be ISO8601-formatted: {text}"
            ))
        })
    }

    /// The same for optional dates (left out when `None`).
    pub mod option {
        use std::time::SystemTime;

        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(
            time: &Option<SystemTime>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match time {
                Some(time) => super::serialize(time, serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<SystemTime>, D::Error> {
            match Option::<String>::deserialize(deserializer)? {
                Some(text) => super::parse(&text).map(Some).ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        "Expected date string to be ISO8601-formatted: {text}"
                    ))
                }),
                None => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    #[test]
    fn keys_sort_as_foundation_sorts_them() {
        let mut keys = vec!["b", "A", "a10", "a9", "_meta", "B", "a"];
        keys.sort_by(|a, b| compare_keys(a, b));
        assert_eq!(keys, ["_meta", "A", "a", "a9", "a10", "B", "b"]);
    }

    #[test]
    fn control_lines_escape_slashes_and_mcp_lines_dont() {
        let value = json!({"path": "/x/y", "a": [1, true, null]});
        assert_eq!(
            write(&value, Style::CONTROL),
            r#"{"a":[1,true,null],"path":"\/x\/y"}"#
        );
        assert_eq!(
            write(&value, Style::COMPACT),
            r#"{"a":[1,true,null],"path":"/x/y"}"#
        );
    }

    #[test]
    fn pretty_is_foundations() {
        let value = json!({"b": {"c": []}, "a": 1});
        assert_eq!(
            write(&value, Style::PRETTY),
            "{\n  \"a\" : 1,\n  \"b\" : {\n    \"c\" : [\n\n    ]\n  }\n}"
        );
    }

    #[test]
    fn lenient_accessors() {
        assert_eq!(int(Some(&json!(3.0))), Some(3));
        assert_eq!(int(Some(&json!("3"))), Some(3));
        assert_eq!(int(Some(&json!(3.5))), None);
        assert_eq!(int(Some(&json!(" 3"))), None);
        assert_eq!(bool(Some(&json!("false"))), Some(false));
        assert_eq!(bool(Some(&json!(1))), None);
    }

    #[test]
    fn dates_round_trip() {
        let time = UNIX_EPOCH + Duration::from_secs(1_790_812_800);
        assert_eq!(iso8601::format(time), "2026-10-01T00:00:00Z");
        assert_eq!(iso8601::parse("2026-10-01T00:00:00Z"), Some(time));
        assert_eq!(iso8601::parse("2026-10-01T02:00:00+02:00"), Some(time));
        assert_eq!(
            iso8601::format(UNIX_EPOCH - Duration::from_secs(1)),
            "1969-12-31T23:59:59Z"
        );
        assert_eq!(iso8601::parse("2026-10-01"), None);
    }
}
