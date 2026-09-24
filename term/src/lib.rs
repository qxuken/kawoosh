//! `term`: `alacritty_terminal` and `portable-pty` behind a facade whose
//! output is a kui `CellGrid` (kui.md D4). Bytes in from the pty reader,
//! key bytes out to the pty, a screenful of cells per frame; scrollback
//! as text for the materialise-into-a-buffer command.

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::*;
use anyhow::{Context as _, Result};
use kui_core::cells::{Cell, CursorShape as CellCursor, flags};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub rows: u16,
    pub cols: u16,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.cols as usize
    }
}

struct Proxy {
    tx: Sender<Event>,
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(event);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// The screen, ready for `ui.cells`: the app owns the `Vec` for a frame.
pub struct Screen {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Cell>,
    pub cursor: Option<(usize, usize, CellCursor)>,
    /// The absolute session line of the top row.
    pub origin_line: u64,
}

/// Default colours a screen is painted with, from the theme
/// ([`Terminal::set_palette`]), and which base they are: what a
/// program's `OSC 10` / `11` question is answered with, and a flip of
/// `dark` is what a program that set mode 2031 is told about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub fg: u32,
    pub bg: u32,
    /// The 16 ANSI colours as 0xRRGGBBAA.
    pub ansi: [u32; 16],
    pub dark: bool,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            fg: 0xE6E6E6FF,
            bg: 0x1D1F21FF,
            ansi: ANSI,
            dark: true,
        }
    }
}

/// What the parser's hooks keep beside alacritty's `Term`
/// ([`Hooked`]): the modes alacritty does not know, and the replies
/// they owe the process.
#[derive(Default)]
struct Modes {
    /// DEC private mode 2031 (contour's, in kitty and neovim 0.10+): the
    /// program wants `CSI ? 997 ; 1 n` (dark) / `; 2 n` (light) when the
    /// appearance flips.
    report_appearance: bool,
    /// Bytes to the process, written once the parser's pass is over.
    replies: Vec<u8>,
    /// Lines the primary screen has scrolled into history since the
    /// start — counted through history's growth, and at its cap by a
    /// line fed at the bottom — so a line has a number that outlives
    /// its place on screen: grid line `l` is `scrolled + l`.
    scrolled: u64,
    /// The scrollback's cap, what `scrolled` counts past at the cap.
    history_max: usize,
}

/// Mode 2031, and the report a program under it gets on a flip, in
/// contour's spelling. (Its `CSI ? 996 n` query is not answered: vte
/// drops a DSR with the private prefix before any handler sees it, and
/// `OSC 11 ; ?` asks the same thing.)
const MODE_APPEARANCE: u16 = 2031;

fn appearance_report(dark: bool) -> String {
    format!("\x1b[?997;{}n", if dark { 1 } else { 2 })
}

/// One command as the shell marked it (OSC 133, FinalTerm's marks —
/// nushell's `shell_integration.osc133`, and the `precmd` lines
/// kawoosh's shell snippets add): where its prompt began (`A`), where
/// the typed line began (`B`), where its output began (`C`) and where
/// it ended with its status (`D`), each a line number as
/// [`Terminal::line_of`] counts them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    pub prompt: u64,
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub end: Option<u64>,
    pub status: Option<i32>,
}

/// The OSC sequences the parser does not know and kawoosh wants — 7
/// (the shell's cwd) and 133 (the prompt marks) — picked out of the
/// byte stream in front of it, across reads.
#[derive(Default)]
struct OscScan {
    state: Scan,
    buf: Vec<u8>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Scan {
    #[default]
    Ground,
    Esc,
    Osc,
    OscEsc,
}

impl OscScan {
    /// The longest payload kept: a path, a mark with its options.
    const MAX: usize = 4096;

    /// One byte on; the payload of an OSC that ended on it, when one
    /// did.
    fn step(&mut self, b: u8) -> Option<Vec<u8>> {
        match (self.state, b) {
            (Scan::Ground, 0x1b) => self.state = Scan::Esc,
            (Scan::Ground, _) => {}
            (Scan::Esc, b']') => {
                self.state = Scan::Osc;
                self.buf.clear();
            }
            (Scan::Esc, 0x1b) => {}
            (Scan::Esc, _) => self.state = Scan::Ground,
            (Scan::Osc, 0x07) => {
                self.state = Scan::Ground;
                return Some(std::mem::take(&mut self.buf));
            }
            (Scan::Osc, 0x1b) => self.state = Scan::OscEsc,
            // CAN and SUB abandon a sequence.
            (Scan::Osc, 0x18 | 0x1a) => self.state = Scan::Ground,
            (Scan::Osc, _) => {
                if self.buf.len() < Self::MAX {
                    self.buf.push(b);
                }
            }
            (Scan::OscEsc, b'\\') => {
                self.state = Scan::Ground;
                return Some(std::mem::take(&mut self.buf));
            }
            // An escape that starts something else ends the OSC unread.
            (Scan::OscEsc, b']') => {
                self.state = Scan::Osc;
                self.buf.clear();
            }
            (Scan::OscEsc, _) => self.state = Scan::Ground,
        }
        None
    }
}

/// The most commands a terminal remembers the marks of.
const COMMANDS_MAX: usize = 2000;

pub struct Terminal {
    term: Term<Proxy>,
    parser: Processor,
    modes: Modes,
    scan: OscScan,
    /// The commands the shell marked, oldest first.
    commands: Vec<Command>,
    /// Whether anything was sent to the process since the shell last
    /// drew a prompt (an OSC 133 `A` or `B`): what makes a prompt not
    /// empty, since the marks do not say where a prompt's own text
    /// ends and the typed line begins.
    typed: bool,
    /// The colours the screen is painted with, as last set.
    palette: Palette,
    pty: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    events: Receiver<Event>,
    size: TermSize,
    pub title: String,
    /// Where the terminal was started.
    spawned_in: Option<std::path::PathBuf>,
    /// The domain the shell runs on, when it is a host's
    /// ([`Terminal::set_domain`]): its reports are that host's paths.
    domain: Option<String>,
    /// The directory the shell last said it is in (OSC 7).
    reported_cwd: Option<std::path::PathBuf>,
    pub bell: bool,
    exited: bool,
    /// Bytes a headless terminal would have sent to its process, for
    /// tests (`Terminal::take_sent`).
    sent: Vec<u8>,
}

impl Terminal {
    /// Spawns `$SHELL` (or `cmd`) in a pty. The reader is handed back for
    /// an io thread to pump into [`Terminal::feed`].
    pub fn spawn(
        cmd: Option<&str>,
        cwd: Option<&std::path::Path>,
        size: TermSize,
        envs: &[(String, String)],
    ) -> Result<(Self, Box<dyn Read + Send>)> {
        // `$SHELL` where there is one (an MSYS bash sets it on Windows
        // too); else `/bin/sh`, or `%ComSpec%` on Windows, whose flags
        // are its own.
        let shell = std::env::var("SHELL").ok();
        let cmd_exe = cfg!(windows) && shell.is_none();
        let shell = shell.unwrap_or_else(|| {
            if cmd_exe {
                std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())
            } else {
                "/bin/sh".into()
            }
        });
        let mut builder = CommandBuilder::new(&shell);
        match (cmd, cmd_exe) {
            (Some(c), false) => {
                builder.args(["-lc", c]);
            }
            (Some(c), true) => {
                builder.args(["/c", c]);
            }
            (None, false) => {
                builder.arg("-l");
            }
            (None, true) => {}
        }
        Self::spawn_with(builder, cwd, size, envs)
    }

