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
//!   theme = { appearance = "dark", accent = "#e0af68", bg = "#1a1b26" },
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
//! - **`theme.appearance`** is `system` (the OS's), `dark` or `light`;
//!   `theme.accent` a colour; every other key under `theme` a role of
//!   kui's `Theme` by name (`bg`, `surface`, `fg`, `muted`, `selection`,
//!   `focus_ring`, `danger`, …). The OS's appearance with no role set
//!   keeps following the OS (`ThemeSource::Derived`, with the accent
//!   when one is given); an appearance named or a role set pins a
//!   palette (`ThemeSource::Pinned`): derived from the appearance and
//!   the accent, the roles written over it, and derived again when the
//!   OS flips under `system`.
//! - **`tokens.colors`** names a syntax token (`keyword`, `string`,
//!   `comment`, … — `Token::name`) and gives it one colour or a light
//!   and a dark half (`{ light, dark }` or `{ "#l", "#d" }`); a token
//!   not named keeps `palette.rs`'s hue. The same table, the defaults
//!   filled in, is declared as the host's kui tokens, so a Lua view
//!   paints `color = "$keyword"` and gets the frame's half. `kawoosh.
//!   colors { keyword = "ff0000" }` from code lands over the file's.

use std::collections::HashMap;

use kawoosh_editor::Setting;
use kawoosh_systems::ts::Token;
use kui::schema::THEME_ROLES;
use kui::{Appearance, Color, FontFeatures, FontId, Theme, ThemeSource, Tokens, Ui};

use crate::app::Kawoosh;
use crate::notify::{Level, Note};
use crate::rows::{FONT, LH};

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

/// The smallest and largest `font.size` honoured.
const SIZE_RANGE: (f32, f32) = (6.0, 96.0);
/// The default `font.line_height`, the ratio that makes 13 px rows of 20.
pub const LINE_HEIGHT: f64 = 1.5;

/// What the look was last built from, so a frame rebuilds it only when
/// the tree or the OS's appearance moved.
#[derive(Default)]
pub struct Look {
    /// The settings version the look was built from; none when
    /// something else asked for a rebuild (`kawoosh.colors`).
    pub seen: Option<u64>,
    /// The OS appearance the theme was resolved under.
    appearance: Appearance,
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
        let sys = ui.env().system.appearance;
        if self.look.seen == Some(v) && self.look.appearance == sys {
            return;
        }
        self.look.seen = Some(v);
        self.look.appearance = sys;
        let mut notes = Vec::new();
        self.sync_font(ui, &mut notes);
        self.sync_theme(ui, sys, &mut notes);
        self.sync_tokens(ui, &mut notes);
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
    }

    fn sync_theme(&mut self, ui: &mut Ui<'_>, sys: Appearance, notes: &mut Vec<String>) {
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
                if name == "appearance" || name == "accent" {
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
        let source = match (appearance, accent) {
            (None, None) if roles.is_empty() => ThemeSource::Derived,
            (None, Some(c)) if roles.is_empty() => ThemeSource::DerivedWithAccent(c),
            (a, c) => {
                let mut t = Theme::derive(a.unwrap_or(sys), c);
                for (role, c) in roles {
                    (role.set)(&mut t, c);
                }
                ThemeSource::Pinned(t)
            }
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
                    crate::palette::syntax_color(*t, false)?,
                    crate::palette::syntax_color(*t, true)?,
                ))
            });
            if let Some((light, dark)) = halves {
                tokens = tokens.color_themed(t.name(), light, dark);
            }
        }
        ui.set_tokens(tokens);
        self.look.syntax = syntax;
    }

    /// The syntax colour for a token on the theme's base: the config's
    /// half, else `palette.rs`'s hue.
    pub fn syntax_color_for(&self, token: Token, dark: bool) -> Option<Color> {
        if let Some(c) = self.scripting.colors.get(&token) {
            return Some(*c);
        }
        match self.look.syntax.get(&token) {
            Some((l, d)) => Some(if dark { *d } else { *l }),
            None => crate::palette::syntax_color(token, dark),
        }
    }
}
