//! The command line's completion, in place like the editor's (mvp.md
//! Decision 5) and on its keys: the current candidate's rest is a
//! ghost after the caret, `<C-n>`/`<C-p>` cycle the candidates, `<Tab>`
//! or `<C-y>` takes the current one; typing narrows. The first word
//! completes to a command — the ex spellings, the shell's, the engine's
//! and Lua's — and an argument to what the command declares it takes
//! (`kawoosh_editor::ArgKind`): a path for `:e`, `:w`, `:cd`, `:vs`,
//! `:dir`; a buffer for `:b`; a tool, a view, an option, a command, a
//! font family (`:font`, the rest of the line, monospaced first);
//! `:compile` its line — the names, the picker's lines, a program's
//! arguments from its completer (`shell_complete.rs`), paths — and
//! `:!`, `:shell`, `:terminal` their program's arguments (compile.md
//! Decision 23). A path's `%` completes to its spellings, each with
//! what it names as the candidate's detail.
//! Past an option's path (`:set font.family=`, `:set font.family `)
//! its value completes: the families for `font.family` (monospaced
//! first, the OS's `.`-faces left out), a one-of's
//! words, `true` and `false`.
//! A command's subcommands complete as its first word (`:memory fo`
//! is `:memory forget`), and the words after complete as the
//! subcommand's own. Nothing is a popup: the candidates are a row in
//! the strip. What completes is the word before the caret — a caret
//! moved back into the line completes there, the text after it kept;
//! one inside a word completes nothing. The keys are the shell's commands `prompt complete`
//! (`<Tab>`, `<C-y>`) and `prompt cycle next|prev` (`<C-n>`, `<C-p>`),
//! bound `when field:cmdline` over the engine's history walk on the
//! same keys, which a search prompt keeps.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::rc::Rc;

use kawoosh_editor::commands::set_value;
use kawoosh_editor::{ArgKind, Editor, Mode, Prompt, Selection, Selections, Spec};
use kawoosh_systems::io::IoMsg;

use crate::commands::{ShellCommand, cmd};

use crate::app::Kawoosh;

/// Each directory a `:` path was completed from and what it held; kept
/// while the prompt is open.
pub(crate) type Listed = RefCell<HashMap<PathBuf, Dir>>;

/// A directory the `:` prompt completes from.
#[derive(Clone)]
pub(crate) enum Dir {
    /// A host's, its listing out on a job: a host over the wire, or one
    /// as small as OpenWrt's listing in a fork or three an entry, is not
    /// waited on at a key.
    Asked,
    /// What it held, or None when it could not be read.
    Read(Option<Rc<Vec<kawoosh_doc::fs::Entry>>>),
}

/// What the command line is completing: the token from `start` to the
/// caret at `at`, the candidates for it, `index` the current one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CmdCompletion {
    pub start: usize,
    pub at: usize,
    pub candidates: Vec<String>,
    pub index: usize,
    /// What is said of each candidate, by its place — a completer's
    /// description, the path a `%` names — empty where nothing is.
    pub details: Vec<String>,
}

impl CmdCompletion {
    pub fn current(&self) -> Option<&str> {
        self.candidates.get(self.index).map(String::as_str)
    }

    /// What is said of the current candidate, if anything.
    pub fn detail(&self) -> Option<&str> {
        self.details
            .get(self.index)
            .map(String::as_str)
            .filter(|d| !d.is_empty())
    }

    /// The current candidate past `token` — the ghost — when it extends
    /// it. An empty line is nothing to extend — the first of every
    /// command is not a suggestion — but after a command's word the
    /// first of its subcommands and arguments is (`:dir ` offers `cd`).
    pub fn ghost(&self, token: &str) -> Option<&str> {
        if token.is_empty() && self.start == 0 {
            return None;
        }
        self.current()?
            .strip_prefix(token)
            .filter(|s| !s.is_empty())
    }
}

/// Where the token being completed starts: after the last whitespace,
/// or the line's start for the command itself.
fn token_start(line: &str) -> usize {
    line.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0)
}

