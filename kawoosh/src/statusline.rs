//! The status line under the panes as modules (roadmap step 69,
//! docs/design/statusline.md): kawoosh's own — the mode, a recording,
//! the path, the keys typed, the strip's marks, the selections, the
//! position, the percent — and the Lua ones `kawoosh.status` put at
//! `place = "statusline"`, placed by two lists, `statusline.left` and
//! `statusline.right`, `...` standing for the Lua ones no list names.
//! The path is relative to the working directory by default, and cut
//! from the left, a directory at a time, until it fits the room the
//! other modules leave it.

use std::path::Path;

use kawoosh_editor::{Mode, Setting, motions};
use kui_native::{Align, Color, CursorShape, NodeSpec, Span, Ui, Value};

use crate::app::Kawoosh;
use crate::chrome::{Block, cut_component};
use crate::layout::Content;
use crate::rows;

/// The lists when the settings say nothing, or not a list.
pub const LEFT: &[&str] = &["mode", "recording", "path", "keys"];
pub const RIGHT: &[&str] = &["...", "strip", "selections", "position", "percent"];

/// Where the Lua modules no list names are drawn.
const REST: &str = "...";

/// Between two left modules, px; the right ones are two cells apart.
const LEFT_GAP: f32 = 8.0;
/// The strip's padding at either end, px.
const PAD: f32 = 8.0;

/// What a module draws: its parts in their colours and what a click on
/// it runs. `path` marks the one fitted to the room left; `apart`
/// draws the parts as words that far apart rather than as one text.
struct Module {
    parts: Vec<(String, Color)>,
    run: Option<String>,
    path: bool,
    apart: Option<f32>,
}

impl Module {
    fn of(parts: Vec<(String, Color)>) -> Module {
        Module {
            parts,
            run: None,
            path: false,
            apart: None,
        }
    }

    fn text(t: impl Into<String>, c: Color) -> Module {
        Module::of(vec![(t.into(), c)])
    }

    fn is_empty(&self) -> bool {
        self.parts.iter().all(|(t, _)| t.is_empty())
    }

    fn joined(&self) -> String {
        self.parts.iter().map(|(t, _)| t.as_str()).collect()
    }
}

/// `path` fitted as far as `fits` allows, split as the part to dim and
/// the name: whole if it fits; else its directories cut as fish cuts
/// them ([`cut_component`]) one at a time from the left, so the ones
/// nearest the name go last; else the cut ones dropped from the left
/// behind `…`; else the name alone. `prefix` — a host's `box:` — leads
/// every answer but the last. A drive (`C:`) is kept whole, as the
/// title bar keeps it.
pub fn fit_path(prefix: &str, path: &str, mut fits: impl FnMut(&str) -> bool) -> (String, String) {
    use std::path::is_separator;
    let Some(at) = path.rfind(is_separator) else {
        return (prefix.to_string(), path.to_string());
    };
    let (head, name) = (&path[..=at], &path[at + 1..]);
    // Each directory with the separator written after it.
    let dirs: Vec<(&str, &str)> = head
        .split_inclusive(is_separator)
        .map(|p| {
            let sep = p.len() - p.chars().next_back().map_or(0, char::len_utf8);
            (&p[..sep], &p[sep..])
        })
        .collect();
    let cut = |i: usize, d: &str| {
        if i == 0 && d.len() > 1 && d.ends_with(':') {
            d.to_string()
        } else {
            cut_component(d)
        }
    };
    let n = dirs.len();
    let mut try_head = |h: String| {
        let whole = format!("{h}{name}");
        fits(&whole).then_some(h)
    };
    for k in 0..=n {
        let mut h = prefix.to_string();
        for (i, (d, sep)) in dirs.iter().enumerate() {
            h.push_str(&if i < k { cut(i, d) } else { d.to_string() });
            h.push_str(sep);
        }
        if k > 0 && dirs[k - 1].0 == cut(k - 1, dirs[k - 1].0) {
            continue; // Cut to what it was: the same as the one before.
        }
        if let Some(h) = try_head(h) {
            return (h, name.to_string());
        }
    }
    for j in 1..=n {
        let mut h = format!("{prefix}…{}", dirs[j - 1].1);
        for (i, (d, sep)) in dirs.iter().enumerate().skip(j) {
            h.push_str(&cut(i, d));
            h.push_str(sep);
        }
        if let Some(h) = try_head(h) {
            return (h, name.to_string());
        }
    }
    (String::new(), name.to_string())
}

