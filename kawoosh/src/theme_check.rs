//! The theme check (docs/design/themes.md Decision 7): every pair of
//! colours the editor draws one over the other, measured. A pair is a
//! foreground and the background it lands on — a translucent wash (the
//! selection, a search hit) laid on the page first, as the rows lay it
//! — with the contrast between them and the floor it has to clear.
//! `:theme check` reports them, and `:theme lab` draws each where the
//! editor would.
//!
//! The floors, and why:
//!
//! - **4.5:1** ([`BODY`]) the body text, on every surface and under a
//!   selection or a search hit, and a label on a fill (the active tab,
//!   the accent's own text): WCAG AA for text.
//! - **3:1** ([`UI`]) what reads but is not the body — a syntax hue, the
//!   quieter grey, a state's colour, a mode's name on the strip, the
//!   terminal's hues — and what must be found at a glance as a thing,
//!   the caret: WCAG's floor for large text and for the parts of a
//!   control.
//! - **2:1** ([`INK`]) a syntax hue under a selection or a search hit,
//!   and the barely-there grey (a gutter's numbers): still told apart
//!   from what it is on, as `themes::legible_selection` holds the inks.
//! - **1.4:1** ([`themes::SEEN`]) a wash or a line that is seen, not
//!   read: the selection, a search hit and the strong border, each
//!   against the page it is on.

use kawoosh_systems::ts::Token;
use kui_native::{Color, Theme};

use crate::themes::{SEEN, Style, Variant};

pub const BODY: f32 = 4.5;
pub const UI: f32 = 3.0;
pub const INK: f32 = 2.0;

/// How strong a search hit's wash is over the page (`rows::emit_line`:
/// the warning's colour at this alpha).
pub const HIT_ALPHA: f32 = 0.35;

/// One pair measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    /// What part of the editor it is: `text`, `code`, `selection`,
    /// `search`, `caret`, `chrome`, `states`, `terminal`.
    pub group: &'static str,
    /// What is drawn over what, in words.
    pub what: String,
    /// The two colours as they meet, opaque.
    pub fg: Color,
    pub bg: Color,
    pub ratio: f32,
    pub need: f32,
}

impl Check {
    /// Whether it clears its floor (to the hundredth a report shows).
    pub fn ok(&self) -> bool {
        self.ratio + 0.005 >= self.need
    }
}

/// `wash` laid on `page`, opaque: what the eye sees under a selection.
pub fn under(page: Color, wash: Color) -> Color {
    page.mix(Color::rgba(wash.r, wash.g, wash.b, 1.0), wash.a)
}

/// Every pair of `theme`, with `syntax` the hue each token is painted
/// in (none for plain text) and `ansi` the terminal's sixteen.
pub fn run(theme: &Theme, syntax: impl Fn(Token) -> Option<Color>, ansi: [u32; 16]) -> Vec<Check> {
    let t = theme;
    let mut out = Vec::new();
    let mut add = |group: &'static str, what: String, fg: Color, bg: Color, need: f32| {
        let fg = under(bg, fg);
        out.push(Check {
            group,
            what,
            fg,
            bg,
            ratio: fg.contrast(bg),
            need,
        });
    };
    let surfaces = [
        ("page", t.bg),
        ("panel", t.surface),
        ("float", t.raised),
        ("well", t.sunken),
    ];
    for (name, s) in surfaces {
        add("text", format!("body on {name}"), t.fg, s, BODY);
    }
    for (name, s) in [surfaces[0], surfaces[1], surfaces[3]] {
        add("text", format!("muted on {name}"), t.muted, s, UI);
    }
    add("text", "faint on page".into(), t.faint, t.bg, INK);

    let sel = under(t.bg, t.selection);
    let hit = under(t.bg, t.warning.with_alpha(HIT_ALPHA));
    add(
        "selection",
        "selection seen on page".into(),
        sel,
        t.bg,
        SEEN,
    );
    add("selection", "body under selection".into(), t.fg, sel, BODY);
    add("search", "hit seen on page".into(), hit, t.bg, SEEN);
    add("search", "body under a hit".into(), t.fg, hit, BODY);
    for tok in Token::ALL {
        let Some(h) = syntax(*tok) else { continue };
        let name = tok.name();
        add("code", format!("{name} on page"), h, t.bg, UI);
        add("selection", format!("{name} under selection"), h, sel, INK);
        add("search", format!("{name} under a hit"), h, hit, INK);
    }

    // The block caret: the page's colour as its glyph, on the ring's.
    add("caret", "caret seen on page".into(), t.focus_ring, t.bg, UI);
    add(
        "caret",
        "glyph under the caret".into(),
        t.bg,
        t.focus_ring,
        UI,
    );

    add(
        "chrome",
        "active tab label".into(),
        t.on_accent,
        t.accent,
        BODY,
    );
    add("chrome", "inactive tab label".into(), t.muted, t.sunken, UI);
    add(
        "chrome",
        "strong border on page".into(),
        t.border_strong,
        t.bg,
        SEEN,
    );
    for (mode, c) in [
        ("NORMAL", t.focus_ring),
        ("INSERT", t.success),
        ("VISUAL", t.warning),
    ] {
        add("chrome", format!("{mode} on the strip"), c, t.sunken, UI);
    }

    for (state, c) in [
        ("error", t.danger),
        ("warning", t.warning),
        ("success", t.success),
    ] {
        add("states", format!("{state} on page"), c, t.bg, UI);
        add("states", format!("{state} on a float"), c, t.raised, UI);
    }

    // The terminal's hues on its page; black and white are the page's
    // own neighbours in every theme's sixteen, so not text to measure.
    const NAMES: [&str; 8] = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    let rgba = |x: u32| Color::hex(x);
    for (i, &c) in ansi.iter().enumerate() {
        let hue = i % 8;
        if hue == 0 || hue == 7 {
            continue;
        }
        let name = if i < 8 {
            NAMES[hue].to_string()
        } else {
            format!("bright {}", NAMES[hue])
        };
        add("terminal", format!("{name} on page"), rgba(c), t.bg, UI);
    }
    out
}

