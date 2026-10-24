//! The panes of a tab as a pure value tree (the Swift `LayoutTree`): leaves
//! are pane ids, splits lay their children out along one axis with ratios
//! that sum to 1. Every operation returns a new tree; the UI renders frames
//! from it.
//!
//! Saved layouts are JSON: a leaf is `{"pane": id}`, a split
//! `{"axis": "horizontal"|"vertical", "children": [...], "ratios": [...]}`.
//! Reading one repairs what it can: ratios that don't match the children
//! become equal shares, the rest are renormalized, a one-child split is its
//! child, and same-axis nesting is flattened. An empty split is an error.

use std::collections::HashMap;
use std::hash::Hash;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{Serialize, SerializeMap, Serializer};

/// How a split arranges its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitAxis {
    /// Side by side, with vertical dividers between children.
    Horizontal,
    /// Stacked top to bottom, with horizontal dividers between children.
    Vertical,
}

/// A direction on screen, for focus movement and resizing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

impl PaneDirection {
    pub const ALL: [PaneDirection; 4] = [Self::Left, Self::Right, Self::Up, Self::Down];

    /// The split axis a move in this direction crosses.
    pub fn axis(self) -> SplitAxis {
        match self {
            Self::Left | Self::Right => SplitAxis::Horizontal,
            Self::Up | Self::Down => SplitAxis::Vertical,
        }
    }

    /// Whether the direction points toward later children (right or down).
    pub fn is_forward(self) -> bool {
        matches!(self, Self::Right | Self::Down)
    }
}

/// A rectangle in a top-left-origin coordinate space (y grows downward), so
/// frames read in the same order as the tree's children.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LayoutRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl LayoutRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn max_x(&self) -> f64 {
        self.x + self.width
    }
    pub fn max_y(&self) -> f64 {
        self.y + self.height
    }
    pub fn mid_x(&self) -> f64 {
        self.x + self.width / 2.0
    }
    pub fn mid_y(&self) -> f64 {
        self.y + self.height / 2.0
    }
}

/// The boundary between two adjacent children of a split.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutDivider {
    /// Child indexes from the root to the split that owns the divider.
    pub path: Vec<usize>,
    /// The divider sits after child `index` (between `index` and `index + 1`).
    pub index: usize,
    /// The owning split's axis: `Horizontal` dividers are vertical lines.
    pub axis: SplitAxis,
    /// The gap the divider occupies (zero thickness when the gap is zero).
    pub rect: LayoutRect,
    /// The owning split's frame, for converting a drag into a ratio.
    pub container: LayoutRect,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutTree<Id> {
    Leaf(Id),
    Split {
        axis: SplitAxis,
        children: Vec<LayoutTree<Id>>,
        ratios: Vec<f64>,
    },
}

/// Smallest share a child may be resized down to.
pub const MINIMUM_RATIO: f64 = 0.05;

impl<Id: Clone + PartialEq> LayoutTree<Id> {
    // Queries