    /// Spawns `argv` in a pty as it is, no shell in between: a program
    /// whose arguments must reach it whole whatever the user's shell
    /// quotes like — an `ssh` to a host (docs/design/domains.md).
    pub fn spawn_argv(
        argv: &[String],
        cwd: Option<&std::path::Path>,
        size: TermSize,
        envs: &[(String, String)],
    ) -> Result<(Self, Box<dyn Read + Send>)> {
        let mut builder = CommandBuilder::new(argv.first().context("an empty command")?);
        builder.args(&argv[1..]);
        Self::spawn_with(builder, cwd, size, envs)
    }

    fn spawn_with(
        mut builder: CommandBuilder,
        cwd: Option<&std::path::Path>,
        size: TermSize,
        envs: &[(String, String)],
    ) -> Result<(Self, Box<dyn Read + Send>)> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("opening pty")?;
        builder.env("TERM", "xterm-256color");
        builder.env("COLORTERM", "truecolor");
        for (k, v) in envs {
            builder.env(k, v);
        }
        if let Some(cwd) = cwd {
            builder.cwd(cwd);
        }
        let child = pair
            .slave
            .spawn_command(builder)
            .context("spawning shell in pty")?;
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .context("cloning pty reader")?;
        let writer = pair.master.take_writer().context("taking pty writer")?;
        let (tx, events) = channel();
        let term = Term::new(config(HISTORY), &size, Proxy { tx });
        Ok((
            Self {
                term,
                parser: Processor::new(),
                modes: Modes {
                    history_max: HISTORY,
                    ..Modes::default()
                },
                scan: OscScan::default(),
                commands: Vec::new(),
                typed: false,
                palette: Palette::default(),
                pty: Some(pair.master),
                writer: Some(writer),
                child: Some(child),
                events,
                size,
                title: String::new(),
                spawned_in: cwd.map(Into::into),
                domain: None,
                reported_cwd: None,
                bell: false,
                exited: false,
                sent: Vec::new(),
            },
            reader,
        ))
    }

    /// A terminal with no process behind it: tests feed it bytes.
    pub fn headless(size: TermSize) -> Self {
        let (tx, events) = channel();
        Self {
            term: Term::new(config(HISTORY), &size, Proxy { tx }),
            parser: Processor::new(),
            modes: Modes {
                history_max: HISTORY,
                ..Modes::default()
            },
            scan: OscScan::default(),
            commands: Vec::new(),
            typed: false,
            palette: Palette::default(),
            pty: None,
            writer: None,
            child: None,
            events,
            size,
            title: String::new(),
            spawned_in: None,
            domain: None,
            reported_cwd: None,
            bell: false,
            exited: false,
            sent: Vec::new(),
        }
    }

    /// What a headless terminal was asked to send since the last take.
    pub fn take_sent(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.sent)
    }

    pub fn size(&self) -> TermSize {
        self.size
    }

    /// Whether the child is still running; a headless terminal never is.
    pub fn is_running(&mut self) -> bool {
        if self.exited {
            return false;
        }
        match &mut self.child {
            Some(child) => {
                let running = matches!(child.try_wait(), Ok(None));
                if !running {
                    self.exited = true;
                }
                running
            }
            None => false,
        }
    }

    /// Bytes from the pty: parsed, replies (a DA answer, a cursor report)
    /// written back, and the terminal's own events read out. The OSCs
    /// the parser drops — 7 and 133 — are read here, the bytes before
    /// each parsed first so a mark lands on the line the cursor is on
    /// when it arrives.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut from = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if let Some(osc) = self.scan.step(b) {
                self.advance(&bytes[from..=i]);
                from = i + 1;
                self.on_osc(&osc);
            }
        }
        self.advance(&bytes[from..]);
        let replies = std::mem::take(&mut self.modes.replies);
        if !replies.is_empty() {
            self.send(&replies);
        }
        self.drain_events();
    }

    fn advance(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.parser.advance(
            &mut Hooked {
                term: &mut self.term,
                modes: &mut self.modes,
            },
            bytes,
        );
    }

    /// An OSC the parser does not read: `7;file://host/path`, the
    /// shell's directory, and `133;X[;…]`, a prompt mark.
    fn on_osc(&mut self, payload: &[u8]) {
        let Ok(s) = std::str::from_utf8(payload) else {
            return;
        };
        if let Some(url) = s.strip_prefix("7;") {
            if let Some(p) = file_url_path(url, self.domain.is_some()) {
                self.reported_cwd = Some(p);
            }
        } else if let Some(mark) = s.strip_prefix("133;") {
            let line = self.cursor_line();
            let mut parts = mark.split(';');
            match parts.next() {
                Some("A") => {
                    self.typed = false;
                    // A prompt drawn again over the same line (a
                    // resize, a `clear`) is the same command's.
                    if self
                        .commands
                        .last()
                        .is_some_and(|c| c.prompt == line && c.end.is_none())
                    {
                        return;
                    }
                    self.commands.push(Command {
                        prompt: line,
                        ..Command::default()
                    });
                    if self.commands.len() > COMMANDS_MAX {
                        self.commands.remove(0);
                    }
                }
                Some("B") => {
                    self.typed = false;
                    if let Some(c) = self.commands.last_mut() {
                        c.input = Some(line);
                    }
                }
                Some("C") => {
                    if let Some(c) = self.commands.last_mut() {
                        c.output = Some(line);
                    }
                }
                Some("D") => {
                    if let Some(c) = self.commands.last_mut()
                        && c.end.is_none()
                    {
                        c.end = Some(line);
                        c.status = parts.next().and_then(|n| n.parse().ok());
                    }
                }
                _ => {}
            }
        }
    }

    /// The number of the line the cursor is on (grid line `l` is
    /// `scrolled + l`), which history scrolling past does not change.
    fn cursor_line(&self) -> u64 {
        let l = self.term.grid().cursor.point.line.0;
        (self.modes.scrolled as i64 + l as i64).max(0) as u64
    }

    /// The number of the line at grid line `l`.
    pub fn line_of(&self, l: i32) -> u64 {
        (self.modes.scrolled as i64 + l as i64).max(0) as u64
    }

    /// The oldest line number history still holds.
    fn oldest_line(&self) -> u64 {
        self.line_of(-(self.term.grid().history_size() as i32))
    }

    /// The commands the shell marked, oldest first, those whose prompt
    /// has left history dropped.
    pub fn commands(&self) -> Vec<Command> {
        let oldest = self.oldest_line();
        self.commands
            .iter()
            .filter(|c| c.prompt >= oldest)
            .cloned()
            .collect()
    }

    /// The last finished command's output as text — the lines from its
    /// `C` mark to its `D` mark, trailing blank lines trimmed — or
    /// None when no command has been marked through to its end.
    pub fn last_output(&self) -> Option<String> {
        let oldest = self.oldest_line();
        let c = self
            .commands
            .iter()
            .rev()
            .find(|c| c.output.is_some() && c.end.is_some())?;
        let (from, to) = (c.output?.max(oldest), c.end?);
        let mut lines: Vec<String> = (from..to)
            .map(|n| self.line_text(n as i64 - self.modes.scrolled as i64))
            .collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        Some(lines.join("\n"))
    }

    /// The view scrolled so the prompt above the top row (`back`), or
    /// the one below it, is at the top — as far as the bottom allows —
    /// and past the last, back to the bottom; false when there is none
    /// that way.
    pub fn jump_prompt(&mut self, back: bool) -> bool {
        let top = self.line_of(-(self.display_offset() as i32));
        let oldest = self.oldest_line();
        let prompts: Vec<u64> = self
            .commands
            .iter()
            .map(|c| c.prompt)
            .filter(|p| *p >= oldest)
            .collect();
        // The offset that puts line `p` at the top, as far as history
        // and the bottom allow; the first prompt that way whose offset
        // moves the view.
        let now = self.display_offset() as i64;
        let offset_of = |p: u64| -> i64 {
            (self.modes.scrolled as i64 - p as i64).clamp(0, self.history_size() as i64)
        };
        let target = if back {
            prompts
                .iter()
                .rev()
                .filter(|p| **p < top)
                .map(|p| offset_of(*p))
                .find(|o| *o > now)
        } else {
            prompts
                .iter()
                .filter(|p| **p > top)
                .map(|p| offset_of(*p))
                .find(|o| *o < now)
        };
        let Some(offset) = target else {
            if !back && now > 0 {
                self.scroll_to_bottom();
                return true;
            }
            return false;
        };
        self.scroll((offset - now) as i32);
        true
    }

    /// The text of grid line `l` (negative in history), trailing blanks
    /// trimmed.
    fn line_text(&self, l: i64) -> String {
        let grid = self.term.grid();
        if l < -(grid.history_size() as i64) || l >= grid.screen_lines() as i64 {
            return String::new();
        }
        let mut s = String::new();
        for col in 0..grid.columns() {
            let cell = &grid[Line(l as i32)][Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN)
            {
                continue;
            }
            s.push(cell.c);
        }
        s.trim_end().to_string()
    }

    /// Where the shell is: the directory it last reported (OSC 7), else
    /// its process's own (`proc_pidinfo` on macOS, `/proc` on Linux),
    /// else where it was started.
    /// The shell is a host's (docs/design/domains.md): `start` is where
    /// it was started, spelled on its domain (`box:/…`), and what its
    /// shell reports (OSC 7, whatever host it names) is a path on `name`.
    pub fn set_domain(&mut self, name: &str, start: std::path::PathBuf) {
        self.domain = Some(name.to_string());
        self.spawned_in = Some(start);
    }

    pub fn cwd(&self) -> Option<std::path::PathBuf> {
        if let Some(d) = &self.domain {
            return self
                .reported_cwd
                .as_ref()
                .map(|p| format!("{d}:{}", p.display()).into())
                .or_else(|| self.spawned_in.clone());
        }
        // A reported directory that is not one here — a shell over ssh
        // that reported its host's, and exited — gives way to the
        // process's own.
        self.reported_cwd
            .clone()
            .filter(|p| p.is_dir())
            .or_else(|| self.process_cwd())
            .or_else(|| self.spawned_in.clone())
    }

    /// Whether the program on the pty is asking for a password: echo
    /// off with the line discipline still canonical — `sudo`, `ssh`,
    /// `gpg` at their prompt; iTerm2's rule for its key icon. A
    /// full-screen program turns canonical mode off too, so it is not
    /// one. Asked of the pty's termios each time (one syscall).
    #[cfg(unix)]
    pub fn password_prompt(&self) -> bool {
        let Some(fd) = self.pty.as_ref().and_then(|p| p.as_raw_fd()) else {
            return false;
        };
        // SAFETY: `termios` is plain data; `tcgetattr` fills it or fails.
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut t) } != 0 {
            return false;
        }
        t.c_lflag & libc::ECHO == 0 && t.c_lflag & libc::ICANON != 0
    }

    #[cfg(not(unix))]
    pub fn password_prompt(&self) -> bool {
        false
    }

    /// The shell process's working directory, asked of the system.
    pub fn process_cwd(&self) -> Option<std::path::PathBuf> {
        let pid = self.child.as_ref()?.process_id()?;
        pid_cwd(pid)
    }

    /// How many lines of history the terminal keeps; lines past a
    /// smaller cap go at once.
    pub fn set_scrollback(&mut self, lines: usize) {
        if lines == self.modes.history_max {
            return;
        }
        self.modes.history_max = lines;
        self.term.set_options(config(lines));
    }

    pub fn scrollback_cap(&self) -> usize {
        self.modes.history_max
    }

    /// The colours the screen is painted with, from the theme: kept for
    /// the next [`Terminal::screen`] and for a program's colour question.
    /// A flip of the base tells a program that asked (mode 2031).
    pub fn set_palette(&mut self, pal: Palette) {
        let flipped = pal.dark != self.palette.dark;
        self.palette = pal;
        if flipped && self.modes.report_appearance {
            self.send(appearance_report(pal.dark).as_bytes());
        }
    }

    pub fn palette(&self) -> Palette {
        self.palette
    }

    /// Whether the program asked to be told when the appearance flips.
    pub fn reports_appearance(&self) -> bool {
        self.modes.report_appearance
    }

    /// The colour a program's `OSC 4` / `10` / `11` / `12` question is
    /// about: what the program set itself, else the palette's.
    fn query_color(&self, index: usize) -> Rgb {
        if index <= NamedColor::DimForeground as usize
            && let Some(c) = self.term.colors()[index]
        {
            return c;
        }
        let pal = &self.palette;
        let hex = match index {
            i if i == NamedColor::Foreground as usize => pal.fg,
            i if i == NamedColor::Background as usize => pal.bg,
            i if i == NamedColor::Cursor as usize => pal.fg,
            i if i < 256 => indexed(i as u8, pal),
            _ => pal.fg,
        };
        Rgb {
            r: (hex >> 24) as u8,
            g: (hex >> 16) as u8,
            b: (hex >> 8) as u8,
        }
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                Event::PtyWrite(text) => self.send(text.as_bytes()),
                // `OSC 10 ; ?` and its kin: answered with the pane's
                // colours, so a neovim or a shell that asks how dark the
                // background is gets an answer rather than a timeout.
                Event::ColorRequest(index, format) => {
                    let reply = format(self.query_color(index));
                    self.send(reply.as_bytes());
                }
                Event::Title(t) => self.title = t,
                Event::ResetTitle => self.title.clear(),
                Event::Bell => self.bell = true,
                Event::ChildExit(_) => self.exited = true,
                _ => {}
            }
        }
    }

    /// Whether the shell sits at an empty prompt: the last command it
    /// marked (OSC 133) has a prompt and no output yet, nothing was
    /// sent to it since the prompt was drawn, and no program has the
    /// whole screen. False for a shell that marks nothing — there is
    /// no telling.
    pub fn at_empty_prompt(&self) -> bool {
        !self.typed
            && !self.is_alt_screen()
            && self
                .commands
                .last()
                .is_some_and(|c| c.output.is_none() && c.end.is_none())
    }

    /// Bytes to the process from the user — a key, a paste, the mouse.
    pub fn input(&mut self, bytes: &[u8]) {
        self.typed = true;
        self.send(bytes);
    }

    /// Bytes to the process: the user's, or the terminal's own answers
    /// (a colour, a report), which are no typing.
    fn send(&mut self, bytes: &[u8]) {
        match &mut self.writer {
            Some(writer) => {
                let _ = writer.write_all(bytes);
                let _ = writer.flush();
            }
            None => self.sent.extend_from_slice(bytes),
        }
    }

    /// Pasted text, bracketed when the program asked for it.
    pub fn paste(&mut self, text: &str) {
        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            self.input(b"\x1b[200~");
            self.input(text.as_bytes());
            self.input(b"\x1b[201~");
        } else {
            self.input(text.replace('\n', "\r").as_bytes());
        }
    }

    pub fn resize(&mut self, size: TermSize) {
        if size == self.size || size.rows == 0 || size.cols == 0 {
            return;
        }
        self.size = size;
        if let Some(pty) = &self.pty {
            let _ = pty.resize(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        // Shrunk, the screen's top lines go into history; grown, lines
        // come back out of it: a line keeps its number (`scrolled + l`)
        // through either. (Past the cap, lines dropped off history's
        // far end are not seen here.)
        let alt = self.term.mode().contains(TermMode::ALT_SCREEN);
        let before = self.term.grid().history_size() as i64;
        self.term.resize(size);
        if !alt {
            let after = self.term.grid().history_size() as i64;
            self.modes.scrolled = (self.modes.scrolled as i64 + after - before).max(0) as u64;
        }
    }

    /// Scrolls the viewport `lines` into history (positive = older).
    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn is_alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    pub fn app_cursor_keys(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    /// Whether the program asked for mouse reports (any of the click,
    /// drag or motion modes).
    pub fn wants_mouse(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// Whether button-drag motion is reported (modes 1002 / 1003).
    pub fn wants_drag(&self) -> bool {
        self.term
            .mode()
            .intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
    }

    /// A mouse report to the program, in the encoding it asked for (SGR
    /// when it can, the X10 bytes otherwise). `button`: 0 left, 1 middle,
    /// 2 right, 64 wheel up, 65 wheel down; `col`/`row` 0-based.
    pub fn mouse(
        &mut self,
        button: u8,
        action: MouseAction,
        col: usize,
        row: usize,
        mods: (bool, bool, bool),
    ) {
        if !self.wants_mouse() {
            return;
        }
        let (shift, alt, ctrl) = mods;
        let mut cb = button as u32;
        if action == MouseAction::Motion {
            cb += 32;
        }
        if shift {
            cb += 4;
        }
        if alt {
            cb += 8;
        }
        if ctrl {
            cb += 16;
        }
        let seq = if self.term.mode().contains(TermMode::SGR_MOUSE) {
            let fin = if action == MouseAction::Release {
                'm'
            } else {
                'M'
            };
            format!("\x1b[<{cb};{};{}{fin}", col + 1, row + 1)
        } else {
            let cb = if action == MouseAction::Release {
                3 + 32
            } else {
                cb + 32
            };
            let enc = |v: usize| (v + 1 + 32).min(255) as u8 as char;
            format!("\x1b[M{}{}{}", cb as u8 as char, enc(col), enc(row))
        };
        self.input(seq.as_bytes());
    }

    /// The wheel over a full-screen program without mouse reporting:
    /// alacritty's "alternate scroll" — arrow keys, three a notch.
    pub fn wheel_as_arrows(&mut self, lines: i32) {
        if !self.term.mode().contains(TermMode::ALTERNATE_SCROLL) || !self.is_alt_screen() {
            return;
        }
        let key: &[u8] = if lines > 0 { b"\x1b[B" } else { b"\x1b[A" };
        let key = if self.app_cursor_keys() {
            if lines > 0 {
                b"\x1bOB".as_slice()
            } else {
                b"\x1bOA".as_slice()
            }
        } else {
            key
        };
        for _ in 0..lines.unsigned_abs() {
            self.input(key);
        }
    }

    /// How far into history the view is scrolled.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    /// The screen as cells, painted with the palette last set.
    pub fn screen(&self) -> Screen {
        let pal = &self.palette;
        let grid = self.term.grid();
        let rows = self.size.rows as usize;
        let cols = self.size.cols as usize;
        let mut cells = vec![Cell::new(' ', pal.fg, 0); rows * cols];
        let display_offset = grid.display_offset() as i32;
        for indexed in grid.display_iter() {
            let row = indexed.point.line.0 + display_offset;
            let Ok(row) = usize::try_from(row) else {
                continue;
            };
            let col = indexed.point.column.0;
            if row >= rows || col >= cols {
                continue;
            }
            let cell = &*indexed;
            if cell
                .flags
                .intersects(Flags::HIDDEN | Flags::WIDE_CHAR_SPACER)
            {
                continue;
            }
            let mut fg = color(cell.fg, pal, pal.fg);
            let mut bg = match cell.bg {
                Color::Named(NamedColor::Background) => 0,
                other => color(other, pal, pal.bg),
            };
            if cell.flags.contains(Flags::INVERSE) {
                let old = fg;
                fg = if bg == 0 { pal.bg } else { bg };
                bg = old;
            }
            if cell.flags.contains(Flags::DIM) {
                fg = dim(fg);
            }
            let mut f = 0u8;
            if cell.flags.contains(Flags::BOLD) {
                f |= flags::BOLD;
            }
            if cell.flags.contains(Flags::ITALIC) {
                f |= flags::ITALIC;
            }
            if cell.flags.contains(Flags::STRIKEOUT) {
                f |= flags::STRIKETHROUGH;
            }
            if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                f |= flags::UNDERLINE;
                if cell.flags.contains(Flags::UNDERCURL) {
                    f |= flags::WAVY;
                } else if cell.flags.contains(Flags::DOTTED_UNDERLINE) {
                    f |= flags::DOTTED;
                }
            }
            if cell.flags.contains(Flags::WIDE_CHAR) {
                f |= flags::WIDE;
            }
            let mut out = Cell::new(cell.c, fg, bg).with(f);
            if let Some(ul) = cell.underline_color() {
                out.ul = color(ul, pal, fg);
            }
            cells[row * cols + col] = out;
        }
        let cursor = if self.term.mode().contains(TermMode::SHOW_CURSOR) {
            let point = grid.cursor.point;
            let row = point.line.0 + display_offset;
            usize::try_from(row).ok().filter(|r| *r < rows).map(|r| {
                let shape = match self.term.cursor_style().shape {
                    CursorShape::Beam => CellCursor::Bar,
                    CursorShape::Underline => CellCursor::Underline,
                    _ => CellCursor::Block,
                };
                (r, point.column.0.min(cols - 1), shape)
            })
        } else {
            None
        };
        Screen {
            rows,
            cols,
            cells,
            cursor,
            origin_line: (grid.history_size() - grid.display_offset()) as u64,
        }
    }

    /// The whole scrollback plus screen as text, trailing blanks trimmed.
    pub fn scrollback_text(&self) -> String {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let mut out = String::new();
        for line in -history..grid.screen_lines() as i32 {
            let mut row = String::new();
            for col in 0..grid.columns() {
                let cell = &grid[Line(line)][Column(col)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN)
                {
                    continue;
                }
                row.push(cell.c);
            }
            out.push_str(row.trim_end());
            out.push('\n');
        }
        while out.ends_with("\n\n") {
            out.pop();
        }
        out
    }

    /// The text of screen row `row` (0-based on the displayed screen).
    pub fn row_text(&self, row: usize) -> String {
        let grid = self.term.grid();
        let line = Line(row as i32 - grid.display_offset() as i32);
        let mut s = String::new();
        for col in 0..grid.columns() {
            let cell = &grid[line][Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN)
            {
                continue;
            }
            s.push(cell.c);
        }
        s
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
        }
    }
}

/// The scrollback a terminal starts with (`terminal.scrollback`).
pub const HISTORY: usize = 10_000;

fn config(history: usize) -> Config {
    Config {
        scrolling_history: history,
        ..Config::default()
    }
}

/// The path of a `file://host/path` URL, percent-decoded, when the host
/// is this machine's (none, `localhost`, or its name): a shell over ssh
/// reports its own host's directory, which is not one here.
fn file_url_path(url: &str, any_host: bool) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    if !(any_host
        || host.is_empty()
        || host.eq_ignore_ascii_case("localhost")
        || is_this_host(host))
    {
        return None;
    }
    let path = &rest[slash..];
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    // On bytes: a `%` before a character of more than one byte is not
    // an escape, and slicing the `str` there would cut the character.
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    let s = String::from_utf8(out).ok()?;
    // `file:///C:/x` on Windows.
    let s = match s.as_bytes() {
        [b'/', d, b':', ..] if d.is_ascii_alphabetic() && cfg!(windows) => s[1..].to_string(),
        _ => s,
    };
    Some(std::path::PathBuf::from(s))
}

