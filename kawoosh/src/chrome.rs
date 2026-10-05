//! The window's chrome (roadmap step 13): the title bar and the tab
//! strip, the two rows above the panes.
//!
//! **The title bar** is kawoosh's own (`kui_native::app(…).custom_titlebar()`),
//! drawn in kui's `titlebar_with` so the platform's controls keep their
//! place — the traffic lights inset on macOS, drawn buttons elsewhere —
//! and the whole row drags the window. It carries what the tab strip
//! used to end with: the focused tab's working directory on the left
//! (docs/design/workspaces.md), shortened the
//! way fish's prompt does (every component but the last to its first
//! letter) so a deep one fits, the full path on hover and a click
//! listing it; the language servers, the dock's tasks and a running
//! compile on the right.
//!
//! **The tab strip** gives every tab an even share of the width, down to
//! a floor; a label longer than its share is cut with an ellipsis. Past
//! the floor the row scrolls, and the active tab is revealed on the
//! frame it changes, the offset easing there as the strip's ribbon does
//! (kui F80). The tab under the pointer carries a close button, the
//! active one as any other, when there is another tab to go to. When the tabs are
//! in more than one directory each label leads with its own. A tab is
//! dragged along the row to another place (`Kawoosh::on_tab_drag`), and
//! tabs whose order changed — by that or by `]T` — glide to their places
//! ([`TabsGlide`]).

use kui_native::{Align, CursorShape, Enter, NodeSpec, Role, Span, Ui, Value, widgets};

use crate::app::Kawoosh;
use crate::icons;
use crate::layout::Content;
use crate::rows;

/// The narrowest a tab gets before the strip scrolls rather than
/// squeezing it further.
pub const TAB_MIN_W: f32 = 140.0;
/// How long the strip takes to bring the active tab into view, and a
/// tab to glide to a new place in it.
const TABS_MS: f32 = 160.0;
/// How long past a reorder the tabs are told to glide: the glide and a
/// frame or two over, so its last frame is drawn while they still are.
const GLIDE_FOR: std::time::Duration = std::time::Duration::from_millis(TABS_MS as u64 + 100);

/// The tabs gliding to their places after their order changed: until
/// when, and how far from its new place each moved tab was (by
/// `Tab::id`).
///
/// A reorder is the one thing in the strip that glides, so the gliding
/// is declared for it alone and for as long as it lasts. kui's `slide`
/// eases a node's place in the *window*, whatever moved it: left on, a
/// wheel over a crowded row would drag the tabs behind it, and a tab
/// made or the window resized would send them drifting through each
/// other to widths they already have. So it is on from the reorder
/// until the glide has run, and a tab that was not gliding — kui has
/// no last place for one — is given where it was as the offset it
/// enters from.
pub(crate) struct TabsGlide {
    until: std::time::Instant,
    from: Vec<(u64, f32)>,
}

/// From one tab's left edge to the next's, in a row laid out as `g`
/// with `n` tabs: they are of one width with a hairline between, so the
/// row's content divides evenly.
pub(crate) fn tab_pitch(g: &kui_native::ScrollGeometry, n: usize) -> f32 {
    (g.content.w + 1.0) / n.max(1) as f32
}

impl Kawoosh {
    /// Where tab `t` is: the directory its terminal's shell last said
    /// it is in (OSC 7), when a terminal has its focus, else the tab's
    /// own, else the editor's.
    pub(crate) fn tab_dir(&self, t: &crate::layout::Tab) -> std::path::PathBuf {
        if let Some(Content::Terminal(id)) = self.layout.content(t.focused)
            && let Some(dir) = self.terms.map.get(&id).and_then(|term| term.cwd())
        {
            return dir;
        }
        t.cwd.clone().unwrap_or_else(|| self.cwd.clone())
    }

