//! Impulse: snippets, as Monaco's snippet controller takes them from a
//! completion (`insertTextFormat: 2`): LSP's snippet syntax parsed
//! (`$1`, `${1:placeholder}`, `${1|one,two|}`, `$0`, nested placeholders,
//! `$VARIABLE`, `${VARIABLE:default}`, transforms and `\` escapes), the text
//! inserted with each line after the first indented as the line it goes
//! into, and its tab stops kept as a session: a stop's places are selected
//! together (the same index edits together), Tab and Shift+Tab move between
//! stops, and Escape, reaching `$0` or moving the cursor out of every stop
//! ends it.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{Context, Window};

use super::{EditorMode, InputBaseState, RopeExt as _};

/// A parsed piece of a snippet.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    Text(String),
    /// `$1`, `${1}`, `${1:children}`, `${1|choices|}`; a transform on a
    /// stop is parsed and left out (the stop mirrors its placeholder).
    TabStop {
        index: u32,
        children: Vec<Node>,
        choices: Option<Vec<String>>,
    },
    /// `$NAME`, `${NAME}`, `${NAME:default}`, `${NAME/regex/format/options}`.
    Variable {
        name: String,
        default: Option<Vec<Node>>,
        transform: Option<Transform>,
    },
}

/// `/regex/format/options`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Transform {
    regex: String,
    format: Vec<Format>,
    options: String,
}

/// A piece of a transform's format.
#[derive(Clone, Debug, PartialEq)]
enum Format {
    Text(String),
    /// `$1`, `${1}`, `${1:/upcase}` (and the other cases).
    Group(usize, Option<String>),
    /// `${1:+if}`, `${1:?if:else}`, `${1:-else}`, `${1:else}`.
    Conditional(usize, String, String),
}

struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    _source: &'a str,
}