/// What a check is run on, whole: a title, kui's theme, and a hue and a
/// style per token (by `Token as usize`) and the sixteen — a variant as
/// it ships, or the look on show with the settings' accent, roles,
/// `tokens.colors` and `tokens.styles` over it.
#[derive(Clone, Debug)]
pub struct Subject {
    pub title: String,
    pub theme: Theme,
    pub syntax: Vec<Option<Color>>,
    pub styles: Vec<Style>,
    pub ansi: [u32; 16],
}

impl Default for Subject {
    fn default() -> Self {
        Subject {
            title: String::new(),
            theme: Theme::dark(),
            syntax: vec![None; Token::ALL.len()],
            styles: vec![Style::PLAIN; Token::ALL.len()],
            ansi: [0; 16],
        }
    }
}

impl Subject {
    /// A variant as it ships.
    pub fn of_variant(v: &Variant) -> Subject {
        Subject {
            title: format!("{} (as it ships)", v.name),
            theme: v.theme,
            syntax: Token::ALL.iter().map(|t| v.syntax(*t)).collect(),
            styles: Token::ALL.iter().map(|t| v.style(*t)).collect(),
            ansi: v.ansi,
        }
    }

    pub fn checks(&self) -> Vec<Check> {
        run(&self.theme, |t| self.syntax[t as usize], self.ansi)
    }

    pub fn report(&self) -> String {
        report(&self.title, &self.checks())
    }
}

/// A report as `:theme check` writes it: a head, then what falls short
/// first, then the rest, each `✗`/`✓`, its group, what, its ratio and
/// its floor.
pub fn report(title: &str, checks: &[Check]) -> String {
    let short = checks.iter().filter(|c| !c.ok()).count();
    let mut s = format!(
        "theme check · {title}\n{} pairs, {} below their floor\n",
        checks.len(),
        short
    );
    let w = checks
        .iter()
        .map(|c| c.what.chars().count())
        .max()
        .unwrap_or(0);
    for pass in [false, true] {
        let rows: Vec<&Check> = checks.iter().filter(|c| c.ok() == pass).collect();
        if rows.is_empty() {
            continue;
        }
        s.push('\n');
        for c in rows {
            s.push_str(&format!(
                "{} {:<9} {:<w$}  {:>5.2}:1  needs {}\n",
                if pass { "✓" } else { "✗" },
                c.group,
                c.what,
                c.ratio,
                c.need,
            ));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes;

    fn of(v: &themes::Variant) -> Vec<Check> {
        run(&v.theme, |t| v.syntax(t), v.ansi)
    }

    #[test]
    fn a_wash_is_measured_where_it_lands() {
        let t = Theme::dark();
        let checks = run(&t, |_| None, [0xffffffff; 16]);
        let sel = checks
            .iter()
            .find(|c| c.what == "body under selection")
            .unwrap();
        assert_eq!(sel.bg, under(t.bg, t.selection));
        assert!((sel.ratio - t.fg.contrast(sel.bg)).abs() < 1e-4);
        // No hue, no code rows; the terminal's hues are twelve.
        assert!(checks.iter().all(|c| c.group != "code"));
        assert_eq!(checks.iter().filter(|c| c.group == "terminal").count(), 12);
    }

    #[test]
    fn high_contrast_clears_every_floor() {
        for name in ["high-contrast-dark", "high-contrast-light"] {
            let short: Vec<String> = of(themes::variant(name).unwrap())
                .iter()
                .filter(|c| !c.ok())
                .map(|c| format!("{} {:.2}", c.what, c.ratio))
                .collect();
            assert!(short.is_empty(), "{name}: {short:?}");
        }
    }

    #[test]
    fn the_report_puts_what_falls_short_first() {
        let checks = vec![
            Check {
                group: "text",
                what: "body on page".into(),
                fg: Color::WHITE,
                bg: Color::BLACK,
                ratio: 21.0,
                need: BODY,
            },
            Check {
                group: "code",
                what: "string on page".into(),
                fg: Color::WHITE,
                bg: Color::WHITE,
                ratio: 1.0,
                need: UI,
            },
        ];
        let r = report("x", &checks);
        assert!(
            r.starts_with("theme check · x\n2 pairs, 1 below their floor\n"),
            "{r}"
        );
        let bad = r.find("✗ code").unwrap();
        let good = r.find("✓ text").unwrap();
        assert!(bad < good, "{r}");
        assert!(r.contains(" 1.00:1  needs 3"), "{r}");
    }
}
