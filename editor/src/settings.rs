//! Settings: a tree of data with a layer per source (kui.md D10).
//!
//! A [`Setting`] is what a Lua table can hold — a scalar, a list, a
//! table of them — and what every reader in the engine and the shell
//! asks for by dotted path (`tabstop`, `lsp.rust.cmd`). [`Settings`]
//! keeps one [`Layer`] per source — the defaults the engine ships, the
//! user's file, the project's files, the session's `:set` — and the
//! effective tree is their merge in that order, so a project overrides
//! the user, `:set` overrides the project, and swapping the project
//! layer (a `:cd`) leaves what was typed alone. A table over a table
//! merges key by key; anything else replaces.

use std::collections::BTreeMap;
use std::fmt;

/// One value in the tree. What Lua data can be, minus functions: a
/// settings file returns one of these, and a reader takes it typed.
#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Setting>),
    Table(BTreeMap<String, Setting>),
}

impl Setting {
    pub fn table() -> Self {
        Setting::Table(BTreeMap::new())
    }

    pub fn is_table(&self) -> bool {
        matches!(self, Setting::Table(_))
    }

    /// The value at a dotted `path`; the tree itself for an empty one.
    pub fn get(&self, path: &str) -> Option<&Setting> {
        if path.is_empty() {
            return Some(self);
        }
        let mut cur = self;
        for key in path.split('.') {
            let Setting::Table(t) = cur else { return None };
            cur = t.get(key)?;
        }
        Some(cur)
    }

    /// Puts `value` at `path`, making tables along the way; a scalar in
    /// the way becomes a table, since the path said so.
    pub fn set(&mut self, path: &str, value: Setting) {
        if path.is_empty() {
            *self = value;
            return;
        }
        let mut cur = self;
        let mut keys = path.split('.').peekable();
        while let Some(key) = keys.next() {
            if !cur.is_table() {
                *cur = Setting::table();
            }
            let Setting::Table(t) = cur else {
                unreachable!()
            };
            if keys.peek().is_none() {
                t.insert(key.to_string(), value);
                return;
            }
            cur = t.entry(key.to_string()).or_insert_with(Setting::table);
        }
    }

    /// Takes the value at `path` out, if it was there.
    pub fn remove(&mut self, path: &str) -> Option<Setting> {
        let (parent, key) = match path.rsplit_once('.') {
            Some((p, k)) => (p, k),
            None => ("", path),
        };
        let parent = if parent.is_empty() {
            self
        } else {
            self.get_mut(parent)?
        };
        match parent {
            Setting::Table(t) => t.remove(key),
            _ => None,
        }
    }

    fn get_mut(&mut self, path: &str) -> Option<&mut Setting> {
        let mut cur = self;
        for key in path.split('.') {
            let Setting::Table(t) = cur else { return None };
            cur = t.get_mut(key)?;
        }
        Some(cur)
    }

    /// Lays `over` onto this: a table over a table merges key by key,
    /// down the tree; anything else — a scalar, a list, a scalar over a
    /// table — replaces.
    pub fn merge(&mut self, over: Setting) {
        match (self, over) {
            (Setting::Table(base), Setting::Table(over)) => {
                for (k, v) in over {
                    match base.get_mut(&k) {
                        Some(slot) => slot.merge(v),
                        None => {
                            base.insert(k, v);
                        }
                    }
                }
            }
            (slot, over) => *slot = over,
        }
    }

