//! Languages the shell adds (kui.md D13): `kawoosh.language` from Lua
//! names one — its files, and where its grammar is, or nothing about
//! that — and the shell finds the grammar by convention under the
//! config directory, loads it here so a failure is a toast, and tells
//! its own registry and the ts thread's.

use std::path::PathBuf;

use kawoosh_editor::Setting;
use kawoosh_languages::{FALLBACK, Grammar, LanguageDef, Library, Locate, Source};

use kawoosh_editor::{ArgKind, Args, Spec};

use crate::Kawoosh;
use crate::app::first_line;
use crate::commands::{ShellCommand, cmd};
use crate::notify::{Level, Note};

impl Kawoosh {
    /// `kawoosh.language(name, t)` arrived: what `t` said about the
    /// grammar becomes a [`Library`] by convention — nothing said and
    /// nothing under the config directory is a language of files alone
    /// — and the language is added. A path that leads nowhere is a
    /// warning, and the language is added without its grammar. One a
    /// bundled plugin registers (`man`) is every launch's, said only at
    /// debug, as the manifest's are.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn language_from_lua(
        &mut self,
        name: String,
        aliases: Vec<String>,
        extensions: Vec<String>,
        filenames: Vec<String>,
        shebangs: Vec<String>,
        path: Option<String>,
        symbol: Option<String>,
        highlights: Option<String>,
        injections: Option<String>,
        comment: Option<String>,
        comment_block: Option<Vec<String>>,
        indent_style: Option<String>,
        indent_size: Option<i64>,
    ) {
        let said = Locate {
            path: path.map(PathBuf::from),
            symbol,
            highlights: highlights.map(PathBuf::from),
            injections: injections.map(PathBuf::from),
        };
        let home = self.config.dir.clone();
        let grammar = match Library::find(&name, &said, home.as_deref()) {
            Ok(lib) => lib.map(|l| Source::Library(Box::new(l))),
            Err(e) => {
                self.notify_with(Note::new(Level::Warn, e).source("language"));
                None
            }
        };
        let def = LanguageDef {
            name,
            aliases,
            extensions,
            filenames,
            shebangs,
            grammar,
            comment: comment.filter(|t| !t.trim().is_empty()),
            comment_block: match comment_block.as_deref() {
                Some([a, b]) if !a.trim().is_empty() && !b.trim().is_empty() => {
                    Some((a.clone(), b.clone()))
                }
                _ => None,
            },
            expandtab: expandtab_of(indent_style.as_deref()),
            tabstop: indent_size.and_then(|n| tabstop_of(u32::try_from(n).ok())),
        };
        let say = !self.scripting.bundled;
        self.put_language(def, say);
    }

    /// Adds a language: its grammar loaded here — a load that fails is
    /// a warning, and the language goes in without one — the registry
    /// told, the ts thread told with the grammar, and the buffers
    /// looked at again: one of the language is sent whole at the next
    /// frame, and a file nothing had claimed may be the language's.
    /// Every name the registry's languages go by, to the trees that
    /// answer the layer's language (`Trees::set_names`): after the
    /// registry is built, and each time it gains a language.
    pub(crate) fn sync_language_names(&mut self) {
        self.indent_trees
            .set_names(self.languages.iter().flat_map(|d| {
                std::iter::once((d.name.clone(), d.name.clone()))
                    .chain(d.aliases.iter().map(|a| (a.clone(), d.name.clone())))
            }));
    }

    pub fn add_language(&mut self, def: LanguageDef) {
        self.put_language(def, true);
    }

    /// [`Kawoosh::add_language`], said in the log or not: a language
    /// the user registered is worth a line, the dozens a manifest lists
    /// at launch are not, and an install has its own word.
    pub(crate) fn put_language(&mut self, def: LanguageDef, say: bool) {
        let grammar = def.load();
        self.put_loaded(def, grammar, say);
    }

    /// [`Kawoosh::put_language`] with its grammar's load done already —
    /// an install's, on its thread — whose failure is the warning.
    pub(crate) fn put_loaded(
        &mut self,
        mut def: LanguageDef,
        grammar: Result<Option<Grammar>, String>,
        say: bool,
    ) {
        let grammar = match grammar {
            Ok(g) => g,
            Err(e) => {
                self.notify_with(
                    Note::new(Level::Warn, format!("{}: {e}", def.name)).source("language"),
                );
                def.grammar = None;
                None
            }
        };
        let replaced = self.languages.add(def.clone()).is_some();
        self.sync_language_names();
        // Its comment tokens are the language's defaults, under a
        // user's `language.NAME.comment` as a builtin's are
        // (docs/design/comments.md Decision 3).
        if let Some(t) = &def.comment {
            self.ed.settings.set(
                kawoosh_editor::Layer::Default,
                &format!("language.{}.comment", def.name),
                Setting::Str(t.clone()),
            );
        }
        if let Some((a, b)) = &def.comment_block {
            self.ed.settings.set(
                kawoosh_editor::Layer::Default,
                &format!("language.{}.comment_block", def.name),
                Setting::List(vec![Setting::Str(a.clone()), Setting::Str(b.clone())]),
            );
        }
        // And how its files indent, likewise.
        if let Some(b) = def.expandtab {
            self.ed.settings.set(
                kawoosh_editor::Layer::Default,
                &format!("language.{}.expandtab", def.name),
                Setting::Bool(b),
            );
        }
        if let Some(n) = def.tabstop {
            self.ed.settings.set(
                kawoosh_editor::Layer::Default,
                &format!("language.{}.tabstop", def.name),
                Setting::Int(n),
            );
        }
        let said = format!(
            "language {}: {}{}",
            def.name,
            match &def.grammar {
                Some(Source::Library(l)) => format!("grammar from {}", l.path.display()),
                Some(Source::Builtin(_)) => "builtin grammar".to_string(),
                None => "no grammar".to_string(),
            },
            if replaced {
                ", replacing the earlier one"
            } else {
                ""
            }
        );
        if say {
            log::info!("{said}");
        } else {
            log::debug!("{said}");
        }
        let name = def.name.clone();
        self.ts.add_language(def, grammar);
        // Its files are what `load_all` sends its server.
        self.lsp.rules_seen = None;
        let mut resend = Vec::new();
        for (id, b) in self.ed.buffers.iter_mut() {
            if *b.language == *name {
                resend.push(id);
            } else if &*b.language == FALLBACK
                && let Some(path) = b.path.clone()
            {
                let lang = self.languages.detect(&path, &first_line(b));
                if lang != FALLBACK {
                    b.language = lang.into();
                }
            }
        }
        for id in resend {
            self.ts_sent.remove(&id);
        }
    }

    /// `:syntax NAME`: the focused buffer read as language NAME (an
    /// alias resolves to its language; `text` is none) — a scratch
    /// given its grammar, a file whose extension says the wrong thing.
    /// The old runs go and the buffer is parsed whole with the new
    /// grammar; a history row keeps the language with the text. Bare,
    /// it says which language the buffer is.
    pub(crate) fn set_syntax(&mut self, name: Option<&str>) {
        let Some(v) = self.focused_view() else {
            self.ed.message = "syntax: no buffer here".into();
            return;
        };
        let id = self.ed.views[v].buffer;
        let Some(name) = name else {
            self.ed.message = format!("syntax {}", self.ed.buffers[id].language);
            return;
        };
        let language = if name == FALLBACK {
            FALLBACK.to_string()
        } else {
            match self.languages.by_name(name) {
                Some(d) => d.name.clone(),
                None => {
                    self.ed.message = format!("syntax: no language {name}");
                    return;
                }
            }
        };
        let b = &mut self.ed.buffers[id];
        b.language = language.as_str().into();
        b.clear_layer(kawoosh_systems::ts::SYNTAX_LAYER);
        self.ts_sent.remove(&id);
        self.inspector.trees.remove(&id);
        if let Some(rt) = &self.scripting.rt {
            rt.set_tree(id, None);
        }
        self.ed.message = format!("syntax {language}");
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("syntax")
            .alias(&["setf", "setfiletype", "filetype", "ft"])
            .args(Args::new(&[ArgKind::Language]))
            .doc("read the buffer as language NAME (`text` for none); bare, say which it is"),
        |k, ctx| k.set_syntax(ctx.args.first().map(String::as_str)),
    )]
}

/// `.editorconfig`'s `indent_style` as `expandtab`: `"tab"` is false,
/// `"space"` true, any other word unsaid.
pub(crate) fn expandtab_of(style: Option<&str>) -> Option<bool> {
    match style? {
        "tab" => Some(false),
        "space" => Some(true),
        _ => None,
    }
}

/// `.editorconfig`'s `indent_size` as `tabstop`: a width, 0 unsaid.
pub(crate) fn tabstop_of(size: Option<u32>) -> Option<i64> {
    size.filter(|n| *n > 0).map(i64::from)
}
