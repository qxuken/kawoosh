//! The themes (roadmap steps 28 and 40, docs/design/themes.md): a
//! theme that holds still. A **variant** is one base's colours whole —
//! kui's roles for the chrome, a hue per syntax token for the code and
//! the ANSI sixteen for a terminal, from one set of colours — and a
//! **family** names a dark variant and a light one for `theme.appearance
//! = "system"` to flip between. `theme.name` picks a family,
//! `theme.dark` and `theme.light` a variant for each base apart from it
//! ([`resolve`]); `system` in any of them is kui's roles off the OS
//! (`palette.rs`'s hues, Tomorrow's sixteen). The default is pinned
//! rather than derived, so no machine's accent reaches the selection.
//!
//! Each family is written in its own vocabulary and turned into
//! variants once ([`variants`]): Rosé Pine (rosepinetheme.com), whose
//! code has no green — pine, foam, iris, gold, rose, love — which is
//! what the todo asked of a theme first; Ayu (ayu-colors v5), its
//! translucent hues laid flat on their page, strings green as Ayu has
//! them; Gruvbox (morhetz), dark and light at three grades; Tokyo Night
//! (folke), night, storm and moon with day; and a high-contrast pair of
//! kawoosh's own, drawn to WCAG AAA.

use std::sync::LazyLock;

use kawoosh_systems::ts::Token;
use kui_native::{Appearance, Color, Theme};

/// How many syntax tokens there are: a variant's hues are one each.
const TOKENS: usize = Token::ALL.len();

/// How a token's text is set beside its hue (docs/design/themes.md
/// Decision 6): the four a kui span can carry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

impl Style {
    pub const PLAIN: Style = Style {
        bold: false,
        italic: false,
        underline: false,
        strike: false,
    };

    /// The words of `tokens.styles`: any of `bold`, `italic`,
    /// `underline`, `strike`, apart by spaces or commas; `none` or
    /// nothing is plain. A word it does not know is none.
    pub fn parse(s: &str) -> Option<Style> {
        let mut out = Style::PLAIN;
        for w in s.split([' ', ',']).filter(|w| !w.is_empty()) {
            match w {
                "bold" => out.bold = true,
                "italic" => out.italic = true,
                "underline" => out.underline = true,
                "strike" | "strikethrough" => out.strike = true,
                "none" | "plain" => {}
                _ => return None,
            }
        }
        Some(out)
    }

    /// Spelt as `parse` reads it, `none` for plain.
    pub fn words(self) -> String {
        let mut w = Vec::new();
        for (on, name) in [
            (self.bold, "bold"),
            (self.italic, "italic"),
            (self.underline, "underline"),
            (self.strike, "strike"),
        ] {
            if on {
                w.push(name);
            }
        }
        if w.is_empty() {
            "none".into()
        } else {
            w.join(" ")
        }
    }
}

/// The styles every theme starts from, and `system`'s: comments in
/// italic, markup's headings and strong in bold, its emphasis in
/// italic and its links underlined — what the families' own editor
/// ports agree on. Code is otherwise upright and regular: weight is
/// kept for what a theme means to stand out.
pub fn base_style(token: Token) -> Style {
    use Token as T;
    let (bold, italic, underline) = match token {
        T::Comment | T::Emphasis => (false, true, false),
        T::Heading | T::Strong => (true, false, false),
        T::Link => (false, false, true),
        _ => return Style::PLAIN,
    };
    Style {
        bold,
        italic,
        underline,
        strike: false,
    }
}

/// One base's colours, whole: what `theme.dark` or `theme.light` names.
#[derive(Clone, Debug)]
pub struct Variant {
    /// What the settings call it: `ayu-mirage`.
    pub name: &'static str,
    /// What a person calls it: `Ayu Mirage`.
    pub title: &'static str,
    /// kui's roles, the appearance among them.
    pub theme: Theme,
    /// A hue per token, by `Token as usize`; none for plain text.
    syntax: [Option<Color>; TOKENS],
    /// A style per token, by `Token as usize`.
    styles: [Style; TOKENS],
    /// The terminal's sixteen, `0xRRGGBBAA`.
    pub ansi: [u32; 16],
}

impl Variant {
    fn new(
        name: &'static str,
        title: &'static str,
        theme: Theme,
        syntax: impl Fn(Token) -> Option<u32>,
        ansi: [u32; 16],
    ) -> Variant {
        let mut hues = [None; TOKENS];
        let mut styles = [Style::PLAIN; TOKENS];
        for t in Token::ALL {
            hues[*t as usize] = syntax(*t).map(c);
            styles[*t as usize] = base_style(*t);
        }
        Variant {
            name,
            title,
            theme,
            syntax: hues,
            styles,
            ansi: ansi.map(|x| (x << 8) | 0xFF),
        }
    }

    /// The variant with `token` set in `style` instead of the base's.
    fn styled(mut self, token: Token, style: Style) -> Variant {
        self.styles[token as usize] = style;
        self
    }

    pub fn dark(&self) -> bool {
        self.theme.is_dark()
    }

    /// A syntax token's hue on this variant's page.
    pub fn syntax(&self, token: Token) -> Option<Color> {
        self.syntax[token as usize]
    }

    /// How a syntax token's text is set.
    pub fn style(&self, token: Token) -> Style {
        self.styles[token as usize]
    }
}

/// A family: a name, and the variant for each base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Family {
    pub name: &'static str,
    pub dark: &'static str,
    pub light: &'static str,
}

/// Every family `theme.name` takes besides `system`, the default first.
pub const FAMILIES: &[Family] = &[
    Family {
        name: "rose-pine",
        dark: "rose-pine",
        light: "rose-pine-dawn",
    },
    Family {
        name: "rose-pine-moon",
        dark: "rose-pine-moon",
        light: "rose-pine-dawn",
    },
    Family {
        name: "ayu",
        dark: "ayu-dark",
        light: "ayu-light",
    },
    Family {
        name: "ayu-mirage",
        dark: "ayu-mirage",
        light: "ayu-light",
    },
    Family {
        name: "gruvbox",
        dark: "gruvbox-dark",
        light: "gruvbox-light",
    },
    Family {
        name: "gruvbox-hard",
        dark: "gruvbox-dark-hard",
        light: "gruvbox-light-hard",
    },
    Family {
        name: "gruvbox-soft",
        dark: "gruvbox-dark-soft",
        light: "gruvbox-light-soft",
    },
    Family {
        name: "tokyo-night",
        dark: "tokyo-night",
        light: "tokyo-night-day",
    },
    Family {
        name: "tokyo-night-storm",
        dark: "tokyo-night-storm",
        light: "tokyo-night-day",
    },
    Family {
        name: "tokyo-night-moon",
        dark: "tokyo-night-moon",
        light: "tokyo-night-day",
    },
    Family {
        name: "high-contrast",
        dark: "high-contrast-dark",
        light: "high-contrast-light",
    },
];