    /// Pane ids in reading order (left to right, top to bottom).
    pub fn leaves(&self) -> Vec<Id> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<Id>) {
        match self {
            Self::Leaf(id) => out.push(id.clone()),
            Self::Split { children, .. } => children.iter().for_each(|c| c.collect_leaves(out)),
        }
    }

    pub fn pane_count(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split { children, .. } => children.iter().map(Self::pane_count).sum(),
        }
    }

    pub fn contains(&self, id: &Id) -> bool {
        match self {
            Self::Leaf(leaf) => leaf == id,
            Self::Split { children, .. } => children.iter().any(|c| c.contains(id)),
        }
    }

    /// The subtree at a child-index path (an empty path is the root).
    pub fn node(&self, path: &[usize]) -> Option<&LayoutTree<Id>> {
        let Some((&first, rest)) = path.split_first() else {
            return Some(self);
        };
        match self {
            Self::Split { children, .. } => children.get(first)?.node(rest),
            Self::Leaf(_) => None,
        }
    }

    /// Child-index path to a pane, or `None` when it isn't in the tree.
    pub fn path_to(&self, id: &Id) -> Option<Vec<usize>> {
        match self {
            Self::Leaf(leaf) => (leaf == id).then(Vec::new),
            Self::Split { children, .. } => children.iter().enumerate().find_map(|(i, child)| {
                child.path_to(id).map(|rest| {
                    let mut path = vec![i];
                    path.extend(rest);
                    path
                })
            }),
        }
    }

    // Structure edits

    /// Split `target`, placing `new_id` after it (or before it). Splitting
    /// in the same direction as the enclosing split adds a sibling there
    /// instead of nesting, halving the target's share.
    pub fn splitting(&self, target: &Id, new_id: Id, axis: SplitAxis, before: bool) -> Self {
        match self {
            Self::Leaf(id) => {
                if id != target {
                    return self.clone();
                }
                let children = if before {
                    vec![Self::Leaf(new_id), self.clone()]
                } else {
                    vec![self.clone(), Self::Leaf(new_id)]
                };
                Self::Split {
                    axis,
                    children,
                    ratios: vec![0.5, 0.5],
                }
            }
            Self::Split {
                axis: split_axis,
                children,
                ratios,
            } => {
                let Some(index) = children.iter().position(|c| c.contains(target)) else {
                    return self.clone();
                };
                let mut children = children.clone();
                let mut ratios = ratios.clone();
                if *split_axis == axis && matches!(&children[index], Self::Leaf(id) if id == target)
                {
                    let half = ratios[index] / 2.0;
                    ratios[index] = half;
                    let insert_at = if before { index } else { index + 1 };
                    children.insert(insert_at, Self::Leaf(new_id));
                    ratios.insert(insert_at, half);
                } else {
                    children[index] = children[index].splitting(target, new_id, axis, before);
                }
                Self::Split {
                    axis: *split_axis,
                    children,
                    ratios,
                }
            }
        }
    }

    /// Remove a pane, giving its space to the adjacent sibling. `None` when
    /// the pane was the whole tree.
    pub fn removing(&self, id: &Id) -> Option<Self> {
        match self {
            Self::Leaf(leaf) => (leaf != id).then(|| self.clone()),
            Self::Split {
                axis,
                children,
                ratios,
            } => {
                let Some(index) = children.iter().position(|c| c.contains(id)) else {
                    return Some(self.clone());
                };
                let mut children = children.clone();
                let mut ratios = ratios.clone();
                if let Some(remaining) = children[index].removing(id) {
                    children[index] = remaining;
                } else {
                    let freed = ratios.remove(index);
                    children.remove(index);
                    let heir = index.saturating_sub(1);
                    if let Some(ratio) = ratios.get_mut(heir) {
                        *ratio += freed;
                    }
                }
                if children.len() == 1 {
                    return children.pop();
                }
                Some(
                    Self::Split {
                        axis: *axis,
                        children,
                        ratios,
                    }
                    .flattened(),
                )
            }
        }
    }

    /// Swap the positions of two panes.
    pub fn swapping(&self, a: &Id, b: &Id) -> Self {
        self.map_panes(&mut |id: &Id| {
            if id == a {
                b.clone()
            } else if id == b {
                a.clone()
            } else {
                id.clone()
            }
        })
    }

    /// Replace a pane id, keeping its position and size.
    pub fn replacing(&self, id: &Id, new_id: Id) -> Self {
        self.map_panes(&mut |leaf: &Id| {
            if leaf == id {
                new_id.clone()
            } else {
                leaf.clone()
            }
        })
    }

    /// The same arrangement with every pane id transformed (e.g. renumbered
    /// for saving).
    pub fn map_panes<New>(&self, transform: &mut impl FnMut(&Id) -> New) -> LayoutTree<New> {
        match self {
            Self::Leaf(id) => LayoutTree::Leaf(transform(id)),
            Self::Split {
                axis,
                children,
                ratios,
            } => LayoutTree::Split {
                axis: *axis,
                children: children.iter().map(|c| c.map_panes(transform)).collect(),
                ratios: ratios.clone(),
            },
        }
    }

    /// Move a pane next to another one, on the given side of it.
    pub fn moving(&self, id: &Id, next_to: &Id, side: PaneDirection) -> Self {
        if id == next_to || !self.contains(id) || !self.contains(next_to) {
            return self.clone();
        }
        let Some(without) = self.removing(id) else {
            return self.clone();
        };
        without.splitting(next_to, id.clone(), side.axis(), !side.is_forward())
    }

    /// Every split's children share its space equally.
    pub fn equalized(&self) -> Self {
        match self {
            Self::Leaf(_) => self.clone(),
            Self::Split { axis, children, .. } => {
                let share = 1.0 / children.len() as f64;
                Self::Split {
                    axis: *axis,
                    children: children.iter().map(Self::equalized).collect(),
                    ratios: vec![share; children.len()],
                }
            }
        }
    }

    // Sizing

    /// Move divider `index` of the split at `path` so it sits at `position`
    /// (0...1, a fraction of that split's length). Neighbors keep at least
    /// `minimum` (default `MINIMUM_RATIO`) of the split each.
    pub fn moving_divider(
        &self,
        path: &[usize],
        index: usize,
        position: f64,
        minimum: Option<f64>,
    ) -> Self {
        let Self::Split {
            axis,
            children,
            ratios,
        } = self
        else {
            return self.clone();
        };
        if let Some((&first, rest)) = path.split_first() {
            if first >= children.len() {
                return self.clone();
            }
            let mut children = children.clone();
            children[first] = children[first].moving_divider(rest, index, position, minimum);
            return Self::Split {
                axis: *axis,
                children,
                ratios: ratios.clone(),
            };
        }
        if index + 1 >= ratios.len() {
            return self.clone();
        }
        let floor = minimum.unwrap_or(MINIMUM_RATIO);
        let start: f64 = ratios[..index].iter().sum();
        let end = start + ratios[index] + ratios[index + 1];
        let clamped = position.max(start + floor).min(end - floor);
        if clamped < start || clamped > end {
            return self.clone();
        }
        let mut ratios = ratios.clone();
        ratios[index] = clamped - start;
        ratios[index + 1] = end - clamped;
        Self::Split {
            axis: *axis,
            children: children.clone(),
            ratios,
        }
    }

    /// Grow (positive `delta`) or shrink a pane by moving its edge on the
    /// `direction` side. `delta` is a fraction of the enclosing split. A
    /// pane whose edge is the window's edge on that side doesn't change.
    pub fn resizing(&self, id: &Id, direction: PaneDirection, delta: f64) -> Self {
        let Some(path) = self.path_to(id) else {
            return self.clone();
        };
        // Walk up from the pane to the nearest split along the right axis
        // that has a neighbor on that side.
        for depth in (0..path.len()).rev() {
            let split_path = &path[..depth];
            let child_index = path[depth];
            let Some(Self::Split { axis, ratios, .. }) = self.node(split_path) else {
                continue;
            };
            if *axis != direction.axis() {
                continue;
            }
            let (neighbor, divider_index) = if direction.is_forward() {
                (Some(child_index + 1), child_index)
            } else {
                (child_index.checked_sub(1), child_index.wrapping_sub(1))
            };
            if !neighbor.is_some_and(|n| n < ratios.len()) {
                continue;
            }
            let boundary: f64 = ratios[..=divider_index].iter().sum();
            let moved = if direction.is_forward() {
                boundary + delta
            } else {
                boundary - delta
            };
            return self.moving_divider(split_path, divider_index, moved, None);
        }
        self.clone()
    }

    // Internals

    /// Merge child splits that run along the same axis as their parent, and
    /// renormalize ratios.
    fn flattened(self) -> Self {
        let Self::Split {
            axis,
            children,
            ratios,
        } = self
        else {
            return self;
        };
        let mut merged_children = Vec::new();
        let mut merged_ratios = Vec::new();
        for (child, ratio) in children.into_iter().zip(ratios) {
            match child {
                Self::Split {
                    axis: child_axis,
                    children: grandchildren,
                    ratios: child_ratios,
                } if child_axis == axis => {
                    merged_children.extend(grandchildren);
                    merged_ratios.extend(child_ratios.into_iter().map(|r| r * ratio));
                }
                other => {
                    merged_children.push(other);
                    merged_ratios.push(ratio);
                }
            }
        }
        Self::Split {
            axis,
            children: merged_children,
            ratios: normalized(&merged_ratios),
        }
    }

    fn walk(
        &self,
        rect: LayoutRect,
        gap: f64,
        path: &mut Vec<usize>,
        visit: &mut impl FnMut(&Self, LayoutRect, &[usize]),
    ) {
        visit(self, rect, path);
        let Self::Split { children, .. } = self else {
            return;
        };
        let frames = self.child_frames(rect, gap);
        for (index, child) in children.iter().enumerate() {
            path.push(index);
            child.walk(frames[index], gap, path, visit);
            path.pop();
        }
    }

    fn child_frames(&self, rect: LayoutRect, gap: f64) -> Vec<LayoutRect> {
        let Self::Split {
            axis,
            children,
            ratios,
        } = self
        else {
            return vec![rect];
        };
        let horizontal = *axis == SplitAxis::Horizontal;
        let length = if horizontal { rect.width } else { rect.height };
        let origin = if horizontal { rect.x } else { rect.y };
        let gaps = gap * (children.len() as f64 - 1.0);
        let available = (length - gaps).max(0.0);
        let mut frames = Vec::with_capacity(children.len());
        let mut cumulative = 0.0;
        for (index, ratio) in ratios.iter().enumerate().take(children.len()) {
            let start_offset = index as f64 * gap;
            let start = (origin + cumulative * available + start_offset).round();
            cumulative += ratio;
            let end = if index == children.len() - 1 {
                origin + length
            } else {
                (origin + cumulative * available + start_offset).round()
            };
            let size = (end - start).max(0.0);
            frames.push(if horizontal {
                LayoutRect::new(start, rect.y, size, rect.height)
            } else {
                LayoutRect::new(rect.x, start, rect.width, size)
            });
        }
        frames
    }

    /// Each pane's frame, in reading order.
    pub fn frame_list(&self, rect: LayoutRect, gap: f64) -> Vec<(Id, LayoutRect)> {
        let mut result = Vec::new();
        self.walk(rect, gap, &mut Vec::new(), &mut |node, frame, _| {
            if let Self::Leaf(id) = node {
                result.push((id.clone(), frame));
            }
        });
        result
    }

    /// Every divider inside `rect`, for hit testing and drawing.
    pub fn dividers(&self, rect: LayoutRect, gap: f64) -> Vec<LayoutDivider> {
        let mut result = Vec::new();
        self.walk(rect, gap, &mut Vec::new(), &mut |node, frame, path| {
            let Self::Split { axis, children, .. } = node else {
                return;
            };
            let frames = node.child_frames(frame, gap);
            for index in 0..children.len() - 1 {
                let a = frames[index];
                let b = frames[index + 1];
                let divider = match axis {
                    SplitAxis::Horizontal => {
                        LayoutRect::new(a.max_x(), frame.y, b.x - a.max_x(), frame.height)
                    }
                    SplitAxis::Vertical => {
                        LayoutRect::new(frame.x, a.max_y(), frame.width, b.y - a.max_y())
                    }
                };
                result.push(LayoutDivider {
                    path: path.to_vec(),
                    index,
                    axis: *axis,
                    rect: divider,
                    container: frame,
                });
            }
        });
        result
    }

    /// The pane next to `id` in a direction: the closest pane on that side
    /// that overlaps it, preferring the one level with its center (then the
    /// one that starts first).
    pub fn neighbor(&self, id: &Id, direction: PaneDirection) -> Option<Id> {
        let all = self.frame_list(LayoutRect::new(0.0, 0.0, 10_000.0, 10_000.0), 0.0);
        let current = all.iter().find(|(leaf, _)| leaf == id)?.1;
        let epsilon = 0.5;

        struct Candidate<Id> {
            id: Id,
            distance: f64,
            covers_center: bool,
            start: f64,
        }
        let mut best: Option<Candidate<Id>> = None;
        for (other, frame) in &all {
            if other == id {
                continue;
            }
            let across_y = (
                current.y.max(frame.y),
                current.max_y().min(frame.max_y()),
                current.mid_y(),
                frame.y,
            );
            let across_x = (
                current.x.max(frame.x),
                current.max_x().min(frame.max_x()),
                current.mid_x(),
                frame.x,
            );
            let (distance, (overlap_start, overlap_end, center, start)) = match direction {
                PaneDirection::Left => (current.x - frame.max_x(), across_y),
                PaneDirection::Right => (frame.x - current.max_x(), across_y),
                PaneDirection::Up => (current.y - frame.max_y(), across_x),
                PaneDirection::Down => (frame.y - current.max_y(), across_x),
            };
            if distance <= -epsilon || overlap_end - overlap_start <= epsilon {
                continue;
            }
            let candidate = Candidate {
                id: other.clone(),
                distance,
                covers_center: overlap_start <= center && center <= overlap_end,
                start,
            };
            let better = match &best {
                None => true,
                Some(b) => {
                    if (candidate.distance - b.distance).abs() > epsilon {
                        candidate.distance < b.distance
                    } else if candidate.covers_center != b.covers_center {
                        candidate.covers_center
                    } else {
                        candidate.start < b.start
                    }
                }
            };
            if better {
                best = Some(candidate);
            }
        }
        best.map(|c| c.id)
    }
}

