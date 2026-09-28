//! kitty's keyboard protocol, the bytes a key becomes once a program has
//! pushed its flags (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>,
//! docs/design/terminal-keys.md Decision 5). The legacy encoding, for a
//! program that pushed nothing, stays [`crate::encode_key`].
//!
//! A key is named as kui names it — `"a"`, `"A"`, `"enter"`, `"f13"`,
//! `"shift"` — with the US-QWERTY key at its position, where it sits
//! (the keypad's `1` is `"1"` at [`Location::Numpad`]) and the lock
//! state; this module turns that into the protocol's key number, its
//! shifted and base-layout alternates, its modifiers and its event type.

/// The progressive enhancement flags, as a program pushes them.
pub const DISAMBIGUATE: u8 = 1;
pub const EVENT_TYPES: u8 = 2;
pub const ALTERNATE_KEYS: u8 = 4;
pub const ALL_KEYS: u8 = 8;
pub const ASSOCIATED_TEXT: u8 = 16;

/// Which of a key's twins it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Location {
    #[default]
    Standard,
    Left,
    Right,
    Numpad,
}

/// A press, an OS repeat, or a release.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    Press,
    Repeat,
    Release,
}

/// One key event, as kawoosh has it from kui.
#[derive(Clone, Debug, Default)]
pub struct KeyInput<'a> {
    /// kui's code: a character as the layout produced it, or a name.
    pub code: &'a str,
    /// kui's `physical`: the US-QWERTY key at that position.
    pub physical: &'a str,
    pub location: Location,
    /// What the press types; none for a chord or a release.
    pub text: Option<&'a str>,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub sup: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
    pub action: Action,
}

/// What a key is to the protocol: a text key by the Unicode code point of
/// its unshifted form, or a functional key by its number and the final
/// byte of its escape code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Text(u32),
    Func(u32, u8),
}

// The functional keys' numbers (the protocol's table). The keypad's are
// KP_0 + n in the order below; the modifiers LEFT_SHIFT + n.
const CAPS_LOCK: u32 = 57358;
const SCROLL_LOCK: u32 = 57359;
const NUM_LOCK: u32 = 57360;
const PRINT_SCREEN: u32 = 57361;
const PAUSE: u32 = 57362;
const MENU: u32 = 57363;
const F13: u32 = 57376;
const KP_0: u32 = 57399;
const KP_DECIMAL: u32 = 57409;
const KP_DIVIDE: u32 = 57410;
const KP_MULTIPLY: u32 = 57411;
const KP_SUBTRACT: u32 = 57412;
const KP_ADD: u32 = 57413;
const KP_ENTER: u32 = 57414;
const KP_EQUAL: u32 = 57415;
const KP_SEPARATOR: u32 = 57416;
const KP_LEFT: u32 = 57417;
const KP_RIGHT: u32 = 57418;
const KP_UP: u32 = 57419;
const KP_DOWN: u32 = 57420;
const KP_PAGE_UP: u32 = 57421;
const KP_PAGE_DOWN: u32 = 57422;
const KP_HOME: u32 = 57423;
const KP_END: u32 = 57424;
const KP_INSERT: u32 = 57425;
const KP_DELETE: u32 = 57426;
const KP_BEGIN: u32 = 57427;
const LEFT_SHIFT: u32 = 57441;
const RIGHT_SHIFT: u32 = 57447;

/// The escape codes the legacy keys keep under the protocol: the number
/// and final byte each is sent with once it carries modifiers or an
/// event type (`CSI 1 ; 5 A`, `CSI 15 ; 2 ~`, `CSI 13 ; 5 u`).
fn legacy_form(name: &str) -> Option<(u32, u8)> {
    Some(match name {
        "escape" => (27, b'u'),
        "enter" => (13, b'u'),
        "tab" => (9, b'u'),
        "backspace" => (127, b'u'),
        "insert" => (2, b'~'),
        "delete" => (3, b'~'),
        "left" => (1, b'D'),
        "right" => (1, b'C'),
        "up" => (1, b'A'),
        "down" => (1, b'B'),
        "pageup" => (5, b'~'),
        "pagedown" => (6, b'~'),
        "home" => (1, b'H'),
        "end" => (1, b'F'),
        "f1" => (1, b'P'),
        "f2" => (1, b'Q'),
        "f3" => (13, b'~'),
        "f4" => (1, b'S'),
        "f5" => (15, b'~'),
        "f6" => (17, b'~'),
        "f7" => (18, b'~'),
        "f8" => (19, b'~'),
        "f9" => (20, b'~'),
        "f10" => (21, b'~'),
        "f11" => (23, b'~'),
        "f12" => (24, b'~'),
        _ => return None,
    })
}

