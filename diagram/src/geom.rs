//! Where things are, in units (docs/design/diagrams.md Decision 5): a
//! box's rectangle and outline, a group's frame, an arrow's route and
//! where its label sits, and what a point or a marquee hits.

use crate::model::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub const fn new(x: f32, y: f32) -> Pt {
        Pt { x, y }
    }

    pub fn dist(self, o: Pt) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }

    fn lerp(self, o: Pt, t: f32) -> Pt {
        Pt::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    /// The rectangle with `a` and `b` at opposite corners.
    pub fn spanning(a: Pt, b: Pt) -> Rect {
        Rect::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        )
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> Pt {
        Pt::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn contains(&self, p: Pt) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    pub fn holds(&self, o: &Rect) -> bool {
        o.x >= self.x && o.y >= self.y && o.right() <= self.right() && o.bottom() <= self.bottom()
    }

    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(
            x,
            y,
            self.right().max(o.right()) - x,
            self.bottom().max(o.bottom()) - y,
        )
    }

    pub fn grown(&self, d: f32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }
}

pub fn node_rect(n: &Node) -> Rect {
    Rect::new(n.x as f32, n.y as f32, n.w as f32, n.h as f32)
}

/// Whether `p` is inside `shape` drawn in `r`: an ellipse's equation for
/// the round ones, a diamond's, else the rectangle.
pub fn inside(shape: Shape, r: &Rect, p: Pt) -> bool {
    let c = r.center();
    let (hw, hh) = (r.w / 2.0, r.h / 2.0);
    if hw <= 0.0 || hh <= 0.0 {
        return false;
    }
    let (dx, dy) = ((p.x - c.x) / hw, (p.y - c.y) / hh);
    if shape.is_round() {
        dx * dx + dy * dy <= 1.0
    } else if shape == Shape::Diamond {
        dx.abs() + dy.abs() <= 1.0
    } else {
        dx.abs() <= 1.0 && dy.abs() <= 1.0
    }
}

/// Where the segment from `from`, inside the shape, to `to`, outside
/// it, crosses the outline: halved until it is a hair's width.
fn crossing(shape: Shape, r: &Rect, from: Pt, to: Pt) -> Pt {
    let (mut a, mut b) = (0.0f32, 1.0f32);
    if !inside(shape, r, from) || inside(shape, r, to) {
        return from;
    }
    for _ in 0..24 {
        let m = (a + b) / 2.0;
        if inside(shape, r, from.lerp(to, m)) {
            a = m;
        } else {
            b = m;
        }
    }
    from.lerp(to, b)
}

/// A group's frame: its members' with a unit round them and two above
/// for its label; `None` when it holds nothing that is there.
pub fn group_rect(doc: &Doc, id: &str, text_size: &dyn Fn(&Text) -> (f32, f32)) -> Option<Rect> {
    group_rect_placed(doc, id, &node_rect, text_size, &mut Vec::new())
}

/// An arrow's route: its points from A's outline to B's, and where its
/// label sits — the route's middle by length.
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub points: Vec<Pt>,
    pub mid: Pt,
}

/// How far apart two arrows between the same boxes run.
pub const LANE: f32 = 0.75;

