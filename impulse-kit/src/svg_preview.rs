//! The SVG preview page (the Swift `SVGPreview`, from
//! `impulse-editor/src/svg.rs`): the SVG inlined into a page on the theme's
//! background, centered. The page's Content-Security-Policy runs no
//! scripts and loads nothing from the network; as a second line, scripts,
//! `foreignObject`, event-handler attributes and `javascript:` links are
//! stripped from the source first.

use std::sync::OnceLock;

use regex::Regex;

use crate::css_color::sanitize_css_color;

/// Maximum SVG source size (in bytes) before preview is refused.
pub const MAX_SVG_SIZE: usize = 1024 * 1024;

/// Whether a path names an SVG file, by its extension (case-insensitive).
pub fn is_svg_file(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

/// The source without scripts, `foreignObject`, `on*` attributes and
/// `javascript:` links.
pub fn sanitize_svg(source: &str) -> String {
    static PATTERNS: OnceLock<[Regex; 4]> = OnceLock::new();
    let [script, foreign_object, event_handler, javascript_uri] = PATTERNS.get_or_init(|| {
        [
            Regex::new(r"(?is)<script[\s>].*?</script\s*>|<script\s*/>").unwrap(),
            Regex::new(r"(?is)<foreignObject[\s>].*?</foreignObject\s*>|<foreignObject\s*/>")
                .unwrap(),
            Regex::new(r#"(?i)\s+on\w+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]*)"#).unwrap(),
            Regex::new(r#"(?i)((?:xlink:)?href\s*=\s*(?:"|'))javascript:"#).unwrap(),
        ]
    });
    let s = script.replace_all(source, "");
    let s = foreign_object.replace_all(&s, "");
    let s = event_handler.replace_all(&s, "");
    let s = javascript_uri.replace_all(&s, "${1}#blocked:");
    s.into_owned()
}

/// The preview page for an SVG source, or `None` past `MAX_SVG_SIZE`.
pub fn render(source: &str, bg_color: &str) -> Option<String> {
    if source.len() > MAX_SVG_SIZE {
        log::warn!("SVG source exceeds {MAX_SVG_SIZE} byte limit, skipping preview");
        return None;
    }
    let bg = sanitize_css_color(bg_color, "#1a1b26");
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
{sanitized}
</div>
</body>
</html>"#
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
        assert!(is_svg_file(".svg"));
    }

    #[test]
    fn render_basic() {
        let html = render("<svg></svg>", "#000000").unwrap();
        assert!(html.contains("<svg></svg>"));
        assert!(html.contains("background: #000000"));
        assert!(html.contains("connect-src 'none'"));
        assert!(html.starts_with("<!DOCTYPE html>\n<html>"));
        assert!(html.ends_with("</html>"));
    }

    #[test]
    fn render_oversized() {
        assert!(render(&"x".repeat(MAX_SVG_SIZE + 1), "#000").is_none());
    }

    #[test]
    fn render_sanitizes_bg_color() {
        let html = render("<svg/>", "evil;injection").unwrap();
        assert!(html.contains("background: #1a1b26"));
    }

    #[test]
    fn sanitize_strips_script_tags() {
        let result = sanitize_svg(r#"<svg><script>alert(1)</script><circle r="5"/></svg>"#);
        assert!(!result.contains("<script"));
        assert!(result.contains("<circle"));
    }

    #[test]
    fn sanitize_strips_foreign_object() {
        let result = sanitize_svg("<svg><foreignObject><div>evil</div></foreignObject></svg>");
        assert!(!result.contains("foreignObject"));
        assert!(!result.contains("evil"));
    }

    #[test]
    fn sanitize_strips_event_handlers() {
        let result =
            sanitize_svg(r#"<svg onload="alert(1)"><circle onclick="evil()" r="5"/></svg>"#);
        assert!(!result.contains("onload"));
        assert!(!result.contains("onclick"));
        assert!(result.contains(r#"r="5""#));
    }

    #[test]
    fn sanitize_blocks_javascript_uris() {
        let result = sanitize_svg(r#"<svg><a href="javascript:alert(1)">click</a></svg>"#);
        assert!(!result.contains("javascript:"));
        assert!(result.contains(r##"href="#blocked:alert(1)""##));
    }

    #[test]
    fn sanitize_preserves_safe_content() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="blue"/></svg>"#;
        assert_eq!(sanitize_svg(input), input);
    }
}
