//! A program's output read without a screen (compile.md Decision 12):
//! lines as a pipe gave them, their escape sequences taken out by the
//! parser the terminal reads with (vte) and the foregrounds they set
//! kept as paints over the text that is left — a buffer's, not a
//! grid's, so only what a buffer shows: the colour, and dim.

use std::ops::Range;

use alacritty_terminal::vte::ansi::{Attr, Color, Handler, NamedColor, Processor};

/// What a run of a line was printed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    /// One of the sixteen: the palette's, whichever it is when drawn.
    Ansi(u8),
    Rgb(u8, u8, u8),
    /// The default colour, dimmed.
    Dim,
}

/// The reader of one program's lines: the parser's state and the
/// attributes set run on from line to line, as a terminal's do.
#[derive(Default)]
pub struct Plain {
    parser: Processor,
    fg: Option<Paint>,
    dim: bool,
}

/// A line being read: its text and paints so far.
struct Line<'a> {
    fg: &'a mut Option<Paint>,
    dim: &'a mut bool,
    text: String,
    paints: Vec<(Range<usize>, Paint)>,
}

impl Handler for Line<'_> {
    fn input(&mut self, c: char) {
        let at = self.text.len();
        self.text.push(c);
        let Some(paint) = self.fg.or(self.dim.then_some(Paint::Dim)) else {
            return;
        };
        match self.paints.last_mut() {
            Some((r, p)) if r.end == at && *p == paint => r.end = self.text.len(),
            _ => self.paints.push((at..self.text.len(), paint)),
        }
    }

    fn put_tab(&mut self, count: u16) {
        for _ in 0..count {
            self.text.push('\t');
        }
    }

    /// A progress line drawn over itself: its last state.
    fn carriage_return(&mut self) {
        self.text.clear();
        self.paints.clear();
    }

    fn terminal_attribute(&mut self, attr: Attr) {
        match attr {
            Attr::Reset => (*self.fg, *self.dim) = (None, false),
            Attr::Dim => *self.dim = true,
            Attr::CancelBoldDim => *self.dim = false,
            Attr::Foreground(c) => {
                *self.fg = match c {
                    Color::Spec(c) => Some(Paint::Rgb(c.r, c.g, c.b)),
                    Color::Indexed(n @ 0..=15) => Some(Paint::Ansi(n)),
                    Color::Indexed(n) => {
                        let c = super::indexed(n, &super::Palette::default());
                        Some(Paint::Rgb((c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8))
                    }
                    Color::Named(n) if (n as usize) < 16 => Some(Paint::Ansi(n as u8)),
                    Color::Named(NamedColor::Foreground) => None,
                    Color::Named(_) => return,
                }
            }
            _ => {}
        }
    }
}

impl Plain {
    /// `line` as a buffer holds it — its escape sequences and control
    /// characters gone, what a carriage return wrote over dropped — and
    /// the paints of what is left, as byte ranges of it.
    pub fn read(&mut self, line: &str) -> (String, Vec<(Range<usize>, Paint)>) {
        let mut out = Line {
            fg: &mut self.fg,
            dim: &mut self.dim,
            text: String::with_capacity(line.len()),
            paints: Vec::new(),
        };
        self.parser.advance(&mut out, line.as_bytes());
        (out.text, out.paints)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> (String, Vec<(Range<usize>, Paint)>) {
        Plain::default().read(line)
    }

    #[test]
    fn colours_become_paints_over_the_text_left() {
        let (text, paints) = one("\x1b[1m\x1b[31merror\x1b[0m: no \x1b[94mx\x1b[39m here");
        assert_eq!(text, "error: no x here");
        assert_eq!(paints, [(0..5, Paint::Ansi(1)), (10..11, Paint::Ansi(12))]);
    }

    #[test]
    fn the_256_and_true_colours_and_dim() {
        let (text, paints) = one("\x1b[38;5;196ma\x1b[38;2;1;2;3mb\x1b[0;2mc\x1b[22md");
        assert_eq!(text, "abcd");
        assert_eq!(
            paints,
            [
                (0..1, Paint::Rgb(255, 0, 0)),
                (1..2, Paint::Rgb(1, 2, 3)),
                (2..3, Paint::Dim),
            ]
        );
        // A background's colour is read past, not taken for more codes.
        let (text, paints) = one("\x1b[48;5;31;32mok");
        assert_eq!(text, "ok");
        assert_eq!(paints, [(0..2, Paint::Ansi(2))]);
        assert_eq!(one("\x1b[38;5;3my").1, [(0..1, Paint::Ansi(3))]);
        assert_eq!(
            one("\x1b[38;5;244my").1,
            [(0..1, Paint::Rgb(128, 128, 128))]
        );
    }

    #[test]
    fn the_state_runs_on_to_the_next_line() {
        let mut plain = Plain::default();
        plain.read("\x1b[33mwarning");
        let (text, paints) = plain.read("still\x1b[m plain");
        assert_eq!(text, "still plain");
        assert_eq!(paints, [(0..5, Paint::Ansi(3))]);
    }

    #[test]
    fn other_sequences_and_overwritten_text_are_dropped() {
        let (text, paints) =
            one("\x1b]8;;http://x\x1b\\link\x1b]8;;\x07 \x1b[2K\x1b[?25l\x1b(Bé\x07");
        assert_eq!((text.as_str(), paints.len()), ("link é", 0));
        let (text, paints) = one("\x1b[32m 10%\r 50%\r\x1b[31mdone");
        assert_eq!(text, "done");
        assert_eq!(paints, [(0..4, Paint::Ansi(1))]);
        assert_eq!(one("a\tb").0, "a\tb");
    }
}
