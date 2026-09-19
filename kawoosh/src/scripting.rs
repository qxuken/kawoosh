//! The Lua runtime's shell side (milestone 7): `init.lua`, commands and
//! keymaps registered from Lua, Lua views as panes (a kui slot each),
//! scratch buffers with `on_write`, tools, and the messages Lua queues
//! applied where only the shell can (mvp.md D8, kui.md D6).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use kawoosh_doc::Buffer;
use kawoosh_editor::{ArgKind, Args, Cond, KeyStroke, Lookup, Mode, Spec, ViewId};
use kawoosh_lua::{Msg, Runtime};
use kawoosh_systems::lsp::ServerDef;
use kawoosh_systems::ts::Token;
use kui::{Color, NodeSpec, Sizing, Ui, Value};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, PaneId, SplitDir};
use crate::notify::{Level, Note, Show, Ttl};

#[derive(Clone, Debug)]
pub struct ToolDef {
    pub cmd: String,
    pub cwd: Option<String>,
    pub dock: bool,
}

#[derive(Default)]
pub struct Scripting {
    pub rt: Option<Rc<Runtime>>,
    pub tools: HashMap<String, ToolDef>,
    /// A tool's running terminal, by tool name.
    pub tool_terms: HashMap<String, u64>,
    pub servers: Vec<ServerDef>,
    /// Syntax colours a config set, by token class.
    pub colors: HashMap<Token, Color>,
    /// `<C-w>` pressed in a Lua pane.
    pub prefix: bool,
}

impl Kawoosh {
    /// Installs the runtime and returns the extension for the launcher
    /// (`extension_as("lua", ..)`). The bundled plugins load with it.
    pub fn attach_lua(&mut self) -> Result<kui_lua::LuaExtension, String> {
        let (rt, ext) = Runtime::new().map_err(|e| e.to_string())?;
        let rt = Rc::new(rt);
        self.scripting.rt = Some(rt.clone());
        for (name, src) in crate::plugins::BUNDLED {
            if let Err(e) = rt.load_source(name, src) {
                log::error!("{name}: {e}");
                self.ed.message = format!("{name}: {e}");
            }
        }
        self.drain_lua();
        Ok(ext)
    }

    /// The config, in layers (kui.md D10): the user's `settings.lua`,
    /// then `init.lua` — which can read it — then the project's
    /// `.kawoosh/settings.lua` files over both; and the files on the
    /// watch, so a save reloads its layer.
    pub fn load_config(&mut self) {
        if let Some(p) = crate::settings::user_settings_path() {
            self.config.user = Some(p.clone());
            if p.is_file() {
                self.load_user_settings(&p);
            }
        }
        if let Some(p) = crate::settings::config_path() {
            self.config.init = Some(p.clone());
            if p.is_file() {
                self.run_init(&p);
            }
        }
        self.reload_project_settings();
        self.rewatch_config();
    }

