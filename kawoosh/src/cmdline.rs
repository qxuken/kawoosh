//! The command line's completion, in place like the editor's (mvp.md
//! Decision 5) and on its keys: the current candidate's rest is a
//! ghost after the caret, `<C-n>`/`<C-p>` cycle the candidates, `<Tab>`
//! or `<C-y>` takes the current one; typing narrows. The first word
//! completes to a command — the ex spellings, the shell's, the engine's
//! and Lua's — and an argument to what the command declares it takes
//! (`kawoosh_editor::ArgKind`): a path for `:e`, `:w`, `:cd`, `:vs`,
//! `:oil`; a buffer for `:b`; a tool, a view, an option, a command.
//! A command's subcommands complete as its first word (`:history dr`
//! is `:history drop`), and the words after complete as the
//! subcommand's own. Nothing is a popup: the candidates are a row in
//! the strip.

use std::path::{MAIN_SEPARATOR, Path};

use kawoosh_editor::{ArgKind, KeyStroke, Mode, Prompt, Spec};

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
    /// it. Nothing typed yet is nothing to extend: the first of every
    /// command is not a suggestion.
    pub fn ghost(&self, token: &str) -> Option<&str> {
        if token.is_empty() {
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
    pub(crate) fn cmd_candidates(&self, line: &str) -> (usize, Vec<String>) {
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
        let kind = self
            .ed
            .command_args(&inv.name)
            .and_then(|a| a.kind_at(index));
        let args: Vec<String> = match kind {
            None | Some(ArgKind::Text) => Vec::new(),
            Some(ArgKind::Path) => self.path_candidates(token),
            Some(ArgKind::Command) => self.command_name_candidates(token),
            Some(ArgKind::Buffer) => {
                let names: Vec<&str> = self.ed.buffers.values().map(|b| b.name.as_str()).collect();
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
                // Every leaf of the effective tree, by dotted path; `no`
                // and `=` are not part of the name.
                let name = token.strip_prefix("no").unwrap_or(token);
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
    /// the command line first: the ex spellings and the commands with
    /// a line of doc (`:view`, `:vsplit`), then the rest — the
    /// keymap's `visual_mode` and its kind, typed rarely. Each group
    /// sorted, a name once.
    fn command_name_candidates(&self, token: &str) -> Vec<String> {
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
        let (documented, bare): (Vec<&Spec>, Vec<&Spec>) = self
            .ed
            .commands
            .specs()
            .into_iter()
            .filter(|s| !s.name.contains(' '))
            .partition(|s| !s.doc.is_empty());
        let mut typed = self.ed.commands.alias_names();
        typed.extend(documented.iter().map(|s| s.name.as_str()));
        push(typed);
        push(bare.iter().map(|s| s.name.as_str()).collect());
        out
    }

    /// The entries of the directory the token names, with the token's
    /// own directory part kept as typed (`~/pro` completes to
    /// `~/projects/`, not to the home's absolute path). A hidden entry
    /// is offered only to a token that starts with `.`.
    fn path_candidates(&self, token: &str) -> Vec<String> {
        let cut = token
            .rfind(['/', MAIN_SEPARATOR])
            .map(|i| i + 1)
            .unwrap_or(0);
        let (dir_text, prefix) = token.split_at(cut);
        let dir = if dir_text.is_empty() {
            self.cwd.clone()
        } else {
            self.resolve(Path::new(dir_text))
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
        self.ed.mode == Mode::Command && self.ed.prompt == Prompt::Command
    }

    /// The ghost after the caret, if the current candidate extends what
    /// is typed.
    pub fn cmdline_ghost(&self) -> Option<String> {
        if !self.at_command_prompt() {
            return None;
        }
        let c = self.cmd_completion.as_ref()?;
        let token = self.ed.cmdline.get(c.start..)?;
        c.ghost(token).map(str::to_string)
    }

    /// Replaces the token with the current candidate.
    fn take_candidate(&mut self) {
        let Some(c) = &self.cmd_completion else {
            return;
        };
        let Some(cand) = c.current() else { return };
        let start = c.start.min(self.ed.cmdline.len());
        let mut line = self.ed.cmdline[..start].to_string();
        line.push_str(cand);
        self.ed.cmdline = line;
    }

    /// Keys the `:` prompt's completion takes before the engine sees
    /// them, the buffer completion's (`Kawoosh::completion_key`):
    /// `<C-n>`/`<C-p>` cycle the candidates — the ghost moves, the line
    /// does not — and `<Tab>` or `<C-y>` takes the current one, then
    /// completes on from it (a directory's entries, a command's
    /// longer spellings). `<Up>`/`<Down>` stay the history's. Returns
    /// true when consumed.
    pub(crate) fn cmdline_key(&mut self, stroke: &KeyStroke) -> bool {
        if !self.at_command_prompt() {
            return false;
        }
        let note = stroke.notation();
        let take = matches!(note.as_str(), "<Tab>" | "<C-y>");
        if !take && note != "<C-n>" && note != "<C-p>" {
            return false;
        }
        let Some(c) = self.cmd_completion.as_mut() else {
            // A `<Tab>` is never a character on the command line.
            return take;
        };
        if c.candidates.is_empty() {
            return take;
        }
        match note.as_str() {
            "<C-n>" => c.index = (c.index + 1) % c.candidates.len(),
            "<C-p>" => c.index = (c.index + c.candidates.len() - 1) % c.candidates.len(),
            _ => {
                self.take_candidate();
                // The candidate is taken: what it opens onto is next.
                self.cmdline_refresh();
            }
        }
        true
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
        let line = self.ed.cmdline.clone();
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