/// The routes of the doc's arrows, by entry: `None` for an entry that is
/// not an arrow. `moved` stands boxes somewhere else first — the canvas
/// during a drag — by their id.
pub fn wires(doc: &Doc, moved: &dyn Fn(&Node) -> Rect) -> Vec<Option<Wire>> {
    // The arrows between each pair of boxes, either way, for their lanes.
    let mut pairs: Vec<((&str, &str), Vec<usize>)> = Vec::new();
    let mut out: Vec<Option<Wire>> = vec![None; doc.entries.len()];
    for (i, e) in doc.entries.iter().enumerate() {
        // Bent through a point of its own: its own way, in no lane.
        if let Item::Edge(edge) = &e.item
            && let Some((x, y)) = edge.via
        {
            if let (Some(a), Some(b)) = (doc.node(&edge.from), doc.node(&edge.to)) {
                let p = Pt::new(x as f32, y as f32);
                out[i] = Some(bent(a, &moved(a), b, &moved(b), edge.route, p));
            }
            continue;
        }
        if let Item::Edge(edge) = &e.item {
            let key = if edge.from <= edge.to {
                (edge.from.as_str(), edge.to.as_str())
            } else {
                (edge.to.as_str(), edge.from.as_str())
            };
            match pairs.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(i),
                None => pairs.push((key, vec![i])),
            }
        }
    }
    for (_, list) in pairs {
        let k = list.len();
        for (n, &i) in list.iter().enumerate() {
            let Item::Edge(edge) = &doc.entries[i].item else {
                continue;
            };
            let (Some(a), Some(b)) = (doc.node(&edge.from), doc.node(&edge.to)) else {
                continue;
            };
            // Lanes across the line from the pair's first box to its
            // second, whichever way this one runs: a straight one's
            // across is its own direction's, so it is turned back for
            // one that runs the other way; an elbow's is the page's.
            let lane = (n as f32 - (k as f32 - 1.0) / 2.0) * LANE;
            let across = if edge.from > edge.to && edge.route == Route::Straight {
                -lane
            } else {
                lane
            };
            out[i] = Some(route(a, moved(a), b, moved(b), edge.route, across, n));
        }
    }
    out
}

fn route(a: &Node, ra: Rect, b: &Node, rb: Rect, how: Route, lane: f32, nth: usize) -> Wire {
    if a.id == b.id {
        let points = loop_points(a.shape, &ra, nth);
        // Over the loop's top, not on it: a label as wide as the loop
        // hid the run it sat on.
        let mid = Pt::new(ra.center().x, points[1].y - 0.6);
        return Wire { points, mid };
    }
    let points = match how {
        Route::Straight => straight(a.shape, &ra, b.shape, &rb, lane),
        Route::Elbow => elbow(a.shape, &ra, b.shape, &rb, lane),
    };
    let mid = midpoint(&points);
    Wire { points, mid }
}

/// An arrow bent through `p` (docs/design/diagrams.md Decision 13): a
/// straight one from A's outline to `p` and on to B's, its label on `p`;
/// an elbow at right angles — round under or over both boxes when `p`
/// is past them that way (a U), else with its turn at `p` — its label
/// where the route passes nearest `p`. A point inside either box, or one
/// no right-angled route goes by, is not taken: the arrow as it would be
/// without it.
fn bent(a: &Node, ra: &Rect, b: &Node, rb: &Rect, how: Route, p: Pt) -> Wire {
    bent_through(a, ra, b, rb, how, p).unwrap_or_else(|| route(a, *ra, b, *rb, how, 0.0, 0))
}

/// [`bent`]'s route when it takes `p`; `None` when it does not.
fn bent_through(a: &Node, ra: &Rect, b: &Node, rb: &Rect, how: Route, p: Pt) -> Option<Wire> {
    if inside(a.shape, ra, p) || inside(b.shape, rb, p) {
        return None;
    }
    let (ca, cb) = (ra.center(), rb.center());
    let points = if a.id == b.id {
        // A loop out to `p` and back: its two legs half a box apart.
        let (dx, dy) = (p.x - ca.x, p.y - ca.y);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let spread = (ra.w.min(ra.h) * 0.25).clamp(0.2, 1.0);
        let (nx, ny) = (-dy / len * spread, dx / len * spread);
        vec![
            crossing(a.shape, ra, Pt::new(ca.x + nx, ca.y + ny), p),
            p,
            crossing(a.shape, ra, Pt::new(ca.x - nx, ca.y - ny), p),
        ]
    } else if how == Route::Straight {
        vec![
            crossing(a.shape, ra, ca, p),
            p,
            crossing(b.shape, rb, cb, p),
        ]
    } else {
        let mut pts = elbow_via(ra, rb, p)?;
        let n = pts.len();
        pts[0] = onto(a.shape, ra, pts[0], pts[1]);
        pts[n - 1] = onto(b.shape, rb, pts[n - 1], pts[n - 2]);
        simplify(pts)
    };
    let mid = nearest_on(&points, p);
    Some(Wire { points, mid })
}

