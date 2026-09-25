//! Named palettes (roadmap step 28): a theme that holds still. A name
//! in `theme.name` sets the three things that have to agree — kui's
//! roles for the chrome, the syntax tokens for the code, and the ANSI
//! sixteen for a terminal — from one set of colours, a dark and a light
//! variant for `theme.appearance = "system"` to flip between. The
//! default is pinned rather than derived from the OS, so no machine's
//! accent reaches the selection; `theme.name = "system"` is the old way
//! (kui's roles off the OS, `palette.rs`'s hues, Tomorrow's sixteen).
//!
//! The one family shipped is Rosé Pine (rosepinetheme.com): its code
//! has no green — pine, foam, iris, gold, rose, love — which is what
//! the todo asked of a theme first. `rose-pine` is main and dawn,
//! `rose-pine-moon` moon and dawn.

use kawoosh_systems::ts::Token;
use kui::{Appearance, Color, Theme};

/// One variant's colours, by Rosé Pine's own names.
#[derive(Clone, Copy, Debug)]
pub struct Flavour {
    pub dark: bool,
    pub base: u32,
    pub surface: u32,
    pub overlay: u32,
    pub muted: u32,
    pub subtle: u32,
    pub text: u32,
    pub love: u32,
    pub gold: u32,
    pub rose: u32,
    pub pine: u32,
    pub foam: u32,
    pub iris: u32,
    pub hl_low: u32,
    pub hl_med: u32,
    pub hl_high: u32,
}

/// A palette by name: the variant for each base.
#[derive(Clone, Copy, Debug)]
pub struct Named {
    pub name: &'static str,
    pub dark: &'static Flavour,
    pub light: &'static Flavour,
}

pub const MAIN: Flavour = Flavour {
    dark: true,
    base: 0x191724,
    surface: 0x1f1d2e,
    overlay: 0x26233a,
    muted: 0x6e6a86,
    subtle: 0x908caa,
    text: 0xe0def4,
    love: 0xeb6f92,
    gold: 0xf6c177,
    rose: 0xebbcba,
    pine: 0x31748f,
    foam: 0x9ccfd8,
    iris: 0xc4a7e7,
    hl_low: 0x21202e,
    hl_med: 0x403d52,
    hl_high: 0x524f67,
};

pub const MOON: Flavour = Flavour {
    dark: true,
    base: 0x232136,
    surface: 0x2a273f,
    overlay: 0x393552,
    muted: 0x6e6a86,
    subtle: 0x908caa,
    text: 0xe0def4,
    love: 0xeb6f92,
    gold: 0xf6c177,
    rose: 0xea9a97,
    pine: 0x3e8fb0,
    foam: 0x9ccfd8,
    iris: 0xc4a7e7,
    hl_low: 0x2a283e,
    hl_med: 0x44415a,
    hl_high: 0x56526e,
};

pub const DAWN: Flavour = Flavour {
    dark: false,
    base: 0xfaf4ed,
    surface: 0xfffaf3,
    overlay: 0xf2e9e1,
    muted: 0x9893a5,
    subtle: 0x797593,
    text: 0x575279,
    love: 0xb4637a,
    gold: 0xea9d34,
    rose: 0xd7827e,
    pine: 0x286983,
    foam: 0x56949f,
    iris: 0x907aa9,
    hl_low: 0xf4ede8,
    hl_med: 0xdfdad9,
    hl_high: 0xcecacd,
};

/// Every name `theme.name` takes besides `system`, the default first.
pub const NAMED: &[Named] = &[
    Named {
        name: "rose-pine",
        dark: &MAIN,
        light: &DAWN,
    },
    Named {
        name: "rose-pine-moon",
        dark: &MOON,
        light: &DAWN,
    },
];

/// The palette `theme.name` names; none for `system` or a stranger.
pub fn named(name: &str) -> Option<&'static Named> {
    NAMED.iter().find(|n| n.name == name)
}

fn c(rgb: u32) -> Color {
    Color::hex((rgb << 8) | 0xFF)
}

impl Named {
    pub fn flavour(&self, dark: bool) -> &'static Flavour {
        if dark { self.dark } else { self.light }
    }
}

impl Flavour {
    /// kui's roles from the variant: the page on `base`, panels on
    /// `surface`, floats on `overlay`; the text, `subtle` and `muted`
    /// for the two quieter greys; `iris` the accent; the selection the
    /// highlight Rosé Pine gives a visual selection, translucent so the
    /// glyphs keep their colours; the states in the family's own hues
    /// (success in foam, as there is no green).
    pub fn theme(&self) -> Theme {
        let base = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let appearance = if self.dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        let t = Theme { appearance, ..base }.with_accent(c(self.iris));
        Theme {
            bg: c(self.base),
            surface: c(self.surface),
            raised: if self.dark {
                c(self.overlay)
            } else {
                c(self.surface)
            },
            sunken: if self.dark {
                c(self.base).mix(Color::BLACK, 0.18)
            } else {
                c(self.overlay)
            },
            border: if self.dark {
                c(self.overlay)
            } else {
                c(self.hl_med)
            },
            border_strong: if self.dark {
                c(self.hl_med)
            } else {
                c(self.hl_high)
            },
            fg: c(self.text),
            muted: c(self.subtle),
            faint: c(self.muted),
            selection: c(self.hl_high).with_alpha(if self.dark { 0.7 } else { 0.6 }),
            success: c(self.foam),
            warning: c(self.gold),
            danger: c(self.love),
            ..t
        }
    }

