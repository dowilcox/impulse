//! A list shown with the items that share a key gathered together at the
//! position of the first one (the sidebar groups the worktrees of one
//! repository this way), and moves through that shown order: a row at a
//! time, or a row or whole group dropped where it was dragged to (the Swift
//! `GroupedOrder`).
//!
//! The moves return the whole list in shown order, or `None` when there's
//! nowhere to go.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Items grouped by key: groups in the order of their first item, items in
/// their own order within a group.
pub fn groups<T: Clone, K: Hash + Eq>(items: &[T], key: impl Fn(&T) -> K) -> Vec<Vec<T>> {
    let mut slots: HashMap<K, usize> = HashMap::new();
    let mut out: Vec<Vec<T>> = Vec::new();
    for item in items {
        let slot = *slots.entry(key(item)).or_insert_with(|| {
            out.push(Vec::new());
            out.len() - 1
        });
        out[slot].push(item.clone());
    }
    out
}

/// The shown order as groups of indexes into `items`.
fn shown_indexes<T, K: Hash + Eq>(items: &[T], key: &impl Fn(&T) -> K) -> Vec<Vec<usize>> {
    let indexes: Vec<usize> = (0..items.len()).collect();
    groups(&indexes, |&i| key(&items[i]))
}

fn flatten<T: Clone>(items: &[T], shown: Vec<Vec<usize>>) -> Vec<T> {
    shown
        .into_iter()
        .flatten()
        .map(|i| items[i].clone())
        .collect()
}

/// `items` in shown order, with the one at `index` moved one row up
/// (`step` -1) or down (+1): past its neighbor within its group, or, from
/// the group's first row (up) or last row (down), the whole group past the
/// next group.
pub fn moving<T: Clone, K: Hash + Eq>(
    items: &[T],
    index: usize,
    step: isize,
    key: impl Fn(&T) -> K,
) -> Option<Vec<T>> {
    if index >= items.len() || (step != 1 && step != -1) {
        return None;
    }
    let mut shown = shown_indexes(items, &key);
    let group = shown.iter().position(|g| g.contains(&index))?;
    let position = shown[group].iter().position(|&i| i == index)?;
    let within = position.checked_add_signed(step);
    let across = group.checked_add_signed(step);
    if let Some(next) = within.filter(|&n| n < shown[group].len()) {
        shown[group].swap(position, next);
    } else {
        let next = across.filter(|&n| n < shown.len())?;
        shown.swap(group, next);
    }
    Some(flatten(items, shown))
}

/// `items` in shown order, with the one at `index` dropped at `position`
/// among its group's rows (0 first). `None` when it's there already or its
/// group has no such position.
pub fn moving_to_position<T: Clone, K: Hash + Eq>(
    items: &[T],
    index: usize,
    position: usize,
    key: impl Fn(&T) -> K,
) -> Option<Vec<T>> {
    if index >= items.len() {
        return None;
    }
    let mut shown = shown_indexes(items, &key);
    let group = shown.iter().position(|g| g.contains(&index))?;
    let from = shown[group].iter().position(|&i| i == index)?;
    if position >= shown[group].len() || position == from {
        return None;
    }
    let moved = shown[group].remove(from);
    shown[group].insert(position, moved);
    Some(flatten(items, shown))
}

/// `items` in shown order, with the group of the one at `index` dropped at
/// `position` among the groups (an item that shares its key with no other
/// is a group of its own). `None` when it's there already or there's no
/// such position.
pub fn moving_group<T: Clone, K: Hash + Eq>(
    items: &[T],
    index: usize,
    position: usize,
    key: impl Fn(&T) -> K,
) -> Option<Vec<T>> {
    if index >= items.len() {
        return None;
    }
    let mut shown = shown_indexes(items, &key);
    let from = shown.iter().position(|g| g.contains(&index))?;
    if position >= shown.len() || position == from {
        return None;
    }
    let moved = shown.remove(from);
    shown.insert(position, moved);
    Some(flatten(items, shown))
}

