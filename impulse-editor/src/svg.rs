use crate::css::sanitize_css_color;

/// Maximum SVG source size (in bytes) before preview is refused.
/// Beyond this threshold, rendering can cause UI lag.
const MAX_SVG_SIZE: usize = 1024 * 1024; // 1 MB

/// Check whether a file path is an SVG file based on its extension.
pub fn is_svg_file(path: &str) -> bool {
    path.rsplit('.').next().is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
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

    // Embed the raw SVG directly — no sanitization, matching VS Code behavior
    // (user is editing their own local files).
    // The CSP blocks scripts, external style imports, and network requests.
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
        svg = source,
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
}
