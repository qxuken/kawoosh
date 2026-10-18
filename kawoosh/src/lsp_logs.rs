//! What each language server said, kept apart from the notifications:
//! its stderr, its `window/logMessage` and `window/showMessage`, every
//! line of it whatever `notes.keep` lets through — a server's stderr is
//! a trace there, dropped by default, and it is where clangd and
//! rust-analyzer say most of what they do. [`LOG_LINES`] a stream — a
//! server's command and the root it serves, so two rust-analyzers on
//! two projects keep their own (lsp-rules.md Decision 10), each line
//! tagged with which instance there said it, the one before a restart
//! or the one after — and [`LOG_TOTAL`] in all, the oldest going first
//! whoever said them, so streams of servers long gone do not pile up.
//! `:lsp logs`
//! shows the caret's instance, a language's server's instances or every
//! one's in `*lsp logs*`, live while it is open; `:lsp` reads each
//! instance's tail.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use kawoosh_editor::{ArgKind, Args, Selection, Selections, Spec};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::notify::{Level, clock};

pub const LSP_LOGS_BUFFER: &str = "*lsp logs*";

/// The most lines kept of one stream; the oldest go first.
pub const LOG_LINES: usize = 5000;

/// The most lines kept of every stream together: a go-to-definition
/// into each crate of a registry starts a server per crate, and each
/// stream a server left kept its lines while kawoosh ran.
pub const LOG_TOTAL: usize = 10_000;

/// One thing a server said: when, how (the protocol's MessageType, 1
/// error … 4 log, and 5 for a line of its stderr) and what.
#[derive(Clone, Debug)]
pub struct Said {
    pub at: SystemTime,
    pub kind: u64,
    pub text: String,
    /// Which instance of its stream said it: 1 the first started at
    /// that root, 2 the one after it, ….
    pub generation: usize,
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

/// Whose lines `*lsp logs*` shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    /// Every instance's, each line labelled with its server and root.
    All,
    /// Every instance of one server, by command — labelled with the
    /// root when there are two or more.
    Server(String),
    /// One instance: a server's command and its root.
    Instance(String, PathBuf),
}

/// What the instances of one server at one root said.
#[derive(Default)]
struct Stream {
    lines: VecDeque<Said>,
    /// The pool's key of each instance that spoke, in order: a line's
    /// generation is its instance's place here.
    instances: Vec<usize>,
}

#[derive(Default)]
pub struct ServerLogs {
    /// By the server's command and the root it serves.
    by_instance: BTreeMap<(String, PathBuf), Stream>,
    /// Lines kept in all.
    total: usize,
    /// Moves with every line.
    version: u64,
    /// What `*lsp logs*` shows, and the version it was drawn at.
    shown: Option<(Shown, u64)>,
}

impl ServerLogs {
    /// A line from `server`'s instance `instance` (the pool's key) at
    /// `root`.
    pub fn push(&mut self, server: &str, root: &Path, instance: usize, kind: u64, text: &str) {
        let stream = self
            .by_instance
            .entry((server.to_string(), root.to_path_buf()))
            .or_default();
        let generation = match stream.instances.iter().position(|k| *k == instance) {
            Some(i) => i + 1,
            None => {
                stream.instances.push(instance);
                stream.instances.len()
            }
        };
        if stream.lines.len() >= LOG_LINES {
            stream.lines.pop_front();
            self.total -= 1;
        }
        stream.lines.push_back(Said {
            at: SystemTime::now(),
            kind,
            text: text.to_string(),
            generation,
        });
        self.total += 1;
        while self.total > LOG_TOTAL {
            self.drop_oldest();
        }
        self.version += 1;
    }

    /// The oldest line of every stream let go of, and its stream with
    /// it once it is empty.
    fn drop_oldest(&mut self) {
        let Some(key) = self
            .by_instance
            .iter()
            .filter_map(|(k, s)| Some((s.lines.front()?.at, k)))
            .min()
            .map(|(_, k)| k.clone())
        else {
            return;
        };
        let stream = self.by_instance.get_mut(&key).expect("found");
        stream.lines.pop_front();
        self.total -= 1;
        if stream.lines.is_empty() {
            self.by_instance.remove(&key);
        }
    }

    /// What the instances of `server` in `root` said, oldest first.
    pub fn of(&self, server: &str, root: &Path) -> impl DoubleEndedIterator<Item = &Said> {
        self.by_instance
            .get(&(server.to_string(), root.to_path_buf()))
            .into_iter()
            .flat_map(|s| s.lines.iter())
    }

