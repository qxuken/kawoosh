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
//!   theme = { name = "rose-pine", dark = "ayu-mirage", appearance = "dark",
//!             accent = "#e0af68" },
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
//! - **`theme.name`** is a family of [`crate::themes`] — `rose-pine`
//!   (the default), `rose-pine-moon`, `ayu`, `ayu-mirage`, `gruvbox`
//!   (`-hard`, `-soft`), `tokyo-night` (`-storm`, `-moon`),
//!   `catppuccin` (`-macchiato`, `-frappe`), `kanagawa` (`-dragon`),
//!   `everforest` (`-hard`, `-soft`), `one`, `dracula`, `mono`,
//!   `mono-soft`, `paper`, `high-contrast` — a dark variant and a light
//!   one, each setting the
//!   chrome's roles, the syntax hues and the terminal's sixteen from one
//!   set of colours, pinned (roadmap step 28); or `system`, the way
//!   below. **`theme.dark`** and **`theme.light`** name a variant for
//!   their base apart from the family (`ayu-dark`, `rose-pine-dawn`, …;
//!   docs/design/themes.md), `system` for the way below on that base
//!   alone, empty for the family's half.
//!   **`theme.appearance`** is `system` (the OS's base), `dark` or
//!   `light`; `theme.accent` a colour, or `system` for the OS's; every
//!   other key under `theme` a role of kui's `Theme` by name (`bg`,
//!   `surface`, `fg`, `muted`, `selection`, `focus_ring`, `danger`, …),
//!   written over the palette. Where the base's half is `system` the OS's
//!   appearance with no role set keeps following the OS
//!   (`ThemeSource::Derived`, with the accent when one is given); an
//!   appearance named or a role set pins a palette derived from the
//!   appearance and the accent. Whatever the source, the selection is
//!   held legible under the text (`themes::legible_selection`, and a
//!   search hit's wash the same way, `themes::legible_hit`): an
//!   accent that would hide it is pinned fainter.
//! - **`tokens.colors`** names a syntax token (`keyword`, `string`,
//!   `comment`, … — `Token::name`) and gives it one colour or a light
//!   and a dark half (`{ light, dark }` or `{ "#l", "#d" }`); a token
//!   not named keeps the palette's hue (`palette.rs`'s under `system`). The same table, the defaults
//!   filled in, is declared as the host's kui tokens, so a Lua view
//!   paints `color = "$keyword"` and gets the frame's half. `kawoosh.
//!   colors { keyword = "ff0000" }` from code lands over the file's.
//! - **`tokens.styles`** sets a token's text beside its hue (themes.md
//!   Decision 6): a string of words — `bold`, `italic`, `underline`,
//!   `strike`, or `none` — in place of the theme's, or a table of
//!   booleans over it (`comment = { italic = false }`). Each theme
//!   starts from `themes::base_style` — comments italic, headings and
//!   strong bold — and the high-contrast pair sets keywords bold.

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
use crate::themes::{self, Pair, Style};

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
    /// The variant for each base, and the family (`themes::resolve`).
    pub pair: Pair,
    /// What is on show, for `kawoosh.themes.current()`: shared with the
    /// Lua door, rewritten at each rebuild.
    pub shown: std::rc::Rc<std::cell::RefCell<Shown>>,
    /// The syntax colours the config set, a light and a dark half each.
    pub syntax: HashMap<Token, (Color, Color)>,
    /// The styles the config set: what each turns on, and off.
    pub styles: HashMap<Token, (Style, Style)>,
    /// A search hit's wash for the look on show, held legible
    /// (`themes::legible_hit`); none before the first build.
    pub hit: Option<Color>,
    /// The family a toast already said was missing.
    missing: Option<String>,
}

/// What the look resolved to, as the Lua door says it: the family, the
/// variant on each base (`system` where kui's roles off the OS stand),
/// the base on show and the appearance setting; and the look on show
/// whole, what `:theme check` and `:theme lab` measure, with the
/// rebuild it is of (`version`, counted up at each).
#[derive(Clone, Debug, Default)]
pub struct Shown {
    pub family: String,
    pub dark: String,
    pub light: String,
    pub is_dark: bool,
    pub appearance: String,
    pub subject: crate::theme_check::Subject,
    pub version: u64,
}

