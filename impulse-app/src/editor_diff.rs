//! Diff View (Toggle Diff View; Monaco's diff editor in the Swift app): the
//! file's index version against the buffer, under a bar with the file, the
//! summary, ↑ ↓ through the changes, Inline or Side by Side, and Done.
//! Side by side, the index version sits read-only on the left, and the
//! shorter side of each change is padded with a hatched view zone, as
//! Monaco's diff editor aligns them, so the two sides are the same height
//! line for line and scroll together at one offset. Inline, the removed
//! lines sit above the lines that replaced them in one read-only document.
//! Changed lines are tinted on both, following edits. Long runs of
//! unchanged lines hide (GPUI Kit's hidden ranges) behind a "12 hidden
//! lines" view zone a click opens, on both sides (Monaco's
//! `hideUnchangedRegions`, three lines of context).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{
    Editor, EditorState, Position, TabSize, ViewZone, ViewZoneContext, ViewZoneId,
};
use gpui_kit::component::{ActiveTheme as _, RopeExt as _, Sizable as _};
use gpui_kit::{
    canvas, div, fill, point, prelude::*, px, AnyElement, Bounds, Context, Entity, FontWeight,
    Hsla, Pixels, Window,
};
use impulse_kit::git::diff_view;
use impulse_kit::git::editor_marks::{line_hunks, LineHunk};
use impulse_kit::text_file::LineEnding;

use crate::editor_tab::EditorTab;

/// How long typing rests before the Diff View marks again.
pub(crate) const DIFF_DELAY: Duration = Duration::from_millis(200);

/// Rows (0-based, end exclusive) tinted across the editor, and the tint.
/// What a side paints over its text: whole lines, and on top the words
/// that changed within them (Monaco's line and character changes).
#[derive(Clone, Default)]
struct Tints {
    /// The rows, their color and the sign beside their numbers.
    rows: Vec<(std::ops::Range<u32>, Hsla, &'static str)>,
    /// Byte ranges in the side's text.
    words: Vec<(std::ops::Range<usize>, Hsla)>,
}

/// The Diff View's other editor and its marks.
pub(crate) struct DiffView {
    base: Entity<EditorState>,
    base_text: String,
    /// A new base (staged or committed since) for the base editor, set at
    /// the next frame (it needs the window).
    base_pending: Option<String>,
    base_tints: Tints,
    own_tints: Tints,
    /// The base's bracket pair colors and guides, as the buffer has them
    /// (`editor_git::bracket_marks`), worked out once per base text:
    /// `base_generation` counts the base texts, `base_marked` is the one
    /// the marks are for.
    base_brackets: crate::editor_brackets::Brackets,
    base_bracket_marks: Option<gpui_kit::component::input::TextDecorationCollection>,
    base_generation: u64,
    base_marked: Option<u64>,
    hunks: Vec<LineHunk>,
    /// Inline: the combined document, rebuilt as the buffer changes.
    inline: Option<InlineView>,
    /// The last scroll position the sync set on either side, so a side
    /// that can't scroll that far isn't asked again every frame.
    synced: Rc<Cell<Option<f32>>>,
    /// Buffer rows whose unchanged runs stay shown.
    revealed: Vec<std::ops::Range<usize>>,
    /// Side by side: the unchanged runs hidden, on both sides.
    hidden: Vec<Hidden>,
    /// Side by side: the zones and hidden lines the two editors have, and
    /// what they were made for.
    zones: SideZones,
    _refresh: Option<gpui_kit::Task<()>>,
}

/// A run of unchanged lines hidden behind one row.
#[derive(Clone, Debug, PartialEq)]
struct Hidden {
    /// The rows it hides: the buffer's (the inline document's, inline),
    /// and the base's (side by side).
    rows: std::ops::Range<usize>,
    base_rows: std::ops::Range<usize>,
    /// The buffer rows it stands for (what a click reveals).
    buffer_rows: std::ops::Range<usize>,
    /// The symbol around its last line, as Monaco's row names it.
    label: Option<String>,
}

/// The view zones and hidden ranges set on the editors side by side.
#[derive(Default)]
struct SideZones {
    /// What they were made for: each zone's side (true: the buffer), line
    /// and height in lines (0 for a hidden run's row), and the runs.
    key: Option<(Vec<(bool, usize, usize)>, Vec<Hidden>)>,
    own: Vec<ViewZoneId>,
    base: Vec<ViewZoneId>,
    /// The hidden runs' rows among them (`tinted` widens them).
    own_rows: Vec<ViewZoneId>,
    base_rows: Vec<ViewZoneId>,
}

impl DiffView {
    /// Inline: one read-only document shows in place of the buffer.
    pub(crate) fn is_inline(&self) -> bool {
        self.inline.is_some()
    }
}

struct InlineView {
    state: Entity<EditorState>,
    tints: Tints,
    rows: diff_view::InlineDiff,
    /// The unchanged runs hidden (in the document's rows).
    hidden: Vec<Hidden>,
    /// The text to show at the next render (setting it needs the window).
    pending: Option<String>,
    /// The hidden runs the document's zones and hidden ranges are for, and
    /// the zones.
    applied: Option<Vec<Hidden>>,
    zones: Vec<ViewZoneId>,
}

impl EditorTab {
    /// Toggle Diff View, in the layout it last had. An error message when
    /// the file isn't in a repository.
    pub fn toggle_diff_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.diff.is_some() {
            self.close_diff_view(window, cx);
            return Ok(());
        }
        // The file's git base, as the change marks have it: its index
        // version (what's staged, else HEAD's).
        // A file git doesn't track yet is all new.
        let base_text = impulse_git::base_content(&self.path.to_string_lossy())
            .map(|b| LineEnding::normalize(&b))
            .or_else(|| self.git.root.is_some().then(String::new))
            .ok_or_else(|| "This file isn't in a git repository.".to_string())?;
        if self.preview == Some(false) {
            self.preview = None;
        }
        self.git.peek = None;
        let base = self.read_only_state(base_text.clone(), window, cx);
        self.diff = Some(DiffView {
            base,
            base_text,
            base_pending: None,
            base_tints: Tints::default(),
            own_tints: Tints::default(),
            base_brackets: Default::default(),
            base_bracket_marks: None,
            base_generation: 0,
            base_marked: None,
            hunks: Vec::new(),
            inline: None,
            synced: Rc::default(),
            revealed: Vec::new(),
            hidden: Vec::new(),
            zones: SideZones::default(),
            _refresh: None,
        });
        if self.diff_inline {
            self.set_diff_inline(true, window, cx);
        }
        self.refresh_diff(Duration::ZERO, cx);
        cx.notify();
        Ok(())
    }

