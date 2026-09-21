//! A which-key: while a key sequence is open — `<leader>`, `g`, `gs`,
//! `]`, `<C-w>` — a small card at the bottom-right lists what can
//! follow it, each key with its command and each group with its name
//! (`Keymap::describe`, `:map group`), so the clusters of
//! docs/design/keys.md can be read off the screen instead of
//! remembered. It is there the moment the prefix is pressed and gone
//! the moment the sequence resolves; `:keys` (`<leader>?`) shows the
//! root — every first key — until the next press. `whichkey = false`
//! in the settings (`:set nowhichkey`) turns it off.
//!
//! The card is in the bottom-right stack, shared with the notification
//! corner: the corner's lines above, the which-key below, so the two
//! never cover each other.

use kawoosh_editor::{ArgKind, Args, Binding, Mode, Spec};
use kui::{Align, FloatConfig, NodeSpec, TextStyle, Ui};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::rows::STRIP_H;

/// Rows per column before the list folds into another column.
const PER_COLUMN: usize = 8;
/// The most columns the root listing spreads over.
const ROOT_COLUMNS: usize = 4;

/// How a key reads in the list: `SPC`, `RET`, `ESC`, `TAB`, a chord
/// without its brackets (`C-w`) and with its shift spelled out
/// (`<C-H>` is `C-S-h`), else the key itself.
fn pretty(key: &str) -> String {
    match key {
        "<Space>" => "SPC".into(),
        "<CR>" => "RET".into(),
        "<Esc>" => "ESC".into(),
        "<Tab>" => "TAB".into(),
        "<BS>" => "BS".into(),
        k => {
            let Some(inner) = k.strip_prefix('<').and_then(|k| k.strip_suffix('>')) else {
                return k.to_string();
            };
            match inner.rsplit_once('-') {
                Some((mods, letter))
                    if letter.len() == 1 && letter.as_bytes()[0].is_ascii_uppercase() =>
                {
                    format!("{mods}-S-{}", letter.to_ascii_lowercase())
                }
                _ => inner.to_string(),
            }
        }
    }
}

impl Kawoosh {
    /// The keys open now and the mode they are looked up in: the
    /// engine's pending sequence, the `<C-w>` a pane without a view
    /// holds, or nothing at all for the root `:keys` asked for.
    fn open_sequence(&self) -> Option<(Vec<String>, Mode)> {
        if !self.ed.pending.is_empty() {
            let mode = if self.ed.pending_op.is_some() {
                Mode::OperatorPending
            } else {
                self.focused_mode()
            };
            return Some((self.ed.pending.clone(), mode));
        }
        if self.terms.prefix {
            return Some((vec!["<C-w>".into()], Mode::Normal));
        }
        if let Some(mode) = self.keys_help {
            return Some((Vec::new(), mode));
        }
        None
    }

    /// The which-key's rows for the open sequence: each next key with
    /// its binding, sorted; none when nothing is open or the setting
    /// is off.
    #[allow(clippy::type_complexity)]
    fn whichkey_rows(&self) -> Option<(Vec<String>, Mode, Vec<(String, Vec<Binding>)>)> {
        if !self.ed.settings.bool("whichkey").unwrap_or(true) {
            return None;
        }
        let (keys, mode) = self.open_sequence()?;
        let km = &self.ed.keymap;
        let mut rows = km.next_keys(mode, &keys);
        // Visual and operator-pending lookups fall through to normal
        // mode's, so its sequences are open there too; a pane's for
        // what every pane shares (`<C-w>`, the leader); insert mode's
        // do not.
        if matches!(mode, Mode::Visual | Mode::OperatorPending)
            || (mode == Mode::Pane && (keys.is_empty() || km.shared_from_pane(&keys)))
        {
            for (k, b) in km.next_keys(Mode::Normal, &keys) {
                if !rows.iter().any(|(o, _)| *o == k) {
                    rows.push((k, b));
                }
            }
            rows.sort_by(|a, b| a.0.cmp(&b.0));
        }
        (!rows.is_empty()).then_some((keys, mode, rows))
    }

