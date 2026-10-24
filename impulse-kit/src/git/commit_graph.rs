//! Lane layout for a commit graph: given commits newest-first (topological
//! order) with their parents, assign each a lane and work out the line
//! segments drawn in its row. Lanes keep their column for as long as they
//! live, so lines run straight down.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCommit {
    pub sha: String,
    pub parents: Vec<String>,
}

impl GraphCommit {
    pub fn new(sha: impl Into<String>, parents: Vec<String>) -> Self {
        Self {
            sha: sha.into(),
            parents,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GraphSegment {
    pub from: usize,
    pub to: usize,
}

impl GraphSegment {
    pub fn new(from: usize, to: usize) -> Self {
        Self { from, to }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRow {
    /// The commit's lane (its dot).
    pub lane: usize,
    /// Segments in the top half: from a lane at the row's top to a lane at
    /// its middle (the dot's lane when the line ends at this commit).
    pub top: Vec<GraphSegment>,
    /// Segments in the bottom half: from the middle to a lane at the bottom.
    pub bottom: Vec<GraphSegment>,
    /// Lanes the row spans (for sizing).
    pub width: usize,
}

impl GraphRow {
    pub fn is_merge(&self) -> bool {
        self.bottom.iter().filter(|s| s.from == self.lane).count() > 1
    }
}

pub fn layout(commits: &[GraphCommit]) -> Vec<GraphRow> {
    Layout::default().append(commits)
}

/// Lays history out a page at a time. A row depends only on the commits
/// above it, so appending pages gives the same rows as laying out all of
/// them at once.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// What each lane is waiting for (the parent its line runs down to).
    lanes: Vec<Option<String>>,
}

impl Layout {
    pub fn new() -> Self {
        Self::default()
    }

    /// The rows for `commits`, which come right after those appended so far.
    pub fn append(&mut self, commits: &[GraphCommit]) -> Vec<GraphRow> {
        commits.iter().map(|c| self.row(c)).collect()
    }

    fn free_slot(&mut self) -> usize {
        if let Some(index) = self.lanes.iter().position(Option::is_none) {
            return index;
        }
        self.lanes.push(None);
        self.lanes.len() - 1
    }

    fn row(&mut self, commit: &GraphCommit) -> GraphRow {
        let before = self.lanes.clone();
        let node = match before
            .iter()
            .position(|s| s.as_deref() == Some(commit.sha.as_str()))
        {
            Some(lane) => lane,
            None => self.free_slot(),
        };

        // Lines arriving from above: lanes waiting for this commit converge
        // on its dot; everything else passes through.
        let mut top = Vec::new();
        for (lane, sha) in before.iter().enumerate() {
            let Some(sha) = sha else { continue };
            if *sha == commit.sha {
                top.push(GraphSegment::new(lane, node));
                self.lanes[lane] = None;
            } else {
                top.push(GraphSegment::new(lane, lane));
            }
        }

        // Lines leaving downward toward the parents.
        let mut bottom = Vec::new();
        self.lanes[node] = None;
        for (index, parent) in commit.parents.iter().enumerate() {
            if let Some(existing) = self.lanes.iter().position(|s| s.as_ref() == Some(parent)) {
                bottom.push(GraphSegment::new(node, existing));
            } else {
                let slot = if index == 0 { node } else { self.free_slot() };
                self.lanes[slot] = Some(parent.clone());
                bottom.push(GraphSegment::new(node, slot));
            }
        }
        // Lanes that pass through this row unchanged.
        for (lane, sha) in self.lanes.iter().enumerate() {
            if lane != node && sha.is_some() && before.get(lane) == Some(sha) {
                bottom.push(GraphSegment::new(lane, lane));
            }
        }

        while matches!(self.lanes.last(), Some(None)) {
            self.lanes.pop();
        }
        GraphRow {
            lane: node,
            top,
            bottom,
            width: before.len().max(self.lanes.len()).max(node + 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn c(sha: &str, parents: &[&str]) -> GraphCommit {
        GraphCommit::new(sha, parents.iter().map(|p| p.to_string()).collect())
    }

    fn s(from: usize, to: usize) -> GraphSegment {
        GraphSegment::new(from, to)
    }

    fn set(segments: &[GraphSegment]) -> HashSet<GraphSegment> {
        segments.iter().copied().collect()
    }

    fn lanes(rows: &[GraphRow]) -> Vec<usize> {
        rows.iter().map(|r| r.lane).collect()
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let rows = layout(&[c("c", &["b"]), c("b", &["a"]), c("a", &[])]);
        assert_eq!(lanes(&rows), [0, 0, 0]);
        assert!(rows[0].top.is_empty());
        assert_eq!(rows[0].bottom, [s(0, 0)]);
        assert_eq!(rows[1].top, [s(0, 0)]);
        assert!(rows[2].bottom.is_empty(), "the root has nothing below");
        assert_eq!(rows.iter().map(|r| r.width).collect::<Vec<_>>(), [1, 1, 1]);
    }

    #[test]
    fn merge_opens_a_second_lane_that_rejoins() {
        let rows = layout(&[
            c("m", &["b", "t"]),
            c("b", &["a"]),
            c("t", &["a"]),
            c("a", &[]),
        ]);
        assert_eq!(lanes(&rows), [0, 0, 1, 0]);
        assert!(rows[0].is_merge());
        assert_eq!(set(&rows[0].bottom), set(&[s(0, 0), s(0, 1)]));
        // b: lane 1 (waiting for t) passes through.
        assert_eq!(set(&rows[1].top), set(&[s(0, 0), s(1, 1)]));
        assert_eq!(set(&rows[1].bottom), set(&[s(0, 0), s(1, 1)]));
        // t's parent a is already expected in lane 0: its line bends over.
        assert_eq!(rows[2].bottom, [s(1, 0), s(0, 0)]);
        assert_eq!(rows[3].top, [s(0, 0)]);
        assert_eq!(rows[3].width, 1);
    }

    #[test]
    fn branches_converge_on_their_common_parent() {
        let rows = layout(&[c("x", &["p"]), c("y", &["p"]), c("p", &[])]);
        assert_eq!(lanes(&rows), [0, 1, 0]);
        assert_eq!(set(&rows[1].bottom), set(&[s(1, 0), s(0, 0)]));
        assert_eq!(rows[2].top, [s(0, 0)]);
        assert_eq!(rows[1].width, 2);
        assert_eq!(rows[2].width, 1);
    }

    #[test]
    fn unrelated_roots_get_their_own_lanes() {
        let rows = layout(&[c("a", &[]), c("b", &[])]);
        assert_eq!(lanes(&rows), [0, 0], "a's lane is free again for b");
        let side = layout(&[c("tip", &["base"]), c("other", &[]), c("base", &[])]);
        assert_eq!(lanes(&side), [0, 1, 0]);
    }

    #[test]
    fn octopus_merges_fan_out() {
        let rows = layout(&[
            c("o", &["a", "b", "c"]),
            c("a", &[]),
            c("b", &[]),
            c("c", &[]),
        ]);
        assert_eq!(set(&rows[0].bottom), set(&[s(0, 0), s(0, 1), s(0, 2)]));
        assert_eq!(lanes(&rows), [0, 0, 1, 2]);
    }

    #[test]
    fn pages_lay_out_like_the_whole_history() {
        let history = [
            c("m2", &["m1", "f2"]),
            c("f2", &["f1"]),
            c("m1", &["b", "t"]),
            c("x", &["b"]),
            c("b", &["a"]),
            c("t", &["a", "s"]),
            c("f1", &["a"]),
            c("a", &["r"]),
            c("s", &[]),
            c("r", &[]),
        ];
        let whole = layout(&history);
        for size in 1..=history.len() {
            let mut paged = Layout::new();
            let rows: Vec<GraphRow> = history
                .chunks(size)
                .flat_map(|page| paged.append(page))
                .collect();
            assert_eq!(rows, whole, "pages of {size}");
        }
    }
}
