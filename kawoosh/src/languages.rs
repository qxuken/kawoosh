//! Languages the shell adds (kui.md D13): `kawoosh.language` from Lua
//! names one — its files, and where its grammar is, or nothing about
//! that — and the shell finds the grammar by convention under the
//! config directory, loads it here so a failure is a toast, and tells
//! its own registry and the ts thread's.

use std::path::{Path, PathBuf};

use kawoosh_doc::BufferId;
use kawoosh_editor::Setting;
use kawoosh_languages::{FALLBACK, Grammar, LanguageDef, Library, Locate, Source};
use kawoosh_systems::store::MomentKey;

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
        let mut unread = Vec::new();
        for (id, b) in self.ed.buffers.iter() {
            if *b.language == *name {
                resend.push(id);
            } else if &*b.language == FALLBACK
                && let Some(path) = &b.path
            {
                unread.push((id, path.clone(), first_line(b)));
            }
        }
        for (id, path, first) in unread {
            let lang = self.language_of(&path, &first);
            if lang != FALLBACK {
                self.ed.buffers[id].language = lang.into();
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
    /// grammar; a history row keeps the language with the text, and
    /// the memory a file's (`remember_syntax`). Bare, it says which
    /// language the buffer is.
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
        self.relanguage(id, &language);
        let also = self.remember_syntax(id, &language);
        self.ed.message = match also {
            0 => format!("syntax {language}"),
            n => format!(
                "syntax {language}, and {n} more open file{} of its kind",
                if n == 1 { "" } else { "s" }
            ),
        };
    }

    /// Buffer `id` read as `language` from now on: the old runs go and
    /// it is parsed whole again.
    fn relanguage(&mut self, id: BufferId, language: &str) {
        let b = &mut self.ed.buffers[id];
        b.language = language.into();
        b.clear_layer(kawoosh_systems::ts::SYNTAX_LAYER);
        self.ts_sent.remove(&id);
        self.inspector.trees.remove(&id);
        if let Some(rt) = &self.scripting.rt {
            rt.set_tree(id, None);
        }
    }

    /// The language of the file at `path` whose first line is
    /// `first_line`, as the memory has it (memory.md, "Syntax"): what
    /// `:syntax` said for this file, else what the languages detect,
    /// else what `:syntax` said for files of its kind (`*.EXT`, or its
    /// name when it has no extension), else [`FALLBACK`]. A remembered
    /// language no longer known is passed over.
    pub(crate) fn language_of(&self, path: &Path, first_line: &str) -> String {
        let known = |l: String| (l == FALLBACK || self.languages.get(&l).is_some()).then_some(l);
        let file = self.resolve(path).display().to_string();
        if let Some(l) = self.remembered_syntax(&file).and_then(known) {
            return l;
        }
        let detected = self.languages.detect(path, first_line);
        if detected != FALLBACK {
            return detected.to_string();
        }
        syntax_kind_of(path)
            .and_then(|k| self.remembered_syntax(&k))
            .and_then(known)
            .unwrap_or_else(|| FALLBACK.to_string())
    }

    /// The language a `syntax` row says for `subject`: what is pending
    /// of it first, else the store's.
    fn remembered_syntax(&self, subject: &str) -> Option<String> {
        let key = MomentKey::new(SYNTAX_KIND, subject, "");
        let meta = self
            .moments
            .pending_meta(&key)
            .or_else(|| Some(self.store.as_ref()?.moment(&key)?.meta))?;
        let meta: serde_json::Value = serde_json::from_str(&meta).ok()?;
        Some(meta.get("language")?.as_str()?.to_string())
    }

    /// `:syntax` gave buffer `id` `language`: the memory keeps it. A
    /// file the languages know is remembered by its path — and
    /// forgotten when it is given back what they say. A file they do
    /// not know is remembered by its kind, so every `*.EXT` reads as
    /// `language` from then on, and the open ones of that kind still
    /// unread are read so now: their count is returned. A scratch is
    /// its history's, a private file nobody's.
    fn remember_syntax(&mut self, id: BufferId, language: &str) -> usize {
        let b = &self.ed.buffers[id];
        let Some(path) = b.path.clone() else {
            return 0;
        };
        if b.private || self.ed.is_field_buffer(id) {
            return 0;
        }
        let detected = self.languages.detect(&path, &first_line(b)).to_string();
        let file = MomentKey::new(SYNTAX_KIND, &self.resolve(&path).display().to_string(), "");
        let kind = syntax_kind_of(&path).filter(|_| detected == FALLBACK);
        let mut also = 0;
        match kind {
            Some(kind) => {
                let key = MomentKey::new(SYNTAX_KIND, &kind, "");
                self.forget_syntax(&file);
                if language == FALLBACK {
                    self.forget_syntax(&key);
                } else {
                    self.moments
                        .set_meta(key, serde_json::json!({ "language": language }).to_string());
                    let unread: Vec<BufferId> = self
                        .ed
                        .buffers
                        .iter()
                        .filter(|(o, b)| {
                            *o != id
                                && &*b.language == FALLBACK
                                && b.path.as_deref().and_then(syntax_kind_of).as_ref()
                                    == Some(&kind)
                                && self.languages.detect(
                                    b.path.as_deref().unwrap_or(Path::new("")),
                                    &first_line(b),
                                ) == FALLBACK
                        })
                        .map(|(o, _)| o)
                        .collect();
                    also = unread.len();
                    for o in unread {
                        self.relanguage(o, language);
                    }
                }
            }
            None if language == detected => self.forget_syntax(&file),
            None => self.moments.set_meta(
                file,
                serde_json::json!({ "language": language }).to_string(),
            ),
        }
        self.flush_moments();
        also
    }

    /// A `syntax` row gone, if there was one.
    fn forget_syntax(&mut self, key: &MomentKey) {
        let known = self.moments.pending_meta(key).is_some()
            || self.store.as_ref().is_some_and(|s| s.moment(key).is_some());
        if known {
            let _ = self.forget_moment(key);
        }
    }
}

/// The memory's kind for a language `:syntax` gave a file (memory.md,
/// "Syntax"): its subject a file's path, or a kind of file the
/// languages do not know ([`syntax_kind_of`]); its `meta` the
/// `language`; under no workspace, since a file reads the same from any
/// root; held, as a mark is — the user's word, never evicted or aged —
/// until its file is gone ([`syntax_gone`]).
pub const SYNTAX_KIND: &str = "syntax";

/// Whether a `syntax` row's file is gone: a local path that is no
/// more. A kind is never gone, nor a host's file — whether it is there
/// is not asked over the wire for a sweep.
pub fn syntax_gone(key: &MomentKey) -> bool {
    let path = Path::new(&key.subject);
    path.is_absolute() && kawoosh_systems::fs::domain_of(path).is_none() && !path.exists()
}

/// The kind of file `path` is, as a `syntax` row names it: `*.EXT`, the
/// extension in lower case as the languages match it, or the file's
/// name when it has none.
pub fn syntax_kind_of(path: &Path) -> Option<String> {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => Some(format!("*.{}", ext.to_ascii_lowercase())),
        _ => path.file_name()?.to_str().map(str::to_string),
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("syntax")
            .alias(&["setf", "setfiletype", "filetype", "ft"])
            .args(Args::new(&[ArgKind::Language]))
            .doc("read the buffer as language NAME (`text` for none), remembered for the file or its unknown kind; bare, say which it is"),
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
