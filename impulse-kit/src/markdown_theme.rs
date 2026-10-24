//! The Markdown preview's colors and fonts from a theme (the Swift
//! `MarkdownThemeColors` and `themeToMarkdownColors`, from
//! `impulse-editor/src/markdown.rs`). The JSON keys are the original Rust
//! snake_case contract the fixtures were captured with.

use serde::{Deserialize, Serialize};

use crate::theme::ResolvedTheme;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownThemeColors {
    pub bg: String,
    pub fg: String,
    pub heading: String,
    pub link: String,
    pub code_bg: String,
    pub border: String,
    pub blockquote_fg: String,
    // highlight.js token colors
    pub hljs_keyword: String,
    pub hljs_string: String,
    pub hljs_number: String,
    pub hljs_comment: String,
    pub hljs_function: String,
    pub hljs_type: String,
    pub font_family: String,
    pub code_font_family: String,
}

const FONT_FAMILY: &str = "Inter, system-ui, sans-serif";
const CODE_FONT_FAMILY: &str = "'JetBrains Mono', monospace";

impl MarkdownThemeColors {
    /// The palette the renderer falls back to color by color (Tokyo
    /// Night's).
    pub fn fallback() -> Self {
        MarkdownThemeColors {
            bg: "#1a1b26".into(),
            fg: "#c0caf5".into(),
            heading: "#7dcfff".into(),
            link: "#7aa2f7".into(),
            code_bg: "#16161e".into(),
            border: "#292e42".into(),
            blockquote_fg: "#565f89".into(),
            hljs_keyword: "#bb9af7".into(),
            hljs_string: "#9ece6a".into(),
            hljs_number: "#ff9e64".into(),
            hljs_comment: "#565f89".into(),
            hljs_function: "#7aa2f7".into(),
            hljs_type: "#e0af68".into(),
            font_family: FONT_FAMILY.into(),
            code_font_family: CODE_FONT_FAMILY.into(),
        }
    }
}

/// The preview's colors for a theme.
pub fn theme_to_markdown_colors(theme: &ResolvedTheme) -> MarkdownThemeColors {
    MarkdownThemeColors {
        bg: theme.bg.clone(),
        fg: theme.fg.clone(),
        heading: theme.cyan.clone(),
        link: theme.syntax_link.clone(),
        code_bg: theme.bg_dark.clone(),
        border: theme.bg_highlight.clone(),
        blockquote_fg: theme.fg_comment.clone(),
        hljs_keyword: theme.syntax_keyword.clone(),
        hljs_string: theme.syntax_string.clone(),
        hljs_number: theme.syntax_number.clone(),
        hljs_comment: theme.syntax_comment.clone(),
        hljs_function: theme.syntax_function.clone(),
        hljs_type: theme.syntax_type.clone(),
        font_family: FONT_FAMILY.into(),
        code_font_family: CODE_FONT_FAMILY.into(),
    }
}