/// A named key's number under the protocol, its place considered: the
/// keypad's Enter and arrows are keys of their own, as are the left and
/// right modifiers.
fn functional(name: &str, at: Location) -> Option<Key> {
    let u = |n| Some(Key::Func(n, b'u'));
    if at == Location::Numpad {
        let n = match name {
            "enter" => KP_ENTER,
            "left" => KP_LEFT,
            "right" => KP_RIGHT,
            "up" => KP_UP,
            "down" => KP_DOWN,
            "pageup" => KP_PAGE_UP,
            "pagedown" => KP_PAGE_DOWN,
            "home" => KP_HOME,
            "end" => KP_END,
            "insert" => KP_INSERT,
            "delete" => KP_DELETE,
            "clear" => KP_BEGIN,
            _ => 0,
        };
        if n != 0 {
            return u(n);
        }
    }
    if let Some((n, t)) = legacy_form(name) {
        return Some(Key::Func(n, t));
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u32>().ok())
        && (13..=35).contains(&n)
    {
        return u(F13 + n - 13);
    }
    let right = at == Location::Right;
    let side = |left: u32| {
        if right {
            left + (RIGHT_SHIFT - LEFT_SHIFT)
        } else {
            left
        }
    };
    match name {
        "clear" => u(KP_BEGIN),
        "capslock" => u(CAPS_LOCK),
        "scrolllock" => u(SCROLL_LOCK),
        "numlock" => u(NUM_LOCK),
        "printscreen" => u(PRINT_SCREEN),
        "pause" => u(PAUSE),
        "menu" => u(MENU),
        "mediaplay" => u(57428),
        "mediapause" => u(57429),
        "mediaplaypause" => u(57430),
        "mediastop" => u(57432),
        "mediafastforward" => u(57433),
        "mediarewind" => u(57434),
        "medianext" => u(57435),
        "mediaprev" => u(57436),
        "mediarecord" => u(57437),
        "volumedown" => u(57438),
        "volumeup" => u(57439),
        "volumemute" => u(57440),
        "shift" => u(side(LEFT_SHIFT)),
        "ctrl" => u(side(LEFT_SHIFT + 1)),
        "alt" => u(side(LEFT_SHIFT + 2)),
        "super" => u(side(LEFT_SHIFT + 3)),
        _ => None,
    }
}

/// The keypad's text keys as functional keys: its digits and operators
/// have numbers of their own.
fn keypad_text(c: char) -> Option<u32> {
    Some(match c {
        '0'..='9' => KP_0 + (c as u32 - '0' as u32),
        '.' => KP_DECIMAL,
        '/' => KP_DIVIDE,
        '*' => KP_MULTIPLY,
        '-' => KP_SUBTRACT,
        '+' => KP_ADD,
        '=' => KP_EQUAL,
        ',' => KP_SEPARATOR,
        _ => return None,
    })
}