/// Where each word of the line starts.
fn word_starts(line: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut prev_space = true;
    for (i, c) in line.char_indices() {
        if !c.is_whitespace() && prev_space {
            out.push(i);
        }
        prev_space = c.is_whitespace();
    }
    out
}

/// Whether a word reads as a path: a directory in it, or from `.`,
/// `~` or the file (`%`).
fn looks_like_path(word: &str) -> bool {
    word.contains(['/', MAIN_SEPARATOR]) || word.starts_with(['.', '~', '%'])
}

/// The candidates for the token from `start`, and what is said of some
/// of them.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub start: usize,
    pub candidates: Vec<String>,
    pub details: HashMap<String, String>,
    /// The candidates so far, to keep each once: a directory of
    /// thousands is not searched for every entry.
    seen: HashSet<String>,
}

impl Found {
    fn new(start: usize, candidates: Vec<String>) -> Self {
        Found {
            start,
            seen: candidates.iter().cloned().collect(),
            candidates,
            details: HashMap::new(),
        }
    }

    /// A candidate, and what is said of it; once.
    fn said(&mut self, candidate: String, about: String) {
        if !self.seen.insert(candidate.clone()) {
            return;
        }
        if !about.is_empty() {
            self.details.insert(candidate.clone(), about);
        }
        self.candidates.push(candidate);
    }

    /// Candidates, each once.
    fn extend(&mut self, candidates: impl IntoIterator<Item = String>) {
        for c in candidates {
            self.said(c, String::new());
        }
    }

    /// The candidates moved `by` bytes along the line: what was found
    /// for a line after a prefix put on it, or taken off it.
    fn shifted(mut self, by: isize) -> Self {
        self.start = self.start.saturating_add_signed(by);
        self
    }
}

