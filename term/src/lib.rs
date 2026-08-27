//! Facade over the terminal model (docs/design/mvp.md, Decision 3).
//!
//! `alacritty_terminal` supplies the state machine, `portable-pty` the pty.
//! The facade keeps both swappable: the rest of kawoosh sees a `Terminal`
//! with cells, a cursor, input, resize, and scrollback — nothing else. The
//! byte reader is handed to the caller's io system; feeding bytes back in is
//! a plain method call, so tests can drive a `Terminal` with no pty at all
//! (see the tests below — that seam is deliberate).

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};
use anyhow::{Context as _, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

#[derive(Clone, Copy, Debug)]
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

/// Forwards the model's pty-write requests (query answers) to the writer.
struct Proxy {
    tx: Sender<Vec<u8>>,
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            let _ = self.tx.send(text.into_bytes());
        }
    }
}

/// One cell as the renderer sees it.
#[derive(Clone, Copy, Debug)]
pub struct CellView {
    pub c: char,
    pub fg: [u8; 4],
    /// `None` means the default background (draw nothing).
    pub bg: Option<[u8; 4]>,
    pub bold: bool,
}

pub struct Terminal {
    term: Term<Proxy>,
    parser: Processor,
    /// Absent in headless (test) terminals.
    pty: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    replies: Receiver<Vec<u8>>,
    size: TermSize,
}

impl Terminal {
    /// Spawn the user's shell in a fresh pty. Returns the terminal and the
    /// raw byte reader for the caller's io thread.
    pub fn spawn(
        cwd: Option<&std::path::Path>,
        size: TermSize,
        envs: &[(&str, String)],
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
        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        for (key, value) in envs {
            cmd.env(key, value);
        }
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("spawning shell in pty")?;
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .context("cloning pty reader")?;
        let writer = pair.master.take_writer().context("taking pty writer")?;

        let (tx, replies) = channel();
        let term = Term::new(
            Config {
                scrolling_history: 10_000,
                ..Config::default()
            },
            &size,
            Proxy { tx },
        );

        Ok((
            Self {
                term,
                parser: Processor::new(),
                pty: Some(pair.master),
                writer: Some(writer),
                child: Some(child),
                replies,
                size,
            },
            reader,
        ))
    }

    /// A pty-less terminal: bytes in via [`Terminal::feed`], nothing out.
    /// The test seam the facade exists for.
    pub fn headless(size: TermSize) -> Self {
        let (tx, replies) = channel();
        Self {
            term: Term::new(
                Config {
                    scrolling_history: 10_000,
                    ..Config::default()
                },
                &size,
                Proxy { tx },
            ),
            parser: Processor::new(),
            pty: None,
            writer: None,
            child: None,
            replies,
            size,
        }
    }

    pub fn size(&self) -> TermSize {
        self.size
    }

    pub fn is_running(&mut self) -> bool {
        match &mut self.child {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Feed pty output bytes into the model, then flush any replies the
    /// model wants written back (terminal queries).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        if let Some(writer) = &mut self.writer {
            while let Ok(reply) = self.replies.try_recv() {
                let _ = writer.write_all(&reply);
            }
            let _ = writer.flush();
        }
    }

    /// User input for the shell.
    pub fn input(&mut self, bytes: &[u8]) {
        if let Some(writer) = &mut self.writer {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
        }
    }