/// Every variant, the dark ones first, each family's in its order.
pub fn variants() -> &'static [Variant] {
    static ALL: LazyLock<Vec<Variant>> = LazyLock::new(|| {
        vec![
            MAIN.variant("rose-pine", "Rosé Pine"),
            MOON.variant("rose-pine-moon", "Rosé Pine Moon"),
            AYU_DARK.variant("ayu-dark", "Ayu Dark"),
            AYU_MIRAGE.variant("ayu-mirage", "Ayu Mirage"),
            gruvbox(true, Grade::Medium),
            gruvbox(true, Grade::Hard),
            gruvbox(true, Grade::Soft),
            TOKYO_NIGHT.variant("tokyo-night", "Tokyo Night"),
            TOKYO_STORM.variant("tokyo-night-storm", "Tokyo Night Storm"),
            TOKYO_MOON.variant("tokyo-night-moon", "Tokyo Night Moon"),
            high_contrast_dark(),
            DAWN.variant("rose-pine-dawn", "Rosé Pine Dawn"),
            AYU_LIGHT.variant("ayu-light", "Ayu Light"),
            gruvbox(false, Grade::Medium),
            gruvbox(false, Grade::Hard),
            gruvbox(false, Grade::Soft),
            TOKYO_DAY.variant("tokyo-night-day", "Tokyo Night Day"),
            high_contrast_light(),
        ]
    });
    &ALL
}

/// The variant by its name.
pub fn variant(name: &str) -> Option<&'static Variant> {
    variants().iter().find(|v| v.name == name)
}

/// The family by its name; none for `system` or a stranger.
pub fn family(name: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.name == name)
}

/// What the three settings resolve to: the variant for each base (none
/// where kui's roles off the OS stand), and the family `theme.name`
/// named.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pair {
    pub family: Option<&'static Family>,
    pub dark: Option<&'static Variant>,
    pub light: Option<&'static Variant>,
}

impl Pair {
    /// The variant shown on a base.
    pub fn of(&self, dark: bool) -> Option<&'static Variant> {
        if dark { self.dark } else { self.light }
    }
}

/// `theme.name`, `theme.dark` and `theme.light` resolved
/// (docs/design/themes.md Decision 2): the family's two halves, each
/// replaced by its own setting when that names a variant of its base.
/// Empty is the family's (the default family's for an empty name);
/// `system` is kui's roles off the OS. A name nobody ships, a variant
/// as the family, or a variant of the other base is a note, and what
/// it would have replaced stands.
pub fn resolve(name: &str, dark: &str, light: &str, notes: &mut Vec<String>) -> Pair {
    let known = || {
        let mut n = vec!["system"];
        n.extend(FAMILIES.iter().map(|f| f.name));
        n.join(", ")
    };
    let fam = match name.trim() {
        "system" => None,
        "" => Some(&FAMILIES[0]),
        n => family(n).or_else(|| {
            match variant(n) {
                Some(v) => {
                    let slot = if v.dark() { "dark" } else { "light" };
                    notes.push(format!(
                        "theme.name: \"{n}\" is a variant — theme.{slot} = \"{n}\" shows it"
                    ));
                }
                None => notes.push(format!("theme.name: no family \"{n}\" ({})", known())),
            }
            Some(&FAMILIES[0])
        }),
    };
    let mut pair = Pair {
        family: fam,
        dark: fam.and_then(|f| variant(f.dark)),
        light: fam.and_then(|f| variant(f.light)),
    };
    for (key, value, is_dark) in [("dark", dark, true), ("light", light, false)] {
        let slot = if is_dark {
            &mut pair.dark
        } else {
            &mut pair.light
        };
        match value.trim() {
            "" => {}
            "system" => *slot = None,
            n => match variant(n) {
                Some(v) if v.dark() == is_dark => *slot = Some(v),
                Some(_) => notes.push(format!("theme.{key}: \"{n}\" is not a {key} theme")),
                None => {
                    let names: Vec<&str> = variants()
                        .iter()
                        .filter(|v| v.dark() == is_dark)
                        .map(|v| v.name)
                        .collect();
                    notes.push(format!(
                        "theme.{key}: no theme \"{n}\" (system, {})",
                        names.join(", ")
                    ));
                }
            },
        }
    }
    pair
}

fn c(rgb: u32) -> Color {
    Color::hex((rgb << 8) | 0xFF)
}

/// A base's kui theme, the appearance set, the accent over it.
fn base(dark: bool, accent: u32) -> Theme {
    let (t, appearance) = if dark {
        (Theme::dark(), Appearance::Dark)
    } else {
        (Theme::light(), Appearance::Light)
    };
    Theme { appearance, ..t }.with_accent(c(accent))
}

/// kui's roles as a family names them: the page, the panels, a float,
/// a well; the hairline and the strong line; the three greys; the
/// accent; the selection's colour and alpha (translucent, so the glyphs
/// keep their hues); the states. What every family but Rosé Pine — whose
/// roles are its own derivation — turns into a theme.
#[derive(Clone, Copy, Debug)]
struct Roles {
    dark: bool,
    bg: u32,
    surface: u32,
    raised: u32,
    sunken: u32,
    border: u32,
    border_strong: u32,
    fg: u32,
    muted: u32,
    faint: u32,
    accent: u32,
    selection: (u32, f32),
    success: u32,
    warning: u32,
    danger: u32,
}

impl Roles {
    fn theme(&self) -> Theme {
        let t = base(self.dark, self.accent);
        // A label on the accent (the active tab, a mode chip) in the
        // theme's own page or text colour when either reads at 4.5:1,
        // else whichever of those and black and white reads best: kui's
        // own pick put white on gruvbox's and Tokyo Night's mid blues,
        // 2.5:1 (the theme check's "active tab label").
        let accent = c(self.accent);
        let on = |cs: &[Color]| {
            cs.iter()
                .copied()
                .max_by(|a, b| a.contrast(accent).total_cmp(&b.contrast(accent)))
                .unwrap_or(t.on_accent)
        };
        let own = on(&[c(self.bg), c(self.fg)]);
        let on_accent = if own.contrast(accent) >= 4.5 {
            own
        } else {
            on(&[own, Color::BLACK, Color::WHITE])
        };
        Theme {
            on_accent,
            bg: c(self.bg),
            surface: c(self.surface),
            raised: c(self.raised),
            sunken: c(self.sunken),
            border: c(self.border),
            border_strong: c(self.border_strong),
            fg: c(self.fg),
            muted: c(self.muted),
            faint: c(self.faint),
            selection: c(self.selection.0).with_alpha(self.selection.1),
            success: c(self.success),
            warning: c(self.warning),
            danger: c(self.danger),
            ..t
        }
    }
}

