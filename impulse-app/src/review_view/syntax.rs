//! Syntax colors in Review (the Swift `SyntaxHighlighter`, which runs
//! highlight.js): each file's old and new sides are tokenized separately,
//! their hunks' lines joined in order, by GPUI Kit's tree-sitter
//! highlighter in the file's language. Its captures are turned into the
//! token kinds Swift draws from highlight.js's classes, colored by the
//! theme's syntax colors (`kit_theme`), so a diff reads as it does in the
//! Swift app: operators and punctuation plain, a property read after `.`
//! and a key before `:` in the attribute color, class names in the type
//! color wherever they appear. Context lines take the new side's colors.
//! Files past `MAX_LINES` of diff stay plain.

use std::ops::Range;

use gpui_kit::component::highlighter::{HighlightTheme, SyntaxHighlighter};
use gpui_kit::component::input::Rope;
use gpui_kit::Hsla;
use impulse_git::hunks::DiffLineKind;
use impulse_git::scoped_diff::FileDiff;

/// More diff lines than this aren't highlighted.
const MAX_LINES: usize = 5_000;

/// A line's colored byte ranges of its content.
pub(super) type LineColors = Vec<(Range<usize>, Hsla)>;

/// Colors by hunk, then line.
#[derive(Default)]
pub(super) struct FileSyntax {
    pub hunks: Vec<Vec<LineColors>>,
}

/// The token kinds Swift's `SyntaxHighlighter` gives highlight.js's
/// classes (`token(forTag:)`), those a capture can become.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token {
    Keyword,
    String,
    Number,
    Comment,
    Function,
    Type,
    Constant,
    Attribute,
    Tag,
    Regexp,
    Link,
}

const TOKENS: [Token; 11] = [
    Token::Keyword,
    Token::String,
    Token::Number,
    Token::Comment,
    Token::Function,
    Token::Type,
    Token::Constant,
    Token::Attribute,
    Token::Tag,
    Token::Regexp,
    Token::Link,
];

impl Token {
    /// The highlight theme's scope holding this token's color (Review's
    /// theme in `kit_theme` maps each to the theme's syntax color Swift's
    /// `syntaxColor` uses).
    fn scope(self) -> &'static str {
        match self {
            Token::Keyword => "keyword",
            Token::String => "string",
            Token::Number => "number",
            Token::Comment => "comment",
            Token::Function => "function",
            Token::Type => "type",
            Token::Constant => "constant",
            Token::Attribute => "attribute",
            Token::Tag => "tag",
            Token::Regexp => "string.regex",
            Token::Link => "link_uri",
        }
    }
}

/// What a capture does to the bytes it covers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Paint {
    /// Leaves them as they are: a capture highlight.js has no class for.
    Keep,
    /// The text color (highlight.js leaves them unclassed, or its class is
    /// drawn in the text color, as `hljs-subst` and `hljs-variable` are).
    Plain,
    Token(Token),
}

/// No token on this byte.
const PLAIN: u8 = u8::MAX;

/// The colors of `diff`'s lines, in the language git's diff names.
pub(super) fn highlight(diff: &FileDiff, theme: &HighlightTheme) -> FileSyntax {
    let mut out = FileSyntax {
        hunks: diff
            .hunks
            .iter()
            .map(|h| vec![Vec::new(); h.lines.len()])
            .collect(),
    };
    let total: usize = diff.hunks.iter().map(|h| h.lines.len()).sum();
    if total == 0 || total > MAX_LINES || diff.is_binary {
        return out;
    }
    let grammar = crate::editor_tab::grammar_for(&diff.language, std::path::Path::new(&diff.path));
    if grammar.is_empty() || grammar == "plaintext" || grammar == "text" {
        return out;
    }
    let colors: Vec<Option<Hsla>> = TOKENS
        .iter()
        .map(|t| theme.style.syntax.style(t.scope()).and_then(|s| s.color))
        .collect();
    for old_side in [true, false] {
        // The side's lines in order, and where each starts.
        let mut text = String::new();
        let mut placed: Vec<(usize, usize, usize)> = Vec::new();
        for (h, hunk) in diff.hunks.iter().enumerate() {
            for (l, line) in hunk.lines.iter().enumerate() {
                let on_side = match line.kind {
                    DiffLineKind::Context => true,
                    DiffLineKind::Removed => old_side,
                    DiffLineKind::Added => !old_side,
                };
                if !on_side {
                    continue;
                }
                placed.push((h, l, text.len()));
                text.push_str(&line.content);
                text.push('\n');
            }
        }
        if text.is_empty() {
            continue;
        }
        let tokens = tokenize(grammar, &text);
        for &(h, l, start) in &placed {
            let line = &diff.hunks[h].lines[l];
            // Context lines are colored once, from the new side.
            if old_side && line.kind == DiffLineKind::Context {
                continue;
            }
            let end = start + line.content.len();
            let colors_out = &mut out.hunks[h][l];
            let mut run = start;
            while run < end {
                let token = tokens[run];
                let mut next = run + 1;
                while next < end && tokens[next] == token {
                    next += 1;
                }
                if let Some(color) = colors.get(token as usize).copied().flatten() {
                    colors_out.push((run - start..next - start, color));
                }
                run = next;
            }
        }
    }
    out
}

