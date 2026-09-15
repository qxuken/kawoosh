//! The ts system: tree-sitter on its own thread (mvp.md Decision 6). A
//! job is a buffer snapshot at a version; the answer is an `Update` for
//! the `syntax` layer at that version, which `doc` carries forward.
//!
//! The parse is incremental, and nothing copies the text: the thread
//! keeps each buffer's last snapshot and tree; a job carries the
//! journal's edits since the job before it, composed into the one edit
//! that covers them, which the tree is told (with tree-sitter's points
//! read off the two piece trees); the parser reads the new snapshot
//! chunk by chunk from its pieces, and the query reads a node's text the
//! same way, only where a predicate asks. tree-sitter reuses every node
//! the edit did not touch, and the answer covers only the span whose
//! syntax changed (the edit and `changed_ranges`), the query run over
//! that span alone; the journal carries the runs outside it. A first
//! sight of a buffer, edits the journal no longer has, or a change of
//! language parse and answer for the whole.

use std::collections::HashMap;
use std::ops::Range;
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use kawoosh_doc::{BufferId, Edit, Run, Snapshot, Update};
use tree_sitter::{InputEdit, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

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
    /// The journal's edits since the job sent before this one for the
    /// buffer, oldest first; `None` when there was none, or the journal
    /// no longer reaches back to it — the parse starts over then.
    pub edits: Option<Vec<Edit>>,
}

pub struct Answer {
    pub buffer: BufferId,
    /// One update per span whose syntax changed — a multicursor keystroke
    /// answers for each cursor's neighbourhood, not the stretch between
    /// them; a whole parse is one update for the whole.
    pub updates: Vec<Update>,
}

