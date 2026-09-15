//! The `kui::App`: the pane tree (milestone 3) with the modal editor in
//! every editor pane (milestone 2). The engine (`kawoosh-editor`) owns
//! buffers, views, selections, modes and the keymap; the layout owns
//! which view is where; this file draws both and routes kui's events —
//! keys to the focused pane, the mouse by the pane it landed in, and the
//! engine's effects to what only the shell can do.

use std::ops::Range;
use std::path::{Path, PathBuf};

use kawoosh_doc::{Buffer, BufferId};
use kawoosh_editor::commands::SEARCH_LAYER;
use kawoosh_editor::{Editor, Effect, KeyStroke, Mode, Prompt, Selection, ViewId, motions};
use kui::{Align, FontId, NodeSpec, Role, Sizing, TextStyle, Ui, UiEvent, Value, WindowCommand};

use crate::Pal;
use crate::layout::{Content, Layout, Node, PaneId, SplitDir};
use crate::rows::{self, Caret, Drawn, GUTTER_W, LH, LineDraw, STRIP_H};

pub const TITLE_H: f32 = 22.0;
pub const TAB_H: f32 = 26.0;
const DIVIDER: f32 = 4.0;

pub struct Kawoosh {
    pub pal: Pal,
    pub font: Option<FontId>,
    pub ed: Editor,
    pub layout: Layout,
    pub quit: bool,
    /// Text for the clipboard at the next frame — `on_event` has no `Ui`.
    clip_out: Option<String>,
    awaiting_paste: bool,
    /// The wheel's fraction of a line carried to the next notch.
    scroll_carry: f32,
    /// False after a wheel scroll, so the view stays where the wheel put
    /// it until the caret moves again.
    follow_caret: bool,
    drag_anchor: Option<usize>,
    /// The split divider being dragged, by path.
    dragging: Option<String>,
    body_h: f32,
}

impl Kawoosh {
    pub fn new(title: impl Into<String>, text: &str) -> Self {
        let mut ed = Editor::new();
        let b = ed.add_buffer(Buffer::new(title, text));
        let view = ed.add_view(b);
        Self {
            pal: Pal::default(),
            font: None,
            ed,
            layout: Layout::new(Content::Editor(view)),
            quit: false,
            clip_out: None,
            awaiting_paste: false,
            scroll_carry: 0.0,
            follow_caret: true,
            drag_anchor: None,
            dragging: None,
            body_h: 600.0,
        }
    }

    pub fn from_file(path: &Path) -> Self {
        let mut app = Self::new("*scratch*", "");
        let scratch = app.ed.buffers.keys().next().unwrap();
        app.open(path);
        if app
            .focused_view()
            .is_some_and(|v| app.ed.views[v].buffer != scratch)
        {
            app.ed.remove_buffer(scratch);
        }
        app
    }

    /// The focused pane's view, if it is an editor pane.
    pub fn focused_view(&self) -> Option<ViewId> {
        match self.layout.focused_content() {
            Some(Content::Editor(v)) => Some(v),
            _ => None,
        }
    }

    fn view_of(&self, pane: PaneId) -> Option<ViewId> {
        match self.layout.content(pane) {
            Some(Content::Editor(v)) => Some(v),
            _ => None,
        }
    }

