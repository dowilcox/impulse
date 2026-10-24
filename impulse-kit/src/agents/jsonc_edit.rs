//! Removing one member of an object in a JSONC file (JSON with comments and
//! trailing commas, as opencode reads its config) without rewriting the
//! rest: the user's comments, key order and formatting stay byte for byte.
//! Text that isn't JSONC this can follow is refused rather than guessed at.

use std::ops::Range;

/// The text isn't JSONC this can follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

type Parsed<T> = Result<T, Malformed>;

/// An object member, by byte offsets into the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub key: String,
    pub key_start: usize,
    pub value_start: usize,
    pub value_end: usize,
    /// The comma after the value (a separator, or a trailing comma).
    pub comma: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub open: usize,
    pub close: usize,
    pub members: Vec<Member>,
}

impl Object {
    /// The last member named `key` (the one a JSON reader keeps).
    pub fn member(&self, key: &str) -> Option<&Member> {
        self.members.iter().rev().find(|m| m.key == key)
    }
}

/// The top-level object of `bytes`.
pub fn root(bytes: &[u8]) -> Parsed<Object> {
    let start = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };
    let open = skip(bytes, start)?;
    if bytes.get(open) != Some(&b'{') {
        return Err(Malformed);
    }
    let object = object(bytes, open)?;
    if skip(bytes, object.close + 1)? != bytes.len() {
        return Err(Malformed);
    }
    Ok(object)
}

/// The object that starts at `open`.
pub fn object(bytes: &[u8], open: usize) -> Parsed<Object> {
    let mut members = Vec::new();
    let mut index = skip(bytes, open + 1)?;
    loop {
        match bytes.get(index) {
            None => return Err(Malformed),
            Some(b'}') => {
                return Ok(Object {
                    open,
                    close: index,
                    members,
                })
            }
            Some(b'"') => {}
            Some(_) => return Err(Malformed),
        }
        let key_end = string_end(bytes, index)?;
        let key: String = serde_json::from_slice(&bytes[index..key_end]).map_err(|_| Malformed)?;
        let colon = skip(bytes, key_end)?;
        if bytes.get(colon) != Some(&b':') {
            return Err(Malformed);
        }
        let value_start = skip(bytes, colon + 1)?;
        let value_end = value_end(bytes, value_start)?;
        let mut member = Member {
            key,
            key_start: index,
            value_start,
            value_end,
            comma: None,
        };
        let next = skip(bytes, value_end)?;
        match bytes.get(next) {
            Some(b',') => {
                member.comma = Some(next);
                members.push(member);
                index = skip(bytes, next + 1)?;
            }
            Some(b'}') => {
                members.push(member);
                return Ok(Object {
                    open,
                    close: next,
                    members,
                });
            }
            _ => return Err(Malformed),
        }
    }
}

/// Where the value that starts at `start` ends.
pub fn value_end(bytes: &[u8], start: usize) -> Parsed<usize> {
    match bytes.get(start) {
        None => Err(Malformed),
        Some(b'"') => string_end(bytes, start),
        Some(b'{') => Ok(object(bytes, start)?.close + 1),
        Some(b'[') => {
            let mut index = skip(bytes, start + 1)?;
            loop {
                match bytes.get(index) {
                    None => return Err(Malformed),
                    Some(b']') => return Ok(index + 1),
                    _ => {}
                }
                let next = skip(bytes, value_end(bytes, index)?)?;
                match bytes.get(next) {
                    Some(b',') => index = skip(bytes, next + 1)?,
                    Some(b']') => return Ok(next + 1),
                    _ => return Err(Malformed),
                }
            }
        }
        Some(_) => {
            // A number, true, false or null.
            let length = bytes[start..]
                .iter()
                .take_while(|&&b| is_bare_character(b))
                .count();
            if length == 0 {
                return Err(Malformed);
            }
            Ok(start + length)
        }
    }
}

fn is_bare_character(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'E')
}

fn string_end(bytes: &[u8], start: usize) -> Parsed<usize> {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return Ok(index + 1),
            b'\n' => return Err(Malformed),
            _ => index += 1,
        }
    }
    Err(Malformed)
}

/// The first offset from `index` that isn't whitespace or a comment.
pub fn skip(bytes: &[u8], mut index: usize) -> Parsed<usize> {
    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\n' | b'\r' => index += 1,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let mut end = index + 2;
                while end + 1 < bytes.len() && !(bytes[end] == b'*' && bytes[end + 1] == b'/') {
                    end += 1;
                }
                if end + 1 >= bytes.len() {
                    return Err(Malformed);
                }
                index = end + 2;
            }
            _ => return Ok(index),
        }
    }
    Ok(index)
}

