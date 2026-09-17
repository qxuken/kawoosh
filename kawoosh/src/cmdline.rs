//! The command line's completion, in place like the editor's (mvp.md
//! Decision 5): the current candidate's rest is a ghost after the
//! caret, `<Tab>` takes it, `<Tab>` again cycles the candidates and
//! `<S-Tab>` cycles back; typing narrows. The first word completes to a
//! command — the ex spellings, the shell's, the engine's and Lua's —
//! and an argument to what the command declares it takes
//! (`kawoosh_editor::ArgKind`): a path for `:e`, `:w`, `:cd`, `:vs`,
//! `:oil`; a buffer for `:b`; a tool, a view, an option, a command.
//! Nothing is a popup: the candidates are a row in the strip.

use std::path::{MAIN_SEPARATOR, Path};

use kawoosh_editor::{ArgKind, Args, KeyStroke, Mode, Prompt, commands};

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

/// The commands the shell runs (`Kawoosh::shell_command`) and what each
/// takes, declared into the engine at start (`Editor::declare`) so a
/// path among the arguments arrives resolved and the command line
/// completes them like the engine's own. `true` is `rest`: the last
/// kind takes every argument past it.
pub const SHELL_COMMANDS: &[(&str, &[ArgKind], bool)] = &[
    ("buffer", &[ArgKind::Buffer], false),
    ("buffer_delete", &[], false),
    ("buffer_delete_others", &[], false),
    ("buffer_list", &[], false),
    ("buffer_next", &[], false),
    ("buffer_prev", &[], false),
    ("cd", &[ArgKind::Path], false),
    ("close", &[], false),
    ("compile", &[ArgKind::Text], true),
    ("dock_toggle", &[], false),
    ("error_next", &[], false),
    ("error_prev", &[], false),
    ("goto_location", &[], false),
    ("kui_debugger", &[ArgKind::Text], false),
    ("kui_framerate_hud", &[ArgKind::Text], false),
    ("lsp_complete", &[], false),
    ("lsp_definition", &[], false),
    ("lsp_hover", &[], false),
    ("lsp_status", &[], false),
    ("lua", &[ArgKind::Text], true),
    ("messages", &[ArgKind::Text], false),
    ("notify", &[ArgKind::Text], true),
    ("oldfiles", &[ArgKind::Text], false),
    ("only", &[], false),
    ("pane_down", &[], false),
    ("pane_left", &[], false),
    ("pane_next", &[], false),
    ("pane_right", &[], false),
    ("pane_up", &[], false),
    ("perf", &[ArgKind::Text], false),
    ("pwd", &[], false),
    ("scrollback", &[], false),
    ("session_restore", &[], false),
    ("session_save", &[], false),
    ("split", &[ArgKind::Path], false),
    ("syntax_tree", &[ArgKind::Text], false),
    ("tab_close", &[], false),
    ("tab_new", &[ArgKind::Path], false),
    ("tab_next", &[], false),
    ("tab_prev", &[], false),
    ("terminal", &[ArgKind::Text], true),
    ("tool", &[ArgKind::Tool], false),
    ("view", &[ArgKind::View], false),
    ("vsplit", &[ArgKind::Path], false),
];

/// Declares every shell command into `ed`.
pub fn declare_shell_commands(ed: &mut kawoosh_editor::Editor) {
    for (name, kinds, rest) in SHELL_COMMANDS {
        let args = if *rest {
            Args::rest(kinds)
        } else {
            Args::new(kinds)
        };
        ed.declare(name, args);
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
        let name = words.next().unwrap_or("").trim_end_matches('!');
        // Which argument the token is (a `!` on its own is not one).
        let index = words.filter(|w| *w != "!").count();
        let full = commands::ex_alias(name).unwrap_or(name);
        let kind = self.ed.command_args(full).and_then(|a| a.kind_at(index));
        let mut out: Vec<String> = match kind {
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
        };
        out.dedup();
        (start, out)
    }

    /// The ex spellings first (`:e`, `:vs` — the short names a user
    /// types), then every command the engine knows — its own, the
    /// shell's declared, Lua's — sorted; a name in both is listed once.
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