/// Parse a snippet. What doesn't parse as syntax is text, as VS Code reads
/// it: a `$` that starts nothing, a `}` outside a placeholder, an unclosed
/// `${`.
pub(crate) fn parse(source: &str) -> Vec<Node> {
    let mut parser = Parser {
        chars: source.chars().collect(),
        pos: 0,
        _source: source,
    };
    parser.any(false)
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).copied()
    }

    /// Nodes up to the end, or (`nested`) up to an unescaped `}`, left for
    /// the caller.
    fn any(&mut self, nested: bool) -> Vec<Node> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        while let Some(c) = self.peek() {
            match c {
                '\\' => match self.peek_at(1) {
                    Some(next @ ('$' | '}' | '\\')) => {
                        text.push(next);
                        self.pos += 2;
                    }
                    _ => {
                        text.push('\\');
                        self.pos += 1;
                    }
                },
                '$' => {
                    let start = self.pos;
                    match self.dollar() {
                        Some(node) => {
                            if !text.is_empty() {
                                nodes.push(Node::Text(std::mem::take(&mut text)));
                            }
                            nodes.push(node);
                        }
                        None => {
                            self.pos = start + 1;
                            text.push('$');
                        }
                    }
                }
                '}' if nested => break,
                _ => {
                    text.push(c);
                    self.pos += 1;
                }
            }
        }
        if !text.is_empty() {
            nodes.push(Node::Text(text));
        }
        nodes
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start {
            return None;
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }

    fn name(&mut self) -> Option<String> {
        let first = self.peek()?;
        if !(first == '_' || first.is_ascii_alphabetic()) {
            return None;
        }
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c == '_' || c.is_ascii_alphanumeric())
        {
            self.pos += 1;
        }
        Some(self.chars[start..self.pos].iter().collect())
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// A `$` construct at the cursor; `None` (and the cursor anywhere) when
    /// there isn't one.
    fn dollar(&mut self) -> Option<Node> {
        self.pos += 1;
        if !self.eat('{') {
            if let Some(index) = self.number() {
                return Some(Node::TabStop {
                    index,
                    children: Vec::new(),
                    choices: None,
                });
            }
            let name = self.name()?;
            return Some(Node::Variable {
                name,
                default: None,
                transform: None,
            });
        }
        if let Some(index) = self.number() {
            if self.eat('}') {
                return Some(Node::TabStop {
                    index,
                    children: Vec::new(),
                    choices: None,
                });
            }
            if self.eat(':') {
                let children = self.any(true);
                return self.eat('}').then_some(Node::TabStop {
                    index,
                    children,
                    choices: None,
                });
            }
            if self.eat('|') {
                let choices = self.choices()?;
                return Some(Node::TabStop {
                    index,
                    children: Vec::new(),
                    choices: Some(choices),
                });
            }
            if self.eat('/') {
                self.transform()?;
                return self.eat('}').then_some(Node::TabStop {
                    index,
                    children: Vec::new(),
                    choices: None,
                });
            }
            return None;
        }
        let name = self.name()?;
        if self.eat('}') {
            return Some(Node::Variable {
                name,
                default: None,
                transform: None,
            });
        }
        if self.eat(':') {
            let default = self.any(true);
            return self.eat('}').then_some(Node::Variable {
                name,
                default: Some(default),
                transform: None,
            });
        }
        if self.eat('/') {
            let transform = self.transform()?;
            return self.eat('}').then_some(Node::Variable {
                name,
                default: None,
                transform: Some(transform),
            });
        }
        None
    }

    /// `one,two|}` after `${1|`: the options, with `\,` `\|` `\\` `\$`
    /// escaped.
    fn choices(&mut self) -> Option<Vec<String>> {
        let mut choices = Vec::new();
        let mut current = String::new();
        loop {
            match self.peek()? {
                '\\' => match self.peek_at(1) {
                    Some(next @ (',' | '|' | '\\' | '$' | '}')) => {
                        current.push(next);
                        self.pos += 2;
                    }
                    _ => {
                        current.push('\\');
                        self.pos += 1;
                    }
                },
                ',' => {
                    choices.push(std::mem::take(&mut current));
                    self.pos += 1;
                }
                '|' if self.peek_at(1) == Some('}') => {
                    choices.push(current);
                    self.pos += 2;
                    return Some(choices);
                }
                c => {
                    current.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// Text up to an unescaped `/` (consumed), `\/` read as `/`; other
    /// escapes kept for the regex.
    fn until_slash(&mut self) -> Option<String> {
        let mut out = String::new();
        loop {
            match self.peek()? {
                '\\' if self.peek_at(1) == Some('/') => {
                    out.push('/');
                    self.pos += 2;
                }
                '\\' => {
                    out.push('\\');
                    if let Some(next) = self.peek_at(1) {
                        out.push(next);
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                    }
                }
                '/' => {
                    self.pos += 1;
                    return Some(out);
                }
                c => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// `regex/format/options` after the first `/`, up to (not past) `}`.
    fn transform(&mut self) -> Option<Transform> {
        let regex = self.until_slash()?;
        let mut format = Vec::new();
        let mut text = String::new();
        loop {
            match self.peek()? {
                '\\' => match self.peek_at(1) {
                    Some(next @ ('/' | '$' | '\\' | '}')) => {
                        text.push(next);
                        self.pos += 2;
                    }
                    _ => {
                        text.push('\\');
                        self.pos += 1;
                    }
                },
                '/' => {
                    self.pos += 1;
                    break;
                }
                '$' => {
                    let start = self.pos;
                    match self.format_item() {
                        Some(item) => {
                            if !text.is_empty() {
                                format.push(Format::Text(std::mem::take(&mut text)));
                            }
                            format.push(item);
                        }
                        None => {
                            self.pos = start + 1;
                            text.push('$');
                        }
                    }
                }
                c => {
                    text.push(c);
                    self.pos += 1;
                }
            }
        }
        if !text.is_empty() {
            format.push(Format::Text(text));
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        let options = self.chars[start..self.pos].iter().collect();
        Some(Transform {
            regex,
            format,
            options,
        })
    }

    fn format_item(&mut self) -> Option<Format> {
        self.pos += 1;
        if !self.eat('{') {
            return Some(Format::Group(self.number()? as usize, None));
        }
        let group = self.number()? as usize;
        if self.eat('}') {
            return Some(Format::Group(group, None));
        }
        if !self.eat(':') {
            return None;
        }
        if self.eat('/') {
            let case = self.name()?;
            return self.eat('}').then_some(Format::Group(group, Some(case)));
        }
        let text_until = |parser: &mut Self, stops: &[char]| -> Option<String> {
            let mut out = String::new();
            loop {
                match parser.peek()? {
                    '\\' if parser.peek_at(1).is_some() => {
                        out.push(parser.peek_at(1)?);
                        parser.pos += 2;
                    }
                    c if stops.contains(&c) => return Some(out),
                    c => {
                        out.push(c);
                        parser.pos += 1;
                    }
                }
            }
        };
        if self.eat('+') {
            let then = text_until(self, &['}'])?;
            self.eat('}');
            return Some(Format::Conditional(group, then, String::new()));
        }
        if self.eat('?') {
            let then = text_until(self, &[':', '}'])?;
            let otherwise = if self.eat(':') {
                text_until(self, &['}'])?
            } else {
                String::new()
            };
            self.eat('}');
            return Some(Format::Conditional(group, then, otherwise));
        }
        self.eat('-');
        let otherwise = text_until(self, &['}'])?;
        self.eat('}');
        Some(Format::Conditional(group, String::new(), otherwise))
    }
}

impl Transform {
    /// `value` through the transform: the regex's matches (all of them with
    /// `g`) replaced by the format; a regex that doesn't compile leaves the
    /// value as it is.
    fn apply(&self, value: &str) -> String {
        let Ok(regex) = regex::RegexBuilder::new(&self.regex)
            .case_insensitive(self.options.contains('i'))
            .multi_line(self.options.contains('m'))
            .build()
        else {
            return value.to_string();
        };
        let global = self.options.contains('g');
        let mut out = String::new();
        let mut last = 0;
        for captures in regex.captures_iter(value) {
            let Some(whole) = captures.get(0) else {
                continue;
            };
            out.push_str(&value[last..whole.start()]);
            for item in &self.format {
                let group = |ix: usize| captures.get(ix).map_or("", |m| m.as_str());
                match item {
                    Format::Text(text) => out.push_str(text),
                    Format::Group(ix, case) => {
                        out.push_str(&change_case(group(*ix), case.as_deref()));
                    }
                    Format::Conditional(ix, then, otherwise) => {
                        out.push_str(if group(*ix).is_empty() {
                            otherwise
                        } else {
                            then
                        });
                    }
                }
            }
            last = whole.end();
            if !global {
                break;
            }
        }
        out.push_str(&value[last..]);
        out
    }
}

fn change_case(text: &str, case: Option<&str>) -> String {
    let capitalize = |word: &str| {
        let mut chars = word.chars();
        chars.next().map_or(String::new(), |first| {
            first.to_uppercase().chain(chars).collect::<String>()
        })
    };
    let words = || text.split(|c: char| c == '_' || c == '-' || c.is_whitespace());
    match case {
        Some("upcase") => text.to_uppercase(),
        Some("downcase") => text.to_lowercase(),
        Some("capitalize") => capitalize(text),
        Some("pascalcase") => words().map(capitalize).collect(),
        Some("camelcase") => {
            let pascal: String = words().map(capitalize).collect();
            let mut chars = pascal.chars();
            chars.next().map_or(String::new(), |first| {
                first.to_lowercase().chain(chars).collect()
            })
        }
        _ => text.to_string(),
    }
}

/// A snippet ready to insert: its text and its tab stops' places in it, in
/// the order they appear (`$0` last, at the end when the snippet has none).
#[derive(Debug, PartialEq)]
pub(crate) struct Expanded {
    pub(crate) text: String,
    pub(crate) stops: Vec<Stop>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stop {
    pub(crate) index: u32,
    pub(crate) range: Range<usize>,
    pub(crate) choices: Option<Vec<String>>,
}

/// The text being built: each line after the first starts with `indent`,
/// and a tab in a line's leading whitespace becomes `tab` (spaces) when
/// given.
struct Out<'a> {
    text: String,
    indent: &'a str,
    tab: Option<&'a str>,
    at_line_start: bool,
}

impl Out<'_> {
    fn push(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\n' => {
                    self.text.push('\n');
                    self.text.push_str(self.indent);
                    self.at_line_start = true;
                }
                '\t' if self.at_line_start && self.tab.is_some() => {
                    self.text.push_str(self.tab.unwrap_or_default());
                }
                ' ' | '\t' => self.text.push(c),
                c => {
                    self.at_line_start = false;
                    self.text.push(c);
                }
            }
        }
    }
}

/// `nodes` as text with their stops: variables from `resolve` (an unknown
/// one is its default, or nothing), a stop without a placeholder of its
/// own taking the first placeholder text of its index, a choice its first
/// option.
pub(crate) fn expand(
    nodes: &[Node],
    resolve: &dyn Fn(&str) -> Option<String>,
    indent: &str,
    tab: Option<&str>,
) -> Expanded {
    // The first placeholder with text of each index gives the others theirs.
    fn defaults<'a>(nodes: &'a [Node], into: &mut HashMap<u32, &'a [Node]>) {
        for node in nodes {
            match node {
                Node::TabStop {
                    index, children, ..
                } => {
                    if !children.is_empty() {
                        into.entry(*index).or_insert(children);
                    }
                    defaults(children, into);
                }
                Node::Variable {
                    default: Some(children),
                    ..
                } => defaults(children, into),
                _ => {}
            }
        }
    }
    let mut placeholders = HashMap::new();
    defaults(nodes, &mut placeholders);

    struct Render<'a, 'b> {
        out: Out<'b>,
        stops: Vec<Stop>,
        placeholders: &'a HashMap<u32, &'a [Node]>,
        resolve: &'a dyn Fn(&str) -> Option<String>,
    }
    impl Render<'_, '_> {
        /// `record`: whether stops are kept (not inside a mirrored copy).
        fn nodes(&mut self, nodes: &[Node], record: bool) {
            for node in nodes {
                match node {
                    Node::Text(text) => self.out.push(text),
                    Node::TabStop {
                        index,
                        children,
                        choices,
                    } => {
                        let start = self.out.text.len();
                        if let Some(first) = choices.as_ref().and_then(|c| c.first()) {
                            self.out.push(first);
                        } else if !children.is_empty() {
                            self.nodes(children, record);
                        } else if let Some(text) = self.placeholders.get(index).copied() {
                            self.nodes(text, false);
                        }
                        if record {
                            self.stops.push(Stop {
                                index: *index,
                                range: start..self.out.text.len(),
                                choices: choices.clone(),
                            });
                        }
                    }
                    Node::Variable {
                        name,
                        default,
                        transform,
                    } => match (self.resolve)(name) {
                        Some(value) if !value.is_empty() || default.is_none() => {
                            let value = match transform {
                                Some(transform) => transform.apply(&value),
                                None => value,
                            };
                            self.out.push(&value);
                        }
                        _ => {
                            if let Some(default) = default {
                                self.nodes(default, record);
                            }
                        }
                    },
                }
            }
        }
    }
    let mut render = Render {
        out: Out {
            text: String::new(),
            indent,
            tab,
            at_line_start: false,
        },
        stops: Vec::new(),
        placeholders: &placeholders,
        resolve,
    };
    render.nodes(nodes, true);
    let Render { out, mut stops, .. } = render;
    let end = out.text.len();
    if !stops.iter().any(|stop| stop.index == 0) {
        stops.push(Stop {
            index: 0,
            range: end..end,
            choices: None,
        });
    }
    Expanded {
        text: out.text,
        stops,
    }
}

/// One tab stop's places in the text, and its choices.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Group {
    pub(crate) index: u32,
    pub(crate) ranges: Vec<Range<usize>>,
    pub(crate) choices: Option<Vec<String>>,
}

/// A snippet being filled in.
#[derive(Clone, Debug)]
pub(crate) struct SnippetSession {
    /// By index, `$0` last.
    pub(crate) groups: Vec<Group>,
    pub(crate) current: usize,
}

impl SnippetSession {
    /// A session over `stops`, placed from `start`.
    pub(crate) fn new(stops: &[Stop], start: usize) -> Self {
        let mut groups: Vec<Group> = Vec::new();
        for stop in stops {
            let range = start + stop.range.start..start + stop.range.end;
            match groups.iter_mut().find(|g| g.index == stop.index) {
                Some(group) => {
                    group.ranges.push(range);
                    if group.choices.is_none() {
                        group.choices = stop.choices.clone();
                    }
                }
                None => groups.push(Group {
                    index: stop.index,
                    ranges: vec![range],
                    choices: stop.choices.clone(),
                }),
            }
        }
        groups.sort_by_key(|g| if g.index == 0 { u32::MAX } else { g.index });
        for group in &mut groups {
            group.ranges.sort_by_key(|r| r.start);
        }
        Self { groups, current: 0 }
    }

    /// The stop after the current one with places left, `$0` included.
    pub(crate) fn next(&self) -> Option<usize> {
        (self.current + 1..self.groups.len()).find(|&ix| !self.groups[ix].ranges.is_empty())
    }

    /// The stop before the current one with places left.
    pub(crate) fn previous(&self) -> Option<usize> {
        (0..self.current)
            .rev()
            .find(|&ix| !self.groups[ix].ranges.is_empty())
    }

    /// Whether `range` lies inside one of the stops' places (ends
    /// included).
    pub(crate) fn contains(&self, range: &Range<usize>) -> bool {
        self.groups
            .iter()
            .flat_map(|g| &g.ranges)
            .any(|r| r.start <= range.start && range.end <= r.end)
    }

    /// The placeholders to tint: every stop's places but `$0`'s, those with
    /// text.
    pub(crate) fn placeholders(&self) -> impl Iterator<Item = &Range<usize>> {
        self.groups
            .iter()
            .filter(|g| g.index != 0)
            .flat_map(|g| &g.ranges)
            .filter(|r| !r.is_empty())
    }

    /// Move the places with an edit replacing `range` with `new_len` bytes.
    /// The current stop's places grow with text typed at their edges; the
    /// others don't, and lose a place the edit replaced whole (a nested
    /// placeholder typed over).
    pub(crate) fn adjust_for_edit(&mut self, range: &Range<usize>, new_len: usize) {
        let (a, b) = (range.start, range.end);
        let delta = new_len as isize - range.len() as isize;
        let replaced = a + new_len;
        // Where an offset goes: kept before the edit (at its start too when
        // `keep_at_start`, or when the edit removes text there), moved after
        // it, and to the end of the new text inside it.
        let map = |x: usize, keep_at_start: bool| {
            if x < a || (x == a && (keep_at_start || a < b)) {
                x
            } else if x >= b {
                x.saturating_add_signed(delta)
            } else {
                replaced
            }
        };
        let current = self.current;
        for (ix, group) in self.groups.iter_mut().enumerate() {
            let growing = ix == current;
            let last = group.index == 0;
            group.ranges.retain_mut(|r| {
                let (s, e) = (r.start, r.end);
                // A place of another stop that the edit replaced whole goes.
                if !growing && !last && a < b && a <= s && e <= b && s < e {
                    return false;
                }
                let start = map(s, growing);
                let end = if growing || e > b || (e == b && a < b) {
                    map(e, false)
                } else if e <= a {
                    // Text typed at its end doesn't join it.
                    e
                } else {
                    replaced
                };
                *r = start..end.max(start);
                true
            });
        }
    }
}

/// A random 64-bit number (for `RANDOM` and `UUID`), from a freshly keyed
/// hasher.
fn random() -> u64 {
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    hasher.finish()
}

impl InputBaseState<EditorMode> {
    /// Impulse: variables a snippet can use that the editor can't know (the
    /// file's: `TM_FILENAME`, `TM_FILENAME_BASE`, `TM_DIRECTORY`,
    /// `TM_FILEPATH`, `RELATIVE_FILEPATH`, `WORKSPACE_NAME`,
    /// `WORKSPACE_FOLDER`, the language's `LINE_COMMENT`,
    /// `BLOCK_COMMENT_START` and `BLOCK_COMMENT_END`), by name. The editor
    /// resolves the selection, line, word, date, time, random and UUID ones.
    pub fn set_snippet_variables(&mut self, variables: HashMap<String, String>) {
        self.snippet_variables = variables;
    }

    /// Impulse: whether a snippet is being filled in.
    pub fn in_snippet(&self) -> bool {
        self.snippet.is_some()
    }

    /// Impulse: insert a snippet (LSP's snippet syntax) in place of `range`,
    /// its first tab stop selected; Tab and Shift+Tab then move between its
    /// stops.
    pub fn insert_snippet(
        &mut self,
        range: Range<usize>,
        snippet: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert_snippet_with(range, snippet, Vec::new(), window, cx);
    }

    /// Impulse: a snippet and edits beside it (a completion's
    /// `additionalTextEdits`, an auto-import) in one undo step. Each edit is
    /// a byte range of the text as it is and its new text, and doesn't
    /// overlap `range`.
    pub(crate) fn insert_snippet_with(
        &mut self,
        range: Range<usize>,
        snippet: &str,
        mut others: Vec<(Range<usize>, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = self.text.offset_to_point(range.start).row;
        let line = self.text.slice_line(row).to_string();
        let indent = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
        let tab = self.mode.tab_size();
        let spaces = (!tab.hard_tabs).then(|| " ".repeat(tab.tab_size));
        let expanded = {
            let resolve = |name: &str| self.snippet_variable(name, &range);
            expand(&parse(snippet), &resolve, indent, spaces.as_deref())
        };

        // From the end, so each edit's offsets stay good: the edits after
        // the snippet, the snippet, then those before it (which move the
        // session's places).
        others.retain(|(r, _)| r.end <= range.start || r.start >= range.end);
        others.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
        let (after, before): (Vec<_>, Vec<_>) =
            others.into_iter().partition(|(r, _)| r.start >= range.end);
        self.undo_manager.begin_transaction();
        for (r, text) in &after {
            let r = self.range_to_utf16(r);
            self.replace_text_in_range_silent(Some(r), text, window, cx);
        }
        let start = range.start;
        let utf16 = self.range_to_utf16(&range);
        self.snippet = None;
        self.replace_text_in_range_silent(Some(utf16), &expanded.text, window, cx);
        self.snippet = Some(SnippetSession::new(&expanded.stops, start));
        for (r, text) in &before {
            let r = self.range_to_utf16(r);
            self.replace_text_in_range_silent(Some(r), text, window, cx);
        }
        self.undo_manager.commit_transaction();

        // The first stop selected; a snippet with only `$0` puts the cursor
        // there and has nothing to fill in.
        self.select_snippet_stop(0, window, cx);
    }

    /// Select the session's stop `ix`: all its places, the first active.
    /// Reaching `$0` ends the session.
    fn select_snippet_stop(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.snippet.as_mut() else {
            return;
        };
        let Some(group) = session.groups.get(ix).cloned() else {
            return;
        };
        session.current = ix;
        if group.index == 0 {
            self.snippet = None;
        }
        let mut ranges = group.ranges.into_iter();
        let Some(first) = ranges.next() else {
            return;
        };
        self.set_selected_range(first.clone(), cx);
        for range in ranges {
            self.add_selection(range, cx);
        }
        // A choice offers its options, as Monaco's does.
        if let Some(choices) = group.choices.filter(|c| c.len() > 1) {
            let start = self.text.offset_to_position(first.start);
            let end = self.text.offset_to_position(first.end);
            let items = choices
                .into_iter()
                .map(|choice| lsp_types::CompletionItem {
                    label: choice.clone(),
                    kind: Some(lsp_types::CompletionItemKind::VALUE),
                    text_edit: Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
                        range: lsp_types::Range::new(start, end),
                        new_text: choice,
                    })),
                    ..Default::default()
                })
                .collect();
            self.present_completion_items(first.start, "", items, cx);
        }
        cx.notify();
    }

    /// Impulse: to the snippet's next tab stop (`forward`) or the one
    /// before; whether there was one (Tab and Shift+Tab do this in a
    /// snippet).
    pub fn jump_in_snippet(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self.snippet.as_ref() else {
            return false;
        };
        let target = if forward {
            session.next()
        } else {
            session.previous()
        };
        let Some(target) = target else {
            return false;
        };
        self.select_snippet_stop(target, window, cx);
        true
    }

    /// Impulse: stop filling in the snippet (Escape); whether there was
    /// one.
    pub fn leave_snippet(&mut self, cx: &mut Context<Self>) -> bool {
        if self.snippet.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    /// A variable's value for a snippet going in at `range`; `None` when
    /// the editor doesn't know it.
    fn snippet_variable(&self, name: &str, range: &Range<usize>) -> Option<String> {
        if let Some(value) = self.snippet_variables.get(name) {
            return Some(value.clone());
        }
        let point = self.text.offset_to_point(range.start);
        let now = chrono::Local::now();
        let date = |format: &str| Some(now.format(format).to_string());
        match name {
            "TM_SELECTED_TEXT" => {
                let selected = self.selected_range();
                Some(if selected.is_empty() {
                    String::new()
                } else {
                    self.text.slice(selected).to_string()
                })
            }
            "TM_CURRENT_LINE" => Some(self.text.slice_line(point.row).to_string()),
            "TM_CURRENT_WORD" => {
                let line = self.text.slice_line(point.row).to_string();
                let word = |c: char| c.is_alphanumeric() || c == '_';
                let column = point.column.min(line.len());
                let start = line[..column]
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| word(*c))
                    .last()
                    .map_or(column, |(ix, _)| ix);
                let end = line[column..]
                    .char_indices()
                    .find(|(_, c)| !word(*c))
                    .map_or(line.len(), |(ix, _)| column + ix);
                Some(line[start..end].to_string())
            }
            "TM_LINE_INDEX" => Some(point.row.to_string()),
            "TM_LINE_NUMBER" => Some((point.row + 1).to_string()),
            "CURRENT_YEAR" => date("%Y"),
            "CURRENT_YEAR_SHORT" => date("%y"),
            "CURRENT_MONTH" => date("%m"),
            "CURRENT_MONTH_NAME" => date("%B"),
            "CURRENT_MONTH_NAME_SHORT" => date("%b"),
            "CURRENT_DATE" => date("%d"),
            "CURRENT_DAY_NAME" => date("%A"),
            "CURRENT_DAY_NAME_SHORT" => date("%a"),
            "CURRENT_HOUR" => date("%H"),
            "CURRENT_MINUTE" => date("%M"),
            "CURRENT_SECOND" => date("%S"),
            "CURRENT_SECONDS_UNIX" => Some(now.timestamp().to_string()),
            "CURRENT_TIMEZONE_OFFSET" => date("%:z"),
            "RANDOM" => Some(format!("{:06}", random() % 1_000_000)),
            "RANDOM_HEX" => Some(format!("{:06x}", random() & 0xff_ffff)),
            "UUID" => {
                let (a, b) = (random(), random());
                Some(format!(
                    "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                    a >> 32,
                    (a >> 16) & 0xffff,
                    a & 0xfff,
                    0x8000 | ((b >> 48) & 0x3fff),
                    b & 0xffff_ffff_ffff
                ))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_plain(source: &str) -> Expanded {
        expand(&parse(source), &|_| None, "", None)
    }

    fn stops(expanded: &Expanded) -> Vec<(u32, &str)> {
        expanded
            .stops
            .iter()
            .map(|stop| (stop.index, &expanded.text[stop.range.clone()]))
            .collect()
    }

    #[test]
    fn tab_stops_and_placeholders() {
        let e = expand_plain("for (${1:i} = 0; $1 < ${2:n}; $1++) {\n\t$0\n}");
        assert_eq!(e.text, "for (i = 0; i < n; i++) {\n\t\n}");
        assert_eq!(
            stops(&e),
            vec![(1, "i"), (1, "i"), (2, "n"), (1, "i"), (0, "")]
        );
        assert_eq!(e.stops.last().unwrap().range, 27..27);
    }

    #[test]
    fn a_final_stop_goes_at_the_end_when_there_is_none() {
        let e = expand_plain("log($1)");
        assert_eq!(e.text, "log()");
        assert_eq!(stops(&e), vec![(1, ""), (0, "")]);
        assert_eq!(e.stops[1].range, 5..5);
    }

    #[test]
    fn nested_placeholders() {
        let e = expand_plain("${1:outer ${2:inner} end}$0");
        assert_eq!(e.text, "outer inner end");
        assert_eq!(
            stops(&e),
            vec![(2, "inner"), (1, "outer inner end"), (0, "")]
        );
    }

    #[test]
    fn a_mirror_takes_a_later_placeholder() {
        let e = expand_plain("$1 = ${1:value}");
        assert_eq!(e.text, "value = value");
        assert_eq!(stops(&e), vec![(1, "value"), (1, "value"), (0, "")]);
    }

    #[test]
    fn choices_insert_their_first_option() {
        let e = expand_plain("${1|let,const,var|} x");
        assert_eq!(e.text, "let x");
        assert_eq!(
            e.stops[0].choices.as_deref().unwrap(),
            ["let", "const", "var"]
        );
        let escaped = expand_plain(r"${1|a\,b,c\|d|}");
        assert_eq!(escaped.stops[0].choices.as_deref().unwrap(), ["a,b", "c|d"]);
    }

    #[test]
    fn escapes_and_stray_characters_are_text() {
        let e = expand_plain(r"\$1 \} \\ $ ${ } a$");
        assert_eq!(e.text, r"$1 } \ $ ${ } a$");
        assert_eq!(stops(&e), vec![(0, "")]);
        // An unclosed placeholder is text, its stop inside still a stop.
        let e = expand_plain("${1:open $2");
        assert_eq!(e.text, "${1:open ");
        assert_eq!(stops(&e), vec![(2, ""), (0, "")]);
        // A backslash before anything else stays.
        assert_eq!(expand_plain(r"a\nb").text, r"a\nb");
    }

    #[test]
    fn variables_resolve_or_take_their_default() {
        let resolve = |name: &str| match name {
            "TM_FILENAME" => Some("main.rs".to_string()),
            "TM_SELECTED_TEXT" => Some(String::new()),
            _ => None,
        };
        let e = expand(
            &parse(
                "$TM_FILENAME ${TM_FILENAME} ${UNKNOWN:fallback} $UNKNOWN. ${TM_SELECTED_TEXT:${1:sel}}",
            ),
            &resolve,
            "",
            None,
        );
        assert_eq!(e.text, "main.rs main.rs fallback . sel");
        assert_eq!(stops(&e), vec![(1, "sel"), (0, "")]);
    }

    #[test]
    fn transforms_change_variables() {
        let resolve = |name: &str| (name == "TM_FILENAME").then(|| "my_file.test.ts".to_string());
        let e = expand(
            &parse(
                r"${TM_FILENAME/(.*)\..+$/${1:/upcase}/} ${TM_FILENAME/[_.]/-/g} ${TM_FILENAME/(x)?.*/${1:?yes:no}/}",
            ),
            &resolve,
            "",
            None,
        );
        assert_eq!(e.text, "MY_FILE.TEST my-file-test-ts no");
        let e = expand(
            &parse("${TM_FILENAME/(.*)/${1:/pascalcase}/}"),
            &resolve,
            "",
            None,
        );
        assert_eq!(e.text, "MyFile.test.ts");
        // A tab stop's transform is parsed and left out.
        let e = expand_plain("${1:a} ${1/a/b/}");
        assert_eq!(e.text, "a a");
    }

    #[test]
    fn lines_take_the_indentation_and_tabs_become_spaces() {
        let e = expand(&parse("if ($1) {\n\t$0\n}"), &|_| None, "    ", Some("  "));
        assert_eq!(e.text, "if () {\n      \n    }");
        assert_eq!(e.stops[0].range, 4..4);
        assert_eq!(e.stops[1].range, 14..14);
        // Hard tabs stay.
        let e = expand(&parse("a\n\tb"), &|_| None, "\t", None);
        assert_eq!(e.text, "a\n\t\tb");
    }

    #[test]
    fn a_session_moves_between_stops_and_follows_edits() {
        let e = expand_plain("f(${1:a}, ${2:b}) $1$0");
        // "f(a, b) a"
        let mut session = SnippetSession::new(&e.stops, 10);
        assert_eq!(session.groups[0].ranges, vec![12..13, 18..19]);
        assert_eq!(session.groups[1].ranges, vec![15..16]);
        assert_eq!(session.groups[2].ranges, vec![19..19]);
        // Typing over stop 1's places (from the end, as a multi-cursor
        // edit is applied) grows them and moves the rest.
        session.adjust_for_edit(&(18..19), 3);
        session.adjust_for_edit(&(12..13), 3);
        assert_eq!(session.groups[0].ranges, vec![12..15, 20..23]);
        assert_eq!(session.groups[1].ranges, vec![17..18]);
        assert_eq!(session.groups[2].ranges, vec![23..23]);
        // Typing at the edge of the current stop grows it.
        session.adjust_for_edit(&(15..15), 1);
        assert_eq!(session.groups[0].ranges[0], 12..16);
        assert_eq!(session.groups[1].ranges, vec![18..19]);
        assert_eq!(session.next(), Some(1));
        session.current = 1;
        assert_eq!(session.previous(), Some(0));
        assert_eq!(session.next(), Some(2));
        // Typing at another stop's edge doesn't grow that stop.
        session.adjust_for_edit(&(16..16), 1);
        assert_eq!(session.groups[0].ranges, vec![12..16, 22..25]);
        assert_eq!(session.groups[1].ranges, vec![19..20]);
        assert!(session.contains(&(19..19)));
        assert!(!session.contains(&(5..5)));
    }

    #[gpui::test]
    fn a_snippet_is_filled_in_stop_by_stop(cx: &mut gpui::TestAppContext) {
        use crate::input::inlay_hints::tests::editor;
        use gpui::EntityInputHandler as _;

        let (state, mut cx) = editor("fn main() {\n    \n}", cx);
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.insert_snippet(16..16, "if ${1:cond} {\n\t${2:body}\n}$0", window, cx);
                // Indented as the line, a tab as the editor's two spaces.
                assert_eq!(
                    state.text().to_string(),
                    "fn main() {\n    if cond {\n      body\n    }\n}"
                );
                assert!(state.in_snippet());
                assert_eq!(state.selected_range(), 19..23);
                state.replace_text_in_range(None, "ok", window, cx);
                assert!(state.in_snippet());
                assert!(state.jump_in_snippet(true, window, cx));
                let body = state.selected_range();
                assert_eq!(state.text().slice(body.clone()).to_string(), "body");
                // Back to the first stop, grown with what was typed.
                assert!(state.jump_in_snippet(false, window, cx));
                assert_eq!(state.text().slice(state.selected_range()).to_string(), "ok");
                assert!(!state.jump_in_snippet(false, window, cx));
                assert!(state.jump_in_snippet(true, window, cx));
                // Reaching `$0` ends it.
                assert!(state.jump_in_snippet(true, window, cx));
                assert!(!state.in_snippet());
                assert_eq!(state.cursor(), state.text().len() - 2);
                assert!(!state.jump_in_snippet(true, window, cx));
            })
        });
        // A stop's places edit together; Escape leaves.
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.insert_snippet(0..0, "${1:a} + $1 = ${2:b}", window, cx);
                assert_eq!(state.selected_ranges().len(), 2);
                state.replace_text_in_range(None, "zz", window, cx);
                assert_eq!(state.text().to_string(), "zz + zz = b");
                assert!(state.leave_snippet(cx));
                assert!(!state.in_snippet());
                assert!(!state.leave_snippet(cx));
            })
        });
        // A snippet with only `$0` just puts the cursor there.
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.insert_snippet(0..0, "a($0)", window, cx);
                assert!(!state.in_snippet());
                assert_eq!(state.cursor(), 2);
            })
        });
        // Moving the cursor out of the snippet ends it.
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_value("x\n", window, cx);
                state.insert_snippet(0..0, "f(${1:a})", window, cx);
                assert!(state.in_snippet());
                state.set_cursor_position(crate::input::Position::new(1, 0), window, cx);
            })
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(!cx.update(|_, cx| state.read(cx).in_snippet()));
    }

    #[gpui::test]
    fn a_completion_snippet_goes_in_with_its_other_edits(cx: &mut gpui::TestAppContext) {
        use crate::input::inlay_hints::tests::editor;
        use lsp_types::{
            CompletionItem, CompletionTextEdit, InsertTextFormat, Position, Range, TextEdit,
        };

        let (state, mut cx) = editor("\nlet v = pri", cx);
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                let item = CompletionItem {
                    label: "print".into(),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: Range::new(Position::new(1, 8), Position::new(1, 11)),
                        new_text: "print(${1:value})$0".into(),
                    })),
                    additional_text_edits: Some(vec![TextEdit {
                        range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                        new_text: "use io::{print};".into(),
                    }]),
                    ..Default::default()
                };
                state.insert_completion(&item, 12..12, window, cx);
                assert_eq!(
                    state.text().to_string(),
                    "use io::{print};\nlet v = print(value)"
                );
                assert_eq!(
                    state.text().slice(state.selected_range()).to_string(),
                    "value"
                );
                // One undo takes it all back.
                state.undo(&crate::input::Undo, window, cx);
                assert_eq!(state.text().to_string(), "\nlet v = pri");
            })
        });
    }

    #[test]
    fn typing_over_a_placeholder_drops_the_ones_inside_it() {
        let e = expand_plain("${1:outer ${2:inner}}");
        let mut session = SnippetSession::new(&e.stops, 0);
        assert_eq!(session.groups[0].index, 1);
        session.adjust_for_edit(&(0..11), 1);
        assert_eq!(session.groups[0].ranges, vec![0..1]);
        assert!(session.groups[1].ranges.is_empty());
        assert_eq!(session.next(), Some(2));
        assert_eq!(session.groups[2].ranges, vec![1..1]);
    }
}
