//! A confirm: one modal float over the window that asks before
//! something happens — the file manager's write above all (mvp.md 5b's
//! "confirm step for destructive ones"). kui.md's "no popups" was
//! revisited once for notifications (two floats that never take the
//! keys); this is the second time, and the opposite case: a question
//! that must be answered takes the keys until it is, and everything
//! under it is inert — kui's `modal`, a `dismiss` on `<Esc>` or a press
//! outside. Asked from Lua as `kawoosh.confirm { title, lines, actions
//! }` (`Msg::Confirm`); the actions are a toast's, `(label, command)`.

use kawoosh_editor::KeyStroke;
use kui_native::{Align, FloatConfig, NodeSpec, Role, TextStyle, Ui, Value};

use crate::app::Kawoosh;
use crate::rows;

/// The lines a confirm shows before folding the rest into "… N more".
const LINES_SHOWN: usize = 14;

#[derive(Clone, Debug, PartialEq)]
pub struct Confirm {
    /// The question.
    pub title: String,
    /// What would happen, one per line, in mono.
    pub lines: Vec<String>,
    /// The answers: a label and the command it runs (none for a plain
    /// "no").
    pub actions: Vec<(String, String)>,
    /// The action the keyboard is on.
    pub chosen: usize,
}

impl Kawoosh {
    /// Puts a confirm up, over one already there.
    pub fn confirm_with(&mut self, c: Confirm) {
        let n = c.actions.len().max(1);
        self.confirm = Some(Confirm {
            chosen: c.chosen.min(n - 1),
            ..c
        });
        self.wake.wake();
    }

    /// Answers the confirm with action `index`: the float goes, and
    /// the action's command runs (none for a label without one).
    pub(crate) fn confirm_answer(&mut self, index: Option<usize>) {
        let Some(c) = self.confirm.take() else {
            return;
        };
        if let Some(cmd) = index
            .and_then(|i| c.actions.get(i))
            .map(|(_, cmd)| cmd.clone())
            && !cmd.is_empty()
        {
            self.run_line(&cmd);
        }
    }

    /// A key while a confirm is up: `<CR>` / `<Space>` take the chosen
    /// action, `y` the first, a digit that one; `h` `l` `<Tab>` and the
    /// arrows move between them; `<Esc>` `n` `q` answer with none.
    /// True when there is a confirm — it has every key.
    pub(crate) fn confirm_key(&mut self, stroke: &KeyStroke) -> bool {
        let Some(c) = self.confirm.as_mut() else {
            return false;
        };
        let n = c.actions.len().max(1);
        let note = stroke.notation();
        match note.as_str() {
            "<Esc>" | "n" | "q" | "<C-c>" => self.confirm_answer(None),
            "<CR>" | "<Space>" => {
                let i = c.chosen;
                self.confirm_answer(Some(i));
            }
            "y" => self.confirm_answer(Some(0)),
            "l" | "<Right>" | "<Tab>" | "j" | "<Down>" => c.chosen = (c.chosen + 1) % n,
            "h" | "<Left>" | "<S-Tab>" | "k" | "<Up>" => c.chosen = (c.chosen + n - 1) % n,
            d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => {
                let i = (d.as_bytes()[0] - b'1') as usize;
                if i < c.actions.len() {
                    self.confirm_answer(Some(i));
                }
            }
            _ => {}
        }
        true
    }

    /// A click on a confirm's button (`{kind = "confirm", action = i}`).
    pub(crate) fn on_confirm(&mut self, p: &Value) {
        if let Some(i) = p.get_int("action") {
            self.confirm_answer(Some(i as usize));
        }
    }

    /// kui's `dismiss` on the confirm's modal: `<Esc>` or a press
    /// outside it, answered with none.
    pub(crate) fn on_dismiss(&mut self, p: &Value) {
        let tag = p.get("tag").and_then(|t| t.get_str("kind"));
        if tag == Some("confirm") {
            self.confirm_answer(None);
        }
    }

    /// The confirm's float: centred, modal, the keys on it.
    pub(crate) fn confirm_float(&mut self, ui: &mut Ui<'_>) {
        let Some(c) = self.confirm.clone() else {
            return;
        };
        let pal = self.pal;
        let small = self.chrome.small;
        let vp = ui.viewport();
        let max_w = (vp.w * 0.6).clamp(280.0, 720.0);
        let tag = Value::map([("kind", "confirm".into())]);
        let mono = rows::mono(self.face, &pal);
        let sink = ui.with_keyed(
            "confirm",
            NodeSpec::column()
                .float(FloatConfig::viewport().inside(Align::Center, Align::Center))
                .modal(tag.clone())
                .role(Role::Dialog)
                .label(c.title.as_str())
                .max_width(max_w)
                .pad_xy(14.0, 12.0)
                .gap(10.0)
                .bg(pal.panel)
                .border(1.0, pal.accent)
                .radius(6.0)
                .on_key(tag.clone())
                .focusable()
                .initial_focus(),
            |ui| {
                ui.text(
                    &c.title,
                    TextStyle::new(self.chrome.face.size).color(pal.fg),
                );
                if !c.lines.is_empty() {
                    ui.with(NodeSpec::column().gap(2.0), |ui| {
                        for l in c.lines.iter().take(LINES_SHOWN) {
                            ui.text(l, mono.color(pal.dim).nowrap());
                        }
                        if c.lines.len() > LINES_SHOWN {
                            ui.text(
                                &format!("… {} more", c.lines.len() - LINES_SHOWN),
                                mono.color(pal.faint),
                            );
                        }
                    });
                }
                // A few short answers are a row of buttons at the right;
                // more, or long ones — a server's code actions — a
                // column, each with its digit, so none is squeezed to
                // nothing. Either takes its contents' width: a grow row
                // in a dialog sized to its contents got the title's.
                let long = c
                    .actions
                    .iter()
                    .map(|(l, _)| l.chars().count())
                    .sum::<usize>()
                    > 48;
                let column = c.actions.len() > 3 || long;
                let spec = if column {
                    NodeSpec::column()
                        .grow_width()
                        .min_width(kui_native::Min::FIT)
                        .gap(2.0)
                } else {
                    NodeSpec::row()
                        .grow_width()
                        .min_width(kui_native::Min::FIT)
                        .gap(6.0)
                        .main_align(Align::End)
                        .cross_align(Align::Center)
                };
                ui.with(spec, |ui| {
                    for (i, (label, _)) in c.actions.iter().enumerate() {
                        let on = i == c.chosen;
                        let mut button = NodeSpec::row()
                            .pad_xy(10.0, 3.0)
                            .gap(10.0)
                            .radius(3.0)
                            .bg(if on { pal.select } else { pal.strip })
                            .hover_bg(pal.select)
                            .role(Role::Button)
                            .label(label.as_str())
                            .on_click(Value::map([
                                ("kind", "confirm".into()),
                                ("action", Value::Int(i as i64)),
                            ]));
                        if column {
                            button = button.grow_width().min_width(kui_native::Min::FIT);
                        }
                        ui.with_indexed(i as u64, button, |ui| {
                            if column && i < 9 {
                                ui.text(
                                    &(i + 1).to_string(),
                                    TextStyle::new(small).color(pal.dim).nowrap(),
                                );
                            }
                            ui.text(label, TextStyle::new(small).color(pal.fg).nowrap());
                        });
                    }
                });
            },
        );
        ui.take_key_focus(sink);
    }
}
