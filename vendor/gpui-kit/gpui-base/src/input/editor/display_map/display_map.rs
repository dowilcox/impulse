/// DisplayMap: Public facade for Editor/Input display mapping.
///
/// This combines WrapMap and FoldMap to provide a unified API:
/// - BufferPoint ↔ DisplayPoint conversion
/// - Fold management (candidates, toggle, query)
/// - Automatic projection updates on text/layout changes
use std::ops::Range;

use gpui::{App, Font, Pixels};
use ropey::Rope;

use super::fold_map::FoldMap;
use super::folding::FoldRange;
use super::view_zones::{ViewZone, ViewZoneId, ViewZones};
pub use super::text_wrapper::WrappingIndent;
use super::text_wrapper::{LineItem, WrapDisplayPoint};
use super::wrap_map::WrapMap;
use super::{BufferPoint, DisplayPoint};
use crate::input::Point as TreeSitterPoint;
use crate::input::display_map::WrapPoint;
use crate::input::rope_ext::RopeExt as _;

/// DisplayMap is the main interface for Editor/Input coordinate mapping.
///
/// It manages the two-layer projection:
/// 1. Buffer → Wrap (soft-wrapping)
/// 2. Wrap → Display (folding)
///
/// Editor/Input only needs to work with BufferPoint and DisplayPoint.
pub struct DisplayMap {
    wrap_map: WrapMap,
    fold_map: FoldMap,
    /// Impulse: blank rows between lines (Monaco's view zones).
    view_zones: ViewZones,
}

impl DisplayMap {
    pub fn new(font: Font, font_size: Pixels, wrap_width: Option<Pixels>) -> Self {
        Self {
            wrap_map: WrapMap::new(font, font_size, wrap_width),
            fold_map: FoldMap::new(),
            view_zones: ViewZones::default(),
        }
    }

    // ==================== Core Coordinate Mapping ====================

    /// Convert buffer position to display position
    pub fn buffer_pos_to_display_pos(&self, pos: BufferPoint) -> DisplayPoint {
        // Buffer → Wrap
        let wrap_pos = self.wrap_map.buffer_pos_to_wrap_pos(pos);

        // Wrap → Display
        if let Some(display_row) = self.fold_map.wrap_row_to_display_row(wrap_pos.row) {
            DisplayPoint::new(display_row, wrap_pos.col)
        } else {
            // Cursor is in a folded region, find nearest visible row
            let display_row = self.fold_map.nearest_visible_display_row(wrap_pos.row);
            DisplayPoint::new(display_row, 0) // Column 0 at fold boundary
        }
    }

    /// Convert display position to buffer position
    pub fn display_pos_to_buffer_pos(&self, pos: DisplayPoint) -> BufferPoint {
        // Display → Wrap
        let wrap_row = self.fold_map.display_row_to_wrap_row(pos.row).unwrap_or(0);

        // Wrap → Buffer
        let wrap_pos = WrapPoint::new(wrap_row, pos.col);
        self.wrap_map.wrap_pos_to_buffer_pos(wrap_pos)
    }

    /// Get total number of visible display rows
    #[inline]
    pub fn display_row_count(&self) -> usize {
        self.fold_map.display_row_count()
    }

    /// Get the buffer line for a given display row
    pub fn display_row_to_buffer_line(&self, display_row: usize) -> usize {
        // Display → Wrap
        let wrap_row = self
            .fold_map
            .display_row_to_wrap_row(display_row)
            .unwrap_or(0);

        // Wrap → Buffer line
        self.wrap_map.wrap_row_to_buffer_line(wrap_row)
    }

    /// Get the display row range for a buffer line: [start, end)
    /// Returns None if the buffer line is completely hidden
    pub fn buffer_line_to_display_row_range(&self, line: usize) -> Option<Range<usize>> {
        // Buffer line → Wrap row range
        let wrap_row_range = self.wrap_map.buffer_line_to_wrap_row_range(line);

        // Find first and last visible display rows in this range
        let mut first_display_row = None;
        let mut last_display_row = None;

        for wrap_row in wrap_row_range {
            if let Some(display_row) = self.fold_map.wrap_row_to_display_row(wrap_row) {
                if first_display_row.is_none() {
                    first_display_row = Some(display_row);
                }
                last_display_row = Some(display_row);
            }
        }

        if let (Some(start), Some(end)) = (first_display_row, last_display_row) {
            Some(start..end + 1)
        } else {
            None // Completely folded
        }
    }

