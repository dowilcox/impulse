//! The History surface's filter field: free text plus `key:value` tokens (the
//! Swift `HistoryQuery`, which lives in ImpulseKit there).
//!
//! ```text
//! author:jane  path:src/  since:2w  until:2026-01-01  fix crash
//! ```
//!
//! Tokens go to `git log` (so they reach commits that aren't loaded yet); the
//! free text filters the loaded rows by subject, author, SHA prefix and refs.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    /// Free text, matched against loaded commits.
    pub text: String,
    pub author: Option<String>,
    pub path: Option<String>,
    /// A git date ("2.weeks.ago", "2026-01-01 00:00:00", "yesterday").
    pub since: Option<String>,
    pub until: Option<String>,
}

/// The key a token names, from the keys the field understands and their
/// aliases.
fn key(name: &str) -> Option<&'static str> {
    match name {
        "author" | "by" => Some("author"),
        "path" | "file" | "in" => Some("path"),
        "since" | "after" => Some("since"),
        "until" | "before" => Some("until"),
        _ => None,
    }
}

/// The key before a `key:value` token's colon (not at its start).
fn token_key(token: &str) -> Option<&'static str> {
    let colon = token.find(':').filter(|&c| c > 0)?;
    key(&token[..colon].to_lowercase())
}

impl HistoryQuery {
    pub fn parse(input: &str) -> HistoryQuery {
        let mut query = HistoryQuery::default();
        let mut words = Vec::new();
        for token in tokenize(input) {
            match token_key(&token) {
                Some(key) => {
                    let colon = token.find(':').unwrap();
                    let value = unquote(&token[colon + 1..]);
                    if value.is_empty() {
                        continue;
                    }
                    match key {
                        "author" => query.author = Some(value),
                        "path" => query.path = Some(value),
                        "since" => query.since = Some(git_date(&value, false)),
                        _ => query.until = Some(git_date(&value, true)),
                    }
                }
                None => words.push(unquote(&token)),
            }
        }
        query.text = words.join(" ");
        query
    }

    /// The parts `git log` applies (everything but the free text).
    pub fn server(&self) -> HistoryQuery {
        HistoryQuery {
            text: String::new(),
            ..self.clone()
        }
    }

    pub fn has_server_filters(&self) -> bool {
        self.server() != HistoryQuery::default()
    }

    /// `git log` options for the tokens (the path goes after `--`
    /// separately).
    pub fn git_arguments(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(author) = &self.author {
            args.push(format!("--author={author}"));
            args.push("--regexp-ignore-case".into());
        }
        if let Some(since) = &self.since {
            args.push(format!("--since={since}"));
        }
        if let Some(until) = &self.until {
            args.push(format!("--until={until}"));
        }
        args
    }

    /// `input` with `key:` set to `value` (replacing an existing token for
    /// the same key or alias), or removed when `value` is `None`.
    pub fn setting(key_name: &str, value: Option<&str>, input: &str) -> String {
        let lowered = key_name.to_lowercase();
        let canonical = key(&lowered).map(str::to_string).unwrap_or(lowered);
        let mut tokens: Vec<String> = tokenize(input)
            .into_iter()
            .filter(|token| token_key(token) != Some(canonical.as_str()))
            .collect();
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            let quoted = if value.contains(' ') {
                format!("\"{value}\"")
            } else {
                value.to_string()
            };
            tokens.insert(0, format!("{canonical}:{quoted}"));
        }
        tokens.join(" ")
    }
}

/// `2w` becomes `2.weeks.ago` (also h, d, m for months, y); a bare
/// `2026-01-31` covers the whole day (git would otherwise read it as that
/// date at the current time); anything else is passed to git as written.
pub fn git_date(value: &str, end_of_day: bool) -> String {
    if let Some(last) = value.chars().last() {
        let unit = match last.to_ascii_lowercase() {
            'h' => Some("hours"),
            'd' => Some("days"),
            'w' => Some("weeks"),
            'm' => Some("months"),
            'y' => Some("years"),
            _ => None,
        };
        if let (Some(unit), Ok(count)) =
            (unit, value[..value.len() - last.len_utf8()].parse::<i64>())
        {
            if count >= 0 {
                return format!("{count}.{unit}.ago");
            }
        }
    }
    let bytes = value.as_bytes();
    let is_day = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if is_day {
        return format!(
            "{value}{}",
            if end_of_day { " 23:59:59" } else { " 00:00:00" }
        );
    }
    value.to_string()
}

/// Whitespace-separated words; double quotes keep spaces
/// (`author:"Jane Doe"`).
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in input.chars() {
        if character == '"' {
            quoted = !quoted;
            current.push(character);
        } else if character.is_whitespace() && !quoted {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn unquote(value: &str) -> String {
    value.replace('"', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tokens_aliases_and_free_text() {
        let query =
            HistoryQuery::parse("by:\"Jane Doe\" in:src/ since:2w until:2026-01-31 fix crash");
        assert_eq!(query.author.as_deref(), Some("Jane Doe"));
        assert_eq!(query.path.as_deref(), Some("src/"));
        assert_eq!(query.since.as_deref(), Some("2.weeks.ago"));
        assert_eq!(query.until.as_deref(), Some("2026-01-31 23:59:59"));
        assert_eq!(query.text, "fix crash");
        assert!(query.has_server_filters());
        assert_eq!(
            query.git_arguments(),
            [
                "--author=Jane Doe",
                "--regexp-ignore-case",
                "--since=2.weeks.ago",
                "--until=2026-01-31 23:59:59"
            ]
        );
        assert_eq!(HistoryQuery::parse("note: x").text, "note: x");
        assert_eq!(git_date("yesterday", false), "yesterday");
        assert_eq!(git_date("2026-01-31", false), "2026-01-31 00:00:00");
    }

    #[test]
    fn setting_replaces_a_key_and_its_aliases() {
        assert_eq!(
            HistoryQuery::setting("author", Some("Jane Doe"), "by:bob fix"),
            "author:\"Jane Doe\" fix"
        );
        assert_eq!(HistoryQuery::setting("path", None, "in:src fix"), "fix");
    }
}
