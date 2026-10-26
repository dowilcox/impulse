//! Impulse: view zones and hidden lines on the editor's state, as Monaco's
//! `changeViewZones` and `setHiddenAreas` give them. The geometry lives in
//! the display map (`display_map/view_zones.rs`); this is the host's API.

use std::ops::Range;

use gpui::{Bounds, Context, Pixels, Point};

use super::{EditorMode, InputBaseState, InputModeKind, ViewZone, ViewZoneId};

impl<M: InputModeKind> InputBaseState<M> {
    /// Impulse: the view zone under `position` (window coordinates), from
    /// the last layout.
    pub(crate) fn view_zone_at(&self, position: Point<Pixels>) -> Option<ViewZoneId> {
        let layout = self.last_layout.as_ref()?;
        layout
            .view_zones
            .iter()
            .find(|(_, bounds)| bounds.contains(&position))
            .map(|(id, _)| *id)
    }
}

impl InputBaseState<EditorMode> {
    /// Impulse: add a view zone, a blank band between two lines that pushes
    /// the lines under it down (Monaco's `changeViewZones` `addZone`).
    pub fn add_view_zone(&mut self, zone: ViewZone, cx: &mut Context<Self>) -> ViewZoneId {
        let id = self.display_map.add_view_zone(zone);
        cx.notify();
        id
    }

    /// Impulse: remove a view zone. Whether it was there.
    pub fn remove_view_zone(&mut self, id: ViewZoneId, cx: &mut Context<Self>) -> bool {
        let removed = self.display_map.remove_view_zone(id);
        if removed {
            cx.notify();
        }
        removed
    }

    /// Impulse: remove these view zones and add `zones` in their place, in
    /// one change; the new zones' ids, in order.
    pub fn replace_view_zones(
        &mut self,
        remove: &[ViewZoneId],
        zones: Vec<ViewZone>,
        cx: &mut Context<Self>,
    ) -> Vec<ViewZoneId> {
        for id in remove {
            self.display_map.remove_view_zone(*id);
        }
        let ids = zones
            .into_iter()
            .map(|zone| self.display_map.add_view_zone(zone))
            .collect();
        cx.notify();
        ids
    }

    /// Impulse: where a view zone was on screen at the last layout (window
    /// coordinates, from the gutter's left edge); `None` when it was off
    /// screen or hidden.
    pub fn view_zone_bounds(&self, id: ViewZoneId) -> Option<Bounds<Pixels>> {
        self.last_layout
            .as_ref()?
            .view_zones
            .iter()
            .find(|(zone, _)| *zone == id)
            .map(|(_, bounds)| *bounds)
    }

    /// Impulse: hide these buffer lines (end exclusive) in place of those
    /// hidden before (Monaco's `setHiddenAreas`). Unlike folds they last
    /// through reparses; they follow edits, and an edit inside one shows its
    /// lines again. The caret steps over them. Put a view zone above a range
    /// (`ViewZone::new(range.start)`) to stand for it.
    pub fn set_hidden_ranges(&mut self, ranges: Vec<Range<usize>>, cx: &mut Context<Self>) {
        if ranges == self.display_map.hidden_ranges() {
            return;
        }
        self.display_map.set_hidden_ranges(ranges);
        cx.notify();
    }

    /// Impulse: the buffer lines hidden by `set_hidden_ranges`, as they now
    /// are.
    pub fn hidden_ranges(&self) -> &[Range<usize>] {
        self.display_map.hidden_ranges()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
        Styled as _, TestAppContext, VisualTestContext, Window, div, px,
    };

    use crate::input::{EditorState, FoldRange, MoveDown, Position, ViewZone};

    struct Root(Entity<EditorState>);