    /// Every leaf's dotted path, in order — what the command line
    /// completes for `:set`. An empty table under a name is a leaf: it
    /// is a name; an empty tree is no leaves at all.
    pub fn paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        if matches!(self, Setting::Table(t) if t.is_empty()) {
            return out;
        }
        self.collect_paths("", &mut out);
        out
    }

    fn collect_paths(&self, prefix: &str, out: &mut Vec<String>) {
        match self {
            Setting::Table(t) if !t.is_empty() => {
                for (k, v) in t {
                    let path = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    v.collect_paths(&path, out);
                }
            }
            _ => out.push(prefix.to_string()),
        }
    }

    // ---------------------------------------------------------- typed reads
    //
    // Lenient where it costs nothing: a `:set tabstop=2` that landed as
    // a string still reads as the number it spells.

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Setting::Bool(b) => Some(*b),
            Setting::Str(s) => match s.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Setting::Int(i) => Some(*i),
            Setting::Float(f) if f.fract() == 0.0 => Some(*f as i64),
            Setting::Str(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Setting::Int(i) => Some(*i as f64),
            Setting::Float(f) => Some(*f),
            Setting::Str(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Setting::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Setting]> {
        match self {
            Setting::List(l) => Some(l),
            _ => None,
        }
    }

    /// `text` as the command line typed it, shaped like `like` when
    /// there is one — `2` stays a string where the setting is one —
    /// and else by what it spells: `true`, a number, or text.
    pub fn parse_like(text: &str, like: Option<&Setting>) -> Setting {
        match like {
            Some(Setting::Bool(_)) => match text {
                "true" => Setting::Bool(true),
                "false" => Setting::Bool(false),
                _ => Setting::Str(text.to_string()),
            },
            Some(Setting::Int(_)) => text
                .parse()
                .map(Setting::Int)
                .or_else(|_| text.parse().map(Setting::Float))
                .unwrap_or_else(|_| Setting::Str(text.to_string())),
            Some(Setting::Float(_)) => text
                .parse()
                .map(Setting::Float)
                .unwrap_or_else(|_| Setting::Str(text.to_string())),
            Some(Setting::Str(_)) => Setting::Str(text.to_string()),
            _ => match text {
                "true" => Setting::Bool(true),
                "false" => Setting::Bool(false),
                _ => text
                    .parse()
                    .map(Setting::Int)
                    .or_else(|_| text.parse().map(Setting::Float))
                    .unwrap_or_else(|_| Setting::Str(text.to_string())),
            },
        }
    }
}

/// A setting as Lua would spell it: `2`, `true`, `"x"`, `{ a = 1 }`.
impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Setting::Bool(b) => write!(f, "{b}"),
            Setting::Int(i) => write!(f, "{i}"),
            Setting::Float(x) => write!(f, "{x}"),
            Setting::Str(s) => write!(f, "{s:?}"),
            Setting::List(l) => {
                write!(f, "{{ ")?;
                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, " }}")
            }
            Setting::Table(t) if t.is_empty() => write!(f, "{{}}"),
            Setting::Table(t) => {
                write!(f, "{{ ")?;
                for (i, (k, v)) in t.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k} = {v}")?;
                }
                write!(f, " }}")
            }
        }
    }
}

/// Where a setting came from, lowest first: each layer overrides the
/// ones before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    /// What the engine ships.
    Default,
    /// The user's `settings.lua`, and what `init.lua` sets.
    User,
    /// The project's `.kawoosh/settings.lua` files, outermost first.
    Project,
    /// `:set`, and what a plugin sets while running.
    Session,
}

impl Layer {
    pub const ALL: [Layer; 4] = [Layer::Default, Layer::User, Layer::Project, Layer::Session];

    pub fn name(self) -> &'static str {
        match self {
            Layer::Default => "default",
            Layer::User => "user",
            Layer::Project => "project",
            Layer::Session => "session",
        }
    }
}

/// What a setting holds, as a declaration says it (roadmap step 34):
/// the scalar kinds, a word of a few, a list, or a table whose keys are
/// the user's own (`tools`, `tokens.colors`, a theme's roles).
#[derive(Clone, Debug, PartialEq)]
pub enum SettingKind {
    Bool,
    Int,
    Float,
    Str,
    OneOf(Vec<String>),
    List,
    Open,
}

impl SettingKind {
    /// The kind a default's value is.
    pub fn of(v: &Setting) -> SettingKind {
        match v {
            Setting::Bool(_) => SettingKind::Bool,
            Setting::Int(_) => SettingKind::Int,
            Setting::Float(_) => SettingKind::Float,
            Setting::Str(_) => SettingKind::Str,
            Setting::List(_) => SettingKind::List,
            Setting::Table(_) => SettingKind::Open,
        }
    }
}

/// A setting declared: its kind and a line saying what it does.
#[derive(Clone, Debug, PartialEq)]
pub struct Decl {
    pub kind: SettingKind,
    pub doc: String,
}

