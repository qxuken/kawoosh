//! The command registry as a pane (`:commands`), a help with a search:
//! every spec the engine holds — its own, the shell's, a plugin's —
//! one a row with its keys, what it takes and what it does, and, in
//! the pane's colours, whether it can run where the keyboard came from
//! (kui.md Decision 12: the spec is data, and this is the data shown).
//! Typing filters — the name first, then an alias, then anything in
//! the row — `<Esc>` clears the query or, empty, hands the keyboard
//! back; `⏎` runs the cursor's command, or puts it on the command line
//! with a space after when it takes arguments; a click lands the cursor
//! on a row. Under the rows, the cursor's spec in full: aliases, the
//! forms and what each means, the conditions and which of them hold,
//! the keys in every mode.
//!
//! The rows are rebuilt when the registry, the keymap or the query
//! changed (`Registry::version`, `Keymap::version`), not once a frame;
//! whether a row can run is asked each frame, since that is what the
//! facts are for. Every size is `devtab::Tab`'s, as the other panes'.

use kawoosh_editor::{ArgKind, Args, Cond, KeyStroke, Mode, Spec, ViewId};
use kui::{Color, NodeSpec, Sizing, Ui, Value, Vec2};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::devtab::Tab;
use crate::layout::{Content, PaneId, SplitDir};
use crate::rows;

/// The pane's share of the width when it opens.
const SHARE: f32 = 0.5;

/// A command as the pane lists it.
#[derive(Clone, Debug)]
pub struct Row {
    pub spec: Spec,
    /// `n <CR>`, `n <leader>cd`: every key bound to it, by mode.
    pub keys: Vec<String>,
}

impl Row {
    fn haystack(&self) -> String {
        let mut s = self.spec.name.clone();
        for a in &self.spec.aliases {
            s.push(' ');
            s.push_str(a);
        }
        s.push(' ');
        s.push_str(&self.spec.doc);
        for k in &self.keys {
            s.push(' ');
            s.push_str(k);
        }
        s.to_lowercase()
    }

    /// How well `q` (lower-case) fits: the name's start, an alias's
    /// start, the name anywhere, anywhere at all; none.
    fn rank(&self, q: &str) -> Option<u8> {
        if q.is_empty() {
            return Some(0);
        }
        let name = self.spec.name.to_lowercase();
        if name.starts_with(q) {
            return Some(0);
        }
        if self
            .spec
            .aliases
            .iter()
            .any(|a| a.to_lowercase().starts_with(q))
        {
            return Some(1);
        }
        if name.contains(q) {
            return Some(2);
        }
        self.haystack().contains(q).then_some(3)
    }
}

#[derive(Default)]
pub struct CommandsPanel {
    /// The query is a field (`Editor::open_field`, named `commands`):
    /// the editor's own line, in insert mode to type, `<Esc>` to normal
    /// mode for motions over it. Opened with the pane, closed with it.
    pub field: Option<ViewId>,
    /// The panel's own cursor: a row's index in `rows`.
    pub cursor: usize,
    rows: Vec<Row>,
    /// The registry and keymap versions and the query the rows were
    /// built from.
    built: Option<(u64, u64, String)>,
    reveal: bool,
}

impl CommandsPanel {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
}

