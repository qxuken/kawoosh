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
use crate::icons::{self, KeyStyle};

/// Rows per column before the list folds into another column, fewer
/// when the window holds fewer.
const PER_COLUMN: usize = 8;
/// The columns the root listing spreads over in a window tall enough;
/// a shorter one takes more, as many as its width holds.
const ROOT_COLUMNS: usize = 4;
/// The card's padding, across and down.
const CARD_PAD_X: f32 = 10.0;
const CARD_PAD_Y: f32 = 6.0;
/// Between the card's title, hint and columns.
const CARD_GAP: f32 = 4.0;
/// Between the columns, the rows, and a row's key and command.
const COLUMN_GAP: f32 = 18.0;
const ROW_GAP: f32 = 2.0;
const KEY_GAP: f32 = 8.0;
/// The bottom-right stack's distance from the window's right edge.
const STACK_INSET: f32 = 12.0;

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

    /// How far the bottom-right stack's foot is above the window's
    /// bottom: over the status line and the message line.
    fn stack_foot(&self) -> f32 {
        2.0 * self.chrome.strip_h + 8.0
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
                        .offset(-STACK_INSET, -self.stack_foot()),
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
        // Each key as caps (icons.md Decision 4), the notation as the
        // keymap has it: `<leader><leader>` lists as the leader's key.
        let show = |k: &str| {
            if k == kawoosh_editor::keymap::LEADER {
                km.leader().to_string()
            } else {
                k.to_string()
            }
        };
        let rows: Vec<Row> = rows
            .iter()
            .map(|(k, b, under)| match b {
                Some(b) => Row {
                    key: show(k),
                    what: b.line(),
                    group: false,
                    to: None,
                },
                None => {
                    let mut deeper = keys.to_vec();
                    deeper.push((*k).clone());
                    Row {
                        key: show(k),
                        what: match km.group_name(&deeper) {
                            Some(name) => format!("+{name}"),
                            None => format!("+{under}"),
                        },
                        group: true,
                        to: None,
                    }
                }
            })
            .collect();
        let rows = fold_numbered(rows);
        let mut title: String = keys.iter().map(|k| pretty(k)).collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            title = format!("{} mode", mode.word());
        }
        if let Some(name) = km.group_name(keys) {
            title = format!("{title} · {name}");
        }
        let key_style = TextStyle::new(small).color(pal.accent).nowrap();
        let caps = KeyStyle::new(key_style, pal.border);
        let icon_set = self.icons.borrow();
        let what_style = TextStyle::new(small).color(pal.fg).nowrap();
        let group_style = TextStyle::new(small).color(pal.dim).nowrap();
        let title_style = TextStyle::new(small).color(pal.dim).nowrap();
        let hint_style = TextStyle::new(small - 1.0).color(pal.dim).nowrap();
        let hint = keys.is_empty().then_some("also :keys n · i · v · o");

        // The card fits the window: as many rows to a column as its
        // height holds between the tab strip and the stack's foot, and
        // as many columns as its width holds; what is past them is
        // counted in the title.
        let vp = ui.viewport();
        let row_h = ui.measure_text("Mg", &key_style, None).height;
        let mut head_h = CARD_PAD_Y * 2.0 + 2.0 + ui.measure_text("Mg", &title_style, None).height;
        if let Some(hint) = hint {
            head_h += CARD_GAP + ui.measure_text(hint, &hint_style, None).height;
        }
        let room_h = vp.h - self.stack_foot() - self.chrome.tab_h - 8.0 - head_h - CARD_GAP;
        let fit = (((room_h + ROW_GAP) / (row_h + ROW_GAP)).floor() as usize).max(1);
        let wanted = if keys.is_empty() {
            rows.len().div_ceil(ROOT_COLUMNS).max(PER_COLUMN)
        } else {
            PER_COLUMN
        };
        let per_column = wanted.min(fit);
        let room_w = vp.w - 2.0 * STACK_INSET - CARD_PAD_X * 2.0 - 2.0;
        let mut used = 0.0;
        let mut shown = 0;
        // Each column's keys take its widest key's width, so its
        // commands start in one line: the UI font is proportional, an
        // `m` wider than an `l`.
        let mut key_w = Vec::new();
        for chunk in rows.chunks(per_column) {
            let (mut kw, mut ww) = (0.0f32, 0.0f32);
            for r in chunk {
                let what = if r.group { &group_style } else { &what_style };
                let mut w = icons::keys_width(ui, &r.key, &caps);
                if let Some(to) = r.to {
                    w += ui.measure_text(&format!("…{to}"), &key_style, None).width;
                }
                kw = kw.max(w.ceil());
                ww = ww.max(ui.measure_text(&r.what, what, None).width);
            }
            let w = kw + KEY_GAP + ww;
            let next = if shown == 0 { w } else { used + COLUMN_GAP + w };
            if shown > 0 && next > room_w {
                break;
            }
            used = next;
            shown += chunk.len();
            key_w.push(kw);
        }
        if shown < rows.len() {
            title = format!("{title} · {} more", rows.len() - shown);
        }
        ui.with_keyed(
            "whichkey",
            NodeSpec::column()
                .bg(pal.panel)
                .border(1.0, pal.border)
                .radius(4.0)
                .pad_xy(CARD_PAD_X, CARD_PAD_Y)
                .gap(CARD_GAP),
            |ui| {
                ui.text(&title, title_style);
                // The root says how to see the other modes' roots.
                if let Some(hint) = hint {
                    ui.text(hint, hint_style);
                }
                ui.with_keyed("cols", NodeSpec::row().gap(COLUMN_GAP), |ui| {
                    for (ci, chunk) in rows[..shown].chunks(per_column).enumerate() {
                        ui.with_indexed(ci as u64, NodeSpec::column().gap(ROW_GAP), |ui| {
                            for (ri, r) in chunk.iter().enumerate() {
                                ui.with_indexed(ri as u64, NodeSpec::row().gap(KEY_GAP), |ui| {
                                    ui.with(
                                        NodeSpec::row().width(key_w[ci]).cross_align(Align::Center),
                                        |ui| {
                                            icons::keys(ui, &icon_set, &r.key, &caps);
                                            if let Some(to) = r.to {
                                                ui.text(&format!("…{to}"), key_style);
                                            }
                                        },
                                    );
                                    let what = if r.group { group_style } else { what_style };
                                    ui.text(&r.what, what);
                                });
                            }
                        });
                    }
                });
            },
        );
    }
}

