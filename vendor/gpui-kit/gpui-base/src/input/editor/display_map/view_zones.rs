//! Impulse: view zones, Monaco's `changeViewZones`: blank rows of a given
//! height between two buffer lines (or above the first). They take no
//! buffer text and have no line numbers; the lines under them move down,
//! and the caret and selections step over them, since they aren't display
//! rows. A host paints into one, or gives it an element, and reads its
//! bounds from the last layout.
//!
//! A zone sits under the line before `before_line`, and hides with it when
//! that line is folded or hidden, as Monaco hides a zone in a hidden area.
//! Zones are placed after the fold projection, by display row: a zone
//! placed at display row `r` sits above row `r` and below row `r - 1`.
use std::ops::Range;
use std::rc::Rc;

use gpui::{AnyElement, App, Pixels, Window, px};

/// Identifies a view zone in the editor that made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewZoneId(u64);

/// What a zone's element is told when it is built: which zone, and where
/// the text column starts, measured from the zone's left edge (the zone
/// spans the gutter too).
#[derive(Debug, Clone, Copy)]
pub struct ViewZoneContext {
    pub id: ViewZoneId,
    pub text_left: Pixels,
}

/// Builds a zone's element, each frame it is on screen. It is laid out in
/// the zone's bounds and painted over them.
pub type ViewZoneRenderer = Rc<dyn Fn(ViewZoneContext, &mut Window, &mut App) -> AnyElement>;

/// A view zone: where it goes, how tall it is (`height_in_lines` line
/// heights plus `height_in_px`), and optionally what it shows.
#[derive(Clone)]
pub struct ViewZone {
    /// The zone sits above this buffer line (0-based): under line
    /// `before_line - 1`, or above the first line when 0. The line count
    /// (or more) puts it under the last line.
    pub before_line: usize,
    pub height_in_lines: f32,
    pub height_in_px: Pixels,
    pub render: Option<ViewZoneRenderer>,
}

impl ViewZone {
    /// An empty zone above buffer line `before_line`, with no height yet.
    pub fn new(before_line: usize) -> Self {
        Self {
            before_line,
            height_in_lines: 0.,
            height_in_px: px(0.),
            render: None,
        }
    }

    /// A zone under buffer line `line`.
    pub fn after_line(line: usize) -> Self {
        Self::new(line + 1)
    }

    /// Its height in line heights (Monaco's `heightInLines`).
    pub fn lines(mut self, lines: f32) -> Self {
        self.height_in_lines = lines;
        self
    }

    /// Its height in pixels, added to the lines (Monaco's `heightInPx`).
    pub fn px(mut self, height: Pixels) -> Self {
        self.height_in_px = height;
        self
    }

    /// What it shows: an element built each frame it's on screen.
    pub fn element(
        mut self,
        render: impl Fn(ViewZoneContext, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.render = Some(Rc::new(render));
        self
    }

    fn height(&self, line_height: Pixels) -> Pixels {
        (line_height * self.height_in_lines + self.height_in_px).max(px(0.))
    }
}

impl std::fmt::Debug for ViewZone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewZone")
            .field("before_line", &self.before_line)
            .field("height_in_lines", &self.height_in_lines)
            .field("height_in_px", &self.height_in_px)
            .field("render", &self.render.is_some())
            .finish()
    }
}

/// A zone that shows, where the projection put it.
#[derive(Debug, Clone, Copy)]
struct Placed {
    /// Into `ViewZones::zones`.
    index: usize,
    /// The display row it sits above.
    display_row: usize,
    /// The lines and pixels of the placed zones before this one.
    lines_before: f32,
    px_before: Pixels,
}

/// The editor's view zones and where they show.
#[derive(Default)]
pub(super) struct ViewZones {
    /// Sorted by `before_line`, then by id (the order they were added).
    zones: Vec<(ViewZoneId, ViewZone)>,
    next_id: u64,
    /// The zones that show, in display order.
    placed: Vec<Placed>,
    total_lines: f32,
    total_px: Pixels,
}

impl ViewZones {
    pub(super) fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    pub(super) fn add(&mut self, zone: ViewZone) -> ViewZoneId {
        self.next_id += 1;
        let id = ViewZoneId(self.next_id);
        let at = self
            .zones
            .partition_point(|(_, z)| z.before_line <= zone.before_line);
        self.zones.insert(at, (id, zone));
        id
    }