/// The layers and their merge. A layer holds its sources in order — a
/// project's files from the root down — each a tree; the effective
/// tree is rebuilt on every change and read for free.
#[derive(Clone, Debug)]
pub struct Settings {
    layers: [Vec<(String, Setting)>; 4],
    /// What each setting is, where more is known than its default
    /// (`declare`): a plugin's with no default, an open table, a doc.
    decls: BTreeMap<String, Decl>,
    effective: Setting,
    /// Bumped on every change, so a reader that derives something from
    /// the tree (the keymap's leader) knows when to look again.
    version: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    /// The engine's defaults, and nothing over them.
    pub fn new() -> Self {
        let mut defaults = Setting::table();
        defaults.set("tabstop", Setting::Int(4));
        defaults.set("expandtab", Setting::Bool(true));
        defaults.set("scrolloff", Setting::Int(3));
        // The gutter numbers each line by its distance from the caret's,
        // which keeps its own number (vim's `number relativenumber`).
        defaults.set("relativenumber", Setting::Bool(false));
        defaults.set("leader", Setting::Str(" ".into()));
        // The which-key float while a key sequence is open.
        defaults.set("whichkey", Setting::Bool(true));
        // The scrolling tab (docs/design/scrolling-tab.md): what a new
        // tab is (`tree` | `scroll`; the strip since 2026-09-22), a
        // new column's width (`third`,
        // `half`, `two-thirds`, `full`, or a fraction), the gap between
        // columns in px, and whether the focus frame centres the column
        // (`always`) or only brings it into view (`never`).
        defaults.set("layout.default", Setting::Str("scroll".into()));
        defaults.set("layout.column_width", Setting::Str("half".into()));
        defaults.set("layout.gap", Setting::Int(4));
        defaults.set("layout.scroll.center", Setting::Str("never".into()));
        // What a pane made bare is (docs/design/launcher.md): a split
        // (`<C-w>v`, `:vsplit`) and a tab (`:tabnew`) without a path —
        // `launcher` (asks), `same` (the buffer split from, vim's),
        // `scratch`, `terminal`, `dir` (the directory as a listing).
        defaults.set("layout.new_pane", Setting::Str("launcher".into()));
        defaults.set("layout.new_tab", Setting::Str("launcher".into()));
        // The dock: a `tree` of splits, or a `scroll` strip of columns
        // (roadmap step 32, the experiment).
        defaults.set("layout.dock", Setting::Str("tree".into()));
        // How the launcher opens: `normal`, where a letter launches and
        // `i` or `/` searches, or `insert`, typing filtering at once.
        defaults.set("launcher.start", Setting::Str("normal".into()));
        // Which buffers the lists show — the buffers picker, `:ls`,
        // `]b` — `tab`: the focused tab's (a file under its directory,
        // or one it shows), or `all`.
        defaults.set("buffers.scope", Setting::Str("tab".into()));
        // The markdown buffer (docs/design/markdown.md): drawn rendered —
        // marks folded, headings at their sizes (h1 to h6, a ratio of
        // the body), prose wrapped — and images past this many MB left
        // as their text.
        defaults.set("markdown.render", Setting::Bool(true));
        defaults.set(
            "markdown.heading",
            Setting::List(vec![
                Setting::Float(1.6),
                Setting::Float(1.35),
                Setting::Float(1.15),
                Setting::Float(1.0),
            ]),
        );
        defaults.set("markdown.image_max_mb", Setting::Int(16));
        // `p` puts what was copied in another program too: the system
        // clipboard, read when the window or an editor pane gets the
        // keys back, is the register's newest when it is news.
        defaults.set("clipboard.system", Setting::Bool(true));
        // Auto-closing brackets (`pairs.lua`, docs/design/pairs.md): on
        // since 2026-09-23; `pairs.rules` is the plugin's, per language.
        defaults.set("pairs.enabled", Setting::Bool(true));
        // Inlay hints from the language server — types, parameter names —
        // drawn in the line, faint (`<leader>cI` flips it).
        defaults.set("lsp.inlay_hints", Setting::Bool(false));
        // Secrets (docs/design/secrets.md): the masks, named rules a
        // user adds to or switches off (`secrets.masks.env = false`);
        // a file a `files` rule names is private. A secret in the
        // register is forgotten after `forget_secs`; `zv` shows a mask
        // for `reveal_secs`; a `--wait` open under the temp directory
        // is private; a buffer past `scan_max_kb` is not scanned.
        {
            let s = |v: &str| Setting::Str(v.into());
            let rule = |pairs: Vec<(&str, Setting)>| {
                Setting::Table(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
            };
            let masks = rule(vec![
                (
                    "env",
                    rule(vec![
                        (
                            "files",
                            Setting::List(vec![s(".env"), s(".env.*"), s("*.env")]),
                        ),
                        (
                            "pattern",
                            s(r"^\s*(?:export\s+)?[A-Za-z_][A-Za-z0-9_.]*\s*=\s*(.+)$"),
                        ),
                    ]),
                ),
                (
                    "vault",
                    rule(vec![
                        (
                            "files",
                            Setting::List(vec![s("vault.yml"), s("vault.yaml")]),
                        ),
                        ("pattern", s(r"^\s*[A-Za-z_][A-Za-z0-9_]*\s*:\s*(\S.*)$")),
                    ]),
                ),
                (
                    "key",
                    rule(vec![
                        ("files", s("*.key")),
                        // Every line but a comment.
                        ("pattern", s(r"^\s*([^#\s].*)$")),
                    ]),
                ),
                (
                    "vault_pass",
                    rule(vec![("files", s(".vault_pass")), ("pattern", s(r"^(.+)$"))]),
                ),
                (
                    "pem",
                    rule(vec![
                        ("from", s(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
                        ("to", s(r"-----END [A-Z ]*PRIVATE KEY-----")),
                    ]),
                ),
                (
                    "secret",
                    rule(vec![
                        ("language", s("secret")),
                        ("pattern", s(r"^[^:#]+:\s*(\S.*)$")),
                    ]),
                ),
            ]);
            defaults.set("secrets.masks", masks);
        }
        defaults.set("secrets.forget_secs", Setting::Int(30));
        defaults.set("secrets.reveal_secs", Setting::Int(10));
        defaults.set("secrets.private_temp", Setting::Bool(true));
        defaults.set("secrets.scan_max_kb", Setting::Int(1024));
        // The lines of history a terminal keeps; a smaller number drops
        // what is past it at once.
        defaults.set("terminal.scrollback", Setting::Int(10_000));
        // The program a terminal runs (`nu`, `pwsh`, a path): empty for
        // `$SHELL`, else `/bin/sh`, or `%ComSpec%` on Windows.
        defaults.set("terminal.shell", Setting::Str(String::new()));
        // What a terminal's BEL does: `sound`, `visual` (its tab marked
        // when it is not in front, no sound) or `off`; and whether the
        // editor rings for its own failures — a search with no match.
        defaults.set("terminal.bell", Setting::Str("sound".into()));
        defaults.set("editor.bell", Setting::Bool(false));
        // The selection's corners, in logical px: 0 square, as a text's
        // span backgrounds are; more rounds the selection as one shape
        // across its lines, the corners where a line reaches past its
        // neighbour convex and where it falls short concave (rows.rs).
        defaults.set("editor.selection_radius", Setting::Float(0.0));
        // The shell whose PATH the window's children get when it was
        // opened outside a terminal — from Finder, the Dock (kawoosh's
        // `shell_env`): a path to it, since a bare name is looked up on
        // the PATH it is there to fix; empty for `$SHELL`. Read at
        // startup.
        defaults.set("env.shell", Setting::Str(String::new()));
        // The memory (docs/design/memory.md): days a moment — a file
        // attended, with its history and draft — may go unattended
        // before it is forgotten; 0 keeps every row.
        defaults.set("memory.keep_days", Setting::Int(90));
        // The most the memory and the histories may add up to in the
        // store, in megabytes; past it the lowest-scored rows go. 0
        // for no cap.
        defaults.set("memory.max_mb", Setting::Int(64));
        // Texts (yanks, deletes, clipboard pastes) kept across a
        // restart: their days, and the most they may weigh — 0 writes
        // none to disk, the register still works for the session.
        defaults.set("memory.text.keep_days", Setting::Int(7));
        defaults.set("memory.text.max_mb", Setting::Int(8));
        // Seconds without a key or a click after which dwell stops
        // counting.
        defaults.set("memory.idle_secs", Setting::Int(60));
        // The picker (`picker.lua`): a preview of the cursor's row
        // beside the list, and whether a row's text wraps to show the
        // whole of a long path; `<A-p>` and `<A-w>` in the picker flip
        // them for the session. `share` is the height the pane opens
        // at, as a fraction of the pane it splits, and `split` the
        // list's share of the pane's width beside the preview; the
        // pane keys `<A-J>` `<A-K>` and the picker's `<A-H>` `<A-L>`
        // move them for the session, and both dividers drag.
        defaults.set("picker.preview", Setting::Bool(true));
        defaults.set("picker.wrap", Setting::Bool(false));
        defaults.set("picker.share", Setting::Float(0.5));
        defaults.set("picker.split", Setting::Float(0.5));
        // The look (kawoosh's `look.rs`): the mono face — a family kui can
        // see, the empty string for the face the editor ships — its size
        // in logical px, the row's height as a ratio of it, and OpenType
        // features in kui's spelling (`-liga tnum`); the chrome's palette
        // follows the OS (`system`) or is pinned `dark` or `light`, with
        // `theme.accent` and any role of the theme by name beside it;
        // `tokens.colors` names a syntax token's colour, one or a light
        // and a dark half.
        defaults.set("font.family", Setting::Str(String::new()));
        defaults.set("font.size", Setting::Int(13));
        defaults.set("font.line_height", Setting::Float(1.5));
        defaults.set("font.features", Setting::Str(String::new()));
        // The chrome's text (tabs, title bars, the strips): `0` follows
        // `font.size` up to a cap, a number is its own size.
        defaults.set("font.chrome_size", Setting::Int(0));
        defaults.set("theme.appearance", Setting::Str("system".into()));
        // A family of kawoosh's own (`themes.rs`), or `system` for kui's
        // roles off the OS; `theme.dark` and `theme.light` a variant for
        // each base apart from it, empty for the family's half.
        defaults.set("theme.name", Setting::Str("rose-pine".into()));
        defaults.set("theme.dark", Setting::Str(String::new()));
        defaults.set("theme.light", Setting::Str(String::new()));
        let mut s = Self {
            layers: Default::default(),
            decls: BTreeMap::new(),
            effective: Setting::table(),
            version: 0,
        };
        s.replace(
            Layer::Default,
            vec![(Layer::Default.name().to_string(), defaults)],
        );
        // The defaults that are a word of a few: what the language
        // server's types and `:set`'s completion offer.
        let words = |w: &[&str]| SettingKind::OneOf(w.iter().map(|w| w.to_string()).collect());
        let bare = ["launcher", "same", "scratch", "terminal", "dir"];
        for (path, kind) in [
            ("layout.default", words(&["tree", "scroll"])),
            ("layout.scroll.center", words(&["always", "never"])),
            ("layout.new_pane", words(&bare)),
            ("layout.new_tab", words(&bare)),
            ("layout.dock", words(&["tree", "scroll"])),
            ("launcher.start", words(&["normal", "insert"])),
            ("buffers.scope", words(&["tab", "all"])),
            ("theme.appearance", words(&["system", "dark", "light"])),
        ] {
            s.declare(path, kind, "");
        }
        s
    }

    /// The merge of every layer.
    pub fn effective(&self) -> &Setting {
        &self.effective
    }

    /// Changes so far: differs from the last value seen when anything
    /// was set, unset or replaced since.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn get(&self, path: &str) -> Option<&Setting> {
        self.effective.get(path)
    }

    pub fn bool(&self, path: &str) -> Option<bool> {
        self.get(path)?.as_bool()
    }

    pub fn int(&self, path: &str) -> Option<i64> {
        self.get(path)?.as_int()
    }

    pub fn str(&self, path: &str) -> Option<&str> {
        self.get(path)?.as_str()
    }

    /// Makes `sources` the whole of `layer`.
    pub fn replace(&mut self, layer: Layer, sources: Vec<(String, Setting)>) {
        self.layers[layer as usize] = sources;
        self.rebuild();
    }

    /// Puts one value into `layer`, in the source named after the
    /// layer — `:set`'s into the session, `init.lua`'s into the user's
    /// — made last in the layer when it is not there yet, so it lays
    /// over the layer's files.
    pub fn set(&mut self, layer: Layer, path: &str, value: Setting) {
        let sources = &mut self.layers[layer as usize];
        let own = match sources.iter().position(|(name, _)| name == layer.name()) {
            Some(i) => i,
            None => {
                sources.push((layer.name().to_string(), Setting::table()));
                sources.len() - 1
            }
        };
        sources[own].1.set(path, value);
        self.rebuild();
    }

    /// Takes `path` out of `layer` — what was under it shows again. The
    /// layer's own source goes with its last value: an empty one would
    /// be a source with nothing to say.
    pub fn unset(&mut self, layer: Layer, path: &str) {
        for (_, s) in &mut self.layers[layer as usize] {
            s.remove(path);
        }
        self.layers[layer as usize].retain(|(name, s)| {
            name != layer.name() || !matches!(s, Setting::Table(t) if t.is_empty())
        });
        self.rebuild();
    }

    /// Declares `path` (roadmap step 34): what it holds and what it
    /// does. A setting with a default in the engine's layer is declared
    /// by it; this is for one read without a default, an open table,
    /// and a doc. Declared again, the last word stands.
    pub fn declare(&mut self, path: &str, kind: SettingKind, doc: &str) {
        self.decls.insert(
            path.to_string(),
            Decl {
                kind,
                doc: doc.to_string(),
            },
        );
    }

    /// What `path` holds: its declaration's kind, else its default's.
    pub fn kind(&self, path: &str) -> Option<SettingKind> {
        if let Some(d) = self.decls.get(path) {
            return Some(d.kind.clone());
        }
        self.layers[Layer::Default as usize]
            .iter()
            .rev()
            .find_map(|(_, s)| s.get(path))
            .map(SettingKind::of)
    }

    /// Whether `path` is a setting anyone declared: itself, under an
    /// open table declared, or in the engine's defaults.
    pub fn is_declared(&self, path: &str) -> bool {
        let mut p = path;
        loop {
            if let Some(d) = self.decls.get(p) {
                return p == path || d.kind == SettingKind::Open;
            }
            match p.rsplit_once('.') {
                Some((up, _)) => p = up,
                None => break,
            }
        }
        self.layers[Layer::Default as usize]
            .iter()
            .any(|(_, s)| s.get(path).is_some())
    }

    /// The keys the settings files set that no one declared, each with
    /// the file it is in — what a misspelling looks like, since a key
    /// nobody reads is otherwise silence (roadmap step 34). An open
    /// table's keys are the user's; one under a scalar is not a key.
    pub fn undeclared(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for layer in [Layer::User, Layer::Project] {
            for (name, tree) in &self.layers[layer as usize] {
                for path in tree.paths() {
                    if !self.is_declared(&path) {
                        out.push((name.clone(), path));
                    }
                }
            }
        }
        out
    }

    /// Every setting known, its kind and doc: the defaults' by their
    /// values, the declarations over them — what the language server's
    /// types are written from.
    pub fn schema(&self) -> BTreeMap<String, Decl> {
        let mut out = BTreeMap::new();
        for (_, tree) in &self.layers[Layer::Default as usize] {
            for path in tree.paths() {
                if let Some(v) = tree.get(&path) {
                    out.insert(
                        path,
                        Decl {
                            kind: SettingKind::of(v),
                            doc: String::new(),
                        },
                    );
                }
            }
        }
        for (path, d) in &self.decls {
            let doc = if d.doc.is_empty() {
                out.get(path)
                    .map(|o: &Decl| o.doc.clone())
                    .unwrap_or_default()
            } else {
                d.doc.clone()
            };
            // A default's own kind is more exact than a declared `Str`
            // with choices lost; the declaration's wins otherwise.
            out.insert(
                path.clone(),
                Decl {
                    kind: d.kind.clone(),
                    doc,
                },
            );
        }
        out
    }

    /// The sources of `layer`, in order.
    pub fn sources(&self, layer: Layer) -> &[(String, Setting)] {
        &self.layers[layer as usize]
    }

    /// Where the effective value at `path` comes from: the layer and
    /// the source in it that set it last.
    pub fn source_of(&self, path: &str) -> Option<(Layer, &str)> {
        for layer in Layer::ALL.iter().rev() {
            for (name, s) in self.layers[*layer as usize].iter().rev() {
                if s.get(path).is_some() {
                    return Some((*layer, name));
                }
            }
        }
        None
    }

    /// Where `path` comes from, for a person: `project:
    /// /repo/.kawoosh/settings.lua`, or just `session` when the source
    /// is the layer itself.
    pub fn origin(&self, path: &str) -> Option<String> {
        let (layer, src) = self.source_of(path)?;
        Some(if src == layer.name() {
            layer.name().to_string()
        } else {
            format!("{}: {src}", layer.name())
        })
    }

    /// Keeps the sources of `layer` that `keep` says to.
    pub fn retain_sources(&mut self, layer: Layer, keep: impl Fn(&str) -> bool) {
        self.layers[layer as usize].retain(|(name, _)| keep(name));
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let mut eff = Setting::table();
        for layer in &self.layers {
            for (_, s) in layer {
                eff.merge(s.clone());
            }
        }
        self.effective = eff;
        self.version += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tbl(pairs: &[(&str, Setting)]) -> Setting {
        let mut t = Setting::table();
        for (k, v) in pairs {
            t.set(k, v.clone());
        }
        t
    }

    #[test]
    fn dotted_paths_read_write_and_remove() {
        let mut s = Setting::table();
        s.set("lsp.rust.cmd", Setting::Str("rust-analyzer".into()));
        s.set("tabstop", Setting::Int(2));
        assert_eq!(
            s.get("lsp.rust.cmd").and_then(Setting::as_str),
            Some("rust-analyzer")
        );
        assert!(s.get("lsp.rust").unwrap().is_table());
        assert_eq!(s.get("lsp.nope"), None);
        assert_eq!(s.get(""), Some(&s));
        // A scalar in the way becomes a table: the path said so.
        s.set("tabstop.deep", Setting::Bool(true));
        assert_eq!(s.get("tabstop.deep"), Some(&Setting::Bool(true)));
        assert_eq!(
            s.remove("lsp.rust.cmd"),
            Some(Setting::Str("rust-analyzer".into()))
        );
        assert_eq!(s.get("lsp.rust"), Some(&Setting::table()));
        assert_eq!(s.remove("lsp.rust.cmd"), None);
        assert_eq!(s.paths(), ["lsp.rust", "tabstop.deep"]);
    }

    #[test]
    fn a_table_merges_and_anything_else_replaces() {
        let mut base = tbl(&[
            ("tabstop", Setting::Int(4)),
            ("lsp.rust.cmd", Setting::Str("ra".into())),
            (
                "lsp.rust.args",
                Setting::List(vec![Setting::Str("-v".into())]),
            ),
            ("lsp.go.cmd", Setting::Str("gopls".into())),
        ]);
        base.merge(tbl(&[
            ("tabstop", Setting::Int(2)),
            ("lsp.rust.args", Setting::List(vec![])),
            (
                "lsp.rust.roots",
                Setting::List(vec![Setting::Str("Cargo.toml".into())]),
            ),
        ]));
        assert_eq!(base.get("tabstop").and_then(Setting::as_int), Some(2));
        assert_eq!(
            base.get("lsp.rust.cmd").and_then(Setting::as_str),
            Some("ra"),
            "untouched beside"
        );
        assert_eq!(
            base.get("lsp.rust.args"),
            Some(&Setting::List(vec![])),
            "a list replaces"
        );
        assert!(base.get("lsp.rust.roots").is_some());
        assert!(base.get("lsp.go.cmd").is_some(), "a sibling table stays");
        // A scalar over a table wipes it.
        base.merge(tbl(&[("lsp", Setting::Bool(false))]));
        assert_eq!(base.get("lsp"), Some(&Setting::Bool(false)));
    }

    #[test]
    fn layers_override_in_order_and_swap_independently() {
        let mut s = Settings::new();
        assert_eq!(s.int("tabstop"), Some(4));
        assert_eq!(s.source_of("tabstop"), Some((Layer::Default, "default")));
        assert_eq!(s.origin("tabstop").as_deref(), Some("default"));
        s.replace(
            Layer::User,
            vec![(
                "~/settings.lua".into(),
                tbl(&[("tabstop", Setting::Int(2))]),
            )],
        );
        assert_eq!(s.int("tabstop"), Some(2));
        s.replace(
            Layer::Project,
            vec![
                (
                    "/repo/.kawoosh".into(),
                    tbl(&[
                        ("tabstop", Setting::Int(8)),
                        ("compile.default", Setting::Str("make".into())),
                    ]),
                ),
                (
                    "/repo/sub/.kawoosh".into(),
                    tbl(&[("tabstop", Setting::Int(3))]),
                ),
            ],
        );
        assert_eq!(s.int("tabstop"), Some(3), "the innermost project file wins");
        assert_eq!(
            s.str("compile.default"),
            Some("make"),
            "the outer one's other keys stay"
        );
        assert_eq!(
            s.source_of("tabstop"),
            Some((Layer::Project, "/repo/sub/.kawoosh"))
        );
        assert_eq!(
            s.origin("tabstop").as_deref(),
            Some("project: /repo/sub/.kawoosh")
        );
        s.set(Layer::Session, "tabstop", Setting::Int(1));
        assert_eq!(s.int("tabstop"), Some(1));
        assert_eq!(s.origin("tabstop").as_deref(), Some("session"));
        // Leaving the project keeps what was typed.
        s.replace(Layer::Project, vec![]);
        assert_eq!(s.int("tabstop"), Some(1));
        assert_eq!(s.str("compile.default"), None);
        s.unset(Layer::Session, "tabstop");
        assert_eq!(
            s.int("tabstop"),
            Some(2),
            "and unsetting shows the user's again"
        );
        assert!(
            s.sources(Layer::Session).is_empty(),
            "an emptied session source is gone"
        );
        assert_eq!(Setting::table().paths(), Vec::<String>::new());
        s.set(Layer::User, "x", Setting::Int(1));
        assert_eq!(
            s.sources(Layer::User).len(),
            2,
            "what init.lua sets is beside the file"
        );
        s.retain_sources(Layer::User, |n| n != "user");
        assert_eq!(s.get("x"), None);
        assert_eq!(s.int("tabstop"), Some(2));
        assert_eq!(
            s.effective().paths(),
            [
                "buffers.scope",
                "clipboard.system",
                "editor.bell",
                "editor.selection_radius",
                "env.shell",
                "expandtab",
                "font.chrome_size",
                "font.family",
                "font.features",
                "font.line_height",
                "font.size",
                "launcher.start",
                "layout.column_width",
                "layout.default",
                "layout.dock",
                "layout.gap",
                "layout.new_pane",
                "layout.new_tab",
                "layout.scroll.center",
                "leader",
                "lsp.inlay_hints",
                "markdown.heading",
                "markdown.image_max_mb",
                "markdown.render",
                "memory.idle_secs",
                "memory.keep_days",
                "memory.max_mb",
                "memory.text.keep_days",
                "memory.text.max_mb",
                "pairs.enabled",
                "picker.preview",
                "picker.share",
                "picker.split",
                "picker.wrap",
                "relativenumber",
                "scrolloff",
                "secrets.forget_secs",
                "secrets.masks.env.files",
                "secrets.masks.env.pattern",
                "secrets.masks.key.files",
                "secrets.masks.key.pattern",
                "secrets.masks.pem.from",
                "secrets.masks.pem.to",
                "secrets.masks.secret.language",
                "secrets.masks.secret.pattern",
                "secrets.masks.vault.files",
                "secrets.masks.vault.pattern",
                "secrets.masks.vault_pass.files",
                "secrets.masks.vault_pass.pattern",
                "secrets.private_temp",
                "secrets.reveal_secs",
                "secrets.scan_max_kb",
                "tabstop",
                "terminal.bell",
                "terminal.scrollback",
                "terminal.shell",
                "theme.appearance",
                "theme.dark",
                "theme.light",
                "theme.name",
                "whichkey"
            ]
        );
    }

    #[test]
    fn parse_like_follows_the_type_that_is_there() {
        assert_eq!(
            Setting::parse_like("2", Some(&Setting::Int(4))),
            Setting::Int(2)
        );
        assert_eq!(
            Setting::parse_like("2", Some(&Setting::Str("x".into()))),
            Setting::Str("2".into())
        );
        assert_eq!(Setting::parse_like("true", None), Setting::Bool(true));
        assert_eq!(Setting::parse_like("1.5", None), Setting::Float(1.5));
        assert_eq!(
            Setting::parse_like("abc", Some(&Setting::Int(1))),
            Setting::Str("abc".into())
        );
        assert_eq!(
            Setting::parse_like("cargo test", None),
            Setting::Str("cargo test".into())
        );
        // Lenient reads: a string that spells a number reads as one.
        assert_eq!(Setting::Str("7".into()).as_int(), Some(7));
        assert_eq!(Setting::Float(7.0).as_int(), Some(7));
        assert_eq!(Setting::Float(7.5).as_int(), None);
        assert_eq!(Setting::Str("true".into()).as_bool(), Some(true));
    }

    #[test]
    fn displays_as_lua() {
        let t = tbl(&[
            ("a", Setting::Int(1)),
            ("b.c", Setting::Str("x\"y".into())),
            (
                "l",
                Setting::List(vec![Setting::Bool(true), Setting::Float(1.5)]),
            ),
            ("e", Setting::table()),
        ]);
        assert_eq!(
            t.to_string(),
            r#"{ a = 1, b = { c = "x\"y" }, e = {}, l = { true, 1.5 } }"#
        );
    }
}
