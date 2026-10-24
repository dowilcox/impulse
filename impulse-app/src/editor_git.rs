//! The editor's git layer (the Swift editor's `web/editor.js` git section):
//! change marks in the gutter against the file's index version, following
//! typing; a click on one opens a peek with the original lines and Revert,
//! Stage, Diff and Review; the inline blame after the cursor's line once
//! it rests; and merge conflicts tinted, with Accept Current, Incoming or
//! Both from a bar over the editor. The texts are worked out by
//! `impulse_kit::git::editor_marks`; the marks are painted over the editor
//! from its own layout (GPUI Kit's editor has no gutter decorations).

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    RangeDecoration, RangeDecorationCollection, RangeDecorationStyle, TextDecoration,
    TextDecorationCollection,
};
use gpui_kit::component::{ActiveTheme as _, RopeExt as _, Sizable as _};
use gpui_kit::{
    canvas, div, fill, point, prelude::*, px, size, AnyElement, App, Bounds, Context,
    DispatchPhase, EventEmitter, Global, Hsla, MouseDownEvent, Pixels, Subscription, Task, Window,
};
use impulse_git::git_blame::BlameLine;
use impulse_kit::git::editor_marks::{self, ConflictBlock, LineHunk, MarkKind, Resolution};
use impulse_kit::text_file::LineEnding;

use crate::editor_tab::EditorTab;
use crate::repository_state::RepositoryState;

/// Typing settles before the marks follow it.
const DIFF_DELAY: Duration = Duration::from_millis(150);
/// The cursor rests this long before its line's blame shows.
const BLAME_DELAY: Duration = Duration::from_millis(450);
/// The repository changes in bursts (a commit, a checkout).
const BASE_DELAY: Duration = Duration::from_millis(300);

/// The theme's git colors, for the marks.
#[derive(Clone, Copy)]
pub(crate) struct GitColors {
    pub added: Hsla,
    pub modified: Hsla,
    pub deleted: Hsla,
}

impl Global for GitColors {}

/// Something the workbench does for the editor: "stage" or "review" the
/// change at `line` (1-based), "commit:<sha>" to show a line's commit, and
/// "conflicts-resolved" when the last conflict in a file is gone.
pub(crate) struct EditorGitAction {
    pub action: String,
    pub line: usize,
}

impl EventEmitter<EditorGitAction> for EditorTab {}

#[derive(Default)]
pub(crate) struct GitLayer {
    /// The file's index version (`\n` line ends); `None`: no marks.
    base: Option<String>,
    /// The repository root, once known, and whether it's trusted.
    root: Option<String>,
    trusted: bool,
    hunks: Vec<LineHunk>,
    /// Who last changed each line of the saved file, and that file's text
    /// (blame says nothing once the buffer differs from it).
    blame: HashMap<usize, BlameLine>,
    blame_for: Option<String>,
    /// The line whose blame shows.
    blame_line: Option<usize>,
    /// The peeked hunk, by its start line.
    peek: Option<usize>,
    conflicts: Vec<ConflictBlock>,
    conflict_marks: Option<RangeDecorationCollection>,
    /// Bracket pair colors (kept here as they follow the same edits).
    brackets: Option<TextDecorationCollection>,
    _base: Option<Task<()>>,
    _diff: Option<Task<()>>,
    _blame: Option<Task<()>>,
    _repository: Option<Subscription>,
}

