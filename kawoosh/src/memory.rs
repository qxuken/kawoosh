//! The working memory as a pane of its own (`:memory`, `<leader>p`),
//! beside the buffer the way the undo tree is: every moment the
//! engine remembers (`Editor::memory`) — what was yanked, deleted,
//! changed away, or pasted in from the clipboard — newest at the top,
//! with how it came, the buffer it came from and when; under the rows,
//! the cursor's moment as lines. The `"` register is the memory's
//! head, so the pane is the register's past: anything that passed
//! through the hands can be put again, and a moment put again is the
//! register from then on — a plugin that reads the register's origin
//! (the file manager, adopting a pasted line) reads a recalled moment's
//! as it would a fresh yank's.
//!
//! `⏎` or a click puts the cursor's moment in the editor pane the
//! keyboard came from, after the caret as `p` does; `y` recalls it —
//! the register, without putting it; `o` goes to where it came from,
//! its bytes carried through the buffer's edits since (`line_carried`);
//! `x` forgets it; `q` closes the pane, `<Esc>` hands the keyboard to
//! an editor pane.
//!
//! Every size is `devtab::Tab`'s, as the undo and history panes' are.

use std::time::Instant;

use kawoosh_doc::BufferId;
use kawoosh_editor::{KeyStroke, Lookup, Mode, Selection, Selections, Spec, Took, ViewId};
use kui::{Color, NodeSpec, Sizing, Ui, Value, Vec2};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::devtab::Tab;
use crate::layout::{Content, PaneId, SplitDir};
use crate::rows;
use crate::undo::PANEL_SHARE;

/// How many of a moment's lines the pane shows under the rows.
const LINES_SHOWN: usize = 200;

/// What the pane keeps between frames.
#[derive(Default)]
pub struct MemoryPanel {
    /// The panel's own cursor: a moment's index in `Memory::moments`
    /// (oldest first; the top row is the last).
    pub cursor: usize,
    /// The editor pane the keyboard came from: where a put goes.
    pub back: Option<ViewId>,
    pub(crate) prefix: bool,
    reveal: bool,
}

/// Where a moment's origin stands now.
enum Origin {
    Unknown,
    Closed,
    Gone,
    At(BufferId, usize),
}

