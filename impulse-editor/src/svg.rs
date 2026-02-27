use crate::css::sanitize_css_color;
use regex::Regex;
use std::sync::OnceLock;

/// Maximum SVG source size (in bytes) before preview is refused.
/// Beyond this threshold, rendering can cause UI lag.
const MAX_SVG_SIZE: usize = 1024 * 1024; // 1 MB

/// Check whether a file path is an SVG file based on its extension.
pub fn is_svg_file(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

/// Sanitize SVG source by removing potentially dangerous elements and
/// attributes. The CSP is the primary defense; this provides defense-in-depth.
fn sanitize_svg(source: &str) -> String {
    static RE_SCRIPT: OnceLock<Regex> = OnceLock::new();
    static RE_FOREIGN_OBJECT: OnceLock<Regex> = OnceLock::new();
    static RE_EVENT_HANDLER: OnceLock<Regex> = OnceLock::new();
    static RE_JAVASCRIPT_URI: OnceLock<Regex> = OnceLock::new();

    // Strip <script>...</script> blocks (including self-closing <script/>)
    let re_script = RE_SCRIPT
        .get_or_init(|| Regex::new(r"(?is)<script[\s>].*?</script\s*>|<script\s*/>").unwrap());
    // Strip <foreignObject>...</foreignObject> blocks
    let re_foreign = RE_FOREIGN_OBJECT.get_or_init(|| {
        Regex::new(r"(?is)<foreignObject[\s>].*?</foreignObject\s*>|<foreignObject\s*/>").unwrap()
    });
    // Strip event handler attributes (on*)
    let re_event = RE_EVENT_HANDLER
        .get_or_init(|| Regex::new(r#"(?i)\s+on\w+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]*)"#).unwrap());
    // Strip javascript: URIs in href/xlink:href attributes
    let re_js_uri = RE_JAVASCRIPT_URI
        .get_or_init(|| Regex::new(r#"(?i)((?:xlink:)?href\s*=\s*(?:"|'))javascript:"#).unwrap());

    let s = re_script.replace_all(source, "");
    let s = re_foreign.replace_all(&s, "");
    let s = re_event.replace_all(&s, "");
    let s = re_js_uri.replace_all(&s, "${1}#blocked:");
    s.into_owned()
}

/// Render an SVG source string to a full standalone HTML document
/// with a themed background and centered layout.
///
/// Returns `None` if the source exceeds the size limit.
pub fn render_svg_preview(source: &str, bg_color: &str) -> Option<String> {
    if source.len() > MAX_SVG_SIZE {
        log::warn!(
            "SVG source ({} bytes) exceeds {} byte limit, skipping preview",
            source.len(),
            MAX_SVG_SIZE
        );
        return None;
    }

    // Sanitise the background color (accept #hex, rgb/rgba, or fallback)
    let bg = sanitize_css_color(bg_color, "#1a1b26");

    // Sanitize SVG to strip dangerous elements (script, foreignObject, event
    // handlers) as defense-in-depth alongside the CSP.
    let sanitized = sanitize_svg(source);

    Some(format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src file: data:; connect-src 'none';">
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{
    background: {bg};
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    padding: 24px;
    overflow: auto;
}}
.svg-container {{
    max-width: 100%;
    max-height: 100%;
}}
.svg-container svg {{
    max-width: 100%;
    height: auto;
    display: block;
}}
</style>
</head>
<body>
<div class="svg-container">
{svg}
</div>
</body>
</html>"#,
        bg = bg,
        svg = sanitized,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_svg_file_basic() {
        assert!(is_svg_file("image.svg"));
        assert!(is_svg_file("IMAGE.SVG"));
        assert!(is_svg_file("path/to/file.Svg"));
        assert!(!is_svg_file("file.png"));
        assert!(!is_svg_file("Makefile"));
        assert!(!is_svg_file(""));
        // Hidden file named .svg — treated as SVG (extension after the dot)
        assert!(is_svg_file(".svg"));
    }

    #[test]
    fn render_svg_preview_basic() {
        let html = render_svg_preview("<svg></svg>", "#000000").unwrap();
        assert!(html.contains("<svg></svg>"));
        assert!(html.contains("background: #000000"));
        assert!(html.contains("connect-src 'none'"));
    }

    #[test]
    fn render_svg_preview_oversized() {
        let big = "x".repeat(MAX_SVG_SIZE + 1);
        assert!(render_svg_preview(&big, "#000").is_none());
    }

    #[test]
    fn render_svg_preview_sanitizes_bg_color() {
        let html = render_svg_preview("<svg/>", "evil;injection").unwrap();
        assert!(html.contains("background: #1a1b26"));
    }

    #[test]
    fn sanitize_svg_strips_script_tags() {
        let input = r#"<svg><script>alert(1)</script><circle r="5"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("<script"));
        assert!(result.contains("<circle"));
    }

    #[test]
    fn sanitize_svg_strips_foreign_object() {
        let input = r#"<svg><foreignObject><div>evil</div></foreignObject></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("foreignObject"));
        assert!(!result.contains("evil"));
    }

    #[test]
    fn sanitize_svg_strips_event_handlers() {
        let input = r#"<svg onload="alert(1)"><circle onclick="evil()" r="5"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("onload"));
        assert!(!result.contains("onclick"));
        assert!(result.contains("r=\"5\""));
    }

    #[test]
    fn sanitize_svg_blocks_javascript_uris() {
        let input = r#"<svg><a href="javascript:alert(1)">click</a></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("javascript:"));
    }

    #[test]
    fn sanitize_svg_preserves_safe_content() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="blue"/></svg>"#;
        let result = sanitize_svg(input);
        assert_eq!(input, result);
    }
}
