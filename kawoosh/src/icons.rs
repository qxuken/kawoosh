//! The icon set and the key caps (docs/design/icons.md): every
//! picture-like element — a close button, a fold, a done mark, a key —
//! drawn with kui's vectors in a box of its own instead of set as a
//! glyph, which sits wherever its face's metrics put it.
//!
//! An icon is data, [`Part`]s in a box of side 1, `y` down; one
//! function ([`resolve`]) turns a shape at a size into what is drawn,
//! and both the chrome ([`icon`]) and a Lua view (`kawoosh.icon`, built
//! by [`lua_door`]) draw that. The set is one, shared: a user's
//! `kawoosh.icons.close = { … }` is the tab's close button too.
//!
//! Keys are caps: [`caps`] reads the keymap's notation (`<C-w>j`) into
//! one cap a key, its modifiers and named keys as icons, and [`keys`]
//! / [`legend_items`] draw them; boot.lua's `ctx.keys` and `ctx.legend`
//! draw the same from Lua with [`CAP`]'s measures.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use kawoosh_editor::keymap::{LEADER, parse_notation};
use kui_native::{Align, Color, NodeSpec, Stroke, TextStyle, Ui, Vec2};

/// One piece of an icon, in a box of side 1, `y` down.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// A round-capped line through the points; `width` a share of the
    /// side (1 px at least); `curve` a smooth curve through them.
    Stroke {
        points: Vec<[f32; 2]>,
        width: f32,
        curve: bool,
    },
    /// A filled outline, up to eight points (kui's polygon).
    Fill { points: Vec<[f32; 2]> },
    /// A disc of radius `r`, centred.
    Dot { r: f32 },
    /// A character centred in the box at `scale` of its side: the way
    /// out to a font's picture (a Nerd Font's).
    Glyph { text: String, scale: f32 },
}

pub type Shape = Vec<Part>;

/// A stroke's width, as a share of the side, unless a part says.
pub const STROKE: f32 = 0.1;
/// The most points kui's polygon takes.
pub const FILL_MAX: usize = 8;

/// The set's names, in the order `kawoosh.icon_names()` lists them.
pub const NAMES: &[&str] = &[
    "close",
    "check",
    "dot",
    "chevron-right",
    "chevron-left",
    "chevron-up",
    "chevron-down",
    "folded",
    "unfolded",
    "arrow-up",
    "arrow-down",
    "arrow-left",
    "arrow-right",
    "return",
    "tab",
    "backspace",
    "delete",
    "ctrl",
    "alt",
    "shift",
    "cmd",
    "missing",
];

fn line(points: &[[f32; 2]]) -> Part {
    Part::Stroke {
        points: points.to_vec(),
        width: STROKE,
        curve: false,
    }
}

fn fill(points: &[[f32; 2]]) -> Part {
    Part::Fill {
        points: points.to_vec(),
    }
}

/// A ring of twelve sides about `c`: a loop of ⌘, as round as a
/// stroke this small shows.
fn ring(c: [f32; 2], r: f32) -> Part {
    let pts: Vec<[f32; 2]> = (0..=12)
        .map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 12.0;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect();
    line(&pts)
}