/// A row of the card as it reads: the key's notation, and its
/// command's line or its group's `+name`; a folded run's last digit.
struct Row {
    key: String,
    what: String,
    group: bool,
    to: Option<char>,
}

/// A run of keys that differ by a digit counting up, each running a
/// command that differs by the same digit — `A-1 memory pin 1` to
/// `A-9 memory pin 9` — is one row, `A-1…9 memory pin 1…9`: the
/// pinned memories and the panes by number are three runs of nine at
/// the root. A key's digit is its last character, or the last inside a
/// chord's brackets (`<A-1>`).
fn fold_numbered(rows: Vec<Row>) -> Vec<Row> {
    // The text around a last digit, and the digit.
    fn split(s: &str) -> Option<((&str, &str), u32)> {
        let end = if s.len() > 1 && s.ends_with('>') {
            s.len() - 1
        } else {
            s.len()
        };
        let d = s[..end].chars().last()?.to_digit(10)?;
        Some(((&s[..end - 1], &s[end..]), d))
    }
    // Row `r` continues a run from `first` at `d0`, `n` rows along.
    let follows = |first: &Row, r: &Row, n: u32| -> bool {
        let (Some((ks, d0)), Some((ws, w0))) = (split(&first.key), split(&first.what)) else {
            return false;
        };
        !first.group
            && !r.group
            && d0 == w0
            && split(&r.key) == Some((ks, d0 + n))
            && split(&r.what) == Some((ws, d0 + n))
    };
    let mut out = Vec::with_capacity(rows.len());
    let mut rows = rows.into_iter().peekable();
    while let Some(first) = rows.next() {
        let mut last = None;
        let mut n = 1;
        while let Some(r) = rows.next_if(|r| follows(&first, r, n)) {
            last = Some(r);
            n += 1;
        }
        match last {
            Some(last) if n >= 3 => {
                let d = last.what.chars().last().unwrap_or_default();
                out.push(Row {
                    key: first.key,
                    what: format!("{}…{d}", first.what),
                    group: false,
                    to: Some(d),
                });
            }
            Some(last) => {
                out.push(first);
                out.push(last);
            }
            None => out.push(first),
        }
    }
    out
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
