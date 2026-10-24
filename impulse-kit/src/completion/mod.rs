//! Completion for the terminal's input bar.
//!
//! - `input`: the inline ghost text (one completed line: history first,
//!   then the word at the cursor) and the path candidates under it;
//! - `engine`: the completion menu, what can come next at the cursor
//!   (commands, subcommands, options, argument values from `specs`, and
//!   paths), with the shell's own answers merged in (`shell_completions`).
//!
//! Offsets are UTF-8 byte offsets into the input, as the shell parser
//! (`crate::shell_parser`) gives them. Candidates serialize as the Swift
//! app's do (`is_dir`, `git_status`, `kept_prefix`).

use serde::{Deserialize, Serialize};

use crate::shell::{quoting, ShellType};
pub use crate::shell_parser::TextSpan;

pub mod engine;
pub mod input;
pub mod shell_completions;
pub mod specs;

/// A single completion candidate for the completion menu.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionCandidate {
    /// Full replacement text for the active token. Directories get a
    /// trailing `/` so accepting one opens the menu again for the next
    /// segment.
    pub value: String,
    /// The label shown in the menu (the entry's basename).
    pub display: String,
    /// The candidate category: `"path"`, or for spec completions
    /// `"command"`, `"subcommand"`, `"option"`, `"branch"`, `"script"`,
    /// `"target"`, ...; `"shell"` for the shell's own answers.
    pub kind: String,
    pub is_dir: bool,
    /// Git status (porcelain code) when cheaply available; `None` on the
    /// typeahead path scan, where computing it per entry would be costly.
    pub git_status: Option<String>,
    /// What it is ("Stage changes", a script's command), for spec
    /// candidates.
    pub detail: Option<String>,
    /// How `value` goes into the input. Impulse's own candidates are literal
    /// text (a file name, a branch), escaped when inserted, after this
    /// leading part kept as is: the `~/` or `$HOME/` a path completion read
    /// as the home folder, usually nothing. `None` for the shell's own
    /// answers, which come as the shell prints them (see
    /// `quoting::completion_text`).
    pub kept_prefix: Option<String>,
}

impl CompletionCandidate {
    /// One of Impulse's own candidates: literal, no detail, no git status.
    pub fn new(
        value: impl Into<String>,
        display: impl Into<String>,
        kind: impl Into<String>,
        is_dir: bool,
    ) -> Self {
        CompletionCandidate {
            value: value.into(),
            display: display.into(),
            kind: kind.into(),
            is_dir,
            git_status: None,
            detail: None,
            kept_prefix: Some(String::new()),
        }
    }

    pub fn with_detail(mut self, detail: Option<impl Into<String>>) -> Self {
        self.detail = detail.map(Into::into);
        self
    }

    /// The shell text that replaces the token `typed` (as typed, up to the
    /// cursor) when this candidate is accepted.
    pub fn insertion(&self, typed: &str, shell: ShellType) -> String {
        let Some(kept) = &self.kept_prefix else {
            return quoting::completion_text(&self.value, typed, shell);
        };
        match self.value.strip_prefix(kept.as_str()) {
            Some(rest) => format!("{kept}{}", quoting::escaped(rest, shell)),
            None => quoting::escaped(&self.value, shell),
        }
    }
}

/// The token span to replace plus the matching candidates.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionResult {
    pub span: TextSpan,
    pub candidates: Vec<CompletionCandidate>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_swift_app_does() {
        let candidate = CompletionCandidate::new("wp-content/", "wp-content", "path", true);
        let json = serde_json::to_value(&candidate).unwrap();
        assert_eq!(json["is_dir"], true);
        assert_eq!(json["git_status"], serde_json::Value::Null);
        assert_eq!(json["kept_prefix"], "");
        // The Swift app leaves out keys that are nil.
        let decoded: CompletionCandidate = serde_json::from_str(
            r#"{"value":"src/","display":"src/","kind":"path","is_dir":true}"#,
        )
        .unwrap();
        assert_eq!(decoded.kept_prefix, None);
        assert_eq!(decoded.git_status, None);
    }
}
