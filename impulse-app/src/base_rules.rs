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
//! where tree-sitter's queries have them as builtins, and give every
//! capitalized name the type color (`API`, `Number`, `Math`), where they
//! are variables or calls. The editor uses its
//! own registrations of those grammars (`editor_grammar`) with those
//! captures as keywords; Review keeps the stock ones, which color them as
//! highlight.js does.
//!
//! Monaco's shell tokenizer gives a fixed list of commands (`git`, `npm`,
//! `node`, `ls`…) the type color and a few more words (`exit`, `set`) the
//! keyword color, wherever they stand; tree-sitter's bash query calls every
//! command a function, which the editor leaves plain. The editor's shell
//! (`monaco-bash`) adds Monaco's lists ahead of the stock patterns, and its
//! Markdown (`monaco-markdown`) highlights `sh` and `shell` fences, the
//! names Monaco knows its shell by, with it. Review and the Markdown preview
//! keep the stock grammars, as highlight.js colors in the Swift app.

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
/// TypeScript, JavaScript, shell scripts and Markdown, otherwise the grammar
/// itself.
pub(crate) fn editor_grammar(grammar: &str) -> &str {
    match grammar {
        "typescript" => "monaco-typescript",
        "tsx" => "monaco-tsx",
        "javascript" => "monaco-javascript",
        "bash" => "monaco-bash",
        "markdown" => "monaco-markdown",
        other => other,
    }
}

/// Monaco's shell tokenizer's `builtins` (`basic-languages/shell/shell.ts`),
/// which it colors `type.identifier`, ignoring case.
const MONACO_SHELL_BUILTINS: &str = "ab|awk|bash|beep|cat|cc|cd|chown|chmod|chroot|clear|cp|curl|cut|diff|echo|find|gawk|gcc|get|git|grep|hg|kill|killall|ln|ls|make|mkdir|openssl|mv|nc|node|npm|ping|ps|restart|rm|rmdir|sed|service|sh|shopt|shred|source|sort|sleep|ssh|start|stop|su|sudo|svn|tee|telnet|top|touch|vi|vim|wall|wc|wget|who|write|yes|zsh";

/// Its `keywords`: the shell's own (which the stock query already has as
/// keywords where they're syntax) and a few commands (`exit`, `set`).
const MONACO_SHELL_KEYWORDS: &str =
    "if|then|do|else|elif|while|until|for|in|esac|fi|fin|fil|done|exit|set|unset|export|function";

/// Monaco colors those words wherever they stand (a command, its
/// arguments, a value). A command's name is matched on `command_name`, the
/// node the stock `(command_name) @function` names: the highlighter keeps
/// the first match of a range, and a node's matches come in pattern order
/// (its `word` child's come after them).
fn monaco_shell_highlights() -> String {
    let mut query = String::new();
    for node in ["command_name", "word"] {
        for (capture, words) in [
            ("keyword", MONACO_SHELL_KEYWORDS),
            ("type", MONACO_SHELL_BUILTINS),
        ] {
            query.push_str(&format!(
                "(({node}) @{capture} (#match? @{capture} \"^(?i:{words})$\"))\n"
            ));
        }
    }
    query
}

/// The fence names Monaco highlights with its shell tokenizer: the
/// language's id and aliases (`shell`, `sh`), ignoring case.
const MONACO_SHELL_FENCES: &str = "^(?i:sh|shell)$";

/// The stock Markdown fence injection, and the editor's: `sh` and `shell`
/// fences go to `monaco-bash`, the rest as before.
const MARKDOWN_FENCE: &str = "(fenced_code_block
  (info_string
    (language) @injection.language)
  (code_fence_content) @injection.content)";