    /// Done: back to the editor, the cursor where it was.
    fn close_diff_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut diff) = self.diff.take() {
            self.clear_side_zones(&mut diff, cx);
        }
        let focus = self.state.clone();
        focus.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// An editor like the buffer's, read-only, holding `text`.
    fn read_only_state(
        &self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorState> {
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let grammar = crate::base_rules::editor_grammar(crate::editor_tab::grammar_for(
            &self.language,
            &self.path,
        ))
        .to_string();
        let (tab_size, use_spaces) = (self.tab_size, self.use_spaces);
        cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language(grammar)
                .line_number(settings.show_line_numbers)
                .folding(settings.folding)
                .indent_guides(settings.indent_guides)
                .soft_wrap(settings.word_wrap)
                .tab_size(TabSize {
                    tab_size,
                    hard_tabs: !use_spaces,
                })
                .default_value(text);
            state.set_readonly(true, cx);
            crate::editor_tab::apply_editor_options(&mut state, &settings, cx);
            state
        })
    }

    /// Switch between side by side and inline (remembered for the tab).
    pub(crate) fn set_diff_inline(
        &mut self,
        inline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.diff_inline = inline;
        let Some(diff) = self.diff.as_mut() else {
            return;
        };
        if !inline {
            diff.inline = None;
            let focus = self.state.clone();
            focus.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        if diff.inline.is_some() {
            return;
        }
        let text = self.state.read(cx).value().to_string();
        let rows = diff_view::inline(&text, &diff.hunks);
        let hidden = inline_hidden(&rows, &text, &diff.revealed, self.tab_size);
        // The cursor's line, where it now is, or the first line after the
        // run hiding it.
        let cursor = self.state.read(cx).cursor_position().line as usize;
        let at = rows
            .buffer_rows
            .iter()
            .position(|r| *r == Some(cursor))
            .map(|row| {
                hidden
                    .iter()
                    .find(|h| h.rows.contains(&row))
                    .map_or(row, |h| h.rows.end)
            });
        let shown = rows.text.clone();
        let state = self.read_only_state(shown, window, cx);
        // The buffer's own zones and hidden lines go while it's not shown.
        let mut diff = self.diff.take();
        if let Some(diff) = diff.as_mut() {
            self.clear_side_zones(diff, cx);
            diff.inline = Some(InlineView {
                state: state.clone(),
                tints: Tints::default(),
                rows,
                hidden,
                pending: None,
                applied: None,
                zones: Vec::new(),
            });
        }
        self.diff = diff;
        self.sync_diff_zones(window, cx);
        match at {
            Some(row) => center(&state, row, window, cx),
            None => state.update(cx, |state, cx| state.focus(window, cx)),
        }
        self.refresh_diff(Duration::ZERO, cx);
        cx.notify();
    }

    /// Mark the changed lines again, after `delay`.
    pub(crate) fn refresh_diff(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.as_mut() else {
            return;
        };
        let old = diff.base_text.clone();
        let new = self.state.read(cx).value().to_string();
        let revealed = diff.revealed.clone();
        let tab_size = self.tab_size;
        // The base's bracket colors, unless it has them already.
        let generation = diff.base_generation;
        let scan_base = (diff.base_marked != Some(generation)).then(|| {
            let on = crate::settings_store::SettingsStore::get(cx).bracket_pair_colorization;
            (self.language.clone(), self.tab_size, on)
        });
        diff._refresh = Some(cx.spawn(async move |tab, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let (
                hunks,
                removed,
                added,
                (inline, inline_hidden, hidden),
                (removed_words, added_words),
                base_brackets,
            ) = cx
                .background_executor()
                .spawn(async move {
                    let base_brackets = scan_base.map(|(language, tab_size, on)| {
                        let scanned =
                            crate::editor_git::scan_brackets(&old, &language, tab_size, on);
                        (old.clone(), scanned)
                    });
                    let hunks = line_hunks(&old, &new);
                    let rows = |start: usize, len: usize| {
                        let start = start.saturating_sub(1) as u32;
                        start..start + len as u32
                    };
                    let removed: Vec<_> = hunks
                        .iter()
                        .filter(|h| !h.old_lines.is_empty())
                        .map(|h| rows(h.old_start, h.old_lines.len()))
                        .collect();
                    let added: Vec<_> = hunks
                        .iter()
                        .filter(|h| h.new_count > 0)
                        .map(|h| rows(h.new_start, h.new_count))
                        .collect();
                    let inline = diff_view::inline(&new, &hunks);
                    let inline_hidden = inline_hidden(&inline, &inline.text, &revealed, tab_size);
                    let hidden = side_hidden(&old, &new, &hunks, &revealed, tab_size);
                    let words = impulse_git::buffer_diff::changed_words(
                        &old,
                        &new,
                        hunks.iter().map(|h| {
                            let (o, n) = (
                                rows(h.old_start, h.old_lines.len()),
                                rows(h.new_start, h.new_count),
                            );
                            (
                                o.start as usize..o.end as usize,
                                n.start as usize..n.end as usize,
                            )
                        }),
                    );
                    (
                        hunks,
                        removed,
                        added,
                        (inline, inline_hidden, hidden),
                        words,
                        base_brackets,
                    )
                })
                .await;
            let _ = tab.update(cx, |tab, cx| {
                let Some(diff) = tab.diff.as_mut() else {
                    return;
                };
                // Colored only once the base editor holds the text scanned
                // (a new base is set at the next frame, which refreshes).
                if let Some((scanned_text, scanned)) = base_brackets {
                    if diff.base_generation == generation
                        && diff.base.read(cx).value().as_ref() == scanned_text.as_str()
                    {
                        let (decorations, painted) = match scanned {
                            Some(scanned) => {
                                crate::editor_git::bracket_marks(scanned, cx.theme().mode.is_dark())
                            }
                            None => (Vec::new(), Default::default()),
                        };
                        diff.base_brackets = painted;
                        let base = diff.base.clone();
                        let marks = diff.base_bracket_marks.get_or_insert_with(|| {
                            base.update(cx, |state, cx| {
                                state.create_decorations_collection(Vec::new(), cx)
                            })
                        });
                        marks.set(decorations, cx);
                        diff.base_marked = Some(generation);
                    }
                }
                let red: Hsla = gpui_kit::rgba(0xff000033).into();
                let green: Hsla = gpui_kit::rgba(0x9bb95533).into();
                diff.hunks = hunks;
                diff.hidden = hidden;
                // Monaco's diff editor colors (its defaults, which the Swift
                // app's themes leave alone).
                let red_word: Hsla = gpui_kit::rgba(0xff000033).into();
                let green_word: Hsla = gpui_kit::rgba(0x9ccc2c33).into();
                diff.base_tints = Tints {
                    words: removed_words.into_iter().map(|r| (r, red_word)).collect(),
                    ..tints(removed, red, "−")
                };
                diff.own_tints = Tints {
                    words: added_words.into_iter().map(|r| (r, green_word)).collect(),
                    ..tints(added, green, "+")
                };
                if let Some(view) = diff.inline.as_mut() {
                    if view.rows.text != inline.text {
                        view.pending = Some(inline.text.clone());
                    }
                    let mut all = tints(inline.removed.clone(), red, "−");
                    all.rows
                        .extend(tints(inline.added.clone(), green, "+").rows);
                    view.tints = all;
                    view.rows = inline;
                    view.hidden = inline_hidden;
                }
                cx.notify();
            });
        }));
    }

    /// The file's git base changed (staged, committed, checked out): the
    /// Diff View compares against the new one.
    pub(crate) fn diff_base_changed(&mut self, base: &str, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.as_mut() else {
            return;
        };
        if diff.base_text == base {
            return;
        }
        diff.base_text = base.to_string();
        diff.base_pending = Some(base.to_string());
        diff.base_generation += 1;
        self.refresh_diff(Duration::ZERO, cx);
        cx.notify();
    }

    /// The inline document's new text, once there's a window to set it in.
    pub(crate) fn apply_inline_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut new_base = false;
        if let Some(diff) = self.diff.as_mut() {
            if let Some(text) = diff.base_pending.take() {
                diff.base
                    .update(cx, |state, cx| state.set_value(text, window, cx));
                // Its zones and hidden lines are set again for the new text.
                diff.zones.key = None;
                new_base = true;
            }
        }
        if new_base {
            // Its bracket colors, now that it holds the text.
            self.refresh_diff(Duration::ZERO, cx);
        }
        let Some(view) = self.diff.as_mut().and_then(|d| d.inline.as_mut()) else {
            return;
        };
        if let Some(text) = view.pending.take() {
            view.state
                .update(cx, |state, cx| state.set_value(text, window, cx));
            view.applied = None;
        }
    }

    /// Show the unchanged buffer rows `rows` (a hidden run's).
    fn reveal_hidden(&mut self, rows: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.as_mut() else {
            return;
        };
        diff.revealed.push(rows);
        self.refresh_diff(Duration::ZERO, cx);
    }

    /// Show the first hidden run (the snapshot's `diff-reveal`).
    pub(crate) fn reveal_first_hidden(&mut self, cx: &mut Context<Self>) {
        let first = self.diff.as_ref().and_then(|d| {
            match &d.inline {
                Some(view) => view.hidden.first(),
                None => d.hidden.first(),
            }
            .map(|h| h.buffer_rows.clone())
        });
        if let Some(rows) = first {
            self.reveal_hidden(rows, cx);
        }
    }

    /// Take the Diff View's zones and hidden lines off the buffer's
    /// editor and the base's.
    fn clear_side_zones(&mut self, diff: &mut DiffView, cx: &mut Context<Self>) {
        let zones = std::mem::take(&mut diff.zones);
        self.state.update(cx, |state, cx| {
            state.replace_view_zones(&zones.own, Vec::new(), cx);
            state.set_hidden_ranges(Vec::new(), cx);
        });
        diff.base.update(cx, |state, cx| {
            state.replace_view_zones(&zones.base, Vec::new(), cx);
            state.set_hidden_ranges(Vec::new(), cx);
        });
    }

    /// Keep the editors' view zones and hidden lines in step with the
    /// changes (run before each render): side by side, each change's
    /// shorter side padded to the longer one's height in rows, as Monaco
    /// aligns them, and each hidden run's row on both sides; inline, the
    /// hidden runs' rows. Set again only when they change.
    pub(crate) fn sync_diff_zones(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut diff) = self.diff.take() else {
            return;
        };
        let tab = cx.entity().downgrade();
        if let Some(view) = diff.inline.as_mut() {
            // Again when an edit moved them (the document's text is
            // replaced as the buffer changes).
            let stale = {
                let state = view.state.read(cx);
                let expected: Vec<_> = view.hidden.iter().map(|h| h.rows.clone()).collect();
                state.hidden_ranges() != expected.as_slice()
                    || view.zones.iter().zip(&view.hidden).any(|(id, h)| {
                        state.display_map().view_zone(*id).map(|z| z.before_line)
                            != Some(h.rows.start)
                    })
            };
            if view.pending.is_none() && (stale || view.applied.as_ref() != Some(&view.hidden)) {
                let zones = view
                    .hidden
                    .iter()
                    .map(|h| hidden_zone(&tab, h.rows.start, h.buffer_rows.start, true))
                    .collect();
                let ranges = view.hidden.iter().map(|h| h.rows.clone()).collect();
                let old = std::mem::take(&mut view.zones);
                view.zones = view.state.update(cx, |state, cx| {
                    state.set_hidden_ranges(ranges, cx);
                    state.replace_view_zones(&old, zones, cx)
                });
                view.applied = Some(view.hidden.clone());
            }
            self.diff = Some(diff);
            return;
        }
        // Each change's rows on each side (wrapped lines count each row).
        let wrap = crate::settings_store::SettingsStore::get(cx).word_wrap;
        let rows = |state: &EditorState, lines: std::ops::Range<usize>| -> usize {
            if !wrap {
                return lines.len();
            }
            let map = state.display_map();
            lines
                .map(|line| map.visible_wrap_row_count_for_buffer_line(line).max(1))
                .sum()
        };
        let (own, base) = (self.state.read(cx), diff.base.read(cx));
        let text = own.text();
        let mut places: Vec<(bool, usize, usize)> = Vec::new();
        for hunk in &diff.hunks {
            let old_start = hunk.old_start.saturating_sub(1);
            let new_start = hunk.new_start.saturating_sub(1);
            let new_lines: Vec<String> = (new_start..new_start + hunk.new_count)
                .filter(|&row| row < text.lines_len())
                .map(|row| {
                    let at = text.line_start_offset(row);
                    text.slice(at..at + text.line_len(row)).to_string()
                })
                .collect();
            // Matching lines inside the change line up, as Monaco aligns
            // them; the rest of each stretch is padded on its shorter side.
            let matches = align_lines(&hunk.old_lines, &new_lines);
            let (mut old_at, mut new_at) = (old_start, new_start);
            let ends = matches
                .iter()
                .map(|&(o, n)| (old_start + o, new_start + n))
                .chain(std::iter::once((
                    old_start + hunk.old_lines.len(),
                    new_start + hunk.new_count,
                )));
            for (old_end, new_end) in ends {
                let old_rows = rows(base, old_at..old_end);
                let new_rows = rows(own, new_at..new_end);
                if old_rows > new_rows {
                    places.push((true, new_end, old_rows - new_rows));
                } else if new_rows > old_rows {
                    places.push((false, old_end, new_rows - old_rows));
                }
                // Past the matched pair, which may wrap differently.
                if old_end < old_start + hunk.old_lines.len() {
                    let (o, n) = (
                        rows(base, old_end..old_end + 1),
                        rows(own, new_end..new_end + 1),
                    );
                    if o > n {
                        places.push((true, new_end + 1, o - n));
                    } else if n > o {
                        places.push((false, old_end + 1, n - o));
                    }
                }
                (old_at, new_at) = (old_end + 1, new_end + 1);
            }
        }
        let key = (places, diff.hidden.clone());
        // Again when an edit moved them (a reload replaces the whole text).
        let stale = |state: &EditorState, zones: &[ViewZoneId], own: bool| {
            let lines = key
                .0
                .iter()
                .filter(|(side, _, _)| *side == own)
                .map(|&(_, line, _)| line)
                .chain(
                    key.1
                        .iter()
                        .map(|h| if own { h.rows.start } else { h.base_rows.start }),
                );
            let hidden: Vec<_> = key
                .1
                .iter()
                .map(|h| {
                    if own {
                        h.rows.clone()
                    } else {
                        h.base_rows.clone()
                    }
                })
                .collect();
            state.hidden_ranges() != hidden.as_slice()
                || zones.len() != lines.clone().count()
                || zones.iter().zip(lines).any(|(id, line)| {
                    state.display_map().view_zone(*id).map(|z| z.before_line) != Some(line)
                })
        };
        let stale = diff.zones.key.is_some()
            && (stale(own, &diff.zones.own, true) || stale(base, &diff.zones.base, false));
        if stale || diff.zones.key.as_ref() != Some(&key) {
            let pad = |side: bool| -> Vec<ViewZone> {
                key.0
                    .iter()
                    .filter(|(own, _, _)| *own == side)
                    .map(|&(_, line, lines)| {
                        ViewZone::new(line)
                            .lines(lines as f32)
                            .element(|zone, _, _| hatch(zone))
                    })
                    .chain(key.1.iter().map(|h| {
                        let at = if side {
                            h.rows.start
                        } else {
                            h.base_rows.start
                        };
                        hidden_zone(&tab, at, h.buffer_rows.start, side)
                    }))
                    .collect()
            };
            let (own_zones, base_zones) = (pad(true), pad(false));
            let own_hidden = key.1.iter().map(|h| h.rows.clone()).collect();
            let base_hidden = key.1.iter().map(|h| h.base_rows.clone()).collect();
            let old = std::mem::take(&mut diff.zones);
            let own = self.state.update(cx, |state, cx| {
                state.set_hidden_ranges(own_hidden, cx);
                state.replace_view_zones(&old.own, own_zones, cx)
            });
            let base = diff.base.update(cx, |state, cx| {
                state.set_hidden_ranges(base_hidden, cx);
                state.replace_view_zones(&old.base, base_zones, cx)
            });
            // A cursor in a hidden run goes to the first line after it, as
            // Monaco's does.
            let cursor = self.state.read(cx).cursor_position().line as usize;
            if let Some(run) = key.1.iter().find(|h| h.rows.contains(&cursor)) {
                let line = run.rows.end as u32;
                self.state.update(cx, |state, cx| {
                    state.set_cursor_position(Position::new(line, 0), window, cx)
                });
            }
            let runs = key.1.len();
            diff.zones = SideZones {
                key: Some(key),
                own_rows: own[own.len() - runs..].to_vec(),
                base_rows: base[base.len() - runs..].to_vec(),
                own,
                base,
            };
        }
        self.diff = Some(diff);
    }

    /// A hidden run's row, for the zone above its first line: "12 hidden
    /// lines" and the symbol it's in, the chevrons in the gutter (Monaco's
    /// unchanged region widget). A click shows the run.
    fn hidden_row(
        &self,
        first: usize,
        side: bool,
        zone: ViewZoneContext,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let diff = self.diff.as_ref()?;
        let hidden = match &diff.inline {
            Some(view) => &view.hidden,
            None => &diff.hidden,
        };
        let run = hidden
            .iter()
            .find(|h| h.buffer_rows.start == first)?
            .clone();
        let count = if side {
            run.rows.len()
        } else {
            run.base_rows.len()
        };
        let theme = cx.theme();
        let widget = cx
            .try_global::<crate::kit_theme::EditorBackdrop>()
            .map_or(theme.secondary, |b| b.widget);
        let (muted, hover, symbol) = (theme.muted_foreground, theme.foreground, theme.blue);
        let rows = run.buffer_rows.clone();
        let row = div()
            .id(("diff-hidden", first * 2 + side as usize))
            .size_full()
            .flex()
            .items_center()
            .bg(widget)
            .text_size(px(12.5))
            .font_family(crate::workbench::UI_FONT)
            .text_color(muted)
            .cursor_pointer()
            .hover(move |d| d.text_color(hover))
            .child(
                div()
                    .w(zone.text_left)
                    .flex_none()
                    .flex()
                    .justify_center()
                    .child(
                        gpui_kit::svg()
                            .path("icons/chevrons-up-down.svg")
                            .size(px(14.))
                            .text_color(muted),
                    ),
            )
            .child(format!("{count} hidden lines"))
            .when_some(run.label, |d, label| {
                d.child(div().px(px(6.)).child("|"))
                    .child(
                        gpui_kit::svg()
                            .path("icons/variable.svg")
                            .size(px(13.))
                            .mr(px(4.))
                            .text_color(symbol),
                    )
                    .child(label)
            })
            .on_click(cx.listener(move |tab, _, _, cx| tab.reveal_hidden(rows.clone(), cx)));
        Some(row.into_any_element())
    }

    /// ↑ ↓: the previous or next change, wrapping around.
    fn diff_go(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.as_ref() else {
            return;
        };
        if let Some(view) = &diff.inline {
            let row = view.state.read(cx).cursor_position().line as usize;
            let mut starts: Vec<usize> = view
                .rows
                .removed
                .iter()
                .chain(&view.rows.added)
                .map(|r| r.start as usize)
                .collect();
            starts.sort_unstable();
            starts.dedup();
            // A change is its removed lines and the added ones under them.
            starts.retain(|&s| {
                !view.rows.added.iter().any(|a| a.start as usize == s)
                    || !view.rows.removed.iter().any(|r| r.end as usize == s)
            });
            let target = if forward {
                starts.iter().find(|&&s| s > row).or(starts.first())
            } else {
                starts.iter().rev().find(|&&s| s < row).or(starts.last())
            };
            if let Some(&target) = target {
                center(&view.state.clone(), target, window, cx);
            }
            return;
        }
        let row = self.state.read(cx).cursor_position().line as usize;
        if let Some(target) = diff_view::next_change(&diff.hunks, row, forward) {
            center(&self.state.clone(), target, window, cx);
        }
    }

    /// The bar and the two sides (or the inline document).
    pub(crate) fn diff_element(&self, editor: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let Some(diff) = self.diff.as_ref() else {
            return editor;
        };
        let settings = crate::settings_store::SettingsStore::get(cx).clone();
        let styled = |state: &Entity<EditorState>| {
            let mut editor = Editor::new(state)
                .size_full()
                // No inset above line 1, as Monaco has none.
                .xsmall()
                .font_family(crate::fonts::family(&settings.font_family))
                .text_size(px(settings.font_size as f32))
                .pl(crate::editor_tab::gutter_inset(
                    state,
                    settings.font_size as f32,
                    cx,
                ))
                .bordered(false);
            if settings.editor_line_height > 0 {
                editor = editor.line_height(px(settings.editor_line_height as f32));
            }
            editor.font_features(if settings.font_ligatures {
                gpui_kit::FontFeatures::default()
            } else {
                gpui_kit::FontFeatures::disable_ligatures()
            })
        };
        let border = cx.theme().border;
        let signs = Signs {
            font: gpui_kit::font(crate::fonts::family(&settings.font_family)),
            size: px(settings.font_size as f32),
            color: cx.theme().muted_foreground,
        };
        let background = cx
            .try_global::<crate::kit_theme::EditorBackdrop>()
            .map(|b| b.background)
            .unwrap_or(cx.theme().background);
        let widget = cx
            .try_global::<crate::kit_theme::EditorBackdrop>()
            .map_or(cx.theme().secondary, |b| b.widget);
        let body = match &diff.inline {
            Some(view) => div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(tinted(
                    styled(&view.state),
                    &view.state,
                    &view.tints,
                    background,
                    &signs,
                    None,
                    view.zones.clone(),
                    widget,
                ))
                .into_any_element(),
            None => {
                let base_brackets = (!diff.base_brackets.pairs.is_empty()).then(|| {
                    (
                        diff.base_brackets.clone(),
                        crate::editor_brackets::BracketColors::new(cx.theme().mode.is_dark()),
                    )
                });
                let sync = scroll_sync(self.state.clone(), diff.base.clone(), diff.synced.clone());
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .flex()
                    .child(div().flex_1().min_w_0().h_full().child(tinted(
                        styled(&diff.base),
                        &diff.base,
                        &diff.base_tints,
                        background,
                        &signs,
                        base_brackets,
                        diff.zones.base_rows.clone(),
                        widget,
                    )))
                    .child(div().w(px(1.)).h_full().bg(border))
                    .child(div().flex_1().min_w_0().h_full().child(tinted(
                        editor,
                        &self.state,
                        &diff.own_tints,
                        background,
                        &signs,
                        None,
                        diff.zones.own_rows.clone(),
                        widget,
                    )))
                    .child(sync)
                    .into_any_element()
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.diff_bar(&diff.hunks, diff.inline.is_some(), cx))
            .child(body)
            .into_any_element()
    }

    fn diff_bar(&self, hunks: &[LineHunk], inline: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        // The web page's .git-peek-button: transparent with a faint border.
        let button =
            |id: &'static str, label: &'static str| Button::new(id).label(label).small().outline();
        let widget = cx
            .try_global::<crate::kit_theme::EditorBackdrop>()
            .map_or(theme.secondary, |b| b.widget);
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.title.clone());
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(4.))
            .text_size(px(12.))
            .bg(widget)
            .border_b_1()
            .border_color(theme.border)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("Index \u{2194} Working copy"),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .text_color(theme.foreground.opacity(0.8))
                    .child(diff_view::summary(hunks)),
            )
            .child(
                button("diff-previous", "↑")
                    .tooltip("Previous change")
                    .on_click(cx.listener(|tab, _, window, cx| tab.diff_go(false, window, cx))),
            )
            .child(
                button("diff-next", "↓")
                    .tooltip("Next change")
                    .on_click(cx.listener(|tab, _, window, cx| tab.diff_go(true, window, cx))),
            )
            .child(
                button(
                    "diff-layout",
                    if inline { "Side by Side" } else { "Inline" },
                )
                .tooltip("Switch between side by side and inline")
                .on_click(
                    cx.listener(move |tab, _, window, cx| tab.set_diff_inline(!inline, window, cx)),
                ),
            )
            .child(
                button("diff-done", "Done")
                    .tooltip("Back to the editor")
                    .on_click(cx.listener(|tab, _, window, cx| tab.close_diff_view(window, cx))),
            )
            .into_any_element()
    }
}

