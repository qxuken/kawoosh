//! Keys in map notation as the presses a keyboard would make: what the
//! harness presses in a test (`Harness::press`) and what a headless
//! kawoosh is sent over its socket (`kawoosh keys`, docs/design/headless.md
//! Decision 4) — one reading, so a script and a live run press the same.

use kawoosh_editor::keymap::{LEADER, parse_notation};
use kui_native::{InputEvent, KeyCode, KeyMods, KeyPress};

/// `jj`, `<C-w>v`, `<leader>f`, `<A-J>`, `:w<CR>` as `(name, mods)` presses,
/// each by the name a binding spells it (`"a"`, `"enter"`, `"f2"`): a
/// named key by its name, a chord's upper-case letter as the letter with
/// Shift, `<leader>` as what `leader` stands for, a plain character as
/// itself and a space as `space`.
pub fn presses(notation: &str, leader: &str) -> Vec<(String, KeyMods)> {
    let mut out = Vec::new();
    for tok in parse_notation(notation) {
        if tok == LEADER {
            // A leader spelt `<leader>` would never end.
            if leader != LEADER {
                out.extend(presses(leader, LEADER));
            }
            continue;
        }
        let Some(inner) = tok.strip_prefix('<').and_then(|s| s.strip_suffix('>')) else {
            for c in tok.chars() {
                let name = if c == ' ' {
                    "space".to_string()
                } else {
                    c.to_string()
                };
                out.push((name, KeyMods::NONE));
            }
            continue;
        };
        let mut mods = KeyMods::default();
        let mut parts: Vec<&str> = inner.split('-').collect();
        // `<C-->`: a dash under a modifier.
        if parts.len() > 1 && parts.last() == Some(&"") {
            parts.pop();
            parts.pop();
            parts.push("-");
        }
        let base = parts.pop().unwrap_or("");
        for m in parts {
            match m {
                "C" | "c" => mods.ctrl = true,
                "A" | "a" | "M" | "m" => mods.alt = true,
                "D" | "d" => mods.super_key = true,
                "S" | "s" => mods.shift = true,
                _ => {}
            }
        }
        let named = match base {
            "Esc" => Some("escape"),
            "CR" | "Enter" | "Return" => Some("enter"),
            "Tab" => Some("tab"),
            "BS" => Some("backspace"),
            "Del" => Some("delete"),
            "Space" => Some("space"),
            "Up" => Some("up"),
            "Down" => Some("down"),
            "Left" => Some("left"),
            "Right" => Some("right"),
            "Home" => Some("home"),
            "End" => Some("end"),
            "PageUp" => Some("pageup"),
            "PageDown" => Some("pagedown"),
            "Insert" => Some("insert"),
            _ => None,
        };
        let chord = mods.ctrl || mods.alt || mods.super_key;
        let name: String = match named {
            Some(n) => n.to_string(),
            None if base.starts_with('F') && base.len() > 1 => base.to_lowercase(),
            None if chord && base.len() == 1 && base.as_bytes()[0].is_ascii_uppercase() => {
                mods.shift = true;
                base.to_ascii_lowercase()
            }
            None => base.to_string(),
        };
        out.push((name, mods));
    }
    out
}

/// One press as the input a keyboard sends: the raw press — its
/// character, or the space, as its text when no chord modifier is held —
/// then the editor event the press means, then the release. kui's
/// `testing::Drive::key` sends the same three.
pub fn key_events(name: &str, mods: KeyMods) -> Vec<InputEvent> {
    let code = KeyCode::from_name(name).unwrap_or(KeyCode::Unknown);
    let mut press = KeyPress::new(code, mods);
    let chord = mods.ctrl || mods.alt || mods.super_key;
    match code {
        KeyCode::Char(c) if !chord => press = press.with_text(c.to_string()),
        KeyCode::Space if !chord => press = press.with_text(" "),
        _ => {}
    }
    let mut out = vec![InputEvent::KeyDown(press.clone())];
    out.extend(press.edit_event());
    out.push(InputEvent::KeyUp(press.released()));
    out
}

#[cfg(test)]
mod tests {
    use super::presses;
    use kui_native::KeyMods;

    fn names(n: &str, leader: &str) -> Vec<String> {
        presses(n, leader)
            .into_iter()
            .map(|(k, m)| {
                let mut s = String::new();
                for (on, c) in [
                    (m.ctrl, "C-"),
                    (m.alt, "A-"),
                    (m.super_key, "D-"),
                    (m.shift, "S-"),
                ] {
                    if on {
                        s.push_str(c);
                    }
                }
                s + &k
            })
            .collect()
    }

    #[test]
    fn notation_reads_as_the_presses_a_keyboard_makes() {
        assert_eq!(names("jj", " "), ["j", "j"]);
        assert_eq!(names("<C-w>v", " "), ["C-w", "v"]);
        assert_eq!(names(":w<CR>", " "), [":", "w", "enter"]);
        assert_eq!(names("<A-J>", " "), ["A-S-j"]);
        assert_eq!(names("<leader>f", " "), ["space", "f"]);
        assert_eq!(names("<leader>f", "<Space>"), ["space", "f"]);
        assert_eq!(names("<C-->", " "), ["C--"]);
        assert_eq!(names("<F2><Esc>", " "), ["f2", "escape"]);
        assert_eq!(presses("x", " ")[0].1, KeyMods::NONE);
    }
}