    /// The buffer for `path`: the one already open, or loaded, or — for a
    /// path that does not exist — a new unwritten buffer named for it.
    fn buffer_for(&mut self, path: &Path) -> Option<BufferId> {
        if let Some(id) = self.ed.buffer_at(path) {
            return Some(id);
        }
        let buf = match Buffer::from_file(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut b = Buffer::new(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    "",
                );
                b.path = Some(path.to_path_buf());
                b.language = kawoosh_doc::language_of(path).into();
                self.ed.message = format!("\"{}\" [new file]", path.display());
                b
            }
            Err(e) => {
                self.ed.message = format!("cannot open {}: {e}", path.display());
                return None;
            }
        };
        Some(self.ed.add_buffer(buf))
    }

    /// Opens `path` in the focused editor pane (or a new pane if the
    /// focus is elsewhere).
    pub fn open(&mut self, path: &Path) {
        let Some(id) = self.buffer_for(path) else {
            return;
        };
        match self.focused_view() {
            Some(v) => self.show_buffer(v, id),
            None => {
                let v = self.ed.add_view(id);
                self.layout.split(SplitDir::H, Content::Editor(v));
            }
        }
    }

    fn show_buffer(&mut self, view: ViewId, id: BufferId) {
        let v = &mut self.ed.views[view];
        if v.buffer != id {
            v.buffer = id;
            v.sels = Default::default();
            v.top = 0;
            v.goal_col = None;
        }
    }

    pub fn title(&self) -> String {
        match self.focused_view() {
            Some(v) => self.ed.buffer_of(v).name.clone(),
            None => "kawoosh".into(),
        }
    }

    // ------------------------------------------------------------ shell commands

    /// Commands the engine does not own: panes, tabs, buffers.
    pub fn shell_command(&mut self, name: &str, args: &[String], count: Option<usize>) {
        let path = args.first().filter(|a| *a != "!").map(PathBuf::from);
        match name {
            "vsplit" | "split" => {
                let dir = if name == "vsplit" {
                    SplitDir::H
                } else {
                    SplitDir::V
                };
                let buffer = match path {
                    Some(p) => match self.buffer_for(&p) {
                        Some(id) => id,
                        None => return,
                    },
                    None => match self.focused_view() {
                        Some(v) => self.ed.views[v].buffer,
                        None => match self.ed.buffers.keys().next() {
                            Some(id) => id,
                            None => return,
                        },
                    },
                };
                let nv = self.ed.add_view(buffer);
                if let Some(v) = self.focused_view()
                    && self.ed.views[v].buffer == buffer
                {
                    let src = self.ed.views[v].clone();
                    self.ed.views[nv] = src;
                }
                self.layout.split(dir, Content::Editor(nv));
            }
            "close" => {
                let pane = self.layout.focused();
                match self.layout.close(pane) {
                    Some(Content::Editor(v)) => {
                        self.ed.views.remove(v);
                    }
                    Some(Content::Terminal(_)) => {}
                    None => self.ed.message = "cannot close the last pane".into(),
                }
            }
            "only" => {
                for c in self.layout.only() {
                    if let Content::Editor(v) = c {
                        self.ed.views.remove(v);
                    }
                }
            }
            "pane_next" => {
                let p = self.layout.next_pane();
                self.layout.focus(p);
            }
            "pane_left" | "pane_right" | "pane_up" | "pane_down" => {
                let (dir, fwd) = match name {
                    "pane_left" => (SplitDir::H, false),
                    "pane_right" => (SplitDir::H, true),
                    "pane_up" => (SplitDir::V, false),
                    _ => (SplitDir::V, true),
                };
                if let Some(p) = self.layout.neighbour(dir, fwd) {
                    self.layout.focus(p);
                }
            }
            "tab_new" => {
                let buffer = match path {
                    Some(p) => self.buffer_for(&p),
                    None => Some(self.ed.add_buffer(Buffer::new("*scratch*", ""))),
                };
                if let Some(id) = buffer {
                    let v = self.ed.add_view(id);
                    self.layout.new_tab(Content::Editor(v));
                }
            }
            "tab_next" => self.layout.next_tab(count.unwrap_or(1) as i64),
            "tab_prev" => self.layout.next_tab(-(count.unwrap_or(1) as i64)),
            "tab_close" => {
                if self.layout.tabs.len() == 1 {
                    self.ed.message = "cannot close the last tab".into();
                    return;
                }
                let mut ps = Vec::new();
                self.layout.tab().root.panes(&mut ps);
                for p in ps {
                    if let Some(Content::Editor(v)) = self.layout.close(p) {
                        self.ed.views.remove(v);
                    }
                }
            }
            "dock_toggle" => {
                if self.layout.dock.is_none() {
                    let id = self.ed.add_buffer(Buffer::new("*dock*", ""));
                    let v = self.ed.add_view(id);
                    let p = self.layout.new_pane(Content::Editor(v));
                    self.layout.dock = Some(p);
                }
                self.layout.dock_open = !self.layout.dock_open;
                self.layout.dock_focused = self.layout.dock_open;
            }
            "buffer_next" | "buffer_prev" => {
                let Some(v) = self.focused_view() else { return };
                let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
                let cur = self.ed.views[v].buffer;
                let i = ids.iter().position(|b| *b == cur).unwrap_or(0);
                let n = ids.len();
                let j = if name == "buffer_next" {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                self.show_buffer(v, ids[j]);
            }
            "buffer" => {
                let Some(v) = self.focused_view() else { return };
                let Some(arg) = args.first() else {
                    self.shell_command("buffer_list", &[], None);
                    return;
                };
                let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
                let target = arg
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| ids.get(n.wrapping_sub(1)).copied())
                    .or_else(|| {
                        ids.iter()
                            .copied()
                            .find(|id| self.ed.buffers[*id].name.contains(arg.as_str()))
                    });
                match target {
                    Some(id) => self.show_buffer(v, id),
                    None => self.ed.message = format!("no buffer matching {arg}"),
                }
            }
            "buffer_delete" => {
                let Some(v) = self.focused_view() else { return };
                let cur = self.ed.views[v].buffer;
                let force = args.iter().any(|a| a == "!");
                if self.ed.buffers[cur].modified && !force {
                    self.ed.message = "unsaved changes (:bd! to discard)".into();
                    return;
                }
                let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
                let next = match ids.iter().copied().find(|b| *b != cur) {
                    Some(n) => n,
                    None => self.ed.add_buffer(Buffer::new("*scratch*", "")),
                };
                for (_, view) in self.ed.views.iter_mut() {
                    if view.buffer == cur {
                        view.buffer = next;
                        view.sels = Default::default();
                        view.top = 0;
                    }
                }
                self.ed.remove_buffer(cur);
            }
            "buffer_list" => {
                let cur = self.focused_view().map(|v| self.ed.views[v].buffer);
                let list: Vec<String> = self
                    .ed
                    .buffers
                    .iter()
                    .enumerate()
                    .map(|(i, (id, b))| {
                        format!(
                            "{}{}{}{}",
                            i + 1,
                            if Some(id) == cur { "%" } else { " " },
                            if b.modified { "+" } else { " " },
                            b.name
                        )
                    })
                    .collect();
                self.ed.message = list.join("   ");
            }
            _ => self.ed.message = format!("not a command: {name}"),
        }
    }

    // ------------------------------------------------------------ events

    fn on_key(&mut self, p: &Value) {
        let code = p
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let flag = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
        let stroke = KeyStroke {
            code,
            ctrl: flag("ctrl"),
            alt: flag("alt"),
            shift: flag("shift"),
            sup: flag("super"),
            text: p.get("text").and_then(Value::as_str).map(str::to_string),
        };
        if let Some(v) = self.focused_view() {
            self.ed.key(v, stroke);
        }
        self.follow_caret = true;
        self.drain_effects();
    }

    fn drain_effects(&mut self) {
        for e in self.ed.take_effects() {
            match e {
                Effect::Quit => self.quit = true,
                Effect::SetClipboard(t) => self.clip_out = Some(t),
                Effect::RequestPaste => self.awaiting_paste = true,
                Effect::Open(p) => self.open(&p),
                Effect::Wrote(_) => {}
                Effect::Shell { name, args, count } => self.shell_command(&name, &args, count),
            }
        }
    }

    /// The mouse over an editor pane: `line` is the ordinal among the
    /// drawn rows, `byte` into that row's drawn text.
    fn on_drag(&mut self, pane: PaneId, p: &Value) {
        let phase = p.get("phase").and_then(Value::as_str).unwrap_or("");
        if phase == "start" {
            self.layout.focus(pane);
        }
        let Some(view) = self.view_of(pane) else {
            return;
        };
        let (Some(line), Some(byte)) = (
            p.get("line").and_then(Value::as_int),
            p.get("byte").and_then(Value::as_int),
        ) else {
            return;
        };
        let clicks = p.get("clicks").and_then(Value::as_int).unwrap_or(1);
        let tabstop = self.ed.tabstop();
        let top = self.ed.views[view].top;
        let buf = self.ed.buffer_of(view);
        let ln = (top + line.max(0) as usize).min(buf.line_count() - 1);
        let drawn = Drawn::new(&buf.line_text(ln), tabstop);
        let range = buf.line_range(ln);
        let off = (range.start + drawn.to_src(byte.max(0) as usize)).min(range.end);
        let word = motions::word_at(buf, off);
        match phase {
            "start" => {
                if self.ed.mode == Mode::Command {
                    self.ed.mode = Mode::Normal;
                }
                let sel = match clicks {
                    1 => Selection::point(off),
                    2 => Selection::new(word.0, word.1),
                    _ => Selection::new(range.start, range.end),
                };
                self.drag_anchor = Some(sel.anchor);
                let v = &mut self.ed.views[view];
                v.sels = kawoosh_editor::Selections::single(sel);
                v.goal_col = None;
                if !sel.is_empty() && self.ed.mode == Mode::Normal {
                    self.ed.mode = Mode::Visual;
                }
            }
            "move" => {
                if let Some(anchor) = self.drag_anchor
                    && anchor != off
                {
                    let v = &mut self.ed.views[view];
                    v.sels = kawoosh_editor::Selections::single(Selection::new(anchor, off));
                    if self.ed.mode == Mode::Normal {
                        self.ed.mode = Mode::Visual;
                    }
                }
            }
            _ => self.drag_anchor = None,
        }
        self.follow_caret = true;
    }

    fn on_scroll(&mut self, pane: PaneId, p: &Value) {
        let Some(view) = self.view_of(pane) else {
            return;
        };
        let dy = p.get("dy").and_then(Value::as_float).unwrap_or(0.0) as f32;
        let total = self.scroll_carry - dy / LH;
        let whole = total.trunc();
        self.scroll_carry = total - whole;
        if whole == 0.0 {
            return;
        }
        let max_top = self.ed.buffer_of(view).line_count().saturating_sub(1);
        let v = &mut self.ed.views[view];
        v.top = (v.top as i64 + whole as i64).clamp(0, max_top as i64) as usize;
        if pane == self.layout.focused() {
            self.follow_caret = false;
        }
    }

    /// A divider drag: the cursor over the split's own rect is the ratio.
    fn on_split_drag(&mut self, p: &Value) {
        let tag = p.get("tag");
        let Some(path) = tag.and_then(|t| t.get("path")).and_then(Value::as_str) else {
            return;
        };
        let path = path.to_string();
        match p.get("phase").and_then(Value::as_str) {
            Some("end") => self.dragging = None,
            Some(_) => {
                let horizontal =
                    tag.and_then(|t| t.get("dir")).and_then(Value::as_str) == Some("h");
                let parent = p.get("parent");
                let get = |m: Option<&Value>, k| {
                    m.and_then(|v| v.get(k))
                        .and_then(Value::as_float)
                        .unwrap_or(0.0)
                };
                let ratio = if horizontal {
                    (get(Some(p), "x") - get(parent, "x")) / get(parent, "w").max(1.0)
                } else {
                    (get(Some(p), "y") - get(parent, "y")) / get(parent, "h").max(1.0)
                };
                let ratio = (ratio as f32).clamp(0.1, 0.9);
                if path == "dock" {
                    self.layout.dock_ratio = 1.0 - ratio;
                } else if let Some(r) = self.layout.tab_mut().root.ratio_mut(&path) {
                    *r = ratio;
                }
                self.dragging = Some(path);
            }
            None => {}
        }
    }

    // ------------------------------------------------------------ view

    fn strip(&self, ui: &mut Ui<'_>, items: &[(&str, kui::Color)], right: &str) {
        let pal = self.pal;
        ui.with(
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Fixed(STRIP_H))
                .bg(pal.strip)
                .pad_xy(8.0, 0.0)
                .gap(8.0)
                .cross_align(Align::Center),
            |ui| {
                for (t, c) in items {
                    if !t.is_empty() {
                        ui.text(t, rows::mono(self.font, &pal).color(*c));
                    }
                }
                ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                ui.text(right, rows::mono(self.font, &pal).color(pal.dim));
            },
        );
    }

    fn tab_strip(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let font = self.font;
        ui.with(
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Fixed(TAB_H))
                .bg(pal.strip)
                .pad_xy(6.0, 0.0)
                .gap(2.0)
                .cross_align(Align::End)
                .role(Role::TabList),
            |ui| {
                for (i, tab) in self.layout.tabs.iter().enumerate() {
                    let active = i == self.layout.tab;
                    let name = match self.layout.content(tab.focused) {
                        Some(Content::Editor(v)) => self.ed.buffer_of(v).name.clone(),
                        Some(Content::Terminal(_)) => "terminal".into(),
                        None => "?".into(),
                    };
                    let mut ps = Vec::new();
                    tab.root.panes(&mut ps);
                    let label = if ps.len() > 1 {
                        format!("{name} +{}", ps.len() - 1)
                    } else {
                        name
                    };
                    ui.with_indexed(
                        i as u64,
                        NodeSpec::row()
                            .pad_xy(10.0, 3.0)
                            .radius_top(5.0)
                            .bg(if active { pal.panel } else { pal.strip })
                            .hover_bg(pal.panel)
                            .on_click(Value::map([
                                ("kind", "tab".into()),
                                ("index", Value::Int(i as i64)),
                            ]))
                            .role(Role::Tab)
                            .selected(active)
                            .label(label.as_str()),
                        |ui| {
                            ui.text(
                                &label,
                                rows::mono(font, &pal).color(if active { pal.fg } else { pal.dim }),
                            );
                        },
                    );
                }
            },
        );
    }

    fn status(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let Some(view) = self.focused_view() else {
            self.strip(ui, &[("TERM", pal.accent)], "");
            return;
        };
        let v = &self.ed.views[view];
        let buf = self.ed.buffer_of(view);
        let mode = if self.ed.mode == Mode::Visual && self.ed.visual_linewise {
            "VIS LINE"
        } else {
            self.ed.mode.name()
        };
        let mode_color = match self.ed.mode {
            Mode::Insert => pal.insert,
            Mode::Visual | Mode::Command => pal.command,
            _ => pal.accent,
        };
        let name = if buf.modified {
            format!("{} [+]", buf.name)
        } else {
            buf.name.clone()
        };
        let (ln, col) = motions::line_col(buf, v.sels.primary().head);
        let mut right = format!("{}:{}", ln + 1, col + 1);
        if v.sels.len() > 1 {
            right = format!("{} sels  {right}", v.sels.len());
        }
        let pct = if buf.line_count() <= 1 {
            100
        } else {
            ln * 100 / (buf.line_count() - 1)
        };
        right.push_str(&format!("  {pct}%"));
        let pending: String = self.ed.pending.join("");
        let op = self
            .ed
            .pending_op
            .map(|(o, _)| o.chars().next().unwrap_or(' ').to_string())
            .unwrap_or_default();
        let count = self.ed.count.map(|c| c.to_string()).unwrap_or_default();
        let keys = format!("{count}{op}{pending}");
        self.strip(
            ui,
            &[
                (mode, mode_color),
                (name.as_str(), pal.fg),
                (keys.as_str(), pal.dim),
            ],
            &right,
        );
    }

    fn command_line(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let font = self.font;
        ui.with(
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Fixed(STRIP_H))
                .bg(pal.bg)
                .pad_xy(8.0, 0.0)
                .cross_align(Align::Center),
            |ui| {
                if self.ed.mode == Mode::Command {
                    let prompt = match self.ed.prompt {
                        Prompt::Command => ":",
                        Prompt::Search { backwards: false } => "/",
                        Prompt::Search { backwards: true } => "?",
                    };
                    ui.text(prompt, rows::mono(font, &pal).color(pal.command));
                    if !self.ed.cmdline.is_empty() {
                        ui.text(&self.ed.cmdline, rows::mono(font, &pal));
                    }
                    ui.with(
                        NodeSpec::column()
                            .width(Sizing::Fixed(2.0))
                            .height(Sizing::Fixed(LH - 4.0))
                            .bg(pal.command),
                        |_| {},
                    );
                } else if !self.ed.message.is_empty() {
                    ui.text(&self.ed.message, TextStyle::new(12.0).color(pal.dim));
                }
            },
        );
    }

    fn render_node(&mut self, ui: &mut Ui<'_>, node: &Node, path: &str) {
        match node {
            Node::Pane(id) => self.render_pane(ui, *id),
            Node::Split { dir, ratio, a, b } => {
                let pal = self.pal;
                let spec = match dir {
                    SplitDir::H => NodeSpec::row(),
                    SplitDir::V => NodeSpec::column(),
                };
                let ratio = ratio.clamp(0.1, 0.9);
                let dragging = self.dragging.as_deref() == Some(path);
                let grow = |f: f32| match dir {
                    SplitDir::H => NodeSpec::column()
                        .width(Sizing::Grow(f))
                        .height(Sizing::Grow(1.0)),
                    SplitDir::V => NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(f)),
                };
                ui.with(spec.fill(), |ui| {
                    ui.with_keyed("a", grow(ratio), |ui| {
                        self.render_node(ui, a, &format!("{path}a"))
                    });
                    let divider = ui.child_key("divider");
                    let active = ui.is_hovered(divider) || ui.is_pressed(divider) || dragging;
                    let bar = match dir {
                        SplitDir::H => NodeSpec::column()
                            .width(Sizing::Fixed(DIVIDER))
                            .height(Sizing::Grow(1.0))
                            .cursor(kui::CursorShape::EwResize),
                        SplitDir::V => NodeSpec::column()
                            .width(Sizing::Grow(1.0))
                            .height(Sizing::Fixed(DIVIDER))
                            .cursor(kui::CursorShape::NsResize),
                    };
                    ui.with_keyed(
                        "divider",
                        bar.bg(if active { pal.accent } else { pal.border })
                            .on_drag(Value::map([
                                ("kind", "split".into()),
                                ("path", Value::str(path)),
                                (
                                    "dir",
                                    Value::str(match dir {
                                        SplitDir::H => "h",
                                        SplitDir::V => "v",
                                    }),
                                ),
                            ])),
                        |_| {},
                    );
                    ui.with_keyed("b", grow(1.0 - ratio), |ui| {
                        self.render_node(ui, b, &format!("{path}b"))
                    });
                });
            }
        }
    }

    fn render_pane(&mut self, ui: &mut Ui<'_>, pane: PaneId) {
        let pal = self.pal;
        let font = self.font;
        let focused = self.layout.focused() == pane;
        let content = self.layout.content(pane);
        let (name, modified) = match content {
            Some(Content::Editor(v)) => {
                let b = self.ed.buffer_of(v);
                (b.name.clone(), b.modified)
            }
            Some(Content::Terminal(_)) => ("terminal".into(), false),
            None => ("?".into(), false),
        };
        ui.with_keyed(
            &format!("pane{pane}"),
            NodeSpec::column()
                .fill()
                .bg(pal.panel)
                .clip()
                .border(1.0, if focused { pal.accent } else { pal.border })
                .on_layout(Value::map([
                    ("kind", "layout".into()),
                    ("pane", Value::Int(pane as i64)),
                ])),
            |ui| {
                // The title bar.
                ui.with(
                    NodeSpec::row()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Fixed(TITLE_H))
                        .bg(if focused { pal.strip } else { pal.bg })
                        .pad_xy(8.0, 0.0)
                        .gap(6.0)
                        .cross_align(Align::Center)
                        .on_click(Value::map([
                            ("kind", "focus".into()),
                            ("pane", Value::Int(pane as i64)),
                        ]))
                        .label(name.as_str()),
                    |ui| {
                        ui.text(
                            &name,
                            TextStyle::new(12.0).color(if focused { pal.fg } else { pal.dim }),
                        );
                        if modified {
                            ui.text("●", TextStyle::new(10.0).color(pal.command));
                        }
                    },
                );
                match content {
                    Some(Content::Editor(v)) => self.render_editor(ui, pane, v, focused),
                    Some(Content::Terminal(_)) => {
                        ui.text(
                            "terminal (milestone 4)",
                            rows::mono(font, &pal).color(pal.dim),
                        );
                    }
                    None => {}
                }
            },
        );
    }

    fn render_editor(&mut self, ui: &mut Ui<'_>, pane: PaneId, view: ViewId, focused: bool) {
        let pal = self.pal;
        let font = self.font;
        let height = self
            .layout
            .rects
            .get(&pane)
            .map(|r| r.h - TITLE_H - 2.0)
            .unwrap_or(self.body_h - TITLE_H);
        let rows_n = ((height / LH).floor().max(1.0)) as usize;
        let scrolloff = self
            .ed
            .option("scrolloff")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(3)
            .min(rows_n / 2);
        let tabstop = self.ed.tabstop();
        let mode = if focused { self.ed.mode } else { Mode::Normal };
        let linewise = self.ed.visual_linewise;
        let blink_on = ui.caret_visible();
        let buf_id = self.ed.views[view].buffer;

        // Scroll the caret into view — a few lines, in the app.
        {
            let line_count = self.ed.buffers[buf_id].line_count();
            let head_line =
                self.ed.buffers[buf_id].line_of(self.ed.views[view].sels.primary().head);
            let v = &mut self.ed.views[view];
            v.rows = rows_n;
            if self.follow_caret || !focused {
                if head_line < v.top + scrolloff {
                    v.top = head_line.saturating_sub(scrolloff);
                }
                if head_line + scrolloff >= v.top + rows_n {
                    v.top = (head_line + scrolloff + 1).saturating_sub(rows_n);
                }
            }
            v.top = v.top.min(line_count.saturating_sub(1));
        }

        let v = &self.ed.views[view];
        let buf = &self.ed.buffers[buf_id];
        let top = v.top;
        let last = (top + rows_n).min(buf.line_count());
        let sels = &v.sels;
        let primary = sels.primary();
        let cur_line = buf.line_of(primary.head);
        let title = buf.name.clone();
        let caret_kind = if mode == Mode::Insert {
            Caret::Bar
        } else {
            Caret::Block
        };
        let tag = Value::map([("kind", "pane".into()), ("pane", Value::Int(pane as i64))]);

        let sink = ui.with_keyed(
            "editor",
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .clip()
                .on_key(tag.clone())
                .on_drag(tag.clone())
                .on_scroll(tag)
                .cursor(kui::CursorShape::Text)
                .role(Role::MultilineTextInput)
                .label(title.as_str()),
            |ui| {
                ui.with(
                    NodeSpec::column()
                        .width(Sizing::Fixed(GUTTER_W))
                        .height(Sizing::Grow(1.0))
                        .pad_xy(12.0, 0.0)
                        .role(Role::None),
                    |ui| {
                        for ln in top..last {
                            rows::gutter_row(ui, font, &pal, ln, ln == cur_line);
                        }
                    },
                );
                ui.with_keyed(
                    "lines",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(1.0))
                        .clip(),
                    |ui| {
                        for ln in top..last {
                            let range = buf.line_range(ln);
                            let src = buf.line_text(ln);
                            let drawn = Drawn::new(&src, tabstop);
                            let clip = |o: usize| {
                                drawn.to_drawn(o.clamp(range.start, range.end) - range.start)
                            };
                            let mut selected: Vec<Range<usize>> = Vec::new();
                            let mut carets: Vec<(usize, Caret)> = Vec::new();
                            let mut access = (None, None);
                            for s in sels.iter() {
                                let r = s.range();
                                let (rs, re) = if mode == Mode::Visual && linewise {
                                    (
                                        buf.line_start(buf.line_of(r.start)),
                                        buf.line_range(buf.line_of(r.end)).end + 1,
                                    )
                                } else if mode == Mode::Visual {
                                    (r.start, buf.next_char(r.end).max(r.end + 1))
                                } else {
                                    (r.start, r.end)
                                };
                                if rs < re && rs <= range.end && re > range.start {
                                    let a = clip(rs);
                                    let b = if re > range.end {
                                        drawn.text.len() + 1
                                    } else {
                                        clip(re)
                                    };
                                    if a < b {
                                        selected.push(a..b);
                                    }
                                }
                                let head_line = buf.line_of(s.head);
                                if head_line == ln && focused && (blink_on || mode != Mode::Insert)
                                {
                                    carets.push((clip(s.head), caret_kind));
                                }
                                if *s == primary && head_line == ln && focused {
                                    access.0 = Some(clip(s.head) as u32);
                                    if !s.is_empty() && buf.line_of(s.anchor) == ln {
                                        access.1 = Some(clip(s.anchor) as u32);
                                    }
                                }
                            }
                            let hits: Vec<Range<usize>> = buf
                                .runs(SEARCH_LAYER, range.clone())
                                .iter()
                                .map(|r| clip(r.range.start)..clip(r.range.end.min(range.end)))
                                .filter(|r| r.start < r.end)
                                .collect();
                            rows::emit_line(
                                ui,
                                font,
                                &pal,
                                &LineDraw {
                                    text: &drawn.text,
                                    selected: &selected,
                                    hits: &hits,
                                    styled: &[],
                                    carets: &carets,
                                    access,
                                    underlined: &[],
                                    trailing: None,
                                },
                            );
                        }
                    },
                );
            },
        );
        if focused {
            ui.take_key_focus(sink);
        }
    }
}

