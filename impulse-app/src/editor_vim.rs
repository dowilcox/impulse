//! Vim mode in the editor (`editor_vim_mode`; monaco-vim in the Swift app):
//! `impulse_kit::vim`'s state machine over GPUI Kit's editor. In every mode
//! but Insert, Vim takes the keys before the editor does: plain keys through
//! a capture listener (so they don't type), and the keys GPUI Kit binds
//! (Enter, Backspace, the arrows, Ctrl+R and the like) by unbinding them in
//! the `VimNormal` context. Escape stays the editor's action, seen on its
//! way in, so it still closes the completion menu. Edits, undo and the
//! selection are the editor's own. GPUI Kit only draws a thin caret, so the
//! character under the cursor is drawn inverted as Vim's block cursor.

use std::ops::Range;

use gpui_kit::component::input::{
    EditorState, Escape, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle, Redo,
    TextDecoration, TextDecorationCollection, Undo,
};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    div, point, prelude::*, px, AnyElement, App, ClipboardItem, Context, Entity, Focusable as _,
    HighlightStyle, KeyBinding, Keystroke, NoAction, Subscription, WeakEntity, Window,
};
use impulse_kit::vim::{Action, CloseSave, Context as VimContext, Key, Reveal, Vim};

use crate::editor_tab::EditorTab;

/// Present on an editor tab with Vim on.
const CONTEXT: &str = "Vim";
/// Present while Vim takes the keys (every mode but Insert).
const NORMAL: &str = "VimNormal";

/// Keys GPUI Kit's editor binds that Vim reads itself outside Insert mode.
const TAKEN: &[&str] = &[
    "enter",
    "shift-enter",
    "backspace",
    "shift-backspace",
    "delete",
    "tab",
    "shift-tab",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "ctrl-r",
    "ctrl-d",
    "ctrl-u",
    "ctrl-f",
    "ctrl-b",
    "ctrl-e",
    "ctrl-y",
    "ctrl-[",
];

/// Unbind GPUI Kit's (and `editor_keys`') bindings of the keys Vim reads.
/// Only the editor's own input (`mode == editor`), not the find field
/// inside it. Bound after `editor_keys`, so these rank above its keys.
pub fn bind_keys(cx: &mut App) {
    let normal = format!("{NORMAL} > Input && mode == editor");
    cx.bind_keys(
        TAKEN
            .iter()
            .map(|key| KeyBinding::new(key, NoAction, Some(normal.as_str()))),
    );
    // Ctrl+[ is Escape in Insert mode too (GPUI Kit outdents with it on
    // Linux).
    let any = format!("{CONTEXT} > Input && mode == editor");
    cx.bind_keys([KeyBinding::new("ctrl-[", NoAction, Some(any.as_str()))]);
}

/// The key Vim reads from a keystroke; `None` for chords that stay the
/// app's (⌘, Alt).
fn key_for(keystroke: &Keystroke) -> Option<Key> {
    let m = &keystroke.modifiers;
    if m.platform || m.alt || m.function {
        return None;
    }
    if m.control {
        let mut chars = keystroke.key.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_lowercase() || c == '[' => Some(Key::Ctrl(c)),
            _ => None,
        };
    }
    Some(match keystroke.key.as_str() {
        "escape" => Key::Escape,
        "enter" => Key::Enter,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "tab" => Key::Tab,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        _ => {
            let typed = keystroke.key_char.as_deref()?;
            let mut chars = typed.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_control() => Key::Char(c),
                _ => return None,
            }
        }
    })
}

pub struct VimLayer {
    vim: Vim,
    state: Entity<EditorState>,
    tab: WeakEntity<EditorTab>,
    /// The block cursor.
    block: TextDecorationCollection,
    /// The last search's matches.
    found: RangeDecorationCollection,
    found_for: Option<String>,
    /// The editor's selection and length when last followed.
    seen: Option<(Range<usize>, usize)>,
    _observe: Subscription,
}

impl VimLayer {
    fn new(
        state: &Entity<EditorState>,
        tab: WeakEntity<EditorTab>,
        cx: &mut Context<Self>,
    ) -> Self {
        let block = state.update(cx, |s, cx| s.create_decorations_collection(Vec::new(), cx));
        let found = state.update(cx, |s, cx| {
            s.create_range_decorations_collection(Vec::new(), cx)
        });
        let observe = cx.observe(state, |layer, _, cx| layer.follow(cx));
        let mut layer = Self {
            vim: Vim::new(),
            state: state.clone(),
            tab,
            block,
            found,
            found_for: None,
            seen: None,
            _observe: observe,
        };
        layer.follow(cx);
        layer
    }

