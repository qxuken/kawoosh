//! Terminal panes (milestone 4): the `term` facade behind each pane, keys
//! encoded to bytes, the pane prefix chord, scrollback into a buffer, and
//! the locations pattern table that turns `src/main.rs:42` under the
//! pointer into an open file (mvp.md Decisions 3, 3b, 5c).

use std::collections::HashMap;
use std::path::Path;

use kawoosh_doc::Buffer;
use kawoosh_editor::{KeyStroke, Lookup, Mode, Selection, motions};
use kawoosh_term::{TermSize, Terminal, encode_key};

use crate::app::Kawoosh;
use crate::layout::{Content, SplitDir};

pub type TermId = u64;

#[derive(Default)]
pub struct Terminals {
    pub map: HashMap<TermId, Terminal>,
    next: TermId,
    /// `<C-w>` was pressed in a terminal pane: the next key is a pane
    /// command (`<C-w>.` sends a literal ^W).
    pub prefix: bool,
    /// `<C-\>` was pressed: `<C-n>` next materialises the scrollback.
    pub backslash: bool,
    /// The wheel's fraction of a line carried per terminal.
    pub carry: HashMap<TermId, f32>,
}

impl Terminals {
    pub fn add(&mut self, t: Terminal) -> TermId {
        self.next += 1;
        self.map.insert(self.next, t);
        self.next
    }
}

