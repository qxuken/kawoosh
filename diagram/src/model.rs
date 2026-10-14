//! What a `.kdraw` file holds once read (docs/design/diagrams.md
//! Decision 1): its items, each with the line and the bytes it came
//! from, and what could not be read.

use std::ops::Range;

/// A box's outline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shape {
    #[default]
    Rect,
    Round,
    Ellipse,
    Diamond,
    Cylinder,
    /// A state machine's start: a filled circle.
    Dot,
    /// A state machine's end: a ring round a dot.
    Final,
}

impl Shape {
    pub const ALL: [Shape; 7] = [
        Shape::Rect,
        Shape::Round,
        Shape::Ellipse,
        Shape::Diamond,
        Shape::Cylinder,
        Shape::Dot,
        Shape::Final,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Shape::Rect => "rect",
            Shape::Round => "round",
            Shape::Ellipse => "ellipse",
            Shape::Diamond => "diamond",
            Shape::Cylinder => "cylinder",
            Shape::Dot => "dot",
            Shape::Final => "final",
        }
    }

    pub fn from_word(w: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|s| s.word() == w)
    }

    /// The one after it, round: what `s` on the canvas steps to.
    pub fn next(self) -> Shape {
        let i = Shape::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Shape::ALL[(i + 1) % Shape::ALL.len()]
    }

    /// Whether its outline is round: an ellipse's equation, not a box's.
    pub fn is_round(self) -> bool {
        matches!(self, Shape::Ellipse | Shape::Dot | Shape::Final)
    }

    /// The size a box of it has when none is said.
    pub fn default_size(self) -> (i32, i32) {
        match self {
            Shape::Dot | Shape::Final => (1, 1),
            _ => (8, 3),
        }
    }
}

/// A colour a line names; the canvas takes each from the theme, the
/// export from Excalidraw's palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Purple,
    Gray,
}

impl Color {
    pub const ALL: [Color; 8] = [
        Color::Red,
        Color::Orange,
        Color::Yellow,
        Color::Green,
        Color::Teal,
        Color::Blue,
        Color::Purple,
        Color::Gray,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Color::Red => "red",
            Color::Orange => "orange",
            Color::Yellow => "yellow",
            Color::Green => "green",
            Color::Teal => "teal",
            Color::Blue => "blue",
            Color::Purple => "purple",
            Color::Gray => "gray",
        }
    }

    pub fn from_word(w: &str) -> Option<Color> {
        Color::ALL.into_iter().find(|c| c.word() == w)
    }

    /// The one after `c`, round through none: what `c` on the canvas
    /// steps to.
    pub fn next(c: Option<Color>) -> Option<Color> {
        match c {
            None => Some(Color::ALL[0]),
            Some(c) => {
                let i = Color::ALL.iter().position(|x| *x == c).unwrap_or(0);
                Color::ALL.get(i + 1).copied()
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub shape: Shape,
    pub color: Option<Color>,
    /// Its outline dashed: something outside, planned, optional.
    pub dashed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub color: Option<Color>,
}

/// Which ends of an arrow have a head: `->`, `<-`, `<->`, `--`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Heads {
    #[default]
    End,
    Start,
    Both,
    None,
}

impl Heads {
    pub fn token(self) -> &'static str {
        match self {
            Heads::End => "->",
            Heads::Start => "<-",
            Heads::Both => "<->",
            Heads::None => "--",
        }
    }

    pub fn from_token(t: &str) -> Option<Heads> {
        [Heads::End, Heads::Start, Heads::Both, Heads::None]
            .into_iter()
            .find(|h| h.token() == t)
    }

    pub fn at_start(self) -> bool {
        matches!(self, Heads::Start | Heads::Both)
    }

    pub fn at_end(self) -> bool {
        matches!(self, Heads::End | Heads::Both)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Route {
    #[default]
    Straight,
    Elbow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: String,
    pub heads: Heads,
    pub route: Route,
    pub dashed: bool,
    pub color: Option<Color>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: String,
    pub label: String,
    pub members: Vec<String>,
    pub color: Option<Color>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Node(Node),
    Text(Text),
    Edge(Edge),
    Group(Group),
}

impl Item {
    /// The id other lines name it by: a box's, a group's.
    pub fn id(&self) -> Option<&str> {
        match self {
            Item::Node(n) => Some(&n.id),
            Item::Group(g) => Some(&g.id),
            _ => None,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Item::Node(n) => &n.label,
            Item::Text(t) => &t.label,
            Item::Edge(e) => &e.label,
            Item::Group(g) => &g.label,
        }
    }

    pub fn label_mut(&mut self) -> &mut String {
        match self {
            Item::Node(n) => &mut n.label,
            Item::Text(t) => &mut t.label,
            Item::Edge(e) => &mut e.label,
            Item::Group(g) => &mut g.label,
        }
    }

    pub fn color(&self) -> Option<Color> {
        match self {
            Item::Node(n) => n.color,
            Item::Text(t) => t.color,
            Item::Edge(e) => e.color,
            Item::Group(g) => g.color,
        }
    }

    pub fn color_mut(&mut self) -> &mut Option<Color> {
        match self {
            Item::Node(n) => &mut n.color,
            Item::Text(t) => &mut t.color,
            Item::Edge(e) => &mut e.color,
            Item::Group(g) => &mut g.color,
        }
    }
}

/// An item, the line it is on (from 0) and that line's bytes, its
/// newline not among them.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub item: Item,
    pub line: usize,
    pub span: Range<usize>,
}

/// A line that could not be read, and why: drawn over the canvas, its
/// line left as it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub line: usize,
    pub message: String,
}

