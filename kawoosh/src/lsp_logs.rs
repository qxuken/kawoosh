//! What each language server said, kept apart from the notifications:
//! its stderr, its `window/logMessage` and `window/showMessage`, every
//! line of it whatever `notes.keep` lets through — a server's stderr is
//! a trace there, dropped by default, and it is where clangd and
//! rust-analyzer say most of what they do. [`LOG_LINES`] a server, by
//! its command. `:lsp logs` shows one server's or every one's in
//! `*lsp logs*`, live while it is open; `:lsp info` reads its tail.

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use kawoosh_editor::{ArgKind, Args, Selection, Selections, Spec};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::notify::{Level, clock};

pub const LSP_LOGS_BUFFER: &str = "*lsp logs*";

/// The most lines kept of one server; the oldest go first.
pub const LOG_LINES: usize = 5000;

/// One thing a server said: when, how (the protocol's MessageType, 1
/// error … 4 log, and 5 for a line of its stderr) and what.
#[derive(Clone, Debug)]
pub struct Said {
    pub at: SystemTime,
    pub kind: u64,
    pub text: String,
}

impl Said {
    /// Its kind as a word: the level's, `stderr` for a line of it.
    fn how(&self) -> &'static str {
        match self.kind {
            5 => "stderr",
            k => Level::from_lsp(k).name(),
        }
    }
}

#[derive(Default)]
pub struct ServerLogs {
    by_server: BTreeMap<String, VecDeque<Said>>,
    /// Moves with every line.
    version: u64,
    /// What `*lsp logs*` shows — one server's by command, or every
    /// one's — and the version it was drawn at.
    shown: Option<(Option<String>, u64)>,
}

impl ServerLogs {
    pub fn push(&mut self, server: &str, kind: u64, text: &str) {
        let lines = self.by_server.entry(server.to_string()).or_default();
        if lines.len() >= LOG_LINES {
            lines.pop_front();
        }
        lines.push_back(Said {
            at: SystemTime::now(),
            kind,
            text: text.to_string(),
        });
        self.version += 1;
    }

    /// What `server` said, oldest first.
    pub fn of(&self, server: &str) -> std::collections::vec_deque::Iter<'_, Said> {
        static NONE: VecDeque<Said> = VecDeque::new();
        self.by_server.get(server).unwrap_or(&NONE).iter()
    }

    /// The servers that said anything.
    pub fn servers(&self) -> impl Iterator<Item = &str> {
        self.by_server.keys().map(String::as_str)
    }

    pub fn clear(&mut self) {
        self.by_server.clear();
        self.version += 1;
    }

    /// `server`'s lines, or every server's in the order they were said
    /// with each one's name — a line each, a message's further lines
    /// indented under it.
    pub fn render(&self, only: Option<&str>) -> String {
        let mut rows: Vec<(&str, &Said)> = match only {
            Some(s) => self.of(s).map(|l| (s, l)).collect(),
            None => self
                .by_server
                .iter()
                .flat_map(|(s, ls)| ls.iter().map(move |l| (s.as_str(), l)))
                .collect(),
        };
        if only.is_none() {
            rows.sort_by_key(|(_, l)| l.at);
        }
        let mut out = String::new();
        for (server, l) in rows {
            let head = match only {
                Some(_) => format!("{}  {:<6}  ", clock(l.at), l.how()),
                None => format!("{}  {server}  {:<6}  ", clock(l.at), l.how()),
            };
            for (i, line) in l.text.lines().enumerate() {
                if i == 0 {
                    out += &head;
                } else {
                    out += &" ".repeat(head.chars().count());
                }
                out += line;
                out.push('\n');
            }
        }
        if out.is_empty() {
            out = match only {
                Some(s) => format!("{s} has said nothing yet\n"),
                None => "no language server has said anything yet\n".into(),
            };
        }
        out
    }
}

