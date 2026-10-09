//! Menus (docs/design/menus.md): a right-click's context menu over a
//! pane, and on macOS the application menu bar at the top of the
//! screen. Both are kui's (ADR 0017's context menu, ADR 0018's bar):
//! rows as data, drawn by kui where the platform draws none and handed
//! to `NSMenu` where it does, a chosen row coming back as one `menu`
//! event. What is kawoosh's is what the rows *do* — each runs a command
//! line on its pane, as the `:` prompt would (the thesis), so a menu is
//! another way to the commands and never a second implementation of
//! one.
//!
//! The keymap stays the keyboard's one source (Decision 4): a row shows
//! the keys its command is bound to as a hint, and a bar row binds a
//! chord only where no binding in any mode takes it — AppKit consumes a
//! bound chord before the window sees it.

use kawoosh_editor::{Mode, Selection, Selections, ViewId};
use kawoosh_systems::lsp::Caps;
use kui_native::{
    Accel, BarMenu, ButtonEvent, ButtonPhase, Core, Key, Menu, MenuBar, MenuItem, MenuRole,
    MouseButton, Ui, Value, Vec2, WindowCommand,
};

use crate::app::Kawoosh;
use crate::layout::{Content, PaneId};

/// Which of the bar's chords no binding takes (`Kawoosh::bar_chords`):
/// one bit each, by [`BAR_CHORDS`]'s order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FreeChords(u8);

impl FreeChords {
    fn has(self, i: usize) -> bool {
        self.0 & (1 << i) != 0
    }
}

/// The chords the bar may bind, in the keymap's notation and the bar's,
/// with the command line of the row that takes them: Quit, Settings,
/// Minimize — what a Mac app has under them. A chord bound only to its
/// row's own command (`<D-,>`, the settings pane's) is still the row's:
/// AppKit taking it first runs what the key would.
const BAR_CHORDS: [(&str, &str, &str); 3] = [
    ("<D-q>", "mod+q", "quit all"),
    ("<D-,>", "mod+,", "settings"),
    ("<D-m>", "mod+m", ""),
];
const QUIT: usize = 0;
const SETTINGS: usize = 1;
const MINIMIZE: usize = 2;

/// A row's `id`: what it does, and the pane it does it on (focused
/// first) when it is about one. Handed back as the `menu` event's
/// `item`, so the row needs no lookup afterwards.
fn id(pane: Option<PaneId>, what: &'static str, arg: &str) -> Value {
    let mut entries = vec![(what, Value::str(arg))];
    if let Some(p) = pane {
        entries.push(("pane", Value::Int(p as i64)));
    }
    Value::map(entries)
}

/// A row running command line `line`.
fn run(label: &str, pane: Option<PaneId>, line: &str) -> MenuItem {
    MenuItem::new(label).id(id(pane, "run", line))
}

/// A row doing one of the edits a menu offers on its pane
/// (`Kawoosh::menu_edit`): `copy`, `cut`, `paste`, `select all`,
/// `undo`, `redo`.
fn edit(label: &str, pane: Option<PaneId>, what: &str) -> MenuItem {
    MenuItem::new(label).id(id(pane, "edit", what))
}

impl Kawoosh {
    /// The secondary button pressed in an editor pane's rows (the
    /// `editor` node claims it, `on_button`, so the press comes with
    /// the row and byte it landed on, as a left press does). The caret
    /// goes there unless the press is inside a selection, which a menu
    /// is usually for; then the editor's menu opens at the pointer.
    pub(crate) fn on_edit_button(
        &mut self,
        pane: PaneId,
        b: ButtonEvent,
        key: Key,
        core: &mut Core,
    ) {
        if b.button != MouseButton::Secondary || b.phase != ButtonPhase::Press {
            return;
        }
        self.layout.focus(pane);
        self.header_blur(pane);
        if self.ed.prompt_view().is_some() {
            self.ed.cancel_prompt();
        }
        let Some(view) = self.view_of(pane) else {
            return;
        };
        if let (Some(line), Some(byte)) = (b.line, b.byte) {
            let (off, _) = self.offset_at(pane, view, line as usize, byte);
            let inside = self.has_selection(view)
                && self
                    .ed
                    .selection_ranges(view)
                    .iter()
                    .any(|r| r.start < r.end && r.contains(&off));
            if !inside {
                if self.ed.mode(view) == Mode::Visual {
                    self.ed.set_mode(view, Mode::Normal);
                }
                let v = &mut self.ed.views[view];
                v.sels = Selections::single(Selection::point(off));
                v.goal_col = None;
            }
            self.follow_caret = true;
        }
        let items = self.editor_menu(pane, view);
        core.open_menu(Menu::new(key, b.pos, items));
    }