impl EditorTab {
    /// Read the file's git base and blame again (opened, saved, or the
    /// repository changed).
    pub(crate) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.untitled.is_some() || self.path.as_os_str().is_empty() {
            return;
        }
        let path = self.path.clone();
        let (trusted, root) = (self.git.trusted, self.git.root.clone());
        self.git._base = Some(cx.spawn(async move |tab, cx| {
            cx.background_executor().timer(BASE_DELAY).await;
            let read = cx.background_executor().spawn(async move {
                let file = path.to_string_lossy().into_owned();
                let base = impulse_git::base_content(&file).map(|b| LineEnding::normalize(&b));
                let root = root.or_else(|| {
                    impulse_git::repo_root(&path.parent()?.to_string_lossy())
                        .map(|r| r.trim_end_matches('/').to_string())
                });
                let disk =
                    impulse_kit::text_file::read(&path).map(|c| LineEnding::normalize(&c.text));
                // Git reads run as the repository's trust allows.
                let blame = match (&root, base.is_some()) {
                    (Some(root), true) => RepositoryState::read_git(
                        root,
                        trusted,
                        &["blame", "--porcelain", "--", &file],
                    )
                    .map(|text| impulse_git::git_blame::parse_porcelain(&text)),
                    _ => None,
                };
                (base, root, disk, blame)
            });
            let (base, root, disk, blame) = read.await;
            let _ = tab.update(cx, |tab, cx| {
                tab.git.base = base;
                if tab.git.root.is_none() {
                    if let Some(root) = root.clone() {
                        tab.watch_repository(&root, cx);
                    }
                }
                tab.git.root = root;
                tab.git.blame = blame.unwrap_or_default();
                tab.git.blame_for = disk;
                tab.update_git_marks(cx);
            });
        }));
    }

    /// Follow the repository: its index, HEAD and conflicts.
    fn watch_repository(&mut self, root: &str, cx: &mut Context<Self>) {
        let state = crate::repository_state::RepositoryStore::state(root, cx);
        self.git.trusted = state.read(cx).trusted;
        self.git._repository = Some(cx.observe(&state, |tab, state, cx| {
            tab.git.trusted = state.read(cx).trusted;
            tab.refresh_git(cx);
        }));
    }

    /// The buffer changed: the marks follow once typing settles.
    pub(crate) fn git_text_changed(&mut self, cx: &mut Context<Self>) {
        self.git.blame_line = None;
        self.git._diff = Some(cx.spawn(async move |tab, cx| {
            cx.background_executor().timer(DIFF_DELAY).await;
            let _ = tab.update(cx, |tab, cx| tab.update_git_marks(cx));
        }));
    }

    fn update_git_marks(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        self.git.hunks = self
            .git
            .base
            .as_deref()
            .map(|base| editor_marks::line_hunks(base, &text))
            .unwrap_or_default();
        if let Some(start) = self.git.peek {
            if !self.git.hunks.iter().any(|h| h.new_start == start) {
                self.git.peek = None;
            }
        }
        self.paint_brackets(&text, cx);
        let conflicts = editor_marks::find_conflicts(&text);
        let resolved = !self.git.conflicts.is_empty() && conflicts.is_empty();
        self.git.conflicts = conflicts;
        self.paint_conflicts(&text, cx);
        if resolved {
            cx.emit(EditorGitAction {
                action: "conflicts-resolved".into(),
                line: 0,
            });
        }
        cx.notify();
    }

    /// Color bracket pairs by depth (`bracket_pair_colorization`, Monaco's
    /// three colors), unmatched ones red.
    fn paint_brackets(&mut self, text: &str, cx: &mut Context<Self>) {
        use impulse_kit::bracket_pairs::{self, Color};
        let on = crate::settings_store::SettingsStore::get(cx).bracket_pair_colorization;
        if !on || text.len() > 2 * 1024 * 1024 {
            if let Some(marks) = self.git.brackets.as_ref() {
                marks.clear(cx);
            }
            return;
        }
        let hex = |h: &str| crate::workbench_chrome::hsla(impulse_kit::chrome::Rgba::from_hex(h));
        let levels = if cx.theme().mode.is_dark() {
            bracket_pairs::DARK
        } else {
            bracket_pairs::LIGHT
        }
        .map(hex);
        let unexpected = hex(bracket_pairs::UNEXPECTED);
        let decorations: Vec<TextDecoration> = bracket_pairs::colorize(text, &self.language)
            .into_iter()
            .map(|(range, color)| {
                let color = match color {
                    Color::Level(depth) => levels[depth % 3],
                    Color::Unexpected => unexpected,
                };
                TextDecoration::new(
                    range,
                    gpui_kit::HighlightStyle {
                        color: Some(color),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let marks = self.git.brackets.get_or_insert_with(|| {
            self.state.update(cx, |state, cx| {
                state.create_decorations_collection(Vec::new(), cx)
            })
        });
        marks.set(decorations, cx);
    }

    /// Tint each conflict's two sides (current green, incoming blue).
    fn paint_conflicts(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.git.conflicts.is_empty() && self.git.conflict_marks.is_none() {
            return;
        }
        let theme = cx.theme();
        let (current, incoming, marker) = (
            theme.green.opacity(0.14),
            theme.blue.opacity(0.14),
            theme.muted_foreground.opacity(0.12),
        );
        let starts = line_starts(text);
        let lines = |from: usize, to: usize| -> std::ops::Range<usize> {
            let at = |line: usize| starts.get(line - 1).copied().unwrap_or(text.len());
            at(from)..at(to + 1)
        };
        let mut decorations = Vec::new();
        for block in &self.git.conflicts {
            let current_end = block.base.unwrap_or(block.middle);
            let fill = |range, color| {
                RangeDecoration::new(range)
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(color)
            };
            decorations.push(fill(lines(block.start, block.start), marker));
            decorations.push(fill(lines(block.start + 1, current_end - 1), current));
            decorations.push(fill(lines(current_end, block.middle), marker));
            decorations.push(fill(lines(block.middle + 1, block.end - 1), incoming));
            decorations.push(fill(lines(block.end, block.end), marker));
        }
        let marks = self.git.conflict_marks.get_or_insert_with(|| {
            self.state.update(cx, |state, cx| {
                state.create_range_decorations_collection(Vec::new(), cx)
            })
        });
        marks.set(
            decorations
                .into_iter()
                .filter(|d| !d.range().is_empty())
                .collect(),
            cx,
        );
    }

    /// The cursor moved: its line's blame shows once it rests.
    pub(crate) fn git_cursor_moved(&mut self, cx: &mut Context<Self>) {
        if self.git.blame_line.take().is_some() {
            cx.notify();
        }
        if self.git.blame.is_empty() {
            return;
        }
        self.git._blame = Some(cx.spawn(async move |tab, cx| {
            cx.background_executor().timer(BLAME_DELAY).await;
            let _ = tab.update(cx, |tab, cx| {
                let line = tab.cursor(cx).0 as usize;
                // Blame describes the saved file, and not lines changed
                // since.
                let saved = tab.git.blame_for.as_deref() == Some(tab.text(cx).as_str());
                let changed = editor_marks::hunk_at_line(&tab.git.hunks, line).is_some();
                let committed = tab
                    .git
                    .blame
                    .get(&line)
                    .is_some_and(|b| !b.is_uncommitted());
                if saved && !changed && committed {
                    tab.git.blame_line = Some(line);
                    cx.notify();
                }
            });
        }));
    }

    /// Put the peeked hunk's base lines back (one undoable edit).
    fn revert_hunk(&mut self, start: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hunk) = self
            .git
            .hunks
            .iter()
            .find(|h| h.new_start == start)
            .cloned()
        else {
            return;
        };
        let text = self.text(cx);
        let (range, replacement) = editor_marks::revert_edit(&text, &hunk);
        self.replace_range(range, &replacement, window, cx);
        self.git.peek = None;
        cx.notify();
    }

    /// Keep one side of the conflict at the cursor, or both.
    pub(crate) fn resolve_conflict(
        &mut self,
        choice: Resolution,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.text(cx);
        let Some(block) = self.conflict_at_cursor(cx) else {
            return;
        };
        let (range, replacement) = editor_marks::resolve_edit(&text, &block, choice);
        self.replace_range(range, &replacement, window, cx);
    }

    /// The conflict holding the cursor, else the next one (else the first).
    fn conflict_at_cursor(&self, cx: &App) -> Option<ConflictBlock> {
        let line = self.cursor(cx).0 as usize;
        let blocks = &self.git.conflicts;
        blocks
            .iter()
            .find(|b| (b.start..=b.end).contains(&line))
            .or_else(|| blocks.iter().find(|b| b.start > line))
            .or(blocks.first())
            .cloned()
    }

    /// Go to the next (1) or previous (-1) conflict.
    pub(crate) fn go_to_conflict(
        &mut self,
        step: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line = self.cursor(cx).0 as usize;
        let blocks = &self.git.conflicts;
        let target = if step > 0 {
            blocks.iter().find(|b| b.start > line).or(blocks.first())
        } else {
            blocks
                .iter()
                .rev()
                .find(|b| b.start < line)
                .or(blocks.last())
        };
        if let Some(block) = target {
            let position =
                gpui_kit::component::input::Position::new(block.start.saturating_sub(1) as u32, 0);
            self.go_to(position, window, cx);
        }
    }

    /// The marks, the blame and the conflict bar over `editor`; the peek
    /// under it.
    pub(crate) fn wrap_git(&self, editor: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.try_global::<GitColors>().copied();
        let state = self.state.clone();
        let hunks = self.git.hunks.clone();
        let blame = self.git.blame_line.and_then(|line| {
            let info = self.git.blame.get(&line)?;
            let ago = SystemTime::now()
                .duration_since(info.author_time)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            Some((
                line,
                editor_marks::blame_text(&info.author, ago, &info.summary),
            ))
        });
        let ghost = cx.theme().muted_foreground.opacity(0.7);
        let margin = {
            let settings = crate::settings_store::SettingsStore::get(cx);
            settings
                .show_right_margin
                .then_some(settings.right_margin_position.max(1) as usize)
        };
        let margin_color = cx.theme().border;
        let font = self.font_for_blame(cx);
        let tab = cx.entity().downgrade();
        let sticky = {
            let settings = crate::settings_store::SettingsStore::get(cx);
            settings.sticky_scroll.then(|| StickyStyle {
                font: gpui_kit::font(settings.font_family.clone()),
                size: px(settings.font_size as f32),
                tab_size: self.tab_size,
                background: cx.theme().background,
                text: cx.theme().foreground,
                number: cx.theme().muted_foreground,
                border: cx.theme().border,
            })
        };
        let overlay = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let Some(colors) = colors else {
                    return;
                };
                let state = state.read(cx);
                let text = state.text().clone();
                let sticky_layout = sticky
                    .as_ref()
                    .and_then(|style| sticky_layout(state, &text, style.tab_size));
                // The right margin, at a column measured on a visible line.
                if let (Some(column), Some(rows)) = (margin, state.visible_row_range()) {
                    let measured = rows.clone().find_map(|row| {
                        if row >= text.lines_len() || text.line_len(row) == 0 {
                            return None;
                        }
                        let start = text.line_start_offset(row);
                        let first = text.slice(start..start + text.line_len(row)).to_string();
                        let width = first.chars().next()?.len_utf8();
                        let at = state.range_to_bounds(&(start..start + width))?;
                        Some((at.origin.x, at.size.width))
                    });
                    if let Some((left, width)) = measured {
                        let x = left + width * column as f32;
                        if x > bounds.left() && x < bounds.right() {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(x, bounds.top()),
                                    size(px(1.), bounds.size.height),
                                ),
                                margin_color,
                            ));
                        }
                    }
                }
                let line_offset = |line: usize| -> Option<usize> {
                    let row = line.checked_sub(1)?;
                    (row < text.lines_len()).then(|| text.line_start_offset(row))
                };
                let mut bars: Vec<(Bounds<Pixels>, usize)> = Vec::new();
                for hunk in &hunks {
                    let (color, lines) = match hunk.kind() {
                        MarkKind::Added => (colors.added, hunk.marked_lines()),
                        MarkKind::Modified => (colors.modified, hunk.marked_lines()),
                        MarkKind::Deleted => (colors.deleted, hunk.marked_lines()),
                    };
                    let first = lines
                        .clone()
                        .find_map(|l| line_offset(l).and_then(|o| state.range_to_bounds(&(o..o))));
                    let last = lines
                        .clone()
                        .rev()
                        .find_map(|l| line_offset(l).and_then(|o| state.range_to_bounds(&(o..o))));
                    let (Some(first), Some(last)) = (first, last) else {
                        continue;
                    };
                    let x = first.origin.x - px(7.);
                    let bar = if hunk.kind() == MarkKind::Deleted {
                        // A small wedge where the lines were, under the line.
                        Bounds::new(
                            point(x - px(1.), last.bottom() - px(3.)),
                            size(px(6.), px(6.)),
                        )
                    } else {
                        Bounds::from_corners(
                            point(x, first.top()),
                            point(x + px(3.), last.bottom()),
                        )
                    };
                    window.paint_quad(fill(bar, color));
                    bars.push((bar, hunk.new_start));
                }
                if let Some((line, label)) = &blame {
                    if let Some(end) = line_offset(*line).map(|o| {
                        let row = line - 1;
                        o + text.line_len(row)
                    }) {
                        if let Some(at) = state.range_to_bounds(&(end..end)) {
                            let run = gpui_kit::TextRun {
                                len: label.len(),
                                font: font.clone(),
                                color: ghost,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let shaped = window.text_system().shape_line(
                                label.clone().into(),
                                font_size(at),
                                &[run],
                                None,
                            );
                            let origin = point(at.origin.x + px(28.), at.origin.y);
                            let _ = shaped.paint(
                                origin,
                                at.size.height,
                                gpui_kit::TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    }
                }
                let pinned = match (&sticky, sticky_layout) {
                    (Some(style), Some(layout)) => paint_sticky(layout, bounds, style, window, cx),
                    _ => Vec::new(),
                };
                // A click on a mark opens its peek, one on a sticky line goes
                // there (before the editor sees the click).
                let tab = tab.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    if let Some((_, row)) = pinned.iter().find(|(r, _)| r.contains(&event.position))
                    {
                        let row = *row;
                        let _ = tab.update(cx, |tab, cx| {
                            let position = gpui_kit::component::input::Position::new(row as u32, 0);
                            tab.go_to(position, window, cx);
                        });
                        cx.stop_propagation();
                        return;
                    }
                    let hit = bars.iter().find(|(bar, _)| {
                        let wide = Bounds::from_corners(
                            point(bar.left() - px(4.), bar.top()),
                            point(bar.right() + px(4.), bar.bottom()),
                        );
                        wide.contains(&event.position)
                    });
                    if let Some((_, start)) = hit {
                        let start = *start;
                        let _ = tab.update(cx, |tab, cx| {
                            tab.git.peek = if tab.git.peek == Some(start) {
                                None
                            } else {
                                Some(start)
                            };
                            cx.notify();
                        });
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(self.conflict_bar(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(editor)
                    .child(overlay),
            )
            .children(self.git_peek(cx))
            .into_any_element()
    }

    fn font_for_blame(&self, cx: &App) -> gpui_kit::Font {
        let settings = crate::settings_store::SettingsStore::get(cx);
        gpui_kit::Font {
            family: settings.font_family.clone().into(),
            style: gpui_kit::FontStyle::Italic,
            ..gpui_kit::font("Inter")
        }
    }

    /// "Conflict 1 of 2 · HEAD ⟷ feature" and its actions, over the editor.
    fn conflict_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let block = self.conflict_at_cursor(cx)?;
        let blocks = &self.git.conflicts;
        let index = blocks
            .iter()
            .position(|b| b.start == block.start)
            .unwrap_or(0);
        let theme = cx.theme();
        let button = |id: &'static str, label: &'static str| Button::new(id).label(label).xsmall();
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(4.))
                .bg(theme.warning.opacity(0.12))
                .border_b_1()
                .border_color(theme.border)
                .text_size(px(12.))
                .child(format!(
                    "Conflict {} of {} · {} ⟷ {}",
                    index + 1,
                    blocks.len(),
                    block.current_label,
                    block.incoming_label
                ))
                .child(div().flex_1())
                .child(
                    button("conflict-current", "Accept Current").on_click(cx.listener(
                        |tab, _, window, cx| tab.resolve_conflict(Resolution::Current, window, cx),
                    )),
                )
                .child(
                    button("conflict-incoming", "Accept Incoming").on_click(cx.listener(
                        |tab, _, window, cx| tab.resolve_conflict(Resolution::Incoming, window, cx),
                    )),
                )
                .child(button("conflict-both", "Accept Both").on_click(cx.listener(
                    |tab, _, window, cx| tab.resolve_conflict(Resolution::Both, window, cx),
                )))
                .child(
                    Button::new("conflict-previous")
                        .label("↑")
                        .xsmall()
                        .ghost()
                        .on_click(
                            cx.listener(|tab, _, window, cx| tab.go_to_conflict(-1, window, cx)),
                        ),
                )
                .child(
                    Button::new("conflict-next")
                        .label("↓")
                        .xsmall()
                        .ghost()
                        .on_click(
                            cx.listener(|tab, _, window, cx| tab.go_to_conflict(1, window, cx)),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The peeked change: what it was, and Revert, Stage, Diff and Review.
    fn git_peek(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let start = self.git.peek?;
        let hunk = self
            .git
            .hunks
            .iter()
            .find(|h| h.new_start == start)?
            .clone();
        let theme = cx.theme();
        let anchor = hunk.anchor_line();
        let button = |id: &'static str, label: &'static str| Button::new(id).label(label).xsmall();
        let settings = crate::settings_store::SettingsStore::get(cx);
        let mono = settings.font_family.clone();
        Some(
            div()
                .flex()
                .flex_col()
                .max_h(px(260.))
                .border_t_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(10.))
                        .py(px(4.))
                        .text_size(px(12.))
                        .child(format!("Line {anchor}: {}", hunk.title()))
                        .child(div().flex_1())
                        .child(button("peek-revert", "Revert").on_click(cx.listener(
                            move |tab, _, window, cx| tab.revert_hunk(start, window, cx),
                        )))
                        .child(button("peek-stage", "Stage").on_click(cx.listener(
                            move |_, _, _, cx| {
                                cx.emit(EditorGitAction {
                                    action: "stage".into(),
                                    line: anchor,
                                })
                            },
                        )))
                        .child(button("peek-diff", "Diff").on_click(cx.listener(
                            |tab, _, window, cx| {
                                tab.git.peek = None;
                                let _ = tab.toggle_diff_view(window, cx);
                            },
                        )))
                        .child(button("peek-review", "Review").on_click(cx.listener(
                            move |_, _, _, cx| {
                                cx.emit(EditorGitAction {
                                    action: "review".into(),
                                    line: anchor,
                                })
                            },
                        )))
                        .child(
                            Button::new("peek-close")
                                .label("✕")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|tab, _, _, cx| {
                                    tab.git.peek = None;
                                    cx.notify();
                                })),
                        ),
                )
                .when(!hunk.old_lines.is_empty(), |d| {
                    d.child(
                        div()
                            .id("peek-old")
                            .overflow_y_scroll()
                            .px(px(10.))
                            .pb(px(6.))
                            .font_family(mono)
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .children(hunk.old_lines.iter().map(|l| {
                                div().whitespace_nowrap().child(if l.is_empty() {
                                    " ".to_string()
                                } else {
                                    l.clone()
                                })
                            })),
                    )
                })
                .into_any_element(),
        )
    }

    /// Show Commit for This Line: the commit that last changed it.
    pub(crate) fn show_line_commit(&mut self, cx: &mut Context<Self>) {
        let line = self.cursor(cx).0 as usize;
        if let Some(info) = self.git.blame.get(&line).filter(|b| !b.is_uncommitted()) {
            cx.emit(EditorGitAction {
                action: format!("commit:{}", info.sha),
                line,
            });
        }
    }
}

/// Where each line starts in `text` (byte offsets).
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// The editor's text size, from a line's height.
fn font_size(line: Bounds<Pixels>) -> Pixels {
    line.size.height / 1.5
}

impl crate::workbench::Workbench {
    /// Hear what an editor's git layer asks of the workbench.
    pub(crate) fn wire_editor_git(
        &mut self,
        editor: &gpui_kit::Entity<EditorTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            editor,
            window,
            |w, editor, event: &EditorGitAction, window, cx| {
                w.editor_git_action(editor.clone(), &event.action, event.line, window, cx)
            },
        )
        .detach();
    }

    /// Stage the change at `line`, open it in the review, show a commit, or
    /// offer to mark a file resolved (the Swift `handleEditorGitAction`).
    fn editor_git_action(
        &mut self,
        editor: gpui_kit::Entity<EditorTab>,
        action: &str,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::tasks::{toast, ToastKind};
        if action == "save" {
            return self.save_editor(editor, window, cx);
        }
        let path = editor.read(cx).path.clone();
        let Some(root) = editor.read(cx).git.root.clone() else {
            return;
        };
        let Ok(relative) = path
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().into_owned())
        else {
            return;
        };
        let repo = crate::repository_state::RepositoryStore::state(&root, cx);
        if let Some(sha) = action.strip_prefix("commit:") {
            let surface = self.make_history(root, None, None, window, cx);
            if let crate::workspace::Surface::History(history) = &surface {
                let sha = sha.to_string();
                history.update(cx, |h, cx| h.reveal(sha, cx));
            }
            let active = self.active;
            return self.add_tab(active, surface, window, cx);
        }
        match action {
            "review" => {
                let scope = impulse_git::DiffScope::Unstaged;
                self.open_review_at(scope, &path, window, cx);
            }
            "conflicts-resolved" => {
                let change = repo
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.conflicted.iter().find(|c| c.path == relative).cloned());
                let Some(change) = change else {
                    return;
                };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let workbench = cx.entity().downgrade();
                let resolve: crate::tasks::Run = std::rc::Rc::new(move |window, cx| {
                    let (editor, repo, change) = (editor.clone(), repo.clone(), change.clone());
                    let _ = workbench.update(cx, |w, cx| {
                        if editor.update(cx, |e, cx| e.write(cx)).is_some() {
                            w.git_mark_resolved(&repo, &[change], window, cx);
                        }
                    });
                });
                toast(
                    window,
                    cx,
                    ToastKind::Success,
                    format!("No conflicts left in {name}."),
                    None,
                    Some(("Save & Mark Resolved", resolve)),
                );
            }
            "stage" => {
                // Staging takes the file from disk: save first.
                if editor.read(cx).dirty && editor.update(cx, |e, cx| e.write(cx)).is_none() {
                    return;
                }
                let task = cx.background_spawn({
                    let (root, relative) = (root.clone(), relative.clone());
                    async move {
                        impulse_git::scoped_diff::file_diff(
                            &root,
                            &relative,
                            None,
                            &impulse_git::DiffScope::Unstaged,
                            &Default::default(),
                        )
                        .ok()
                    }
                });
                cx.spawn_in(window, async move |w, cx| {
                    let Some(diff) = task.await else {
                        return;
                    };
                    let _ = w.update_in(cx, |w, window, cx| {
                        let index = diff.hunks.iter().position(|hunk| {
                            let start = hunk.new_start as usize;
                            let count = hunk
                                .lines
                                .iter()
                                .filter(|l| l.kind != impulse_git::DiffLineKind::Removed)
                                .count();
                            line + 1 >= start && line <= start + count.max(1)
                        });
                        let Some(index) = index else {
                            return toast(
                                window,
                                cx,
                                ToastKind::Warning,
                                "That change isn't in the unstaged diff.".into(),
                                None,
                                None,
                            );
                        };
                        let expected = diff
                            .hunk_ids
                            .get(index)
                            .map(|id| std::collections::HashMap::from([(index, id.clone())]))
                            .unwrap_or_default();
                        w.git_run(
                            &repo,
                            None,
                            None,
                            false,
                            move |root, _| {
                                impulse_git::operations::apply(
                                    impulse_git::operations::PatchTarget::Stage,
                                    &impulse_git::patch::PatchSelection::whole_hunks(&[index]),
                                    &relative,
                                    None,
                                    &expected,
                                    &Default::default(),
                                    root,
                                )
                            },
                            window,
                            cx,
                            |w, result, _, window, cx| match result {
                                Ok(_) => toast(
                                    window,
                                    cx,
                                    ToastKind::Success,
                                    "Staged the change".into(),
                                    None,
                                    None,
                                ),
                                Err(error) => {
                                    w.report(Err(error), "Couldn't stage the change", window, cx)
                                }
                            },
                        );
                    });
                })
                .detach();
            }
            _ => {}
        }
    }
}

