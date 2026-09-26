//! The look, from the settings tree to kui (kui.md D7, roadmap step 5):
//! the face the buffers and the terminals are drawn in (`font.*`), the
//! chrome's palette (`theme.*`) and the syntax colours (`tokens.colors`),
//! read at the frame after the tree moves and pushed into the core —
//! `add_system_font`, `set_theme_source`, `set_tokens` — so a save of
//! `settings.lua` or a `:set` is the window changing under the hand.
//!
//! ```lua
//! return {
//!   font = { family = "JetBrains Mono", size = 14, line_height = 1.5,
//!            features = "-liga +calt" },
//!   theme = { name = "rose-pine", appearance = "dark", accent = "#e0af68" },
//!   tokens = { colors = { keyword = { light = "#7a2fb0", dark = "#c78fe8" },
//!                         string = "#9cc87a" } },
//! }
//! ```
//!
//! - **`font.family`** is a family kui can see — installed, or one of
//!   the bundled faces — and the empty string is the face kawoosh ships
//!   ([`Kawoosh::bundled_font`]). A family that resolves to nothing is a
//!   toast, once, and the face stays. `font.size` is the mono text's
//!   size in logical px, `font.line_height` a ratio of it (the row's
//!   height is the product, rounded), `font.features` OpenType tags in
//!   kui's spelling (`liga=0`, `-liga`, `tnum`). Every mono run — the
//!   rows, the gutter, the terminals' cells, the panes' tables — is
//!   [`rows::mono`] over the one [`Face`], so the cell size follows.
//! - **`theme.name`** is a palette of [`crate::themes`] — `rose-pine`
//!   (the default), `rose-pine-moon` — which sets the chrome's roles,
//!   the syntax hues and the terminal's sixteen from one set of colours,
//!   pinned (roadmap step 28); or `system`, the way below.
//!   **`theme.appearance`** is `system` (the OS's base), `dark` or
//!   `light`; `theme.accent` a colour, or `system` for the OS's; every
//!   other key under `theme` a role of kui's `Theme` by name (`bg`,
//!   `surface`, `fg`, `muted`, `selection`, `focus_ring`, `danger`, …),
//!   written over the palette. Under `theme.name = "system"` the OS's
//!   appearance with no role set keeps following the OS
//!   (`ThemeSource::Derived`, with the accent when one is given); an
//!   appearance named or a role set pins a palette derived from the
//!   appearance and the accent. Whatever the source, the selection is
//!   held legible under the text (`themes::legible_selection`): an
//!   accent that would hide it is pinned fainter.
//! - **`tokens.colors`** names a syntax token (`keyword`, `string`,
//!   `comment`, … — `Token::name`) and gives it one colour or a light
//!   and a dark half (`{ light, dark }` or `{ "#l", "#d" }`); a token
//!   not named keeps the palette's hue (`palette.rs`'s under `system`). The same table, the defaults
//!   filled in, is declared as the host's kui tokens, so a Lua view
//!   paints `color = "$keyword"` and gets the frame's half. `kawoosh.
//!   colors { keyword = "ff0000" }` from code lands over the file's.

use std::collections::HashMap;

use kawoosh_editor::Setting;
use kawoosh_systems::ts::Token;
use kui_native::schema::THEME_ROLES;
use kui_native::{
    Appearance, Color, FontFeatures, FontId, SystemEnv, Theme, ThemeSource, Tokens, Ui,
};

use crate::app::Kawoosh;
use crate::notify::{Level, Note};
use crate::rows::{FONT, LH};
use crate::themes::{self, Named};

/// The face every mono run is shaped in: the font, its size, the row's
/// height and the shaper's features. `Copy`, since it rides into every
/// row's style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    /// None shapes in kui's generic mono.
    pub id: Option<FontId>,
    pub size: f32,
    pub line_height: f32,
    pub features: FontFeatures,
}

impl Default for Face {
    fn default() -> Self {
        Self {
            id: None,
            size: FONT,
            line_height: LH,
            features: FontFeatures::new(),
        }
    }
}

/// The chrome's metrics (2026-09-23): the rows around the panes — the
/// tab strip, a pane's title bar, the status and command strips, the
/// title bar's text — follow the editor's font up to [`CHROME_MAX`], so
/// a big font for reading does not make the chrome a banner, and every
/// height is its text's line height plus the padding it had at the
/// default size (13 px: tabs 22, strips 24, pane titles 22).
/// `font.chrome_size` pins the size instead.
#[derive(Clone, Copy, Debug)]
pub struct Chrome {
    /// The mono face the chrome's text is set in.
    pub face: Face,
    /// The smaller text: a pane's title, the message line.
    pub small: f32,
    pub tab_h: f32,
    pub strip_h: f32,
    pub pane_title_h: f32,
}

