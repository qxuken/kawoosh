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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use kawoosh_editor::Setting;
use kui_native::{FontId, Ui};

use crate::app::Kawoosh;
use crate::notify::{Level, Note};

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
    /// Read at the first frame, and again when the user's folder
    /// changed; the face kawoosh ships first, then by name. None before.
    pub families: Option<Vec<Family>>,
    /// The family kawoosh ships, when it loaded.
    pub bundled: Option<String>,
    pub shown: Shown,
    /// Counted up whenever `shown` changes.
    pub version: u64,
    /// Counted up whenever the families are read again, so a view
    /// holding a list of them knows to take it again.
    pub generation: u64,
    /// The families registered for a view, by name: every one, once a
    /// view asked.
    ids: HashMap<String, FontId>,
    /// A view asked for a family before they were registered.
    asked: bool,
    registered: bool,
    /// The families shaped once — their file read and parsed, what a
    /// first sight costs (14 ms each on the machine it was measured on)
    /// — so a view's text in them costs a frame nothing more.
    warm: HashSet<String>,
    /// Asked for since the last frame and not warm, in the order asked.
    cold: Vec<String>,
    /// The user's folder's files loaded, each with its handle.
    user: HashMap<PathBuf, FontId>,
    /// The watch saw the user's folder change: read it again at the
    /// frame.
    pub(crate) rescan: bool,
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

/// How long a frame spends warming families a view asked for: past it,
/// the rest wait for the next frame, which is asked for. One family is
/// warmed whatever its cost.
const WARM_BUDGET: Duration = Duration::from_millis(6);

/// What a family is warmed with: the characters a card shows.
const WARM_TEXT: &str = "fn greet(name: &str) -> String { let n = 0x1F; // O0 Il1 != => }";

/// The user's fonts: `$KAWOOSH_FONTS`, else `fonts/` beside
/// `settings.lua` — loaded at start, watched, a file dropped in or taken
/// out seen within a second (fonts.md Decision 7).
pub fn user_fonts_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KAWOOSH_FONTS") {
        return Some(PathBuf::from(p));
    }
    Some(crate::settings::config_dir()?.join("fonts"))
}

/// Whether `path` is the user's fonts folder or under it.
pub fn in_user_fonts(path: &Path) -> bool {
    user_fonts_dir().is_some_and(|d| path.starts_with(d))
}

/// What the watch keeps an eye on for the user's fonts: the folder and
/// every folder under it — a folder's stamp moves when an entry is added
/// or taken out, and the watch stats paths — or the folder alone, made
/// later, while there is none.
pub fn user_fonts_watch() -> Vec<PathBuf> {
    let Some(dir) = user_fonts_dir() else {
        return Vec::new();
    };
    let mut out = vec![dir.clone()];
    walk(&dir, &mut |p, is_dir| {
        if is_dir {
            out.push(p.to_path_buf());
        }
    });
    out
}

/// The font files under `dir`, sorted.
fn font_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, &mut |p, is_dir| {
        let font = p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        });
        if !is_dir && font {
            out.push(p.to_path_buf());
        }
    });
    out.sort();
    out
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path, bool)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        f(&p, is_dir);
        if is_dir {
            walk(&p, f);
        }
    }
}