/// The shipped shape of `name`, or none for a name the set lacks.
pub fn default_shape(name: &str) -> Option<Shape> {
    Some(match name {
        "close" => vec![
            line(&[[0.3, 0.3], [0.7, 0.7]]),
            line(&[[0.3, 0.7], [0.7, 0.3]]),
        ],
        "check" => vec![line(&[[0.22, 0.52], [0.42, 0.72], [0.78, 0.32]])],
        "dot" => vec![Part::Dot { r: 0.22 }],
        "chevron-right" => vec![line(&[[0.4, 0.25], [0.62, 0.5], [0.4, 0.75]])],
        "chevron-left" => vec![line(&[[0.6, 0.25], [0.38, 0.5], [0.6, 0.75]])],
        "chevron-up" => vec![line(&[[0.25, 0.6], [0.5, 0.38], [0.75, 0.6]])],
        "chevron-down" => vec![line(&[[0.25, 0.4], [0.5, 0.62], [0.75, 0.4]])],
        "folded" => vec![fill(&[[0.36, 0.24], [0.7, 0.5], [0.36, 0.76]])],
        "unfolded" => vec![fill(&[[0.24, 0.36], [0.76, 0.36], [0.5, 0.7]])],
        "arrow-up" => vec![
            line(&[[0.5, 0.82], [0.5, 0.2]]),
            line(&[[0.27, 0.43], [0.5, 0.2], [0.73, 0.43]]),
        ],
        "arrow-down" => vec![
            line(&[[0.5, 0.18], [0.5, 0.8]]),
            line(&[[0.27, 0.57], [0.5, 0.8], [0.73, 0.57]]),
        ],
        "arrow-left" => vec![
            line(&[[0.82, 0.5], [0.2, 0.5]]),
            line(&[[0.43, 0.27], [0.2, 0.5], [0.43, 0.73]]),
        ],
        "arrow-right" => vec![
            line(&[[0.18, 0.5], [0.8, 0.5]]),
            line(&[[0.57, 0.27], [0.8, 0.5], [0.57, 0.73]]),
        ],
        "return" => vec![
            line(&[[0.78, 0.22], [0.78, 0.62], [0.22, 0.62]]),
            line(&[[0.4, 0.44], [0.22, 0.62], [0.4, 0.8]]),
        ],
        "tab" => vec![
            line(&[[0.16, 0.5], [0.72, 0.5]]),
            line(&[[0.52, 0.3], [0.72, 0.5], [0.52, 0.7]]),
            line(&[[0.84, 0.26], [0.84, 0.74]]),
        ],
        "backspace" => vec![
            line(&[
                [0.1, 0.5],
                [0.34, 0.24],
                [0.9, 0.24],
                [0.9, 0.76],
                [0.34, 0.76],
                [0.1, 0.5],
            ]),
            line(&[[0.5, 0.4], [0.7, 0.6]]),
            line(&[[0.5, 0.6], [0.7, 0.4]]),
        ],
        "delete" => vec![
            line(&[
                [0.9, 0.5],
                [0.66, 0.24],
                [0.1, 0.24],
                [0.1, 0.76],
                [0.66, 0.76],
                [0.9, 0.5],
            ]),
            line(&[[0.3, 0.4], [0.5, 0.6]]),
            line(&[[0.3, 0.6], [0.5, 0.4]]),
        ],
        "ctrl" => vec![line(&[[0.26, 0.6], [0.5, 0.34], [0.74, 0.6]])],
        "alt" => vec![
            line(&[[0.14, 0.32], [0.38, 0.32], [0.62, 0.7], [0.86, 0.7]]),
            line(&[[0.6, 0.32], [0.86, 0.32]]),
        ],
        "shift" => vec![line(&[
            [0.5, 0.16],
            [0.84, 0.5],
            [0.66, 0.5],
            [0.66, 0.82],
            [0.34, 0.82],
            [0.34, 0.5],
            [0.16, 0.5],
            [0.5, 0.16],
        ])],
        "cmd" => vec![
            line(&[[0.38, 0.28], [0.38, 0.72]]),
            line(&[[0.62, 0.28], [0.62, 0.72]]),
            line(&[[0.28, 0.38], [0.72, 0.38]]),
            line(&[[0.28, 0.62], [0.72, 0.62]]),
            ring([0.28, 0.28], 0.1),
            ring([0.72, 0.28], 0.1),
            ring([0.28, 0.72], 0.1),
            ring([0.72, 0.72], 0.1),
        ],
        "missing" => vec![line(&[
            [0.22, 0.22],
            [0.78, 0.22],
            [0.78, 0.78],
            [0.22, 0.78],
            [0.22, 0.22],
        ])],
        _ => return None,
    })
}

/// The set: the shipped shapes and the user's over them, and the
/// colour an icon is drawn in when its caller names none.
pub struct Icons {
    own: BTreeMap<String, Shape>,
    /// The theme's foreground, as the frame last set it.
    pub fg: Color,
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            own: BTreeMap::new(),
            fg: crate::palette::Pal::default().fg,
        }
    }
}

pub type Shared = Rc<RefCell<Icons>>;

impl Icons {
    /// `name`'s shape: the user's, else the shipped one, else
    /// `missing`'s, so a name the set lacks is seen.
    pub fn shape(&self, name: &str) -> Shape {
        self.own
            .get(name)
            .cloned()
            .or_else(|| default_shape(name))
            .unwrap_or_else(|| self.shape("missing"))
    }

