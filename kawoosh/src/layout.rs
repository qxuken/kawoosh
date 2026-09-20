//! The pane tree, tabs and the dock as plain data (mvp.md Decision 5;
//! kui's splitmux is the reference shape). No kui types: the tree says
//! what is where, `panes.rs` draws it, and sessions serialise it.

use std::collections::HashMap;

use kawoosh_editor::ViewId;

pub type PaneId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDir {
    /// Side by side (`:vsplit`).
    H,
    /// Stacked (`:split`).
    V,
}

#[derive(Clone, Debug)]
pub enum Node {
    Pane(PaneId),
    Split {
        dir: SplitDir,
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

impl Node {
    pub fn panes(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Pane(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.panes(out);
                b.panes(out);
            }
        }
    }

    pub fn contains(&self, target: PaneId) -> bool {
        match self {
            Node::Pane(id) => *id == target,
            Node::Split { a, b, .. } => a.contains(target) || b.contains(target),
        }
    }

    /// Replaces the leaf `target` with a split of it and `new`, `new`
    /// after it.
    pub fn split(&mut self, target: PaneId, dir: SplitDir, new: PaneId) -> bool {
        self.split_beside(target, dir, new, false)
    }

    /// `split`, with `new` before `target` when `before`: the left or the
    /// top of the pair.
    pub fn split_beside(
        &mut self,
        target: PaneId,
        dir: SplitDir,
        new: PaneId,
        before: bool,
    ) -> bool {
        match self {
            Node::Pane(id) if *id == target => {
                let (first, second) = if before { (new, *id) } else { (*id, new) };
                *self = Node::Split {
                    dir,
                    ratio: 0.5,
                    a: Box::new(Node::Pane(first)),
                    b: Box::new(Node::Pane(second)),
                };
                true
            }
            Node::Pane(_) => false,
            Node::Split { a, b, .. } => {
                a.split_beside(target, dir, new, before) || b.split_beside(target, dir, new, before)
            }
        }
    }

    /// Exchanges two leaves, the splits around them as they were.
    pub fn swap(&mut self, x: PaneId, y: PaneId) {
        match self {
            Node::Pane(id) if *id == x => *id = y,
            Node::Pane(id) if *id == y => *id = x,
            Node::Pane(_) => {}
            Node::Split { a, b, .. } => {
                a.swap(x, y);
                b.swap(x, y);
            }
        }
    }

    /// The path ("a"/"b" steps from the root) of the split whose leaf
    /// `target` is, or `None` when it is the root or not here.
    pub fn split_of(&self, target: PaneId) -> Option<String> {
        match self {
            Node::Pane(_) => None,
            Node::Split { a, b, .. } => {
                if matches!(**a, Node::Pane(id) if id == target)
                    || matches!(**b, Node::Pane(id) if id == target)
                {
                    return Some(String::new());
                }
                a.split_of(target)
                    .map(|p| format!("a{p}"))
                    .or_else(|| b.split_of(target).map(|p| format!("b{p}")))
            }
        }
    }

    /// The split at `path` ("a"/"b" steps from the root).
    pub fn ratio_mut(&mut self, path: &str) -> Option<&mut f32> {
        match self {
            Node::Pane(_) => None,
            Node::Split { ratio, a, b, .. } => match path.split_at_checked(1) {
                None => Some(ratio),
                Some(("a", rest)) => a.ratio_mut(rest),
                Some((_, rest)) => b.ratio_mut(rest),
            },
        }
    }

    /// Removes a leaf, collapsing its split; None if the tree is empty.
    pub fn without(self, target: PaneId) -> Option<Node> {
        match self {
            Node::Pane(id) if id == target => None,
            Node::Pane(id) => Some(Node::Pane(id)),
            Node::Split { dir, ratio, a, b } => match (a.without(target), b.without(target)) {
                (Some(a), Some(b)) => Some(Node::Split {
                    dir,
                    ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                (Some(x), None) | (None, Some(x)) => Some(x),
                (None, None) => None,
            },
        }
    }

    /// Ratios along a path, for a session's layout.
    pub fn depth(&self) -> usize {
        match self {
            Node::Pane(_) => 0,
            Node::Split { a, b, .. } => 1 + a.depth().max(b.depth()),
        }
    }
}

/// Where a dragged pane lands on the pane under the pointer
/// (`Layout::drop_at`): the middle of it trades places, an edge puts it
/// beside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    Swap,
    Left,
    Right,
    Up,
    Down,
}

impl Drop {
    /// The zone a point in a rect falls in: the outer quarter on each
    /// side is that side's, the rest is the middle.
    pub fn in_rect(r: &Rect, x: f32, y: f32) -> Drop {
        let u = ((x - r.x) / r.w.max(1.0)).clamp(0.0, 1.0);
        let v = ((y - r.y) / r.h.max(1.0)).clamp(0.0, 1.0);
        let edges = [
            (u, Drop::Left),
            (1.0 - u, Drop::Right),
            (v, Drop::Up),
            (1.0 - v, Drop::Down),
        ];
        let (d, side) = edges
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap();
        if d < 0.25 { side } else { Drop::Swap }
    }
}

/// What a pane shows. Views outlive panes; a terminal outlives its pane
/// too (milestone 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Editor(ViewId),
    Terminal(u64),
    /// A Lua view by name, drawn through a kui slot.
    Lua(String),
    /// The undo history of whichever buffer has the keyboard (`undo.rs`).
    Undo,
    /// The histories in the store (`history_pane.rs`).
    History,
    /// The working memory: the register's past (`memory.rs`).
    Memory,
    /// The command registry, searched (`commands_pane.rs`).
    Commands,
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub root: Node,
    pub focused: PaneId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub tabs: Vec<Tab>,
    pub tab: usize,
    pub panes: HashMap<PaneId, Content>,
    /// The bottom dock: one pane, visible from every tab (mvp.md D5).
    pub dock: Option<PaneId>,
    pub dock_open: bool,
    /// The dock's share of the height.
    pub dock_ratio: f32,
    /// Whether the keyboard is in the dock rather than the tab.
    pub dock_focused: bool,
    /// Where each pane was drawn last frame, for directional moves.
    pub rects: HashMap<PaneId, Rect>,
    next_pane: PaneId,
}

impl Layout {
    pub fn new(first: Content) -> Self {
        let mut panes = HashMap::new();
        panes.insert(1, first);
        Self {
            tabs: vec![Tab {
                root: Node::Pane(1),
                focused: 1,
            }],
            tab: 0,
            panes,
            dock: None,
            dock_open: false,
            dock_ratio: 0.3,
            dock_focused: false,
            rects: HashMap::new(),
            next_pane: 2,
        }
    }