/// Whether `host` names this machine: its host name, or that name's
/// first label (`mac.local` reports `mac`, a shell may report either).
fn is_this_host(host: &str) -> bool {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: `gethostname` writes at most `buf.len()` bytes into
        // `buf`, which it owns for the call.
        let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if r != 0 {
            return false;
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        let Ok(name) = std::str::from_utf8(&buf[..end]) else {
            return false;
        };
        let short = |s: &str| s.split('.').next().unwrap_or(s).to_ascii_lowercase();
        name.eq_ignore_ascii_case(host) || short(name) == short(host)
    }
    #[cfg(not(unix))]
    {
        let _ = host;
        false
    }
}

#[cfg(target_os = "macos")]
fn pid_cwd(pid: u32) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: `proc_pidinfo` fills at most `size` bytes of `info`, a
    // plain C struct zeroed first; the path is a NUL-terminated string
    // inside it.
    unsafe {
        let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
        let n = libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if n != size {
            return None;
        }
        let raw = &info.pvi_cdir.vip_path;
        let bytes: &[u8] = std::slice::from_raw_parts(raw.as_ptr() as *const u8, 32 * 32);
        let end = bytes.iter().position(|b| *b == 0)?;
        (end > 0).then(|| std::ffi::OsStr::from_bytes(&bytes[..end]).into())
    }
}