    /// A `contextmenu` event: a pane's title bar, or a terminal's grid.
    /// The rest of a pane declares none, so kui's own menu answers over
    /// a Lua view's fields and selectable text (Decision 1).
    pub(crate) fn on_context_menu(&mut self, ev_key: Key, p: &Value, core: &mut Core) {
        let tag = p.get("tag");
        let Some(pane) = tag.and_then(|t| t.get_int("pane")).map(|n| n as PaneId) else {
            return;
        };
        if self.layout.content(pane).is_none() {
            return;
        }
        let num = |name: &str| p.get(name).and_then(Value::as_float).unwrap_or(0.0) as f32;
        let at = Vec2::new(num("x"), num("y"));
        let title = tag.and_then(|t| t.get_bool("title")).unwrap_or(false);
        let items = match self.layout.content(pane) {
            Some(Content::Terminal(_)) if !title => self.terminal_menu(pane, core),
            _ => self.pane_menu(pane),
        };
        core.open_menu(Menu::new(ev_key, at, items));
    }

    /// A chosen row of any of kawoosh's menus, the bar's or a pane's:
    /// its pane focused, then its command line run or its edit done. A
    /// standard row (`role`) kui has already performed, and carries no
    /// `id` of kawoosh's.
    pub(crate) fn on_menu(&mut self, item: Option<&Value>, core: &mut Core) {
        let Some(item) = item.filter(|i| matches!(i, Value::Map(_))) else {
            return;
        };
        if let Some(pane) = item.get_int("pane").map(|n| n as PaneId) {
            // Closed while its menu was open.
            if self.layout.content(pane).is_none() {
                return;
            }
            self.layout.focus(pane);
        }
        if let Some(line) = item.get_str("run") {
            let line = line.to_string();
            self.run_line(&line);
            self.follow_caret = true;
        } else if let Some(what) = item.get_str("edit") {
            let what = what.to_string();
            self.menu_edit(&what);
        } else if item.get_str("window") == Some("minimize") {
            // The bar's events carry no window of their own: this core's.
            core.push_window_command(WindowCommand::Minimize(core.env.window.id));
        }
    }

    /// An edit chosen from a menu, on the focused pane: in an editor
    /// the commands its keys run — Copy and Cut are visual mode's `y`
    /// and `d` over the selection, and a selection made outside visual
    /// mode is taken into it first; in a terminal, Paste as ⌘V pastes.
    pub fn menu_edit(&mut self, what: &str) {
        let pane = self.layout.focused();
        let Some(view) = self.view_of(pane) else {
            if what == "paste" && self.term_of(pane).is_some() {
                self.awaiting_paste = true;
            }
            return;
        };
        let command = match what {
            "copy" | "cut" => {
                if !self.has_selection(view) {
                    return;
                }
                if self.ed.mode(view) != Mode::Visual {
                    self.ed.set_mode(view, Mode::Visual);
                }
                if what == "copy" { "yank" } else { "delete" }
            }
            "paste" => "paste clipboard",
            "select all" => "select all",
            "undo" => "undo",
            "redo" => "redo",
            _ => return,
        };
        self.shell_command(command, &[], None);
        self.follow_caret = true;
    }

