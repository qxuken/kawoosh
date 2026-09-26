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
//! listing it; the language servers and a running compile on the right.
//!
//! **The tab strip** gives every tab an even share of the width, down to
//! a floor; a label longer than its share is cut with an ellipsis. Past
//! the floor the row scrolls, and the active tab is revealed on the
//! frame it changes, the offset easing there as the strip's ribbon does
//! (kui F80). The active tab and the one under the pointer carry a
//! close button, when there is another tab to go to. When the tabs are
//! in more than one directory each label leads with its own.

use kui_native::{Align, CursorShape, NodeSpec, Role, Sizing, Span, Ui, Value, widgets};

use crate::app::Kawoosh;
use crate::layout::Content;
use crate::rows;

/// The narrowest a tab gets before the strip scrolls rather than
/// squeezing it further.
pub const TAB_MIN_W: f32 = 140.0;
/// How long the strip takes to bring the active tab into view.
const TABS_MS: f32 = 160.0;

impl Kawoosh {
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
        // Each block with what a click on it runs, if anything.
        let mut blocks: Vec<(String, kui_native::Color, Option<&str>)> = Vec::new();
        if !self.lsp.status.is_empty() {
            let n: usize = self.lsp.status.iter().map(|s| s.2).sum();
            let servers = self.lsp.status.len();
            blocks.push((
                format!(
                    "{servers} server{} · {n} docs",
                    if servers == 1 { "" } else { "s" }
                ),
                pal.dim,
                Some("lsp info"),
            ));
        }
        if self.compile.running {
            blocks.push(("compiling…".into(), pal.command, None));
        }
        ui.with(
            NodeSpec::column()
                .width(Sizing::Grow(1.0))
                .bg(pal.strip)
                .label("titlebar"),
            |ui| {
                widgets::titlebar_with(ui, |ui| {
                    let cwd = ui.child_key("cwd");
                    let hovered = ui.is_hovered(cwd);
                    let (dim, fg) = if focused {
                        (pal.dim, pal.fg)
                    } else {
                        (pal.dim, pal.dim)
                    };
                    ui.with_keyed(
                        "cwd",
                        NodeSpec::row()
                            .height(Sizing::Grow(1.0))
                            .pad_xy(8.0, 0.0)
                            .cross_align(Align::Center)
                            .hover_bg(pal.panel)
                            .cursor(CursorShape::Pointer)
                            .on_click(Value::map([("kind", "cwd".into())]))
                            .label("cwd")
                            .description(full.as_str()),
                        |ui| {
                            let spans = [Span::new(&head).color(dim), Span::new(&last).color(fg)];
                            ui.rich_text(&spans, rows::mono(font, &pal));
                            if hovered {
                                widgets::tooltip(ui, &full);
                            }
                        },
                    );
                    ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                    for (i, (text, color, run)) in blocks.iter().enumerate() {
                        if i > 0 {
                            ui.with_indexed(
                                2000 + i as u64,
                                NodeSpec::column()
                                    .width(Sizing::Fixed(1.0))
                                    .height(Sizing::Fixed(tab_h - 10.0))
                                    .bg(pal.border),
                                |_| {},
                            );
                        }
                        let mut spec = NodeSpec::row()
                            .height(Sizing::Grow(1.0))
                            .pad_xy(10.0, 0.0)
                            .cross_align(Align::Center);
                        if let Some(run) = run {
                            spec = spec
                                .hover_bg(pal.panel)
                                .cursor(CursorShape::Pointer)
                                .on_click(Value::map([
                                    ("kind", "chrome".into()),
                                    ("run", Value::str(*run)),
                                ]))
                                .label(*run);
                        }
                        ui.with_keyed(&format!("block{i}"), spec, |ui| {
                            ui.text(text, rows::mono(font, &pal).color(*color))
                        });
                    }
                });
            },
        );
        ui.with(
            NodeSpec::column()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Fixed(1.0))
                .bg(pal.border),
            |_| {},
        );
    }

    /// The tabs, each an even share of the row down to [`TAB_MIN_W`],
    /// the row scrolling past that with the active one revealed.
    pub(crate) fn tab_strip(&mut self, ui: &mut Ui<'_>) {
        let pal = self.pal;
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
        // Tabs in more than one directory lead with theirs, so the strip
        // says which project each is (docs/design/workspaces.md
        // Decision 6).
        let dirs: std::collections::HashSet<_> = self
            .layout
            .tabs
            .iter()
            .map(|t| t.cwd.as_ref().unwrap_or(&self.cwd))
            .collect();
        let show_dir = dirs.len() > 1;
        let labels: Vec<(String, bool, bool)> = self
            .layout
            .tabs
            .iter()
            .map(|tab| {
                let name = match self.layout.content(tab.focused) {
                    // A host's file says which host (domains.md
                    // Decision 8): `box: x.rs`.
                    Some(Content::Editor(v)) => {
                        let b = self.ed.buffer_of(v);
                        match b.path.as_deref().and_then(kawoosh_systems::fs::domain_of) {
                            Some((d, _)) => format!("{d}: {}", b.name),
                            None => b.name.clone(),
                        }
                    }
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
                let modified = ps
                    .iter()
                    .any(|p| matches!(self.view_of(*p), Some(v) if self.ed.buffer_of(v).modified));
                let name = match tab.cwd.as_ref().unwrap_or(&self.cwd).file_name() {
                    Some(d) if show_dir => format!("{} · {name}", d.to_string_lossy()),
                    _ => name,
                };
                (name, modified, tab.bell)
            })
            .collect();
        let mut active_key = None;
        ui.with_keyed(
            "tabs",
            NodeSpec::row()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Fixed(self.chrome.tab_h))
                .bg(pal.strip)
                .scroll_x()
                // No bar: at the strip's height it would lie over the
                // labels and take their clicks; the wheel and the
                // reveal move it.
                .scrollbar(kui_native::ScrollbarMode::Hidden)
                .transition(TABS_MS)
                .role(Role::TabList),
            |ui| {
                for (i, (name, modified, bell)) in labels.iter().enumerate() {
                    let is_active = i == active;
                    let label = format!("{}: {}{}", i + 1, name, if *modified { " ●" } else { "" });
                    // The block, its item and its close button are one
                    // hover group: the pointer is on the item or the
                    // button, never on the block itself, and the button
                    // must stay while the pointer goes to it.
                    let group = format!("tab-hover{i}");
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
                    let key = ui.with_keyed(
                        &format!("tab{i}"),
                        NodeSpec::column()
                            .width(Sizing::Grow(1.0))
                            .min_width(TAB_MIN_W)
                            .height(Sizing::Grow(1.0))
                            .bg(bg)
                            .hover_bg(if is_active {
                                theme.accent_hover
                            } else {
                                pal.panel
                            })
                            .hoverable()
                            .hover_group(&group),
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
                                    .width(Sizing::Grow(1.0))
                                    .height(Sizing::Grow(1.0))
                                    .gap(0.0)
                                    .cross_align(Align::Center),
                                |ui| {
                                    ui.with_keyed(
                                        "item",
                                        NodeSpec::row()
                                            .width(Sizing::Grow(1.0))
                                            .height(Sizing::Grow(1.0))
                                            .pad_xy(10.0, 0.0)
                                            .cross_align(Align::Center)
                                            .clip()
                                            .hover_group(&group)
                                            .on_click(Value::map([
                                                ("kind", "tab".into()),
                                                ("index", Value::Int(i as i64)),
                                            ]))
                                            .role(Role::Tab)
                                            .selected(is_active)
                                            .label(label.as_str()),
                                        |ui| {
                                            ui.text(
                                                &label,
                                                rows::mono(font, &pal).color(fg).ellipsis(),
                                            )
                                        },
                                    );
                                    if n > 1 && (is_active || hovered) {
                                        // Its own colour under the pointer
                                        // alone: a group's hover lights
                                        // every member.
                                        let on = ui.is_hovered(ui.child_key("close"));
                                        let mut close = NodeSpec::row()
                                            .pad_xy(4.0, 0.0)
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
                                            |ui| ui.text("×", rows::mono(font, &pal).color(fg)),
                                        );
                                        ui.with(NodeSpec::row().width(Sizing::Fixed(6.0)), |_| {});
                                    }
                                },
                            );
                        },
                    );
                    if is_active {
                        active_key = Some(key);
                    }
                    // A hairline between blocks, as i3 draws.
                    if i + 1 < n {
                        ui.with_indexed(
                            1000 + i as u64,
                            NodeSpec::column()
                                .width(Sizing::Fixed(1.0))
                                .height(Sizing::Grow(1.0))
                                .bg(pal.border),
                            |_| {},
                        );
                    }
                }
            },
        );
        if reveal && let Some(key) = active_key {
            ui.reveal(key);
        }
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
        match chars.next() {
            _ if i == 0 && name.len() > 1 && name.ends_with(':') => out.push_str(name),
            Some('.') => {
                out.push('.');
                out.extend(chars.next());
            }
            Some(c) => out.push(c),
            None => {}
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
