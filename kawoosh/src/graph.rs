//! A tree of rows drawn as lanes, the way `git log --graph` draws one:
//! a dot a row, a line from each dot down to its parent's, a branch in
//! a lane of its own that merges into the row it forked from. The undo
//! pane draws its tree with it; a commit log or a task tree would be
//! drawn the same way.
//!
//! The input is data — a parent per row, the rows in the order made,
//! so a parent comes before its children — and the output is data too:
//! a lane per row and the strokes across any row, in the row's own box.
//! Rows are drawn newest at the top, so a parent is below its children
//! and a line runs down to it. [`Graph::row`] draws one row's box with
//! kui, sized by a [`Geometry`].

use kui_native::{Color, NodeSpec, Stroke, Ui, Vec2};

/// How a graph is drawn: a lane's width, a row's height, the stroke,
/// the dot.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub lane_w: f32,
    pub row_h: f32,
    pub line_w: f32,
    pub dot: f32,
}

/// A run of rows from a leaf down through the rows whose newest
/// descendant it is: one lane from the leaf to the row it branches
/// from, where it merges in with a diagonal.
#[derive(Clone, Copy, Debug)]
struct Chain {
    lane: usize,
    /// The row it branches from; `None` for the root's chain.
    from: Option<usize>,
    /// Its lowest row.
    end: usize,
    leaf: usize,
}

/// The tree laid out in lanes. The newest leaf's chain runs to the root
/// in lane 0 — the live line stays at the left — and every other chain
/// takes the first lane that is free over its run, so a lane is reused
/// once a branch has merged.
#[derive(Default, Debug)]
pub struct Graph {
    lane: Vec<usize>,
    chains: Vec<Chain>,
    lanes: usize,
}

impl Graph {
    /// Lays out rows whose parents are `parents`: `parents[i]` is row
    /// `i`'s parent, before it, or `None` for a root.
    pub fn of(parents: &[Option<usize>]) -> Self {
        let n = parents.len();
        // A parent is before its children, so walking back, a row's
        // newest descendant is known before its parent asks: the child
        // under which it lies is the one the chain comes down through.
        let mut newest: Vec<usize> = (0..n).collect();
        let mut main_child = vec![None; n];
        for i in (0..n).rev() {
            if let Some(p) = parents[i]
                && newest[i] > newest[p]
            {
                newest[p] = newest[i];
                main_child[p] = Some(i);
            }
        }
        let mut lane = vec![0; n];
        let mut chains: Vec<Chain> = Vec::new();
        // The newest leaf comes first walking back.
        for leaf in (0..n).rev() {
            if main_child[leaf].is_some() {
                continue;
            }
            let mut end = leaf;
            while let Some(p) = parents[end]
                && main_child[p] == Some(end)
            {
                end = p;
            }
            let from = parents[end];
            // The chain's run is the rows above what it branches from,
            // up to its leaf; a lane is free when no chain placed so
            // far runs over any of them.
            let lo = from.map_or(-1, |f| f as i64);
            let taken = |l: usize| {
                chains.iter().any(|c| {
                    c.lane == l
                        && c.from.map_or(-1, |f| f as i64) < leaf as i64
                        && lo < c.leaf as i64
                })
            };
            let l = (0..).find(|l| !taken(*l)).unwrap_or(0);
            let mut node = leaf;
            loop {
                lane[node] = l;
                if node == end {
                    break;
                }
                node = parents[node].unwrap_or(end);
            }
            chains.push(Chain {
                lane: l,
                from,
                end,
                leaf,
            });
        }
        let lanes = chains.iter().map(|c| c.lane + 1).max().unwrap_or(1);
        Self {
            lane,
            chains,
            lanes,
        }
    }

    /// The lane row `i`'s dot is in.
    pub fn lane(&self, i: usize) -> usize {
        self.lane[i]
    }

    /// The lane of every row.
    pub fn lanes(&self) -> &[usize] {
        &self.lane
    }

    /// How many lanes the graph is wide.
    pub fn width(&self) -> usize {
        self.lanes
    }

    /// How many leaves the tree has past its first: the branches.
    pub fn branches(&self) -> usize {
        self.chains.len().saturating_sub(1)
    }