impl Kawoosh {
    /// The token the command line is on and its candidates: command
    /// names for the first word, what the command takes after it.
    pub(crate) fn cmd_candidates(&mut self, line: &str) -> Found {
        // The selections' range visual mode's `:` puts: the command
        // after it is completed as one on a line of its own.
        const SELECTIONS: &str = "'<,'>";
        if let Some(rest) = line.strip_prefix(SELECTIONS) {
            return self.cmd_candidates(rest).shifted(SELECTIONS.len() as isize);
        }
        // `:!CMD` is `:shell CMD` (the engine reads it so): its words
        // complete as that line's.
        if let Some(rest) = line.strip_prefix('!') {
            const SHELL: &str = "shell ";
            if rest.trim().is_empty() {
                return Found::new(line.len(), Vec::new());
            }
            return self
                .cmd_candidates(&format!("{SHELL}{rest}"))
                .shifted(1 - SHELL.len() as isize);
        }
        let start = token_start(line);
        let token = &line[start..];
        if start == 0 {
            return Found::new(0, self.command_name_candidates(token));
        }
        let head = line[..start].trim();
        let mut words = head.split_whitespace();
        let name = words.next().unwrap_or("");
        let words: Vec<String> = words.map(str::to_string).collect();
        // The command the words so far name — its alias resolved, its
        // subcommands consumed — and which of its arguments the token is.
        let inv = self.ed.commands.resolve(name, &words);
        let index = inv.args.len();
        let args_at = self.ed.command_args(&inv.name);
        let first = word_starts(line)
            .get(1 + words.len().saturating_sub(index))
            .copied()
            .unwrap_or(start);
        match args_at.and_then(|a| a.kind_at(0)) {
            // An option's value, once its path is whole: the rest of the
            // line, spaces and all, from where `:set` reads it.
            Some(ArgKind::Option) => {
                if let Some((path, value)) = set_value(&line[first..]) {
                    let at = line.len() - value.len();
                    return Found::new(at, self.setting_value_candidates(path, value));
                }
            }
            // A family is the rest of the line too (`:font Fira Code`).
            Some(ArgKind::Font) => {
                let names = self.look.fonts.borrow().names();
                return Found::new(first, narrow(names, &line[first..]));
            }
            _ => {}
        }
        // The first word after a command with subcommands is one of
        // them, or its own first argument: both are offered.
        let mut out: Vec<String> = Vec::new();
        if index == 0 {
            out.extend(
                self.ed
                    .commands
                    .subcommands(&inv.name)
                    .into_iter()
                    .filter(|w| w.starts_with(token))
                    .map(str::to_string),
            );
        }
        // `:compile`: a line of its own (compile.md Decisions 7 and 23).
        if inv.name == "compile" {
            return self.compile_candidates(line, first, index, &inv.args);
        }
        // A shell line's program's arguments: what its completer says,
        // then paths (Decision 23) — `:!`, `:shell`, `:terminal [here]`.
        if matches!(inv.name.as_str(), "shell" | "terminal" | "terminal here") && index > 0 {
            let mut found = Found::new(start, out);
            let dir = self.cwd.clone();
            for (c, about) in self.shell_arg_candidates(&line[first..start], token, &dir) {
                found.said(c, about);
            }
            self.path_found(&mut found, token, &dir, self.percent_file().as_deref());
            return found;
        }
        let kind = args_at.and_then(|a| a.kind_at(index));
        if kind == Some(ArgKind::Path) {
            let mut found = Found::new(start, out);
            self.path_found(&mut found, token, &self.cwd, self.percent_file().as_deref());
            return found;
        }
        let args: Vec<String> = match kind {
            None | Some(ArgKind::Text) | Some(ArgKind::Font) | Some(ArgKind::Path) => Vec::new(),
            Some(ArgKind::Host) => self.host_candidates(token),
            Some(ArgKind::Command) => self.command_name_candidates(token),
            Some(ArgKind::Buffer) => {
                let names: Vec<&str> = self
                    .ed
                    .listed_buffers()
                    .into_iter()
                    .map(|id| self.ed.buffers[id].name.as_str())
                    .collect();
                let mut v: Vec<String> = names
                    .iter()
                    .filter(|n| n.starts_with(token))
                    .map(|n| n.to_string())
                    .collect();
                v.extend(
                    names
                        .iter()
                        .filter(|n| !n.starts_with(token) && n.contains(token))
                        .map(|n| n.to_string()),
                );
                v
            }
            Some(ArgKind::Tool) => {
                let mut v: Vec<String> = self
                    .scripting
                    .tools
                    .keys()
                    .filter(|n| n.starts_with(token))
                    .cloned()
                    .collect();
                v.sort();
                v
            }
            Some(ArgKind::Language) => {
                let mut v: Vec<String> = self
                    .languages
                    .iter()
                    .map(|d| d.name.clone())
                    .filter(|n| n.starts_with(token))
                    .collect();
                v.sort();
                v.dedup();
                v
            }
            Some(ArgKind::View) => {
                let mut v: Vec<String> = self
                    .scripting
                    .rt
                    .as_ref()
                    .map(|rt| rt.view_names())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|n| n.starts_with(token))
                    .collect();
                v.sort();
                v
            }
            Some(ArgKind::Option) => {
                // Every leaf of the effective tree, by dotted path; a
                // sign before it and `=` after are not part of the name.
                let name = token.strip_prefix(['+', '-']).unwrap_or(token);
                let name = name.split('=').next().unwrap_or(name);
                let prefix = &token[..token.len() - name.len()];
                self.ed
                    .settings
                    .effective()
                    .paths()
                    .into_iter()
                    .filter(|p| p.starts_with(name))
                    .map(|p| format!("{prefix}{p}"))
                    .collect()
            }
        };
        out.extend(args);
        out.dedup();
        Found::new(start, out)
    }

    /// `:compile`'s candidates (compile.md Decision 23). Its first word:
    /// the names of `compile.commands`, the lines the compile picker
    /// offers in its order, `commands:NAME` once a letter is typed,
    /// paths for a word that reads as one. After it: a nushell
    /// command's parameter (Decision 16), what the line's program's
    /// completer says unless the line is a name's, paths from where the
    /// line runs (Decision 7) — after the picker's lines that go on
    /// from what is typed, when there are any, each candidate then the
    /// line whole.
    fn compile_candidates(
        &mut self,
        line: &str,
        first: usize,
        index: usize,
        args: &[String],
    ) -> Found {
        let start = token_start(line);
        let token = &line[start..];
        let lines = match &self.cmd_compile_lines {
            Some(l) => l.clone(),
            None => {
                let l = Rc::new(self.compile_lines());
                self.cmd_compile_lines = Some(l.clone());
                l
            }
        };
        let mut names: Vec<String> = self
            .compile_commands()
            .into_iter()
            .map(|n| n.name)
            .collect();
        names.sort();
        let mut found = Found::new(start, Vec::new());
        if index == 0 {
            found.extend(names.iter().filter(|n| n.starts_with(token)).cloned());
            found.extend(narrow(lines.to_vec(), token));
            if !token.is_empty() {
                found.extend(
                    names
                        .iter()
                        .map(|n| format!("commands:{n}"))
                        .filter(|n| n.starts_with(token)),
                );
            }
            if looks_like_path(token) {
                let dir = self.compile_dir_of("");
                self.path_found(&mut found, token, &dir, self.compile_file().as_deref());
            }
            return found;
        }
        found.extend(self.compile_arg_candidates(args, token));
        let dir = self.compile_dir_of(&args.join(" "));
        let word = args.first().map(String::as_str).unwrap_or_default();
        if !word.starts_with("commands:") && !names.iter().any(|n| n == word) {
            for (c, about) in self.shell_arg_candidates(&line[first..start], token, &dir) {
                found.said(c, about);
            }
        }
        self.path_found(&mut found, token, &dir, self.compile_file().as_deref());
        // A picker's line that goes on from what is typed: the line
        // whole is the candidate, so the rest are written whole too.
        let typed = &line[first..];
        let whole: Vec<&String> = lines
            .iter()
            .filter(|l| l.len() > typed.len() && l.starts_with(typed))
            .collect();
        if whole.is_empty() {
            return found;
        }
        let before = &line[first..start];
        let mut wide = Found::new(first, whole.into_iter().cloned().collect());
        for c in found.candidates {
            let about = found.details.remove(&c).unwrap_or_default();
            wide.said(format!("{before}{c}"), about);
        }
        wide
    }

    /// Paths for `token` read from `dir` into `found`, and `file`'s own
    /// spellings when it starts `%` (`%`, `%:h`, `%:t`), each with what
    /// it names; `%:h/` lists `file`'s directory.
    fn path_found(&self, found: &mut Found, token: &str, dir: &Path, file: Option<&Path>) {
        if token.starts_with('%') && !token.contains(['/', MAIN_SEPARATOR]) {
            for form in ["%", "%:h", "%:t"]
                .into_iter()
                .filter(|f| f.starts_with(token))
            {
                let about = file
                    .map(|f| {
                        let p = Editor::percent_of(f, form);
                        kawoosh_systems::fs::abbreviate_home(Path::new(&p))
                    })
                    .unwrap_or_else(|| "no file for %".into());
                found.said(form.to_string(), about);
            }
        }
        found.extend(self.path_candidates_in(token, dir, file));
    }

    /// The file `%` names at the prompt: the caret's.
    fn percent_file(&self) -> Option<PathBuf> {
        self.ed.percent_path(self.focused_view()?).ok()
    }

    /// What a setting's value may be, narrowed by `typed`: the value it
    /// has first, then its words — the families kui can see for
    /// `font.family`, the families for `theme.name` and a base's
    /// variants for `theme.dark` and `theme.light`, a one-of's words,
    /// `true` and `false` for a flag. A candidate that starts with what
    /// is typed comes before one that only contains it, case aside.
    fn setting_value_candidates(&self, path: &str, typed: &str) -> Vec<String> {
        use kawoosh_editor::{Setting, SettingKind};
        let settings = &self.ed.settings;
        let mut all: Vec<String> = Vec::new();
        match settings.get(path) {
            Some(Setting::Str(s)) if !s.is_empty() => all.push(s.clone()),
            Some(v @ (Setting::Bool(_) | Setting::Int(_) | Setting::Float(_))) => {
                all.push(v.to_string())
            }
            _ => {}
        }
        match (path, settings.kind(path)) {
            ("font.family", _) => all.extend(self.look.fonts.borrow().names()),
            ("font.prose", _) => {
                all.extend(["sans", "serif"].map(String::from));
                all.extend(self.look.fonts.borrow().names());
            }
            ("theme.name", _) => {
                all.push("system".into());
                all.extend(crate::themes::FAMILIES.iter().map(|f| f.name.to_string()));
            }
            ("theme.dark" | "theme.light", _) => {
                let dark = path == "theme.dark";
                all.push("system".into());
                all.extend(
                    crate::themes::variants()
                        .iter()
                        .filter(|v| v.dark() == dark)
                        .map(|v| v.name.to_string()),
                );
            }
            ("cmdline.complete_with", _) => {
                all.extend(["carapace".into(), "false".into()]);
            }
            (_, Some(SettingKind::OneOf(words))) => all.extend(words),
            (_, Some(SettingKind::Bool)) => all.extend(["true".into(), "false".into()]),
            _ => {}
        }
        let mut seen = HashSet::new();
        all.retain(|v| seen.insert(v.clone()));
        narrow(all, typed)
    }
}

