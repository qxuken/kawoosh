//! Languages the shell adds (kui.md D13): `kawoosh.language` from Lua
//! names one — its files, and where its grammar is, or nothing about
//! that — and the shell finds the grammar by convention under the
//! config directory, loads it here so a failure is a toast, and tells
//! its own registry and the ts thread's.

use std::path::PathBuf;

use kawoosh_languages::{FALLBACK, LanguageDef, Library, Locate, Source};

use crate::Kawoosh;
use crate::app::first_line;
use crate::notify::{Level, Note};

impl Kawoosh {
    /// `kawoosh.language(name, t)` arrived: what `t` said about the
    /// grammar becomes a [`Library`] by convention — nothing said and
    /// nothing under the config directory is a language of files alone
    /// — and the language is added. A path that leads nowhere is a
    /// warning, and the language is added without its grammar.
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
    ) {
        let said = Locate {
            path: path.map(PathBuf::from),
            symbol,
            highlights: highlights.map(PathBuf::from),
            injections: injections.map(PathBuf::from),
        };
        let home = crate::settings::config_dir();
        let grammar = match Library::find(&name, &said, home.as_deref()) {
            Ok(lib) => lib.map(Source::Library),
            Err(e) => {
                self.notify_with(Note::new(Level::Warn, e).source("language"));
                None
            }
        };
        self.add_language(LanguageDef {
            name,
            aliases,
            extensions,
            filenames,
            shebangs,
            grammar,
        });
    }

    /// Adds a language: its grammar loaded here — a load that fails is
    /// a warning, and the language goes in without one — the registry
    /// told, the ts thread told with the grammar, and the buffers
    /// looked at again: one of the language is sent whole at the next
    /// frame, and a file nothing had claimed may be the language's.
    pub fn add_language(&mut self, mut def: LanguageDef) {
        let grammar = match def.load() {
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
        log::info!(
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
        let name = def.name.clone();
        self.ts.add_language(def, grammar);
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
}