/// Where arrow `i` bent through `(x, y)` puts its bend: the grid point
/// of its route nearest it — the point itself for a straight one, kept
/// on the run a U or a moved turn takes. A bend past where its route
/// goes changed nothing on the screen, and each key that took it further
/// was a key to take back before one showed (asked 2026-10-23). The
/// point as it is where the route does not take it — inside a box, a
/// move through one.
pub fn bend_at(doc: &Doc, i: usize, (x, y): (i32, i32)) -> (i32, i32) {
    let Some(Item::Edge(e)) = doc.entries.get(i).map(|e| &e.item) else {
        return (x, y);
    };
    let (Some(a), Some(b)) = (doc.node(&e.from), doc.node(&e.to)) else {
        return (x, y);
    };
    let p = Pt::new(x as f32, y as f32);
    match bent_through(a, &node_rect(a), b, &node_rect(b), e.route, p) {
        Some(w) => (w.mid.x.round() as i32, w.mid.y.round() as i32),
        None => (x, y),
    }
}

/// An elbow's route through `p`, its ends on the boxes' sides: a U under
/// or over both — or, for boxes one above the other, a C beside both —
/// at `p`'s height (or across); else the turn of a plain elbow moved to
/// `p`. `None` when `p` is where neither goes.
fn elbow_via(ra: &Rect, rb: &Rect, p: Pt) -> Option<Vec<Pt>> {
    let (ca, cb) = (ra.center(), rb.center());
    let gap_x = (rb.x - ra.right()).max(ra.x - rb.right());
    let gap_y = (rb.y - ra.bottom()).max(ra.y - rb.bottom());
    let beside = gap_x >= gap_y;
    let above = p.y < ra.y.min(rb.y);
    let below = p.y > ra.bottom().max(rb.bottom());
    let left = p.x < ra.x.min(rb.x);
    let right = p.x > ra.right().max(rb.right());
    // Out of the tops or the bottoms, across at `p`'s height.
    let u = |top: bool| {
        let (ya, yb) = if top {
            (ra.y, rb.y)
        } else {
            (ra.bottom(), rb.bottom())
        };
        vec![
            Pt::new(ca.x, ya),
            Pt::new(ca.x, p.y),
            Pt::new(cb.x, p.y),
            Pt::new(cb.x, yb),
        ]
    };
    // Out of the left sides or the right ones, down at `p`'s place.
    let c = |l: bool| {
        let (xa, xb) = if l {
            (ra.x, rb.x)
        } else {
            (ra.right(), rb.right())
        };
        vec![
            Pt::new(xa, ca.y),
            Pt::new(p.x, ca.y),
            Pt::new(p.x, cb.y),
            Pt::new(xb, cb.y),
        ]
    };
    if beside {
        if above || below {
            return Some(u(above));
        }
        let (xa, xb) = if cb.x >= ca.x {
            (ra.right(), rb.x)
        } else {
            (ra.x, rb.right())
        };
        if gap_x > 0.0 && p.x > xa.min(xb) && p.x < xa.max(xb) {
            return Some(vec![
                Pt::new(xa, ca.y),
                Pt::new(p.x, ca.y),
                Pt::new(p.x, cb.y),
                Pt::new(xb, cb.y),
            ]);
        }
        if left || right {
            return Some(c(left));
        }
    } else {
        if left || right {
            return Some(c(left));
        }
        let (ya, yb) = if cb.y >= ca.y {
            (ra.bottom(), rb.y)
        } else {
            (ra.y, rb.bottom())
        };
        if gap_y > 0.0 && p.y > ya.min(yb) && p.y < ya.max(yb) {
            return Some(vec![
                Pt::new(ca.x, ya),
                Pt::new(ca.x, p.y),
                Pt::new(cb.x, p.y),
                Pt::new(cb.x, yb),
            ]);
        }
        if above || below {
            return Some(u(above));
        }
    }
    None
}

/// A route's runs with no turn in them made one: a point twice, or on
/// the line between its neighbours, goes.
fn simplify(mut pts: Vec<Pt>) -> Vec<Pt> {
    pts.dedup_by(|a, b| a.dist(*b) < 1e-4);
    let mut i = 1;
    while i + 1 < pts.len() {
        let (a, b, c) = (pts[i - 1], pts[i], pts[i + 1]);
        if ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() < 1e-4 {
            pts.remove(i);
        } else {
            i += 1;
        }
    }
    pts
}