    /// Check if a buffer line is completely hidden
    #[inline]
    pub fn is_buffer_line_hidden(&self, line: usize) -> bool {
        self.buffer_line_to_display_row_range(line).is_none()
    }

    /// First display row of a buffer line. If the line is fully folded, returns the
    /// nearest visible display row.
    pub fn buffer_line_to_display_row(&self, line: usize) -> usize {
        match self.buffer_line_to_display_row_range(line) {
            Some(range) => range.start,
            None => {
                let wrap_row = self.wrap_map.buffer_line_to_first_wrap_row(line);
                self.fold_map.nearest_visible_display_row(wrap_row)
            }
        }
    }

    /// Set fold candidates (from tree-sitter/LSP)
    pub fn set_fold_candidates(&mut self, candidates: Vec<FoldRange>) {
        self.fold_map.set_candidates(candidates);
        self.rebuild_fold_projection();
    }

    /// Set a fold at the given start_line (must be in candidates)
    pub fn set_folded(&mut self, start_line: usize, folded: bool) {
        self.fold_map.set_folded(start_line, folded);
        self.rebuild_fold_projection();
    }

    /// Toggle fold at the given start_line
    pub fn toggle_fold(&mut self, start_line: usize) {
        self.fold_map.toggle_fold(start_line);
        self.rebuild_fold_projection();
    }

    /// Check if a line is currently folded
    #[inline]
    pub fn is_folded_at(&self, start_line: usize) -> bool {
        self.fold_map.is_folded_at(start_line)
    }

    /// Check if a line is a fold candidate
    #[inline]
    pub fn is_fold_candidate(&self, start_line: usize) -> bool {
        self.fold_map.is_fold_candidate(start_line)
    }

    /// Impulse: every range that can fold, for folding from the keyboard.
    #[inline]
    pub fn fold_candidates(&self) -> &[FoldRange] {
        self.fold_map.fold_candidates()
    }

    /// Get all currently folded ranges
    #[inline]
    pub fn folded_ranges(&self) -> &[FoldRange] {
        self.fold_map.folded_ranges()
    }

    /// Clear all folds
    pub fn clear_folds(&mut self) {
        self.fold_map.clear_folds();
        self.rebuild_fold_projection();
    }

    // ==================== Text and Layout Updates ====================

    /// Adjust folds and candidates for a text edit before updating the wrap map.
    ///
    /// Must be called with the OLD text (before replacement) and the edit range/new_text
    /// so we can compute which old lines were affected.
    pub fn adjust_folds_for_edit(&mut self, old_text: &Rope, range: &Range<usize>, new_text: &str) {
        // Impulse: the host's hidden lines and view zones follow edits too.
        if self.fold_map.folded_ranges().is_empty()
            && self.fold_map.fold_candidates().is_empty()
            && self.fold_map.host_hidden().is_empty()
            && self.view_zones.is_empty()
        {
            return;
        }

        let edit_start_line = old_text.offset_to_point(range.start).row;
        let edit_end_line = old_text.offset_to_point(range.end.min(old_text.len())).row;

        let old_lines_in_range = edit_end_line.saturating_sub(edit_start_line);
        let new_lines_in_range = new_text.chars().filter(|c| *c == '\n').count();
        let line_delta = new_lines_in_range as isize - old_lines_in_range as isize;

        self.fold_map
            .adjust_folds_for_edit(edit_start_line, edit_end_line, line_delta);
        // Impulse: view zones move with their lines.
        self.view_zones
            .adjust_for_edit(edit_start_line, edit_end_line, line_delta);
    }

