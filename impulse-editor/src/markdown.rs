use pulldown_cmark::{Options, Parser};
use serde::{Deserialize, Serialize};

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

/// Render a markdown source string to a full standalone HTML document
/// with themed CSS and highlight.js code highlighting.
///
/// `highlight_js_path` should be an absolute file path or URL to `highlight.min.js`.
pub fn render_markdown_preview(
    source: &str,
    theme: &MarkdownThemeColors,
    highlight_js_path: &str,
) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(source, options);
    let mut html_body = String::new();
    pulldown_cmark::html::push_html(&mut html_body, parser);

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
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
        bg = theme.bg,
        fg = theme.fg,
        heading = theme.heading,
        link = theme.link,
        code_bg = theme.code_bg,
        border = theme.border,
        blockquote_fg = theme.blockquote_fg,
        font_family = theme.font_family,
        code_font_family = theme.code_font_family,
        hljs_keyword = theme.hljs_keyword,
        hljs_string = theme.hljs_string,
        hljs_number = theme.hljs_number,
        hljs_comment = theme.hljs_comment,
        hljs_function = theme.hljs_function,
        hljs_type = theme.hljs_type,
        body = html_body,
        hljs_path = highlight_js_path,
    )
}
