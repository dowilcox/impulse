//! Command blocks in a terminal (the Swift `TerminalTab` block commands and
//! `TerminalRenderer`'s block toolbar, hints and links): moving between
//! blocks, bookmarks, selecting blocks to copy, the toolbar over the block
//! under the pointer (copy output, run again, more), hints mode (labels
//! over URLs, file references, SHAs and ports; typing one acts on it), and
//! links under the pointer (OSC 8 hyperlinks, URLs, and file references
//! that exist, which Cmd-click (Ctrl-click on Linux) opens).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    anchored, deferred, div, prelude::*, px, AnyElement, ClipboardItem, Context, KeyDownEvent,
    MouseButton, Pixels, Point, Window,
};
use impulse_kit::terminal_hints::{self, HintKind};
use impulse_terminal::{TerminalBlockId, TerminalCommandBlockSummary};

use crate::terminal_view::{TerminalView, TerminalViewEvent};
use crate::workbench_chrome::hsla;

/// Block navigation, selection, bookmarks and the hover toolbar.
#[derive(Default)]
pub(crate) struct Blocks {
    /// The block moved to with Previous / Next Block (accent wash).
    pub highlighted: Option<u64>,
    /// Blocks picked with Cmd+Up, Up / Down, Shift or Cmd-click, in order.
    pub selection: Vec<u64>,
    cursor: Option<u64>,
    anchor: Option<u64>,
    pub bookmarks: HashSet<u64>,
    /// The block under the pointer.
    pub hovered: Option<u64>,
    /// The toolbar's "more" menu or the right-click menu, where it opens.
    pub menu: Option<BlockMenu>,
}

/// An open block menu: the toolbar's ⋯ (one block's actions and block
/// navigation) or the right-click menu (also the terminal's own items, and
/// the clicked block's first when there is one).
#[derive(Clone, Copy)]
pub(crate) struct BlockMenu {
    pub block: Option<u64>,
    pub at: Point<Pixels>,
    pub full: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolbarButton {
    CopyOutput,
    Rerun,
    SendToAgent,
    Menu,
}

/// One thing hints mode labeled.
pub(crate) struct HintTarget {
    pub row: usize,
    pub start_col: usize,
    /// Exclusive.
    pub end_col: usize,
    pub label: String,
    pub kind: HintKind,
    pub text: String,
    /// For file references: the absolute path, line and column.
    pub path: Option<(PathBuf, Option<usize>, Option<usize>)>,
}

pub(crate) struct Hints {
    pub targets: Vec<HintTarget>,
    pub typed: String,
}

/// What a link under the pointer opens.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LinkTarget {
    Url(String),
    File {
        path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HoverLink {
    pub row: usize,
    pub start_col: usize,
    /// Exclusive.
    pub end_col: usize,
    pub target: LinkTarget,
}

/// A viewport row's text, with each byte's column and each column's first
/// byte (`None` for a wide character's spacer) and OSC 8 flag.
pub(crate) struct RowText {
    pub text: String,
    pub col_for_byte: Vec<usize>,
    pub byte_for_col: Vec<Option<usize>>,
    pub hyperlink: Vec<bool>,
}

/// Recent answers to "is this an existing file?", so hovering doesn't look
/// the same path up on every move.
#[derive(Default)]
pub(crate) struct FileChecks(HashMap<PathBuf, (bool, Instant)>);

impl FileChecks {
    pub fn is_file(&mut self, path: &PathBuf) -> bool {
        if let Some((is_file, at)) = self.0.get(path) {
            if at.elapsed() < Duration::from_secs(5) {
                return *is_file;
            }
        }
        let is_file = path.is_file();
        if self.0.len() > 512 {
            self.0.clear();
        }
        self.0.insert(path.clone(), (is_file, Instant::now()));
        is_file
    }
}

/// Which block command (`perform_block_command`) an id names.
pub const BLOCK_COMMANDS: &[&str] = &[
    "select_blocks",
    "terminal_hints",
    "previous_block",
    "next_block",
    "last_failed_block",
    "toggle_block_bookmark",
    "previous_block_bookmark",
    "next_block_bookmark",
];

impl TerminalView {
    /// A Command Blocks menu or keybinding command.
    pub fn perform_block_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "select_blocks" => {
                self.begin_block_selection(window, cx);
            }
            "terminal_hints" => {
                if !self.begin_hints(window, cx) {
                    cx.emit(TerminalViewEvent::Notice(
                        "Nothing to open on screen.".into(),
                    ));
                }
            }
            "previous_block" => self.jump_to_previous_block(cx),
            "next_block" => self.jump_to_next_block(cx),
            "last_failed_block" => self.jump_to_last_failed_block(cx),
            "toggle_block_bookmark" => {
                // The selected blocks, else the one moved to, else the newest.
                let targets = if !self.blocks.selection.is_empty() {
                    self.blocks.selection.clone()
                } else {
                    self.blocks
                        .highlighted
                        .or_else(|| self.navigable_blocks().last().map(|b| b.id.0))
                        .into_iter()
                        .collect()
                };
                self.toggle_bookmarks(&targets, cx);
            }
            "previous_block_bookmark" => self.jump_to_bookmark(-1, cx),
            "next_block_bookmark" => self.jump_to_bookmark(1, cx),
            _ => {}
        }
    }