    pub(super) fn remove(&mut self, id: ViewZoneId) -> bool {
        let before = self.zones.len();
        self.zones.retain(|(zone_id, _)| *zone_id != id);
        self.zones.len() != before
    }

    pub(super) fn clear(&mut self) {
        self.zones.clear();
    }

    pub(super) fn get(&self, id: ViewZoneId) -> Option<&ViewZone> {
        self.zones
            .iter()
            .find(|(zone_id, _)| *zone_id == id)
            .map(|(_, zone)| zone)
    }

    /// Move zones for an edit that replaced old lines `start..=end`, the
    /// new text ending on line `end + delta`. A zone above the edit stays,
    /// one under it moves with the lines, and one inside it stays where it
    /// was unless the lines it was under went.
    pub(super) fn adjust_for_edit(&mut self, start: usize, end: usize, delta: isize) {
        if self.zones.is_empty() {
            return;
        }
        let new_end = (end as isize + delta).max(start as isize) as usize;
        for (_, zone) in &mut self.zones {
            let line = zone.before_line;
            zone.before_line = if line <= start {
                line
            } else if line > end {
                (line as isize + delta).max(0) as usize
            } else {
                line.min(new_end + 1)
            };
        }
        // Stable, so zones on one line keep their order.
        self.zones.sort_by_key(|(_, zone)| zone.before_line);
    }

    /// Place the zones: `display_row_after(line)` is the display row under
    /// buffer line `line`, or `None` when the line is hidden.
    pub(super) fn place(
        &mut self,
        line_count: usize,
        display_row_after: impl Fn(usize) -> Option<usize>,
    ) {
        self.placed.clear();
        let (mut lines, mut pixels) = (0., px(0.));
        for (index, (_, zone)) in self.zones.iter().enumerate() {
            let display_row = if zone.before_line == 0 || line_count == 0 {
                0
            } else {
                match display_row_after(zone.before_line.min(line_count) - 1) {
                    Some(row) => row,
                    None => continue,
                }
            };
            self.placed.push(Placed {
                index,
                display_row,
                lines_before: lines,
                px_before: pixels,
            });
            lines += zone.height_in_lines;
            pixels += zone.height_in_px;
        }
        // Rows are non-decreasing in `before_line` order, since display rows
        // follow buffer lines; sort anyway so the searches below hold.
        self.placed.sort_by_key(|placed| placed.display_row);
        let (mut lines, mut pixels) = (0., px(0.));
        for placed in &mut self.placed {
            placed.lines_before = lines;
            placed.px_before = pixels;
            let zone = &self.zones[placed.index].1;
            lines += zone.height_in_lines;
            pixels += zone.height_in_px;
        }
        self.total_lines = lines;
        self.total_px = pixels;
    }

    fn height_of(lines: f32, pixels: Pixels, line_height: Pixels) -> Pixels {
        (line_height * lines + pixels).max(px(0.))
    }

    /// The height of the placed zones above display row `row` (those at
    /// `row` and before).
    pub(super) fn height_above_row(&self, row: usize, line_height: Pixels) -> Pixels {
        if self.placed.is_empty() {
            return px(0.);
        }
        let count = self.placed.partition_point(|p| p.display_row <= row);
        match self.placed.get(count) {
            Some(next) => Self::height_of(next.lines_before, next.px_before, line_height),
            None => self.total_height(line_height),
        }
    }

    /// The height of every placed zone.
    pub(super) fn total_height(&self, line_height: Pixels) -> Pixels {
        Self::height_of(self.total_lines, self.total_px, line_height)
    }

    /// The height of the placed zones at display row `row` (between rows
    /// `row - 1` and `row`).
    pub(super) fn height_at_row(&self, row: usize, line_height: Pixels) -> Pixels {
        self.placed_at(row)
            .map(|placed| self.zones[placed.index].1.height(line_height))
            .fold(px(0.), |sum, height| sum + height)
    }

    fn placed_at(&self, row: usize) -> impl Iterator<Item = &Placed> {
        let first = self.placed.partition_point(|p| p.display_row < row);
        self.placed[first..]
            .iter()
            .take_while(move |p| p.display_row == row)
    }

