//! The command line's completion, in place like the editor's (mvp.md
//! Decision 5): the current candidate's rest is a ghost after the
//! caret, `<Tab>` takes it, `<Tab>` again cycles the candidates and
//! `<S-Tab>` cycles back; typing narrows. The first word completes to a
//! command — the ex spellings, the shell's, the engine's and Lua's —
//! and an argument to what the command takes: a path for `:e`, `:w`,
//! `:cd`, `:vs`, `:oil`; a buffer for `:b`; a tool, a view, an option.
//! Nothing is a popup: the candidates are a row in the strip.

use std::path::{MAIN_SEPARATOR, Path};

use kawoosh_editor::{KeyStroke, Mode, Prompt, commands};

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

/// The commands the shell runs (`Kawoosh::shell_command`), for the
/// command line to complete beside the engine's.
pub const SHELL_COMMANDS: &[&str] = &[
    "buffer",
    "buffer_delete",
    "buffer_delete_others",
    "buffer_list",
    "buffer_next",
    "buffer_prev",
    "cd",
    "close",
    "compile",
    "dock_toggle",
    "error_next",
    "error_prev",
    "goto_location",
    "kui_debugger",
    "kui_framerate_hud",
    "lsp_complete",
    "lsp_definition",
    "lsp_hover",
    "lsp_status",
    "lua",
    "oldfiles",
    "only",
    "pane_down",
    "pane_left",
    "pane_next",
    "pane_right",
    "pane_up",
    "perf",
    "pwd",
    "scrollback",
    "session_restore",
    "session_save",
    "split",
    "syntax_tree",
    "tab_close",
    "tab_new",
    "tab_next",
    "tab_prev",
    "terminal",
    "tool",
    "view",
    "vsplit",
];

/// The commands whose argument is a path.
const PATH_COMMANDS: &[&str] = &[
    "edit", "write", "vsplit", "split", "tab_new", "cd", "oil", "source",
];

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
        let name = head.split_whitespace().next().unwrap_or("");
        let name = name.trim_end_matches('!');
        let full = commands::ex_alias(name).unwrap_or(name);
        let mut out: Vec<String> = match full {
            f if PATH_COMMANDS.contains(&f) => self.path_candidates(token),
            "buffer" => {
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
            "tool" => {
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
            "view" => {
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
            "set" => {
                let mut v: Vec<String> = self
                    .ed
                    .options
                    .keys()
                    .filter(|n| n.starts_with(token))
                    .cloned()
                    .collect();
                v.sort();
                v
            }
            _ => Vec::new(),
        };
        out.dedup();
        (start, out)
    }

    /// The ex spellings first (`:e`, `:vs` — the short names a user
    /// types), then the shell's and the engine's full names, each group
    /// sorted; a name in two groups is listed once.
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
        push(
            commands::EX_ALIASES
                .iter()
                .flat_map(|(spellings, _)| spellings.iter().copied())
                .collect(),
        );
        push(SHELL_COMMANDS.to_vec());
        push(self.ed.command_names());
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
    /// them: `<Tab>` takes the candidate — one candidate is then
    /// completed further (a directory's entries), several are cycled
    /// by the next `<Tab>` — and `<S-Tab>` cycles back. Returns true
    /// when consumed.
    pub(crate) fn cmdline_key(&mut self, stroke: &KeyStroke) -> bool {
        if !self.at_command_prompt() {
            return false;
        }
        let note = stroke.notation();
        if note != "<Tab>" && note != "<S-Tab>" {
            return false;
        }
        let Some(c) = self.cmd_completion.as_mut() else {
            // A `<Tab>` is never a character on the command line.
            return true;
        };
        if c.candidates.is_empty() {
            return true;
        }
        let token = self.ed.cmdline.get(c.start..).unwrap_or("");
        let at_candidate = c.current() == Some(token);
        if note == "<S-Tab>" {
            c.index = (c.index + c.candidates.len() - 1) % c.candidates.len();
        } else if at_candidate {
            c.index = (c.index + 1) % c.candidates.len();
        }
        let only = c.candidates.len() == 1;
        self.take_candidate();
        if only && note == "<Tab>" {
            // The one candidate is taken: what it opens onto is next.
            self.cmdline_refresh();
        }
        true
    }

    /// After a key at the `:` prompt: the candidates for the line as it
    /// now reads, the first current; off the prompt, none.
    pub(crate) fn cmdline_refresh(&mut self) {
        if !self.at_command_prompt() {
            self.cmd_completion = None;
            return;
        }
        let line = self.ed.cmdline.clone();
        let (start, candidates) = self.cmd_candidates(&line);
        self.cmd_completion = Some(CmdCompletion {
            start,
            candidates,
            index: 0,
        });
    }
}
