/// Maximum SVG source size (in bytes) before preview is refused.
/// Beyond this threshold, rendering can cause UI lag.
const MAX_SVG_SIZE: usize = 1024 * 1024; // 1 MB

/// Check whether a file path is an SVG file based on its extension.
pub fn is_svg_file(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    ext == "svg"
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
    Some(format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src file: data:;">
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

/// Sanitise a CSS color value. Accepts `#hex`, `rgb(…)`, `rgba(…)`.
/// Anything else is replaced by the fallback.
fn sanitize_css_color(value: &str, fallback: &str) -> String {
    let v = value.trim();
    // Hex: #abc, #aabbcc, #aabbccdd
    if v.starts_with('#')
        && (v.len() == 4 || v.len() == 7 || v.len() == 9)
        && v[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return v.to_string();
    }
    // rgb(…) / rgba(…)
    if (v.starts_with("rgb(") || v.starts_with("rgba(")) && v.ends_with(')') {
        let inner = &v[v.find('(').unwrap() + 1..v.len() - 1];
        if inner
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == '.' || c == ' ' || c == '%')
        {
            return v.to_string();
        }
    }
    fallback.to_string()
}