// ------------------------------------------------------------ Rosé Pine

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

impl Flavour {
    /// kui's roles from the variant: the page on `base`, panels on
    /// `surface`, floats on `overlay`; the text, `subtle` and `muted`
    /// for the two quieter greys; `iris` the accent; the selection
    /// translucent so the glyphs keep their colours — dark, the highlight
    /// Rosé Pine gives a visual selection; light, iris, as dawn's grey
    /// highlight is barely a step off the page — the states in the
    /// family's own hues (success in foam, as there is no green).
    pub fn theme(&self) -> Theme {
        let t = base(self.dark, self.iris);
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
            selection: if self.dark {
                c(self.hl_high).with_alpha(0.7)
            } else {
                c(self.iris).with_alpha(0.33)
            },
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
        self.sixteen().map(rgba)
    }

    fn sixteen(&self) -> [u32; 16] {
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
        out[..8].copy_from_slice(&eight);
        out[8..].copy_from_slice(&eight);
        out[8] = self.muted;
        out
    }

    fn variant(&self, name: &'static str, title: &'static str) -> Variant {
        let mut v = Variant::new(name, title, self.theme(), |_| None, self.sixteen());
        for t in Token::ALL {
            v.syntax[*t as usize] = self.syntax(*t);
        }
        v
    }
}

// ------------------------------------------------------------------ Ayu

/// One Ayu variant, by ayu-colors' own names (v5): the editor's page
/// and the UI's panels, the lines, and the syntax roles — `entity` a
/// type, `markup` a member, `special` an attribute. The translucent
/// ones (the comment, the punctuation) are laid flat on the page.
#[derive(Clone, Copy, Debug)]
pub struct Ayu {
    pub dark: bool,
    pub bg: u32,
    pub panel: u32,
    pub raised: u32,
    pub sunken: u32,
    pub line: u32,
    pub line_strong: u32,
    pub fg: u32,
    pub muted: u32,
    pub faint: u32,
    pub accent: u32,
    pub selection: (u32, f32),
    pub tag: u32,
    pub func: u32,
    pub entity: u32,
    pub string: u32,
    pub regexp: u32,
    pub markup: u32,
    pub keyword: u32,
    pub special: u32,
    pub comment: u32,
    pub constant: u32,
    pub operator: u32,
    pub punct: u32,
    pub added: u32,
    pub removed: u32,
    pub error: u32,
    /// Ayu's own terminal colours, black to bright white.
    pub ansi: [u32; 16],
}

pub const AYU_DARK: Ayu = Ayu {
    dark: true,
    bg: 0x0d1017,
    panel: 0x0b0e14,
    raised: 0x151a23,
    sunken: 0x0a0d12,
    line: 0x1b1f29,
    line_strong: 0x2d3340,
    fg: 0xbfbdb6,
    muted: 0x7c828c,
    faint: 0x565b66,
    accent: 0xe6b450,
    selection: (0x409fff, 0.3),
    tag: 0x39bae6,
    func: 0xffb454,
    entity: 0x59c2ff,
    string: 0xaad94c,
    regexp: 0x95e6cb,
    markup: 0xf07178,
    keyword: 0xff8f40,
    special: 0xe6b673,
    comment: 0x646b73,
    constant: 0xd2a6ff,
    operator: 0xf29668,
    punct: 0x8a8986,
    added: 0x7fd962,
    removed: 0xf26d78,
    error: 0xd95757,
    ansi: [
        0x01060e, 0xea6c73, 0x91b362, 0xf9af4f, 0x53bdfa, 0xfae994, 0x90e1c6, 0xc7c7c7, 0x686868,
        0xf07178, 0xc2d94c, 0xffb454, 0x59c2ff, 0xffee99, 0x95e6cb, 0xffffff,
    ],
};

pub const AYU_MIRAGE: Ayu = Ayu {
    dark: true,
    bg: 0x242936,
    panel: 0x1f2430,
    raised: 0x2b3140,
    sunken: 0x1c212b,
    line: 0x333a48,
    line_strong: 0x464e5e,
    fg: 0xcccac2,
    muted: 0x8a919e,
    faint: 0x707a8c,
    accent: 0xffcc66,
    selection: (0x409fff, 0.25),
    tag: 0x5ccfe6,
    func: 0xffd173,
    entity: 0x73d0ff,
    string: 0xd5ff80,
    regexp: 0x95e6cb,
    markup: 0xf28779,
    keyword: 0xffad66,
    special: 0xffdfb3,
    comment: 0x6e7c8e,
    constant: 0xdfbfff,
    operator: 0xf29e74,
    punct: 0x9a9a98,
    added: 0x87d96c,
    removed: 0xf27983,
    error: 0xff6666,
    ansi: [
        0x191e2a, 0xed8274, 0xa6cc70, 0xfad07b, 0x6dcbfa, 0xcfbafa, 0x90e1c6, 0xc7c7c7, 0x686868,
        0xf28779, 0xbae67e, 0xffd580, 0x73d0ff, 0xd4bfff, 0x95e6cb, 0xffffff,
    ],
};

pub const AYU_LIGHT: Ayu = Ayu {
    dark: false,
    bg: 0xfcfcfc,
    panel: 0xf8f9fa,
    raised: 0xffffff,
    sunken: 0xeff1f3,
    line: 0xe1e4e8,
    line_strong: 0xc4c9cf,
    // A step darker than Ayu's #5c6166, and the selection twice its
    // 0.15: at Ayu's own no selection could be seen at a glance (`SEEN`)
    // and keep the body text over it at 4.5:1 — the two bounds on the
    // page under it did not meet.
    fg: 0x53585e,
    muted: 0x787b80,
    faint: 0xadaeb1,
    accent: 0xffaa33,
    selection: (0x035bd6, 0.25),
    tag: 0x55b4d4,
    func: 0xf2ae49,
    entity: 0x399ee6,
    string: 0x86b300,
    regexp: 0x4cbf99,
    markup: 0xf07171,
    keyword: 0xfa8d3e,
    special: 0xe59645,
    comment: 0x929599,
    constant: 0xa37acc,
    operator: 0xed9366,
    punct: 0x8c8f93,
    added: 0x6cbf43,
    removed: 0xff7383,
    error: 0xe65050,
    ansi: [
        0x000000, 0xea6c6d, 0x6cbf43, 0xeca944, 0x3199e1, 0x9e75c7, 0x46ba94, 0xbababa, 0x686868,
        0xf07171, 0x86b300, 0xf2ae49, 0x399ee6, 0xa37acc, 0x4cbf99, 0xd1d1d1,
    ],
};