/// How sticky lines look (the editor's font and colors).
struct StickyStyle {
    font: gpui_kit::Font,
    size: Pixels,
    tab_size: usize,
    background: Hsla,
    text: Hsla,
    number: Hsla,
    border: Hsla,
}

/// The sticky lines for the view's top: where text starts on screen, the
/// line height, and each line's row and text.
struct StickyLayout {
    text_x: Pixels,
    line_height: Pixels,
    lines: Vec<(usize, String)>,
}

/// Work out the lines opening the scopes around the top of the view
/// (`impulse_kit::sticky_scroll`).
fn sticky_layout(
    state: &gpui_kit::component::input::EditorState,
    text: &gpui_kit::component::input::Rope,
    tab_size: usize,
) -> Option<StickyLayout> {
    let rows = state.visible_row_range()?;
    let line_height = state.line_height()?;
    let top = rows.start;
    let text_x = rows.clone().find_map(|row| {
        let start = text.line_start_offset(row);
        state.range_to_bounds(&(start..start)).map(|b| b.origin.x)
    })?;
    let lines: Vec<String> = (0..text.lines_len().min(top + 2))
        .map(|row| {
            let at = text.line_start_offset(row);
            text.slice(at..at + text.line_len(row)).to_string()
        })
        .collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let sticky = impulse_kit::sticky_scroll::scope_rows(
        &refs,
        top + 1,
        tab_size,
        impulse_kit::sticky_scroll::MAX_LINES,
    );
    Some(StickyLayout {
        text_x,
        line_height,
        lines: sticky
            .into_iter()
            .map(|row| {
                let line: String = refs[row].chars().take(400).collect();
                (row, line.replace('\t', &" ".repeat(tab_size)))
            })
            .collect(),
    })
}

