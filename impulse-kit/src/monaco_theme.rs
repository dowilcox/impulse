//! A theme as a Monaco editor theme definition (the Swift `themeToMonaco`,
//! from `theme_to_monaco` in `impulse-editor/src/protocol.rs`). The JSON is
//! what `monaco.editor.defineTheme` takes: token rules in a fixed order
//! (colors without `#`, `foreground`/`font_style` left out when unset) and
//! the editor's colors under their Monaco keys, plus Impulse's own diff
//! colors.

use serde::{Deserialize, Serialize};

use crate::theme::ResolvedTheme;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonacoThemeDefinition {
    /// `"vs"` for light themes, `"vs-dark"` for dark ones.
    pub base: String,
    pub inherit: bool,
    pub rules: Vec<MonacoTokenRule>,
    pub colors: MonacoThemeColors,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonacoTokenRule {
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_style: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonacoThemeColors {
    #[serde(rename = "editor.background")]
    pub editor_background: String,
    #[serde(rename = "editor.foreground")]
    pub editor_foreground: String,
    #[serde(rename = "editor.lineHighlightBackground")]
    pub editor_line_highlight_background: String,
    #[serde(rename = "editor.selectionBackground")]
    pub editor_selection_background: String,
    #[serde(rename = "editorCursor.foreground")]
    pub editor_cursor_foreground: String,
    #[serde(rename = "editorLineNumber.foreground")]
    pub editor_line_number_foreground: String,
    #[serde(rename = "editorLineNumber.activeForeground")]
    pub editor_line_number_active_foreground: String,
    #[serde(rename = "editorWidget.background")]
    pub editor_widget_background: String,
    #[serde(rename = "editorSuggestWidget.background")]
    pub editor_suggest_widget_background: String,
    #[serde(rename = "editorSuggestWidget.selectedBackground")]
    pub editor_suggest_widget_selected_background: String,
    #[serde(rename = "editorHoverWidget.background")]
    pub editor_hover_widget_background: String,
    #[serde(rename = "editorGutter.background")]
    pub editor_gutter_background: String,
    #[serde(rename = "minimap.background")]
    pub minimap_background: String,
    #[serde(rename = "scrollbarSlider.background")]
    pub scrollbar_slider_background: String,
    #[serde(rename = "scrollbarSlider.hoverBackground")]
    pub scrollbar_slider_hover_background: String,
    #[serde(rename = "scrollbarSlider.activeBackground")]
    pub scrollbar_slider_active_background: String,
    #[serde(rename = "impulse.diffAddedColor")]
    pub diff_added_color: String,
    #[serde(rename = "impulse.diffModifiedColor")]
    pub diff_modified_color: String,
    #[serde(rename = "impulse.diffDeletedColor")]
    pub diff_deleted_color: String,
}

/// The Monaco theme for a resolved theme.
pub fn theme_to_monaco(theme: &ResolvedTheme) -> MonacoThemeDefinition {
    let strip = |c: &str| c.trim_start_matches('#').to_string();
    let hash = |c: &str| format!("#{}", strip(c));
    let rule = |token: &str, color: &str| MonacoTokenRule {
        token: token.into(),
        foreground: Some(strip(color)),
        font_style: None,
    };
    let styled = |token: &str, color: &str, style: &str| MonacoTokenRule {
        font_style: Some(style.into()),
        ..rule(token, color)
    };
    let t = theme;

    let rules = vec![
        // Comments (italic)
        styled("comment", &t.syntax_comment, "italic"),
        styled("comment.doc", &t.syntax_comment, "italic"),
        // Keywords
        rule("keyword", &t.syntax_keyword),
        rule("keyword.control", &t.syntax_keyword),
        rule("keyword.declaration", &t.syntax_keyword),
        rule("keyword.type", &t.syntax_keyword),
        rule("keyword.other", &t.syntax_keyword),
        rule("keyword.flow", &t.syntax_keyword),
        rule("keyword.block", &t.syntax_keyword),
        rule("keyword.try", &t.syntax_keyword),
        rule("keyword.catch", &t.syntax_keyword),
        rule("keyword.choice", &t.syntax_keyword),
        rule("keyword.modifier", &t.syntax_keyword),
        // Constants & numbers
        rule("keyword.constant", &t.syntax_constant),
        rule("number", &t.syntax_number),
        rule("number.hex", &t.syntax_number),
        rule("number.float", &t.syntax_number),
        rule("number.binary", &t.syntax_number),
        rule("number.octal", &t.syntax_number),
        rule("constant", &t.syntax_constant),
        rule("string.escape", &t.syntax_escape),
        // Strings
        rule("string", &t.syntax_string),
        rule("string.heredoc", &t.syntax_string),
        rule("string.raw", &t.syntax_string),
        rule("attribute.value", &t.syntax_string),
        // Operators, special strings, predefined
        rule("string.key", &t.syntax_operator),
        rule("string.link", &t.syntax_link),
        rule("operator", &t.syntax_operator),
        rule("keyword.operator", &t.syntax_operator),
        rule("variable.predefined", &t.syntax_operator),
        rule("predefined", &t.syntax_operator),
        // Types, classes, annotations
        rule("type", &t.syntax_type),
        rule("type.identifier", &t.syntax_type),
        rule("class", &t.syntax_type),
        rule("annotation", &t.syntax_attribute),
        rule("namespace", &t.syntax_type),
        rule("constructor", &t.syntax_type),
        rule("attribute.name", &t.syntax_attribute),
        // Functions
        rule("function", &t.syntax_function),
        rule("function.declaration", &t.syntax_function),
        rule("function.call", &t.syntax_function),
        rule("predefined.function", &t.syntax_function),
        // Tags, invalid, regexp
        rule("string.escape.invalid", &t.syntax_tag),
        rule("string.invalid", &t.syntax_tag),
        rule("regexp", &t.syntax_regexp),
        rule("tag", &t.syntax_tag),
        rule("metatag", &t.syntax_tag),
        rule("invalid", &t.syntax_tag),
        // Variables, emphasis
        rule("variable", &t.syntax_variable),
        styled("emphasis", &t.syntax_variable, "italic"),
        // Delimiters
        rule("delimiter", &t.syntax_delimiter),
        // Strong (bold)
        styled("strong", &t.orange, "bold"),
    ];

    let colors = MonacoThemeColors {
        editor_background: hash(&t.bg),
        editor_foreground: hash(&t.fg),
        editor_line_highlight_background: hash(&t.bg_highlight),
        editor_selection_background: hash(&t.selection),
        editor_cursor_foreground: hash(&t.cursor),
        editor_line_number_foreground: hash(&t.fg_comment),
        editor_line_number_active_foreground: hash(&t.fg),
        editor_widget_background: hash(&t.bg_dark),
        editor_suggest_widget_background: hash(&t.bg_dark),
        editor_suggest_widget_selected_background: hash(&t.bg_highlight),
        editor_hover_widget_background: hash(&t.bg_dark),
        editor_gutter_background: hash(&t.bg),
        minimap_background: hash(&t.bg_dark),
        scrollbar_slider_background: format!("{}40", hash(&t.fg_comment)),
        scrollbar_slider_hover_background: format!("{}80", hash(&t.fg_comment)),
        scrollbar_slider_active_background: format!("{}A0", hash(&t.fg_comment)),
        diff_added_color: hash(&t.git_added),
        diff_modified_color: hash(&t.git_modified),
        diff_deleted_color: hash(&t.git_deleted),
    };

    MonacoThemeDefinition {
        base: if t.is_light { "vs" } else { "vs-dark" }.into(),
        inherit: true,
        rules,
        colors,
    }
}