    /// Whether `name` is in the set, shipped or the user's.
    pub fn has(&self, name: &str) -> bool {
        self.own.contains_key(name) || default_shape(name).is_some()
    }

    /// Replaces `name`'s shape; `None` puts the shipped one back.
    pub fn define(&mut self, name: &str, shape: Option<Shape>) {
        match shape {
            Some(s) => {
                self.own.insert(name.to_string(), s);
            }
            None => {
                self.own.remove(name);
            }
        }
    }

    /// Every name: the shipped ones, then the user's own.
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = NAMES.iter().map(|s| s.to_string()).collect();
        for k in self.own.keys() {
            if !out.contains(k) {
                out.push(k.clone());
            }
        }
        out
    }
}

/// What a shape is drawn as in a box of side `size`, in the box's
/// space: what the chrome draws and what `kawoosh.icon` returns.
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Line {
        points: Vec<Vec2>,
        width: f32,
        curve: bool,
    },
    Poly {
        points: Vec<Vec2>,
    },
    Dot {
        d: f32,
    },
    Glyph {
        text: String,
        size: f32,
    },
}

/// A shape at `size`: points scaled into the box, a stroke 1 px at the
/// least.
pub fn resolve(shape: &Shape, size: f32) -> Vec<Prim> {
    let at = |p: &[f32; 2]| Vec2::new(p[0] * size, p[1] * size);
    shape
        .iter()
        .map(|p| match p {
            Part::Stroke {
                points,
                width,
                curve,
            } => Prim::Line {
                points: points.iter().map(at).collect(),
                width: (width * size).max(1.0),
                curve: *curve,
            },
            Part::Fill { points } => Prim::Poly {
                points: points.iter().take(FILL_MAX).map(at).collect(),
            },
            Part::Dot { r } => Prim::Dot {
                d: (2.0 * r * size).max(1.0),
            },
            Part::Glyph { text, scale } => Prim::Glyph {
                text: text.clone(),
                size: scale * size,
            },
        })
        .collect()
}

/// An icon's box: a square of `size`, what is in flow (a dot, a glyph)
/// centred in it. A caller adds its key, click and label.
pub fn icon_box(size: f32) -> NodeSpec {
    NodeSpec::row()
        .size(size, size)
        .main_align(Align::Center)
        .cross_align(Align::Center)
}

/// Draws `name` into the box open now, which is `size` square.
pub fn draw(ui: &mut Ui<'_>, icons: &Icons, name: &str, size: f32, color: Color) {
    for p in resolve(&icons.shape(name), size) {
        match p {
            Prim::Line {
                points,
                width,
                curve,
            } => {
                let mut s = Stroke::new(width, color);
                if curve {
                    s = s.curve();
                }
                ui.polyline(&points, s, NodeSpec::row());
            }
            Prim::Poly { points } => ui.polygon(&points, NodeSpec::row().bg(color)),
            Prim::Dot { d } => {
                ui.leaf(NodeSpec::row().size(d, d).radius(d / 2.0).bg(color));
            }
            Prim::Glyph { text, size } => {
                ui.text(&text, TextStyle::new(size).color(color).nowrap());
            }
        }
    }
}

/// `name` in a box of its own, `size` square, in the row open now.
pub fn icon(ui: &mut Ui<'_>, icons: &Icons, name: &str, size: f32, color: Color) {
    ui.with(icon_box(size), |ui| draw(ui, icons, name, size, color));
}

// ------------------------------------------------------------------ keys

/// What is on a cap: an icon, or a word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapPart {
    Icon(&'static str),
    Text(String),
}

/// A cap's measures, the Lua half's too (`kawoosh._cap`).
pub struct CapMeasures {
    /// Inside the cap, either side.
    pub pad: f32,
    pub radius: f32,
    pub border: f32,
    /// Between two caps of one sequence.
    pub gap: f32,
    /// Between a cap's modifiers and its key.
    pub part_gap: f32,
    /// Between two sequences of one legend item (`j` `k`).
    pub alt_gap: f32,
    /// Between an item's keys and its words.
    pub word_gap: f32,
    /// Between a legend's items.
    pub item_gap: f32,
}

pub const CAP: CapMeasures = CapMeasures {
    pad: 3.0,
    radius: 3.0,
    border: 1.0,
    gap: 2.0,
    part_gap: 1.0,
    alt_gap: 5.0,
    word_gap: 4.0,
    item_gap: 12.0,
};

