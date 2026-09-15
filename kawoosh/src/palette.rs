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
