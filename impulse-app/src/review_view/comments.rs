//! Review comments in the list (the Swift `ReviewCommentRow`,
//! `ReviewComposerRow` and the comment parts of `ReviewSurface`): a
//! comment is written on a line (the gutter's + on hover, a hunk's Comment
//! button, or c for the focused hunk), shows under the last line it covers,
//! and floats to the top of its file's card once its lines read
//! differently. Comments can be edited and deleted; one an agent marked
//! addressed (`impulse comments resolve`) shows the agent's note, with
//! Accept (delete it) and Reopen (clear the mark). They're kept per
//! repository in the data directory (`impulse_kit::review_comments::
//! CommentStore`), the same file the Swift app keeps, and read again when
//! that file changes.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    div, prelude::*, px, AnyElement, ClipboardItem, Context, Entity, FontWeight, KeyDownEvent,
    Subscription, Window,
};
use impulse_git::hunks::{DiffHunk, DiffLine, DiffLineKind};
use impulse_kit::chrome::Rgba as KitRgba;
use impulse_kit::review_comments::{self, CommentStore, ReviewComment, Side as CommentSide};

use super::{Layout, ReviewView, Row};
use crate::workbench_chrome::{hsla, icon};

/// Space after the code in a comment card's row, as `ReviewMetrics`.
const CODE_TRAILING: f32 = 12.0;
/// The composer's card (Swift's 136 row less its 4 and 8 margins).
const COMPOSER_CARD: f32 = 124.0;

/// A comment being written, anchored after one line of a hunk.
pub(super) struct Composer {
    pub path: String,
    pub hunk: usize,
    /// The line the composer shows below.
    pub line_index: usize,
    pub side: CommentSide,
    pub line: usize,
    pub end_line: usize,
    pub snippet: String,
    pub(super) input: Entity<TextareaState>,
    _events: Subscription,
}

/// A comment being edited in place.
pub(super) struct Editing {
    pub id: String,
    input: Entity<TextareaState>,
    _events: Subscription,
}

/// Where a comment on line `target` of a hunk lands: the side it's on, its
/// first and last line there, and those lines' text (context included), so
/// it reads as outdated once line `first + k` no longer reads like snippet
/// line k.
#[cfg(test)]
pub(super) fn anchor(lines: &[DiffLine], target: usize) -> (CommentSide, usize, usize, String) {
    anchor_range(lines, &[target], target)
}

/// `anchor` for a comment on `indices` (selected lines, `target` among
/// them): the range of their numbers on `target`'s side, and every line of
/// that range as the snippet, context included, so the comment is outdated
/// when line `first + k` no longer reads like snippet line k.
pub(super) fn anchor_range(
    lines: &[DiffLine],
    indices: &[usize],
    target: usize,
) -> (CommentSide, usize, usize, String) {
    let line = &lines[target];
    let side = if line.kind == DiffLineKind::Removed {
        CommentSide::Old
    } else {
        CommentSide::New
    };
    let number = |l: &DiffLine| match side {
        CommentSide::Old => l.old_lineno.filter(|_| l.kind != DiffLineKind::Added),
        CommentSide::New => l.new_lineno.filter(|_| l.kind != DiffLineKind::Removed),
    };
    let numbers: Vec<usize> = indices
        .iter()
        .filter_map(|&i| lines.get(i))
        .filter_map(number)
        .map(|n| n as usize)
        .filter(|&n| n > 0)
        .collect();
    let first = numbers
        .iter()
        .copied()
        .min()
        .unwrap_or_else(|| number(line).unwrap_or(1) as usize);
    let last = numbers.iter().copied().max().unwrap_or(first);
    let spanned: Vec<&str> = lines
        .iter()
        .filter(|l| number(l).is_some_and(|n| (first..=last).contains(&(n as usize))))
        .map(|l| l.content.as_str())
        .collect();
    (side, first, last, spanned.join("\n"))
}

/// The keys of the comments that show after `line`: its new-side number
/// unless it was removed, its old-side number if it was.
pub(super) fn anchor_keys(line: &DiffLine) -> Vec<(CommentSide, usize)> {
    let mut keys = Vec::new();
    if let (Some(new), false) = (line.new_lineno, line.kind == DiffLineKind::Removed) {
        keys.push((CommentSide::New, new as usize));
    }
    if let (Some(old), true) = (line.old_lineno, line.kind == DiffLineKind::Removed) {
        keys.push((CommentSide::Old, old as usize));
    }
    keys
}