impl Kawoosh {
    /// `:lsp logs [LANGUAGE]`: what the server of `LANGUAGE` — bare, of
    /// the caret buffer's language, else every server — said, in a pane
    /// with the keys, the caret on the newest line.
    fn show_lsp_logs(&mut self, language: Option<String>) {
        let caret = self
            .focused_view()
            .map(|v| self.ed.buffer_of(v).language.to_string());
        let named = language.is_some();
        let server = language.or(caret).and_then(|l| {
            self.lsp
                .defs
                .iter()
                .chain(self.scripting.servers.iter())
                .find(|d| d.serves(&l))
                .map(|d| d.command.clone())
        });
        if named && server.is_none() {
            self.ed.message = "no language server for that language".into();
            return;
        }
        let text = self.lsp.logs.render(server.as_deref());
        self.show_in_pane(LSP_LOGS_BUFFER, &text);
        self.lsp.logs.shown = Some((server, self.lsp.logs.version));
        self.lsp_logs_to_end();
    }

    /// The caret of the pane showing `*lsp logs*` on its newest line.
    fn lsp_logs_to_end(&mut self) {
        for p in self.layout.visible_panes() {
            let Some(v) = self.view_of(p) else { continue };
            let buf = self.ed.buffer_of(v);
            if buf.name != LSP_LOGS_BUFFER {
                continue;
            }
            let last = buf.line_start(buf.line_count().saturating_sub(2));
            self.ed.views[v].sels = Selections::single(Selection::point(last));
        }
    }

    /// `*lsp logs*` drawn again when a server said more, while it is
    /// open; the caret kept on the newest line if it was there.
    pub(crate) fn sync_lsp_logs(&mut self) {
        let Some((server, drawn)) = self.lsp.logs.shown.clone() else {
            return;
        };
        if drawn == self.lsp.logs.version {
            return;
        }
        let Some(id) = self
            .ed
            .buffers
            .iter()
            .find(|(_, b)| b.name == LSP_LOGS_BUFFER)
            .map(|(id, _)| id)
        else {
            self.lsp.logs.shown = None;
            return;
        };
        let at_end = self.ed.views.values().any(|v| {
            v.buffer == id && {
                let b = &self.ed.buffers[id];
                b.line_of(v.sels.primary().head) + 2 >= b.line_count()
            }
        });
        let text = self.lsp.logs.render(server.as_deref());
        let b = &mut self.ed.buffers[id];
        b.set_text(&text);
        b.mark_saved();
        self.lsp.logs.shown = Some((server, self.lsp.logs.version));
        if at_end {
            self.lsp_logs_to_end();
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("lsp logs")
                .args(Args::new(&[ArgKind::Language]))
                .doc("what the server of LANGUAGE — bare, the caret's language's, else every one — said: its stderr and log messages, live"),
            |k, ctx| k.show_lsp_logs(ctx.args.first().cloned()),
        ),
        cmd(
            Spec::new("lsp logs clear").doc("forget what the servers said"),
            |k, _| {
                k.lsp.logs.clear();
                k.ed.message = "lsp logs cleared".into();
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_servers_lines_are_kept_capped_and_rendered() {
        let mut l = ServerLogs::default();
        l.push("clangd", 5, "I[1] started");
        l.push("gopls", 4, "two\nlines");
        l.push("clangd", 2, "careful");
        let one = l.render(Some("clangd"));
        let rows: Vec<&str> = one.lines().collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].ends_with("  stderr  I[1] started"), "{one}");
        assert!(rows[1].ends_with("  warn    careful"), "{one}");
        let all = l.render(None);
        assert!(all.contains("  gopls  debug   two\n"), "{all}");
        assert!(
            all.lines().any(|r| r.trim() == "lines"),
            "indented under it: {all}"
        );
        assert_eq!(
            l.render(Some("rust-analyzer")),
            "rust-analyzer has said nothing yet\n"
        );
        for i in 0..LOG_LINES + 5 {
            l.push("clangd", 5, &i.to_string());
        }
        assert_eq!(l.of("clangd").len(), LOG_LINES);
        assert_eq!(l.of("clangd").next().unwrap().text, "5");
    }
}
