//! settings.json checked against its schema (`settings::json_schema`), as
//! the Swift app's Monaco checks it with the schema registered: a syntax
//! error where it is, a value of the wrong type, outside its range or not
//! one of its choices (in Monaco's JSON service's words), each setting's
//! description on hover, and completions for the keys not yet set and for
//! a key's values. Positions are byte offsets into the text.

use std::ops::Range;

use serde_json::Value;

/// Something wrong with the file.
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaProblem {
    pub range: Range<usize>,
    pub message: String,
    /// An error (the file doesn't parse, or a wrong type) rather than a
    /// warning (a value the schema doesn't accept).
    pub error: bool,
}

/// A completion: `insert` replaces `replace`.
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaCompletion {
    pub label: String,
    pub insert: String,
    pub replace: Range<usize>,
    pub detail: Option<String>,
    pub documentation: Option<String>,
}

/// A top-level member: its key, the key's string (quotes included) and
/// its value.
#[derive(Clone, Debug, PartialEq)]
struct Member {
    key: String,
    key_range: Range<usize>,
    value_range: Range<usize>,
}

fn schema_properties() -> serde_json::Map<String, Value> {
    crate::settings::json_schema()["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default()
}

/// What's wrong with `text`.
pub fn problems(text: &str) -> Vec<SchemaProblem> {
    if let Err(error) = serde_json::from_str::<Value>(text) {
        let at = offset_of(text, error.line(), error.column());
        let end = text[at..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| at + i)
            .unwrap_or(text.len());
        return vec![SchemaProblem {
            range: at..end.max(at),
            message: syntax_message(&error),
            error: true,
        }];
    }
    let Some(members) = top_level_members(text) else {
        return Vec::new();
    };
    let properties = schema_properties();
    members
        .iter()
        .filter_map(|member| {
            let schema = properties.get(&member.key)?;
            let value: Value = serde_json::from_str(&text[member.value_range.clone()]).ok()?;
            check(&value, schema).map(|(message, error)| SchemaProblem {
                range: member.value_range.clone(),
                message,
                error,
            })
        })
        .collect()
}

/// Why `value` doesn't fit `schema`, and whether that's an error.
fn check(value: &Value, schema: &Value) -> Option<(String, bool)> {
    let expected = schema["type"].as_str()?;
    let fits = match expected {
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    };
    if !fits {
        return Some((format!("Incorrect type. Expected \"{expected}\"."), true));
    }
    if let (Some(choices), Some(given)) = (schema["enum"].as_array(), value.as_str()) {
        if !choices.iter().any(|c| c.as_str() == Some(given)) {
            let listed: Vec<String> = choices.iter().map(|c| c.to_string()).collect();
            return Some((
                format!(
                    "Value is not accepted. Valid values: {}.",
                    listed.join(", ")
                ),
                false,
            ));
        }
    }
    if let Some(number) = value.as_f64() {
        if let Some(min) = schema["minimum"].as_f64().filter(|min| number < *min) {
            return Some((
                format!("Value is below the minimum of {}.", plain(min)),
                false,
            ));
        }
        if let Some(max) = schema["maximum"].as_f64().filter(|max| number > *max) {
            return Some((
                format!("Value is above the maximum of {}.", plain(max)),
                false,
            ));
        }
    }
    None
}

/// A number as JSON writes it (no `.0` on whole numbers).
fn plain(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}

fn syntax_message(error: &serde_json::Error) -> String {
    let text = error.to_string();
    // "expected `,` or `}` at line 3 column 5": the position is shown.
    let message = text.split(" at line ").next().unwrap_or(&text);
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Invalid JSON".into(),
    }
}

/// The byte offset of 1-based `line` and `column` (in characters).
fn offset_of(text: &str, line: usize, column: usize) -> usize {
    let line_start = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>()
        .min(text.len());
    text[line_start..]
        .char_indices()
        .nth(column.saturating_sub(1))
        .map(|(i, _)| line_start + i)
        .unwrap_or_else(|| {
            text[line_start..]
                .find('\n')
                .map_or(text.len(), |n| line_start + n)
        })
}

/// The setting under `offset` (on its key or value), described.
pub fn hover(text: &str, offset: usize) -> Option<String> {
    let member = top_level_members(text)?
        .into_iter()
        .find(|m| m.key_range.contains(&offset) || m.value_range.contains(&offset))?;
    describe(&member.key, schema_properties().get(&member.key)?)
}