/// How many spans an answer may carry; past that the nearest are joined,
/// since each is a query of its own.
const SPANS_MAX: usize = 64;

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
                let mut parsed = Parsed::default();
                while let Ok(job) = job_rx.recv() {
                    // Only the newest job per buffer matters: skip ahead,
                    // the skipped job's edits carried into the next one's.
                    let mut job = job;
                    while let Ok(mut next) = job_rx.try_recv() {
                        if next.buffer == job.buffer {
                            next.edits = match (job.edits.take(), next.edits.take()) {
                                (Some(mut a), Some(b)) => {
                                    a.extend(b);
                                    Some(a)
                                }
                                _ => None,
                            };
                            job = next;
                        } else {
                            // A different buffer: handle it after this one.
                            let _ = answer_tx
                                .send(highlight(&mut parser, &mut grammars, &mut parsed, &job));
                            job = next;
                        }
                    }
                    let _ = answer_tx.send(highlight(&mut parser, &mut grammars, &mut parsed, &job));
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

/// What the thread parsed last, per buffer: the snapshot (a piece tree,
/// shared with the buffer's own blocks) and its tree, for the next
/// parse to start from.
#[derive(Default)]
struct Parsed {
    by_buffer: HashMap<BufferId, (String, text_buffer::Buffer, Tree)>,
}

/// tree-sitter's point for byte `b`: the row and the column in bytes,
/// two tree walks. Right, not approximate — the tree's node positions
/// are read nowhere here, but a wrong point can mislead the reparse.
fn point_at(text: &text_buffer::Buffer, b: usize) -> Point {
    let b = b.min(text.len());
    let row = text.line_of_offset(b);
    let line_start = text.get_line_range(row).map_or(0, |r| r.start);
    Point::new(row, b - line_start)
}

/// The one edit that covers a sequence: in the text before them all,
/// `old` is the range they touched; in the text after, it became
/// `new_end - old.start` bytes. Each edit is spelled in the text the
/// ones before it made, so the cover grows by what an edit removes past
/// its end (old bytes, at the same distance) and starts where the
/// earliest started. `None` for no edits.
fn cover(edits: &[Edit]) -> Option<(Range<usize>, usize)> {
    let mut it = edits.iter();
    let first = it.next()?;
    let (mut os, mut oe) = (first.range.start, first.range.end);
    let (mut ns, mut ne) = (first.range.start, first.range.start + first.new_len);
    for e in it {
        let (s, r, n) = (e.range.start, e.removed(), e.new_len);
        if s < ns {
            os -= ns - s;
            ns = s;
        }
        if s + r > ne {
            oe += s + r - ne;
        }
        ne = ne.max(s + r) + n - r;
    }
    Some((os..oe, ne))
}

/// Tells the tree each edit of a descending, disjoint sequence — a
/// multicursor keystroke, journaled from the last cursor to the first —
/// one at a time, so tree-sitter reuses everything between the cursors
/// where one covering edit would have it re-lex the lot. Each edit's
/// start and old end are points of the old text (nothing before an edit
/// has moved when it lands), its new end the start advanced over the
/// inserted text, read off the new text where the edit finally sits.
/// Answers the span each edit occupies in the new text, or `None` for a
/// sequence that is not of that shape (the cover is the fallback).
fn tell_each(
    tree: &mut Tree,
    edits: &[Edit],
    old_text: &text_buffer::Buffer,
    text: &text_buffer::Buffer,
) -> Option<Vec<Range<usize>>> {
    if edits.is_empty() || edits.windows(2).any(|w| w[1].range.end > w[0].range.start) {
        return None;
    }
    // Where each edit's start lands in the new text: shifted by the
    // edits before it in the text, which are the ones after it here.
    let mut below = vec![0isize; edits.len()];
    let mut acc = 0isize;
    for (i, e) in edits.iter().enumerate().rev() {
        below[i] = acc;
        acc += e.new_len as isize - e.removed() as isize;
    }
    let mut spans = Vec::with_capacity(edits.len());
    for (e, below) in edits.iter().zip(&below) {
        let final_start = (e.range.start as isize + below) as usize;
        let start = point_at(old_text, e.range.start);
        let inserted = text.collect_range(final_start..final_start + e.new_len);
        let rows = inserted.iter().filter(|&&b| b == b'\n').count();
        let new_end = match inserted.iter().rposition(|&b| b == b'\n') {
            Some(i) => Point::new(start.row + rows, e.new_len - i - 1),
            None => Point::new(start.row, start.column + e.new_len),
        };
        tree.edit(&InputEdit {
            start_byte: e.range.start,
            old_end_byte: e.range.end,
            new_end_byte: e.range.start + e.new_len,
            start_position: start,
            old_end_position: point_at(old_text, e.range.end),
            new_end_position: new_end,
        });
        spans.push(final_start..final_start + e.new_len);
    }
    Some(spans)
}

/// Sorted, overlapping and touching ones joined, and no more than
/// [`SPANS_MAX`]: past that the smallest gaps close first.
fn merge_spans(mut spans: Vec<Range<usize>>) -> Vec<Range<usize>> {
    spans.sort_by_key(|r| r.start);
    let mut out: Vec<Range<usize>> = Vec::with_capacity(spans.len());
    for r in spans {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    while out.len() > SPANS_MAX {
        let (i, _) = out
            .windows(2)
            .enumerate()
            .map(|(i, w)| (i, w[1].start - w[0].end))
            .min_by_key(|(_, gap)| *gap)
            .expect("more than one span");
        let end = out[i + 1].end;
        out[i].end = end;
        out.remove(i + 1);
    }
    out
}

fn highlight(parser: &mut Parser, grammars: &mut Grammars, parsed: &mut Parsed, job: &Job) -> Answer {
    let text = &job.snapshot.text;
    let len = text.len();
    let mut spans: Vec<Range<usize>> = std::iter::once(0..len).collect();
    let runs: Vec<Vec<Run>> = match grammars.get(&job.language) {
        Some(g) if parser.set_language(&g.language).is_ok() => {
            // The last tree, told the edits, when it is this language's
            // and the journal reached back to it.
            let old = parsed
                .by_buffer
                .remove(&job.buffer)
                .filter(|(lang, _, _)| *lang == job.language)
                .and_then(|(_, old_text, mut tree)| {
                    let edits = job.edits.as_ref()?;
                    let edited = if let Some(spans) = tell_each(&mut tree, edits, &old_text, text) {
                        spans
                    } else {
                        match cover(edits) {
                            Some((old_range, new_end)) => {
                                tree.edit(&InputEdit {
                                    start_byte: old_range.start,
                                    old_end_byte: old_range.end,
                                    new_end_byte: new_end,
                                    start_position: point_at(&old_text, old_range.start),
                                    old_end_position: point_at(&old_text, old_range.end),
                                    new_end_position: point_at(text, new_end),
                                });
                                std::iter::once(old_range.start..new_end).collect()
                            }
                            None => Vec::new(),
                        }
                    };
                    Some((tree, edited))
                });
            let mut read = |byte: usize, _: Point| text.chunk_at(byte);
            match parser.parse_with_options(&mut read, old.as_ref().map(|(t, _)| t), None) {
                Some(tree) => {
                    if let Some((old_tree, edited)) = &old {
                        // The edits themselves, and every range whose
                        // syntax the reparse changed; the runs elsewhere
                        // stand.
                        let mut all = edited.clone();
                        all.extend(
                            tree.changed_ranges(old_tree)
                                .map(|r| r.start_byte.min(len)..r.end_byte.min(len)),
                        );
                        spans = merge_spans(all);
                    }
                    let runs = spans
                        .iter()
                        .map(|span| capture_runs(g, tree.root_node(), text, span.clone()))
                        .collect();
                    parsed
                        .by_buffer
                        .insert(job.buffer, (job.language.clone(), text.clone(), tree));
                    runs
                }
                None => vec![Vec::new()],
            }
        }
        _ => {
            parsed.by_buffer.remove(&job.buffer);
            vec![Vec::new()]
        }
    };
    Answer {
        buffer: job.buffer,
        updates: spans
            .into_iter()
            .zip(runs)
            .map(|(span, runs)| Update {
                layer: SYNTAX_LAYER,
                version: job.snapshot.version,
                span,
                runs,
            })
            .collect(),
    }
}

/// Paints every capture in `span` over a byte map, outer captures first
/// so an inner one wins (tree-sitter's own precedence), then coalesces.
/// A capture reaching past the span is cut at it: the layer keeps its
/// own run for the part outside.
fn capture_runs(
    g: &Grammar,
    root: tree_sitter::Node,
    text: &text_buffer::Buffer,
    span: Range<usize>,
) -> Vec<Run> {
    let mut caps: Vec<(Range<usize>, Token)> = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(span.clone());
    // A node's text, for the predicates (`#match?` on a builtin's name):
    // asked for that node alone, off the pieces.
    let mut node_text = |n: tree_sitter::Node| std::iter::once(text.collect_range(n.byte_range()));
    let mut it = cursor.captures(&g.query, root, &mut node_text);
    while let Some((m, i)) = it.next() {
        let c = m.captures[*i];
        let Some(Some(tok)) = g.classes.get(c.index as usize) else {
            continue;
        };
        let r = c.node.byte_range();
        let r = r.start.max(span.start)..r.end.min(span.end);
        if r.start < r.end {
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
    let base = span.start;
    let mut paint = vec![0u8; span.len()];
    for (r, tok) in caps {
        for p in &mut paint[r.start - base..r.end - base] {
            *p = tok as u8;
        }
    }
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..=paint.len() {
        if i == paint.len() || paint[i] != paint[start] {
            if paint[start] != 0 {
                runs.push(Run {
                    range: base + start..base + i,
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
            edits: None,
        };
        let a = highlight(&mut parser, &mut g, &mut Parsed::default(), &job).update();
        let tok_at = |o: usize| {
            a.runs
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
                edits: None,
            };
            let a = highlight(&mut parser, &mut g, &mut Parsed::default(), &job).update();
            assert!(!a.runs.is_empty(), "{lang}: no runs");
            for (needle, tok) in *want {
                let o = src.find(needle).unwrap();
                let got = a
                    .runs
                    .iter()
                    .find(|r| r.range.contains(&o))
                    .map(|r| Token::from_style(r.style));
                assert_eq!(got, Some(*tok), "{lang}: {needle:?}");
            }
        }
    }

    impl Answer {
        /// The one update of a whole-parse answer.
        fn update(mut self) -> Update {
            assert_eq!(self.updates.len(), 1, "one span");
            self.updates.remove(0)
        }
    }

    /// Joins adjacent runs of one style, which is how a row reads them.
    fn joined(runs: &[Run]) -> Vec<(Range<usize>, u32)> {
        let mut out: Vec<(Range<usize>, u32)> = Vec::new();
        for r in runs {
            match out.last_mut() {
                Some((last, style)) if *style == r.style && last.end == r.range.start => {
                    last.end = r.range.end;
                }
                _ => out.push((r.range.clone(), r.style)),
            }
        }
        out
    }

    /// The second parse of a buffer is incremental — its answer spans the
    /// edit and what it changed, not the document — and, applied through
    /// the journal over the first, the layer reads as a whole parse of
    /// the new text would.
    #[test]
    fn a_reparse_answers_for_the_changed_span_alone() {
        let mut g = Grammars::load();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let body: String = (0..200)
            .map(|i| format!("fn f{i}(x: u32) -> u32 {{ x + {i} }} // c{i}\n"))
            .collect();
        let mut buf = Buffer::new("t", &body);
        buf.language = "rust".into();
        // Jobs the way the app sends them: each with the journal's edits
        // since the version the one before it read.
        let mut sent: Option<kawoosh_doc::Version> = None;
        let mut job = |buf: &Buffer| {
            let edits = sent
                .and_then(|v| buf.journal().edits_since(v).ok())
                .map(|it| it.cloned().collect());
            sent = Some(buf.version());
            Job {
                buffer: BufferId::default(),
                language: "rust".into(),
                snapshot: buf.snapshot(),
                edits,
            }
        };
        let whole_job = |buf: &Buffer| Job {
            buffer: BufferId::default(),
            language: "rust".into(),
            snapshot: buf.snapshot(),
            edits: None,
        };
        let first = highlight(&mut parser, &mut g, &mut parsed, &job(&buf)).update();
        assert_eq!(first.span, 0..body.len());
        buf.apply(first).unwrap();
        // Rename a function in the middle: a string that was a name.
        let at = body.find("fn f100(").unwrap() + 3;
        buf.replace(at..at + 4, "renamed");
        let second = highlight(&mut parser, &mut g, &mut parsed, &job(&buf));
        let second = second.update();
        let span = second.span.clone();
        assert!(span.start >= at.saturating_sub(64), "{span:?} for an edit at {at}");
        assert!(span.end <= at + 64, "{span:?} for an edit at {at}");
        assert!(second.runs.iter().all(|r| span.start <= r.range.start && r.range.end <= span.end));
        buf.apply(second).unwrap();
        let whole = highlight(&mut parser, &mut g, &mut Parsed::default(), &whole_job(&buf)).update();
        assert_eq!(whole.span, 0..buf.len());
        assert_eq!(joined(buf.runs(SYNTAX_LAYER, 0..buf.len())), joined(&whole.runs));
        let renamed = buf.text().find("renamed").unwrap();
        assert_eq!(
            buf.runs(SYNTAX_LAYER, renamed..renamed + 1).first().map(|r| Token::from_style(r.style)),
            Some(Token::Function)
        );
        // A multicursor keystroke: three insertions at
        // once, told to the tree one by one.
        {
            let at = |n: usize, buf: &Buffer| buf.text().find(&format!("fn f{n}(")).unwrap() + 3;
            let (a, b, c) = (at(10, &buf), at(20, &buf), at(30, &buf));
            buf.replace_many(&[(a..a, "x_"), (b..b, "y_"), (c..c, "z_")]);
            let inc = highlight(&mut parser, &mut g, &mut parsed, &job(&buf));
            assert!(inc.updates.len() >= 3, "a span per cursor: {:?}", inc.updates.iter().map(|u| u.span.clone()).collect::<Vec<_>>());
            assert!(inc.updates.iter().all(|u| u.span.len() < buf.len() / 4));
            for u in inc.updates {
                buf.apply(u).unwrap();
            }
            let whole = highlight(&mut parser, &mut g, &mut Parsed::default(), &whole_job(&buf));
            assert_eq!(joined(buf.runs(SYNTAX_LAYER, 0..buf.len())), joined(&whole.update().runs));
            let renamed = buf.text().find("fn y_f20(").unwrap() + 3;
            assert_eq!(
                buf.runs(SYNTAX_LAYER, renamed..renamed + 1).first().map(|r| Token::from_style(r.style)),
                Some(Token::Function)
            );
        }
        // Edits whose syntax reaches past them — a `/*` that swallows
        // the rest of the file, then its `*/` half way down, then both
        // deleted — each still read as a whole parse would.
        let open = buf.text().find("fn f50(").unwrap();
        buf.replace(open..open, "/*");
        let close = buf.text().find("fn f150(").unwrap();
        buf.replace(close..close, "*/");
        let edits = [open..open + 2, close..close + 2];
        for step in 0..3 {
            if step == 2 {
                for r in edits.iter().rev() {
                    buf.replace(r.clone(), "");
                }
            }
            let inc = highlight(&mut parser, &mut g, &mut parsed, &job(&buf));
            for u in inc.updates {
                buf.apply(u).unwrap();
            }
            let whole = highlight(&mut parser, &mut g, &mut Parsed::default(), &whole_job(&buf));
            assert_eq!(
                joined(buf.runs(SYNTAX_LAYER, 0..buf.len())),
                joined(&whole.update().runs),
                "step {step}"
            );
            let f60 = buf.text().find("fn f60(").unwrap();
            let tok = buf
                .runs(SYNTAX_LAYER, f60..f60 + 1)
                .first()
                .map(|r| Token::from_style(r.style));
            let want = if step < 2 { Token::Comment } else { Token::Keyword };
            assert_eq!(tok, Some(want), "step {step}: f60 in the block comment until it goes");
        }
    }

    /// The cover of a sequence of edits is the one edit that replaces
    /// what they touched, in the coordinates before and after them all.
    #[test]
    fn edits_compose_into_one_cover() {
        let e = |s: usize, r: usize, n: usize| Edit {
            range: s..s + r,
            new_len: n,
        };
        assert_eq!(cover(&[]), None);
        assert_eq!(cover(&[e(3, 2, 5)]), Some((3..5, 8)));
        // "abcdef" → "aXdef" → "aXdYYf" → "QdYYf": old "abcde" became
        // "QdYY".
        assert_eq!(cover(&[e(1, 2, 1), e(3, 1, 2), e(0, 2, 1)]), Some((0..5, 4)));
        // Typing three chars in a row: old 3..3 became 3..6.
        assert_eq!(cover(&[e(3, 0, 1), e(4, 0, 1), e(5, 0, 1)]), Some((3..3, 6)));
        // Then backspacing four: one char before the typing went too.
        assert_eq!(
            cover(&[e(3, 0, 1), e(4, 0, 1), e(5, 0, 1), e(5, 1, 0), e(4, 1, 0), e(3, 1, 0), e(2, 1, 0)]),
            Some((2..3, 2))
        );
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
            edits: None,
        };
        assert!(
            highlight(&mut parser, &mut g, &mut Parsed::default(), &job)
                .update()
                .runs
                .is_empty()
        );
    }
}
