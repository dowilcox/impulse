//! Monaco's own token colors that show through the Swift app's themes.
//! Its themes inherit `vs-dark` (or `vs`), and Monaco gives the most
//! specific rule a token matches, so the base theme's language rules win
//! over a theme's general ones: JSON keys, string values and `true`,
//! `false` and `null` keep VS Code's colors, and HTML's angle brackets its
//! gray. GPUI Kit colors a fixed set of capture names, so JSON and HTML are
//! registered again with these tokens on four names no grammar uses, which
//! Review (highlight.js colors in Swift, not Monaco's) gives the theme's
//! colors as before.
//!
//! Monaco's TypeScript and JavaScript tokenizers also call `this`, `super`,
//! `true`, `false`, `null`, `undefined` and the primitive types keywords,
//! where tree-sitter's queries have them as builtins. The editor uses its
//! own registrations of those grammars (`editor_grammar`) with those
//! captures as keywords; Review keeps the stock ones, which color them as
//! highlight.js does.

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

/// GPUI Kit's JSON query with keys, string values and keywords moved,
/// and the brackets apart from the other punctuation (`kit_theme`).
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
] @punctuation.delimiter

[
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket
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

/// The grammar the editor highlights `grammar` with: Monaco's take on
/// TypeScript and JavaScript, otherwise the grammar itself.
pub(crate) fn editor_grammar(grammar: &str) -> &str {
    match grammar {
        "typescript" => "monaco-typescript",
        "tsx" => "monaco-tsx",
        "javascript" => "monaco-javascript",
        other => other,
    }
}

/// The stock TypeScript/JavaScript captures Monaco's tokenizer calls
/// keywords.
const MONACO_KEYWORDS: [(&str, &str); 4] = [
    ("(this) @variable.builtin", "(this) @keyword"),
    ("(super) @variable.builtin", "(super) @keyword"),
    (
        "  (undefined)\n] @constant.builtin",
        "  (undefined)\n] @keyword",
    ),
    (
        "(predefined_type) @type.builtin",
        "(predefined_type) @keyword",
    ),
];

/// Register `name` as `base`'s grammar (on `language`'s parser, when given)
/// with Monaco's keywords, if the result compiles.
fn register_monaco(name: &str, base: &str, language: Option<&str>) {
    let registry = LanguageRegistry::singleton();
    let (Some(stock), Some(parser)) = (
        registry.language(base),
        registry.language(language.unwrap_or(base)),
    ) else {
        return;
    };
    let Some(grammar) = parser.language.clone() else {
        return;
    };
    let mut highlights = stock.highlights.to_string();
    for (from, to) in MONACO_KEYWORDS {
        highlights = highlights.replace(from, to);
    }
    let compiles =
        |query: &str| query.is_empty() || tree_sitter::Query::new(&grammar, query).is_ok();
    if !compiles(&highlights) {
        log::warn!("{name}: the highlight query doesn't compile; keeping {base}'s");
        return;
    }
    let injections = if compiles(&stock.injections) {
        stock.injections.clone()
    } else {
        Default::default()
    };
    let locals = if compiles(&parser.locals) {
        parser.locals.clone()
    } else {
        Default::default()
    };
    registry.register(
        name,
        &GrammarConfig {
            name: name.to_string().into(),
            language: Some(grammar),
            injection_languages: stock.injection_languages.clone(),
            highlights: highlights.into(),
            injections,
            locals,
        },
    );
}

/// Register JSON, HTML and the editor's TypeScript and JavaScript, once at
/// launch.
pub(crate) fn register() {
    register_monaco("monaco-typescript", "typescript", None);
    register_monaco("monaco-tsx", "typescript", Some("tsx"));
    register_monaco("monaco-javascript", "javascript", None);
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