    /// The title row: the platform's inset and controls around the cwd
    /// and the status blocks.
    pub(crate) fn title_bar(&mut self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let font = self.chrome.face;
        let tab_h = self.chrome.tab_h;
        let focused = ui.env().focused;
        // A host's directory keeps its domain whole: `box:` then the
        // host's path shortened (docs/design/domains.md Decision 8).
        let (head, last) = match kawoosh_systems::fs::domain_of(&self.cwd) {
            Some((d, rest)) => {
                let (h, l) = shorten_path(&rest.display().to_string());
                (format!("{d}:{h}"), l)
            }
            None => shorten_path(&kawoosh_systems::fs::abbreviate_home(&self.cwd)),
        };
        let full = kawoosh_systems::fs::abbreviate_home(&self.cwd);
        // Each block — its parts in their colours — with what a click on
        // it runs, if anything: the plugins' segments first
        // (`kawoosh.status`, status.md), then kawoosh's own.
        let mut blocks: Vec<Block> = self.status_blocks(ui, "title");
        if !self.lsp.status.is_empty() {
            let n: usize = self.lsp.status.iter().map(|s| s.2).sum();
            let servers = self.lsp.status.len();
            blocks.push(Block {
                parts: vec![(
                    format!(
                        "{servers} server{} · {n} docs",
                        if servers == 1 { "" } else { "s" }
                    ),
                    pal.dim,
                )],
                run: Some("lsp info".into()),
            });
        }
        // The dock's tasks, counted: bright while the dock is hidden,
        // where they run unseen; a click shows or hides it.
        let tasks = self.dock_tasks();
        if tasks > 0 {
            blocks.push(Block {
                parts: vec![(
                    format!("{tasks} docked"),
                    if self.layout.dock_open {
                        pal.dim
                    } else {
                        pal.fg
                    },
                )],
                run: Some("dock".into()),
            });
        }
        if self.compile.running {
            blocks.push(Block {
                parts: vec![("compiling…".into(), pal.command)],
                run: None,
            });
        }
        ui.with(
            // Its clicks run a command or list the cwd: the keyboard
            // stays with the pane.
            NodeSpec::column()
                .grow_width()
                .bg(pal.strip)
                .keep_focus()
                .label("titlebar"),
            |ui| {
                widgets::titlebar_with(ui, |ui| {
                    let (dim, fg) = if focused {
                        (pal.dim, pal.fg)
                    } else {
                        (pal.dim, pal.dim)
                    };
                    ui.with_keyed(
                        "cwd",
                        NodeSpec::row()
                            .grow_height()
                            .pad_xy(8.0, 0.0)
                            .cross_align(Align::Center)
                            .hover_bg(pal.panel)
                            .cursor(CursorShape::Pointer)
                            .on_click(Value::map([("kind", "cwd".into())]))
                            .label("cwd")
                            .tooltip(&full),
                        |ui| {
                            let spans = [Span::new(&head).color(dim), Span::new(&last).color(fg)];
                            ui.rich_text(&spans, rows::mono(font, &pal));
                        },
                    );
                    ui.leaf(NodeSpec::row().grow_width());
                    draw_blocks(ui, &blocks, font, &pal, tab_h);
                });
            },
        );
        ui.leaf(NodeSpec::column().grow_width().height(1.0).bg(pal.border));
    }