    /// The strokes across row `i`, in the row's box: the verticals of
    /// every chain running over it, the diagonal of a chain merging in
    /// here, and the leaf's own stem.
    pub fn strokes(&self, i: usize, geo: &Geometry) -> Vec<(Vec2, Vec2)> {
        let x = |l: usize| l as f32 * geo.lane_w + geo.lane_w / 2.0;
        let (mid, bottom) = (geo.row_h / 2.0, geo.row_h);
        let mut out = Vec::new();
        for c in &self.chains {
            let lo = c.from.map_or(-1, |f| f as i64);
            if i == c.leaf {
                // The leaf: a stem down, unless the chain is the root
                // alone.
                if i > 0 {
                    out.push((Vec2::new(x(c.lane), mid), Vec2::new(x(c.lane), bottom)));
                }
            } else if lo < i as i64 && i < c.leaf {
                if c.from.is_none() && i == c.end {
                    // The root: the line comes down to it and stops.
                    out.push((Vec2::new(x(c.lane), 0.0), Vec2::new(x(c.lane), mid)));
                } else {
                    out.push((Vec2::new(x(c.lane), 0.0), Vec2::new(x(c.lane), bottom)));
                }
            } else if c.from == Some(i) {
                // Merging into this row.
                out.push((Vec2::new(x(c.lane), 0.0), Vec2::new(x(self.lane[i]), mid)));
            }
        }
        out
    }

    /// Draws row `i`'s box: as wide as the graph, a row tall, the lines
    /// through it in `line`, then the dot in `dot` — `size` px, or the
    /// geometry's when `None`.
    pub fn row(
        &self,
        ui: &mut Ui<'_>,
        i: usize,
        geo: &Geometry,
        line: Color,
        dot: Color,
        size: Option<f32>,
    ) {
        let w = self.lanes as f32 * geo.lane_w;
        ui.with(NodeSpec::row().size(w, geo.row_h), |ui| {
            for (a, b) in self.strokes(i, geo) {
                ui.line(a, b, Stroke::new(geo.line_w, line), NodeSpec::row());
            }
            let x = self.lane[i] as f32 * geo.lane_w + geo.lane_w / 2.0;
            let c = Vec2::new(x, geo.row_h / 2.0);
            ui.polygon(
                &octagon(c, size.unwrap_or(geo.dot)),
                NodeSpec::row().bg(dot),
            );
        });
    }
}

/// An octagon of `size` about `c`: a dot, as near a circle as a
/// polygon this small needs.
fn octagon(c: Vec2, size: f32) -> [Vec2; 8] {
    let h = size / 2.0;
    let q = h / 2.0;
    [
        Vec2::new(c.x - h, c.y - q),
        Vec2::new(c.x - q, c.y - h),
        Vec2::new(c.x + q, c.y - h),
        Vec2::new(c.x + h, c.y - q),
        Vec2::new(c.x + h, c.y + q),
        Vec2::new(c.x + q, c.y + h),
        Vec2::new(c.x - q, c.y + h),
        Vec2::new(c.x - h, c.y + q),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEO: Geometry = Geometry {
        lane_w: 12.0,
        row_h: 20.0,
        line_w: 1.0,
        dot: 6.0,
    };

    /// The live line is lane 0; a branch takes the next free lane and
    /// gives it back once it has merged, so a later branch reuses it;
    /// two branches alive over the same rows take two.
    #[test]
    fn branches_take_lanes_and_give_them_back() {
        // 0 - 1 - 2 - 4 - 6, with 3 off 1 and 5 off 4: the newest leaf
        // (6) is the live line through 4, 2, 1 and 0.
        let g = Graph::of(&[None, Some(0), Some(1), Some(1), Some(2), Some(4), Some(4)]);
        assert_eq!(g.lanes(), [0, 0, 0, 1, 0, 1, 0]);
        assert_eq!(g.width(), 2);
        assert_eq!(g.branches(), 2);
        let merges: Vec<Option<usize>> = g.chains.iter().map(|c| c.from).collect();
        assert!(merges.contains(&Some(4)) && merges.contains(&Some(1)));
        // Row 2 has the branch at 3 passing beside it, top to bottom.
        let s = g.strokes(2, &GEO);
        assert!(
            s.iter()
                .any(|(a, b)| a.x == b.x && a.x > GEO.lane_w && a.y == 0.0 && b.y == GEO.row_h)
        );
        // Row 1 has the diagonal merging in.
        let s = g.strokes(1, &GEO);
        assert!(s.iter().any(|(a, b)| a.x != b.x && b.y == GEO.row_h / 2.0));
        // The root: a line down to it, no stem below.
        let s = g.strokes(0, &GEO);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].0.y, s[0].1.y), (0.0, GEO.row_h / 2.0));
        let g = Graph::of(&[None, Some(0), Some(0), Some(0)]);
        assert_eq!(g.lanes(), [0, 2, 1, 0]);
        assert_eq!(g.width(), 3);
        // A single row: a dot with no stem; no rows, nothing.
        assert!(Graph::of(&[None]).strokes(0, &GEO).is_empty());
        assert!(Graph::of(&[]).lanes().is_empty());
        assert_eq!(Graph::of(&[]).branches(), 0);
    }
}