/// The candidates for `typed`: the ones it starts, then the ones that
/// only contain it, case aside.
fn narrow(all: Vec<String>, typed: &str) -> Vec<String> {
    let lower = typed.to_lowercase();
    let (mut out, rest): (Vec<String>, Vec<String>) =
        all.into_iter().partition(|v| v.starts_with(typed));
    out.extend(
        rest.into_iter()
            .filter(|v| v.to_lowercase().contains(&lower)),
    );
    out
}

impl Kawoosh {
    /// Every name the engine knows a command by, the ones meant for
    /// the command line first: the ex spellings, the commands that have
    /// one (`:vsplit`, `:tab` for `:tabnew`) and the ones no key runs
    /// (`:view`); then the keymap's — `visual`, `move`, typed rarely.
    /// Each group sorted, a name once.
    fn command_name_candidates(&mut self, token: &str) -> Vec<String> {
        self.refresh_bound_names();
        let aliased: HashSet<&str> = self
            .ed
            .commands
            .specs()
            .into_iter()
            .filter(|s| !s.aliases.is_empty())
            .map(|s| s.name.split(' ').next().unwrap_or(&s.name))
            .collect();
        let bound = &self.bound_names.1;
        let mut out: Vec<String> = Vec::new();
        let mut push = |names: Vec<&str>| {
            let mut names: Vec<&str> = names.into_iter().filter(|n| n.starts_with(token)).collect();
            names.sort_unstable();
            for n in names {
                if !out.iter().any(|o| o == n) {
                    out.push(n.to_string());
                }
            }
        };
        let (keyed, typed): (Vec<&str>, Vec<&str>) = self
            .ed
            .command_names()
            .into_iter()
            .partition(|n| bound.contains(*n) && !aliased.contains(n));
        let mut first = self.ed.commands.alias_names();
        first.extend(typed);
        push(first);
        push(keyed);
        out
    }