/// A `path[:line[:col]]` in `text` around byte `at` — rustc, tsc, grep
/// and shell spellings. Extensible from Lua later (Decision 5c).
pub fn location_at(text: &str, at: usize) -> Option<(String, Option<usize>, Option<usize>)> {
    let is_path_char = |c: char| c.is_alphanumeric() || "./_-~+@:%".contains(c);
    let at = at.min(text.len());
    let start = text[..at]
        .char_indices()
        .rev()
        .find(|(_, c)| !is_path_char(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| !is_path_char(*c))
        .map(|(i, _)| at + i)
        .unwrap_or(text.len());
    // Trailing punctuation is the sentence's, a leading `./` is the path's.
    let token = text[start..end]
        .trim_end_matches(|c: char| ":.,;".contains(c))
        .trim_start_matches([':', ',', ';']);
    if token.is_empty() || !token.contains(['/', '.']) {
        return None;
    }
    let mut parts = token.split(':');
    let path = parts.next()?.to_string();
    let line = parts.next().and_then(|s| s.parse().ok());
    let col = parts.next().and_then(|s| s.parse().ok());
    Some((path, line, col))
}

impl Kawoosh {
    /// Spawns a shell (or `cmd`) sized for a pane, with the `$EDITOR`
    /// handoff in its environment (Decision 3b).
    pub fn spawn_terminal(&mut self, cmd: Option<&str>, cwd: Option<&Path>) -> Option<TermId> {
        let mut envs = vec![
            ("TERM_PROGRAM".to_string(), "kawoosh".to_string()),
            (
                "TERM_APPEARANCE".to_string(),
                if self.dark { "dark" } else { "light" }.to_string(),
            ),
        ];
        if let Some(sock) = &self.socket {
            envs.push(("KAWOOSH_SOCKET".into(), sock.display().to_string()));
            if let Ok(exe) = std::env::current_exe() {
                let shim = format!("{} edit --wait", exe.display());
                envs.push(("EDITOR".into(), shim.clone()));
                envs.push(("VISUAL".into(), shim));
                envs.push((
                    "GIT_EDITOR".into(),
                    format!("{} edit --wait", exe.display()),
                ));
            }
        }
        let size = TermSize { rows: 24, cols: 80 };
        let cwd = cwd
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.cwd.clone());
        match Terminal::spawn(cmd, Some(&cwd), size, &envs) {
            Ok((term, reader)) => {
                let id = self.terms.add(term);
                self.io.watch_pty(id, reader);
                Some(id)
            }
            Err(e) => {
                self.ed.message = format!("cannot spawn a terminal: {e:#}");
                None
            }
        }
    }

    /// A terminal with no process, in a new split — for tests, which feed
    /// it bytes through [`Kawoosh::feed_terminal`].
    pub fn add_headless_terminal(&mut self) -> TermId {
        let t = Terminal::headless(TermSize { rows: 24, cols: 80 });
        let id = self.terms.add(t);
        self.layout.split(SplitDir::V, Content::Terminal(id));
        id
    }

    pub fn feed_terminal(&mut self, id: TermId, bytes: &[u8]) {
        if let Some(t) = self.terms.map.get_mut(&id) {
            t.feed(bytes);
        }
    }

    /// A key in a focused terminal pane.
    pub(crate) fn term_key(&mut self, id: TermId, stroke: KeyStroke) {
        let note = stroke.notation();
        if self.terms.backslash {
            self.terms.backslash = false;
            if note == "<C-n>" {
                self.scrollback_to_buffer(id);
                return;
            }
        }
        if self.terms.prefix {
            self.terms.prefix = false;
            if note == "." {
                if let Some(t) = self.terms.map.get_mut(&id) {
                    t.input(&[0x17]);
                }
                return;
            }
            if note == ":" {
                self.ed.mode = Mode::Command;
                self.ed.prompt = kawoosh_editor::Prompt::Command;
                self.ed.cmdline.clear();
                return;
            }
            let keys = ["<C-w>".to_string(), note.clone()];
            if let Lookup::Exact(b) = self.ed.keymap.lookup_lenient(Mode::Normal, &keys) {
                let b = b.clone();
                self.shell_command(&b.command, &b.args, None);
            }
            return;
        }
        match note.as_str() {
            "<C-w>" => {
                self.terms.prefix = true;
                return;
            }
            "<C-\\>" => {
                self.terms.backslash = true;
                return;
            }
            _ => {}
        }
        let Some(t) = self.terms.map.get_mut(&id) else {
            return;
        };
        if let Some(bytes) = encode_key(
            &stroke.code,
            stroke.text.as_deref(),
            stroke.ctrl,
            stroke.alt,
            stroke.shift,
            t.app_cursor_keys(),
        ) {
            t.scroll_to_bottom();
            t.input(&bytes);
        }
    }

    /// The scrollback and screen of terminal `id` as a buffer in a new
    /// split, with full modal editing (Decision 3).
    pub fn scrollback_to_buffer(&mut self, id: TermId) {
        let Some(t) = self.terms.map.get(&id) else {
            return;
        };
        let text = t.scrollback_text();
        let name = format!(
            "*scrollback {}*",
            if t.title.is_empty() {
                id.to_string()
            } else {
                t.title.clone()
            }
        );
        let buf = Buffer::new(name, &text);
        let bid = self.ed.add_buffer(buf);
        let v = self.ed.add_view(bid);
        // Land at the end, where the prompt was.
        let len = self.ed.buffers[bid].len();
        let last =
            self.ed.buffers[bid].line_start(self.ed.buffers[bid].line_count().saturating_sub(1));
        self.ed.views[v].sels = kawoosh_editor::Selections::single(Selection::point(last.min(len)));
        self.layout.split(SplitDir::V, Content::Editor(v));
    }

    /// Opens the file named at `(row, col)` of terminal `id`'s screen —
    /// `gf` across the terminal/editor boundary (Decision 5c).
    pub fn open_location_at(&mut self, id: TermId, row: usize, col: usize) -> bool {
        let Some(t) = self.terms.map.get(&id) else {
            return false;
        };
        let text = t.row_text(row);
        let at = text
            .char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        let Some((path, line, colno)) = location_at(&text, at) else {
            self.ed.message = "no path under the pointer".into();
            return false;
        };
        let base = t.cwd.clone().unwrap_or_else(|| self.cwd.clone());
        let full = kawoosh_systems::fs::expand(Path::new(&path), &base);
        if !full.exists() {
            self.ed.message = format!("not found: {}", full.display());
            return false;
        }
        self.open_in_editor(&full, line, colno);
        true
    }

    /// Opens `path` in an editor pane — the focused one, or a split
    /// beside a terminal — and moves to `line:col` when given.
    pub fn open_in_editor(&mut self, path: &Path, line: Option<usize>, col: Option<usize>) {
        if self.focused_view().is_none() {
            // From a terminal: prefer an editor pane already on screen.
            let editor_pane = self
                .layout
                .visible_panes()
                .into_iter()
                .find(|p| matches!(self.layout.content(*p), Some(Content::Editor(_))));
            match editor_pane {
                Some(p) => self.layout.focus(p),
                None => {
                    let Some(id) = self.buffer_for(path) else {
                        return;
                    };
                    let v = self.ed.add_view(id);
                    self.layout.split(SplitDir::H, Content::Editor(v));
                }
            }
        }
        self.open(path);
        if let (Some(v), Some(ln)) = (self.focused_view(), line) {
            let buf = self.ed.buffer_of(v);
            let ln = ln.max(1).min(buf.line_count()) - 1;
            let off = match col {
                Some(c) => motions::offset_at(buf, ln, c.saturating_sub(1)),
                None => motions::first_nonblank(buf, ln),
            };
            self.ed.views[v].sels = kawoosh_editor::Selections::single(Selection::point(off));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations() {
        let l = "error: x\n  --> src/main.rs:42:7\n";
        let at = l.find("main").unwrap();
        assert_eq!(
            location_at(l, at),
            Some(("src/main.rs".into(), Some(42), Some(7)))
        );
        assert_eq!(
            location_at("see ./a.txt.", 6),
            Some(("./a.txt".into(), None, None))
        );
        assert_eq!(location_at("just words here", 6), None);
        assert_eq!(
            location_at("lib/foo.rb:10: warning", 4),
            Some(("lib/foo.rb".into(), Some(10), None))
        );
    }
}