    /// Incrementally update fold candidates after a text edit.
    ///
    /// Extracts new fold candidates only within the edited byte range
    /// and merges them with existing (already adjusted) candidates.
    pub fn update_fold_candidates_for_edit(
        &mut self,
        extract_fold_ranges: impl FnOnce(Range<usize>, &Rope) -> Vec<FoldRange>,
        edit_byte_range: Range<usize>,
        new_text: &Rope,
    ) {
        let new_start_line = new_text.offset_to_point(edit_byte_range.start).row;
        let new_end_line = new_text
            .offset_to_point(edit_byte_range.end.min(new_text.len()))
            .row;

        let new_candidates = extract_fold_ranges(edit_byte_range, new_text);
        self.fold_map
            .merge_candidates_for_edit(new_start_line, new_end_line, new_candidates);
    }

    /// Update text (incremental or full)
    pub fn on_text_changed(
        &mut self,
        changed_text: &Rope,
        range: &Range<usize>,
        new_text: &Rope,
        cx: &mut App,
    ) {
        self.wrap_map
            .on_text_changed(changed_text, range, new_text, cx);
        self.rebuild_fold_projection();
    }

    /// Update layout parameters (wrap width or font)
    pub(crate) fn set_inline_metrics(
        &mut self,
        metrics: std::rc::Rc<[(std::ops::Range<usize>, Pixels)]>,
        cx: &mut App,
    ) {
        self.wrap_map.set_inline_metrics(metrics, cx);
        self.rebuild_fold_projection();
    }

    pub fn on_layout_changed(&mut self, wrap_width: Option<Pixels>, cx: &mut App) {
        self.wrap_map.on_layout_changed(wrap_width, cx);
        self.rebuild_fold_projection();
    }

    /// Set the wrapping indent for continuation lines.
    pub fn set_wrapping_indent(&mut self, wrapping_indent: WrappingIndent, cx: &mut App) {
        self.wrap_map.set_wrapping_indent(wrapping_indent, cx);
        self.rebuild_fold_projection();
    }

    /// Set font parameters
    pub fn set_font(&mut self, font: Font, font_size: Pixels, cx: &mut App) {
        self.wrap_map.set_font(font, font_size, cx);
        self.rebuild_fold_projection();
    }

    /// Ensure text is prepared (initializes wrapper if needed)
    pub fn ensure_text_prepared(&mut self, text: &Rope, cx: &mut App) {
        let did_initialize = self.wrap_map.ensure_text_prepared(text, cx);
        if did_initialize {
            self.rebuild_fold_projection();
        }
    }

    /// Initialize with text
    pub fn set_text(&mut self, text: &Rope, cx: &mut App) {
        self.wrap_map.set_text(text, cx);
        self.rebuild_fold_projection();
    }

    // ==================== Internal Helpers ====================

    /// Rebuild fold projection after wrap_map or fold state changes
    /// Only rebuilds if there are actually folded ranges
    fn rebuild_fold_projection(&mut self) {
        // Impulse: the host's hidden lines need the projection too.
        if self.fold_map.has_hidden() {
            self.fold_map.rebuild(&self.wrap_map);
        } else {
            // No active folds: identity mapping (wrap_row == display_row).
            // Just update cached count so query methods work without Vec allocation.
            self.fold_map
                .mark_dirty_with_wrap_count(self.wrap_map.wrap_row_count());
        }
        // Impulse: the view zones follow the rows.
        self.place_view_zones();
    }

    /// Impulse: put each view zone under its line's last display row, or
    /// hide it with that line.
    fn place_view_zones(&mut self) {
        if self.view_zones.is_empty() {
            self.view_zones.place(0, |_| None);
            return;
        }
        let line_count = self.wrap_map.buffer_line_count();
        let (wrap_map, fold_map) = (&self.wrap_map, &self.fold_map);
        self.view_zones.place(line_count, |line| {
            let last = wrap_map.buffer_line_to_wrap_row_range(line).end.checked_sub(1)?;
            fold_map.wrap_row_to_display_row(last).map(|row| row + 1)
        });
    }

    // ==================== Impulse: view zones and hidden lines ====================

