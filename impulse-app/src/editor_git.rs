//! The editor's git layer (the Swift editor's `web/editor.js` git section):
//! change marks in the gutter against the file's index version, following
//! typing; a click on one opens a peek with the original lines and Revert,
//! Stage, Diff and Review; Show Commit for This Line; and merge conflicts
//! tinted, with Accept Current, Incoming or Both from a bar over the
//! editor. The texts are worked out by
//! `impulse_kit::git::editor_marks`; the marks are painted over the editor
//! from its own layout (GPUI Kit's editor has no gutter decorations).

use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{TextDecoration, TextDecorationCollection};
use gpui_kit::component::{ActiveTheme as _, RopeExt as _, Sizable as _};
use gpui_kit::{
    canvas, div, fill, point, prelude::*, px, size, AnyElement, App, Bounds, Context,
    DispatchPhase, EventEmitter, Global, Hsla, MouseDownEvent, Pixels, Subscription, Task, Window,
};
use impulse_kit::git::editor_marks::{self, ConflictBlock, LineHunk, MarkKind, Resolution};
use impulse_kit::text_file::LineEnding;

use crate::editor_tab::EditorTab;
use crate::repository_state::RepositoryState;

/// Typing settles before the marks follow it.
const DIFF_DELAY: Duration = Duration::from_millis(150);
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
    pub(crate) root: Option<String>,
    trusted: bool,
    hunks: Vec<LineHunk>,
    /// The peeked hunk, by its start line.
    pub(crate) peek: Option<usize>,
    /// Where the editor's text area was last painted (the peek sits under
    /// its change, as Monaco's view zone does).
    origin: std::rc::Rc<std::cell::Cell<Option<gpui_kit::Point<Pixels>>>>,
    conflicts: Vec<ConflictBlock>,
    /// Whole lines tinted (1-based, inclusive): the conflicts' sides.
    line_fills: Vec<(std::ops::RangeInclusive<usize>, Hsla)>,
    /// Bracket pair colors (kept here as they follow the same edits).
    brackets: Option<TextDecorationCollection>,
    /// The pairs and their guides, for the matching-bracket box and the
    /// bracket pair guides (`editor_brackets`).
    bracket_pairs: crate::editor_brackets::Brackets,
    /// The links' underlines (`editor_links`).
    links: Option<TextDecorationCollection>,
    _base: Option<Task<()>>,
    _diff: Option<Task<()>>,
    _marks: Option<Task<()>>,
    _line_commit: Option<Task<()>>,
    _repository: Option<Subscription>,
}

type ScannedBrackets = (
    Vec<(std::ops::Range<usize>, impulse_kit::bracket_pairs::Color)>,
    Vec<impulse_kit::bracket_pairs::Pair>,
    Vec<impulse_kit::bracket_pairs::Guide>,
);

/// What `update_git_marks` works out off the main thread.
struct Marks {
    hunks: Vec<LineHunk>,
    conflicts: Vec<ConflictBlock>,
    links: Vec<std::ops::Range<usize>>,
    brackets: Option<ScannedBrackets>,
}

/// Documents past this size get no links or bracket colors.
const MAX_DECORATED: usize = 2 * 1024 * 1024;

impl Marks {
    fn compute(
        text: &str,
        base: Option<&str>,
        language: &str,
        tab_size: usize,
        brackets_on: bool,
    ) -> Marks {
        use impulse_kit::bracket_pairs;
        let small = text.len() <= MAX_DECORATED;
        Marks {
            hunks: base
                .map(|base| editor_marks::line_hunks(base, text))
                .unwrap_or_default(),
            conflicts: editor_marks::find_conflicts(text),
            links: if small {
                crate::editor_links::links(text)
            } else {
                Vec::new()
            },
            brackets: (brackets_on && small).then(|| {
                let (brackets, pairs) = bracket_pairs::scan(text, language);
                let guides = bracket_pairs::guides(text, &pairs, tab_size);
                (brackets, pairs, guides)
            }),
        }
    }
}

