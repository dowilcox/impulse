use crate::css::sanitize_css_color;
use pulldown_cmark::{Options, Parser};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Maximum markdown source size (in bytes) before preview is refused.
/// Beyond this threshold, rendering + highlight.js can cause UI lag.
const MAX_MARKDOWN_SIZE: usize = 1024 * 1024; // 1 MB

/// Theme colors for the rendered markdown preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownThemeColors {
    pub bg: String,
    pub fg: String,
    pub heading: String,
    pub link: String,
    pub code_bg: String,
    pub border: String,
    pub blockquote_fg: String,
    /// highlight.js token colors
    pub hljs_keyword: String,
    pub hljs_string: String,
    pub hljs_number: String,
    pub hljs_comment: String,
    pub hljs_function: String,
    pub hljs_type: String,
    pub font_family: String,
    pub code_font_family: String,
}

/// Recognised markdown file extensions.
const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd", "mkdn"];

/// Check whether a file path is a markdown file based on its extension.
pub fn is_markdown_file(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    MARKDOWN_EXTENSIONS.contains(&ext.as_str())
}

/// Sanitise a CSS font-family value by stripping dangerous chars.
fn sanitize_font_family(value: &str) -> String {
    // Allow alphanumerics, spaces, commas, quotes, hyphens, underscores
    if value
        .chars()
        .all(|c| c.is_alphanumeric() || " ,'\"-_".contains(c))
    {
        value.to_string()
    } else {
        "system-ui, sans-serif".to_string()
    }
}

