//! Drawing the pane tree: splits with drag dividers, a title bar per
//! pane, the editor pane as rows (kui.md D3), the terminal pane as
//! `cells` (D4), the tab strip, the status strip and the command line.

use std::ops::Range;

use kawoosh_editor::commands::SEARCH_LAYER;
use kawoosh_editor::{Mode, Prompt, ViewId, motions};
use kui::{Align, NodeSpec, Role, Sizing, TextStyle, Ui, Value, Vec2};

use crate::app::{DIVIDER, Kawoosh, TAB_H, TITLE_H};
use crate::layout::{Content, Node, PaneId, SplitDir};
use crate::rows::{self, Caret, Drawn, GUTTER_W, LH, LineDraw, STRIP_H, Window};
use crate::terminals::TermId;
use kawoosh_systems::lsp::DIAG_LAYER;
use kawoosh_systems::ts::{SYNTAX_LAYER, Token};

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
                        ui.text(t, rows::mono(self.font, &pal).color(*c));
                    }
                }
                ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                ui.text(right, rows::mono(self.font, &pal).color(pal.dim));
            },
        );
    }

    /// The bar, i3-style: workspaces as numbered blocks on the left — the
    /// focused one in the accent, the others quiet — and the facts on the
    /// right (the working directory, the LSP pool).
    pub(crate) fn tab_strip(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let font = self.font;
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
                        None => "?".into(),
                    };
                    let mut ps = Vec::new();
                    tab.root.panes(&mut ps);
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
                let home = std::env::var_os("HOME")
                    .map(|h| h.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let cwd = self.cwd.display().to_string();
                let cwd = match cwd.strip_prefix(&home) {
                    Some(rest) if !home.is_empty() => format!("~{rest}"),
                    _ => cwd,
                };
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
            self.strip(ui, &[("TERM", pal.accent)], "");
            return;
        };
        let v = &self.ed.views[view];
        let buf = self.ed.buffer_of(view);
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

    pub(crate) fn command_line(&self, ui: &mut Ui<'_>) {
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
            None => ("?".into(), false),
        };
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
                // The title bar.
                ui.with(
                    NodeSpec::row()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Fixed(TITLE_H))
                        .bg(if focused { pal.strip } else { pal.bg })
                        .pad_xy(8.0, 0.0)
                        .gap(6.0)
                        .cross_align(Align::Center)
                        .on_click(Value::map([
                            ("kind", "focus".into()),
                            ("pane", Value::Int(pane as i64)),
                        ]))
                        .label(name.as_str()),
                    |ui| {
                        ui.text(
                            &name,
                            TextStyle::new(12.0).color(if focused { pal.fg } else { pal.dim }),
                        );
                        if modified {
                            ui.text("●", TextStyle::new(10.0).color(pal.command));
                        }
                    },
                );
                match &content {
                    Some(Content::Editor(v)) => self.render_editor(ui, pane, *v, focused),
                    Some(Content::Terminal(t)) => self.render_terminal(ui, pane, *t, focused),
                    Some(Content::Lua(n)) => self.render_lua_pane(ui, pane, n, focused),
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
        let pal = self.pal;
        let font = self.font;
        let pad = 4.0;
        let (w, h) = self
            .layout
            .rects
            .get(&pane)
            .map(|r| (r.w - 2.0 - 2.0 * pad, r.h - TITLE_H - 2.0 - 2.0 * pad))
            .unwrap_or((800.0, self.body_h - TITLE_H));
        self.fit_terminal(id, w, h);
        let term_pal = kawoosh_term::Palette {
            fg: pal.fg.to_hex(),
            bg: pal.panel.to_hex(),
            ..Default::default()
        };
        let Some(term) = self.terms.map.get(&id) else {
            return;
        };
        let screen = term.screen(&term_pal);
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
            ui.take_key_focus(sink);
        }
    }

    pub(crate) fn render_editor(
        &mut self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        view: ViewId,
        focused: bool,
    ) {
        let pal = self.pal;
        let font = self.font;
        let height = self
            .layout
            .rects
            .get(&pane)
            .map(|r| r.h - TITLE_H - 2.0)
            .unwrap_or(self.body_h - TITLE_H);
        let rows_n = ((height / LH).floor().max(1.0)) as usize;
        let scrolloff = self
            .ed
            .option("scrolloff")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(3)
            .min(rows_n / 2);
        let tabstop = self.ed.tabstop();
        let mode = if focused { self.ed.mode } else { Mode::Normal };
        let linewise = self.ed.visual_linewise;
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
            let (drawn, (c0, c1)) =
                Drawn::for_line(buf, range.clone(), tabstop, Some(window), head_rel);
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
            let x1 = if head_rel >= range.len() { x0 + 8.0 } else { x1 };
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
                            let (drawn, _) =
                                Drawn::for_line(buf, range.clone(), tabstop, Some(window), 0);
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
                                if head_line == ln && focused {
                                    let end = if caret_kind == Caret::Block {
                                        clip(buf.next_char(s.head))
                                    } else {
                                        clip(s.head)
                                    };
                                    carets.push((clip(s.head)..end, caret_kind));
                                }
                                if *s == primary && head_line == ln && focused {
                                    access.0 = Some(clip(s.head) as u32);
                                    if !s.is_empty() && buf.line_of(s.anchor) == ln {
                                        access.1 = Some(clip(s.anchor) as u32);
                                    }
                                }
                            }
                            let hits: Vec<Range<usize>> = buf
                                .runs(SEARCH_LAYER, src.clone())
                                .iter()
                                .map(|r| clip(r.range.start)..clip(r.range.end.min(range.end)))
                                .filter(|r| r.start < r.end)
                                .collect();
                            let styled: Vec<(Range<usize>, kui::Color)> = buf
                                .runs(SYNTAX_LAYER, src.clone())
                                .iter()
                                .filter_map(|r| {
                                    let c = token_colors
                                        .get(r.style as usize)
                                        .copied()
                                        .flatten()?;
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
                            let trailing = diags.first().and_then(|r| {
                                let m = diag_messages?.get(r.tag as usize)?;
                                Some((m.as_str(), diag_colors[(r.style as usize).min(4)]))
                            });
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
        if focused {
            ui.take_key_focus(sink);
        }
    }
}
