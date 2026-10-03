//! Terminal panes (milestone 4): the `term` facade behind each pane, keys
//! encoded to bytes, the escape to normal mode's keys, scrollback into a buffer, and
//! the locations pattern table that turns `src/main.rs:42` under the
//! pointer into an open file (mvp.md Decisions 3, 3b, 5c).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kawoosh_doc::Buffer;
use kawoosh_editor::keymap::{LEADER, parse_notation};
use kawoosh_editor::{ArgKind, Args, KeyStroke, Lookup, Mode, Selection, Spec};
use kawoosh_systems::io::PtyOutput;
use kawoosh_term::{TermSize, Terminal, encode_key, encode_super_key};
use kui_native::KeyPress;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, PaneId, Place};
use crate::links::Target;

pub type TermId = u64;

/// The parsing a frame does of the terminals' output, at most. A flood
/// of output takes this of every frame and leaves the rest to the next,
/// the window drawing and its keys heard between (a `^C` among them);
/// the reader waits meanwhile ([`PTY_HELD`]). What a frame at 120 Hz
/// has left after drawing a full-window grid: measured with a 512 MB
/// `cat` on a 237×109 grid, 6 ms kept every frame in its refresh and
/// 8 ms made every other one miss it — fewer frames, and less parsed.
///
/// [`PTY_HELD`]: kawoosh_systems::io::PTY_HELD
const PTY_BUDGET: Duration = Duration::from_millis(6);

/// The output parsed at a go, each terminal with some in turn: a
/// fraction of a millisecond, so the budget is kept to near it.
const PTY_CHUNK: usize = 64 * 1024;

/// A terminal's output on its way to its screen: what its pty's reader
/// holds, and what the frame took of it and has not yet parsed.
pub struct Flow {
    output: PtyOutput,
    taken: Vec<u8>,
    /// How far into `taken` was parsed.
    at: usize,
}

/// A link's cells on a terminal's screen: a row and its columns, for
/// each row it is on.
pub(crate) type LinkCells = Vec<(usize, std::ops::Range<usize>)>;

/// A link on a terminal's screen ([`Kawoosh::term_link`]).
struct TermLink {
    target: Target,
    /// The screen rows it covers and their columns: more than one when
    /// a long line wrapped it.
    rows: LinkCells,
    /// The address a program printed with it (OSC 8), where the text
    /// need not be it.
    uri: Option<String>,
    /// Where a relative path is looked for.
    bases: Vec<PathBuf>,
}

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

/// A terminal spawned as one step among the process's spawns
/// (`kawoosh_systems::spawn`): its pty opened and its shell started with
/// no language server or job started in between, which would take the
/// pty's sides with it — and the window would wait in here on a shell
/// that had started long since.
fn in_a_pty<T>(spawn: impl FnOnce() -> T) -> T {
    let _held = kawoosh_systems::spawn::lock();
    spawn()
}

#[derive(Default)]
pub struct Terminals {
    pub map: HashMap<TermId, Terminal>,
    /// Each terminal's output from its pty, while the pty is read.
    flows: HashMap<TermId, Flow>,
    /// The terminals with output to parse: its reader said so, or a
    /// frame's budget left some. In order, for each to have its turn.
    pub flowing: BTreeSet<TermId>,
    /// Terminals whose pty closed while their output was still to
    /// parse: closed once it is, the output on screen before the close.
    closing: HashSet<TermId>,
    /// The bell's sound, registered with kui the first time one rings.
    bell_sound: Option<kui_native::SoundId>,
    /// When the bell was last heard: at most one in [`BELL_GAP`].
    bell_at: Option<std::time::Instant>,
    next: TermId,
    /// The presses the ptys were sent under kitty's keyboard protocol and
    /// have not seen come up: a release is the program's only when its
    /// press was (terminal-keys.md Decision 5) — the escape's, a kept
    /// chord's, never.
    pub held: Vec<KeyPress>,
    /// Raw set by hand (`terminal raw`, `<C-\>r`), for the foreground
    /// process group in front when it was set: it holds while that
    /// program is in front — the shell's through an `ls` — and the next
    /// program is `terminal.raw`'s to say (terminal-keys.md Decision 2).
    pub raw: HashMap<TermId, (Option<i32>, bool)>,
    /// The keys typed since `terminal.escape` in a terminal pane
    /// (terminal-keys.md Decision 1): normal mode's, looked up as they
    /// come, the which-key open on them. None while the pty has them.
    pub escape: Option<Vec<String>>,
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
    /// Kept terminals whose process ended (a `:!CMD`'s, [`Spawned::keep`]),
    /// with how it did: the pane stays, its output there to read, and
    /// every key is normal mode's — `r` runs the line again, `q` closes.
    pub done: HashMap<TermId, Option<kawoosh_term::Exit>>,
}