/// A keypad key read as the main block's, for a program that did not ask
/// to tell them apart.
fn keypad_as_main(n: u32) -> Option<Key> {
    let text = |c: char| Some(Key::Text(c as u32));
    let func = |name: &str| legacy_form(name).map(|(n, t)| Key::Func(n, t));
    match n {
        n if (KP_0..KP_0 + 10).contains(&n) => text(char::from_digit(n - KP_0, 10)?),
        KP_DECIMAL => text('.'),
        KP_DIVIDE => text('/'),
        KP_MULTIPLY => text('*'),
        KP_SUBTRACT => text('-'),
        KP_ADD => text('+'),
        KP_EQUAL => text('='),
        KP_SEPARATOR => text(','),
        KP_ENTER => func("enter"),
        KP_LEFT => func("left"),
        KP_RIGHT => func("right"),
        KP_UP => func("up"),
        KP_DOWN => func("down"),
        KP_PAGE_UP => func("pageup"),
        KP_PAGE_DOWN => func("pagedown"),
        KP_HOME => func("home"),
        KP_END => func("end"),
        KP_INSERT => func("insert"),
        KP_DELETE => func("delete"),
        KP_BEGIN => Some(Key::Func(1, b'E')),
        _ => None,
    }
}

fn is_modifier(name: &str) -> bool {
    matches!(
        name,
        "shift" | "ctrl" | "alt" | "super" | "capslock" | "numlock" | "scrolllock"
    )
}

/// The one character of a single-character name.
fn single(s: &str) -> Option<char> {
    let mut it = s.chars();
    let c = it.next()?;
    it.next().is_none().then_some(c)
}

/// The key, and its shifted and base-layout alternates, for a key event.
fn identify(k: &KeyInput<'_>) -> Option<(Key, Option<u32>, Option<u32>)> {
    let code = if k.code == "space" { " " } else { k.code };
    let Some(c) = single(code) else {
        return functional(k.code, k.location).map(|key| (key, None, None));
    };
    if k.location == Location::Numpad
        && let Some(n) = keypad_text(c)
    {
        return Some((Key::Func(n, b'u'), None, None));
    }
    let physical = if k.physical == "space" {
        Some(' ')
    } else {
        single(k.physical)
    };
    // The key is its unshifted form: a letter's lower case, and for
    // anything else under Shift the key at its position — exact on a
    // US layout and every layout whose digit row and punctuation keys sit
    // where US-QWERTY's do, which is what kui can say of a shifted symbol.
    let lower: String = c.to_lowercase().collect();
    let base = match single(&lower) {
        Some(l) if l != c => l,
        _ if k.shift && physical.is_some_and(|p| p != c) => physical?,
        _ => c,
    };
    let shifted = (k.shift && c != base).then_some(c as u32);
    // The base-layout key: the US-QWERTY key at the position, when the
    // layout put another there (Cyrillic, Greek) — what makes ctrl+С
    // match a ctrl+c binding.
    let alternate = physical.filter(|p| *p != base && *p != c).map(|p| p as u32);
    Some((Key::Text(base as u32), shifted, alternate))
}

/// C0 control for ctrl held on an ASCII key, the table every terminal
/// uses; a key without one is itself.
fn ctrl_byte(c: u8) -> u8 {
    match c {
        b' ' | b'2' | b'@' => 0,
        b'a'..=b'z' => c - b'a' + 1,
        b'3' | b'[' => 27,
        b'4' | b'\\' => 28,
        b'5' | b']' => 29,
        b'6' | b'^' | b'~' => 30,
        b'7' | b'/' | b'_' => 31,
        b'8' | b'?' => 127,
        _ => c,
    }
}

/// The modifier bits: shift 1, alt 2, ctrl 4, super 8, Caps Lock 64,
/// Num Lock 128.
fn mod_bits(k: &KeyInput<'_>) -> u32 {
    let b = |on: bool, v: u32| if on { v } else { 0 };
    b(k.shift, 1)
        | b(k.alt, 2)
        | b(k.ctrl, 4)
        | b(k.sup, 8)
        | b(k.caps_lock, 64)
        | b(k.num_lock, 128)
}