    /// Whether view `view` has something to copy: visual mode, or a
    /// selection wider than a caret (a drag's, before its mode caught
    /// up).
    fn has_selection(&self, view: ViewId) -> bool {
        self.ed.mode(view) == Mode::Visual || self.ed.views[view].sels.iter().any(|s| !s.is_empty())
    }

    /// Whether `command` can run on view `view` now — `format` where a
    /// formatter is — so its row is lit or dimmed, never missing.
    fn can_run(&self, view: ViewId, command: &str) -> bool {
        self.ed.can(Some(view), command).is_ok()
    }

    /// Whether a language server's row is lit on `view`: a server
    /// answers for its buffer and does what the row asks. Asked of the
    /// shell, not of the commands' `when` — the `lsp` fact is the
    /// focused pane's, and a command run without a server says why.
    fn lsp_lit(&self, view: ViewId, does: impl Fn(&Caps) -> bool) -> bool {
        let buffer = self.ed.views[view].buffer;
        self.lsp_answers(buffer) && does(&self.caps_of(buffer))
    }

    /// The keys normal mode runs `command` with, the shortest of them,
    /// as a hint on its row: `gd` beside Go to Definition. Only a hint
    /// no platform could take for a shortcut of its own (a menu binds a
    /// chord with a modifier): `<D-s>` is not drawn, the keymap's
    /// spelling being no platform's.
    fn keys_for(&self, command: &str) -> Option<String> {
        self.ed
            .keymap
            .binding_strokes(Mode::Normal)
            .into_iter()
            .filter(|(_, b)| b.scope.is_none() && b.line() == command)
            .map(|(keys, _)| keys.concat())
            .filter(|k| !k.contains('<') && Accel::parse(k).is_none_or(|a| !a.mods.any()))
            .min_by_key(|k| (k.chars().count(), k.clone()))
    }

    /// A command row with its keys as the hint.
    fn run_hinted(&self, label: &str, pane: PaneId, line: &str) -> MenuItem {
        let row = run(label, Some(pane), line);
        match self.keys_for(line) {
            Some(keys) => row.accel(keys),
            None => row,
        }
    }

    /// What every pane's menu ends with: the pane split, and closed.
    fn pane_rows(&self, pane: PaneId, items: &mut Vec<MenuItem>) {
        items.push(run("Split Right", Some(pane), "vsplit"));
        items.push(run("Split Down", Some(pane), "split"));
        items.push(MenuItem::separator());
        items.push(
            run("Close Other Panes", Some(pane), "only")
                .enabled(self.layout.visible_panes().len() > 1),
        );
        items.push(run("Close Pane", Some(pane), "close"));
    }

    /// An editor pane's menu: the language server's moves, the edits,
    /// the pane's own rows. Every row present whatever is possible, its
    /// enabling what moves (kui's rule for menus, ADR 0017).
    pub(crate) fn editor_menu(&self, pane: PaneId, view: ViewId) -> Vec<MenuItem> {
        let lsp = |label: &str, command: &str, does: fn(&Caps) -> bool| {
            self.run_hinted(label, pane, command)
                .enabled(self.lsp_lit(view, does))
        };
        let selected = self.has_selection(view);
        let mut items = vec![
            lsp("Go to Definition", "lsp definition", |c| c.definition),
            lsp("Go to References", "lsp references", |c| c.references),
            lsp("Rename Symbol…", "lsp rename", |c| c.rename),
            lsp("Code Actions…", "lsp action", |c| c.code_action),
            self.run_hinted("Format", pane, "format")
                .enabled(self.can_run(view, "format")),
            MenuItem::separator(),
            edit("Cut", Some(pane), "cut").enabled(selected),
            edit("Copy", Some(pane), "copy").enabled(selected),
            edit("Paste", Some(pane), "paste"),
            edit("Select All", Some(pane), "select all"),
            MenuItem::separator(),
        ];
        self.pane_rows(pane, &mut items);
        items
    }

