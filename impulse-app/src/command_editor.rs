//! The terminal input bar's editor (the Swift `CommandEditor` and its
//! `CommandTextView`): multi-line command text where Return runs the
//! command (Shift or Alt with Return adds a line), colored by the shell
//! parser, with a suggestion shown as ghost text after the cursor and a
//! dashed underline under a command the shell can't run. Input methods and
//! dead keys go through GPUI's text input. Keys the input bar acts on
//! (history, completion, accepting the suggestion) are handed to the owner
//! as `CommandEditorEvent::Key` first.
//!
//! The find bars use it too, single-line and without shell colors.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    actions, canvas, div, fill, point, prelude::*, px, size, App, Bounds, ClipboardItem, Context,
    ElementInputHandler, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font, Hsla,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point,
    SharedString, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine,
};
use impulse_kit::shell::lookup::CommandLookup;
use impulse_kit::shell_parser::{parse_shell_input, ShellTokenRole};
use impulse_kit::theme::ResolvedTheme;

use crate::terminal_view::rgba;

/// Lines shown before the editor stops growing.
const MAX_VISIBLE_LINES: usize = 8;

actions!(command_editor, [Copy, Cut, Paste, SelectAll, Undo, Redo]);

pub const CONTEXT: &str = "CommandEditor";

pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    // Tab completes (the window would move focus with it).
    cx.bind_keys([
        KeyBinding::new("tab", gpui_kit::NoAction, c),
        KeyBinding::new("shift-tab", gpui_kit::NoAction, c),
    ]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-c", Copy, c),
            KeyBinding::new("cmd-x", Cut, c),
            KeyBinding::new("cmd-v", Paste, c),
            KeyBinding::new("cmd-a", SelectAll, c),
            KeyBinding::new("cmd-z", Undo, c),
            KeyBinding::new("cmd-shift-z", Redo, c),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-c", Copy, c),
            KeyBinding::new("ctrl-shift-x", Cut, c),
            KeyBinding::new("ctrl-shift-v", Paste, c),
            KeyBinding::new("ctrl-v", Paste, c),
            KeyBinding::new("ctrl-shift-a", SelectAll, c),
            KeyBinding::new("ctrl-z", Undo, c),
            KeyBinding::new("ctrl-shift-z", Redo, c),
        ]);
    }
}

/// Keys the editor hands to its owner instead of handling them itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKey {
    /// Up on the first line, Down on the last (history, the menu).
    Up,
    Down,
    Tab,
    /// Right at the end of the text (accept the suggestion).
    Right,
    /// Alt+Right (Ctrl+Right on Linux) at the end (the suggestion's next word).
    WordRight,
    Escape,
    ControlC,
    ControlR,
    /// Cmd+Up (Ctrl+Up on Linux): select the newest command block.
    SelectBlock,
    /// Alt+Cmd+Return (Ctrl+Alt+Return on Linux) where Return adds a line
    /// (the agent composer's "paste without sending").
    AlternateSubmit,
}

pub enum CommandEditorEvent {
    /// The text changed (not mid-composition).
    Changed,
    /// Return (Shift+Return in a single-line editor reports `shift`).
    Submit {
        shift: bool,
    },
    Key(EditorKey),
    /// Copy with nothing selected here (the terminal copies its own
    /// selection instead).
    CopyNothing,
    /// Any click in the editor.
    Clicked,
    /// Paste with no text on the clipboard (files or an image): the owner
    /// may insert their paths.
    PasteOther,
}

impl EventEmitter<CommandEditorEvent> for CommandEditor {}

/// The colors the editor draws with (`CommandEditorColors`).
#[derive(Clone, PartialEq)]
pub struct EditorColors {
    pub text: Hsla,
    pub ghost: Hsla,
    pub caret: Hsla,
    pub selection: Hsla,
    pub command: Hsla,
    pub flag: Hsla,
    pub string: Hsla,
    pub variable: Hsla,
    pub operator: Hsla,
    pub error: Hsla,
}