    /// What the newest instance of `server` in `root` said: the one
    /// running, after a restart there.
    pub fn current(&self, server: &str, root: &Path) -> impl DoubleEndedIterator<Item = &Said> {
        let stream = self
            .by_instance
            .get(&(server.to_string(), root.to_path_buf()));
        let newest = stream.map_or(0, |s| s.instances.len());
        stream
            .into_iter()
            .flat_map(|s| s.lines.iter())
            .filter(move |l| l.generation == newest)
    }

    /// Lines kept in all.
    pub fn total(&self) -> usize {
        self.total
    }

    /// The instances that said anything: each one's server and root.
    pub fn instances(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.by_instance
            .keys()
            .map(|(s, r)| (s.as_str(), r.as_path()))
    }

    pub fn clear(&mut self) {
        self.by_instance.clear();
        self.total = 0;
        self.version += 1;
    }

    /// The lines `shown` takes, a line each — in the order they were
    /// said, each with its instance's label when they are several
    /// instances' — a message's further lines indented under it.
    pub fn render(&self, shown: &Shown) -> String {
        let instances: Vec<(&(String, PathBuf), &Stream)> = self
            .by_instance
            .iter()
            .filter(|((s, r), _)| match shown {
                Shown::All => true,
                Shown::Server(c) => s == c,
                Shown::Instance(c, root) => s == c && r == root,
            })
            .collect();
        let label = |s: &str, r: &Path| match shown {
            Shown::All => format!("{s} {}  ", kawoosh_systems::fs::abbreviate_home(r)),
            Shown::Server(_) if instances.len() > 1 => {
                format!("{}  ", kawoosh_systems::fs::abbreviate_home(r))
            }
            _ => String::new(),
        };
        let mut rows: Vec<(String, &Said)> = instances
            .iter()
            .flat_map(|((s, r), stream)| {
                let l = label(s, r);
                // Which instance, once a restart made two there.
                let tagged = stream.instances.len() > 1;
                stream.lines.iter().map(move |said| {
                    let l = if tagged {
                        format!("{l}#{} ", said.generation)
                    } else {
                        l.clone()
                    };
                    (l, said)
                })
            })
            .collect();
        if instances.len() > 1 {
            rows.sort_by_key(|(_, l)| l.at);
        }
        let mut out = String::new();
        for (label, l) in rows {
            let head = format!("{}  {label}{:<6}  ", clock(l.at), l.how());
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
            out = match shown {
                Shown::All => "no language server has said anything yet\n".into(),
                Shown::Server(s) => format!("{s} has said nothing yet\n"),
                Shown::Instance(s, r) => format!(
                    "{s} in {} has said nothing yet\n",
                    kawoosh_systems::fs::abbreviate_home(r)
                ),
            };
        }
        out
    }
}

