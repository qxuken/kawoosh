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
use alacritty_terminal::vte::ansi::{Color, CursorShape as VtCursor, NamedColor, Processor, Rgb};
use anyhow::{Context as _, Result};
use kui_core::cells::{Cell, CursorShape, flags};
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

/// The screen, ready for `ui.cells`: the app owns the `Vec` for a frame.
pub struct Screen {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Cell>,
    pub cursor: Option<(usize, usize, CursorShape)>,
    /// The absolute session line of the top row.
    pub origin_line: u64,
}

/// Default colours a screen is painted with, from the theme.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub fg: u32,
    pub bg: u32,
    /// The 16 ANSI colours as 0xRRGGBBAA.
    pub ansi: [u32; 16],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            fg: 0xE6E6E6FF,
            bg: 0x1D1F21FF,
            ansi: ANSI,
        }
    }
}

pub struct Terminal {
    term: Term<Proxy>,
    parser: Processor,
    pty: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    events: Receiver<Event>,
    size: TermSize,
    pub title: String,
    /// Set by an OSC 7 or the shell's cwd report, for `gf` and `:tool`.
    pub cwd: Option<std::path::PathBuf>,
    pub bell: bool,
    exited: bool,
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
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("opening pty")?;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut builder = match cmd {
            Some(c) => {
                let mut b = CommandBuilder::new(&shell);
                b.args(["-lc", c]);
                b
            }
            None => {
                let mut b = CommandBuilder::new(&shell);
                b.arg("-l");
                b
            }
        };
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
        let term = Term::new(config(), &size, Proxy { tx });
        Ok((
            Self {
                term,
                parser: Processor::new(),
                pty: Some(pair.master),
                writer: Some(writer),
                child: Some(child),
                events,
                size,
                title: String::new(),
                cwd: cwd.map(Into::into),
                bell: false,
                exited: false,
            },
            reader,
        ))
    }

    /// A terminal with no process behind it: tests feed it bytes.
    pub fn headless(size: TermSize) -> Self {
        let (tx, events) = channel();
        Self {
            term: Term::new(config(), &size, Proxy { tx }),
            parser: Processor::new(),
            pty: None,
            writer: None,
            child: None,
            events,
            size,
            title: String::new(),
            cwd: None,
            bell: false,
            exited: false,
        }
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
    /// written back, and the terminal's own events read out.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        self.drain_events();
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                Event::PtyWrite(text) => self.input(text.as_bytes()),
                Event::Title(t) => self.title = t,
                Event::ResetTitle => self.title.clear(),
                Event::Bell => self.bell = true,
                Event::ChildExit(_) => self.exited = true,
                _ => {}
            }
        }
    }

    /// Bytes to the process.
    pub fn input(&mut self, bytes: &[u8]) {
        if let Some(writer) = &mut self.writer {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
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
        self.term.resize(size);
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

    pub fn wants_mouse(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// How far into history the view is scrolled.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    /// The screen as cells, painted with `pal`.
    pub fn screen(&self, pal: &Palette) -> Screen {
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
                    VtCursor::Beam => CursorShape::Bar,
                    VtCursor::Underline => CursorShape::Underline,
                    _ => CursorShape::Block,
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

fn config() -> Config {
    Config {
        scrolling_history: 10_000,
        ..Config::default()
    }
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

pub const ANSI: [u32; 16] = [
    0x1D1F21FF, 0xCC6666FF, 0xB5BD68FF, 0xF0C674FF, 0x81A2BEFF, 0xB294BBFF, 0x8ABEB7FF, 0xC5C8C6FF,
    0x666666FF, 0xD54E53FF, 0xB9CA4AFF, 0xE7C547FF, 0x7AA6DAFF, 0xC397D8FF, 0x70C0B1FF, 0xEAEAEAFF,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(term: &Terminal) -> Vec<String> {
        let s = term.screen(&Palette::default());
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

    #[test]
    fn output_cursor_and_colours() {
        let mut t = Terminal::headless(TermSize { rows: 5, cols: 20 });
        t.feed(b"hello\r\n\x1b[31mworld\x1b[0m");
        let s = t.screen(&Palette::default());
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
        let s = t.screen(&Palette::default());
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
