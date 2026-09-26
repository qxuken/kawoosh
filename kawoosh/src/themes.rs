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
//! them; and a high-contrast pair of kawoosh's own, drawn to WCAG AAA.

use std::sync::LazyLock;

use kawoosh_systems::ts::Token;
use kui_native::{Appearance, Color, Theme};

/// How many syntax tokens there are: a variant's hues are one each.
const TOKENS: usize = Token::ALL.len();

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
        for t in Token::ALL {
            hues[*t as usize] = syntax(*t).map(c);
        }
        Variant {
            name,
            title,
            theme,
            syntax: hues,
            ansi: ansi.map(|x| (x << 8) | 0xFF),
        }
    }

    pub fn dark(&self) -> bool {
        self.theme.is_dark()
    }

    /// A syntax token's hue on this variant's page.
    pub fn syntax(&self, token: Token) -> Option<Color> {
        self.syntax[token as usize]
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
            high_contrast_dark(),
            DAWN.variant("rose-pine-dawn", "Rosé Pine Dawn"),
            AYU_LIGHT.variant("ayu-light", "Ayu Light"),
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
        let t = base(self.dark, self.accent);
        Theme {
            bg: c(self.bg),
            surface: c(self.panel),
            raised: c(self.raised),
            sunken: c(self.sunken),
            border: c(self.line),
            border_strong: c(self.line_strong),
            fg: c(self.fg),
            muted: c(self.muted),
            faint: c(self.faint),
            selection: c(self.selection.0).with_alpha(self.selection.1),
            success: c(self.added),
            warning: c(self.func),
            danger: c(self.error),
            ..t
        }
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
        let t = base(self.dark, self.accent);
        let theme = Theme {
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
        };
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
        Variant::new(name, title, theme, syntax, self.ansi)
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
/// system accent, a `theme.accent`, a `selection` role. Glyphs are drawn
/// over it in their own colours, so what is checked is the text over
/// the selection laid on the page — body text at 4.5:1, and every other
/// ink at 2:1 where it had that much on the page — and the selection's
/// alpha is stepped down until they clear. The body text may take it
/// to a trace; the other inks only as far as the selection is still
/// seen, [`SEEN`] off the page — an ink with no room to spare (dawn's
/// gold, 2.05:1 on the page) had washed any selection out of sight.
pub fn legible_selection(mut t: Theme, inks: &[Color]) -> Theme {
    let under = |s: Color| t.bg.mix(Color::rgba(s.r, s.g, s.b, 1.0), s.a);
    let text = |s: Color| t.fg.contrast(under(s)) >= 4.5;
    let inked = |s: Color| {
        let u = under(s);
        inks.iter()
            .all(|i| i.contrast(t.bg) < 2.0 || i.contrast(u) >= 2.0)
    };
    let seen = |s: Color| under(s).contrast(t.bg) >= SEEN;
    let mut sel = t.selection;
    while sel.a > 0.12 {
        let next = sel.with_alpha(sel.a * 0.85);
        if !text(sel) || (!inked(sel) && seen(next)) {
            sel = next;
        } else {
            break;
        }
    }
    t.selection = sel;
    t
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
                "theme.light: no theme \"nope\" (system, rose-pine-dawn, ayu-light, high-contrast-light)",
            ]
        );
        notes.clear();
        resolve("gruvbox", "", "", &mut notes);
        assert_eq!(
            notes,
            [
                "theme.name: no family \"gruvbox\" (system, rose-pine, rose-pine-moon, ayu, ayu-mirage, high-contrast)"
            ]
        );
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
