/// Sanitise a CSS color value. Accepts `#hex`, `rgb(…)`, `rgba(…)`.
/// Anything else is replaced by the fallback.
pub fn sanitize_css_color(value: &str, fallback: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_hex_colors() {
        assert_eq!(sanitize_css_color("#abc", "x"), "#abc");
        assert_eq!(sanitize_css_color("#aabbcc", "x"), "#aabbcc");
        assert_eq!(sanitize_css_color("#aabbccdd", "x"), "#aabbccdd");
        assert_eq!(sanitize_css_color("  #abc  ", "x"), "#abc");
    }

    #[test]
    fn invalid_hex_colors() {
        assert_eq!(sanitize_css_color("#ab", "x"), "x");
        assert_eq!(sanitize_css_color("#abcg", "x"), "x");
        assert_eq!(sanitize_css_color("#aabbccdde", "x"), "x");
        assert_eq!(sanitize_css_color("#", "x"), "x");
    }

    #[test]
    fn valid_rgb_rgba() {
        assert_eq!(sanitize_css_color("rgb(255, 0, 0)", "x"), "rgb(255, 0, 0)");
        assert_eq!(
            sanitize_css_color("rgba(0, 0, 0, 0.5)", "x"),
            "rgba(0, 0, 0, 0.5)"
        );
        assert_eq!(
            sanitize_css_color("rgb(100%, 50%, 0%)", "x"),
            "rgb(100%, 50%, 0%)"
        );
    }

    #[test]
    fn malicious_rgb_rejected() {
        assert_eq!(
            sanitize_css_color("rgb(0, 0, 0); background: url(evil)", "x"),
            "x"
        );
        assert_eq!(
            sanitize_css_color("rgb(0, 0, 0)</style><script>alert(1)</script>", "x"),
            "x"
        );
    }

    #[test]
    fn empty_and_named_colors_fall_back() {
        assert_eq!(sanitize_css_color("", "x"), "x");
        assert_eq!(sanitize_css_color("   ", "x"), "x");
        assert_eq!(sanitize_css_color("red", "x"), "x");
        assert_eq!(sanitize_css_color("transparent", "x"), "x");
    }
}
