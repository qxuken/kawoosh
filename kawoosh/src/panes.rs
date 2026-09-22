//! Drawing the pane tree: splits with drag dividers, a title bar per
//! pane, the editor pane as rows (kui.md D3), the terminal pane as
//! `cells` (D4), the tab strip, the status strip and the command line;
//! and the scrolling tab (scrolling-tab.md) as a `scroll_x` row of
//! columns that slide.

use std::ops::Range;

use kawoosh_editor::search;
use kawoosh_editor::{Mode, ViewId, motions};
use kui::{Align, Enter, FloatConfig, NodeSpec, Role, Sizing, TextStyle, Ui, Value, Vec2};

use crate::app::{DIVIDER, Kawoosh, TAB_H, TITLE_H};
use crate::layout::{Content, Drop, Kind, Node, PaneId, SplitDir, Strip};
use crate::rows::{self, Caret, Drawn, GUTTER_W, LineDraw, STRIP_H, Window};
use crate::terminals::TermId;
use kawoosh_systems::lsp::DIAG_LAYER;
use kawoosh_systems::ts::{SYNTAX_LAYER, Token};

/// How long the ribbon takes to reach the column a key revealed, and
/// a column its place after a width step or a move: one duration, so
/// the two motions a key starts run together.
const RIBBON_MS: f32 = 160.0;

/// Frames on which the focused column is revealed after the strip's
/// shape changed. One: kui lays the reveal out in the same frame, and
/// a second ask against a ribbon already gliding would measure from
/// where the content has got to and stop the leg short of the column.
const STRIP_SETTLING: u8 = 1;

/// Where `zs` / `ze` / `zz` (`strip left` / `right` / `center`) put
/// the focused column in the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripAlign {
    Left,
    Right,
    Center,
}

/// A strip as last drawn, as far as a reveal cares: the tab, the
/// focus and its column, the columns' order and widths in px.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StripShape {
    pub tab: usize,
    pub focused: PaneId,
    pub column: Option<usize>,
    pub columns: Vec<(u64, u32)>,
}

impl Kawoosh {
    // ------------------------------------------------------------ view