impl Kawoosh {
    /// `:memory`: the pane in a split beside the focused pane, or
    /// focused when on show, or — focused already — closed: a toggle,
    /// as the undo pane is. The editor pane the keyboard leaves is
    /// where a put goes.
    pub(crate) fn toggle_memory_panel(&mut self) {
        if let Some(v) = self.focused_view() {
            self.memory_pane.back = Some(v);
        }
        let shown = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| self.layout.content(*p) == Some(Content::Memory));
        match shown {
            Some(p) if self.layout.focused() == p => self.close_memory_panel(p),
            Some(p) => self.layout.focus(p),
            None => {
                let pane = self.layout.split(SplitDir::H, Content::Memory);
                if let Some(path) = self.layout.tab().root.split_of(pane)
                    && let Some(r) = self.layout.tab_mut().root.ratio_mut(&path)
                {
                    *r = 1.0 - PANEL_SHARE;
                }
                self.memory_pane.cursor = self.ed.memory.len().saturating_sub(1);
                self.memory_pane.reveal = true;
            }
        }
    }

    fn close_memory_panel(&mut self, pane: PaneId) {
        if self.layout.close(pane).is_none() {
            self.ed.message = "cannot close the last pane".into();
        }
    }

    /// The editor pane a put or a jump goes to: the one the keyboard
    /// came from while it is on show, else any editor pane on show.
    fn memory_target(&self) -> Option<(PaneId, ViewId)> {
        let panes = self.layout.visible_panes();
        if let Some(v) = self.memory_pane.back
            && let Some(p) = panes.iter().find(|p| self.view_of(**p) == Some(v))
        {
            return Some((*p, v));
        }
        panes
            .into_iter()
            .find_map(|p| self.view_of(p).map(|v| (p, v)))
    }

    /// Moment `i` recalled — the `"` register — and put after the caret
    /// of the editor pane the keyboard came from, which takes the
    /// keyboard.
    fn put_moment(&mut self, i: usize) {
        if !self.ed.memory.recall(i) {
            return;
        }
        self.memory_pane.cursor = self.ed.memory.len().saturating_sub(1);
        let Some((pane, view)) = self.memory_target() else {
            self.ed.message = "no editor pane to put it in".into();
            return;
        };
        self.layout.focus(pane);
        self.ed.run(view, "paste after", &[], None);
        self.follow_caret = true;
    }

    /// Moment `i` recalled: the register, nothing put.
    fn recall_moment(&mut self, i: usize) {
        if !self.ed.memory.recall(i) {
            return;
        }
        self.memory_pane.cursor = self.ed.memory.len().saturating_sub(1);
        let head = self
            .ed
            .memory
            .head()
            .map(|m| preview(&m.text))
            .unwrap_or_default();
        self.ed.message = format!("recalled: {head}");
    }

    /// Where moment `i` came from, as of now.
    fn origin_of(&self, i: usize) -> Origin {
        let Some(m) = self.ed.memory.moments().get(i) else {
            return Origin::Unknown;
        };
        let Some(o) = &m.origin else {
            return Origin::Unknown;
        };
        let Some(b) = self.ed.buffers.get(o.buffer) else {
            return Origin::Closed;
        };
        match b.line_carried(o.range.clone(), o.version, b.version()) {
            Some(r) if !r.is_empty() => Origin::At(o.buffer, r.start),
            _ => Origin::Gone,
        }
    }

    /// The editor pane the keyboard came from shows where moment `i`
    /// came from, the caret on it.
    fn goto_origin(&mut self, i: usize) {
        let from = self
            .ed
            .memory
            .moments()
            .get(i)
            .map(|m| m.from.clone())
            .unwrap_or_default();
        match self.origin_of(i) {
            Origin::Unknown => self.ed.message = format!("{from}: where it came from is not known"),
            Origin::Closed => self.ed.message = format!("{from} is not open"),
            Origin::Gone => self.ed.message = format!("its text is gone from {from}"),
            Origin::At(id, at) => {
                let Some((pane, view)) = self.memory_target() else {
                    self.ed.message = "no editor pane to show it in".into();
                    return;
                };
                self.layout.focus(pane);
                self.show_buffer(view, id);
                let len = self.ed.buffers[id].len();
                self.ed.views[view].sels = Selections::single(Selection::point(at.min(len)));
                self.follow_caret = true;
            }
        }
    }

    /// A key while the pane has the keyboard.
    pub(crate) fn memory_key_press(&mut self, pane: PaneId, stroke: KeyStroke) {
        let note = stroke.notation();
        if self.memory_pane.prefix {
            self.memory_pane.prefix = false;
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
        let n = self.ed.memory.len();
        let cursor = self.memory_pane.cursor.min(n.saturating_sub(1));
        self.memory_pane.cursor = cursor;
        match note.as_str() {
            "<C-w>" => self.memory_pane.prefix = true,
            ":" => self.open_cmdline(),
            // Newest at the top, as the undo pane: down is older.
            "j" | "<Down>" => {
                self.memory_pane.cursor = cursor.saturating_sub(1);
                self.memory_pane.reveal = true;
            }
            "k" | "<Up>" => {
                self.memory_pane.cursor = (cursor + 1).min(n.saturating_sub(1));
                self.memory_pane.reveal = true;
            }
            "G" => {
                self.memory_pane.cursor = 0;
                self.memory_pane.reveal = true;
            }
            "<CR>" | "<Space>" | "p" => self.put_moment(cursor),
            "y" => self.recall_moment(cursor),
            "o" => self.goto_origin(cursor),
            "x" => {
                if self.ed.memory.forget(cursor) {
                    self.memory_pane.cursor = cursor.min(self.ed.memory.len().saturating_sub(1));
                    self.memory_pane.reveal = true;
                }
            }
            "q" => self.close_memory_panel(pane),
            "<Esc>" => {
                if let Some((p, _)) = self.memory_target() {
                    self.layout.focus(p);
                }
            }
            _ => {}
        }
    }

    /// A click on a row: the pane takes the keyboard and the moment is
    /// put.
    pub(crate) fn on_memory_click(&mut self, p: &Value) {
        if let Some(pane) = p.get("pane").and_then(Value::as_int) {
            self.layout.focus(pane as PaneId);
        }
        if let Some(i) = p.get("row").and_then(Value::as_int) {
            let i = (i.max(0) as usize).min(self.ed.memory.len().saturating_sub(1));
            self.memory_pane.cursor = i;
            self.put_moment(i);
        }
    }

    pub(crate) fn render_memory(&mut self, ui: &mut Ui<'_>, pane: PaneId, focused: bool) {
        let tm = Tab::of(&ui.metrics(), self.face.line_height);
        let pal = self.pal;
        let font = self.face;
        let (cell_w, _) = self.cell;
        let style = move || rows::mono(font, &pal);
        let dim = move || style().color(pal.dim);
        let small = move |c: Color| tm.small(c);
        let col = move |cells: f32| tm.cell(cells, cell_w);
        let n = self.ed.memory.len();
        let cursor = self.memory_pane.cursor.min(n.saturating_sub(1));
        self.memory_pane.cursor = cursor;
        let reveal = std::mem::take(&mut self.memory_pane.reveal);
        let now = Instant::now();
        let origin = match self.origin_of(cursor) {
            Origin::Unknown => "where from is not known",
            Origin::Closed => "its buffer is closed",
            Origin::Gone => "its text is gone from there",
            Origin::At(..) => "still there (o)",
        };
        let moments: Vec<(Took, bool, String, String, std::time::Duration)> = self
            .ed
            .memory
            .moments()
            .iter()
            .map(|m| {
                (
                    m.took,
                    m.linewise,
                    m.text.clone(),
                    m.from.clone(),
                    now.saturating_duration_since(m.at),
                )
            })
            .collect();
        let tag = Value::map([("kind", "memory".into()), ("pane", Value::Int(pane as i64))]);
        let sink = ui.with_keyed(
            "memory",
            NodeSpec::column()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .clip()
                .on_key(tag.clone())
                .on_click(tag.clone())
                .label("memory"),
            |ui| {
                ui.with(tm.strip(&pal).on_click(tag.clone()), |ui| {
                    let head = format!("{n} moment{}", if n == 1 { "" } else { "s" });
                    ui.text(&head, small(pal.dim));
                    ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                    ui.text(
                        "⏎ put · y recall · o origin · x forget · q close",
                        small(pal.faint),
                    );
                });
                ui.with(tm.line(&pal, 0).hover_bg(Color::TRANSPARENT), |ui| {
                    ui.with(col(9.0).main_align(kui::Align::Start), |ui| {
                        ui.text("took", small(pal.faint))
                    });
                    ui.with(tm.rest(), |ui| ui.text("text", small(pal.faint)));
                    ui.with(col(14.0).main_align(kui::Align::Start), |ui| {
                        ui.text("from", small(pal.faint))
                    });
                    ui.with(col(8.0), |ui| ui.text("when", small(pal.faint)));
                });
                kui::widgets::virtual_column(
                    ui,
                    "rows",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(3.0)),
                    n,
                    tm.line_h,
                    |ui, d| {
                        // Newest at the top.
                        let i = n - 1 - d;
                        let (took, _, text, from, age) = &moments[i];
                        let mut line = tm.line(&pal, d);
                        if i == cursor {
                            line = line.bg(if focused {
                                pal.select
                            } else {
                                pal.select.with_alpha(0.4)
                            });
                        } else if i + 1 == n {
                            line = line.bg(pal.strip);
                        }
                        let payload = Value::map([
                            ("kind", "memory".into()),
                            ("pane", Value::Int(pane as i64)),
                            ("row", Value::Int(i as i64)),
                        ]);
                        let label = format!("moment {}", i + 1);
                        let lines = text.matches('\n').count() + usize::from(!text.ends_with('\n'));
                        ui.with_keyed(
                            &label,
                            line.on_click(payload)
                                .cursor(kui::CursorShape::Pointer)
                                .label(label.as_str()),
                            |ui| {
                                ui.with(col(9.0).main_align(kui::Align::Start), |ui| {
                                    let color = match took {
                                        Took::Yank => pal.insert,
                                        Took::Delete | Took::Change => pal.danger,
                                        Took::Clipboard => pal.dim,
                                    };
                                    ui.text(took.word(), small(color));
                                });
                                ui.with(tm.rest(), |ui| {
                                    ui.text(&preview(text), style().color(pal.fg));
                                    if lines > 1 {
                                        ui.text(
                                            &format!(
                                                "+{} line{}",
                                                lines - 1,
                                                if lines == 2 { "" } else { "s" }
                                            ),
                                            small(pal.faint),
                                        );
                                    }
                                });
                                ui.with(col(14.0).main_align(kui::Align::Start), |ui| {
                                    ui.text(from, dim());
                                });
                                ui.with(col(8.0), |ui| {
                                    ui.text(&crate::settings::ago(*age), style().color(pal.faint));
                                });
                            },
                        );
                    },
                );
                let list = ui.child_key("rows");
                if reveal && n > 0 {
                    let y = (n - 1 - cursor) as f32 * tm.line_h;
                    let seen = ui
                        .scroll_geometry(list)
                        .is_some_and(|g| g.offset.y <= y && y + tm.line_h <= g.offset.y + g.rect.h);
                    if !seen {
                        let h = ui.scroll_geometry(list).map_or(0.0, |g| g.rect.h);
                        ui.set_scroll(list, Vec2::new(0.0, (y - h / 2.0).max(0.0)));
                    }
                }
                // The cursor's moment: what it is, then its lines.
                ui.with(tm.strip(&pal), |ui| {
                    let head = match moments.get(cursor) {
                        Some((took, linewise, text, from, age)) => {
                            let lines =
                                text.matches('\n').count() + usize::from(!text.ends_with('\n'));
                            format!(
                                "{} from {from} · {lines} line{}{} · {} · {} · {origin}",
                                took.word(),
                                if lines == 1 { "" } else { "s" },
                                if *linewise { ", whole" } else { "" },
                                crate::perf::bytes(text.len() as u64),
                                crate::settings::ago(*age),
                            )
                        }
                        None => {
                            "nothing remembered yet: a yank, a delete, a paste from the clipboard"
                                .into()
                        }
                    };
                    ui.text(&head, small(pal.dim));
                });
                ui.with_keyed(
                    "moment",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(2.0))
                        .scroll_y()
                        .clip(),
                    |ui| {
                        let Some((_, _, text, _, _)) = moments.get(cursor) else {
                            return;
                        };
                        let shown: Vec<&str> = text.lines().take(LINES_SHOWN).collect();
                        for (k, l) in shown.iter().enumerate() {
                            ui.with(tm.line(&pal, k).hover_bg(Color::TRANSPARENT), |ui| {
                                ui.with(tm.rest(), |ui| {
                                    ui.text(&l.replace('\t', "    "), style());
                                });
                            });
                        }
                        let all = text.lines().count();
                        if all > shown.len() {
                            ui.with(
                                tm.line(&pal, shown.len()).hover_bg(Color::TRANSPARENT),
                                |ui| {
                                    ui.text(
                                        &format!("… {} more", all - shown.len()),
                                        small(pal.faint),
                                    );
                                },
                            );
                        }
                    },
                );
            },
        );
        if focused {
            self.focus_sink(ui, sink);
        }
    }
}

/// A moment's first line, as a row shows it.
fn preview(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    let first = first.replace('\t', "    ");
    if first.is_empty() && !text.is_empty() {
        "⏎".into()
    } else {
        first
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("memory")
            .doc("the working memory: what was yanked, deleted or pasted in, to put again"),
        |k, _| k.toggle_memory_panel(),
    )]
}
