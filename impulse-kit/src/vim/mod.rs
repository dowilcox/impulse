//! Vim mode for the editor (the Swift app runs monaco-vim), as a pure state
//! machine: keys go in with the text and the cursor, and out come one
//! replacement, the selection afterwards, and what only the editor can do
//! (undo, save, close, scroll, the system clipboard). Normal, Insert, Visual
//! (characters and lines) and Replace modes; counts; motions; the
//! operators with motions and text objects; registers; `.`; search; and the
//! `:` commands in `ex`. Text typed in Insert mode is the editor's own: the
//! machine only sees the text before and after, to repeat it with `.`.

mod ex;
#[cfg(test)]
mod tests;
mod text;

use std::collections::HashMap;
use std::ops::Range;

use regex::{Regex, RegexBuilder};
use text::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Replace,
    Visual,
    VisualLine,
}

impl Mode {
    /// The mode as monaco-vim's status bar shows it.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "--NORMAL--",
            Mode::Insert => "--INSERT--",
            Mode::Replace => "--REPLACE--",
            Mode::Visual => "--VISUAL--",
            Mode::VisualLine => "--VISUAL LINE--",
        }
    }

    fn is_visual(self) -> bool {
        matches!(self, Mode::Visual | Mode::VisualLine)
    }
}

/// A key as the machine reads it: a typed character (Shift already applied),
/// Ctrl with a letter or `[`, or a named key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Escape,
    Enter,
    Backspace,
    Delete,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

impl Key {
    /// Keys in Vim's notation: `d2w`, `ifoo<Esc>`, `<C-r>`, `<CR>`.
    pub fn parse_all(s: &str) -> Vec<Key> {
        let mut keys = Vec::new();
        let mut rest = s;
        while let Some(c) = rest.chars().next() {
            if c == '<' {
                if let Some(end) = rest.find('>') {
                    let name = &rest[1..end];
                    let key = match name.to_ascii_lowercase().as_str() {
                        "esc" => Some(Key::Escape),
                        "cr" | "enter" => Some(Key::Enter),
                        "bs" => Some(Key::Backspace),
                        "del" => Some(Key::Delete),
                        "tab" => Some(Key::Tab),
                        "up" => Some(Key::Up),
                        "down" => Some(Key::Down),
                        "left" => Some(Key::Left),
                        "right" => Some(Key::Right),
                        "home" => Some(Key::Home),
                        "end" => Some(Key::End),
                        "lt" => Some(Key::Char('<')),
                        n if n.starts_with("c-") && n.chars().count() == 3 => {
                            n.chars().nth(2).map(Key::Ctrl)
                        }
                        _ => None,
                    };
                    if let Some(key) = key {
                        keys.push(key);
                        rest = &rest[end + 1..];
                        continue;
                    }
                }
            }
            keys.push(Key::Char(c));
            rest = &rest[c.len_utf8()..];
        }
        keys
    }

    /// How the status bar shows a pending key.
    fn label(self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Ctrl(c) => format!("^{}", c.to_ascii_uppercase()),
            Key::Escape => "<Esc>".into(),
            Key::Enter => "<CR>".into(),
            Key::Backspace => "<BS>".into(),
            Key::Delete => "<Del>".into(),
            Key::Tab => "<Tab>".into(),
            Key::Up => "<Up>".into(),
            Key::Down => "<Down>".into(),
            Key::Left => "<Left>".into(),
            Key::Right => "<Right>".into(),
            Key::Home => "<Home>".into(),
            Key::End => "<End>".into(),
            Key::PageUp => "<PageUp>".into(),
            Key::PageDown => "<PageDown>".into(),
        }
    }
}

/// When `:q`, `:wq` and `:x` save before closing the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseSave {
    Never,
    Always,
    IfModified,
}

/// Where `zt`, `zz` and `zb` put the cursor's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reveal {
    Top,
    Center,
    Bottom,
}

/// What the editor does after a key, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Undo(usize),
    Redo(usize),
    Save,
    /// Close the tab (asking about unsaved changes, as closing does).
    Close(CloseSave),
    /// The `+` and `*` registers.
    CopyToClipboard(String),
    /// Scroll by this many lines (down when positive).
    Scroll(i64),
    Reveal(Reveal),
}

/// The result of a key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// False when the editor should handle the key itself (Insert mode).
    pub handled: bool,
    /// One replacement of the text's bytes, as one undo step.
    pub edit: Option<(Range<usize>, String)>,
    /// The selection afterwards, in the new text (empty: the cursor).
    pub selection: Option<Range<usize>>,
    pub actions: Vec<Action>,
}

/// What the machine needs to know about the editor for a key.
pub struct Context<'a> {
    pub text: &'a str,
    /// The cursor: the moving end of the selection.
    pub cursor: usize,
    /// Lines on screen (end exclusive), for `H`, `M`, `L` and scrolling;
    /// empty before the editor's first layout.
    pub visible_lines: Range<usize>,
    /// One level of indentation (`>`), and the width of a tab (`<`).
    pub indent: &'a str,
    pub tab_size: usize,
    /// Reads the system clipboard, for the `+` and `*` registers.
    pub clipboard: &'a mut dyn FnMut() -> Option<String>,
}

