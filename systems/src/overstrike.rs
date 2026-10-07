//! Text a terminal pager would style, read into plain text and its
//! styles: the overstrikes `man` and `nroff` print (`c\bc` bold, `_\bc`
//! underline) and the SGR sequences in their place (`ESC[1m`,
//! `ESC[4m`). What `kawoosh.overstrike` answers; `man.lua` reads a page
//! through it (docs/design/man.md, docs/design/lua-boundary.md).

/// A character's style, as bits.
pub const BOLD: u8 = 1;
pub const UNDERLINE: u8 = 2;

/// What [`read`] made of a text.
#[derive(Debug, Default, PartialEq)]
pub struct Read {
    /// The text with no backspace and no escape in it, each line's
    /// trailing whitespace off (the formatter's, not the page's), and no
    /// line after the last newline.
    pub text: Vec<u8>,
    /// Runs of one style, `(from, to, style)` in bytes of `text` (end
    /// exclusive), in order; unstyled runs left out.
    pub spans: Vec<(usize, usize, u8)>,
    /// The lines (from 0) whose every character but spaces is bold, and
    /// that have one: a page's heads.
    pub bold_lines: Vec<usize>,
}

/// The byte length of the UTF-8 character `b` leads.
fn char_len(b: u8) -> usize {
    match b {
        0..0x80 => 1,
        0x80..0xE0 => 2,
        0xE0..0xF0 => 3,
        _ => 4,
    }
}

/// One line's characters (its own bytes, or the bullet's) and a style
/// each.
fn read_line(line: &[u8]) -> (Vec<&[u8]>, Vec<u8>) {
    let (mut chars, mut styles) = (Vec::new(), Vec::new());
    let (mut bold, mut under) = (false, false);
    let n = line.len();
    let ch_at = |i: usize| -> &[u8] {
        let len = line.get(i).map_or(0, |b| char_len(*b));
        &line[i..(i + len).min(n)]
    };
    let mut i = 0;
    while i < n {
        let b = line[i];
        if b == 0x1b {
            // An escape: a CSI ending in `m` sets the style, any other is
            // dropped with its final byte; `ESC ( B` and the like with
            // their two; a two-byte escape otherwise.
            let mut j = i + 1;
            if line.get(j) == Some(&b'[') {
                j += 1;
                let from = j;
                while j < n && (line[j] == b';' || line[j].is_ascii_digit()) {
                    j += 1;
                }
                if line.get(j) == Some(&b'm') {
                    let params = &line[from..j];
                    if params.is_empty() {
                        (bold, under) = (false, false);
                    }
                    for word in params.split(|c| *c == b';').filter(|w| !w.is_empty()) {
                        match word {
                            b"0" => (bold, under) = (false, false),
                            b"1" => bold = true,
                            b"4" => under = true,
                            b"22" => bold = false,
                            b"24" => under = false,
                            _ => {}
                        }
                    }
                }
                i = j + 1;
            } else {
                let c = line.get(j).copied();
                i = if c == Some(b'(') || c == Some(b')') {
                    j + 2
                } else {
                    j + 1
                };
            }
        } else if b == 0x08 {
            // A backspace with nothing before it: dropped.
            i += 1;
        } else {
            let mut ch = ch_at(i);
            i += ch.len();
            let mut style = (if bold { BOLD } else { 0 }) | (if under { UNDERLINE } else { 0 });
            // Overstrikes: the character struck over by the next, any
            // number of times.
            while line.get(i) == Some(&0x08) && i + 1 < n {
                let over = ch_at(i + 1);
                i += 1 + over.len();
                if over == ch {
                    style |= BOLD;
                } else if ch == b"_" {
                    style |= UNDERLINE;
                    ch = over;
                } else if over == b"_" {
                    style |= UNDERLINE;
                } else {
                    // `+\bo`: groff's bullet; anything else, the last
                    // wins, bold.
                    ch = if ch == b"+" && over == b"o" {
                        "•".as_bytes()
                    } else {
                        over
                    };
                    style |= BOLD;
                }
            }
            chars.push(ch);
            styles.push(style);
        }
    }
    (chars, styles)
}

/// `raw`, as a pager would show it, read into plain text and styles.
pub fn read(raw: &[u8]) -> Read {
    let mut out = Read::default();
    let mut lines: Vec<&[u8]> = raw.split(|b| *b == b'\n').collect();
    // What follows the last newline is no line when it is empty.
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    for (ln, line) in lines.iter().enumerate() {
        if ln > 0 {
            out.text.push(b'\n');
        }
        let offset = out.text.len();
        let (chars, styles) = read_line(line);
        // Trailing whitespace is the formatter's.
        let mut keep = chars.len();
        while keep > 0 && chars[keep - 1].iter().all(|b| b.is_ascii_whitespace()) {
            keep -= 1;
        }
        let (mut run_from, mut run_style) = (0, 0u8);
        let (mut all_bold, mut any) = (true, false);
        for (ch, &st) in chars[..keep].iter().zip(&styles[..keep]) {
            let at = out.text.len() - offset;
            if !ch.iter().all(|b| b.is_ascii_whitespace()) {
                any = true;
                all_bold &= st & BOLD != 0;
            }
            if st != run_style {
                if run_style != 0 && at > run_from {
                    out.spans.push((offset + run_from, offset + at, run_style));
                }
                (run_from, run_style) = (at, st);
            }
            out.text.extend_from_slice(ch);
        }
        let at = out.text.len() - offset;
        if run_style != 0 && at > run_from {
            out.spans.push((offset + run_from, offset + at, run_style));
        }
        if any && all_bold {
            out.bold_lines.push(ln);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(r: &Read) -> &str {
        std::str::from_utf8(&r.text).unwrap()
    }

    /// Overstrikes bold and underline, the bullet, a stray backspace,
    /// trailing spaces off, and a heads' line found.
    #[test]
    fn overstrikes_are_read_into_text_and_spans() {
        let r = read(
            b"N\x08NA\x08AM\x08ME\x08E\n     _\x08f_\x08i_\x08l_\x08e  x  \n+\x08o item\x08\n",
        );
        assert_eq!(text(&r), "NAME\n     file  x\n• item");
        assert_eq!(
            r.spans,
            vec![(0, 4, BOLD), (10, 14, UNDERLINE), (18, 21, BOLD)]
        );
        assert_eq!(r.bold_lines, vec![0]);
    }

    /// SGR sets and resets the style; other escapes are dropped.
    #[test]
    fn sgr_sequences_style_and_other_escapes_go() {
        let r = read(b"a\x1b[1mbold\x1b[22m \x1b[4mund\x1b[0m\x1b(Bz\x1b[2Kq\n");
        assert_eq!(text(&r), "abold undzq");
        assert_eq!(r.spans, vec![(1, 5, BOLD), (6, 9, UNDERLINE)]);
        assert!(r.bold_lines.is_empty());
    }

    /// Bold and underline at once; multibyte characters struck over;
    /// a blank line is no head.
    #[test]
    fn styles_combine_and_characters_are_whole() {
        let r = read("_\x08ж\x08ж\n\n  \x1b[1mНАЗВАНИЕ\x1b[0m".as_bytes());
        assert_eq!(text(&r), "ж\n\n  НАЗВАНИЕ");
        assert_eq!(r.spans, vec![(0, 2, BOLD | UNDERLINE), (6, 22, BOLD)]);
        assert_eq!(r.bold_lines, vec![0, 2]);
    }
}