/// The point of the route `pts` nearest `p`.
pub fn nearest_on(pts: &[Pt], p: Pt) -> Pt {
    let mut best = (f32::MAX, pts.first().copied().unwrap_or(p));
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let l2 = dx * dx + dy * dy;
        let t = if l2 <= 0.0 {
            0.0
        } else {
            (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0)
        };
        let q = Pt::new(a.x + t * dx, a.y + t * dy);
        let d = q.dist(p);
        if d < best.0 {
            best = (d, q);
        }
    }
    best.1
}

/// From centre to centre, moved `lane` across, cut at both outlines.
fn straight(sa: Shape, ra: &Rect, sb: Shape, rb: &Rect, lane: f32) -> Vec<Pt> {
    let (ca, cb) = (ra.center(), rb.center());
    let (dx, dy) = (cb.x - ca.x, cb.y - ca.y);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    // Across: the line turned a quarter, a lane long; the ends kept
    // inside their boxes.
    let (nx, ny) = (-dy / len * lane, dx / len * lane);
    let clamp_in = |r: &Rect, p: Pt| {
        Pt::new(
            p.x.clamp(r.x + r.w * 0.1, r.right() - r.w * 0.1),
            p.y.clamp(r.y + r.h * 0.1, r.bottom() - r.h * 0.1),
        )
    };
    let pa = clamp_in(ra, Pt::new(ca.x + nx, ca.y + ny));
    let pb = clamp_in(rb, Pt::new(cb.x + nx, cb.y + ny));
    // Overlapping boxes: centre to centre is all there is.
    if inside(sb, rb, pa) || inside(sa, ra, pb) {
        return vec![pa, pb];
    }
    vec![crossing(sa, ra, pa, pb), crossing(sb, rb, pb, pa)]
}

/// Out of the side facing B, a turn at the gap's middle, into the side
/// facing A — across or down, whichever gap is the wider.
fn elbow(sa: Shape, ra: &Rect, sb: Shape, rb: &Rect, lane: f32) -> Vec<Pt> {
    let gap_x = (rb.x - ra.right()).max(ra.x - rb.right());
    let gap_y = (rb.y - ra.bottom()).max(ra.y - rb.bottom());
    if gap_x <= 0.0 && gap_y <= 0.0 {
        return straight(sa, ra, sb, rb, lane);
    }
    let (ca, cb) = (ra.center(), rb.center());
    let mut pts = if gap_x >= gap_y {
        let ay = (ca.y + lane).clamp(ra.y, ra.bottom());
        let by = (cb.y + lane).clamp(rb.y, rb.bottom());
        let (xa, xb) = if cb.x >= ca.x {
            (ra.right(), rb.x)
        } else {
            (ra.x, rb.right())
        };
        let mx = (xa + xb) / 2.0;
        vec![
            Pt::new(xa, ay),
            Pt::new(mx, ay),
            Pt::new(mx, by),
            Pt::new(xb, by),
        ]
    } else {
        let ax = (ca.x + lane).clamp(ra.x, ra.right());
        let bx = (cb.x + lane).clamp(rb.x, rb.right());
        let (ya, yb) = if cb.y >= ca.y {
            (ra.bottom(), rb.y)
        } else {
            (ra.y, rb.bottom())
        };
        let my = (ya + yb) / 2.0;
        vec![
            Pt::new(ax, ya),
            Pt::new(ax, my),
            Pt::new(bx, my),
            Pt::new(bx, yb),
        ]
    };
    // A round or a diamond box's side is not its outline: each end
    // moved in to where the outline is, along its first leg.
    let n = pts.len();
    pts[0] = onto(sa, ra, pts[0], pts[1]);
    pts[n - 1] = onto(sb, rb, pts[n - 1], pts[n - 2]);
    simplify(pts)
}