impl Ayu {
    /// kui's roles: the editor's page, the UI's panels, Ayu's accent
    /// and its blue selection; the states in its vcs and error hues.
    pub fn theme(&self) -> Theme {
        Roles {
            dark: self.dark,
            bg: self.bg,
            surface: self.panel,
            raised: self.raised,
            sunken: self.sunken,
            border: self.line,
            border_strong: self.line_strong,
            fg: self.fg,
            muted: self.muted,
            faint: self.faint,
            accent: self.accent,
            selection: self.selection,
            success: self.added,
            warning: self.func,
            danger: self.error,
        }
        .theme()
    }

    /// A syntax token's hue, as Ayu's own editor themes scope them.
    pub fn syntax(&self, token: Token) -> Option<u32> {
        use Token as T;
        Some(match token {
            T::Plain | T::Variable => return None,
            T::Keyword | T::Heading => self.keyword,
            T::Function | T::Macro | T::Strong => self.func,
            T::Type | T::Constructor => self.entity,
            T::Property | T::Tag | T::Link => self.tag,
            T::Label => self.markup,
            T::String => self.string,
            T::Number | T::Constant => self.constant,
            T::Comment => self.comment,
            T::Operator => self.operator,
            T::Punctuation => self.punct,
            T::Attribute | T::Emphasis => self.special,
            T::Raw => self.regexp,
            T::Added => self.added,
            T::Removed => self.removed,
        })
    }

    fn variant(&self, name: &'static str, title: &'static str) -> Variant {
        Variant::new(name, title, self.theme(), |t| self.syntax(t), self.ansi)
    }
}

// -------------------------------------------------------------- Gruvbox

/// Gruvbox's contrast: the page a step darker or lighter (`hard`), or
/// softer, the rest of the palette the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grade {
    Hard,
    Medium,
    Soft,
}

/// Gruvbox (morhetz/gruvbox), by its own names: the dark ramp from
/// `dark0_hard` to `dark4` and the light from `light0_hard` to `light4`;
/// the bright hues on the dark page, the faded on the light, the
/// neutral eight the terminal's darker half.
mod gruvbox {
    pub const DARK0_HARD: u32 = 0x1d2021;
    pub const DARK0: u32 = 0x282828;
    pub const DARK0_SOFT: u32 = 0x32302f;
    pub const DARK1: u32 = 0x3c3836;
    pub const DARK2: u32 = 0x504945;
    pub const DARK3: u32 = 0x665c54;
    pub const DARK4: u32 = 0x7c6f64;
    pub const GRAY: u32 = 0x928374;
    pub const LIGHT0_HARD: u32 = 0xf9f5d7;
    pub const LIGHT0: u32 = 0xfbf1c7;
    pub const LIGHT0_SOFT: u32 = 0xf2e5bc;
    pub const LIGHT1: u32 = 0xebdbb2;
    pub const LIGHT2: u32 = 0xd5c4a1;
    pub const LIGHT3: u32 = 0xbdae93;
    pub const LIGHT4: u32 = 0xa89984;
    /// red, green, yellow, blue, purple, aqua, orange.
    pub const BRIGHT: [u32; 7] = [
        0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xfe8019,
    ];
    pub const NEUTRAL: [u32; 7] = [
        0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xd65d0e,
    ];
    pub const FADED: [u32; 7] = [
        0x9d0006, 0x79740e, 0xb57614, 0x076678, 0x8f3f71, 0x427b58, 0xaf3a03,
    ];
}

/// A Gruvbox variant: the page by its grade, the panels a step off it,
/// floats and wells either side; the accent blue (yellow is the
/// warning's, and the visual mode's name), the selection the ramp's
/// third step as gruvbox's visual mode has it; the code as gruvbox.nvim
/// scopes it — keywords red, functions green and bold, types yellow,
/// strings green, numbers and constants purple, members blue, macros
/// and attributes aqua, operators orange, comments grey and italic.
fn gruvbox(dark: bool, grade: Grade) -> Variant {
    use gruvbox as g;
    let [red, green, yellow, blue, purple, aqua, orange] = if dark { g::BRIGHT } else { g::FADED };
    let roles = if dark {
        let bg = match grade {
            Grade::Hard => g::DARK0_HARD,
            Grade::Medium => g::DARK0,
            Grade::Soft => g::DARK0_SOFT,
        };
        Roles {
            dark,
            bg,
            surface: if grade == Grade::Soft {
                g::DARK0
            } else {
                g::DARK0_SOFT
            },
            raised: g::DARK1,
            sunken: if grade == Grade::Hard {
                0x161819
            } else {
                g::DARK0_HARD
            },
            border: g::DARK1,
            border_strong: g::DARK2,
            fg: g::LIGHT1,
            muted: g::LIGHT4,
            faint: g::DARK4,
            accent: blue,
            selection: (g::DARK3, 0.6),
            success: green,
            warning: yellow,
            danger: red,
        }
    } else {
        let bg = match grade {
            Grade::Hard => g::LIGHT0_HARD,
            Grade::Medium => g::LIGHT0,
            Grade::Soft => g::LIGHT0_SOFT,
        };
        Roles {
            dark,
            bg,
            surface: if grade == Grade::Soft {
                g::LIGHT0
            } else {
                g::LIGHT0_SOFT
            },
            raised: if grade == Grade::Hard {
                0xfdfbe8
            } else {
                g::LIGHT0_HARD
            },
            sunken: g::LIGHT1,
            border: g::LIGHT2,
            border_strong: g::LIGHT3,
            fg: g::DARK1,
            muted: g::DARK4,
            faint: g::LIGHT4,
            accent: blue,
            selection: (g::LIGHT3, 0.7),
            success: green,
            warning: yellow,
            danger: red,
        }
    };
    let fg4 = if dark { g::LIGHT4 } else { g::DARK4 };
    let syntax = |token: Token| {
        use Token as T;
        Some(match token {
            T::Plain | T::Variable => return None,
            T::Keyword | T::Label | T::Removed => red,
            T::Function | T::String | T::Added => green,
            T::Type | T::Heading => yellow,
            T::Constructor | T::Operator | T::Strong => orange,
            T::Number | T::Constant | T::Emphasis => purple,
            T::Comment => g::GRAY,
            T::Property | T::Link => blue,
            T::Punctuation => fg4,
            T::Attribute | T::Macro | T::Tag | T::Raw => aqua,
        })
    };
    let n = g::NEUTRAL;
    let ansi = if dark {
        [
            g::DARK0,
            n[0],
            n[1],
            n[2],
            n[3],
            n[4],
            n[5],
            g::LIGHT4,
            g::GRAY,
            red,
            green,
            yellow,
            blue,
            purple,
            aqua,
            g::LIGHT1,
        ]
    } else {
        [
            g::LIGHT0,
            n[0],
            n[1],
            n[2],
            n[3],
            n[4],
            n[5],
            g::DARK4,
            g::GRAY,
            red,
            green,
            yellow,
            blue,
            purple,
            aqua,
            g::DARK1,
        ]
    };
    let (name, title) = match (dark, grade) {
        (true, Grade::Hard) => ("gruvbox-dark-hard", "Gruvbox Dark Hard"),
        (true, Grade::Medium) => ("gruvbox-dark", "Gruvbox Dark"),
        (true, Grade::Soft) => ("gruvbox-dark-soft", "Gruvbox Dark Soft"),
        (false, Grade::Hard) => ("gruvbox-light-hard", "Gruvbox Light Hard"),
        (false, Grade::Medium) => ("gruvbox-light", "Gruvbox Light"),
        (false, Grade::Soft) => ("gruvbox-light-soft", "Gruvbox Light Soft"),
    };
    let bold = Style {
        bold: true,
        ..Style::PLAIN
    };
    Variant::new(name, title, roles.theme(), syntax, ansi).styled(Token::Function, bold)
}

