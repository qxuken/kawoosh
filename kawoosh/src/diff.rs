//! A diff as rows: a line each, a sign in the margin, the line washed
//! in the sign's colour — the shape every diff view has, whoever made
//! the lines. The undo pane shows a state against its parent with it;
//! a buffer against its file, or a commit, would be shown the same way.
//!
//! The input is data: [`Line`]s, each a kind and its text. A
//! [`kawoosh_doc::Hunk`] turns into them with [`lines_of`].

use std::borrow::Cow;

use kawoosh_doc::Hunk;
use kui_native::{Align, Color, NodeSpec, TextStyle, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Added,
    Removed,
    /// A line both sides have, shown for its bearings.
    Context,
    /// Lines not shown, told by count — the text says how many and
    /// which side; the row is drawn as a cut, not a line.
    Cut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line<'a> {
    pub kind: Kind,
    pub text: Cow<'a, str>,
}

impl<'a> Line<'a> {
    pub fn new(kind: Kind, text: &'a str) -> Self {
        Self {
            kind,
            text: Cow::Borrowed(text),
        }
    }

    /// A cut of `n` more lines of `side`'s kind.
    pub fn cut(n: usize, side: Kind) -> Self {
        let what = match side {
            Kind::Removed => "removed",
            Kind::Added => "added",
            _ => "",
        };
        Self {
            kind: Kind::Cut,
            text: Cow::Owned(format!(
                "… {} more {what} line{} not shown",
                count(n),
                if n == 1 { "" } else { "s" }
            )),
        }
    }
}

/// How the rows are drawn: a row's height, the inset and the gap of
/// the sign from the text, the text's style, and the colour of each
/// side — a line's wash is its colour at [`Style::WASH`].
#[derive(Clone, Debug)]
pub struct Style {
    pub row_h: f32,
    pub pad_x: f32,
    pub gap: f32,
    pub text: TextStyle,
    pub added: Color,
    pub removed: Color,
    /// A context line's sign.
    pub dim: Color,
}

impl Style {
    /// A line's wash: its colour, faint.
    pub const WASH: f32 = 0.12;
}

/// A hunk as lines: what it took out, then what it put in — each side
/// ending in a cut when it has lines past the ones carried.
pub fn lines_of(h: &Hunk) -> Vec<Line<'_>> {
    fn side<'a>(lines: &'a [String], total: usize, kind: Kind) -> Vec<Line<'a>> {
        let cut = total.saturating_sub(lines.len());
        lines
            .iter()
            .map(|t| Line::new(kind, t))
            .chain((cut > 0).then(|| Line::cut(cut, kind)))
            .collect()
    }
    let mut out = side(&h.old, h.old_total, Kind::Removed);
    out.extend(side(&h.new, h.new_total, Kind::Added));
    out
}

/// Draws `lines`, one row each, into the column open in `ui`.
pub fn rows<'a>(ui: &mut Ui<'_>, lines: impl IntoIterator<Item = Line<'a>>, style: &Style) {
    for l in lines {
        let (sign, color, wash) = match l.kind {
            Kind::Added => ("+", style.added, style.added.with_alpha(Style::WASH)),
            Kind::Removed => ("−", style.removed, style.removed.with_alpha(Style::WASH)),
            Kind::Context => (" ", style.dim, Color::TRANSPARENT),
            Kind::Cut => ("", style.dim, Color::TRANSPARENT),
        };
        ui.with(
            NodeSpec::row()
                .grow_width()
                .height(style.row_h)
                .pad_xy(style.pad_x, 0.0)
                .gap(style.gap)
                .cross_align(Align::Center)
                .bg(wash),
            |ui| {
                if l.kind == Kind::Cut {
                    // Dim, in the sign's place and on: a seam, not a line.
                    ui.text(&l.text, style.text.color(style.dim));
                } else {
                    ui.text(sign, style.text.color(color));
                    ui.text(&l.text, style.text);
                }
            },
        );
    }
}

/// A hunk's summary for a header: its first line and how many lines
/// it took out and put in — all of them, with how many are shown when
/// that is fewer.
pub fn summary(h: &Hunk) -> String {
    let mut s = format!(
        "line {} · −{} +{} lines",
        h.line,
        count(h.old_total),
        count(h.new_total)
    );
    if h.clipped() {
        s.push_str(&format!(" · first {} of each shown", Hunk::MAX_LINES));
    }
    s
}

/// A count with its thousands apart, for the eye.
pub fn count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hunk_is_its_removed_then_added_lines() {
        let h = Hunk {
            line: 3,
            old: vec!["a".into(), "b".into()],
            new: vec!["ab".into()],
            old_total: 2,
            new_total: 1,
        };
        let lines = lines_of(&h);
        assert_eq!(
            lines
                .iter()
                .map(|l| (l.kind, l.text.as_ref()))
                .collect::<Vec<_>>(),
            [
                (Kind::Removed, "a"),
                (Kind::Removed, "b"),
                (Kind::Added, "ab")
            ]
        );
        assert_eq!(summary(&h), "line 3 · −2 +1 lines");
        // A side with more than it carries ends in a cut that says how
        // many, and the summary counts them all.
        let h = Hunk {
            old_total: 10_745_771,
            new_total: 1,
            ..h
        };
        let lines = lines_of(&h);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[2].kind, Kind::Cut);
        assert_eq!(lines[2].text, "… 10 745 769 more removed lines not shown");
        assert_eq!(lines[3].kind, Kind::Added);
        assert_eq!(
            summary(&h),
            "line 3 · −10 745 771 +1 lines · first 200 of each shown"
        );
        assert_eq!(
            Line::cut(1, Kind::Added).text,
            "… 1 more added line not shown"
        );
    }
}