impl Kawoosh {
    /// `:commands [QUERY]`: the pane in a split beside the focused pane,
    /// or focused when on show, or — focused already — closed: a
    /// toggle, as the other panes are. A query given is typed in.
    pub(crate) fn toggle_commands_panel(&mut self, query: Option<&str>) {
        let shown = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| self.layout.content(*p) == Some(Content::Commands));
        match shown {
            Some(p) if self.layout.focused() == p && query.is_none() => {
                self.close_commands_panel(p)
            }
            Some(p) => self.layout.focus(p),
            None => {
                // Half the width: three columns of text need it, where
                // the undo and history panes' third does for theirs.
                let pane = self.layout.split(SplitDir::H, Content::Commands);
                if let Some(path) = self.layout.tab().root.split_of(pane)
                    && let Some(r) = self.layout.tab_mut().root.ratio_mut(&path)
                {
                    *r = 1.0 - SHARE;
                }
            }
        }
        let field = self.commands_field();
        if let Some(q) = query {
            self.ed.set_field_text(field, q);
            self.commands_pane.cursor = 0;
        }
        self.commands_pane.reveal = true;
    }

    /// The query's field, opened the first time it is asked for.
    pub(crate) fn commands_field(&mut self) -> ViewId {
        match self.commands_pane.field {
            Some(f) if self.ed.views.contains_key(f) => f,
            _ => {
                let f = self.ed.open_field("commands", "");
                self.commands_pane.field = Some(f);
                f
            }
        }
    }

    /// The query as it reads.
    pub fn commands_query(&self) -> String {
        self.commands_pane
            .field
            .and_then(|f| self.ed.field_text(f))
            .unwrap_or_default()
    }

    /// The pane is gone: its field goes with it.
    pub(crate) fn close_commands_field(&mut self) {
        if let Some(f) = self.commands_pane.field.take() {
            self.ed.close_field(f);
        }
    }

    fn close_commands_panel(&mut self, pane: PaneId) {
        if self.layout.close(pane).is_none() {
            self.ed.message = "cannot close the last pane".into();
            return;
        }
        self.close_commands_field();
    }

    /// The view the pane answers "can it run here" for: the keyboard's
    /// editor pane, else the first on show.
    fn commands_view(&self) -> Option<ViewId> {
        self.focused_view().or_else(|| {
            self.layout
                .visible_panes()
                .into_iter()
                .find_map(|p| self.view_of(p))
        })
    }

    /// The rows for the query, built again when the registry, the
    /// keymap or the query changed.
    fn sync_command_rows(&mut self) {
        let key = (
            self.ed.commands.version(),
            self.ed.keymap.version(),
            self.commands_query(),
        );
        if self.commands_pane.built.as_ref() == Some(&key) {
            return;
        }
        let q = key.2.trim().to_lowercase();
        let mut rows: Vec<(u8, Row)> = self
            .ed
            .commands
            .specs()
            .into_iter()
            .map(|spec| {
                let keys = self.keys_of(&spec.name);
                Row {
                    spec: spec.clone(),
                    keys,
                }
            })
            .filter_map(|r| r.rank(&q).map(|k| (k, r)))
            .collect();
        rows.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.spec.name.cmp(&b.1.spec.name))
        });
        let selected = self
            .commands_pane
            .rows
            .get(self.commands_pane.cursor)
            .map(|r| r.spec.name.clone());
        self.commands_pane.rows = rows.into_iter().map(|(_, r)| r).collect();
        // The cursor stays on its row when the row is still there; a
        // narrowed query lands it on the best match.
        let same_query = self.commands_pane.built.as_ref().map(|b| &b.2) == Some(&key.2);
        if !same_query {
            self.commands_pane.reveal = true;
        }
        self.commands_pane.cursor = match selected {
            Some(name) if same_query => self
                .commands_pane
                .rows
                .iter()
                .position(|r| r.spec.name == name)
                .unwrap_or(0),
            _ => 0,
        };
        self.commands_pane.built = Some(key);
    }

    /// Every key bound to `name` (its aliases too), as `mode keys`.
    fn keys_of(&self, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        for mode in [
            Mode::Normal,
            Mode::Visual,
            Mode::Insert,
            Mode::OperatorPending,
        ] {
            for (keys, b) in self.ed.keymap.bindings(mode) {
                let inv = self.ed.commands.resolve(&b.command, &b.args);
                if inv.name == name {
                    out.push(format!("{} {keys}", mode.short()));
                }
            }
        }
        out
    }

    /// `⏎`: the cursor's command run, or on the command line with a
    /// space after when it takes arguments.
    fn run_command_row(&mut self, i: usize) {
        let Some(row) = self.commands_pane.rows.get(i) else {
            return;
        };
        let name = row.spec.name.clone();
        if row.spec.args.kinds.is_empty() {
            self.shell_command(&name, &[], None);
        } else {
            self.open_cmdline();
            self.ed.set_prompt_text(&format!("{name} "));
            self.cmdline_refresh();
        }
    }

    /// A key on the pane: the query's field takes it — the editor's
    /// keys, and the pane's own as bindings `when field:commands`
    /// (`commands next|prev|run|leave`).
    pub(crate) fn commands_key_press(&mut self, _pane: PaneId, stroke: KeyStroke) {
        let field = self.commands_field();
        self.ed.key(field, stroke);
        self.sync_command_rows();
    }

    fn commands_step(&mut self, down: bool) {
        self.sync_command_rows();
        let n = self.commands_pane.rows.len();
        let p = &mut self.commands_pane;
        p.cursor = if down {
            (p.cursor + 1).min(n.saturating_sub(1))
        } else {
            p.cursor.saturating_sub(1)
        };
        p.reveal = true;
    }

    /// `<Esc>` in normal mode: the keyboard back to an editor pane.
    fn commands_leave(&mut self) {
        let back = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| self.view_of(*p).is_some());
        if let Some(p) = back {
            self.layout.focus(p);
        }
    }

    /// A click on a row: the pane takes the keyboard and the cursor
    /// lands on the row.
    pub(crate) fn on_commands_click(&mut self, p: &Value) {
        if let Some(pane) = p.get("pane").and_then(Value::as_int) {
            self.layout.focus(pane as PaneId);
        }
        if let Some(i) = p.get("row").and_then(Value::as_int) {
            self.sync_command_rows();
            let n = self.commands_pane.rows.len();
            self.commands_pane.cursor = (i.max(0) as usize).min(n.saturating_sub(1));
        }
    }

    pub(crate) fn render_commands(&mut self, ui: &mut Ui<'_>, pane: PaneId, focused: bool) {
        self.sync_facts();
        self.sync_command_rows();
        let tm = Tab::of(&ui.metrics());
        let pal = self.pal;
        let font = self.font;
        let (cell_w, _) = self.cell;
        let style = move || rows::mono(font, &pal);
        let small = move |c: Color| tm.small(c);
        let col = move |cells: f32| {
            tm.cell(cells, cell_w)
                .main_align(kui::Align::Start)
                .gap(tm.cell_gap)
                .clip()
        };
        let rows = std::mem::take(&mut self.commands_pane.rows);
        let n = rows.len();
        let cursor = self.commands_pane.cursor.min(n.saturating_sub(1));
        let reveal = std::mem::take(&mut self.commands_pane.reveal);
        let query = self.commands_query();
        let field = self.commands_field();
        let view = self.commands_view();
        let can: Vec<Result<(), String>> = rows
            .iter()
            .map(|r| self.ed.can(view, &r.spec.name))
            .collect();
        let runnable = can.iter().filter(|c| c.is_ok()).count();
        let total = self.ed.commands.specs().len();
        let tag = Value::map([
            ("kind", "commands".into()),
            ("pane", Value::Int(pane as i64)),
        ]);
        let sink = ui.with_keyed(
            "commands",
            NodeSpec::column()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .clip()
                .on_key(tag.clone())
                .label("commands"),
            |ui| {
                ui.with(tm.strip(&pal).on_click(tag.clone()), |ui| {
                    let head = if query.is_empty() {
                        format!("{total} commands · {runnable} can run here")
                    } else {
                        format!("{n} of {total} commands · {runnable} can run here")
                    };
                    ui.text(&head, small(pal.dim));
                    ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                    ui.text("type to search · ⏎ run · <Esc> back", small(pal.faint));
                });
                // The query line: the field, drawn as the prompt is —
                // its selections, a bar caret on kui's blink in insert
                // mode, a block in normal — and a placeholder after it
                // while it is empty.
                ui.with(
                    tm.line(&pal, 0).hover_bg(Color::TRANSPARENT).gap(0.0),
                    |ui| {
                        self.field_line(ui, field, None);
                        if query.is_empty() {
                            ui.text("search commands, keys, docs", style().color(pal.faint));
                        }
                    },
                );
                ui.with(tm.line(&pal, 0).hover_bg(Color::TRANSPARENT), |ui| {
                    ui.with(col(26.0), |ui| ui.text("command", small(pal.faint)));
                    ui.with(col(13.0), |ui| ui.text("keys", small(pal.faint)));
                    ui.with(tm.rest(), |ui| ui.text("does", small(pal.faint)));
                });
                kui::widgets::virtual_column(
                    ui,
                    "rows",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(3.0)),
                    n,
                    tm.line_h,
                    |ui, i| {
                        let r = &rows[i];
                        let ok = can[i].is_ok();
                        let mut line = tm.line(&pal, i);
                        if i == cursor {
                            line = line.bg(if focused {
                                pal.select
                            } else {
                                pal.select.with_alpha(0.4)
                            });
                        }
                        let payload = Value::map([
                            ("kind", "commands".into()),
                            ("pane", Value::Int(pane as i64)),
                            ("row", Value::Int(i as i64)),
                        ]);
                        let label = format!("command {}", r.spec.name);
                        let fg = if ok { pal.fg } else { pal.dim };
                        ui.with_keyed(
                            &label,
                            line.on_click(payload)
                                .cursor(kui::CursorShape::Pointer)
                                .label(label.as_str()),
                            |ui| {
                                ui.with(col(26.0), |ui| {
                                    // The name, its forms as marks on
                                    // it, the first ex spelling after.
                                    let marks = format!(
                                        "{}{}{}",
                                        r.spec.name,
                                        if r.spec.bang.is_some() { "!" } else { "" },
                                        if r.spec.query.is_some() { "?" } else { "" }
                                    );
                                    ui.text(&marks, style().color(fg));
                                    if let Some(a) = r.spec.aliases.first() {
                                        ui.text(&format!(":{a}"), small(pal.faint));
                                    }
                                });
                                ui.with(col(13.0), |ui| {
                                    if let Some(k) = r.keys.first() {
                                        ui.text(
                                            k,
                                            style().color(if ok { pal.dim } else { pal.faint }),
                                        );
                                    }
                                });
                                ui.with(tm.rest(), |ui| match &can[i] {
                                    Ok(()) => ui.text(&r.spec.doc, small(fg)),
                                    Err(reason) => ui.text(reason, small(pal.danger)),
                                });
                            },
                        );
                    },
                );
                let list = ui.child_key("rows");
                if reveal && n > 0 {
                    let y = cursor as f32 * tm.line_h;
                    let seen = ui
                        .scroll_geometry(list)
                        .is_some_and(|g| g.offset.y <= y && y + tm.line_h <= g.offset.y + g.rect.h);
                    if !seen {
                        let h = ui.scroll_geometry(list).map_or(0.0, |g| g.rect.h);
                        ui.set_scroll(list, Vec2::new(0.0, (y - h / 2.0).max(0.0)));
                    }
                }
                // The cursor's spec in full.
                ui.with(tm.strip(&pal), |ui| {
                    let head = match rows.get(cursor) {
                        Some(r) => match &can[cursor] {
                            Ok(()) => format!("{} · can run here", r.spec.name),
                            Err(reason) => reason.clone(),
                        },
                        None => "no command matches".into(),
                    };
                    ui.text(&head, small(pal.dim));
                });
                ui.with_keyed(
                    "inspect",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(2.0))
                        .scroll_y()
                        .clip(),
                    |ui| {
                        let Some(r) = rows.get(cursor) else {
                            return;
                        };
                        let s = &r.spec;
                        let facts = self.ed.facts(view);
                        let when = if s.when.is_empty() {
                            "always".to_string()
                        } else {
                            s.when
                                .iter()
                                .map(|c| {
                                    let holds = facts.holds(&c.fact) == c.holds;
                                    format!(
                                        "{c} ({})",
                                        if holds { "holds" } else { "does not hold" }
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(" · ")
                        };
                        let mut lines: Vec<(&str, String)> = vec![
                            ("does", s.doc.clone()),
                            (
                                "aliases",
                                if s.aliases.is_empty() {
                                    "none".into()
                                } else {
                                    s.aliases
                                        .iter()
                                        .map(|a| format!(":{a}"))
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                },
                            ),
                            (
                                "takes",
                                if s.args.kinds.is_empty() {
                                    "nothing".into()
                                } else {
                                    args_text(&s.args)
                                },
                            ),
                        ];
                        if let Some(b) = &s.bang {
                            lines.push(("with !", b.clone()));
                        }
                        if let Some(q) = &s.query {
                            lines.push(("with ?", q.clone()));
                        }
                        lines.push(("when", when));
                        lines.push((
                            "keys",
                            if r.keys.is_empty() {
                                "not bound".into()
                            } else {
                                r.keys.join(" · ")
                            },
                        ));
                        let subs = self.ed.commands.subcommands(&s.name);
                        if !subs.is_empty() {
                            lines.push(("subcommands", subs.join(" ")));
                        }
                        for (k, (name, value)) in lines.iter().enumerate() {
                            ui.with(tm.line(&pal, k).hover_bg(Color::TRANSPARENT), |ui| {
                                ui.with(col(12.0), |ui| ui.text(name, style().color(pal.dim)));
                                ui.with(tm.rest(), |ui| ui.text(value, style()));
                            });
                        }
                    },
                );
            },
        );
        self.commands_pane.rows = rows;
        if focused {
            self.focus_sink(ui, sink);
        }
    }
}