    /// The placed zones, each with its zone, its top in content space (from
    /// the first row's top, given each display row's top) and its height.
    pub(super) fn layouts<'a>(
        &'a self,
        line_height: Pixels,
        rows: Range<usize>,
    ) -> impl Iterator<Item = (ViewZoneId, &'a ViewZone, Pixels, Pixels)> + 'a {
        let first = self.placed.partition_point(|p| p.display_row < rows.start);
        self.placed[first..]
            .iter()
            .take_while(move |p| p.display_row <= rows.end)
            .map(move |placed| {
                let (id, zone) = &self.zones[placed.index];
                let top = line_height * placed.display_row as f32
                    + Self::height_of(placed.lines_before, placed.px_before, line_height);
                (*id, zone, top, zone.height(line_height))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten lines, one display row each, with `hidden` lines hidden.
    fn place(zones: &mut ViewZones, hidden: Range<usize>) {
        let rows: Vec<Option<usize>> = (0..10)
            .scan(0, |row, line| {
                Some(if hidden.contains(&line) {
                    None
                } else {
                    *row += 1;
                    Some(*row - 1)
                })
            })
            .collect();
        zones.place(10, |line| rows[line].map(|row| row + 1));
    }

    #[test]
    fn zones_sit_between_rows() {
        let lh = px(10.);
        let mut zones = ViewZones::default();
        zones.add(ViewZone::new(0).px(px(4.)));
        zones.add(ViewZone::after_line(2).lines(2.));
        zones.add(ViewZone::after_line(2).px(px(3.)));
        zones.add(ViewZone::new(99).lines(1.));
        place(&mut zones, 0..0);
        // Above row 0: the first zone.
        assert_eq!(zones.height_above_row(0, lh), px(4.));
        assert_eq!(zones.height_above_row(2, lh), px(4.));
        // Under line 2 (above row 3): both of its zones.
        assert_eq!(zones.height_at_row(3, lh), px(23.));
        assert_eq!(zones.height_above_row(3, lh), px(27.));
        // Under the last line.
        assert_eq!(zones.height_at_row(10, lh), px(10.));
        assert_eq!(zones.total_height(lh), px(37.));
        let tops: Vec<_> = zones
            .layouts(lh, 0..10)
            .map(|(_, _, top, height)| (top, height))
            .collect();
        assert_eq!(
            tops,
            [
                (px(0.), px(4.)),
                (px(34.), px(20.)),
                (px(54.), px(3.)),
                (px(127.), px(10.))
            ]
        );
    }

    #[test]
    fn a_zone_under_a_hidden_line_hides() {
        let lh = px(10.);
        let mut zones = ViewZones::default();
        // Over the hidden lines (the placeholder a host puts there), inside
        // them, and under them.
        zones.add(ViewZone::new(3).lines(1.));
        zones.add(ViewZone::new(5).lines(1.));
        zones.add(ViewZone::new(7).px(px(2.)));
        place(&mut zones, 3..7);
        // Lines 0-2 are rows 0-2, line 7 row 3.
        assert_eq!(zones.height_at_row(3, lh), px(10.));
        assert_eq!(zones.total_height(lh), px(10.));
    }

    #[test]
    fn edits_move_zones_with_their_lines() {
        let mut zones = ViewZones::default();
        let above = zones.add(ViewZone::new(2));
        let inside = zones.add(ViewZone::new(5));
        let under = zones.add(ViewZone::new(8));
        // Lines 3..=6 become lines 3..=4.
        zones.adjust_for_edit(3, 6, -2);
        assert_eq!(zones.get(above).unwrap().before_line, 2);
        assert_eq!(zones.get(inside).unwrap().before_line, 5);
        assert_eq!(zones.get(under).unwrap().before_line, 6);
        // A line typed above everything.
        zones.adjust_for_edit(0, 0, 1);
        assert_eq!(zones.get(above).unwrap().before_line, 3);
        // Lines 3..=6 removed into one line: the zone inside them follows.
        zones.adjust_for_edit(4, 6, -2);
        assert_eq!(zones.get(inside).unwrap().before_line, 5);
        assert!(zones.remove(inside));
        assert!(zones.get(inside).is_none());
    }
}