fn monaco_markdown_injections(stock: &str) -> Option<String> {
    if !stock.contains(MARKDOWN_FENCE) {
        return None;
    }
    let fences = format!(
        "(fenced_code_block
  (info_string
    (language) @_language)
  (code_fence_content) @injection.content
  (#match? @_language \"{MONACO_SHELL_FENCES}\")
  (#set! injection.language \"monaco-bash\"))

(fenced_code_block
  (info_string
    (language) @injection.language)
  (code_fence_content) @injection.content
  (#not-match? @injection.language \"{MONACO_SHELL_FENCES}\"))"
    );
    Some(stock.replace(MARKDOWN_FENCE, &fences))
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

/// Capitalized names as types, Monaco's `type.identifier`; first, so they
/// win over the stock query's variable, property and call captures.
const MONACO_TYPES: &str = r#"((identifier) @type (#match? @type "^[A-Z]"))
((property_identifier) @type (#match? @type "^[A-Z]"))
(call_expression function: ((identifier) @type (#match? @type "^[A-Z]")))
(call_expression
  function: (member_expression
    property: ((property_identifier) @type (#match? @type "^[A-Z]"))))
"#;

/// Register `name` as `base`'s grammar (on `language`'s parser, when given)
/// with Monaco's keywords, if the result compiles.
fn register_monaco(name: &str, base: &str, language: Option<&str>) {
    register_variant(name, base, language, |stock| {
        let mut highlights = format!("{MONACO_TYPES}\n{stock}");
        for (from, to) in MONACO_KEYWORDS {
            highlights = highlights.replace(from, to);
        }
        highlights
    });
}

/// Register `name` as `base`'s grammar (on `language`'s parser, when given)
/// with its highlight query rewritten by `highlights`, if the result
/// compiles.
fn register_variant(
    name: &str,
    base: &str,
    language: Option<&str>,
    highlights: impl FnOnce(&str) -> String,
) {
    register_variant_with(name, base, language, highlights, |stock| {
        Some(stock.to_string())
    });
}

/// `register_variant` with the injection query rewritten too (`None`
/// keeps the stock one).
fn register_variant_with(
    name: &str,
    base: &str,
    language: Option<&str>,
    highlights: impl FnOnce(&str) -> String,
    injections: impl FnOnce(&str) -> Option<String>,
) {
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
    let highlights = highlights(&stock.highlights);
    let compiles =
        |query: &str| query.is_empty() || tree_sitter::Query::new(&grammar, query).is_ok();
    if !compiles(&highlights) {
        log::warn!("{name}: the highlight query doesn't compile; keeping {base}'s");
        return;
    }
    let injections = match injections(&stock.injections) {
        Some(rewritten) if compiles(&rewritten) => rewritten.into(),
        _ if compiles(&stock.injections) => stock.injections.clone(),
        _ => Default::default(),
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

/// Register JSON, HTML and the editor's TypeScript, JavaScript, shell and
/// Markdown, once at launch.
pub(crate) fn register() {
    register_monaco("monaco-typescript", "typescript", None);
    register_monaco("monaco-tsx", "typescript", Some("tsx"));
    register_monaco("monaco-javascript", "javascript", None);
    register_variant("monaco-bash", "bash", None, |stock| {
        format!("{}\n{stock}", monaco_shell_highlights())
    });
    register_variant_with(
        "monaco-markdown",
        "markdown",
        None,
        str::to_string,
        monaco_markdown_injections,
    );
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

#[cfg(test)]
mod tests {
    use gpui_kit::component::highlighter::SyntaxHighlighter;
    use gpui_kit::component::input::Rope;

    /// The first capture's name for each of `words` in `code` (the one the
    /// editor colors it with), or "" when nothing captures it.
    fn names(grammar: &str, code: &str, words: &[&str]) -> Vec<String> {
        super::register();
        let mut highlighter = SyntaxHighlighter::new(grammar);
        highlighter.update(None, &Rope::from(code), None);
        let captures = highlighter.captures(0..code.len());
        words
            .iter()
            .map(|word| {
                let start = code.find(word).unwrap();
                let range = start..start + word.len();
                captures
                    .iter()
                    .find(|(r, _)| *r == range)
                    .map(|(_, name)| name.to_string())
                    .unwrap_or_default()
            })
            .collect()
    }

    #[test]
    fn the_editors_shell_colors_monacos_builtins() {
        let code = "sudo npm install && cargo build\nGIT=git\nexit 1\n";
        assert_eq!(
            names(
                "monaco-bash",
                code,
                &["sudo", "npm", "cargo", "=git", "exit"]
            ),
            ["type", "type", "function", "", "keyword"]
        );
        // `cargo` keeps the stock capture, which the editor leaves plain.
        // `=git` isn't a word; check the value on its own.
        let start = code.find("=git").unwrap() + 1;
        assert_eq!(names("monaco-bash", &code[start..], &["git"]), ["type"]);
    }

    #[test]
    fn sh_fences_in_the_editors_markdown_use_monacos_shell() {
        let code =
            "# Setup\n\n```sh\nnpm install\ncargo build\n```\n\n```js\nconst npm = 1;\n```\n";
        assert_eq!(
            names("monaco-markdown", code, &["npm", "cargo", "const"]),
            ["type", "function", "keyword"]
        );
    }
}