pub use kawoosh_editor::host::Host;

/// The caps a notation is drawn as: one a key, a chord's modifiers in
/// it before its key — on a Mac ⌃⌥⇧⌘ as icons, in the Mac's order;
/// elsewhere as that keyboard's words, `ctrl+shift+j`, `win+s`
/// (`super+s` on Linux). A letter keeps its case (`j` and `J` are two
/// keys); a chord's capital is its shift spelled out (`<C-H>` is ⌃⇧h,
/// the which-key's rule).
pub fn caps(notation: &str) -> Vec<Vec<CapPart>> {
    caps_on(notation, Host::HERE)
}

/// [`caps`] as `host` spells them.
pub fn caps_on(notation: &str, host: Host) -> Vec<Vec<CapPart>> {
    parse_notation(notation)
        .iter()
        .map(|k| cap(k, host))
        .collect()
}

fn named(key: &str) -> Option<CapPart> {
    Some(match key {
        "CR" => CapPart::Icon("return"),
        "Tab" => CapPart::Icon("tab"),
        "BS" => CapPart::Icon("backspace"),
        "Del" => CapPart::Icon("delete"),
        "Up" => CapPart::Icon("arrow-up"),
        "Down" => CapPart::Icon("arrow-down"),
        "Left" => CapPart::Icon("arrow-left"),
        "Right" => CapPart::Icon("arrow-right"),
        "Esc" => CapPart::Text("esc".into()),
        "Space" => CapPart::Text("spc".into()),
        "Home" => CapPart::Text("home".into()),
        "End" => CapPart::Text("end".into()),
        "PageUp" => CapPart::Text("pgup".into()),
        "PageDown" => CapPart::Text("pgdn".into()),
        "Insert" => CapPart::Text("ins".into()),
        _ => return None,
    })
}

fn cap(key: &str, host: Host) -> Vec<CapPart> {
    if key == LEADER {
        return vec![CapPart::Text("spc".into())];
    }
    let Some(inner) = key.strip_prefix('<').and_then(|k| k.strip_suffix('>')) else {
        return vec![CapPart::Text(key.to_string())];
    };
    // `<D-->`: the minus key under ⌘.
    let (mods, base) = match inner.strip_suffix("--") {
        Some(m) => (m, "-"),
        None => match inner.rsplit_once('-') {
            Some((m, b)) => (m, b),
            None => ("", inner),
        },
    };
    let has = |m: &str| mods.split('-').any(|x| x == m);
    let capital = base.len() == 1 && base.as_bytes()[0].is_ascii_uppercase();
    let (ctrl, alt) = (has("C"), has("A") || has("M"));
    let (shift, sup) = (has("S") || (capital && !mods.is_empty()), has("D"));
    let base = match named(base) {
        Some(p) => p,
        None if capital && !mods.is_empty() => CapPart::Text(base.to_ascii_lowercase()),
        None => CapPart::Text(base.to_string()),
    };
    let mut out = Vec::new();
    if host == Host::Mac {
        for (held, name) in [(ctrl, "ctrl"), (alt, "alt"), (shift, "shift"), (sup, "cmd")] {
            if held {
                out.push(CapPart::Icon(name));
            }
        }
        out.push(base);
        return out;
    }
    // A PC's order, the system's key first (`win+shift+s`), and one
    // text with the key when the key is text, so the face spaces it.
    let sup_word = if host == Host::Windows {
        "win"
    } else {
        "super"
    };
    let mut words = String::new();
    for (held, word) in [
        (sup, sup_word),
        (ctrl, "ctrl"),
        (alt, "alt"),
        (shift, "shift"),
    ] {
        if held {
            words.push_str(word);
            words.push('+');
        }
    }
    match base {
        CapPart::Text(t) => out.push(CapPart::Text(words + &t)),
        icon => {
            if !words.is_empty() {
                out.push(CapPart::Text(words));
            }
            out.push(icon);
        }
    }
    out
}

/// How caps are drawn: the text of their keys (its size the icons'),
/// and the outline's colour.
#[derive(Clone, Copy)]
pub struct KeyStyle {
    pub text: TextStyle,
    pub border: Color,
}

impl KeyStyle {
    pub fn new(text: TextStyle, border: Color) -> Self {
        Self { text, border }
    }

    fn color(&self) -> Color {
        self.text.color.unwrap_or(Color::TRANSPARENT)
    }
}