    pub fn new_pane(&mut self, content: Content) -> PaneId {
        let id = self.next_pane;
        self.next_pane += 1;
        self.panes.insert(id, content);
        id
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.tab]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.tab]
    }

    /// The pane the keyboard goes to.
    pub fn focused(&self) -> PaneId {
        match (self.dock_focused, self.dock) {
            (true, Some(d)) if self.dock_open => d,
            _ => self.tab().focused,
        }
    }

    pub fn content(&self, pane: PaneId) -> Option<Content> {
        self.panes.get(&pane).cloned()
    }

    pub fn focused_content(&self) -> Option<Content> {
        self.content(self.focused())
    }

    /// Every pane on screen: the tab's and, if open, the dock's.
    pub fn visible_panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.tab().root.panes(&mut out);
        if let (true, Some(d)) = (self.dock_open, self.dock) {
            out.push(d);
        }
        out
    }

    /// Every pane in any tab or the dock.
    pub fn all_panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        for t in &self.tabs {
            t.root.panes(&mut out);
        }
        if let Some(d) = self.dock {
            out.push(d);
        }
        out
    }

    pub fn focus(&mut self, pane: PaneId) {
        if self.dock == Some(pane) {
            self.dock_focused = true;
            return;
        }
        self.dock_focused = false;
        if let Some(i) = self.tabs.iter().position(|t| t.root.contains(pane)) {
            self.tab = i;
            self.tabs[i].focused = pane;
        }
    }

    /// Splits the focused pane; the new pane takes focus.
    pub fn split(&mut self, dir: SplitDir, content: Content) -> PaneId {
        let new = self.new_pane(content);
        if self.dock_focused && self.dock.is_some() {
            // Splitting the dock puts the new pane in the tab instead.
            self.dock_focused = false;
        }
        let target = self.tab().focused;
        let t = self.tab_mut();
        t.root.split(target, dir, new);
        t.focused = new;
        new
    }

    /// Closes a pane. The last pane of the last tab stays. Returns the
    /// content it showed, so the caller can decide what to keep.
    pub fn close(&mut self, pane: PaneId) -> Option<Content> {
        if self.dock == Some(pane) {
            self.dock = None;
            self.dock_open = false;
            self.dock_focused = false;
            return self.panes.remove(&pane);
        }
        let ti = self.tabs.iter().position(|t| t.root.contains(pane))?;
        let root = std::mem::replace(&mut self.tabs[ti].root, Node::Pane(0));
        match root.without(pane) {
            Some(root) => {
                self.tabs[ti].root = root;
                if self.tabs[ti].focused == pane {
                    let mut ps = Vec::new();
                    self.tabs[ti].root.panes(&mut ps);
                    self.tabs[ti].focused = ps[0];
                }
            }
            None if self.tabs.len() > 1 => {
                self.tabs.remove(ti);
                if self.tab >= self.tabs.len() {
                    self.tab = self.tabs.len() - 1;
                }
            }
            None => {
                // The last pane of the last tab: keep it.
                self.tabs[ti].root = Node::Pane(pane);
                return None;
            }
        }
        self.panes.remove(&pane)
    }

    /// Closes every other pane in the tab.
    pub fn only(&mut self) -> Vec<Content> {
        let keep = self.tab().focused;
        let mut gone = Vec::new();
        let mut ps = Vec::new();
        self.tab().root.panes(&mut ps);
        for p in ps {
            if p != keep {
                gone.extend(self.close(p));
            }
        }
        gone
    }

    pub fn new_tab(&mut self, content: Content) -> PaneId {
        let p = self.new_pane(content);
        self.tabs.push(Tab {
            root: Node::Pane(p),
            focused: p,
        });
        self.tab = self.tabs.len() - 1;
        self.dock_focused = false;
        p
    }

    pub fn next_tab(&mut self, by: i64) {
        let n = self.tabs.len() as i64;
        self.tab = ((self.tab as i64 + by).rem_euclid(n)) as usize;
        self.dock_focused = false;
    }

    /// The tab's other pane nearest in `dir` from the focused one, by
    /// last frame's rects.
    pub fn neighbour(&self, dir: SplitDir, forward: bool) -> Option<PaneId> {
        let from = self.focused();
        let r = *self.rects.get(&from)?;
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let mut best: Option<(f32, PaneId)> = None;
        for p in self.visible_panes() {
            if p == from {
                continue;
            }
            let Some(q) = self.rects.get(&p) else {
                continue;
            };
            let (qx, qy) = (q.x + q.w / 2.0, q.y + q.h / 2.0);
            let (ahead, overlap, dist) = match dir {
                SplitDir::H => (
                    if forward {
                        q.x >= r.x + r.w - 1.0
                    } else {
                        q.x + q.w <= r.x + 1.0
                    },
                    q.y < r.y + r.h && q.y + q.h > r.y,
                    (qx - cx).abs() + (qy - cy).abs() * 0.1,
                ),
                SplitDir::V => (
                    if forward {
                        q.y >= r.y + r.h - 1.0
                    } else {
                        q.y + q.h <= r.y + 1.0
                    },
                    q.x < r.x + r.w && q.x + q.w > r.x,
                    (qy - cy).abs() + (qx - cx).abs() * 0.1,
                ),
            };
            if ahead && overlap && best.is_none_or(|(d, _)| dist < d) {
                best = Some((dist, p));
            }
        }
        best.map(|(_, p)| p)
    }

    /// The tab's pane under a point (never the dock) and where a drop
    /// there would land, by last frame's rects.
    pub fn drop_at(&self, x: f32, y: f32) -> Option<(PaneId, Drop)> {
        let mut ps = Vec::new();
        self.tab().root.panes(&mut ps);
        ps.into_iter().find_map(|p| {
            let r = self.rects.get(&p)?;
            let inside = x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
            inside.then(|| (p, Drop::in_rect(r, x, y)))
        })
    }

    /// Moves `pane` onto `target` in the tab: a `Swap` trades their
    /// places, a side takes `pane` out of its split and puts it beside
    /// `target` in a new one. Nothing when either is the dock, not in
    /// the tab, or the same pane. The moved pane keeps the keyboard.
    pub fn move_pane(&mut self, pane: PaneId, target: PaneId, at: Drop) -> bool {
        if pane == target || self.dock == Some(pane) || self.dock == Some(target) {
            return false;
        }
        let t = self.tab_mut();
        if !t.root.contains(pane) || !t.root.contains(target) {
            return false;
        }
        match at {
            Drop::Swap => t.root.swap(pane, target),
            side => {
                let root = std::mem::replace(&mut t.root, Node::Pane(0));
                // `target` stays, so the tree is never empty.
                let mut root = root.without(pane).unwrap_or(Node::Pane(target));
                let (dir, before) = match side {
                    Drop::Left => (SplitDir::H, true),
                    Drop::Right => (SplitDir::H, false),
                    Drop::Up => (SplitDir::V, true),
                    _ => (SplitDir::V, false),
                };
                root.split_beside(target, dir, pane, before);
                t.root = root;
            }
        }
        t.focused = pane;
        self.dock_focused = false;
        true
    }

    /// The next pane in tree order (`<C-w>w`).
    pub fn next_pane(&self) -> PaneId {
        let ps = self.visible_panes();
        let i = ps.iter().position(|p| *p == self.focused()).unwrap_or(0);
        ps[(i + 1) % ps.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> Content {
        Content::Terminal(0)
    }

    #[test]
    fn split_close_only() {
        let mut l = Layout::new(view());
        let b = l.split(SplitDir::H, view());
        let c = l.split(SplitDir::V, view());
        assert_eq!(l.visible_panes(), [1, b, c]);
        assert_eq!(l.focused(), c);
        l.close(c);
        assert_eq!(l.visible_panes(), [1, b]);
        assert_eq!(l.focused(), 1);
        l.close(1);
        assert_eq!(l.visible_panes(), [b]);
        assert!(l.close(b).is_none(), "the last pane stays");
        assert_eq!(l.visible_panes(), [b]);
        l.split(SplitDir::H, view());
        l.split(SplitDir::H, view());
        l.only();
        assert_eq!(l.visible_panes().len(), 1);
    }

    #[test]
    fn a_leaf_names_its_split() {
        let mut l = Layout::new(view());
        assert_eq!(l.tab().root.split_of(1), None, "the root is no split");
        let b = l.split(SplitDir::H, view());
        assert_eq!(l.tab().root.split_of(1).as_deref(), Some(""));
        assert_eq!(l.tab().root.split_of(b).as_deref(), Some(""));
        let c = l.split(SplitDir::V, view());
        assert_eq!(l.tab().root.split_of(c).as_deref(), Some("b"));
        assert_eq!(l.tab().root.split_of(b).as_deref(), Some("b"));
        assert_eq!(l.tab().root.split_of(1).as_deref(), Some(""));
        assert_eq!(l.tab().root.split_of(99), None);
        *l.tab_mut().root.ratio_mut("b").unwrap() = 0.25;
    }

    #[test]
    fn a_pane_moves_onto_another() {
        // 1 | b over c
        let mut l = Layout::new(view());
        let b = l.split(SplitDir::H, view());
        let c = l.split(SplitDir::V, view());
        let rect = |x, y, w, h| Rect { x, y, w, h };
        l.rects.insert(1, rect(0.0, 0.0, 100.0, 100.0));
        l.rects.insert(b, rect(100.0, 0.0, 100.0, 50.0));
        l.rects.insert(c, rect(100.0, 50.0, 100.0, 50.0));
        assert_eq!(l.drop_at(50.0, 50.0), Some((1, Drop::Swap)));
        assert_eq!(l.drop_at(5.0, 50.0), Some((1, Drop::Left)));
        assert_eq!(l.drop_at(150.0, 45.0), Some((b, Drop::Down)));
        assert_eq!(l.drop_at(199.0, 75.0), Some((c, Drop::Right)));
        assert_eq!(l.drop_at(300.0, 75.0), None);
        // A swap keeps the splits and gives the moved pane the keyboard.
        l.focus(1);
        assert!(l.move_pane(1, c, Drop::Swap));
        assert_eq!(l.visible_panes(), [c, b, 1]);
        assert_eq!(l.focused(), 1);
        assert_eq!(l.tab().root.depth(), 2);
        // Beside: out of its split, into a new one on the side asked.
        assert!(l.move_pane(1, c, Drop::Left));
        assert_eq!(l.visible_panes(), [1, c, b]);
        assert_eq!(l.tab().root.split_of(1).as_deref(), Some("a"));
        assert!(matches!(
            l.tab().root,
            Node::Split { dir: SplitDir::H, ref a, .. }
                if matches!(**a, Node::Split { dir: SplitDir::H, .. })
        ));
        assert!(l.move_pane(b, 1, Drop::Up));
        assert_eq!(l.visible_panes(), [b, 1, c]);
        assert_eq!(l.tab().root.depth(), 2, "the split b left collapsed");
        // Onto itself, or the dock: nothing.
        assert!(!l.move_pane(b, b, Drop::Swap));
        let d = l.new_pane(view());
        l.dock = Some(d);
        assert!(!l.move_pane(b, d, Drop::Swap));
        assert!(!l.move_pane(d, b, Drop::Left));
        assert_eq!(l.visible_panes(), [b, 1, c]);
    }

    #[test]
    fn tabs_and_dock() {
        let mut l = Layout::new(view());
        let t2 = l.new_tab(view());
        assert_eq!(l.tab, 1);
        assert_eq!(l.focused(), t2);
        l.next_tab(1);
        assert_eq!(l.tab, 0);
        let d = l.new_pane(view());
        l.dock = Some(d);
        l.dock_open = true;
        assert_eq!(l.visible_panes(), [1, d]);
        l.focus(d);
        assert_eq!(l.focused(), d);
        l.next_tab(1);
        assert_eq!(l.focused(), t2);
        l.close(t2);
        assert_eq!(l.tabs.len(), 1);
    }
}
