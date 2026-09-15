//! The `kui::App`: the modal editor over one view (milestone 2). The
//! engine (`kawoosh-editor`) owns buffers, selections, modes and the
//! keymap; this file turns its state into rows and its effects into what
//! only the shell can do, and turns kui's events into key strokes, text
//! and mouse positions.

use std::ops::Range;
use std::path::Path;

use kawoosh_doc::Buffer;
use kawoosh_editor::commands::SEARCH_LAYER;
use kawoosh_editor::{Editor, Effect, KeyStroke, Mode, Prompt, Selection, ViewId, motions};
use kui::{Align, FontId, NodeSpec, Role, Sizing, TextStyle, Ui, UiEvent, Value, WindowCommand};

use crate::Pal;
use crate::rows::{self, Caret, Drawn, GUTTER_W, LH, LineDraw, STRIP_H};

pub struct Kawoosh {
    pub pal: Pal,
    pub font: Option<FontId>,
    pub ed: Editor,
    pub view: ViewId,
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
            view,
            quit: false,
            clip_out: None,
            awaiting_paste: false,
            scroll_carry: 0.0,
            follow_caret: true,
            drag_anchor: None,
        }
    }

    pub fn from_file(path: &Path) -> Self {
        let mut app = Self::new("*scratch*", "");
        app.open(path);
        app
    }

    /// Opens `path` in the current view: an already-open buffer is reused;
    /// a missing file is a new, unwritten buffer named for it.
    pub fn open(&mut self, path: &Path) {
        let id = match self.ed.buffer_at(path) {
            Some(id) => id,
            None => {
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
                        return;
                    }
                };
                self.ed.add_buffer(buf)
            }
        };
        let v = &mut self.ed.views[self.view];
        v.buffer = id;
        v.sels = Default::default();
        v.top = 0;
        v.goal_col = None;
    }

    pub fn title(&self) -> String {
        self.ed.buffer_of(self.view).name.clone()
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
        self.ed.key(self.view, stroke);
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
            }
        }
    }

    /// The mouse over the editor: `line` is the ordinal among the drawn
    /// rows, `byte` into that row's drawn text.
    fn on_drag(&mut self, p: &Value) {
        let Some(line) = p.get("line").and_then(Value::as_int) else {
            return;
        };
        let Some(byte) = p.get("byte").and_then(Value::as_int) else {
            return;
        };
        let clicks = p.get("clicks").and_then(Value::as_int).unwrap_or(1);
        let phase = p.get("phase").and_then(Value::as_str).unwrap_or("");
        let tabstop = self.ed.tabstop();
        let top = self.ed.views[self.view].top;
        let buf = self.ed.buffer_of(self.view);
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
                let v = &mut self.ed.views[self.view];
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
                    let v = &mut self.ed.views[self.view];
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

    fn on_scroll(&mut self, p: &Value) {
        let dy = p.get("dy").and_then(Value::as_float).unwrap_or(0.0) as f32;
        let total = self.scroll_carry - dy / LH;
        let whole = total.trunc();
        self.scroll_carry = total - whole;
        if whole == 0.0 {
            return;
        }
        let buf = self.ed.buffer_of(self.view);
        let max_top = buf.line_count().saturating_sub(1);
        let v = &mut self.ed.views[self.view];
        v.top = (v.top as i64 + whole as i64).clamp(0, max_top as i64) as usize;
        self.follow_caret = false;
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

    fn status(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let v = &self.ed.views[self.view];
        let buf = self.ed.buffer_of(self.view);
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

    fn editor(&mut self, ui: &mut Ui<'_>, height: f32) {
        let pal = self.pal;
        let font = self.font;
        let rows_n = ((height / LH).floor().max(1.0)) as usize;
        let scrolloff = self
            .ed
            .option("scrolloff")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(3)
            .min(rows_n / 2);
        let tabstop = self.ed.tabstop();
        let mode = self.ed.mode;
        let linewise = self.ed.visual_linewise;
        let blink_on = ui.caret_visible();
        let view = self.view;
        let buf_id = self.ed.views[view].buffer;

        // Scroll the caret into view — a few lines, in the app.
        {
            let line_count = self.ed.buffers[buf_id].line_count();
            let head_line =
                self.ed.buffers[buf_id].line_of(self.ed.views[view].sels.primary().head);
            let v = &mut self.ed.views[view];
            v.rows = rows_n;
            if self.follow_caret {
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

        let sink = ui.with_keyed(
            "editor",
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .bg(pal.panel)
                .clip()
                .on_key(Value::Null)
                .on_drag(Value::Null)
                .on_scroll(Value::Null)
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
                                // Visual mode shows the char under the head as
                                // selected too; a bare cursor shows nothing.
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
                                if head_line == ln && (blink_on || mode != Mode::Insert) {
                                    carets.push((clip(s.head), caret_kind));
                                }
                                if *s == primary && head_line == ln {
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
        ui.take_key_focus(sink);
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
        let editor_h = (vp.h - 2.0 * STRIP_H).max(LH);
        ui.with(NodeSpec::column().fill().bg(pal.bg), |ui| {
            self.editor(ui, editor_h);
            self.status(ui);
            self.command_line(ui);
        });
    }

    fn on_event(&mut self, ev: UiEvent) {
        match ev.payload.get("kind").and_then(Value::as_str) {
            Some("key") => self.on_key(&ev.payload),
            Some("text") => {
                let text = ev
                    .payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if std::mem::take(&mut self.awaiting_paste) {
                    self.ed.paste_text(self.view, &text);
                } else {
                    self.ed.text(self.view, &text);
                }
                self.follow_caret = true;
                self.drain_effects();
            }
            Some("drag") => self.on_drag(&ev.payload),
            Some("scroll") => self.on_scroll(&ev.payload),
            _ => {}
        }
    }
}