    pub fn run_lua_file(&mut self, path: &Path) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        rt.publish(&self.ed, self.focused_view());
        if let Err(e) = rt.load_file(path) {
            self.ed.message = e.lines().next().unwrap_or("lua error").to_string();
            log::error!("{e}");
        }
        self.drain_lua();
    }

    pub fn run_lua_source(&mut self, name: &str, src: &str) {
        let Some(rt) = self.scripting.rt.clone() else {
            self.ed.message = "lua is not available".into();
            return;
        };
        rt.publish(&self.ed, self.focused_view());
        if let Err(e) = rt.load_source(name, src) {
            self.ed.message = e.lines().next().unwrap_or("lua error").to_string();
        }
        self.drain_lua();
    }

    /// Applies everything Lua queued since the last drain.
    pub(crate) fn drain_lua(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        self.sync_facts();
        let msgs = rt.take_msgs();
        if msgs.is_empty() {
            return;
        }
        let view = self.focused_view().or_else(|| self.ed.any_view());
        let rest = match view {
            Some(v) => Runtime::apply_editor_msgs(&mut self.ed, v, msgs),
            None => msgs,
        };
        for m in rest {
            self.apply_lua_msg(&rt, m);
        }
        self.drain_effects();
    }

    fn apply_lua_msg(&mut self, rt: &Rc<Runtime>, m: Msg) {
        match m {
            Msg::RegisterCommand(spec) => {
                let rt = rt.clone();
                let cmd_name = spec.name.clone();
                self.ed.register_spec(spec, move |ed, ctx| {
                    rt.publish(ed, Some(ctx.view));
                    rt.run_command(&cmd_name, ctx);
                    let msgs = rt.take_msgs();
                    let rest = Runtime::apply_editor_msgs(ed, ctx.view, msgs);
                    for m in rest {
                        rt.push(m);
                    }
                });
            }
            Msg::Map {
                mode,
                keys,
                command,
                when,
            } => match Mode::from_short(&mode) {
                Some(m) => {
                    let when: Vec<Cond> = when.iter().map(|c| Cond::parse(c)).collect();
                    self.ed.keymap.bind_when(m, &keys, &command, &when);
                }
                None => self.ed.message = format!("map: unknown mode {mode}"),
            },
            Msg::Open(p) => self.open_in_editor(&p, None, None),
            Msg::OpenScratch {
                name,
                text,
                hooked,
                read_only,
                language,
                reuse,
                line,
            } => {
                let existing = self
                    .ed
                    .buffers
                    .iter()
                    .find(|(_, b)| b.name == name)
                    .map(|(id, _)| id);
                // A scratch buffer handed back (`reuse`) becomes this one
                // — renamed and refilled — so a listing that moves to the
                // next directory leaves no buffer behind per directory.
                let reused = existing.or_else(|| {
                    reuse
                        .map(kawoosh_lua::id_of)
                        .filter(|id| self.ed.buffers.get(*id).is_some_and(|b| b.path.is_none()))
                });
                let id = match reused {
                    Some(id) => {
                        let b = &mut self.ed.buffers[id];
                        b.set_text(&text);
                        b.mark_saved();
                        if existing.is_none() {
                            b.name = name.clone();
                            b.read_only = read_only;
                            b.hook = hooked.then(|| name.clone());
                            if let Some(l) = language {
                                b.language = l.into();
                            }
                            // Another listing now: where the caret was in
                            // the last one is not where it goes in this.
                            self.last_pos.remove(&id);
                        }
                        id
                    }
                    None => {
                        let mut b = Buffer::new(name.clone(), &text);
                        b.read_only = read_only;
                        if hooked {
                            b.hook = Some(name.clone());
                        }
                        if let Some(l) = language {
                            b.language = l.into();
                        }
                        self.ed.add_buffer(b)
                    }
                };
                if hooked {
                    rt.track_lines(&self.ed, id);
                }
                match self.focused_view() {
                    Some(v) => self.show_buffer(v, id),
                    None => {
                        let v = self.ed.add_view(id);
                        self.layout.split(SplitDir::H, Content::Editor(v));
                    }
                }
                // The caret on the line asked for, else at the top: a
                // refilled buffer is new text, whatever the caret was in
                // the old.
                if let Some(v) = self.focused_view() {
                    let buf = self.ed.buffer_of(v);
                    let ln = line.unwrap_or(1).max(1).min(buf.line_count()) - 1;
                    let off = buf.line_start(ln);
                    self.ed.views[v].sels =
                        kawoosh_editor::Selections::single(kawoosh_editor::Selection::point(off));
                    self.ed.views[v].goal_col = None;
                }
            }
            Msg::OpenView(name) => self.open_lua_view(&name),
            Msg::Tool {
                name,
                cmd,
                cwd,
                dock,
            } => {
                self.scripting
                    .tools
                    .insert(name, ToolDef { cmd, cwd, dock });
            }
            Msg::Compile(cmd) => self.compile(&cmd),
            Msg::Notify {
                level,
                source,
                text,
                show,
                timeout,
                actions,
            } => {
                let level = match level.as_deref() {
                    None => Level::Info,
                    Some(l) => match Level::parse(l) {
                        Some(l) => l,
                        None => {
                            self.ed.message = format!("notify: unknown level {l}");
                            return;
                        }
                    },
                };
                let mut note = Note::new(level, text);
                if let Some(s) = source {
                    note = note.source(s);
                }
                note.show = match show.as_deref() {
                    None => None,
                    Some("toast") => Some(Show::Toast),
                    Some("corner") => Some(Show::Corner),
                    Some("log") => Some(Show::Log),
                    Some(other) => {
                        self.ed.message = format!("notify: unknown show {other}");
                        return;
                    }
                };
                note.ttl = match timeout {
                    None => Ttl::Default,
                    Some(ms) if ms <= 0.0 => Ttl::Never,
                    Some(ms) => Ttl::After(std::time::Duration::from_millis(ms as u64)),
                };
                for (label, command) in actions {
                    note = note.action(label, command);
                }
                self.notify_with(note);
            }
            Msg::LspServer {
                language,
                command,
                args,
                roots,
            } => {
                self.add_lsp_server(ServerDef {
                    language,
                    command,
                    args,
                    roots,
                });
            }
            Msg::Language {
                name,
                aliases,
                extensions,
                filenames,
                shebangs,
                path,
                symbol,
                highlights,
                injections,
            } => self.language_from_lua(
                name, aliases, extensions, filenames, shebangs, path, symbol, highlights,
                injections,
            ),
            Msg::Colors(list) => {
                for (name, hex) in list {
                    let tok = Token::ALL.iter().copied().find(|t| t.name() == name);
                    let color = hex.trim_start_matches('#').parse_hex();
                    match (tok, color) {
                        (Some(t), Some(c)) => {
                            self.scripting.colors.insert(t, c);
                        }
                        _ => self.ed.message = format!("colors: bad entry {name} = {hex}"),
                    }
                }
            }
            // What `init.lua` sets is the user's layer, under a project's
            // files; what runs later is the session's, over them.
            Msg::Option { path, value } => {
                let layer = if self.config.loading {
                    kawoosh_editor::Layer::User
                } else {
                    kawoosh_editor::Layer::Session
                };
                match value {
                    Some(v) => self.ed.settings.set(layer, &path, v),
                    None => self.ed.settings.unset(layer, &path),
                }
            }
            Msg::Chdir(p) => self.set_cwd(&p),
            // A fact needs no view: published even before there is one.
            Msg::Fact { name, on } => self.ed.fact(&name, on),
            Msg::FieldOpen(name) => {
                if self.ed.find_field(&name).is_none() {
                    self.ed.open_field(&name, "");
                }
            }
            Msg::FieldFocus { view, field } => {
                // A field focused before its first draw is opened here,
                // as `field_set` opens one.
                if let Some(f) = &field
                    && self.ed.find_field(f).is_none()
                {
                    self.ed.open_field(f, "");
                }
                rt.set_field_focus(&view, field);
            }
            Msg::FieldSet { name, text } => {
                let v = match self.ed.find_field(&name) {
                    Some(v) => v,
                    None => self.ed.open_field(&name, ""),
                };
                self.ed.set_field_text(v, &text);
            }
            Msg::Edit { .. }
            | Msg::SetText { .. }
            | Msg::SetCursor { .. }
            | Msg::Echo(_)
            | Msg::Ex(_) => {
                // Editor messages already applied; here only when there
                // was no view at all.
            }
        }
    }

    /// Opens Lua view `name` in a split (or focuses its pane).
    pub fn open_lua_view(&mut self, name: &str) {
        let shown = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| matches!(self.layout.content(*p), Some(Content::Lua(n)) if n == name));
        match shown {
            Some(p) => self.layout.focus(p),
            None => {
                self.layout
                    .split(SplitDir::H, Content::Lua(name.to_string()));
            }
        }
    }

    /// `:tool NAME`: opens the tool's terminal (dock or split), or
    /// focuses it, or toggles the dock away when it is already focused.
    pub(crate) fn tool(&mut self, name: &str) {
        let Some(def) = self.scripting.tools.get(name).cloned() else {
            self.ed.message = format!("no tool named {name}");
            return;
        };
        if let Some(&t) = self.scripting.tool_terms.get(name)
            && self.terms.map.contains_key(&t)
        {
            let pane = self
                .layout
                .all_panes()
                .into_iter()
                .find(|p| self.term_of(*p) == Some(t));
            match pane {
                Some(p) if self.layout.dock == Some(p) => {
                    if self.layout.dock_open && self.layout.focused() == p {
                        self.layout.dock_open = false;
                        self.layout.dock_focused = false;
                    } else {
                        self.layout.dock_open = true;
                        self.layout.focus(p);
                    }
                }
                Some(p) => self.layout.focus(p),
                None => {}
            }
            return;
        }
        let cwd = def
            .cwd
            .as_deref()
            .map(|c| match c {
                "root" | "cwd" => std::env::current_dir().unwrap_or_default(),
                other => PathBuf::from(other),
            })
            .or_else(|| {
                self.focused_view()
                    .and_then(|v| self.ed.buffer_of(v).path.clone())
                    .and_then(|p| p.parent().map(Path::to_path_buf))
            });
        let Some(t) = self.spawn_terminal(Some(&def.cmd), cwd.as_deref()) else {
            return;
        };
        self.scripting.tool_terms.insert(name.to_string(), t);
        if def.dock {
            if let Some(old) = self.layout.dock.take()
                && let Some(Content::Terminal(ot)) = self.layout.close(old)
            {
                self.terms.map.remove(&ot);
            }
            let p = self.layout.new_pane(Content::Terminal(t));
            self.layout.dock = Some(p);
            self.layout.dock_open = true;
            self.layout.dock_focused = true;
        } else {
            self.layout.split(SplitDir::V, Content::Terminal(t));
        }
    }

    /// A key in a focused Lua pane: the pane prefix, else the view's
    /// `on_event` as `{kind="key", ...}`.
    pub(crate) fn lua_pane_key(&mut self, name: &str, stroke: KeyStroke) {
        // A field of the view with the keys: the editor's own — insert
        // mode types, `<Esc>` is normal mode over the line, `<C-w>l` in
        // it moves panes as everywhere — until `field blur` (`<Esc>` in
        // normal mode) hands them back to the view.
        if let Some(f) = self.lua_field_focused(name) {
            self.ed.key(f, stroke);
            self.drain_effects();
            self.drain_lua();
            return;
        }
        let note = stroke.notation();
        if self.scripting.prefix {
            self.scripting.prefix = false;
            if note == ":" {
                self.open_cmdline();
                return;
            }
            let keys = ["<C-w>".to_string(), note];
            self.ed.sync_settings();
            if let Lookup::Exact(bs) = self.ed.keymap.lookup_lenient(Mode::Normal, &keys) {
                let bs = bs.to_vec();
                self.run_bindings(&bs);
            }
            return;
        }
        if note == "<C-w>" {
            self.scripting.prefix = true;
            return;
        }
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        rt.publish(&self.ed, self.focused_view());
        if let Ok(f) = rt
            .lua()
            .globals()
            .get::<mlua::Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_key"))
        {
            let t = rt.lua().create_table().unwrap();
            let _ = t.set("kind", "key");
            let _ = t.set("code", stroke.code.clone());
            let _ = t.set("key", note);
            let _ = t.set("text", stroke.text.clone());
            let _ = t.set("ctrl", stroke.ctrl);
            let _ = t.set("alt", stroke.alt);
            let _ = t.set("shift", stroke.shift);
            if let Err(e) = f.call::<()>((name, t)) {
                self.ed.message = format!("{name}: {e}");
            }
        }
        self.drain_lua();
    }

    /// The field the view `name`'s keys are on, if it has one and the
    /// field is still open.
    pub fn lua_field_focused(&self, name: &str) -> Option<ViewId> {
        let rt = self.scripting.rt.as_ref()?;
        let f = rt.field_focus(name)?;
        self.ed.find_field(&f)
    }

    /// `field blur`: the keys back from the field `view` to the Lua
    /// view that drew it (`lua:<view>/<name>`).
    fn blur_lua_field(&mut self, view: ViewId) {
        let Some(name) = self.ed.field_name(view) else {
            return;
        };
        let Some(rest) = name.strip_prefix("lua:") else {
            return;
        };
        let Some((owner, _)) = rest.rsplit_once('/') else {
            return;
        };
        let owner = owner.to_string();
        if let Some(rt) = &self.scripting.rt {
            rt.set_field_focus(&owner, None);
        }
    }

    pub(crate) fn render_lua_pane(
        &mut self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        name: &str,
        focused: bool,
    ) {
        let rect = self.layout.rects.get(&pane).copied();
        let params = Value::map([
            ("pane", Value::Int(pane as i64)),
            ("focused", Value::Bool(focused)),
            (
                "width",
                Value::Float(rect.map(|r| r.w as f64).unwrap_or(0.0)),
            ),
            (
                "height",
                Value::Float(rect.map(|r| r.h as f64).unwrap_or(0.0)),
            ),
        ]);
        let tag = Value::map([
            ("kind", "luapane".into()),
            ("pane", Value::Int(pane as i64)),
        ]);
        let sink = ui.with_keyed(
            "lua",
            NodeSpec::column()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .clip()
                .on_key(tag.clone())
                // A click the view's own nodes do not take focuses the
                // pane.
                .on_click(tag),
            |ui| {
                ui.slot_with(&format!("lua/{name}@{pane}"), &params);
            },
        );
        if focused {
            self.focus_sink(ui, sink);
        }
    }

    pub fn lua_name_of(&self, pane: PaneId) -> Option<String> {
        match self.layout.content(pane) {
            Some(Content::Lua(n)) => Some(n),
            _ => None,
        }
    }

    /// A view's `focused` param and the shell's colours: the current
    /// syntax colour for a token, config first.
    pub(crate) fn syntax_color_for(&self, token: Token, dark: bool) -> Option<Color> {
        self.scripting
            .colors
            .get(&token)
            .copied()
            .or_else(|| crate::palette::syntax_color(token, dark))
    }

    /// The shell's part of `:w` on a hooked scratch buffer.
    pub(crate) fn write_hooked(&mut self, buffer: kawoosh_doc::BufferId, view: ViewId) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let Some(b) = self.ed.buffers.get(buffer) else {
            return;
        };
        let (Some(hook), text) = (b.hook.clone(), b.text()) else {
            return;
        };
        rt.publish(&self.ed, Some(view));
        rt.write_hook(&hook, &text);
        // The hook rewrote the listing (`kawoosh.buf.set_text`, queued):
        // apply that first, then track the lines as they now are — a
        // tracking of the old text would call every line of the next
        // write renamed.
        self.drain_lua();
        if let Some(rt) = self.scripting.rt.clone() {
            rt.track_lines(&self.ed, buffer);
        }
    }
}