impl EditorColors {
    pub fn new(theme: &ResolvedTheme) -> Self {
        let c = |hex: &str| -> Hsla { rgba(impulse_kit::chrome::Rgba::from_hex(hex)).into() };
        let mut selection = c(&theme.accent);
        selection.a = 0.3;
        EditorColors {
            text: c(&theme.fg),
            ghost: c(&theme.fg_comment),
            caret: c(&theme.accent),
            selection,
            command: c(&theme.blue),
            flag: c(&theme.yellow),
            string: c(&theme.green),
            variable: c(&theme.magenta),
            operator: c(&theme.cyan),
            error: c(&theme.red),
        }
    }
}

/// What the last paint laid out, for mouse hits and caret movement.
#[derive(Default)]
struct Layout {
    /// Each logical line: its shaped (wrapped) text, its first byte in the
    /// display text, and its top in the editor.
    lines: Vec<(WrappedLine, usize, Pixels)>,
    origin: Point<Pixels>,
    line_height: Pixels,
}

pub struct CommandEditor {
    focus: FocusHandle,
    text: String,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    pub placeholder: SharedString,
    /// The full suggested line; the part after the text is drawn dimmed.
    pub suggestion: Option<String>,
    /// Color the text as a shell command.
    pub shell_syntax: bool,
    /// Draw bullets instead of the text (password input).
    pub masked: bool,
    /// Return submits and pasted newlines become spaces (find fields).
    single_line: bool,
    /// Return submits (the input bar). Otherwise it adds a line and
    /// Cmd+Return (Ctrl+Return on Linux) submits (the agent composer).
    pub return_submits: bool,
    /// What the shell can run, with the folder relative commands start in.
    pub lookup: Option<(CommandLookup, Option<String>)>,
    pub colors: EditorColors,
    pub font: Font,
    pub font_size: Pixels,
    undo: Vec<(String, Range<usize>)>,
    redo: Vec<(String, Range<usize>)>,
    /// The last edit was typing, which undoes as one step with the typing
    /// before it.
    typing: bool,
    selecting: bool,
    caret_on: bool,
    /// Whether it had the focus when last drawn: only then does the caret
    /// blink (each blink redraws the window).
    shown_focused: bool,
    layout: Rc<RefCell<Layout>>,
    /// The width the text wrapped to in the last paint.
    width: Rc<Cell<Pixels>>,
    _blink: gpui_kit::Task<()>,
}

impl CommandEditor {
    pub fn new(single_line: bool, theme: &ResolvedTheme, cx: &mut Context<Self>) -> Self {
        let blink = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(530))
                .await;
            if this
                .update(cx, |editor, cx| {
                    let on = !editor.shown_focused || !editor.caret_on;
                    if on != editor.caret_on {
                        editor.caret_on = on;
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        });
        CommandEditor {
            focus: cx.focus_handle(),
            text: String::new(),
            selected: 0..0,
            reversed: false,
            marked: None,
            placeholder: SharedString::default(),
            suggestion: None,
            shell_syntax: !single_line,
            masked: false,
            single_line,
            return_submits: true,
            lookup: None,
            colors: EditorColors::new(theme),
            font: gpui_kit::font("JetBrains Mono"),
            font_size: px(13.),
            undo: Vec::new(),
            redo: Vec::new(),
            typing: false,
            selecting: false,
            caret_on: true,
            shown_focused: false,
            layout: Rc::default(),
            width: Rc::new(Cell::new(px(0.))),
            _blink: blink,
        }
    }

    /// Insert text at the cursor, as typing would.
    pub fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.insert(text, cx);
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Empty, with nothing to undo or redo: a password typed in it (or a
    /// command typed before one was asked for) must not come back.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.text.clear();
        self.selected = 0..0;
        self.marked = None;
        self.suggestion = None;
        self.undo.clear();
        self.redo.clear();
        self.typing = false;
        cx.notify();
    }