    /// The tabs, each an even share of the row down to [`TAB_MIN_W`],
    /// the row scrolling past that with the active one revealed.
    pub(crate) fn tab_strip(&mut self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let icon_set = self.icons.clone();
        let icon_set = icon_set.borrow();
        let font = self.chrome.face;
        let theme = ui.theme();
        let n = self.layout.tabs.len();
        let active = self.layout.tab;
        let shape = (active, n);
        // Revealed on the frame the active tab or the count changed:
        // kui lays the reveal out in the same frame, beside the strip's
        // own reveal of its column (kui F82).
        let reveal = self.tabs_seen != Some(shape);
        self.tabs_seen = Some(shape);
        // A tab's directory: a terminal's is where its shell says it
        // is (OSC 7), else the tab's own. Shown before the name as
        // `tabs.directory` says: `auto` while the tabs are in more than
        // one, so the strip says which project each is
        // (docs/design/workspaces.md Decision 6), `always`, `never`.
        let dirs: Vec<std::path::PathBuf> =
            self.layout.tabs.iter().map(|t| self.tab_dir(t)).collect();
        let show_dir = match self.ed.settings.str("tabs.directory") {
            Some("always") => true,
            Some("never") => false,
            _ => dirs.iter().collect::<std::collections::HashSet<_>>().len() > 1,
        };
        let hook = self
            .scripting
            .rt
            .clone()
            .filter(|rt| rt.has_tab_title_hook());
        let labels: Vec<(String, bool, bool, u64)> = self
            .layout
            .tabs
            .iter()
            .zip(&dirs)
            .enumerate()
            .map(|(i, (tab, cwd))| {
                let (kind, name, path) = match self.layout.content(tab.focused) {
                    // A host's file says which host (domains.md
                    // Decision 8): `box: x.rs`.
                    Some(Content::Editor(v)) => {
                        let b = self.ed.buffer_of(v);
                        let name = match b.path.as_deref().and_then(kawoosh_systems::fs::domain_of)
                        {
                            Some((d, _)) => format!("{d}: {}", b.name),
                            None => b.name.clone(),
                        };
                        let path = b.path.as_ref().map(|p| p.display().to_string());
                        ("editor", name, path)
                    }
                    Some(Content::Terminal(t)) => {
                        let title = self
                            .terms
                            .map
                            .get(&t)
                            .filter(|t| !t.title.is_empty())
                            .map(|t| t.title.clone())
                            .unwrap_or_else(|| "term".into());
                        ("terminal", title, None)
                    }
                    Some(Content::Lua(n)) => ("lua", n, None),
                    Some(Content::Undo) => ("undo", "undo".into(), None),
                    Some(Content::Memory) => ("memory", "memory".into(), None),
                    None => ("", "?".into(), None),
                };
                let mut ps = Vec::new();
                tab.panes(&mut ps);
                let modified = ps
                    .iter()
                    .any(|p| matches!(self.view_of(*p), Some(v) if self.ed.buffer_of(v).modified));
                let dir = kawoosh_systems::fs::basename(cwd).unwrap_or_default();
                let shown = if show_dir && !dir.is_empty() {
                    format!("{dir} · {name}")
                } else {
                    name.clone()
                };
                let plain = format!("{}: {shown}", i + 1);
                // The hook is handed the label as text, the mark in it;
                // kawoosh's own draws the mark as the `dot` icon.
                let title = if modified {
                    format!("{plain} ●")
                } else {
                    plain.clone()
                };
                // A plugin's label over it (`kawoosh.tab_title`).
                let label = hook.as_ref().and_then(|rt| {
                    rt.tab_title_hook(&kawoosh_lua::TabTitle {
                        index: i + 1,
                        active: i == active,
                        title: &title,
                        dir: &dir,
                        cwd: &cwd.display().to_string(),
                        kind,
                        name: &name,
                        path: path.as_deref(),
                        modified,
                        bell: tab.bell,
                        panes: ps.len(),
                    })
                });
                let (label, dot) = match label {
                    Some(l) => (l, false),
                    None => (plain, modified),
                };
                (label, dot, tab.bell, tab.id)
            })
            .collect();
        let mut active_key = None;
        let mut tabs_key = None;
        // A tab held and moved follows the pointer (`on_tab_drag`).
        let dragging = self.tab_drag.is_some_and(|(_, moved)| moved);
        // The same tabs in another order than last drawn: each glides
        // from the place it had, which the row's last geometry gives.
        if !self
            .layout
            .tabs
            .iter()
            .map(|t| t.id)
            .eq(self.tabs_order.iter().copied())
        {
            let was = std::mem::replace(
                &mut self.tabs_order,
                self.layout.tabs.iter().map(|t| t.id).collect(),
            );
            let place = |id: &u64| was.iter().position(|w| w == id);
            let pitch = self
                .tabs_key
                .and_then(|k| ui.scroll_geometry(k))
                .map(|g| tab_pitch(&g, n));
            if was.len() == n
                && self.tabs_order.iter().all(|id| place(id).is_some())
                && let Some(pitch) = pitch
            {
                self.tabs_glide = Some(TabsGlide {
                    until: std::time::Instant::now() + GLIDE_FOR,
                    from: self
                        .tabs_order
                        .iter()
                        .enumerate()
                        .filter_map(|(now, id)| {
                            let was = place(id)?;
                            (was != now).then_some((*id, (was as f32 - now as f32) * pitch))
                        })
                        .collect(),
                });
            } else {
                // A tab made or closed: the widths changed with it, and
                // the tabs take their new places whole.
                self.tabs_glide = None;
            }
        }
        if self
            .tabs_glide
            .as_ref()
            .is_some_and(|g| g.until <= std::time::Instant::now())
        {
            self.tabs_glide = None;
        }
        let glide = self.tabs_glide.as_ref();
        // The plugins' segments at the strip's right end (status.md):
        // the tabs in a row beside them when there are any.
        let segs = self.status_blocks(ui, "tabs");
        let tab_h = self.chrome.tab_h;
        let mut tabs = |ui: &mut Ui<'_>| {
            tabs_key = Some(
                ui.with_keyed(
                    "tabs",
                    NodeSpec::row()
                        .grow_width()
                        .height(self.chrome.tab_h)
                        .bg(pal.strip)
                        .scroll_x()
                        // No bar: at the strip's height it would lie over the
                        // labels and take their clicks; the wheel and the
                        // reveal move it.
                        .scrollbar(kui_native::ScrollbarMode::Hidden)
                        .transition(TABS_MS)
                        .keep_focus()
                        .role(Role::TabList),
                    |ui| {
                        for (i, (label, dot, bell, id)) in labels.iter().enumerate() {
                            let is_active = i == active;
                            // The block, its item and its close button are one
                            // hover group: the pointer is on the item or the
                            // button, never on the block itself, and the button
                            // must stay while the pointer goes to it. Named by
                            // the tab's own number, as its key is: kui keeps
                            // the pressed node lit while a drag holds it, and
                            // named by its place the light stayed where the
                            // press was as the tab went along the row.
                            let group = format!("tab-hover{id}");
                            let hovered = ui.is_group_hovered(NodeSpec::hover_group_id(&group));
                            // A terminal in it rang unseen (`terminal.bell`): i3's
                            // urgent workspace, the edge and the label in the
                            // warning's colour until the tab is visited.
                            let (bg, fg, edge) = if is_active {
                                (theme.accent, theme.on_accent, theme.accent_hover)
                            } else if *bell {
                                (pal.strip, theme.warning, theme.warning)
                            } else {
                                (pal.strip, pal.dim, pal.border)
                            };
                            // The block holds the tab item and its close button
                            // side by side: a button inside the item would sit
                            // inside one roving Tab stop, where the ring never
                            // reaches it.
                            let mut block = NodeSpec::column()
                                .grow_width()
                                .min_width(TAB_MIN_W)
                                .grow_height()
                                .bg(bg)
                                .hoverable()
                                .hover_group(&group);
                            // Under a drag the held tab alone answers the
                            // pointer: the others pass beneath it as they
                            // make room, and would light and show their
                            // close buttons on the way.
                            if !dragging || is_active {
                                block = block.hover_bg(if is_active {
                                    theme.accent_hover
                                } else {
                                    pal.panel
                                });
                            }
                            if let Some(g) = glide {
                                block = block.transition(TABS_MS).slide();
                                if let Some((_, dx)) = g.from.iter().find(|(t, _)| t == id) {
                                    block = block.enter(Enter::from(*dx, 0.0));
                                }
                            }
                            let key = ui.with_keyed(&format!("tab{id}"), block, |ui| {
                                // i3's coloured top edge on the block.
                                ui.leaf(NodeSpec::row().grow_width().height(2.0).bg(edge));
                                ui.with(
                                    NodeSpec::row().fill().gap(0.0).cross_align(Align::Center),
                                    |ui| {
                                        // A click goes to the tab, a
                                        // drag takes it along the row.
                                        let mut item = NodeSpec::row()
                                            .fill()
                                            .pad_xy(10.0, 0.0)
                                            .cross_align(Align::Center)
                                            .clip()
                                            .hover_group(&group)
                                            .on_click(Value::map([
                                                ("kind", "tab".into()),
                                                ("index", Value::Int(i as i64)),
                                            ]))
                                            .on_drag(Value::map([
                                                ("kind", "tabdrag".into()),
                                                ("index", Value::Int(i as i64)),
                                            ]))
                                            .role(Role::Tab)
                                            .selected(is_active)
                                            .label(label.as_str());
                                        if dragging {
                                            item = item.cursor(CursorShape::Grabbing);
                                        }
                                        ui.with_keyed("item", item.gap(4.0), |ui| {
                                            ui.text(
                                                label,
                                                rows::mono(font, &pal).color(fg).ellipsis(),
                                            );
                                            if *dot {
                                                icons::icon(ui, &icon_set, "dot", font.size, fg);
                                            }
                                        });
                                        if n > 1 && hovered && !dragging {
                                            // Its own colour under the pointer
                                            // alone: a group's hover lights
                                            // every member.
                                            let on = ui.is_hovered(ui.child_key("close"));
                                            // A square shorter than the row,
                                            // centred in it, its × the `close`
                                            // icon about the middle: a glyph
                                            // sits on the font's math axis,
                                            // below it.
                                            let side = (font.line_height * 0.75).round();
                                            let mut close = icons::icon_box(side)
                                                .radius(3.0)
                                                .hover_group(&group);
                                            if on {
                                                close = close.bg(if is_active {
                                                    theme.accent_hover
                                                } else {
                                                    pal.border
                                                });
                                            }
                                            ui.with_keyed(
                                                "close",
                                                close
                                                    .on_click(Value::map([
                                                        ("kind", "tab close".into()),
                                                        ("index", Value::Int(i as i64)),
                                                    ]))
                                                    .label("close tab"),
                                                |ui| {
                                                    icons::icon(
                                                        ui,
                                                        &icon_set,
                                                        "close",
                                                        font.size.round(),
                                                        fg,
                                                    );
                                                },
                                            );
                                            ui.leaf(NodeSpec::row().width(6.0));
                                        }
                                    },
                                );
                            });
                            if is_active {
                                active_key = Some(key);
                            }
                            // A hairline between blocks, as i3 draws.
                            if i + 1 < n {
                                let sep = ui.child_key("sep").index(i as u64);
                                ui.leaf_key(
                                    sep,
                                    NodeSpec::column().width(1.0).grow_height().bg(pal.border),
                                );
                            }
                        }
                    },
                ),
            );
        };
        if segs.is_empty() {
            tabs(ui);
        } else {
            ui.with(
                NodeSpec::row()
                    .grow_width()
                    .height(tab_h)
                    .bg(pal.strip)
                    .keep_focus(),
                |ui| {
                    tabs(ui);
                    ui.leaf(NodeSpec::column().width(1.0).grow_height().bg(pal.border));
                    draw_blocks(ui, &segs, font, &pal, tab_h);
                },
            );
        }
        self.tabs_key = tabs_key;
        if reveal && let Some(key) = active_key {
            ui.reveal(key);
        }
    }
}