#[cfg(target_os = "linux")]
fn pid_cwd(pid: u32) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn pid_cwd(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

/// Key bytes for a terminal, from kui's key payload fields. `None` for a
/// key the terminal has no encoding for (a lone modifier, F13…).
pub fn encode_key(
    code: &str,
    text: Option<&str>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    if alt {
        out.push(0x1b);
    }
    let seq: Vec<u8> = match code {
        "enter" => b"\r".to_vec(),
        "tab" if shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "backspace" if ctrl => b"\x08".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "space" if ctrl => b"\0".to_vec(),
        "space" => b" ".to_vec(),
        "delete" => b"\x1b[3~".to_vec(),
        "insert" => b"\x1b[2~".to_vec(),
        "pageup" => b"\x1b[5~".to_vec(),
        "pagedown" => b"\x1b[6~".to_vec(),
        "up" | "down" | "right" | "left" | "home" | "end" => {
            let ch = match code {
                "up" => b'A',
                "down" => b'B',
                "right" => b'C',
                "left" => b'D',
                "home" => b'H',
                _ => b'F',
            };
            let modifier = 1 + shift as u8 + (alt as u8) * 2 + (ctrl as u8) * 4;
            if modifier > 1 {
                out.clear();
                format!("\x1b[1;{modifier}{}", ch as char).into_bytes()
            } else if app_cursor {
                vec![0x1b, b'O', ch]
            } else {
                vec![0x1b, b'[', ch]
            }
        }
        f if f.len() >= 2 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
            let n: u8 = f[1..].parse().ok()?;
            match n {
                1 => b"\x1bOP".to_vec(),
                2 => b"\x1bOQ".to_vec(),
                3 => b"\x1bOR".to_vec(),
                4 => b"\x1bOS".to_vec(),
                5 => b"\x1b[15~".to_vec(),
                6 => b"\x1b[17~".to_vec(),
                7 => b"\x1b[18~".to_vec(),
                8 => b"\x1b[19~".to_vec(),
                9 => b"\x1b[20~".to_vec(),
                10 => b"\x1b[21~".to_vec(),
                11 => b"\x1b[23~".to_vec(),
                12 => b"\x1b[24~".to_vec(),
                _ => return None,
            }
        }
        _ => {
            let c = code.chars().next()?;
            if code.chars().count() != 1 {
                return None;
            }
            if ctrl {
                let lower = c.to_ascii_lowercase();
                match lower {
                    'a'..='z' => vec![(lower as u8) - b'a' + 1],
                    '[' | '3' => vec![0x1b],
                    '\\' | '4' => vec![0x1c],
                    ']' | '5' => vec![0x1d],
                    '^' | '6' => vec![0x1e],
                    '_' | '7' | '/' => vec![0x1f],
                    '?' | '8' => vec![0x7f],
                    '2' | '@' => vec![0],
                    _ => return None,
                }
            } else {
                match text {
                    Some(t) if !t.is_empty() => t.as_bytes().to_vec(),
                    _ => c.to_string().into_bytes(),
                }
            }
        }
    };
    out.extend(seq);
    Some(out)
}

