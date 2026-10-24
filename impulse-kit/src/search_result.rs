//! One project-search hit (a file name or a line of content), with the
//! snake_case JSON keys of the original Rust `impulse_core::search` and the
//! Swift `SearchResult`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub line_number: Option<u32>,
    #[serde(default)]
    pub line_content: Option<String>,
    #[serde(default)]
    pub column_start: Option<u32>,
    #[serde(default)]
    pub column_end: Option<u32>,
    /// `"file"` for a name match, `"content"` for a line.
    pub match_type: String,
}

impl SearchResult {
    /// A file-name hit.
    pub fn file(path: impl Into<String>, name: impl Into<String>) -> Self {
        SearchResult {
            path: path.into(),
            name: name.into(),
            line_number: None,
            line_content: None,
            column_start: None,
            column_end: None,
            match_type: "file".into(),
        }
    }
}