    /// A terminal's menu: its grid's selection is kui's (a `selectable`
    /// grid), so Copy and Select All are kui's standard rows, performed
    /// by kui; Paste is ⌘V's.
    fn terminal_menu(&self, pane: PaneId, core: &Core) -> Vec<MenuItem> {
        let selected = core.copy_selection().is_some_and(|t| !t.is_empty());
        let mut items = vec![
            MenuItem::role(MenuRole::Copy).enabled(selected),
            edit("Paste", Some(pane), "paste"),
            MenuItem::role(MenuRole::SelectAll),
            MenuItem::separator(),
        ];
        self.pane_rows(pane, &mut items);
        items
    }

    /// The menu of every pane's title bar: an editor's path, and the
    /// pane's own rows.
    fn pane_menu(&self, pane: PaneId) -> Vec<MenuItem> {
        let mut items = Vec::new();
        if let Some(view) = self.view_of(pane) {
            let has_path = self.ed.buffer_of(view).path.is_some();
            items.push(run("Copy Path", Some(pane), "path copy").enabled(has_path));
            items.push(MenuItem::separator());
        }
        self.pane_rows(pane, &mut items);
        items
    }

    /// The application menu bar, on a platform that owns one (macOS):
    /// declared every frame and diffed by kui, so an unchanged bar
    /// rebuilds no `NSMenu`. Nothing is declared elsewhere — kawoosh
    /// draws its own chrome, and a menu strip in the window would be a
    /// second title bar — so on Windows and Linux the context menus and
    /// the keys are the way (Decision 5).
    pub(crate) fn sync_menu_bar(&mut self, ui: &mut Ui<'_>) {
        if !ui.core().native_menu_bar() {
            return;
        }
        let bar = self.menu_bar();
        ui.core().declare_menu_bar(bar);
    }

    /// Which of [`BAR_CHORDS`] no binding takes, in any mode, but one
    /// running the row's own command: read once a keymap version.
    fn free_chords(&mut self) -> FreeChords {
        let version = self.ed.keymap.version();
        if let Some((v, free)) = self.bar_chords
            && v == version
        {
            return free;
        }
        let mut taken = [false; BAR_CHORDS.len()];
        for mode in [
            Mode::Normal,
            Mode::Insert,
            Mode::Visual,
            Mode::OperatorPending,
            Mode::Pane,
        ] {
            for (keys, b) in self.ed.keymap.binding_strokes(mode) {
                for (i, (chord, _, line)) in BAR_CHORDS.iter().enumerate() {
                    let rows = keys.len() == 1
                        && b.scope.is_none()
                        && b.when.is_empty()
                        && b.line() == *line;
                    if keys.first().is_some_and(|k| k == chord) && !rows {
                        taken[i] = true;
                    }
                }
            }
        }
        let free = FreeChords(
            taken
                .iter()
                .enumerate()
                .filter(|(_, t)| !**t)
                .fold(0, |bits, (i, _)| bits | (1 << i)),
        );
        self.bar_chords = Some((version, free));
        free
    }

