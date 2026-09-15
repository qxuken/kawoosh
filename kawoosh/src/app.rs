//! The `kui::App`: milestone 1 — one read-only buffer drawn as rows, a
//! root key sink moving through it. Everything the later milestones add
//! (the modal engine, the pane tree, terminals, systems, Lua) hangs off
//! this struct; what is here is the frame's skeleton and the event path.

use kui::{Align, FontId, NodeSpec, Role, Sizing, TextStyle, Ui, UiEvent, Value, WindowCommand};
use text_buffer::Buffer;

use crate::Pal;
use crate::rows::{self, GUTTER_W, LH, STRIP_H};

pub struct Kawoosh {
    pub pal: Pal,
    pub font: Option<FontId>,
    pub title: String,
    pub text: Buffer,
    /// The line the caret is on; milestone 2 replaces it with selections.
    pub line: usize,
    /// First visible line; scroll-into-view runs in `view`.
    pub top: usize,
    /// Visible rows, written by `view` for the paging motions.
    pub rows: usize,
    pending: Option<char>,
    pub quit: bool,
}

impl Kawoosh {
    pub fn new(title: impl Into<String>, text: &[u8]) -> Self {
        Self {
            pal: Pal::default(),
            font: None,
            title: title.into(),
            text: Buffer::with_text(text),
            line: 0,
            top: 0,
            rows: 24,
            pending: None,
            quit: false,
        }
    }

    pub fn line_count(&self) -> usize {
        self.text.line_count().max(1)
    }

    /// The text of line `ln`, without its newline.
    pub fn line_text(&self, ln: usize) -> String {
        let Some(range) = self.text.get_line_range(ln) else {
            return String::new();
        };
        let bytes = self.text.collect_range(range);
        let s = String::from_utf8_lossy(&bytes);
        s.trim_end_matches(['\n', '\r']).replace('\t', "    ")
    }

    fn move_line(&mut self, dy: i64) {
        let last = self.line_count() as i64 - 1;
        self.line = (self.line as i64 + dy).clamp(0, last) as usize;
    }

    fn on_key(&mut self, code: &str, ctrl: bool) {
        if let Some(p) = self.pending.take() {
            if p == 'g' && code == "g" {
                self.line = 0;
            }
            return;
        }
        let page = self.rows as i64 - 1;
        match (code, ctrl) {
            ("j", false) | ("down", false) => self.move_line(1),
            ("k", false) | ("up", false) => self.move_line(-1),
            ("pagedown", _) | ("f", true) => self.move_line(page.max(1)),
            ("pageup", _) | ("b", true) => self.move_line(-page.max(1)),
            ("d", true) => self.move_line((page / 2).max(1)),
            ("u", true) => self.move_line(-(page / 2).max(1)),
            ("G", false) => self.line = self.line_count() - 1,
            ("g", false) => self.pending = Some('g'),
            ("q", false) => self.quit = true,
            _ => {}
        }
    }

    fn strip(&self, ui: &mut Ui<'_>, left: &str, right: &str) {
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
                ui.text(left, TextStyle::new(12.0).color(pal.fg));
                ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                ui.text(right, rows::mono(self.font, &pal).color(pal.dim));
            },
        );
    }

    fn editor(&mut self, ui: &mut Ui<'_>, height: f32) {
        let pal = self.pal;
        let font = self.font;
        let rows = ((height / LH).floor().max(1.0)) as usize;
        self.rows = rows;
        if self.line < self.top {
            self.top = self.line;
        }
        if self.line >= self.top + rows {
            self.top = self.line + 1 - rows;
        }
        let last = (self.top + rows).min(self.line_count());
        let (top, cur) = (self.top, self.line);

        let sink = ui.with_keyed(
            "editor",
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .bg(pal.panel)
                .clip()
                .on_key(Value::Null)
                .role(Role::MultilineTextInput)
                .label(self.title.as_str()),
            |ui| {
                ui.with(
                    NodeSpec::column()
                        .width(Sizing::Fixed(GUTTER_W))
                        .height(Sizing::Grow(1.0))
                        .pad_xy(12.0, 0.0)
                        .role(Role::None),
                    |ui| {
                        for ln in top..last {
                            rows::gutter_row(ui, font, &pal, ln, ln == cur);
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
                            let text = self.line_text(ln);
                            rows::emit_line(ui, font, &pal, &text);
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
        ui.window_title(&format!("{} — kawoosh", self.title));
        let vp = ui.viewport();
        let editor_h = (vp.h - 2.0 * STRIP_H).max(LH);
        ui.with(NodeSpec::column().fill().bg(pal.bg), |ui| {
            let title = self.title.clone();
            self.strip(ui, &title, &format!("{} lines", self.line_count()));
            self.editor(ui, editor_h);
            let pos = format!("{}:{}", self.line + 1, 1);
            self.strip(ui, "NOR", &pos);
        });
    }

    fn on_event(&mut self, ev: UiEvent) {
        if ev.payload.get("kind").and_then(Value::as_str) == Some("key") {
            let code = ev.payload.get("code").and_then(Value::as_str).unwrap_or("");
            let ctrl = ev
                .payload
                .get("ctrl")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            self.on_key(code, ctrl);
        }
    }
}
