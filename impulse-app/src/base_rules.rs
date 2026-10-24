//! Monaco's own token colors that show through the Swift app's themes.
//! Its themes inherit `vs-dark` (or `vs`), and Monaco gives the most
//! specific rule a token matches, so the base theme's language rules win
//! over a theme's general ones: JSON keys, string values and `true`,
//! `false` and `null` keep VS Code's colors, and HTML's angle brackets its
//! gray. GPUI Kit colors a fixed set of capture names, so JSON and HTML are
//! registered again with these tokens on four names no grammar uses, which
//! Review (highlight.js colors in Swift, not Monaco's) gives the theme's
//! colors as before.

use gpui_kit::component::highlighter::{GrammarConfig, LanguageRegistry};

/// JSON keys (`string.key.json`).
pub(crate) const JSON_KEY: &str = "hint";
/// JSON string values (`string.value.json`).
pub(crate) const JSON_VALUE: &str = "predictive";
/// JSON's `true`, `false` and `null` (`keyword.json`).
pub(crate) const JSON_KEYWORD: &str = "variant";
/// HTML's `<`, `>`, `</` and `/>` (`delimiter.html`).
pub(crate) const HTML_DELIMITER: &str = "primary";

/// The colors for those names, from Monaco's `vs-dark` or `vs` rules.
pub(crate) fn colors(light: bool) -> [(&'static str, &'static str); 4] {
    if light {
        [
            (JSON_KEY, "#A31515"),
            (JSON_VALUE, "#0451A5"),
            (JSON_KEYWORD, "#0451A5"),
            (HTML_DELIMITER, "#383838"),
        ]
    } else {
        [
            (JSON_KEY, "#9CDCFE"),
            (JSON_VALUE, "#CE9178"),
            (JSON_KEYWORD, "#CE9178"),
            (HTML_DELIMITER, "#808080"),
        ]
    }
}

/// GPUI Kit's JSON query with keys, string values and keywords moved.
const JSON_HIGHLIGHTS: &str = r#"(comment) @comment

(string) @predictive
(escape_sequence) @string.escape

(number) @number

(pair key: (string) @hint)

[
  (true)
  (false)
  (null)
] @variant

[
  ","
  ":"
  "{"
  "}"
  "["
  "]"
] @punctuation
"#;

/// GPUI Kit's HTML query with the angle brackets moved.
const HTML_HIGHLIGHTS: &str = r#"(tag_name) @tag
(erroneous_end_tag_name) @tag.error
(doctype) @constant
(attribute_name) @attribute
(attribute_value) @string
(comment) @comment

[
  "<"
  ">"
  "</"
  "/>"
] @primary
"#;

/// GPUI Kit's HTML injections: scripts and styles.
const HTML_INJECTIONS: &str = r#"((script_element
  (raw_text) @injection.content)
 (#set! injection.language "javascript"))

((style_element
  (raw_text) @injection.content)
 (#set! injection.language "css"))
"#;

/// Register JSON and HTML with the queries above, once at launch.
pub(crate) fn register() {
    let registry = LanguageRegistry::singleton();
    registry.register(
        "json",
        &GrammarConfig::new(
            "json",
            tree_sitter_json::LANGUAGE.into(),
            Vec::new(),
            JSON_HIGHLIGHTS,
            "",
            "",
        ),
    );
    registry.register(
        "html",
        &GrammarConfig::new(
            "html",
            tree_sitter_html::LANGUAGE.into(),
            vec!["javascript".into(), "css".into()],
            HTML_HIGHLIGHTS,
            HTML_INJECTIONS,
            "",
        ),
    );
}