/// What a terminal was started for: a command (none for the shell),
/// and the tool it is, if one.
#[derive(Clone, Debug, Default)]
pub struct Spawned {
    pub cmd: Option<String>,
    pub tool: Option<String>,
    /// A `:!CMD`'s: the directory its line ran in. Its pane outlives
    /// the process ([`Terminals::done`]), for `r` to run it there again.
    pub keep: Option<PathBuf>,
    /// A kept one's program and arguments, run with no shell between
    /// ([`Kawoosh::spawn_bang_argv`]): what `r` runs again.
    pub argv: Option<Vec<String>>,
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

    /// Terminal `id` gone: its screen, and its output — the reader, if
    /// it still reads, let go.
    pub fn forget(&mut self, id: TermId) {
        self.map.remove(&id);
        self.flows.remove(&id);
        self.flowing.remove(&id);
        self.closing.remove(&id);
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

    /// The directory of what is in front: a terminal's, where its shell
    /// is; a Lua view's, what its `here` says (`:du`'s directory); a
    /// listing's, the directory it lists; a file's, the one it is in;
    /// else the working directory. Where `:terminal here` starts.
    pub(crate) fn here_dir(&self) -> PathBuf {
        if let Some(d) = self
            .term_of_focused()
            .and_then(|t| self.terms.map.get(&t))
            .and_then(|t| t.cwd())
        {
            return d;
        }
        let focused = self.layout.focused();
        if let Some(Content::Lua(name)) = self.layout.content(focused)
            && let Some(d) = self
                .scripting
                .rt
                .as_ref()
                .and_then(|rt| rt.view_here(&name, focused))
        {
            return d;
        }
        let from_buffer = self.focused_view().and_then(|v| {
            let b = self.ed.buffer_of(v);
            // A listing is named for the directory it lists (`dir.lua`'s
            // `PREFIX`); it is no file, so `%` there stays refused.
            let listed = (b.language.as_ref() == "dir")
                .then(|| b.name.strip_prefix("dir: "))
                .flatten()
                .map(PathBuf::from);
            listed.or_else(|| {
                b.path
                    .as_deref()
                    .or(b.about.as_deref())
                    .and_then(kawoosh_systems::fs::parent)
            })
        });
        from_buffer.unwrap_or_else(|| self.cwd.clone())
    }

    /// `:!CMD`'s terminal: `cmd` in `cwd`, kept when it ends
    /// ([`Spawned::keep`]).
    pub fn spawn_bang(&mut self, cmd: &str, cwd: &Path) -> Option<TermId> {
        let t = self.spawn_terminal(Some(cmd), Some(cwd))?;
        self.terms.spawned.entry(t).or_default().keep = Some(cwd.to_path_buf());
        Some(t)
    }

    /// [`Kawoosh::spawn_bang`] for a program and its arguments, with no
    /// shell between — whatever the user's shell quotes like — given the
    /// PATH a shell made: `:lsp install`'s `kawoosh lsp install …`.
    pub fn spawn_bang_argv(&mut self, argv: &[String], cwd: &Path) -> Option<TermId> {
        let mut envs = vec![("TERM_PROGRAM".to_string(), "kawoosh".to_string())];
        if let Some(path) = kawoosh_systems::shell_env::path() {
            envs.push(("PATH".into(), path.to_string_lossy().into_owned()));
        }
        let size = TermSize { rows: 24, cols: 80 };
        let spawned = in_a_pty(|| Terminal::spawn_argv(argv, Some(cwd), size, &envs));
        let t = self.adopt_terminal(None, spawned, None)?;
        let s = self.terms.spawned.entry(t).or_default();
        s.keep = Some(cwd.to_path_buf());
        s.argv = Some(argv.to_vec());
        Some(t)
    }

    /// Terminal `id`'s pty closed: its process is gone. A `:!`'s stays
    /// in its pane with how it ended printed under its output, as
    /// `*compile*` ends; any other's pane closes, keeping nothing.
    pub fn term_closed(&mut self, id: TermId) {
        self.domain_term_closed(id);
        let keep = self
            .terms
            .spawned
            .get(&id)
            .is_some_and(|s| s.keep.is_some());
        if keep && let Some(t) = self.terms.map.get_mut(&id) {
            let exit = t.exit();
            let status = exit
                .as_ref()
                .map_or_else(|| "ended".to_string(), ToString::to_string);
            // Back on the main screen, the mouse and the cursor let go
            // of: what a program left set is no one's now.
            let mut tail = String::new();
            if t.is_alt_screen() {
                tail.push_str("\x1b[?1049l");
            }
            tail.push_str("\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?25l");
            tail.push_str(&format!("\x1b[0m\r\n\x1b[2m[{status}]\x1b[0m"));
            t.feed(tail.as_bytes());
            t.scroll_to_bottom();
            let ok = matches!(exit, Some(kawoosh_term::Exit::Code(0)));
            self.terms.done.insert(id, exit);
            // Kept by the pane, for `r` to install again.
            if let Some(command) = self.lsp.installs.get(&id).cloned() {
                self.lsp_installed(command, ok);
            }
            return;
        }
        if let Some(t) = self.terms.map.get_mut(&id) {
            t.is_running();
        }
        let panes: Vec<PaneId> = self
            .layout
            .all_panes()
            .into_iter()
            .filter(|p| self.term_of(*p) == Some(id))
            .collect();
        for p in panes {
            self.close_gone(p);
        }
        self.terms.forget(id);
    }

    /// Terminal `id`'s output was all read (its pty closed): closed now,
    /// or once the output still to parse is.
    pub(crate) fn pty_closed(&mut self, id: TermId) {
        if self.terms.flowing.contains(&id) {
            self.terms.closing.insert(id);
        } else {
            self.term_closed(id);
        }
    }

    /// The terminals' output parsed, each in turn a [`PTY_CHUNK`] at a
    /// time, until none is left or the frame's [`PTY_BUDGET`] is spent;
    /// what is left stays in [`Terminals::flowing`] for the next frame.
    pub(crate) fn pump_ptys(&mut self) {
        let started = Instant::now();
        let mut closed = Vec::new();
        while !self.terms.flowing.is_empty() && started.elapsed() < PTY_BUDGET {
            let ids: Vec<TermId> = self.terms.flowing.iter().copied().collect();
            for id in ids {
                let Some(flow) = self.terms.flows.get_mut(&id) else {
                    self.terms.flowing.remove(&id);
                    continue;
                };
                if flow.at == flow.taken.len() {
                    flow.taken.clear();
                    flow.at = 0;
                    if !flow.output.take(&mut flow.taken) {
                        self.terms.flowing.remove(&id);
                        if self.terms.closing.remove(&id) {
                            closed.push(id);
                        }
                        continue;
                    }
                }
                let end = (flow.at + PTY_CHUNK).min(flow.taken.len());
                if let Some(t) = self.terms.map.get_mut(&id) {
                    t.feed(&flow.taken[flow.at..end]);
                }
                flow.at = end;
            }
        }
        for id in closed {
            self.term_closed(id);
        }
    }

    /// `terminal again` (`r` in a finished `:!` pane): its line run
    /// again where it ran, in a new terminal in the same pane.
    fn term_again(&mut self) {
        let pane = self.layout.focused();
        let Some(old) = self.term_of(pane) else {
            return;
        };
        let again = match self.terms.spawned.get(&old).cloned() {
            Some(Spawned {
                argv: Some(argv),
                keep: Some(cwd),
                ..
            }) => self.spawn_bang_argv(&argv, &cwd),
            Some(Spawned {
                cmd: Some(cmd),
                keep: Some(cwd),
                ..
            }) => self.spawn_bang(&cmd, &cwd),
            _ => {
                self.ed.message = "terminal again: only in a finished `:!` pane".into();
                return;
            }
        };
        let Some(t) = again else {
            return;
        };
        if let Some(command) = self.lsp.installs.remove(&old) {
            self.lsp.installs.insert(t, command);
        }
        if let Some(c) = self.layout.panes.insert(pane, Content::Terminal(t)) {
            self.drop_content(c);
        }
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
                in_a_pty(|| Terminal::spawn_argv(&argv, Some(&home), size, &[])).map(
                    |(mut t, r)| {
                        t.set_domain(name, cwd.clone());
                        (t, r)
                    },
                )
            }
            None => {
                let shell = self.ed.settings.str("terminal.shell");
                in_a_pty(|| Terminal::spawn(shell, cmd, Some(&cwd), size, &envs))
            }
        };
        self.adopt_terminal(id, spawned, cmd)
    }

    /// A program in a pty with no shell in between (`Terminal::spawn_argv`)
    /// — the ssh master of a domain, whose arguments must reach it whole
    /// whatever the user's shell quotes like.
    pub(crate) fn spawn_terminal_argv(&mut self, argv: &[String], cwd: &Path) -> Option<TermId> {
        let size = TermSize { rows: 24, cols: 80 };
        let spawned = in_a_pty(|| Terminal::spawn_argv(argv, Some(cwd), size, &[]));
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
                        ..Default::default()
                    },
                );
                let output = self.io.watch_pty(id, reader, exited);
                self.terms.flows.insert(
                    id,
                    Flow {
                        output,
                        taken: Vec::new(),
                        at: 0,
                    },
                );
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
                        self.close_gone(pane);
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
        self.layout.open(Content::Terminal(id), Place::Under);
        id
    }

    /// Where a bare `:terminal` and `:!` open (pane-placement.md
    /// Decision 3): a column of its own, or under the focused pane when
    /// `terminal.place` is `under`; the launcher's `<C-w>s t` is the
    /// per-pane way.
    pub(crate) fn terminal_place(&self) -> Place {
        self.ed
            .settings
            .str("terminal.place")
            .and_then(Place::parse)
            .filter(|p| *p != Place::Dock)
            .unwrap_or(Place::Column)
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

    /// Whether terminal `id` is raw (terminal-keys.md Decision 2): every
    /// key but the escape and a bound ⌘ chord the program's. Set by hand
    /// for the program in front, else while one `terminal.raw` names is.
    pub(crate) fn term_raw(&self, id: TermId) -> bool {
        let Some(t) = self.terms.map.get(&id) else {
            return false;
        };
        let front = t.foreground();
        let pgid = front.as_ref().map(|(g, _)| *g);
        if let Some((set_for, on)) = self.terms.raw.get(&id)
            && *set_for == pgid
        {
            return *on;
        }
        let Some((_, name)) = front else {
            return false;
        };
        match self.ed.settings.get("terminal.raw") {
            Some(kawoosh_editor::Setting::List(l)) => {
                l.iter().any(|x| x.as_str() == Some(name.as_str()))
            }
            _ => false,
        }
    }

    /// `terminal raw`: raw flipped for the focused terminal, for the
    /// program in front; `on` / `off` say which.
    fn toggle_raw(&mut self, arg: Option<&str>) {
        let Some(t) = self.focused_term() else {
            self.ed.message = "terminal raw: not a terminal pane".into();
            return;
        };
        let on = match arg {
            Some("on") => true,
            Some("off") => false,
            _ => !self.term_raw(t),
        };
        let pgid = self
            .terms
            .map
            .get(&t)
            .and_then(|t| t.foreground())
            .map(|(g, _)| g);
        self.terms.raw.insert(t, (pgid, on));
        let escape = self.term_escape_key().unwrap_or_default();
        self.ed.message = if on {
            format!("raw: every key the program's but {escape} and ⌘")
        } else {
            "raw off".into()
        };
    }

    /// The terminal's escape (`terminal.escape`, `<C-\>` by default)
    /// as a stroke's notation; none when the setting is empty or not
    /// one key.
    pub(crate) fn term_escape_key(&self) -> Option<String> {
        let s = self.ed.settings.str("terminal.escape").unwrap_or("<C-\\>");
        match parse_notation(s).as_slice() {
            [one] if one != LEADER => Some(one.clone()),
            _ => None,
        }
    }

    /// A key in a focused terminal pane: `stroke` as the keymap reads it,
    /// `press` as kui delivered it, for kitty's keyboard protocol.
    pub(crate) fn term_key(&mut self, id: TermId, stroke: KeyStroke, press: &KeyPress) {
        let note = stroke.notation();
        // No process to hear them: every key is normal mode's, as after
        // the escape — `r` again, `q` closes, `<C-S-x>` copy mode.
        if self.terms.done.contains_key(&id) {
            let keys = self.terms.escape.take().unwrap_or_default();
            self.term_escaped(id, &stroke, keys);
            return;
        }
        if let Some(keys) = self.terms.escape.take() {
            self.term_escaped(id, &stroke, keys);
            return;
        }
        if self.term_escape_key().as_deref() == Some(note.as_str()) {
            self.terms.escape = Some(Vec::new());
            return;
        }
        // The view's own keys, where a program on the whole screen
        // keeps them: shift with the page keys moves through history —
        // and not in raw, where they are the program's too.
        let alt_screen =
            self.terms.map.get(&id).is_some_and(|t| t.is_alt_screen()) || self.term_raw(id);
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
        // A program that pushed kitty's keyboard protocol hears the key
        // whole — an unbound ⌘ chord as super-modified (Decision 6).
        if self
            .terms
            .map
            .get(&id)
            .is_some_and(|t| t.keyboard_flags() != 0)
        {
            self.term_kitty(id, press, false);
            return;
        }
        let Some(t) = self.terms.map.get_mut(&id) else {
            return;
        };
        // A ⌘ chord bound to nothing is nothing to the shell, not its
        // letter: a pty has no use for ⌘ (`Kawoosh::pane_chord`) — but
        // for a Mac's line-editing keys, which a shell hears as
        // readline's (`^U` for ⌘⌫).
        if stroke.sup {
            if let Some(bytes) =
                encode_super_key(&stroke.code, stroke.ctrl, stroke.alt, stroke.shift)
            {
                t.scroll_to_bottom();
                t.input(bytes);
            }
            return;
        }
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

    /// A release, or a modifier key pressed alone, on terminal `id`'s
    /// pane: the program's under kitty's keyboard protocol — a release
    /// when it asked for event types and its press was sent, a modifier
    /// key when it asked for every key — and nothing's otherwise. Never
    /// while the escape is open, whose keys are kawoosh's.
    pub(crate) fn term_key_aside(&mut self, id: TermId, k: &KeyPress, up: bool) {
        if self.terms.done.contains_key(&id) {
            return;
        }
        if up {
            let Some(i) = self.terms.held.iter().position(|h| h.same_key(k)) else {
                return;
            };
            self.terms.held.remove(i);
        } else if self.terms.escape.is_some() {
            return;
        }
        self.term_kitty(id, k, up);
    }

    /// `k` to terminal `id`'s program in kitty's encoding, for the flags
    /// it pushed; a press sent is held until its release comes.
    fn term_kitty(&mut self, id: TermId, k: &KeyPress, up: bool) {
        use kawoosh_term::kitty;
        let Some(t) = self.terms.map.get_mut(&id) else {
            return;
        };
        let (code, physical) = (k.code.name(), k.physical.name());
        let input = kitty::KeyInput {
            code: &code,
            physical: &physical,
            location: match k.location {
                kui_native::KeyLocation::Standard => kitty::Location::Standard,
                kui_native::KeyLocation::Left => kitty::Location::Left,
                kui_native::KeyLocation::Right => kitty::Location::Right,
                kui_native::KeyLocation::Numpad => kitty::Location::Numpad,
            },
            text: k.text.as_deref(),
            shift: k.mods.shift,
            alt: k.mods.alt,
            ctrl: k.mods.ctrl,
            sup: k.mods.super_key,
            caps_lock: k.locks.caps,
            num_lock: k.locks.num,
            action: if up {
                kitty::Action::Release
            } else if k.repeat {
                kitty::Action::Repeat
            } else {
                kitty::Action::Press
            },
        };
        if let Some(bytes) = kitty::encode(&input, t.keyboard_flags(), t.app_cursor_keys()) {
            if !up {
                t.scroll_to_bottom();
            }
            t.input(&bytes);
        }
        if !up && !self.terms.held.iter().any(|h| h.same_key(k)) {
            self.terms.held.push(k.clone());
        }
    }

    /// A key after the terminal's escape (terminal-keys.md Decision 1):
    /// normal mode's keys, the sequence looked up as it grows — a
    /// binding runs, a prefix waits with the which-key open on it,
    /// anything else is said to be unbound. First after the escape,
    /// `<C-n>` is copy mode (vim's `<C-\><C-n>`), `:` the command line,
    /// the escape again the key itself to the pty, `<Esc>` nothing.
    fn term_escaped(&mut self, id: TermId, stroke: &KeyStroke, mut keys: Vec<String>) {
        let note = stroke.notation();
        if note == "<Esc>" {
            return;
        }
        if keys.is_empty() {
            match note.as_str() {
                "<C-n>" => return self.scrollback_to_buffer(id),
                ":" => return self.open_cmdline(),
                _ if self.term_escape_key().as_deref() == Some(note.as_str()) => {
                    if let Some(t) = self.terms.map.get_mut(&id)
                        && let Some(bytes) = encode_key(
                            &stroke.code,
                            stroke.text.as_deref(),
                            stroke.ctrl,
                            stroke.alt,
                            stroke.shift,
                            t.app_cursor_keys(),
                        )
                    {
                        t.scroll_to_bottom();
                        t.input(&bytes);
                    }
                    return;
                }
                _ => {}
            }
        }
        keys.push(note);
        self.ed.sync_settings();
        // The terminal pane's places (`terminal`, `exited`) and the
        // global map: another pane's local keys are not looked at
        // (local-maps.md — the launcher's `z` once answered `<C-\>z`).
        self.sync_facts();
        let v = self.focused_view().unwrap_or_else(|| self.ed.pane_view());
        match self.ed.lookup_keys(v, Mode::Normal, &keys) {
            Lookup::Exact(bs) => {
                // As in an editor pane: a binding that cannot run here
                // does not shadow the longer ones under it, and one
                // gated off here by its own `when` is as good as
                // unbound.
                if self.ed.pick_binding(v, &bs).is_err() {
                    if self.ed.keys_deeper(v, Mode::Normal, &keys) {
                        self.terms.escape = Some(keys);
                        return;
                    }
                    if self.ed.gated_off(v, &bs) {
                        self.ed.message = format!("{}: not bound", keys.concat());
                        return;
                    }
                }
                self.run_bindings(&bs);
            }
            Lookup::Prefix => self.terms.escape = Some(keys),
            Lookup::None => self.ed.message = format!("{}: not bound", keys.concat()),
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
                self.layout.open(Content::Editor(v), Place::Under);
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

    /// The link at `(row, col)` of terminal `id`'s screen: the one a
    /// program printed there on purpose (OSC 8) first, else one found in
    /// the text of the line the row is part of — the rows a long line
    /// wrapped onto joined, so a URL across the edge is whole. A path in the text is looked for in the
    /// terminal's directory, then the working directory.
    fn term_link(&self, id: TermId, row: usize, col: usize) -> Option<TermLink> {
        let t = self.terms.map.get(&id)?;
        let mut bases = vec![t.cwd().unwrap_or_else(|| self.cwd.clone())];
        if !bases.contains(&self.cwd) {
            bases.push(self.cwd.clone());
        }
        if let Some(h) = t.hyperlink_at(row, col) {
            let target = if h.uri.starts_with("file:") {
                match t.file_link(&h.uri) {
                    Some((path, line)) => Target::Path {
                        path: path.display().to_string(),
                        line,
                        col: None,
                        anchor: None,
                    },
                    None => Target::Elsewhere(h.uri.clone()),
                }
            } else {
                Target::Url(h.uri.clone())
            };
            return Some(TermLink {
                target,
                rows: h.rows,
                uri: Some(h.uri),
                bases,
            });
        }
        let line = t.wrapped_line(row);
        let text = &line.text;
        let at = line
            .char_at(row, col)
            .and_then(|n| text.char_indices().nth(n))
            .map_or(text.len(), |(i, _)| i);
        let link = crate::links::link_at(text, at)?;
        let first = text[..link.span.start].chars().count();
        let chars = first..first + text[link.span.clone()].chars().count();
        Some(TermLink {
            target: link.target,
            rows: line.rows_of(chars, t.size().rows as usize),
            uri: None,
            bases,
        })
    }

    /// What a ⌘-click at `(row, col)` of terminal `id`'s screen would
    /// open, for the hover to underline: the cells of a program's link
    /// (OSC 8) and its address, since its text need not be it; or of a
    /// URL, or a path that names a file or a directory there, in the
    /// text.
    pub(crate) fn location_cells(
        &self,
        id: TermId,
        row: usize,
        col: usize,
    ) -> Option<(LinkCells, Option<String>)> {
        let link = self.term_link(id, row, col)?;
        match &link.target {
            _ if link.uri.is_some() => Some((link.rows, link.uri)),
            Target::Url(_) | Target::Elsewhere(_) => Some((link.rows, None)),
            Target::Path { path, .. } => {
                crate::links::resolve(path, &link.bases).map(|_| (link.rows, None))
            }
        }
    }

    /// Opens the link at `(row, col)` of terminal `id`'s screen — a URL
    /// in the OS, a path in an editor pane at its line: `gx` across the
    /// terminal/editor boundary (Decision 5c).
    pub fn open_location_at(&mut self, id: TermId, row: usize, col: usize) -> bool {
        let Some(link) = self.term_link(id, row, col) else {
            self.ed.message = "no link under the pointer".into();
            return false;
        };
        self.follow_link(link.target, &link.bases)
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
                    self.layout.open(Content::Editor(v), Place::Column);
                }
            }
        }
        self.open_file(path);
        if let (Some(v), Some(ln)) = (self.focused_view(), line) {
            // A place landed: a jump however near (jumps.md Decision 2);
            // in a file still arriving, once its text has.
            self.ed.jumping = true;
            let col = col.map(|c| c.saturating_sub(1));
            self.place_at_line(v, ln.max(1) - 1, col);
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("terminal")
                .alias(&["term"])
                .args(Args::rest(&[ArgKind::Text]))
                .doc("a terminal in a column of its own (`terminal.place`), running CMD or the shell"),
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
                    k.fill_or_open(k.terminal_place(), Content::Terminal(t));
                }
            },
        ),
        cmd(
            Spec::new("terminal here")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("a terminal where the pane in front is (`<C-w>.`): the file's directory, the listing's, the shell's"),
            |k, ctx| {
                let cmd = (!ctx.args.is_empty()).then(|| ctx.args.join(" "));
                let cwd = k.here_dir();
                if let Some(t) = k.spawn_terminal(cmd.as_deref(), Some(&cwd)) {
                    k.fill_or_open(k.terminal_place(), Content::Terminal(t));
                }
            },
        ),
        cmd(
            Spec::new("shell")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("`:!CMD`: CMD in a terminal of its own (`terminal.place`), `%` the file (`%:h` its directory, `%:t` its name), quoted — a prompt it asks is answered there"),
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
                if let Some(t) = k.spawn_bang(&cmd, &cwd) {
                    k.fill_or_open(k.terminal_place(), Content::Terminal(t));
                }
            },
        ),
        cmd(
            Spec::new("terminal again")
                .when(&["exited"])
                .doc("the line a finished `:!` pane ran, run again in its place (`r` there)"),
            |k, _| k.term_again(),
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
                    k.ed.message = "scrollback: only in a terminal pane".into();
                }
            },
        ),
        cmd(
            Spec::new("terminal raw")
                .when(&["terminal"])
                .args(Args::new(&[ArgKind::Text]))
                .doc("every key but the escape and ⌘ to the terminal's program, or back; `on` / `off`, bare flips it (`<C-\\>r`)"),
            |k, ctx| k.toggle_raw(ctx.args.first().map(String::as_str)),
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