/// `p` on the side of a box of `shape` in `r`, moved onto its outline
/// along the line to `toward`.
fn onto(shape: Shape, r: &Rect, p: Pt, toward: Pt) -> Pt {
    if matches!(shape, Shape::Rect | Shape::Round | Shape::Cylinder) {
        return p;
    }
    // Along the leg's own line: from where it passes nearest the centre,
    // inside, out past the side towards the next point — so the leg
    // stays straight (a vertical one vertical).
    let len = p.dist(toward).max(1e-3);
    let (ux, uy) = ((toward.x - p.x) / len, (toward.y - p.y) / len);
    let c = r.center();
    let t = (c.x - p.x) * ux + (c.y - p.y) * uy;
    let start = Pt::new(p.x + ux * t, p.y + uy * t);
    if !inside(shape, r, start) {
        return p;
    }
    let deep = r.w.max(r.h);
    crossing(
        shape,
        r,
        start,
        Pt::new(start.x + ux * deep, start.y + uy * deep),
    )
}

/// A loop on a box: up out of its top left of the middle, across, and
/// back down into its top right of it — the sides are where the other
/// arrows come in; the `nth` a lane higher and wider. Its ends sit on
/// the outline, an ellipse's or a diamond's as a box's.
fn loop_points(shape: Shape, r: &Rect, nth: usize) -> Vec<Pt> {
    let out = 1.5 + nth as f32 * LANE;
    let half = (r.w / 4.0).clamp(0.25, 1.5) + nth as f32 * LANE / 2.0;
    let half = half.min(r.w / 2.0);
    let c = r.center();
    let (x0, x1) = (c.x - half, c.x + half);
    let top = r.y - out;
    vec![
        onto(shape, r, Pt::new(x0, r.y), Pt::new(x0, top)),
        Pt::new(x0, top),
        Pt::new(x1, top),
        onto(shape, r, Pt::new(x1, r.y), Pt::new(x1, top)),
    ]
}

/// The point halfway along `pts` by length.
pub fn midpoint(pts: &[Pt]) -> Pt {
    let total: f32 = pts.windows(2).map(|w| w[0].dist(w[1])).sum();
    let mut left = total / 2.0;
    for w in pts.windows(2) {
        let d = w[0].dist(w[1]);
        if d >= left && d > 0.0 {
            return w[0].lerp(w[1], left / d);
        }
        left -= d;
    }
    pts.first().copied().unwrap_or_default()
}

/// How far `p` is from the segment `a`–`b`.
pub fn seg_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    let t = if l2 <= 0.0 {
        0.0
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0)
    };
    p.dist(Pt::new(a.x + t * dx, a.y + t * dy))
}

/// Where everything is, once for a frame and its events: each entry's
/// bounds (`None` for a group with nothing, or an arrow, whose wire is
/// its), and each arrow's wire.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub bounds: Vec<Option<Rect>>,
    pub wires: Vec<Option<Wire>>,
}

impl Layout {
    /// `doc` laid out, a text's size in units by `text_size`, a box
    /// placed by `place` (where a drag has it).
    pub fn of(
        doc: &Doc,
        place: &dyn Fn(&Node) -> Rect,
        text_size: &dyn Fn(&Text) -> (f32, f32),
    ) -> Layout {
        let wires = wires(doc, place);
        let bounds = doc
            .entries
            .iter()
            .map(|e| match &e.item {
                Item::Node(n) => Some(place(n)),
                Item::Text(t) => {
                    let (w, h) = text_size(t);
                    Some(Rect::new(t.x as f32, t.y as f32, w, h))
                }
                Item::Group(g) => group_rect_placed(doc, &g.id, place, text_size, &mut Vec::new()),
                Item::Edge(_) => None,
            })
            .collect();
        Layout { bounds, wires }
    }