/// A cap's box: as tall as its text's line, padded only sideways, so a
/// line with a cap in it is no taller than without.
fn cap_box(line_h: f32, border: Color) -> NodeSpec {
    NodeSpec::row()
        .min_height(line_h)
        .pad_xy(CAP.pad, 0.0)
        .gap(CAP.part_gap)
        .radius(CAP.radius)
        .border(CAP.border, border)
        .cross_align(Align::Center)
}

fn line_h(ui: &mut Ui<'_>, style: &KeyStyle) -> f32 {
    ui.measure_text("Mg", &style.text, None).height
}

/// `notation` as caps, a row of its own in the row open now, keyed by
/// the notation (a test finds `<C-w>`'s caps by it).
pub fn keys(ui: &mut Ui<'_>, icons: &Icons, notation: &str, style: &KeyStyle) {
    let lh = line_h(ui, style);
    ui.with_keyed(
        notation,
        NodeSpec::row().gap(CAP.gap).cross_align(Align::Center),
        |ui| {
            for c in caps(notation) {
                ui.with(cap_box(lh, style.border), |ui| {
                    for p in &c {
                        match p {
                            CapPart::Icon(name) => {
                                icon(ui, icons, name, style.text.size, style.color())
                            }
                            CapPart::Text(t) => ui.text(t, style.text),
                        }
                    }
                });
            }
        },
    );
}

/// How wide [`keys`] draws `notation`.
pub fn keys_width(ui: &mut Ui<'_>, notation: &str, style: &KeyStyle) -> f32 {
    let all = caps(notation);
    let mut w = CAP.gap * all.len().saturating_sub(1) as f32;
    for c in &all {
        w += 2.0 * CAP.pad + CAP.part_gap * c.len().saturating_sub(1) as f32;
        for p in c {
            w += match p {
                CapPart::Icon(_) => style.text.size,
                CapPart::Text(t) => ui.measure_text(t, &style.text, None).width,
            };
        }
    }
    w
}

/// A legend's items, each its keys as caps — the alternatives a few
/// px apart — and its words after, drawn into the row open now, which
/// wraps between them (a strip, `Tab::strip`), never inside one.
pub fn legend_items(
    ui: &mut Ui<'_>,
    icons: &Icons,
    items: &[(&[&str], &str)],
    style: &KeyStyle,
    words: TextStyle,
) {
    for (i, (alts, what)) in items.iter().enumerate() {
        ui.with_key(
            ui.child_key("legend").index(i as u64),
            NodeSpec::row().gap(CAP.word_gap).cross_align(Align::Center),
            |ui| {
                ui.with(
                    NodeSpec::row().gap(CAP.alt_gap).cross_align(Align::Center),
                    |ui| {
                        for k in alts.iter() {
                            keys(ui, icons, k, style);
                        }
                    },
                );
                ui.text(what, words);
            },
        );
    }
}

// ------------------------------------------------------------------ Lua

fn shape_table(lua: &mlua::Lua, shape: &Shape) -> mlua::Result<mlua::Table> {
    let pts = |points: &[[f32; 2]]| -> mlua::Result<mlua::Table> {
        let t = lua.create_table()?;
        for (i, p) in points.iter().enumerate() {
            t.set(i + 1, lua.create_sequence_from([p[0], p[1]])?)?;
        }
        Ok(t)
    };
    let out = lua.create_table()?;
    for (i, p) in shape.iter().enumerate() {
        let t = lua.create_table()?;
        match p {
            Part::Stroke {
                points,
                width,
                curve,
            } => {
                t.set("stroke", pts(points)?)?;
                t.set("width", *width)?;
                if *curve {
                    t.set("curve", true)?;
                }
            }
            Part::Fill { points } => t.set("fill", pts(points)?)?,
            Part::Dot { r } => t.set("dot", *r)?,
            Part::Glyph { text, scale } => {
                t.set("glyph", text.as_str())?;
                t.set("scale", *scale)?;
            }
        }
        out.set(i + 1, t)?;
    }
    Ok(out)
}

fn bad(s: String) -> mlua::Error {
    mlua::Error::runtime(s)
}