    /// The first word of every command a key runs, read off the keymap
    /// once per change of it.
    fn refresh_bound_names(&mut self) {
        let version = self.ed.keymap.version();
        if self.bound_names.0 != version || self.bound_names.1.is_empty() {
            let mut set = HashSet::new();
            for mode in [
                Mode::Normal,
                Mode::Visual,
                Mode::Insert,
                Mode::OperatorPending,
            ] {
                for (_, b) in self.ed.keymap.bindings(mode) {
                    let name = self.ed.commands.resolve(&b.command, &b.args).name;
                    set.insert(name.split(' ').next().unwrap_or(&name).to_string());
                }
            }
            self.bound_names = (version, set);
        }
    }

    /// The entries of the directory the token names, with the token's
    /// own directory part kept as typed (`~/pro` completes to
    /// `~/projects/`, not to the home's absolute path). A hidden entry
    /// is offered only to a token that starts with `.`. A relative token
    /// is read from `base`; one under `%:h/` from the file's directory.
    fn path_candidates_in(&self, token: &str, base: &Path, file: Option<&Path>) -> Vec<String> {
        // On a host `/` is the one separator — a `\` is a name's there —
        // and what a directory is completed with.
        use kawoosh_systems::fs::domain_of;
        let host = domain_of(Path::new(token)).is_some() || domain_of(base).is_some();
        let sep = if host { '/' } else { MAIN_SEPARATOR };
        let cut = token.rfind(['/', sep]).map(|i| i + 1).unwrap_or(0);
        let (dir_text, prefix) = token.split_at(cut);
        let dir = if dir_text.is_empty() {
            base.to_path_buf()
        } else if dir_text.starts_with('%') {
            // `%:h/` is the file's directory (`Editor::percent_of`).
            let Some(file) = file else {
                return Vec::new();
            };
            kawoosh_doc::paths::expand(Path::new(&Editor::percent_of(file, dir_text)), base)
        } else {
            kawoosh_doc::paths::expand(Path::new(dir_text), base)
        };
        // Listed once while the prompt is open, not at every key: a
        // path typed through `$TMPDIR` or `node_modules` stats each of
        // thousands of entries, on a host over the wire. A host's on a
        // job, the candidates there when it lands (`prompt_listed`).
        let listed = self
            .cmd_dirs
            .borrow_mut()
            .entry(dir.clone())
            .or_insert_with(|| {
                if !host || self.jobs_inline {
                    return Dir::Read(kawoosh_systems::fs::list(&dir).ok().map(Rc::new));
                }
                let at = dir.clone();
                self.io.run("prompt list", move || IoMsg::PromptListed {
                    entries: kawoosh_systems::fs::list(&at).ok(),
                    dir: at,
                });
                Dir::Asked
            })
            .clone();
        let Dir::Read(Some(entries)) = listed else {
            return Vec::new();
        };
        entries
            .iter()
            .filter(|e| e.name.starts_with(prefix))
            .filter(|e| prefix.starts_with('.') || !e.name.starts_with('.'))
            .map(|e| {
                let mut s = format!("{dir_text}{}", e.name);
                if e.is_dir {
                    s.push(sep);
                }
                s
            })
            .collect()
    }

