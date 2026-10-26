//! JSON written byte for byte the way Foundation writes it: by default as
//! `JSONSerialization` with `.sortedKeys` and `.withoutEscapingSlashes`
//! (and `.prettyPrinted` for [`pretty`]), the options the Swift app's agent
//! installers use. Agent configs Impulse writes then come out the same
//! from either app, so switching between them doesn't rewrite a user's
//! settings.json, and the Agent Integration preview shows the same lines.
//! [`Format::ENCODER`] writes as `JSONEncoder` with `.sortedKeys` instead
//! (`.git/impulse/tasks.json`).
//!
//! What differs from serde_json's writer:
//! - `"key" : value` with a space before the colon, and an empty array or
//!   object as `[`, a blank line, then `]` at the container's indent;
//! - keys sorted the way Foundation sorts them. `JSONSerialization`
//!   compares them with `compare(_:options: [.numeric, .caseInsensitive,
//!   .forcedOrdering], locale: .system)`, an ICU collation: punctuation
//!   before digits before letters, letters ignoring case (lowercase first
//!   on a tie), digit runs by value (`plugin9` before `plugin10`).
//!   [`compare_keys`] follows it exactly for ASCII; other characters sort
//!   after ASCII letters, by code point. `JSONEncoder` sorts by code point
//!   (`AB`, `Ab`, `B`, `_x`, `a10`, `a9`, `b`);
//! - doubles as `%.17g` (`0.1` is `0.10000000000000001`, `1e+20`).
//!   `JSONEncoder` writes doubles in their shortest form (`0.1`) instead,
//!   which isn't reproduced: the files written in its format hold none;
//! - `/` as `\/` unless `.withoutEscapingSlashes` ([`Format::escape_slashes`]).

use std::cmp::Ordering;

use serde_json::{Map, Number, Value};

/// Which Foundation writer's choices to follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// `/` written as `\/`: Foundation's default, turned off by
    /// `.withoutEscapingSlashes`.
    pub escape_slashes: bool,
    pub key_order: KeyOrder,
}

/// How `.sortedKeys` orders an object's keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOrder {
    /// `JSONSerialization`'s: [`compare_keys`].
    Collated,
    /// `JSONEncoder`'s: by code point (UTF-8 byte order).
    CodePoint,
}

impl Format {
    /// `JSONSerialization` with `[.sortedKeys, .withoutEscapingSlashes]`:
    /// the agent installers' files.
    pub const SERIALIZATION: Format = Format {
        escape_slashes: false,
        key_order: KeyOrder::Collated,
    };

    /// `JSONEncoder` with `.sortedKeys` and without
    /// `.withoutEscapingSlashes`: `.git/impulse/tasks.json`.
    pub const ENCODER: Format = Format {
        escape_slashes: true,
        key_order: KeyOrder::CodePoint,
    };
}

/// `JSONSerialization` with `[.prettyPrinted, .sortedKeys,
/// .withoutEscapingSlashes]` (no final newline, as Foundation writes it).
pub fn pretty(value: &Value) -> String {
    pretty_with(value, Format::SERIALIZATION)
}

/// `JSONSerialization` with `[.sortedKeys, .withoutEscapingSlashes]`.
pub fn compact(value: &Value) -> String {
    compact_with(value, Format::SERIALIZATION)
}

/// Pretty printed in `format` (no final newline, as Foundation writes it).
pub fn pretty_with(value: &Value, format: Format) -> String {
    let mut out = String::new();
    write(value, Some(0), format, &mut out);
    out
}

/// On one line in `format`.
pub fn compact_with(value: &Value, format: Format) -> String {
    let mut out = String::new();
    write(value, None, format, &mut out);
    out
}

fn write(value: &Value, indent: Option<usize>, format: Format, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number_text(number)),
        Value::String(text) => write_string(text, format, out),
        Value::Array(items) => write_container(
            ('[', ']'),
            items.iter().map(|item| (None, item)),
            items.len(),
            indent,
            format,
            out,
        ),
        Value::Object(map) => {
            let entries = sorted_entries(map, format.key_order);
            let count = entries.len();
            write_container(
                ('{', '}'),
                entries
                    .into_iter()
                    .map(|(key, value)| (Some(key.as_str()), value)),
                count,
                indent,
                format,
                out,
            )
        }
    }
}

