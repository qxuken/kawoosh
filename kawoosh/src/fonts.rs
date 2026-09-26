//! The fonts as data (docs/design/fonts.md): the families kui can see,
//! what each is (kui's `system_fonts`: monospaced, its weights, an
//! italic), the face on show, and the handles a Lua view draws a family
//! with — `kawoosh.fonts`, the door the fonts' pane and the lab read —
//! and `:font`, `:font NAME`.
//!
//! The families are registered with kui when a view first asks for one
//! (`kawoosh.fonts.face`) — every family at once, at the next frame,
//! which is asked for: a few milliseconds for six hundred, where one at
//! a time drew each card the pane scrolled to in the wrong face for a
//! frame, a flicker. Until then `face` answers nil; nobody asking, none
//! is registered.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use kawoosh_editor::Setting;
use kui_native::{FontId, Ui};

use crate::app::Kawoosh;

/// One family as the door says it.
#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    pub name: String,
    /// Every face fixed-pitch, as the font says.
    pub mono: bool,
    /// The weights its faces come in, sorted.
    pub weights: Vec<u16>,
    pub italic: bool,
}

/// The face on show, as the door says it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    /// `font.family` as set; empty for the face kawoosh ships.
    pub family: String,
    /// The family it resolved to; empty where kui's generic mono stands.
    pub name: String,
    pub size: f32,
    /// `font.line_height`, the ratio.
    pub line_height: f32,
    /// The row's height, px.
    pub row: f32,
    /// A cell's width, px.
    pub cell: f32,
    /// `font.features` as set.
    pub features: String,
    /// The chrome's text size.
    pub chrome: f32,
    pub id: Option<FontId>,
}

/// The families, the face, and the handles — shared with the Lua door.
#[derive(Debug, Default)]
pub struct Fonts {
    /// Read once, at the first frame; the face kawoosh ships first,
    /// then by name. None before.
    pub families: Option<Vec<Family>>,
    /// The family kawoosh ships, when it loaded.
    pub bundled: Option<String>,
    pub shown: Shown,
    /// Counted up whenever `shown` changes.
    pub version: u64,
    /// The families registered for a view, by name: every one, once a
    /// view asked.
    ids: HashMap<String, FontId>,
    /// A view asked for a family before they were registered.
    asked: bool,
    registered: bool,
}

impl Fonts {
    /// The family by its name.
    pub fn family(&self, name: &str) -> Option<&Family> {
        self.families.as_ref()?.iter().find(|f| f.name == name)
    }

    /// The names, the monospaced first — what `font.family` completes
    /// to.
    pub fn names(&self) -> Vec<String> {
        let all = self.families.as_deref().unwrap_or_default();
        let (mono, rest): (Vec<&Family>, Vec<&Family>) = all.iter().partition(|f| f.mono);
        mono.into_iter()
            .chain(rest)
            .map(|f| f.name.clone())
            .collect()
    }
}

pub type SharedFonts = Rc<RefCell<Fonts>>;

impl Kawoosh {
    /// The families read, once; every one registered once a view asked
    /// for one, and a frame asked for so it draws with them.
    pub(crate) fn sync_fonts(&mut self, ui: &mut Ui<'_>) {
        let mut f = self.look.fonts.borrow_mut();
        if f.families.is_none() {
            let bundled = self
                .bundled_font
                .and_then(|id| ui.core().font_family(id).map(str::to_string));
            let mut all: Vec<Family> = ui
                .core()
                .system_fonts()
                .into_iter()
                // The OS's own (`.SF NS`) left out; the face kawoosh ships is
                // loaded under a `.` name too (`.IosevkaNavcon`), and kept.
                .filter(|s| !s.family.starts_with('.') || bundled.as_ref() == Some(&s.family))
                .map(|s| Family {
                    name: s.family,
                    mono: s.monospaced,
                    weights: s.weights,
                    italic: s.italic,
                })
                .collect();
            if let Some(b) = &bundled
                && let Some(at) = all.iter().position(|f| &f.name == b)
            {
                let own = all.remove(at);
                all.insert(0, own);
            }
            f.families = Some(all);
            f.bundled = bundled;
        }
        if !f.asked || f.registered {
            return;
        }
        let t = std::time::Instant::now();
        let names: Vec<String> = f
            .families
            .iter()
            .flatten()
            .map(|x| x.name.clone())
            .collect();
        for name in names {
            let id = if f.bundled.as_deref() == Some(name.as_str()) {
                self.bundled_font
            } else {
                ui.core().add_system_font(&name)
            };
            if let Some(id) = id {
                f.ids.insert(name, id);
            }
        }
        f.registered = true;
        log::debug!(
            "registered {} font families in {:?}",
            f.ids.len(),
            t.elapsed()
        );
        ui.request_frame();
    }

    /// The face on show into the door, after the look was built or the
    /// cell measured.
    pub(crate) fn publish_face(&mut self) {
        let s = &self.ed.settings;
        let family = s.str("font.family").unwrap_or("").trim().to_string();
        let mut f = self.look.fonts.borrow_mut();
        let name = match self.face.id {
            Some(id) if Some(id) == self.bundled_font => f.bundled.clone().unwrap_or_default(),
            Some(_) if f.family(&family).is_some() => family.clone(),
            // A family that did not resolve keeps the face it had.
            Some(_) => f.shown.name.clone(),
            None => String::new(),
        };
        let shown = Shown {
            family,
            name,
            size: self.face.size,
            line_height: s
                .get("font.line_height")
                .and_then(Setting::as_float)
                .unwrap_or(crate::look::LINE_HEIGHT) as f32,
            row: self.face.line_height,
            cell: self.cell.0,
            features: s.str("font.features").unwrap_or("").to_string(),
            chrome: self.chrome.face.size,
            id: self.face.id,
        };
        if f.shown != shown {
            f.shown = shown;
            f.version += 1;
        }
    }