/// The largest the chrome's text follows `font.size` to.
pub const CHROME_MAX: f32 = 16.0;

impl Chrome {
    pub fn of(face: Face, size: f32) -> Chrome {
        let line_height = (size * LINE_HEIGHT as f32).round().max(size + 2.0);
        let small = (size - 1.0).max(8.0);
        let small_lh = (small * LINE_HEIGHT as f32).round();
        Chrome {
            face: Face {
                size,
                line_height,
                ..face
            },
            small,
            tab_h: line_height + 2.0,
            strip_h: line_height + 4.0,
            pane_title_h: small_lh + 4.0,
        }
    }
}

impl Default for Chrome {
    fn default() -> Self {
        Chrome::of(Face::default(), FONT)
    }
}

/// The smallest and largest `font.size` honoured.
const SIZE_RANGE: (f32, f32) = (6.0, 96.0);
/// The default `font.line_height`, the ratio that makes 13 px rows of 20.
pub const LINE_HEIGHT: f64 = 1.5;

/// What the look was last built from, so a frame rebuilds it only when
/// the tree, the OS's appearance or its accent moved.
#[derive(Default)]
pub struct Look {
    /// The settings version the look was built from; none when
    /// something else asked for a rebuild (`kawoosh.colors`).
    pub seen: Option<u64>,
    /// The OS appearance the theme was resolved under.
    appearance: Appearance,
    /// The OS accent the theme was resolved under.
    accent: Option<Color>,
    /// The palette `theme.name` names; none under `system`.
    pub named: Option<&'static Named>,
    /// The syntax colours the config set, a light and a dark half each.
    pub syntax: HashMap<Token, (Color, Color)>,
    /// The family a toast already said was missing.
    missing: Option<String>,
}

/// `#rrggbb` or `#rrggbbaa`, the `#` optional.
pub(crate) fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(match s.len() {
        6 => Color::hex((v << 8) | 0xFF),
        8 => Color::hex(v),
        _ => return None,
    })
}

/// A token's colour as the tree spells it: one colour, `{ light, dark
/// }`, or a list of the two.
fn halves(v: &Setting) -> Option<(Color, Color)> {
    match v {
        Setting::Str(s) => {
            let c = parse_color(s)?;
            Some((c, c))
        }
        Setting::Table(t) => {
            let light = parse_color(t.get("light")?.as_str()?)?;
            let dark = parse_color(t.get("dark")?.as_str()?)?;
            Some((light, dark))
        }
        Setting::List(l) => match l.as_slice() {
            [a, b] => Some((parse_color(a.as_str()?)?, parse_color(b.as_str()?)?)),
            [a] => {
                let c = parse_color(a.as_str()?)?;
                Some((c, c))
            }
            _ => None,
        },
        _ => None,
    }
}

impl Kawoosh {
    /// Builds the look again when the settings or the OS's appearance
    /// moved since the last frame, and pushes it into the core.
    pub(crate) fn sync_look(&mut self, ui: &mut Ui<'_>) {
        let v = self.ed.settings.version();
        let system = ui.env().system;
        let sys = system.appearance;
        if self.look.seen == Some(v)
            && self.look.appearance == sys
            && self.look.accent == system.accent
        {
            return;
        }
        self.look.seen = Some(v);
        self.look.appearance = sys;
        self.look.accent = system.accent;
        self.sync_layout_settings();
        self.note_undeclared();
        let mut notes = Vec::new();
        self.sync_font(ui, &mut notes);
        self.look.named = self.theme_named(&mut notes);
        self.sync_tokens(ui, &mut notes);
        self.sync_theme(ui, &system, &mut notes);
        for n in notes {
            self.notify_with(Note::new(Level::Warn, n).source("settings"));
        }
    }

    fn sync_font(&mut self, ui: &mut Ui<'_>, notes: &mut Vec<String>) {
        let s = &self.ed.settings;
        let family = s.str("font.family").unwrap_or("").trim().to_string();
        let size = s
            .get("font.size")
            .and_then(Setting::as_float)
            .map(|f| f as f32)
            .unwrap_or(FONT)
            .clamp(SIZE_RANGE.0, SIZE_RANGE.1);
        let ratio = s
            .get("font.line_height")
            .and_then(Setting::as_float)
            .unwrap_or(LINE_HEIGHT)
            .clamp(1.0, 3.0) as f32;
        let features = FontFeatures::parse(s.str("font.features").unwrap_or(""));
        let id = if family.is_empty() {
            self.bundled_font
        } else {
            match ui.core().add_system_font(&family) {
                Some(id) => Some(id),
                None => {
                    if self.look.missing.as_deref() != Some(family.as_str()) {
                        notes.push(format!("font: no family \"{family}\" — the face stays"));
                        self.look.missing = Some(family);
                    }
                    self.face.id
                }
            }
        };
        self.face = Face {
            id,
            size,
            line_height: (size * ratio).round().max(size + 2.0),
            features,
        };
        let chrome = match s.get("font.chrome_size").and_then(Setting::as_float) {
            Some(n) if n > 0.0 => (n as f32).clamp(SIZE_RANGE.0, SIZE_RANGE.1),
            _ => size.min(CHROME_MAX),
        };
        self.chrome = Chrome::of(self.face, chrome);
    }