fn write_container<'a>(
    (open, close): (char, char),
    members: impl Iterator<Item = (Option<&'a str>, &'a Value)>,
    count: usize,
    indent: Option<usize>,
    format: Format,
    out: &mut String,
) {
    out.push(open);
    let Some(level) = indent else {
        for (index, (key, value)) in members.enumerate() {
            if index > 0 {
                out.push(',');
            }
            if let Some(key) = key {
                write_string(key, format, out);
                out.push(':');
            }
            write(value, None, format, out);
        }
        out.push(close);
        return;
    };
    out.push('\n');
    if count == 0 {
        // Foundation leaves an empty line inside an empty container.
        out.push('\n');
    }
    for (index, (key, value)) in members.enumerate() {
        if index > 0 {
            out.push_str(",\n");
        }
        push_indent(level + 1, out);
        if let Some(key) = key {
            write_string(key, format, out);
            out.push_str(" : ");
        }
        write(value, Some(level + 1), format, out);
    }
    if count > 0 {
        out.push('\n');
    }
    push_indent(level, out);
    out.push(close);
}

fn push_indent(level: usize, out: &mut String) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn write_string(text: &str, format: Format, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '/' if format.escape_slashes => out.push_str("\\/"),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn number_text(number: &Number) -> String {
    if let Some(value) = number.as_i64() {
        return value.to_string();
    }
    if let Some(value) = number.as_u64() {
        return value.to_string();
    }
    number.as_f64().map(format_double).unwrap_or_default()
}

/// C's `%.17g`, which Foundation writes doubles with.
fn format_double(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.into();
    }
    // 17 significant digits; the exponent is the one after rounding.
    let scientific = format!("{value:.16e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if !(-4..17).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!(
            "{}e{sign}{:02}",
            without_trailing_zeros(mantissa),
            exponent.abs()
        );
    }
    let decimals = (16 - exponent) as usize;
    without_trailing_zeros(&format!("{value:.decimals$}")).to_string()
}

fn without_trailing_zeros(text: &str) -> &str {
    if !text.contains('.') {
        return text;
    }
    text.trim_end_matches('0').trim_end_matches('.')
}

fn sorted_entries(map: &Map<String, Value>, order: KeyOrder) -> Vec<(&String, &Value)> {
    let mut entries: Vec<(&String, &Value)> = map.iter().collect();
    match order {
        KeyOrder::Collated => entries.sort_by(|a, b| compare_keys(a.0, b.0)),
        KeyOrder::CodePoint => entries.sort_by(|a, b| a.0.cmp(b.0)),
    }
    entries
}

/// The order `.sortedKeys` puts object keys in: Foundation compares them
/// with `[.numeric, .caseInsensitive, .forcedOrdering]` in the system
/// locale, which is ICU's root collation. For ASCII that is: control
/// characters ignored, then tab, newline, vertical tab, form feed, return,
/// space, the punctuation in [`PUNCTUATION`]'s order, digit runs by value,
/// then letters ignoring case. Keys that tie compare by fewer leading
/// zeros, then lowercase before uppercase, then byte by byte.
pub fn compare_keys(a: &str, b: &str) -> Ordering {
    let (a_elements, b_elements) = (elements(a), elements(b));
    primary(&a_elements, &b_elements)
        .then_with(|| leading_zeros(&a_elements).cmp(&leading_zeros(&b_elements)))
        .then_with(|| case_marks(a).cmp(&case_marks(b)))
        .then_with(|| a.cmp(b))
}

/// Whitespace and punctuation in ICU root collation order (each sorts
/// before the next, all before digits and letters).
const PUNCTUATION: &str = "\t\n\u{b}\u{c}\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

const DIGITS_WEIGHT: u32 = 100;
const LETTERS_WEIGHT: u32 = 200;
const OTHER_WEIGHT: u32 = 1000;

#[derive(Debug)]
enum Element<'a> {
    /// A character's primary weight.
    Weight(u32),
    /// A run of digits: its value's digits (no leading zeros) and how many
    /// zeros led it.
    Number(&'a str, usize),
}

impl Element<'_> {
    fn weight(&self) -> u32 {
        match self {
            Element::Weight(weight) => *weight,
            Element::Number(..) => DIGITS_WEIGHT,
        }
    }
}