/// The cursor to the start of `row`, scrolled to the middle of the view
/// (Monaco's `revealPositionInCenter`).
pub(crate) fn center(
    state: &Entity<EditorState>,
    row: usize,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    center_at(state, Position::new(row as u32, 0), window, cx);
}

/// The cursor to `position`, its line scrolled to the middle of the view;
/// false before the editor's first layout, which this needs (the cursor
/// is put there either way).
pub(crate) fn center_at(
    state: &Entity<EditorState>,
    position: Position,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> bool {
    state.update(cx, |state, cx| {
        state.set_cursor_position(position, window, cx);
        let (Some(_), Some(lh)) = (state.visible_row_range(), state.line_height()) else {
            return false;
        };
        // In display rows, which wrapped and folded lines change.
        let row = state
            .display_map()
            .buffer_line_to_display_row(position.line as usize);
        let rows = (state.input_bounds().size.height / lh).floor() as usize;
        // From the row's top, view zones included, half a view up.
        let top =
            (state.display_map().display_row_top(row, lh) - lh * (rows / 2) as f32).max(px(0.));
        // The scroll set here replaces the one GPUI Kit's layout would
        // pick to show the cursor, so it has to show the column too: back
        // to the left edge where the column fits, else the column in the
        // middle (its place guessed from a character's width).
        let column = position.character as f32;
        let x = match (crate::editor_git::text_area(state), char_width(state)) {
            _ if column == 0. => px(0.),
            (Some(area), Some(width)) => {
                let shown = state.input_bounds().right() - area.text_left;
                let at = width * column;
                if at < shown - px(20.) {
                    px(0.)
                } else {
                    -(at - shown / 2.)
                }
            }
            _ => state.scroll_offset().x,
        };
        state.set_scroll_offset(point(x, -top), cx);
        true
    })
}

/// A character's width, measured on a visible line.
fn char_width(state: &EditorState) -> Option<Pixels> {
    let text = state.text();
    state.visible_row_range()?.find_map(|row| {
        if row >= text.lines_len() || text.line_len(row) == 0 {
            return None;
        }
        let start = text.line_start_offset(row);
        let width = text
            .slice(start..start + text.line_len(row))
            .chars()
            .next()?
            .len_utf8();
        Some(state.range_to_bounds(&(start..start + width))?.size.width)
    })
}

fn tints(rows: Vec<std::ops::Range<u32>>, color: Hsla, sign: &'static str) -> Tints {
    Tints {
        rows: rows
            .into_iter()
            .filter(|r| !r.is_empty())
            .map(|r| (r, color, sign))
            .collect(),
        words: Vec::new(),
    }
}

/// `editor` with `tints` painted over its rows across its whole width, as
/// Monaco's whole-line decorations are (a range decoration stops at the
/// end of the text).
/// How the + and − beside the line numbers look.
#[derive(Clone)]
struct Signs {
    font: gpui_kit::Font,
    size: Pixels,
    color: Hsla,
}

/// `rows` are the hidden runs' zones, whose rows reach left past the
/// editor's gutter to the pane's edge (the editor clips its zones to its
/// own padding), in `row_color`.
#[allow(clippy::too_many_arguments)]
fn tinted(
    editor: impl IntoElement,
    state: &Entity<EditorState>,
    tints: &Tints,
    background: Hsla,
    signs: &Signs,
    brackets: Option<(
        crate::editor_brackets::Brackets,
        crate::editor_brackets::BracketColors,
    )>,
    rows: Vec<ViewZoneId>,
    row_color: Hsla,
) -> AnyElement {
    let signs = signs.clone();
    let (state, tints) = (state.clone(), tints.clone());
    div()
        .relative()
        .size_full()
        // The theme leaves code editors' background to us.
        .bg(background)
        .child(editor)
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let state = state.read(cx);
                    let (Some(visible), Some(area)) = (
                        state.visible_row_range(),
                        crate::editor_git::text_area(state),
                    ) else {
                        return;
                    };
                    window.with_content_mask(
                        Some(gpui_kit::ContentMask {
                            bounds: area.view(bounds),
                        }),
                        |window| {
                            for id in &rows {
                                if let Some(zone) = state.view_zone_bounds(*id) {
                                    window.paint_quad(fill(
                                        Bounds::from_corners(
                                            point(bounds.left(), zone.top()),
                                            point(zone.left(), zone.bottom()),
                                        ),
                                        row_color,
                                    ));
                                }
                            }
                        },
                    );
                    let text = state.text();
                    let at = |row: usize| {
                        (row < text.lines_len())
                            .then(|| text.line_start_offset(row))
                            .and_then(|o| state.range_to_bounds(&(o..o)))
                    };
                    // The tints stay right of the line numbers when the text
                    // scrolls sideways.
                    let column = gpui_kit::ContentMask {
                        bounds: area.text_column(bounds.right()),
                    };
                    window.with_content_mask(Some(column), |window| {
                        // The base's bracket pair guides and matching
                        // brackets, as the buffer's own layer paints them.
                        if let Some((brackets, colors)) = &brackets {
                            brackets.paint(*colors, state, column.bounds, window);
                        }
                        for (rows, color, _) in &tints.rows {
                            let first = (rows.start as usize).max(visible.start);
                            let last = (rows.end as usize).min(visible.end);
                            if first >= last {
                                continue;
                            }
                            // Line by line, so the view zones between
                            // them (the other side's padding) stay clear;
                            // each line to its last wrapped row.
                            for row in first..last {
                                let end = (row < text.lines_len())
                                    .then(|| text.line_end_offset(row))
                                    .and_then(|o| state.range_to_bounds(&(o..o)));
                                if let (Some(top), Some(bottom)) = (at(row), end) {
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
                        let offset = |row: usize| {
                            if row < text.lines_len() {
                                text.line_start_offset(row)
                            } else {
                                text.len()
                            }
                        };
                        let shown = offset(visible.start)..offset(visible.end);
                        for (range, color) in &tints.words {
                            if range.end <= shown.start || range.start >= shown.end {
                                continue;
                            }
                            if let Some(word) = state.range_to_bounds(range) {
                                window.paint_quad(fill(word, *color));
                            }
                        }
                    });
                    // The diff editor's overview ruler: each change where
                    // it falls in the whole file, in the right edge's lane
                    // the buffer's change marks use.
                    let view = area.view(bounds);
                    // Where a line falls in the content, hidden lines and
                    // view zones counted as they show.
                    let map = state.display_map();
                    let lh = state.line_height().unwrap_or(px(1.));
                    // Over the content, or the view when that's taller, as
                    // Monaco's ruler spans its scroll height.
                    let total = f32::from(map.content_height(lh))
                        .max(f32::from(view.size.height))
                        .max(1.);
                    let place = |line: usize| {
                        let row = map.buffer_line_to_display_row(line);
                        f32::from(map.display_row_top(row, lh)) / total
                    };
                    for (rows, color, _) in &tints.rows {
                        let last = (rows.end as usize).saturating_sub(1);
                        let top = view.top() + view.size.height * place(rows.start as usize);
                        let bottom = (view.top()
                            + view.size.height * (place(last) + f32::from(lh) / total))
                            .max(top + px(3.));
                        window.paint_quad(fill(
                            Bounds::from_corners(
                                point(bounds.right() - px(10.), top),
                                point(bounds.right() - px(5.), bottom),
                            ),
                            Hsla { a: 0.7, ..*color },
                        ));
                    }
                    // Monaco's diff indicators, in the folding lane, fixed
                    // whatever the horizontal scroll (painted last: painting
                    // text needs the app, which `state` holds).
                    let mut spots = Vec::new();
                    for (rows, _, sign) in &tints.rows {
                        for row in (rows.start as usize).max(visible.start)
                            ..(rows.end as usize).min(visible.end)
                        {
                            if let Some(line) = at(row) {
                                spots.push((line, *sign));
                            }
                        }
                    }
                    let view = gpui_kit::ContentMask { bounds: view };
                    window.with_content_mask(Some(view), |window| {
                        for (line, sign) in spots {
                            let run = gpui_kit::TextRun {
                                len: sign.len(),
                                font: signs.font.clone(),
                                color: signs.color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let shaped = window.text_system().shape_line(
                                sign.into(),
                                signs.size,
                                &[run],
                                None,
                            );
                            let _ = shaped.paint(
                                point(area.text_left - px(14.), line.origin.y),
                                line.size.height,
                                gpui_kit::TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .into_any_element()
}

/// Keeps the two sides at one scroll offset: the padding zones make them
/// the same height line for line, so matching lines sit at the same
/// height, as in Monaco's diff editor. The side under the pointer leads
/// (the buffer when the pointer is elsewhere, as when typing).
fn scroll_sync(
    own: Entity<EditorState>,
    base: Entity<EditorState>,
    synced: Rc<Cell<Option<f32>>>,
) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let mouse = window.mouse_position();
            let base_leads =
                bounds.contains(&mouse) && mouse.x < bounds.left() + bounds.size.width / 2.;
            let (leader, follower) = if base_leads {
                (&base, &own)
            } else {
                (&own, &base)
            };
            let target = -f32::from(leader.read(cx).scroll_offset().y);
            let current = -f32::from(follower.read(cx).scroll_offset().y);
            if (target - current).abs() < 0.5 || synced.get() == Some(target) {
                return;
            }
            synced.set(Some(target));
            follower.update(cx, |state, cx| {
                let x = state.scroll_offset().x;
                state.set_scroll_offset(point(x, px(-target)), cx);
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

/// A hidden run's zone, above its first line (`at`) on one side: the
/// row the tab builds (`hidden_row`), a little taller than a line as
/// Monaco's is.
fn hidden_zone(
    tab: &gpui_kit::WeakEntity<EditorTab>,
    at: usize,
    first: usize,
    side: bool,
) -> ViewZone {
    let tab = tab.clone();
    ViewZone::new(at)
        .px(px(HIDDEN_ROW))
        .element(move |zone, _, cx| {
            tab.update(cx, |tab, cx| tab.hidden_row(first, side, zone, cx))
                .ok()
                .flatten()
                .unwrap_or_else(|| div().into_any_element())
        })
}

/// A hidden run's row: Monaco's unchanged region widget is 24 px.
const HIDDEN_ROW: f32 = 24.;

/// A padding zone's diagonal fill (Monaco's `diffEditor.diagonalFill`),
/// across the text column.
fn hatch(zone: ViewZoneContext) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let color: Hsla = if cx.theme().mode.is_dark() {
                gpui_kit::rgba(0xcccccc33).into()
            } else {
                gpui_kit::rgba(0x22222233).into()
            };
            let area = Bounds::from_corners(
                point(bounds.left() + zone.text_left, bounds.top()),
                bounds.bottom_right(),
            );
            let mut path = gpui_kit::PathBuilder::stroke(px(1.4));
            let height = area.size.height;
            // `/` strokes 8 px apart, as the CSS gradient's tiles draw them.
            let mut x = area.left() - height;
            while x < area.right() {
                path.move_to(point(x, area.bottom()));
                path.line_to(point(x + height, area.top()));
                x += px(8.);
            }
            if let Ok(path) = path.build() {
                window.with_content_mask(Some(gpui_kit::ContentMask { bounds: area }), |window| {
                    window.paint_path(path, color)
                });
            }
        },
    )
    .size_full()
    .into_any_element()
}

/// The lines of a change that match, as pairs of indexes into `old` and
/// `new` in order: the pairing with the most similarity (Dice over their
/// words), counting pairs at least half alike, as Monaco's diff editor
/// lines up a line and its edit. Larger changes aren't paired.
fn align_lines(old: &[String], new: &[String]) -> Vec<(usize, usize)> {
    let (m, n) = (old.len(), new.len());
    if m == 0 || n == 0 || m * n > 2500 {
        return Vec::new();
    }
    let words = |line: &str| -> std::collections::HashSet<String> {
        line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let (old_words, new_words): (Vec<_>, Vec<_>) = (
        old.iter().map(|l| words(l)).collect(),
        new.iter().map(|l| words(l)).collect(),
    );
    let similarity = |i: usize, j: usize| -> f32 {
        let (a, b) = (&old_words[i], &new_words[j]);
        if a.is_empty() || b.is_empty() {
            return 0.;
        }
        2. * a.intersection(b).count() as f32 / (a.len() + b.len()) as f32
    };
    // best[i][j]: the most similarity pairing old[i..] with new[j..].
    let mut best = vec![vec![0f32; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            let sim = similarity(i, j);
            let pair = if sim >= 0.5 {
                sim + best[i + 1][j + 1]
            } else {
                0.
            };
            best[i][j] = pair.max(best[i + 1][j]).max(best[i][j + 1]);
        }
    }
    let (mut i, mut j, mut pairs) = (0, 0, Vec::new());
    while i < m && j < n {
        let sim = similarity(i, j);
        if sim >= 0.5 && best[i][j] == sim + best[i + 1][j + 1] {
            pairs.push((i, j));
            (i, j) = (i + 1, j + 1);
        } else if best[i][j] == best[i + 1][j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// Where an unchanged run of `len` rows starting at `start` (of `total`)
/// hides, keeping the context lines beside the changes around it (none
/// at the document's ends), when enough of it would go.
fn hidden_part(start: usize, len: usize, total: usize) -> Option<std::ops::Range<usize>> {
    let end = start + len;
    let lead = if start == 0 {
        0
    } else {
        diff_view::CONTEXT_LINES
    };
    let trail = if end >= total {
        0
    } else {
        diff_view::CONTEXT_LINES
    };
    (len >= lead + trail + diff_view::MINIMUM_HIDDEN).then(|| start + lead..end - trail)
}

/// The symbol a hidden run sits in, as Monaco's row names it: the scope
/// around its last line (by indentation, as sticky scroll finds them),
/// named by its opening line.
fn run_label(lines: &[&str], last: usize, tab_size: usize) -> Option<String> {
    let scope = impulse_kit::sticky_scroll::scope_rows(lines, last, tab_size, 1);
    symbol_name(lines.get(*scope.last()?)?)
}

/// The name a scope's opening line gives: the word after a declaring
/// keyword (`function forecastFor`, `type Forecast`, `const forecast`),
/// else the word before its parameters.
fn symbol_name(line: &str) -> Option<String> {
    const DECLARING: &[&str] = &[
        "function",
        "class",
        "type",
        "interface",
        "enum",
        "struct",
        "trait",
        "impl",
        "fn",
        "def",
        "mod",
        "module",
        "namespace",
        "const",
        "let",
        "var",
        "func",
        "object",
        "record",
    ];
    let words: Vec<(usize, &str)> = line
        .match_indices(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .scan(0, |start, (at, sep)| {
            let word = (*start, &line[*start..at]);
            *start = at + sep.len();
            Some(word)
        })
        .chain(std::iter::once({
            let start = line
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                .map_or(0, |at| at + 1);
            (start, &line[start..])
        }))
        .filter(|(_, w)| !w.is_empty())
        .collect();
    let is_name = |w: &str| {
        w.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
            && !DECLARING.contains(&w)
    };
    if let Some(at) = words.iter().rposition(|(_, w)| DECLARING.contains(w)) {
        if let Some((_, name)) = words[at + 1..].iter().find(|(_, w)| is_name(w)) {
            return Some(name.to_string());
        }
    }
    let paren = line.find('(')?;
    words
        .iter()
        .rev()
        .find(|(start, w)| start + w.len() <= paren && is_name(w))
        .map(|(_, w)| w.to_string())
}

/// Side by side: the unchanged runs between the changes that hide, on the
/// buffer's side (`new`) and the base's, leaving out runs over `revealed`
/// buffer rows.
fn side_hidden(
    old: &str,
    new: &str,
    hunks: &[LineHunk],
    revealed: &[std::ops::Range<usize>],
    tab_size: usize,
) -> Vec<Hidden> {
    let new_lines: Vec<&str> = new.split('\n').collect();
    let old_len = old.split('\n').count();
    let mut hidden = Vec::new();
    let (mut new_at, mut old_at) = (0, 0);
    let ends = hunks
        .iter()
        .map(|h| {
            (
                h.new_start.saturating_sub(1),
                h.new_count,
                h.old_start.saturating_sub(1),
                h.old_lines.len(),
            )
        })
        .chain(std::iter::once((new_lines.len(), 0, old_len, 0)));
    for (new_start, new_count, old_start, old_count) in ends {
        let len = new_start
            .saturating_sub(new_at)
            .min(old_start.saturating_sub(old_at));
        // A run reaching the end of either side keeps no trailing context.
        let total = if new_at + len >= new_lines.len() || old_at + len >= old_len {
            new_at + len
        } else {
            usize::MAX
        };
        if let Some(rows) = hidden_part(new_at, len, total) {
            let base_rows = rows.start - new_at + old_at..rows.end - new_at + old_at;
            if !revealed
                .iter()
                .any(|r| r.start < rows.end && rows.start < r.end)
            {
                hidden.push(Hidden {
                    label: run_label(&new_lines, rows.end - 1, tab_size),
                    buffer_rows: rows.clone(),
                    rows,
                    base_rows,
                });
            }
        }
        new_at = new_start + new_count;
        old_at = old_start + old_count;
    }
    hidden
}

/// Inline: the unchanged runs of the document that hide, leaving out runs
/// over `revealed` buffer rows.
fn inline_hidden(
    diff: &diff_view::InlineDiff,
    text: &str,
    revealed: &[std::ops::Range<usize>],
    tab_size: usize,
) -> Vec<Hidden> {
    let lines: Vec<&str> = text.split('\n').collect();
    let rows = diff.buffer_rows.len();
    let changed = |row: usize| {
        let row = row as u32;
        diff.removed
            .iter()
            .chain(&diff.added)
            .any(|r| r.contains(&row))
    };
    let mut hidden = Vec::new();
    let mut row = 0;
    while row < rows {
        if changed(row) {
            row += 1;
            continue;
        }
        let start = row;
        while row < rows && !changed(row) {
            row += 1;
        }
        let Some(part) = hidden_part(start, row - start, rows) else {
            continue;
        };
        let (Some(Some(first)), Some(Some(last))) = (
            diff.buffer_rows.get(part.start),
            diff.buffer_rows.get(part.end - 1),
        ) else {
            continue;
        };
        let buffer_rows = *first..*last + 1;
        if revealed
            .iter()
            .any(|r| r.start < buffer_rows.end && buffer_rows.start < r.end)
        {
            continue;
        }
        hidden.push(Hidden {
            label: run_label(&lines, part.end - 1, tab_size),
            base_rows: part.clone(),
            rows: part,
            buffer_rows,
        });
    }
    hidden
}

#[cfg(test)]
mod tests {
    use super::*;
    use impulse_kit::git::editor_marks::line_hunks;

    fn numbered(lines: std::ops::Range<usize>) -> String {
        lines.map(|n| format!("line {n}\n")).collect()
    }

    #[test]
    fn unchanged_runs_hide_on_both_sides_with_context() {
        // 30 lines; line 10 changed, two lines added after line 20.
        let old = numbered(0..30);
        let new = old
            .replace("line 10\n", "LINE 10\n")
            .replace("line 20\n", "line 20\nadded\nadded\n");
        let hunks = line_hunks(&old, &new);
        let hidden = side_hidden(&old, &new, &hunks, &[], 4);
        let rows: Vec<_> = hidden
            .iter()
            .map(|h| (h.rows.clone(), h.base_rows.clone()))
            .collect();
        assert_eq!(
            rows,
            [
                // From the top to three lines before the change.
                (0..7, 0..7),
                // Between the changes, three lines either side.
                (14..18, 14..18),
                // After the addition to the end (the final empty line too).
                (26..33, 24..31),
            ]
        );
        // Revealed rows stay shown.
        let hidden = side_hidden(&old, &new, &hunks, &[15..16], 4);
        assert_eq!(hidden.len(), 2);
    }

    #[test]
    fn lines_of_a_change_pair_with_their_edits() {
        let old: Vec<String> = [
            "const TTL_MS = 60_000;",
            "const cache = new TTLCache<Forecast>(TTL_MS);",
        ]
        .map(String::from)
        .to_vec();
        let new: Vec<String> = [
            "// Ten minutes unless FORECAST_TTL_MS says otherwise.",
            "const DEFAULT_TTL_MS = 10 * 60 * 1000;",
            "const TTL_MS = Number(process.env.FORECAST_TTL_MS ?? DEFAULT_TTL_MS);",
            "export const forecastCache = new TTLCache<Forecast>(TTL_MS);",
        ]
        .map(String::from)
        .to_vec();
        // As the Swift app's shot pairs them: the cache lines.
        assert_eq!(align_lines(&old, &new), [(1, 3)]);
        assert!(align_lines(&old, &[]).is_empty());
    }

    #[test]
    fn short_runs_stay_shown() {
        let old = numbered(0..8);
        let new = old.replace("line 4\n", "LINE 4\n");
        let hunks = line_hunks(&old, &new);
        // Four lines before the change: one would hide, fewer than three.
        assert!(side_hidden(&old, &new, &hunks, &[], 4).is_empty());
    }

    #[test]
    fn inline_runs_map_to_buffer_rows() {
        let old = numbered(0..20);
        let new = old.replace("line 10\n", "LINE 10\n");
        let hunks = line_hunks(&old, &new);
        let inline = diff_view::inline(&new, &hunks);
        let hidden = inline_hidden(&inline, &inline.text, &[], 4);
        // The removed line 10 sits above the buffer's line 10 (row 11).
        let rows: Vec<_> = hidden
            .iter()
            .map(|h| (h.rows.clone(), h.buffer_rows.clone()))
            .collect();
        assert_eq!(rows, [(0..7, 0..7), (15..22, 14..21)]);
    }

    #[test]
    fn runs_are_named_by_the_scope_around_them() {
        assert_eq!(
            symbol_name("export type Forecast = {"),
            Some("Forecast".into())
        );
        assert_eq!(
            symbol_name("  const forecast: Forecast = {"),
            Some("forecast".into())
        );
        assert_eq!(
            symbol_name("export async function forecastFor(trailId: string): P {"),
            Some("forecastFor".into())
        );
        assert_eq!(symbol_name("  render(props) {"), Some("render".into()));
        assert_eq!(symbol_name("}"), None);
        let lines = ["type A = {", "  x: number;", "  y: number;", "};"];
        assert_eq!(run_label(&lines, 2, 4), Some("A".into()));
    }
}
