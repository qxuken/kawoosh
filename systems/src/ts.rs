//! The ts system: tree-sitter on its own thread (mvp.md Decision 6). A
//! job is a buffer snapshot at a version; the answer is an `Update` for
//! the `syntax` layer at that version, which `doc` carries forward.
//! Whole-document parses for the MVP — a source file parses in
//! milliseconds, and the journal makes a late answer harmless.

use std::ops::Range;
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use kawoosh_doc::{BufferId, Run, Snapshot, Update};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

use crate::WakeHandle;

pub const SYNTAX_LAYER: &str = "syntax";

/// Token classes a run's `style` names — the app maps them to colours
/// (tokens from Lua, kui.md D7). Fixed so a theme is a table, not code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Token {
    Plain = 0,
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Variable,
    Property,
    Operator,
    Punctuation,
    Attribute,
    Constant,
    Macro,
    Label,
    Constructor,
}

impl Token {
    pub const ALL: &[Token] = &[
        Token::Plain,
        Token::Keyword,
        Token::Function,
        Token::Type,
        Token::String,
        Token::Number,
        Token::Comment,
        Token::Variable,
        Token::Property,
        Token::Operator,
        Token::Punctuation,
        Token::Attribute,
        Token::Constant,
        Token::Macro,
        Token::Label,
        Token::Constructor,
    ];

    pub fn from_style(style: u32) -> Token {
        Token::ALL
            .get(style as usize)
            .copied()
            .unwrap_or(Token::Plain)
    }

    pub fn name(self) -> &'static str {
        match self {
            Token::Plain => "plain",
            Token::Keyword => "keyword",
            Token::Function => "function",
            Token::Type => "type",
            Token::String => "string",
            Token::Number => "number",
            Token::Comment => "comment",
            Token::Variable => "variable",
            Token::Property => "property",
            Token::Operator => "operator",
            Token::Punctuation => "punctuation",
            Token::Attribute => "attribute",
            Token::Constant => "constant",
            Token::Macro => "macro",
            Token::Label => "label",
            Token::Constructor => "constructor",
        }
    }

    /// A tree-sitter capture name to a class: `keyword.control` is a
    /// keyword, `function.method` a function, `punctuation.bracket`
    /// punctuation.
    pub fn from_capture(name: &str) -> Option<Token> {
        let head = name.split('.').next().unwrap_or(name);
        Some(match head {
            "keyword" => Token::Keyword,
            "function" | "method" => Token::Function,
            "type" => Token::Type,
            "string" | "character" | "escape" => Token::String,
            "number" | "float" | "boolean" => Token::Number,
            "comment" => Token::Comment,
            "variable" | "parameter" => Token::Variable,
            "property" | "field" => Token::Property,
            "operator" => Token::Operator,
            "punctuation" => Token::Punctuation,
            "attribute" => Token::Attribute,
            "constant" => Token::Constant,
            "macro" => Token::Macro,
            "label" => Token::Label,
            "constructor" => Token::Constructor,
            _ => return None,
        })
    }
}

pub struct Job {
    pub buffer: BufferId,
    pub language: String,
    pub snapshot: Snapshot,
}

pub struct Answer {
    pub buffer: BufferId,
    pub update: Update,
}

pub struct Ts {
    jobs: Sender<Job>,
    pub answers: Receiver<Answer>,
}

impl Ts {
    /// Starts the parser thread. Languages it does not know answer with
    /// an empty layer, so a file that lost its grammar loses its colours
    /// rather than keeping stale ones.
    pub fn spawn(wake: WakeHandle) -> Self {
        let (jobs, job_rx) = unbounded::<Job>();
        let (answer_tx, answers) = unbounded::<Answer>();
        thread::Builder::new()
            .name("ts".into())
            .spawn(move || {
                let mut parser = Parser::new();
                let mut grammars = Grammars::load();
                while let Ok(job) = job_rx.recv() {
                    // Only the newest job per buffer matters: skip ahead.
                    let mut job = job;
                    while let Ok(next) = job_rx.try_recv() {
                        if next.buffer == job.buffer {
                            job = next;
                        } else {
                            // A different buffer: handle it after this one.
                            let _ = answer_tx.send(highlight(&mut parser, &mut grammars, &job));
                            job = next;
                        }
                    }
                    let _ = answer_tx.send(highlight(&mut parser, &mut grammars, &job));
                    wake.wake();
                }
            })
            .expect("spawning the ts thread");
        Self { jobs, answers }
    }