    pub fn resize(&mut self, size: TermSize) {
        if size.rows == self.size.rows && size.cols == self.size.cols {
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

    /// Cursor position in viewport coordinates, if visible.
    pub fn cursor(&self) -> Option<(usize, usize)> {
        let point = self.term.grid().cursor.point;
        let row = point.line.0 + self.term.grid().display_offset() as i32;
        usize::try_from(row).ok().map(|row| (row, point.column.0))
    }

    /// Visit every viewport cell: `(row, col, cell)`.
    pub fn for_each_cell(&self, mut f: impl FnMut(usize, usize, CellView)) {
        let display_offset = self.term.grid().display_offset() as i32;
        for indexed in self.term.grid().display_iter() {
            let row = indexed.point.line.0 + display_offset;
            let Ok(row) = usize::try_from(row) else {
                continue;
            };
            let cell = &*indexed;
            if cell.flags.contains(Flags::HIDDEN) {
                continue;
            }

            let inverse = cell.flags.contains(Flags::INVERSE);
            let mut fg = color_to_rgba(cell.fg, DEFAULT_FG);
            let mut bg = match cell.bg {
                Color::Named(NamedColor::Background) => None,
                other => Some(color_to_rgba(other, DEFAULT_BG)),
            };
            if inverse {
                let old_fg = fg;
                fg = bg.unwrap_or(DEFAULT_BG);
                bg = Some(old_fg);
            }

            f(
                row,
                indexed.point.column.0,
                CellView {
                    c: cell.c,
                    fg,
                    bg,
                    bold: cell.flags.contains(Flags::BOLD),
                },
            );
        }
    }

    /// Materialize scrollback + viewport as text, one line per row, trailing
    /// blanks trimmed — the buffer for Decision 3's scrollback-to-buffer.
    pub fn scrollback_text(&self) -> Vec<u8> {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let mut out = Vec::new();

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
            let trimmed = row.trim_end();
            out.extend_from_slice(trimmed.as_bytes());
            out.push(b'\n');
        }

        // Trim trailing empty lines.
        while out.ends_with(b"\n\n") {
            out.pop();
        }
        out
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless() -> Terminal {
        Terminal::headless(TermSize { rows: 5, cols: 20 })
    }

    fn viewport_text(term: &Terminal) -> Vec<String> {
        let mut rows = vec![String::new(); term.size().rows as usize];
        term.for_each_cell(|row, _col, cell| {
            rows[row].push(cell.c);
        });
        rows.into_iter().map(|r| r.trim_end().to_string()).collect()
    }

    #[test]
    fn parses_plain_output_and_cursor() {
        let mut term = headless();
        term.feed(b"hello\r\nworld");

        let rows = viewport_text(&term);
        assert_eq!(rows[0], "hello");
        assert_eq!(rows[1], "world");
        assert_eq!(term.cursor(), Some((1, 5)));
    }

    #[test]
    fn sgr_colors_reach_cells() {
        let mut term = headless();
        term.feed(b"\x1b[31mred\x1b[0m");

        let mut first_fg = None;
        term.for_each_cell(|row, col, cell| {
            if row == 0 && col == 0 {
                first_fg = Some(cell.fg);
            }
        });
        let [r, g, b] = ANSI[1];
        assert_eq!(first_fg, Some([r, g, b, 0xFF]));
    }

    #[test]
    fn alternate_screen_and_back() {
        let mut term = headless();
        term.feed(b"shell line\r\n");
        term.feed(b"\x1b[?1049h"); // enter alt screen (full-screen TUIs)
        term.feed(b"\x1b[2J\x1b[HTUI");
        assert_eq!(viewport_text(&term)[0], "TUI");

        term.feed(b"\x1b[?1049l"); // leave
        assert_eq!(viewport_text(&term)[0], "shell line");
    }

    #[test]
    fn scrollback_materializes() {
        let mut term = headless();
        for i in 0..12 {
            term.feed(format!("line {i}\r\n").as_bytes());
        }
        let text = String::from_utf8(term.scrollback_text()).unwrap();
        // 12 lines with a 5-row viewport: early lines live in scrollback.
        assert!(text.contains("line 0\n"), "scrollback kept: {text:?}");
        assert!(text.contains("line 11\n"));
    }

    #[test]
    fn resize_reflows_model() {
        let mut term = headless();
        term.feed(b"abc");
        term.resize(TermSize { rows: 10, cols: 40 });
        assert_eq!(term.size().rows, 10);
        assert_eq!(viewport_text(&term)[0], "abc");
    }
}

pub const DEFAULT_FG: [u8; 4] = [0xE6, 0xE6, 0xE6, 0xFF];
pub const DEFAULT_BG: [u8; 4] = [0x21, 0x21, 0x21, 0xFF];

/// The standard 16 colors, xterm-flavored.
const ANSI: [[u8; 3]; 16] = [
    [0x1D, 0x1F, 0x21], // black
    [0xCC, 0x66, 0x66], // red
    [0xB5, 0xBD, 0x68], // green
    [0xF0, 0xC6, 0x74], // yellow
    [0x81, 0xA2, 0xBE], // blue
    [0xB2, 0x94, 0xBB], // magenta
    [0x8A, 0xBE, 0xB7], // cyan
    [0xC5, 0xC8, 0xC6], // white
    [0x66, 0x66, 0x66], // bright black
    [0xD5, 0x4E, 0x53], // bright red
    [0xB9, 0xCA, 0x4A], // bright green
    [0xE7, 0xC5, 0x47], // bright yellow
    [0x7A, 0xA6, 0xDA], // bright blue
    [0xC3, 0x97, 0xD8], // bright magenta
    [0x70, 0xC0, 0xB1], // bright cyan
    [0xEA, 0xEA, 0xEA], // bright white
];

fn rgb(rgb: Rgb) -> [u8; 4] {
    [rgb.r, rgb.g, rgb.b, 0xFF]
}

fn indexed(index: u8) -> [u8; 4] {
    match index {
        0..=15 => {
            let [r, g, b] = ANSI[index as usize];
            [r, g, b, 0xFF]
        }
        16..=231 => {
            // 6x6x6 color cube.
            let index = index - 16;
            let step = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [
                step(index / 36),
                step(index % 36 / 6),
                step(index % 6),
                0xFF,
            ]
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            [gray, gray, gray, 0xFF]
        }
    }
}

fn color_to_rgba(color: Color, default: [u8; 4]) -> [u8; 4] {
    match color {
        Color::Spec(spec) => rgb(spec),
        Color::Indexed(index) => indexed(index),
        Color::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground => DEFAULT_FG,
            NamedColor::Background => DEFAULT_BG,
            NamedColor::Black => indexed(0),
            NamedColor::Red => indexed(1),
            NamedColor::Green => indexed(2),
            NamedColor::Yellow => indexed(3),
            NamedColor::Blue => indexed(4),
            NamedColor::Magenta => indexed(5),
            NamedColor::Cyan => indexed(6),
            NamedColor::White => indexed(7),
            NamedColor::BrightBlack => indexed(8),
            NamedColor::BrightRed => indexed(9),
            NamedColor::BrightGreen => indexed(10),
            NamedColor::BrightYellow => indexed(11),
            NamedColor::BrightBlue => indexed(12),
            NamedColor::BrightMagenta => indexed(13),
            NamedColor::BrightCyan => indexed(14),
            NamedColor::BrightWhite => indexed(15),
            _ => default,
        },
    }
}