/// `CSI key[:shifted[:alternate]] ; mods[:event] ; text u`, each part
/// left out when it says nothing.
fn csi(
    key: u32,
    alts: Option<(Option<u32>, Option<u32>)>,
    mods: u32,
    action: Option<Action>,
    text: Option<&str>,
    fin: u8,
) -> Vec<u8> {
    let mut s = String::from("\x1b[");
    let second = mods != 0 || action.is_some();
    if key != 1 || alts.is_some() || second || text.is_some() {
        s.push_str(&key.to_string());
    }
    if let Some((shifted, alternate)) = alts {
        s.push(':');
        if let Some(sh) = shifted {
            s.push_str(&sh.to_string());
        }
        if let Some(a) = alternate {
            s.push(':');
            s.push_str(&a.to_string());
        }
    }
    if second || text.is_some() {
        s.push(';');
        if second {
            s.push_str(&(mods + 1).to_string());
        }
        if let Some(a) = action {
            s.push(':');
            s.push(match a {
                Action::Press => '1',
                Action::Repeat => '2',
                Action::Release => '3',
            });
        }
    }
    if let Some(t) = text {
        let points: Vec<String> = t.chars().map(|c| (c as u32).to_string()).collect();
        s.push(';');
        s.push_str(&points.join(":"));
    }
    s.push(fin as char);
    s.into_bytes()
}

/// A text key with modifiers the way a legacy terminal sends it, for a
/// program that asked for the protocol's other parts but not to tell
/// keys apart; `None` where the legacy encoding has no room.
fn legacy_text(key: u32, shifted: Option<u32>, mods: u32) -> Option<Vec<u8>> {
    let mut c = u8::try_from(key).ok().filter(u8::is_ascii)?;
    let mut rest = mods;
    if mods & 1 != 0
        && let Some(sh) = shifted
            .and_then(|s| u8::try_from(s).ok())
            .filter(u8::is_ascii)
        && (mods & 4 == 0 || !c.is_ascii_lowercase())
    {
        c = sh;
        rest &= !1;
    }
    Some(match rest {
        0 => vec![c],
        2 => vec![0x1b, c],
        4 => vec![ctrl_byte(c)],
        6 => vec![0x1b, ctrl_byte(c)],
        5 if c == b' ' => vec![0],
        3 if c == b' ' => vec![0x1b, b' '],
        _ => return None,
    })
}