    /// `theme.name`: a palette of `themes.rs`, or `system` for none. A
    /// name nobody ships is a toast, and the default stands.
    fn theme_named(&self, notes: &mut Vec<String>) -> Option<&'static Named> {
        let name = self.ed.settings.str("theme.name").unwrap_or("").trim();
        match name {
            "system" => None,
            "" => Some(&themes::NAMED[0]),
            n => themes::named(n).or_else(|| {
                let known: Vec<&str> = themes::NAMED.iter().map(|n| n.name).collect();
                notes.push(format!(
                    "theme.name: no palette \"{n}\" (system, {})",
                    known.join(", ")
                ));
                Some(&themes::NAMED[0])
            }),
        }
    }

    fn sync_theme(&mut self, ui: &mut Ui<'_>, system: &SystemEnv, notes: &mut Vec<String>) {
        let sys = system.appearance;
        let s = &self.ed.settings;
        let named = s.str("theme.appearance").unwrap_or("system").trim();
        let appearance = match named {
            "dark" => Some(Appearance::Dark),
            "light" => Some(Appearance::Light),
            "system" | "" => None,
            other => {
                notes.push(format!(
                    "theme.appearance: \"{other}\" is not system, dark or light"
                ));
                None
            }
        };
        let accent = match s.str("theme.accent") {
            // The OS's, over a palette of kawoosh's own.
            Some(a) if a.trim() == "system" => system.accent,
            Some(a) if !a.trim().is_empty() => {
                let c = parse_color(a);
                if c.is_none() {
                    notes.push(format!("theme.accent: \"{a}\" is not a colour"));
                }
                c
            }
            _ => None,
        };
        let mut roles = Vec::new();
        if let Some(Setting::Table(t)) = s.get("theme") {
            for (name, v) in t {
                if name == "appearance" || name == "accent" || name == "name" {
                    continue;
                }
                let Some(role) = THEME_ROLES.iter().find(|r| r.name == name) else {
                    notes.push(format!("theme: no role \"{name}\""));
                    continue;
                };
                match v.as_str().and_then(parse_color) {
                    Some(c) => roles.push((role, c)),
                    None => notes.push(format!("theme.{name}: {v} is not a colour")),
                }
            }
        }
        let source = match self.look.named {
            // A palette: its variant for the base, the accent over it
            // (the palette's selection kept — it is the palette's, not
            // the accent's), the roles over that. Pinned, whatever the
            // OS says, but for the base under `system`.
            Some(p) => {
                let f = p.flavour(appearance.unwrap_or(sys) != Appearance::Light);
                let mut t = f.theme();
                if let Some(c) = accent {
                    let selection = t.selection;
                    t = t.with_accent(c);
                    t.selection = selection;
                }
                for (role, c) in roles {
                    (role.set)(&mut t, c);
                }
                ThemeSource::Pinned(t)
            }
            None => match (appearance, accent) {
                (None, None) if roles.is_empty() => ThemeSource::Derived,
                (None, Some(c)) if roles.is_empty() => ThemeSource::DerivedWithAccent(c),
                (a, c) => {
                    let mut t = Theme::derive(a.unwrap_or(sys), c);
                    for (role, c) in roles {
                        (role.set)(&mut t, c);
                    }
                    ThemeSource::Pinned(t)
                }
            },
        };
        // The selection held legible, whoever made it: a source that
        // would hide the text under it is pinned with a fainter one.
        let t = source.resolve(system);
        let inks: Vec<Color> = Token::ALL
            .iter()
            .filter_map(|tok| self.syntax_color_for(*tok, t.is_dark()))
            .collect();
        let held = themes::legible_selection(t, &inks);
        let source = if held.selection == t.selection {
            source
        } else {
            ThemeSource::Pinned(held)
        };
        if ui.core().theme_source() != source {
            ui.core().set_theme_source(source);
        }
    }

    fn sync_tokens(&mut self, ui: &mut Ui<'_>, notes: &mut Vec<String>) {
        let mut syntax = HashMap::new();
        if let Some(Setting::Table(t)) = self.ed.settings.get("tokens.colors") {
            for (name, v) in t {
                let Some(tok) = Token::ALL.iter().copied().find(|t| t.name() == name) else {
                    notes.push(format!("tokens.colors: no token \"{name}\""));
                    continue;
                };
                match halves(v) {
                    Some(h) => {
                        syntax.insert(tok, h);
                    }
                    None => notes.push(format!("tokens.colors.{name}: {v} is not a colour")),
                }
            }
        }
        for (t, c) in &self.scripting.colors {
            syntax.insert(*t, (*c, *c));
        }
        // The whole vocabulary, so `$comment` resolves whether or not
        // the config named it; a token with no hue (plain text) has no
        // entry, and a `$plain` is kui's `unknown-token`.
        let mut tokens = Tokens::new();
        for t in Token::ALL {
            let halves = syntax.get(t).copied().or_else(|| {
                Some((
                    self.default_syntax(*t, false)?,
                    self.default_syntax(*t, true)?,
                ))
            });
            if let Some((light, dark)) = halves {
                tokens = tokens.color_themed(t.name(), light, dark);
            }
        }
        // The sizes as length tokens, so a Lua view's text follows the
        // font as the editor's own chrome does — `size = "$chrome"` —
        // and its arithmetic can read them (`env.tokens.lengths`): the
        // editor's text and row, the chrome's text, smaller text and row.
        let c = self.chrome;
        tokens = tokens
            .length("font", self.face.size)
            .length("font_row", self.face.line_height)
            .length("chrome", c.face.size)
            .length("chrome_small", c.small)
            .length("chrome_row", c.face.line_height);
        ui.set_tokens(tokens);
        self.look.syntax = syntax;
    }

    /// The syntax colour for a token on the theme's base: the config's
    /// half, else the palette's hue (`theme.name`'s, or `palette.rs`'s
    /// under `system`).
    pub fn syntax_color_for(&self, token: Token, dark: bool) -> Option<Color> {
        if let Some(c) = self.scripting.colors.get(&token) {
            return Some(*c);
        }
        match self.look.syntax.get(&token) {
            Some((l, d)) => Some(if dark { *d } else { *l }),
            None => self.default_syntax(token, dark),
        }
    }

    /// A token's hue when the config names none.
    fn default_syntax(&self, token: Token, dark: bool) -> Option<Color> {
        match self.look.named {
            Some(p) => p.flavour(dark).syntax(token),
            None => crate::palette::syntax_color(token, dark),
        }
    }

    /// The terminal's sixteen on a base: the palette's, or Tomorrow's
    /// under `system` (`palette::ansi`).
    pub fn ansi_for(&self, dark: bool) -> [u32; 16] {
        match self.look.named {
            Some(p) => p.flavour(dark).ansi(),
            None => crate::palette::ansi(dark),
        }
    }
}