/// HTML-escape a string for safe interpolation in attributes or text.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Render a markdown source string to a full standalone HTML document
/// with themed CSS and highlight.js code highlighting.
///
/// `highlight_js_path` should be an absolute `file://` path to `highlight.min.js`.
///
/// Returns `None` if the source exceeds the size limit.
pub fn render_markdown_preview(
    source: &str,
    theme: &MarkdownThemeColors,
    highlight_js_path: &str,
) -> Option<String> {
    if source.len() > MAX_MARKDOWN_SIZE {
        log::warn!(
            "Markdown source ({} bytes) exceeds {} byte limit, skipping preview",
            source.len(),
            MAX_MARKDOWN_SIZE
        );
        return None;
    }

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(source, options);
    let mut html_body = String::new();
    pulldown_cmark::html::push_html(&mut html_body, parser);

    // Sanitise rendered HTML to strip scripts, event handlers, iframes, etc.
    let clean_body = ammonia::Builder::default()
        .add_tags(&[
            "pre",
            "code",
            "table",
            "thead",
            "tbody",
            "tfoot",
            "tr",
            "th",
            "td",
            "details",
            "summary",
            "span",
            "dl",
            "dt",
            "dd",
            "sup",
            "sub",
            "kbd",
            "var",
            "samp",
            "mark",
            "figure",
            "figcaption",
            "picture",
            "source",
        ])
        .add_generic_attributes(&["class", "id"])
        .add_tag_attribute_values("input", "type", &["checkbox"])
        .add_tag_attributes("input", &["checked", "disabled"])
        .url_schemes(HashSet::from(["http", "https", "mailto"]))
        .link_rel(Some("noopener noreferrer"))
        .clean(&html_body)
        .to_string();

    // Sanitise theme values
    let bg = sanitize_css_color(&theme.bg, "#1a1b26");
    let fg = sanitize_css_color(&theme.fg, "#c0caf5");
    let heading = sanitize_css_color(&theme.heading, "#7dcfff");
    let link = sanitize_css_color(&theme.link, "#7aa2f7");
    let code_bg = sanitize_css_color(&theme.code_bg, "#16161e");
    let border = sanitize_css_color(&theme.border, "#292e42");
    let blockquote_fg = sanitize_css_color(&theme.blockquote_fg, "#565f89");
    let hljs_keyword = sanitize_css_color(&theme.hljs_keyword, "#bb9af7");
    let hljs_string = sanitize_css_color(&theme.hljs_string, "#9ece6a");
    let hljs_number = sanitize_css_color(&theme.hljs_number, "#ff9e64");
    let hljs_comment = sanitize_css_color(&theme.hljs_comment, "#565f89");
    let hljs_function = sanitize_css_color(&theme.hljs_function, "#7aa2f7");
    let hljs_type = sanitize_css_color(&theme.hljs_type, "#e0af68");
    let font_family = sanitize_font_family(&theme.font_family);
    let code_font_family = sanitize_font_family(&theme.code_font_family);

    // HTML-escape the highlight.js path for safe attribute interpolation
    let hljs_path = html_escape(highlight_js_path);

    Some(format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src file:; img-src file: data: https:; font-src file:;">
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{
    background: {bg};
    color: {fg};
    font-family: {font_family};
    font-size: 15px;
    line-height: 1.6;
    padding: 24px 32px;
    max-width: 900px;
    margin: 0 auto;
}}
h1, h2, h3, h4, h5, h6 {{
    color: {heading};
    margin-top: 1.2em;
    margin-bottom: 0.4em;
    line-height: 1.3;
}}
h1 {{ font-size: 2em; border-bottom: 1px solid {border}; padding-bottom: 0.3em; }}
h2 {{ font-size: 1.5em; border-bottom: 1px solid {border}; padding-bottom: 0.3em; }}
h3 {{ font-size: 1.25em; }}
p {{ margin: 0.6em 0; }}
a {{ color: {link}; text-decoration: none; }}
a:hover {{ text-decoration: underline; }}
code {{
    font-family: {code_font_family};
    background: {code_bg};
    padding: 0.15em 0.4em;
    border-radius: 4px;
    font-size: 0.9em;
}}
pre {{
    background: {code_bg};
    padding: 14px 16px;
    border-radius: 6px;
    overflow-x: auto;
    margin: 0.8em 0;
    border: 1px solid {border};
}}
pre code {{
    background: none;
    padding: 0;
    font-size: 0.88em;
    line-height: 1.5;
}}
blockquote {{
    border-left: 3px solid {border};
    padding-left: 16px;
    color: {blockquote_fg};
    margin: 0.8em 0;
}}
table {{
    border-collapse: collapse;
    width: 100%;
    margin: 0.8em 0;
}}
th, td {{
    border: 1px solid {border};
    padding: 8px 12px;
    text-align: left;
}}
th {{
    background: {code_bg};
    font-weight: 600;
}}
ul, ol {{ padding-left: 2em; margin: 0.6em 0; }}
li {{ margin: 0.2em 0; }}
li input[type="checkbox"] {{ margin-right: 0.5em; }}
del {{ opacity: 0.6; }}
hr {{
    border: none;
    border-top: 1px solid {border};
    margin: 1.5em 0;
}}
img {{ max-width: 100%; height: auto; border-radius: 4px; }}

/* highlight.js theme overrides */
.hljs {{ background: {code_bg} !important; color: {fg}; }}
.hljs-keyword, .hljs-selector-tag, .hljs-built_in {{ color: {hljs_keyword}; }}
.hljs-string, .hljs-attr {{ color: {hljs_string}; }}
.hljs-number, .hljs-literal {{ color: {hljs_number}; }}
.hljs-comment, .hljs-doctag {{ color: {hljs_comment}; font-style: italic; }}
.hljs-function, .hljs-title {{ color: {hljs_function}; }}
.hljs-type, .hljs-class, .hljs-title.class_ {{ color: {hljs_type}; }}
.hljs-variable {{ color: {fg}; }}
.hljs-meta {{ color: {hljs_keyword}; }}
.hljs-params {{ color: {fg}; }}
</style>
</head>
<body>
{body}
<script src="{hljs_path}"></script>
<script>hljs.highlightAll();</script>
</body>
</html>"#,
        bg = bg,
        fg = fg,
        heading = heading,
        link = link,
        code_bg = code_bg,
        border = border,
        blockquote_fg = blockquote_fg,
        font_family = font_family,
        code_font_family = code_font_family,
        hljs_keyword = hljs_keyword,
        hljs_string = hljs_string,
        hljs_number = hljs_number,
        hljs_comment = hljs_comment,
        hljs_function = hljs_function,
        hljs_type = hljs_type,
        body = clean_body,
        hljs_path = hljs_path,
    ))
}