impl kui::App for Kawoosh {
    fn view(&mut self, ui: &mut Ui<'_>) {
        if self.quit {
            ui.window_command(WindowCommand::Close(ui.env().window.id));
        }
        self.pal = ui.theme().into();
        let pal = self.pal;
        if let Some(text) = self.clip_out.take() {
            ui.set_clipboard(text, None);
        }
        if self.awaiting_paste {
            ui.request_paste();
        }
        ui.window_title(&format!("{} — kawoosh", self.title()));
        let vp = ui.viewport();
        self.body_h = (vp.h - TAB_H - 2.0 * STRIP_H).max(LH);
        let body_h = self.body_h;
        ui.with(NodeSpec::column().fill().bg(pal.bg), |ui| {
            self.tab_strip(ui);
            ui.with(
                NodeSpec::column()
                    .width(Sizing::Grow(1.0))
                    .height(Sizing::Fixed(body_h)),
                |ui| {
                    let root = self.layout.tab().root.clone();
                    let dock = self.layout.dock.filter(|_| self.layout.dock_open);
                    let dock_h = if dock.is_some() {
                        (body_h * self.layout.dock_ratio).clamp(LH * 3.0, body_h - LH * 3.0)
                    } else {
                        0.0
                    };
                    ui.with(
                        NodeSpec::column()
                            .width(Sizing::Grow(1.0))
                            .height(Sizing::Grow(1.0)),
                        |ui| self.render_node(ui, &root, ""),
                    );
                    if let Some(d) = dock {
                        let divider = ui.child_key("dockdiv");
                        let active = ui.is_hovered(divider)
                            || ui.is_pressed(divider)
                            || self.dragging.as_deref() == Some("dock");
                        ui.with_keyed(
                            "dockdiv",
                            NodeSpec::column()
                                .width(Sizing::Grow(1.0))
                                .height(Sizing::Fixed(DIVIDER))
                                .bg(if active { pal.accent } else { pal.border })
                                .cursor(kui::CursorShape::NsResize)
                                .on_drag(Value::map([
                                    ("kind", "split".into()),
                                    ("path", "dock".into()),
                                    ("dir", "v".into()),
                                ])),
                            |_| {},
                        );
                        ui.with_keyed(
                            "dock",
                            NodeSpec::column()
                                .width(Sizing::Grow(1.0))
                                .height(Sizing::Fixed(dock_h)),
                            |ui| self.render_pane(ui, d),
                        );
                    }
                },
            );
            self.status(ui);
            self.command_line(ui);
        });
    }

