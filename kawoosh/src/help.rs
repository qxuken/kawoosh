//! `:help` and `:tutor` (roadmap step 53). The pages are
//! `kawoosh/help/*.md`, shipped in the binary. `:help` writes them to a
//! directory of this run's own and opens one in the markdown buffer,
//! read-only (a page there is, however it is reached) and so rendered
//! whole whatever `markdown.reveal` says, its links followed with `gx`
//! as any markdown file's are, a link's `#heading` landing on the
//! heading. Two pages
//! are written from the editor as it runs, so they are never behind it:
//! `commands.md` (every command, a plugin's among them, with its doc) and
//! `keys.md` (every binding by mode). `:tutor` opens the tutorial as a
//! scratch to edit freely.

use std::fmt::Write as _;
use std::path::PathBuf;

use kawoosh_editor::{ArgKind, Args, Mode, Spec};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::icons::Host;

/// The pages, by name.
pub const PAGES: &[(&str, &str)] = &[
    ("index", include_str!("../help/index.md")),
    ("start", include_str!("../help/start.md")),
    ("editing", include_str!("../help/editing.md")),
    ("panes", include_str!("../help/panes.md")),
    ("files", include_str!("../help/files.md")),
    ("search", include_str!("../help/search.md")),
    ("code", include_str!("../help/code.md")),
    ("vcs", include_str!("../help/vcs.md")),
    ("terminal", include_str!("../help/terminal.md")),
    ("memory", include_str!("../help/memory.md")),
    ("look", include_str!("../help/look.md")),
    ("settings", include_str!("../help/settings.md")),
    ("lua", include_str!("../help/lua.md")),
    ("remote", include_str!("../help/remote.md")),
];

/// The tutorial `:tutor` opens.
pub const TUTOR: &str = include_str!("../help/tutor.md");

/// A page as `host`'s keyboard reads it. The pages are written for a
/// Mac and say where another keyboard differs:
///
/// - `{{mac:TEXT}}` is kept on a Mac and `{{pc:TEXT}}` on Windows and
///   Linux, the other dropped — where the key is another key, not
///   another spelling: ``{{mac:`⌘s`}}{{pc:`<C-s>`}}``. `TEXT` runs to
///   the next `}}`, over lines too.
/// - What is left of ⌃ ⌥ ⇧ ⌘ off a Mac is spelled as the caps spell
///   it (`icons::caps_on`): before a key the chord, `⌥/` as `alt+/`
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

/// This run's directory for the pages.
fn help_dir() -> PathBuf {
    std::env::temp_dir().join(format!("kawoosh-help-{}", std::process::id()))
}

/// Whether `path` is one of this run's pages, however it is reached —
/// `:help`, a link's `gx`, `:e` — so it opens read-only.
pub(crate) fn is_page(path: &std::path::Path) -> bool {
    path.starts_with(help_dir())
}

/// Where a topic is: a page, and a line of it to land on.
struct Place {
    page: String,
    needle: Option<String>,
}

impl Kawoosh {
    /// The pages written out as they are now, the generated two among
    /// them.
    fn write_help(&self) -> std::io::Result<PathBuf> {
        let dir = help_dir();
        std::fs::create_dir_all(&dir)?;
        for (name, text) in PAGES {
            let text = for_host(text, Host::HERE);
            std::fs::write(dir.join(format!("{name}.md")), text)?;
        }
        std::fs::write(dir.join("commands.md"), self.commands_page())?;
        std::fs::write(dir.join("keys.md"), self.keys_page())?;
        Ok(dir)
    }

    /// Every command, name first, its aliases, its arguments and its doc.
    fn commands_page(&self) -> String {
        let mut s = String::from(
            "# Commands\n\nEvery command kawoosh has as it runs now, a plugin's and your \
             `init.lua`'s among them. `:NAME` runs one, `:help NAME` comes to it here; a \
             key runs one too ([keys](keys.md)).\n\n",
        );
        let mut specs = self.ed.commands.specs();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        for spec in specs {
            let _ = write!(s, "- `:{}`", spec.name);
            let args = spec.args.names();
            if !args.is_empty() {
                let _ = write!(s, " {}", args.join(" ").to_uppercase());
            }
            if !spec.aliases.is_empty() {
                let aliases: Vec<String> = spec.aliases.iter().map(|a| format!("`:{a}`")).collect();
                let _ = write!(s, " ({})", aliases.join(", "));
            }
            if !spec.doc.is_empty() {
                let _ = write!(s, " — {}", spec.doc);
            }
            s.push('\n');
        }
        s
    }

    /// Every binding, by mode: the keys, the command, its doc.
    fn keys_page(&self) -> String {
        let mut s = String::from(
            "# Keys\n\nEvery key bound as kawoosh runs now, by mode, a plugin's and your \
             `init.lua`'s among them. `<leader>` is the leader key (Space unless `leader` \
             says otherwise); a key marked *in* belongs to that place and is nothing \
             elsewhere, one marked *where* runs only there. `:help KEY` comes to a \
             key here; the [commands](commands.md) say more.\n",
        );
        for (mode, title) in [
            (Mode::Normal, "Normal mode"),
            (Mode::Visual, "Visual mode"),
            (Mode::Insert, "Insert mode"),
            (Mode::OperatorPending, "Operator pending"),
            (Mode::Pane, "In a pane that is not an editor"),
        ] {
            let _ = write!(s, "\n## {title}\n\n");
            for (keys, b) in self.ed.keymap.binding_strokes(mode) {
                if !kawoosh_editor::keymap::listed(&keys) {
                    continue;
                }
                let keys = keys.concat();
                let _ = write!(s, "- `{keys}` — `:{}`", b.line());
                let name = self.ed.commands.resolve(&b.command, &b.args).name;
                if let Some(doc) = self.ed.commands.spec(&name).map(|c| c.doc.as_str())
                    && !doc.is_empty()
                {
                    let _ = write!(s, ": {doc}");
                }
                if let Some(scope) = &b.scope {
                    let _ = write!(s, " *in {}*", self.ed.place_words(scope));
                }
                if !b.when.is_empty() {
                    let facts: Vec<String> = b
                        .when
                        .iter()
                        .map(|c| format!("{}{}", if c.holds { "" } else { "not " }, c.fact))
                        .collect();
                    let _ = write!(s, " *where {}*", facts.join(", "));
                }
                s.push('\n');
            }
        }
        s
    }