impl Kawoosh {
    /// `:lsp logs [LANGUAGE]`: what the server of `LANGUAGE` said, every
    /// instance of it; bare, what the instance holding the caret's
    /// buffer said — the server of its language whose root it is under
    /// — or, with none, every instance of that server, else every one's.
    /// In a pane with the keys, the caret on the newest line.
    fn show_lsp_logs(&mut self, language: Option<String>) {
        let caret = self.focused_view().map(|v| self.ed.buffer_of(v));
        let named = language.is_some();
        let server = language
            .or_else(|| caret.map(|b| b.language.to_string()))
            .and_then(|l| {
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
        let shown = match server {
            None => Shown::All,
            Some(s) if named => Shown::Server(s),
            Some(s) => {
                let path = caret.and_then(|b| b.path.clone());
                // The nearest root above it: a project inside another
                // has a server of its own.
                let root = self
                    .lsp
                    .status
                    .iter()
                    .filter(|(root, cmd, ..)| {
                        *cmd == s && path.as_ref().is_some_and(|p| p.starts_with(root))
                    })
                    .map(|(root, ..)| root.clone())
                    .max_by_key(|r| r.components().count());
                match root {
                    Some(r) => Shown::Instance(s, r),
                    None => Shown::Server(s),
                }
            }
        };
        self.show_lsp_logs_of(shown);
    }

    fn show_lsp_logs_of(&mut self, shown: Shown) {
        let text = self.lsp.logs.render(&shown);
        self.show_in_pane(LSP_LOGS_BUFFER, &text);
        self.lsp.logs.shown = Some((shown, self.lsp.logs.version));
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
        let Some((shown, drawn)) = self.lsp.logs.shown.clone() else {
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
        let text = self.lsp.logs.render(&shown);
        let b = &mut self.ed.buffers[id];
        b.set_text(&text);
        b.mark_saved();
        self.lsp.logs.shown = Some((shown, self.lsp.logs.version));
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
                .doc("what the server of LANGUAGE said, each of its projects' — bare, the one serving the caret's file, else every one: its stderr and log messages, live"),
            |k, ctx| k.show_lsp_logs(ctx.args.first().cloned()),
        ),
        cmd(
            Spec::new("lsp logs all")
                .doc("what every language server said, each line labelled with the server and its project, live"),
            |k, _| k.show_lsp_logs_of(Shown::All),
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
    fn an_instances_lines_are_kept_capped_and_rendered() {
        let (a, b) = (Path::new("/w/a"), Path::new("/w/b"));
        let mut l = ServerLogs::default();
        l.push("clangd", a, 0, 5, "I[1] started");
        l.push("gopls", a, 1, 4, "two\nlines");
        l.push("clangd", a, 0, 2, "careful");
        let one = l.render(&Shown::Server("clangd".into()));
        let rows: Vec<&str> = one.lines().collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].ends_with("  stderr  I[1] started"), "{one}");
        assert!(rows[1].ends_with("  warn    careful"), "{one}");
        let all = l.render(&Shown::All);
        assert!(all.contains("  gopls /w/a  debug   two\n"), "{all}");
        assert!(
            all.lines().any(|r| r.trim() == "lines"),
            "indented under it: {all}"
        );
        assert_eq!(
            l.render(&Shown::Server("rust-analyzer".into())),
            "rust-analyzer has said nothing yet\n"
        );
        for i in 0..LOG_LINES + 5 {
            l.push("clangd", a, 0, 5, &i.to_string());
        }
        assert_eq!(l.of("clangd", a).count(), LOG_LINES);
        assert_eq!(l.of("clangd", a).next().unwrap().text, "5");

        // A second instance of one server keeps its own.
        l.push("clangd", b, 2, 3, "in b");
        assert_eq!(
            l.of("clangd", b).map(|s| &*s.text).collect::<Vec<_>>(),
            ["in b"]
        );
        assert!(l.of("clangd", a).all(|s| s.text != "in b"));
        let just_b = l.render(&Shown::Instance("clangd".into(), b.into()));
        assert_eq!(just_b.lines().count(), 1);
        assert!(just_b.ends_with("  info    in b\n"), "unlabelled: {just_b}");
        let both = l.render(&Shown::Server("clangd".into()));
        assert!(both.contains("  /w/b  info    in b\n"), "by root: {both}");
        assert!(both.contains("  /w/a  stderr  "), "by root: {both}");
        assert_eq!(
            l.render(&Shown::Instance("clangd".into(), "/w/c".into())),
            "clangd in /w/c has said nothing yet\n"
        );

        // A restart at one root: the new instance's lines tagged apart,
        // and its own the current ones.
        l.push("clangd", b, 7, 3, "again in b");
        let texts =
            |it: &mut dyn Iterator<Item = &Said>| it.map(|s| s.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&mut l.current("clangd", b)), ["again in b"]);
        assert_eq!(texts(&mut l.of("clangd", b)), ["in b", "again in b"]);
        let b_now = l.render(&Shown::Instance("clangd".into(), b.into()));
        assert!(b_now.contains("  #1 info    in b\n"), "{b_now}");
        assert!(b_now.contains("  #2 info    again in b\n"), "{b_now}");
    }

    #[test]
    fn every_stream_together_is_capped() {
        let mut l = ServerLogs::default();
        // A server per crate of a registry, each saying a little.
        for root in 0..(LOG_TOTAL / 100 + 10) {
            let r = PathBuf::from(format!("/registry/crate{root}"));
            for i in 0..100 {
                l.push("rust-analyzer", &r, root, 5, &i.to_string());
            }
        }
        assert_eq!(l.total(), LOG_TOTAL);
        let kept: usize = l.instances().map(|(s, r)| l.of(s, r).count()).sum();
        assert_eq!(kept, LOG_TOTAL);
        // The oldest went first: the first crates' streams are gone.
        assert_eq!(
            l.of("rust-analyzer", Path::new("/registry/crate0")).count(),
            0
        );
        assert!(
            !l.instances()
                .any(|(_, r)| r == Path::new("/registry/crate0")),
            "an emptied stream is let go of"
        );
        let last = PathBuf::from(format!("/registry/crate{}", LOG_TOTAL / 100 + 9));
        assert_eq!(l.of("rust-analyzer", &last).count(), 100);
    }
}
