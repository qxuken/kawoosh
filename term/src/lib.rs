//! `term`: `alacritty_terminal` and `portable-pty` behind a facade whose
//! output is a kui `CellGrid` (kui.md D4). Bytes in from the pty reader,
//! key bytes out to the pty, a screenful of cells per frame; scrollback
//! as text for the materialise-into-a-buffer command.

mod graphics;
pub mod kitty;

pub use graphics::Placed;

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

/// A link a program printed on purpose (OSC 8,
/// [`Terminal::hyperlink_at`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub uri: String,
    /// The screen rows it covers and their columns, top first: more
    /// than one when a long line wrapped it.
    pub rows: Vec<(usize, std::ops::Range<usize>)>,
}

/// The line a screen row is part of, as the program printed it: the
/// rows a long line wrapped onto joined ([`Terminal::wrapped_line`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedLine {
    pub text: String,
    /// Each character's cells: its screen row, negative above the
    /// screen or past its foot below, and columns — two for a wide one.
    pub cells: Vec<(i32, std::ops::Range<usize>)>,
}

impl WrappedLine {
    /// The character at `col` of screen row `row`, a wide one's spacer
    /// its; None past the text.
    pub fn char_at(&self, row: usize, col: usize) -> Option<usize> {
        self.cells
            .iter()
            .position(|(r, c)| *r == row as i32 && c.contains(&col))
    }

    /// Where characters `chars` of the text are on the screen: a range
    /// of columns for each row they are on, the rows off it left out.
    pub fn rows_of(
        &self,
        chars: std::ops::Range<usize>,
        screen_rows: usize,
    ) -> Vec<(usize, std::ops::Range<usize>)> {
        let mut out: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
        for (r, c) in
            &self.cells[chars.start.min(self.cells.len())..chars.end.min(self.cells.len())]
        {
            let Ok(r) = usize::try_from(*r) else { continue };
            if r >= screen_rows {
                continue;
            }
            match out.last_mut() {
                Some((last, cols)) if *last == r => cols.end = c.end,
                _ => out.push((r, c.clone())),
            }
        }
        out
    }
}

/// How many rows either way a wrapped line is followed: a line that
/// long is output, not a link.
const WRAP_ROWS: i32 = 64;

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
    /// The screen was cleared since the last look: 1 the screen
    /// (`ED 2`), 2 with its history (`ED 3`), 3 reset (`RIS`) — what the
    /// images on it go with (`graphics.rs`).
    cleared: u8,
    /// How deep the kitty keyboard mode stack is on the primary screen
    /// and on the alternate (alacritty keeps one each, swapped with the
    /// screens): past `KEYBOARD_STACK_MAX` alacritty's push evicts from
    /// its *title* stack — a panic when that is empty — so the push there
    /// is made a set instead (`Hooked::push_keyboard_mode`).
    keyboard_depth: [usize; 2],
}

/// alacritty's keyboard mode stack's cap (`KEYBOARD_MODE_STACK_MAX_DEPTH`).
const KEYBOARD_STACK_MAX: usize = 4096;

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
    /// `ESC _`: an APC, kitty's graphics commands' envelope.
    Apc,
    ApcEsc,
}

/// A sequence the scan found whole.
enum Seq {
    Osc(Vec<u8>),
    Apc(Vec<u8>),
}

impl OscScan {
    /// The longest payload kept: a path, a mark with its options.
    const MAX: usize = 4096;

    /// One byte on; the payload of an OSC or an APC that ended on it,
    /// when one did.
    fn step(&mut self, b: u8) -> Option<Seq> {
        match (self.state, b) {
            (Scan::Ground, 0x1b) => self.state = Scan::Esc,
            (Scan::Ground, _) => {}
            (Scan::Esc, b']') => {
                self.state = Scan::Osc;
                self.buf.clear();
            }
            (Scan::Esc, b'_') => {
                self.state = Scan::Apc;
                self.buf.clear();
            }
            (Scan::Apc, 0x1b) => self.state = Scan::ApcEsc,
            (Scan::Apc, 0x18 | 0x1a) => self.state = Scan::Ground,
            (Scan::Apc, _) => {
                if self.buf.len() < graphics::APC_MAX {
                    self.buf.push(b);
                }
            }
            (Scan::ApcEsc, b'\\') => {
                self.state = Scan::Ground;
                return Some(Seq::Apc(std::mem::take(&mut self.buf)));
            }
            (Scan::ApcEsc, _) => self.state = Scan::Ground,
            (Scan::Esc, 0x1b) => {}
            (Scan::Esc, _) => self.state = Scan::Ground,
            (Scan::Osc, 0x07) => {
                self.state = Scan::Ground;
                return Some(Seq::Osc(std::mem::take(&mut self.buf)));
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
                return Some(Seq::Osc(std::mem::take(&mut self.buf)));
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
    /// Kitty's images and where they are placed (`graphics.rs`).
    graphics: graphics::Graphics,
    /// A cell's size in pixels, as the pty is told it
    /// ([`Terminal::set_cell_pixels`]): what a program sizes an image by.
    cell_px: (u16, u16),
}

impl Terminal {
    /// Spawns `shell` (or `cmd` through it) in a pty. The reader is
    /// handed back for an io thread to pump into [`Terminal::feed`].
    pub fn spawn(
        shell: Option<&str>,
        cmd: Option<&str>,
        cwd: Option<&std::path::Path>,
        size: TermSize,
        envs: &[(String, String)],
    ) -> Result<(Self, Box<dyn Read + Send>)> {
        // The one asked for (`terminal.shell`); else `$SHELL` where
        // there is one (an MSYS bash sets it on Windows too); else
        // `/bin/sh`, or `%ComSpec%` on Windows.
        let shell = shell
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())
                } else {
                    "/bin/sh".into()
                }
            });
        let mut builder = CommandBuilder::new(&shell);
        builder.args(shell_args(&shell, cmd));
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
                graphics: graphics::Graphics::default(),
                cell_px: (0, 0),
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
            graphics: graphics::Graphics::default(),
            cell_px: (0, 0),
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