/// `items` with those whose id is in `order` put in that order, in the
/// positions they hold between them; the rest stay where they are. A
/// restored session uses it: Scratch is there before the restore (and
/// reused), so it would otherwise keep the first row. So does a drop in the
/// sidebar, so a workspace that opened during the drag keeps its row.
pub fn arranging<T: Clone, Id: Hash + Eq>(
    items: &[T],
    order: &[Id],
    id: impl Fn(&T) -> Id,
) -> Vec<T> {
    let wanted: HashSet<&Id> = order.iter().collect();
    let mut slots: Vec<usize> = Vec::new();
    let mut listed: HashMap<Id, &T> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        let key = id(item);
        if !wanted.contains(&key) || listed.contains_key(&key) {
            continue;
        }
        listed.insert(key, item);
        slots.push(index);
    }
    let mut placed: HashSet<&Id> = HashSet::new();
    let ordered: Vec<&T> = order
        .iter()
        .filter_map(|key| {
            if placed.insert(key) {
                listed.get(key).copied()
            } else {
                None
            }
        })
        .collect();
    let mut result = items.to_vec();
    for (slot, item) in slots.into_iter().zip(ordered) {
        result[slot] = item.clone();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names whose first letter is the group ("a1" and "a2" share one).
    fn key(name: &&str) -> char {
        name.chars().next().unwrap()
    }

    fn same<'a>(id: &&'a str) -> &'a str {
        id
    }

    #[test]
    fn groups_gather_at_the_first_member() {
        assert_eq!(
            groups(&["a1", "b", "a2", "c"], key),
            vec![vec!["a1", "a2"], vec!["b"], vec!["c"]]
        );
        assert!(groups(&Vec::<&str>::new(), key).is_empty());
    }

    #[test]
    fn ungrouped_items_swap_with_their_neighbor() {
        let items = ["a", "b", "c"];
        assert_eq!(moving(&items, 1, -1, key), Some(vec!["b", "a", "c"]));
        assert_eq!(moving(&items, 1, 1, key), Some(vec!["a", "c", "b"]));
        assert_eq!(moving(&items, 0, -1, key), None, "already first");
        assert_eq!(moving(&items, 2, 1, key), None, "already last");
    }

    #[test]
    fn members_move_within_their_group_first() {
        // Shown as a1, a2, b.
        let items = ["a1", "b", "a2"];
        assert_eq!(moving(&items, 2, -1, key), Some(vec!["a2", "a1", "b"]));
        assert_eq!(moving(&items, 0, 1, key), Some(vec!["a2", "a1", "b"]));
    }

    #[test]
    fn group_edges_move_the_whole_group() {
        // Shown as a1, a2, b: a2 is the group's last row, b a group of one.
        let items = ["a1", "b", "a2"];
        assert_eq!(moving(&items, 2, 1, key), Some(vec!["b", "a1", "a2"]));
        assert_eq!(moving(&items, 1, -1, key), Some(vec!["b", "a1", "a2"]));
        assert_eq!(moving(&items, 0, -1, key), None, "first row shown");
        assert_eq!(moving(&items, 1, 1, key), None, "last row shown");
    }

    #[test]
    fn invalid_moves_are_refused() {
        assert_eq!(moving(&["a", "b"], 5, 1, key), None);
        assert_eq!(moving(&["a", "b"], 0, 2, key), None);
    }

    #[test]
    fn dropped_members_stay_in_their_group() {
        // Shown as a1, a2, a3, b.
        let items = ["a1", "b", "a2", "a3"];
        assert_eq!(
            moving_to_position(&items, 0, 2, key),
            Some(vec!["a2", "a3", "a1", "b"])
        );
        assert_eq!(
            moving_to_position(&items, 3, 0, key),
            Some(vec!["a3", "a1", "a2", "b"])
        );
        assert_eq!(moving_to_position(&items, 2, 1, key), None, "already there");
        assert_eq!(
            moving_to_position(&items, 0, 3, key),
            None,
            "past the group's end"
        );
        assert_eq!(
            moving_to_position(&items, 1, 0, key),
            None,
            "alone in its group"
        );
    }

    #[test]
    fn dropped_groups_move_whole() {
        // Shown as a1, a2, b, c.
        let items = ["a1", "b", "a2", "c"];
        assert_eq!(
            moving_group(&items, 2, 2, key),
            Some(vec!["b", "c", "a1", "a2"])
        );
        assert_eq!(
            moving_group(&items, 3, 0, key),
            Some(vec!["c", "a1", "a2", "b"])
        );
        assert_eq!(moving_group(&items, 1, 1, key), None, "already there");
        assert_eq!(moving_group(&items, 1, 3, key), None, "only three groups");
        assert_eq!(moving_group(&items, 9, 0, key), None);
    }

    #[test]
    fn arranging_puts_listed_items_in_their_order() {
        // A restore: Scratch is first, the folders were appended after it.
        assert_eq!(
            arranging(&["scratch", "A", "B"], &["A", "scratch", "B"], same),
            vec!["A", "scratch", "B"]
        );
        assert_eq!(
            arranging(&["scratch", "A", "B"], &["A", "B", "scratch"], same),
            vec!["A", "B", "scratch"]
        );
        assert_eq!(
            arranging(&["scratch", "A"], &["scratch", "A"], same),
            vec!["scratch", "A"]
        );
    }

    #[test]
    fn arranging_leaves_unlisted_items_in_place() {
        // x isn't listed: the others fill the slots they hold around it.
        assert_eq!(
            arranging(&["a", "x", "b", "c"], &["c", "a", "b"], same),
            vec!["c", "x", "a", "b"]
        );
        // Ids that aren't there, and repeats, are skipped.
        assert_eq!(
            arranging(&["a", "b"], &["z", "b", "b", "a"], same),
            vec!["b", "a"]
        );
        assert_eq!(arranging(&["a", "b"], &[], same), vec!["a", "b"]);
    }
}
