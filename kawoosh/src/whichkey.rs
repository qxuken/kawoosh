//! A which-key: while a key sequence is open — `<leader>`, `g`, `gs`,
//! `]`, `<C-w>` — a small float at the bottom-left lists what can
//! follow it, each key with its command, so the clusters of
//! docs/design/keys.md can be read off the screen instead of
//! remembered. It is there the moment the prefix is pressed and gone
//! the moment the sequence resolves; `whichkey = false` in the
//! settings (`:set nowhichkey`) turns it off.

use kawoosh_editor::Mode;
use kui::{Align, FloatConfig, NodeSpec, TextStyle, Ui};

use crate::app::Kawoosh;
use crate::rows::STRIP_H;

/// Rows per column before the list folds into another column.
const PER_COLUMN: usize = 8;

/// How a key reads in the list: `SPC`, `RET`, `ESC`, `TAB`, a chord
/// without its brackets (`C-w`), else the key itself.
fn pretty(key: &str) -> String {
    match key {
        "<Space>" => "SPC".into(),
        "<CR>" => "RET".into(),
        "<Esc>" => "ESC".into(),
        "<Tab>" => "TAB".into(),
        "<BS>" => "BS".into(),
        k => k
            .strip_prefix('<')
            .and_then(|k| k.strip_suffix('>'))
            .unwrap_or(k)
            .to_string(),
    }
}

impl Kawoosh {
    /// The keys open now and the mode they are looked up in: the
    /// engine's pending sequence, or the `<C-w>` a pane without a view
    /// holds.
    fn open_sequence(&self) -> Option<(Vec<String>, Mode)> {
        if !self.ed.pending.is_empty() {
            let mode = if self.ed.pending_op.is_some() {
                Mode::OperatorPending
            } else {
                self.focused_mode()
            };
            return Some((self.ed.pending.clone(), mode));
        }
        if self.terms.prefix
            || self.scripting.prefix
            || self.undo.prefix
            || self.history_pane.prefix
        {
            return Some((vec!["<C-w>".into()], Mode::Normal));
        }
        None
    }

    /// The float, when a sequence is open and the setting is on.
    pub(crate) fn whichkey(&self, ui: &mut Ui<'_>) {
        if !self.ed.settings.bool("whichkey").unwrap_or(true) {
            return;
        }
        let Some((keys, mode)) = self.open_sequence() else {
            return;
        };
        let km = &self.ed.keymap;
        let mut rows = km.next_keys(mode, &keys);
        // A mode's lookup falls through to normal mode's, so its
        // sequences are open here too.
        if mode != Mode::Normal {
            for (k, b) in km.next_keys(Mode::Normal, &keys) {
                if !rows.iter().any(|(o, _)| *o == k) {
                    rows.push((k, b));
                }
            }
            rows.sort_by(|a, b| a.0.cmp(&b.0));
        }
        if rows.is_empty() {
            return;
        }
        let pal = self.pal;
        let title: Vec<String> = keys.iter().map(|k| pretty(k)).collect();
        let title = title.join(" ");
        // `<leader><leader>` lists as the leader's key, not the word.
        let show = |k: &str| {
            if k == kawoosh_editor::keymap::LEADER {
                pretty(km.leader())
            } else {
                pretty(k)
            }
        };
        let key_style = TextStyle::new(12.0).color(pal.accent).nowrap();
        let what_style = TextStyle::new(12.0).color(pal.fg).nowrap();
        let group_style = TextStyle::new(12.0).color(pal.dim).nowrap();
        ui.with_keyed(
            "whichkey",
            NodeSpec::column()
                .float(
                    FloatConfig::viewport()
                        .at(Align::Start, Align::End)
                        .self_at(Align::Start, Align::End)
                        .offset(12.0, -(2.0 * STRIP_H + 8.0)),
                )
                .bg(pal.panel)
                .border(1.0, pal.border)
                .radius(4.0)
                .pad_xy(10.0, 6.0)
                .gap(4.0),
            |ui| {
                ui.text(&title, TextStyle::new(12.0).color(pal.dim).nowrap());
                ui.with_keyed("cols", NodeSpec::row().gap(18.0), |ui| {
                    for (ci, chunk) in rows.chunks(PER_COLUMN).enumerate() {
                        ui.with_indexed(ci as u64, NodeSpec::column().gap(2.0), |ui| {
                            for (ri, (k, b)) in chunk.iter().enumerate() {
                                ui.with_indexed(ri as u64, NodeSpec::row().gap(8.0), |ui| {
                                    ui.text(&show(k), key_style);
                                    match b {
                                        Some(b) => ui.text(&b.line(), what_style),
                                        None => {
                                            let mut deeper = keys.clone();
                                            deeper.push(k.clone());
                                            let n = km.next_keys(mode, &deeper).len();
                                            ui.text(&format!("+{n}"), group_style);
                                        }
                                    }
                                });
                            }
                        });
                    }
                });
            },
        );
    }
}