    /// Something that blocks until the process exits, for a thread to
    /// wait on where the pty's reader does not end with it: ConPTY keeps
    /// its output pipe open until the pseudoconsole is closed, so a
    /// shell that exits leaves the reader waiting. None elsewhere, where
    /// the reader's end is the exit.
    #[cfg(windows)]
    pub fn exit_waiter(&self) -> Option<Box<dyn FnOnce() + Send>> {
        use std::os::windows::io::{AsRawHandle as _, BorrowedHandle};
        use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};
        let raw = self.child.as_ref()?.as_raw_handle()?;
        // SAFETY: the child keeps its process handle open while `self`
        // lives, and it is duplicated here, before this returns.
        let owned = unsafe { BorrowedHandle::borrow_raw(raw) }
            .try_clone_to_owned()
            .ok()?;
        Some(Box::new(move || {
            // SAFETY: `owned` is a process handle this closure owns.
            unsafe { WaitForSingleObject(owned.as_raw_handle(), INFINITE) };
        }))
    }

    #[cfg(not(windows))]
    pub fn exit_waiter(&self) -> Option<Box<dyn FnOnce() + Send>> {
        None
    }

    /// Bytes from the pty: parsed, replies (a DA answer, a cursor report)
    /// written back, and the terminal's own events read out. The OSCs
    /// the parser drops — 7 and 133 — are read here, the bytes before
    /// each parsed first so a mark lands on the line the cursor is on
    /// when it arrives.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut from = 0;
        for (i, &b) in bytes.iter().enumerate() {
            match self.scan.step(b) {
                Some(Seq::Osc(osc)) => {
                    self.advance(&bytes[from..=i]);
                    from = i + 1;
                    self.on_osc(&osc);
                }
                Some(Seq::Apc(apc)) => {
                    self.advance(&bytes[from..=i]);
                    from = i + 1;
                    if let Some(cmd) = apc.strip_prefix(b"G") {
                        self.on_graphics(cmd);
                    }
                }
                None => {}
            }
        }
        self.advance(&bytes[from..]);
        self.graphics.poll(false);
        let replies = std::mem::take(&mut self.graphics.replies);
        if !replies.is_empty() {
            self.send(&replies);
        }
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
        let alt = self.is_alt_screen();
        self.parser.advance(
            &mut Hooked {
                term: &mut self.term,
                modes: &mut self.modes,
            },
            bytes,
        );
        // What the images on screen go with: a clear, a reset, the
        // alternate screen left, history's far end (`graphics.rs`).
        match std::mem::take(&mut self.modes.cleared) {
            0 => {}
            3 => self.graphics.reset(),
            n => {
                let mut here = self.here();
                here.top = self.line_of(0);
                self.graphics.cleared(&here, n == 2);
            }
        }
        if alt && !self.is_alt_screen() {
            self.graphics.left_alt();
        }
        self.graphics.trimmed(self.oldest_line());
    }

    /// What a graphics command is told about the terminal.
    fn here(&self) -> graphics::Here {
        let grid = self.term.grid();
        let point = grid.cursor.point;
        graphics::Here {
            line: self.cursor_line(),
            col: point.column.0 as u16,
            top: self.line_of(-(grid.display_offset() as i32)),
            rows: self.size.rows,
            cols: self.size.cols,
            cell: (self.cell_px.0 as u32, self.cell_px.1 as u32),
            local: self.domain.is_none(),
            alt: self.is_alt_screen(),
        }
    }

    /// A kitty graphics command (an APC's payload after its `G`): run,
    /// and the cursor moved past the image it placed — right by its
    /// columns and down by its rows less one, a column past the edge
    /// the next line's first, as kitty moves it.
    fn on_graphics(&mut self, cmd: &[u8]) {
        let mut here = self.here();
        // A command is about the live screen, not a scrolled view.
        here.top = self.line_of(0);
        let effect = self.graphics.command(cmd, &here);
        if let Some((cols, rows)) = effect.cursor {
            let mut seq = vec![b'\n'; rows.saturating_sub(1) as usize];
            let col = here.col as usize + cols as usize;
            if col >= here.cols as usize {
                seq.extend_from_slice(b"\r\n");
            } else {
                seq.extend_from_slice(format!("\x1b[{}G", col + 1).as_bytes());
            }
            self.advance(&seq);
        }
    }

    /// What is on the screen as it shows now (scrolled or not) of the
    /// images kitty's protocol placed, lowest `z` first, those still
    /// decoding left out ([`Terminal::poll_graphics`]).
    pub fn images(&self) -> Vec<Placed> {
        self.graphics.on_screen(&self.here())
    }

    /// Takes in the images whose decoding finished, and writes their
    /// replies: true when one did, and the screen has an image to draw
    /// it did not have.
    pub fn poll_graphics(&mut self) -> bool {
        let changed = self.graphics.poll(false);
        let replies = std::mem::take(&mut self.graphics.replies);
        if !replies.is_empty() {
            self.send(&replies);
        }
        changed
    }

    /// Whether an image is still decoding: a frame to come back for.
    pub fn graphics_busy(&self) -> bool {
        self.graphics.busy()
    }

    /// Waits for every image still decoding (for tests).
    pub fn settle_graphics(&mut self) {
        self.graphics.poll(true);
        let replies = std::mem::take(&mut self.graphics.replies);
        if !replies.is_empty() {
            self.send(&replies);
        }
    }

    /// A cell's size in pixels — the window's pixels, what a program
    /// sizes an image by: the pty is told the grid's size in them
    /// (`TIOCGWINSZ`), and `CSI 14 t` is answered from them.
    pub fn set_cell_pixels(&mut self, w: u16, h: u16) {
        if (w, h) == self.cell_px {
            return;
        }
        self.cell_px = (w, h);
        self.tell_pty_size();
    }

    fn tell_pty_size(&self) {
        if let Some(pty) = &self.pty {
            let _ = pty.resize(PtySize {
                rows: self.size.rows,
                cols: self.size.cols,
                pixel_width: self.size.cols.saturating_mul(self.cell_px.0),
                pixel_height: self.size.rows.saturating_mul(self.cell_px.1),
            });
        }
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

    /// The pty's foreground process group — the program in front, a
    /// shell at its prompt or what it ran — and its name, asked of the
    /// system (terminal-keys.md Decision 2's `terminal.raw`).
    #[cfg(unix)]
    pub fn foreground(&self) -> Option<(i32, String)> {
        let pgid = self.pty.as_ref()?.process_group_leader()?;
        Some((pgid, pid_name(pgid as u32)?))
    }

    #[cfg(not(unix))]
    pub fn foreground(&self) -> Option<(i32, String)> {
        None
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
                // `CSI 14 t`: the text area in pixels.
                Event::TextAreaSizeRequest(format) => {
                    let reply = format(alacritty_terminal::event::WindowSize {
                        num_lines: self.size.rows,
                        num_cols: self.size.cols,
                        cell_width: self.cell_px.0,
                        cell_height: self.cell_px.1,
                    });
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
        self.tell_pty_size();
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

    /// The kitty keyboard protocol's flags the program has pushed on this
    /// screen, as `kitty::DISAMBIGUATE` … `kitty::ASSOCIATED_TEXT`; 0 for
    /// none, the legacy encoding.
    pub fn keyboard_flags(&self) -> u8 {
        let m = self.term.mode();
        let bit = |f: TermMode, v: u8| if m.contains(f) { v } else { 0 };
        bit(TermMode::DISAMBIGUATE_ESC_CODES, kitty::DISAMBIGUATE)
            | bit(TermMode::REPORT_EVENT_TYPES, kitty::EVENT_TYPES)
            | bit(TermMode::REPORT_ALTERNATE_KEYS, kitty::ALTERNATE_KEYS)
            | bit(TermMode::REPORT_ALL_KEYS_AS_ESC, kitty::ALL_KEYS)
            | bit(TermMode::REPORT_ASSOCIATED_TEXT, kitty::ASSOCIATED_TEXT)
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
        self.scrollback_styled().0
    }

    /// [`Self::scrollback_text`] with the colours it was printed in
    /// (roadmap step 31): each run of text whose foreground is not the
    /// palette's own, as a byte range of the text and an RGBA colour —
    /// resolved as the screen draws it, inverse and dim included — so
    /// copy mode's buffer reads as the pane did.
    pub fn scrollback_styled(&self) -> (String, Vec<(std::ops::Range<usize>, u32)>) {
        let pal = &self.palette;
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let mut out = String::new();
        let mut runs: Vec<(std::ops::Range<usize>, u32)> = Vec::new();
        for line in -history..grid.screen_lines() as i32 {
            let start = out.len();
            let mut row_runs: Vec<(std::ops::Range<usize>, u32)> = Vec::new();
            for col in 0..grid.columns() {
                let cell = &grid[Line(line)][Column(col)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN)
                {
                    continue;
                }
                let at = out.len();
                out.push(cell.c);
                let mut fg = color(cell.fg, pal, pal.fg);
                if cell.flags.contains(Flags::INVERSE) {
                    fg = match cell.bg {
                        Color::Named(NamedColor::Background) => pal.bg,
                        other => color(other, pal, pal.bg),
                    };
                }
                if cell.flags.contains(Flags::DIM) {
                    fg = dim(fg);
                }
                if fg == pal.fg || cell.c == ' ' {
                    continue;
                }
                match row_runs.last_mut() {
                    Some((r, c)) if *c == fg && r.end == at => r.end = out.len(),
                    _ => row_runs.push((at..out.len(), fg)),
                }
            }
            let kept = out[start..].trim_end().len();
            out.truncate(start + kept);
            runs.extend(
                row_runs
                    .into_iter()
                    .filter(|(r, _)| r.start < start + kept)
                    .map(|(r, c)| (r.start..r.end.min(start + kept), c)),
            );
            out.push('\n');
        }
        while out.ends_with("\n\n") {
            out.pop();
        }
        (out, runs)
    }

    /// Where the cursor is in [`Self::scrollback_styled`]'s text: its
    /// line, and its column as the characters that line has before it
    /// — a wide character's spacer and a hidden cell not counted, as
    /// the text leaves them out. The column can be past the line's end,
    /// where trailing blanks were trimmed.
    pub fn scrollback_cursor(&self) -> (usize, usize) {
        let grid = self.term.grid();
        let point = grid.cursor.point;
        let row = &grid[point.line];
        let col = (0..point.column.0.min(grid.columns()))
            .filter(|c| {
                !row[Column(*c)]
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN)
            })
            .count();
        (grid.history_size() + point.line.0 as usize, col)
    }

    /// The link a program printed on purpose (OSC 8) at `(row, col)` of
    /// the screen: its URI and the cells it covers — the run of cells
    /// around `col` with the same link, across the rows a long line
    /// wrapped it onto.
    pub fn hyperlink_at(&self, row: usize, col: usize) -> Option<Hyperlink> {
        let grid = self.term.grid();
        if row >= grid.screen_lines() || col >= grid.columns() {
            return None;
        }
        let offset = grid.display_offset() as i32;
        let points = self.wrapped_points(row);
        let here = (Line(row as i32 - offset), Column(col));
        let i = points.iter().position(|p| *p == here)?;
        let link = |i: usize| grid[points[i].0][points[i].1].hyperlink();
        let at = link(i)?;
        let same = |i: usize| link(i).as_ref() == Some(&at);
        let mut start = i;
        while start > 0 && same(start - 1) {
            start -= 1;
        }
        let mut end = i + 1;
        while end < points.len() && same(end) {
            end += 1;
        }
        let mut rows: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
        for (line, c) in &points[start..end] {
            let Ok(r) = usize::try_from(line.0 + offset) else {
                continue;
            };
            if r >= grid.screen_lines() {
                continue;
            }
            match rows.last_mut() {
                Some((last, cols)) if *last == r => cols.end = c.0 + 1,
                _ => rows.push((r, c.0..c.0 + 1)),
            }
        }
        Some(Hyperlink {
            uri: at.uri().to_string(),
            rows,
        })
    }

    /// A `file://` link's path as this terminal's, and the line its
    /// fragment names (`#12`, `#L12`): on its domain whatever host the
    /// URL names (`box:/…`), else here when the host is this machine.
    /// None for another host's file, or a URL that is not a file's.
    pub fn file_link(&self, uri: &str) -> Option<(std::path::PathBuf, Option<usize>)> {
        let (url, fragment) = uri.split_once('#').unwrap_or((uri, ""));
        let path = file_url_path(url, self.domain.is_some())?;
        let path = match &self.domain {
            Some(d) => format!("{d}:{}", path.display()).into(),
            None => path,
        };
        let line = fragment
            .trim_start_matches(['L', 'l'])
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|n| n.parse().ok())
            .filter(|n| *n > 0);
        Some((path, line))
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

    /// The line screen row `row` (0-based on the displayed screen) is
    /// part of: the rows above and below it that a long line wrapped
    /// onto joined, as text, with where each character is — a URL or a
    /// path printed across the terminal's edge is one again.
    pub fn wrapped_line(&self, row: usize) -> WrappedLine {
        let grid = self.term.grid();
        let offset = grid.display_offset() as i32;
        let mut line = WrappedLine {
            text: String::new(),
            cells: Vec::new(),
        };
        for (l, c) in self.wrapped_points(row) {
            let cell = &grid[l][c];
            if cell.flags.intersects(
                Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER | Flags::HIDDEN,
            ) {
                continue;
            }
            let width = if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
            line.text.push(cell.c);
            line.cells.push((l.0 + offset, c.0..c.0 + width));
        }
        line
    }

    /// The cells of the line screen row `row` is part of, in order: a
    /// row whose last cell says it wrapped (`WRAPLINE`) goes on in the
    /// next, [`WRAP_ROWS`] rows either way at most.
    fn wrapped_points(&self, row: usize) -> Vec<(Line, Column)> {
        let grid = self.term.grid();
        let cols = grid.columns();
        if cols == 0 || row >= grid.screen_lines() {
            return Vec::new();
        }
        let wraps = |l: Line| grid[l][Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        let here = Line(row as i32 - grid.display_offset() as i32);
        let mut first = here;
        while first > grid.topmost_line() && here.0 - first.0 < WRAP_ROWS && wraps(first - 1) {
            first -= 1;
        }
        let mut last = here;
        while last < grid.bottommost_line() && last.0 - here.0 < WRAP_ROWS && wraps(last) {
            last += 1;
        }
        (first.0..=last.0)
            .flat_map(|l| (0..cols).map(move |c| (Line(l), Column(c))))
            .collect()
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
        // kitty's keyboard protocol (terminal-keys.md Decision 5): the
        // program's flags kept, pushed and popped by alacritty, the bytes
        // `kitty::encode`'s.
        kitty_keyboard: true,
        ..Config::default()
    }
}

/// The flags that start `shell` as a login shell, or run `cmd` through
/// it: cmd.exe and PowerShell have their own; every other shell (sh,
/// bash, zsh, fish, nu) takes `-l` and `-c`, apart, since not all of
/// them read `-lc` as two.
fn shell_args(shell: &str, cmd: Option<&str>) -> Vec<String> {
    // Split on both separators by hand: std's `Path` splits only on its
    // own platform's, and a shell is named the same on every one.
    let name = shell.rsplit(['/', '\\']).next().unwrap_or(shell);
    let mut stem = name.to_ascii_lowercase();
    if stem.ends_with(".exe") {
        stem.truncate(stem.len() - 4);
    }
    let args: &[&str] = match (stem.as_str(), cmd) {
        ("cmd", Some(_)) => &["/c"],
        ("cmd", None) => &[],
        ("pwsh" | "powershell", Some(_)) => &["-NoLogo", "-Command"],
        ("pwsh" | "powershell", None) => &["-NoLogo"],
        (_, Some(_)) => &["-l", "-c"],
        (_, None) => &["-l"],
    };
    args.iter()
        .map(|s| s.to_string())
        .chain(cmd.map(str::to_string))
        .collect()
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

/// A process's name: its executable's file name, as `ps -o comm` has it.
#[cfg(target_os = "macos")]
fn pid_name(pid: u32) -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: `proc_name` writes at most `buf.len()` bytes of the name and
    // returns how many; any pid is sound to ask about.
    let n = unsafe { libc::proc_name(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

#[cfg(target_os = "linux")]
fn pid_name(pid: u32) -> Option<String> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(s.trim_end().to_string())
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn pid_name(_pid: u32) -> Option<String> {
    None
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
        // The `~` keys carry a modifier as xterm spells it, `CSI 5 ; 2 ~`
        // for Shift+PageUp — reached once raw hands them to the program.
        "delete" | "insert" | "pageup" | "pagedown" => {
            let n = match code {
                "delete" => 3,
                "insert" => 2,
                "pageup" => 5,
                _ => 6,
            };
            let modifier = 1 + shift as u8 + (alt as u8) * 2 + (ctrl as u8) * 4;
            if modifier > 1 {
                out.clear();
                format!("\x1b[{n};{modifier}~").into_bytes()
            } else {
                format!("\x1b[{n}~").into_bytes()
            }
        }
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
            // Under a modifier as xterm spells them: `CSI 1 ; m P` for
            // F1–F4, `CSI 15 ; m ~` from F5 on.
            let modifier = 1 + shift as u8 + (alt as u8) * 2 + (ctrl as u8) * 4;
            if modifier > 1 {
                out.clear();
            }
            match (n, modifier > 1) {
                (1..=4, false) => vec![0x1b, b'O', b"PQRS"[n as usize - 1]],
                (1..=4, true) => {
                    format!("\x1b[1;{modifier}{}", b"PQRS"[n as usize - 1] as char).into_bytes()
                }
                (5..=12, _) => {
                    let code = [15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5];
                    if modifier > 1 {
                        format!("\x1b[{code};{modifier}~").into_bytes()
                    } else {
                        format!("\x1b[{code}~").into_bytes()
                    }
                }
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
        self.modes.cleared = self.modes.cleared.max(match mode {
            ClearMode::All => 1,
            ClearMode::Saved => 2,
            _ => 0,
        });
        self.counted(false, |t| t.clear_screen(mode))
    }
    fn clear_tabs(&mut self, mode: TabulationClearMode) {
        self.term.clear_tabs(mode)
    }
    fn set_tabs(&mut self, interval: u16) {
        self.term.set_tabs(interval)
    }
    fn reset_state(&mut self) {
        self.modes.cleared = 3;
        self.modes.keyboard_depth = [0, 0];
        self.term.reset_state()
    }

    // kitty's keyboard protocol: alacritty keeps the program's flags, a
    // stack a screen, and answers `CSI ? u`; the depth is mirrored here
    // so a push past the cap does not reach alacritty's (see
    // `Modes::keyboard_depth`) — there it replaces the top instead.
    fn push_keyboard_mode(&mut self, mode: KeyboardModes) {
        let screen = self.term.mode().contains(TermMode::ALT_SCREEN) as usize;
        let depth = &mut self.modes.keyboard_depth[screen];
        if *depth >= KEYBOARD_STACK_MAX {
            self.term
                .set_keyboard_mode(mode, KeyboardModesApplyBehavior::Replace);
            return;
        }
        *depth += 1;
        self.term.push_keyboard_mode(mode)
    }
    fn pop_keyboard_modes(&mut self, to_pop: u16) {
        let screen = self.term.mode().contains(TermMode::ALT_SCREEN) as usize;
        let depth = &mut self.modes.keyboard_depth[screen];
        *depth = depth.saturating_sub(to_pop as usize);
        self.term.pop_keyboard_modes(to_pop)
    }
    fn set_keyboard_mode(&mut self, mode: KeyboardModes, how: KeyboardModesApplyBehavior) {
        self.term.set_keyboard_mode(mode, how)
    }
    fn report_keyboard_mode(&mut self) {
        self.term.report_keyboard_mode()
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
    // OSC 8: the cells printed after it carry the link
    // (`Terminal::hyperlink_at`). xterm's `modifyOtherKeys` stays
    // unforwarded: nothing here speaks it, and a program told it is on
    // would wait for keys it never gets — kitty's protocol is the one
    // spoken (above).
    fn set_hyperlink(&mut self, link: Option<alacritty_terminal::vte::ansi::Hyperlink>) {
        self.term.set_hyperlink(link);
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
        let (t, _r) = Terminal::spawn(
            None,
            Some("/bin/sh -c 'stty -echo; sleep 3'"),
            None,
            size,
            &[],
        )
        .unwrap();
        assert!(wait(&t, true), "stty -echo is a password prompt");
        let (t, _r) = Terminal::spawn(
            None,
            Some("/bin/sh -c 'stty -echo -icanon; sleep 3'"),
            None,
            size,
            &[],
        )
        .unwrap();
        assert!(wait(&t, false) && !wait(&t, true), "a raw program is not");
        let (t, _r) = Terminal::spawn(None, Some("/bin/sh -c 'sleep 3'"), None, size, &[]).unwrap();
        assert!(!wait(&t, true), "a shell with echo on is not");
    }

    #[test]
    fn each_shell_gets_its_own_flags() {
        let args = |shell, cmd| shell_args(shell, cmd);
        assert_eq!(args("nu", None), ["-l"]);
        assert_eq!(args("/usr/bin/nu", Some("ls")), ["-l", "-c", "ls"]);
        assert_eq!(
            args(r"C:\Windows\system32\cmd.exe", Some("dir")),
            ["/c", "dir"]
        );
        assert!(args("CMD.EXE", None).is_empty());
        assert_eq!(args("pwsh.exe", Some("ls")), ["-NoLogo", "-Command", "ls"]);
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

    /// A terminal with 10×20-pixel cells, for kitty's graphics.
    fn graphic_term(rows: u16, cols: u16) -> Terminal {
        let mut t = Terminal::headless(TermSize { rows, cols });
        t.set_cell_pixels(10, 20);
        t
    }

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn apc(keys: &str, data: &[u8]) -> Vec<u8> {
        format!("\x1b_G{keys};{}\x1b\\", b64(data)).into_bytes()
    }

    fn cursor(t: &Terminal) -> (i32, usize) {
        let p = t.term.grid().cursor.point;
        (p.line.0, p.column.0)
    }

    #[test]
    fn a_query_is_answered_and_stores_nothing() {
        let mut t = graphic_term(5, 20);
        t.feed(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
        assert_eq!(t.take_sent(), b"\x1b_Gi=31;OK\x1b\\");
        t.feed(b"\x1b_Gi=32,s=2,v=2,a=q,f=24;AAAA\x1b\\");
        let sent = String::from_utf8(t.take_sent()).unwrap();
        assert!(sent.starts_with("\x1b_Gi=32;ENODATA"), "{sent:?}");
        t.feed(b"\x1b_Ga=p,i=31\x1b\\");
        assert!(
            String::from_utf8(t.take_sent()).unwrap().contains("ENOENT"),
            "a query keeps nothing to put"
        );
    }

    #[test]
    fn an_image_is_placed_at_the_cursor_and_moves_it() {
        let mut t = graphic_term(6, 20);
        t.feed(b"ab");
        // 20×40 pixels: two cells by two, the cursor past them on the
        // image's last row.
        t.feed(&apc("a=T,f=32,s=20,v=40,i=1", &[200; 20 * 40 * 4]));
        assert_eq!(cursor(&t), (1, 4));
        // Drawn once decoded — which a worker may already have done by
        // the time `feed` looked, so no look before the wait.
        t.settle_graphics();
        assert_eq!(t.take_sent(), b"\x1b_Gi=1;OK\x1b\\");
        let placed = t.images();
        assert_eq!(placed.len(), 1);
        let p = &placed[0];
        assert_eq!(
            (p.row, p.col, p.size, p.src),
            (0, 2, (20, 40), (0, 0, 20, 40))
        );
        assert_eq!(p.rgba.len(), 20 * 40 * 4);
        // Put again at cells asked for, the cursor left where it is.
        t.feed(b"\r\n");
        t.feed(b"\x1b_Ga=p,i=1,c=4,r=1,C=1,q=1\x1b\\");
        assert_eq!(cursor(&t), (2, 0));
        assert!(t.take_sent().is_empty(), "q=1: no OK");
        let placed = t.images();
        assert_eq!(placed.len(), 2);
        assert_eq!((placed[1].row, placed[1].size), (2, (40, 20)));
    }

    #[test]
    fn a_transmission_in_chunks_and_a_png() {
        let mut t = graphic_term(6, 20);
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(30, 20, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let data = b64(&png);
        let (first, rest) = data.split_at(data.len() / 2);
        t.feed(format!("\x1b_Ga=T,f=100,I=5,m=1;{first}\x1b\\").as_bytes());
        assert!(
            t.images().is_empty() && cursor(&t) == (0, 0),
            "not whole yet"
        );
        t.feed(format!("\x1b_Gm=0;{rest}\x1b\\").as_bytes());
        // Its size from the header: three cells by one.
        assert_eq!(cursor(&t), (0, 3));
        t.settle_graphics();
        let sent = String::from_utf8(t.take_sent()).unwrap();
        assert!(
            sent.starts_with("\x1b_Gi=") && sent.ends_with(",I=5;OK\x1b\\"),
            "{sent:?}"
        );
        assert_eq!(t.images()[0].rgba[..4], [1, 2, 3, 255]);
        // Neither an id nor a number: stored, never answered.
        t.feed(&apc("a=T,f=24,s=1,v=1", &[0, 0, 0]));
        t.settle_graphics();
        assert!(t.take_sent().is_empty());
        assert_eq!(t.images().len(), 2);
    }

    #[test]
    fn a_placement_scrolls_with_its_line_and_goes_with_it() {
        let mut t = graphic_term(4, 20);
        t.set_scrollback(3);
        t.feed(&apc("a=T,f=24,s=10,v=20,i=2,C=1", &[5; 10 * 20 * 3]));
        t.settle_graphics();
        t.feed(b"\r\n\r\n");
        assert_eq!(t.images()[0].row, 0);
        t.feed(b"\r\n\r\n");
        assert_eq!(t.images().len(), 0, "scrolled off the screen");
        t.scroll(1);
        assert_eq!(t.images()[0].row, 0, "and into history");
        t.scroll_to_bottom();
        for _ in 0..5 {
            t.feed(b"\r\n");
        }
        t.scroll(3);
        assert!(t.images().is_empty(), "history let its line go");
        assert!(t.graphics.placements.is_empty());
    }

    #[test]
    fn a_clear_a_delete_and_the_alternate_screen() {
        let mut t = graphic_term(6, 20);
        let img = |id: u32| apc(&format!("a=T,f=24,s=10,v=20,i={id},q=2"), &[5; 10 * 20 * 3]);
        t.feed(&img(1));
        t.settle_graphics();
        t.feed(b"\x1b[2J");
        assert!(t.images().is_empty(), "ED 2 clears the images on screen");
        t.feed(b"\x1b_Ga=p,i=1,q=2\x1b\\");
        assert_eq!(t.images().len(), 1, "the image itself is kept");
        // `d=i` drops the placements, `d=I` the image too.
        t.feed(b"\x1b_Ga=d,d=i,i=1\x1b\\");
        assert!(t.images().is_empty());
        t.feed(b"\x1b_Ga=p,i=1,q=2\x1b\\\x1b_Ga=d,d=I,i=1\x1b\\\x1b_Ga=p,i=1\x1b\\");
        assert!(String::from_utf8(t.take_sent()).unwrap().contains("ENOENT"));
        // The alternate screen's images go when it is left; the main
        // screen's are back.
        t.feed(&img(2));
        t.settle_graphics();
        t.feed(b"\x1b[?1049h");
        assert!(t.images().is_empty(), "the main screen's are not the alt's");
        t.feed(&img(3));
        t.settle_graphics();
        assert_eq!(t.images().len(), 1);
        t.feed(b"\x1b[?1049l");
        let placed = t.images();
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].image, 2);
        // A reset takes everything.
        t.feed(b"\x1bc");
        assert!(t.images().is_empty() && t.graphics.images.is_empty());
    }

    #[test]
    fn a_file_is_read_here_and_not_from_a_domain() {
        let path = std::env::temp_dir().join(format!("kawoosh-g-{}.rgb", std::process::id()));
        std::fs::write(&path, [7u8; 3 * 4]).unwrap();
        let mut t = graphic_term(6, 20);
        let keys = "a=T,f=24,s=2,v=2,t=f,i=9";
        t.feed(&apc(keys, path.to_string_lossy().as_bytes()));
        t.settle_graphics();
        assert_eq!(t.take_sent(), b"\x1b_Gi=9;OK\x1b\\");
        assert_eq!(t.images()[0].rgba[..4], [7, 7, 7, 255]);
        // A temporary file must say so in its name.
        t.feed(&apc(
            "a=t,f=24,s=2,v=2,t=t,i=10",
            path.to_string_lossy().as_bytes(),
        ));
        assert!(String::from_utf8(t.take_sent()).unwrap().contains("EPERM"));
        let mut far = graphic_term(6, 20);
        far.set_domain("box", "box:/".into());
        far.feed(&apc(keys, path.to_string_lossy().as_bytes()));
        assert!(
            String::from_utf8(far.take_sent())
                .unwrap()
                .contains("EBADF")
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_pixel_size_is_told() {
        let mut t = graphic_term(6, 20);
        t.feed(b"\x1b[14t");
        assert_eq!(t.take_sent(), b"\x1b[4;120;200t");
    }

    #[test]
    fn what_is_not_supported_says_so() {
        let mut t = graphic_term(6, 20);
        t.feed(&apc("a=T,f=24,s=1,v=1,i=4,U=1", &[0, 0, 0]));
        t.feed(&apc("a=T,f=24,s=1,v=1,i=5,t=s", b"/kitty-shm"));
        t.feed(b"\x1b_Ga=f,i=4\x1b\\");
        let sent = String::from_utf8(t.take_sent()).unwrap();
        assert_eq!(sent.matches("ENOTSUP").count(), 3, "{sent:?}");
    }

    #[test]
    fn a_hyperlink_is_found_across_its_cells() {
        let mut t = Terminal::headless(TermSize { rows: 3, cols: 30 });
        t.feed(b"see \x1b]8;;https://kawoosh.dev/x\x1b\\the docs\x1b]8;;\x1b\\ now");
        let h = t.hyperlink_at(0, 6).unwrap();
        assert_eq!(h.uri, "https://kawoosh.dev/x");
        assert_eq!(h.rows, [(0, 4..12)], "`the docs`, not the text around it");
        assert_eq!(t.hyperlink_at(0, 2), None);
        assert_eq!(t.hyperlink_at(0, 13), None);
        // Two links side by side with the same text stay two.
        t.feed(b"\r\n\x1b]8;id=a;https://a\x1b\\ab\x1b]8;id=b;https://b\x1b\\cd\x1b]8;;\x1b\\");
        assert_eq!(t.hyperlink_at(1, 1).unwrap().rows, [(1, 0..2)]);
        assert_eq!(t.hyperlink_at(1, 2).unwrap().uri, "https://b");
        // Scrolled back, the rows are the screen's.
        for _ in 0..4 {
            t.feed(b"\r\n");
        }
        assert_eq!(t.hyperlink_at(0, 6), None, "off the screen");
        t.scroll(t.history_size() as i32);
        assert_eq!(t.hyperlink_at(0, 6).unwrap().uri, "https://kawoosh.dev/x");
    }

    #[test]
    fn a_wrapped_line_is_one_text_across_its_rows() {
        let mut t = Terminal::headless(TermSize { rows: 5, cols: 10 });
        t.feed(b"$ ls\r\nsee https://kawoosh.dev/x ok\r\nnext");
        // Rows 1-3 are one line the terminal wrapped; row 0 and the
        // prompt's own row are not part of it.
        let w = t.wrapped_line(2);
        assert_eq!(w.text.trim_end(), "see https://kawoosh.dev/x ok");
        assert_eq!(t.wrapped_line(1), w);
        assert_eq!(t.wrapped_line(0).text.trim_end(), "$ ls");
        // `https://kawoosh.dev/x` is characters 4..25: the end of row 1,
        // all of row 2, the start of row 3.
        assert_eq!(w.char_at(2, 3), Some(13));
        assert_eq!(w.rows_of(4..25, 5), [(1, 4..10), (2, 0..10), (3, 0..5)]);
        // Scrolled off the top, its first row is left out.
        t.feed(b"\r\n\r\n");
        let w = t.wrapped_line(0);
        assert_eq!(w.text.trim_end(), "see https://kawoosh.dev/x ok");
        assert_eq!(w.rows_of(4..25, 5), [(0, 0..10), (1, 0..5)]);
    }

    #[test]
    fn a_wrapped_hyperlink_covers_both_rows() {
        let mut t = Terminal::headless(TermSize { rows: 3, cols: 10 });
        t.feed(b"see \x1b]8;;https://kawoosh.dev/x\x1b\\the long docs\x1b]8;;\x1b\\ now");
        let h = t.hyperlink_at(1, 2).unwrap();
        assert_eq!(h.uri, "https://kawoosh.dev/x");
        assert_eq!(h.rows, [(0, 4..10), (1, 0..7)]);
        assert_eq!(t.hyperlink_at(0, 5), Some(h));
    }

    #[test]
    fn a_file_link_is_a_path_here_or_on_the_domain() {
        let mut t = Terminal::headless(TermSize { rows: 3, cols: 30 });
        assert_eq!(
            t.file_link("file:///tmp/a%20b.rs#L12"),
            Some(("/tmp/a b.rs".into(), Some(12)))
        );
        assert_eq!(
            t.file_link("file://localhost/tmp/x#7"),
            Some(("/tmp/x".into(), Some(7)))
        );
        assert_eq!(
            t.file_link("file:///tmp/x#top"),
            Some(("/tmp/x".into(), None))
        );
        assert_eq!(t.file_link("file://elsewhere.example/tmp/x"), None);
        assert_eq!(t.file_link("https://kawoosh.dev"), None);
        t.set_domain("box", "box:/home".into());
        assert_eq!(
            t.file_link("file://elsewhere.example/tmp/x"),
            Some(("box:/tmp/x".into(), None)),
            "a host's shell names its own files"
        );
    }

    #[test]
    fn alt_screen_and_scrollback() {
        let mut t = Terminal::headless(TermSize { rows: 3, cols: 10 });
        for i in 0..6 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        assert_eq!(t.history_size(), 4);
        assert!(t.scrollback_text().starts_with("l0\nl1\n"));
        t.feed("日本$ ".as_bytes());
        assert_eq!(t.scrollback_cursor(), (6, 4), "the spacers not counted");
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
        // The `~` keys and the function keys under a modifier, as xterm
        // spells them (raw hands them to the program).
        let enc = |code: &str, ctrl: bool, alt: bool, shift: bool| {
            String::from_utf8(encode_key(code, None, ctrl, alt, shift, false).unwrap()).unwrap()
        };
        assert_eq!(enc("pageup", false, false, true), "\x1b[5;2~");
        assert_eq!(enc("delete", true, false, false), "\x1b[3;5~");
        assert_eq!(
            enc("pagedown", false, true, false),
            "\x1b[6;3~",
            "no second ESC"
        );
        assert_eq!(enc("f1", false, false, false), "\x1bOP");
        assert_eq!(enc("f1", false, false, true), "\x1b[1;2P");
        assert_eq!(enc("f12", true, false, true), "\x1b[24;6~");
    }

    /// kitty's keyboard protocol's flags (terminal-keys.md Decision 5):
    /// pushed and popped a stack a screen, the query answered, and a
    /// push past alacritty's cap no crash — its push there evicts from
    /// the title stack, which panics when that is empty.
    #[test]
    fn the_keyboard_flags_are_the_programs_a_stack_a_screen() {
        let mut t = Terminal::headless(TermSize { rows: 5, cols: 20 });
        assert_eq!(t.keyboard_flags(), 0);
        t.feed(b"\x1b[>1u");
        assert_eq!(t.keyboard_flags(), kitty::DISAMBIGUATE);
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_sent(), b"\x1b[?1u");
        t.feed(b"\x1b[>11u");
        assert_eq!(t.keyboard_flags(), 11);
        // The alternate screen has its own stack, empty; leaving it
        // brings the primary's back.
        t.feed(b"\x1b[?1049h");
        assert_eq!(t.keyboard_flags(), 0);
        t.feed(b"\x1b[>31u");
        assert_eq!(t.keyboard_flags(), 31);
        t.feed(b"\x1b[?1049l");
        assert_eq!(t.keyboard_flags(), 11);
        t.feed(b"\x1b[<u");
        assert_eq!(t.keyboard_flags(), 1);
        t.feed(b"\x1b[<u");
        assert_eq!(t.keyboard_flags(), 0);
        // Past the cap: set in place, no panic, and still popped.
        for _ in 0..KEYBOARD_STACK_MAX + 5 {
            t.feed(b"\x1b[>1u");
        }
        t.feed(b"\x1b[>9u");
        assert_eq!(t.keyboard_flags(), 9);
        t.feed(b"\x1b[<1u");
        assert_eq!(t.keyboard_flags(), 1);
        // A reset forgets it all.
        t.feed(b"\x1bc");
        assert_eq!(t.keyboard_flags(), 0);
    }
}