    /// Impulse: add a view zone (Monaco's `changeViewZones` `addZone`).
    pub fn add_view_zone(&mut self, zone: ViewZone) -> ViewZoneId {
        let id = self.view_zones.add(zone);
        self.place_view_zones();
        id
    }

    /// Impulse: remove a view zone; whether it was there.
    pub fn remove_view_zone(&mut self, id: ViewZoneId) -> bool {
        let removed = self.view_zones.remove(id);
        if removed {
            self.place_view_zones();
        }
        removed
    }

    /// Impulse: remove every view zone.
    pub fn clear_view_zones(&mut self) {
        self.view_zones.clear();
        self.place_view_zones();
    }

    /// Impulse: a view zone, as it now is (its line follows edits).
    pub fn view_zone(&self, id: ViewZoneId) -> Option<&ViewZone> {
        self.view_zones.get(id)
    }

    /// Impulse: hide these buffer lines (end exclusive), replacing the lines
    /// hidden before (Monaco's `setHiddenAreas`). Unlike folds, they stay
    /// through reparses; they move with edits, and an edit inside one shows
    /// it again. A host puts a view zone above a range to stand for it.
    pub fn set_hidden_ranges(&mut self, ranges: Vec<Range<usize>>) {
        self.fold_map.set_host_hidden(ranges);
        self.rebuild_fold_projection();
    }

    /// Impulse: the buffer lines the host hides.
    pub fn hidden_ranges(&self) -> &[Range<usize>] {
        self.fold_map.host_hidden()
    }

    /// Impulse: the run of hidden buffer lines holding `line` (folded or
    /// hidden by the host), or `None` when it shows.
    pub fn hidden_lines_at(&self, line: usize) -> Option<Range<usize>> {
        let wrap_row = self.wrap_map.buffer_line_to_first_wrap_row(line);
        let rows = self.fold_map.hidden_run_at(wrap_row)?;
        let start = self.wrap_map.wrap_row_to_buffer_line(rows.start);
        let end = if rows.end < self.wrap_map.wrap_row_count() {
            self.wrap_map.wrap_row_to_buffer_line(rows.end)
        } else {
            self.buffer_line_count()
        };
        Some(start..end)
    }

    /// Impulse: the top of display row `row` in content space: the rows
    /// above it and the view zones above it.
    pub fn display_row_top(&self, row: usize, line_height: Pixels) -> Pixels {
        line_height * row as f32 + self.view_zones.height_above_row(row, line_height)
    }