/// `kawoosh.themes` (docs/design/themes.md Decision 4), set on the
/// runtime before the bundled plugins load: `variants`, each one's
/// `name`, `title`, `dark`, `roles` (every kui role by name), `syntax`
/// (a hue per token by name), `styles` (a token's that is not plain, as
/// a kui span's flags: `{ italic = true }`) and `ansi` (the sixteen), colours as
/// `0xRRGGBBAA`; `families`, each `{ name, dark, light }`; and
/// `current()`, what is on show — `family`, `dark` and `light` (a
/// variant's name, or `system`), `base` (`dark` or `light`),
/// `appearance` and `version`, counted up at each rebuild — as of the
/// last frame the look was built; `check([NAME])`, the theme check
/// (docs/design/themes.md Decision 7) of the look on show, or of a
/// variant as it ships.
pub(crate) fn lua_door(
    lua: &mlua::Lua,
    shown: std::rc::Rc<std::cell::RefCell<Shown>>,
) -> mlua::Result<()> {
    let hex = |c: Color| c.to_hex() as i64;
    let door = lua.create_table()?;
    let variants = lua.create_table()?;
    for (i, v) in themes::variants().iter().enumerate() {
        let t = lua.create_table()?;
        t.set("name", v.name)?;
        t.set("title", v.title)?;
        t.set("dark", v.dark())?;
        let roles = lua.create_table()?;
        for r in THEME_ROLES {
            roles.set(r.name, hex((r.get)(&v.theme)))?;
        }
        t.set("roles", roles)?;
        let syntax = lua.create_table()?;
        for tok in Token::ALL {
            if let Some(c) = v.syntax(*tok) {
                syntax.set(tok.name(), hex(c))?;
            }
        }
        t.set("syntax", syntax)?;
        t.set("styles", styles_table(lua, |tok| v.style(tok))?)?;
        t.set("ansi", lua.create_sequence_from(v.ansi.map(|x| x as i64))?)?;
        variants.set(i + 1, t)?;
    }
    door.set("variants", variants)?;
    let families = lua.create_table()?;
    for (i, f) in themes::FAMILIES.iter().enumerate() {
        let t = lua.create_table()?;
        t.set("name", f.name)?;
        t.set("dark", f.dark)?;
        t.set("light", f.light)?;
        families.set(i + 1, t)?;
    }
    door.set("families", families)?;
    let at = shown.clone();
    door.set(
        "check",
        lua.create_function(move |lua, name: Option<String>| {
            let subject = match name.as_deref() {
                None | Some("") => at.borrow().subject.clone(),
                Some(n) => match themes::variant(n) {
                    Some(v) => crate::theme_check::Subject::of_variant(v),
                    None => return Err(mlua::Error::runtime(format!("no theme \"{n}\""))),
                },
            };
            subject_table(lua, &subject)
        })?,
    )?;
    door.set(
        "current",
        lua.create_function(move |lua, ()| {
            let s = shown.borrow();
            let t = lua.create_table()?;
            t.set("family", s.family.as_str())?;
            t.set("dark", s.dark.as_str())?;
            t.set("light", s.light.as_str())?;
            t.set("base", if s.is_dark { "dark" } else { "light" })?;
            t.set("appearance", s.appearance.as_str())?;
            t.set("version", s.version)?;
            Ok(t)
        })?,
    )?;
    lua.globals()
        .get::<mlua::Table>("kawoosh")?
        .set("themes", door)
}

/// A token's style for Lua, as a kui span's flags — `{ italic = true }`
/// — for each token whose style is not plain.
fn styles_table(lua: &mlua::Lua, style: impl Fn(Token) -> Style) -> mlua::Result<mlua::Table> {
    let styles = lua.create_table()?;
    for tok in Token::ALL {
        let st = style(*tok);
        if st != Style::PLAIN {
            let f = lua.create_table()?;
            for (on, k) in [
                (st.bold, "bold"),
                (st.italic, "italic"),
                (st.underline, "underline"),
                (st.strike, "strikethrough"),
            ] {
                if on {
                    f.set(k, true)?;
                }
            }
            styles.set(tok.name(), f)?;
        }
    }
    Ok(styles)
}