    /// The bar's menus. The first is the application menu, which macOS
    /// titles with the app's name; one named `Window` is the platform's
    /// Window menu, with its tiling and full screen (kui ADR 0030); the
    /// `Edit` rows bind no chord, so ⌘C and the rest reach the keymap as
    /// they do without a bar (Decision 4).
    pub(crate) fn menu_bar(&mut self) -> MenuBar {
        let free = self.free_chords();
        let chord = |row: MenuItem, i: usize| {
            if free.has(i) {
                row.accel(BAR_CHORDS[i].1)
            } else {
                row
            }
        };
        let pane = self.layout.focused();
        let view = self.view_of(pane);
        let term = self.term_of(pane).is_some();
        let on_view = |command: &str| view.is_some_and(|v| self.can_run(v, command));
        let lsp_lit = |does: fn(&Caps) -> bool| view.is_some_and(|v| self.lsp_lit(v, does));
        let editing = view.is_some();
        let app = vec![
            run("Keys", None, "keys"),
            run("Commands…", None, "commands"),
            MenuItem::separator(),
            chord(run("Settings…", None, BAR_CHORDS[SETTINGS].2), SETTINGS),
            run("Project Settings", None, "settings project"),
            MenuItem::separator(),
            chord(run("Quit kawoosh", None, BAR_CHORDS[QUIT].2), QUIT),
        ];
        let file = vec![
            run("New Tab", None, "tab new"),
            run("New Terminal", None, "terminal"),
            run("New Scratch", None, "enew"),
            MenuItem::separator(),
            run("Open File…", None, "picker files"),
            run("Open Recent…", None, "picker recent"),
            MenuItem::separator(),
            run("Save", None, "write").enabled(editing),
            run("Save All", None, "write all"),
            MenuItem::separator(),
            run("Close Pane", None, "close"),
            run("Close Tab", None, "tab close"),
        ];
        // A terminal's selection is kui's: Copy is kui's standard row
        // there, with no chord of its own (an empty accelerator binds
        // nothing, where a role's default would take ⌘C).
        let copy = if term {
            MenuItem::role(MenuRole::Copy).accel("")
        } else {
            edit("Copy", None, "copy").enabled(view.is_some_and(|v| self.has_selection(v)))
        };
        let edit_menu = vec![
            edit("Undo", None, "undo").enabled(editing),
            edit("Redo", None, "redo").enabled(editing),
            MenuItem::separator(),
            edit("Cut", None, "cut").enabled(view.is_some_and(|v| self.has_selection(v))),
            copy,
            edit("Paste", None, "paste").enabled(editing || term),
            edit("Select All", None, "select all").enabled(editing),
            MenuItem::separator(),
            run("Find…", None, "search").enabled(editing),
            run("Find in Files…", None, "picker grep"),
        ];
        let view_menu = vec![
            // The focused pane's text, as ⌘= ⌘- ⌘0; the window's under
            // them, as ⌘⌥ (docs/design/pane-settings.md Decision 5).
            run("Bigger Text", None, "pane font bigger"),
            run("Smaller Text", None, "pane font smaller"),
            run("Actual Size", None, "pane font reset"),
            run("Bigger Text Everywhere", None, "font bigger"),
            run("Smaller Text Everywhere", None, "font smaller"),
            run("Every Pane's Actual Size", None, "font reset"),
            MenuItem::separator(),
            run("Toggle Light and Dark", None, "theme toggle"),
            run("Toggle the Dock", None, "dock"),
            MenuItem::separator(),
            run("Split Right", None, "vsplit"),
            run("Split Down", None, "split"),
        ];
        let go = vec![
            run("Go to Definition", None, "lsp definition").enabled(lsp_lit(|c| c.definition)),
            run("Go to References", None, "lsp references").enabled(lsp_lit(|c| c.references)),
            run("Go to Symbol…", None, "picker symbols"),
            MenuItem::separator(),
            run("Back", None, "jump back"),
            run("Forward", None, "jump forward"),
            MenuItem::separator(),
            run("Next Problem", None, "lsp diagnostic next")
                .enabled(on_view("lsp diagnostic next")),
            run("Previous Problem", None, "lsp diagnostic prev")
                .enabled(on_view("lsp diagnostic prev")),
        ];
        let window = vec![
            chord(
                MenuItem::new("Minimize").id(Value::map([("window", Value::str("minimize"))])),
                MINIMIZE,
            ),
            MenuItem::separator(),
            run("Next Tab", None, "tab next"),
            run("Previous Tab", None, "tab prev"),
            run("Next Pane", None, "pane next"),
        ];
        let help = vec![
            run("kawoosh Help", None, "help"),
            run("Tutor", None, "tutor"),
            run("Messages", None, "messages"),
        ];
        MenuBar::new(vec![
            BarMenu::new("kawoosh", app),
            BarMenu::new("File", file),
            BarMenu::new("Edit", edit_menu),
            BarMenu::new("View", view_menu),
            BarMenu::new("Go", go),
            BarMenu::new("Window", window),
            BarMenu::new("Help", help),
        ])
    }
}
