//! The pane tree, tabs and the dock as plain data (mvp.md Decision 5;
//! kui's splitmux is the reference shape), and the scrolling tab beside
//! the tree (scrolling-tab.md): a strip of columns, each a tree. No kui
//! types: the data says what is where, `panes.rs` draws it, and
//! sessions serialise it.

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

    /// Whether a split of `dir` is anywhere in the tree.
    pub fn has_split(&self, dir: SplitDir) -> bool {
        match self {
            Node::Pane(_) => false,
            Node::Split { dir: d, a, b, .. } => *d == dir || a.has_split(dir) || b.has_split(dir),
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

    /// The share of its split that pane `target` takes: the ratio
    /// when it is the first half, the rest when the second; None for
    /// a pane that is the whole tree.
    pub fn share_of(&self, target: PaneId) -> Option<f32> {
        match self {
            Node::Pane(_) => None,
            Node::Split { ratio, a, b, .. } => {
                if matches!(**a, Node::Pane(id) if id == target) {
                    Some(*ratio)
                } else if matches!(**b, Node::Pane(id) if id == target) {
                    Some(1.0 - *ratio)
                } else {
                    a.share_of(target).or_else(|| b.share_of(target))
                }
            }
        }
    }

    /// Grows pane `target` by `by` (a fraction, negative to shrink)
    /// along `dir`: the nearest split of that direction above it moves
    /// its ratio the target's way, clamped as a drag is. False when
    /// no split of the direction holds it — a lone pane, or a pane
    /// beside nothing in that axis.
    pub fn resize(&mut self, target: PaneId, dir: SplitDir, by: f32) -> bool {
        match self {
            Node::Pane(_) => false,
            Node::Split {
                dir: d,
                ratio,
                a,
                b,
            } => {
                // The nearest split first: one deeper than this wins.
                if a.contains(target) {
                    if a.resize(target, dir, by) {
                        return true;
                    }
                    if *d == dir {
                        *ratio = (*ratio + by).clamp(0.1, 0.9);
                        return true;
                    }
                } else if b.contains(target) {
                    if b.resize(target, dir, by) {
                        return true;
                    }
                    if *d == dir {
                        *ratio = (*ratio - by).clamp(0.1, 0.9);
                        return true;
                    }
                }
                false
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
    /// The working memory: the register's past (`memory.rs`).
    Memory,
}
/// A column's width in a scrolling tab, as a fraction of the viewport
/// (scrolling-tab.md Decision 1): niri's presets, or the fraction a
/// drag left it at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Width {
    Third,
    Half,
    TwoThirds,
    Full,
    Ratio(f32),
}

impl Width {
    pub const PRESETS: [Width; 4] = [Width::Third, Width::Half, Width::TwoThirds, Width::Full];

    pub fn fraction(self) -> f32 {
        match self {
            Width::Third => 1.0 / 3.0,
            Width::Half => 0.5,
            Width::TwoThirds => 2.0 / 3.0,
            Width::Full => 1.0,
            Width::Ratio(r) => r.clamp(0.1, 1.0),
        }
    }

    /// The nearest preset.
    pub fn snap(self) -> Width {
        let f = self.fraction();
        Self::PRESETS
            .into_iter()
            .min_by(|a, b| {
                (a.fraction() - f)
                    .abs()
                    .total_cmp(&(b.fraction() - f).abs())
            })
            .unwrap()
    }

    /// A snap that moves the column less than this much of the
    /// viewport is not a step anyone sees, so the key steps on past it.
    const SEEN: f32 = 0.04;

    /// The next preset up (`<A-S-l>`); `Full` stays. A `Ratio` snaps
    /// to the nearest first: when that alone widens it by a visible
    /// amount, that is the step; else it steps from there.
    pub fn up(self) -> Width {
        let snapped = self.snap();
        if snapped.fraction() > self.fraction() + Self::SEEN {
            return snapped;
        }
        let i = Self::PRESETS.iter().position(|p| *p == snapped).unwrap();
        Self::PRESETS[(i + 1).min(Self::PRESETS.len() - 1)]
    }

    /// The next preset down (`<A-S-h>`); `Third` stays.
    pub fn down(self) -> Width {
        let snapped = self.snap();
        if snapped.fraction() < self.fraction() - Self::SEEN {
            return snapped;
        }
        let i = Self::PRESETS.iter().position(|p| *p == snapped).unwrap();
        Self::PRESETS[i.saturating_sub(1)]
    }

    /// A setting's or a session's spelling: a preset's name, or a
    /// fraction.
    pub fn parse(s: &str) -> Option<Width> {
        match s.trim() {
            "third" => Some(Width::Third),
            "half" => Some(Width::Half),
            "two-thirds" | "two_thirds" | "twothirds" => Some(Width::TwoThirds),
            "full" => Some(Width::Full),
            n => n
                .parse::<f32>()
                .ok()
                .filter(|r| (0.1..=1.0).contains(r))
                .map(Width::Ratio),
        }
    }

    pub fn name(self) -> String {
        match self {
            Width::Third => "third".into(),
            Width::Half => "half".into(),
            Width::TwoThirds => "two-thirds".into(),
            Width::Full => "full".into(),
            Width::Ratio(r) => format!("{r:.3}"),
        }
    }
}

/// One column of a strip: a pane, or a stack of `V` splits — a `Node`,
/// so everything the tree knows works inside it unchanged.
#[derive(Clone, Debug)]
pub struct Column {
    /// Stable across inserts and closes around it, so the drawn column
    /// keeps its key (its slide, its enter and exit) whatever its
    /// panes do.
    pub id: u64,
    pub node: Node,
    pub width: Width,
}

/// The scrolling tab (scrolling-tab.md): columns on a ribbon wider
/// than the window, the viewport following the focus.
#[derive(Clone, Debug, Default)]
pub struct Strip {
    pub columns: Vec<Column>,
}

impl Strip {
    pub fn column_of(&self, pane: PaneId) -> Option<usize> {
        self.columns.iter().position(|c| c.node.contains(pane))
    }

    /// The column's left edge on the ribbon, in fractions of the
    /// viewport, gaps not counted.
    pub fn left_of(&self, i: usize) -> f32 {
        self.columns[..i].iter().map(|c| c.width.fraction()).sum()
    }
}

/// What a tab is made of: i3's tree, or niri's strip of columns.
#[derive(Clone, Debug)]
pub enum Kind {
    Tree(Node),
    Scroll(Strip),
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub layout: Kind,
    pub focused: PaneId,
}

impl Tab {
    pub fn tree(root: Node, focused: PaneId) -> Tab {
        Tab {
            layout: Kind::Tree(root),
            focused,
        }
    }

    pub fn is_scroll(&self) -> bool {
        matches!(self.layout, Kind::Scroll(_))
    }

    pub fn strip(&self) -> Option<&Strip> {
        match &self.layout {
            Kind::Scroll(s) => Some(s),
            Kind::Tree(_) => None,
        }
    }

    pub fn strip_mut(&mut self) -> Option<&mut Strip> {
        match &mut self.layout {
            Kind::Scroll(s) => Some(s),
            Kind::Tree(_) => None,
        }
    }

    /// The tab's panes in reading order: the tree's, or the columns'
    /// left to right.
    pub fn panes(&self, out: &mut Vec<PaneId>) {
        match &self.layout {
            Kind::Tree(n) => n.panes(out),
            Kind::Scroll(s) => s.columns.iter().for_each(|c| c.node.panes(out)),
        }
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        match &self.layout {
            Kind::Tree(n) => n.contains(pane),
            Kind::Scroll(s) => s.column_of(pane).is_some(),
        }
    }

    /// The column `pane` is in, for a strip.
    pub fn column_of(&self, pane: PaneId) -> Option<usize> {
        self.strip().and_then(|s| s.column_of(pane))
    }

    /// The tree that holds `pane`: the root, or its column's node.
    pub fn node_of(&self, pane: PaneId) -> Option<&Node> {
        match &self.layout {
            Kind::Tree(n) => n.contains(pane).then_some(n),
            Kind::Scroll(s) => s.column_of(pane).map(|i| &s.columns[i].node),
        }
    }

    pub fn node_of_mut(&mut self, pane: PaneId) -> Option<&mut Node> {
        match &mut self.layout {
            Kind::Tree(n) => n.contains(pane).then_some(n),
            Kind::Scroll(s) => s.column_of(pane).map(|i| &mut s.columns[i].node),
        }
    }

    /// The path of the split whose leaf `pane` is (`Node::split_of`),
    /// in the tab's grammar: `"ab"` in a tree, `"2/ab"` in the third
    /// column of a strip. None for a pane that is a whole tree or a
    /// whole column.
    pub fn split_of(&self, pane: PaneId) -> Option<String> {
        match &self.layout {
            Kind::Tree(n) => n.split_of(pane),
            Kind::Scroll(s) => {
                let i = s.column_of(pane)?;
                s.columns[i].node.split_of(pane).map(|p| format!("{i}/{p}"))
            }
        }
    }

    /// The split at a path in the tab's grammar.
    pub fn ratio_mut(&mut self, path: &str) -> Option<&mut f32> {
        match &mut self.layout {
            Kind::Tree(n) => n.ratio_mut(path),
            Kind::Scroll(s) => {
                let (i, rest) = path.split_once('/')?;
                s.columns
                    .get_mut(i.parse::<usize>().ok()?)?
                    .node
                    .ratio_mut(rest)
            }
        }
    }

    /// The share of its split `pane` takes (`Node::share_of`); in a
    /// strip, a pane that is a whole column answers its width.
    pub fn share_of(&self, pane: PaneId) -> Option<f32> {
        match &self.layout {
            Kind::Tree(n) => n.share_of(pane),
            Kind::Scroll(s) => {
                let c = &s.columns[s.column_of(pane)?];
                c.node.share_of(pane).or(Some(c.width.fraction()))
            }
        }
    }

    /// `Node::resize` on the tree that holds `pane`. In a strip the
    /// horizontal axis is the column's width, which `Layout::step_width`
    /// moves by preset; this answers false for it.
    pub fn resize(&mut self, pane: PaneId, dir: SplitDir, by: f32) -> bool {
        if self.is_scroll() && dir == SplitDir::H {
            return false;
        }
        self.node_of_mut(pane)
            .is_some_and(|n| n.resize(pane, dir, by))
    }

    /// `:layout scroll` (scrolling-tab.md Decision 5): each arm of the
    /// top-level `H` splits becomes a column, a `V` arm as a stack; an
    /// `H` nested under a `V` cannot be a column's inside, so that arm
    /// is flattened to its leaves. Widths from the ratios, snapped to
    /// the nearest preset. Already a strip: nothing.
    pub fn to_scroll(&mut self, next_id: &mut u64) {
        let Kind::Tree(root) = &self.layout else {
            return;
        };
        fn arms(node: &Node, weight: f32, out: &mut Vec<(Node, f32)>) {
            match node {
                Node::Split {
                    dir: SplitDir::H,
                    ratio,
                    a,
                    b,
                } => {
                    arms(a, weight * ratio, out);
                    arms(b, weight * (1.0 - ratio), out);
                }
                n if n.has_split(SplitDir::H) => {
                    let mut leaves = Vec::new();
                    n.panes(&mut leaves);
                    let each = weight / leaves.len() as f32;
                    out.extend(leaves.into_iter().map(|p| (Node::Pane(p), each)));
                }
                n => out.push((n.clone(), weight)),
            }
        }
        let mut out = Vec::new();
        arms(root, 1.0, &mut out);
        let columns = out
            .into_iter()
            .map(|(node, w)| {
                let id = *next_id;
                *next_id += 1;
                Column {
                    id,
                    node,
                    width: Width::Ratio(w).snap(),
                }
            })
            .collect();
        self.layout = Kind::Scroll(Strip { columns });
    }

    /// `:layout tree`: the columns folded into `H` splits, right-nested,
    /// the ratios from the widths; a column's stack kept as it is.
    /// Already a tree: nothing.
    pub fn to_tree(&mut self) {
        let Kind::Scroll(s) = &self.layout else {
            return;
        };
        fn fold(cols: &[Column]) -> Node {
            match cols {
                [] => Node::Pane(0),
                [one] => one.node.clone(),
                [first, rest @ ..] => {
                    let total: f32 = cols.iter().map(|c| c.width.fraction()).sum();
                    Node::Split {
                        dir: SplitDir::H,
                        ratio: (first.width.fraction() / total).clamp(0.1, 0.9),
                        a: Box::new(first.node.clone()),
                        b: Box::new(fold(rest)),
                    }
                }
            }
        }
        self.layout = Kind::Tree(fold(&s.columns));
    }
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
    /// What a new tab is (`layout.default`).
    pub new_tabs_scroll: bool,
    /// What a new column is given (`layout.column_width`).
    pub column_width: Width,
    next_pane: PaneId,
    /// Columns are numbered apart from panes: a pane's number is what
    /// Lua and the session see, a column's only its drawn key.
    next_column: u64,
}

impl Layout {
    pub fn new(first: Content) -> Self {
        let mut panes = HashMap::new();
        panes.insert(1, first);
        Self {
            tabs: vec![Tab::tree(Node::Pane(1), 1)],
            tab: 0,
            panes,
            dock: None,
            dock_open: false,
            dock_ratio: 0.3,
            dock_focused: false,
            rects: HashMap::new(),
            new_tabs_scroll: false,
            column_width: Width::Half,
            next_pane: 2,
            next_column: 1,
        }
    }

    pub fn new_pane(&mut self, content: Content) -> PaneId {
        let id = self.next_pane;
        self.next_pane += 1;
        self.panes.insert(id, content);
        id
    }

    /// A column with a number of its own, for the drawn column's key.
    pub fn new_column(&mut self, node: Node, width: Width) -> Column {
        let id = self.next_column;
        self.next_column += 1;
        Column { id, node, width }
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
        self.tab().panes(&mut out);
        if let (true, Some(d)) = (self.dock_open, self.dock) {
            out.push(d);
        }
        out
    }

    /// Every pane in any tab or the dock.
    pub fn all_panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        for t in &self.tabs {
            t.panes(&mut out);
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
        if let Some(i) = self.tabs.iter().position(|t| t.contains(pane)) {
            self.tab = i;
            self.tabs[i].focused = pane;
        }
    }

    /// Splits the focused pane; the new pane takes focus. In a strip a
    /// split beside is a new column after the focused one, at the
    /// default width; a split below is a split below inside the column.
    pub fn split(&mut self, dir: SplitDir, content: Content) -> PaneId {
        let new = self.new_pane(content);
        if self.dock_focused && self.dock.is_some() {
            // Splitting the dock puts the new pane in the tab instead.
            self.dock_focused = false;
        }
        let target = self.tab().focused;
        let width = self.column_width;
        match (&self.tab().layout, dir) {
            (Kind::Scroll(s), SplitDir::H) => {
                let at = s
                    .column_of(target)
                    .map(|i| i + 1)
                    .unwrap_or(s.columns.len());
                let col = self.new_column(Node::Pane(new), width);
                self.tab_mut().strip_mut().unwrap().columns.insert(at, col);
            }
            _ => {
                if let Some(n) = self.tab_mut().node_of_mut(target) {
                    n.split(target, dir, new);
                }
            }
        }
        self.tab_mut().focused = new;
        new
    }

    /// Closes a pane. The last pane of the last tab stays. Returns the
    /// content it showed, so the caller can decide what to keep. A
    /// column whose last pane closes goes with it, the focus to the
    /// column before.
    pub fn close(&mut self, pane: PaneId) -> Option<Content> {
        if self.dock == Some(pane) {
            self.dock = None;
            self.dock_open = false;
            self.dock_focused = false;
            return self.panes.remove(&pane);
        }
        let ti = self.tabs.iter().position(|t| t.contains(pane))?;
        let was_focused = self.tabs[ti].focused == pane;
        let mut next_focus = None;
        let empty = match &mut self.tabs[ti].layout {
            Kind::Tree(root) => {
                let r = std::mem::replace(root, Node::Pane(0));
                match r.without(pane) {
                    Some(r) => {
                        *root = r;
                        false
                    }
                    None => true,
                }
            }
            Kind::Scroll(s) => {
                let i = s.column_of(pane).unwrap();
                let node = std::mem::replace(&mut s.columns[i].node, Node::Pane(0));
                match node.without(pane) {
                    Some(n) => {
                        // The keyboard stays in the column.
                        let mut ps = Vec::new();
                        n.panes(&mut ps);
                        next_focus = ps.first().copied();
                        s.columns[i].node = n;
                    }
                    None => {
                        s.columns.remove(i);
                        let before = &s.columns[..i];
                        let mut ps = Vec::new();
                        // The column before, else the one that took its
                        // place.
                        if let Some(c) = before.last().or(s.columns.first()) {
                            c.node.panes(&mut ps);
                        }
                        next_focus = ps.first().copied();
                    }
                }
                s.columns.is_empty()
            }
        };
        if !empty {
            if was_focused {
                let mut ps = Vec::new();
                self.tabs[ti].panes(&mut ps);
                self.tabs[ti].focused = next_focus.unwrap_or(ps[0]);
            }
        } else if self.tabs.len() > 1 {
            self.tabs.remove(ti);
            if self.tab >= self.tabs.len() {
                self.tab = self.tabs.len() - 1;
            }
        } else {
            // The last pane of the last tab: keep it.
            let width = self.column_width;
            match &mut self.tabs[ti].layout {
                Kind::Tree(root) => *root = Node::Pane(pane),
                Kind::Scroll(_) => {
                    let col = self.new_column(Node::Pane(pane), width);
                    self.tabs[ti].strip_mut().unwrap().columns.push(col);
                }
            }
            return None;
        }
        self.panes.remove(&pane)
    }

    /// Closes every other pane in the tab.
    pub fn only(&mut self) -> Vec<Content> {
        let keep = self.tab().focused;
        let mut gone = Vec::new();
        let mut ps = Vec::new();
        self.tab().panes(&mut ps);
        for p in ps {
            if p != keep {
                gone.extend(self.close(p));
            }
        }
        gone
    }

    pub fn new_tab(&mut self, content: Content) -> PaneId {
        let p = self.new_pane(content);
        let width = self.column_width;
        let layout = if self.new_tabs_scroll {
            let col = self.new_column(Node::Pane(p), width);
            Kind::Scroll(Strip { columns: vec![col] })
        } else {
            Kind::Tree(Node::Pane(p))
        };
        self.tabs.push(Tab { layout, focused: p });
        self.tab = self.tabs.len() - 1;
        self.dock_focused = false;
        p
    }

    /// The share of the width a pane was opened with — `view_open`'s
    /// `share`, the undo panel's, the memory pane's: the ratio of the
    /// split it sits in for a tree, the width of its column for a
    /// strip, where a panel is a column like any other. `share` is the
    /// pane's own share, not what it leaves behind.
    pub fn set_share(&mut self, pane: PaneId, share: f32) {
        let share = share.clamp(0.1, 0.9);
        // A pane in a split — a tree's, or a column's own stack — takes
        // its share of that split: a picker opened below is the same
        // pane under the same buffer in either kind.
        if let Some(path) = self.tab().split_of(pane)
            && let Some(r) = self.tab_mut().ratio_mut(&path)
        {
            *r = 1.0 - share;
            return;
        }
        // A pane that is a whole column takes its share of the
        // viewport, which is what a column's width is — and the column
        // it opened beside gives up what it must for the two to be on
        // screen together, since that is what asking for a share of
        // the width means. An ordinary `<C-w>v` asks for no share and
        // pushes the ribbon instead, as Decision 3 says.
        if let Some(s) = self.tab_mut().strip_mut()
            && let Some(i) = s.column_of(pane)
        {
            s.columns[i].width = Width::Ratio(share);
            if let Some(before) = i.checked_sub(1)
                && s.columns[before].width.fraction() + share > 1.0
            {
                s.columns[before].width = Width::Ratio(1.0 - share);
            }
        }
    }

    /// `layout.default` applied to the tabs the app starts with, once:
    /// a tab that is still a lone pane in a tree becomes a strip of
    /// one column, so `layout.default = scroll` is what the window
    /// opens as and not only what `:tabnew` makes. A tab a session
    /// brought back with splits keeps the kind the file gave it.
    pub fn apply_default_kind(&mut self) {
        if !self.new_tabs_scroll {
            return;
        }
        let mut next = self.next_column;
        for t in &mut self.tabs {
            if matches!(&t.layout, Kind::Tree(Node::Pane(_))) {
                t.to_scroll(&mut next);
            }
        }
        self.next_column = next;
    }

    /// `:layout scroll` / `:layout tree` on the current tab.
    pub fn set_scroll(&mut self, scroll: bool) {
        let mut next = self.next_column;
        let t = self.tab_mut();
        if scroll {
            t.to_scroll(&mut next);
        } else {
            t.to_tree();
        }
        self.next_column = next;
    }

    pub fn next_tab(&mut self, by: i64) {
        let n = self.tabs.len() as i64;
        self.tab = ((self.tab as i64 + by).rem_euclid(n)) as usize;
        self.dock_focused = false;
    }

    /// The current tab moved to position `to` in the strip (clamped),
    /// the others shifting to make room; the keyboard stays on it.
    /// Returns where it landed.
    pub fn move_tab_to(&mut self, to: usize) -> usize {
        let to = to.min(self.tabs.len().saturating_sub(1));
        if to != self.tab {
            let t = self.tabs.remove(self.tab);
            self.tabs.insert(to, t);
            self.tab = to;
        }
        self.tab
    }

    /// The tab's other pane nearest in `dir` from the focused one, by
    /// last frame's rects. On a strip's own axis — beside, from a pane
    /// in a column — it is the column before or after by index, since
    /// that column may be off the viewport: the pane in it at the
    /// focused pane's row, else its top (scrolling-tab.md Decision 6).
    pub fn neighbour(&self, dir: SplitDir, forward: bool) -> Option<PaneId> {
        let from = self.focused();
        if dir == SplitDir::H
            && self.dock != Some(from)
            && let Some(s) = self.tab().strip()
        {
            let i = s.column_of(from)?;
            let j = if forward { i + 1 } else { i.checked_sub(1)? };
            return self.pane_in_column(j);
        }
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

    /// The pane the keyboard takes when it lands in column `i`: the one
    /// at the focused pane's row, else the column's top
    /// (scrolling-tab.md Decision 2).
    pub fn pane_in_column(&self, i: usize) -> Option<PaneId> {
        let col = self.tab().strip()?.columns.get(i)?;
        let mut ps = Vec::new();
        col.node.panes(&mut ps);
        let cy = self.rects.get(&self.tab().focused).map(|r| r.y + r.h / 2.0);
        ps.iter()
            .find(|p| {
                cy.is_some_and(|cy| {
                    self.rects
                        .get(p)
                        .is_some_and(|q| q.y <= cy && cy < q.y + q.h)
                })
            })
            .or(ps.first())
            .copied()
    }

    /// The Nth thing the keyboard can go to, one-based, clamped to the
    /// last: a strip's Nth column (`<C-3>`), a tree's Nth pane in
    /// reading order. Returns the pane focused, or None for an empty
    /// tab.
    pub fn goto_nth(&mut self, n: usize) -> Option<PaneId> {
        let n = n.max(1) - 1;
        let pane = match self.tab().strip() {
            Some(s) => {
                let i = n.min(s.columns.len().saturating_sub(1));
                self.pane_in_column(i)?
            }
            None => {
                let mut ps = Vec::new();
                self.tab().panes(&mut ps);
                *ps.get(n).or(ps.last())?
            }
        };
        self.focus(pane);
        Some(pane)
    }

    /// The focused pane out of its column's stack and into a column of
    /// its own, after it (`<C-w>e`), at `layout.column_width`; the
    /// keyboard goes with it. None when the tab is a tree, or the pane
    /// is already a whole column — there is nothing to leave.
    pub fn expel(&mut self) -> Option<usize> {
        let focused = self.tab().focused;
        let width = self.column_width;
        let s = self.tab_mut().strip_mut()?;
        let i = s.column_of(focused)?;
        let mut ps = Vec::new();
        s.columns[i].node.panes(&mut ps);
        if ps.len() < 2 {
            return None;
        }
        let node = std::mem::replace(&mut s.columns[i].node, Node::Pane(0));
        s.columns[i].node = node.without(focused)?;
        let col = self.new_column(Node::Pane(focused), width);
        let s = self.tab_mut().strip_mut()?;
        s.columns.insert(i + 1, col);
        Some(i + 1)
    }

    /// The focused pane one place up or down inside its column
    /// (`<A-S-k>` / `<A-S-j>` in a strip): the two panes trade places,
    /// the stack's splits as they were. False in a tree, or at the
    /// stack's end.
    pub fn move_in_column(&mut self, down: bool) -> bool {
        let focused = self.tab().focused;
        let Some(s) = self.tab_mut().strip_mut() else {
            return false;
        };
        let Some(i) = s.column_of(focused) else {
            return false;
        };
        let node = &mut s.columns[i].node;
        let mut ps = Vec::new();
        node.panes(&mut ps);
        let Some(at) = ps.iter().position(|p| *p == focused) else {
            return false;
        };
        let to = if down { at + 1 } else { at.wrapping_sub(1) };
        let Some(other) = ps.get(to).copied() else {
            return false;
        };
        node.swap(focused, other);
        true
    }

    /// The tab's pane under a point (never the dock) and where a drop
    /// there would land, by last frame's rects.
    pub fn drop_at(&self, x: f32, y: f32) -> Option<(PaneId, Drop)> {
        let mut ps = Vec::new();
        self.tab().panes(&mut ps);
        ps.into_iter().find_map(|p| {
            let r = self.rects.get(&p)?;
            let inside = x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
            inside.then(|| (p, Drop::in_rect(r, x, y)))
        })
    }

    /// Moves `pane` onto `target` in the tab: a `Swap` trades their
    /// places, a side takes `pane` out of its split and puts it beside
    /// `target` in a new one — in a strip, a new column beside the
    /// target's for `Left` / `Right`, a split inside the target's
    /// column for `Up` / `Down`. Nothing when either is the dock, not
    /// in the tab, or the same pane. The moved pane keeps the keyboard.
    pub fn move_pane(&mut self, pane: PaneId, target: PaneId, at: Drop) -> bool {
        if pane == target || self.dock == Some(pane) || self.dock == Some(target) {
            return false;
        }
        if !self.tab().contains(pane) || !self.tab().contains(target) {
            return false;
        }
        let width = self.column_width;
        match (&mut self.tabs[self.tab].layout, at) {
            (Kind::Tree(root), Drop::Swap) => root.swap(pane, target),
            (Kind::Scroll(s), Drop::Swap) => {
                // Each column's tree turns its own leaf into the other.
                let (i, j) = (s.column_of(pane).unwrap(), s.column_of(target).unwrap());
                s.columns[i].node.swap(pane, target);
                if j != i {
                    s.columns[j].node.swap(pane, target);
                }
            }
            (Kind::Tree(root), side) => {
                let r = std::mem::replace(root, Node::Pane(0));
                // `target` stays, so the tree is never empty.
                let mut r = r.without(pane).unwrap_or(Node::Pane(target));
                let (dir, before) = side_of(side);
                r.split_beside(target, dir, pane, before);
                *root = r;
            }
            (Kind::Scroll(s), side) => {
                // Out of its column — the column goes when it was the
                // whole of it, its width travelling with the pane.
                let i = s.column_of(pane).unwrap();
                let node = std::mem::replace(&mut s.columns[i].node, Node::Pane(0));
                let own_width = match node.without(pane) {
                    Some(n) => {
                        s.columns[i].node = n;
                        None
                    }
                    None => Some(s.columns.remove(i).width),
                };
                let j = s.column_of(target).unwrap();
                let (dir, before) = side_of(side);
                if dir == SplitDir::H {
                    let at = if before { j } else { j + 1 };
                    let id = self.next_column;
                    self.next_column += 1;
                    s.columns.insert(
                        at,
                        Column {
                            id,
                            node: Node::Pane(pane),
                            width: own_width.unwrap_or(width),
                        },
                    );
                } else {
                    s.columns[j]
                        .node
                        .split_beside(target, SplitDir::V, pane, before);
                }
            }
        }
        self.tabs[self.tab].focused = pane;
        self.dock_focused = false;
        true
    }

    /// The focused pane's column one place left or right (`<C-w>H`,
    /// `<C-w>L`), COUNT places; false in a tree, or at the strip's end.
    pub fn move_column(&mut self, by: i64) -> bool {
        let focused = self.tab().focused;
        let Some(s) = self.tab_mut().strip_mut() else {
            return false;
        };
        let Some(i) = s.column_of(focused) else {
            return false;
        };
        let to = (i as i64 + by).clamp(0, s.columns.len() as i64 - 1) as usize;
        if to == i {
            return false;
        }
        let c = s.columns.remove(i);
        s.columns.insert(to, c);
        true
    }

    /// The focused pane's column to the next preset up or down
    /// (`<A-S-l>`, `<A-S-h>` in a strip); false in a tree, or when the
    /// preset is the last one that way.
    pub fn step_width(&mut self, up: bool) -> bool {
        let focused = self.tab().focused;
        let Some(s) = self.tab_mut().strip_mut() else {
            return false;
        };
        let Some(i) = s.column_of(focused) else {
            return false;
        };
        let w = s.columns[i].width;
        let next = if up { w.up() } else { w.down() };
        if next == w {
            return false;
        }
        s.columns[i].width = next;
        true
    }

    /// The next pane in tree order (`<C-w>w`).
    pub fn next_pane(&self) -> PaneId {
        let ps = self.visible_panes();
        let i = ps.iter().position(|p| *p == self.focused()).unwrap_or(0);
        ps[(i + 1) % ps.len()]
    }
}

fn side_of(side: Drop) -> (SplitDir, bool) {
    match side {
        Drop::Left => (SplitDir::H, true),
        Drop::Right => (SplitDir::H, false),
        Drop::Up => (SplitDir::V, true),
        _ => (SplitDir::V, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> Content {
        Content::Terminal(0)
    }

    fn cols(l: &Layout) -> Vec<(Vec<PaneId>, Width)> {
        l.tab()
            .strip()
            .unwrap()
            .columns
            .iter()
            .map(|c| {
                let mut ps = Vec::new();
                c.node.panes(&mut ps);
                (ps, c.width)
            })
            .collect()
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
        assert_eq!(l.tab().split_of(1), None, "the root is no split");
        let b = l.split(SplitDir::H, view());
        assert_eq!(l.tab().split_of(1).as_deref(), Some(""));
        assert_eq!(l.tab().split_of(b).as_deref(), Some(""));
        let c = l.split(SplitDir::V, view());
        assert_eq!(l.tab().split_of(c).as_deref(), Some("b"));
        assert_eq!(l.tab().split_of(b).as_deref(), Some("b"));
        assert_eq!(l.tab().split_of(1).as_deref(), Some(""));
        assert_eq!(l.tab().split_of(99), None);
        *l.tab_mut().ratio_mut("b").unwrap() = 0.25;
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
        assert_eq!(l.tab().node_of(1).unwrap().depth(), 2);
        // Beside: out of its split, into a new one on the side asked.
        assert!(l.move_pane(1, c, Drop::Left));
        assert_eq!(l.visible_panes(), [1, c, b]);
        assert_eq!(l.tab().split_of(1).as_deref(), Some("a"));
        assert!(matches!(
            l.tab().layout,
            Kind::Tree(Node::Split { dir: SplitDir::H, ref a, .. })
                if matches!(**a, Node::Split { dir: SplitDir::H, .. })
        ));
        assert!(l.move_pane(b, 1, Drop::Up));
        assert_eq!(l.visible_panes(), [b, 1, c]);
        assert_eq!(
            l.tab().node_of(1).unwrap().depth(),
            2,
            "the split b left collapsed"
        );
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

    #[test]
    fn a_strip_inserts_closes_moves_and_sizes_its_columns() {
        let mut l = Layout::new(view());
        l.set_scroll(true);
        assert_eq!(cols(&l), [(vec![1], Width::Full)], "a lone pane fills");
        // Beside: a column after the focused one at the default width.
        let b = l.split(SplitDir::H, view());
        let c = l.split(SplitDir::H, view());
        assert_eq!(l.focused(), c);
        // Below: a stack inside the column.
        let d = l.split(SplitDir::V, view());
        assert_eq!(
            cols(&l),
            [
                (vec![1], Width::Full),
                (vec![b], Width::Half),
                (vec![c, d], Width::Half)
            ]
        );
        assert_eq!(l.visible_panes(), [1, b, c, d]);
        assert_eq!(l.tab().split_of(d).as_deref(), Some("2/"));
        assert_eq!(l.tab().split_of(b), None, "a whole column");
        *l.tab_mut().ratio_mut("2/").unwrap() = 0.3;
        assert_eq!(l.tab().share_of(c), Some(0.3));
        assert_eq!(l.tab().share_of(b), Some(0.5), "a whole column's width");
        // Beside from the middle: the column lands between.
        l.focus(b);
        let e = l.split(SplitDir::H, view());
        assert_eq!(l.visible_panes(), [1, b, e, c, d]);
        // The column moves along the strip, the keyboard on it.
        assert!(l.move_column(-1));
        assert_eq!(l.visible_panes(), [1, e, b, c, d]);
        assert!(l.move_column(-1));
        assert_eq!(l.visible_panes(), [e, 1, b, c, d]);
        assert!(!l.move_column(-1), "the strip's edge");
        assert_eq!(l.focused(), e);
        // The width steps through the presets and stops at the ends.
        assert!(l.step_width(true));
        assert_eq!(cols(&l)[0].1, Width::TwoThirds);
        assert!(l.step_width(true));
        assert!(!l.step_width(true));
        assert_eq!(cols(&l)[0].1, Width::Full);
        l.tab_mut().strip_mut().unwrap().columns[0].width = Width::Ratio(0.48);
        assert!(l.step_width(false));
        assert_eq!(cols(&l)[0].1, Width::Third, "a ratio snaps, then steps");
        l.tab_mut().strip_mut().unwrap().columns[0].width = Width::Ratio(0.58);
        assert!(l.step_width(true));
        assert_eq!(
            cols(&l)[0].1,
            Width::TwoThirds,
            "a snap seen to widen is the step"
        );
        l.tab_mut().strip_mut().unwrap().columns[0].width = Width::Ratio(0.65);
        assert!(l.step_width(true));
        assert_eq!(cols(&l)[0].1, Width::Full, "a snap not seen is not");
        assert!(
            !l.tab_mut().resize(e, SplitDir::H, 0.05),
            "the axis is the width's"
        );
        assert!(
            l.tab_mut().resize(c, SplitDir::V, 0.05),
            "inside the column, the tree's"
        );
        // Closing a column's last pane takes the column, the focus to
        // the one before; closing inside a stack keeps the column.
        l.focus(e);
        l.close(e);
        assert_eq!(l.visible_panes(), [1, b, c, d]);
        assert_eq!(l.focused(), 1, "the first column took the place");
        l.focus(c);
        l.close(c);
        assert_eq!(l.visible_panes(), [1, b, d]);
        assert_eq!(l.focused(), d);
        l.close(d);
        assert_eq!(l.visible_panes(), [1, b]);
        assert_eq!(l.focused(), b, "the column before");
        l.close(b);
        assert!(l.close(1).is_none(), "the last pane stays, as a column");
        assert_eq!(cols(&l).len(), 1);
    }

    #[test]
    fn a_pane_moves_across_a_strip() {
        let mut l = Layout::new(view());
        l.set_scroll(true);
        let b = l.split(SplitDir::H, view());
        let c = l.split(SplitDir::V, view());
        // 1 | b over c. A swap across columns trades the leaves.
        assert!(l.move_pane(1, c, Drop::Swap));
        assert_eq!(l.visible_panes(), [c, b, 1]);
        assert_eq!(l.focused(), 1);
        // Beside: a new column, before or after the target's; a whole
        // column moved keeps its width.
        l.tab_mut().strip_mut().unwrap().columns[0].width = Width::Third;
        assert!(l.move_pane(c, 1, Drop::Right));
        assert_eq!(
            cols(&l).iter().map(|c| c.0.clone()).collect::<Vec<_>>(),
            [vec![b, 1], vec![c]]
        );
        assert_eq!(cols(&l)[1].1, Width::Third);
        // Up: into the target's stack; the emptied column goes.
        assert!(l.move_pane(c, b, Drop::Up));
        assert_eq!(
            cols(&l).iter().map(|c| c.0.clone()).collect::<Vec<_>>(),
            [vec![c, b, 1]]
        );
        assert!(l.move_pane(b, c, Drop::Left));
        assert_eq!(
            cols(&l).iter().map(|c| c.0.clone()).collect::<Vec<_>>(),
            [vec![b], vec![c, 1]]
        );
        assert_eq!(
            cols(&l)[0].1,
            Width::Half,
            "a pane out of a stack takes the default"
        );
    }

    #[test]
    fn a_tree_converts_both_ways() {
        // 1 | (b over c): two columns at a half each.
        let mut l = Layout::new(view());
        let b = l.split(SplitDir::H, view());
        let c = l.split(SplitDir::V, view());
        let before = format!("{:?}", l.tab().layout);
        l.set_scroll(true);
        assert_eq!(
            cols(&l),
            [(vec![1], Width::Half), (vec![b, c], Width::Half)]
        );
        l.set_scroll(true);
        assert_eq!(cols(&l).len(), 2, "already a strip: nothing");
        l.set_scroll(false);
        assert_eq!(format!("{:?}", l.tab().layout), before, "the round trip");
        // Three across: thirds, the nested ratio scaled by the outer.
        l.focus(1);
        let d = l.split(SplitDir::H, view());
        *l.tab_mut().ratio_mut("").unwrap() = 2.0 / 3.0;
        l.set_scroll(true);
        assert_eq!(
            cols(&l),
            [
                (vec![1], Width::Third),
                (vec![d], Width::Third),
                (vec![b, c], Width::Third)
            ]
        );
        l.set_scroll(false);
        assert!(matches!(
            l.tab().layout,
            Kind::Tree(Node::Split { dir: SplitDir::H, ratio, .. }) if (ratio - 1.0 / 3.0).abs() < 1e-3
        ));
        // An H under a V cannot be a column's inside: the arm flattens
        // to its leaves, consecutive columns in reading order.
        let mut l = Layout::new(view());
        let b = l.split(SplitDir::V, view());
        let c = l.split(SplitDir::H, view());
        l.focus(1);
        let d = l.split(SplitDir::H, view());
        // (1 | d) over (b | c)
        l.set_scroll(true);
        assert_eq!(
            cols(&l).iter().map(|c| c.0.clone()).collect::<Vec<_>>(),
            [vec![1], vec![d], vec![b], vec![c]]
        );
        assert!(
            cols(&l).iter().all(|c| c.1 == Width::Third),
            "a quarter snaps to a third"
        );
    }
}