    /// The topmost entry under `p`, `tol` units of slack for a line: the
    /// last drawn first (Decision 14) — a box, an arrow or its label, a
    /// text, by the file's order — then a group by its frame's edge or its
    /// label's band.
    pub fn hit(&self, doc: &Doc, p: Pt, tol: f32) -> Option<usize> {
        let order: Vec<usize> = doc.draw_order().collect();
        for &i in order.iter().rev() {
            let hit = match (&doc.entries[i].item, &self.bounds[i], &self.wires[i]) {
                (Item::Text(_), Some(r), _) => r.grown(tol / 2.0).contains(p),
                (Item::Node(n), Some(r), _) => {
                    inside(n.shape, r, p) || (n.w.min(n.h) <= 1 && r.grown(tol).contains(p))
                }
                (Item::Edge(e), _, Some(w)) => {
                    // Its bend's point too, where the canvas draws a dot
                    // to pull: an elbow passes by it, not always through it.
                    let bend = e
                        .via
                        .is_some_and(|(x, y)| Pt::new(x as f32, y as f32).dist(p) <= tol * 2.0);
                    bend || w.points.windows(2).any(|s| seg_dist(p, s[0], s[1]) <= tol)
                        || (!e.label.is_empty() && w.mid.dist(p) <= tol * 2.0)
                }
                (Item::Group(_), Some(r), _) => {
                    let band = Rect::new(r.x, r.y, r.w, 2.0);
                    let on_edge = r.grown(tol).contains(p) && !r.grown(-tol).contains(p);
                    on_edge || band.contains(p)
                }
                _ => false,
            };
            if hit {
                return Some(i);
            }
        }
        None
    }

    /// The entries a marquee over `r` holds whole: an arrow when both its
    /// ends' boxes are among them.
    pub fn within(&self, doc: &Doc, r: &Rect) -> Vec<usize> {
        let mut out: Vec<usize> = (0..doc.entries.len())
            .filter(|&i| self.bounds[i].is_some_and(|b| r.holds(&b)))
            .collect();
        let held = |id: &str| {
            out.iter()
                .any(|&i| matches!(&doc.entries[i].item, Item::Node(n) if n.id == id))
        };
        let edges: Vec<usize> = doc
            .entries
            .iter()
            .enumerate()
            .filter(
                |(_, e)| matches!(&e.item, Item::Edge(edge) if held(&edge.from) && held(&edge.to)),
            )
            .map(|(i, _)| i)
            .collect();
        out.extend(edges);
        out.sort_unstable();
        out
    }

    /// Everything's bounds together: what `fit` frames.
    pub fn extent(&self) -> Option<Rect> {
        let wires = self
            .wires
            .iter()
            .flatten()
            .flat_map(|w| w.points.iter())
            .map(|p| Rect::new(p.x, p.y, 0.0, 0.0));
        self.bounds
            .iter()
            .flatten()
            .copied()
            .chain(wires)
            .reduce(|a, b| a.union(&b))
    }
}

