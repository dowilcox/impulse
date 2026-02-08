use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

/// Information about a single extra cursor (the primary cursor is managed by GtkTextBuffer).
#[derive(Clone)]
struct CursorInfo {
    /// Left-gravity mark at the cursor position.
    position: gtk4::TextMark,
    /// Optional selection: (anchor, bound) marks.
    selection: Option<(gtk4::TextMark, gtk4::TextMark)>,
}

/// Manages multiple cursor positions in a GtkSourceView buffer.
pub struct MultiCursorState {
    cursors: Vec<CursorInfo>,
    /// The text tag used to render extra cursor indicators.
    cursor_tag: gtk4::TextTag,
    /// The text tag used to render extra selections.
    selection_tag: gtk4::TextTag,
    /// The buffer we are attached to.
    buffer: sourceview5::Buffer,
}

impl MultiCursorState {
    pub fn new(buffer: &sourceview5::Buffer) -> Self {
        let tag_table = buffer.tag_table();

        let cursor_tag = gtk4::TextTag::builder()
            .name("multi-cursor-marker")
            .background("#e0af68")
            .foreground("#1a1b26")
            .build();
        tag_table.add(&cursor_tag);

        let selection_tag = gtk4::TextTag::builder()
            .name("multi-cursor-selection")
            .background("rgba(224,175,104,0.35)")
            .build();
        tag_table.add(&selection_tag);

        Self {
            cursors: Vec::new(),
            cursor_tag,
            selection_tag,
            buffer: buffer.clone(),
        }
    }

    /// Whether there are any extra cursors active.
    pub fn is_active(&self) -> bool {
        !self.cursors.is_empty()
    }

    /// Number of extra cursors (not counting the primary).
    pub fn cursor_count(&self) -> usize {
        self.cursors.len()
    }

    /// Add an extra cursor at the given offset, optionally with a selection range.
    pub fn add_cursor(&mut self, offset: i32, selection: Option<(i32, i32)>) {
        let iter = self.buffer.iter_at_offset(offset);
        let mark = self.buffer.create_mark(None, &iter, true);

        let sel_marks = selection.map(|(start, end)| {
            let s_iter = self.buffer.iter_at_offset(start);
            let e_iter = self.buffer.iter_at_offset(end);
            let anchor = self.buffer.create_mark(None, &s_iter, true);
            let bound = self.buffer.create_mark(None, &e_iter, false);
            (anchor, bound)
        });

        self.cursors.push(CursorInfo {
            position: mark,
            selection: sel_marks,
        });

        self.refresh_visual_markers();
    }

    /// Remove all extra cursors and visual markers.
    pub fn clear_all(&mut self) {
        self.clear_visual_markers();
        for cursor in self.cursors.drain(..) {
            self.buffer.delete_mark(&cursor.position);
            if let Some((anchor, bound)) = cursor.selection {
                self.buffer.delete_mark(&anchor);
                self.buffer.delete_mark(&bound);
            }
        }
    }

    /// Insert text at all extra cursor positions. The primary cursor is handled by GtkTextView.
    pub fn apply_insert(&mut self, text: &str) {
        if self.cursors.is_empty() {
            return;
        }
        self.clear_visual_markers();
        self.buffer.begin_user_action();

        // Collect offsets sorted in reverse order so insertions don't shift later offsets.
        let mut ops: Vec<(i32, Option<(i32, i32)>)> = self
            .cursors
            .iter()
            .map(|c| {
                let pos = self.buffer.iter_at_mark(&c.position).offset();
                let sel = c.selection.as_ref().map(|(a, b)| {
                    (
                        self.buffer.iter_at_mark(a).offset(),
                        self.buffer.iter_at_mark(b).offset(),
                    )
                });
                (pos, sel)
            })
            .collect();
        ops.sort_by(|a, b| b.0.cmp(&a.0));

        for (pos, sel) in &ops {
            if let Some((start, end)) = sel {
                // Delete the selection first, then insert
                let mut s = self.buffer.iter_at_offset(*start);
                let mut e = self.buffer.iter_at_offset(*end);
                self.buffer.delete(&mut s, &mut e);
                let mut iter = self.buffer.iter_at_offset(*start);
                self.buffer.insert(&mut iter, text);
            } else {
                let mut iter = self.buffer.iter_at_offset(*pos);
                self.buffer.insert(&mut iter, text);
            }
        }

        self.buffer.end_user_action();

        // Update cursor marks to new positions after insertion
        self.reposition_cursors_after_edit();
        self.refresh_visual_markers();
    }