    pub(crate) fn strip(&self, ui: &mut Ui<'_>, items: &[(&str, kui::Color)], right: &str) {
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
                        ui.text(t, rows::mono(self.face, &pal).color(*c));
                    }
                }
                ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                ui.text(right, rows::mono(self.face, &pal).color(pal.dim));
            },
        );
    }

    /// The bar, i3-style: workspaces as numbered blocks on the left — the
    /// focused one in the accent, the others quiet — and the facts on the
    /// right (the working directory, the LSP pool).
    pub(crate) fn tab_strip(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let font = self.face;
        let theme = ui.theme();
        ui.with(
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Fixed(TAB_H))
                .bg(pal.strip)
                .cross_align(Align::Center)
                .role(Role::TabList),
            |ui| {
                for (i, tab) in self.layout.tabs.iter().enumerate() {
                    let active = i == self.layout.tab;
                    let name = match self.layout.content(tab.focused) {
                        Some(Content::Editor(v)) => self.ed.buffer_of(v).name.clone(),
                        Some(Content::Terminal(t)) => self
                            .terms
                            .map
                            .get(&t)
                            .filter(|t| !t.title.is_empty())
                            .map(|t| t.title.clone())
                            .unwrap_or_else(|| "term".into()),
                        Some(Content::Lua(n)) => n,
                        Some(Content::Undo) => "undo".into(),
                        Some(Content::Memory) => "memory".into(),
                        None => "?".into(),
                    };
                    let mut ps = Vec::new();
                    tab.panes(&mut ps);
                    let modified = ps.iter().any(
                        |p| matches!(self.view_of(*p), Some(v) if self.ed.buffer_of(v).modified),
                    );
                    let label = format!("{}: {}{}", i + 1, name, if modified { " ●" } else { "" });
                    let (bg, fg, edge) = if active {
                        (theme.accent, theme.on_accent, theme.accent_hover)
                    } else {
                        (pal.strip, pal.dim, pal.border)
                    };
                    ui.with_indexed(
                        100 + i as u64,
                        NodeSpec::column()
                            .height(Sizing::Grow(1.0))
                            .bg(bg)
                            .hover_bg(if active {
                                theme.accent_hover
                            } else {
                                pal.panel
                            })
                            .on_click(Value::map([
                                ("kind", "tab".into()),
                                ("index", Value::Int(i as i64)),
                            ]))
                            .role(Role::Tab)
                            .selected(active)
                            .label(label.as_str()),
                        |ui| {
                            // i3's coloured top edge on the block.
                            ui.with(
                                NodeSpec::row()
                                    .width(Sizing::Grow(1.0))
                                    .height(Sizing::Fixed(2.0))
                                    .bg(edge),
                                |_| {},
                            );
                            ui.with(
                                NodeSpec::row()
                                    .height(Sizing::Grow(1.0))
                                    .pad_xy(10.0, 0.0)
                                    .cross_align(Align::Center),
                                |ui| {
                                    ui.text(&label, rows::mono(font, &pal).color(fg));
                                },
                            );
                        },
                    );
                    // A hairline between blocks, as i3 draws.
                    ui.with_indexed(
                        1000 + i as u64,
                        NodeSpec::column()
                            .width(Sizing::Fixed(1.0))
                            .height(Sizing::Grow(1.0))
                            .bg(pal.border),
                        |_| {},
                    );
                }
                ui.with_indexed(500, NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                // The status block: cwd, the pool.
                let cwd = kawoosh_systems::fs::abbreviate_home(&self.cwd);
                let mut blocks: Vec<(String, kui::Color)> = vec![(cwd, pal.fg)];
                if !self.lsp.status.is_empty() {
                    let n: usize = self.lsp.status.iter().map(|s| s.2).sum();
                    blocks.push((
                        format!(
                            "{} server{} · {n} docs",
                            self.lsp.status.len(),
                            if self.lsp.status.len() == 1 { "" } else { "s" }
                        ),
                        pal.dim,
                    ));
                }
                if self.compile.running {
                    blocks.push(("compiling…".into(), pal.command));
                }
                for (i, (text, color)) in blocks.iter().enumerate() {
                    if i > 0 {
                        ui.with_indexed(
                            2000 + i as u64,
                            NodeSpec::column()
                                .width(Sizing::Fixed(1.0))
                                .height(Sizing::Fixed(TAB_H - 10.0))
                                .bg(pal.border),
                            |_| {},
                        );
                    }
                    ui.with_indexed(
                        3000 + i as u64,
                        NodeSpec::row().pad_xy(10.0, 0.0).cross_align(Align::Center),
                        |ui| {
                            ui.text(text, rows::mono(font, &pal).color(*color));
                        },
                    );
                }
            },
        );
    }

    pub(crate) fn status(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let Some(view) = self.focused_view() else {
            let lua_name;
            let what = match self.layout.focused_content() {
                Some(Content::Undo) => "UNDO",
                Some(Content::Memory) => "MEMORY",
                Some(Content::Lua(n)) => {
                    lua_name = n.to_uppercase();
                    lua_name.as_str()
                }
                _ => "TERM",
            };
            // A pane with a field, or the prompt over it: the field's
            // mode first, as a buffer pane shows its own.
            match self.keyed_view().map(|v| &self.ed.views[v]) {
                Some(kv) => {
                    let (mode, color) = match kv.mode {
                        Mode::Insert => ("INS", pal.insert),
                        Mode::Visual if kv.visual_linewise => ("VIS LINE", pal.command),
                        Mode::Visual => ("VIS", pal.command),
                        _ => ("NOR", pal.accent),
                    };
                    self.strip(
                        ui,
                        &[(mode, color), (what, pal.accent)],
                        &self.strip_marks(),
                    );
                }
                None => self.strip(ui, &[(what, pal.accent)], &self.strip_marks()),
            }
            return;
        };
        let v = &self.ed.views[view];
        let buf = self.ed.buffer_of(view);
        let on_toast = self.notes.focus.is_some();
        // The mode shown is the keyboard's: the prompt's field while
        // one is open, else the pane's.
        let keyed = self.ed.prompt_view().unwrap_or(view);
        let kv = &self.ed.views[keyed];
        let mode = if on_toast {
            "TOAST"
        } else if kv.mode == Mode::Visual && kv.visual_linewise {
            "VIS LINE"
        } else {
            kv.mode.name()
        };
        let mode_color = match kv.mode {
            _ if on_toast => pal.command,
            Mode::Insert => pal.insert,
            Mode::Visual => pal.command,
            _ => pal.accent,
        };
        let name = match buf.loading {
            // Still on its way from the io thread: how far.
            Some((done, total)) => format!(
                "{} [opening {}%]",
                buf.name,
                (done * 100).checked_div(total).unwrap_or(100)
            ),
            None if buf.modified => format!("{} [+]", buf.name),
            None => buf.name.clone(),
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
        let marks = self.strip_marks();
        if !marks.is_empty() {
            right = format!("{marks}  {right}");
        }
        let pending: String = self.ed.pending.join("");
        let op = self
            .ed
            .pending_op
            .map(|(o, _)| o.chars().next().unwrap_or(' ').to_string())
            .unwrap_or_default();
        let count = self.ed.count.map(|c| c.to_string()).unwrap_or_default();
        let keys = format!("{count}{op}{pending}");
        // A recording under way, vim's `recording @a`, beside the mode.
        let rec = self.ed.recording().map(|c| format!("REC @{c}"));
        let mut items = vec![
            (mode, mode_color),
            (name.as_str(), pal.fg),
            (keys.as_str(), pal.dim),
        ];
        if let Some(rec) = &rec {
            items.insert(1, (rec.as_str(), pal.insert));
        }
        self.strip(ui, &items, &right);
    }

    /// A field's one line, drawn as a pane's row is (`rows::emit_line`):
    /// its selections, and when `keyed` — the keyboard is on it — a
    /// caret per selection, a bar in insert mode on kui's blink, a
    /// block otherwise, with `ghost` after the primary one. The line
    /// declares the caret, so kui's blink clock runs while the field
    /// has the keyboard; a field without it shows no caret, so one
    /// caret is on the screen at a time.
    pub(crate) fn field_line(
        &self,
        ui: &mut Ui<'_>,
        view: ViewId,
        keyed: bool,
        ghost: Option<&str>,
    ) {
        let pal = self.pal;
        let font = self.face;
        let Some(v) = self.ed.views.get(view) else {
            return;
        };
        let buf = &self.ed.buffers[v.buffer];
        let text = buf.text();
        let drawn = Drawn::new(&text, self.ed.tabstop());
        let clip = |o: usize| drawn.to_drawn(o.min(text.len()));
        let caret_kind = if v.mode == Mode::Insert {
            Caret::Bar
        } else {
            Caret::Block
        };
        let mut selected: Vec<Range<usize>> = Vec::new();
        let mut carets: Vec<(Range<usize>, Caret)> = Vec::new();
        let mut access = (None, None);
        let primary = v.sels.primary();
        for s in v.sels.iter() {
            let r = s.range();
            let (rs, re) = if v.mode == Mode::Visual {
                (r.start, buf.next_char(r.end).max(r.end + 1))
            } else {
                (r.start, r.end)
            };
            if rs < re {
                let b = if re > text.len() {
                    drawn.text.len() + 1
                } else {
                    clip(re)
                };
                selected.push(clip(rs)..b);
            }
            if !keyed {
                continue;
            }
            let end = if caret_kind == Caret::Block {
                clip(buf.next_char(s.head))
            } else {
                clip(s.head)
            };
            let kind = if caret_kind == Caret::Block && *s != primary {
                Caret::Extra
            } else {
                caret_kind
            };
            carets.push((clip(s.head)..end, kind));
            if *s == primary {
                access.0 = Some(clip(s.head) as u32);
                if !s.is_empty() {
                    access.1 = Some(clip(s.anchor) as u32);
                }
            }
        }
        rows::emit_line(
            ui,
            font,
            &pal,
            &LineDraw {
                text: &drawn.text,
                selected: &selected,
                hits: &[],
                flashed: &[],
                styled: &[],
                carets: &carets,
                escapes: &drawn.escapes,
                caret_on: ui.caret_visible() || v.mode != Mode::Insert,
                access,
                underlined: &[],
                trailing: None,
                ghost: ghost.map(|g| (clip(primary.head), g)),
                before: 0.0,
                after: 0.0,
            },
        );
    }

    pub(crate) fn command_line(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let font = self.face;
        ui.with(
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Fixed(STRIP_H))
                .bg(pal.bg)
                .pad_xy(8.0, 0.0)
                .cross_align(Align::Center),
            |ui| {
                if let (Some(field), Some(kind)) = (self.ed.prompt_view(), self.ed.prompt_kind()) {
                    ui.text(kind.sigil(), rows::mono(font, &pal).color(pal.command));
                    // The prompt's field: the editor's own line, with
                    // the completion's ghost after the caret, and the
                    // candidates as a row — the current one lit —
                    // clipped at the strip's end.
                    let ghost = self.cmdline_ghost();
                    self.field_line(ui, field, true, ghost.as_deref());
                    let line_len = self.ed.prompt_text().map_or(0, |t| t.len());
                    let candidates = self
                        .cmd_completion
                        .as_ref()
                        .filter(|c| c.candidates.len() > 1 && line_len > c.start)
                        .map(|c| (c.candidates.clone(), c.index));
                    if let Some((cands, index)) = candidates {
                        ui.with(
                            NodeSpec::row()
                                .width(Sizing::Grow(1.0))
                                .height(Sizing::Fixed(STRIP_H))
                                .pad_xy(16.0, 0.0)
                                .gap(12.0)
                                .cross_align(Align::Center)
                                .clip(),
                            |ui| {
                                for (i, c) in cands.iter().enumerate().take(40) {
                                    let color = if i == index { pal.fg } else { pal.dim };
                                    ui.with_indexed(i as u64, NodeSpec::row(), |ui| {
                                        ui.text(c, TextStyle::new(12.0).color(color).nowrap());
                                    });
                                }
                            },
                        );
                    }
                } else if !self.ed.message.is_empty() {
                    ui.text(&self.ed.message, TextStyle::new(12.0).color(pal.dim));
                }
            },
        );
    }

    /// The tab's body: the tree, or the strip.
    pub(crate) fn render_tab(&mut self, ui: &mut Ui<'_>) {
        match self.layout.tab().layout.clone() {
            Kind::Tree(root) => {
                self.culled.clear();
                self.render_node(ui, &root, "");
            }
            Kind::Scroll(strip) => self.render_strip(ui, &strip),
        }
    }

    /// A pane whose column is far off the viewport draws its chrome —
    /// so its rect, its title and its hit region are the ones it
    /// would have — and an empty box where its rows would be. A
    /// screenful of rows is what a pane costs (`render_editor` shapes
    /// and highlights every one); the box is what makes a ribbon of a
    /// hundred columns cost what the two on screen cost. True when it
    /// filled the box, so the caller returns.
    fn culled(&mut self, ui: &mut Ui<'_>, pane: PaneId) -> bool {
        if !self.culled.contains(&pane) {
            return false;
        }
        ui.with(
            NodeSpec::column()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .bg(self.pal.panel),
            |_| {},
        );
        true
    }

    /// The strip (scrolling-tab.md Decisions 3 and 4): one `scroll_x`
    /// row per tab — keyed by the tab, so each keeps the offset kui
    /// retains for it — a keyed column per `Column` at its width in
    /// viewport fractions, a draggable gap between.
    ///
    /// **The ribbon is what moves.** The row declares a `transition`,
    /// so the offset a reveal takes it to is eased over `RIBBON_MS`
    /// (kui F80, asked for from here) while the thumb, the wheel and a
    /// swipe — the hand's own, which must never lag a finger — land
    /// whole and drop any leg in flight. Nothing else eases a position:
    /// a column's width and place snap, so a preset step and `<C-w>H`
    /// are as fast as the key, and a width easing cannot retarget the
    /// neighbours every frame, which was the wobble. `slide` on a
    /// column is what that costs, and it is no loss: `slide` eases a
    /// node's *viewport* position, so a column carrying one eases the
    /// ribbon's scrolling too — the lag under the scrollbar's thumb,
    /// found 2026-09-22 — and cannot tell the two apart.
    ///
    /// What is left is a column arriving in a strip already on show: it
    /// comes in from a third of its width away and fades up, the same
    /// `RIBBON_MS`, so it lands as the ribbon does. The strip's first
    /// frame (a conversion, a restore, a tab switched to) does neither,
    /// those columns not being arrivals. A closing one goes at once: an
    /// `exit` fade would replay on a tab switch too, kui playing a
    /// ghost whether or not its ancestors survived.
    ///
    /// None of it costs a keystroke: kui hands a key to the sink that
    /// holds focus wherever the frame drew it (its F79, also asked for
    /// from here — before it, a key typed at a column an animation had
    /// not finished moving fell on the floor).
    ///
    /// On a frame the strip's shape changed — the focus, the order, a
    /// width, the tab — the focused column is revealed, so a column
    /// widened at the right edge comes wholly into view; `zs` `ze` `zz`
    /// (`strip left` / `right` / `center`) and `layout.scroll.center`
    /// put it at an edge or in the middle instead.
    pub(crate) fn render_strip(&mut self, ui: &mut Ui<'_>, strip: &Strip) {
        let pal = self.pal;
        let vw = ui.viewport().w.max(1.0);
        let gap = self.strip_gap();
        let focused = self.layout.focused();
        let fi = strip.column_of(focused);
        let n = strip.columns.len();
        let widths: Vec<f32> = strip
            .columns
            .iter()
            .map(|c| (c.width.fraction() * vw).round().max(120.0))
            .collect();
        let on_show = strip
            .columns
            .iter()
            .any(|c| self.strip_known.contains(&c.id));
        let arriving = on_show
            && strip
                .columns
                .iter()
                .any(|c| !self.strip_known.contains(&c.id));
        let tab = self.layout.tab;
        // The shape a reveal answers to, read before the columns are
        // declared: whether they slide this frame is the same
        // question.
        let shape = StripShape {
            tab,
            focused,
            column: fi,
            columns: strip
                .columns
                .iter()
                .zip(&widths)
                .map(|(c, w)| (c.id, *w as u32))
                .collect(),
        };
        if self.strip_seen.as_ref() != Some(&shape) {
            self.strip_seen = Some(shape);
            self.strip_settling = STRIP_SETTLING;
        }
        let settling = self.strip_settling > 0 && self.dragging.is_none();
        // Which columns are worth their rows this frame: the ones the
        // ribbon's offset puts within half a viewport of it, and the
        // focused one wherever it is. Read from the model — the widths
        // and the retained offset — so a column swiped into view has
        // its rows on the frame it arrives, rather than one frame
        // later as a drawn rect would give. Not while anything moves:
        // a sliding column is drawn between two places, and only the
        // one it is going to is known here.
        self.culled.clear();
        if !settling && !arriving {
            // Where the ribbon *is*, which during a glide is not where
            // it is going (kui F80): the geometry answers the drawn
            // offset, and describes the frame before, which the half a
            // viewport of slack covers.
            let key = ui.child_key(&format!("strip{tab}"));
            let offset = ui
                .scroll_geometry(key)
                .map(|g| g.offset.x)
                .unwrap_or_else(|| ui.scroll_offset(key).x);
            let mut left = 0.0;
            for (i, col) in strip.columns.iter().enumerate() {
                let (x0, x1) = (left - offset, left - offset + widths[i]);
                left += widths[i] + gap;
                if Some(i) == fi || (x1 > -vw * 0.5 && x0 < vw * 1.5) {
                    continue;
                }
                let mut ps = Vec::new();
                col.node.panes(&mut ps);
                self.culled.extend(ps);
            }
        }
        let mut focus_key = None;
        let row = ui.with_keyed(
            &format!("strip{tab}"),
            NodeSpec::row()
                .fill()
                .scroll_x()
                // The ribbon glides to the column a key reveals (kui's
                // F80, asked for from here); the thumb and a swipe are
                // the hand's and land whole.
                .transition(RIBBON_MS)
                .cross_align(Align::Start)
                .label("strip"),
            |ui| {
                for (i, col) in strip.columns.iter().enumerate() {
                    let px = widths[i];
                    let mut wrap = NodeSpec::column().height(Sizing::Grow(1.0));
                    if arriving && !self.strip_known.contains(&col.id) {
                        wrap = wrap
                            .transition(RIBBON_MS)
                            .enter(Enter::from((px / 3.0).min(200.0), 0.0).opacity(0.0));
                    }
                    let key = ui.with_keyed(&format!("col{}", col.id), wrap, |ui| {
                        ui.with(
                            NodeSpec::column()
                                .width(Sizing::Fixed(px))
                                .height(Sizing::Grow(1.0)),
                            |ui| self.render_node(ui, &col.node, &format!("{i}/")),
                        );
                    });
                    if Some(i) == fi {
                        focus_key = Some(key);
                    }
                    if i + 1 < n {
                        let path = format!("gap{i}");
                        let divider = ui.child_key(&format!("gap{}", col.id));
                        let active = ui.is_hovered(divider)
                            || ui.is_pressed(divider)
                            || self.dragging.as_deref() == Some(path.as_str());
                        let bar = NodeSpec::column()
                            .width(Sizing::Fixed(gap))
                            .height(Sizing::Grow(1.0))
                            .bg(if active { pal.accent } else { pal.border })
                            .cursor(kui::CursorShape::EwResize)
                            .on_drag(Value::map([
                                ("kind", "split".into()),
                                ("path", Value::str(&path)),
                                ("dir", "h".into()),
                            ]));
                        ui.with_keyed(&format!("gap{}", col.id), bar, |_| {});
                    }
                }
            },
        );
        self.strip_known.extend(strip.columns.iter().map(|c| c.id));
        // Asked on the frame the shape changed and the two after it,
        // so the reveal reads a layout the `on_layout` events have
        // caught up with; a reveal of a column in view is a no-op. Not
        // under a gap drag: the offset moving under the pointer would
        // feed the width it is measuring.
        let align = self.strip_align.take().or_else(|| {
            (self.ed.settings.str("layout.scroll.center") == Some("always"))
                .then_some(StripAlign::Center)
        });
        if settling || align.is_some() {
            self.strip_settling = self.strip_settling.saturating_sub(1);
            if let (Some(i), Some(key)) = (fi, focus_key) {
                match align {
                    Some(a) => {
                        let left: f32 = widths[..i].iter().sum::<f32>() + gap * i as f32;
                        let x = match a {
                            StripAlign::Left => left,
                            StripAlign::Right => left + widths[i] - vw,
                            StripAlign::Center => left - (vw - widths[i]) / 2.0,
                        };
                        ui.set_scroll(row, Vec2::new(x.max(0.0), 0.0));
                    }
                    None => ui.reveal(key),
                }
            }
        }
    }

    /// Where the keyboard is along a strip, for the status line: a
    /// mark per column, the focused one filled — `▯▮▯` — so a column
    /// off the viewport is not out of mind. Empty for a tree.
    pub(crate) fn strip_marks(&self) -> String {
        let Some(s) = self.layout.tab().strip() else {
            return String::new();
        };
        let at = s.column_of(self.layout.tab().focused);
        (0..s.columns.len())
            .map(|i| if Some(i) == at { '▮' } else { '▯' })
            .collect()
    }

    /// The gap between a strip's columns (`layout.gap`, px; the
    /// divider's width by default).
    pub(crate) fn strip_gap(&self) -> f32 {
        self.ed
            .settings
            .int("layout.gap")
            .map(|g| g.clamp(0, 64) as f32)
            .unwrap_or(DIVIDER)
    }

    pub(crate) fn render_node(&mut self, ui: &mut Ui<'_>, node: &Node, path: &str) {
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

    pub(crate) fn render_pane(&mut self, ui: &mut Ui<'_>, pane: PaneId) {
        let pal = self.pal;
        let focused = self.layout.focused() == pane;
        let content = self.layout.content(pane);
        let (name, modified) = match &content {
            Some(Content::Editor(v)) => {
                let b = self.ed.buffer_of(*v);
                (b.name.clone(), b.modified)
            }
            Some(Content::Terminal(t)) => (
                self.terms
                    .map
                    .get(t)
                    .map(|t| {
                        if t.title.is_empty() {
                            "terminal".to_string()
                        } else {
                            t.title.clone()
                        }
                    })
                    .unwrap_or_else(|| "terminal".into()),
                false,
            ),
            Some(Content::Lua(n)) => (n.clone(), false),
            // Named for the buffer it follows.
            Some(Content::Undo) => (
                match self.undo.view.filter(|v| self.ed.views.contains_key(*v)) {
                    Some(v) => format!("undo · {}", self.ed.buffer_of(v).name),
                    None => "undo".into(),
                },
                false,
            ),
            Some(Content::Memory) => ("memory".into(), false),
            None => ("?".into(), false),
        };
        // A tab's pane goes where its title bar is dragged; the dock is
        // not in the tree and stays put.
        let draggable = self.layout.dock != Some(pane);
        let dragged = self.pane_drag.is_some_and(|(p, _, _)| p == pane);
        let drop = self
            .pane_drag
            .and_then(|(p, x, y)| self.layout.drop_at(x, y).filter(|(t, _)| *t != p))
            .filter(|(t, _)| *t == pane)
            .map(|(_, d)| d);
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
                // The title bar: a click focuses, a drag moves the pane.
                let mut title = NodeSpec::row()
                    .width(Sizing::Grow(1.0))
                    .height(Sizing::Fixed(TITLE_H))
                    .bg(if dragged {
                        pal.accent.with_alpha(0.3)
                    } else if focused {
                        pal.strip
                    } else {
                        pal.bg
                    })
                    .pad_xy(8.0, 0.0)
                    .gap(6.0)
                    .cross_align(Align::Center)
                    .on_click(Value::map([
                        ("kind", "focus".into()),
                        ("pane", Value::Int(pane as i64)),
                    ]))
                    .label(name.as_str());
                if draggable {
                    title = title
                        .on_drag(Value::map([
                            ("kind", "panedrag".into()),
                            ("pane", Value::Int(pane as i64)),
                        ]))
                        .cursor(if dragged {
                            kui::CursorShape::Grabbing
                        } else {
                            kui::CursorShape::Grab
                        });
                }
                ui.with(title, |ui| {
                    ui.text(
                        &name,
                        TextStyle::new(12.0).color(if focused { pal.fg } else { pal.dim }),
                    );
                    if modified {
                        ui.text("●", TextStyle::new(10.0).color(pal.command));
                    }
                });
                // Where the dragged pane would land here: the whole pane
                // for a swap, the half on the side it would take.
                if let Some(drop) = drop {
                    let (x, y, w, h) = match drop {
                        Drop::Swap => (Align::Start, Align::Start, 1.0, 1.0),
                        Drop::Left => (Align::Start, Align::Start, 0.5, 1.0),
                        Drop::Right => (Align::End, Align::Start, 0.5, 1.0),
                        Drop::Up => (Align::Start, Align::Start, 1.0, 0.5),
                        Drop::Down => (Align::Start, Align::End, 1.0, 0.5),
                    };
                    ui.with_keyed(
                        "drop",
                        NodeSpec::column()
                            .float(FloatConfig::parent().at(x, y).self_at(x, y))
                            .width(Sizing::Percent(w))
                            .height(Sizing::Percent(h))
                            .bg(pal.accent.with_alpha(0.25))
                            .border(2.0, pal.accent)
                            .label("drop"),
                        |_| {},
                    );
                }
                match &content {
                    Some(Content::Editor(v)) => self.render_editor(ui, pane, *v, focused),
                    Some(Content::Terminal(t)) => self.render_terminal(ui, pane, *t, focused),
                    Some(Content::Lua(n)) => self.render_lua_pane(ui, pane, n, focused),
                    Some(Content::Undo) => self.render_undo(ui, pane, focused),
                    Some(Content::Memory) => self.render_memory(ui, pane, focused),
                    None => {}
                }
            },
        );
    }

    pub(crate) fn render_terminal(
        &mut self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        id: TermId,
        focused: bool,
    ) {
        if self.culled(ui, pane) {
            return;
        }
        let pal = self.pal;
        let font = self.face;
        let pad = 4.0;
        let (w, h) = self
            .layout
            .rects
            .get(&pane)
            .map(|r| (r.w - 2.0 - 2.0 * pad, r.h - TITLE_H - 2.0 - 2.0 * pad))
            .unwrap_or((800.0, self.body_h - TITLE_H));
        self.fit_terminal(id, w, h);
        let Some(term) = self.terms.map.get(&id) else {
            return;
        };
        // The palette is the frame's (`sync_term_palettes`).
        let screen = term.screen();
        let cursor = screen.cursor.map(|(r, c, shape)| {
            (
                r,
                c,
                shape,
                if focused {
                    pal.accent
                } else {
                    pal.accent.with_alpha(0.4)
                },
            )
        });
        let grid = kui::CellGrid {
            rows: screen.rows,
            cols: screen.cols,
            cells: &screen.cells,
            style: rows::mono(font, &pal),
            cursor,
            origin_line: screen.origin_line,
        };
        let tag = Value::map([("kind", "term".into()), ("pane", Value::Int(pane as i64))]);
        // A program reporting the mouse gets drags as reports and no cell
        // selection — unless shift is held, the terminal convention for
        // "my selection, not yours".
        let reporting = term.wants_mouse() && !self.mods.3;
        let drag_tag = Value::map([
            ("kind", "termmouse".into()),
            ("pane", Value::Int(pane as i64)),
        ]);
        let sink = ui.with_keyed(
            "term",
            NodeSpec::column()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .pad(pad)
                .clip()
                .on_key(tag.clone())
                // A click past the grid's last cell focuses too.
                .on_click(tag.clone())
                .cursor(kui::CursorShape::Text)
                .label("terminal"),
            |ui| {
                let mut spec = NodeSpec::column().on_click(tag.clone()).on_scroll(tag);
                spec = if reporting {
                    spec.on_drag(drag_tag)
                } else {
                    spec.selectable()
                };
                ui.cells_keyed("cells", &grid, spec);
            },
        );
        if focused {
            self.focus_sink(ui, sink);
        }
    }

    pub(crate) fn render_editor(
        &mut self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        view: ViewId,
        focused: bool,
    ) {
        if self.culled(ui, pane) {
            return;
        }
        let pal = self.pal;
        let font = self.face;
        let height = self
            .layout
            .rects
            .get(&pane)
            .map(|r| r.h - TITLE_H - 2.0)
            .unwrap_or(self.body_h - TITLE_H);
        let rows_n = ((height / self.face.line_height).floor().max(1.0)) as usize;
        let scrolloff = self
            .ed
            .settings
            .int("scrolloff")
            .map(|n| n.max(0) as usize)
            .unwrap_or(3)
            .min(rows_n / 2);
        let tabstop = self.ed.tabstop();
        // The long lines' cell indexes, out of `self` for the rows below
        // (which borrow the buffer) and back at the end.
        let mut cells = std::mem::take(&mut self.line_cells);
        let mode = if focused {
            self.ed.mode(view)
        } else {
            Mode::Normal
        };
        let linewise = self.ed.views[view].visual_linewise;
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
        let search = self
            .ed
            .search
            .as_ref()
            .filter(|_| self.ed.search_hl)
            .map(|s| s.re.clone());
        // The last yank's ranges, while they are washed (`sync_flash`
        // ends it; an edit since makes the ranges another text's).
        let flash: Vec<Range<usize>> = self
            .ed
            .flash
            .as_ref()
            .filter(|f| f.buffer == buf_id && f.version == buf.version())
            .filter(|f| f.at.elapsed() < crate::app::FLASH)
            .map(|f| f.ranges.clone())
            .unwrap_or_default();
        // One caret on the screen: the view the keyboard is on. A pane
        // whose keyboard is on the prompt draws none — while a search
        // prompt previews, the match it landed on is drawn as a
        // selection instead (vim's IncSearch).
        let keyed = focused && self.ed.prompt_view().is_none();
        let previewing = focused
            && matches!(
                self.ed.prompt_kind(),
                Some(kawoosh_editor::Prompt::Search { .. })
            )
            && self.ed.prompt_from() == Some(view);
        let top = v.top;
        let mut left = v.left;
        let last = (top + rows_n).min(buf.line_count());
        let sels = &v.sels;
        let primary = sels.primary();
        let cur_line = buf.line_of(primary.head);
        let title = buf.name.clone();
        let dark = ui.theme().is_dark();
        let diag_messages = self.lsp.messages.get(&buf_id);
        let diag_colors = [pal.dim, pal.danger, pal.command, pal.dim, pal.faint];
        // The notes on the rows drawn (`kawoosh.buf.annotate`): each
        // on the tracked line it was put on, wherever the line is now
        // (`Runtime::notes_on`, as of this frame's publish) — none once
        // the line is deleted. A listing of forty thousand lines notes
        // them all, and draws forty.
        let annotated: std::collections::HashMap<usize, String> = self
            .scripting
            .rt
            .as_ref()
            .map(|rt| rt.notes_on(buf_id, buf.version(), top..last))
            .unwrap_or_default();
        let ghost = self
            .completion_typed()
            .filter(|(v, _)| *v == view && focused)
            .and_then(|(_, typed)| self.lsp.completion.as_ref()?.ghost(&typed));
        let caret_kind = if mode == Mode::Insert {
            Caret::Bar
        } else {
            Caret::Block
        };
        // The token colours, once: a minified line has ten thousand runs.
        let token_colors: Vec<Option<kui::Color>> = Token::ALL
            .iter()
            .map(|t| self.syntax_color_for(*t, dark))
            .collect();
        let tag = Value::map([("kind", "pane".into()), ("pane", Value::Int(pane as i64))]);
        let cell_w = self.cell.0;
        // The lines column's width, for the sideways follow and the
        // window a long line is sliced to: the pane's less the gutter
        // and its border.
        let width = self
            .layout
            .rects
            .get(&pane)
            .map(|r| (r.w - GUTTER_W - 2.0).max(0.0))
            .unwrap_or(0.0);
        // Scroll the caret into view sideways, a few columns of margin,
        // the way `top` follows it down — before the rows, which are
        // sliced to the window this lands on. A long line's caret is
        // placed by column, as its slice is.
        if self.follow_caret || !focused {
            let range = buf.line_range(cur_line);
            let head_rel = primary.head.clamp(range.start, range.end) - range.start;
            let window = Window {
                left,
                width,
                cell_w,
            };
            let index = (range.len() >= rows::LONG_LINE_BYTES)
                .then(|| cells.get(buf_id, buf, &range, tabstop).clone());
            let (drawn, (c0, c1)) = Drawn::for_line(
                buf,
                range.clone(),
                tabstop,
                Some(window),
                head_rel,
                index.as_ref(),
            );
            let head = drawn.to_drawn(head_rel);
            let style = rows::mono(font, &pal);
            let long = range.len() >= rows::LONG_LINE_BYTES;
            let (x0, x1) = if long {
                (c0 as f32 * cell_w, c1 as f32 * cell_w)
            } else {
                let x0 = ui.measure_text(&drawn.text[..head], &style, None).width;
                let x1 = if head < drawn.text.len() {
                    let next = rows::next_char(&drawn.text, head);
                    ui.measure_text(&drawn.text[..next], &style, None).width
                } else {
                    x0
                };
                (x0, x1)
            };
            let x1 = if head_rel >= range.len() {
                x0 + 8.0
            } else {
                x1
            };
            let margin = (cell_w * 3.0).min(width / 4.0);
            if x0 - margin < left {
                left = (x0 - margin).max(0.0);
            } else if width > 0.0 && x1 + margin > left + width {
                left = x1 + margin - width;
            }
        }

        let sink = ui.with_keyed(
            "editor",
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .clip()
                .on_key(tag.clone())
                .on_drag(tag.clone())
                .on_scroll(tag.clone())
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
                // The column scrolls sideways under a line wider than the
                // pane, at an offset the view owns (`left`, as `top`):
                // the wheel over it reaches the app through `on_scroll`
                // — a kui scroll container under the pointer would take
                // the notch itself, both axes, and `top` would never
                // hear it — and the app hands the offset back each frame.
                let lines = ui.with_keyed(
                    "lines",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(1.0))
                        .scroll_x()
                        .on_scroll(tag.clone()),
                    |ui| {
                        for ln in top..last {
                            let range = buf.line_range(ln);
                            let window = Window {
                                left,
                                width,
                                cell_w,
                            };
                            let index = (range.len() >= rows::LONG_LINE_BYTES)
                                .then(|| cells.get(buf_id, buf, &range, tabstop));
                            let (drawn, _) = Drawn::for_line(
                                buf,
                                range.clone(),
                                tabstop,
                                Some(window),
                                0,
                                index,
                            );
                            let clip = |o: usize| {
                                drawn.to_drawn(o.clamp(range.start, range.end) - range.start)
                            };
                            // The runs of the drawn slice alone: a long
                            // line's window, not its ten thousand runs.
                            let src = drawn.src_range();
                            let src = range.start + src.start..range.start + src.end;
                            let mut selected: Vec<Range<usize>> = Vec::new();
                            let mut carets: Vec<(Range<usize>, Caret)> = Vec::new();
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
                                if head_line == ln && keyed {
                                    let end = if caret_kind == Caret::Block {
                                        clip(buf.next_char(s.head))
                                    } else {
                                        clip(s.head)
                                    };
                                    let kind = if caret_kind == Caret::Block && *s != primary {
                                        Caret::Extra
                                    } else {
                                        caret_kind
                                    };
                                    carets.push((clip(s.head)..end, kind));
                                }
                                if *s == primary && head_line == ln && keyed {
                                    access.0 = Some(clip(s.head) as u32);
                                    if !s.is_empty() && buf.line_of(s.anchor) == ln {
                                        access.1 = Some(clip(s.anchor) as u32);
                                    }
                                }
                            }
                            // The search's matches in the drawn slice, found
                            // now: a few kilobytes of regex per row, and
                            // nothing kept for the rows off screen.
                            let hits: Vec<Range<usize>> = match &search {
                                Some(re) => search::hits_in(buf.tree(), re, src.clone())
                                    .into_iter()
                                    .map(|r| clip(r.start)..clip(r.end.min(range.end)))
                                    .filter(|r| r.start < r.end)
                                    .collect(),
                                None => Vec::new(),
                            };
                            if previewing && ln == cur_line {
                                let at = clip(primary.head);
                                if let Some(h) = hits.iter().find(|h| h.start == at) {
                                    selected.push(h.clone());
                                }
                            }
                            let flashed: Vec<Range<usize>> = flash
                                .iter()
                                .filter(|r| r.start < range.end && r.end > range.start)
                                .map(|r| clip(r.start.max(range.start))..clip(r.end.min(range.end)))
                                .filter(|r| r.start < r.end)
                                .collect();
                            let styled: Vec<(Range<usize>, kui::Color)> = buf
                                .runs(SYNTAX_LAYER, src.clone())
                                .iter()
                                .filter_map(|r| {
                                    let c =
                                        token_colors.get(r.style as usize).copied().flatten()?;
                                    let a = clip(r.range.start);
                                    let b = clip(r.range.end.min(range.end));
                                    (a < b).then_some((a..b, c))
                                })
                                .collect();
                            let diags = buf.runs(DIAG_LAYER, range.clone());
                            let underlined: Vec<(Range<usize>, kui::Color)> = diags
                                .iter()
                                .filter_map(|r| {
                                    let a = clip(r.range.start);
                                    let b = clip(r.range.end.min(range.end));
                                    let c = diag_colors[(r.style as usize).min(4)];
                                    (a < b).then_some((a..b, c))
                                })
                                .collect();
                            let trailing = diags
                                .first()
                                .and_then(|r| {
                                    let m = diag_messages?.get(r.tag as usize)?;
                                    Some((m.as_str(), diag_colors[(r.style as usize).min(4)]))
                                })
                                .or_else(|| annotated.get(&ln).map(|t| (t.as_str(), pal.dim)));
                            let ghost_here = ghost
                                .as_deref()
                                .filter(|_| ln == cur_line)
                                .map(|g| (clip(primary.head), g));
                            rows::emit_line(
                                ui,
                                font,
                                &pal,
                                &LineDraw {
                                    text: &drawn.text,
                                    selected: &selected,
                                    hits: &hits,
                                    flashed: &flashed,
                                    styled: &styled,
                                    carets: &carets,
                                    escapes: &drawn.escapes,
                                    caret_on: blink_on || mode != Mode::Insert,
                                    access,
                                    underlined: &underlined,
                                    trailing,
                                    ghost: ghost_here,
                                    before: drawn.before_cols as f32 * cell_w,
                                    after: drawn.after_cols as f32 * cell_w,
                                },
                            );
                        }
                    },
                );
                // The offset lands in this frame's positions; the clamp
                // is against last frame's content (a resize is one frame
                // late), and what the wheel pushed past it comes back.
                if let Some(geo) = ui.scroll_geometry(lines) {
                    left = left.min(geo.max_offset.x);
                }
                ui.set_scroll(lines, Vec2::new(left, 0.0));
            },
        );
        self.ed.views[view].left = left;
        self.line_cells = cells;
        if focused {
            self.focus_sink(ui, sink);
        }
    }
}