    /// The mode, the command line or a message, and pending keys, as
    /// monaco-vim's status bar shows them.
    pub fn status(&self) -> String {
        let status = self.vim.status();
        [
            Some(status.mode.to_string()),
            status.command_line.map(|line| format!("{line}\u{258f}")),
            status.message,
            Some(status.keys).filter(|k| !k.is_empty()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("  ")
    }

    /// The editor moved the cursor or selection itself (a click, a drag,
    /// typing): let Vim follow, then redraw the block cursor.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let (selection, len) = {
            let state = self.state.read(cx);
            (state.selected_range(), state.text().len())
        };
        if self.seen.as_ref() == Some(&(selection.clone(), len)) {
            return;
        }
        let (text, cursor) = {
            let state = self.state.read(cx);
            (state.value().to_string(), state.cursor())
        };
        if let Some(fixed) = self.vim.sync(&text, cursor, selection.clone()) {
            self.state
                .update(cx, |state, cx| state.set_selected_range(fixed, cx));
        }
        self.seen = Some((self.state.read(cx).selected_range(), len));
        self.paint(&text, cx);
        cx.notify();
    }

    /// The block cursor and search matches for the current text.
    fn paint(&mut self, text: &str, cx: &mut Context<Self>) {
        let cursor = self.state.read(cx).cursor();
        let block = self.vim.block_cursor(text, cursor);
        // Decorations move with edits: compare with where it is now.
        if block.as_slice() != self.block.get_ranges(cx).as_slice() {
            let theme = cx.theme();
            let style = HighlightStyle {
                color: Some(theme.background),
                background_color: Some(theme.caret),
                ..Default::default()
            };
            let decorations = block
                .map(|range| vec![TextDecoration::new(range, style)])
                .unwrap_or_default();
            self.block.set(decorations, cx);
        }
    }

    fn paint_matches(&mut self, text: &str, edited: bool, cx: &mut Context<Self>) {
        let pattern = self.vim.highlighted_pattern().map(str::to_string);
        if pattern == self.found_for && !edited {
            return;
        }
        let color = cx.theme().warning.opacity(0.3);
        let ranges = self.vim.search_highlights(text);
        self.found.set(
            ranges
                .into_iter()
                .map(|range| {
                    RangeDecoration::new(range)
                        .with_style(RangeDecorationStyle::Fill)
                        .with_color(color)
                })
                .collect(),
            cx,
        );
        self.found_for = pattern;
    }

    /// A key on its way to the editor; true when Vim took it.
    fn key_down(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let state = self.state.clone();
        // Not the find field inside the editor.
        if !state.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        if !self.vim.takes_keys() && !matches!(key, Key::Escape | Key::Ctrl('[' | 'c')) {
            return false;
        }
        let Some(tab) = self.tab.upgrade() else {
            return false;
        };
        let (read_only, dirty, indent, tab_size) = {
            let tab = tab.read(cx);
            let indent = if tab.use_spaces {
                " ".repeat(tab.tab_size)
            } else {
                "\t".to_string()
            };
            (tab.read_only, tab.dirty, indent, tab.tab_size)
        };
        let (text, cursor, rows) = {
            let state = state.read(cx);
            (
                state.value().to_string(),
                state.cursor(),
                state.visible_row_range().unwrap_or(0..0),
            )
        };
        let outcome = {
            let app: &App = cx;
            let mut clipboard = || app.read_from_clipboard().and_then(|item| item.text());
            let mut context = VimContext {
                text: &text,
                cursor,
                visible_lines: rows.clone(),
                indent: &indent,
                tab_size,
                clipboard: &mut clipboard,
            };
            self.vim.key(key, &mut context)
        };
        if !outcome.handled {
            return false;
        }
        // Nothing changes in a read-only file.
        let blocked = read_only && outcome.edit.is_some();
        if blocked {
            self.vim.reset();
        }
        let edited = outcome.edit.is_some() && !blocked;
        let head = self.vim.visual_head();
        state.update(cx, |state, cx| {
            if blocked {
                return;
            }
            if let Some((range, new)) = &outcome.edit {
                // `replace` edits the selection quietly: no auto-closed
                // pairs or completions, as typing would bring.
                state.set_selected_range(range.clone(), cx);
                state.replace(new.clone(), window, cx);
            }
            if let Some(selection) = outcome.selection.clone() {
                state.set_selected_range(selection, cx);
            }
        });
        if let Some(head) = head {
            self.reveal_line(&text, head, &rows, cx);
        }
        for action in outcome.actions {
            match action {
                Action::Undo(n) | Action::Redo(n) => {
                    let focus = state.read(cx).focus_handle(cx);
                    for _ in 0..n {
                        if matches!(action, Action::Undo(_)) {
                            focus.dispatch_action(&Undo, window, cx);
                        } else {
                            focus.dispatch_action(&Redo, window, cx);
                        }
                    }
                    // Vim puts the cursor at the change, not around it.
                    state.update(cx, |state, cx| {
                        let start = state.selected_range().start;
                        state.set_selected_range(start..start, cx);
                    });
                    self.seen = None;
                    self.follow(cx);
                }
                Action::Save => {
                    tab.update(cx, |tab, cx| tab.save(cx));
                }
                Action::Close(save) => {
                    let save = match save {
                        CloseSave::Never => false,
                        CloseSave::Always => true,
                        CloseSave::IfModified => dirty,
                    };
                    if save {
                        tab.update(cx, |tab, cx| tab.save(cx));
                    }
                    window.dispatch_action(
                        Box::new(crate::commands::RunCommand::new("close_tab")),
                        cx,
                    );
                }
                Action::CopyToClipboard(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text))
                }
                Action::Scroll(lines) => self.scroll_by(lines, cx),
                Action::Reveal(at) => self.reveal_cursor(at, &rows, cx),
            }
        }
        let text = state.read(cx).value().to_string();
        self.seen = Some((state.read(cx).selected_range(), text.len()));
        self.paint(&text, cx);
        self.paint_matches(&text, edited, cx);
        cx.notify();
        true
    }

    fn line_height(&self, cx: &App) -> f32 {
        self.state.read(cx).line_height().map_or(20., f32::from)
    }

    /// Scroll by whole lines (down when positive).
    fn scroll_by(&mut self, lines: i64, cx: &mut Context<Self>) {
        let height = self.line_height(cx);
        self.state.update(cx, |state, cx| {
            let offset = state.scroll_offset();
            let y = f32::from(offset.y) - lines as f32 * height;
            state.set_scroll_offset(point(offset.x, px(y.min(0.))), cx);
        });
    }

    /// `zt`, `zz`, `zb`: put the cursor's line at the top, middle or bottom.
    /// Screen rows are taken as lines (no wrapping or folds).
    fn reveal_cursor(&mut self, at: Reveal, rows: &Range<usize>, cx: &mut Context<Self>) {
        let height = self.line_height(cx);
        let line = self.state.read(cx).cursor_position().line as f32;
        let shown = rows.len().max(1) as f32;
        let top = match at {
            Reveal::Top => line,
            Reveal::Center => line - (shown / 2.).floor(),
            Reveal::Bottom => line - (shown - 1.),
        }
        .max(0.);
        self.state.update(cx, |state, cx| {
            let x = state.scroll_offset().x;
            state.set_scroll_offset(point(x, px(-top * height)), cx);
        });
    }

    /// Keep a Visual selection's moving end on screen (the editor scrolls
    /// to the selection's start).
    fn reveal_line(
        &mut self,
        text: &str,
        offset: usize,
        rows: &Range<usize>,
        cx: &mut Context<Self>,
    ) {
        if rows.is_empty() {
            return;
        }
        let line = text[..offset.min(text.len())].matches('\n').count();
        let height = self.line_height(cx);
        let top = if line < rows.start {
            line
        } else if line >= rows.end {
            line + 1 - rows.len()
        } else {
            return;
        };
        self.state.update(cx, |state, cx| {
            let x = state.scroll_offset().x;
            state.set_scroll_offset(point(x, px(-(top as f32) * height)), cx);
        });
    }
}