/// The value at `start..end` as JSON (comments and trailing commas
/// dropped); `None` when it isn't JSON then.
pub fn value(bytes: &[u8], start: usize, end: usize) -> Option<serde_json::Value> {
    let mut plain: Vec<u8> = Vec::new();
    let mut index = start;
    while index < end {
        let byte = bytes[index];
        if byte == b'"' {
            if let Ok(close) = string_end(bytes, index) {
                plain.extend_from_slice(&bytes[index..close]);
                index = close;
                continue;
            }
        }
        if byte == b'/' {
            if let Ok(next) = skip(bytes, index) {
                if next > index {
                    plain.push(b' ');
                    index = next;
                    continue;
                }
            }
        }
        if byte == b',' {
            if let Ok(next) = skip(bytes, index + 1) {
                if next < end && matches!(bytes[next], b'}' | b']') {
                    index = next;
                    continue;
                }
            }
        }
        plain.push(byte);
        index += 1;
    }
    serde_json::from_slice(&plain).ok()
}

/// The leading whitespace of the line holding `offset`, and whether only
/// whitespace comes before `offset` on it.
fn line_indent(bytes: &[u8], offset: usize) -> (usize, bool) {
    let mut start = offset;
    while start > 0 && bytes[start - 1] != b'\n' {
        start -= 1;
    }
    let mut end = start;
    while end < offset && matches!(bytes[end], b' ' | b'\t') {
        end += 1;
    }
    (end - start, end == offset)
}

/// A change: replace `range` with `text`.
pub type Change = (Range<usize>, String);

/// `bytes` with `changes` made (from the end backwards, so earlier offsets
/// stay right).
pub fn applying(changes: &[Change], bytes: &[u8]) -> Vec<u8> {
    let mut sorted: Vec<&Change> = changes.iter().collect();
    sorted.sort_by_key(|change| std::cmp::Reverse(change.0.start));
    let mut result = bytes.to_vec();
    for (range, text) in sorted {
        result.splice(range.clone(), text.bytes());
    }
    result
}

/// Take `member` out of `object`: its lines when it has them to itself,
/// with the comma that separated it.
pub fn removing(member: &Member, object: &Object, bytes: &[u8]) -> Vec<Change> {
    let mut start = member.key_start;
    let mut end = member.comma.map_or(member.value_end, |comma| comma + 1);
    let (indent, first) = line_indent(bytes, start);
    let mut rest = end;
    while rest < bytes.len() && matches!(bytes[rest], b' ' | b'\t' | b'\r') {
        rest += 1;
    }
    if first && bytes.get(rest) == Some(&b'\n') {
        start -= indent;
        end = rest + 1;
    }
    let mut changes: Vec<Change> = vec![(start..end, String::new())];
    // The last member, without a trailing comma: the one before it loses
    // its separator.
    if member.comma.is_none() {
        let index = object
            .members
            .iter()
            .position(|m| m.key_start == member.key_start);
        if let Some(previous) = index.filter(|&i| i > 0).map(|i| &object.members[i - 1]) {
            if let Some(comma) = previous.comma {
                changes.push((comma..comma + 1, String::new()));
            }
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_comments_and_trailing_commas() {
        let text = br#"{
  // a comment
  "a": [1, 2,], /* another */
  "b": { "c": "d//e", },
  "a": true,
}"#;
        let object = root(text).unwrap();
        let keys: Vec<&str> = object.members.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "a"]);
        let a = object.member("a").unwrap();
        assert_eq!(
            &text[a.value_start..a.value_end],
            b"true",
            "the last one counts"
        );
        let b = &object.members[1];
        assert_eq!(
            value(text, b.value_start, b.value_end),
            Some(serde_json::json!({"c": "d//e"}))
        );
        for broken in [
            &b"{"[..],
            b"[]",
            b"{ \"a\": 1 } x",
            b"{ \"a\" 1 }",
            b"{ /* }",
        ] {
            assert_eq!(
                root(broken),
                Err(Malformed),
                "{}",
                String::from_utf8_lossy(broken)
            );
        }
    }

    #[test]
    fn removing_a_member_keeps_the_rest() {
        let text = b"{\n  \"a\": 1,\n  \"b\": 2\n}\n";
        let object = root(text).unwrap();
        let b = object.member("b").unwrap();
        let removed = applying(&removing(b, &object, text), text);
        assert_eq!(String::from_utf8(removed).unwrap(), "{\n  \"a\": 1\n}\n");
        let a = object.member("a").unwrap();
        let removed = applying(&removing(a, &object, text), text);
        assert_eq!(String::from_utf8(removed).unwrap(), "{\n  \"b\": 2\n}\n");
    }
}