/// Each byte of `text`'s token (an index into `TOKENS`, or `PLAIN`).
fn tokenize(grammar: &str, text: &str) -> Vec<u8> {
    let mut highlighter = SyntaxHighlighter::new(grammar);
    highlighter.update(None, &Rope::from(text), None);
    let mut captures = highlighter.captures(0..text.len());
    // Outer captures before the ones inside them, so the innermost wins (a
    // template string's substitution comes after its contents); captures
    // of the same range keep their order, the first one's name.
    captures.sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
    let script = matches!(grammar, "javascript" | "typescript" | "tsx");
    // Template substitutions (`${…}`), where highlight.js colors little.
    let substitutions: Vec<Range<usize>> = if script {
        captures
            .iter()
            .filter(|(_, name)| name.as_ref() == "embedded")
            .map(|(range, _)| range.clone())
            .collect()
    } else {
        Vec::new()
    };
    let mut tokens = vec![PLAIN; text.len()];
    for (range, name) in &captures {
        let range = range.start.min(text.len())..range.end.min(text.len());
        if range.is_empty()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            continue;
        }
        let mut paint = scoped(name, grammar);
        if script {
            let inside = substitutions
                .iter()
                .any(|s| s.start < range.start && range.end <= s.end);
            paint = script_paint(name, paint, text, range.clone(), inside);
        }
        // Nothing inside a comment takes another color (JSDoc's tags are
        // `doctag`, drawn as the comment).
        if tokens[range.start] == Token::Comment as u8
            && tokens[range.end - 1] == Token::Comment as u8
        {
            continue;
        }
        let byte = match paint {
            Paint::Keep => continue,
            Paint::Plain => PLAIN,
            Paint::Token(token) => token as u8,
        };
        tokens[range].fill(byte);
    }
    tokens
}

/// The paint for a capture by its name alone: the highlight.js class the
/// same text gets in most grammars.
fn scoped(name: &str, grammar: &str) -> Paint {
    use crate::base_rules::{HTML_DELIMITER, JSON_KEY, JSON_KEYWORD, JSON_VALUE};
    use Token::*;
    // The app's own JSON and HTML queries (`base_rules`) put Monaco's
    // tokens on names no grammar uses: highlight.js's `attr`, `string`,
    // `literal`, and the `tag` around a tag's name.
    match name {
        n if n == JSON_KEY => return Paint::Token(Attribute),
        n if n == JSON_VALUE => return Paint::Token(String),
        n if n == JSON_KEYWORD => return Paint::Token(Constant),
        n if n == HTML_DELIMITER => return Paint::Token(Tag),
        _ => {}
    }
    let token = match name {
        "string.regex" | "string.regexp" => Regexp,
        "string.special.symbol" => Constant,
        "constant.builtin" | "boolean" => Constant,
        // highlight.js's `built_in`.
        "function.builtin" => Type,
        // `meta`.
        "tag.doctype" => Keyword,
        // Markdown: `bullet`, `section`, `link`, `string` (a link's text);
        // code is `code`, which Swift leaves plain.
        "punctuation.list_marker" => Keyword,
        "text.title" => Function,
        "text.uri" | "link_uri" => Link,
        "text.reference" | "link_text" => String,
        "text.literal" | "text.code.span" => return Paint::Plain,
        _ => match name.split('.').next().unwrap_or(name) {
            "comment" => Comment,
            "keyword" | "preproc" => Keyword,
            "string" | "character" => String,
            "number" | "float" => Number,
            "function" | "method" => Function,
            "constructor" | "type" | "enum" => Type,
            "attribute" => Attribute,
            "tag" => Tag,
            "title" => Function,
            // Keys are `attr` in highlight.js's data and style grammars;
            // elsewhere a field is unclassed.
            "property"
                if matches!(
                    grammar,
                    "css" | "scss" | "less" | "json" | "jsonc" | "yaml" | "toml"
                ) =>
            {
                Attribute
            }
            // Constants are `variable.constant` (the text color),
            // operators and punctuation unclassed.
            "property" | "variable" | "constant" | "operator" | "punctuation" | "embedded"
            | "label" | "variant" | "emphasis" => return Paint::Plain,
            _ => return Paint::Keep,
        },
    };
    Paint::Token(token)
}