    /// Blocks with a command or output, oldest first.
    pub(crate) fn navigable_blocks(&self) -> Vec<TerminalCommandBlockSummary> {
        self.backend
            .command_block_summaries()
            .into_iter()
            .filter(|b| b.command.as_ref().is_some_and(|c| !c.trim().is_empty()) || b.has_output)
            .collect()
    }

    /// Highlight block `id` and scroll it to the top.
    fn go_to_block(&mut self, id: u64, cx: &mut Context<Self>) {
        self.blocks.highlighted = Some(id);
        self.backend.scroll_to_command_block(TerminalBlockId(id));
        cx.notify();
    }

    fn jump_to_previous_block(&mut self, cx: &mut Context<Self>) {
        let blocks = self.navigable_blocks();
        let Some(last) = blocks.last() else { return };
        let target = self
            .blocks
            .highlighted
            .and_then(|id| blocks.iter().position(|b| b.id.0 == id))
            .filter(|&i| i > 0)
            .map_or(last.id.0, |i| blocks[i - 1].id.0);
        self.go_to_block(target, cx);
    }

    fn jump_to_next_block(&mut self, cx: &mut Context<Self>) {
        let blocks = self.navigable_blocks();
        let Some(first) = blocks.first() else { return };
        let Some(index) = self
            .blocks
            .highlighted
            .and_then(|id| blocks.iter().position(|b| b.id.0 == id))
        else {
            return self.go_to_block(first.id.0, cx);
        };
        match blocks.get(index + 1) {
            Some(next) => self.go_to_block(next.id.0, cx),
            None => {
                // Past the newest: back to the live bottom.
                self.blocks.highlighted = None;
                self.backend.scroll_to_bottom();
                cx.notify();
            }
        }
    }

    fn jump_to_last_failed_block(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self
            .navigable_blocks()
            .iter()
            .rev()
            .find(|b| b.exit_code.unwrap_or(0) != 0)
        {
            self.go_to_block(target.id.0, cx);
        }
    }

    pub(crate) fn toggle_bookmarks(&mut self, ids: &[u64], cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let adding = !ids.iter().all(|id| self.blocks.bookmarks.contains(id));
        for id in ids {
            if adding {
                self.blocks.bookmarks.insert(*id);
            } else {
                self.blocks.bookmarks.remove(id);
            }
        }
        cx.notify();
    }