/// A block on the chrome's right: its parts in their colours, and the
/// command line a click on it runs.
pub(crate) struct Block {
    pub parts: Vec<(String, kui_native::Color)>,
    pub run: Option<String>,
}

/// Blocks side by side, a hairline between them, each running its
/// command on a click.
fn draw_blocks(
    ui: &mut Ui<'_>,
    blocks: &[Block],
    font: crate::look::Face,
    pal: &crate::palette::Pal,
    tab_h: f32,
) {
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            let sep = ui.child_key("sep").index(i as u64);
            ui.leaf_key(
                sep,
                NodeSpec::column().size(1.0, tab_h - 10.0).bg(pal.border),
            );
        }
        let mut spec = NodeSpec::row()
            .grow_height()
            .pad_xy(10.0, 0.0)
            .cross_align(Align::Center);
        if let Some(run) = &b.run {
            spec = spec
                .hover_bg(pal.panel)
                .cursor(CursorShape::Pointer)
                .on_click(Value::map([
                    ("kind", "chrome".into()),
                    ("run", Value::str(run.as_str())),
                ]))
                .label(run.as_str());
        }
        ui.with_keyed(&format!("block{i}"), spec, |ui| {
            let spans: Vec<Span<'_>> = b
                .parts
                .iter()
                .map(|(t, c)| Span::new(t).color(*c))
                .collect();
            ui.rich_text(&spans, rows::mono(font, pal));
        });
    }
}