/// A shape from Lua: a list of parts, each `stroke =`, `fill =`, `dot =`
/// or `glyph =` (docs/design/icons.md Decision 2).
fn shape_of(name: &str, t: &mlua::Table) -> mlua::Result<Shape> {
    let pts = |what: &str, v: mlua::Table| -> mlua::Result<Vec<[f32; 2]>> {
        let mut out = Vec::new();
        for p in v.sequence_values::<mlua::Table>() {
            let p = p.map_err(|_| bad(format!("icon {name}: a {what} point is {{x, y}}")))?;
            out.push([p.get(1)?, p.get(2)?]);
        }
        if out.len() < 2 && what == "stroke" {
            return Err(bad(format!(
                "icon {name}: a stroke wants two points or more"
            )));
        }
        if !(3..=FILL_MAX).contains(&out.len()) && what == "fill" {
            return Err(bad(format!(
                "icon {name}: a fill wants 3 to {FILL_MAX} points"
            )));
        }
        Ok(out)
    };
    let mut shape = Vec::new();
    for part in t.sequence_values::<mlua::Table>() {
        let part = part.map_err(|_| bad(format!("icon {name}: a part is a table")))?;
        if let Some(s) = part.get::<Option<mlua::Table>>("stroke")? {
            shape.push(Part::Stroke {
                points: pts("stroke", s)?,
                width: part.get::<Option<f32>>("width")?.unwrap_or(STROKE),
                curve: part.get::<Option<bool>>("curve")?.unwrap_or(false),
            });
        } else if let Some(f) = part.get::<Option<mlua::Table>>("fill")? {
            shape.push(Part::Fill {
                points: pts("fill", f)?,
            });
        } else if let Some(r) = part.get::<Option<f32>>("dot")? {
            shape.push(Part::Dot { r });
        } else if let Some(g) = part.get::<Option<String>>("glyph")? {
            shape.push(Part::Glyph {
                text: g,
                scale: part.get::<Option<f32>>("scale")?.unwrap_or(1.0),
            });
        } else {
            return Err(bad(format!(
                "icon {name}: a part is stroke =, fill =, dot = or glyph ="
            )));
        }
    }
    Ok(shape)
}

/// The node `kawoosh.icon(name, opts)` returns: the box and what
/// [`resolve`] puts in it, as kui-lua's tables.
fn icon_node(
    lua: &mlua::Lua,
    icons: &Icons,
    name: &str,
    opts: Option<mlua::Table>,
) -> mlua::Result<mlua::Table> {
    let mut size = 13.0;
    let mut color = icons.fg.to_hex() as i64;
    let node = lua.create_table()?;
    if let Some(o) = &opts {
        for pair in o.pairs::<mlua::Value, mlua::Value>() {
            let (k, v) = pair?;
            match k.as_string().map(|s| s.to_string_lossy()).as_deref() {
                Some("size") => size = lua_f32(&v).unwrap_or(size),
                Some("color") => {
                    if let Some(c) = v.as_i64() {
                        color = c;
                    }
                }
                _ => node.set(k, v)?,
            }
        }
    }
    let size = size.round();
    node.set("type", "row")?;
    node.set("width", size)?;
    node.set("height", size)?;
    node.set("main_align", "center")?;
    node.set("cross_align", "center")?;
    let vec = |p: &Vec2| lua.create_sequence_from([p.x, p.y]);
    for (i, p) in resolve(&icons.shape(name), size).into_iter().enumerate() {
        let t = lua.create_table()?;
        match p {
            Prim::Line {
                points,
                width,
                curve,
            } => {
                t.set("type", "line")?;
                let ps = lua.create_table()?;
                for (j, p) in points.iter().enumerate() {
                    ps.set(j + 1, vec(p)?)?;
                }
                t.set("points", ps)?;
                t.set("width", width)?;
                t.set("color", color)?;
                if curve {
                    t.set("curve", true)?;
                }
            }
            Prim::Poly { points } => {
                t.set("type", "polygon")?;
                let ps = lua.create_table()?;
                for (j, p) in points.iter().enumerate() {
                    ps.set(j + 1, vec(p)?)?;
                }
                t.set("points", ps)?;
                t.set("bg", color)?;
            }
            Prim::Dot { d } => {
                t.set("type", "row")?;
                t.set("width", d)?;
                t.set("height", d)?;
                t.set("radius", d / 2.0)?;
                t.set("bg", color)?;
            }
            Prim::Glyph { text, size } => {
                t.set("type", "text")?;
                t.set("value", text)?;
                t.set("size", size)?;
                t.set("color", color)?;
                t.set("wrap", "none")?;
            }
        }
        node.set(i + 1, t)?;
    }
    Ok(node)
}

