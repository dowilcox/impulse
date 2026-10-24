//! What the terminal's find bar asks for (the Swift `TerminalFindQuery`),
//! turned into the regex the terminal core searches with (alacritty's regex
//! syntax, which would otherwise use smart case).

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalFindQuery {
    pub text: String,
    pub case_sensitive: bool,
    /// Treat `text` as a regular expression instead of literal text.
    pub regex: bool,
    pub whole_word: bool,
}

impl TerminalFindQuery {
    /// Literal, case-insensitive text.
    pub fn new(text: impl Into<String>) -> Self {
        TerminalFindQuery {
            text: text.into(),
            ..Default::default()
        }
    }

    /// The pattern for the terminal core, or `None` when there's nothing to
    /// find.
    pub fn pattern(&self) -> Option<String> {
        if self.text.is_empty() {
            return None;
        }
        let mut body = if self.regex {
            self.text.clone()
        } else {
            escape(&self.text)
        };
        if self.whole_word {
            // ASCII boundaries: the core's lazy DFA can't do Unicode ones.
            body = format!("(?-u:\\b)(?:{body})(?-u:\\b)");
        }
        // Explicit either way, overriding the core's smart case.
        let case = if self.case_sensitive { "(?-i)" } else { "(?i)" };
        Some(format!("{case}{body}"))
    }
}

/// Escape regex metacharacters (the same set as `regex::escape`).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(text: &str) -> TerminalFindQuery {
        TerminalFindQuery::new(text)
    }

    #[test]
    fn literal_text_is_escaped_and_case_is_explicit() {
        assert_eq!(query("").pattern(), None);
        assert_eq!(
            query("a.b(c)").pattern().as_deref(),
            Some("(?i)a\\.b\\(c\\)")
        );
        let sensitive = TerminalFindQuery {
            case_sensitive: true,
            ..query("Error")
        };
        assert_eq!(sensitive.pattern().as_deref(), Some("(?-i)Error"));
        assert_eq!(
            query("Error").pattern().as_deref(),
            Some("(?i)Error"),
            "insensitive even with capitals"
        );
    }

    #[test]
    fn regex_and_whole_word() {
        let regex = TerminalFindQuery {
            regex: true,
            ..query("err(or)?")
        };
        assert_eq!(regex.pattern().as_deref(), Some("(?i)err(or)?"));
        let word = TerminalFindQuery {
            whole_word: true,
            ..query("id")
        };
        assert_eq!(
            word.pattern().as_deref(),
            Some("(?i)(?-u:\\b)(?:id)(?-u:\\b)")
        );
        let both = TerminalFindQuery {
            regex: true,
            whole_word: true,
            ..query("a|b")
        };
        assert_eq!(
            both.pattern().as_deref(),
            Some("(?i)(?-u:\\b)(?:a|b)(?-u:\\b)"),
            "alternation stays inside the boundaries"
        );
    }

    #[test]
    fn escape_covers_every_metacharacter() {
        assert_eq!(
            escape("\\.+*?()|[]{}^$#&-~"),
            "\\\\\\.\\+\\*\\?\\(\\)\\|\\[\\]\\{\\}\\^\\$\\#\\&\\-\\~"
        );
        assert_eq!(escape("plain words"), "plain words");
        assert_eq!(
            escape("\\.+*?()|[]{}^$#&-~"),
            regex::escape("\\.+*?()|[]{}^$#&-~")
        );
    }
}