    /// Where on `line` the completion reads up to: the caret in insert
    /// mode, the line's end otherwise (a block caret is on a letter, and
    /// no key completes there). None when the caret is inside a word —
    /// a candidate would be put in the middle of it.
    fn cmdline_caret(&self, line: &str) -> Option<usize> {
        let v = self.ed.prompt_view()?;
        if self.ed.views[v].mode != Mode::Insert {
            return Some(line.len());
        }
        let at = self.ed.prompt_caret()?.min(line.len());
        let at = (0..=at).rev().find(|&i| line.is_char_boundary(i))?;
        match line[at..].chars().next() {
            Some(c) if !c.is_whitespace() => None,
            _ => Some(at),
        }
    }

    /// Whether the command line is at the `:` prompt.
    fn at_command_prompt(&self) -> bool {
        self.ed.prompt_kind() == Some(Prompt::Command)
    }

    /// The ghost after the caret, if the current candidate extends what
    /// is typed.
    pub fn cmdline_ghost(&self) -> Option<String> {
        if !self.at_command_prompt() {
            return None;
        }
        let c = self.cmd_completion.as_ref()?;
        let line = self.ed.prompt_text()?;
        let v = self.ed.prompt_view()?;
        // Drawn at the caret, so only in insert mode and only while the
        // caret is where the candidates were read (a click moves it
        // without a key).
        if self.ed.views[v].mode != Mode::Insert || self.ed.prompt_caret()? != c.at {
            return None;
        }
        let token = line.get(c.start..c.at)?;
        c.ghost(token).map(str::to_string)
    }

    /// `<Tab>` / `<C-y>`: the token before the caret replaced with the
    /// current candidate, the text after it kept, the caret after the
    /// candidate; then completed on from it (a directory's entries, a
    /// command's longer spellings). Nothing to take is nothing done — a
    /// `<Tab>` is never a character on the command line.
    fn take_candidate(&mut self) {
        // A caret moved since the candidates were read (a click) reads
        // them again where it is.
        if self.cmd_completion.as_ref().map(|c| c.at) != self.ed.prompt_caret() {
            self.cmdline_refresh();
        }
        let Some(c) = &self.cmd_completion else {
            return;
        };
        let Some(cand) = c.current() else { return };
        let Some(line) = self.ed.prompt_text() else {
            return;
        };
        let at = c.at.min(line.len());
        let start = c.start.min(at);
        let caret = start + cand.len();
        let line = format!("{}{cand}{}", &line[..start], &line[at..]);
        self.ed.set_prompt_text(&line);
        if let Some(v) = self.ed.prompt_view() {
            self.ed.views[v].sels = Selections::single(Selection::point(caret));
        }
        self.cmdline_refresh();
    }