impl EditorTab {
    /// Read the file's git base again (opened, saved, or the repository
    /// changed).
    pub(crate) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.untitled.is_some() || self.path.as_os_str().is_empty() {
            return;
        }
        let path = self.path.clone();
        let root = self.git.root.clone();
        self.git._base = Some(cx.spawn(async move |tab, cx| {
            cx.background_executor().timer(BASE_DELAY).await;
            let read = cx.background_executor().spawn(async move {
                let file = path.to_string_lossy().into_owned();
                let base = impulse_git::base_content(&file).map(|b| LineEnding::normalize(&b));
                let root = root.or_else(|| {
                    impulse_git::repo_root(&path.parent()?.to_string_lossy())
                        .map(|r| r.trim_end_matches('/').to_string())
                });
                (base, root)
            });
            let (base, root) = read.await;
            let _ = tab.update(cx, |tab, cx| {
                // The Diff View compares against the same base.
                let diff_base = base
                    .clone()
                    .or_else(|| root.as_ref().map(|_| String::new()));
                if let Some(diff_base) = diff_base {
                    tab.diff_base_changed(&diff_base, cx);
                }
                tab.git.base = base;
                if tab.git.root.is_none() {
                    if let Some(root) = root.clone() {
                        tab.watch_repository(&root, cx);
                    }
                }
                tab.git.root = root;
                tab.update_git_marks(cx);
            });
        }));
    }

    /// Follow the repository: its index, HEAD and conflicts.
    fn watch_repository(&mut self, root: &str, cx: &mut Context<Self>) {
        let state = crate::repository_state::RepositoryStore::state(root, cx);
        let read = |state: &RepositoryState| (state.trusted, state.generation);
        let mut seen = read(state.read(cx));
        self.git.trusted = seen.0;
        self.git._repository = Some(cx.observe(&state, move |tab, state, cx| {
            // Not for progress lines and activity, only the repository.
            let now = read(state.read(cx));
            if now != seen {
                seen = now;
                tab.git.trusted = now.0;
                tab.refresh_git(cx);
            }
        }));
    }

    /// The buffer changed: the marks follow once typing settles.
    pub(crate) fn git_text_changed(&mut self, cx: &mut Context<Self>) {
        self.git._diff = Some(cx.spawn(async move |tab, cx| {
            cx.background_executor().timer(DIFF_DELAY).await;
            let _ = tab.update(cx, |tab, cx| tab.update_git_marks(cx));
        }));
    }

    /// Work out the change marks, conflicts, links and bracket pairs in the
    /// background, then paint them (unless the text moved on meanwhile).
    fn update_git_marks(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let base = self.git.base.clone();
        let language = self.language.clone();
        let tab_size = self.tab_size;
        let brackets_on = crate::settings_store::SettingsStore::get(cx).bracket_pair_colorization;
        self.git._marks = Some(cx.spawn(async move |tab, cx| {
            let (text, marks) = cx
                .background_spawn(async move {
                    let marks =
                        Marks::compute(&text, base.as_deref(), &language, tab_size, brackets_on);
                    (text, marks)
                })
                .await;
            let _ = tab.update(cx, |tab, cx| {
                if tab.text(cx) == text {
                    tab.apply_marks(marks, cx);
                }
            });
        }));
    }

    fn apply_marks(&mut self, marks: Marks, cx: &mut Context<Self>) {
        self.git.hunks = marks.hunks;
        if let Some(start) = self.git.peek {
            if !self.git.hunks.iter().any(|h| h.new_start == start) {
                self.git.peek = None;
            }
        }
        self.paint_brackets(marks.brackets, cx);
        self.paint_links(marks.links, cx);
        let resolved = !self.git.conflicts.is_empty() && marks.conflicts.is_empty();
        self.git.conflicts = marks.conflicts;
        self.paint_conflicts(cx);
        if resolved {
            cx.emit(EditorGitAction {
                action: "conflicts-resolved".into(),
                line: 0,
            });
        }
        cx.notify();
    }

    /// Underline the links, as Monaco does.
    fn paint_links(&mut self, links: Vec<std::ops::Range<usize>>, cx: &mut Context<Self>) {
        if links.is_empty() && self.git.links.is_none() {
            return;
        }
        let underlines = crate::editor_links::underlines_at(links);
        let marks = self.git.links.get_or_insert_with(|| {
            self.state.update(cx, |state, cx| {
                state.create_decorations_collection(Vec::new(), cx)
            })
        });
        marks.set(underlines, cx);
    }

    /// Color bracket pairs by depth (`bracket_pair_colorization`, Monaco's
    /// three colors), unmatched ones red.
    fn paint_brackets(&mut self, scanned: Option<ScannedBrackets>, cx: &mut Context<Self>) {
        use impulse_kit::bracket_pairs::{self, Color};
        let Some((brackets, pairs, guides)) = scanned else {
            if let Some(marks) = self.git.brackets.as_ref() {
                marks.clear(cx);
            }
            self.git.bracket_pairs = Default::default();
            return;
        };
        let hex = |h: &str| crate::workbench_chrome::hsla(impulse_kit::chrome::Rgba::from_hex(h));
        let levels = if cx.theme().mode.is_dark() {
            bracket_pairs::DARK
        } else {
            bracket_pairs::LIGHT
        }
        .map(hex);
        let unexpected = hex(bracket_pairs::UNEXPECTED);
        self.git.bracket_pairs = crate::editor_brackets::Brackets {
            guides: std::rc::Rc::new(guides),
            pairs: std::rc::Rc::new(pairs),
        };
        let decorations: Vec<TextDecoration> = brackets
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

    /// Tint each conflict's lines across the editor, as Monaco's whole-line
    /// decorations do: the current side green, the incoming blue, the
    /// marker lines in the modified color.
    fn paint_conflicts(&mut self, cx: &mut Context<Self>) {
        let Some(colors) = cx.try_global::<GitColors>().copied() else {
            return;
        };
        let (current, incoming, marker) = (
            colors.added.opacity(0.13),
            gpui_kit::rgb(0x7aa2f7).into(),
            colors.modified.opacity(0.18 * 0.85),
        );
        let incoming: Hsla = Hsla::opacity(&incoming, 0.15);
        let mut fills = Vec::new();
        for block in &self.git.conflicts {
            let current_end = block.base.unwrap_or(block.middle);
            fills.push((block.start..=block.start, marker));
            fills.push((block.start + 1..=current_end - 1, current));
            if let Some(base) = block.base {
                fills.push((base..=block.middle - 1, marker));
            }
            fills.push((block.middle..=block.middle, marker));
            fills.push((block.middle + 1..=block.end - 1, incoming));
            fills.push((block.end..=block.end, marker));
        }
        fills.retain(|(lines, _)| !lines.is_empty());
        self.git.line_fills = fills;
    }

    /// Peek the change on buffer line `line` (the snapshot's `git-peek=`,
    /// what a click on its mark does).
    #[cfg(debug_assertions)]
    pub(crate) fn peek_change_at(&mut self, line: usize, cx: &mut Context<Self>) -> bool {
        let start = impulse_kit::git::editor_marks::hunk_at_line(&self.git.hunks, line)
            .map(|h| h.new_start);
        self.git.peek = start;
        cx.notify();
        start.is_some()
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

    /// Keep one side of the conflict starting at line `start`, or both.
    fn resolve_conflict(
        &mut self,
        start: usize,
        choice: Resolution,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.text(cx);
        let Some(block) = self
            .git
            .conflicts
            .iter()
            .find(|b| b.start == start)
            .cloned()
        else {
            return;
        };
        let (range, replacement) = editor_marks::resolve_edit(&text, &block, choice);
        self.replace_range(range, &replacement, window, cx);
    }

    /// Go to the next (1) or previous (-1) conflict from the cursor.
    /// The file has merge conflicts (its markers are in the text).
    pub(crate) fn has_conflicts(&self) -> bool {
        !self.git.conflicts.is_empty()
    }

    /// The bracket pairs last found in the text.
    pub(crate) fn bracket_pairs(&self) -> std::rc::Rc<Vec<impulse_kit::bracket_pairs::Pair>> {
        self.git.bracket_pairs.pairs.clone()
    }

    pub(crate) fn go_to_conflict(
        &mut self,
        step: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line = self.cursor(cx).0 as usize;
        self.go_to_conflict_from(line, step, window, cx);
    }

    /// Go to the next (1) or previous (-1) conflict after or before `line`
    /// (1-based), wrapping around.
    fn go_to_conflict_from(
        &mut self,
        line: usize,
        step: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    /// The marks and the conflict bar over `editor`; the peek under it.
    pub(crate) fn wrap_git(&self, editor: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.try_global::<GitColors>().copied();
        let state = self.state.clone();
        // The Diff View marks the changes its own way.
        let hunks = if self.diff.is_some() {
            Vec::new()
        } else {
            self.git.hunks.clone()
        };
        let line_fills = self.git.line_fills.clone();
        let origin = self.git.origin.clone();
        let brackets = (!self.git.bracket_pairs.pairs.is_empty()).then(|| {
            (
                self.git.bracket_pairs.clone(),
                crate::editor_brackets::BracketColors::new(cx.theme().mode.is_dark()),
            )
        });
        let margin = {
            let settings = crate::settings_store::SettingsStore::get(cx);
            settings
                .show_right_margin
                .then_some(settings.right_margin_position.max(1) as usize)
        };
        // Monaco's `editorRuler.foreground`, which Swift's themes don't set.
        let margin_color: gpui_kit::Hsla = if cx.theme().mode.is_dark() {
            gpui_kit::rgb(0x5a5a5a).into()
        } else {
            gpui_kit::rgb(0xd3d3d3).into()
        };
        let tab = cx.entity().downgrade();
        let sticky = {
            let settings = crate::settings_store::SettingsStore::get(cx);
            settings.sticky_scroll.then(|| StickyStyle {
                font: gpui_kit::font(crate::fonts::family(&settings.font_family)),
                size: px(settings.font_size as f32),
                tab_size: self.tab_size,
                background: cx.theme().background,
                text: cx.theme().foreground,
                number: cx.theme().muted_foreground,
                border: cx.theme().border,
            })
        };
        let overlay = canvas(
            // Where the overlay takes clicks: not under a menu or palette
            // drawn over the editor.
            |bounds, window, _| window.insert_hitbox(bounds, gpui_kit::HitboxBehavior::Normal),
            move |bounds, hitbox, window, cx| {
                origin.set(Some(bounds.origin));
                // Painted after the editor, so its layout is this frame's.
                // Only the text area: the input's own bounds, inside the
                // editor element (the find bar sits above that element).
                let Some(area) = text_area(state.read(cx)) else {
                    return;
                };
                let view = area.view(bounds);
                // What scrolls sideways stays right of the line numbers.
                let column = gpui_kit::ContentMask {
                    bounds: area.text_column(bounds.right()),
                };
                if let Some((brackets, bracket_colors)) = &brackets {
                    window.with_content_mask(Some(column), |window| {
                        brackets.paint(*bracket_colors, state.read(cx), column.bounds, window)
                    });
                }
                let Some(colors) = colors else {
                    return;
                };
                let state = state.read(cx);
                let text = state.text().clone();
                let sticky_layout = sticky
                    .as_ref()
                    .and_then(|style| sticky_layout(state, &text, style.tab_size));
                let line_offset = |line: usize| -> Option<usize> {
                    let row = line.checked_sub(1)?;
                    (row < text.lines_len()).then(|| text.line_start_offset(row))
                };
                window.with_content_mask(Some(column), |window| {
                    // The right margin, at a column measured on a visible
                    // line.
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
                            if x >= area.text_left && x < bounds.right() {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(x, view.top()),
                                        size(px(1.), view.size.height),
                                    ),
                                    margin_color,
                                ));
                            }
                        }
                    }
                    // Whole-line tints, from the text's left edge to the
                    // right.
                    if let Some(rows) = state.visible_row_range() {
                        for (lines, color) in &line_fills {
                            let first = (*lines.start()).max(rows.start + 1);
                            let last = (*lines.end()).min(rows.end);
                            if first > last {
                                continue;
                            }
                            let top =
                                line_offset(first).and_then(|o| state.range_to_bounds(&(o..o)));
                            let bottom =
                                line_offset(last).and_then(|o| state.range_to_bounds(&(o..o)));
                            if let (Some(top), Some(bottom)) = (top, bottom) {
                                window.paint_quad(fill(
                                    Bounds::from_corners(
                                        point(area.text_left, top.top()),
                                        point(bounds.right(), bottom.bottom()),
                                    ),
                                    *color,
                                ));
                            }
                        }
                    }
                });
                // The change bars stay in the gutter whatever the
                // horizontal scroll (inside the text area's height).
                let mut bars: Vec<(Bounds<Pixels>, usize)> = Vec::new();
                window.with_content_mask(Some(gpui_kit::ContentMask { bounds: view }), |window| {
                    for hunk in &hunks {
                        let (color, lines) = match hunk.kind() {
                            MarkKind::Added => (colors.added, hunk.marked_lines()),
                            MarkKind::Modified => (colors.modified, hunk.marked_lines()),
                            MarkKind::Deleted => (colors.deleted, hunk.marked_lines()),
                        };
                        let first = lines.clone().find_map(|l| {
                            line_offset(l).and_then(|o| state.range_to_bounds(&(o..o)))
                        });
                        let last = lines.clone().rev().find_map(|l| {
                            line_offset(l).and_then(|o| state.range_to_bounds(&(o..o)))
                        });
                        let (Some(first), Some(last)) = (first, last) else {
                            continue;
                        };
                        // In Monaco's decorations lane, left of the folding
                        // one.
                        let x = area.text_left - px(18.);
                        let bar = if hunk.kind() == MarkKind::Deleted {
                            // A small wedge where the lines were, under the
                            // line.
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
                });
                // The same marks in the overview ruler's left lane, where the
                // whole file fits the text area's height (Monaco's ruler).
                let total = text.lines_len().max(1) as f32;
                let height = view.size.height;
                for hunk in &hunks {
                    let color = match hunk.kind() {
                        MarkKind::Added => colors.added,
                        MarkKind::Modified => colors.modified,
                        MarkKind::Deleted => colors.deleted,
                    };
                    let lines = hunk.marked_lines();
                    let (Some(first), Some(last)) = (lines.clone().next(), lines.last()) else {
                        continue;
                    };
                    let top = view.top() + height * ((first - 1) as f32 / total);
                    let bottom = (view.top() + height * (last as f32 / total)).max(top + px(3.));
                    window.paint_quad(fill(
                        Bounds::from_corners(
                            point(bounds.right() - px(10.), top),
                            point(bounds.right() - px(5.), bottom),
                        ),
                        color,
                    ));
                }
                let pinned = match (&sticky, sticky_layout) {
                    (Some(style), Some(layout)) => {
                        paint_sticky(layout, view, area, style, window, cx)
                    }
                    _ => Vec::new(),
                };
                // A click on a mark opens its peek, one on a sticky line goes
                // there (before the editor sees the click). The find bar is
                // outside this overlay, so its clicks stay its own.
                let tab = tab.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture
                        || !hitbox.is_hovered(window)
                        || !view.contains(&event.position)
                    {
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
        // Under the editor: its background, and the current line while it
        // has focus (the theme leaves both to us).
        let backdrop = cx.try_global::<crate::kit_theme::EditorBackdrop>().copied();
        let focus = gpui_kit::Focusable::focus_handle(self.state.read(cx), cx);
        let under_state = self.state.clone();
        let underlay = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let Some(backdrop) = backdrop else {
                    return;
                };
                window.paint_quad(fill(bounds, backdrop.background));
                let Some(color) = backdrop.current_line else {
                    return;
                };
                if !focus.contains_focused(window, cx) {
                    return;
                }
                let state = under_state.read(cx);
                if !state.selected_range().is_empty() {
                    return;
                }
                // Painted before the editor lays this frame out: the caret's
                // row from this frame's prepaint, so the band doesn't trail
                // the text as it scrolls. From the text column on, as
                // Monaco's `renderLineHighlight: "line"` (the Swift app's)
                // leaves the gutter alone.
                let (Some(geometry), Some(area)) =
                    (state.prepaint_caret_geometry(), text_area(state))
                else {
                    return;
                };
                let caret = geometry.cursor_bounds();
                let line_height = geometry.line_height();
                let input = geometry.input_bounds();
                let top = caret.top() - (line_height - caret.size.height) / 2.
                    + geometry.scroll_offset().y;
                let left = input.left() + (area.text_left - state.input_bounds().left());
                let band = Bounds::from_corners(
                    point(left, top.max(input.top())),
                    point(bounds.right(), (top + line_height).min(input.bottom())),
                );
                if band.size.height > px(0.) {
                    window.paint_quad(fill(band, color));
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        // The peek under its change's last line when that's on screen, else
        // under the editor.
        let peek = self.git_peek(cx);
        let (inline, below) = match (peek, self.peek_position(cx)) {
            (Some(peek), Some((left, top))) => (
                Some(
                    FollowScroll::new(
                        &self.state,
                        false,
                        div().absolute().top(top).left(left).right_0().child(peek),
                    )
                    .into_any_element(),
                ),
                None,
            ),
            (peek, _) => (None, peek),
        };
        let conflicts = self.conflict_bars(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(underlay)
                    .child(editor)
                    .child(overlay)
                    .children(conflicts)
                    .children(inline),
            )
            .children(below)
            .into_any_element()
    }

    /// Each conflict's actions on its `<<<<<<<` line, after the marker:
    /// Accept Current, Incoming or Both, "HEAD ⟷ feature", and with more
    /// than one, "1 of 2" with ↑ and ↓. Monaco puts the same bar in a view
    /// zone above the conflict; GPUI Kit has no view zones, so it floats
    /// on the marker line, as the peek floats under its change, clipped
    /// to the text column (it scrolls sideways with the text).
    fn conflict_bars(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.git.conflicts.is_empty() {
            return None;
        }
        let origin = self.git.origin.get()?;
        let state = self.state.read(cx);
        let rows = state.visible_row_range()?;
        let area = text_area(state)?;
        let text = state.text();
        let places: Vec<(ConflictBlock, Pixels, Pixels, Pixels)> = self
            .git
            .conflicts
            .iter()
            .filter_map(|block| {
                let row = block.start.checked_sub(1)?;
                if row >= text.lines_len() || !rows.contains(&row) {
                    return None;
                }
                let end = text.line_end_offset(row);
                let at = state.range_to_bounds(&(end..end))?;
                Some((
                    block.clone(),
                    at.left() - area.text_left + px(16.),
                    at.top() - area.top,
                    at.size.height,
                ))
            })
            .collect();
        let count = self.git.conflicts.len();
        let index_of = |start: usize| {
            self.git
                .conflicts
                .iter()
                .position(|b| b.start == start)
                .unwrap_or(0)
        };
        let theme = cx.theme();
        let color = theme.foreground.opacity(0.9);
        let muted = theme.foreground.opacity(0.72);
        let bars = places
            .into_iter()
            .map(|(block, left, top, height)| {
                let start = block.start;
                let index = index_of(start);
                // Swift's .git-peek-button: transparent with a faint border.
                let button = |id: &'static str, label: &'static str| {
                    Button::new((id, start)).label(label).xsmall().outline()
                };
                let resolve = |choice: Resolution| {
                    cx.listener(
                        move |tab: &mut Self, _: &gpui_kit::ClickEvent, window, cx| {
                            tab.resolve_conflict(start, choice, window, cx)
                        },
                    )
                };
                let mut bar = div()
                    .absolute()
                    .top(top)
                    .left(left)
                    .h(height)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .font_family(crate::workbench::UI_FONT)
                    .text_color(color)
                    .child(
                        button("conflict-current", "Accept Current")
                            .on_click(resolve(Resolution::Current)),
                    )
                    .child(
                        button("conflict-incoming", "Accept Incoming")
                            .on_click(resolve(Resolution::Incoming)),
                    )
                    .child(
                        button("conflict-both", "Accept Both").on_click(resolve(Resolution::Both)),
                    )
                    .child(div().text_color(muted).child(format!(
                        "{} ⟷ {}",
                        block.current_label, block.incoming_label
                    )));
                if count > 1 {
                    bar = bar
                        .child(
                            div()
                                .ml(px(10.))
                                .text_color(muted)
                                .child(format!("{} of {count}", index + 1)),
                        )
                        .child(button("conflict-previous", "↑").on_click(cx.listener(
                            move |tab: &mut Self, _: &gpui_kit::ClickEvent, window, cx| {
                                tab.go_to_conflict_from(start, -1, window, cx)
                            },
                        )))
                        .child(button("conflict-next", "↓").on_click(cx.listener(
                            move |tab: &mut Self, _: &gpui_kit::ClickEvent, window, cx| {
                                tab.go_to_conflict_from(start, 1, window, cx)
                            },
                        )));
                }
                bar.into_any_element()
            })
            .collect::<Vec<_>>();
        Some(
            div()
                .absolute()
                .left(area.text_left - origin.x)
                .top(area.top - origin.y)
                .right_0()
                .h(area.bottom - area.top)
                .overflow_hidden()
                .child(FollowScroll::new(
                    &self.state,
                    true,
                    div().absolute().top_0().left_0().size_full().children(bars),
                ))
                .into_any_element(),
        )
    }

    /// Where the peek goes: under the peeked change's last line, from the
    /// text column's left edge (it doesn't scroll sideways, as Monaco's
    /// view zones don't), relative to the editor (from the last frame;
    /// `FollowScroll` brings it up to date).
    fn peek_position(&self, cx: &App) -> Option<(Pixels, Pixels)> {
        let start = self.git.peek?;
        let hunk = self.git.hunks.iter().find(|h| h.new_start == start)?;
        let origin = self.git.origin.get()?;
        let state = self.state.read(cx);
        let text = state.text();
        let row = hunk.marked_lines().end.checked_sub(2)?;
        if row >= text.lines_len() {
            return None;
        }
        let rows = state.visible_row_range()?;
        if !rows.contains(&row) {
            return None;
        }
        let offset = text.line_start_offset(row);
        let line = state.range_to_bounds(&(offset..offset))?;
        let area = text_area(state)?;
        Some((
            area.text_left - origin.x - px(4.),
            line.bottom() - origin.y + px(4.),
        ))
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
        // Swift's .git-peek-button: transparent with a faint border.
        let button =
            |id: &'static str, label: &'static str| Button::new(id).label(label).small().outline();
        let settings = crate::settings_store::SettingsStore::get(cx);
        let mono = crate::fonts::family(&settings.font_family);
        let colors = cx.try_global::<GitColors>().copied();
        let (edge, removed) = colors
            .map(|c| (c.modified, c.deleted.opacity(0.12)))
            .unwrap_or((theme.border, theme.secondary));
        let widget = cx
            .try_global::<crate::kit_theme::EditorBackdrop>()
            .map_or(theme.secondary, |b| b.widget);
        Some(
            div()
                .flex()
                .flex_col()
                .max_h(px(260.))
                .border_t_1()
                .border_b_1()
                .border_color(edge)
                .bg(widget)
                .shadow_md()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(10.))
                        .py(px(4.))
                        .text_size(px(12.))
                        .child(div().flex_1().opacity(0.8).child(hunk.title()))
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
                                .small()
                                .outline()
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
                            .pl(px(14.))
                            .pr(px(10.))
                            .pt(px(2.))
                            .pb(px(6.))
                            .bg(removed)
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

    /// Show Commit for This Line: the commit that last changed it, blamed
    /// when asked, against the buffer's own text (`--contents`), so lines
    /// keep their numbers through unsaved edits. A line not committed yet
    /// has none.
    pub(crate) fn show_line_commit(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        let line = self.cursor(cx).0 as usize;
        let (trusted, path, text) = (self.git.trusted, self.path.clone(), self.text(cx));
        let read = cx.background_executor().spawn(async move {
            let contents = tempfile_with(&text)?;
            let file = path.to_string_lossy().into_owned();
            let range = format!("{line},{line}");
            let contents_path = contents.to_string_lossy().into_owned();
            // Git reads run as the repository's trust allows.
            let found = RepositoryState::read_git(
                &root,
                trusted,
                &[
                    "blame",
                    "--porcelain",
                    "--no-textconv",
                    "-L",
                    &range,
                    "--contents",
                    &contents_path,
                    "--",
                    &file,
                ],
            );
            let _ = std::fs::remove_file(&contents);
            impulse_git::git_blame::parse_porcelain(&found?)
                .remove(&line)
                .filter(|b| !b.is_uncommitted())
        });
        self.git._line_commit = Some(cx.spawn(async move |tab, cx| {
            let Some(info) = read.await else {
                return;
            };
            let _ = tab.update(cx, |_, cx| {
                cx.emit(EditorGitAction {
                    action: format!("commit:{}", info.sha),
                    line,
                })
            });
        }));
    }
}

/// `text` in a new file of the system's temporary folder, readable only by
/// the user.
fn tempfile_with(text: &str) -> Option<std::path::PathBuf> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "impulse-blame-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .ok()?;
    file.write_all(text.as_bytes()).ok()?;
    Some(path)
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
        if let Some(command) = action.strip_prefix("run:") {
            return self.run_from_preview(&editor, command, window, cx);
        }
        // The file changed on disk while it can't simply reload.
        if action == "disk-changed" {
            let (name, dirty) = {
                let e = editor.read(cx);
                (e.title.clone(), e.dirty)
            };
            let reload: crate::tasks::Run = std::rc::Rc::new({
                let editor = editor.clone();
                move |window, cx| editor.update(cx, |e, cx| e.take_disk(window, cx))
            });
            let message = if dirty {
                format!("{name} changed on disk while you have unsaved edits.")
            } else {
                format!("{name} changed on disk.")
            };
            let toast = if dirty {
                crate::toasts::Toast::warning(message)
            } else {
                crate::toasts::Toast::info(message)
            };
            return crate::toasts::show(
                window,
                cx,
                toast
                    .action("Reload", reload)
                    .lifetime(Some(std::time::Duration::from_secs(20))),
            );
        }
        if action == "too-large" {
            let (path, name) = {
                let e = editor.read(cx);
                (e.path.clone(), e.title.clone())
            };
            return self.offer_pager(&path, &name, window, cx);
        }
        // A link to a file in a Markdown preview.
        if let Some(path) = action.strip_prefix("open:") {
            return self.open_file(std::path::Path::new(path), window, cx);
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

/// Where an editor's text sits on screen, from its last paint.
#[derive(Clone, Copy)]
pub(crate) struct TextArea {
    /// The text column's left edge, right of the line numbers, wherever
    /// the text is scrolled sideways: gutter marks sit left of it.
    pub text_left: Pixels,
    /// The text area's top and bottom: the input's own, inside the editor
    /// element.
    pub top: Pixels,
    pub bottom: Pixels,
}

impl TextArea {
    /// The text column, from `text_left` to `right`: what scrolls
    /// sideways is painted inside it, so it never slides over the gutter.
    pub fn text_column(&self, right: Pixels) -> Bounds<Pixels> {
        Bounds::from_corners(point(self.text_left, self.top), point(right, self.bottom))
    }

    /// The whole text area across `bounds`, gutter included.
    pub fn view(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        Bounds::from_corners(
            point(bounds.left(), self.top),
            point(bounds.right(), self.bottom),
        )
    }
}

/// The text area of `state`'s last layout. `range_to_bounds` counts the
/// horizontal scroll in; a line's start, less the painted text origin,
/// is the line numbers' width.
pub(crate) fn text_area(state: &gpui_kit::component::input::EditorState) -> Option<TextArea> {
    let painted = state.text_bounds()?;
    let input = state.input_bounds();
    let text = state.text();
    let row = state
        .visible_row_range()?
        .start
        .min(text.lines_len().saturating_sub(1));
    let start = text.line_start_offset(row);
    let line = state.range_to_bounds(&(start..start))?;
    Some(TextArea {
        text_left: input.left() + (line.left() - painted.left()),
        top: input.top(),
        bottom: input.bottom(),
    })
}

/// How far the editor has scrolled since its last paint, by this frame's
/// prepaint (`prepaint_caret_geometry`, which GPUI Kit's completion menu
/// reads for the same reason); zero before then.
fn scrolled_since_paint(
    state: &gpui_kit::component::input::EditorState,
) -> gpui_kit::Point<Pixels> {
    let (Some(geometry), Some(painted)) = (state.prepaint_caret_geometry(), state.text_bounds())
    else {
        return gpui_kit::Point::default();
    };
    let before = painted.origin - state.input_bounds().origin;
    geometry.scroll_offset() - before
}

/// Elements laid over the editor in render, from its last frame's layout
/// (the peek, the conflict bars, the Diff View's hidden-lines rows),
/// moved in prepaint by however far the text scrolled since, so they keep
/// up with it rather than trailing it by a frame.
pub(crate) struct FollowScroll {
    state: gpui_kit::Entity<gpui_kit::component::input::EditorState>,
    /// Follow the horizontal scroll too (what sits on a line's text).
    sideways: bool,
    child: AnyElement,
}

impl FollowScroll {
    pub fn new(
        state: &gpui_kit::Entity<gpui_kit::component::input::EditorState>,
        sideways: bool,
        child: impl IntoElement,
    ) -> Self {
        FollowScroll {
            state: state.clone(),
            sideways,
            child: child.into_any_element(),
        }
    }
}

impl IntoElement for FollowScroll {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui_kit::Element for FollowScroll {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui_kit::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui_kit::LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut delta = scrolled_since_paint(self.state.read(cx));
        if !self.sideways {
            delta.x = px(0.);
        }
        window.with_element_offset(delta, |window| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
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
    // Only the lines it looks at are copied out (not every line above).
    let line = |row: usize| {
        let at = text.line_start_offset(row);
        text.slice(at..at + text.line_len(row)).to_string()
    };
    let sticky = impulse_kit::sticky_scroll::scope_rows_by(
        text.lines_len().min(top + 2),
        line,
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
                let line: String = line(row).chars().take(400).collect();
                (row, line.replace('\t', &" ".repeat(tab_size)))
            })
            .collect(),
    })
}

/// Paint the sticky lines over the text area's first rows (`view`): each
/// line's bounds and row. The text scrolls sideways with the editor's,
/// inside the text column; the numbers stay in the gutter.
fn paint_sticky(
    layout: StickyLayout,
    view: Bounds<Pixels>,
    area: TextArea,
    style: &StickyStyle,
    window: &mut Window,
    cx: &mut App,
) -> Vec<(Bounds<Pixels>, usize)> {
    let StickyLayout {
        text_x,
        line_height,
        lines,
    } = layout;
    let column = gpui_kit::ContentMask {
        bounds: area.text_column(view.right()),
    };
    let mut painted = Vec::new();
    for (i, (row, line)) in lines.iter().enumerate() {
        let y = view.top() + line_height * i as f32;
        let area_row = Bounds::new(point(view.left(), y), size(view.size.width, line_height));
        window.paint_quad(fill(area_row, style.background));
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
        window.with_content_mask(Some(column), |window| {
            let _ = shaped.paint(
                point(text_x, y),
                line_height,
                gpui_kit::TextAlign::Left,
                None,
                window,
                cx,
            );
        });
        let number = shape((row + 1).to_string(), style.number, window);
        let _ = number.paint(
            point(area.text_left - number.width - px(18.), y),
            line_height,
            gpui_kit::TextAlign::Left,
            None,
            window,
            cx,
        );
        painted.push((area_row, *row));
    }
    if !lines.is_empty() {
        let y = view.top() + line_height * lines.len() as f32;
        window.paint_quad(fill(
            Bounds::new(point(view.left(), y), size(view.size.width, px(1.))),
            style.border,
        ));
    }
    painted
}