fn lua_f32(v: &mlua::Value) -> Option<f32> {
    match v {
        mlua::Value::Integer(i) => Some(*i as f32),
        mlua::Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// The Lua half: `kawoosh.icon(name, opts)`, the `kawoosh.icons` table
/// a shape is read from and written to, `kawoosh.icon_names()`, and
/// what boot.lua's `ctx.keys` / `ctx.legend` draw from
/// (`kawoosh._key_caps`, `kawoosh._cap`).
pub(crate) fn lua_door(lua: &mlua::Lua, icons: Shared) -> mlua::Result<()> {
    let k: mlua::Table = lua.globals().get("kawoosh")?;
    let at = icons.clone();
    k.set(
        "icon",
        lua.create_function(move |lua, (name, opts): (String, Option<mlua::Table>)| {
            icon_node(lua, &at.borrow(), &name, opts)
        })?,
    )?;
    let at = icons.clone();
    k.set(
        "icon_names",
        lua.create_function(move |lua, ()| lua.create_sequence_from(at.borrow().names()))?,
    )?;
    // `kawoosh.icons.NAME` reads a shape, `= { … }` replaces it, `= nil`
    // puts the shipped one back.
    let proxy = lua.create_table()?;
    let meta = lua.create_table()?;
    let at = icons.clone();
    meta.set(
        "__index",
        lua.create_function(move |lua, (_, name): (mlua::Value, String)| {
            let set = at.borrow();
            if !set.has(&name) {
                return Ok(mlua::Value::Nil);
            }
            Ok(mlua::Value::Table(shape_table(lua, &set.shape(&name))?))
        })?,
    )?;
    let at = icons;
    meta.set(
        "__newindex",
        lua.create_function(
            move |_, (_, name, shape): (mlua::Value, String, Option<mlua::Table>)| {
                let shape = match shape {
                    Some(t) => Some(shape_of(&name, &t)?),
                    None => None,
                };
                at.borrow_mut().define(&name, shape);
                Ok(())
            },
        )?,
    )?;
    proxy.set_metatable(Some(meta))?;
    k.set("icons", proxy)?;
    k.set(
        "_key_caps",
        lua.create_function(|lua, notation: String| {
            let out = lua.create_table()?;
            for (i, c) in caps(&notation).iter().enumerate() {
                let cap = lua.create_table()?;
                for (j, p) in c.iter().enumerate() {
                    let t = lua.create_table()?;
                    match p {
                        CapPart::Icon(n) => t.set("icon", *n)?,
                        CapPart::Text(s) => t.set("text", s.as_str())?,
                    }
                    cap.set(j + 1, t)?;
                }
                out.set(i + 1, cap)?;
            }
            Ok(out)
        })?,
    )?;
    let m = lua.create_table()?;
    m.set("pad", CAP.pad)?;
    m.set("radius", CAP.radius)?;
    m.set("border", CAP.border)?;
    m.set("gap", CAP.gap)?;
    m.set("part_gap", CAP.part_gap)?;
    m.set("alt_gap", CAP.alt_gap)?;
    m.set("word_gap", CAP.word_gap)?;
    m.set("item_gap", CAP.item_gap)?;
    k.set("_cap", m)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shipped shape's ink is about the middle of its box: what
    /// a glyph's was not.
    #[test]
    fn every_icon_is_centred_in_its_box() {
        for name in NAMES {
            let shape = default_shape(name).unwrap();
            let (mut lo, mut hi) = ([1.0f32, 1.0], [0.0f32, 0.0]);
            let mut take = |x: f32, y: f32| {
                lo = [lo[0].min(x), lo[1].min(y)];
                hi = [hi[0].max(x), hi[1].max(y)];
            };
            for p in &shape {
                match p {
                    Part::Stroke { points, .. } | Part::Fill { points } => {
                        points.iter().for_each(|p| take(p[0], p[1]))
                    }
                    Part::Dot { r } => {
                        take(0.5 - r, 0.5 - r);
                        take(0.5 + r, 0.5 + r);
                    }
                    Part::Glyph { .. } => take(0.5, 0.5),
                }
            }
            let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
            // A triangle's middle is its outline's; at 13 px, 0.04 is
            // half a pixel.
            assert!(
                (c[0] - 0.5).abs() <= 0.04 && (c[1] - 0.5).abs() <= 0.04,
                "{name}: ink centred at {c:?}"
            );
            assert!(
                lo[0] >= 0.0 && lo[1] >= 0.0 && hi[0] <= 1.0 && hi[1] <= 1.0,
                "{name} stays in its box"
            );
        }
    }

    #[test]
    fn a_notation_reads_as_caps_modifiers_first() {
        use CapPart::{Icon, Text};
        let t = |s: &str| Text(s.into());
        let caps = |n: &str| caps_on(n, Host::Mac);
        assert_eq!(
            caps("<C-w>j"),
            vec![vec![Icon("ctrl"), t("w")], vec![t("j")]]
        );
        assert_eq!(caps("J"), vec![vec![t("J")]], "a capital alone is its key");
        assert_eq!(
            caps("<C-S-j>"),
            vec![vec![Icon("ctrl"), Icon("shift"), t("j")]],
            "a chord's capital is its shift"
        );
        assert_eq!(caps("<C-H>"), caps("<C-S-h>"));
        assert_eq!(caps("<A-/>"), vec![vec![Icon("alt"), t("/")]]);
        assert_eq!(caps("<D-s>"), vec![vec![Icon("cmd"), t("s")]]);
        assert_eq!(caps("<CR>"), vec![vec![Icon("return")]]);
        assert_eq!(caps("<S-Tab>"), vec![vec![Icon("shift"), Icon("tab")]]);
        assert_eq!(caps("<S-End>"), vec![vec![Icon("shift"), t("end")]]);
        assert_eq!(caps("<leader>f"), vec![vec![t("spc")], vec![t("f")]]);
        assert_eq!(caps("<Esc>"), vec![vec![t("esc")]]);
        assert_eq!(caps("<D-->"), vec![vec![Icon("cmd"), t("-")]]);
        assert_eq!(caps("g-"), vec![vec![t("g")], vec![t("-")]]);
    }

    /// A PC's keyboard has no ⌃ ⌥ ⌘: its modifiers are the words on its
    /// keys, the system's first, and ⌘'s key is Win or Super.
    #[test]
    fn a_pc_spells_a_chords_modifiers_as_words() {
        use CapPart::{Icon, Text};
        let t = |s: &str| Text(s.into());
        let win = |n: &str| caps_on(n, Host::Windows);
        assert_eq!(win("<C-w>j"), vec![vec![t("ctrl+w")], vec![t("j")]]);
        assert_eq!(win("<C-S-j>"), vec![vec![t("ctrl+shift+j")]]);
        assert_eq!(win("<C-H>"), win("<C-S-h>"));
        assert_eq!(win("<A-/>"), vec![vec![t("alt+/")]]);
        assert_eq!(win("<A-S-h>"), vec![vec![t("alt+shift+h")]]);
        assert_eq!(win("<D-s>"), vec![vec![t("win+s")]]);
        assert_eq!(win("<D-S-f>"), vec![vec![t("win+shift+f")]]);
        assert_eq!(win("<D-->"), vec![vec![t("win+-")]]);
        assert_eq!(win("<S-Tab>"), vec![vec![t("shift+"), Icon("tab")]]);
        assert_eq!(win("<S-End>"), vec![vec![t("shift+end")]]);
        assert_eq!(caps_on("<D-s>", Host::Linux), vec![vec![t("super+s")]]);
        // What has no modifier reads the same everywhere.
        for n in ["J", "<CR>", "<leader>f", "<Esc>", "g-"] {
            assert_eq!(win(n), caps_on(n, Host::Mac), "{n}");
        }
        assert_eq!(caps("<C-w>"), caps_on("<C-w>", Host::HERE));
    }

    #[test]
    fn a_user_shape_replaces_the_shipped_one_until_cleared() {
        let mut set = Icons::default();
        let mine = vec![Part::Dot { r: 0.4 }];
        set.define("close", Some(mine.clone()));
        assert_eq!(set.shape("close"), mine);
        set.define("close", None);
        assert_eq!(set.shape("close"), default_shape("close").unwrap());
        assert_eq!(set.shape("no such"), default_shape("missing").unwrap());
        set.define("star", Some(mine.clone()));
        assert!(set.names().contains(&"star".to_string()));
    }
}
