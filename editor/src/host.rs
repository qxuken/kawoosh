//! The keyboard a text is written for (docs/design/icons.md Decision
//! 4): which system this build is for, and a text — a help page, a
//! command's or a setting's description — read as its keyboard has it.

/// The system whose keyboard a chord's modifiers are spelled for: a
/// Mac's caps print ⌃ ⌥ ⇧ ⌘, a PC's the words, and the key between
/// Ctrl and Alt is Win on Windows and Super on Linux.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Mac,
    Windows,
    Linux,
}

impl Host {
    /// The one this build is for.
    pub const HERE: Host = if cfg!(target_os = "macos") {
        Host::Mac
    } else if cfg!(windows) {
        Host::Windows
    } else {
        Host::Linux
    };
}

/// A text as `host`'s keyboard reads it. The help's pages and the
/// commands' and settings' descriptions are written for a Mac and say
/// where another keyboard differs:
///
/// - `{{mac:TEXT}}` is kept on a Mac and `{{pc:TEXT}}` on Windows and
///   Linux, the other dropped — where the key is another key, not
///   another spelling: ``{{mac:`⌘s`}}{{pc:`<C-s>`}}``. `TEXT` runs to
///   the next `}}`, over lines too.
/// - What is left of ⌃ ⌥ ⇧ ⌘ off a Mac is spelled as the caps spell
///   it (kawoosh's `icons::caps_on`): before a key the chord, `⌥/` as `alt+/`
///   and `⌘⇧F` as `win+shift+f` (`super+` on Linux); alone, or before
///   `-click`, the key's name, `Alt`, `Win`. A key is a letter, a
///   digit, one of `⌫ ⌦ ← → ↑ ↓`, or inside backticks anything but a
///   space: in prose a `.` or a `)` after the sign ends a sentence.
pub fn for_host(text: &str, host: Host) -> String {
    let mac = host == Host::Mac;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("{{") {
        out.push_str(&rest[..at]);
        let span = &rest[at + 2..];
        let kept = match (span.strip_prefix("mac:"), span.strip_prefix("pc:")) {
            (Some(inner), _) => Some((inner, mac)),
            (_, Some(inner)) => Some((inner, !mac)),
            _ => None,
        };
        match kept.and_then(|(inner, keep)| Some((inner, inner.find("}}")?, keep))) {
            Some((inner, end, keep)) => {
                if keep {
                    out.push_str(&inner[..end]);
                }
                rest = &inner[end + 2..];
            }
            // Not a span of ours: braces a page writes for themselves.
            None => {
                out.push_str("{{");
                rest = span;
            }
        }
    }
    out.push_str(rest);
    if mac {
        out
    } else {
        spell_modifiers(&out, host)
    }
}

/// The Mac's modifier signs in `text` as a PC's words ([`for_host`]).
fn spell_modifiers(text: &str, host: Host) -> String {
    const KEYS: &str = "⌫⌦←→↑↓";
    let sign = |c: char| "⌃⌥⇧⌘".contains(c);
    let sup = if host == Host::Windows {
        "win"
    } else {
        "super"
    };
    let mut out = String::with_capacity(text.len());
    let mut code = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !sign(c) {
            code ^= c == '`';
            out.push(c);
            continue;
        }
        let mut held = vec![c];
        while let Some(&m) = chars.peek().filter(|m| sign(**m)) {
            held.push(m);
            chars.next();
        }
        // A PC's order, the system's key first.
        let words: Vec<&str> = [('⌘', sup), ('⌃', "ctrl"), ('⌥', "alt"), ('⇧', "shift")]
            .into_iter()
            .filter(|(m, _)| held.contains(m))
            .map(|(_, w)| w)
            .collect();
        let key = chars.peek().copied().filter(|k| {
            k.is_alphanumeric() || KEYS.contains(*k) || (code && *k != '`' && !k.is_whitespace())
        });
        match key {
            Some(k) => {
                chars.next();
                out.push_str(&words.join("+"));
                out.push('+');
                out.push(k.to_ascii_lowercase());
            }
            None => {
                let names: Vec<String> = words
                    .iter()
                    .map(|w| w[..1].to_uppercase() + &w[1..])
                    .collect();
                out.push_str(&names.join("+"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A text's host spans keep their own keyboard's text, and what is
    /// left of the Mac's signs is a PC's words there.
    #[test]
    fn a_page_reads_as_the_hosts_keyboard() {
        let page = "| {{mac:`⌘=` `⌘+`}}{{pc:`<C-=>` `<C-+>`}} | bigger |\n\
                    `<C-s>`{{mac: (`⌘s`)}} writes. A legend is `⌥/ keys`; `⌥m` `⌥u` filter.\n\
                    {{mac:| `<D-BS>` (⌘⌫) | to the start |\n}}\
                    Search with ⌘⇧F, hold `⌘`, and ⌘-click; raw keeps ⌘. ⌥⌫ deletes a word (no ⌘).";
        assert_eq!(
            for_host(page, Host::Mac),
            "| `⌘=` `⌘+` | bigger |\n\
             `<C-s>` (`⌘s`) writes. A legend is `⌥/ keys`; `⌥m` `⌥u` filter.\n\
             | `<D-BS>` (⌘⌫) | to the start |\n\
             Search with ⌘⇧F, hold `⌘`, and ⌘-click; raw keeps ⌘. ⌥⌫ deletes a word (no ⌘)."
        );
        assert_eq!(
            for_host(page, Host::Windows),
            "| `<C-=>` `<C-+>` | bigger |\n\
             `<C-s>` writes. A legend is `alt+/ keys`; `alt+m` `alt+u` filter.\n\
             Search with win+shift+f, hold `Win`, and Win-click; raw keeps Win. \
             alt+⌫ deletes a word (no Win)."
        );
        assert!(for_host(page, Host::Linux).contains("super+shift+f, hold `Super`"));
        // Braces that are not a span stay, and so does an open one.
        assert_eq!(
            for_host("a {{b}} {{mac:c", Host::Windows),
            "a {{b}} {{mac:c"
        );
    }
}