/// A setting's description and default, as Markdown.
fn describe(key: &str, schema: &Value) -> Option<String> {
    let mut lines = vec![format!("**{key}**")];
    if let Some(description) = schema["description"].as_str() {
        lines.push(description.to_string());
    }
    if let Some(choices) = schema["enum"].as_array() {
        let listed: Vec<String> = choices.iter().map(|c| format!("`{c}`")).collect();
        lines.push(format!("One of {}.", listed.join(", ")));
    }
    if !schema["default"].is_null() {
        lines.push(format!("Default: `{}`", schema["default"]));
    }
    Some(lines.join("\n\n"))
}

/// What can go at `offset`: a setting not set yet where a key goes, or a
/// key's values after its colon.
pub fn completions(text: &str, offset: usize) -> Vec<SchemaCompletion> {
    let properties = schema_properties();
    match context_at(text, offset) {
        Some(Context::Key { replace, quoted }) => {
            let set: Vec<String> = top_level_members(text)
                .unwrap_or_default()
                .into_iter()
                .filter(|m| !m.key_range.contains(&offset))
                .map(|m| m.key)
                .collect();
            properties
                .iter()
                .filter(|(key, _)| !set.contains(key))
                .map(|(key, schema)| SchemaCompletion {
                    label: key.clone(),
                    insert: if quoted {
                        key.clone()
                    } else {
                        format!("\"{key}\": {}", sample(schema))
                    },
                    replace: replace.clone(),
                    detail: schema["description"].as_str().map(str::to_string),
                    documentation: describe(key, schema),
                })
                .collect()
        }
        Some(Context::Value { key, replace }) => {
            let Some(schema) = properties.get(&key) else {
                return Vec::new();
            };
            let mut values: Vec<String> = match schema["type"].as_str() {
                Some("boolean") => vec!["true".into(), "false".into()],
                _ => schema["enum"]
                    .as_array()
                    .map(|c| c.iter().map(|v| v.to_string()).collect())
                    .unwrap_or_default(),
            };
            if values.is_empty() && !schema["default"].is_null() {
                values.push(schema["default"].to_string());
            }
            values
                .into_iter()
                .map(|value| SchemaCompletion {
                    label: value.clone(),
                    insert: value,
                    replace: replace.clone(),
                    detail: None,
                    documentation: None,
                })
                .collect()
        }
        None => Vec::new(),
    }
}

/// A new member's value to start from: the default, else an empty one.
fn sample(schema: &Value) -> String {
    if !schema["default"].is_null() {
        return schema["default"].to_string();
    }
    match schema["type"].as_str() {
        Some("array") => "[]".into(),
        Some("object") => "{}".into(),
        Some("string") => "\"\"".into(),
        Some("boolean") => "false".into(),
        _ => "0".into(),
    }
}

/// Where in the top-level object `offset` is.
#[derive(Debug, PartialEq)]
enum Context {
    /// Where a key goes (`quoted`: inside its quotes).
    Key { replace: Range<usize>, quoted: bool },
    /// After `key`'s colon.
    Value { key: String, replace: Range<usize> },
}

fn context_at(text: &str, offset: usize) -> Option<Context> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    // At depth 1: the last key read, and whether its colon came.
    let mut key: Option<String> = None;
    let mut after_colon = false;
    let mut i = 0;
    while i < offset.min(bytes.len()) {
        match bytes[i] {
            b'"' => {
                let end = string_end(bytes, i);
                if offset <= end && offset > i {
                    // Inside this string.
                    let word = i + 1..offset;
                    if depth == 1 && !after_colon {
                        return Some(Context::Key {
                            replace: word.start..end.saturating_sub(1).max(word.start),
                            quoted: true,
                        });
                    }
                    if depth == 1 {
                        return Some(Context::Value {
                            key: key.clone()?,
                            replace: i..end.min(bytes.len()),
                        });
                    }
                    return None;
                }
                if depth == 1 && !after_colon {
                    key = Some(text[i + 1..end.saturating_sub(1)].to_string());
                }
                i = end;
                continue;
            }
            b'{' | b'[' => {
                depth += 1;
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
            }
            b':' if depth == 1 => after_colon = true,
            b',' if depth == 1 => {
                after_colon = false;
                key = None;
            }
            _ => {}
        }
        i += 1;
    }
    if depth != 1 {
        return None;
    }
    // A word being typed outside quotes.
    let start = text[..offset]
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .map_or(0, |i| i + 1);
    if after_colon {
        Some(Context::Value {
            key: key?,
            replace: start..offset,
        })
    } else {
        Some(Context::Key {
            replace: start..offset,
            quoted: false,
        })
    }
}