/// Paint the sticky lines over the view's first rows: each line's bounds
/// and row.
fn paint_sticky(
    layout: StickyLayout,
    bounds: Bounds<Pixels>,
    style: &StickyStyle,
    window: &mut Window,
    cx: &mut App,
) -> Vec<(Bounds<Pixels>, usize)> {
    let StickyLayout {
        text_x,
        line_height,
        lines,
    } = layout;
    let mut painted = Vec::new();
    for (i, (row, line)) in lines.iter().enumerate() {
        let y = bounds.top() + line_height * i as f32;
        let area = Bounds::new(
            point(bounds.left(), y),
            size(bounds.size.width, line_height),
        );
        window.paint_quad(fill(area, style.background));
        let shape = |content: String, color: Hsla, window: &mut Window| {
            let run = gpui_kit::TextRun {
                len: content.len(),
                font: style.font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(content.into(), style.size, &[run], None)
        };
        let shaped = shape(line.clone(), style.text, window);
        let _ = shaped.paint(
            point(text_x, y),
            line_height,
            gpui_kit::TextAlign::Left,
            None,
            window,
            cx,
        );
        let number = shape((row + 1).to_string(), style.number, window);
        let _ = number.paint(
            point(text_x - number.width - px(18.), y),
            line_height,
            gpui_kit::TextAlign::Left,
            None,
            window,
            cx,
        );
        painted.push((area, *row));
    }
    if !lines.is_empty() {
        let y = bounds.top() + line_height * lines.len() as f32;
        window.paint_quad(fill(
            Bounds::new(point(bounds.left(), y), size(bounds.size.width, px(1.))),
            style.border,
        ));
    }
    painted
}