    pub fn submit(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    pub fn drain(&self) -> Vec<Answer> {
        self.answers.try_iter().collect()
    }

    pub fn supports(language: &str) -> bool {
        matches!(language, "rust" | "toml" | "css" | "javascript" | "go")
    }
}

struct Grammar {
    language: tree_sitter::Language,
    query: Query,
    /// Capture index → token class.
    classes: Vec<Option<Token>>,
}

/// The grammars, each loaded with its own crate's highlight query on the
/// thread's first job; `Ts::supports` is the same list.
struct Grammars {
    rust: Option<Grammar>,
    toml: Option<Grammar>,
    css: Option<Grammar>,
    javascript: Option<Grammar>,
    go: Option<Grammar>,
}

impl Grammars {
    fn load() -> Self {
        Self {
            rust: Grammar::new(
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY,
            ),
            // toml-ng captures every bare key as `@type` (the `@property`
            // is on the pair around it, so the key wins); a key is a
            // property here, as in every other table-shaped language.
            toml: Grammar::new(
                tree_sitter_toml_ng::LANGUAGE.into(),
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            )
            .map(|g| g.recapture("type", Token::Property)),
            css: Grammar::new(
                tree_sitter_css::LANGUAGE.into(),
                tree_sitter_css::HIGHLIGHTS_QUERY,
            ),
            javascript: Grammar::new(
                tree_sitter_javascript::LANGUAGE.into(),
                tree_sitter_javascript::HIGHLIGHT_QUERY,
            ),
            go: Grammar::new(
                tree_sitter_go::LANGUAGE.into(),
                tree_sitter_go::HIGHLIGHTS_QUERY,
            ),
        }
    }

