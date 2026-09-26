//! Terminal panes (milestone 4): the `term` facade behind each pane, keys
//! encoded to bytes, the pane prefix chord, scrollback into a buffer, and
//! the locations pattern table that turns `src/main.rs:42` under the
//! pointer into an open file (mvp.md Decisions 3, 3b, 5c).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kawoosh_doc::Buffer;
use kawoosh_editor::{ArgKind, Args, KeyStroke, Lookup, Mode, Selection, Spec, motions};
use kawoosh_term::{TermSize, Terminal, encode_key};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, SplitDir};

pub type TermId = u64;

/// A port on a host's loopback for one terminal's way back to this
/// window (`-R`): from the high range, different for each terminal —
/// two sessions asking the master for one port would lose the second's.
fn remote_port() -> u16 {
    use std::hash::{BuildHasher, Hasher};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    h.write_u32(std::process::id());
    20_000 + (h.finish() % 40_000) as u16
}

#[derive(Default)]
pub struct Terminals {
    pub map: HashMap<TermId, Terminal>,
    /// The bell's sound, registered with kui the first time one rings.
    bell_sound: Option<kui_native::SoundId>,
    /// When the bell was last heard: at most one in [`BELL_GAP`].
    bell_at: Option<std::time::Instant>,
    next: TermId,
    /// `<C-w>` was pressed in a terminal pane: the next key is a pane
    /// command (`<C-w>.` sends a literal ^W).
    pub prefix: bool,
    /// `<C-\>` was pressed: `<C-n>` next materialises the scrollback.
    pub backslash: bool,
    /// The wheel's fraction of a line carried per terminal.
    pub carry: HashMap<TermId, f32>,
    /// The scrollback buffers open, each with the terminal it stands in
    /// for: `q` in one goes back to it (`scrollback close`).
    pub scrollbacks: HashMap<kawoosh_doc::BufferId, TermId>,
    /// What each terminal was started for — a session starts a shell
    /// again, and a tool that says `restore`, but not `:term CMD`.
    pub spawned: HashMap<TermId, Spawned>,
    /// Terminals a session brought back, their panes made and their
    /// processes not yet: started on the next frame, once the command
    /// socket is up and `$EDITOR` can reach this window.
    pub pending: Vec<(TermId, Pending)>,
}

/// What a terminal was started for: a command (none for the shell),
/// and the tool it is, if one.
#[derive(Clone, Debug, Default)]
pub struct Spawned {
    pub cmd: Option<String>,
    pub tool: Option<String>,
}

/// A terminal to start: in `cwd`, the tool's command or the shell.
#[derive(Clone, Debug)]
pub struct Pending {
    pub cwd: PathBuf,
    pub tool: Option<String>,
}

impl Terminals {
    pub fn add(&mut self, t: Terminal) -> TermId {
        let id = self.reserve();
        self.map.insert(id, t);
        id
    }

    /// A number for a terminal to come.
    pub fn reserve(&mut self) -> TermId {
        self.next += 1;
        self.next
    }
}

/// Where a terminal's view goes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum TermScroll {
    /// Pages into history (positive) or back toward the bottom.
    Page(i32),
    Top,
    Bottom,
}