fn group_rect_placed(
    doc: &Doc,
    id: &str,
    place: &dyn Fn(&Node) -> Rect,
    text_size: &dyn Fn(&Text) -> (f32, f32),
    seen: &mut Vec<String>,
) -> Option<Rect> {
    if seen.iter().any(|s| s == id) {
        return None;
    }
    seen.push(id.to_string());
    let g = doc.group(id)?;
    let mut out: Option<Rect> = None;
    for m in &g.members {
        let text = doc.text_index(m).and_then(|i| match &doc.entries[i].item {
            Item::Text(t) => Some(t),
            _ => None,
        });
        let r = match (doc.node(m), text) {
            (Some(n), _) => Some(place(n)),
            (None, Some(t)) => {
                let (w, h) = text_size(t);
                Some(Rect::new(t.x as f32, t.y as f32, w, h))
            }
            (None, None) => group_rect_placed(doc, m, place, text_size, seen),
        };
        if let Some(r) = r {
            out = Some(out.map_or(r, |o| o.union(&r)));
        }
    }
    out.map(|r| Rect::new(r.x - 1.0, r.y - 2.0, r.w + 2.0, r.h + 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn at_rest(n: &Node) -> Rect {
        node_rect(n)
    }

    fn layout(text: &str) -> (Doc, Layout) {
        let doc = parse(text);
        assert_eq!(doc.problems, vec![]);
        let l = Layout::of(&doc, &at_rest, &|t| {
            (t.label.chars().count() as f32 * 0.5, 1.0)
        });
        (doc, l)
    }

    fn close(a: Pt, b: Pt) -> bool {
        a.dist(b) < 1e-3
    }

    #[test]
    fn a_straight_arrow_leaves_and_meets_the_outlines() {
        let (_, l) = layout(
            "box a at 0,0 size 4x2\nbox b at 10,0 size 4x2\nbox c at 10,10 size 4x4 shape ellipse\narrow a -> b\narrow a -> c\n",
        );
        let w = l.wires[3].as_ref().unwrap();
        assert!(close(w.points[0], Pt::new(4.0, 1.0)), "{w:?}");
        assert!(close(w.points[1], Pt::new(10.0, 1.0)), "{w:?}");
        assert!(close(w.mid, Pt::new(7.0, 1.0)));
        // Into the ellipse at its outline, not its box's corner.
        let w = l.wires[4].as_ref().unwrap();
        let end = w.points[1];
        let c = Pt::new(12.0, 12.0);
        assert!((end.dist(c) - 2.0).abs() < 1e-2, "{end:?} on the circle");
    }

    #[test]
    fn two_arrows_between_two_boxes_run_side_by_side() {
        let (_, l) =
            layout("box a at 0,0 size 4x4\nbox b at 10,0 size 4x4\narrow a -> b\narrow b -> a\n");
        let (w1, w2) = (l.wires[2].as_ref().unwrap(), l.wires[3].as_ref().unwrap());
        // Horizontal, a lane apart.
        assert!((w1.points[0].y - w1.points[1].y).abs() < 1e-3);
        assert!(
            ((w1.points[0].y - w2.points[0].y).abs() - LANE).abs() < 1e-3,
            "{w1:?} {w2:?}"
        );
    }

    #[test]
    fn an_elbow_turns_once_in_the_gap() {
        let (_, l) = layout("box a at 0,0 size 4x2\nbox b at 12,6 size 4x2\narrow a -> b elbow\n");
        let w = l.wires[2].as_ref().unwrap();
        assert_eq!(
            w.points,
            vec![
                Pt::new(4.0, 1.0),
                Pt::new(8.0, 1.0),
                Pt::new(8.0, 7.0),
                Pt::new(12.0, 7.0)
            ]
        );
        // Lined up, it is one straight run.
        let (_, l) = layout("box a at 0,0 size 4x2\nbox b at 12,0 size 4x2\narrow a -> b elbow\n");
        assert_eq!(l.wires[2].as_ref().unwrap().points.len(), 2);
        // Below rather than beside: down, across, down.
        let (_, l) = layout("box a at 0,0 size 4x2\nbox b at 2,10 size 4x2\narrow a -> b elbow\n");
        let w = l.wires[2].as_ref().unwrap();
        assert_eq!(w.points[0], Pt::new(2.0, 2.0));
        assert_eq!(*w.points.last().unwrap(), Pt::new(4.0, 10.0));
    }

    #[test]
    fn a_loop_goes_up_from_its_top_and_back_down_into_it() {
        let (_, l) = layout(
            "box a at 0,0 size 4x2\narrow a -> a\nbox e at 10,0 size 6x4 shape ellipse\narrow e -> e\n",
        );
        let w = l.wires[1].as_ref().unwrap();
        assert_eq!(
            w.points,
            vec![
                Pt::new(1.0, 0.0),
                Pt::new(1.0, -1.5),
                Pt::new(3.0, -1.5),
                Pt::new(3.0, 0.0)
            ]
        );
        // On an ellipse's outline, not its box's top.
        let w = l.wires[3].as_ref().unwrap();
        let (first, last) = (w.points[0], *w.points.last().unwrap());
        let on = |p: Pt| ((p.x - 13.0) / 3.0).powi(2) + ((p.y - 2.0) / 2.0).powi(2);
        assert!(
            (on(first) - 1.0).abs() < 1e-2 && (on(last) - 1.0).abs() < 1e-2,
            "{w:?}"
        );
        assert!(first.y > 0.0, "below the box's top: {first:?}");
        // Its legs straight up.
        assert!((first.x - w.points[1].x).abs() < 1e-3, "{w:?}");
        assert!((last.x - w.points[2].x).abs() < 1e-3, "{w:?}");
    }

    #[test]
    fn an_arrow_bent_through_a_point_goes_by_it_its_label_on_it() {
        let (_, l) = layout(
            "box a at 0,0 size 4x2\nbox b at 10,0 size 4x2\narrow a -> b via 7,6\narrow a -> b \"u\" via 7,6 elbow\narrow a -> b via 7,-1 elbow\narrow a -> b via 2,1\n",
        );
        // Straight: A's outline, the point, B's outline.
        let w = l.wires[2].as_ref().unwrap();
        assert_eq!(w.points.len(), 3, "{w:?}");
        assert_eq!(w.points[1], Pt::new(7.0, 6.0));
        assert_eq!(w.mid, Pt::new(7.0, 6.0), "the label on the point");
        assert!(
            (w.points[0].y - 2.0).abs() < 1e-3,
            "out of A's bottom: {w:?}"
        );
        // An elbow under both: a U at the point's height.
        let w = l.wires[3].as_ref().unwrap();
        assert_eq!(
            w.points,
            vec![
                Pt::new(2.0, 2.0),
                Pt::new(2.0, 6.0),
                Pt::new(12.0, 6.0),
                Pt::new(12.0, 2.0)
            ]
        );
        assert_eq!(w.mid, Pt::new(7.0, 6.0));
        // Over both, a unit up: a U over them.
        let w = l.wires[4].as_ref().unwrap();
        assert_eq!(w.points[1], Pt::new(2.0, -1.0));
        // A point inside a box is not taken.
        let w = l.wires[5].as_ref().unwrap();
        assert_eq!(w.points.len(), 2, "{w:?}");
        assert!(
            close(w.points[0], Pt::new(4.0, 1.0)) && close(w.points[1], Pt::new(10.0, 1.0)),
            "{w:?}"
        );
        // Between them, an elbow's turn moved to it.
        let (_, l) =
            layout("box a at 0,0 size 4x2\nbox b at 12,6 size 4x2\narrow a -> b via 5,4 elbow\n");
        let w = l.wires[2].as_ref().unwrap();
        assert_eq!(
            w.points,
            vec![
                Pt::new(4.0, 1.0),
                Pt::new(5.0, 1.0),
                Pt::new(5.0, 7.0),
                Pt::new(12.0, 7.0)
            ]
        );
        assert_eq!(w.mid, Pt::new(5.0, 4.0));
    }

    #[test]
    fn a_point_hits_the_topmost_thing_under_it() {
        let (doc, l) = layout(
            "box a at 0,0 size 4x2\nbox b at 10,0 size 4x2 shape diamond\narrow a -> b \"x\"\ntext \"hi\" at 0,5\ngroup g \"G\" { a b }\n",
        );
        assert_eq!(l.hit(&doc, Pt::new(1.0, 1.0), 0.3), Some(0));
        assert_eq!(l.hit(&doc, Pt::new(12.0, 1.0), 0.3), Some(1));
        // A diamond's corner is not the diamond.
        assert_eq!(l.hit(&doc, Pt::new(10.2, 0.2), 0.3), None);
        assert_eq!(l.hit(&doc, Pt::new(6.0, 1.1), 0.3), Some(2), "the arrow");
        assert_eq!(l.hit(&doc, Pt::new(0.5, 5.5), 0.3), Some(3), "the text");
        // The group by its label's band and its edge, not inside.
        let g = l.bounds[4].unwrap();
        assert_eq!(g, Rect::new(-1.0, -2.0, 16.0, 5.0));
        assert_eq!(l.hit(&doc, Pt::new(3.0, -1.5), 0.3), Some(4));
        assert_eq!(l.hit(&doc, Pt::new(-1.0, 2.5), 0.3), Some(4));
        assert_eq!(l.hit(&doc, Pt::new(7.0, 2.5), 0.3), None);
    }

    #[test]
    fn a_marquee_takes_what_it_holds_whole() {
        let (doc, l) = layout(
            "box a at 0,0 size 4x2\nbox b at 10,0 size 4x2\nbox c at 30,0 size 4x2\narrow a -> b\narrow a -> c\n",
        );
        assert_eq!(
            l.within(&doc, &Rect::new(-1.0, -1.0, 20.0, 5.0)),
            vec![0, 1, 3]
        );
        assert_eq!(l.extent(), Some(Rect::new(0.0, 0.0, 34.0, 2.0)));
    }
}