    fn on_event(&mut self, ev: UiEvent) {
        let p = &ev.payload;
        let pane_of = |p: &Value| {
            p.get("tag")
                .and_then(|t| t.get("pane"))
                .and_then(Value::as_int)
                .map(|n| n as PaneId)
        };
        let tag_kind = p
            .get("tag")
            .and_then(|t| t.get("kind"))
            .and_then(Value::as_str);
        match p.get("kind").and_then(Value::as_str) {
            Some("key") => self.on_key(p),
            Some("text") => {
                let text = p
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Some(v) = self.focused_view() {
                    if std::mem::take(&mut self.awaiting_paste) {
                        self.ed.paste_text(v, &text);
                    } else {
                        self.ed.text(v, &text);
                    }
                }
                self.follow_caret = true;
                self.drain_effects();
            }
            Some("drag") => match tag_kind {
                Some("split") => self.on_split_drag(p),
                _ => {
                    if let Some(pane) = pane_of(p) {
                        self.on_drag(pane, p);
                    }
                }
            },
            Some("scroll") => {
                if let Some(pane) = pane_of(p) {
                    self.on_scroll(pane, p);
                }
            }
            Some("click") => match tag_kind {
                Some("focus") => {
                    if let Some(pane) = pane_of(p) {
                        self.layout.focus(pane);
                    }
                }
                Some("tab") => {
                    if let Some(i) = p
                        .get("tag")
                        .and_then(|t| t.get("index"))
                        .and_then(Value::as_int)
                    {
                        self.layout.tab = (i as usize).min(self.layout.tabs.len() - 1);
                        self.layout.dock_focused = false;
                    }
                }
                _ => {}
            },
            Some("layout") => {
                if let Some(pane) = pane_of(p) {
                    let f = |k| p.get(k).and_then(Value::as_float).unwrap_or(0.0) as f32;
                    self.layout.rects.insert(
                        pane,
                        crate::layout::Rect {
                            x: f("x"),
                            y: f("y"),
                            w: f("w"),
                            h: f("h"),
                        },
                    );
                }
            }
            _ => {}
        }
    }
}