/// What the status bar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub mode: &'static str,
    /// The `:`, `/` or `?` line being typed.
    pub command_line: Option<String>,
    /// An error or notice from the last key.
    pub message: Option<String>,
    /// Keys of a command not finished yet ("2d", "\"a").
    pub keys: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Register {
    text: String,
    linewise: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordStart(bool),
    WordBack(bool),
    WordEnd(bool),
    WordEndBack(bool),
    LineStart,
    FirstNonBlank,
    LineEnd,
    NextLine,
    PrevLine,
    CurrentLine,
    Column,
    FileStart,
    FileEnd,
    Find {
        ch: char,
        forward: bool,
        till: bool,
    },
    RepeatFind(bool),
    MatchPair,
    ParagraphForward,
    ParagraphBack,
    ScreenTop,
    ScreenMiddle,
    ScreenBottom,
    SearchNext(bool),
    SearchWord(bool),
    /// `/` or `?`: open the search line.
    SearchPrompt(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Object {
    Word(bool),
    Paragraph,
    Quote(char),
    Bracket(char, char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextObject {
    object: Object,
    around: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Lower,
    Upper,
    ToggleCase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Line,
    Motion(Motion),
    Object(TextObject),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsertAt {
    Before,
    After,
    LineStart,
    LineEnd,
    Below,
    Above,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scroll {
    HalfPage,
    Page,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    DeleteChar,
    DeleteCharBefore,
    Substitute,
    SubstituteLine,
    DeleteToEnd,
    ChangeToEnd,
    YankLine,
    ReplaceChar(char),
    ReplaceNewline,
    Join,
    Put { before: bool },
    Undo,
    Redo,
    Repeat,
    Insert(InsertAt),
    ReplaceMode,
    Visual { linewise: bool },
    ToggleCaseChar,
    Scroll { by: Scroll, down: bool },
    Reveal { at: Reveal, first_non_blank: bool },
    Reselect,
    CommandLine,
    SwapEnds,
    Escape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Move(Motion),
    Operate(Op, Target),
    /// An operator on the Visual selection (`linewise`: `D`, `X`, `Y`, `C`,
    /// `S` and `R` take whole lines).
    VisualOperate {
        op: Op,
        linewise: bool,
    },
    Select(TextObject),
    Do(Act),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cmd {
    register: Option<char>,
    count: Option<usize>,
    kind: Kind,
}

enum Parse {
    More,
    Bad,
    Done(Cmd),
}

/// Text typed in Insert mode, relative to where it started: characters
/// removed before and after that place, and what's there now.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Typed {
    delete_before: usize,
    delete_after: usize,
    text: String,
}

/// A Visual selection's size, for `.` after a Visual operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VisualSize {
    linewise: bool,
    /// Lines below the first.
    lines: usize,
    /// On one line, characters selected; over several, the last line's
    /// column.
    chars: usize,
}

/// The last change, for `.`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    cmd: Cmd,
    typed: Option<Typed>,
    visual: Option<VisualSize>,
}

struct InsertSession {
    before: String,
    start: usize,
    count: usize,
    /// `o` and `O`: repeats go on new lines with this indentation.
    new_line: Option<String>,
    change: Option<Change>,
}

struct CommandLine {
    prompt: char,
    text: String,
    /// An operator waiting for a search (`d/foo`).
    operator: Option<(Op, Option<usize>, Option<char>)>,
    /// A count for the search (`3/foo`).
    count: Option<usize>,
}

#[derive(Clone)]
struct Search {
    pattern: String,
    forward: bool,
}

/// The editor's state as the machine changes it during one key.
struct Work {
    text: String,
    cursor: usize,
    actions: Vec<Action>,
}

/// Where a motion goes, and how an operator takes it.
#[derive(Clone, Copy, Debug)]
struct Hit {
    pos: usize,
    linewise: bool,
    inclusive: bool,
}

const MAX_COUNT: usize = 99_999;

pub struct Vim {
    mode: Mode,
    pending: Vec<Key>,
    registers: HashMap<char, Register>,
    /// The column `j` and `k` keep to (`usize::MAX` after `$`).
    want_column: Option<usize>,
    last_find: Option<(char, bool, bool)>,
    search: Option<Search>,
    regex: Option<(String, bool, Regex)>,
    highlight: bool,
    command_line: Option<CommandLine>,
    message: Option<String>,
    anchor: usize,
    head: usize,
    last_visual: Option<(usize, usize, bool)>,
    /// The selection last handed to the editor, to tell its own changes
    /// (mouse, Select All) from ours.
    shown: Option<Range<usize>>,
    insert: Option<InsertSession>,
    last_change: Option<Change>,
    replaying: bool,
    /// Replace mode: the characters typed over, for Backspace.
    replaced: Vec<Option<char>>,
    last_inserted: String,
    last_ex: String,
}

impl Default for Vim {
    fn default() -> Self {
        Self::new()
    }
}

impl Vim {
    pub fn new() -> Self {
        Vim {
            mode: Mode::Normal,
            pending: Vec::new(),
            registers: HashMap::new(),
            want_column: None,
            last_find: None,
            search: None,
            regex: None,
            highlight: false,
            command_line: None,
            message: None,
            anchor: 0,
            head: 0,
            last_visual: None,
            shown: None,
            insert: None,
            last_change: None,
            replaying: false,
            replaced: Vec::new(),
            last_inserted: String::new(),
            last_ex: String::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn status(&self) -> Status {
        Status {
            mode: self.mode.label(),
            command_line: self
                .command_line
                .as_ref()
                .map(|line| format!("{}{}", line.prompt, line.text)),
            message: self.message.clone(),
            keys: self.pending.iter().map(|k| k.label()).collect(),
        }
    }

    /// Whether keys go to the machine rather than the editor's own typing:
    /// every mode but Insert (and the command line, typed in any mode).
    pub fn takes_keys(&self) -> bool {
        self.mode != Mode::Insert || self.command_line.is_some()
    }

    /// The character the cursor is on, drawn as a block in Normal mode.
    pub fn block_cursor(&self, text: &str, cursor: usize) -> Option<Range<usize>> {
        if !matches!(self.mode, Mode::Normal | Mode::Replace) || self.command_line.is_some() {
            return None;
        }
        let cursor = cursor.min(text.len());
        (cursor < line_end(text, cursor)).then(|| cursor..next(text, cursor))
    }

    /// The moving end of the Visual selection, which the view follows.
    pub fn visual_head(&self) -> Option<usize> {
        self.mode.is_visual().then_some(self.head)
    }

    /// The search pattern while its matches are highlighted.
    pub fn highlighted_pattern(&self) -> Option<&str> {
        self.search
            .as_ref()
            .filter(|_| self.highlight)
            .map(|s| s.pattern.as_str())
    }

    /// The last search's matches while they're highlighted (until `:noh`).
    pub fn search_highlights(&mut self, text: &str) -> Vec<Range<usize>> {
        if !self.highlight {
            return Vec::new();
        }
        let Some(regex) = self.search_regex() else {
            return Vec::new();
        };
        regex
            .find_iter(text)
            .filter(|m| !m.is_empty())
            .take(5000)
            .map(|m| m.range())
            .collect()
    }

    /// The editor changed the cursor or selection itself (a click, a drag,
    /// Select All, an undo): follow it. A selection in Normal mode starts
    /// Visual mode; a cursor past a line's end moves back onto it. Returns
    /// the selection to put in the editor instead, if any.
    pub fn sync(
        &mut self,
        text: &str,
        cursor: usize,
        selection: Range<usize>,
    ) -> Option<Range<usize>> {
        if self.shown.as_ref() == Some(&selection)
            || self.command_line.is_some()
            || matches!(self.mode, Mode::Insert | Mode::Replace)
        {
            return None;
        }
        self.want_column = None;
        self.pending.clear();
        if selection.is_empty() {
            if self.mode.is_visual() {
                self.mode = Mode::Normal;
            }
            let at = clamp_normal(text, cursor);
            self.shown = Some(at..at);
            return (at != cursor).then_some(at..at);
        }
        if !self.mode.is_visual() {
            self.mode = Mode::Visual;
        }
        let (start, end) = (selection.start, selection.end);
        if cursor == start {
            self.anchor = prev(text, end);
            self.head = start;
        } else {
            self.anchor = start;
            self.head = prev(text, end);
        }
        self.shown = Some(selection);
        None
    }

    /// Back to Normal mode with nothing pending (after the setting is turned
    /// on, or the tab gets the focus back).
    pub fn reset(&mut self) {
        self.mode = Mode::Normal;
        self.pending.clear();
        self.command_line = None;
        self.insert = None;
        self.shown = None;
    }

    /// Handle one key.
    pub fn key(&mut self, key: Key, cx: &mut Context) -> Outcome {
        self.message = None;
        // Insert mode's typing is the editor's.
        if self.mode == Mode::Insert
            && self.command_line.is_none()
            && !matches!(key, Key::Escape | Key::Ctrl('[') | Key::Ctrl('c'))
        {
            return Outcome::default();
        }
        let mut w = Work {
            text: cx.text.to_string(),
            cursor: cx.cursor.min(cx.text.len()),
            actions: Vec::new(),
        };
        if self.command_line.is_some() {
            self.command_line_key(key, &mut w, cx);
        } else {
            match self.mode {
                Mode::Insert => self.finish_insert(&mut w),
                Mode::Replace => self.replace_key(key, &mut w),
                Mode::Normal | Mode::Visual | Mode::VisualLine => {
                    self.pending.push(key);
                    match self.parse(&self.pending) {
                        Parse::More => {}
                        Parse::Bad => self.pending.clear(),
                        Parse::Done(cmd) => {
                            self.pending.clear();
                            self.run(cmd, &mut w, cx);
                        }
                    }
                }
            }
        }
        self.outcome(cx.text, w)
    }

    fn outcome(&mut self, original: &str, w: Work) -> Outcome {
        let edit = diff(original, &w.text);
        let selection = match self.mode {
            Mode::Visual | Mode::VisualLine => self.visual_range(&w.text),
            Mode::Normal => {
                let at = clamp_normal(&w.text, w.cursor);
                at..at
            }
            Mode::Insert | Mode::Replace => w.cursor..w.cursor,
        };
        self.shown = Some(selection.clone());
        Outcome {
            handled: true,
            edit,
            selection: Some(selection),
            actions: w.actions,
        }
    }

    // MARK: Parsing

    fn parse(&self, keys: &[Key]) -> Parse {
        let visual = self.mode.is_visual();
        let mut i = 0;
        let mut register = None;
        if keys.first() == Some(&Key::Char('"')) {
            match keys.get(1) {
                None => return Parse::More,
                Some(Key::Char(c)) if is_register(*c) => register = Some(*c),
                _ => return Parse::Bad,
            }
            i = 2;
        }
        let (count, next_i) = parse_count(keys, i);
        i = next_i;
        let Some(&key) = keys.get(i) else {
            return Parse::More;
        };
        let done = |kind| {
            Parse::Done(Cmd {
                register,
                count,
                kind,
            })
        };
        // Operators.
        let operator = match key {
            Key::Char('d') => Some((Op::Delete, 1)),
            Key::Char('c') => Some((Op::Change, 1)),
            Key::Char('y') => Some((Op::Yank, 1)),
            Key::Char('>') => Some((Op::Indent, 1)),
            Key::Char('<') => Some((Op::Outdent, 1)),
            Key::Char('g') => match keys.get(i + 1) {
                None => return Parse::More,
                Some(Key::Char('~')) => Some((Op::ToggleCase, 2)),
                Some(Key::Char('u')) => Some((Op::Lower, 2)),
                Some(Key::Char('U')) => Some((Op::Upper, 2)),
                _ => None,
            },
            _ => None,
        };
        if let Some((op, len)) = operator {
            if visual {
                return done(Kind::VisualOperate {
                    op,
                    linewise: false,
                });
            }
            let at = i + len;
            let (count2, at) = parse_count(keys, at);
            let count = match (count, count2) {
                (None, None) => None,
                (a, b) => Some((a.unwrap_or(1) * b.unwrap_or(1)).min(MAX_COUNT)),
            };
            let done = |target| {
                Parse::Done(Cmd {
                    register,
                    count,
                    kind: Kind::Operate(op, target),
                })
            };
            let Some(&next_key) = keys.get(at) else {
                return Parse::More;
            };
            // `dd`, `cc`, `>>`, `guu`, `gUU`, `g~~`, `gugu`.
            let doubled = keys[i + len - 1];
            if next_key == doubled {
                return done(Target::Line);
            }
            if len == 2 && next_key == Key::Char('g') {
                return match keys.get(at + 1) {
                    None => Parse::More,
                    Some(k) if *k == doubled => done(Target::Line),
                    _ => match parse_motion(keys, at) {
                        Step::More => Parse::More,
                        Step::Bad => Parse::Bad,
                        Step::Done(m) => done(Target::Motion(m)),
                    },
                };
            }
            if let Key::Char(c @ ('i' | 'a')) = next_key {
                return match parse_object(keys.get(at + 1).copied(), c == 'a') {
                    Step::More => Parse::More,
                    Step::Bad => Parse::Bad,
                    Step::Done(object) => done(Target::Object(object)),
                };
            }
            return match parse_motion(keys, at) {
                Step::More => Parse::More,
                Step::Bad => Parse::Bad,
                Step::Done(m) => done(Target::Motion(m)),
            };
        }
        if visual {
            if let Key::Char(c @ ('i' | 'a')) = key {
                return match parse_object(keys.get(i + 1).copied(), c == 'a') {
                    Step::More => Parse::More,
                    Step::Bad => Parse::Bad,
                    Step::Done(object) => done(Kind::Select(object)),
                };
            }
            let visual_op = |op, linewise| done(Kind::VisualOperate { op, linewise });
            match key {
                Key::Char('x') | Key::Delete => return visual_op(Op::Delete, false),
                Key::Char('X') | Key::Char('D') => return visual_op(Op::Delete, true),
                Key::Char('s') => return visual_op(Op::Change, false),
                Key::Char('C') | Key::Char('S') | Key::Char('R') => {
                    return visual_op(Op::Change, true)
                }
                Key::Char('Y') => return visual_op(Op::Yank, true),
                Key::Char('~') => return visual_op(Op::ToggleCase, false),
                Key::Char('u') => return visual_op(Op::Lower, false),
                Key::Char('U') => return visual_op(Op::Upper, false),
                Key::Char('o') | Key::Char('O') => return done(Kind::Do(Act::SwapEnds)),
                Key::Char('I') => return done(Kind::Do(Act::Insert(InsertAt::LineStart))),
                Key::Char('A') => return done(Kind::Do(Act::Insert(InsertAt::LineEnd))),
                _ => {}
            }
        }
        // Motions.
        match parse_motion(keys, i) {
            Step::More => return Parse::More,
            Step::Done(m) => return done(Kind::Move(m)),
            Step::Bad => {}
        }
        let act = |act| done(Kind::Do(act));
        match key {
            Key::Char('x') | Key::Delete => act(Act::DeleteChar),
            Key::Char('X') => act(Act::DeleteCharBefore),
            Key::Char('s') => act(Act::Substitute),
            Key::Char('S') => act(Act::SubstituteLine),
            Key::Char('D') => act(Act::DeleteToEnd),
            Key::Char('C') => act(Act::ChangeToEnd),
            Key::Char('Y') => act(Act::YankLine),
            Key::Char('r') => match keys.get(i + 1) {
                None => Parse::More,
                Some(Key::Char(c)) => act(Act::ReplaceChar(*c)),
                Some(Key::Enter) => act(Act::ReplaceNewline),
                Some(Key::Tab) => act(Act::ReplaceChar('\t')),
                _ => Parse::Bad,
            },
            Key::Char('J') => act(Act::Join),
            Key::Char('p') => act(Act::Put { before: false }),
            Key::Char('P') => act(Act::Put { before: true }),
            Key::Char('u') => act(Act::Undo),
            Key::Ctrl('r') => act(Act::Redo),
            Key::Char('.') => act(Act::Repeat),
            Key::Char('i') => act(Act::Insert(InsertAt::Before)),
            Key::Char('a') => act(Act::Insert(InsertAt::After)),
            Key::Char('I') => act(Act::Insert(InsertAt::LineStart)),
            Key::Char('A') => act(Act::Insert(InsertAt::LineEnd)),
            Key::Char('o') => act(Act::Insert(InsertAt::Below)),
            Key::Char('O') => act(Act::Insert(InsertAt::Above)),
            Key::Char('R') => act(Act::ReplaceMode),
            Key::Char('v') => act(Act::Visual { linewise: false }),
            Key::Char('V') => act(Act::Visual { linewise: true }),
            Key::Char('~') => act(Act::ToggleCaseChar),
            Key::Char(':') => act(Act::CommandLine),
            Key::Ctrl('d') => act(Act::Scroll {
                by: Scroll::HalfPage,
                down: true,
            }),
            Key::Ctrl('u') => act(Act::Scroll {
                by: Scroll::HalfPage,
                down: false,
            }),
            Key::Ctrl('f') | Key::PageDown => act(Act::Scroll {
                by: Scroll::Page,
                down: true,
            }),
            Key::Ctrl('b') | Key::PageUp => act(Act::Scroll {
                by: Scroll::Page,
                down: false,
            }),
            Key::Ctrl('e') => act(Act::Scroll {
                by: Scroll::Line,
                down: true,
            }),
            Key::Ctrl('y') => act(Act::Scroll {
                by: Scroll::Line,
                down: false,
            }),
            Key::Char('z') => {
                let reveal = |at, first_non_blank| {
                    act(Act::Reveal {
                        at,
                        first_non_blank,
                    })
                };
                match keys.get(i + 1) {
                    None => Parse::More,
                    Some(Key::Char('z')) => reveal(Reveal::Center, false),
                    Some(Key::Char('t')) => reveal(Reveal::Top, false),
                    Some(Key::Char('b')) => reveal(Reveal::Bottom, false),
                    Some(Key::Char('.')) => reveal(Reveal::Center, true),
                    Some(Key::Enter) => reveal(Reveal::Top, true),
                    Some(Key::Char('-')) => reveal(Reveal::Bottom, true),
                    _ => Parse::Bad,
                }
            }
            Key::Char('g') => match keys.get(i + 1) {
                None => Parse::More,
                Some(Key::Char('v')) => act(Act::Reselect),
                _ => Parse::Bad,
            },
            Key::Escape | Key::Ctrl('[') | Key::Ctrl('c') => act(Act::Escape),
            _ => Parse::Bad,
        }
    }

    // MARK: Running commands

    fn run(&mut self, cmd: Cmd, w: &mut Work, cx: &mut Context) {
        let count = cmd.count.unwrap_or(1).clamp(1, MAX_COUNT);
        let visual_before = self.mode.is_visual().then(|| self.visual_size(&w.text));
        let changes = is_change(&cmd.kind);
        match cmd.kind {
            Kind::Move(Motion::SearchPrompt(forward)) => {
                self.open_command_line(if forward { '/' } else { '?' }, String::new(), None);
                if let Some(line) = &mut self.command_line {
                    line.count = cmd.count;
                }
            }
            Kind::Move(m) => self.move_by(m, cmd.count, w, cx),
            Kind::Operate(op, Target::Motion(Motion::SearchPrompt(forward))) => {
                self.open_command_line(
                    if forward { '/' } else { '?' },
                    String::new(),
                    Some((op, cmd.count, cmd.register)),
                );
            }
            Kind::Operate(op, target) => {
                let Some(span) = self.target_span(op, target, cmd.count, w, cx) else {
                    return;
                };
                self.operate(op, span, cmd.register, 1, w, cx);
            }
            Kind::VisualOperate { op, linewise } => {
                let mut span = self.visual_span(&w.text);
                if linewise && !span.linewise {
                    span = lines_span(&w.text, span.start, prev(&w.text, span.end));
                }
                // Operators leave the cursor at the selection's start.
                w.cursor = self.anchor.min(self.head);
                self.end_visual();
                self.mode = Mode::Normal;
                self.operate(op, span, cmd.register, count, w, cx);
            }
            Kind::Select(object) => {
                if let Some(span) = self.object_span(object, cmd.count, self.head, &w.text) {
                    if span.end > span.start {
                        self.anchor = span.start;
                        self.head = prev(&w.text, span.end);
                        if span.linewise {
                            self.mode = Mode::VisualLine;
                        }
                    }
                }
            }
            Kind::Do(act) => self.act(act, cmd, count, w, cx),
        }
        if changes && !self.replaying {
            let change = Change {
                cmd,
                typed: None,
                visual: visual_before,
            };
            match &mut self.insert {
                Some(session) if self.mode == Mode::Insert => session.change = Some(change),
                _ => self.last_change = Some(change),
            }
        }
    }

    fn move_by(&mut self, m: Motion, count: Option<usize>, w: &mut Work, cx: &mut Context) {
        let from = if self.mode.is_visual() {
            self.head
        } else {
            w.cursor
        };
        let column = self.want_column;
        let Some(hit) = self.target(m, count, false, from, &w.text, cx) else {
            return;
        };
        let pos = clamp_normal(&w.text, hit.pos);
        if self.mode.is_visual() {
            self.head = pos;
        } else {
            w.cursor = pos;
        }
        self.want_column = match m {
            Motion::Up | Motion::Down => {
                Some(column.unwrap_or_else(|| text::column(&w.text, from)))
            }
            Motion::LineEnd => Some(usize::MAX),
            _ => None,
        };
    }

    /// Where motion `m` goes from `from`; `None` when it can't move (or, for
    /// an operator, has nothing to take).
    fn target(
        &mut self,
        m: Motion,
        count: Option<usize>,
        op: bool,
        from: usize,
        t: &str,
        cx: &Context,
    ) -> Option<Hit> {
        let n = count.unwrap_or(1).clamp(1, MAX_COUNT);
        let exclusive = |pos| {
            Some(Hit {
                pos,
                linewise: false,
                inclusive: false,
            })
        };
        let inclusive = |pos| {
            Some(Hit {
                pos,
                linewise: false,
                inclusive: true,
            })
        };
        let linewise = |pos| {
            Some(Hit {
                pos,
                linewise: true,
                inclusive: false,
            })
        };
        let line = line_of(t, from);
        let last_line = line_count(t) - 1;
        let first_of = |l: usize| first_non_blank(t, start_of_line(t, l));
        let (view_first, view_last) = if cx.visible_lines.is_empty() {
            (0, last_line)
        } else {
            (
                cx.visible_lines.start.min(last_line),
                (cx.visible_lines.end - 1).min(last_line),
            )
        };
        match m {
            Motion::Left => {
                let start = line_start(t, from);
                if from <= start {
                    return None;
                }
                let mut p = from;
                for _ in 0..n {
                    if p <= start {
                        break;
                    }
                    p = prev(t, p);
                }
                exclusive(p)
            }
            Motion::Right => {
                let end = line_end(t, from);
                let limit = if op {
                    end
                } else {
                    prev(t, end).max(line_start(t, from))
                };
                if from >= limit {
                    return None;
                }
                let mut p = from;
                for _ in 0..n {
                    if p >= limit {
                        break;
                    }
                    p = next(t, p);
                }
                exclusive(p)
            }
            Motion::Up | Motion::Down => {
                let target = if m == Motion::Up {
                    line.checked_sub(n.min(line))?
                } else {
                    (line + n).min(last_line)
                };
                if target == line {
                    return None;
                }
                let col = self.want_column.unwrap_or_else(|| column(t, from));
                let start = start_of_line(t, target);
                let pos = if col == usize::MAX {
                    line_end(t, start)
                } else {
                    at_column(t, start, col)
                };
                linewise(pos)
            }
            Motion::WordStart(big) => {
                let mut p = from;
                for i in 0..n {
                    let step_from = p;
                    p = word_forward(t, p, big);
                    if op && i + 1 == n && line_of(t, p) > line_of(t, step_from) {
                        // The last word of a line: up to its end, not into
                        // the next line.
                        let end = line_end(t, step_from);
                        if end > step_from || i > 0 {
                            p = end;
                        }
                    }
                }
                if p == from {
                    return None;
                }
                exclusive(p)
            }
            Motion::WordBack(big) => {
                let mut p = from;
                for _ in 0..n {
                    p = word_backward(t, p, big);
                }
                (p != from).then_some(())?;
                exclusive(p)
            }
            Motion::WordEnd(big) => {
                let mut p = from;
                for _ in 0..n {
                    p = word_end(t, p, big);
                }
                inclusive(p)
            }
            Motion::WordEndBack(big) => {
                let mut p = from;
                for _ in 0..n {
                    p = word_end_backward(t, p, big);
                }
                inclusive(p)
            }
            Motion::LineStart => exclusive(line_start(t, from)),
            Motion::FirstNonBlank => exclusive(first_non_blank(t, from)),
            Motion::LineEnd => {
                let target = (line + n - 1).min(last_line);
                let end = line_end(t, start_of_line(t, target));
                if op {
                    exclusive(end)
                } else {
                    inclusive(end)
                }
            }
            Motion::NextLine => {
                if line == last_line {
                    return None;
                }
                linewise(first_of((line + n).min(last_line)))
            }
            Motion::PrevLine => {
                if line == 0 {
                    return None;
                }
                linewise(first_of(line.saturating_sub(n)))
            }
            Motion::CurrentLine => linewise(first_of((line + n - 1).min(last_line))),
            Motion::Column => exclusive(at_column(t, line_start(t, from), n - 1)),
            Motion::FileStart => linewise(first_of(count.map_or(0, |c| c - 1).min(last_line))),
            Motion::FileEnd => {
                linewise(first_of(count.map_or(last_line, |c| c - 1).min(last_line)))
            }
            Motion::Find { ch, forward, till } => {
                self.last_find = Some((ch, forward, till));
                let pos = find_in_line(t, from, ch, forward, till, n, false)?;
                if forward {
                    inclusive(pos)
                } else {
                    exclusive(pos)
                }
            }
            Motion::RepeatFind(reverse) => {
                let (ch, forward, till) = self.last_find?;
                let forward = forward != reverse;
                let pos = find_in_line(t, from, ch, forward, till, n, true)?;
                if forward {
                    inclusive(pos)
                } else {
                    exclusive(pos)
                }
            }
            Motion::MatchPair => {
                if let Some(percent) = count {
                    let lines = last_line + 1;
                    let target = (percent.min(100) * lines).div_ceil(100).max(1) - 1;
                    return linewise(first_of(target));
                }
                inclusive(match_pair(t, from)?)
            }
            Motion::ParagraphForward => {
                let mut p = from;
                for _ in 0..n {
                    p = paragraph_forward(t, p);
                }
                exclusive(p)
            }
            Motion::ParagraphBack => {
                let mut p = from;
                for _ in 0..n {
                    p = paragraph_backward(t, p);
                }
                exclusive(p)
            }
            Motion::ScreenTop => linewise(first_of((view_first + n - 1).min(view_last))),
            Motion::ScreenMiddle => linewise(first_of(view_first + (view_last - view_first) / 2)),
            Motion::ScreenBottom => {
                linewise(first_of(view_last.saturating_sub(n - 1).max(view_first)))
            }
            Motion::SearchNext(reverse) => {
                let Some(search) = self.search.clone() else {
                    self.message = Some("No previous search pattern".into());
                    return None;
                };
                self.highlight = true;
                let forward = search.forward != reverse;
                let pos = self.search_from(t, from, forward, n)?;
                exclusive(pos)
            }
            Motion::SearchWord(forward) => {
                let (word, start) = word_for_search(t, from)?;
                self.search = Some(Search {
                    pattern: format!("\\b{}\\b", regex::escape(word)),
                    forward,
                });
                self.highlight = true;
                let pos = self.search_from(t, start, forward, n)?;
                exclusive(pos)
            }
            Motion::SearchPrompt(_) => None,
        }
    }

    /// The span an operator takes: whole lines, a motion's reach, or a text
    /// object.
    fn target_span(
        &mut self,
        op: Op,
        target: Target,
        count: Option<usize>,
        w: &Work,
        cx: &Context,
    ) -> Option<Span> {
        let t = &w.text;
        let from = w.cursor;
        let n = count.unwrap_or(1).clamp(1, MAX_COUNT);
        match target {
            Target::Line => {
                let last = (line_of(t, from) + n - 1).min(line_count(t) - 1);
                Some(lines_span(t, from, start_of_line(t, last)))
            }
            Target::Object(object) => self.object_span(object, count, from, t),
            Target::Motion(Motion::WordStart(big))
                if op == Op::Change && char_at(t, from).is_some_and(|c| !c.is_whitespace()) =>
            {
                // `cw` on a word changes to its end, like `ce`.
                let at_end = {
                    let after = next(t, from);
                    after >= line_end(t, from)
                        || class(char_at(t, after).unwrap_or(' '), big)
                            != class(char_at(t, from).unwrap_or(' '), big)
                };
                let mut end = if at_end { from } else { word_end(t, from, big) };
                for _ in 1..n {
                    end = word_end(t, end, big);
                }
                Some(Span::chars(from..inclusive_end(t, end)))
            }
            Target::Motion(m) => {
                let hit = self.target(m, count, true, from, t, cx)?;
                let (start, end) = (from.min(hit.pos), from.max(hit.pos));
                if hit.linewise {
                    return Some(lines_span(t, start, end));
                }
                if hit.inclusive {
                    return Some(Span::chars(start..next(t, end).min(t.len())));
                }
                // An exclusive motion ending at the start of a later line
                // stops at the end of the line before, or takes whole lines
                // when it also started at its line's first non-blank.
                if end > start && end == line_start(t, end) && line_of(t, end) > line_of(t, start) {
                    if start <= first_non_blank(t, start) {
                        return Some(lines_span(t, start, prev(t, end)));
                    }
                    return Some(Span::chars(start..line_end(t, prev(t, end))));
                }
                Some(Span::chars(start..end))
            }
        }
    }

    fn object_span(
        &self,
        object: TextObject,
        count: Option<usize>,
        at: usize,
        t: &str,
    ) -> Option<Span> {
        let n = count.unwrap_or(1).clamp(1, MAX_COUNT);
        match object.object {
            Object::Word(big) => Some(word_object(t, at, big, object.around, n)),
            Object::Paragraph => Some(paragraph_object(t, at, object.around, n)),
            Object::Quote(q) => quote_object(t, at, q, object.around),
            Object::Bracket(open, close) => bracket_object(t, at, open, close, object.around, n),
        }
    }

    /// Apply an operator to a span.
    fn operate(
        &mut self,
        op: Op,
        span: Span,
        register: Option<char>,
        levels: usize,
        w: &mut Work,
        cx: &mut Context,
    ) {
        let t = w.text.clone();
        match op {
            Op::Delete => {
                let mut range = span.start..span.end;
                let mut yanked = t[range.clone()].to_string();
                if span.linewise && !yanked.ends_with('\n') {
                    yanked.push('\n');
                    // The last lines go with the line break before them.
                    if range.start > 0 {
                        range.start -= 1;
                        if t[..range.start].ends_with('\r') {
                            range.start -= 1;
                        }
                    }
                }
                self.store(register, yanked, span.linewise, false, w);
                w.text.replace_range(range.clone(), "");
                let at = range.start.min(w.text.len());
                w.cursor = if span.linewise {
                    first_non_blank(&w.text, line_start(&w.text, at))
                } else {
                    at
                };
            }
            Op::Change => {
                if span.linewise {
                    let last = prev(&t, span.end).max(span.start);
                    let end = line_end(&t, last).max(span.start);
                    let indent = indentation(&t, span.start).to_string();
                    let mut yanked = t[span.start..span.end].to_string();
                    if !yanked.ends_with('\n') {
                        yanked.push('\n');
                    }
                    self.store(register, yanked, true, false, w);
                    w.text.replace_range(span.start..end, &indent);
                    w.cursor = span.start + indent.len();
                } else {
                    self.store(
                        register,
                        t[span.start..span.end].to_string(),
                        false,
                        false,
                        w,
                    );
                    w.text.replace_range(span.start..span.end, "");
                    w.cursor = span.start;
                }
                self.start_insert(w, 1, None);
            }
            Op::Yank => {
                let mut yanked = t[span.start..span.end].to_string();
                if span.linewise && !yanked.ends_with('\n') {
                    yanked.push('\n');
                }
                self.store(register, yanked, span.linewise, true, w);
                if span.linewise {
                    if line_of(&t, span.start) < line_of(&t, w.cursor) {
                        w.cursor = at_column(&t, span.start, column(&t, w.cursor));
                    }
                } else if span.start < w.cursor {
                    w.cursor = span.start;
                }
            }
            Op::Indent | Op::Outdent => {
                let first = line_start(&t, span.start);
                let last_pos = if span.end > span.start {
                    prev(&t, span.end)
                } else {
                    span.start
                };
                let last = line_end(&t, last_pos.max(first));
                // `>>` and `3>>` indent once; a Visual `3>` three times.
                let block = &t[first..last];
                let changed: Vec<String> = block
                    .split('\n')
                    .map(|line| {
                        let mut line = line.to_string();
                        for _ in 0..levels {
                            line = if op == Op::Indent {
                                if line.trim().is_empty() {
                                    line
                                } else {
                                    format!("{}{line}", cx.indent)
                                }
                            } else {
                                outdent(&line, cx.tab_size)
                            };
                        }
                        line
                    })
                    .collect();
                w.text.replace_range(first..last, &changed.join("\n"));
                w.cursor = first_non_blank(&w.text, first);
            }
            Op::Lower | Op::Upper | Op::ToggleCase => {
                let changed: String = t[span.start..span.end]
                    .chars()
                    .map(|c| match op {
                        Op::Lower => c.to_lowercase().collect::<String>(),
                        Op::Upper => c.to_uppercase().collect(),
                        _ => toggle_case(c),
                    })
                    .collect();
                w.text.replace_range(span.start..span.end, &changed);
                w.cursor = span.start;
            }
        }
    }
}

/// Split from the impl above to keep it readable: the simple commands, the
/// Visual helpers, registers, Insert and Replace mode, the command line.
impl Vim {
    fn act(&mut self, act: Act, cmd: Cmd, count: usize, w: &mut Work, cx: &mut Context) {
        let visual = self.mode.is_visual();
        match act {
            Act::Escape => {
                if visual {
                    self.end_visual();
                    w.cursor = self.head;
                    self.mode = Mode::Normal;
                }
            }
            Act::DeleteChar | Act::DeleteCharBefore | Act::Substitute => {
                let motion = if act == Act::DeleteCharBefore {
                    Motion::Left
                } else {
                    Motion::Right
                };
                let op = if act == Act::Substitute {
                    Op::Change
                } else {
                    Op::Delete
                };
                match self.target_span(op, Target::Motion(motion), cmd.count, w, cx) {
                    Some(span) => self.operate(op, span, cmd.register, 1, w, cx),
                    None if act == Act::Substitute => self.start_insert(w, 1, None),
                    None => {}
                }
            }
            Act::SubstituteLine => {
                if let Some(span) = self.target_span(Op::Change, Target::Line, cmd.count, w, cx) {
                    self.operate(Op::Change, span, cmd.register, 1, w, cx);
                }
            }
            Act::DeleteToEnd | Act::ChangeToEnd => {
                let op = if act == Act::DeleteToEnd {
                    Op::Delete
                } else {
                    Op::Change
                };
                if let Some(span) =
                    self.target_span(op, Target::Motion(Motion::LineEnd), cmd.count, w, cx)
                {
                    self.operate(op, span, cmd.register, 1, w, cx);
                }
            }
            Act::YankLine => {
                if visual {
                    let span = self.visual_span(&w.text);
                    let span = lines_span(&w.text, span.start, prev(&w.text, span.end));
                    self.end_visual();
                    self.mode = Mode::Normal;
                    self.operate(Op::Yank, span, cmd.register, 1, w, cx);
                } else if let Some(span) =
                    self.target_span(Op::Yank, Target::Line, cmd.count, w, cx)
                {
                    self.operate(Op::Yank, span, cmd.register, 1, w, cx);
                }
            }
            Act::ReplaceChar(_) | Act::ReplaceNewline if visual => {
                let span = self.visual_span(&w.text);
                self.end_visual();
                self.mode = Mode::Normal;
                let c = if let Act::ReplaceChar(c) = act {
                    c
                } else {
                    '\n'
                };
                let replaced: String = w.text[span.start..span.end]
                    .chars()
                    .map(|ch| if ch == '\n' || ch == '\r' { ch } else { c })
                    .collect();
                w.text.replace_range(span.start..span.end, &replaced);
                w.cursor = span.start;
            }
            Act::ReplaceChar(c) => {
                let end = line_end(&w.text, w.cursor);
                let chars: Vec<(usize, char)> = w.text[w.cursor..end].char_indices().collect();
                if chars.len() < count {
                    return;
                }
                let stop = chars.get(count).map_or(end, |(i, _)| w.cursor + i);
                let replacement: String = std::iter::repeat_n(c, count).collect();
                w.text.replace_range(w.cursor..stop, &replacement);
                w.cursor = w.cursor + replacement.len() - c.len_utf8();
            }
            Act::ReplaceNewline => {
                let end = line_end(&w.text, w.cursor);
                let chars: Vec<(usize, char)> = w.text[w.cursor..end].char_indices().collect();
                if chars.len() < count {
                    return;
                }
                let stop = chars.get(count).map_or(end, |(i, _)| w.cursor + i);
                w.text.replace_range(w.cursor..stop, "\n");
                w.cursor += 1;
            }
            Act::Join => {
                let (first, lines) = if visual {
                    let span = self.visual_span(&w.text);
                    self.end_visual();
                    self.mode = Mode::Normal;
                    let first = line_of(&w.text, span.start);
                    let last = line_of(&w.text, prev(&w.text, span.end).max(span.start));
                    (span.start, (last - first + 1).max(2))
                } else {
                    (w.cursor, count.max(2))
                };
                self.join(first, lines, w);
            }
            Act::Put { before } => self.put(before, cmd.register, count, w, cx),
            Act::Undo => w.actions.push(Action::Undo(count)),
            Act::Redo => w.actions.push(Action::Redo(count)),
            Act::Repeat => self.repeat(cmd.count, w, cx),
            Act::Insert(at) => {
                if visual {
                    let span = self.visual_span(&w.text);
                    let linewise = self.mode == Mode::VisualLine;
                    self.end_visual();
                    self.mode = Mode::Normal;
                    w.cursor = match (at, linewise) {
                        (InsertAt::LineStart, true) => first_non_blank(&w.text, span.start),
                        (InsertAt::LineStart, false) => span.start,
                        (_, true) => line_end(&w.text, prev(&w.text, span.end).max(span.start)),
                        (_, false) => span.end,
                    };
                    self.start_insert(w, 1, None);
                    return;
                }
                let t = &w.text;
                let mut new_line = None;
                match at {
                    InsertAt::Before => {}
                    InsertAt::After => {
                        if w.cursor < line_end(t, w.cursor) {
                            w.cursor = next(t, w.cursor);
                        }
                    }
                    InsertAt::LineStart => w.cursor = first_non_blank(t, w.cursor),
                    InsertAt::LineEnd => w.cursor = line_end(t, w.cursor),
                    InsertAt::Below => {
                        let indent = indentation(t, w.cursor).to_string();
                        let end = line_end(t, w.cursor);
                        let eol = line_ending(t);
                        w.text.insert_str(end, &format!("{eol}{indent}"));
                        w.cursor = end + eol.len() + indent.len();
                        new_line = Some(indent);
                    }
                    InsertAt::Above => {
                        let indent = indentation(t, w.cursor).to_string();
                        let start = line_start(t, w.cursor);
                        let eol = line_ending(t);
                        w.text.insert_str(start, &format!("{indent}{eol}"));
                        w.cursor = start + indent.len();
                        new_line = Some(indent);
                    }
                }
                self.start_insert(w, count, new_line);
            }
            Act::ReplaceMode => {
                self.mode = Mode::Replace;
                self.replaced.clear();
            }
            Act::Visual { linewise } => {
                let wanted = if linewise {
                    Mode::VisualLine
                } else {
                    Mode::Visual
                };
                if self.mode == wanted {
                    self.end_visual();
                    w.cursor = self.head;
                    self.mode = Mode::Normal;
                } else {
                    if !visual {
                        self.anchor = w.cursor;
                        self.head = w.cursor;
                    }
                    self.mode = wanted;
                }
            }
            Act::ToggleCaseChar => {
                let end = line_end(&w.text, w.cursor);
                let mut stop = w.cursor;
                for _ in 0..count {
                    if stop >= end {
                        break;
                    }
                    stop = next(&w.text, stop);
                }
                if stop == w.cursor {
                    return;
                }
                let changed: String = w.text[w.cursor..stop].chars().map(toggle_case).collect();
                w.text.replace_range(w.cursor..stop, &changed);
                w.cursor += changed.len();
            }
            Act::Scroll { by, down } => {
                let lines = cx.visible_lines.len().max(1);
                let amount = match by {
                    Scroll::HalfPage => cmd.count.unwrap_or((lines / 2).max(1)),
                    Scroll::Page => (lines.saturating_sub(2)).max(1) * count,
                    Scroll::Line => count,
                } as i64;
                w.actions
                    .push(Action::Scroll(if down { amount } else { -amount }));
                let from = if visual { self.head } else { w.cursor };
                let line = line_of(&w.text, from) as i64;
                let last = line_count(&w.text) as i64 - 1;
                let target = match by {
                    Scroll::Line => {
                        // Keep the cursor on screen.
                        let (first, end) =
                            (cx.visible_lines.start as i64, cx.visible_lines.end as i64);
                        if cx.visible_lines.is_empty() {
                            line
                        } else if down {
                            line.max(first + amount)
                        } else {
                            line.min(end - 1 - amount)
                        }
                    }
                    _ => line + if down { amount } else { -amount },
                }
                .clamp(0, last) as usize;
                let column = self.want_column.unwrap_or_else(|| column(&w.text, from));
                let pos = clamp_normal(
                    &w.text,
                    at_column(&w.text, start_of_line(&w.text, target), column),
                );
                if visual {
                    self.head = pos;
                } else {
                    w.cursor = pos;
                }
            }
            Act::Reveal {
                at,
                first_non_blank: to_first,
            } => {
                if to_first {
                    w.cursor = first_non_blank(&w.text, w.cursor);
                }
                w.actions.push(Action::Reveal(at));
            }
            Act::Reselect => {
                let Some((anchor, head, linewise)) = self.last_visual else {
                    return;
                };
                let len = w.text.len();
                self.anchor = clamp_normal(&w.text, anchor.min(len));
                self.head = clamp_normal(&w.text, head.min(len));
                self.mode = if linewise {
                    Mode::VisualLine
                } else {
                    Mode::Visual
                };
            }
            Act::CommandLine => {
                let text = if visual {
                    self.end_visual();
                    "'<,'>".to_string()
                } else if let Some(n) = cmd.count {
                    if n == 1 {
                        ".".to_string()
                    } else {
                        format!(".,.+{}", n - 1)
                    }
                } else {
                    String::new()
                };
                self.open_command_line(':', text, None);
            }
            Act::SwapEnds => std::mem::swap(&mut self.anchor, &mut self.head),
        }
    }

    /// `J`: join `lines` lines from the one at `at`, with one space between
    /// (none after blanks or before `)`).
    fn join(&mut self, at: usize, lines: usize, w: &mut Work) {
        let mut cursor = None;
        for _ in 1..lines {
            let t = &w.text;
            let end = line_end(t, at.min(t.len()));
            let Some(next_start) = next_line_start(t, end) else {
                break;
            };
            let next_first = first_non_blank(t, next_start);
            let next_content = &t[next_first..line_end(t, next_start)];
            let line = &t[line_start(t, end)..end];
            let separator = if line.is_empty()
                || line.ends_with([' ', '\t'])
                || next_content.is_empty()
                || next_content.starts_with(')')
            {
                ""
            } else {
                " "
            };
            w.text.replace_range(end..next_first, separator);
            cursor = Some(end);
        }
        if let Some(c) = cursor {
            w.cursor = c;
        }
    }

    fn put(
        &mut self,
        before: bool,
        register: Option<char>,
        count: usize,
        w: &mut Work,
        cx: &mut Context,
    ) {
        let Some(reg) = self.read_register(register, cx) else {
            return;
        };
        let content: String = reg.text.repeat(count);
        if self.mode.is_visual() {
            let span = self.visual_span(&w.text);
            let selected_linewise = self.mode == Mode::VisualLine;
            self.end_visual();
            self.mode = Mode::Normal;
            let range = span.start..span.end;
            let mut removed = w.text[range.clone()].to_string();
            if selected_linewise && !removed.ends_with('\n') {
                removed.push('\n');
            }
            let (replacement, cursor) = if selected_linewise {
                let mut lines = if reg.linewise {
                    content
                } else {
                    format!("{content}\n")
                };
                // The last lines, which have no line break of their own.
                if range.end == w.text.len() && !w.text[range.clone()].ends_with('\n') {
                    lines.pop();
                }
                (lines, None)
            } else if reg.linewise {
                // Lines put into a line: they go on lines of their own.
                (format!("\n{content}"), Some(range.start + 1))
            } else {
                let last = content.chars().last().map_or(0, char::len_utf8);
                let at = range.start + content.len() - last;
                (content, Some(at))
            };
            w.text.replace_range(range.clone(), &replacement);
            w.cursor = match cursor {
                Some(at) if !reg.linewise => at,
                Some(at) => first_non_blank(&w.text, at.min(w.text.len())),
                None => first_non_blank(&w.text, range.start.min(w.text.len())),
            };
            // What was selected takes the unnamed register's place.
            self.store(None, removed, selected_linewise, false, w);
            return;
        }
        let t = &w.text;
        if reg.linewise {
            let at = if before {
                line_start(t, w.cursor)
            } else {
                next_line_start(t, w.cursor).unwrap_or(t.len())
            };
            if !before && next_line_start(t, w.cursor).is_none() {
                // After the last line, which has no line break of its own.
                let body = content.strip_suffix('\n').unwrap_or(&content);
                w.text.push('\n');
                let start = w.text.len();
                w.text.push_str(body);
                w.cursor = first_non_blank(&w.text, start);
            } else {
                w.text.insert_str(at, &content);
                w.cursor = first_non_blank(&w.text, at);
            }
        } else {
            let at = if before || w.cursor >= line_end(t, w.cursor) {
                w.cursor
            } else {
                next(t, w.cursor)
            };
            w.text.insert_str(at, &content);
            w.cursor = if content.contains('\n') {
                at
            } else {
                at + content.len() - content.chars().last().map_or(0, char::len_utf8)
            };
        }
    }

    /// `.`: the last change again, at the cursor (`count` replacing its own).
    fn repeat(&mut self, count: Option<usize>, w: &mut Work, cx: &mut Context) {
        let Some(change) = self.last_change.clone() else {
            return;
        };
        let mut cmd = change.cmd;
        if count.is_some() {
            cmd.count = count;
        }
        self.replaying = true;
        if let Some(size) = change.visual {
            // The same size of selection, from the cursor.
            let t = &w.text;
            self.anchor = w.cursor;
            let line = line_of(t, w.cursor);
            let last_line = start_of_line(t, (line + size.lines).min(line_count(t) - 1));
            self.head = if size.lines == 0 {
                let mut p = w.cursor;
                for _ in 1..size.chars.max(1) {
                    p = next(t, p).min(prev(t, line_end(t, p)).max(p));
                }
                p
            } else {
                clamp_normal(t, at_column(t, last_line, size.chars))
            };
            self.mode = if size.linewise {
                Mode::VisualLine
            } else {
                Mode::Visual
            };
        }
        self.run(cmd, w, cx);
        if self.mode == Mode::Insert {
            if let Some(typed) = &change.typed {
                let mut start = w.cursor;
                for _ in 0..typed.delete_before {
                    start = prev(&w.text, start);
                }
                let mut end = w.cursor;
                for _ in 0..typed.delete_after {
                    end = next(&w.text, end);
                }
                w.text.replace_range(start..end, &typed.text);
                w.cursor = start + typed.text.len();
            }
            self.finish_insert(w);
        }
        self.replaying = false;
    }

    // MARK: Visual mode

    fn visual_range(&self, t: &str) -> Range<usize> {
        let span = self.visual_span(t);
        span.start..span.end
    }

    fn visual_span(&self, t: &str) -> Span {
        let len = t.len();
        let (a, h) = (self.anchor.min(len), self.head.min(len));
        let (start, end) = (a.min(h), a.max(h));
        if self.mode == Mode::VisualLine {
            lines_span(t, start, end)
        } else {
            Span::chars(start..next(t, end).min(len))
        }
    }

    fn visual_size(&self, t: &str) -> VisualSize {
        let span = self.visual_span(t);
        let last = prev(t, span.end).max(span.start);
        let lines = line_of(t, last) - line_of(t, span.start);
        VisualSize {
            linewise: self.mode == Mode::VisualLine,
            lines,
            chars: if lines == 0 {
                t[span.start..span.end].chars().count()
            } else {
                column(t, last)
            },
        }
    }

    /// Remember the selection for `gv` and `'<,'>`.
    fn end_visual(&mut self) {
        self.last_visual = Some((self.anchor, self.head, self.mode == Mode::VisualLine));
    }

    // MARK: Registers

    /// Put yanked or deleted text in `register` (the unnamed one, `0` for a
    /// yank, `1`…`9` or `-` for a delete, when none is named).
    fn store(
        &mut self,
        register: Option<char>,
        text: String,
        linewise: bool,
        yank: bool,
        w: &mut Work,
    ) {
        let reg = Register { text, linewise };
        match register {
            Some('_') => return,
            Some('+' | '*') => w.actions.push(Action::CopyToClipboard(reg.text.clone())),
            Some(c) if c.is_ascii_uppercase() => {
                let lower = c.to_ascii_lowercase();
                let mut joined = self.registers.get(&lower).cloned().unwrap_or(Register {
                    text: String::new(),
                    linewise,
                });
                if joined.linewise && !reg.linewise {
                    joined.text.push_str(&reg.text);
                    joined.text.push('\n');
                } else {
                    joined.text.push_str(&reg.text);
                }
                joined.linewise |= reg.linewise;
                self.registers.insert(lower, joined.clone());
                self.registers.insert('"', joined);
                return;
            }
            Some(c) if c != '"' => {
                self.registers.insert(c, reg.clone());
            }
            _ => {
                if yank {
                    self.registers.insert('0', reg.clone());
                } else if linewise || reg.text.contains('\n') {
                    for n in (1..9).rev() {
                        let from = char::from(b'0' + n);
                        if let Some(r) = self.registers.get(&from).cloned() {
                            self.registers.insert(char::from(b'0' + n + 1), r);
                        }
                    }
                    self.registers.insert('1', reg.clone());
                } else {
                    self.registers.insert('-', reg.clone());
                }
            }
        }
        self.registers.insert('"', reg);
    }

    fn read_register(&self, register: Option<char>, cx: &mut Context) -> Option<Register> {
        let name = register.unwrap_or('"');
        match name {
            '+' | '*' => {
                let text = (cx.clipboard)()?;
                let linewise = text.ends_with('\n');
                Some(Register { text, linewise })
            }
            '.' => Some(Register {
                text: self.last_inserted.clone(),
                linewise: false,
            }),
            ':' => Some(Register {
                text: self.last_ex.clone(),
                linewise: false,
            }),
            '/' => self.search.as_ref().map(|s| Register {
                text: s.pattern.clone(),
                linewise: false,
            }),
            c => self.registers.get(&c.to_ascii_lowercase()).cloned(),
        }
        .filter(|r| !r.text.is_empty())
    }

    // MARK: Insert and Replace

    fn start_insert(&mut self, w: &Work, count: usize, new_line: Option<String>) {
        self.mode = Mode::Insert;
        self.insert = Some(InsertSession {
            before: w.text.clone(),
            start: w.cursor,
            count,
            new_line,
            change: None,
        });
    }

    /// Escape in Insert mode: repeat what was typed for a count, step back
    /// onto the last character, and keep it for `.`.
    fn finish_insert(&mut self, w: &mut Work) {
        self.mode = Mode::Normal;
        let Some(session) = self.insert.take() else {
            return;
        };
        let typed = typed_text(&session.before, &w.text, session.start);
        if session.count > 1 && typed.delete_before == 0 && typed.delete_after == 0 {
            let piece = match &session.new_line {
                Some(indent) => format!("{}{indent}{}", line_ending(&w.text), typed.text),
                None => typed.text.clone(),
            };
            let copies = piece.repeat(session.count - 1);
            w.text.insert_str(w.cursor, &copies);
            w.cursor += copies.len();
        }
        self.last_inserted = typed.text.clone();
        if let Some(mut change) = session.change {
            if !self.replaying {
                change.typed = Some(typed);
                self.last_change = Some(change);
            }
        }
        if w.cursor > line_start(&w.text, w.cursor) {
            w.cursor = prev(&w.text, w.cursor);
        }
    }

    fn replace_key(&mut self, key: Key, w: &mut Work) {
        match key {
            Key::Escape | Key::Ctrl('[') | Key::Ctrl('c') => {
                self.mode = Mode::Normal;
                if w.cursor > line_start(&w.text, w.cursor) {
                    w.cursor = prev(&w.text, w.cursor);
                }
            }
            Key::Char(c) => {
                let end = line_end(&w.text, w.cursor);
                if w.cursor < end {
                    let old = char_at(&w.text, w.cursor);
                    let stop = next(&w.text, w.cursor);
                    w.text
                        .replace_range(w.cursor..stop, c.encode_utf8(&mut [0; 4]));
                    self.replaced.push(old);
                } else {
                    w.text.insert(w.cursor, c);
                    self.replaced.push(None);
                }
                w.cursor += c.len_utf8();
            }
            Key::Enter => {
                w.text.insert(w.cursor, '\n');
                w.cursor += 1;
                self.replaced.clear();
            }
            Key::Backspace => {
                if w.cursor == 0 {
                    return;
                }
                let at = prev(&w.text, w.cursor);
                match self.replaced.pop() {
                    Some(Some(old)) => {
                        w.text
                            .replace_range(at..w.cursor, old.encode_utf8(&mut [0; 4]));
                    }
                    Some(None) => w.text.replace_range(at..w.cursor, ""),
                    None => {}
                }
                w.cursor = at;
            }
            Key::Left => {
                if w.cursor > line_start(&w.text, w.cursor) {
                    w.cursor = prev(&w.text, w.cursor);
                }
                self.replaced.clear();
            }
            Key::Right => {
                if w.cursor < line_end(&w.text, w.cursor) {
                    w.cursor = next(&w.text, w.cursor);
                }
                self.replaced.clear();
            }
            Key::Up | Key::Down => {
                let line = line_of(&w.text, w.cursor);
                let target = if key == Key::Up {
                    line.saturating_sub(1)
                } else {
                    (line + 1).min(line_count(&w.text) - 1)
                };
                let col = column(&w.text, w.cursor);
                w.cursor = at_column(&w.text, start_of_line(&w.text, target), col);
                self.replaced.clear();
            }
            _ => {}
        }
    }

    // MARK: The command line

    fn open_command_line(
        &mut self,
        prompt: char,
        text: String,
        operator: Option<(Op, Option<usize>, Option<char>)>,
    ) {
        self.command_line = Some(CommandLine {
            prompt,
            text,
            operator,
            count: None,
        });
    }

    fn command_line_key(&mut self, key: Key, w: &mut Work, cx: &mut Context) {
        let Some(line) = self.command_line.as_mut() else {
            return;
        };
        match key {
            Key::Escape | Key::Ctrl('[') | Key::Ctrl('c') => {
                self.command_line = None;
                self.leave_visual(w);
            }
            Key::Backspace => {
                if line.text.pop().is_none() {
                    self.command_line = None;
                    self.leave_visual(w);
                }
            }
            Key::Ctrl('u') => line.text.clear(),
            Key::Char(c) => line.text.push(c),
            Key::Enter => {
                let Some(line) = self.command_line.take() else {
                    return;
                };
                if line.prompt == ':' {
                    self.leave_visual(w);
                    self.ex(&line.text, w, cx);
                } else {
                    self.finish_search(line, w, cx);
                }
            }
            _ => {}
        }
    }

    fn leave_visual(&mut self, w: &mut Work) {
        if self.mode.is_visual() {
            self.end_visual();
            w.cursor = self.head;
            self.mode = Mode::Normal;
        }
    }

    fn finish_search(&mut self, line: CommandLine, w: &mut Work, cx: &mut Context) {
        let forward = line.prompt == '/';
        if !line.text.is_empty() {
            self.search = Some(Search {
                pattern: line.text.clone(),
                forward,
            });
        } else if let Some(search) = &mut self.search {
            search.forward = forward;
        }
        if self.search.is_none() {
            self.message = Some("No previous search pattern".into());
            return;
        }
        match line.operator {
            Some((op, count, register)) => {
                let span =
                    self.target_span(op, Target::Motion(Motion::SearchNext(false)), count, w, cx);
                if let Some(span) = span {
                    self.operate(op, span, register, 1, w, cx);
                    if op != Op::Yank && !self.replaying {
                        let cmd = Cmd {
                            register,
                            count,
                            kind: Kind::Operate(op, Target::Motion(Motion::SearchNext(false))),
                        };
                        let change = Change {
                            cmd,
                            typed: None,
                            visual: None,
                        };
                        match &mut self.insert {
                            Some(session) if self.mode == Mode::Insert => {
                                session.change = Some(change)
                            }
                            _ => self.last_change = Some(change),
                        }
                    }
                }
            }
            None => self.move_by(Motion::SearchNext(false), line.count, w, cx),
        }
    }

    fn search_regex(&mut self) -> Option<Regex> {
        let pattern = self.search.as_ref()?.pattern.clone();
        compile_cached(&mut self.regex, &pattern, false)
    }

    /// The `count`th match of the last search from `from`, wrapping around
    /// the end.
    fn search_from(&mut self, t: &str, from: usize, forward: bool, count: usize) -> Option<usize> {
        let regex = self.search_regex()?;
        let pattern = self
            .search
            .as_ref()
            .map(|s| s.pattern.clone())
            .unwrap_or_default();
        let mut at = from;
        for _ in 0..count {
            let found = if forward {
                regex
                    .find_at(t, next(t, at).min(t.len()))
                    .map(|m| m.start())
                    .filter(|&s| s > at || at == t.len())
                    .or_else(|| {
                        self.message = Some("Search hit BOTTOM, continuing at TOP".into());
                        regex.find(t).map(|m| m.start())
                    })
            } else {
                let before = regex.find_iter(t).take_while(|m| m.start() < at).last();
                before.map(|m| m.start()).or_else(|| {
                    self.message = Some("Search hit TOP, continuing at BOTTOM".into());
                    regex.find_iter(t).last().map(|m| m.start())
                })
            };
            match found {
                Some(p) => at = p,
                None => {
                    self.message = Some(format!("Pattern not found: {pattern}"));
                    return None;
                }
            }
        }
        Some(at)
    }

    fn ex(&mut self, line: &str, w: &mut Work, cx: &mut Context) {
        self.last_ex = line.to_string();
        let visual = self.last_visual.map(|(a, h, _)| {
            let len = w.text.len();
            let (a, h) = (line_of(&w.text, a.min(len)), line_of(&w.text, h.min(len)));
            (a.min(h), a.max(h))
        });
        let lines = ex::Lines::of(&w.text, w.cursor, visual);
        let command = match ex::parse(line, &lines) {
            Ok(command) => command,
            Err(message) => {
                if !message.is_empty() {
                    self.message = Some(message);
                }
                return;
            }
        };
        let line_span_of = |t: &str, r: ex::LineRange| {
            lines_span(t, start_of_line(t, r.first), start_of_line(t, r.last))
        };
        match command {
            ex::Ex::Goto(line) => {
                w.cursor = first_non_blank(&w.text, start_of_line(&w.text, line));
            }
            ex::Ex::Write => w.actions.push(Action::Save),
            ex::Ex::Quit => w.actions.push(Action::Close(CloseSave::Never)),
            ex::Ex::WriteQuit => w.actions.push(Action::Close(CloseSave::Always)),
            ex::Ex::Exit => w.actions.push(Action::Close(CloseSave::IfModified)),
            ex::Ex::NoHighlight => self.highlight = false,
            ex::Ex::Undo => w.actions.push(Action::Undo(1)),
            ex::Ex::Redo => w.actions.push(Action::Redo(1)),
            ex::Ex::Delete(range) => {
                let span = line_span_of(&w.text, range);
                self.operate(Op::Delete, span, None, 1, w, cx);
            }
            ex::Ex::Yank(range) => {
                let span = line_span_of(&w.text, range);
                let cursor = w.cursor;
                self.operate(Op::Yank, span, None, 1, w, cx);
                w.cursor = cursor;
            }
            ex::Ex::Substitute {
                lines,
                pattern,
                replacement,
                global,
                ignore_case,
            } => {
                let pattern = if pattern.is_empty() {
                    match &self.search {
                        Some(s) => s.pattern.clone(),
                        None => {
                            self.message = Some("No previous search pattern".into());
                            return;
                        }
                    }
                } else {
                    self.search = Some(Search {
                        pattern: pattern.clone(),
                        forward: true,
                    });
                    pattern
                };
                let mut cache = None;
                let Some(regex) = compile_cached(&mut cache, &pattern, ignore_case) else {
                    return;
                };
                // The lines as one block, each substituted on its own.
                let start = start_of_line(&w.text, lines.first);
                let end = line_end(&w.text, start_of_line(&w.text, lines.last));
                let block = w.text[start..end].to_string();
                let mut out = Vec::new();
                let mut line_index = 0;
                let mut last_changed = None;
                for line in block.split('\n') {
                    let (content, cr) = match line.strip_suffix('\r') {
                        Some(content) => (content, "\r"),
                        None => (line, ""),
                    };
                    let mut changed = String::new();
                    let mut last = 0;
                    for caps in regex.captures_iter(content) {
                        let m = caps.get(0).expect("a match");
                        changed.push_str(&content[last..m.start()]);
                        changed.push_str(&ex::expand_replacement(&replacement, &caps));
                        last = m.end();
                        if !global {
                            break;
                        }
                    }
                    let added = if regex.is_match(content) {
                        changed.push_str(&content[last..]);
                        let added = changed.matches('\n').count();
                        last_changed = Some(line_index + added);
                        out.push(format!("{changed}{cr}"));
                        added
                    } else {
                        out.push(line.to_string());
                        0
                    };
                    line_index += 1 + added;
                }
                match last_changed {
                    Some(n) => {
                        w.text.replace_range(start..end, &out.join("\n"));
                        let at = start_of_line(&w.text, lines.first + n);
                        w.cursor = first_non_blank(&w.text, at);
                    }
                    None => self.message = Some(format!("Pattern not found: {pattern}")),
                }
            }
        }
    }
}

// MARK: Parsing helpers

fn is_register(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '"' | '-' | '+' | '*' | '_' | '.' | ':' | '/')
}

/// A count at `i`, and where the keys after it start.
fn parse_count(keys: &[Key], mut i: usize) -> (Option<usize>, usize) {
    let mut count: Option<usize> = None;
    while let Some(Key::Char(c)) = keys.get(i) {
        let Some(d) = c.to_digit(10) else { break };
        if d == 0 && count.is_none() {
            break;
        }
        count = Some((count.unwrap_or(0) * 10 + d as usize).min(MAX_COUNT));
        i += 1;
    }
    (count, i)
}

enum Step<T> {
    More,
    Bad,
    Done(T),
}

fn parse_motion(keys: &[Key], i: usize) -> Step<Motion> {
    let Some(&key) = keys.get(i) else {
        return Step::More;
    };
    let one = |m| Step::Done(m);
    match key {
        Key::Char('h') | Key::Left | Key::Backspace => one(Motion::Left),
        Key::Char('l') | Key::Right | Key::Char(' ') => one(Motion::Right),
        Key::Char('k') | Key::Up => one(Motion::Up),
        Key::Char('j') | Key::Down => one(Motion::Down),
        Key::Char('w') => one(Motion::WordStart(false)),
        Key::Char('W') => one(Motion::WordStart(true)),
        Key::Char('b') => one(Motion::WordBack(false)),
        Key::Char('B') => one(Motion::WordBack(true)),
        Key::Char('e') => one(Motion::WordEnd(false)),
        Key::Char('E') => one(Motion::WordEnd(true)),
        Key::Char('0') | Key::Home => one(Motion::LineStart),
        Key::Char('^') => one(Motion::FirstNonBlank),
        Key::Char('$') | Key::End => one(Motion::LineEnd),
        Key::Char('+') | Key::Enter => one(Motion::NextLine),
        Key::Char('-') => one(Motion::PrevLine),
        Key::Char('_') => one(Motion::CurrentLine),
        Key::Char('|') => one(Motion::Column),
        Key::Char('G') => one(Motion::FileEnd),
        Key::Char(';') => one(Motion::RepeatFind(false)),
        Key::Char(',') => one(Motion::RepeatFind(true)),
        Key::Char('%') => one(Motion::MatchPair),
        Key::Char('}') => one(Motion::ParagraphForward),
        Key::Char('{') => one(Motion::ParagraphBack),
        Key::Char('H') => one(Motion::ScreenTop),
        Key::Char('M') => one(Motion::ScreenMiddle),
        Key::Char('L') => one(Motion::ScreenBottom),
        Key::Char('n') => one(Motion::SearchNext(false)),
        Key::Char('N') => one(Motion::SearchNext(true)),
        Key::Char('*') => one(Motion::SearchWord(true)),
        Key::Char('#') => one(Motion::SearchWord(false)),
        Key::Char('/') => one(Motion::SearchPrompt(true)),
        Key::Char('?') => one(Motion::SearchPrompt(false)),
        Key::Char(c @ ('f' | 't' | 'F' | 'T')) => match keys.get(i + 1) {
            None => Step::More,
            Some(Key::Char(ch)) => Step::Done(Motion::Find {
                ch: *ch,
                forward: c.is_lowercase(),
                till: c.eq_ignore_ascii_case(&'t'),
            }),
            Some(Key::Tab) => Step::Done(Motion::Find {
                ch: '\t',
                forward: c.is_lowercase(),
                till: c.eq_ignore_ascii_case(&'t'),
            }),
            _ => Step::Bad,
        },
        Key::Char('g') => match keys.get(i + 1) {
            None => Step::More,
            Some(Key::Char('g')) => Step::Done(Motion::FileStart),
            Some(Key::Char('e')) => Step::Done(Motion::WordEndBack(false)),
            Some(Key::Char('E')) => Step::Done(Motion::WordEndBack(true)),
            Some(Key::Char('_')) => Step::Done(Motion::CurrentLine),
            _ => Step::Bad,
        },
        _ => Step::Bad,
    }
}

fn parse_object(key: Option<Key>, around: bool) -> Step<TextObject> {
    let Some(Key::Char(c)) = key else {
        return if key.is_none() { Step::More } else { Step::Bad };
    };
    let object = match c {
        'w' => Object::Word(false),
        'W' => Object::Word(true),
        'p' => Object::Paragraph,
        '"' | '\'' | '`' => Object::Quote(c),
        '(' | ')' | 'b' => Object::Bracket('(', ')'),
        '{' | '}' | 'B' => Object::Bracket('{', '}'),
        '[' | ']' => Object::Bracket('[', ']'),
        '<' | '>' => Object::Bracket('<', '>'),
        _ => return Step::Bad,
    };
    Step::Done(TextObject { object, around })
}

/// Whether `.` repeats the command (`d/foo` is recorded when its search
/// line is done).
fn is_change(kind: &Kind) -> bool {
    match kind {
        Kind::Move(_) | Kind::Select(_) => false,
        Kind::Operate(_, Target::Motion(Motion::SearchPrompt(_))) => false,
        Kind::Operate(op, _) | Kind::VisualOperate { op, .. } => *op != Op::Yank,
        Kind::Do(act) => matches!(
            act,
            Act::DeleteChar
                | Act::DeleteCharBefore
                | Act::Substitute
                | Act::SubstituteLine
                | Act::DeleteToEnd
                | Act::ChangeToEnd
                | Act::ReplaceChar(_)
                | Act::ReplaceNewline
                | Act::Join
                | Act::Put { .. }
                | Act::Insert(_)
                | Act::ToggleCaseChar
        ),
    }
}

// MARK: Text helpers

/// The whole lines from `first`'s to `last`'s.
fn lines_span(t: &str, first: usize, last: usize) -> Span {
    let start = line_start(t, first.min(t.len()));
    let end = next_line_start(t, last.max(first).min(t.len())).unwrap_or(t.len());
    Span {
        start,
        end,
        linewise: true,
    }
}

/// The text's line break, for new lines (`o`, `O`).
fn line_ending(t: &str) -> &'static str {
    if t.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn toggle_case(c: char) -> String {
    if c.is_lowercase() {
        c.to_uppercase().collect()
    } else if c.is_uppercase() {
        c.to_lowercase().collect()
    } else {
        c.to_string()
    }
}

/// One level less indentation: a leading tab, or up to a tab's width of
/// spaces.
fn outdent(line: &str, tab_size: usize) -> String {
    if let Some(rest) = line.strip_prefix('\t') {
        return rest.to_string();
    }
    let spaces = line.len() - line.trim_start_matches(' ').len();
    line[spaces.min(tab_size.max(1))..].to_string()
}

/// The word `*` and `#` look for: under the cursor, or the next on the line.
fn word_for_search(t: &str, from: usize) -> Option<(&str, usize)> {
    let end = line_end(t, from);
    let start = t[from..end]
        .char_indices()
        .find(|(_, c)| class(*c, false) == 1)
        .map(|(i, _)| from + i)?;
    let mut s = start;
    while s > line_start(t, from) && class(char_at(t, prev(t, s))?, false) == 1 {
        s = prev(t, s);
    }
    let mut e = start;
    while e < end && class(char_at(t, e)?, false) == 1 {
        e = next(t, e);
    }
    Some((&t[s..e], s))
}

/// A search pattern as a regex: `\<` and `\>` are word boundaries, `\c`
/// ignores case, and a pattern that isn't a valid regex is plain text.
fn compile(pattern: &str, ignore_case: bool) -> Option<Regex> {
    let ignore_case = ignore_case || pattern.contains("\\c");
    let translated = pattern
        .replace("\\c", "")
        .replace("\\<", "\\b")
        .replace("\\>", "\\b");
    RegexBuilder::new(&translated)
        .case_insensitive(ignore_case)
        .multi_line(true)
        .build()
        .or_else(|_| {
            RegexBuilder::new(&regex::escape(pattern))
                .case_insensitive(ignore_case)
                .build()
        })
        .ok()
}

fn compile_cached(
    cache: &mut Option<(String, bool, Regex)>,
    pattern: &str,
    ignore_case: bool,
) -> Option<Regex> {
    if let Some((p, ic, regex)) = cache {
        if p == pattern && *ic == ignore_case {
            return Some(regex.clone());
        }
    }
    let regex = compile(pattern, ignore_case)?;
    *cache = Some((pattern.to_string(), ignore_case, regex.clone()));
    Some(regex)
}

/// What Insert mode typed, from the text when it started (`before`, with the
/// cursor at `start`) and the text now.
fn typed_text(before: &str, after: &str, start: usize) -> Typed {
    let start = start.min(before.len());
    let mut prefix = before
        .bytes()
        .zip(after.bytes())
        .take_while(|(a, b)| a == b)
        .count()
        .min(start);
    while !before.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = before
        .bytes()
        .rev()
        .zip(after.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(before.len() - start)
        .min(after.len() - prefix);
    while !before.is_char_boundary(before.len() - suffix)
        || !after.is_char_boundary(after.len() - suffix)
    {
        suffix -= 1;
    }
    Typed {
        delete_before: before[prefix..start].chars().count(),
        delete_after: before[start..before.len() - suffix].chars().count(),
        text: after[prefix..after.len() - suffix].to_string(),
    }
}

/// The one replacement that turns `old` into `new`, if they differ.
fn diff(old: &str, new: &str) -> Option<(Range<usize>, String)> {
    if old == new {
        return None;
    }
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = old[prefix..]
        .bytes()
        .rev()
        .zip(new[prefix..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    Some((
        prefix..old.len() - suffix,
        new[prefix..new.len() - suffix].to_string(),
    ))
}