    /// The next (+1) or previous (-1) bookmark, wrapping around.
    fn jump_to_bookmark(&mut self, direction: i32, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.navigable_blocks().iter().map(|b| b.id.0).collect();
        let marked: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|id| self.blocks.bookmarks.contains(id))
            .collect();
        if marked.is_empty() {
            return;
        }
        let position = self
            .blocks
            .cursor
            .or(self.blocks.highlighted)
            .and_then(|current| ids.iter().position(|&id| id == current));
        let index_of = |id: u64| ids.iter().position(|&i| i == id).unwrap_or(0);
        let target = if direction > 0 {
            marked
                .iter()
                .copied()
                .find(|&id| position.is_none_or(|p| index_of(id) > p))
                .unwrap_or(marked[0])
        } else {
            marked
                .iter()
                .rev()
                .copied()
                .find(|&id| position.is_none_or(|p| index_of(id) < p))
                .unwrap_or(marked[marked.len() - 1])
        };
        self.go_to_block(target, cx);
    }

    // MARK: Block selection

    /// Select the newest block and take the keyboard for Up / Down / Shift /
    /// Copy / Escape. False when there's nothing to select.
    pub(crate) fn begin_block_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(last) = self.navigable_blocks().last().map(|b| b.id.0) else {
            return false;
        };
        self.select_blocks(last, false, cx);
        window.focus(&self.focus, cx);
        true
    }

    /// Select the newest `count` blocks (the snapshot's `select-blocks=`).
    #[cfg(debug_assertions)]
    pub(crate) fn select_last_blocks(
        &mut self,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.begin_block_selection(window, cx) {
            return;
        }
        let ids: Vec<u64> = self.navigable_blocks().iter().map(|b| b.id.0).collect();
        if let Some(&first) = ids.get(ids.len().saturating_sub(count.max(1))) {
            self.select_blocks(first, true, cx);
        }
    }

    fn select_blocks(&mut self, cursor: u64, extend: bool, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.navigable_blocks().iter().map(|b| b.id.0).collect();
        let anchor = self.blocks.anchor.or(self.blocks.cursor);
        match (
            extend,
            anchor.and_then(|a| ids.iter().position(|&i| i == a)),
            ids.iter().position(|&i| i == cursor),
        ) {
            (true, Some(a), Some(b)) => {
                self.blocks.selection = ids[a.min(b)..=a.max(b)].to_vec();
                self.blocks.anchor = anchor;
            }
            _ => {
                self.blocks.selection = vec![cursor];
                self.blocks.anchor = Some(cursor);
            }
        }
        self.blocks.cursor = Some(cursor);
        self.backend
            .scroll_to_command_block(TerminalBlockId(cursor));
        cx.notify();
    }

    /// Cmd-click: add or remove one block.
    pub(crate) fn toggle_block_selection(&mut self, id: u64, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.navigable_blocks().iter().map(|b| b.id.0).collect();
        if let Some(i) = self.blocks.selection.iter().position(|&b| b == id) {
            self.blocks.selection.remove(i);
        } else {
            self.blocks.selection.push(id);
            self.blocks
                .selection
                .sort_by_key(|b| ids.iter().position(|i| i == b).unwrap_or(0));
        }
        self.blocks.cursor = if self.blocks.selection.contains(&id) {
            Some(id)
        } else {
            self.blocks.selection.last().copied()
        };
        self.blocks.anchor = self.blocks.cursor;
        cx.notify();
    }

    pub(crate) fn end_block_selection(
        &mut self,
        focus_input: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.blocks.selection.clear();
        self.blocks.cursor = None;
        self.blocks.anchor = None;
        if focus_input {
            self.focus(window, cx);
        }
        cx.notify();
    }

    /// Keys while blocks are selected: Up / Down (Shift extends), Copy,
    /// Escape. Anything else leaves the selection and goes on (to the input
    /// bar at a prompt). True when the key was used.
    pub(crate) fn block_selection_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let k = &event.keystroke;
        let m = &k.modifiers;
        let secondary = if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        };
        let ids: Vec<u64> = self.navigable_blocks().iter().map(|b| b.id.0).collect();
        match k.key.as_str() {
            "up" | "down" if !m.alt && (!secondary || k.key == "up") => {
                let delta: i64 = if k.key == "up" { -1 } else { 1 };
                let Some(index) = self
                    .blocks
                    .cursor
                    .and_then(|c| ids.iter().position(|&i| i == c))
                else {
                    self.begin_block_selection(window, cx);
                    return true;
                };
                let next = index as i64 + delta;
                if next >= ids.len() as i64 && !m.shift {
                    // Past the newest block: back to the input.
                    self.end_block_selection(true, window, cx);
                    self.backend.scroll_to_bottom();
                    return true;
                }
                if next >= 0 && (next as usize) < ids.len() {
                    self.select_blocks(ids[next as usize], m.shift && !secondary, cx);
                }
                true
            }
            "escape" => {
                self.end_block_selection(true, window, cx);
                true
            }
            "c" if secondary && (cfg!(target_os = "macos") || m.shift) => {
                self.copy_selected_blocks(cx);
                true
            }
            "a" if secondary && m.shift => {
                let ids = self.blocks.selection.clone();
                self.send_blocks_to_agent(&ids, cx);
                true
            }
            _ => {
                if !secondary {
                    self.end_block_selection(false, window, cx);
                }
                false
            }
        }
    }

    /// Copy on selected blocks: each command and its output.
    pub(crate) fn copy_selected_blocks(&mut self, cx: &mut Context<Self>) {
        let parts: Vec<String> = self
            .blocks
            .selection
            .iter()
            .filter_map(|&id| self.backend.command_block(id))
            .filter_map(|block| {
                let command = block.command.as_deref().unwrap_or("").trim().to_string();
                let output = block.output.trim_matches('\n').to_string();
                let text = [
                    (!command.is_empty()).then(|| format!("$ {command}")),
                    (!output.is_empty()).then_some(output),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n");
                (!text.is_empty()).then_some(text)
            })
            .collect();
        if !parts.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(parts.join("\n\n")));
        }
    }

    // MARK: Per-block actions

    /// Copy a block's command and/or output.
    pub(crate) fn copy_block(
        &mut self,
        id: u64,
        command: bool,
        output: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.backend.command_block(id) else {
            return;
        };
        let mut parts = Vec::new();
        if let Some(c) = block
            .command
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            if command {
                parts.push(c.to_string());
            }
        }
        if output {
            let out = block.output.trim_matches('\n');
            if !out.is_empty() {
                parts.push(out.to_string());
            }
        }
        if !parts.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(parts.join("\n\n")));
        }
    }

    pub(crate) fn rerun_block(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(command) = self
            .backend
            .command_block(id)
            .and_then(|b| b.command)
            .filter(|c| !c.trim().is_empty())
        {
            self.backend.scroll_to_bottom();
            self.backend.rerun_command(&command);
            cx.notify();
        }
    }

    /// The most recent block with a command.
    fn latest_command(&self) -> Option<String> {
        self.backend
            .command_blocks()
            .into_iter()
            .rev()
            .filter_map(|b| b.command)
            .find(|c| !c.trim().is_empty())
    }

    /// The output of the most recent block that printed something.
    fn latest_output(&self) -> Option<String> {
        self.backend
            .command_blocks()
            .into_iter()
            .rev()
            .map(|b| b.output)
            .find(|o| !o.is_empty())
    }

    /// The ⋯ menu or the right-click menu, when open (Swift's
    /// `buildBlockActionMenu` and `buildTerminalContextMenu`).
    pub(crate) fn block_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let BlockMenu {
            block: id,
            at,
            full,
        } = self.blocks.menu?;
        let p = &self.palette;
        let block = id.and_then(|id| {
            self.backend
                .block_overlay()
                .blocks
                .into_iter()
                .find(|b| b.id == id)
        });
        let flags = self.backend.command_block_flags();
        let mut items: Vec<(&str, &str, bool)> = Vec::new();
        if let (Some(id), Some(block)) = (id, block.as_ref()) {
            let has_command = block.command.as_ref().is_some_and(|c| !c.is_empty());
            let running = block.is_running;
            let bookmarked = self.blocks.bookmarks.contains(&id);
            items.extend([
                ("copy_command", "Copy Command", has_command),
                ("copy_output", "Copy Output", true),
                ("copy_both", "Copy Command & Output", true),
                ("rerun", "Rerun Command", has_command && !running),
                ("send_to_agent", "Send to Agent", !running),
                (
                    "bookmark",
                    if bookmarked {
                        "Remove Bookmark"
                    } else {
                        "Bookmark Block"
                    },
                    true,
                ),
                ("-", "", false),
            ]);
        }
        if full {
            let has_selection = self.backend.selected_text().is_some_and(|t| !t.is_empty());
            let can_paste = cx.read_from_clipboard().is_some();
            let has_history = !crate::command_history::CommandHistory::recent(cx).is_empty()
                || !self.backend.command_blocks().is_empty();
            items.extend([
                ("copy", "Copy", has_selection),
                ("paste", "Paste", can_paste),
                ("-", "", false),
                ("copy_last_command", "Copy Last Command", flags.has_command),
                (
                    "copy_last_output",
                    "Copy Last Command Output",
                    flags.has_output,
                ),
                ("rerun_last", "Rerun Last Command", flags.has_command),
                ("command_history", "Command History…", has_history),
                ("-", "", false),
            ]);
        }
        items.extend([
            (
                "previous_block",
                "Previous Block",
                flags.has_command || flags.has_output,
            ),
            (
                "next_block",
                "Next Block",
                flags.has_command || flags.has_output,
            ),
            ("last_failed_block", "Last Failed Block", flags.has_failed),
        ]);
        if full {
            items.extend([
                ("-", "", false),
                ("select_all", "Select All", true),
                ("clear", "Clear", true),
            ]);
        }
        let mut list = div()
            .id("block-menu")
            .occlude()
            .min_w(px(200.))
            .p(px(4.))
            .rounded(px(6.))
            .border_1()
            .border_color(hsla(p.hairline_strong))
            .bg(hsla(p.overlay))
            .shadow_lg()
            .text_size(px(13.))
            .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                view.blocks.menu = None;
                cx.notify();
            }));
        for (action, title, enabled) in items {
            if action == "-" {
                list = list.child(div().my(px(4.)).h(px(1.)).bg(hsla(p.hairline)));
                continue;
            }
            let action = action.to_string();
            list = list.child(
                div()
                    .id(gpui_kit::ElementId::Name(action.clone().into()))
                    .px(px(8.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .text_color(hsla(if enabled { p.text } else { p.text_tertiary }))
                    .when(enabled, |d| d.hover(|d| d.bg(hsla(p.hover))))
                    .child(title.to_string())
                    .when(enabled, |d| {
                        d.on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, window, cx| {
                                cx.stop_propagation();
                                view.blocks.menu = None;
                                match (action.as_str(), id) {
                                    ("copy_command", Some(id)) => {
                                        view.copy_block(id, true, false, cx)
                                    }
                                    ("copy_output", Some(id)) => {
                                        view.copy_block(id, false, true, cx)
                                    }
                                    ("copy_both", Some(id)) => view.copy_block(id, true, true, cx),
                                    ("rerun", Some(id)) => view.rerun_block(id, cx),
                                    ("send_to_agent", Some(id)) => {
                                        view.send_blocks_to_agent(&[id], cx)
                                    }
                                    ("bookmark", Some(id)) => view.toggle_bookmarks(&[id], cx),
                                    ("copy", _) => view.copy_selection(cx),
                                    ("paste", _) => view.paste_clipboard(window, cx),
                                    ("copy_last_command", _) => {
                                        if let Some(command) = view.latest_command() {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                command,
                                            ));
                                        }
                                    }
                                    ("copy_last_output", _) => {
                                        if let Some(output) = view.latest_output() {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                output,
                                            ));
                                        }
                                    }
                                    ("rerun_last", _) => {
                                        if let Some(command) = view.latest_command() {
                                            view.backend.scroll_to_bottom();
                                            view.backend.rerun_command(&command);
                                        }
                                    }
                                    ("command_history", _) => cx.emit(
                                        TerminalViewEvent::RunCommand("command_history".into()),
                                    ),
                                    ("select_all", _) => view.backend.select_all(),
                                    // ⌃L, which the shell takes as clearing the screen.
                                    ("clear", _) => view.backend.write(b"\x0c"),
                                    (other, _) => view.perform_block_command(other, window, cx),
                                }
                                cx.notify();
                            }),
                        )
                    }),
            );
        }
        Some(
            deferred(anchored().position(at).snap_to_window().child(list))
                .with_priority(1)
                .into_any_element(),
        )
    }

    // MARK: Rows

    /// Row `row` of the screen as text.
    pub(crate) fn row_text(&mut self, row: usize) -> Option<RowText> {
        let grid = self.grid_now()?;
        (row < grid.rows).then(|| grid.row_text(row))
    }

    // MARK: Links

    /// The link at a cell: an OSC 8 hyperlink, a URL, or a file reference
    /// that exists.
    pub(crate) fn link_at(&mut self, col: usize, row: usize) -> Option<HoverLink> {
        let text = self.row_text(row)?;
        if text.hyperlink.get(col).copied().unwrap_or(false) {
            let mut start = col;
            while start > 0 && text.hyperlink[start - 1] {
                start -= 1;
            }
            let mut end = col + 1;
            while end < text.hyperlink.len() && text.hyperlink[end] {
                end += 1;
            }
            let url = self.backend.hyperlink_at(col, row)?;
            return Some(HoverLink {
                row,
                start_col: start,
                end_col: end,
                target: LinkTarget::Url(url),
            });
        }
        let byte = (*text.byte_for_col.get(col)?)?;
        let cols = |range: &std::ops::Range<usize>| -> Option<(usize, usize)> {
            let first = *text.col_for_byte.get(range.start)?;
            let last = *text.col_for_byte.get(range.end.checked_sub(1)?)?;
            Some((first, last + 1))
        };
        if let Some(url) = terminal_hints::matches(&text.text)
            .into_iter()
            .find(|m| m.kind == HintKind::Url && m.range.contains(&byte))
        {
            let (start_col, end_col) = cols(&url.range)?;
            return Some(HoverLink {
                row,
                start_col,
                end_col,
                target: LinkTarget::Url(url.text),
            });
        }
        let found = impulse_kit::terminal_paths::match_at(&text.text, byte)?;
        let path = PathBuf::from(impulse_kit::terminal_paths::resolve(&found.path, &self.cwd));
        if !self.file_checks.is_file(&path) {
            return None;
        }
        let (start_col, end_col) = cols(&found.range)?;
        Some(HoverLink {
            row,
            start_col,
            end_col,
            target: LinkTarget::File {
                path,
                line: found.line,
                column: found.column,
            },
        })
    }

    /// Open a link as the Swift terminal does: web and mail links in the
    /// browser, a `file:` link's file in the editor (its folder revealed),
    /// and any other scheme only once asked, since the program in the
    /// terminal chose it.
    pub(crate) fn open_link(
        &mut self,
        target: LinkTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let url = match target {
            LinkTarget::File { path, line, column } => {
                return cx.emit(TerminalViewEvent::OpenFile { path, line, column })
            }
            LinkTarget::Url(url) => url,
        };
        let scheme = url
            .split_once(':')
            .map(|(scheme, _)| scheme.to_ascii_lowercase())
            .unwrap_or_default();
        match scheme.as_str() {
            "http" | "https" | "mailto" => cx.open_url(&url),
            "file" => {
                let path = PathBuf::from(impulse_kit::uri::uri_to_file_path(&url));
                if path.is_file() {
                    cx.emit(TerminalViewEvent::OpenFile {
                        path,
                        line: None,
                        column: None,
                    })
                } else if path.exists() {
                    crate::file_tree_actions::reveal(&path, cx)
                }
            }
            _ => {
                let open = url.clone();
                crate::alert::Alert::new("Open this link?")
                    .message(format!("A program in this terminal linked to:\n\n{url}"))
                    .button("Open", move |window, cx| {
                        window.close_dialog(cx);
                        cx.open_url(&open);
                    })
                    .button("Cancel", |window, cx| window.close_dialog(cx))
                    .show(window, cx);
            }
        }
    }

    // MARK: Hints

    /// Label everything actionable on screen. False when there's nothing.
    pub(crate) fn begin_hints(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(grid) = self.grid_now() else {
            return false;
        };
        let last_row = self.last_content_row.unwrap_or(grid.rows.saturating_sub(1));
        let mut found = Vec::new();
        for row in 0..grid.rows.min(last_row + 1) {
            let text = grid.row_text(row);
            if text.text.trim().is_empty() {
                continue;
            }
            for hint in terminal_hints::matches(&text.text) {
                let (Some(&start), Some(&last)) = (
                    text.col_for_byte.get(hint.range.start),
                    hint.range
                        .end
                        .checked_sub(1)
                        .and_then(|i| text.col_for_byte.get(i)),
                ) else {
                    continue;
                };
                let mut path = None;
                if hint.kind == HintKind::Path {
                    let Some(reference) = &hint.path else {
                        continue;
                    };
                    let absolute = PathBuf::from(impulse_kit::terminal_paths::resolve(
                        &reference.path,
                        &self.cwd,
                    ));
                    if !self.file_checks.is_file(&absolute) {
                        continue;
                    }
                    path = Some((absolute, reference.line, reference.column));
                }
                found.push((row, start, last + 1, hint.kind, hint.text, path));
            }
        }
        if found.is_empty() {
            return false;
        }
        let labels = terminal_hints::labels(found.len());
        self.hints = Some(Hints {
            targets: found
                .into_iter()
                .zip(labels)
                .map(
                    |((row, start_col, end_col, kind, text, path), label)| HintTarget {
                        row,
                        start_col,
                        end_col,
                        label,
                        kind,
                        text,
                        path,
                    },
                )
                .collect(),
            typed: String::new(),
        });
        window.focus(&self.focus, cx);
        cx.notify();
        true
    }

    pub(crate) fn end_hints(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hints.take().is_some() {
            self.focus(window, cx);
            cx.notify();
        }
    }

    /// Letters pick a label (Shift copies, Alt inserts it into the input),
    /// Backspace backs up, Escape leaves. Every key is taken.
    pub(crate) fn hint_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &event.keystroke;
        let Some(hints) = self.hints.as_mut() else {
            return;
        };
        match k.key.as_str() {
            "escape" => return self.end_hints(window, cx),
            "backspace" => {
                hints.typed.pop();
                return cx.notify();
            }
            _ => {}
        }
        let letter = k.key.to_lowercase();
        if letter.chars().count() != 1 || !letter.chars().all(|c| c.is_ascii_alphabetic()) {
            return;
        }
        let typed = format!("{}{letter}", hints.typed);
        let candidates: Vec<usize> = hints
            .targets
            .iter()
            .enumerate()
            .filter(|(_, t)| t.label.starts_with(&typed))
            .map(|(i, _)| i)
            .collect();
        match candidates.as_slice() {
            [] => {}
            [only] if hints.targets[*only].label == typed => {
                let target = hints.targets.swap_remove(*only);
                let (copy, insert) = (k.modifiers.shift, k.modifiers.alt);
                self.hints = None;
                self.focus(window, cx);
                self.hint_chosen(target, copy, insert, window, cx);
            }
            _ => hints.typed = typed,
        }
        cx.notify();
    }

    /// Act on a hint (`performHint`): copy it, insert it into the input,
    /// or open it (a URL or port in the browser, a file in the editor).
    fn hint_chosen(
        &mut self,
        target: HintTarget,
        copy: bool,
        insert: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = target.path.as_ref().map(|(p, _, _)| p.clone());
        if copy || target.kind == HintKind::Sha && !insert {
            // A SHA would open in History, which the Rust app doesn't have
            // yet: copy it instead.
            let text = path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(target.text.clone());
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            let name = path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(target.text);
            cx.emit(TerminalViewEvent::Notice(format!("Copied {name}")));
            return;
        }
        if insert {
            if self.bar_shown(cx) {
                self.insert_into_bar(&target.text, window, cx);
            } else {
                self.backend.write(target.text.as_bytes());
            }
            return;
        }
        match target.kind {
            HintKind::Url | HintKind::Port => {
                let hint = terminal_hints::TerminalHintMatch {
                    kind: target.kind,
                    range: 0..1,
                    text: target.text,
                    path: None,
                };
                if let Some(url) = hint.url() {
                    cx.open_url(url.as_str());
                }
            }
            HintKind::Path => {
                if let Some((path, line, column)) = target.path {
                    cx.emit(TerminalViewEvent::OpenFile { path, line, column });
                }
            }
            HintKind::Sha => {}
        }
    }
}