/// highlight.js's JavaScript and TypeScript grammars class a name by how it
/// looks and what's around it rather than by what it is: `.name` is a
/// `property`, `name:` an `attr`, a capitalized name a `title.class`
/// anywhere, `name(` a `title.function`, and inside `${…}` only keywords,
/// literals and built-ins are colored.
fn script_paint(name: &str, paint: Paint, text: &str, range: Range<usize>, inside: bool) -> Paint {
    use Token::*;
    let word = &text[range.clone()];
    if name == "string.special" && word.starts_with('/') {
        return Paint::Token(Regexp);
    }
    // GPUI Kit names a capture of the same range as an earlier one by the
    // first, so a parameter's `@variable.parameter` arrives as the locals
    // query's `@local.definition`.
    let named = matches!(
        name.split('.').next().unwrap_or(name),
        "variable"
            | "type"
            | "constant"
            | "property"
            | "function"
            | "method"
            | "constructor"
            | "local"
    );
    if !named || !is_identifier(word) {
        return paint;
    }
    if matches!(word, "NaN" | "Infinity" | "undefined") {
        return Paint::Token(Constant);
    }
    if is_builtin_global(word) || is_class_reference(word) || is_builtin_type(word) {
        return Paint::Token(Type);
    }
    // `this`, `console`…: `variable.language`.
    if name.starts_with("variable.builtin") {
        return Paint::Plain;
    }
    if inside {
        return Paint::Plain;
    }
    let before = &text[..range.start];
    let after = text[range.end..].trim_start();
    let call = after.starts_with('(');
    if before.ends_with('.') {
        return Paint::Token(if call { Function } else { Attribute });
    }
    if after.starts_with(':') && !after.starts_with("::") {
        return Paint::Token(Attribute);
    }
    if call || name.starts_with("function") || name.starts_with("method") {
        return Paint::Token(Function);
    }
    Paint::Plain
}