impl<Id: Clone + Eq + Hash> LayoutTree<Id> {
    /// Each pane's frame inside `rect`, with `gap` points between siblings.
    /// Edges are rounded to whole points so neighbors never overlap or leave
    /// a seam.
    pub fn frames(&self, rect: LayoutRect, gap: f64) -> HashMap<Id, LayoutRect> {
        self.frame_list(rect, gap).into_iter().collect()
    }
}

fn normalized(ratios: &[f64]) -> Vec<f64> {
    let positive: Vec<f64> = ratios
        .iter()
        .map(|&r| if r.is_finite() && r > 0.0 { r } else { 0.0 })
        .collect();
    let total: f64 = positive.iter().sum();
    if total <= 0.0 {
        let share = 1.0 / ratios.len().max(1) as f64;
        return vec![share; ratios.len()];
    }
    positive.into_iter().map(|r| r / total).collect()
}

impl<Id: Serialize> Serialize for LayoutTree<Id> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Leaf(id) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("pane", id)?;
                map.end()
            }
            Self::Split {
                axis,
                children,
                ratios,
            } => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("axis", axis)?;
                map.serialize_entry("children", children)?;
                map.serialize_entry("ratios", ratios)?;
                map.end()
            }
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(bound(deserialize = "Id: Deserialize<'de> + Clone + PartialEq"))]
struct RawTree<Id> {
    pane: Option<Id>,
    axis: Option<SplitAxis>,
    children: Option<Vec<LayoutTree<Id>>>,
    ratios: Option<Vec<f64>>,
}

