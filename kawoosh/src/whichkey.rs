//! A which-key: while a key sequence is open — `<leader>`, `g`, `gs`,
//! `]`, `<C-w>` — a small card at the bottom-right lists what can
//! follow it, each key with its command and each group with its name
//! (`Keymap::describe`, `:map group`), so the clusters of
//! docs/design/keys.md can be read off the screen instead of
//! remembered. It is there the moment the prefix is pressed and gone
//! the moment the sequence resolves; `:keys` (`<leader>?`) shows the
//! root — every first key — until the next press. `whichkey = false`
//! in the settings (`:set -whichkey`) turns it off.
//!
//! The card is in the bottom-right stack, shared with the notification
//! corner: the corner's lines above, the which-key below, so the two
//! never cover each other.

use kawoosh_editor::{ArgKind, Args, Binding, Mode, Spec, ViewId};
use kui_native::{Align, FloatConfig, NodeSpec, TextStyle, Ui};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};

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
        // After the terminal's escape: normal mode's keys, from the
        // first (terminal-keys.md Decision 1).
        if let Some(keys) = &self.terms.escape
            && self.term_of(self.layout.focused()).is_some()
        {
            return Some((keys.clone(), Mode::Normal));
        }
        if let Some(mode) = self.keys_help {
            return Some((Vec::new(), mode));
        }
        None
    }

    /// The view the keys are resolved on: the one the keyboard is on,
    /// else the resident pane view a terminal's escape and a pane
    /// without one read their keys on.
    fn keys_view(&self) -> Option<ViewId> {
        self.keyed_view()
            .or_else(|| self.ed.find_field(kawoosh_editor::PANE_FIELD))
            .or_else(|| self.ed.any_view())
    }

    /// The places of the view the keys are resolved on, innermost
    /// first (local-maps.md): asked once for a card, not per row.
    fn keys_scopes(&self) -> Vec<String> {
        self.keys_view()
            .map(|v| self.ed.key_scopes(v))
            .unwrap_or_default()
    }

    /// How many of the keys that can follow `keys` in `mode` do
    /// something here: a key whose binding `pick` finds runnable, or
    /// one with such a key under it, `depth` levels down at most.
    fn live_next(
        &self,
        scopes: &[String],
        mode: Mode,
        keys: &[String],
        pick: &dyn Fn(&[Binding]) -> Option<Binding>,
        depth: usize,
    ) -> usize {
        let km = &self.ed.keymap;
        let mut next = km.next_keys_in(scopes, mode, keys);
        // As the rows are gathered: visual and operator-pending fall
        // through to normal mode's keys, and a pane to what every pane
        // shares (`<C-w>`, the leader).
        if matches!(mode, Mode::Visual | Mode::OperatorPending)
            || (mode == Mode::Pane && (keys.is_empty() || km.shared_from_pane(keys)))
        {
            for (k, b) in km.next_keys_in(scopes, Mode::Normal, keys) {
                if !next.iter().any(|(o, _)| *o == k) {
                    next.push((k, b));
                }
            }
        }
        next.iter()
            .filter(|(k, bs)| {
                pick(bs).is_some() || {
                    depth > 0 && {
                        let mut deeper = keys.to_vec();
                        deeper.push(k.clone());
                        self.live_next(scopes, mode, &deeper, pick, depth - 1) > 0
                    }
                }
            })
            .count()
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
        let scopes = self.keys_scopes();
        let mut rows = km.next_keys_in(&scopes, mode, &keys);
        // Visual and operator-pending lookups fall through to normal
        // mode's, so its sequences are open there too; a pane's for
        // what every pane shares (`<C-w>`, the leader); insert mode's
        // do not.
        if matches!(mode, Mode::Visual | Mode::OperatorPending)
            || (mode == Mode::Pane && (keys.is_empty() || km.shared_from_pane(&keys)))
        {
            for (k, b) in km.next_keys_in(&scopes, Mode::Normal, &keys) {
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
                        .inside(Align::End, Align::End)
                        .offset(-12.0, -(2.0 * self.chrome.strip_h + 8.0)),
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
        let small = self.chrome.small;
        let km = &self.ed.keymap;
        // The binding a key would run now: a `j` is `commands next`
        // in the commands pane's field and `move down` elsewhere. A
        // key none of whose bindings can run here, and under which no
        // key can either, is left out (roadmap step 61): a group whose
        // every key is gated off here is no group here.
        let view = self.keys_view();
        let scopes = self.keys_scopes();
        let pick = |bs: &[Binding]| -> Option<Binding> {
            view.and_then(|v| self.ed.pick_binding(v, bs).ok().cloned())
        };
        let live_under = |k: &str| -> usize {
            let mut deeper = keys.to_vec();
            deeper.push(k.to_string());
            self.live_next(&scopes, mode, &deeper, &pick, 4)
        };
        let rows: Vec<(&String, Option<Binding>, usize)> = rows
            .iter()
            .map(|(k, bs)| (k, pick(bs), live_under(k)))
            .filter(|(_, b, under)| b.is_some() || *under > 0)
            .collect();
        if rows.is_empty() {
            return;
        }
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
        let key_style = TextStyle::new(small).color(pal.accent).nowrap();
        let what_style = TextStyle::new(small).color(pal.fg).nowrap();
        let group_style = TextStyle::new(small).color(pal.dim).nowrap();
        ui.with_keyed(
            "whichkey",
            NodeSpec::column()
                .bg(pal.panel)
                .border(1.0, pal.border)
                .radius(4.0)
                .pad_xy(10.0, 6.0)
                .gap(4.0),
            |ui| {
                ui.text(&title, TextStyle::new(small).color(pal.dim).nowrap());
                // The root says how to see the other modes' roots.
                if keys.is_empty() {
                    ui.text(
                        "also :keys n · i · v · o",
                        TextStyle::new(small - 1.0).color(pal.dim).nowrap(),
                    );
                }
                ui.with_keyed("cols", NodeSpec::row().gap(18.0), |ui| {
                    for (ci, chunk) in rows.chunks(per_column).enumerate() {
                        ui.with_indexed(ci as u64, NodeSpec::column().gap(2.0), |ui| {
                            for (ri, (k, b, under)) in chunk.iter().enumerate() {
                                ui.with_indexed(ri as u64, NodeSpec::row().gap(8.0), |ui| {
                                    ui.text(&show(k), key_style);
                                    match b {
                                        Some(b) => ui.text(&b.line(), what_style),
                                        None => {
                                            let mut deeper = keys.to_vec();
                                            deeper.push((*k).clone());
                                            let what = match km.group_name(&deeper) {
                                                Some(name) => format!("+{name}"),
                                                None => format!("+{under}"),
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