fn elements(key: &str) -> Vec<Element<'_>> {
    let mut elements = Vec::new();
    let mut chars = key.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if c.is_ascii_digit() {
            let mut end = start + 1;
            while let Some(&(index, next)) = chars.peek() {
                if !next.is_ascii_digit() {
                    break;
                }
                end = index + 1;
                chars.next();
            }
            let run = &key[start..end];
            let digits = run.trim_start_matches('0');
            let digits = if digits.is_empty() { "0" } else { digits };
            let zeros = run.len() - digits.len();
            elements.push(Element::Number(digits, zeros));
            continue;
        }
        let weight = if let Some(index) = PUNCTUATION.find(c) {
            index as u32 + 1
        } else if c.is_ascii_alphabetic() {
            LETTERS_WEIGHT + (c.to_ascii_lowercase() as u32 - 'a' as u32)
        } else if c.is_ascii() {
            // Other control characters are ignored.
            continue;
        } else {
            OTHER_WEIGHT + c as u32
        };
        elements.push(Element::Weight(weight));
    }
    elements
}

fn primary(a: &[Element], b: &[Element]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let order = x.weight().cmp(&y.weight()).then_with(|| match (x, y) {
            (Element::Number(x, _), Element::Number(y, _)) => {
                x.len().cmp(&y.len()).then_with(|| x.cmp(y))
            }
            _ => Ordering::Equal,
        });
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

fn leading_zeros(elements: &[Element]) -> Vec<usize> {
    elements
        .iter()
        .filter_map(|element| match element {
            Element::Number(_, zeros) => Some(*zeros),
            Element::Weight(_) => None,
        })
        .collect()
}

/// Each letter's case, lowercase first.
fn case_marks(key: &str) -> Vec<bool> {
    key.chars()
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.is_ascii_uppercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(keys: &[&str]) -> Vec<String> {
        let mut keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        keys.sort_by(|a, b| compare_keys(a, b));
        keys
    }

    // The expected orders and texts below are what `JSONSerialization`
    // wrote for the same input on macOS 26 (a Swift probe run when this was
    // ported).

    #[test]
    fn printable_ascii_sorts_like_foundation() {
        let singles: Vec<String> = (0x20u8..=0x7e).map(|b| (b as char).to_string()).collect();
        let refs: Vec<&str> = singles.iter().map(String::as_str).collect();
        assert_eq!(
            sorted(&refs).concat(),
            " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ"
        );
    }

    #[test]
    fn keys_sort_like_foundation() {
        assert_eq!(
            sorted(&["ab", "Ab", "aB", "AB", "a", "A", "abc", "aBc", "ABC", "Abc"]),
            ["a", "A", "ab", "aB", "Ab", "AB", "abc", "aBc", "Abc", "ABC"]
        );
        assert_eq!(
            sorted(&[
                "a1", "a01", "a001", "a2", "a10", "x1y", "x01y", "1", "01", "10", "2", "a1b",
                "a1a", "a 1"
            ]),
            [
                "1", "01", "2", "10", "a 1", "a1", "a01", "a001", "a1a", "a1b", "a2", "a10", "x1y",
                "x01y"
            ]
        );
        assert_eq!(
            sorted(&[
                "Stop",
                "StopFailure",
                "stop",
                "STOP",
                "PreToolUse",
                "PostToolUse",
                "CLAUDE_CODE_X",
                "CLAUDEX",
                "CLAUDE",
                "claude_code",
                "$schema",
                "_a",
                "A_b",
                "AB"
            ]),
            [
                "_a",
                "$schema",
                "A_b",
                "AB",
                "CLAUDE",
                "claude_code",
                "CLAUDE_CODE_X",
                "CLAUDEX",
                "PostToolUse",
                "PreToolUse",
                "stop",
                "Stop",
                "STOP",
                "StopFailure"
            ]
        );
        assert_eq!(
            sorted(&["a.b", "a-b", "a_b", "ab", "a/b", "a@b", "a:b", "aB", "a1", "a b"]),
            ["a b", "a_b", "a-b", "a:b", "a.b", "a@b", "a/b", "a1", "ab", "aB"]
        );
        assert_eq!(
            sorted(&["\t", "\n", "\u{1}", " ", "a"]),
            ["\u{1}", "\t", "\n", " ", "a"]
        );
        assert_eq!(
            sorted(&["x\u{1}y", "xy", "x y", "x\u{0}y"]),
            ["x y", "x\u{0}y", "x\u{1}y", "xy"]
        );
    }

    #[test]
    fn writes_like_foundation() {
        let input = r#"{"b":1,"a":[],"c":{},"B":true,"_x":null,"a10":1.0,"a9":0.1,"f":false,"s":"/\u0001\t\n\r\b\f\"\\ \u007f \u001f é","n":[1,2.5,-0.0,1e20,1e-7,123456789012345678,12345678901234567890,3.14159265358979,100.0,1e16,1e17,0.0001,0.00001,1.5e300,-5],"nested":{"z":[{"y":[]}],"Z":"x"}}"#;
        let value: Value = serde_json::from_str(input).unwrap();
        let expected = "{\n  \"_x\" : null,\n  \"a\" : [\n\n  ],\n  \"a9\" : 0.10000000000000001,\n  \"a10\" : 1,\n  \"b\" : 1,\n  \"B\" : true,\n  \"c\" : {\n\n  },\n  \"f\" : false,\n  \"n\" : [\n    1,\n    2.5,\n    -0,\n    1e+20,\n    9.9999999999999995e-08,\n    123456789012345678,\n    12345678901234567890,\n    3.14159265358979,\n    100,\n    10000000000000000,\n    1e+17,\n    0.0001,\n    1.0000000000000001e-05,\n    1.5000000000000001e+300,\n    -5\n  ],\n  \"nested\" : {\n    \"z\" : [\n      {\n        \"y\" : [\n\n        ]\n      }\n    ],\n    \"Z\" : \"x\"\n  },\n  \"s\" : \"/\\u0001\\t\\n\\r\\b\\f\\\"\\\\ \u{7f} \\u001f é\"\n}";
        assert_eq!(pretty(&value), expected);
        assert_eq!(
            compact(&value),
            "{\"_x\":null,\"a\":[],\"a9\":0.10000000000000001,\"a10\":1,\"b\":1,\"B\":true,\"c\":{},\"f\":false,\"n\":[1,2.5,-0,1e+20,9.9999999999999995e-08,123456789012345678,12345678901234567890,3.14159265358979,100,10000000000000000,1e+17,0.0001,1.0000000000000001e-05,1.5000000000000001e+300,-5],\"nested\":{\"z\":[{\"y\":[]}],\"Z\":\"x\"},\"s\":\"/\\u0001\\t\\n\\r\\b\\f\\\"\\\\ \u{7f} \\u001f é\"}"
        );
        assert_eq!(pretty(&Value::Object(Map::new())), "{\n\n}");
    }

    #[test]
    fn json_encoder_sorts_by_code_point_and_escapes_slashes() {
        // `JSONEncoder` with `.sortedKeys` (a `[String: Int]`), then
        // `JSONSerialization` with `.sortedKeys` for the same keys, from the
        // Swift probe (macOS 27, Swift 6.4).
        let encoder = r#"{"\u0001":28,"\t":29,"$s":5,"01":20,"1":19,"10":21,"2":22,"AB":7,"A_b":6,"Ab":13,"B":1,"Z":10,"_x":4,"a b":16,"a-b":17,"a.b":18,"a\/b":8,"a01":14,"a1":15,"a10":2,"a9":3,"aB":12,"ab":11,"b":0,"created":25,"createdBy":24,"requestedBy":27,"requestedFrom":26,"x\u0001y":30,"xy":31,"z":23,"é":9}"#;
        let value: Value = serde_json::from_str(encoder).unwrap();
        assert_eq!(compact_with(&value, Format::ENCODER), encoder);
        // `compare_keys` places non-ASCII letters after ASCII ones, where
        // ICU puts `é` beside `e`: left out here.
        let serialization = r#"{"\u0001":28,"\t":29,"_x":4,"$s":5,"1":19,"01":20,"2":22,"10":21,"a b":16,"A_b":6,"a-b":17,"a.b":18,"a\/b":8,"a1":15,"a01":14,"a9":3,"a10":2,"ab":11,"aB":12,"Ab":13,"AB":7,"b":0,"B":1,"created":25,"createdBy":24,"requestedBy":27,"requestedFrom":26,"x\u0001y":30,"xy":31,"z":23,"Z":10}"#;
        let value: Value = serde_json::from_str(serialization).unwrap();
        let escaping = Format {
            escape_slashes: true,
            ..Format::SERIALIZATION
        };
        assert_eq!(compact_with(&value, escaping), serialization);
        assert_eq!(compact(&value), serialization.replace("\\/", "/"));

        let nested: Value = serde_json::from_str(r#"{"b":{"a/":[],"B":{}},"a":"/"}"#).unwrap();
        assert_eq!(
            pretty_with(&nested, Format::ENCODER),
            "{\n  \"a\" : \"\\/\",\n  \"b\" : {\n    \"B\" : {\n\n    },\n    \"a\\/\" : [\n\n    ]\n  }\n}"
        );
    }
}