impl Kawoosh {
    /// A wake asked for the next beat of the shortest `every` a status
    /// segment has, on the wall clock — a minute's clock turns over on
    /// the minute — unless one is out already by then.
    pub(crate) fn sync_status_tick(&mut self) {
        let Some(every) = self.scripting.rt.as_ref().and_then(|rt| rt.status_every()) else {
            return;
        };
        let now = std::time::SystemTime::now();
        let secs = now
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        let next = ((secs / every).floor() + 1.0) * every;
        let due = std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(next);
        if self.status_due.is_some_and(|d| d <= due && d > now) {
            return;
        }
        self.status_due = Some(due);
        let wait = due.duration_since(now).unwrap_or_default();
        self.io.run("status tick", move || {
            std::thread::sleep(wait);
            kawoosh_systems::io::IoMsg::Tick
        });
    }

    /// The segments `kawoosh.status` put at `place`, as blocks: each
    /// part's colour word the theme's.
    fn status_blocks(&self, ui: &mut Ui<'_>, place: &str) -> Vec<Block> {
        self.status_segments(ui, place)
            .into_iter()
            .map(|(_, b)| b)
            .collect()
    }

    /// The same, each by its name.
    pub(crate) fn status_segments(&self, ui: &mut Ui<'_>, place: &str) -> Vec<(String, Block)> {
        let Some(rt) = &self.scripting.rt else {
            return Vec::new();
        };
        let theme = ui.theme();
        let pal = self.pal;
        rt.status(place)
            .into_iter()
            .map(|s| {
                let parts = s
                    .parts
                    .into_iter()
                    .map(|(t, c)| {
                        let color = match c.as_str() {
                            "dim" => pal.dim,
                            "accent" => pal.accent,
                            "ok" => theme.success,
                            "warning" => theme.warning,
                            "danger" => pal.danger,
                            _ => pal.fg,
                        };
                        (t, color)
                    })
                    .collect();
                (s.name, Block { parts, run: s.run })
            })
            .collect()
    }
}