fn rgb(c: Rgb) -> u32 {
    ((c.r as u32) << 24) | ((c.g as u32) << 16) | ((c.b as u32) << 8) | 0xFF
}

fn dim(c: u32) -> u32 {
    let r = ((c >> 24) & 0xFF) * 2 / 3;
    let g = ((c >> 16) & 0xFF) * 2 / 3;
    let b = ((c >> 8) & 0xFF) * 2 / 3;
    (r << 24) | (g << 16) | (b << 8) | (c & 0xFF)
}

fn indexed(i: u8, pal: &Palette) -> u32 {
    match i {
        0..=15 => pal.ansi[i as usize],
        16..=231 => {
            let i = i - 16;
            let step = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            let (r, g, b) = (step(i / 36), step((i % 36) / 6), step(i % 6));
            ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            ((v as u32) << 24) | ((v as u32) << 16) | ((v as u32) << 8) | 0xFF
        }
    }
}

fn color(c: Color, pal: &Palette, default: u32) -> u32 {
    match c {
        Color::Spec(spec) => rgb(spec),
        Color::Indexed(i) => indexed(i, pal),
        Color::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground => pal.fg,
            NamedColor::Background => pal.bg,
            NamedColor::Black => pal.ansi[0],
            NamedColor::Red => pal.ansi[1],
            NamedColor::Green => pal.ansi[2],
            NamedColor::Yellow => pal.ansi[3],
            NamedColor::Blue => pal.ansi[4],
            NamedColor::Magenta => pal.ansi[5],
            NamedColor::Cyan => pal.ansi[6],
            NamedColor::White => pal.ansi[7],
            NamedColor::BrightBlack => pal.ansi[8],
            NamedColor::BrightRed => pal.ansi[9],
            NamedColor::BrightGreen => pal.ansi[10],
            NamedColor::BrightYellow => pal.ansi[11],
            NamedColor::BrightBlue => pal.ansi[12],
            NamedColor::BrightMagenta => pal.ansi[13],
            NamedColor::BrightCyan => pal.ansi[14],
            NamedColor::BrightWhite => pal.ansi[15],
            NamedColor::DimForeground => dim(pal.fg),
            _ => default,
        },
    }
}

