//! Impulse's theme for GPUI Kit's own components (the editor's search panel,
//! completion and hover popovers, menus, scrollbars), so nothing in the window
//! shows GPUI Kit's default theme. Chrome colors come from `ChromePalette`,
//! syntax colors from the theme the way `themeToMonaco` maps them for Monaco.

use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::component::{Theme, ThemeMode, ThemeTokens};
use gpui_kit::{px, App, Hsla};
use impulse_kit::chrome::{ChromePalette, Rgba};
use impulse_kit::theme::ResolvedTheme;
use serde_json::json;

fn hsla(c: Rgba) -> Hsla {
    gpui_kit::Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
    .into()
}

/// `settings` gives the editor's font (`font_family`, `font_size`).
pub fn apply(
    theme: &ResolvedTheme,
    p: &ChromePalette,
    settings: &impulse_kit::settings::Settings,
    cx: &mut App,
) {
    let mode = if theme.is_light {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(mode, None, cx);

    let k = Theme::global_mut(cx);
    k.font_family = "Inter".into();
    k.font_size = px(13.);
    k.mono_font_family = settings.font_family.clone().into();
    k.mono_font_size = px(settings.font_size as f32);
    k.radius = px(impulse_kit::chrome::metrics::RADIUS);
    k.radius_lg = px(impulse_kit::chrome::metrics::RADIUS_LARGE);

    let c = &mut k.colors;
    let text = hsla(p.text);
    let secondary_text = hsla(p.text_secondary);
    c.background = hsla(p.content);
    c.foreground = text;
    c.border = hsla(p.hairline);
    c.input = hsla(p.hairline_strong);
    c.ring = hsla(p.focus_ring);
    c.caret = hsla(Rgba::from_hex(&theme.cursor));
    c.selection = hsla(p.selection_strong);
    c.link = hsla(p.info);
    c.link_hover = hsla(p.info);
    c.link_active = hsla(p.info);

    c.primary = hsla(p.accent);
    c.primary_hover = hsla(p.accent.mix(p.text, 0.12));
    c.primary_active = hsla(p.accent.mix(p.text, 0.2));
    c.primary_foreground = hsla(p.text_on_accent);
    c.secondary = hsla(p.raised);
    c.secondary_hover = hsla(p.raised.mix(p.text, 0.06));
    c.secondary_active = hsla(p.raised.mix(p.text, 0.1));
    c.secondary_foreground = text;
    c.accent = hsla(p.hover);
    c.accent_foreground = text;
    c.muted = hsla(p.raised);
    c.muted_foreground = secondary_text;

    c.popover = hsla(p.overlay.mix(p.text, 0.02));
    c.popover_foreground = text;
    c.overlay = hsla(Rgba::BLACK.opacity(0.3));
    c.window_border = hsla(p.hairline_strong);

    c.list = hsla(p.overlay);
    c.list_even = hsla(p.overlay);
    c.list_head = hsla(p.panel);
    c.list_hover = hsla(p.hover);
    c.list_active = hsla(p.selection);
    c.list_active_border = hsla(p.accent);

    c.tab = hsla(p.chrome);
    c.tab_active = hsla(p.raised);
    c.tab_foreground = secondary_text;
    c.tab_active_foreground = text;
    c.tab_bar = hsla(p.chrome);
    c.title_bar = hsla(p.chrome);
    c.title_bar_border = hsla(p.hairline);
    c.status_bar = hsla(p.chrome);
    c.status_bar_border = hsla(p.hairline);
    c.sidebar = hsla(p.panel);
    c.sidebar_foreground = text;
    c.sidebar_border = hsla(p.hairline);
    c.sidebar_accent = hsla(p.selection);
    c.sidebar_accent_foreground = text;
    c.sidebar_primary = hsla(p.accent);
    c.sidebar_primary_foreground = hsla(p.text_on_accent);

    c.scrollbar = hsla(Rgba::BLACK.opacity(0.0));
    c.scrollbar_thumb = hsla(p.text.opacity(0.2));
    c.scrollbar_thumb_hover = hsla(p.text.opacity(0.35));

    c.success = hsla(p.success);
    c.warning = hsla(p.warning);
    c.danger = hsla(p.danger);
    c.info = hsla(p.info);
    c.red = hsla(p.danger);
    c.green = hsla(p.success);
    c.blue = hsla(p.info);
    c.yellow = hsla(p.warning);
    c.magenta = hsla(p.working);
    c.cyan = hsla(Rgba::from_hex(&theme.cyan));

    k.tokens = ThemeTokens::from(&k.colors);
    k.highlight_theme = highlight_theme(theme).into();
    Theme::sync_base(cx);
    cx.refresh_windows();
}

/// Impulse's theme as GPUI Kit's highlight theme (Zed's theme format), with
/// the scopes `themeToMonaco` colors.
fn highlight_theme(theme: &ResolvedTheme) -> HighlightTheme {
    let style = |color: &str| json!({ "color": color });
    let italic = |color: &str| json!({ "color": color, "font_style": "italic" });
    let syntax: serde_json::Map<String, serde_json::Value> = [
        ("comment", italic(&theme.syntax_comment)),
        ("comment_doc", italic(&theme.syntax_comment)),
        ("keyword", style(&theme.syntax_keyword)),
        ("boolean", style(&theme.syntax_constant)),
        ("constant", style(&theme.syntax_constant)),
        ("number", style(&theme.syntax_number)),
        ("string", style(&theme.syntax_string)),
        ("string.escape", style(&theme.syntax_escape)),
        ("string.regex", style(&theme.syntax_regexp)),
        ("string.special", style(&theme.syntax_string)),
        ("operator", style(&theme.syntax_operator)),
        ("type", style(&theme.syntax_type)),
        ("constructor", style(&theme.syntax_type)),
        ("enum", style(&theme.syntax_type)),
        ("variant", style(&theme.syntax_constant)),
        ("function", style(&theme.syntax_function)),
        ("attribute", style(&theme.syntax_attribute)),
        ("tag", style(&theme.syntax_tag)),
        ("property", style(&theme.syntax_variable)),
        ("variable", style(&theme.syntax_variable)),
        ("variable.special", style(&theme.syntax_keyword)),
        ("label", style(&theme.syntax_attribute)),
        ("punctuation", style(&theme.syntax_delimiter)),
        ("punctuation.bracket", style(&theme.syntax_delimiter)),
        ("punctuation.delimiter", style(&theme.syntax_delimiter)),
        ("link_uri", style(&theme.syntax_link)),
        ("link_text", style(&theme.syntax_link)),
        ("title", style(&theme.syntax_keyword)),
    ]
    .into_iter()
    .map(|(scope, style)| (scope.to_string(), style))
    .collect();
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
            "error": theme.red,
            "warning": theme.yellow,
            "info": theme.blue,
            "hint": theme.fg_comment,
            "syntax": syntax,
        }
    });
    serde_json::from_value(highlight).unwrap_or_else(|error| {
        log::warn!("theme {}: {error}", theme.id);
        if theme.is_light {
            (*HighlightTheme::default_light()).clone()
        } else {
            (*HighlightTheme::default_dark()).clone()
        }
    })
}