/// `font bigger` / `smaller` step `font.size` by a pixel in the session
/// layer, within what is honoured; `font reset` takes the session's
/// value out, back to the settings files'. ⌘= ⌘+ ⌘- ⌘_ ⌘0, Ctrl where
/// there is no ⌘ (keys.md).
pub(crate) fn commands() -> Vec<crate::commands::ShellCommand> {
    use crate::commands::cmd;
    use kawoosh_editor::{Layer, Spec};
    let step = |k: &mut Kawoosh, by: f64| {
        let now =
            k.ed.settings
                .get("font.size")
                .and_then(Setting::as_float)
                .unwrap_or(FONT as f64);
        let next = (now + by).clamp(SIZE_RANGE.0 as f64, SIZE_RANGE.1 as f64);
        k.ed.settings
            .set(Layer::Session, "font.size", Setting::Float(next));
        k.ed.message = format!("font {next}");
    };
    vec![
        cmd(
            Spec::new("font bigger").doc("the font a pixel bigger, for the session"),
            move |k, ctx| step(k, ctx.count.max(1) as f64),
        ),
        cmd(
            Spec::new("font smaller").doc("the font a pixel smaller, for the session"),
            move |k, ctx| step(k, -(ctx.count.max(1) as f64)),
        ),
        cmd(
            Spec::new("font reset").doc("the font back to the size the settings say"),
            |k, _| {
                k.ed.settings.unset(Layer::Session, "font.size");
                let size =
                    k.ed.settings
                        .get("font.size")
                        .and_then(Setting::as_float)
                        .unwrap_or(FONT as f64);
                k.ed.message = format!("font {size}");
            },
        ),
    ]
}