/// The sixteen on a dark base (Tomorrow Night).
pub const ANSI: [u32; 16] = [
    0x1D1F21FF, 0xCC6666FF, 0xB5BD68FF, 0xF0C674FF, 0x81A2BEFF, 0xB294BBFF, 0x8ABEB7FF, 0xC5C8C6FF,
    0x666666FF, 0xD54E53FF, 0xB9CA4AFF, 0xE7C547FF, 0x7AA6DAFF, 0xC397D8FF, 0x70C0B1FF, 0xEAEAEAFF,
];

/// The sixteen on a light base (Tomorrow): the same hues, each dark
/// enough to read on white, the "bright" eight a shade lighter than the
/// plain ones rather than lighter than the page.
pub const ANSI_LIGHT: [u32; 16] = [
    0x4D4D4CFF, 0xC82829FF, 0x718C00FF, 0xB58900FF, 0x4271AEFF, 0x8959A8FF, 0x3E999FFF, 0x8E908CFF,
    0x6E7070FF, 0xD84C4DFF, 0x85A000FF, 0xC79C00FF, 0x5A87C4FF, 0xA070BFFF, 0x50ADB3FF, 0xB4B4B0FF,
];

/// alacritty's `Term` with the hooks kawoosh adds in front of it: every
/// `Handler` method is the term's, and the few the term does not know
/// — mode 2031 set, reset and reported, the `? 996` colour-scheme
/// query — are answered here (the replies in [`Modes`], written after
/// the parser's pass).
struct Hooked<'a> {
    term: &'a mut Term<Proxy>,
    modes: &'a mut Modes,
}

impl Hooked<'_> {
    /// `f` on the term, the lines it scrolled into history counted:
    /// history's growth, or at its cap a line fed at the bottom — the
    /// primary screen's alone, the alternate keeping no history.
    fn counted(&mut self, feeds_line: bool, f: impl FnOnce(&mut Term<Proxy>)) {
        let alt = self.term.mode().contains(TermMode::ALT_SCREEN);
        let grid = self.term.grid();
        let before = grid.history_size();
        let bottom = grid.screen_lines() as i32 - 1;
        let was_bottom = grid.cursor.point.line.0 == bottom;
        f(self.term);
        if alt {
            return;
        }
        let grid = self.term.grid();
        let after = grid.history_size();
        if after > before {
            self.modes.scrolled += (after - before) as u64;
        } else if feeds_line
            && before >= self.modes.history_max
            && was_bottom
            && grid.cursor.point.line.0 == bottom
        {
            self.modes.scrolled += 1;
        }
    }
}