impl Kawoosh {
    /// The user's folder read (at the first frame, and when the watch saw
    /// it change) and the families read again; every family registered
    /// once a view asked for one; and what a view asked for this frame
    /// warmed, within the budget — a frame asked for either way, so the
    /// view draws what is ready and asks again for the rest.
    pub(crate) fn sync_fonts(&mut self, ui: &mut Ui<'_>) {
        let mut notes = Vec::new();
        let mut rewatch = false;
        {
            let mut f = self.look.fonts.borrow_mut();
            if f.families.is_none() || f.rescan {
                let first = f.families.is_none();
                f.rescan = false;
                let (added, removed) = load_user_fonts(&mut f, ui);
                if !first && (!added.is_empty() || removed > 0) {
                    let mut said = Vec::new();
                    if !added.is_empty() {
                        said.push(format!("added {}", added.join(", ")));
                    }
                    if removed > 0 {
                        said.push(format!("{removed} files gone"));
                    }
                    notes.push(format!("fonts: {}", said.join(" · ")));
                }
                read_families(&mut f, ui, self.bundled_font);
                // Registered again at the next ask: a family taken out
                // shapes in a fallback, one added has no handle yet.
                f.ids.clear();
                f.registered = false;
                f.generation += 1;
                rewatch = !first;
                // A `font.family` that named a family not there yet.
                self.look.seen = None;
            }
            if f.asked && !f.registered {
                let t = Instant::now();
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
            if !f.cold.is_empty() {
                let t = Instant::now();
                for name in std::mem::take(&mut f.cold) {
                    if t.elapsed() > WARM_BUDGET {
                        break;
                    }
                    if let Some(&id) = f.ids.get(&name) {
                        let style = kui_native::TextStyle::new(14.0).font(id);
                        ui.measure_text(&format!("{name} {WARM_TEXT}"), &style, None);
                    }
                    f.warm.insert(name);
                }
                ui.request_frame();
            }
        }
        if rewatch {
            self.rewatch_config();
        }
        for n in notes {
            self.notify_with(Note::new(Level::Info, n).source("fonts"));
        }
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

/// The user's folder against what was loaded from it: a new file
/// loaded, a gone one's faces taken out. The families added, by name,
/// and how many files went.
fn load_user_fonts(f: &mut Fonts, ui: &mut Ui<'_>) -> (Vec<String>, usize) {
    let files = user_fonts_dir().map(|d| font_files(&d)).unwrap_or_default();
    let gone: Vec<PathBuf> = f
        .user
        .keys()
        .filter(|p| !files.contains(p))
        .cloned()
        .collect();
    for p in &gone {
        if let Some(id) = f.user.remove(p) {
            ui.core().remove_font(id);
        }
    }
    let mut added = Vec::new();
    for p in files {
        if f.user.contains_key(&p) {
            continue;
        }
        match ui.core().load_font_file(p.clone()) {
            Some(id) => {
                if let Some(name) = ui.core().font_family(id)
                    && !added.iter().any(|a| a == name)
                {
                    added.push(name.to_string());
                }
                f.user.insert(p, id);
            }
            None => log::warn!("fonts: no usable face in {}", p.display()),
        }
    }
    (added, gone.len())
}

/// The families kui can see, the OS's `.` ones left out but the face
/// kawoosh ships (`.IosevkaNavcon`), which comes first.
fn read_families(f: &mut Fonts, ui: &mut Ui<'_>, bundled_font: Option<FontId>) {
    let bundled = bundled_font.and_then(|id| ui.core().font_family(id).map(str::to_string));
    let mut all: Vec<Family> = ui
        .core()
        .system_fonts()
        .into_iter()
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

/// `kawoosh.fonts` (docs/design/fonts.md Decision 2): `families()`,
/// each `{ name, mono, weights, italic, bundled }`, the shipped face
/// first; `current()`, the face on show — `family`, `name`, `size`,
/// `line_height`, `row`, `cell`, `features`, `chrome`, `font` (a handle
/// for a text's `font =`), the family's `mono`, `weights` and `italic`,
/// `version`, and `generation` (counted up when the families are read
/// again); `face(NAME)`, a family's handle — nil until the frame after
/// the first ask registers them all, and until the family is warm: read
/// and shaped once, a few a frame within a budget, so a view that shows
/// many families for the first time draws each as it is ready rather
/// than stalling the frame on all of them.
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
            t.set("generation", f.generation)?;
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
            if !f.registered {
                f.asked = true;
                return Ok(None);
            }
            let Some(&id) = f.ids.get(&name) else {
                return Ok(None);
            };
            // Warm, or the face on show — the editor draws in it.
            if f.warm.contains(&name) || f.shown.id == Some(id) || f.shown.name == name {
                return Ok(Some(id.to_ffi() as i64));
            }
            if !f.cold.contains(&name) {
                f.cold.push(name);
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