/// `kawoosh.themes.check()`'s answer: the subject whole — `title`,
/// `dark`, `roles`, `hit` (a search hit's wash), `syntax`, `styles`,
/// `ansi` — and its `checks`, each
/// `{ group, what, fg, bg, ratio, need, ok }`, colours as `0xRRGGBBAA`.
fn subject_table(lua: &mlua::Lua, s: &crate::theme_check::Subject) -> mlua::Result<mlua::Table> {
    let hex = |c: Color| c.to_hex() as i64;
    let t = lua.create_table()?;
    t.set("title", s.title.as_str())?;
    t.set("dark", s.theme.is_dark())?;
    t.set("hit", hex(s.hit))?;
    let roles = lua.create_table()?;
    for r in THEME_ROLES {
        roles.set(r.name, hex((r.get)(&s.theme)))?;
    }
    t.set("roles", roles)?;
    let syntax = lua.create_table()?;
    for tok in Token::ALL {
        if let Some(c) = s.syntax[*tok as usize] {
            syntax.set(tok.name(), hex(c))?;
        }
    }
    t.set("syntax", syntax)?;
    t.set("styles", styles_table(lua, |tok| s.styles[tok as usize])?)?;
    t.set("ansi", lua.create_sequence_from(s.ansi.map(|x| x as i64))?)?;
    let checks = lua.create_table()?;
    for (i, c) in s.checks().iter().enumerate() {
        let r = lua.create_table()?;
        r.set("group", c.group)?;
        r.set("what", c.what.as_str())?;
        r.set("fg", hex(c.fg))?;
        r.set("bg", hex(c.bg))?;
        r.set("ratio", c.ratio)?;
        r.set("need", c.need)?;
        r.set("ok", c.ok())?;
        checks.set(i + 1, r)?;
    }
    t.set("checks", checks)?;
    Ok(t)
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

/// A token's style as `tokens.styles` spells it: what it turns on and
/// what off. Words replace the theme's style, so what they leave out is
/// off; a table of booleans turns on and off only what it names.
fn style_of(v: &Setting) -> Option<(Style, Style)> {
    let all = Style {
        bold: true,
        italic: true,
        underline: true,
        strike: true,
    };
    match v {
        Setting::Str(s) => Some((Style::parse(s)?, all)),
        Setting::Table(t) => {
            let (mut on, mut off) = (Style::PLAIN, Style::PLAIN);
            for (k, v) in t {
                let b = v.as_bool()?;
                let (on, off) = match k.as_str() {
                    "bold" => (&mut on.bold, &mut off.bold),
                    "italic" => (&mut on.italic, &mut off.italic),
                    "underline" => (&mut on.underline, &mut off.underline),
                    "strike" | "strikethrough" => (&mut on.strike, &mut off.strike),
                    _ => return None,
                };
                *on = b;
                *off = !b;
            }
            Some((on, off))
        }
        _ => None,
    }
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
        self.look.pair = self.theme_pair(&mut notes);
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

    /// `theme.name`, `theme.dark` and `theme.light` resolved
    /// (`themes::resolve`); what cannot be is a toast, and the default
    /// stands.
    fn theme_pair(&self, notes: &mut Vec<String>) -> Pair {
        let s = &self.ed.settings;
        let get = |k: &str| s.str(k).unwrap_or("");
        themes::resolve(
            get("theme.name"),
            get("theme.dark"),
            get("theme.light"),
            notes,
        )
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
                if matches!(
                    name.as_str(),
                    "appearance" | "accent" | "name" | "dark" | "light"
                ) {
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
        let base_dark = appearance.unwrap_or(sys) != Appearance::Light;
        let source = match self.look.pair.of(base_dark) {
            // A variant for the base: the accent over it (its selection
            // kept — it is the variant's, not the accent's), the roles
            // over that. Pinned, whatever the OS says, but for the base
            // under `system`.
            Some(v) => {
                let mut t = v.theme;
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
        let hit = themes::legible_hit(&held, &inks);
        self.look.hit = Some(hit);
        let source = if held.selection == t.selection {
            source
        } else {
            ThemeSource::Pinned(held)
        };
        if ui.core().theme_source() != source {
            ui.core().set_theme_source(source);
        }
        let pair = self.look.pair;
        let name = |v: Option<&themes::Variant>| v.map_or("system", |v| v.name).to_string();
        let dark = held.is_dark();
        let (dark_name, light_name) = (name(pair.dark), name(pair.light));
        let on = if dark { &dark_name } else { &light_name };
        let subject = crate::theme_check::Subject {
            title: format!("{on} (selected, {})", if dark { "dark" } else { "light" }),
            theme: held,
            hit,
            syntax: Token::ALL
                .iter()
                .map(|t| self.syntax_color_for(*t, dark))
                .collect(),
            styles: Token::ALL
                .iter()
                .map(|t| self.syntax_style_for(*t, dark))
                .collect(),
            ansi: self.ansi_for(dark),
        };
        let version = self.look.shown.borrow().version + 1;
        *self.look.shown.borrow_mut() = Shown {
            family: pair.family.map_or("system", |f| f.name).to_string(),
            dark: dark_name,
            light: light_name,
            is_dark: dark,
            appearance: if named.is_empty() { "system" } else { named }.to_string(),
            subject,
            version,
        };
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
        let mut styles = HashMap::new();
        if let Some(Setting::Table(t)) = self.ed.settings.get("tokens.styles") {
            for (name, v) in t {
                let Some(tok) = Token::ALL.iter().copied().find(|t| t.name() == name) else {
                    notes.push(format!("tokens.styles: no token \"{name}\""));
                    continue;
                };
                match style_of(v) {
                    Some(st) => {
                        styles.insert(tok, st);
                    }
                    None => notes.push(format!(
                        "tokens.styles.{name}: {v} is not a style (bold, italic, underline, strike, none)"
                    )),
                }
            }
        }
        self.look.styles = styles;
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

    /// How a token's text is set on the theme's base: the variant's
    /// style (`themes::base_style` under `system`), with what
    /// `tokens.styles` turns on and off.
    pub fn syntax_style_for(&self, token: Token, dark: bool) -> Style {
        let base = match self.look.pair.of(dark) {
            Some(v) => v.style(token),
            None => themes::base_style(token),
        };
        match self.look.styles.get(&token) {
            Some((on, off)) => Style {
                bold: on.bold || (base.bold && !off.bold),
                italic: on.italic || (base.italic && !off.italic),
                underline: on.underline || (base.underline && !off.underline),
                strike: on.strike || (base.strike && !off.strike),
            },
            None => base,
        }
    }

    /// A token's hue when the config names none.
    fn default_syntax(&self, token: Token, dark: bool) -> Option<Color> {
        match self.look.pair.of(dark) {
            Some(v) => v.syntax(token),
            None => crate::palette::syntax_color(token, dark),
        }
    }

    /// The terminal's sixteen on a base: the variant's, or Tomorrow's
    /// under `system` (`palette::ansi`).
    pub fn ansi_for(&self, dark: bool) -> [u32; 16] {
        match self.look.pair.of(dark) {
            Some(v) => v.ansi,
            None => crate::palette::ansi(dark),
        }
    }
}

/// The `theme.*` keys a pick sets, and `theme reset` takes out.
const PICKED: [&str; 4] = [
    "theme.name",
    "theme.dark",
    "theme.light",
    "theme.appearance",
];

impl Kawoosh {
    /// `:theme`'s line: the variant on show and its base, both halves,
    /// the family and the appearance.
    pub(crate) fn theme_line(&self) -> String {
        let s = self.look.shown.borrow();
        let (on, base) = if s.is_dark {
            (&s.dark, "dark")
        } else {
            (&s.light, "light")
        };
        format!(
            "theme {on} ({base}) · dark {} · light {} · family {} · appearance {}",
            s.dark, s.light, s.family, s.appearance
        )
    }

    /// `theme.dark` or `theme.light` set to `name` for the session — a
    /// variant of that base, or `system` — else a message saying why
    /// not.
    fn pick_half(&mut self, dark: bool, name: &str) {
        use kawoosh_editor::Layer;
        let key = if dark { "theme.dark" } else { "theme.light" };
        let base = if dark { "dark" } else { "light" };
        let ok = name == "system" || themes::variant(name).is_some_and(|v| v.dark() == dark);
        if !ok {
            let names: Vec<&str> = themes::variants()
                .iter()
                .filter(|v| v.dark() == dark)
                .map(|v| v.name)
                .collect();
            self.ed.message = format!(
                "theme {base}: no {base} theme \"{name}\" (system, {})",
                names.join(", ")
            );
            return;
        }
        self.ed
            .settings
            .set(Layer::Session, key, Setting::Str(name.into()));
        self.ed.message = if self.dark == dark {
            format!("{key} = {name}")
        } else {
            format!("{key} = {name} — shown when the base is {base}")
        };
    }

    /// `:theme check [NAME|all]`: the report in `*theme check*`, and a
    /// line saying how many pairs fall short — of the look on show, a
    /// variant as it ships, or every variant, each its own report.
    fn theme_check(&mut self, name: Option<&str>) {
        use crate::theme_check::Subject;
        let subjects: Vec<Subject> = match name {
            None => vec![self.look.shown.borrow().subject.clone()],
            Some("all") => themes::variants().iter().map(Subject::of_variant).collect(),
            Some(n) => match themes::variant(n) {
                Some(v) => vec![Subject::of_variant(v)],
                None => {
                    let names: Vec<&str> = themes::variants().iter().map(|v| v.name).collect();
                    self.ed.message =
                        format!("theme check: no theme \"{n}\" (all, {})", names.join(", "));
                    return;
                }
            },
        };
        let mut text = String::new();
        let mut short = Vec::new();
        for s in &subjects {
            let checks = s.checks();
            let n = checks.iter().filter(|c| !c.ok()).count();
            short.push(format!("{} {n}", s.title.split(' ').next().unwrap_or("")));
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&crate::theme_check::report(&s.title, &checks));
        }
        self.show_in_pane("*theme check*", &text);
        self.ed.message = format!("theme check: below their floor — {}", short.join(", "));
    }

    /// `theme.appearance` pinned (or `system`) for the session.
    fn pick_appearance(&mut self, word: &str) {
        use kawoosh_editor::Layer;
        self.ed.settings.set(
            Layer::Session,
            "theme.appearance",
            Setting::Str(word.into()),
        );
        self.ed.message = format!("theme.appearance = {word}");
    }
}

/// `font bigger` / `smaller` step `font.size` by a pixel in the session
/// layer, within what is honoured; `font reset` takes the session's
/// value out, back to the settings files'. ⌘= ⌘+ ⌘- ⌘_ ⌘0, Ctrl where
/// there is no ⌘ (keys.md).
///
/// `theme` (docs/design/themes.md Decision 3) says what is shown;
/// `theme toggle` pins the other base, `theme system` follows the OS
/// again, `theme dark` and `theme light` pin a base — or with a name set
/// that base's variant — and `theme FAMILY` takes a family whole: its
/// name and both its halves, so a lower layer's half cannot hide the
/// pick. `theme reset` takes the session's out. All the session's.
pub(crate) fn commands() -> Vec<crate::commands::ShellCommand> {
    use crate::commands::cmd;
    use kawoosh_editor::{ArgKind, Args, Layer, Spec};
    let half = |dark: bool| {
        let base = if dark { "dark" } else { "light" };
        cmd(
            Spec::new(&format!("theme {base}"))
                .args(Args::new(&[ArgKind::Text]))
                .doc(&format!(
                    "the base {base} for the session; with a NAME, the theme shown when it is"
                )),
            move |k, ctx| match ctx.args.first() {
                Some(n) => k.pick_half(dark, n),
                None => k.pick_appearance(base),
            },
        )
    };
    let mut theme = vec![
        cmd(
            Spec::new("theme").doc("the selected theme, both halves, the family, the appearance"),
            |k, _| k.ed.message = k.theme_line(),
        ),
        cmd(
            Spec::new("theme toggle")
                .doc("the other base — dark for light, light for dark — for the session"),
            |k, _| {
                let to = if k.dark { "light" } else { "dark" };
                k.pick_appearance(to);
            },
        ),
        cmd(
            Spec::new("theme system").doc("the base the OS's again, for the session"),
            |k, _| k.pick_appearance("system"),
        ),
        half(true),
        half(false),
        cmd(
            Spec::new("theme check")
                .args(Args::new(&[ArgKind::Text]))
                .doc("every pair of colours the editor draws, measured: the selected theme, a NAME, or all"),
            |k, ctx| k.theme_check(ctx.args.first().map(String::as_str)),
        ),
        cmd(
            Spec::new("theme reset").doc("the session's theme picks taken out, back to the files'"),
            |k, _| {
                for key in PICKED {
                    k.ed.settings.unset(Layer::Session, key);
                }
                k.ed.message = "theme: the files' again".into();
            },
        ),
    ];
    for f in themes::FAMILIES {
        theme.push(cmd(
            Spec::new(&format!("theme {}", f.name)).doc(&format!(
                "the {} family for the session: {} and {}",
                f.name, f.dark, f.light
            )),
            move |k, _| {
                for (key, v) in [
                    ("theme.name", f.name),
                    ("theme.dark", f.dark),
                    ("theme.light", f.light),
                ] {
                    k.ed.settings
                        .set(Layer::Session, key, Setting::Str(v.into()));
                }
                k.ed.message = format!("theme {}: {} and {}", f.name, f.dark, f.light);
            },
        ));
    }
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
    theme.extend([
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
    ]);
    theme
}