    /// Delete one character before each extra cursor (backspace).
    pub fn apply_backspace(&mut self) {
        if self.cursors.is_empty() {
            return;
        }
        self.clear_visual_markers();
        self.buffer.begin_user_action();

        let mut ops: Vec<(i32, Option<(i32, i32)>)> = self
            .cursors
            .iter()
            .map(|c| {
                let pos = self.buffer.iter_at_mark(&c.position).offset();
                let sel = c.selection.as_ref().map(|(a, b)| {
                    (
                        self.buffer.iter_at_mark(a).offset(),
                        self.buffer.iter_at_mark(b).offset(),
                    )
                });
                (pos, sel)
            })
            .collect();
        ops.sort_by(|a, b| b.0.cmp(&a.0));

        for (pos, sel) in &ops {
            if let Some((start, end)) = sel {
                let mut s = self.buffer.iter_at_offset(*start);
                let mut e = self.buffer.iter_at_offset(*end);
                self.buffer.delete(&mut s, &mut e);
            } else if *pos > 0 {
                let mut start = self.buffer.iter_at_offset(*pos - 1);
                let mut end = self.buffer.iter_at_offset(*pos);
                self.buffer.delete(&mut start, &mut end);
            }
        }

        self.buffer.end_user_action();
        self.reposition_cursors_after_edit();
        self.refresh_visual_markers();
    }

    /// Delete one character after each extra cursor (Delete key).
    pub fn apply_delete(&mut self) {
        if self.cursors.is_empty() {
            return;
        }
        self.clear_visual_markers();
        self.buffer.begin_user_action();

        let mut ops: Vec<(i32, Option<(i32, i32)>)> = self
            .cursors
            .iter()
            .map(|c| {
                let pos = self.buffer.iter_at_mark(&c.position).offset();
                let sel = c.selection.as_ref().map(|(a, b)| {
                    (
                        self.buffer.iter_at_mark(a).offset(),
                        self.buffer.iter_at_mark(b).offset(),
                    )
                });
                (pos, sel)
            })
            .collect();
        ops.sort_by(|a, b| b.0.cmp(&a.0));

        for (pos, sel) in &ops {
            if let Some((start, end)) = sel {
                let mut s = self.buffer.iter_at_offset(*start);
                let mut e = self.buffer.iter_at_offset(*end);
                self.buffer.delete(&mut s, &mut e);
            } else {
                let end_offset = self.buffer.end_iter().offset();
                if *pos < end_offset {
                    let mut start = self.buffer.iter_at_offset(*pos);
                    let mut end = self.buffer.iter_at_offset(*pos + 1);
                    self.buffer.delete(&mut start, &mut end);
                }
            }
        }

        self.buffer.end_user_action();
        self.reposition_cursors_after_edit();
        self.refresh_visual_markers();
    }

    /// After an edit, clear selections on extra cursors (they've been consumed).
    fn reposition_cursors_after_edit(&mut self) {
        for cursor in &mut self.cursors {
            if let Some((ref anchor, ref bound)) = cursor.selection {
                // After edit the selection is consumed; move cursor mark to the anchor position
                let pos = self.buffer.iter_at_mark(anchor);
                self.buffer.move_mark(&cursor.position, &pos);
                self.buffer.delete_mark(anchor);
                self.buffer.delete_mark(bound);
            }
            cursor.selection = None;
        }
    }

    /// Remove duplicate cursors that ended up at the same offset.
    pub fn deduplicate(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut to_remove = Vec::new();
        for (i, cursor) in self.cursors.iter().enumerate() {
            let offset = self.buffer.iter_at_mark(&cursor.position).offset();
            if !seen.insert(offset) {
                to_remove.push(i);
            }
        }
        for i in to_remove.into_iter().rev() {
            let cursor = self.cursors.remove(i);
            self.buffer.delete_mark(&cursor.position);
            if let Some((a, b)) = cursor.selection {
                self.buffer.delete_mark(&a);
                self.buffer.delete_mark(&b);
            }
        }
    }