// ---------------------------------------------------------- Tokyo Night

/// One Tokyo Night style (folke/tokyonight.nvim), by its own names: the
/// backgrounds, the foregrounds, and the hues.
#[derive(Clone, Copy, Debug)]
struct Tokyo {
    dark: bool,
    bg: u32,
    bg_dark: u32,
    bg_highlight: u32,
    terminal_black: u32,
    fg: u32,
    fg_dark: u32,
    fg_gutter: u32,
    dark3: u32,
    comment: u32,
    blue0: u32,
    blue: u32,
    cyan: u32,
    blue1: u32,
    blue5: u32,
    magenta: u32,
    purple: u32,
    orange: u32,
    yellow: u32,
    green: u32,
    green1: u32,
    teal: u32,
    red: u32,
    /// The terminal's black, its bright black being `terminal_black`.
    black: u32,
}

const TOKYO_NIGHT: Tokyo = Tokyo {
    dark: true,
    bg: 0x1a1b26,
    bg_dark: 0x16161e,
    bg_highlight: 0x292e42,
    terminal_black: 0x414868,
    fg: 0xc0caf5,
    fg_dark: 0xa9b1d6,
    fg_gutter: 0x3b4261,
    dark3: 0x545c7e,
    comment: 0x565f89,
    blue0: 0x3d59a1,
    blue: 0x7aa2f7,
    cyan: 0x7dcfff,
    blue1: 0x2ac3de,
    blue5: 0x89ddff,
    magenta: 0xbb9af7,
    purple: 0x9d7cd8,
    orange: 0xff9e64,
    yellow: 0xe0af68,
    green: 0x9ece6a,
    green1: 0x73daca,
    teal: 0x1abc9c,
    red: 0xf7768e,
    black: 0x15161e,
};

const TOKYO_STORM: Tokyo = Tokyo {
    bg: 0x24283b,
    bg_dark: 0x1f2335,
    black: 0x1d202f,
    ..TOKYO_NIGHT
};

const TOKYO_MOON: Tokyo = Tokyo {
    dark: true,
    bg: 0x222436,
    bg_dark: 0x1e2030,
    bg_highlight: 0x2f334d,
    terminal_black: 0x444a73,
    fg: 0xc8d3f5,
    fg_dark: 0x828bb8,
    fg_gutter: 0x3b4261,
    dark3: 0x545c7e,
    comment: 0x636da6,
    blue0: 0x3e68d7,
    blue: 0x82aaff,
    cyan: 0x86e1fc,
    blue1: 0x65bcff,
    blue5: 0x89ddff,
    magenta: 0xc099ff,
    purple: 0xfca7ea,
    orange: 0xff966c,
    yellow: 0xffc777,
    green: 0xc3e88d,
    green1: 0x4fd6be,
    teal: 0x4fd6be,
    red: 0xff757f,
    black: 0x1b1d2b,
};

const TOKYO_DAY: Tokyo = Tokyo {
    dark: false,
    bg: 0xe1e2e7,
    bg_dark: 0xd0d5e3,
    bg_highlight: 0xc4c8da,
    terminal_black: 0xa1a6c5,
    // A step darker than Tokyo Night Day's #3760bf, as Ayu Light's text
    // is: at its own, 4.5:1 on the page and no more, no selection could
    // be seen and keep the text readable over it.
    fg: 0x25479a,
    fg_dark: 0x6172b0,
    fg_gutter: 0xa8aecb,
    dark3: 0x8990b3,
    comment: 0x848cb5,
    blue0: 0x7890dd,
    blue: 0x2e7de9,
    cyan: 0x007197,
    blue1: 0x188092,
    blue5: 0x006a83,
    magenta: 0x9854f1,
    purple: 0x7847bd,
    orange: 0xb15c00,
    yellow: 0x8c6c3e,
    green: 0x587539,
    green1: 0x387068,
    teal: 0x118c74,
    red: 0xf52a65,
    black: 0xe9e9ed,
};