impl Kawoosh {
    /// A list of module names from `statusline.<side>`, or its default.
    fn statusline_list(&self, side: &str, default: &[&str]) -> Vec<String> {
        match self.ed.settings.get(&format!("statusline.{side}")) {
            Some(Setting::List(l)) => l
                .iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect(),
            _ => default.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// The line: the left modules a cell apart, the right ones two, the
    /// path fitted to what the others leave of the window's width.
    pub(crate) fn status(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let style = rows::mono(self.chrome.face, &pal);
        let left = self.statusline_list("left", LEFT);
        let right = self.statusline_list("right", RIGHT);
        let mut lua: Vec<(String, Block)> = self.status_segments(ui, "statusline");
        let named = |n: &String| left.contains(n) || right.contains(n);
        // A Lua module named in a list is its own there, even while it
        // shows nothing: `position` taken by Lua is not `line:col`.
        let lua_names: Vec<String> = self
            .scripting
            .rt
            .as_ref()
            .map(|rt| rt.status_names("statusline"))
            .unwrap_or_default();
        let resolve = |names: &[String], lua: &mut Vec<(String, Block)>| -> Vec<Module> {
            let mut out = Vec::new();
            for n in names {
                if n == REST {
                    for (name, b) in lua.iter() {
                        if !named(name) {
                            let mut m = Module::of(b.parts.clone());
                            m.run = b.run.clone();
                            out.push(m);
                        }
                    }
                } else if lua_names.contains(n) {
                    if let Some(i) = lua.iter().position(|(name, _)| name == n) {
                        let (_, b) = lua.remove(i);
                        let mut m = Module::of(b.parts);
                        m.run = b.run;
                        out.push(m);
                    }
                } else if let Some(m) = self.module(n) {
                    out.push(m);
                }
            }
            out.retain(|m| !m.is_empty());
            out
        };
        let mut l = resolve(&left, &mut lua);
        let mut r = resolve(&right, &mut lua);
        let two = ui.measure_text("  ", &style, None).width;
        // The path's room: the width less the padding, every other
        // module, the gaps between them and a cell between the sides.
        if l.iter().chain(&r).any(|m| m.path) {
            let mut used = 2.0 * PAD + two / 2.0;
            for m in l.iter().chain(&r).filter(|m| !m.path) {
                used += ui.measure_text(&m.joined(), &style, None).width;
                used += m.apart.unwrap_or(0.0) * m.parts.len().saturating_sub(1) as f32;
            }
            used += LEFT_GAP * l.len().saturating_sub(1) as f32;
            used += two * r.len().saturating_sub(1) as f32;
            let room = ui.viewport().w - used;
            for m in l.iter_mut().chain(r.iter_mut()).filter(|m| m.path) {
                self.fit_path_module(ui, m, room, style);
            }
        }
        ui.with_keyed(
            "statusline",
            NodeSpec::row()
                .grow_width()
                .height(self.chrome.strip_h)
                .bg(pal.strip)
                .pad_xy(PAD, 0.0)
                .cross_align(Align::Center)
                .label("statusline"),
            |ui| {
                ui.with_keyed(
                    "left",
                    NodeSpec::row().gap(LEFT_GAP).cross_align(Align::Center),
                    |ui| draw_modules(ui, &l, style, &pal),
                );
                ui.leaf(NodeSpec::row().grow_width());
                ui.with_keyed(
                    "right",
                    NodeSpec::row().gap(two).cross_align(Align::Center),
                    |ui| draw_modules(ui, &r, style, &pal),
                );
            },
        );
    }

    /// The path module's parts fitted to `room` px: the head dim, the
    /// name as it was, what follows it (`[+]`) kept.
    fn fit_path_module(
        &self,
        ui: &mut Ui<'_>,
        m: &mut Module,
        room: f32,
        style: kui_native::TextStyle,
    ) {
        let [(prefix, _), (path, _), rest @ ..] = m.parts.as_slice() else {
            return;
        };
        let tail: String = rest.iter().map(|(t, _)| t.as_str()).collect();
        let (head, name) = fit_path(prefix, path, |s| {
            ui.measure_text(&format!("{s}{tail}"), &style, None).width <= room
        });
        let mut parts = vec![(head, self.pal.dim), (name, self.pal.fg)];
        parts.extend(rest.iter().cloned());
        m.parts = parts;
    }

    /// kawoosh's own module `name`, `None` for a name it has not.
    fn module(&self, name: &str) -> Option<Module> {
        let pal = self.pal;
        let view = self.focused_view();
        let none = || Some(Module::of(Vec::new()));
        match name {
            "mode" => Some(self.mode_module()),
            "recording" => Some(
                self.ed
                    .recording()
                    .map(|c| Module::text(format!("REC @{c}"), pal.insert))
                    .unwrap_or_else(|| Module::of(Vec::new())),
            ),
            "path" => {
                let Some(view) = view else { return none() };
                Some(self.path_module(view))
            }
            "keys" => {
                if view.is_none() {
                    return none();
                }
                let pending: String = self.ed.pending.join("");
                let op = self
                    .ed
                    .pending_op
                    .map(|(o, _)| o.chars().next().unwrap_or(' ').to_string())
                    .unwrap_or_default();
                let count = self.ed.count.map(|c| c.to_string()).unwrap_or_default();
                Some(Module::text(format!("{count}{op}{pending}"), pal.dim))
            }
            "strip" => Some(Module::text(self.strip_marks(), pal.dim)),
            "selections" => {
                let Some(view) = view else { return none() };
                let n = self.ed.views[view].sels.len();
                Some(Module::text(
                    if n > 1 {
                        format!("{n} sels")
                    } else {
                        String::new()
                    },
                    pal.dim,
                ))
            }
            "position" => {
                let Some(view) = view else { return none() };
                let v = &self.ed.views[view];
                let (ln, col) = motions::line_col(self.ed.buffer_of(view), v.sels.primary().head);
                Some(Module::text(format!("{}:{}", ln + 1, col + 1), pal.dim))
            }
            "percent" => {
                let Some(view) = view else { return none() };
                let v = &self.ed.views[view];
                let buf = self.ed.buffer_of(view);
                let (ln, _) = motions::line_col(buf, v.sels.primary().head);
                let pct = if buf.line_count() <= 1 {
                    100
                } else {
                    ln * 100 / (buf.line_count() - 1)
                };
                Some(Module::text(format!("{pct}%"), pal.dim))
            }
            _ => None,
        }
    }

    /// The mode the keyboard is in: the prompt's field while one is
    /// open, else the pane's; a pane without a buffer its kind, after
    /// its field's mode if it has one.
    fn mode_module(&self) -> Module {
        let pal = self.pal;
        let Some(view) = self.focused_view() else {
            let lua_name;
            let what = match self.layout.focused_content() {
                Some(Content::Undo) => "UNDO",
                Some(Content::Memory) => "MEMORY",
                Some(Content::Lua(n)) => {
                    lua_name = n.to_uppercase();
                    lua_name.as_str()
                }
                // A raw terminal says so: the keys are its program's
                // but the escape and ⌘ (terminal-keys.md Decision 2).
                // A `:!` whose line ended: its keys are normal mode's.
                Some(Content::Terminal(t)) if self.terms.done.contains_key(&t) => "DONE",
                Some(Content::Terminal(t)) if self.term_raw(t) => "RAW",
                _ => "TERM",
            };
            // A pane with a field, or the prompt over it: the field's
            // mode first, as a buffer pane shows its own: two words,
            // apart as two modules are.
            let mut parts = Vec::new();
            if let Some(kv) = self.keyed_view().map(|v| &self.ed.views[v]) {
                let (mode, color) = match kv.mode {
                    Mode::Insert => ("INS", pal.insert),
                    Mode::Visual if kv.visual_linewise => ("VIS LINE", pal.command),
                    Mode::Visual => ("VIS", pal.command),
                    _ => ("NOR", pal.accent),
                };
                parts.push((mode.to_string(), color));
            }
            parts.push((what.to_string(), pal.accent));
            let mut m = Module::of(parts);
            m.apart = Some(LEFT_GAP);
            return m;
        };
        let buf = self.ed.buffer_of(view);
        let on_toast = self.notes.focus.is_some();
        let keyed = self.ed.prompt_view().unwrap_or(view);
        let kv = &self.ed.views[keyed];
        // A terminal's copy mode is a mode of its own to the eye
        // (roadmap step 31): `COPY` where normal mode would say so.
        let copy = keyed == view && buf.language.as_ref() == "scrollback";
        let mode = if on_toast {
            "TOAST"
        } else if kv.mode == Mode::Visual && kv.visual_linewise {
            "VIS LINE"
        } else if copy && kv.mode == Mode::Normal {
            "COPY"
        } else {
            kv.mode.name()
        };
        let color = match kv.mode {
            _ if on_toast => pal.command,
            Mode::Insert => pal.insert,
            Mode::Visual => pal.command,
            _ => pal.accent,
        };
        Module::text(mode, color)
    }

    /// The buffer's file as `statusline.path` says, then `[+]` or how
    /// far it has come: parts `[prefix, path, tail…]` until fitted.
    fn path_module(&self, view: kawoosh_editor::ViewId) -> Module {
        let pal = self.pal;
        let buf = self.ed.buffer_of(view);
        let waiting_on = buf
            .path
            .as_deref()
            .and_then(kawoosh_systems::fs::domain_of)
            .map(|(d, _)| d)
            .filter(|d| buf.loading.is_some() && !kawoosh_doc::fs::is_registered(d));
        let tail = match buf.loading {
            // A session's file on a host not connected yet.
            Some(_) if waiting_on.is_some() => {
                format!(" [{}: :domain connect]", waiting_on.unwrap_or_default())
            }
            // Still on its way from the io thread: how far.
            Some((done, total)) => format!(
                " [opening {}%]",
                (done * 100).checked_div(total).unwrap_or(100)
            ),
            None if buf.modified => " [+]".into(),
            None => String::new(),
        };
        let how = self
            .ed
            .settings
            .str("statusline.path")
            .unwrap_or("relative");
        let (prefix, path) = match buf.path.as_deref() {
            Some(p) if how != "name" => self.shown_path(p, how == "relative"),
            _ => (String::new(), buf.name.clone()),
        };
        let mut m = Module::of(vec![(prefix, pal.dim), (path, pal.fg)]);
        if !tail.is_empty() {
            m.parts.push((tail, pal.fg));
        }
        m.path = true;
        m
    }

    /// `path` for the line, as a host's prefix (`box:`) and the rest:
    /// under the working directory and `relative`, relative to it;
    /// else whole, the home as `~`.
    fn shown_path(&self, path: &Path, relative: bool) -> (String, String) {
        if relative
            && let Ok(rel) = path.strip_prefix(&self.cwd)
            && !rel.as_os_str().is_empty()
        {
            return (String::new(), kawoosh_systems::fs::display(rel));
        }
        match kawoosh_systems::fs::domain_of(path) {
            Some((d, rest)) => (format!("{d}:"), kawoosh_systems::fs::display(rest)),
            None => (String::new(), kawoosh_systems::fs::abbreviate_home(path)),
        }
    }
}

/// Modules side by side in the row they are drawn in: one part a text,
/// more a rich text, a module with a command a button for it.
fn draw_modules(
    ui: &mut Ui<'_>,
    modules: &[Module],
    style: kui_native::TextStyle,
    pal: &crate::palette::Pal,
) {
    for (i, m) in modules.iter().enumerate() {
        let draw = |ui: &mut Ui<'_>| {
            let parts: Vec<&(String, Color)> =
                m.parts.iter().filter(|(t, _)| !t.is_empty()).collect();
            match (parts.as_slice(), m.apart) {
                ([(t, c)], _) => ui.text(t, style.color(*c)),
                (_, Some(gap)) => {
                    ui.with(NodeSpec::row().gap(gap), |ui| {
                        for (t, c) in &parts {
                            ui.text(t, style.color(*c));
                        }
                    });
                }
                _ => {
                    let spans: Vec<Span<'_>> =
                        parts.iter().map(|(t, c)| Span::new(t).color(*c)).collect();
                    ui.rich_text(&spans, style);
                }
            }
        };
        match &m.run {
            Some(run) => ui.with_keyed(
                &format!("m{i}"),
                NodeSpec::row()
                    .grow_height()
                    .cross_align(Align::Center)
                    .hover_bg(pal.panel)
                    .cursor(CursorShape::Pointer)
                    .on_click(Value::map([
                        ("kind", "chrome".into()),
                        ("run", Value::str(run.as_str())),
                    ]))
                    .label(run.as_str()),
                draw,
            ),
            None => ui.with_keyed(&format!("m{i}"), NodeSpec::row(), draw),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::fit_path;

    /// Fitted to `w` characters.
    fn fit(prefix: &str, p: &str, w: usize) -> String {
        let (a, b) = fit_path(prefix, p, |s| s.chars().count() <= w);
        a + &b
    }

    #[test]
    fn a_path_is_cut_from_the_left_only_as_far_as_it_must() {
        let p = "src/routes/users/index.tsx";
        assert_eq!(fit("", p, 80), p);
        assert_eq!(fit("", p, 25), "s/routes/users/index.tsx");
        assert_eq!(fit("", p, 20), "s/r/users/index.tsx");
        assert_eq!(fit("", p, 15), "s/r/u/index.tsx");
        assert_eq!(fit("", p, 14), "…/u/index.tsx", "cut ones dropped");
        assert_eq!(fit("", p, 12), "…/index.tsx");
        assert_eq!(fit("", p, 3), "index.tsx", "the name at the least");
        assert_eq!(fit("", "index.tsx", 3), "index.tsx");
        assert_eq!(
            fit("", "~/projects/kawoosh/.claude/x.rs", 20),
            "~/p/k/.claude/x.rs",
            "a home kept, a dot kept with its letter"
        );
        assert_eq!(
            fit("", "~/projects/kawoosh/.claude/x.rs", 14),
            "~/p/k/.c/x.rs"
        );
        assert_eq!(fit("", "/usr/local/bin/x", 12), "/u/l/bin/x");
        assert_eq!(
            fit("box:", "/home/me/src/app.ts", 18),
            "box:/h/m/s/app.ts",
            "a host kept in front"
        );
        let (head, name) = fit_path("", p, |s| s.len() <= 20);
        assert_eq!((head.as_str(), name.as_str()), ("s/r/users/", "index.tsx"));
    }
}