    /// Impulse: the display row at `y` in content space; a `y` in a view
    /// zone gives the row above the zone (row 0 above the first row).
    pub fn display_row_at_y(&self, y: Pixels, line_height: Pixels) -> usize {
        let count = self.display_row_count();
        if count == 0 {
            return 0;
        }
        // The first row whose top is past `y`, less one.
        let (mut low, mut high) = (0, count);
        while low < high {
            let mid = (low + high) / 2;
            if self.display_row_top(mid, line_height) <= y {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low.saturating_sub(1)
    }

    /// Impulse: the height of every display row and view zone (the
    /// content's height, before any room under the last line).
    pub fn content_height(&self, line_height: Pixels) -> Pixels {
        line_height * self.display_row_count() as f32 + self.view_zones.total_height(line_height)
    }

    /// Impulse: the height of the view zones under buffer line `line` (none
    /// when it is hidden).
    pub(crate) fn view_zone_height_after_line(&self, line: usize, line_height: Pixels) -> Pixels {
        if self.view_zones.is_empty() {
            return Pixels::ZERO;
        }
        match self.buffer_line_to_display_row_range(line) {
            Some(rows) => self.view_zones.height_at_row(rows.end, line_height),
            None => Pixels::ZERO,
        }
    }

    /// Impulse: the view zones that show over display rows `rows` (zones
    /// under the last one included), each with its top in content space and
    /// its height.
    pub(crate) fn view_zone_layouts(
        &self,
        line_height: Pixels,
        rows: Range<usize>,
    ) -> impl Iterator<Item = (ViewZoneId, &ViewZone, Pixels, Pixels)> + '_ {
        self.view_zones.layouts(line_height, rows)
    }

    // ==================== Wrap Display Point Operations ====================

    /// Convert byte offset to wrap display point (with soft wrap info).
    #[inline]
    pub(crate) fn offset_to_wrap_display_point(&self, offset: usize) -> WrapDisplayPoint {
        self.wrap_map.wrapper().offset_to_display_point(offset)
    }

    /// Like [`Self::offset_to_wrap_display_point`], but honours the caret's line-end affinity so
    /// an offset on a soft wrap boundary resolves to the row the caret is drawn on.
    #[inline]
    pub(crate) fn offset_to_wrap_display_point_with_affinity(
        &self,
        offset: usize,
        line_end_affinity: bool,
    ) -> WrapDisplayPoint {
        self.wrap_map
            .wrapper()
            .offset_to_display_point_with_affinity(offset, line_end_affinity)
    }

    /// Resolve the caret's visual row through both wrapping and folded regions.
    pub(crate) fn offset_to_display_row(&self, offset: usize, line_end_affinity: bool) -> usize {
        let point = self.offset_to_wrap_display_point_with_affinity(offset, line_end_affinity);
        self.wrap_row_to_display_row(point.row)
            .unwrap_or_else(|| self.nearest_visible_display_row(point.row))
    }

    /// Convert wrap display point to byte offset.
    #[inline]
    pub(crate) fn wrap_display_point_to_offset(&self, point: WrapDisplayPoint) -> usize {
        self.wrap_map.wrapper().display_point_to_offset(point)
    }

    /// Convert wrap display point to TreeSitterPoint (buffer line/col).
    #[inline]
    pub(crate) fn wrap_display_point_to_point(&self, point: WrapDisplayPoint) -> TreeSitterPoint {
        self.wrap_map.wrapper().display_point_to_point(point)
    }

    /// Convert a wrap row to a display row (skipping folded rows).
    /// Returns None if the wrap row is folded.
    #[inline]
    pub fn wrap_row_to_display_row(&self, wrap_row: usize) -> Option<usize> {
        self.fold_map.wrap_row_to_display_row(wrap_row)
    }

    /// Find the nearest visible display row for a given wrap row.
    #[inline]
    pub fn nearest_visible_display_row(&self, wrap_row: usize) -> usize {
        self.fold_map.nearest_visible_display_row(wrap_row)
    }

    /// Convert a display row to a wrap row.
    #[inline]
    pub fn display_row_to_wrap_row(&self, display_row: usize) -> Option<usize> {
        self.fold_map.display_row_to_wrap_row(display_row)
    }

    pub(crate) fn display_row_column_to_offset(&self, display_row: usize, column: usize) -> usize {
        let wrap_row = self
            .fold_map
            .display_row_to_wrap_row(display_row)
            .unwrap_or(0);
        self.wrap_map
            .wrapper()
            .display_point_to_offset(WrapDisplayPoint::new(wrap_row, 0, column))
    }

    /// Get the longest row index (by byte length).
    #[inline]
    pub(crate) fn longest_row(&self) -> usize {
        self.wrap_map.wrapper().longest_row()
    }

    // ==================== Access Methods ====================

    /// Get the line item by buffer row index.
    #[inline]
    pub(crate) fn line(&self, row: usize) -> Option<&LineItem> {
        self.wrap_map.line(row)
    }

    /// Get the rope text
    #[inline]
    pub fn text(&self) -> &Rope {
        self.wrap_map.text()
    }

    /// Calculate how many wrap rows of a buffer line are visible (not folded)
    #[inline]
    pub fn visible_wrap_row_count_for_buffer_line(&self, line: usize) -> usize {
        self.wrap_map
            .visible_wrap_row_count_for_line(line, &self.fold_map)
    }

    /// Get the wrap row count (before folding)
    #[inline]
    pub fn wrap_row_count(&self) -> usize {
        self.wrap_map.wrap_row_count()
    }

    /// Get the buffer line count (logical lines)
    #[inline]
    pub fn buffer_line_count(&self) -> usize {
        self.wrap_map.buffer_line_count()
    }
}