    fn get(&mut self, language: &str) -> Option<&Grammar> {
        match language {
            "rust" => self.rust.as_ref(),
            "toml" => self.toml.as_ref(),
            "css" => self.css.as_ref(),
            "javascript" => self.javascript.as_ref(),
            "go" => self.go.as_ref(),
            _ => None,
        }
    }
}

impl Grammar {
    fn new(language: tree_sitter::Language, highlights: &str) -> Option<Self> {
        let query = match Query::new(&language, highlights) {
            Ok(q) => q,
            Err(e) => {
                log::error!("highlight query: {e}");
                return None;
            }
        };
        let classes = query
            .capture_names()
            .iter()
            .map(|n| Token::from_capture(n))
            .collect();
        Some(Self {
            language,
            query,
            classes,
        })
    }
}

impl Grammar {
    /// Reads capture `name` (its head) as `token` instead of the default.
    fn recapture(mut self, name: &str, token: Token) -> Self {
        for (i, n) in self.query.capture_names().iter().enumerate() {
            if n.split('.').next() == Some(name) {
                self.classes[i] = Some(token);
            }
        }
        self
    }
}

fn highlight(parser: &mut Parser, grammars: &mut Grammars, job: &Job) -> Answer {
    let text = job.snapshot.text.collect();
    let runs = match grammars.get(&job.language) {
        Some(g) if parser.set_language(&g.language).is_ok() => match parser.parse(&text, None) {
            Some(tree) => capture_runs(g, tree.root_node(), &text),
            None => Vec::new(),
        },
        _ => Vec::new(),
    };
    Answer {
        buffer: job.buffer,
        update: Update {
            layer: SYNTAX_LAYER,
            version: job.snapshot.version,
            span: 0..text.len(),
            runs,
        },
    }
}

/// Paints every capture over a byte map, outer captures first so an
/// inner one wins (tree-sitter's own precedence), then coalesces.
fn capture_runs(g: &Grammar, root: tree_sitter::Node, text: &[u8]) -> Vec<Run> {
    let mut caps: Vec<(Range<usize>, Token)> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut it = cursor.captures(&g.query, root, text);
    while let Some((m, i)) = it.next() {
        let c = m.captures[*i];
        let Some(Some(tok)) = g.classes.get(c.index as usize) else {
            continue;
        };
        let r = c.node.byte_range();
        if !r.is_empty() {
            caps.push((r, *tok));
        }
    }
    caps.sort_by(|a, b| {
        (a.0.start, std::cmp::Reverse(a.0.end)).cmp(&(b.0.start, std::cmp::Reverse(b.0.end)))
    });
    // One node under two patterns is the later pattern's, as
    // tree-sitter's own highlighter reads it (javascript lists
    // `(identifier) @variable` first and the function patterns after) —
    // unless the later is the bare `@variable`, the class every
    // identifier has, over a specific one (go lists its `(identifier)
    // @variable` last). The sort is stable, so equal ranges are in
    // capture order: keep the last that is not a plain variable.
    caps.dedup_by(|later, first| {
        if later.0 != first.0 {
            return false;
        }
        if later.1 != Token::Variable || first.1 == Token::Variable {
            first.1 = later.1;
        }
        true
    });
    let mut paint = vec![0u8; text.len()];
    for (r, tok) in caps {
        let end = r.end.min(paint.len());
        for p in &mut paint[r.start.min(end)..end] {
            *p = tok as u8;
        }
    }
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..=paint.len() {
        if i == paint.len() || paint[i] != paint[start] {
            if paint[start] != 0 {
                runs.push(Run {
                    range: start..i,
                    style: paint[start] as u32,
                    tag: 0,
                });
            }
            start = i;
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_doc::Buffer;

    #[test]
    fn rust_gets_keywords_strings_and_comments() {
        let mut g = Grammars::load();
        let mut parser = Parser::new();
        let src = "// hi\nfn main() { let s = \"x\"; }\n";
        let buf = Buffer::new("t", src);
        let job = Job {
            buffer: BufferId::default(),
            language: "rust".into(),
            snapshot: buf.snapshot(),
        };
        let a = highlight(&mut parser, &mut g, &job);
        let tok_at = |o: usize| {
            a.update
                .runs
                .iter()
                .find(|r| r.range.contains(&o))
                .map(|r| Token::from_style(r.style))
        };
        assert_eq!(tok_at(1), Some(Token::Comment));
        assert_eq!(tok_at(src.find("fn").unwrap()), Some(Token::Keyword));
        assert_eq!(tok_at(src.find("main").unwrap()), Some(Token::Function));
        assert_eq!(tok_at(src.find('"').unwrap() + 1), Some(Token::String));
        assert_eq!(tok_at(src.find("let").unwrap()), Some(Token::Keyword));
    }

    /// Each grammar's query compiles and lands the classes a theme
    /// colours: a keyword, a string, a comment, and one of its own.
    #[test]
    fn toml_css_javascript_and_go_highlight() {
        let mut g = Grammars::load();
        let mut parser = Parser::new();
        type Case = (&'static str, &'static str, &'static [(&'static str, Token)]);
        let cases: &[Case] = &[
            (
                "toml",
                "# c\n[pkg]\nname = \"x\"\nn = 1\n",
                &[("# c", Token::Comment), ("name", Token::Property), ("\"x\"", Token::String), ("1", Token::Number)],
            ),
            (
                "css",
                "/* c */\n.a { color: red; }\n",
                &[("/* c */", Token::Comment), ("color", Token::Property), ("{", Token::Punctuation)],
            ),
            (
                "javascript",
                "// c\nfunction f(a) { return \"s\" + 1; }\n",
                &[("// c", Token::Comment), ("function", Token::Keyword), ("f(", Token::Function), ("\"s\"", Token::String), ("1;", Token::Number)],
            ),
            (
                "go",
                "// c\npackage main\nfunc main() { s := \"x\" }\n",
                &[("// c", Token::Comment), ("func", Token::Keyword), ("main()", Token::Function), ("\"x\"", Token::String)],
            ),
        ];
        for (lang, src, want) in cases {
            let buf = Buffer::new("t", src);
            let job = Job {
                buffer: BufferId::default(),
                language: (*lang).into(),
                snapshot: buf.snapshot(),
            };
            let a = highlight(&mut parser, &mut g, &job);
            assert!(!a.update.runs.is_empty(), "{lang}: no runs");
            for (needle, tok) in *want {
                let o = src.find(needle).unwrap();
                let got = a
                    .update
                    .runs
                    .iter()
                    .find(|r| r.range.contains(&o))
                    .map(|r| Token::from_style(r.style));
                assert_eq!(got, Some(*tok), "{lang}: {needle:?}");
            }
        }
    }

    #[test]
    fn unknown_language_answers_empty() {
        let mut g = Grammars::load();
        let mut parser = Parser::new();
        let buf = Buffer::new("t", "whatever");
        let job = Job {
            buffer: BufferId::default(),
            language: "brainfuck".into(),
            snapshot: buf.snapshot(),
        };
        assert!(highlight(&mut parser, &mut g, &job).update.runs.is_empty());
    }
}