    /// The bottom-right stack: the notification corner, then the
    /// which-key under it. One float, so they stack instead of
    /// covering each other.
    pub(crate) fn right_stack(&self, ui: &mut Ui<'_>) {
        let rows = self.whichkey_rows();
        if !self.corner_shown() && rows.is_none() {
            return;
        }
        ui.with_keyed(
            "right",
            NodeSpec::column()
                .float(
                    FloatConfig::viewport()
                        .at(Align::End, Align::End)
                        .self_at(Align::End, Align::End)
                        .offset(-12.0, -(2.0 * STRIP_H + 8.0)),
                )
                .gap(8.0)
                .cross_align(Align::End),
            |ui| {
                self.corner(ui);
                if let Some((keys, mode, rows)) = rows {
                    self.whichkey(ui, &keys, mode, &rows);
                }
            },
        );
    }

    /// The which-key card: the open keys and their group's name as the
    /// title, then the rows in columns.
    fn whichkey(
        &self,
        ui: &mut Ui<'_>,
        keys: &[String],
        mode: Mode,
        rows: &[(String, Vec<Binding>)],
    ) {
        let pal = self.pal;
        let km = &self.ed.keymap;
        // The binding a key would run now: a `j` is `commands next`
        // in the commands pane's field and `move down` elsewhere. A
        // key none of whose bindings can run here is left out.
        let view = self.keyed_view().or_else(|| self.ed.any_view());
        let pick = |bs: &[Binding]| -> Option<Binding> {
            view.and_then(|v| self.ed.pick_binding(v, bs).ok().cloned())
        };
        let rows: Vec<&(String, Vec<Binding>)> = rows
            .iter()
            .filter(|(_, bs)| bs.is_empty() || pick(bs).is_some())
            .collect();
        // `<leader><leader>` lists as the leader's key, not the word.
        let show = |k: &str| {
            if k == kawoosh_editor::keymap::LEADER {
                pretty(km.leader())
            } else {
                pretty(k)
            }
        };
        let mut title: String = keys.iter().map(|k| pretty(k)).collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            title = format!("{} mode", mode.word());
        }
        if let Some(name) = km.group_name(keys) {
            title = format!("{title} · {name}");
        }
        let per_column = if keys.is_empty() {
            rows.len().div_ceil(ROOT_COLUMNS).max(PER_COLUMN)
        } else {
            PER_COLUMN
        };
        let key_style = TextStyle::new(12.0).color(pal.accent).nowrap();
        let what_style = TextStyle::new(12.0).color(pal.fg).nowrap();
        let group_style = TextStyle::new(12.0).color(pal.dim).nowrap();
        ui.with_keyed(
            "whichkey",
            NodeSpec::column()
                .bg(pal.panel)
                .border(1.0, pal.border)
                .radius(4.0)
                .pad_xy(10.0, 6.0)
                .gap(4.0),
            |ui| {
                ui.text(&title, TextStyle::new(12.0).color(pal.dim).nowrap());
                // The root says how to see the other modes' roots.
                if keys.is_empty() {
                    ui.text(
                        "also :keys n · i · v · o",
                        TextStyle::new(11.0).color(pal.dim).nowrap(),
                    );
                }
                ui.with_keyed("cols", NodeSpec::row().gap(18.0), |ui| {
                    for (ci, chunk) in rows.chunks(per_column).enumerate() {
                        let chunk: Vec<&(String, Vec<Binding>)> = chunk.to_vec();
                        ui.with_indexed(ci as u64, NodeSpec::column().gap(2.0), |ui| {
                            for (ri, (k, b)) in chunk.iter().enumerate() {
                                ui.with_indexed(ri as u64, NodeSpec::row().gap(8.0), |ui| {
                                    ui.text(&show(k), key_style);
                                    match if b.is_empty() { None } else { pick(b) } {
                                        Some(b) => ui.text(&b.line(), what_style),
                                        None => {
                                            let mut deeper = keys.to_vec();
                                            deeper.push(k.clone());
                                            let what = match km.group_name(&deeper) {
                                                Some(name) => format!("+{name}"),
                                                None => format!(
                                                    "+{}",
                                                    km.next_keys(mode, &deeper).len()
                                                ),
                                            };
                                            ui.text(&what, group_style);
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

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("keys")
            .alias(&["whichkey"])
            .args(Args::new(&[ArgKind::Text]))
            .doc("the which-key for every first key of MODE (n, i, v, o; the current one bare), until the next press (<leader>?)"),
        |k, ctx| {
            let mode = match ctx.args.first() {
                None => Some(k.focused_mode()),
                Some(m) => Mode::from_short(m),
            };
            match mode {
                Some(m) => k.keys_help = Some(m),
                None => k.ed.message = "keys of which mode? (n, i, v, o)".into(),
            }
        },
    )]
}
