//! The History surface's filter field: free text plus `key:value` tokens.
//!
//! ```text
//! author:jane  path:src/  since:2w  until:2026-01-01  fix crash
//! ```
//!
//! Tokens go to `git log` (so they reach commits that aren't loaded yet);
//! the free text filters the loaded rows by subject, author, SHA prefix and
//! refs.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    /// Free text, matched against loaded commits.
    pub text: String,
    pub author: Option<String>,
    pub path: Option<String>,
    /// A git date ("2.weeks.ago", "2026-01-01", "yesterday").
    pub since: Option<String>,
    pub until: Option<String>,
}

/// The key a token's name stands for, aliases included.
fn canonical_key(name: &str) -> Option<&'static str> {
    Some(match name {
        "author" | "by" => "author",
        "path" | "file" | "in" => "path",
        "since" | "after" => "since",
        "until" | "before" => "until",
        _ => return None,
    })
}

/// The key of a `key:value` token, when it names one the field knows.
fn token_key(token: &str) -> Option<(&'static str, &str)> {
    let colon = token.find(':').filter(|&c| c > 0)?;
    let key = canonical_key(&token[..colon].to_lowercase())?;
    Some((key, &token[colon + 1..]))
}

impl HistoryQuery {
    pub fn parse(input: &str) -> HistoryQuery {
        let mut query = HistoryQuery::default();
        let mut words: Vec<String> = Vec::new();
        for token in tokenize(input) {
            if let Some((key, value)) = token_key(&token) {
                let value = unquote(value);
                if value.is_empty() {
                    continue;
                }
                match key {
                    "author" => query.author = Some(value),
                    "path" => query.path = Some(value),
                    "since" => query.since = Some(git_date(&value, false)),
                    _ => query.until = Some(git_date(&value, true)),
                }
            } else {
                words.push(unquote(&token));
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
    pub fn setting(key: &str, value: Option<&str>, input: &str) -> String {
        let lower = key.to_lowercase();
        let canonical = canonical_key(&lower).map_or(lower.clone(), str::to_string);
        let mut tokens: Vec<String> = tokenize(input)
            .into_iter()
            .filter(|token| token_key(token).is_none_or(|(k, _)| k != canonical))
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

/// `2w` to `2.weeks.ago` (also h, d, m for months, y); a bare `2026-01-31`
/// covers the whole day (git would otherwise read it as that date at the
/// current time); anything else is passed to git as written.
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
        let count = value[..value.len() - last.len_utf8()].parse::<i64>().ok();
        if let (Some(unit), Some(count)) = (unit, count.filter(|c| *c >= 0)) {
            return format!("{count}.{unit}.ago");
        }
    }
    let bytes = value.as_bytes();
    let is_day = bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        });
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
    for c in input.chars() {
        if c == '"' {
            quoted = !quoted;
            current.push(c);
        } else if c.is_whitespace() && !quoted {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
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
    fn tokens_and_free_text() {
        let query = HistoryQuery::parse("author:jane fix  crash path:src/ since:2w");
        assert_eq!(query.author.as_deref(), Some("jane"));
        assert_eq!(query.path.as_deref(), Some("src/"));
        assert_eq!(query.since.as_deref(), Some("2.weeks.ago"));
        assert_eq!(query.until, None);
        assert_eq!(query.text, "fix crash");
        assert!(query.has_server_filters());
    }

    #[test]
    fn aliases_quotes_and_unknown_keys() {
        let query =
            HistoryQuery::parse("by:\"Jane Doe\" before:2026-01-01 after:yesterday fix:bug");
        assert_eq!(query.author.as_deref(), Some("Jane Doe"));
        assert_eq!(
            query.until.as_deref(),
            Some("2026-01-01 23:59:59"),
            "the whole day"
        );
        assert_eq!(query.since.as_deref(), Some("yesterday"));
        assert_eq!(query.text, "fix:bug", "unknown keys stay free text");
    }

    #[test]
    fn empty_values_and_plain_text() {
        let query = HistoryQuery::parse("author: refactor");
        assert_eq!(query.author, None);
        assert_eq!(query.text, "refactor");
        assert!(!query.has_server_filters());
        assert_eq!(HistoryQuery::parse("").server(), HistoryQuery::default());
    }

    #[test]
    fn git_arguments() {
        let query = HistoryQuery::parse("author:jane since:3d until:1y");
        assert_eq!(
            query.git_arguments(),
            [
                "--author=jane",
                "--regexp-ignore-case",
                "--since=3.days.ago",
                "--until=1.years.ago"
            ]
        );
        assert!(HistoryQuery::parse("words only").git_arguments().is_empty());
    }

    #[test]
    fn relative_dates() {
        assert_eq!(git_date("12h", false), "12.hours.ago");
        assert_eq!(git_date("1M", false), "1.months.ago");
        assert_eq!(git_date("2026-03-01", false), "2026-03-01 00:00:00");
        assert_eq!(git_date("2026-03-01", true), "2026-03-01 23:59:59");
        assert_eq!(git_date("last friday", false), "last friday");
        assert_eq!(git_date("w", false), "w");
    }

    #[test]
    fn setting_replaces_or_removes_a_token_and_its_aliases() {
        assert_eq!(
            HistoryQuery::setting("since", Some("1w"), "fix after:2d"),
            "since:1w fix"
        );
        assert_eq!(
            HistoryQuery::setting("author", Some("Jane Doe"), ""),
            "author:\"Jane Doe\""
        );
        assert_eq!(HistoryQuery::setting("author", None, "by:jane fix"), "fix");
        assert_eq!(
            HistoryQuery::parse(&HistoryQuery::setting("author", Some("Jane Doe"), "x"))
                .author
                .as_deref(),
            Some("Jane Doe")
        );
    }
}