    /// Clear all text tags used for visual markers.
    fn clear_visual_markers(&self) {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        self.buffer.remove_tag(&self.cursor_tag, &start, &end);
        self.buffer.remove_tag(&self.selection_tag, &start, &end);
    }

    /// Reapply visual markers at all extra cursor positions.
    pub fn refresh_visual_markers(&self) {
        self.clear_visual_markers();
        for cursor in &self.cursors {
            // Draw cursor marker (1-char highlight or end-of-line indicator)
            let pos = self.buffer.iter_at_mark(&cursor.position);
            let offset = pos.offset();
            let end_offset = if pos.ends_line() {
                // At end of line, apply to last char if available
                offset
            } else {
                offset + 1
            };

            if end_offset > offset {
                let end_iter = self.buffer.iter_at_offset(end_offset);
                self.buffer.apply_tag(&self.cursor_tag, &pos, &end_iter);
            } else if offset > 0 {
                // End of line: highlight the char before cursor
                let prev = self.buffer.iter_at_offset(offset - 1);
                self.buffer.apply_tag(&self.cursor_tag, &prev, &pos);
            }

            // Draw selection highlight
            if let Some((ref anchor, ref bound)) = cursor.selection {
                let a_iter = self.buffer.iter_at_mark(anchor);
                let b_iter = self.buffer.iter_at_mark(bound);
                let (s, e) = if a_iter.offset() <= b_iter.offset() {
                    (a_iter, b_iter)
                } else {
                    (b_iter, a_iter)
                };
                self.buffer.apply_tag(&self.selection_tag, &s, &e);
            }
        }
    }
}

impl Drop for MultiCursorState {
    fn drop(&mut self) {
        self.clear_all();
        self.buffer.tag_table().remove(&self.cursor_tag);
        self.buffer.tag_table().remove(&self.selection_tag);
    }
}

/// Find the next occurrence of `needle` in the buffer starting after `start_offset`,
/// wrapping around if needed. Returns (match_start_offset, match_end_offset) or None.
pub fn find_next_occurrence(
    buffer: &sourceview5::Buffer,
    needle: &str,
    start_offset: i32,
) -> Option<(i32, i32)> {
    if needle.is_empty() {
        return None;
    }
    let text = {
        let s = buffer.start_iter();
        let e = buffer.end_iter();
        buffer.text(&s, &e, true).to_string()
    };

    // Search forward from start_offset
    if let Some(rel_pos) = text[(start_offset as usize)..].find(needle) {
        let abs_start = start_offset + rel_pos as i32;
        let abs_end = abs_start + needle.len() as i32;
        return Some((abs_start, abs_end));
    }

    // Wrap around: search from beginning
    if let Some(rel_pos) = text[..(start_offset as usize).min(text.len())].find(needle) {
        let abs_start = rel_pos as i32;
        let abs_end = abs_start + needle.len() as i32;
        return Some((abs_start, abs_end));
    }

    None
}

/// Get the word under the cursor or the current selection text.
pub fn get_word_or_selection(buffer: &sourceview5::Buffer) -> Option<String> {
    if buffer.has_selection() {
        let (start, end) = buffer.selection_bounds()?;
        return Some(buffer.text(&start, &end, true).to_string());
    }

    // Get the word at cursor
    let insert = buffer.get_insert();
    let iter = buffer.iter_at_mark(&insert);
    let mut start = iter;
    let mut end = iter;

    // Move start backwards to word boundary
    while !start.starts_line() {
        let mut prev = start;
        prev.backward_char();
        let ch = prev.char();
        if ch.is_alphanumeric() || ch == '_' {
            start = prev;
        } else {
            break;
        }
    }

    // Move end forwards to word boundary
    while !end.ends_line() {
        let ch = end.char();
        if ch.is_alphanumeric() || ch == '_' {
            end.forward_char();
        } else {
            break;
        }
    }

    if start.offset() == end.offset() {
        return None;
    }

    Some(buffer.text(&start, &end, true).to_string())
}

/// Shared multi-cursor state wrapped for GTK signal closures.
pub type SharedMultiCursorState = Rc<RefCell<Option<MultiCursorState>>>;

