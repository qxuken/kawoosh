//! Chrome colours are kui theme roles (kui.md D7): the window follows the
//! OS appearance and accent without kawoosh naming a hue. Syntax colours
//! are tokens, declared from Lua, and are not here.

use kui::{Color, Theme};

#[derive(Clone, Copy, Debug)]
pub struct Pal {
    pub bg: Color,
    pub panel: Color,
    pub strip: Color,
    pub fg: Color,
    pub dim: Color,
    pub faint: Color,
    pub accent: Color,
    pub select: Color,
    pub insert: Color,
    pub command: Color,
    pub border: Color,
    pub danger: Color,
}

impl From<Theme> for Pal {
    fn from(t: Theme) -> Self {
        Self {
            bg: t.bg,
            panel: t.surface,
            strip: t.sunken,
            fg: t.fg,
            dim: t.muted,
            faint: t.faint,
            accent: t.focus_ring,
            select: t.selection,
            insert: t.success,
            command: t.warning,
            border: t.border,
            danger: t.danger,
        }
    }
}

impl Default for Pal {
    fn default() -> Self {
        Theme::default().into()
    }
}

/// Syntax hues are the app's own, not theme roles (kui.md D7): one set
/// per base, each checked to read on that base's surface. `init.lua`
/// replaces them through tokens (milestone 7).
pub fn syntax_color(token: kawoosh_systems::ts::Token, dark: bool) -> Option<Color> {
    use kawoosh_systems::ts::Token as T;
    let hue = |d: u32, l: u32| Some(Color::hex(((if dark { d } else { l }) << 8) | 0xFF));
    match token {
        T::Plain => None,
        T::Keyword => hue(0xC78FE8, 0x7A2FB0),
        T::Function => hue(0x82AAFF, 0x2455B8),
        T::Type => hue(0x6FC3D6, 0x1A7A8F),
        T::String => hue(0x9CC87A, 0x3E7A1F),
        T::Number => hue(0xD9A14D, 0x9A5A00),
        T::Comment => hue(0x6B7385, 0x7A7F8C),
        T::Variable => None,
        T::Property => hue(0xB8C4E0, 0x3E4A6B),
        T::Operator => hue(0xA0A8B8, 0x5A6070),
        T::Punctuation => hue(0x8A93A6, 0x6A7080),
        T::Attribute => hue(0xE0B070, 0x8A5A10),
        T::Constant => hue(0xD9A14D, 0x9A5A00),
        T::Macro => hue(0x7FC9B6, 0x1F7A66),
        T::Label => hue(0xE08A8A, 0xA02020),
        T::Constructor => hue(0x6FC3D6, 0x1A7A8F),
    }
}