/// The comments whose lines no longer read as they did in this diff.
pub(super) fn outdated_ids(
    comments: &[ReviewComment],
    hunks: Option<&[DiffHunk]>,
) -> HashSet<String> {
    let Some(hunks) = hunks else {
        return HashSet::new();
    };
    let (mut old, mut new) = (HashMap::new(), HashMap::new());
    for line in hunks.iter().flat_map(|h| &h.lines) {
        if let (Some(n), false) = (line.old_lineno, line.kind == DiffLineKind::Added) {
            old.insert(n as usize, line.content.clone());
        }
        if let (Some(n), false) = (line.new_lineno, line.kind == DiffLineKind::Removed) {
            new.insert(n as usize, line.content.clone());
        }
    }
    comments
        .iter()
        .filter(|c| {
            review_comments::is_outdated(
                c,
                if c.side == CommentSide::Old {
                    &old
                } else {
                    &new
                },
            )
        })
        .map(|c| c.id.clone())
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ButtonKind {
    Plain,
    Primary,
    Danger,
}

/// The small text buttons of comment cards and file and hunk headers
/// (`ReviewActionButton`): 22 high, primary outlined, danger red only on
/// hover.
pub(super) fn action_button(
    id: impl Into<gpui_kit::ElementId>,
    title: impl Into<gpui_kit::SharedString>,
    kind: ButtonKind,
    p: &impulse_kit::chrome::ChromePalette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let (hover, danger, text) = (p.hover, p.danger, p.text);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .px(px(8.))
        .h(px(22.))
        .rounded(px(5.))
        .text_size(px(11.))
        .whitespace_nowrap()
        .text_color(hsla(if kind == ButtonKind::Primary {
            p.text
        } else {
            p.text_secondary
        }))
        .when(kind == ButtonKind::Primary, |d| {
            d.border_1().border_color(hsla(p.hairline_strong))
        })
        .hover(move |s| {
            let s = s.bg(hsla(hover));
            s.text_color(hsla(if kind == ButtonKind::Danger {
                danger
            } else {
                text
            }))
        })
        .child(title.into())
}

impl ReviewView {
    fn comments_file(&self) -> std::path::PathBuf {
        CommentStore::file(
            &impulse_kit::paths::data_dir(cfg!(debug_assertions)),
            &self.root,
        )
    }

    /// The repository's comments as saved now.
    fn read_store(&mut self) -> &mut CommentStore {
        self.comments = CommentStore::load(self.comments_file());
        self.comments_stamp = Self::stamp(self.comments.path());
        &mut self.comments
    }

    fn stamp(path: &std::path::Path) -> Option<SystemTime> {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    /// Give each file its comments and which of them are outdated.
    pub(super) fn assign_comments(&mut self) {
        for file in &mut self.files {
            file.comments = self.comments.for_path(&file.change.path);
            file.outdated = outdated_ids(
                &file.comments,
                file.diff.as_ref().map(|d| d.hunks.as_slice()),
            );
        }
    }

    /// The comments were changed elsewhere (an agent marked one
    /// addressed, another window): show them as saved.
    pub fn reload_comments(&mut self, cx: &mut Context<Self>) {
        self.read_store();
        self.assign_comments();
        self.rebuild_rows();
        cx.notify();
    }

    /// Read the comments again when their file changed since.
    pub(super) fn reload_comments_if_changed(&mut self, cx: &mut Context<Self>) {
        if Self::stamp(&self.comments_file()) != self.comments_stamp {
            self.reload_comments(cx);
        }
    }

    /// A comment is being written or edited (background refreshes wait).
    pub(super) fn is_composing(&self) -> bool {
        self.composer.is_some() || self.editing.is_some()
    }

    /// The comments on the files this scope shows. Comments on other files
    /// stay in the repository's store, out of this review's count and
    /// prompt.
    fn scoped_comments(&self) -> Vec<ReviewComment> {
        self.files.iter().flat_map(|f| f.comments.clone()).collect()
    }

    /// The rows a file's comments and composer add to its card: outdated
    /// comments first, `after` gives those that follow a line.
    pub(super) fn outdated_rows(&self, f: usize) -> Vec<Row> {
        let file = &self.files[f];
        let outdated: Vec<Row> = file
            .comments
            .iter()
            .filter(|c| file.outdated.contains(&c.id))
            .map(|c| Row::Comment(f, c.id.clone()))
            .collect();
        if outdated.is_empty() {
            return outdated;
        }
        std::iter::once(Row::OutdatedTitle(f))
            .chain(outdated)
            .collect()
    }

    /// Comments that end on `line` (and the composer, when it's there).
    pub(super) fn rows_after(
        &self,
        f: usize,
        hunk: usize,
        index: usize,
        line: &DiffLine,
    ) -> Vec<Row> {
        let file = &self.files[f];
        let keys = anchor_keys(line);
        let mut rows: Vec<Row> = file
            .comments
            .iter()
            .filter(|c| !file.outdated.contains(&c.id) && keys.contains(&(c.side, c.end_line)))
            .map(|c| Row::Comment(f, c.id.clone()))
            .collect();
        if self
            .composer
            .as_ref()
            .is_some_and(|c| c.path == file.change.path && c.hunk == hunk && c.line_index == index)
        {
            rows.push(Row::Composer(f));
        }
        rows
    }

    /// Write a comment after line `line` of a hunk (`None`: its last
    /// changed line).
    pub(super) fn open_composer(
        &mut self,
        f: usize,
        hunk: usize,
        line: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = self.files.get(f) else {
            return;
        };
        let Some(lines) = file
            .diff
            .as_ref()
            .and_then(|d| d.hunks.get(hunk))
            .map(|h| &h.lines)
            .filter(|l| !l.is_empty())
        else {
            return;
        };
        let selected = self.selected_lines(f, hunk).unwrap_or_default();
        // For the hunk: the selection's last line, else its last changed line.
        let target = line
            .or_else(|| selected.last().copied())
            .or_else(|| lines.iter().rposition(|l| l.kind != DiffLineKind::Context))
            .unwrap_or(lines.len() - 1)
            .min(lines.len() - 1);
        // A selection including the line widens the comment to the range.
        let indices = if selected.contains(&target) {
            selected
        } else {
            vec![target]
        };
        let (side, first, last, snippet) = anchor_range(lines, &indices, target);
        let target = indices.last().copied().unwrap_or(target);
        let path = file.change.path.clone();
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 10)
                .placeholder("Leave a comment for the agent or yourself…")
        });
        let events = cx.subscribe_in(&input, window, |view, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter {
                secondary: true, ..
            } = event
            {
                view.save_composer(window, cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.composer = Some(Composer {
            path,
            hunk,
            line_index: target,
            side,
            line: first,
            end_line: last,
            snippet,
            input,
            _events: events,
        });
        self.editing = None;
        self.focused = Some((f, hunk));
        self.rebuild_rows();
        cx.notify();
    }

    pub(super) fn save_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composer) = self.composer.take() else {
            return;
        };
        let text = composer.input.read(cx).value().trim().to_string();
        if !text.is_empty() {
            self.read_store().add(ReviewComment::new(
                composer.path,
                composer.side,
                composer.line,
                composer.end_line,
                composer.snippet,
                text,
            ));
            self.comments_stamp = Self::stamp(self.comments.path());
        }
        self.assign_comments();
        self.rebuild_rows();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn cancel_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer = None;
        self.rebuild_rows();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn edit_comment(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self
            .comments
            .comments
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.text.clone())
        else {
            return;
        };
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 10));
        input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        let events = cx.subscribe_in(&input, window, |view, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter {
                secondary: true, ..
            } = event
            {
                view.save_edit(window, cx);
            }
        });
        self.editing = Some(Editing {
            id,
            input,
            _events: events,
        });
        self.composer = None;
        self.rebuild_rows();
        cx.notify();
    }

    fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.editing.as_ref() else {
            return;
        };
        let text = editing.input.read(cx).value().trim().to_string();
        // An empty edit is no edit: keep typing or cancel.
        if text.is_empty() {
            return;
        }
        let id = editing.id.clone();
        self.editing = None;
        self.read_store().update(&id, &text);
        self.comments_stamp = Self::stamp(self.comments.path());
        self.assign_comments();
        self.rebuild_rows();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self.rebuild_rows();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// An agent's "addressed" mark: open the comment again.
    fn reopen_comment(&mut self, id: &str, cx: &mut Context<Self>) {
        self.read_store().resolve(id, None);
        self.comments_stamp = Self::stamp(self.comments.path());
        self.assign_comments();
        self.rebuild_rows();
        cx.notify();
    }

    /// Delete, or Accept an addressed comment.
    fn delete_comment(&mut self, id: &str, cx: &mut Context<Self>) {
        self.read_store().remove(id);
        self.comments_stamp = Self::stamp(self.comments.path());
        self.assign_comments();
        self.rebuild_rows();
        cx.notify();
    }

    /// The view's comments as a prompt for an agent; `None` (and a note
    /// saying so) when there are none.
    fn prompt_for_agent(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let prompt = review_comments::prompt(&self.scoped_comments());
        if prompt.is_empty() {
            self.message = Some("There are no review comments in this view yet.".into());
            cx.notify();
            return None;
        }
        Some(prompt)
    }

    fn copy_prompt(&mut self, cx: &mut Context<Self>) {
        let scoped = self.scoped_comments();
        let prompt = review_comments::prompt(&scoped);
        if prompt.is_empty() {
            self.message = Some("There are no review comments in this view yet.".into());
        } else {
            cx.write_to_clipboard(ClipboardItem::new_string(prompt));
            self.message = None;
        }
        cx.notify();
    }

    /// Delete every comment in the repository, after asking.
    fn confirm_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let total = self.read_store().comments.len();
        if total == 0 {
            return;
        }
        let elsewhere = total.saturating_sub(self.scoped_comments().len());
        let message = format!(
            "{total} comment{} will be removed{}",
            if total == 1 { "" } else { "s" },
            if elsewhere > 0 {
                format!(", including {elsewhere} on files this view doesn't show.")
            } else {
                ".".into()
            }
        );
        let view = cx.entity().downgrade();
        crate::overlays::open_dialog(window, cx, move |dialog, _, _| {
            let (ok, confirm) = (view.clone(), view.clone());
            dialog
                .title("Delete every review comment in this repository?")
                .w(px(440.))
                .child(div().child(message.clone()))
                .on_ok(move |_, window, cx| {
                    window.close_dialog(cx);
                    let _ = ok.update(cx, |view, cx| view.clear_comments(cx));
                    false
                })
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("comments-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("comments-delete")
                                .label("Delete")
                                .danger()
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = confirm.update(cx, |view, cx| view.clear_comments(cx));
                                }),
                        ),
                )
        });
    }

    fn clear_comments(&mut self, cx: &mut Context<Self>) {
        self.read_store().remove_all();
        self.comments_stamp = Self::stamp(self.comments.path());
        self.assign_comments();
        self.rebuild_rows();
        cx.notify();
    }

    // MARK: Drawing

    /// The header's comments menu: Copy Comments as Prompt and Delete All,
    /// with this view's count.
    pub(super) fn comments_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.files.iter().map(|f| f.comments.len()).sum::<usize>();
        let total = self.comments.comments.len();
        let view = cx.entity().downgrade();
        let mut button = Button::new("review-comments")
            .ghost()
            .compact()
            .icon(gpui_kit::component::Icon::empty().path("icons/message-square.svg"))
            .tooltip("Review comments");
        if count > 0 {
            button = button.label(count.to_string());
        }
        button.dropdown_menu(move |menu: PopupMenu, window, cx| {
            let (copy, clear, send, mark) =
                (view.clone(), view.clone(), view.clone(), view.clone());
            let menu = crate::agent_send::agent_menu_items(
                menu,
                "Send Comments to",
                count > 0,
                std::rc::Rc::new(move |_, cx| {
                    send.update(cx, |view, cx| view.prompt_for_agent(cx))
                        .ok()
                        .flatten()
                }),
                Some(std::rc::Rc::new(move |cx| {
                    if let Some(root) = mark.upgrade().map(|v| v.read(cx).root.clone()) {
                        crate::agent_send::mark_reviewed(&root);
                    }
                })),
                window,
                cx,
            );
            menu.item(
                PopupMenuItem::new("Copy Comments as Prompt")
                    .disabled(count == 0)
                    .on_click(move |_, _, cx| {
                        let _ = copy.update(cx, |view, cx| view.copy_prompt(cx));
                    }),
            )
            .separator()
            .item(
                PopupMenuItem::new("Delete All Comments in Repository…")
                    .disabled(total == 0)
                    .on_click(move |_, window, cx| {
                        let _ = clear.update(cx, |view, cx| view.confirm_clear(window, cx));
                    }),
            )
        })
    }

    fn comment_indent(&self) -> f32 {
        if self.layout == Layout::Split {
            64.
        } else {
            100.
        }
    }

    /// A card holding `content` (a comment or the composer), in its file's
    /// card.
    fn comment_card(&self, indent: f32, bar: KitRgba, content: gpui_kit::Div) -> gpui_kit::Div {
        let p = &self.palette;
        div()
            .w_full()
            .border_l_1()
            .border_r_1()
            .border_color(hsla(self.colors.border))
            .bg(hsla(self.colors.page))
            .pt(px(4.))
            .pb(px(8.))
            .pl(px(indent))
            .pr(px(CODE_TRAILING))
            .child(
                div()
                    .relative()
                    .py(px(8.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .bg(hsla(p.panel))
                    .border_1()
                    .border_color(hsla(p.hairline_strong))
                    .overflow_hidden()
                    .child(content.flex().flex_col())
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .bottom_0()
                            .w(px(3.))
                            // Inside the card's corners, which don't clip it.
                            .rounded_l(px(5.))
                            .bg(hsla(bar)),
                    ),
            )
    }

    pub(super) fn outdated_title_row(&self) -> AnyElement {
        div()
            .w_full()
            .h(px(26.))
            .flex()
            .items_end()
            .pb(px(4.))
            .pl(px(12.))
            .border_l_1()
            .border_r_1()
            .border_color(hsla(self.colors.border))
            .bg(hsla(self.colors.page))
            .text_size(px(11.))
            .text_color(hsla(self.palette.text_tertiary))
            .child("Outdated comments: their lines changed or aren't in this diff")
            .into_any_element()
    }

    pub(super) fn comment_row(&self, f: usize, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette.clone();
        let Some(file) = self.files.get(f) else {
            return div().into_any_element();
        };
        let Some(comment) = file.comments.iter().find(|c| c.id == id).cloned() else {
            return div().into_any_element();
        };
        let outdated = file.outdated.contains(id);
        let editing = self
            .editing
            .as_ref()
            .filter(|e| e.id == id)
            .map(|e| e.input.clone());
        let range = if comment.end_line > comment.line {
            format!("lines {}–{}", comment.line, comment.end_line)
        } else {
            format!("line {}", comment.line)
        };
        let title = format!(
            "{}{range}",
            if comment.side == CommentSide::Old {
                "removed "
            } else {
                ""
            }
        );
        let mut buttons = div().flex().items_center().gap(px(4.));
        if editing.is_some() {
            buttons = buttons
                .child(
                    action_button("comment-cancel", "Cancel", ButtonKind::Plain, &p)
                        .on_click(cx.listener(|view, _, window, cx| view.cancel_edit(window, cx))),
                )
                .child(
                    action_button("comment-save", "Save", ButtonKind::Primary, &p)
                        .on_click(cx.listener(|view, _, window, cx| view.save_edit(window, cx))),
                );
        } else if comment.resolution.is_some() {
            let (reopen, accept) = (comment.id.clone(), comment.id.clone());
            buttons = buttons
                .child(
                    action_button(("comment-reopen", f), "Reopen", ButtonKind::Plain, &p).on_click(
                        cx.listener(move |view, _, _, cx| view.reopen_comment(&reopen, cx)),
                    ),
                )
                .child(
                    action_button(("comment-accept", f), "Accept", ButtonKind::Primary, &p)
                        .on_click(
                            cx.listener(move |view, _, _, cx| view.delete_comment(&accept, cx)),
                        ),
                );
        } else {
            let (edit, delete) = (comment.id.clone(), comment.id.clone());
            buttons = buttons
                .child(
                    action_button(("comment-edit", f), "Edit", ButtonKind::Plain, &p).on_click(
                        cx.listener(move |view, _, window, cx| {
                            view.edit_comment(edit.clone(), window, cx)
                        }),
                    ),
                )
                .child(
                    action_button(("comment-delete", f), "Delete", ButtonKind::Danger, &p)
                        .on_click(
                            cx.listener(move |view, _, _, cx| view.delete_comment(&delete, cx)),
                        ),
                );
        }
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(hsla(p.text_tertiary))
                    .child(title),
            )
            .child(div().flex_1())
            .child(buttons);
        let body: AnyElement = match editing {
            Some(input) => div()
                .id("comment-editing")
                .pt(px(3.))
                .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        view.cancel_edit(window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(Textarea::new(&input).appearance(false))
                .into_any_element(),
            None => div()
                .flex()
                .flex_col()
                .pt(px(3.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(hsla(p.text))
                        .child(comment.text.clone()),
                )
                .children(comment.resolution.as_ref().map(|resolution| {
                    div()
                        .mt(px(6.))
                        .p(px(6.))
                        .rounded(px(4.))
                        .bg(hsla(p.raised))
                        .text_size(px(11.5))
                        .text_color(hsla(p.text_secondary))
                        .child(format!("{}: {}", resolution.by, resolution.note))
                }))
                .into_any_element(),
        };
        let bar = if outdated {
            p.text_tertiary
        } else if comment.resolution.is_some() {
            p.success
        } else {
            p.accent
        };
        let indent = if outdated { 12. } else { self.comment_indent() };
        self.comment_card(indent, bar, div().child(header).child(body))
            .when(outdated, |d| d.opacity(0.85))
            .into_any_element()
    }

    pub(super) fn composer_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette.clone();
        let Some(composer) = self.composer.as_ref() else {
            return div().into_any_element();
        };
        let several = composer.end_line > composer.line;
        let range = if several {
            format!("{}–{}", composer.line, composer.end_line)
        } else {
            composer.line.to_string()
        };
        let lines = if several { "lines" } else { "line" };
        let save_key = if cfg!(target_os = "macos") {
            "⌘↩"
        } else {
            "Ctrl+↩"
        };
        let what = if composer.side == CommentSide::Old {
            format!("Removed {lines} {range}")
        } else {
            format!("{}{} {range}", lines[..1].to_uppercase(), &lines[1..])
        };
        // Swift's composer row is a fixed 136 (`ReviewMetrics.composerHeight`):
        // a 124 card whose editor scrolls, its text 5 in from the padding.
        let content = div()
            .h(px(COMPOSER_CARD - 18.))
            .child(
                div()
                    .id("composer")
                    .flex_1()
                    .min_h_0()
                    .ml(px(-6.))
                    .mt(px(-3.))
                    .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            view.cancel_composer(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(Textarea::new(&composer.input).appearance(false).h_full()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pt(px(6.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(hsla(p.text_tertiary))
                            .child(format!("{what} · {save_key} to save")),
                    )
                    .child(div().flex_1())
                    .child(
                        action_button("composer-cancel", "Cancel", ButtonKind::Plain, &p).on_click(
                            cx.listener(|view, _, window, cx| view.cancel_composer(window, cx)),
                        ),
                    )
                    .child(
                        action_button("composer-save", "Comment", ButtonKind::Primary, &p)
                            .on_click(
                                cx.listener(|view, _, window, cx| view.save_composer(window, cx)),
                            ),
                    ),
            );
        self.comment_card(self.comment_indent(), p.accent, content)
            .into_any_element()
    }

    /// The hunk header's Comment button.
    pub(super) fn hunk_comment_button(
        &self,
        f: usize,
        h: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        action_button(
            ("hunk-comment", h),
            "Comment",
            ButtonKind::Plain,
            &self.palette,
        )
        .tooltip(super::help("Comment (c)"))
        .on_click(cx.listener(move |view, _, window, cx| {
            cx.stop_propagation();
            view.open_composer(f, h, None, window, cx)
        }))
    }

    /// The + that shows over a line's gutter on hover.
    pub(super) fn gutter_comment_button(
        &self,
        f: usize,
        h: usize,
        l: usize,
        side: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = &self.palette;
        div()
            .id(("comment-line", l * 2 + side))
            .absolute()
            .left(px(3.))
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .opacity(0.)
            .group_hover(super::LINE_GROUP, |s| s.opacity(1.))
            .on_click(cx.listener(move |view, _, window, cx| {
                cx.stop_propagation();
                view.open_composer(f, h, Some(l), window, cx)
            }))
            .child(
                div()
                    .size(px(15.))
                    .rounded(px(4.))
                    .bg(hsla(p.accent))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("plus", 11., p.text_on_accent)),
            )
    }

    /// The navigator's count of a file's comments.
    pub(super) fn navigator_count(&self, f: usize) -> Option<impl IntoElement> {
        let count = self.files.get(f)?.comments.len();
        let p = &self.palette;
        (count > 0).then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(3.))
                .child(icon("message-square", 10., p.accent))
                .child(
                    div()
                        .font_family("JetBrains Mono")
                        .text_size(px(10.))
                        .text_color(hsla(p.accent))
                        .font_weight(FontWeight::NORMAL)
                        .child(count.to_string()),
                )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: DiffLineKind, old: Option<u32>, new: Option<u32>, content: &str) -> DiffLine {
        DiffLine {
            kind,
            old_lineno: old,
            new_lineno: new,
            content: content.into(),
            spans: Vec::new(),
        }
    }

    fn hunk() -> Vec<DiffLine> {
        vec![
            line(DiffLineKind::Context, Some(4), Some(4), "fn a() {"),
            line(DiffLineKind::Removed, Some(5), None, "    old();"),
            line(DiffLineKind::Added, None, Some(5), "    new();"),
            line(DiffLineKind::Context, Some(6), Some(6), "}"),
        ]
    }

    #[test]
    fn a_comment_anchors_on_its_lines_side() {
        let lines = hunk();
        assert_eq!(
            anchor(&lines, 2),
            (CommentSide::New, 5, 5, "    new();".to_string())
        );
        assert_eq!(
            anchor(&lines, 1),
            (CommentSide::Old, 5, 5, "    old();".to_string())
        );
        assert_eq!(anchor(&lines, 3), (CommentSide::New, 6, 6, "}".to_string()));
        // A selection: its range on the target's side, context included.
        assert_eq!(
            anchor_range(&lines, &[2, 3], 2),
            (CommentSide::New, 5, 6, "    new();\n}".to_string())
        );
        assert_eq!(anchor_keys(&lines[1]), vec![(CommentSide::Old, 5)]);
        assert_eq!(anchor_keys(&lines[2]), vec![(CommentSide::New, 5)]);
        assert_eq!(anchor_keys(&lines[0]), vec![(CommentSide::New, 4)]);
    }

    #[test]
    fn comments_go_outdated_when_their_lines_change() {
        let hunks = vec![DiffHunk {
            old_start: 4,
            old_lines: 3,
            new_start: 4,
            new_lines: 3,
            header: "@@ -4,3 +4,3 @@".into(),
            lines: hunk(),
        }];
        let current = ReviewComment::new("a.rs", CommentSide::New, 5, 5, "    new();", "Hm.");
        let removed = ReviewComment::new("a.rs", CommentSide::Old, 5, 5, "    old();", "Why?");
        let moved = ReviewComment::new("a.rs", CommentSide::New, 5, 5, "    newer();", "Hm.");
        let gone = ReviewComment::new("a.rs", CommentSide::New, 40, 40, "x", "Hm.");
        let outdated = outdated_ids(
            &[
                current.clone(),
                removed.clone(),
                moved.clone(),
                gone.clone(),
            ],
            Some(&hunks),
        );
        assert!(!outdated.contains(&current.id));
        assert!(!outdated.contains(&removed.id));
        assert!(outdated.contains(&moved.id));
        assert!(outdated.contains(&gone.id));
        assert!(outdated_ids(&[moved], None).is_empty());
    }
}