/// A `path[:line[:col]]` in `text` around byte `at` — rustc, tsc, grep
/// and shell spellings. Extensible from Lua later (Decision 5c).
pub fn location_at(text: &str, at: usize) -> Option<(String, Option<usize>, Option<usize>)> {
    let span = location_span(text, at)?;
    let token = &text[span.clone()];
    // A drive's colon (`C:\x`) is the path's, not a line's.
    let drive = token
        .as_bytes()
        .get(..3)
        .filter(|b| b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        .map_or(0, |_| 2);
    let mut parts = token[drive..].split(':');
    let path = format!("{}{}", &token[..drive], parts.next()?);
    let line = parts.next().and_then(|s| s.parse().ok());
    let col = parts.next().and_then(|s| s.parse().ok());
    if line.is_none()
        && let Some((l, c)) = paren_position(&text[span.end..])
    {
        return Some((path, Some(l), c));
    }
    Some((path, line, col))
}

/// `(3,5)` or `(3)` right after a path — how `tsc` and MSVC print a
/// place (docs/design/compile.md Decision 5).
fn paren_position(after: &str) -> Option<(usize, Option<usize>)> {
    let (inner, _) = after.strip_prefix('(')?.split_once(')')?;
    let (l, c) = match inner.split_once(',') {
        Some((l, c)) => (l, Some(c.trim().parse().ok()?)),
        None => (inner, None),
    };
    Some((l.trim().parse().ok()?, c))
}

/// Where in `text` the location [`location_at`] reads at byte `at` is:
/// the token around it, a sentence's trailing punctuation off — what a
/// ⌘-click opens and a ⌘-hover underlines.
pub fn location_span(text: &str, at: usize) -> Option<std::ops::Range<usize>> {
    // `\` for the paths Windows tools print (`src\main.rs:42`).
    let is_path_char = |c: char| c.is_alphanumeric() || "./_-~+@:%\\".contains(c);
    let at = at.min(text.len());
    let start = text[..at]
        .char_indices()
        .rev()
        .find(|(_, c)| !is_path_char(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| !is_path_char(*c))
        .map(|(i, _)| at + i)
        .unwrap_or(text.len());
    // Trailing punctuation is the sentence's, a leading `./` is the path's.
    let raw = &text[start..end];
    let tail = raw.trim_end_matches(|c: char| ":.,;".contains(c));
    let token = tail.trim_start_matches([':', ',', ';']);
    if token.is_empty() || !token.contains(['/', '.', '\\']) {
        return None;
    }
    let from = start + (tail.len() - token.len());
    Some(from..from + token.len())
}

/// The shortest time between two bells heard: a program that prints a
/// stream of BELs is one chime, not a buzz.
pub const BELL_GAP: std::time::Duration = std::time::Duration::from_millis(250);

impl Kawoosh {
    /// The bells of the frame (roadmap step 29): a BEL from any
    /// terminal, and the editor's own when `editor.bell` is on (off by
    /// default — a search that finds nothing). `terminal.bell` says what a
    /// terminal's does: `sound` (the default) a short chime, `visual`
    /// none, `off` nothing at all. Unless off, a terminal not on screen
    /// marks its tab, i3's urgent workspace, until the tab is visited.
    pub(crate) fn ring_bells(&mut self, ui: &mut kui_native::Ui<'_>) {
        let rang: Vec<TermId> = self
            .terms
            .map
            .iter_mut()
            .filter_map(|(id, t)| std::mem::take(&mut t.bell).then_some(*id))
            .collect();
        let editor =
            std::mem::take(&mut self.ed.bell) && self.ed.settings.bool("editor.bell") == Some(true);
        let active = self.layout.tab;
        if let Some(t) = self.layout.tabs.get_mut(active) {
            t.bell = false;
        }
        if rang.is_empty() && !editor {
            return;
        }
        let how = self
            .ed
            .settings
            .str("terminal.bell")
            .unwrap_or("sound")
            .to_string();
        let mut sound = editor;
        if how != "off" && !rang.is_empty() {
            sound |= how == "sound";
            let shown = self.layout.visible_panes();
            for id in &rang {
                let Some(pane) = self
                    .layout
                    .panes
                    .iter()
                    .find(|(_, c)| matches!(c, Content::Terminal(t) if t == id))
                    .map(|(p, _)| *p)
                else {
                    continue;
                };
                if shown.contains(&pane) {
                    continue;
                }
                for (i, tab) in self.layout.tabs.iter_mut().enumerate() {
                    let mut ps = Vec::new();
                    tab.panes(&mut ps);
                    if i != active && ps.contains(&pane) {
                        tab.bell = true;
                    }
                }
            }
        }
        let now = std::time::Instant::now();
        if !sound || self.terms.bell_at.is_some_and(|t| now - t < BELL_GAP) {
            return;
        }
        self.terms.bell_at = Some(now);
        let id = *self.terms.bell_sound.get_or_insert_with(|| {
            ui.core()
                .add_sound(kui_native::audio::blip(44_100, 988.0, 120.0, 0.25))
        });
        ui.play(id, kui_native::PlayOptions::default());
    }

    /// Spawns a shell (or `cmd`) sized for a pane, with the `$EDITOR`
    /// handoff in its environment (Decision 3b).
    pub fn spawn_terminal(&mut self, cmd: Option<&str>, cwd: Option<&Path>) -> Option<TermId> {
        self.spawn_terminal_as(None, cmd, cwd)
    }

    /// [`Kawoosh::spawn_terminal`] under a number reserved for it — a
    /// session's pane, made before its process.
    fn spawn_terminal_as(
        &mut self,
        id: Option<TermId>,
        cmd: Option<&str>,
        cwd: Option<&Path>,
    ) -> Option<TermId> {
        let mut envs = vec![
            ("TERM_PROGRAM".to_string(), "kawoosh".to_string()),
            (
                "TERM_PROGRAM_VERSION".to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ),
            (
                "TERM_APPEARANCE".to_string(),
                if self.dark { "dark" } else { "light" }.to_string(),
            ),
        ];
        if let Some(sock) = &self.socket {
            envs.push(("KAWOOSH_SOCKET".into(), sock.display().to_string()));
            if let Ok(exe) = std::env::current_exe() {
                // The binary itself, for a hook that asks it something
                // (`kawoosh theme`) without it being on the PATH.
                envs.push(("KAWOOSH_BIN".into(), exe.display().to_string()));
                // One program with no arguments where there can be one
                // (`kawoosh-edit`, `app::shipped_editor`): a shell that
                // runs `$EDITOR` as a path — nushell's `config env` —
                // finds no program called `kawoosh edit --wait`.
                let shim = match &self.editor_shim {
                    Some(p) => p.display().to_string(),
                    None => format!("{} edit --wait", exe.display()),
                };
                envs.push(("EDITOR".into(), shim.clone()));
                envs.push(("VISUAL".into(), shim.clone()));
                envs.push(("GIT_EDITOR".into(), shim));
            }
        }
        // The PATH a shell made, where the window was opened outside one
        // (`shell_env`): the terminal's shell is looked up on it, and
        // starts from it.
        if let Some(path) = kawoosh_systems::shell_env::path() {
            envs.push(("PATH".into(), path.to_string_lossy().into_owned()));
        }
        let size = TermSize { rows: 24, cols: 80 };
        let cwd = cwd
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.cwd.clone());
        let host =
            kawoosh_systems::fs::domain_of(&cwd).map(|(n, d)| (n.to_string(), d.to_path_buf()));
        let spawned = match &host {
            Some((name, dir)) => {
                let Some(argv) = self.remote_terminal_argv(name, dir, cmd) else {
                    self.ed.message = format!("{name}: not connected (:domain connect {name})");
                    return None;
                };
                // The local end runs from a local directory.
                let home = kawoosh_systems::fs::home().unwrap_or_else(std::env::temp_dir);
                Terminal::spawn_argv(&argv, Some(&home), size, &[]).map(|(mut t, r)| {
                    t.set_domain(name, cwd.clone());
                    (t, r)
                })
            }
            None => {
                let shell = self.ed.settings.str("terminal.shell");
                Terminal::spawn(shell, cmd, Some(&cwd), size, &envs)
            }
        };
        self.adopt_terminal(id, spawned, cmd)
    }

    /// A program in a pty with no shell in between (`Terminal::spawn_argv`)
    /// — the ssh master of a domain, whose arguments must reach it whole
    /// whatever the user's shell quotes like.
    pub(crate) fn spawn_terminal_argv(&mut self, argv: &[String], cwd: &Path) -> Option<TermId> {
        let size = TermSize { rows: 24, cols: 80 };
        let spawned = Terminal::spawn_argv(argv, Some(cwd), size, &[]);
        self.adopt_terminal(None, spawned, None)
    }

    /// A terminal just spawned, taken in: its scrollback set, its number
    /// (`id`, or a new one), its reader on the io thread.
    fn adopt_terminal(
        &mut self,
        id: Option<TermId>,
        spawned: anyhow::Result<(Terminal, Box<dyn std::io::Read + Send>)>,
        cmd: Option<&str>,
    ) -> Option<TermId> {
        match spawned {
            Ok((mut term, reader)) => {
                term.set_scrollback(self.scrollback_setting());
                let exited = term.exit_waiter();
                let id = match id {
                    Some(id) => {
                        self.terms.map.insert(id, term);
                        id
                    }
                    None => self.terms.add(term),
                };
                self.terms.spawned.insert(
                    id,
                    Spawned {
                        cmd: cmd.map(str::to_string),
                        tool: None,
                    },
                );
                self.io.watch_pty(id, reader, exited);
                Some(id)
            }
            Err(e) => {
                self.ed.message = format!("cannot spawn a terminal: {e:#}");
                None
            }
        }
    }

    /// The `ssh -t` that runs a shell (or `cmd`) on domain `name` in
    /// `dir` (docs/design/domains.md Decisions 6 and 7): the environment
    /// a local terminal gets put on the host's command line, `$EDITOR`
    /// the CLI written to the host at connect (`~/.cache/kawoosh`), which
    /// talks back to this window's socket over a port forwarded for this
    /// terminal. None when the domain is not up.
    fn remote_terminal_argv(
        &self,
        name: &str,
        dir: &Path,
        cmd: Option<&str>,
    ) -> Option<Vec<String>> {
        use kawoosh_systems::io::{remote_script, shell_quote, transport_of};
        let t = transport_of(name)?;
        let mut envs = vec![
            ("TERM_PROGRAM".to_string(), "kawoosh".to_string()),
            (
                "TERM_PROGRAM_VERSION".to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ),
            (
                "TERM_APPEARANCE".to_string(),
                if self.dark { "dark" } else { "light" }.to_string(),
            ),
            ("KAWOOSH_DOMAIN".to_string(), name.to_string()),
        ];
        let port = remote_port();
        if self.socket.is_some() {
            let shim = "$HOME/.cache/kawoosh".to_string();
            envs.push(("KAWOOSH_PORT".into(), port.to_string()));
            envs.push(("KAWOOSH_BIN".into(), format!("{shim}/kawoosh")));
            for k in ["EDITOR", "VISUAL", "GIT_EDITOR"] {
                envs.push((k.into(), format!("{shim}/kawoosh-edit")));
            }
        }
        let exec = match cmd {
            Some(c) => format!("exec \"${{SHELL:-/bin/sh}}\" -lc {}", shell_quote(c)),
            None => "exec \"${SHELL:-/bin/sh}\" -l".to_string(),
        };
        let script = remote_script(dir, &envs, &exec, cmd.is_none());
        let forward = self.socket.as_deref().map(|s| (port, s));
        Some(t.remote_argv(&script, true, forward))
    }

    /// `terminal.scrollback`: the lines of history a terminal keeps.
    fn scrollback_setting(&self) -> usize {
        self.ed
            .settings
            .int("terminal.scrollback")
            .map_or(kawoosh_term::HISTORY, |n| n.clamp(0, 1_000_000) as usize)
    }

    /// Once a frame: the scrollback setting into every terminal (a
    /// smaller cap drops what is past it at once).
    pub(crate) fn sync_term_settings(&mut self) {
        let lines = self.scrollback_setting();
        for t in self.terms.map.values_mut() {
            t.set_scrollback(lines);
        }
    }

    /// The terminals a session brought back, started: the shell, or the
    /// tool's command, in the directory it was left in (the session's
    /// cwd when that is gone). One that will not start takes its pane
    /// with it.
    pub(crate) fn spawn_pending(&mut self) {
        for (id, p) in std::mem::take(&mut self.terms.pending) {
            // A shell on a host waits, its pane kept, for the domain to
            // be connected (docs/design/domains.md Decision 8).
            if let Some((d, _)) = kawoosh_systems::fs::domain_of(&p.cwd)
                && !kawoosh_doc::fs::is_registered(d)
            {
                let d = d.to_string();
                self.domains.terminals.push((d, id, p));
                continue;
            }
            let cmd = p
                .tool
                .as_ref()
                .and_then(|n| self.scripting.tools.get(n))
                .map(|d| d.cmd.clone());
            let cwd = if kawoosh_systems::fs::domain_of(&p.cwd).is_some() || p.cwd.is_dir() {
                p.cwd
            } else {
                self.cwd.clone()
            };
            // A tool no plugin registers any more is not a shell under
            // its name: the pane goes, as one that cannot spawn does.
            let spawned = if p.tool.is_some() && cmd.is_none() {
                None
            } else {
                self.spawn_terminal_as(Some(id), cmd.as_deref(), Some(&cwd))
            };
            match spawned {
                Some(id) => {
                    if let Some(name) = p.tool {
                        self.scripting.tool_terms.insert(name.clone(), id);
                        self.terms.spawned.entry(id).or_default().tool = Some(name);
                    }
                }
                None => {
                    let pane = self
                        .layout
                        .all_panes()
                        .into_iter()
                        .find(|q| self.term_of(*q) == Some(id));
                    if let Some(pane) = pane {
                        self.layout.close(pane);
                    }
                }
            }
        }
    }

    /// Terminal `id`'s view moved: a page up or down (`by` pages), to
    /// the top or the bottom.
    pub(crate) fn term_scroll(&mut self, id: TermId, how: TermScroll) {
        let Some(t) = self.terms.map.get_mut(&id) else {
            return;
        };
        let page = t.size().rows.saturating_sub(1).max(1) as i32;
        match how {
            TermScroll::Page(by) => t.scroll(by * page),
            TermScroll::Top => t.scroll(t.history_size() as i32),
            TermScroll::Bottom => t.scroll_to_bottom(),
        }
    }

    /// The focused terminal, or the message saying there is none.
    fn focused_term(&mut self) -> Option<TermId> {
        let t = self.term_of(self.layout.focused());
        if t.is_none() {
            self.ed.message = "not a terminal".into();
        }
        t
    }

    /// A terminal with no process, in a new split — for tests, which feed
    /// it bytes through [`Kawoosh::feed_terminal`].
    pub fn add_headless_terminal(&mut self) -> TermId {
        let t = Terminal::headless(TermSize { rows: 24, cols: 80 });
        let id = self.terms.add(t);
        self.layout.split(SplitDir::V, Content::Terminal(id));
        id
    }

    /// The frame's palette into every terminal, shown or not: the
    /// theme's fg and panel, the ANSI sixteen of its base
    /// (`palette::ansi`), and the base itself — what a program's colour
    /// question is answered with, and a flip of the base is what one
    /// under mode 2031 is told of (`Terminal::set_palette`).
    pub(crate) fn sync_term_palettes(&mut self) {
        let pal = kawoosh_term::Palette {
            fg: self.pal.fg.to_hex(),
            bg: self.pal.panel.to_hex(),
            ansi: self.ansi_for(self.dark),
            dark: self.dark,
        };
        for term in self.terms.map.values_mut() {
            if term.palette() != pal {
                term.set_palette(pal);
            }
        }
    }

    pub fn feed_terminal(&mut self, id: TermId, bytes: &[u8]) {
        if let Some(t) = self.terms.map.get_mut(&id) {
            t.feed(bytes);
        }
    }

    /// A key in a focused terminal pane.
    pub(crate) fn term_key(&mut self, id: TermId, stroke: KeyStroke) {
        let note = stroke.notation();
        if self.terms.backslash {
            self.terms.backslash = false;
            if note == "<C-n>" {
                self.scrollback_to_buffer(id);
                return;
            }
        }
        if self.terms.prefix {
            self.terms.prefix = false;
            if note == "." {
                if let Some(t) = self.terms.map.get_mut(&id) {
                    t.input(&[0x17]);
                }
                return;
            }
            if note == ":" {
                self.open_cmdline();
                return;
            }
            let keys = ["<C-w>".to_string(), note.clone()];
            self.ed.sync_settings();
            if let Lookup::Exact(bs) = self.ed.keymap.lookup_lenient(Mode::Normal, &keys) {
                let bs = bs.to_vec();
                self.run_bindings(&bs);
            }
            return;
        }
        // The view's own keys, where a program on the whole screen
        // keeps them: shift with the page keys moves through history.
        let alt_screen = self.terms.map.get(&id).is_some_and(|t| t.is_alt_screen());
        let scroll = match note.as_str() {
            "<S-PageUp>" => Some(TermScroll::Page(1)),
            "<S-PageDown>" => Some(TermScroll::Page(-1)),
            "<S-Home>" => Some(TermScroll::Top),
            "<S-End>" => Some(TermScroll::Bottom),
            _ => None,
        };
        if let Some(how) = scroll.filter(|_| !alt_screen) {
            self.term_scroll(id, how);
            return;
        }
        match note.as_str() {
            "<C-w>" => {
                self.terms.prefix = true;
                return;
            }
            "<C-\\>" => {
                self.terms.backslash = true;
                return;
            }
            _ => {}
        }
        // A ⌘ chord bound to nothing is nothing to the shell, not its
        // letter: a pty has no use for ⌘ (`Kawoosh::pane_chord`).
        if stroke.sup {
            return;
        }
        let Some(t) = self.terms.map.get_mut(&id) else {
            return;
        };
        if let Some(bytes) = encode_key(
            &stroke.code,
            stroke.text.as_deref(),
            stroke.ctrl,
            stroke.alt,
            stroke.shift,
            t.app_cursor_keys(),
        ) {
            t.scroll_to_bottom();
            t.input(&bytes);
        }
    }

    /// The scrollback and screen of terminal `id` as a buffer with full
    /// modal editing (Decision 3) — copy mode, wezterm's `<C-S-x>`. The
    /// buffer takes the terminal's pane, the caret lands where the
    /// terminal's cursor was, and `q` gives the pane back
    /// (`scrollback_close`), so the round trip is two keys. A terminal
    /// with no pane of its own gets a split.
    pub fn scrollback_to_buffer(&mut self, id: TermId) {
        let Some(t) = self.terms.map.get(&id) else {
            return;
        };
        let (text, runs) = t.scrollback_styled();
        // The view's top row as a line of the text: the view starts at
        // what the pane showed (roadmap step 31).
        let top_line = t.history_size().saturating_sub(t.display_offset());
        // The caret at the terminal's cursor when the pane shows it, as
        // wezterm's copy mode starts; scrolled back past it, at the top
        // row shown, where the eye was.
        let (cursor_line, cursor_col) = t.scrollback_cursor();
        let caret = if cursor_line < top_line + t.size().rows as usize {
            (cursor_line, cursor_col)
        } else {
            (top_line, 0)
        };
        let name = format!(
            "*scrollback {}*",
            if t.title.is_empty() {
                id.to_string()
            } else {
                t.title.clone()
            }
        );
        let mut buf = Buffer::new(name, &text);
        buf.language = "scrollback".into();
        let bid = self.ed.add_buffer(buf);
        let v = self.ed.add_view(bid);
        // The view scrolled to the top row the pane showed, the caret on
        // its character — or, past where trailing blanks (a prompt's
        // space) were trimmed, the line's last one, as `$` lands.
        let b = &self.ed.buffers[bid];
        let last = b.line_count().saturating_sub(1);
        let r = b.line_range(caret.0.min(last));
        let at = b.slice(r.clone()).char_indices().nth(caret.1).map_or(
            if r.is_empty() {
                r.end
            } else {
                b.prev_char(r.end)
            },
            |(i, _)| r.start + i,
        );
        self.ed.views[v].sels = kawoosh_editor::Selections::single(Selection::point(at));
        self.ed.views[v].top = top_line.min(last);
        // The colours it was printed in, as a paint of their own.
        let version = self.ed.buffers[bid].version();
        let spans = runs
            .into_iter()
            .map(|(r, c)| (r, format!("#{:06x}", c >> 8)))
            .collect();
        self.scripting.paints.entry(bid).or_default().insert(
            "terminal".into(),
            crate::scripting::Painted { version, spans },
        );
        self.terms.scrollbacks.insert(bid, id);
        let pane = self
            .layout
            .all_panes()
            .into_iter()
            .find(|p| matches!(self.layout.content(*p), Some(Content::Terminal(t)) if t == id));
        match pane {
            Some(p) => {
                self.layout.panes.insert(p, Content::Editor(v));
                self.layout.focus(p);
            }
            None => {
                self.layout.split(SplitDir::V, Content::Editor(v));
            }
        }
    }

    /// `q` in a scrollback buffer: the buffer goes and its terminal has
    /// the pane again. A terminal that is gone leaves the buffer as it
    /// is, an ordinary pane to `:close`.
    pub fn scrollback_close(&mut self) {
        let pane = self.layout.focused();
        let Some(v) = self.view_of(pane) else {
            return;
        };
        let bid = self.ed.views[v].buffer;
        let Some(t) = self.terms.scrollbacks.get(&bid).copied() else {
            return;
        };
        if !self.terms.map.contains_key(&t) {
            self.ed.message = "the terminal is gone".into();
            return;
        }
        self.terms.scrollbacks.remove(&bid);
        self.layout.panes.insert(pane, Content::Terminal(t));
        self.drop_view(v);
        if !self.buffer_shown(bid) {
            self.ed.remove_buffer(bid);
            self.release_waiters(bid);
        }
    }

    /// The columns of terminal `id`'s screen row `row` a ⌘-click at `col`
    /// would open: the location's span when what it names is a file or a
    /// directory there, else none — so the hover underlines only what a
    /// click will open. A column is a character of the row's text, as
    /// [`Self::open_location_at`] counts them.
    pub(crate) fn location_cols(
        &self,
        id: TermId,
        row: usize,
        col: usize,
    ) -> Option<std::ops::Range<usize>> {
        let t = self.terms.map.get(&id)?;
        let text = t.row_text(row);
        let at = text
            .char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        let span = location_span(&text, at)?;
        let (path, _, _) = location_at(&text, at)?;
        let base = t.cwd().unwrap_or_else(|| self.cwd.clone());
        if !kawoosh_systems::fs::expand(Path::new(&path), &base).exists() {
            return None;
        }
        let first = text[..span.start].chars().count();
        Some(first..first + text[span].chars().count())
    }

    /// Opens the file named at `(row, col)` of terminal `id`'s screen —
    /// `gf` across the terminal/editor boundary (Decision 5c).
    pub fn open_location_at(&mut self, id: TermId, row: usize, col: usize) -> bool {
        let Some(t) = self.terms.map.get(&id) else {
            return false;
        };
        let text = t.row_text(row);
        let at = text
            .char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        let Some((path, line, colno)) = location_at(&text, at) else {
            self.ed.message = "no path under the pointer".into();
            return false;
        };
        let base = t.cwd().unwrap_or_else(|| self.cwd.clone());
        let full = kawoosh_systems::fs::expand(Path::new(&path), &base);
        if !full.exists() {
            self.ed.message = format!("not found: {}", full.display());
            return false;
        }
        self.open_in_editor(&full, line, colno);
        true
    }

    /// Opens `path` in an editor pane — the focused one, or a split
    /// beside a terminal — and moves to `line:col` when given.
    pub fn open_in_editor(&mut self, path: &Path, line: Option<usize>, col: Option<usize>) {
        let resolved = self.resolve(path);
        if self.domain_gate(&resolved, crate::domains::Pending::Open(resolved.clone())) {
            return;
        }
        if self.opened_by_plugin(path) {
            return;
        }
        if self.focused_view().is_none() && self.claim_launcher().is_none() {
            // From a terminal: prefer an editor pane already on screen.
            let editor_pane = self
                .layout
                .visible_panes()
                .into_iter()
                .find(|p| matches!(self.layout.content(*p), Some(Content::Editor(_))));
            match editor_pane {
                Some(p) => self.layout.focus(p),
                None => {
                    let Some(id) = self.buffer_for(path) else {
                        return;
                    };
                    let v = self.ed.add_view(id);
                    self.layout.split(SplitDir::H, Content::Editor(v));
                }
            }
        }
        self.open_file(path);
        if let (Some(v), Some(ln)) = (self.focused_view(), line) {
            let buf = self.ed.buffer_of(v);
            let ln = ln.max(1).min(buf.line_count()) - 1;
            let off = match col {
                Some(c) => motions::offset_at(buf, ln, c.saturating_sub(1)),
                None => motions::first_nonblank(buf, ln),
            };
            self.ed.views[v].sels = kawoosh_editor::Selections::single(Selection::point(off));
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("terminal")
                .alias(&["term"])
                .args(Args::rest(&[ArgKind::Text]))
                .doc("a terminal in a split below, running CMD or the shell"),
            |k, ctx| {
                let cmd = if ctx.args.is_empty() {
                    None
                } else {
                    Some(ctx.args.join(" "))
                };
                // From a terminal, where its shell is.
                let cwd = k
                    .term_of(k.layout.focused())
                    .and_then(|t| k.terms.map.get(&t))
                    .and_then(|t| t.cwd())
                    .unwrap_or_else(|| k.cwd.clone());
                if let Some(t) = k.spawn_terminal(cmd.as_deref(), Some(&cwd)) {
                    k.fill_or_split(SplitDir::V, Content::Terminal(t));
                }
            },
        ),
        cmd(
            Spec::new("shell")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("`:!CMD`: CMD in a terminal below, `%` the file (`%:h` its directory, `%:t` its name), quoted — a prompt it asks is answered there"),
            |k, ctx| {
                let line = ctx.args.join(" ");
                if line.trim().is_empty() {
                    k.ed.message = "shell: what to run".into();
                    return;
                }
                let cmd = match k.ed.expand_percent(ctx.view, &line) {
                    Ok(c) => c,
                    Err(e) => {
                        k.ed.message = e;
                        return;
                    }
                };
                let cwd = k.cwd.clone();
                if let Some(t) = k.spawn_terminal(Some(&cmd), Some(&cwd)) {
                    k.fill_or_split(SplitDir::V, Content::Terminal(t));
                }
            },
        ),
        cmd(
            Spec::new("scrollback").doc(
                "the terminal's scrollback as a buffer in its pane (`<C-S-x>`; `q` or `<C-S-x>` again goes back)",
            ),
            |k, _| {
                if let Some(t) = k.term_of(k.layout.focused()) {
                    k.scrollback_to_buffer(t);
                } else if k
                    .focused_view()
                    .is_some_and(|v| k.ed.buffer_of(v).language.as_ref() == "scrollback")
                {
                    k.scrollback_close();
                } else {
                    k.ed.message = "scrollback needs terminal".into();
                }
            },
        ),
        cmd(
            Spec::new("terminal page up")
                .when(&["terminal"])
                .doc("the terminal's view a page into its history, COUNT pages (`<S-PageUp>`)"),
            |k, ctx| {
                if let Some(t) = k.focused_term() {
                    k.term_scroll(t, TermScroll::Page(ctx.count.max(1) as i32));
                }
            },
        ),
        cmd(
            Spec::new("terminal page down")
                .when(&["terminal"])
                .doc("the terminal's view a page back toward the prompt, COUNT pages (`<S-PageDown>`)"),
            |k, ctx| {
                if let Some(t) = k.focused_term() {
                    k.term_scroll(t, TermScroll::Page(-(ctx.count.max(1) as i32)));
                }
            },
        ),
        cmd(
            Spec::new("terminal bottom")
                .when(&["terminal"])
                .doc("the terminal's view back at the prompt (`<S-End>`)"),
            |k, _| {
                if let Some(t) = k.focused_term() {
                    k.term_scroll(t, TermScroll::Bottom);
                }
            },
        ),
        cmd(
            Spec::new("terminal prompt prev")
                .when(&["terminal"])
                .doc("the prompt above the view at its top (the shell's OSC 133 marks; `<D-Up>` `<C-S-Up>`)"),
            |k, _| k.jump_prompt(true),
        ),
        cmd(
            Spec::new("terminal prompt next")
                .when(&["terminal"])
                .doc("the prompt below at the top, past the last back at the bottom (`<D-Down>` `<C-S-Down>`)"),
            |k, _| k.jump_prompt(false),
        ),
        cmd(
            Spec::new("terminal output")
                .when(&["terminal"])
                .doc("the last command's output to the clipboard (the shell's OSC 133 marks; `<C-S-o>`)"),
            |k, _| {
                let Some(t) = k.focused_term() else { return };
                match k.terms.map.get(&t).and_then(|t| t.last_output()) {
                    Some(text) => {
                        let n = if text.is_empty() { 0 } else { text.lines().count() };
                        k.clip_out = Some(text);
                        k.ed.message = format!(
                            "the last command's output: {n} line{} copied",
                            if n == 1 { "" } else { "s" }
                        );
                    }
                    None => {
                        k.ed.message =
                            "no command marked through to its end — the shell sends no OSC 133 (see :terminal integration)"
                                .into()
                    }
                }
            },
        ),
        cmd(
            Spec::new("terminal integration")
                .doc("the lines that make zsh, bash or nushell say where it is and mark its prompts (OSC 7, OSC 133)"),
            |k, _| k.show_in_pane("*shell integration*", INTEGRATION),
        ),
        cmd(
            Spec::new("scrollback close")
                .when(&["language:scrollback"])
                .doc("close the scrollback buffer, its terminal back in the pane (`q`)"),
            |k, _| k.scrollback_close(),
        ),
        // `<Esc>` in copy mode's normal mode (roadmap step 31): the
        // ladder's rungs first — the extra carets, the search's paint —
        // and, with nothing left to clear, the pane back to the
        // terminal, as wezterm's copy mode leaves on `<Esc>`.
        cmd(
            Spec::new("scrollback escape")
                .when(&["language:scrollback"])
                .doc("`<Esc>` in copy mode: clear what the ladder clears, else leave it"),
            |k, _| {
                let Some(v) = k.focused_view() else { return };
                let busy = k.ed.views[v].sels.len() > 1 || (k.ed.search.is_some() && k.ed.search_hl);
                if busy {
                    k.ed.run(v, "normal", &[], None);
                } else {
                    k.scrollback_close();
                }
            },
        ),
    ]
}

impl Kawoosh {
    fn jump_prompt(&mut self, back: bool) {
        let Some(t) = self.focused_term() else { return };
        let Some(term) = self.terms.map.get_mut(&t) else {
            return;
        };
        if term.commands().is_empty() {
            self.ed.message =
                "no prompts marked — the shell sends no OSC 133 (see :terminal integration)".into();
        } else if !term.jump_prompt(back) {
            self.ed.message = if back {
                "no prompt further back".into()
            } else {
                "at the prompt".into()
            };
        }
    }
}

/// What a shell needs to say where it is (OSC 7, for `gf`, a new
/// terminal from this one, a session) and to mark its prompts (OSC 133,
/// for `<D-Up>` and `terminal output`). Without it the directory is
/// still read from the shell's process; the marks need the shell.
const INTEGRATION: &str = r#"# Shell integration for kawoosh: OSC 7 (the directory) and OSC 133 (the
# prompts). kawoosh sets TERM_PROGRAM=kawoosh in every terminal.

# ── nushell (config.nu) — both are built in:
$env.config.shell_integration.osc7 = true
$env.config.shell_integration.osc133 = true

# ── zsh (~/.zshrc)
if [[ "$TERM_PROGRAM" == kawoosh ]]; then
  _kawoosh_precmd() {
    local s=$?
    printf '\e]133;D;%s' "$s"
    printf '\e]7;file://%s%s' "$HOST" "${PWD// /%20}"
    printf '\e]133;A'
  }
  _kawoosh_preexec() { printf '\e]133;C' }
  precmd_functions+=(_kawoosh_precmd)
  preexec_functions+=(_kawoosh_preexec)
  PS1="$PS1%{$(printf '\e]133;B')%}"
fi

# ── bash (~/.bashrc)
if [[ "$TERM_PROGRAM" == kawoosh ]]; then
  _kawoosh_prompt() {
    local s=$?
    printf '\e]133;D;%s' "$s"
    printf '\e]7;file://%s%s' "$HOSTNAME" "${PWD// /%20}"
    printf '\e]133;A'
  }
  PROMPT_COMMAND="_kawoosh_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  PS0=$'\e]133;C'
  PS1="$PS1"$'\[\e]133;B\]'
fi

# ── Directory jumps in kawoosh's picker (`kawoosh pick dirs`: zoxide's
# directories, the pick on stdout, status 1 when closed). `<C-S-z>` in
# the pane does the same at an empty prompt, with the marks above.
# nushell:
def --env zk [...q] { cd (^$env.KAWOOSH_BIN pick dirs ...$q) }
# zsh, bash:
zk() { local d; d=$("$KAWOOSH_BIN" pick dirs "$@") && cd "$d"; }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations() {
        let l = "error: x\n  --> src/main.rs:42:7\n";
        let at = l.find("main").unwrap();
        assert_eq!(
            location_at(l, at),
            Some(("src/main.rs".into(), Some(42), Some(7)))
        );
        assert_eq!(
            location_at("see ./a.txt.", 6),
            Some(("./a.txt".into(), None, None))
        );
        assert_eq!(location_at("just words here", 6), None);
        assert_eq!(
            location_at("lib/foo.rb:10: warning", 4),
            Some(("lib/foo.rb".into(), Some(10), None))
        );
        // Windows spellings: a backslash path, and a drive's colon.
        assert_eq!(
            location_at("  --> src\\main.rs:42:7", 8),
            Some(("src\\main.rs".into(), Some(42), Some(7)))
        );
        assert_eq!(
            location_at("at C:\\work\\a.rs:3 here", 6),
            Some(("C:\\work\\a.rs".into(), Some(3), None))
        );
        // tsc's and MSVC's parentheses.
        assert_eq!(
            location_at("src/a.ts(3,5): error TS2322: no", 2),
            Some(("src/a.ts".into(), Some(3), Some(5)))
        );
        assert_eq!(
            location_at("main.c(12): warning C4996", 2),
            Some(("main.c".into(), Some(12), None))
        );
        assert_eq!(
            location_at("see a.ts (the file)", 5),
            Some(("a.ts".into(), None, None))
        );
    }
}
