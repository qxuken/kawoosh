//! Input model (docs/design/mvp.md, Decision 4b).
//!
//! Bindings resolve keycode-first, scancode-fallback: a key is matched by
//! what the active layout says it means when that is a mappable (ASCII)
//! symbol — so OS-level Dvorak works — and falls back to the US-layout
//! meaning of its physical position when the layout produces a non-latin
//! symbol — so normal mode works in a Cyrillic layout. Text never comes
//! from keycodes; insert-mode content arrives via text-input events.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
    };
    pub const CTRL: Mods = Mods {
        ctrl: true,
        shift: false,
        alt: false,
    };
    pub const SHIFT: Mods = Mods {
        ctrl: false,
        shift: true,
        alt: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// The unshifted symbol; `Mods::shift` carries the shift state.
    Char(char),
    Esc,
    Enter,
    Backspace,
    Tab,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key: Key,
    pub mods: Mods,
}

impl KeyPress {
    pub fn plain(c: char) -> Self {
        Self {
            key: Key::Char(c),
            mods: Mods::NONE,
        }
    }

    pub fn shifted(c: char) -> Self {
        Self {
            key: Key::Char(c),
            mods: Mods::SHIFT,
        }
    }

    pub fn of(key: Key) -> Self {
        Self {
            key,
            mods: Mods::NONE,
        }
    }
}

/// US-layout meaning of an SDL scancode (the positional fallback).
/// Scancode values follow the USB HID usage tables SDL uses.
pub fn us_scancode_char(scancode: u16) -> Option<char> {
    Some(match scancode {
        4..=29 => (b'a' + (scancode - 4) as u8) as char,
        30..=38 => (b'1' + (scancode - 30) as u8) as char,
        39 => '0',
        44 => ' ',
        45 => '-',
        46 => '=',
        47 => '[',
        48 => ']',
        49 => '\\',
        51 => ';',
        52 => '\'',
        53 => '`',
        54 => ',',
        55 => '.',
        56 => '/',
        _ => return None,
    })
}

/// Resolve a physical key event to a binding key.
///
/// `keycode_char` is what the active layout produced (SDL keycodes are
/// unshifted unicode values for printable keys); `scancode` is the physical
/// position. ASCII layout meanings win; anything else falls back to the US
/// meaning of the position.
pub fn resolve_char(keycode_char: Option<char>, scancode: u16) -> Option<char> {
    match keycode_char {
        Some(c) if c.is_ascii_graphic() || c == ' ' => Some(c),
        _ => us_scancode_char(scancode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_layout_wins_over_position() {
        // Dvorak: the key at QWERTY-'s' position produces 'o' — 'o' it is.
        assert_eq!(resolve_char(Some('o'), 22), Some('o'));
    }

    #[test]
    fn non_latin_layout_falls_back_to_position() {
        // Cyrillic ЙЦУКЕН: the key at QWERTY-'j' produces 'о' (U+043E).
        assert_eq!(resolve_char(Some('\u{043E}'), 13), Some('j'));
        // And digits/punctuation positions resolve too.
        assert_eq!(resolve_char(Some('\u{0431}'), 54), Some(','));
    }

    #[test]
    fn no_keycode_uses_position() {
        assert_eq!(resolve_char(None, 4), Some('a'));
        assert_eq!(resolve_char(None, 100), None);
    }
}