/// The bytes a key event is to a program that pushed `flags`, in cursor
/// key mode or not; `None` for an event the program is not to hear (a
/// release it did not ask for, a modifier key alone). `flags` 0 is the
/// legacy encoding's business, not this one's.
pub fn encode(k: &KeyInput<'_>, flags: u8, cursor_keys: bool) -> Option<Vec<u8>> {
    let disambiguate = flags & DISAMBIGUATE != 0;
    let events = flags & EVENT_TYPES != 0;
    let all = flags & ALL_KEYS != 0;
    let legacy = !events && !disambiguate && !all;
    if !all && is_modifier(k.code) {
        return None;
    }
    let text = k
        .text
        .filter(|t| t.chars().next().is_some_and(|c| !c.is_control()));
    let (mut key, shifted, alternate) = identify(k)?;
    // A program that did not ask to tell them apart reads the keypad as
    // the main block.
    if !disambiguate
        && !all
        && let Key::Func(n, _) = key
        && let Some(main) = keypad_as_main(n)
    {
        key = main;
    }
    // A key that types, types — unless every key is to be an escape code.
    if !all
        && k.action != Action::Release
        && let Some(t) = text
    {
        return Some(t.as_bytes().to_vec());
    }
    if !events && k.action == Action::Release {
        return None;
    }
    let mods = mod_bits(k);
    let action = (events && k.action != Action::Press).then_some(k.action);
    match key {
        Key::Func(n, fin) => functional_bytes(k, n, fin, mods, action, legacy, cursor_keys, flags),
        Key::Text(key) => {
            let alts = (flags & ALTERNATE_KEYS != 0
                && ((shifted.is_some() && k.shift) || alternate.is_some()))
            .then(|| (shifted.filter(|_| k.shift), alternate));
            let embed = (flags & ASSOCIATED_TEXT != 0).then_some(text).flatten();
            if action.is_none() && alts.is_none() && embed.is_none() {
                if mods == 0 {
                    return Some(if all {
                        csi(key, None, 0, None, None, b'u')
                    } else {
                        char::from_u32(key)?.to_string().into_bytes()
                    });
                }
                if !disambiguate && !all {
                    let as_legacy = legacy_text(key, shifted, mods).or_else(|| {
                        // A layout whose key is not ASCII sends its US
                        // key's control under ctrl or alt, as one would.
                        (matches!(mods, 2 | 4 | 6) && key > 127)
                            .then(|| legacy_text(alternate?, None, mods))
                            .flatten()
                    });
                    if let Some(b) = as_legacy {
                        return Some(b);
                    }
                }
            }
            Some(csi(key, alts, mods, action, embed, b'u'))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn functional_bytes(
    k: &KeyInput<'_>,
    mut n: u32,
    mut fin: u8,
    mods: u32,
    action: Option<Action>,
    legacy: bool,
    cursor_keys: bool,
    flags: u8,
) -> Option<Vec<u8>> {
    let all = flags & ALL_KEYS != 0;
    let release = k.action == Action::Release;
    let plain = |b: &[u8]| Some(b.to_vec());
    if legacy && mods == 0 && cursor_keys {
        let ss3 = match (n, fin) {
            (1, b'A' | b'B' | b'C' | b'D' | b'E' | b'F' | b'H') => Some(fin),
            _ => None,
        };
        if let Some(f) = ss3 {
            return plain(&[0x1b, b'O', f]);
        }
    }
    if mods == 0 {
        if flags & DISAMBIGUATE == 0 && !all && (n, fin) == (27, b'u') {
            return plain(b"\x1b");
        }
        if legacy && fin != b'~' && n == 1 {
            let ss3 = match fin {
                b'P' | b'Q' | b'S' => Some(fin),
                _ => None,
            };
            if let Some(f) = ss3 {
                return plain(&[0x1b, b'O', f]);
            }
        }
        if legacy && (n, fin) == (13, b'~') {
            return plain(b"\x1bOR");
        }
    } else if legacy && let Some(b) = legacy_modified(n, fin, mods) {
        return Some(b);
    }
    // Enter, Tab and Backspace keep their bytes while nothing but a lock
    // is held — a shell a program left in this mode still takes `reset`
    // — and have no release.
    if mods & !(64 | 128) == 0 && !all {
        let b: Option<&[u8]> = match (n, fin) {
            (13, b'u') => Some(b"\r"),
            (127, b'u') => Some(b"\x7f"),
            (9, b'u') => Some(b"\t"),
            _ => None,
        };
        if let Some(b) = b {
            return (!release).then(|| b.to_vec());
        }
    }
    if n == MENU && legacy {
        (n, fin) = (29, b'~');
    }
    Some(csi(n, None, mods, action, None, fin))
}

/// Enter, Escape, Backspace and Tab under a modifier, the legacy way.
fn legacy_modified(n: u32, fin: u8, mods: u32) -> Option<Vec<u8>> {
    let alt = mods & 2 != 0;
    let mut out = if alt { vec![0x1b] } else { Vec::new() };
    match (n, fin) {
        (13, b'u') => out.push(b'\r'),
        (27, b'u') => out.push(0x1b),
        (127, b'u') => out.push(if mods & 4 != 0 { 0x08 } else { 0x7f }),
        (9, b'u') if mods & 1 != 0 => {
            out.push(0x1b);
            out.extend_from_slice(b"[Z");
        }
        (9, b'u') => out.push(b'\t'),
        _ => return None,
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: &'static str) -> KeyInput<'static> {
        let text = single(code).filter(|c| !c.is_control()).map(|_| code);
        KeyInput {
            code,
            physical: code,
            text,
            ..Default::default()
        }
    }

    fn enc(k: &KeyInput<'_>, flags: u8) -> String {
        String::from_utf8(encode(k, flags, false).unwrap_or_default()).unwrap()
    }

    /// Flag 1: a key that types, types; Escape and every chord are
    /// `CSI u`, the keys with legacy forms keep them under a modifier;
    /// Enter, Tab and Backspace keep their bytes alone.
    #[test]
    fn disambiguate() {
        assert_eq!(enc(&key("a"), 1), "a");
        assert_eq!(enc(&key("escape"), 1), "\x1b[27u");
        let ctrl_c = KeyInput {
            ctrl: true,
            text: None,
            ..key("c")
        };
        assert_eq!(enc(&ctrl_c, 1), "\x1b[99;5u");
        // ctrl+shift+l: the key is `l`, Shift a modifier — what the
        // legacy encoding had no room for.
        let ctrl_shift_l = KeyInput {
            code: "L",
            physical: "l",
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(enc(&ctrl_shift_l, 1), "\x1b[108;6u");
        let alt_bracket = KeyInput {
            alt: true,
            text: None,
            ..key("[")
        };
        assert_eq!(enc(&alt_bracket, 1), "\x1b[91;3u");
        assert_eq!(enc(&key("enter"), 1), "\r");
        assert_eq!(enc(&key("tab"), 1), "\t");
        assert_eq!(enc(&key("backspace"), 1), "\x7f");
        let ctrl_tab = KeyInput {
            ctrl: true,
            ..key("tab")
        };
        assert_eq!(enc(&ctrl_tab, 1), "\x1b[9;5u");
        let shift_tab = KeyInput {
            shift: true,
            ..key("tab")
        };
        assert_eq!(enc(&shift_tab, 1), "\x1b[9;2u");
        let ctrl_up = KeyInput {
            ctrl: true,
            ..key("up")
        };
        assert_eq!(enc(&ctrl_up, 1), "\x1b[1;5A");
        assert_eq!(enc(&key("up"), 1), "\x1b[A");
        assert_eq!(enc(&key("f1"), 1), "\x1b[P");
        assert_eq!(enc(&key("f3"), 1), "\x1b[13~");
        assert_eq!(enc(&key("f5"), 1), "\x1b[15~");
        assert_eq!(enc(&key("f13"), 1), "\x1b[57376u");
        // The keypad's Enter is its own key; its digits type.
        let kp_enter = KeyInput {
            location: Location::Numpad,
            ..key("enter")
        };
        assert_eq!(enc(&kp_enter, 1), "\x1b[57414u");
        let kp_1 = KeyInput {
            location: Location::Numpad,
            ..key("1")
        };
        assert_eq!(enc(&kp_1, 1), "1");
        // A modifier alone is nothing, and so is a release.
        assert_eq!(enc(&key("shift"), 1), "");
        let up = KeyInput {
            action: Action::Release,
            text: None,
            ..key("a")
        };
        assert_eq!(enc(&up, 1), "");
        // ⌘ is a modifier like the others.
        let super_j = KeyInput {
            sup: true,
            text: None,
            ..key("j")
        };
        assert_eq!(enc(&super_j, 1), "\x1b[106;9u");
        // A lock rides on a chord, not on a key that types.
        let caps_ctrl_a = KeyInput {
            ctrl: true,
            caps_lock: true,
            text: None,
            ..key("a")
        };
        assert_eq!(enc(&caps_ctrl_a, 1), "\x1b[97;69u");
        let caps_a = KeyInput {
            caps_lock: true,
            code: "A",
            text: Some("A"),
            physical: "a",
            ..Default::default()
        };
        assert_eq!(enc(&caps_a, 1), "A");
    }

    /// Flag 2: repeats and releases, as `:2` and `:3`; Enter, Tab and
    /// Backspace have no release unless every key is an escape code.
    #[test]
    fn event_types() {
        let a = KeyInput {
            text: None,
            ..key("a")
        };
        let rep = KeyInput {
            action: Action::Repeat,
            ..key("a")
        };
        assert_eq!(enc(&rep, 3), "a", "a repeat that types, types");
        let up = KeyInput {
            action: Action::Release,
            ..a.clone()
        };
        assert_eq!(enc(&up, 3), "\x1b[97;1:3u");
        let ctrl_rep = KeyInput {
            ctrl: true,
            action: Action::Repeat,
            ..a.clone()
        };
        assert_eq!(enc(&ctrl_rep, 3), "\x1b[97;5:2u");
        let enter_up = KeyInput {
            action: Action::Release,
            ..key("enter")
        };
        assert_eq!(enc(&enter_up, 3), "");
        assert_eq!(enc(&enter_up, 3 | 8), "\x1b[13;1:3u");
        let left_up = KeyInput {
            action: Action::Release,
            ..key("left")
        };
        assert_eq!(enc(&left_up, 3), "\x1b[1;1:3D");
    }

    /// Flag 4: the shifted key beside the key under Shift, and the
    /// base-layout key where the layout put another there.
    #[test]
    fn alternate_keys() {
        let ctrl_shift_a = KeyInput {
            code: "A",
            physical: "a",
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(enc(&ctrl_shift_a, 1 | 4), "\x1b[97:65;6u");
        // ctrl+shift+; on US: key `;`, shifted `:`.
        let ctrl_colon = KeyInput {
            code: ":",
            physical: ";",
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(enc(&ctrl_colon, 1 | 4), "\x1b[59:58;6u");
        // ctrl+С on a Russian layout: the key is `с`, its base `c`.
        let ctrl_es = KeyInput {
            code: "с",
            physical: "c",
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(enc(&ctrl_es, 1 | 4), "\x1b[1089::99;5u");
        assert_eq!(enc(&ctrl_es, 1), "\x1b[1089;5u", "no alternates unasked");
    }

    /// Flag 8: every key an escape code, the modifier keys too with
    /// their side; flag 16 the text inside it.
    #[test]
    fn all_keys_and_text() {
        assert_eq!(enc(&key("a"), 8), "\x1b[97u");
        assert_eq!(enc(&key("enter"), 8), "\x1b[13u");
        let shift_l = KeyInput {
            shift: true,
            location: Location::Left,
            ..key("shift")
        };
        assert_eq!(enc(&shift_l, 8), "\x1b[57441;2u");
        let ctrl_r = KeyInput {
            ctrl: true,
            location: Location::Right,
            ..key("ctrl")
        };
        assert_eq!(enc(&ctrl_r, 8), "\x1b[57448;5u");
        assert_eq!(enc(&key("capslock"), 8), "\x1b[57358u");
        let shift_a = KeyInput {
            code: "A",
            physical: "a",
            text: Some("A"),
            shift: true,
            ..Default::default()
        };
        assert_eq!(enc(&shift_a, 8), "\x1b[97;2u");
        assert_eq!(enc(&shift_a, 8 | 16), "\x1b[97;2;65u");
        assert_eq!(enc(&key("a"), 8 | 16), "\x1b[97;;97u");
        let kp_1 = KeyInput {
            location: Location::Numpad,
            num_lock: true,
            ..key("1")
        };
        assert_eq!(enc(&kp_1, 8), "\x1b[57400;129u");
    }

    /// Without flag 1 or 8 the keypad reads as the main block, and a
    /// chord keeps its legacy bytes where they exist.
    #[test]
    fn the_other_flags_alone_keep_legacy_bytes() {
        let kp_enter = KeyInput {
            location: Location::Numpad,
            ..key("enter")
        };
        assert_eq!(enc(&kp_enter, 2), "\r");
        let ctrl_c = KeyInput {
            ctrl: true,
            text: None,
            ..key("c")
        };
        assert_eq!(enc(&ctrl_c, 2), "\x03");
        assert_eq!(enc(&key("escape"), 2), "\x1b");
        // Event types are no legacy mode: F1 is `CSI P`. Only the
        // alternates or the text alone leave the SS3 forms and xterm's
        // menu key as they were.
        let f1 = key("f1");
        assert_eq!(enc(&f1, 2), "\x1b[P");
        assert_eq!(enc(&f1, 4), "\x1bOP");
        assert_eq!(
            String::from_utf8(encode(&key("up"), 4, true).unwrap()).unwrap(),
            "\x1bOA"
        );
        assert_eq!(enc(&key("menu"), 4), "\x1b[29~");
        assert_eq!(enc(&key("menu"), 1), "\x1b[57363u");
    }
}