impl Tokyo {
    /// The roles as tokyonight.nvim sets its UI: the page `bg`, sidebars
    /// and floats on `bg_dark` (a float here on `bg_highlight`, so it is
    /// seen), the gutter in `dark3`, the accent blue, the selection
    /// `blue0` at the 0.4 its `bg_visual` is; the code as its treesitter
    /// groups — keywords purple and italic, functions blue, types
    /// `blue1`, strings green, numbers and constants orange, members
    /// `green1`, operators and punctuation `blue5`, tags red.
    fn variant(&self, name: &'static str, title: &'static str) -> Variant {
        let roles = Roles {
            dark: self.dark,
            bg: self.bg,
            surface: self.bg_dark,
            raised: if self.dark {
                self.bg_highlight
            } else {
                0xeeeff3
            },
            sunken: self.bg_dark,
            border: if self.dark {
                self.fg_gutter
            } else {
                self.bg_highlight
            },
            border_strong: if self.dark {
                self.dark3
            } else {
                self.fg_gutter
            },
            fg: self.fg,
            muted: self.fg_dark,
            faint: self.dark3,
            accent: self.blue,
            // `blue0`, a step stronger than its `bg_visual`'s 0.4: at
            // 0.4 the selection was not quite seen (`SEEN`) off either
            // page.
            selection: (self.blue0, if self.dark { 0.5 } else { 0.45 }),
            success: self.green,
            warning: self.yellow,
            danger: self.red,
        };
        let syntax = |token: Token| {
            use Token as T;
            Some(match token {
                T::Plain | T::Variable => return None,
                T::Keyword | T::Emphasis => self.purple,
                T::Function | T::Label | T::Heading => self.blue,
                T::Type => self.blue1,
                T::Constructor => self.magenta,
                T::String | T::Added => self.green,
                T::Number | T::Constant | T::Strong => self.orange,
                T::Comment => self.comment,
                T::Property => self.green1,
                T::Operator | T::Punctuation => self.blue5,
                T::Attribute | T::Macro => self.cyan,
                T::Tag | T::Removed => self.red,
                T::Link | T::Raw => self.teal,
            })
        };
        let ansi = [
            self.black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.magenta,
            self.cyan,
            self.fg_dark,
            self.terminal_black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.magenta,
            self.cyan,
            self.fg,
        ];
        // Tokyo Night sets its keywords in italic, as its comments.
        let italic = Style {
            italic: true,
            ..Style::PLAIN
        };
        Variant::new(name, title, roles.theme(), syntax, ansi).styled(Token::Keyword, italic)
    }
}

// -------------------------------------------------------- high contrast

/// The high-contrast pair (docs/design/themes.md Decision 1), kawoosh's
/// own, to WCAG AAA: the body text over 15:1 on the page, `muted` and
/// every syntax hue 7:1, `faint` 4.5:1, the borders 3:1 — the lines do
/// the work the surfaces' steps do elsewhere. No green, as the default
/// has none: what would be green is cyan (dark) or teal (light).
struct Contrast {
    dark: bool,
    bg: u32,
    surface: u32,
    raised: u32,
    sunken: u32,
    border: u32,
    border_strong: u32,
    fg: u32,
    muted: u32,
    faint: u32,
    accent: u32,
    selection: (u32, f32),
    success: u32,
    warning: u32,
    danger: u32,
    keyword: u32,
    function: u32,
    kind: u32,
    property: u32,
    label: u32,
    string: u32,
    number: u32,
    comment: u32,
    operator: u32,
    punct: u32,
    attribute: u32,
    ansi: [u32; 16],
}

impl Contrast {
    fn variant(&self, name: &'static str, title: &'static str) -> Variant {
        let theme = Roles {
            dark: self.dark,
            bg: self.bg,
            surface: self.surface,
            raised: self.raised,
            sunken: self.sunken,
            border: self.border,
            border_strong: self.border_strong,
            fg: self.fg,
            muted: self.muted,
            faint: self.faint,
            accent: self.accent,
            selection: self.selection,
            success: self.success,
            warning: self.warning,
            danger: self.danger,
        }
        .theme();
        let syntax = |token: Token| {
            use Token as T;
            Some(match token {
                T::Plain | T::Variable => return None,
                T::Keyword | T::Emphasis => self.keyword,
                T::Function | T::Heading => self.function,
                T::Type | T::Constructor | T::Tag | T::Link | T::Added => self.kind,
                T::Property => self.property,
                T::Label => self.label,
                T::String | T::Strong | T::Raw => self.string,
                T::Number | T::Constant => self.number,
                T::Comment => self.comment,
                T::Operator => self.operator,
                T::Punctuation => self.punct,
                T::Attribute | T::Macro => self.attribute,
                T::Removed => self.danger,
            })
        };
        // Weight carries what a hue alone might not to a reader who
        // needs the contrast: the keywords in bold.
        let bold = Style {
            bold: true,
            ..Style::PLAIN
        };
        Variant::new(name, title, theme, syntax, self.ansi).styled(Token::Keyword, bold)
    }
}

fn high_contrast_dark() -> Variant {
    Contrast {
        dark: true,
        bg: 0x000000,
        surface: 0x0a0a0a,
        raised: 0x141414,
        sunken: 0x050505,
        border: 0x8c8c8c,
        border_strong: 0xffffff,
        fg: 0xffffff,
        muted: 0xd4d4d4,
        faint: 0xa6a6a6,
        accent: 0xffd24a,
        selection: (0x2b6cff, 0.55),
        success: 0x5ce1e6,
        warning: 0xffa24a,
        danger: 0xff7b7b,
        keyword: 0xff9ae0,
        function: 0x82cfff,
        kind: 0x5ce1e6,
        property: 0xc8e0ff,
        label: 0xff9e7a,
        string: 0xffd580,
        number: 0xffab70,
        comment: 0xb8b8b8,
        operator: 0xe0e0e0,
        punct: 0xc8c8c8,
        attribute: 0xd7a6ff,
        ansi: [
            0x3a3a3a, 0xff7b7b, 0x5ce1e6, 0xffd580, 0x82cfff, 0xff9ae0, 0xd7a6ff, 0xe6e6e6,
            0x9a9a9a, 0xff7b7b, 0x5ce1e6, 0xffd580, 0x82cfff, 0xff9ae0, 0xd7a6ff, 0xffffff,
        ],
    }
    .variant("high-contrast-dark", "High Contrast Dark")
}

fn high_contrast_light() -> Variant {
    Contrast {
        dark: false,
        bg: 0xffffff,
        surface: 0xf4f4f4,
        raised: 0xffffff,
        sunken: 0xebebeb,
        border: 0x767676,
        border_strong: 0x000000,
        fg: 0x000000,
        muted: 0x333333,
        faint: 0x595959,
        accent: 0x0040c0,
        selection: (0x0060ff, 0.3),
        success: 0x005f73,
        warning: 0x8a4b00,
        danger: 0xb00020,
        keyword: 0xa3006b,
        function: 0x0040a8,
        kind: 0x005f73,
        property: 0x1f3f7a,
        label: 0x9c2c00,
        string: 0x7a3e00,
        number: 0x9c2c00,
        comment: 0x595959,
        operator: 0x262626,
        punct: 0x404040,
        attribute: 0x5a1fa8,
        ansi: [
            0xd6d6d6, 0xb00020, 0x005f73, 0x7a3e00, 0x0040a8, 0xa3006b, 0x5a1fa8, 0x1a1a1a,
            0x595959, 0xb00020, 0x005f73, 0x7a3e00, 0x0040a8, 0xa3006b, 0x5a1fa8, 0x000000,
        ],
    }
    .variant("high-contrast-light", "High Contrast Light")
}