/// An editor tab's Vim layer, while the setting is on.
pub struct Attached {
    pub layer: Entity<VimLayer>,
    _observe: Subscription,
}

impl Attached {
    /// The layer for `state` when `enabled`.
    pub fn new(
        enabled: bool,
        state: &Entity<EditorState>,
        cx: &mut Context<EditorTab>,
    ) -> Option<Self> {
        if !enabled {
            return None;
        }
        let tab = cx.weak_entity();
        let layer = cx.new(|cx| VimLayer::new(state, tab, cx));
        let observe = cx.observe(&layer, |_, _, cx| cx.notify());
        Some(Self {
            layer,
            _observe: observe,
        })
    }

    /// The setting changed: turn the layer on or off.
    pub fn update(
        this: &mut Option<Self>,
        enabled: bool,
        state: &Entity<EditorState>,
        cx: &mut Context<EditorTab>,
    ) {
        if enabled == this.is_some() {
            return;
        }
        if let Some(old) = this.take() {
            old.layer.update(cx, |layer, cx| {
                layer.block.clear(cx);
                layer.found.clear(cx);
            });
        } else {
            *this = Self::new(true, state, cx);
        }
    }
}

/// The editor element inside the key context and listeners Vim needs.
pub fn wrap(vim: Option<&Attached>, editor: impl IntoElement, cx: &App) -> AnyElement {
    let Some(vim) = vim else {
        return editor.into_any_element();
    };
    let layer = vim.layer.clone();
    let context = if layer.read(cx).vim.takes_keys() {
        "Vim VimNormal"
    } else {
        CONTEXT
    };
    let on_key = layer.clone();
    div()
        .key_context(context)
        .size_full()
        .capture_key_down(move |event, window, cx| {
            let Some(key) = key_for(&event.keystroke) else {
                return;
            };
            if on_key.update(cx, |layer, cx| layer.key_down(key, window, cx)) {
                cx.stop_propagation();
            }
        })
        // Escape is the editor's action: Vim sees it first, and the editor
        // still closes its menus with it.
        .capture_action(move |_: &Escape, window, cx| {
            layer.update(cx, |layer, cx| layer.key_down(Key::Escape, window, cx));
        })
        .child(editor)
        .into_any_element()
}

impl EditorTab {
    /// What the status bar shows for Vim, with it on.
    pub fn vim_status(&self, cx: &App) -> Option<String> {
        self.vim.as_ref().map(|vim| vim.layer.read(cx).status())
    }
}