    impl Render for Root {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    /// An editor holding `text`, in a window, laid out once.
    fn editor(text: &str, cx: &mut TestAppContext) -> (Entity<EditorState>, VisualTestContext) {
        let text = text.to_string();
        let mut state = None;
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                cx.set_global(crate::theme::Theme::default());
                crate::input::init(cx);
                let editor = cx.new(|cx| EditorState::new(window, cx).default_value(text));
                state = Some(editor.clone());
                cx.new(|_| Root(editor))
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (state.unwrap(), cx)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    #[gpui::test]
    fn hidden_ranges_last_through_reparses_and_follow_edits(cx: &mut TestAppContext) {
        let (state, mut cx) = editor("0\n1\n2\n3\n4\n5\n6\n7\n8\n9", cx);
        cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.set_hidden_ranges(vec![6..8, 2..4, 3..5], cx);
                // Merged and sorted.
                assert_eq!(state.hidden_ranges(), [2..5, 6..8]);
                assert_eq!(state.display_map.display_row_count(), 5);
                assert_eq!(state.display_map.hidden_lines_at(3), Some(2..5));
                assert_eq!(state.display_map.hidden_lines_at(5), None);
                // The highlighter's folds come and go; the ranges stay.
                state.apply_highlighter_fold_candidates(vec![FoldRange::new(0, 9)], cx);
                state.apply_highlighter_fold_candidates(Vec::new(), cx);
                assert_eq!(state.hidden_ranges(), [2..5, 6..8]);
                assert_eq!(state.display_map.display_row_count(), 5);
            });
        });
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_cursor_position(Position::new(1, 0), window, cx);
            });
        });
        draw(&mut cx);
        // The caret steps over them.
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.down(&MoveDown, window, cx);
                assert_eq!(state.cursor_position().line, 5);
                state.down(&MoveDown, window, cx);
                assert_eq!(state.cursor_position().line, 8);
            });
        });
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                // A line typed above them moves them down.
                state.set_cursor_position(Position::new(0, 1), window, cx);
                state.replace_text_in_range_silent(None, "\nnew", window, cx);
                assert_eq!(state.hidden_ranges(), [3..6, 7..9]);
                // An edit inside one shows it.
                state.set_cursor_position(Position::new(4, 0), window, cx);
                state.replace_text_in_range_silent(None, "x", window, cx);
                assert_eq!(state.hidden_ranges(), [7..9]);
            });
        });
    }

    #[gpui::test]
    fn view_zones_push_lines_down(cx: &mut TestAppContext) {
        let (state, mut cx) = editor("a\nb\nc\nd", cx);
        let lh = px(20.);
        let (top, middle) = cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                let top = state.add_view_zone(ViewZone::new(0).px(px(5.)), cx);
                let middle = state.add_view_zone(ViewZone::after_line(1).lines(2.), cx);
                let map = &state.display_map;
                assert_eq!(map.display_row_top(0, lh), px(5.));
                assert_eq!(map.display_row_top(2, lh), px(85.));
                assert_eq!(map.content_height(lh), px(125.));
                // A point in a zone is the row above it.
                assert_eq!(map.display_row_at_y(px(50.), lh), 1);
                assert_eq!(map.display_row_at_y(px(2.), lh), 0);
                assert_eq!(map.display_row_at_y(px(90.), lh), 2);
                // Hiding the line a zone is under hides the zone.
                state.set_hidden_ranges(vec![1..2], cx);
                assert_eq!(state.display_map.content_height(lh), px(65.));
                state.set_hidden_ranges(Vec::new(), cx);
                (top, middle)
            })
        });
        draw(&mut cx);
        cx.update(|_, cx| {
            state.update(cx, |state, _| {
                let line_height = state.line_height().unwrap();
                // Line 2 sits under both zones, and its bounds say so.
                let line = state.range_to_bounds(&(4..4)).unwrap();
                let first = state.range_to_bounds(&(0..0)).unwrap();
                assert_eq!(line.top() - first.top(), line_height * 4.);
                let zone = state.view_zone_bounds(middle).unwrap();
                assert_eq!(zone.size.height, line_height * 2.);
                assert_eq!(zone.top(), first.top() + line_height * 2.);
                // A press in the zone is the zone's.
                assert_eq!(state.view_zone_at(zone.center()), Some(middle));
            });
        });
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                // Lines typed above a zone move it.
                state.set_cursor_position(Position::new(0, 0), window, cx);
                state.replace_text_in_range_silent(None, "z\n", window, cx);
                assert_eq!(state.display_map.view_zone(middle).unwrap().before_line, 3);
                assert_eq!(state.display_map.view_zone(top).unwrap().before_line, 0);
                assert!(state.remove_view_zone(middle, cx));
                assert_eq!(state.display_map.content_height(lh), px(105.));
            });
        });
    }
}