trait ParseHex {
    fn parse_hex(&self) -> Option<Color>;
}

impl ParseHex for str {
    fn parse_hex(&self) -> Option<Color> {
        let v = u32::from_str_radix(self, 16).ok()?;
        Some(match self.len() {
            6 => Color::hex((v << 8) | 0xFF),
            8 => Color::hex(v),
            _ => return None,
        })
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("tool")
                .args(Args::new(&[ArgKind::Tool]))
                .query("list the tools")
                .doc("run the tool NAME the config registered; bare, list them"),
            |k, ctx| match ctx.args.first().filter(|_| !ctx.query()) {
                Some(n) => k.tool(n),
                None => {
                    let mut names: Vec<&String> = k.scripting.tools.keys().collect();
                    names.sort();
                    k.ed.message = format!(
                        "tools: {}",
                        names
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                }
            },
        ),
        cmd(
            Spec::new("view")
                .args(Args::new(&[ArgKind::View]))
                .doc("open the Lua view NAME in a pane"),
            |k, ctx| match ctx.args.first() {
                Some(n) => k.open_lua_view(n),
                None => k.ed.message = "view what?".into(),
            },
        ),
        cmd(
            Spec::new("field blur")
                .when(&["field", "!prompt", "!field:commands"])
                .doc("the keys back from a view's field to the view"),
            |k, ctx| k.blur_lua_field(ctx.view),
        ),
        cmd(
            Spec::new("lua")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("run CODE in the Lua state"),
            |k, ctx| {
                let src = ctx.args.join(" ");
                k.run_lua_source("<lua>", &src);
            },
        ),
    ]
}