/// A directory's name cut as fish's prompt cuts it: to its first
/// character, a leading dot kept with the one after it (`.claude` is
/// `.c`).
pub fn cut_component(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some('.') => chars.next().map_or(".".into(), |c| format!(".{c}")),
        Some(c) => c.to_string(),
        None => String::new(),
    }
}

/// A path shortened as fish's prompt shortens it: every component but
/// the last cut to its first character (a leading dot kept with it),
/// split as the part to dim and the last component. `~/projects/kawoosh`
/// is `("~/p/", "kawoosh")`. The separators are the platform's — on
/// Windows `\` and a host's `/` both — each kept as written, and a
/// drive (`C:`) is kept whole.
pub fn shorten_path(path: &str) -> (String, String) {
    use std::path::is_separator;
    let Some(at) = path.rfind(is_separator) else {
        return (String::new(), path.to_string());
    };
    let (head, last) = (&path[..=at], &path[at + 1..]);
    let mut out = String::new();
    // Each part ends in its separator.
    for (i, part) in head.split_inclusive(is_separator).enumerate() {
        let mut chars = part.chars();
        let sep = chars.next_back();
        let name = chars.as_str();
        if i == 0 && name.len() > 1 && name.ends_with(':') {
            out.push_str(name);
        } else {
            out.push_str(&cut_component(name));
        }
        out.extend(sep);
    }
    (out, last.to_string())
}

#[cfg(test)]
mod tests {
    use super::shorten_path;

    #[test]
    fn a_path_is_shortened_as_fish_does() {
        let s = |p: &str| {
            let (a, b) = shorten_path(p);
            a + &b
        };
        assert_eq!(s("~/projects/kawoosh"), "~/p/kawoosh");
        assert_eq!(
            s("~/projects/kawoosh/.claude/worktrees/launcher-pane"),
            "~/p/k/.c/w/launcher-pane"
        );
        assert_eq!(s("/usr/local/bin"), "/u/l/bin");
        assert_eq!(s("/"), "/");
        assert_eq!(s("~"), "~");
        assert_eq!(shorten_path("~/projects/kawoosh").1, "kawoosh");
        if cfg!(windows) {
            assert_eq!(s(r"~\projects\kawoosh"), r"~\p\kawoosh");
            assert_eq!(s(r"C:\Users\me\src"), r"C:\U\m\src");
            assert_eq!(s(r"C:\"), r"C:\");
        }
    }
}