/// The byte after the string starting at `start` (its closing quote), or
/// the text's end for one left open.
fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            b'\n' => return i,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// The end of the JSON value starting at `start` (whitespace skipped).
fn value_end(bytes: &[u8], start: usize) -> Option<usize> {
    let i = skip_space(bytes, start);
    match *bytes.get(i)? {
        b'"' => Some(string_end(bytes, i)),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < bytes.len() {
                match bytes[j] {
                    b'"' => {
                        j = string_end(bytes, j);
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < bytes.len()
                && !matches!(bytes[j], b',' | b'}' | b']')
                && !bytes[j].is_ascii_whitespace()
            {
                j += 1;
            }
            Some(j)
        }
    }
}

fn skip_space(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// The top-level object's members, when the text is one.
fn top_level_members(text: &str) -> Option<Vec<Member>> {
    let bytes = text.as_bytes();
    let mut i = skip_space(bytes, 0);
    if bytes.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let mut members = Vec::new();
    loop {
        i = skip_space(bytes, i);
        match bytes.get(i)? {
            b'}' => return Some(members),
            b',' => {
                i += 1;
                continue;
            }
            b'"' => {}
            _ => return Some(members),
        }
        let key_end = string_end(bytes, i);
        let key = text[i + 1..key_end.saturating_sub(1).max(i + 1)].to_string();
        let key_range = i..key_end;
        i = skip_space(bytes, key_end);
        if bytes.get(i) != Some(&b':') {
            return Some(members);
        }
        let value_start = skip_space(bytes, i + 1);
        let value_end = value_end(bytes, value_start)?;
        members.push(Member {
            key,
            key_range,
            value_range: value_start..value_end,
        });
        i = value_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_values_are_reported_where_they_are() {
        let text = "{\n  \"font_size\": \"big\",\n  \"render_whitespace\": \"lots\",\n  \"tab_width\": 99\n}";
        let found = problems(text);
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[0].message, "Incorrect type. Expected \"integer\".");
        assert_eq!(&text[found[0].range.clone()], "\"big\"");
        assert!(found[1]
            .message
            .starts_with("Value is not accepted. Valid values: \"none\""));
        assert!(found[2]
            .message
            .starts_with("Value is above the maximum of"));
        assert!(problems("{\"font_size\": 14}").is_empty());
    }

    #[test]
    fn a_syntax_error_points_at_itself() {
        let text = "{\n  \"font_size\": 14\n  \"tab_width\": 4\n}";
        let found = problems(text);
        assert_eq!(found.len(), 1);
        assert!(found[0].error);
        assert_eq!(text[..found[0].range.start].lines().count(), 3);
    }

    #[test]
    fn hover_describes_the_setting() {
        let text = "{\"render_whitespace\": \"all\"}";
        let hovered = hover(text, 5).unwrap();
        assert!(hovered.starts_with("**render_whitespace**"));
        assert!(hovered.contains("`\"boundary\"`"));
        assert_eq!(hover(text, 0), None);
    }

    #[test]
    fn keys_and_values_complete() {
        let text = "{\n  \"font_size\": 14,\n  \"render_w\n}";
        let at = text.find("render_w").unwrap() + "render_w".len();
        let keys = completions(text, at);
        assert!(keys
            .iter()
            .any(|c| c.label == "render_whitespace" && c.insert == "render_whitespace"));
        assert!(!keys.iter().any(|c| c.label == "font_size"));

        let text = "{\"render_whitespace\": }";
        let values = completions(text, text.find(": ").unwrap() + 2);
        assert!(values.iter().any(|c| c.insert == "\"selection\""));

        let text = "{\n  \n}";
        let bare = completions(text, 4);
        let tab = bare.iter().find(|c| c.label == "tab_width").unwrap();
        assert!(tab.insert.starts_with("\"tab_width\": "));
    }
}