/// `path text...` as the spec spells it.
fn args_text(args: &Args) -> String {
    args.names().join(" ")
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("commands")
                .alias(&["cmds", "help"])
                .args(Args::new(&[ArgKind::Command]))
                .doc("every command as a pane, searched as you type; QUERY starts the search"),
            |k, ctx| k.toggle_commands_panel(ctx.args.first().map(String::as_str)),
        ),
        cmd(
            Spec::new("commands next")
                .when(&["field:commands"])
                .doc("the pane's cursor a row down"),
            |k, _| k.commands_step(true),
        ),
        cmd(
            Spec::new("commands prev")
                .when(&["field:commands"])
                .doc("the pane's cursor a row up"),
            |k, _| k.commands_step(false),
        ),
        cmd(
            Spec::new("commands run").when(&["field:commands"]).doc(
                "run the cursor's command, or put it on the command line when it takes arguments",
            ),
            |k, _| {
                k.sync_command_rows();
                k.run_command_row(k.commands_pane.cursor);
            },
        ),
        cmd(
            Spec::new("commands leave")
                .when(&["field:commands"])
                .doc("the keyboard back to an editor pane"),
            |k, _| k.commands_leave(),
        ),
    ]
}

/// The pane's keys on its query, over the editor's on the same keys.
pub(crate) fn bind(km: &mut kawoosh_editor::Keymap) {
    let at = [Cond::parse("field:commands")];
    for mode in [Mode::Insert, Mode::Normal] {
        km.bind_when(mode, "<CR>", "commands run", &at);
        km.bind_when(mode, "<Down>", "commands next", &at);
        km.bind_when(mode, "<Up>", "commands prev", &at);
        km.bind_when(mode, "<C-n>", "commands next", &at);
        km.bind_when(mode, "<C-p>", "commands prev", &at);
        km.bind_when(mode, "<C-j>", "commands next", &at);
        km.bind_when(mode, "<C-k>", "commands prev", &at);
    }
    km.bind_when(Mode::Normal, "<Esc>", "commands leave", &at);
}