    /// Where `topic` is: a page by name, a command, a key, a heading.
    fn find_help(&self, topic: &str) -> Option<Place> {
        let topic = topic.trim().trim_start_matches(':');
        let at = |page: &str, needle: Option<String>| {
            Some(Place {
                page: page.into(),
                needle,
            })
        };
        if topic.is_empty() {
            return at("index", None);
        }
        let page = topic.trim_end_matches(".md");
        if page == "commands" || page == "keys" || PAGES.iter().any(|(n, _)| *n == page) {
            return at(page, None);
        }
        let name = self.ed.commands.canonical(topic).to_string();
        if self.ed.commands.spec(&name).is_some() {
            return at("commands", Some(format!("- `:{name}`")));
        }
        for mode in [Mode::Normal, Mode::Visual, Mode::Insert, Mode::Pane] {
            if self
                .ed
                .keymap
                .bindings(mode)
                .iter()
                .any(|(k, _)| k == topic)
            {
                return at("keys", Some(format!("- `{topic}`")));
            }
        }
        let want = topic.to_lowercase();
        for (name, text) in PAGES {
            if let Some(line) = text
                .lines()
                .find(|l| l.starts_with('#') && l.to_lowercase().contains(&want))
            {
                return at(name, Some(line.to_string()));
            }
        }
        None
    }

    /// `:help [TOPIC]`: the page on TOPIC, read-only, at the topic's line.
    pub(crate) fn help(&mut self, topic: &str) {
        let Some(place) = self.find_help(topic) else {
            self.ed.message = format!("no help for {topic}");
            return;
        };
        let dir = match self.write_help() {
            Ok(dir) => dir,
            Err(e) => {
                self.ed.message = format!("help: {e}");
                return;
            }
        };
        let path = dir.join(format!("{}.md", place.page));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let line = place
            .needle
            .and_then(|n| text.lines().position(|l| l.starts_with(&n)))
            .map(|i| i + 1);
        self.open_in_editor(&path, line, None);
    }

    /// `:tutor`: the tutorial in a scratch of its own, to edit freely.
    /// A scratch has no directory for its links to be relative to, so
    /// they name the pages written out where they are.
    pub(crate) fn tutor(&mut self) {
        let text = match self.write_help() {
            Ok(dir) => absolute_links(TUTOR, &dir),
            Err(_) => TUTOR.to_string(),
        };
        let mut b = kawoosh_doc::Buffer::new("tutor", &text);
        b.language = "markdown".into();
        let id = self.ed.add_buffer(b);
        match self.focused_view().or_else(|| self.claim_launcher()) {
            Some(v) => self.show_buffer(v, id),
            None => {
                let v = self.ed.add_view(id);
                self.layout.open(
                    crate::layout::Content::Editor(v),
                    crate::layout::Place::Column,
                );
            }
        }
    }

    /// The pages' directory gone with the run.
    pub(crate) fn help_teardown(&mut self) {
        let _ = std::fs::remove_dir_all(help_dir());
    }
}

/// `text` with each link to a page — a relative `NAME.md`, its
/// `#heading` kept — made to name the page in `dir`; a path with a space
/// in it inside `<…>`.
fn absolute_links(text: &str, dir: &std::path::Path) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("](") {
        out.push_str(&rest[..i + 2]);
        rest = &rest[i + 2..];
        let end = rest.find([')', ' ']).unwrap_or(rest.len());
        let dest = &rest[..end];
        let page = dest.split('#').next().unwrap_or("");
        if page.ends_with(".md") && !page.contains("://") && !page.starts_with('/') {
            let path = dir.join(dest).display().to_string();
            if path.contains(char::is_whitespace) {
                let _ = write!(out, "<{path}>");
            } else {
                out.push_str(&path);
            }
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    out
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("help")
                .alias(&["h"])
                .args(Args::rest(&[ArgKind::Text]))
                .doc("the help on TOPIC — a page, a command, a key — or the index; read-only, `gx` follows its links"),
            |k, ctx| k.help(&ctx.args.join(" ")),
        ),
        cmd(
            Spec::new("tutor").doc("a hands-on tutorial, in a scratch of its own to try the keys on"),
            |k, _| k.tutor(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page's host spans keep their own keyboard's text, and what is
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

    /// No page leaves a span open, and off a Mac none is left with a
    /// Mac's sign or a chord on the system's key: where ⌘ has a key
    /// there, the page says which.
    #[test]
    fn the_pages_say_a_pc_s_keys() {
        for (name, text) in PAGES.iter().copied().chain([("tutor", TUTOR)]) {
            for host in [Host::Mac, Host::Windows, Host::Linux] {
                let page = for_host(text, host);
                assert!(
                    !page.contains("{{mac:") && !page.contains("{{pc:"),
                    "{name}"
                );
                if host != Host::Mac {
                    for sign in ["⌘", "⌥", "⌃", "⇧", "win+", "super+"] {
                        assert!(!page.contains(sign), "{name}.md on {host:?}: {sign}");
                    }
                }
            }
        }
    }
}
