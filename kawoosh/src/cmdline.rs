//! The command line's completion, in place like the editor's (mvp.md
//! Decision 5) and on its keys: the current candidate's rest is a
//! ghost after the caret, `<C-n>`/`<C-p>` cycle the candidates, `<Tab>`
//! or `<C-y>` takes the current one; typing narrows. The first word
//! completes to a command — the ex spellings, the shell's, the engine's
//! and Lua's — and an argument to what the command declares it takes
//! (`kawoosh_editor::ArgKind`): a path for `:e`, `:w`, `:cd`, `:vs`,
//! `:dir`; a buffer for `:b`; a tool, a view, an option, a command.
//! A command's subcommands complete as its first word (`:memory fo`
//! is `:memory forget`), and the words after complete as the
//! subcommand's own. Nothing is a popup: the candidates are a row in
//! the strip. The keys are the shell's commands `prompt complete`
//! (`<Tab>`, `<C-y>`) and `prompt cycle next|prev` (`<C-n>`, `<C-p>`),
//! bound `when field:cmdline` over the engine's history walk on the
//! same keys, which a search prompt keeps.

use std::collections::HashSet;
use std::path::{MAIN_SEPARATOR, Path};

use kawoosh_editor::{ArgKind, Cond, Mode, Prompt, Spec};

use crate::commands::{ShellCommand, cmd};

use crate::app::Kawoosh;

/// What the command line is completing: the token from `start` and the
/// candidates for it, `index` the current one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CmdCompletion {
    pub start: usize,
    pub candidates: Vec<String>,
    pub index: usize,
}

impl CmdCompletion {
    pub fn current(&self) -> Option<&str> {
        self.candidates.get(self.index).map(String::as_str)
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

impl Kawoosh {
    /// The token the command line is on and its candidates: command
    /// names for the first word, what the command takes after it.
    pub(crate) fn cmd_candidates(&mut self, line: &str) -> (usize, Vec<String>) {
        let start = token_start(line);
        let token = &line[start..];
        if start == 0 {
            return (0, self.command_name_candidates(token));
        }
        let head = line[..start].trim();
        let mut words = head.split_whitespace();
        let name = words.next().unwrap_or("");
        let words: Vec<String> = words.map(str::to_string).collect();
        // The command the words so far name — its alias resolved, its
        // subcommands consumed — and which of its arguments the token is.
        let inv = self.ed.commands.resolve(name, &words);
        let index = inv.args.len();
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
        // `:compile`: its commands' names first, then paths from where
        // the line will run (compile.md Decision 7).
        if inv.name == "compile" {
            if index == 0 {
                let mut names: Vec<String> = self
                    .compile_commands()
                    .into_iter()
                    .map(|n| n.name)
                    .filter(|n| n.starts_with(token))
                    .collect();
                names.sort();
                out.extend(names);
            } else {
                let dir = self.compile_dir_of(&inv.args.join(" "));
                out.extend(self.path_candidates_in(token, &dir));
            }
            out.dedup();
            return (start, out);
        }
        let kind = self
            .ed
            .command_args(&inv.name)
            .and_then(|a| a.kind_at(index));
        let args: Vec<String> = match kind {
            None | Some(ArgKind::Text) => Vec::new(),
            Some(ArgKind::Path) => self.path_candidates(token),
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
        (start, out)
    }

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
    /// is offered only to a token that starts with `.`.
    fn path_candidates(&self, token: &str) -> Vec<String> {
        self.path_candidates_in(token, &self.cwd)
    }

    /// [`Self::path_candidates`] with a relative token read from `base`.
    fn path_candidates_in(&self, token: &str, base: &Path) -> Vec<String> {
        let cut = token
            .rfind(['/', MAIN_SEPARATOR])
            .map(|i| i + 1)
            .unwrap_or(0);
        let (dir_text, prefix) = token.split_at(cut);
        let dir = if dir_text.is_empty() {
            base.to_path_buf()
        } else {
            kawoosh_doc::paths::expand(Path::new(dir_text), base)
        };
        let Ok(entries) = kawoosh_systems::fs::list(&dir) else {
            return Vec::new();
        };
        entries
            .into_iter()
            .filter(|e| e.name.starts_with(prefix))
            .filter(|e| prefix.starts_with('.') || !e.name.starts_with('.'))
            .map(|e| {
                let mut s = format!("{dir_text}{}", e.name);
                if e.is_dir {
                    s.push(MAIN_SEPARATOR);
                }
                s
            })
            .collect()
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
        let token = line.get(c.start..)?;
        c.ghost(token).map(str::to_string)
    }

    /// `<Tab>` / `<C-y>`: the token replaced with the current candidate,
    /// then completed on from it (a directory's entries, a command's
    /// longer spellings). Nothing to take is nothing done — a `<Tab>`
    /// is never a character on the command line.
    fn take_candidate(&mut self) {
        let Some(c) = &self.cmd_completion else {
            return;
        };
        let Some(cand) = c.current() else { return };
        let Some(line) = self.ed.prompt_text() else {
            return;
        };
        let start = c.start.min(line.len());
        let mut line = line[..start].to_string();
        line.push_str(cand);
        self.ed.set_prompt_text(&line);
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
            return;
        }
        let line = self.ed.prompt_text().unwrap_or_default();
        let (start, candidates) = self.cmd_candidates(&line);
        let index = match &self.cmd_completion {
            Some(c) if c.start == start && c.candidates == candidates => c.index,
            _ => 0,
        };
        self.cmd_completion = Some(CmdCompletion {
            start,
            candidates,
            index,
        });
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
    let at = [Cond::parse("field:cmdline")];
    km.bind_when(Mode::Insert, "<Tab>", "prompt complete", &at);
    km.bind_when(Mode::Insert, "<C-y>", "prompt complete", &at);
    km.bind_when(Mode::Insert, "<C-n>", "prompt cycle next", &at);
    km.bind_when(Mode::Insert, "<C-p>", "prompt cycle prev", &at);
}
