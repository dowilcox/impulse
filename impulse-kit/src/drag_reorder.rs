//! Reordering a list by dragging one of its rows (the Swift `DragReorder`):
//! where the row lands, and which rows move aside to make room for it
//! meanwhile.

use std::ops::RangeInclusive;

/// The position the row dragged from `source` lands at, from every row's
/// middle along the drag (in list order, the dragged row's where it
/// started) and the span the dragged row covers now: past each row whose
/// middle its leading edge has crossed (its start dragged back, its end
/// dragged forward). A row held against either end of the list lands first
/// or last.
pub fn target(middles: &[f64], source: usize, dragged_span: RangeInclusive<f64>) -> usize {
    if source >= middles.len() {
        return source;
    }
    let after = (source + 1..middles.len())
        .filter(|&i| middles[i] < *dragged_span.end())
        .max();
    let before = (0..source)
        .filter(|&i| middles[i] > *dragged_span.start())
        .min();
    after.or(before).unwrap_or(source)
}

/// Which way the row at `index` moves aside while the one dragged from
/// `source` is over `target`: -1 back (up or left), 1 forward, 0 not at
/// all.
pub fn shift(index: usize, source: usize, target: usize) -> i32 {
    if source < index && index <= target {
        -1
    } else if target <= index && index < source {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows 20 points tall with no gap: row i spans 20i...20i+20.
    const MIDDLES: [f64; 4] = [10.0, 30.0, 50.0, 70.0];

    /// Row `index` dragged `distance` points (negative is up).
    fn span(index: usize, distance: f64) -> RangeInclusive<f64> {
        let top = 20.0 * index as f64;
        (top + distance)..=(top + 20.0 + distance)
    }

    #[test]
    fn lands_past_every_middle_its_leading_edge_crossed() {
        assert_eq!(target(&MIDDLES, 0, span(0, 9.0)), 0);
        assert_eq!(target(&MIDDLES, 0, span(0, 11.0)), 1);
        assert_eq!(target(&MIDDLES, 0, span(0, 200.0)), 3);
        assert_eq!(target(&MIDDLES, 3, span(3, -9.0)), 3);
        assert_eq!(target(&MIDDLES, 3, span(3, -11.0)), 2);
        assert_eq!(target(&MIDDLES, 3, span(3, -200.0)), 0);
        assert_eq!(target(&MIDDLES, 1, span(1, 0.0)), 1, "not moved");
    }

    #[test]
    fn held_against_the_ends_lands_first_or_last() {
        // The sidebar stops a dragged row at the list's ends: its edge meets
        // the first row's top or the last row's bottom, its middle the other
        // row's middle (equal heights) or short of it.
        assert_eq!(target(&MIDDLES, 2, span(2, -40.0)), 0);
        assert_eq!(target(&MIDDLES, 1, span(1, 40.0)), 3);
        // A tall row (40...100) under two short ones.
        let mixed = [10.0, 30.0, 70.0];
        assert_eq!(target(&mixed, 2, 0.0..=60.0), 0);
        assert_eq!(target(&mixed, 0, 80.0..=100.0), 2);
    }

    #[test]
    fn rows_of_different_heights() {
        // A tall row (an expanded workspace, 20...120) between two short ones.
        let middles = [10.0, 70.0, 130.0];
        assert_eq!(target(&middles, 0, 49.0..=69.0), 0);
        assert_eq!(target(&middles, 0, 51.0..=71.0), 1);
        assert_eq!(target(&middles, 2, 71.0..=91.0), 2);
        assert_eq!(target(&middles, 2, 69.0..=89.0), 1);
        // The tall row dragged down past the short one under it.
        assert_eq!(target(&middles, 1, 29.0..=129.0), 1);
        assert_eq!(target(&middles, 1, 31.0..=131.0), 2);
    }

    #[test]
    fn unknown_source_stays_put() {
        assert_eq!(target(&MIDDLES, 7, 0.0..=20.0), 7);
        assert_eq!(target(&[], 0, 0.0..=20.0), 0);
    }

    #[test]
    fn rows_between_move_aside() {
        let shifts = |source, target| (0..4).map(|i| shift(i, source, target)).collect::<Vec<_>>();
        // Dragged down from 0 to 2: rows 1 and 2 move up.
        assert_eq!(shifts(0, 2), vec![0, -1, -1, 0]);
        // Dragged up from 3 to 1: rows 1 and 2 move down.
        assert_eq!(shifts(3, 1), vec![0, 1, 1, 0]);
        assert_eq!(shifts(2, 2), vec![0, 0, 0, 0]);
    }
}