impl Handler for Hooked<'_> {
    fn set_private_mode(&mut self, mode: PrivateMode) {
        if mode == PrivateMode::Unknown(MODE_APPEARANCE) {
            self.modes.report_appearance = true;
            return;
        }
        self.term.set_private_mode(mode)
    }

    fn unset_private_mode(&mut self, mode: PrivateMode) {
        if mode == PrivateMode::Unknown(MODE_APPEARANCE) {
            self.modes.report_appearance = false;
            return;
        }
        self.term.unset_private_mode(mode)
    }

    /// DECRQM on 2031: `CSI ? 2031 ; 1 $ y` while set, `; 2` while
    /// reset — the "recognised" answers, where alacritty's would be
    /// "not recognised" (`; 0`).
    fn report_private_mode(&mut self, mode: PrivateMode) {
        if mode == PrivateMode::Unknown(MODE_APPEARANCE) {
            let state = if self.modes.report_appearance { 1 } else { 2 };
            self.modes
                .replies
                .extend_from_slice(format!("\x1b[?{MODE_APPEARANCE};{state}$y").as_bytes());
            return;
        }
        self.term.report_private_mode(mode)
    }

    fn set_title(&mut self, a0: Option<String>) {
        self.term.set_title(a0)
    }
    fn set_cursor_style(&mut self, a0: Option<CursorStyle>) {
        self.term.set_cursor_style(a0)
    }
    fn set_cursor_shape(&mut self, shape: CursorShape) {
        self.term.set_cursor_shape(shape)
    }
    fn input(&mut self, c: char) {
        self.counted(false, |t| t.input(c))
    }
    fn goto(&mut self, line: i32, col: usize) {
        self.term.goto(line, col)
    }
    fn goto_line(&mut self, line: i32) {
        self.term.goto_line(line)
    }
    fn goto_col(&mut self, col: usize) {
        self.term.goto_col(col)
    }
    fn insert_blank(&mut self, a0: usize) {
        self.term.insert_blank(a0)
    }
    fn move_up(&mut self, a0: usize) {
        self.term.move_up(a0)
    }
    fn move_down(&mut self, a0: usize) {
        self.term.move_down(a0)
    }
    fn identify_terminal(&mut self, intermediate: Option<char>) {
        self.term.identify_terminal(intermediate)
    }
    fn device_status(&mut self, a0: usize) {
        self.term.device_status(a0)
    }
    fn move_forward(&mut self, col: usize) {
        self.term.move_forward(col)
    }
    fn move_backward(&mut self, col: usize) {
        self.term.move_backward(col)
    }
    fn move_down_and_cr(&mut self, row: usize) {
        self.counted(false, |t| t.move_down_and_cr(row))
    }
    fn move_up_and_cr(&mut self, row: usize) {
        self.term.move_up_and_cr(row)
    }
    fn put_tab(&mut self, count: u16) {
        self.term.put_tab(count)
    }
    fn backspace(&mut self) {
        self.term.backspace()
    }
    fn carriage_return(&mut self) {
        self.term.carriage_return()
    }
    fn linefeed(&mut self) {
        self.counted(true, |t| t.linefeed())
    }
    fn bell(&mut self) {
        self.term.bell()
    }
    fn substitute(&mut self) {
        self.term.substitute()
    }
    fn newline(&mut self) {
        self.counted(true, |t| t.newline())
    }
    fn set_horizontal_tabstop(&mut self) {
        self.term.set_horizontal_tabstop()
    }
    fn scroll_up(&mut self, a0: usize) {
        self.counted(false, |t| t.scroll_up(a0))
    }
    fn scroll_down(&mut self, a0: usize) {
        self.term.scroll_down(a0)
    }
    fn insert_blank_lines(&mut self, a0: usize) {
        self.term.insert_blank_lines(a0)
    }
    fn delete_lines(&mut self, a0: usize) {
        self.term.delete_lines(a0)
    }
    fn erase_chars(&mut self, a0: usize) {
        self.term.erase_chars(a0)
    }
    fn delete_chars(&mut self, a0: usize) {
        self.term.delete_chars(a0)
    }
    fn move_backward_tabs(&mut self, count: u16) {
        self.term.move_backward_tabs(count)
    }
    fn move_forward_tabs(&mut self, count: u16) {
        self.term.move_forward_tabs(count)
    }
    fn save_cursor_position(&mut self) {
        self.term.save_cursor_position()
    }
    fn restore_cursor_position(&mut self) {
        self.term.restore_cursor_position()
    }
    fn clear_line(&mut self, mode: LineClearMode) {
        self.term.clear_line(mode)
    }
    fn clear_screen(&mut self, mode: ClearMode) {
        self.counted(false, |t| t.clear_screen(mode))
    }
    fn clear_tabs(&mut self, mode: TabulationClearMode) {
        self.term.clear_tabs(mode)
    }
    fn set_tabs(&mut self, interval: u16) {
        self.term.set_tabs(interval)
    }
    fn reset_state(&mut self) {
        self.term.reset_state()
    }
    fn reverse_index(&mut self) {
        self.term.reverse_index()
    }
    fn terminal_attribute(&mut self, attr: Attr) {
        self.term.terminal_attribute(attr)
    }
    fn set_mode(&mut self, mode: Mode) {
        self.term.set_mode(mode)
    }
    fn unset_mode(&mut self, mode: Mode) {
        self.term.unset_mode(mode)
    }
    fn report_mode(&mut self, mode: Mode) {
        self.term.report_mode(mode)
    }
    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        self.term.set_scrolling_region(top, bottom)
    }
    fn set_keypad_application_mode(&mut self) {
        self.term.set_keypad_application_mode()
    }
    fn unset_keypad_application_mode(&mut self) {
        self.term.unset_keypad_application_mode()
    }
    fn set_active_charset(&mut self, a0: CharsetIndex) {
        self.term.set_active_charset(a0)
    }
    fn configure_charset(&mut self, a0: CharsetIndex, a1: StandardCharset) {
        self.term.configure_charset(a0, a1)
    }
    fn set_color(&mut self, a0: usize, a1: Rgb) {
        self.term.set_color(a0, a1)
    }
    fn dynamic_color_sequence(&mut self, a0: String, a1: usize, a2: &str) {
        self.term.dynamic_color_sequence(a0, a1, a2)
    }
    fn reset_color(&mut self, a0: usize) {
        self.term.reset_color(a0)
    }
    fn clipboard_store(&mut self, a0: u8, a1: &[u8]) {
        self.term.clipboard_store(a0, a1)
    }
    fn clipboard_load(&mut self, a0: u8, a1: &str) {
        self.term.clipboard_load(a0, a1)
    }
    fn decaln(&mut self) {
        self.term.decaln()
    }
    fn push_title(&mut self) {
        self.term.push_title()
    }
    fn pop_title(&mut self) {
        self.term.pop_title()
    }
    fn text_area_size_pixels(&mut self) {
        self.term.text_area_size_pixels()
    }
    fn text_area_size_chars(&mut self) {
        self.term.text_area_size_chars()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(term: &Terminal) -> Vec<String> {
        let s = term.screen();
        (0..s.rows)
            .map(|r| {
                s.cells[r * s.cols..(r + 1) * s.cols]
                    .iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// A program that turns echo off at a prompt is asking for a
    /// password; one that turns canonical mode off too (a full-screen
    /// program) is not.
    #[cfg(unix)]
    #[test]
    fn a_prompt_with_echo_off_is_a_password_prompt() {
        let size = TermSize { rows: 5, cols: 40 };
        let wait = |t: &Terminal, want: bool| {
            (0..200).any(|_| {
                std::thread::sleep(std::time::Duration::from_millis(10));
                t.password_prompt() == want
            })
        };
        let (t, _r) =
            Terminal::spawn(Some("/bin/sh -c 'stty -echo; sleep 3'"), None, size, &[]).unwrap();
        assert!(wait(&t, true), "stty -echo is a password prompt");
        let (t, _r) = Terminal::spawn(
            Some("/bin/sh -c 'stty -echo -icanon; sleep 3'"),
            None,
            size,
            &[],
        )
        .unwrap();
        assert!(wait(&t, false) && !wait(&t, true), "a raw program is not");
        let (t, _r) = Terminal::spawn(Some("/bin/sh -c 'sleep 3'"), None, size, &[]).unwrap();
        assert!(!wait(&t, true), "a shell with echo on is not");
    }

    #[test]
    fn output_cursor_and_colours() {
        let mut t = Terminal::headless(TermSize { rows: 5, cols: 20 });
        t.feed(b"hello\r\n\x1b[31mworld\x1b[0m");
        let s = t.screen();
        assert_eq!(rows(&t)[..2], ["hello", "world"]);
        assert_eq!(s.cursor.map(|(r, c, _)| (r, c)), Some((1, 5)));
        assert_eq!(s.cells[s.cols].fg, ANSI[1]);
    }

    #[test]
    fn alt_screen_and_scrollback() {
        let mut t = Terminal::headless(TermSize { rows: 3, cols: 10 });
        for i in 0..6 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        assert_eq!(t.history_size(), 4);
        assert!(t.scrollback_text().starts_with("l0\nl1\n"));
        t.scroll(2);
        let s = t.screen();
        assert_eq!(s.origin_line, 2);
        assert_eq!(rows(&t)[0], "l2");
        t.scroll_to_bottom();
        t.feed(b"\x1b[?1049h\x1b[2J\x1b[HTUI");
        assert!(t.is_alt_screen());
        assert_eq!(rows(&t)[0], "TUI");
        t.feed(b"\x1b[?1049l");
        assert!(!t.is_alt_screen());
    }

    #[test]
    fn mouse_reports_in_the_mode_asked_for() {
        let mut t = Terminal::headless(TermSize { rows: 5, cols: 20 });
        t.mouse(0, MouseAction::Press, 3, 1, (false, false, false));
        assert!(t.take_sent().is_empty(), "no report without a mouse mode");
        t.feed(b"\x1b[?1000h\x1b[?1006h");
        assert!(t.wants_mouse() && !t.wants_drag());
        t.mouse(0, MouseAction::Press, 3, 1, (false, false, false));
        t.mouse(0, MouseAction::Release, 3, 1, (false, false, false));
        assert_eq!(t.take_sent(), b"\x1b[<0;4;2M\x1b[<0;4;2m");
        t.mouse(65, MouseAction::Press, 0, 0, (false, false, true));
        assert_eq!(t.take_sent(), b"\x1b[<81;1;1M");
        t.feed(b"\x1b[?1002h");
        assert!(t.wants_drag());
        t.mouse(0, MouseAction::Motion, 5, 2, (false, false, false));
        assert_eq!(t.take_sent(), b"\x1b[<32;6;3M");
        // X10 bytes without SGR.
        t.feed(b"\x1b[?1006l");
        t.mouse(0, MouseAction::Press, 0, 0, (false, false, false));
        assert_eq!(t.take_sent(), b"\x1b[M !!");
    }

    /// `OSC 11 ; ?` is answered with the palette's background in the
    /// asker's terminator, a program's own `OSC 11` colour over it;
    /// mode 2031 set is reported by DECRQM and told of a flip of the
    /// base, and reset is neither.
    #[test]
    fn colour_questions_and_the_appearance_mode() {
        let mut t = Terminal::headless(TermSize { rows: 3, cols: 20 });
        t.feed(b"\x1b]11;?\x1b\\");
        assert_eq!(t.take_sent(), b"\x1b]11;rgb:1d1d/1f1f/2121\x1b\\");
        t.set_palette(Palette {
            bg: 0xFFFFFFFF,
            dark: false,
            ..Palette::default()
        });
        assert_eq!(t.take_sent(), b"", "nothing asked to hear of the flip");
        t.feed(b"\x1b]11;?\x07");
        assert_eq!(t.take_sent(), b"\x1b]11;rgb:ffff/ffff/ffff\x07");
        t.feed(b"\x1b]11;rgb:10/20/30\x1b\\\x1b]11;?\x1b\\");
        assert_eq!(
            t.take_sent(),
            b"\x1b]11;rgb:1010/2020/3030\x1b\\",
            "what the program set is what it is told"
        );
        t.feed(b"\x1b[?2031$p");
        assert_eq!(t.take_sent(), b"\x1b[?2031;2$y", "recognised, reset");
        t.feed(b"\x1b[?2031h\x1b[?2031$p");
        assert!(t.reports_appearance());
        assert_eq!(t.take_sent(), b"\x1b[?2031;1$y");
        t.set_palette(Palette::default());
        assert_eq!(t.take_sent(), b"\x1b[?997;1n", "dark now");
        t.set_palette(Palette {
            dark: false,
            ..Palette::default()
        });
        assert_eq!(t.take_sent(), b"\x1b[?997;2n", "light now");
        t.feed(b"\x1b[?2031l");
        t.set_palette(Palette::default());
        assert_eq!(t.take_sent(), b"");
        // The other modes still reach the term through the hook.
        t.feed(b"\x1b[?1049h");
        assert!(t.is_alt_screen());
    }

    /// OSC 7 sets where the shell is, split across reads and with its
    /// escapes decoded; the parser still sees the bytes around it. A
    /// report from another host, or of a directory not here, is not
    /// where this shell is; a `%` before a character of two bytes is not
    /// an escape, and does not cut the character.
    #[test]
    fn the_shell_reports_its_directory() {
        let dir = std::env::temp_dir().join(format!("kawoosh term {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A URL's path: `/tmp/x`, or `/C:/x` on Windows.
        let url = dir.to_str().unwrap().replace(' ', "%20").replace('\\', "/");
        let url = match url.starts_with('/') {
            true => url,
            false => format!("/{url}"),
        };
        let (a, b) = url.split_at(url.len() - 2);
        let mut t = Terminal::headless(TermSize { rows: 3, cols: 20 });
        assert_eq!(t.cwd(), None);
        t.feed(format!("a\x1b]7;file://{a}").as_bytes());
        t.feed(format!("{b}\x1b\\b").as_bytes());
        assert_eq!(t.cwd(), Some(dir.clone()));
        assert_eq!(rows(&t)[0], "ab");
        t.feed(b"\x1b]7;file://elsewhere.example/tmp\x07");
        assert_eq!(t.cwd(), Some(dir.clone()), "another host's is not taken");
        t.feed(b"\x1b]7;file://localhost/no/such/dir\x07");
        assert_eq!(t.cwd(), None, "nor one that is not here");
        t.feed("\x1b]7;file:///tmp/%a\u{e9}\x07".as_bytes());
        assert_eq!(t.cwd(), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A resize moves lines between the screen and history, and the
    /// marks stay on their lines: the last output reads the same after
    /// shrinking and growing, and a command after it is marked where it
    /// ran.
    #[test]
    fn an_empty_prompt_is_one_with_nothing_sent_since() {
        let mut t = Terminal::headless(TermSize { rows: 10, cols: 20 });
        assert!(!t.at_empty_prompt(), "no marks, no telling");
        t.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        assert!(t.at_empty_prompt());
        t.input(b"l");
        assert!(!t.at_empty_prompt(), "something typed");
        t.feed(b"l\r\n\x1b]133;C\x07");
        assert!(!t.at_empty_prompt(), "running");
        t.feed(b"out\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ \x1b]133;B\x07");
        assert!(t.at_empty_prompt(), "the next prompt");
        t.feed(b"\x1b[?1049h");
        assert!(!t.at_empty_prompt(), "a program has the screen");
    }

    #[test]
    fn marks_survive_a_resize() {
        let mut t = Terminal::headless(TermSize { rows: 10, cols: 20 });
        for i in 0..6 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        t.feed(
            b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out1\r\nout2\r\n\x1b]133;D;0\x07",
        );
        t.feed(b"\x1b]133;A\x07$ ");
        assert_eq!(t.last_output().as_deref(), Some("out1\nout2"));
        t.resize(TermSize { rows: 4, cols: 20 });
        assert_eq!(t.last_output().as_deref(), Some("out1\nout2"), "shrunk");
        t.resize(TermSize { rows: 10, cols: 20 });
        assert_eq!(t.last_output().as_deref(), Some("out1\nout2"), "grown back");
        t.resize(TermSize { rows: 4, cols: 20 });
        t.feed(b"echo x\r\n\x1b]133;C\x07x\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ ");
        assert_eq!(t.last_output().as_deref(), Some("x"));
    }

    /// OSC 133's marks: each command's prompt, output and end on the
    /// lines they arrived on, those lines' numbers kept as history
    /// scrolls; the last command's output as text; the view jumped
    /// from prompt to prompt.
    #[test]
    fn prompt_marks_find_commands_and_their_output() {
        let mut t = Terminal::headless(TermSize { rows: 4, cols: 20 });
        let command = |t: &mut Terminal, cmd: &str, out: &[&str]| {
            t.feed(b"\x1b]133;A\x07$ \x1b]133;B\x07");
            t.feed(cmd.as_bytes());
            t.feed(b"\r\n\x1b]133;C\x07");
            for l in out {
                t.feed(format!("{l}\r\n").as_bytes());
            }
            t.feed(b"\x1b]133;D;0\x07");
        };
        command(&mut t, "ls", &["a", "b", "c"]);
        command(&mut t, "echo hi", &["hi"]);
        let cs = t.commands();
        assert_eq!(cs.len(), 2);
        assert_eq!(
            (cs[0].prompt, cs[0].output, cs[0].end),
            (0, Some(1), Some(4))
        );
        assert_eq!(cs[1].prompt, 4);
        assert_eq!(cs[1].status, Some(0));
        assert_eq!(t.last_output().as_deref(), Some("hi"));
        t.feed(b"\x1b]133;A\x07$ ");
        assert!(t.history_size() > 0, "the first command scrolled off");
        // From the bottom, the second prompt is on show; back is the
        // first, above the view, and then nowhere further. Forward is
        // the second, which only the bottom can show.
        assert_eq!(rows(&t)[1], "$ echo hi");
        assert!(t.jump_prompt(true));
        assert_eq!(rows(&t)[0], "$ ls");
        assert!(!t.jump_prompt(true));
        assert!(t.jump_prompt(false));
        assert_eq!(t.display_offset(), 0);
        assert!(!t.jump_prompt(false), "at the bottom, nothing further");
    }

    #[test]
    fn keys_encode() {
        assert_eq!(
            encode_key("a", Some("a"), false, false, false, false),
            Some(b"a".to_vec())
        );
        assert_eq!(
            encode_key("c", None, true, false, false, false),
            Some(vec![3])
        );
        assert_eq!(
            encode_key("up", None, false, false, false, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_key("up", None, false, false, false, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode_key("left", None, true, false, false, false),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(
            encode_key("x", Some("x"), false, true, false, false),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(
            encode_key("enter", None, false, false, false, false),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            encode_key("f5", None, false, false, false, false),
            Some(b"\x1b[15~".to_vec())
        );
    }
}