    /// `:font`'s line: the face on show.
    fn font_line(&self) -> String {
        let f = self.look.fonts.borrow();
        let s = &f.shown;
        let name = if s.name.is_empty() {
            "kui's mono"
        } else {
            &s.name
        };
        let own = if s.family.is_empty() {
            " (kawoosh's)"
        } else {
            ""
        };
        let features = if s.features.is_empty() {
            String::new()
        } else {
            format!(" · features {}", s.features)
        };
        format!(
            "font {name}{own} · {} px · row {} px ({}×){features}",
            s.size, s.row, s.line_height
        )
    }

    /// `:font NAME`: the family for the session — the shipped face's
    /// name is the empty string — or a message saying there is none.
    fn pick_font(&mut self, name: &str) {
        use kawoosh_editor::Layer;
        let (known, bundled) = {
            let f = self.look.fonts.borrow();
            (f.family(name).is_some(), f.bundled.clone())
        };
        if !known {
            self.ed.message = format!("font: no family \"{name}\" (:fonts lists them)");
            return;
        }
        let value = if bundled.as_deref() == Some(name) {
            ""
        } else {
            name
        };
        self.ed
            .settings
            .set(Layer::Session, "font.family", Setting::Str(value.into()));
        self.ed.message = format!("font.family = {name}");
    }
}

/// `kawoosh.fonts` (docs/design/fonts.md Decision 2): `families()`,
/// each `{ name, mono, weights, italic, bundled }`, the shipped face
/// first; `current()`, the face on show — `family`, `name`, `size`,
/// `line_height`, `row`, `cell`, `features`, `chrome`, `font` (a handle
/// for a text's `font =`), the family's `mono`, `weights` and `italic`,
/// and `version`; `face(NAME)`, a family's handle, nil until the frame
/// after the first ask registers them all.
pub(crate) fn lua_door(lua: &mlua::Lua, fonts: SharedFonts) -> mlua::Result<()> {
    let door = lua.create_table()?;
    let family_table =
        |lua: &mlua::Lua, fam: &Family, bundled: bool| -> mlua::Result<mlua::Table> {
            let t = lua.create_table()?;
            t.set("name", fam.name.as_str())?;
            t.set("mono", fam.mono)?;
            t.set(
                "weights",
                lua.create_sequence_from(fam.weights.iter().copied())?,
            )?;
            t.set("italic", fam.italic)?;
            t.set("bundled", bundled)?;
            Ok(t)
        };
    let at = fonts.clone();
    door.set(
        "families",
        lua.create_function(move |lua, ()| {
            let f = at.borrow();
            let out = lua.create_table()?;
            for (i, fam) in f.families.iter().flatten().enumerate() {
                let own = f.bundled.as_deref() == Some(fam.name.as_str());
                out.set(i + 1, family_table(lua, fam, own)?)?;
            }
            Ok(out)
        })?,
    )?;
    let at = fonts.clone();
    door.set(
        "current",
        lua.create_function(move |lua, ()| {
            let f = at.borrow();
            let s = &f.shown;
            let t = lua.create_table()?;
            t.set("family", s.family.as_str())?;
            t.set("name", s.name.as_str())?;
            t.set("size", s.size)?;
            t.set("line_height", s.line_height)?;
            t.set("row", s.row)?;
            t.set("cell", s.cell)?;
            t.set("features", s.features.as_str())?;
            t.set("chrome", s.chrome)?;
            t.set("font", s.id.map(|id| id.to_ffi() as i64))?;
            t.set("version", f.version)?;
            if let Some(fam) = f.family(&s.name) {
                t.set("mono", fam.mono)?;
                t.set(
                    "weights",
                    lua.create_sequence_from(fam.weights.iter().copied())?,
                )?;
                t.set("italic", fam.italic)?;
            }
            Ok(t)
        })?,
    )?;
    door.set(
        "face",
        lua.create_function(move |_, name: String| {
            let mut f = fonts.borrow_mut();
            if let Some(id) = f.ids.get(&name) {
                return Ok(Some(id.to_ffi() as i64));
            }
            if !f.registered {
                f.asked = true;
            }
            Ok(None)
        })?,
    )?;
    lua.globals()
        .get::<mlua::Table>("kawoosh")?
        .set("fonts", door)
}

/// `font` (docs/design/fonts.md Decision 5) says the face on show;
/// `font NAME` takes a family for the session.
pub(crate) fn commands() -> Vec<crate::commands::ShellCommand> {
    use crate::commands::cmd;
    use kawoosh_editor::{ArgKind, Args, Spec};
    vec![cmd(
        Spec::new("font")
            .args(Args::rest(&[ArgKind::Font]))
            .doc("the face on show; with a NAME, that family for the session"),
        |k, ctx| {
            let name = ctx.args.join(" ");
            if name.is_empty() {
                k.ed.message = k.font_line();
            } else {
                k.pick_font(&name);
            }
        },
    )]
}