/// The selection held legible whatever made it (roadmap step 28): a
/// system accent, a `theme.accent`, a `selection` role — the selection
/// as [`legible_wash`] holds any wash, on the page, under the body text
/// and the inks.
pub fn legible_selection(mut t: Theme, inks: &[Color]) -> Theme {
    t.selection = legible_wash(t.bg, t.fg, t.selection, inks);
    t
}

/// How strong a search hit's wash is meant to be over the page: the
/// warning's colour at this alpha, before [`legible_wash`] holds it.
pub const HIT_ALPHA: f32 = 0.35;

/// A search hit's wash for a theme and its inks: the warning's colour
/// at [`HIT_ALPHA`], held as [`legible_wash`] holds any.
pub fn legible_hit(t: &Theme, inks: &[Color]) -> Color {
    legible_wash(t.bg, t.fg, t.warning.with_alpha(HIT_ALPHA), inks)
}

/// A translucent wash under text — the selection, a search hit — held
/// legible (roadmap step 28, themes.md Decision 8). Glyphs are drawn
/// over it in their own colours, so what is checked is the text over
/// the wash laid on the page, and the wash's alpha is what gives: the
/// body text first, at 4.5:1 over it; then the wash seen at all, [`SEEN`]
/// off the page; then as many of the other inks at 2:1 over it as can be,
/// each that had that much on the page — and of the alphas that do as
/// well as any, the one nearest the wash's own. So a wash too strong for
/// the text is faded (a pale accent under white text, a dark theme's
/// gold hit under its body text), one too faint to be seen is
/// strengthened (a light theme's gold hit on its cream page), and one
/// that clears everything is left as it is. No alpha keeps the body
/// text: the most seen that keeps it, else the wash as given.
pub fn legible_wash(page: Color, fg: Color, wash: Color, inks: &[Color]) -> Color {
    let under = |a: f32| page.mix(Color::rgba(wash.r, wash.g, wash.b, 1.0), a);
    let room: Vec<Color> = inks
        .iter()
        .copied()
        .filter(|i| i.contrast(page) >= 2.0)
        .collect();
    // Best first: (the body reads and it is seen, inks clear, closeness);
    // with the body only, how seen.
    let score = |a: f32| -> Option<(u8, usize, f32)> {
        let u = under(a);
        if fg.contrast(u) < 4.5 {
            return None;
        }
        let seen = u.contrast(page);
        if seen < SEEN {
            return Some((0, 0, seen));
        }
        let inked = room.iter().filter(|i| i.contrast(u) >= 2.0).count();
        Some((1, inked, -(a - wash.a).abs()))
    };
    let better = |x: &(u8, usize, f32), y: &(u8, usize, f32)| {
        (x.0, x.1)
            .cmp(&(y.0, y.1))
            .then(x.2.total_cmp(&y.2))
            .is_gt()
    };
    // The wash's own alpha first, so a tie keeps it exactly.
    let mut best: Option<((u8, usize, f32), f32)> = score(wash.a).map(|s| (s, wash.a));
    for step in 5..=90 {
        let a = step as f32 / 100.0;
        if let Some(s) = score(a)
            && best.as_ref().is_none_or(|(b, _)| better(&s, b))
        {
            best = Some((s, a));
        }
    }
    match best {
        Some((_, a)) => wash.with_alpha(a),
        None => wash,
    }
}