/// Create a new shared multi-cursor state.
pub fn new_shared() -> SharedMultiCursorState {
    Rc::new(RefCell::new(None))
}

/// Handle Ctrl+D: select next occurrence and add a cursor.
/// Returns true if the event was handled.
pub fn handle_ctrl_d(state: &SharedMultiCursorState, buffer: &sourceview5::Buffer) -> bool {
    let word = match get_word_or_selection(buffer) {
        Some(w) if !w.is_empty() => w,
        _ => return false,
    };

    let mut state_ref = state.borrow_mut();

    // Initialize multi-cursor state if not yet active
    if state_ref.is_none() {
        *state_ref = Some(MultiCursorState::new(buffer));
    }

    let mc = state_ref.as_mut().unwrap();

    // If the primary cursor doesn't have a selection yet, select the current word first
    if !buffer.has_selection() {
        let insert = buffer.get_insert();
        let iter = buffer.iter_at_mark(&insert);
        let mut start = iter;
        let mut end = iter;

        while !start.starts_line() {
            let mut prev = start;
            prev.backward_char();
            let ch = prev.char();
            if ch.is_alphanumeric() || ch == '_' {
                start = prev;
            } else {
                break;
            }
        }
        while !end.ends_line() {
            let ch = end.char();
            if ch.is_alphanumeric() || ch == '_' {
                end.forward_char();
            } else {
                break;
            }
        }

        buffer.select_range(&start, &end);
        return true;
    }

    // Find the farthest cursor position to search after
    let mut search_after = {
        let (_, sel_end) = buffer.selection_bounds().unwrap();
        sel_end.offset()
    };
    for cursor in &mc.cursors {
        if let Some((_, ref bound)) = cursor.selection {
            let off = buffer.iter_at_mark(bound).offset();
            if off > search_after {
                search_after = off;
            }
        }
        let off = buffer.iter_at_mark(&cursor.position).offset();
        if off > search_after {
            search_after = off;
        }
    }

    // Find the next occurrence
    if let Some((match_start, match_end)) = find_next_occurrence(buffer, &word, search_after) {
        // Check we're not adding a cursor at the primary selection
        if let Some((prim_start, prim_end)) = buffer.selection_bounds() {
            if match_start == prim_start.offset() && match_end == prim_end.offset() {
                return false;
            }
        }

        // Check we're not duplicating an existing extra cursor
        for cursor in &mc.cursors {
            if let Some((ref anchor, ref bound)) = cursor.selection {
                let a = buffer.iter_at_mark(anchor).offset();
                let b = buffer.iter_at_mark(bound).offset();
                if a == match_start && b == match_end {
                    return false;
                }
            }
        }

        mc.add_cursor(match_end, Some((match_start, match_end)));
        true
    } else {
        false
    }
}

/// Handle Escape: clear all extra cursors. Returns true if there were cursors to clear.
pub fn handle_escape(state: &SharedMultiCursorState) -> bool {
    let mut state_ref = state.borrow_mut();
    if let Some(ref mut mc) = *state_ref {
        if mc.is_active() {
            mc.clear_all();
            *state_ref = None;
            return true;
        }
    }
    *state_ref = None;
    false
}

/// Handle character insertion at all extra cursors. Returns true if handled.
pub fn handle_insert(state: &SharedMultiCursorState, text: &str) -> bool {
    let mut state_ref = state.borrow_mut();
    if let Some(ref mut mc) = *state_ref {
        if mc.is_active() {
            mc.apply_insert(text);
            mc.deduplicate();
            return true;
        }
    }
    false
}

/// Handle backspace at all extra cursors. Returns true if handled.
pub fn handle_backspace(state: &SharedMultiCursorState) -> bool {
    let mut state_ref = state.borrow_mut();
    if let Some(ref mut mc) = *state_ref {
        if mc.is_active() {
            mc.apply_backspace();
            mc.deduplicate();
            return true;
        }
    }
    false
}

/// Handle delete at all extra cursors. Returns true if handled.
pub fn handle_delete(state: &SharedMultiCursorState) -> bool {
    let mut state_ref = state.borrow_mut();
    if let Some(ref mut mc) = *state_ref {
        if mc.is_active() {
            mc.apply_delete();
            mc.deduplicate();
            return true;
        }
    }
    false
}
