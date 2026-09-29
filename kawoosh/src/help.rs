//! `:help` and `:tutor` (roadmap step 53). The pages are
//! `kawoosh/help/*.md`, shipped in the binary. `:help` writes them to a
//! directory of this run's own and opens one read-only in the markdown
//! buffer, so a page renders, its links follow with `gx` as any markdown
//! file's do, and a link's `#heading` lands on the heading. Two pages
//! are written from the editor as it runs, so they are never behind it:
//! `commands.md` (every command, a plugin's among them, with its doc) and
//! `keys.md` (every binding by mode). `:tutor` opens the tutorial as a
//! scratch to edit freely.

use std::fmt::Write as _;
use std::path::PathBuf;

use kawoosh_editor::{ArgKind, Args, Mode, Spec};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};

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

/// This run's directory for the pages.
fn help_dir() -> PathBuf {
    std::env::temp_dir().join(format!("kawoosh-help-{}", std::process::id()))
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
            for (keys, b) in self.ed.keymap.bindings(mode) {
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
        if let Some(id) = self.ed.buffer_at(&path) {
            self.ed.buffers[id].read_only = true;
        }
    }

    /// `:tutor`: the tutorial in a scratch of its own, to edit freely.
    pub(crate) fn tutor(&mut self) {
        let mut b = kawoosh_doc::Buffer::new("tutor", TUTOR);
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
