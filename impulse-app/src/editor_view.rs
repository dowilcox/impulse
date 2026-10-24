//! An editor pane on GPUI Kit's `Editor`: tree-sitter highlighting, folding,
//! line numbers, multi-cursor and search, colored from Impulse's theme with
//! the same choices `themeToMonaco` makes for Monaco.

use std::path::Path;

use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::component::input::{Editor, EditorState, TabSize};
use gpui_kit::component::Theme;
use gpui_kit::{prelude::*, px, App, Context, Entity, Window};
use impulse_kit::theme::ResolvedTheme;
use serde_json::json;

pub struct EditorView {
    state: Entity<EditorState>,
}

/// The editor's language from the file name (the names GPUI Kit's grammars
/// register).
fn language_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
    {
        "rs" => "rust",
        "toml" => "toml",
        "md" => "markdown",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "sh" | "bash" | "zsh" => "bash",
        _ => "text",
    }
}

/// Impulse's theme as GPUI Kit's highlight theme (Zed's theme format), with
/// the scopes `themeToMonaco` colors.
pub fn apply_theme(theme: &ResolvedTheme, cx: &mut App) {
    let style = |color: &str| json!({ "color": color });
    let italic = |color: &str| json!({ "color": color, "font_style": "italic" });
    let highlight = json!({
        "name": theme.name,
        "appearance": if theme.is_light { "light" } else { "dark" },
        "style": {
            "editor.background": theme.bg,
            "editor.foreground": theme.fg,
            "editor.active_line.background": theme.bg_highlight,
            "editor.line_number": theme.fg_comment,
            "editor.active_line_number": theme.fg,
            "editor.invisible": theme.fg_comment,
            "syntax": {
                "comment": italic(&theme.syntax_comment),
                "comment_doc": italic(&theme.syntax_comment),
                "keyword": style(&theme.syntax_keyword),
                "boolean": style(&theme.syntax_constant),
                "constant": style(&theme.syntax_constant),
                "number": style(&theme.syntax_number),
                "string": style(&theme.syntax_string),
                "string.escape": style(&theme.syntax_escape),
                "string.regex": style(&theme.syntax_regexp),
                "string.special": style(&theme.syntax_string),
                "operator": style(&theme.syntax_operator),
                "type": style(&theme.syntax_type),
                "constructor": style(&theme.syntax_type),
                "enum": style(&theme.syntax_type),
                "variant": style(&theme.syntax_constant),
                "function": style(&theme.syntax_function),
                "attribute": style(&theme.syntax_attribute),
                "tag": style(&theme.syntax_tag),
                "property": style(&theme.syntax_variable),
                "variable": style(&theme.syntax_variable),
                "variable.special": style(&theme.syntax_keyword),
                "label": style(&theme.syntax_attribute),
                "punctuation": style(&theme.syntax_delimiter),
                "punctuation.bracket": style(&theme.syntax_delimiter),
                "punctuation.delimiter": style(&theme.syntax_delimiter),
                "link_uri": style(&theme.syntax_link),
                "link_text": style(&theme.syntax_link),
                "title": style(&theme.syntax_keyword),
            }
        }
    });
    match serde_json::from_value::<HighlightTheme>(highlight) {
        Ok(highlight) => Theme::global_mut(cx).highlight_theme = highlight.into(),
        Err(error) => log::warn!("theme {}: {error}", theme.id),
    }
}

impl EditorView {
    pub fn open(path: &Path, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language_for(path))
                .line_number(true)
                .folding(true)
                .indent_guides(true)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: false,
                })
                .default_value(text)
        });
        Self { state }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.focus(window, cx));
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Editor::new(&self.state)
            .size_full()
            .font_family("JetBrains Mono")
            .text_size(px(14.))
            .bordered(false)
    }
}
