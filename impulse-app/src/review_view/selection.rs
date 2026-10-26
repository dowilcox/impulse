//! Line selection in Review (the Swift `ReviewFile.LineSelection`): a click
//! in a changed line's gutter selects it (again: unselects), a Shift-click
//! adds every changed line from the last click's; stage, unstage and revert
//! of the hunk then act on those lines only, a comment on one of them spans
//! them all, ⌘C copies them and Esc clears them. One hunk per file holds a
//! selection; reading the diff again clears it, as hunks merge and split.

use std::collections::BTreeSet;

use gpui_kit::{App, ClipboardItem, Context, Window};
use impulse_git::hunks::{DiffLine, DiffLineKind};

use super::{Action, ReviewEvent, ReviewView};
use crate::context_menu::MenuSpec;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LineSelection {
    pub hunk: usize,
    /// Indices into the hunk's lines, changed lines only.
    pub lines: BTreeSet<usize>,
    /// Where a Shift-click extends from.
    pub anchor: usize,
}

/// `current` after a click on line `line` of hunk `hunk`; `None` once
/// nothing is selected.
pub(super) fn toggled(
    current: Option<LineSelection>,
    lines: &[DiffLine],
    hunk: usize,
    line: usize,
    extend: bool,
) -> Option<LineSelection> {
    let mut selection = current.filter(|s| s.hunk == hunk).unwrap_or(LineSelection {
        hunk,
        lines: BTreeSet::new(),
        anchor: line,
    });
    if extend {
        let (from, to) = (selection.anchor.min(line), selection.anchor.max(line));
        for (index, diff_line) in lines
            .iter()
            .enumerate()
            .take(to.min(lines.len().saturating_sub(1)) + 1)
            .skip(from)
        {
            if diff_line.kind != DiffLineKind::Context {
                selection.lines.insert(index);
            }
        }
    } else {
        if !selection.lines.remove(&line) {
            selection.lines.insert(line);
        }
        selection.anchor = line;
    }
    (!selection.lines.is_empty()).then_some(selection)
}

impl ReviewView {
    /// A click in line `line`'s gutter (Shift: extend).
    pub(super) fn toggle_line(
        &mut self,
        f: usize,
        h: usize,
        line: usize,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = self.files.get_mut(f) else {
            return;
        };
        let Some(lines) = file
            .diff
            .as_ref()
            .and_then(|d| d.hunks.get(h))
            .map(|h| &h.lines)
        else {
            return;
        };
        if lines
            .get(line)
            .is_none_or(|l| l.kind == DiffLineKind::Context)
        {
            return;
        }
        file.selection = toggled(file.selection.take(), lines, h, line, extend);
        self.focused = Some((f, h));
        cx.notify();
    }

    /// Esc: no line selected anywhere. False when nothing was.
    pub(super) fn clear_selections(&mut self, cx: &mut Context<Self>) -> bool {
        let mut cleared = false;
        for file in &mut self.files {
            cleared |= file.selection.take().is_some();
        }
        if cleared {
            cx.notify();
        }
        cleared
    }

    /// ⌘C: the selected lines' text. False when none are selected.
    pub(super) fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        for file in &self.files {
            let (Some(selection), Some(diff)) = (&file.selection, &file.diff) else {
                continue;
            };
            let Some(hunk) = diff.hunks.get(selection.hunk) else {
                continue;
            };
            let text = selection
                .lines
                .iter()
                .filter_map(|&i| hunk.lines.get(i).map(|l| l.content.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            return true;
        }
        false
    }

    /// The selected lines of hunk `h` in file `f`, if it holds the selection.
    pub(super) fn selected_lines(&self, f: usize, h: usize) -> Option<Vec<usize>> {
        let selection = self.files.get(f)?.selection.as_ref()?;
        (selection.hunk == h).then(|| selection.lines.iter().copied().collect())
    }

    /// Whether line `line` of hunk `h` in file `f` is selected.
    pub(super) fn is_selected(&self, f: usize, h: usize, line: usize) -> bool {
        self.files
            .get(f)
            .and_then(|file| file.selection.as_ref())
            .is_some_and(|s| s.hunk == h && s.lines.contains(&line))
    }

    /// A diff line's context menu, as the Swift row's
    /// (`DiffLineCellView.menu(for:)`).
    pub(super) fn line_menu(
        &self,
        f: usize,
        h: usize,
        line: usize,
        view: gpui_kit::WeakEntity<Self>,
    ) -> MenuSpec {
        let menu = MenuSpec::new();
        let Some(file) = self.files.get(f) else {
            return menu;
        };
        let Some(diff_line) = file
            .diff
            .as_ref()
            .and_then(|d| d.hunks.get(h))
            .and_then(|hunk| hunk.lines.get(line))
            .cloned()
        else {
            return menu;
        };
        // A diff cut short can only be acted on as a whole file.
        let truncated = file.diff.as_ref().is_some_and(|d| d.truncated);
        let (stage, unstage, revert) = if truncated {
            (false, false, false)
        } else {
            self.capabilities()
        };
        let item = |act: WindowAct| {
            let view = view.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = view.update(cx, |v, cx| act(v, window, cx));
            }
        };
        let mut menu = menu.item(
            "Comment on This Line",
            item(Box::new(move |v, window, cx| {
                v.open_composer(f, h, Some(line), window, cx)
            })),
        );
        for (allowed, label, action) in [
            (stage, "Stage Hunk", Action::Stage),
            (unstage, "Unstage Hunk", Action::Unstage),
            (revert, "Revert Hunk", Action::Revert),
        ] {
            if allowed {
                menu = menu.item(
                    label,
                    item(Box::new(move |v, _, cx| v.act(action, f, Some(h), cx))),
                );
            }
        }
        let content = diff_line.content.clone();
        let number = diff_line.new_lineno.or(diff_line.old_lineno).unwrap_or(1);
        let path = std::path::PathBuf::from(&self.root).join(&file.change.path);
        menu.separator()
            .item("Copy Line", move |_, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(content.clone()))
            })
            .item(
                "Open File at This Line",
                item(Box::new(move |_, _, cx| {
                    cx.emit(ReviewEvent::OpenFile(path.clone(), Some(number)))
                })),
            )
    }
}

/// A menu item's action on the view.
type WindowAct = Box<dyn Fn(&mut ReviewView, &mut Window, &mut Context<ReviewView>)>;

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: DiffLineKind) -> DiffLine {
        DiffLine {
            kind,
            old_lineno: None,
            new_lineno: None,
            content: String::new(),
            spans: Vec::new(),
        }
    }

    #[test]
    fn clicks_toggle_and_shift_extends_over_changed_lines() {
        use DiffLineKind::*;
        let lines = [
            line(Context),
            line(Removed),
            line(Added),
            line(Context),
            line(Added),
        ];
        let one = toggled(None, &lines, 0, 1, false).unwrap();
        assert_eq!(one.lines.iter().copied().collect::<Vec<_>>(), [1]);
        // Shift-click: every changed line between, context left out.
        let range = toggled(Some(one.clone()), &lines, 0, 4, true).unwrap();
        assert_eq!(range.lines.iter().copied().collect::<Vec<_>>(), [1, 2, 4]);
        // Again on a selected line: unselected; the last one gone, nothing.
        assert!(toggled(Some(one), &lines, 0, 1, false).is_none());
        // Another hunk starts over.
        let other = toggled(Some(range), &lines, 1, 2, false).unwrap();
        assert_eq!((other.hunk, other.lines.len()), (1, 1));
    }
}