    /// `<C-n>` / `<C-p>`: the ghost moves through the candidates; the
    /// line stays as typed.
    fn cycle_candidate(&mut self, forward: bool) {
        let Some(c) = self.cmd_completion.as_mut() else {
            return;
        };
        if c.candidates.is_empty() {
            return;
        }
        let n = c.candidates.len();
        c.index = if forward {
            (c.index + 1) % n
        } else {
            (c.index + n - 1) % n
        };
    }

    /// After a key at the `:` prompt: the candidates for the line as it
    /// now reads, the first current — unless they are the ones already
    /// up, when a key that left the line alone leaves the cycling alone
    /// too; off the prompt, none.
    pub(crate) fn cmdline_refresh(&mut self) {
        if !self.at_command_prompt() {
            self.cmd_completion = None;
            self.cmd_dirs.get_mut().clear();
            self.cmd_compile_lines = None;
            self.line_complete.clear();
            self.nu_asks_dropped();
            return;
        }
        let line = self.ed.prompt_text().unwrap_or_default();
        let Some(at) = self.cmdline_caret(&line) else {
            self.cmd_completion = None;
            return;
        };
        let Found {
            start,
            candidates,
            mut details,
            ..
        } = self.cmd_candidates(&line[..at]);
        // The same token as before, cycled off its first: the candidate
        // current stays current where it still is — a completer's answer
        // landing mid-cycle.
        let index = match &self.cmd_completion {
            Some(c) if c.start == start && c.at == at && c.index > 0 => c
                .current()
                .and_then(|cur| candidates.iter().position(|x| x == cur))
                .unwrap_or(0),
            _ => 0,
        };
        let details = if details.is_empty() {
            Vec::new()
        } else {
            candidates
                .iter()
                .map(|c| details.remove(c).unwrap_or_default())
                .collect()
        };
        self.cmd_completion = Some(CmdCompletion {
            start,
            at,
            candidates,
            index,
            details,
        });
    }

    /// A host's directory listed for the prompt: kept, and the line
    /// completed again with it — unless the prompt it was asked for has
    /// closed since, which cleared what it asked.
    pub(crate) fn prompt_listed(
        &mut self,
        dir: PathBuf,
        entries: Option<Vec<kawoosh_doc::fs::Entry>>,
    ) {
        match self.cmd_dirs.get_mut().get_mut(&dir) {
            Some(d @ Dir::Asked) => *d = Dir::Read(entries.map(Rc::new)),
            _ => return,
        }
        self.cmdline_refresh();
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("prompt complete")
                .when(&["field:cmdline"])
                .doc("take the completion's current candidate and complete on from it"),
            |k, _| k.take_candidate(),
        ),
        cmd(
            Spec::new("prompt cycle next")
                .when(&["field:cmdline"])
                .doc("the next completion candidate; the line stays as typed"),
            |k, _| k.cycle_candidate(true),
        ),
        cmd(
            Spec::new("prompt cycle prev")
                .when(&["field:cmdline"])
                .doc("the previous completion candidate"),
            |k, _| k.cycle_candidate(false),
        ),
    ]
}

/// The completion's keys at the `:` prompt, over the engine's on the
/// same keys.
pub(crate) fn bind(km: &mut kawoosh_editor::Keymap) {
    let at = "field:cmdline";
    km.bind_local(at, Mode::Insert, "<Tab>", "prompt complete", &[]);
    km.bind_local(at, Mode::Insert, "<C-y>", "prompt complete", &[]);
    km.bind_local(at, Mode::Insert, "<C-n>", "prompt cycle next", &[]);
    km.bind_local(at, Mode::Insert, "<C-p>", "prompt cycle prev", &[]);
}