    /// A syntax token's hue, Rosé Pine's own mapping: keywords pine,
    /// functions rose, types foam, strings and numbers gold, comments
    /// muted, the punctuation subtle; plain text and variables none.
    pub fn syntax(&self, token: Token) -> Option<Color> {
        use Token as T;
        let hue = match token {
            T::Plain | T::Variable => return None,
            T::Keyword => self.pine,
            T::Function => self.rose,
            T::Type | T::Constructor | T::Property | T::Label | T::Tag => self.foam,
            T::String | T::Number | T::Constant => self.gold,
            T::Comment => self.muted,
            T::Operator | T::Punctuation => self.subtle,
            T::Attribute | T::Macro => self.iris,
            T::Heading => self.iris,
            T::Strong => self.gold,
            T::Emphasis => self.rose,
            T::Link => self.foam,
            T::Raw => self.gold,
            T::Added => self.foam,
            T::Removed => self.love,
        };
        Some(c(hue))
    }

    /// The terminal's sixteen as Rosé Pine's own terminal ports set
    /// them: "green" is pine and "cyan" rose, so a shell's green `ls`
    /// is not green; the bright eight the same hues, black a step up.
    pub fn ansi(&self) -> [u32; 16] {
        let rgba = |x: u32| (x << 8) | 0xFF;
        let eight = [
            self.overlay,
            self.love,
            self.pine,
            self.gold,
            self.foam,
            self.iris,
            self.rose,
            self.text,
        ];
        let mut out = [0; 16];
        for i in 0..8 {
            out[i] = rgba(eight[i]);
            out[i + 8] = rgba(eight[i]);
        }
        out[8] = rgba(self.muted);
        out
    }
}

/// The selection held legible whatever made it (roadmap step 28): a
/// system accent, a `theme.accent`, a `selection` role. Glyphs are drawn
/// over it in their own colours, so what is checked is the text over
/// the selection laid on the page — body text at 4.5:1, and every other
/// ink at 2:1 where it had that much on the page — and the selection's
/// alpha is stepped down until they clear, or to a trace.
pub fn legible_selection(mut t: Theme, inks: &[Color]) -> Theme {
    let mut sel = t.selection;
    let passes = |s: Color| {
        let under = t.bg.mix(Color::rgba(s.r, s.g, s.b, 1.0), s.a);
        t.fg.contrast(under) >= 4.5
            && inks
                .iter()
                .all(|i| i.contrast(t.bg) < 2.0 || i.contrast(under) >= 2.0)
    };
    while !passes(sel) && sel.a > 0.12 {
        sel = sel.with_alpha(sel.a * 0.85);
    }
    t.selection = sel;
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_first_and_every_name_resolves() {
        assert_eq!(NAMED[0].name, "rose-pine");
        for n in NAMED {
            assert_eq!(named(n.name).map(|m| m.name), Some(n.name));
            assert!(n.dark.dark && !n.light.dark);
        }
        assert!(named("system").is_none());
    }

    #[test]
    fn rose_pine_has_no_green_and_its_text_reads() {
        for f in [&MAIN, &MOON, &DAWN] {
            let t = f.theme();
            assert!(t.fg.contrast(t.bg) >= 4.5, "{f:?}");
            assert!(t.muted.contrast(t.bg) >= 3.0, "{f:?}");
            // The selection already reads under the body text.
            let kept = legible_selection(t, &[]);
            assert_eq!(kept.selection, t.selection, "{f:?}");
            for tok in Token::ALL {
                if let Some(h) = f.syntax(*tok) {
                    // No hue whose green channel leads both others by
                    // much: the todo's "i don't like green text".
                    let green = h.g > h.r + 0.15 && h.g > h.b + 0.15;
                    assert!(!green, "{tok:?} is green in {f:?}");
                }
            }
            assert_eq!(f.ansi()[2], (f.pine << 8) | 0xFF);
        }
    }

    #[test]
    fn an_accent_that_would_hide_the_text_is_washed_down() {
        // A pale accent over the dark base at kui's 0.40: the page
        // under white text turns pale, and the body text goes.
        let t = Theme::dark().with_accent(Color::hex(0xfff3a0ff));
        let under = |t: &Theme| {
            let s = t.selection;
            t.bg.mix(Color::rgba(s.r, s.g, s.b, 1.0), s.a)
        };
        assert!(t.fg.contrast(under(&t)) < 4.5);
        let fixed = legible_selection(t, &[]);
        assert!(fixed.fg.contrast(under(&fixed)) >= 4.5);
        assert!(fixed.selection.a < t.selection.a && fixed.selection.a > 0.1);
    }
}