/// A file read: its items in the file's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doc {
    pub entries: Vec<Entry>,
    pub problems: Vec<Problem>,
    /// The text's length, where a new line goes.
    pub len: usize,
    /// Whether the text ends in a newline, so one appended needs none
    /// before it.
    pub ends_in_newline: bool,
}

impl Doc {
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.entries.iter().map(|e| &e.item)
    }

    /// The entry of the box named `id`.
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| matches!(&e.item, Item::Node(n) if n.id == id))
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.node_index(id)
            .and_then(|i| match &self.entries[i].item {
                Item::Node(n) => Some(n),
                _ => None,
            })
    }

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.entries.iter().find_map(|e| match &e.item {
            Item::Group(g) if g.id == id => Some(g),
            _ => None,
        })
    }

    /// Whether `id` names a box or a group.
    pub fn has_id(&self, id: &str) -> bool {
        self.entries.iter().any(|e| e.item.id() == Some(id))
    }

    /// An id no item has: `prefix` and the first number free.
    pub fn fresh_id(&self, prefix: &str) -> String {
        self.fresh_id_avoiding(prefix, &[])
    }

    /// [`Self::fresh_id`], none of `taken` either.
    pub fn fresh_id_avoiding(&self, prefix: &str, taken: &[String]) -> String {
        (1..)
            .map(|n| format!("{prefix}{n}"))
            .find(|id| !self.has_id(id) && !taken.iter().any(|t| t == id))
            .unwrap()
    }

    /// The boxes a group holds, through the groups among its members,
    /// each once.
    pub fn group_nodes(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = Vec::new();
        self.collect_nodes(id, &mut out, &mut seen);
        out
    }

    fn collect_nodes(&self, id: &str, out: &mut Vec<String>, seen: &mut Vec<String>) {
        if seen.iter().any(|s| s == id) {
            return;
        }
        seen.push(id.to_string());
        let Some(g) = self.group(id) else {
            return;
        };
        for m in &g.members {
            if self.node(m).is_some() {
                if !out.contains(m) {
                    out.push(m.clone());
                }
            } else {
                self.collect_nodes(m, out, seen);
            }
        }
    }
}
