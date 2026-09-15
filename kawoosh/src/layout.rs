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
        match self {
            Node::Pane(id) if *id == target => {
                *self = Node::Split {
                    dir,
                    ratio: 0.5,
                    a: Box::new(Node::Pane(*id)),
                    b: Box::new(Node::Pane(new)),
                };
                true
            }
            Node::Pane(_) => false,
            Node::Split { a, b, .. } => a.split(target, dir, new) || b.split(target, dir, new),
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

/// What a pane shows. Views outlive panes; a terminal outlives its pane
/// too (milestone 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    Editor(ViewId),
    Terminal(u64),
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
        self.panes.get(&pane).copied()
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