impl<'de, Id: Deserialize<'de> + Clone + PartialEq> Deserialize<'de> for LayoutTree<Id> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawTree::<Id>::deserialize(deserializer)?;
        if let Some(id) = raw.pane {
            return Ok(Self::Leaf(id));
        }
        let axis = raw.axis.ok_or_else(|| de::Error::missing_field("axis"))?;
        let mut children = raw
            .children
            .ok_or_else(|| de::Error::missing_field("children"))?;
        if children.is_empty() {
            return Err(de::Error::custom("A split needs at least one child"));
        }
        if children.len() == 1 {
            return Ok(children.remove(0));
        }
        let mut ratios = raw.ratios.unwrap_or_default();
        if ratios.len() != children.len() {
            ratios = vec![1.0; children.len()];
        }
        Ok(Self::Split {
            axis,
            children,
            ratios: normalized(&ratios),
        }
        .flattened())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SplitAxis::{Horizontal, Vertical};

    type Tree = LayoutTree<i32>;

    fn ratios(tree: &Tree) -> Vec<f64> {
        match tree {
            Tree::Split { ratios, .. } => ratios.clone(),
            Tree::Leaf(_) => vec![],
        }
    }

    fn approx(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    fn split(axis: SplitAxis, children: Vec<Tree>, ratios: Vec<f64>) -> Tree {
        Tree::Split {
            axis,
            children,
            ratios,
        }
    }

    /// ┌───┬───┐
    /// │ 1 │ 2 │
    /// ├───┼───┤
    /// │ 3 │ 4 │
    /// └───┴───┘
    fn grid() -> Tree {
        Tree::Leaf(1)
            .splitting(&1, 2, Horizontal, false)
            .splitting(&1, 3, Vertical, false)
            .splitting(&2, 4, Vertical, false)
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> LayoutRect {
        LayoutRect::new(x, y, width, height)
    }

    #[test]
    fn splitting_a_leaf_makes_an_even_pair() {
        let tree = Tree::Leaf(1).splitting(&1, 2, Horizontal, false);
        assert_eq!(
            tree,
            split(
                Horizontal,
                vec![Tree::Leaf(1), Tree::Leaf(2)],
                vec![0.5, 0.5]
            )
        );
        let before = Tree::Leaf(1).splitting(&1, 2, Vertical, true);
        assert_eq!(before.leaves(), vec![2, 1]);
    }

    #[test]
    fn splitting_along_the_same_axis_adds_a_sibling() {
        let tree = Tree::Leaf(1)
            .splitting(&1, 2, Horizontal, false)
            .splitting(&2, 3, Horizontal, false);
        let Tree::Split {
            axis: Horizontal,
            children,
            ratios,
        } = &tree
        else {
            panic!("expected a horizontal split");
        };
        assert_eq!(children, &vec![Tree::Leaf(1), Tree::Leaf(2), Tree::Leaf(3)]);
        assert!(approx(ratios, &[0.5, 0.25, 0.25]));
    }

    #[test]
    fn splitting_across_the_axis_nests() {
        let grid = grid();
        assert_eq!(grid.leaves(), vec![1, 3, 2, 4]);
        assert_eq!(grid.pane_count(), 4);
        assert_eq!(grid.path_to(&4), Some(vec![1, 1]));
        assert_eq!(
            grid.node(&[0]),
            Some(&split(
                Vertical,
                vec![Tree::Leaf(1), Tree::Leaf(3)],
                vec![0.5, 0.5]
            ))
        );
        assert!(grid.contains(&3));
        assert!(!grid.contains(&9));
        // Unknown targets leave the tree alone.
        assert_eq!(grid.splitting(&9, 10, Horizontal, false), grid);
    }

    #[test]
    fn removing_gives_space_to_the_adjacent_sibling() {
        let tree = Tree::Leaf(1)
            .splitting(&1, 2, Horizontal, false)
            .splitting(&2, 3, Horizontal, false); // [0.5, 0.25, 0.25]
        let without_middle = tree.removing(&2).unwrap();
        assert_eq!(without_middle.leaves(), vec![1, 3]);
        assert!(approx(&ratios(&without_middle), &[0.75, 0.25]));
        let without_first = tree.removing(&1).unwrap();
        assert!(approx(&ratios(&without_first), &[0.75, 0.25]));
    }

    #[test]
    fn removing_collapses_single_child_splits() {
        let pair = Tree::Leaf(1).splitting(&1, 2, Horizontal, false);
        assert_eq!(pair.removing(&2), Some(Tree::Leaf(1)));
        assert_eq!(Tree::Leaf(1).removing(&1), None);
        assert_eq!(Tree::Leaf(1).removing(&2), Some(Tree::Leaf(1)));

        let after_remove = grid().removing(&3).unwrap();
        assert_eq!(after_remove.leaves(), vec![1, 2, 4]);
        assert_eq!(after_remove.node(&[0]), Some(&Tree::Leaf(1)));
    }

    #[test]
    fn removing_flattens_same_axis_nesting() {
        // H[1, V[2, H[3, 4]]] -> remove 2 -> H[1, 3, 4]
        let tree = Tree::Leaf(1)
            .splitting(&1, 2, Horizontal, false)
            .splitting(&2, 3, Vertical, false)
            .splitting(&3, 4, Horizontal, false);
        let result = tree.removing(&2).unwrap();
        let Tree::Split {
            axis: Horizontal,
            children,
            ratios,
        } = &result
        else {
            panic!("expected a flat horizontal split");
        };
        assert_eq!(children, &vec![Tree::Leaf(1), Tree::Leaf(3), Tree::Leaf(4)]);
        assert!(approx(ratios, &[0.5, 0.25, 0.25]));
    }

    #[test]
    fn swap_replace_and_equalize() {
        let grid = grid();
        assert_eq!(grid.swapping(&1, &4).leaves(), vec![4, 3, 2, 1]);
        assert_eq!(grid.replacing(&3, 7).leaves(), vec![1, 7, 2, 4]);
        let lopsided = Tree::Leaf(1)
            .splitting(&1, 2, Horizontal, false)
            .splitting(&2, 3, Horizontal, false);
        assert!(approx(
            &ratios(&lopsided.equalized()),
            &[1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0]
        ));
    }

    #[test]
    fn mapping_pane_ids() {
        let grid = grid();
        let renumbered = grid.map_panes(&mut |id| id * 10);
        assert_eq!(renumbered.leaves(), vec![10, 30, 20, 40]);
        let named: LayoutTree<String> = grid.map_panes(&mut |id| format!("p{id}"));
        assert_eq!(named.leaves(), vec!["p1", "p3", "p2", "p4"]);
        let frames = named.frames(rect(0.0, 0.0, 100.0, 100.0), 0.0);
        assert_eq!(frames["p4"].x, 50.0);
    }

    #[test]
    fn moving_a_pane_next_to_another() {
        let grid = grid();
        // Move 4 to the left of 1: the left column becomes H[4, 1] over 3.
        let moved = grid.moving(&4, &1, PaneDirection::Left);
        assert_eq!(moved.leaves(), vec![4, 1, 3, 2]);
        assert_eq!(moved.neighbor(&1, PaneDirection::Left), Some(4));
        // Moving onto itself or an unknown pane does nothing.
        assert_eq!(grid.moving(&1, &1, PaneDirection::Right), grid);
        assert_eq!(grid.moving(&1, &9, PaneDirection::Right), grid);
    }

    #[test]
    fn frames_tile_the_rect_with_gaps() {
        let frames = grid().frames(rect(0.0, 0.0, 801.0, 601.0), 1.0);
        assert_eq!(frames[&1], rect(0.0, 0.0, 400.0, 300.0));
        assert_eq!(frames[&3], rect(0.0, 301.0, 400.0, 300.0));
        assert_eq!(frames[&2], rect(401.0, 0.0, 400.0, 300.0));
        assert_eq!(frames[&4].max_x(), 801.0);
        assert_eq!(frames[&4].max_y(), 601.0);
        // Vertical splits: the gap sits between the stacked halves.
        let column = Tree::Leaf(1).splitting(&1, 2, Vertical, false);
        let stacked = column.frames(rect(10.0, 20.0, 100.0, 101.0), 1.0);
        assert_eq!(stacked[&1], rect(10.0, 20.0, 100.0, 50.0));
        assert_eq!(stacked[&2], rect(10.0, 71.0, 100.0, 50.0));
    }

    #[test]
    fn dividers_sit_in_the_gaps() {
        let dividers = grid().dividers(rect(0.0, 0.0, 801.0, 601.0), 1.0);
        assert_eq!(dividers.len(), 3);
        let root = dividers.iter().find(|d| d.path.is_empty()).unwrap();
        assert_eq!(root.axis, Horizontal);
        assert_eq!(root.rect, rect(400.0, 0.0, 1.0, 601.0));
        let left = dividers.iter().find(|d| d.path == [0]).unwrap();
        assert_eq!(left.axis, Vertical);
        assert_eq!(left.rect.y, 300.0);
        assert_eq!(left.rect.height, 1.0);
        assert_eq!(left.container.width, 400.0);
    }

    #[test]
    fn neighbors_follow_geometry() {
        use PaneDirection::*;
        let grid = grid();
        assert_eq!(grid.neighbor(&1, Right), Some(2));
        assert_eq!(grid.neighbor(&1, Down), Some(3));
        assert_eq!(grid.neighbor(&4, Up), Some(2));
        assert_eq!(grid.neighbor(&4, Left), Some(3));
        assert_eq!(grid.neighbor(&1, Left), None);
        assert_eq!(grid.neighbor(&1, Up), None);
        assert_eq!(grid.neighbor(&9, Up), None);

        // A tall pane on the left next to two stacked panes: moving right
        // picks the one level with its center (the top one wins ties).
        let tall = Tree::Leaf(1)
            .splitting(&1, 2, Horizontal, false)
            .splitting(&2, 3, Vertical, false);
        assert_eq!(tall.neighbor(&1, Right), Some(2));
        assert_eq!(tall.neighbor(&3, Left), Some(1));
    }

    #[test]
    fn dividers_move_within_bounds() {
        let pair = Tree::Leaf(1).splitting(&1, 2, Horizontal, false);
        assert!(approx(
            &ratios(&pair.moving_divider(&[], 0, 0.7, None)),
            &[0.7, 0.3]
        ));
        assert!(approx(
            &ratios(&pair.moving_divider(&[], 0, 0.99, None)),
            &[0.95, 0.05]
        ));
        assert!(approx(
            &ratios(&pair.moving_divider(&[], 0, 0.2, Some(0.3))),
            &[0.3, 0.7]
        ));
        // Nested divider by path.
        let grid = grid();
        let moved = grid.moving_divider(&[1], 0, 0.25, None);
        assert!(approx(&ratios(moved.node(&[1]).unwrap()), &[0.25, 0.75]));
        // Bad paths and indexes are ignored.
        assert_eq!(grid.moving_divider(&[5], 0, 0.3, None), grid);
        assert_eq!(pair.moving_divider(&[], 3, 0.3, None), pair);
    }

    #[test]
    fn resizing_moves_the_nearest_matching_edge() {
        use PaneDirection::*;
        let grid = grid();
        // Pane 3 (bottom-left) grows rightward: the root divider moves.
        let wider = grid.resizing(&3, Right, 0.1);
        assert!(approx(&ratios(&wider), &[0.6, 0.4]));
        // Pane 3 grows upward: its column's divider moves up.
        let taller = grid.resizing(&3, Up, 0.1);
        assert!(approx(&ratios(taller.node(&[0]).unwrap()), &[0.4, 0.6]));
        // Shrinking is a negative delta.
        assert!(approx(
            &ratios(&grid.resizing(&1, Right, -0.2)),
            &[0.3, 0.7]
        ));
        // No neighbor on that side: unchanged.
        assert_eq!(grid.resizing(&1, Left, 0.1), grid);
        assert_eq!(Tree::Leaf(1).resizing(&1, Right, 0.1), Tree::Leaf(1));
    }

    #[test]
    fn json_round_trip() {
        let grid = grid();
        let json = serde_json::to_string(&grid).unwrap();
        let decoded: Tree = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, grid);
        let leaf: Tree = serde_json::from_str(r#"{"pane": 5}"#).unwrap();
        assert_eq!(leaf, Tree::Leaf(5));
    }

    #[test]
    fn decoding_repairs_bad_input() {
        // Mismatched ratios become equal shares; ratios are renormalized.
        let mismatched = r#"{"axis":"horizontal","children":[{"pane":1},{"pane":2}],"ratios":[1]}"#;
        let repaired: Tree = serde_json::from_str(mismatched).unwrap();
        assert!(approx(&ratios(&repaired), &[0.5, 0.5]));
        let unnormalized =
            r#"{"axis":"vertical","children":[{"pane":1},{"pane":2}],"ratios":[3,1]}"#;
        let scaled: Tree = serde_json::from_str(unnormalized).unwrap();
        assert!(approx(&ratios(&scaled), &[0.75, 0.25]));
        // A one-child split collapses to the child.
        let single = r#"{"axis":"vertical","children":[{"pane":4}]}"#;
        assert_eq!(serde_json::from_str::<Tree>(single).unwrap(), Tree::Leaf(4));
        // An empty split is an error.
        let empty = r#"{"axis":"vertical","children":[]}"#;
        assert!(serde_json::from_str::<Tree>(empty).is_err());
    }
}