/// How far off the page a selection has to be to be seen at a glance:
/// the contrast of the page under it against the page.
pub const SEEN: f32 = 1.4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_names_variants_of_its_bases() {
        assert_eq!(FAMILIES[0].name, "rose-pine");
        for f in FAMILIES {
            assert_eq!(family(f.name), Some(f));
            assert!(variant(f.dark).expect(f.dark).dark(), "{f:?}");
            assert!(!variant(f.light).expect(f.light).dark(), "{f:?}");
        }
        assert!(family("system").is_none());
        // Names are one each, and every variant is some family's half.
        for (i, v) in variants().iter().enumerate() {
            assert!(
                variants()[..i].iter().all(|w| w.name != v.name),
                "{}",
                v.name
            );
            assert!(
                FAMILIES
                    .iter()
                    .any(|f| f.dark == v.name || f.light == v.name),
                "{}",
                v.name
            );
        }
    }

    #[test]
    fn every_variant_reads() {
        for v in variants() {
            let t = v.theme;
            assert!(t.fg.contrast(t.bg) >= 4.5, "{}", v.name);
            assert!(t.muted.contrast(t.bg) >= 3.0, "{}", v.name);
            // The body text reads over the selection as it ships.
            let held = legible_selection(t, &[]);
            assert_eq!(held.selection, t.selection, "{}", v.name);
            // And the selection is seen once the inks have had their say.
            let inks: Vec<Color> = Token::ALL.iter().filter_map(|k| v.syntax(*k)).collect();
            let held = legible_selection(t, &inks);
            let s = held.selection;
            let under = t.bg.mix(Color::rgba(s.r, s.g, s.b, 1.0), s.a);
            assert!(under.contrast(t.bg) >= SEEN, "{}", v.name);
        }
    }

    #[test]
    fn high_contrast_is_aaa_and_has_no_green() {
        for name in ["high-contrast-dark", "high-contrast-light"] {
            let v = variant(name).unwrap();
            let t = v.theme;
            assert!(t.fg.contrast(t.bg) >= 15.0, "{name}");
            assert!(t.muted.contrast(t.bg) >= 7.0, "{name}");
            assert!(t.faint.contrast(t.bg) >= 4.5, "{name}");
            assert!(t.border.contrast(t.bg) >= 3.0, "{name}");
            assert!(t.danger.contrast(t.bg) >= 4.5, "{name}");
            assert!(t.warning.contrast(t.bg) >= 4.5, "{name}");
            for tok in Token::ALL {
                if let Some(h) = v.syntax(*tok) {
                    assert!(
                        h.contrast(t.bg) >= 7.0,
                        "{tok:?} in {name}: {}",
                        h.contrast(t.bg)
                    );
                    assert!(!green(h), "{tok:?} is green in {name}");
                }
            }
            // The selection keeps the body text at AAA too.
            let s = t.selection;
            let under = t.bg.mix(Color::rgba(s.r, s.g, s.b, 1.0), s.a);
            assert!(t.fg.contrast(under) >= 7.0, "{name}");
        }
    }

    #[test]
    fn the_three_settings_resolve() {
        let mut notes = Vec::new();
        let names = |p: Pair| (p.dark.map(|v| v.name), p.light.map(|v| v.name));
        // The default family, and a family by name.
        let p = resolve("", "", "", &mut notes);
        assert_eq!(names(p), (Some("rose-pine"), Some("rose-pine-dawn")));
        assert_eq!(p.family.map(|f| f.name), Some("rose-pine"));
        let p = resolve("ayu-mirage", "", "", &mut notes);
        assert_eq!(names(p), (Some("ayu-mirage"), Some("ayu-light")));
        // Each half apart from the family.
        let p = resolve("rose-pine", "ayu-dark", "high-contrast-light", &mut notes);
        assert_eq!(names(p), (Some("ayu-dark"), Some("high-contrast-light")));
        assert_eq!(p.of(true).map(|v| v.name), Some("ayu-dark"));
        // `system`: the whole, or one half.
        let p = resolve("system", "", "", &mut notes);
        assert_eq!(names(p), (None, None));
        let p = resolve("system", "", "ayu-light", &mut notes);
        assert_eq!(names(p), (None, Some("ayu-light")));
        let p = resolve("ayu", "system", "", &mut notes);
        assert_eq!(names(p), (None, Some("ayu-light")));
        assert!(notes.is_empty(), "{notes:?}");
        // A variant as the family, a stranger, a half of the wrong base:
        // each a note, and what it would have replaced stands.
        let p = resolve("ayu-dark", "rose-pine-dawn", "nope", &mut notes);
        assert_eq!(names(p), (Some("rose-pine"), Some("rose-pine-dawn")));
        assert_eq!(
            notes,
            [
                "theme.name: \"ayu-dark\" is a variant — theme.dark = \"ayu-dark\" shows it",
                "theme.dark: \"rose-pine-dawn\" is not a dark theme",
                "theme.light: no theme \"nope\" (system, rose-pine-dawn, ayu-light, gruvbox-light, gruvbox-light-hard, gruvbox-light-soft, tokyo-night-day, high-contrast-light)",
            ]
        );
        notes.clear();
        resolve("solarized", "", "", &mut notes);
        assert_eq!(
            notes,
            [
                "theme.name: no family \"solarized\" (system, rose-pine, rose-pine-moon, ayu, ayu-mirage, gruvbox, gruvbox-hard, gruvbox-soft, tokyo-night, tokyo-night-storm, tokyo-night-moon, high-contrast)"
            ]
        );
    }

    /// A search hit's wash (themes.md Decision 8): faded where it hid
    /// the body text (a dark theme's gold), strengthened where it was
    /// not seen (a light theme's gold on cream), kept where it clears.
    #[test]
    fn a_hit_is_held_both_ways() {
        let under = |t: &Theme, w: Color| t.bg.mix(Color::rgba(w.r, w.g, w.b, 1.0), w.a);
        let inks = |v: &Variant| -> Vec<Color> {
            Token::ALL.iter().filter_map(|k| v.syntax(*k)).collect()
        };
        let mirage = variant("ayu-mirage").unwrap();
        let t = mirage.theme;
        let raw = t.warning.with_alpha(HIT_ALPHA);
        assert!(t.fg.contrast(under(&t, raw)) < 4.5, "the gold hid the text");
        let hit = legible_hit(&t, &inks(mirage));
        assert!(hit.a < HIT_ALPHA);
        assert!(t.fg.contrast(under(&t, hit)) >= 4.5);
        assert!(under(&t, hit).contrast(t.bg) >= SEEN);
        let dawn = variant("rose-pine-dawn").unwrap();
        let t = dawn.theme;
        let raw = t.warning.with_alpha(HIT_ALPHA);
        assert!(
            under(&t, raw).contrast(t.bg) < SEEN,
            "the gold was not seen"
        );
        let hit = legible_hit(&t, &inks(dawn));
        assert!(hit.a > HIT_ALPHA);
        assert!(under(&t, hit).contrast(t.bg) >= SEEN);
        assert!(t.fg.contrast(under(&t, hit)) >= 4.5);
        let hc = variant("high-contrast-dark").unwrap();
        assert_eq!(
            legible_hit(&hc.theme, &inks(hc)).a,
            HIT_ALPHA,
            "kept exactly"
        );
    }

    #[test]
    fn styles_are_words_and_every_variant_has_the_base() {
        assert_eq!(Style::parse("bold italic").unwrap().words(), "bold italic");
        assert_eq!(
            Style::parse("italic, underline").unwrap().words(),
            "italic underline"
        );
        assert_eq!(Style::parse(""), Some(Style::PLAIN));
        assert_eq!(Style::parse("none"), Some(Style::PLAIN));
        assert_eq!(Style::parse("strikethrough").unwrap().words(), "strike");
        assert_eq!(Style::parse("bold loud"), None);
        for v in variants() {
            assert!(v.style(Token::Comment).italic, "{}", v.name);
            assert!(v.style(Token::Heading).bold, "{}", v.name);
            assert_eq!(v.style(Token::Plain), Style::PLAIN, "{}", v.name);
        }
        assert!(
            variant("high-contrast-dark")
                .unwrap()
                .style(Token::Keyword)
                .bold
        );
        assert!(!variant("ayu-dark").unwrap().style(Token::Keyword).bold);
    }

    /// A hue whose green channel leads both others by much: the todo's
    /// "i don't like green text".
    fn green(h: Color) -> bool {
        h.g > h.r + 0.15 && h.g > h.b + 0.15
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
                    assert!(!green(h), "{tok:?} is green in {f:?}");
                }
            }
            assert_eq!(f.ansi()[2], (f.pine << 8) | 0xFF);
        }
        // The variants are the flavours, whole.
        let v = variant("rose-pine-moon").unwrap();
        assert_eq!(v.theme.bg, MOON.theme().bg);
        assert_eq!(v.ansi, MOON.ansi());
        assert_eq!(v.syntax(Token::Keyword), MOON.syntax(Token::Keyword));
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

    #[test]
    fn an_ink_with_no_room_does_not_wash_the_selection_away() {
        let under = |t: &Theme| {
            let s = t.selection;
            t.bg.mix(Color::rgba(s.r, s.g, s.b, 1.0), s.a)
        };
        for f in [&MAIN, &MOON, &DAWN] {
            let t = f.theme();
            let inks: Vec<Color> = Token::ALL.iter().filter_map(|k| f.syntax(*k)).collect();
            let held = legible_selection(t, &inks);
            assert!(under(&held).contrast(held.bg) >= SEEN, "{f:?}");
            assert!(held.fg.contrast(under(&held)) >= 4.5, "{f:?}");
        }
        // A loud accent is still washed for the inks, down to seen.
        let t = DAWN.theme().with_accent(Color::hex(0x2060ffff));
        let inks: Vec<Color> = Token::ALL.iter().filter_map(|k| DAWN.syntax(*k)).collect();
        let held = legible_selection(t, &inks);
        assert!(held.selection.a < t.selection.a);
        assert!(under(&held).contrast(held.bg) >= SEEN * 0.85);
    }
}