    /// Replace the text (the owner's edits: history, completion), with the
    /// cursor at the end. Undoable. Emits no `Changed`.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let text = text.into();
        if text == self.text {
            return;
        }
        self.push_undo(false);
        self.text = text;
        self.selected = self.text.len()..self.text.len();
        self.marked = None;
        self.show_caret();
        cx.notify();
    }

    /// The cursor is at the end of the text with nothing selected.
    pub fn is_at_end(&self) -> bool {
        self.selected.is_empty() && self.selected.start == self.text.len()
    }

    fn line_height(&self) -> Pixels {
        (self.font_size * 1.32).ceil()
    }

    /// The editor's height for its text: one line at least, at most
    /// `MAX_VISIBLE_LINES`.
    pub fn height(&self, window: &mut Window) -> Pixels {
        let lines = self.visual_lines(window).clamp(1, MAX_VISIBLE_LINES);
        self.line_height() * lines as f32
    }

    fn visual_lines(&self, window: &mut Window) -> usize {
        let width = self.width.get();
        if self.text.is_empty() || width <= px(0.) {
            return self.text.split('\n').count();
        }
        let display = self.display_text();
        let run = self.plain_run(display.len(), self.colors.text);
        window
            .text_system()
            .shape_text(display.into(), self.font_size, &[run], Some(width), None)
            .map(|lines| {
                lines
                    .iter()
                    .map(|l| l.wrap_boundaries().len() + 1)
                    .sum::<usize>()
            })
            .unwrap_or(1)
    }

    fn plain_run(&self, len: usize, color: Hsla) -> TextRun {
        TextRun {
            len,
            font: self.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    // MARK: Text and offsets

    /// What's drawn: the text, or a bullet per character when masked.
    fn display_text(&self) -> String {
        if self.masked {
            "•".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        }
    }

    fn text_to_display(&self, offset: usize) -> usize {
        if self.masked {
            self.text[..offset].chars().count() * '•'.len_utf8()
        } else {
            offset
        }
    }

    fn display_to_text(&self, offset: usize) -> usize {
        if self.masked {
            let chars = offset / '•'.len_utf8();
            self.text
                .char_indices()
                .nth(chars)
                .map_or(self.text.len(), |(i, _)| i)
        } else {
            offset.min(self.text.len())
        }
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.text[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text[offset..]
            .chars()
            .next()
            .map_or(self.text.len(), |c| offset + c.len_utf8())
    }

    fn previous_word(&self, offset: usize) -> usize {
        let before: Vec<(usize, char)> = self.text[..offset].char_indices().collect();
        let mut i = before.len();
        while i > 0 && before[i - 1].1.is_whitespace() {
            i -= 1;
        }
        if i > 0 {
            let word = is_word(before[i - 1].1);
            while i > 0 && !before[i - 1].1.is_whitespace() && is_word(before[i - 1].1) == word {
                i -= 1;
            }
        }
        before.get(i).map_or(offset, |(at, _)| *at).min(offset)
    }

    fn next_word(&self, offset: usize) -> usize {
        let mut chars = self.text[offset..].char_indices().peekable();
        while chars.next_if(|(_, c)| c.is_whitespace()).is_some() {}
        if let Some(&(_, first)) = chars.peek() {
            let word = is_word(first);
            while chars
                .next_if(|(_, c)| !c.is_whitespace() && is_word(*c) == word)
                .is_some()
            {}
        }
        chars.peek().map_or(self.text.len(), |(i, _)| offset + i)
    }

    fn line_start(&self, offset: usize) -> usize {
        self.text[..offset].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self, offset: usize) -> usize {
        self.text[offset..]
            .find('\n')
            .map_or(self.text.len(), |i| offset + i)
    }

    fn is_on_first_line(&self) -> bool {
        !self.text[..self.selected.start].contains('\n')
    }

    fn is_on_last_line(&self) -> bool {
        !self.text[self.selected.end..].contains('\n')
    }

    // MARK: Editing

    fn push_undo(&mut self, typing: bool) {
        if !(typing && self.typing) {
            self.undo.push((self.text.clone(), self.selected.clone()));
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.typing = typing;
    }

    fn show_caret(&mut self) {
        self.caret_on = true;
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected = offset..offset;
        self.reversed = false;
        self.typing = false;
        self.show_caret();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        self.typing = false;
        self.show_caret();
        cx.notify();
    }

    /// Replace `range` with `new`, leaving the cursor after it.
    fn replace(&mut self, range: Range<usize>, new: &str, typing: bool, cx: &mut Context<Self>) {
        self.push_undo(typing);
        self.text.replace_range(range.clone(), new);
        let at = range.start + new.len();
        self.selected = at..at;
        self.reversed = false;
        self.marked = None;
        self.show_caret();
        cx.emit(CommandEditorEvent::Changed);
        cx.notify();
    }

    /// Delete the selection, or from the cursor to `to`.
    fn delete_to(&mut self, to: usize, cx: &mut Context<Self>) {
        let range = if self.selected.is_empty() {
            let at = self.cursor();
            at.min(to)..at.max(to)
        } else {
            self.selected.clone()
        };
        if !range.is_empty() {
            self.replace(range, "", false, cx);
        }
    }

    fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = if self.single_line {
            text.replace(['\r', '\n'], " ")
        } else {
            text.replace("\r\n", "\n").replace('\r', "\n")
        };
        let range = self.marked.clone().unwrap_or(self.selected.clone());
        self.replace(range, &text, false, cx);
    }

    fn selected_text(&self) -> Option<String> {
        (!self.selected.is_empty() && !self.masked)
            .then(|| self.text[self.selected.clone()].to_string())
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        match self.selected_text() {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            None => cx.emit(CommandEditorEvent::CopyNothing),
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.replace(self.selected.clone(), "", false, cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard() {
            match item.text() {
                Some(text) => self.insert(&text, cx),
                None => cx.emit(CommandEditorEvent::PasteOther),
            }
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = 0..self.text.len();
        self.reversed = false;
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((text, selected)) = self.undo.pop() {
            self.redo.push((
                std::mem::replace(&mut self.text, text),
                self.selected.clone(),
            ));
            self.selected = selected;
            self.typing = false;
            cx.emit(CommandEditorEvent::Changed);
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((text, selected)) = self.redo.pop() {
            self.undo.push((
                std::mem::replace(&mut self.text, text),
                self.selected.clone(),
            ));
            self.selected = selected;
            self.typing = false;
            cx.emit(CommandEditorEvent::Changed);
            cx.notify();
        }
    }

    // MARK: Keys

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = &k.modifiers;
        let mac = cfg!(target_os = "macos");
        // The word modifier: Alt on the Mac, Ctrl elsewhere.
        let word = if mac { m.alt } else { m.control };
        let secondary = if mac { m.platform } else { m.control };
        let plain = !m.control && !m.alt && !m.platform;
        let ctrl_only = m.control && !m.alt && !m.platform && !m.shift;
        let handled = match k.key.as_str() {
            "enter" if self.single_line => {
                cx.emit(CommandEditorEvent::Submit { shift: m.shift });
                true
            }
            "enter" if !self.return_submits && secondary && m.alt => {
                cx.emit(CommandEditorEvent::Key(EditorKey::AlternateSubmit));
                true
            }
            "enter" if !self.return_submits && secondary => {
                cx.emit(CommandEditorEvent::Submit { shift: false });
                true
            }
            "enter" if m.shift || m.alt || !self.return_submits => {
                self.insert("\n", cx);
                true
            }
            "enter" => {
                cx.emit(CommandEditorEvent::Submit { shift: false });
                true
            }
            "tab" if !m.control => {
                cx.emit(CommandEditorEvent::Key(EditorKey::Tab));
                true
            }
            "escape" => {
                cx.emit(CommandEditorEvent::Key(EditorKey::Escape));
                true
            }
            "backspace" => {
                let at = self.cursor();
                let to = if mac && m.platform {
                    self.line_start(at)
                } else if word {
                    self.previous_word(at)
                } else {
                    self.previous_boundary(at)
                };
                self.delete_to(to, cx);
                true
            }
            "delete" => {
                let at = self.cursor();
                let to = if word {
                    self.next_word(at)
                } else {
                    self.next_boundary(at)
                };
                self.delete_to(to, cx);
                true
            }
            "left" | "right" | "home" | "end" => {
                let right = matches!(k.key.as_str(), "right" | "end");
                let at = self.cursor();
                if right && !m.shift && self.is_at_end() && (plain || (word && !secondary)) {
                    cx.emit(CommandEditorEvent::Key(if word {
                        EditorKey::WordRight
                    } else {
                        EditorKey::Right
                    }));
                    return cx.stop_propagation();
                }
                let line = matches!(k.key.as_str(), "home" | "end") || (mac && m.platform);
                let to = match (right, line, word) {
                    (false, true, _) => self.line_start(at),
                    (true, true, _) => self.line_end(at),
                    (false, _, true) => self.previous_word(at),
                    (true, _, true) => self.next_word(at),
                    (false, _, _) if !m.shift && !self.selected.is_empty() => self.selected.start,
                    (true, _, _) if !m.shift && !self.selected.is_empty() => self.selected.end,
                    (false, _, _) => self.previous_boundary(at),
                    (true, _, _) => self.next_boundary(at),
                };
                if m.shift {
                    self.select_to(to, cx);
                } else {
                    self.move_to(to, cx);
                }
                true
            }
            "up" if secondary && !m.shift && !m.alt => {
                cx.emit(CommandEditorEvent::Key(EditorKey::SelectBlock));
                true
            }
            "up" | "down" => {
                let up = k.key == "up";
                let edge = if up {
                    self.is_on_first_line()
                } else {
                    self.is_on_last_line()
                };
                if edge && !m.shift {
                    cx.emit(CommandEditorEvent::Key(if up {
                        EditorKey::Up
                    } else {
                        EditorKey::Down
                    }));
                } else {
                    let to = self.vertical(self.cursor(), up);
                    if m.shift {
                        self.select_to(to, cx);
                    } else {
                        self.move_to(to, cx);
                    }
                }
                true
            }
            // The shell's line-editing keys, as NSTextView has them.
            "c" if ctrl_only => {
                cx.emit(CommandEditorEvent::Key(EditorKey::ControlC));
                true
            }
            "r" if ctrl_only => {
                cx.emit(CommandEditorEvent::Key(EditorKey::ControlR));
                true
            }
            "a" if ctrl_only => {
                self.move_to(self.line_start(self.cursor()), cx);
                true
            }
            "e" if ctrl_only => {
                self.move_to(self.line_end(self.cursor()), cx);
                true
            }
            "k" if ctrl_only => {
                let at = self.cursor();
                let end = self.line_end(at);
                let end = if end == at {
                    self.next_boundary(at)
                } else {
                    end
                };
                self.delete_to(end, cx);
                true
            }
            "u" if ctrl_only => {
                let at = self.cursor();
                self.delete_to(self.line_start(at), cx);
                true
            }
            "w" if ctrl_only => {
                let at = self.cursor();
                self.delete_to(self.previous_word(at), cx);
                true
            }
            "d" if ctrl_only => {
                let at = self.cursor();
                self.delete_to(self.next_boundary(at), cx);
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    /// The offset a line above or below `offset`, at the same x.
    fn vertical(&self, offset: usize, up: bool) -> usize {
        let layout = self.layout.borrow();
        let Some(position) = self.position_of(&layout, offset) else {
            return if up { 0 } else { self.text.len() };
        };
        let y = if up {
            position.y - layout.line_height / 2.
        } else {
            position.y + layout.line_height * 1.5
        };
        self.index_at(&layout, point(position.x, y))
    }

    // MARK: Layout queries

    /// Where `offset` is drawn, relative to the editor's origin.
    fn position_of(&self, layout: &Layout, offset: usize) -> Option<Point<Pixels>> {
        position(layout, self.text_to_display(offset))
    }

    /// The offset closest to `position` (relative to the editor's origin).
    fn index_at(&self, layout: &Layout, position: Point<Pixels>) -> usize {
        if layout.lines.is_empty() {
            return self.text.len();
        }
        if position.y < px(0.) {
            return 0;
        }
        for (line, start, top) in &layout.lines {
            let height = layout.line_height * (line.wrap_boundaries().len() + 1) as f32;
            if position.y < *top + height {
                let local = point(position.x.max(px(0.)), position.y - *top);
                let index = match line.closest_index_for_position(local, layout.line_height) {
                    Ok(i) | Err(i) => i,
                };
                return self.display_to_text(start + index);
            }
        }
        self.text.len()
    }

    // MARK: Mouse

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CommandEditorEvent::Clicked);
        window.focus(&self.focus, cx);
        let index = {
            let layout = self.layout.borrow();
            self.index_at(&layout, event.position - layout.origin)
        };
        match event.click_count {
            2 => {
                let start = self.previous_word(self.next_boundary(index).min(self.text.len()));
                let end = self.next_word(start);
                self.selected = start..end;
                self.reversed = false;
                cx.notify();
            }
            n if n >= 3 => {
                self.selected = self.line_start(index)..self.line_end(index);
                self.reversed = false;
                cx.notify();
            }
            _ if event.modifiers.shift => self.select_to(index, cx),
            _ => {
                self.move_to(index, cx);
                self.selecting = true;
            }
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return;
        }
        let index = {
            let layout = self.layout.borrow();
            self.index_at(&layout, event.position - layout.origin)
        };
        self.select_to(index, cx);
    }

    // MARK: Highlighting

    /// The runs for the display text: shell colors, and a dashed underline
    /// under a command the shell can't run once the cursor has left it.
    fn runs(&self, display_len: usize) -> Vec<TextRun> {
        if self.masked || !self.shell_syntax || self.text.is_empty() || self.marked.is_some() {
            let mut run = self.plain_run(display_len, self.colors.text);
            if let Some(marked) = &self.marked {
                // The composition, underlined (one run per part).
                let (a, b) = (
                    self.text_to_display(marked.start),
                    self.text_to_display(marked.end),
                );
                let mut runs = Vec::new();
                if a > 0 {
                    runs.push(self.plain_run(a, self.colors.text));
                }
                let mut composing = self.plain_run(b - a, self.colors.text);
                composing.underline = Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(self.colors.text),
                    wavy: false,
                });
                runs.push(composing);
                if display_len > b {
                    runs.push(self.plain_run(display_len - b, self.colors.text));
                }
                return runs;
            }
            run.len = display_len;
            return vec![run];
        }
        let text = &self.text;
        let mut colors: Vec<Hsla> = vec![self.colors.text; text.len()];
        let mut errors = vec![false; text.len()];
        let parsed = parse_shell_input(text, text.len());
        let cursor = self.cursor();
        for token in parsed.tokens.iter().chain(&parsed.assignments) {
            let span = token.span;
            if span.start >= span.end || span.end > text.len() {
                continue;
            }
            let color = match token.role {
                ShellTokenRole::Command => Some(self.colors.command),
                ShellTokenRole::Assignment => Some(self.colors.variable),
                ShellTokenRole::PipelineSeparator
                | ShellTokenRole::ControlOperator
                | ShellTokenRole::RedirectionOperator => Some(self.colors.operator),
                ShellTokenRole::RedirectionTarget | ShellTokenRole::Argument => {
                    if token.quoted {
                        Some(self.colors.string)
                    } else if token.text.starts_with('-') {
                        Some(self.colors.flag)
                    } else if token.text.starts_with('$') {
                        Some(self.colors.variable)
                    } else {
                        None
                    }
                }
            };
            if let Some(color) = color {
                colors[span.start..span.end].fill(color);
            }
            if token.role == ShellTokenRole::Command
                && !token.quoted
                && (cursor < span.start || cursor > span.end)
            {
                if let Some((lookup, cwd)) = &self.lookup {
                    if lookup.is_known(&token.text, cwd.as_deref()) == Some(false) {
                        errors[span.start..span.end].fill(true);
                    }
                }
            }
        }
        for redirect in &parsed.redirects {
            let span = redirect.operator_span;
            if span.start < span.end && span.end <= text.len() {
                colors[span.start..span.end].fill(self.colors.operator);
            }
        }
        // Runs of one style, split only at character boundaries.
        let mut runs: Vec<TextRun> = Vec::new();
        let mut style: Option<(Hsla, bool)> = None;
        for (i, c) in text.char_indices() {
            let current = (colors[i], errors[i]);
            match (&mut runs.last_mut(), style) {
                (Some(run), Some(s)) if s == current => run.len += c.len_utf8(),
                _ => {
                    let mut run = self.plain_run(c.len_utf8(), current.0);
                    if current.1 {
                        run.underline = Some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(self.colors.error),
                            wavy: false,
                        });
                    }
                    runs.push(run);
                    style = Some(current);
                }
            }
        }
        runs
    }

    /// The suggestion's first line past the text, when the cursor is at
    /// the end.
    fn ghost(&self) -> Option<String> {
        let suggestion = self.suggestion.as_ref()?;
        if self.text.is_empty()
            || self.masked
            || self.marked.is_some()
            || !self.is_at_end()
            || !suggestion.starts_with(&self.text)
            || suggestion.len() <= self.text.len()
        {
            return None;
        }
        let rest = &suggestion[self.text.len()..];
        Some(rest.split('\n').next().unwrap_or(rest).to_string())
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

impl CommandEditor {
    fn offset_from_utf16(&self, utf16: usize) -> usize {
        let mut count = 0;
        for (i, c) in self.text.char_indices() {
            if count >= utf16 {
                return i;
            }
            count += c.len_utf16();
        }
        self.text.len()
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        utf16_len(&self.text[..offset.min(self.text.len())])
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

impl EntityInputHandler for CommandEditor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range);
        adjusted.replace(self.range_to_utf16(&range));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            // What's left of the composition is now plain text.
            cx.emit(CommandEditorEvent::Changed);
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // An unbound Ctrl key can arrive as its control character.
        let text: String = text
            .chars()
            .filter(|c| !impulse_kit::shell::quoting::is_control(*c) || *c == '\n')
            .collect();
        let range = range
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        if text.is_empty() && range.is_empty() {
            self.marked = None;
            return;
        }
        let typing = range.is_empty() && !text.contains(char::is_whitespace);
        self.replace(range, &text, typing, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.text.replace_range(range.clone(), new_text);
        self.marked = (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        let selected = new_selected
            .map(|r| {
                let local = new_text;
                let to_offset = |u: usize| {
                    let mut count = 0;
                    for (i, c) in local.char_indices() {
                        if count >= u {
                            return i;
                        }
                        count += c.len_utf16();
                    }
                    local.len()
                };
                range.start + to_offset(r.start)..range.start + to_offset(r.end)
            })
            .unwrap_or(range.start + new_text.len()..range.start + new_text.len());
        self.selected = selected;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.borrow();
        let start = self.offset_from_utf16(range.start);
        let position = self.position_of(&layout, start)?;
        Some(Bounds::new(
            layout.origin + position,
            size(px(2.), layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.layout.borrow();
        let index = self.index_at(&layout, position - layout.origin);
        Some(self.offset_to_utf16(index))
    }
}

impl Focusable for CommandEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Everything a frame paints.
struct Frame {
    display: String,
    runs: Vec<TextRun>,
    placeholder: Option<SharedString>,
    ghost: Option<String>,
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    colors: EditorColors,
    font: Font,
    font_size: Pixels,
    line_height: Pixels,
}

impl Render for CommandEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        self.shown_focused = focused;
        let display = self.display_text();
        let runs = self.runs(display.len());
        let frame = Frame {
            placeholder: (self.text.is_empty() && self.marked.is_none())
                .then(|| self.placeholder.clone()),
            ghost: self.ghost(),
            selection: (!self.selected.is_empty()).then(|| {
                self.text_to_display(self.selected.start)..self.text_to_display(self.selected.end)
            }),
            caret: (focused && self.caret_on && self.selected.is_empty())
                .then(|| self.text_to_display(self.cursor())),
            display,
            runs,
            colors: self.colors.clone(),
            font: self.font.clone(),
            font_size: self.font_size,
            line_height: self.line_height(),
        };
        let height = self.height(window);
        let entity = cx.entity();
        let layout = self.layout.clone();
        let width = self.width.clone();
        div()
            .id("command-editor")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|editor, _, _, _| editor.selecting = false),
            )
            .cursor_text()
            .w_full()
            .h(height)
            .overflow_hidden()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let focus = entity.read(cx).focus.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        let changed_width = width.get() != bounds.size.width;
                        width.set(bounds.size.width);
                        frame.paint(bounds, &layout, window, cx);
                        if changed_width {
                            // The height depends on how the text wraps.
                            entity.update(cx, |_, cx| cx.notify());
                        }
                    },
                )
                .size_full(),
            )
    }
}

impl Frame {
    fn paint(
        self,
        bounds: Bounds<Pixels>,
        layout: &Rc<RefCell<Layout>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let lh = self.line_height;
        let run = |len: usize, color: Hsla| TextRun {
            len,
            font: self.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        // Lay out the text first, to scroll the caret into view.
        let lines: Vec<WrappedLine> = if self.display.is_empty() {
            Vec::new()
        } else {
            window
                .text_system()
                .shape_text(
                    self.display.clone().into(),
                    self.font_size,
                    &self.runs,
                    Some(bounds.size.width),
                    None,
                )
                .map(|lines| lines.into_vec())
                .unwrap_or_default()
        };
        let mut placed = Vec::new();
        let (mut start, mut top) = (0usize, px(0.));
        for line in lines {
            let rows = line.wrap_boundaries().len() + 1;
            let len = line.len();
            placed.push((line, start, top));
            start += len + 1;
            top += lh * rows as f32;
        }
        let content_height = top;
        let mut new_layout = Layout {
            lines: placed,
            origin: bounds.origin,
            line_height: lh,
        };
        // The caret's line stays in view when the text is taller than the
        // editor.
        let caret_y = self
            .caret
            .or(self.selection.as_ref().map(|s| s.end))
            .and_then(|c| position(&new_layout, c))
            .map_or(px(0.), |p| p.y);
        let scroll = if content_height > bounds.size.height {
            (caret_y + lh - bounds.size.height)
                .max(px(0.))
                .min(content_height - bounds.size.height)
        } else {
            px(0.)
        };
        new_layout.origin = bounds.origin - point(px(0.), scroll);
        let origin = new_layout.origin;

        window.with_content_mask(Some(gpui_kit::ContentMask { bounds }), |window| {
            if let Some(placeholder) = &self.placeholder {
                let line = window.text_system().shape_line(
                    placeholder.clone(),
                    self.font_size,
                    &[run(placeholder.len(), self.colors.ghost)],
                    None,
                );
                let _ = line.paint(origin, lh, TextAlign::Left, None, window, cx);
            }
            // The selection, row by row.
            if let Some(selection) = &self.selection {
                for (line, start, top) in &new_layout.lines {
                    let end = start + line.len();
                    if selection.end < *start || selection.start > end {
                        continue;
                    }
                    let a = selection.start.max(*start) - start;
                    let b = selection.end.min(end) - start;
                    let (Some(pa), Some(pb)) = (
                        line.position_for_index(a, lh),
                        line.position_for_index(b, lh),
                    ) else {
                        continue;
                    };
                    let mut y = pa.y;
                    while y <= pb.y {
                        let x0 = if y == pa.y { pa.x } else { px(0.) };
                        let x1 = if y == pb.y { pb.x } else { bounds.size.width };
                        // A selected line end shows as a little space.
                        let x1 = if y == pb.y && selection.end > end {
                            x1 + px(6.)
                        } else {
                            x1
                        };
                        if x1 > x0 {
                            window.paint_quad(fill(
                                Bounds::new(origin + point(x0, *top + y), size(x1 - x0, lh)),
                                self.colors.selection,
                            ));
                        }
                        y += lh;
                    }
                }
            }
            for (line, _, top) in &new_layout.lines {
                let _ = line.paint(
                    origin + point(px(0.), *top),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            // The suggestion continues where the text ends.
            if let (Some(ghost), Some(end)) =
                (&self.ghost, position(&new_layout, self.display.len()))
            {
                let line = window.text_system().shape_line(
                    ghost.clone().into(),
                    self.font_size,
                    &[run(ghost.len(), self.colors.ghost)],
                    None,
                );
                let _ = line.paint(origin + end, lh, TextAlign::Left, None, window, cx);
            }
            if let Some(caret) = self.caret {
                let at = position(&new_layout, caret).unwrap_or_default();
                window.paint_quad(fill(
                    Bounds::new(origin + at, size(px(2.), lh)),
                    self.colors.caret,
                ));
            }
        });
        *layout.borrow_mut() = new_layout;
    }
}

/// Where display offset `offset` is drawn, relative to the layout origin.
fn position(layout: &Layout, offset: usize) -> Option<Point<Pixels>> {
    if layout.lines.is_empty() {
        return Some(point(px(0.), px(0.)));
    }
    let (line, start, top) = layout
        .lines
        .iter()
        .rev()
        .find(|(_, start, _)| *start <= offset)?;
    line.position_for_index(offset - start, layout.line_height)
        .map(|p| point(p.x, p.y + *top))
}