fn is_identifier(word: &str) -> bool {
    let mut chars = word.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// highlight.js's `CLASS_REFERENCE`: `JSON…`, `Float32Array`, `OutT`,
/// `CSSFactory`, `FPs` (a capital, then a lowercase letter after at most a
/// run of capitals, or a digit after two or more), but not `V`, `URL` or
/// `TTL_MS`.
fn is_class_reference(word: &str) -> bool {
    if word.starts_with("JSON") {
        return true;
    }
    let bytes = word.as_bytes();
    let capitals = bytes.iter().take_while(|b| b.is_ascii_uppercase()).count();
    match bytes.get(capitals) {
        Some(b) if b.is_ascii_lowercase() => capitals >= 1,
        Some(b) if b.is_ascii_digit() => capitals >= 2,
        _ => false,
    }
}

/// highlight.js's `BUILT_IN_GLOBALS` (its `built_in` class).
fn is_builtin_global(word: &str) -> bool {
    matches!(
        word,
        "setInterval"
            | "setTimeout"
            | "clearInterval"
            | "clearTimeout"
            | "require"
            | "exports"
            | "eval"
            | "isFinite"
            | "isNaN"
            | "parseFloat"
            | "parseInt"
            | "decodeURI"
            | "decodeURIComponent"
            | "encodeURI"
            | "encodeURIComponent"
            | "escape"
            | "unescape"
    )
}

/// TypeScript's predefined types, `built_in` in highlight.js.
fn is_builtin_type(word: &str) -> bool {
    matches!(
        word,
        "any"
            | "void"
            | "number"
            | "boolean"
            | "string"
            | "object"
            | "never"
            | "symbol"
            | "bigint"
            | "unknown"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The token of each word in `code`, as `word:token` pairs (plain
    /// words left out).
    fn words(grammar: &str, code: &str) -> Vec<String> {
        let tokens = tokenize(grammar, code);
        let mut out = Vec::new();
        let mut i = 0;
        while i < code.len() {
            let token = tokens[i];
            let mut j = i + 1;
            while j < code.len() && tokens[j] == token {
                j += 1;
            }
            let word = code[i..j].trim();
            if token != PLAIN && !word.is_empty() {
                out.push(format!("{word}:{:?}", TOKENS[token as usize]));
            }
            i = j;
        }
        out
    }

    fn has(words: &[String], expected: &str) -> bool {
        words.iter().any(|w| w == expected)
    }

    /// The demo's `cache.ts`, colored as highlight.js colors it
    /// (`hljs.highlight(…, { language: "typescript" })`).
    #[test]
    fn typescript_matches_highlight_js() {
        let code = "export class TTLCache<V> {\n  private entries = new Map<string, { value: V; expires: number }>();\n  private hits = 0;\n  private ttlMs: number;\n\n  get(key: string): V | undefined {\n    const entry = this.entries.get(key);\n    if (!entry || entry.expires <= this.now()) {\n      this.misses++;\n      return undefined;\n    }\n    return entry.value;\n  }\n}\nconst API = process.env.FORECAST_API ?? \"x\";\nconst cache = new TTLCache<Forecast>(TTL_MS);\nconst s = `${API}?t=${encodeURIComponent(trail.id)}`;\n";
        let w = words("typescript", code);
        for expected in [
            "export:Keyword",
            "TTLCache:Type",
            "Map:Type",
            "string:Type",
            "value:Attribute",
            "expires:Attribute",
            "ttlMs:Attribute",
            "0:Number",
            "get:Function",
            "key:Attribute",
            "undefined:Constant",
            "entries:Attribute",
            "now:Function",
            "misses:Attribute",
            "env:Attribute",
            "FORECAST_API:Attribute",
            "Forecast:Type",
            "encodeURIComponent:Type",
        ] {
            assert!(has(&w, expected), "{expected} missing from {w:?}");
        }
        // Operators, `this`, generic parameters, constants, fields and
        // what's inside `${…}` stay plain.
        for word in [
            "=", "!", "||", "++", "??", "<=", "V", "this", "TTL_MS", "API", "hits", "id",
        ] {
            assert!(
                !w.iter().any(|x| x.split(':').next() == Some(word)),
                "{word} colored in {w:?}"
            );
        }
    }

    /// Conflict markers are left out of the parse (GPUI Kit's
    /// `conflict_free_ranges`), so both sides' keywords keep their colors.
    #[test]
    fn conflict_markers_leave_code_colored() {
        let code = "function f(c: number): number {\n<<<<<<< HEAD\n  return c;\n=======\n  return c + 1;\n>>>>>>> branch\n}\n";
        let w = words("typescript", code);
        assert_eq!(
            w.iter().filter(|x| *x == "return:Keyword").count(),
            2,
            "{w:?}"
        );
        assert!(has(&w, "1:Number"), "{w:?}");
    }

    /// JSON through the app's own query (`base_rules`), as highlight.js
    /// colors it: keys `attr`, values `string`, `true` a `literal`.
    #[test]
    fn json_keys_and_values() {
        crate::base_rules::register();
        let w = words("json", "{\"a\": \"b\", \"c\": true}\n");
        for expected in ["\"a\":Attribute", "\"b\":String", "true:Constant"] {
            assert!(has(&w, expected), "{expected} missing from {w:?}");
        }
    }
}
