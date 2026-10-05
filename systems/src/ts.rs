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
//! syntax changed (the edit and `changed_ranges`, each widened to the
//! edited node's parent on its lines), the query run over that span
//! alone; the journal carries the runs outside it. A first sight of a buffer, edits the journal no longer
//! has, or a change of language parse and answer for the whole.
//!
//! The languages are a `kawoosh_languages::Registry`'s (kui.md Decision
//! 13): the builtins, and what the shell adds with [`Ts::add_language`]
//! — a language of its own, its grammar from a shared library, loaded
//! on the shell's side so its errors are the user's to see, and handed
//! over here. A builtin grammar is loaded the first time a job names
//! it. A grammar with an
//! injections query has other languages inside it — a fenced code
//! block, a JSDoc comment, a regex literal — and each such node
//! reaching into an answered span is parsed over that node alone by
//! its own grammar, its captures painted over the host's, its
//! injections under it in turn; those parses start over every time,
//! since the spans are small and the tree kept is the host's.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use kawoosh_doc::{BufferId, Edit, Run, Snapshot, Update};
pub use kawoosh_languages::Token;
use kawoosh_languages::{Grammar, LanguageDef, Registry, Structure};
use tree_sitter::{InputEdit, Node, Parser, Point, QueryCursor, StreamingIterator, Tree};

use crate::WakeHandle;

pub const SYNTAX_LAYER: &str = "syntax";
/// The structure layer: which bytes are which block (a grammar's
/// structure query — markdown's fences, tables, headings by level), a
/// run's style a [`kawoosh_languages::Block`].
pub const STRUCT_LAYER: &str = "structure";

/// How deep injections nest: markdown's inline in its blocks, and one
/// more under that.
pub const INJECTION_DEPTH: usize = 3;

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
    /// The version the answer is for — the job's snapshot's.
    pub version: kawoosh_doc::Version,
    /// One update per span whose syntax changed — a multicursor keystroke
    /// answers for each cursor's neighbourhood, not the stretch between
    /// them; a whole parse is one update for the whole.
    pub updates: Vec<Update>,
    /// The tree the runs were read off, for the shell's syntax inspector:
    /// a handle over nodes shared with the thread's own copy (a clone is
    /// a count, not a walk). `None` for a language without a grammar.
    pub tree: Option<Tree>,
    /// The text the tree was parsed from and the grammar that read it,
    /// for the shell's indenter to bring the tree up to the buffer's
    /// text on the spot (docs/design/indent.md Decision 4) — its indent
    /// and its text objects (docs/design/nodes.md Decision 9); only for
    /// a grammar with an indent or a text-object query that reads the
    /// text as it is.
    pub parse: Option<Parse>,
    /// What the parse and the queries took on the thread — the devtools'
    /// reading of a system the frame never waits on.
    pub elapsed: std::time::Duration,
}

/// An answer's tree's text and grammar ([`Answer::parse`]).
#[derive(Clone)]
pub struct Parse {
    pub text: text_buffer::Buffer,
    pub grammar: Arc<Grammar>,
}

/// A text that is no buffer's, highlighted once: a picker's preview,
/// a plugin's pane. Parsed whole, nothing kept.
pub struct TextJob {
    pub token: u64,
    pub language: String,
    pub text: String,
}

/// The runs of a [`TextJob`], byte ranges over its text; empty for a
/// language without a grammar.
pub struct TextAnswer {
    pub token: u64,
    pub runs: Vec<Run>,
}

/// A buffer's outline asked for (docs/design/marks.md Decision 1):
/// the grammar's outline query run over the tree the thread keeps for
/// it, answered on `outline_answers`.
pub struct OutlineJob {
    pub token: u64,
    pub buffer: BufferId,
}

/// One definition of an outline, in the file's order — a definition
/// before the ones inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outlined {
    /// The name's first line, trimmed.
    pub name: String,
    /// The capture's `@definition.KIND`: `function`, `impl`, `h2`.
    pub kind: String,
    /// The `@detail` capture's text: an `impl`'s trait, a method's
    /// receiver.
    pub detail: Option<String>,
    /// The whole definition, bytes of the text as parsed.
    pub range: Range<usize>,
    /// Where the name starts: its line and its column in characters,
    /// both from 0; and the definition's last line.
    pub line: u32,
    pub character: u32,
    pub end_line: u32,
    /// How many definitions it lies inside.
    pub depth: u32,
}

pub struct OutlineAnswer {
    pub token: u64,
    pub buffer: BufferId,
    pub result: Result<Vec<Outlined>, String>,
}

/// The most definitions an outline answers: a JSON file's every key is
/// one, and a list past this is no outline to read.
pub const OUTLINE_MAX: usize = 20_000;

/// How many spans an answer may carry; past that the nearest are joined,
/// since each is a query of its own.
const SPANS_MAX: usize = 64;

/// The most edits the tree is told one at a time (`tell_each`); past it,
/// the one edit that covers them. Each is a walk down the tree — 11µs
/// on a minified bundle, where a format's hundred thousand told took a
/// second, and the cover's reparse of the lot a fifth of one.
const TELL_EACH_MAX: usize = 4096;

/// What the thread is told.
enum Cmd {
    Job(Job),
    Text(TextJob),
    Outline(OutlineJob),
    /// A language registered: the registry entry, and its grammar when
    /// the shell loaded one — `None` for a language of files alone,
    /// or a builtin, which the thread loads itself.
    Language {
        def: Box<LanguageDef>,
        grammar: Option<Arc<Grammar>>,
    },
}

pub struct Ts {
    cmds: Sender<Cmd>,
    pub answers: Receiver<Answer>,
    pub text_answers: Receiver<TextAnswer>,
    pub outline_answers: Receiver<OutlineAnswer>,
}

impl Ts {
    /// Starts the parser thread. Languages it does not know answer with
    /// an empty layer, so a file that lost its grammar loses its colours
    /// rather than keeping stale ones.
    pub fn spawn(wake: WakeHandle) -> Self {
        let (cmds, cmd_rx) = unbounded::<Cmd>();
        let (answer_tx, answers) = unbounded::<Answer>();
        let (text_tx, text_answers) = unbounded::<TextAnswer>();
        let (outline_tx, outline_answers) = unbounded::<OutlineAnswer>();
        thread::Builder::new()
            .name("ts".into())
            .spawn(move || {
                let mut parser = Parser::new();
                let mut grammars = Grammars::default();
                let mut parsed = Parsed::default();
                while let Ok(cmd) = cmd_rx.recv() {
                    let mut job = match cmd {
                        Cmd::Job(job) => job,
                        Cmd::Language { def, grammar } => {
                            grammars.add(*def, grammar, &mut parsed);
                            continue;
                        }
                        Cmd::Text(t) => {
                            let _ = text_tx.send(highlight_text(&mut parser, &mut grammars, &t));
                            wake.wake();
                            continue;
                        }
                        Cmd::Outline(o) => {
                            let _ = outline_tx.send(outline_of(&parsed, &mut grammars, &o));
                            wake.wake();
                            continue;
                        }
                    };
                    // An outline asked while jobs wait is answered after
                    // them, so it reads the tree of the text as it is.
                    let mut outlines = Vec::new();
                    // Only the newest job per buffer matters: skip ahead,
                    // the skipped job's edits carried into the next one's.
                    while let Ok(next) = cmd_rx.try_recv() {
                        let mut next = match next {
                            Cmd::Job(job) => job,
                            Cmd::Language { def, grammar } => {
                                grammars.add(*def, grammar, &mut parsed);
                                continue;
                            }
                            Cmd::Text(t) => {
                                let _ =
                                    text_tx.send(highlight_text(&mut parser, &mut grammars, &t));
                                continue;
                            }
                            Cmd::Outline(o) => {
                                outlines.push(o);
                                continue;
                            }
                        };
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
                            let _ = answer_tx.send(highlight(
                                &mut parser,
                                &mut grammars,
                                &mut parsed,
                                &job,
                            ));
                            job = next;
                        }
                    }
                    let _ =
                        answer_tx.send(highlight(&mut parser, &mut grammars, &mut parsed, &job));
                    for o in outlines {
                        let _ = outline_tx.send(outline_of(&parsed, &mut grammars, &o));
                    }
                    wake.wake();
                }
            })
            .expect("spawning the ts thread");
        Self {
            cmds,
            answers,
            text_answers,
            outline_answers,
        }
    }

    /// A buffer's outline, answered on `outline_answers` after every
    /// job sent before it.
    pub fn outline(&self, job: OutlineJob) {
        let _ = self.cmds.send(Cmd::Outline(job));
    }

    pub fn submit(&self, job: Job) {
        let _ = self.cmds.send(Cmd::Job(job));
    }

    /// A text of its own to highlight, answered on `text_answers`.
    pub fn submit_text(&self, job: TextJob) {
        let _ = self.cmds.send(Cmd::Text(job));
    }

    /// Tells the thread a language: its registry entry, and the grammar
    /// the shell loaded for it (the same `LanguageDef` the shell's own
    /// registry took, so both name the same files). A buffer of the
    /// language parses whole at its next job.
    pub fn add_language(&self, def: LanguageDef, grammar: Option<Grammar>) {
        let grammar = grammar.map(Arc::new);
        let _ = self.cmds.send(Cmd::Language {
            def: Box::new(def),
            grammar,
        });
    }

    pub fn drain(&self) -> Vec<Answer> {
        self.answers.try_iter().collect()
    }
}

/// The thread's registry and the grammars it has loaded, by their
/// language's name (an alias resolves to it): a builtin's on its first
/// job — a query compiles in a moment, but there are two dozen — a
/// registered one as it came; a language without one, or whose load
/// failed, remembered as `None` so it is not asked again. Shared
/// handles, so a host grammar and the one it injects are held at once.
#[derive(Default)]
struct Grammars {
    registry: Registry,
    loaded: HashMap<String, Option<Arc<Grammar>>>,
}

impl Grammars {
    fn get(&mut self, language: &str) -> Option<Arc<Grammar>> {
        let def = self.registry.by_name(language)?;
        if let Some(g) = self.loaded.get(&def.name) {
            return g.clone();
        }
        let name = def.name.clone();
        let g = match def.load() {
            Ok(g) => g,
            Err(e) => {
                log::error!("{name}: {e}");
                None
            }
        }
        .map(Arc::new);
        self.loaded.insert(name, g.clone());
        g
    }

    /// A language registered: in the registry, its grammar as given
    /// (or loaded at the next job, for one without), and every tree
    /// kept under its name dropped, so its buffers parse whole.
    fn add(&mut self, def: LanguageDef, grammar: Option<Arc<Grammar>>, parsed: &mut Parsed) {
        let name = def.name.clone();
        self.registry.add(def);
        match grammar {
            Some(g) => {
                self.loaded.insert(name.clone(), Some(g));
            }
            None => {
                self.loaded.remove(&name);
            }
        }
        parsed.by_buffer.retain(|_, (lang, _, _)| *lang != name);
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
/// sequence that is not of that shape, or longer than [`TELL_EACH_MAX`]
/// (the cover is the fallback).
fn tell_each(
    tree: &mut Tree,
    edits: &[Edit],
    old_text: &text_buffer::Buffer,
    text: &text_buffer::Buffer,
) -> Option<Vec<Range<usize>>> {
    if edits.is_empty()
        || edits.len() > TELL_EACH_MAX
        || edits.windows(2).any(|w| w[1].range.end > w[0].range.start)
    {
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

/// Tells `tree` (parsed from `old_text`) the `edits` that made `text`,
/// one at a time when they are a multicursor keystroke's
/// ([`tell_each`]), else as the one edit that covers them; answers the
/// spans they occupy in `text`.
fn tell(
    tree: &mut Tree,
    edits: &[Edit],
    old_text: &text_buffer::Buffer,
    text: &text_buffer::Buffer,
) -> Vec<Range<usize>> {
    if let Some(spans) = tell_each(tree, edits, old_text, text) {
        return spans;
    }
    match cover(edits) {
        Some((old_range, new_end)) => {
            tree.edit(&InputEdit {
                start_byte: old_range.start,
                old_end_byte: old_range.end,
                new_end_byte: new_end,
                start_position: point_at(old_text, old_range.start),
                old_end_position: point_at(old_text, old_range.end),
                new_end_position: point_at(text, new_end),
            });
            std::iter::once(old_range.start..new_end).collect()
        }
        None => Vec::new(),
    }
}

/// `text` parsed on the caller's thread by `language`: from `old` — a
/// tree of `old`'s text, told the `edits` that made this one — when
/// given, else whole. The indenter's, for a line break typed before the
/// thread answered (docs/design/indent.md Decision 4).
pub fn reparse(
    parser: &mut Parser,
    language: &tree_sitter::Language,
    text: &text_buffer::Buffer,
    old: Option<(Tree, &text_buffer::Buffer, &[Edit])>,
) -> Option<Tree> {
    parser.set_language(language).ok()?;
    let old = old.map(|(mut tree, old_text, edits)| {
        tell(&mut tree, edits, old_text, text);
        tree
    });
    let mut read = |byte: usize, _: Point| text.chunk_at(byte);
    parser.parse_with_options(&mut read, old.as_ref(), None)
}

/// Sorted, overlapping and touching ones joined, and no more than
/// [`SPANS_MAX`]: past that the smallest gaps close first. Closing a gap
/// leaves the others as they were, so the ones to close are chosen at
/// once — a format's hundred thousand spans closed one at a time, each
/// a walk over the rest.
fn merge_spans(mut spans: Vec<Range<usize>>) -> Vec<Range<usize>> {
    spans.sort_by_key(|r| r.start);
    let mut out: Vec<Range<usize>> = Vec::with_capacity(spans.len());
    for r in spans {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    if out.len() <= SPANS_MAX {
        return out;
    }
    // The gaps after each span, smallest (then leftmost) first.
    let mut gaps: Vec<(usize, usize)> = out
        .windows(2)
        .enumerate()
        .map(|(i, w)| (w[1].start - w[0].end, i))
        .collect();
    gaps.sort_unstable();
    let mut closed = vec![false; out.len()];
    for &(_, i) in &gaps[..out.len() - SPANS_MAX] {
        closed[i] = true;
    }
    let mut joined: Vec<Range<usize>> = Vec::with_capacity(SPANS_MAX);
    let mut open = true;
    for (r, closed) in out.into_iter().zip(closed) {
        if open {
            joined.push(r);
        } else {
            joined.last_mut().expect("a span before").end = r.end;
        }
        open = !closed;
    }
    joined
}

fn highlight(
    parser: &mut Parser,
    grammars: &mut Grammars,
    parsed: &mut Parsed,
    job: &Job,
) -> Answer {
    let started = std::time::Instant::now();
    let text = &job.snapshot.text;
    let len = text.len();
    let mut spans: Vec<Range<usize>> = std::iter::once(0..len).collect();
    // The structure layer's runs beside the syntax's, span for span;
    // none for a grammar without a structure query, so what an earlier
    // language painted goes.
    let mut blocks: Vec<Vec<Run>> = Vec::new();
    let mut block_spans: Option<Vec<Range<usize>>> = None;
    let mut parse = None;
    let (runs, tree): (Vec<Vec<Run>>, Option<Tree>) = match grammars.get(&job.language) {
        Some(g) if parser.set_language(&g.language).is_ok() => {
            // What the parser reads instead of lines the grammar gets
            // wrong (`Grammar::stand_ins`): a text with any is parsed
            // whole and its tree not kept, so the next parse is whole
            // too — an edit anywhere can change which lines they are.
            let stood_in = stand_in(&g, text);
            // The last tree, told the edits, when it is this language's
            // and the journal reached back to it.
            let old = parsed
                .by_buffer
                .remove(&job.buffer)
                .filter(|(lang, _, _)| *lang == job.language && stood_in.is_none())
                .and_then(|(_, old_text, mut tree)| {
                    let edits = job.edits.as_ref()?;
                    let edited = tell(&mut tree, edits, &old_text, text);
                    Some((tree, edited))
                });
            let mut read = |byte: usize, _: Point| match &stood_in {
                Some(b) => &b[byte.min(b.len())..],
                None => text.chunk_at(byte),
            };
            match parser.parse_with_options(&mut read, old.as_ref().map(|(t, _)| t), None) {
                Some(tree) => {
                    if let Some((old_tree, edited)) = &old {
                        // The edits themselves, and every range whose
                        // syntax the reparse changed, each over its
                        // neighbourhood (`neighbourhood`); the runs
                        // elsewhere stand. tree-sitter's changed ranges
                        // leave out what a capture reads off an edited
                        // node's neighbourhood: `tru` typed on to
                        // `true` is no change to them, the node's kind
                        // notwithstanding, and `x = ,` given its value
                        // turns `x` from an error's into the field's
                        // `name:` unreported.
                        let mut all = edited.clone();
                        all.extend(
                            tree.changed_ranges(old_tree)
                                .map(|r| r.start_byte.min(len)..r.end_byte.min(len)),
                        );
                        // A format's thousands joined first: each is a
                        // descent and a scan of its own, and the joined
                        // span's neighbourhood holds each one's.
                        if all.len() > SPANS_MAX {
                            all = merge_spans(all);
                        }
                        let root = tree.root_node();
                        spans = merge_spans(
                            all.into_iter()
                                .map(|s| neighbourhood(root, text, s))
                                .collect(),
                        );
                    }
                    let runs = spans
                        .iter()
                        .map(|span| {
                            capture_runs(parser, grammars, &g, tree.root_node(), text, span.clone())
                        })
                        .collect();
                    if let Some(st) = &g.structure {
                        // Over whole lines: a block's kind is its line's
                        // (a heading's level, a list's marker), and an
                        // edit of one byte of it changes all of its
                        // bytes' — `## ` typed a `#` at a time left the
                        // first `#` an h1.
                        //
                        // And over the block the edit was in before as
                        // well as the one it is in now: a line's block
                        // can be made by the line below it, and a
                        // setext heading's `-` typed on into `- [` left
                        // the paragraph above painted an h2 — its
                        // heading was the old tree's block, the new
                        // one's is the list. The old tree was told the
                        // edits, so its offsets are the new text's.
                        let lines: Vec<Range<usize>> = spans
                            .iter()
                            .map(|s| {
                                let now = enclosing_block(st, tree.root_node(), s.clone());
                                let was = old.as_ref().map_or(now.clone(), |(t, _)| {
                                    enclosing_block(st, t.root_node(), s.clone())
                                });
                                let cover = now.start.min(was.start)..now.end.max(was.end).min(len);
                                whole_lines(text, cover)
                            })
                            .collect();
                        blocks = lines
                            .iter()
                            .map(|span| structure_runs(st, tree.root_node(), text, span.clone()))
                            .collect();
                        block_spans = Some(lines);
                    }
                    let handle = tree.clone();
                    if stood_in.is_none()
                        && (g.indents.is_some()
                            || g.textobjects.is_some()
                            || g.injections.is_some())
                    {
                        parse = Some(Parse {
                            text: text.clone(),
                            grammar: g.clone(),
                        });
                    }
                    if stood_in.is_none() {
                        parsed
                            .by_buffer
                            .insert(job.buffer, (job.language.clone(), text.clone(), tree));
                    }
                    (runs, Some(handle))
                }
                None => (vec![Vec::new()], None),
            }
        }
        _ => {
            parsed.by_buffer.remove(&job.buffer);
            (vec![Vec::new()], None)
        }
    };
    blocks.resize_with(spans.len(), Vec::new);
    let block_spans = block_spans.unwrap_or_else(|| spans.clone());
    let mut updates: Vec<Update> = Vec::with_capacity(spans.len() * 2);
    for (((span, runs), blocks), block_span) in
        spans.into_iter().zip(runs).zip(blocks).zip(block_spans)
    {
        updates.push(Update {
            layer: STRUCT_LAYER,
            version: job.snapshot.version,
            span: block_span,
            runs: blocks,
        });
        updates.push(Update {
            layer: SYNTAX_LAYER,
            version: job.snapshot.version,
            span,
            runs,
        });
    }
    Answer {
        buffer: job.buffer,
        version: job.snapshot.version,
        tree,
        parse,
        elapsed: started.elapsed(),
        updates,
    }
}

/// The text as `g` reads it, when it has stand-ins for some of its
/// lines (`Grammar::stand_ins`); None when it reads the text itself.
fn stand_in(g: &Grammar, text: &text_buffer::Buffer) -> Option<Vec<u8>> {
    let f = g.stand_ins?;
    let mut bytes = text.collect();
    let subs = f(std::str::from_utf8(&bytes).ok()?);
    if subs.is_empty() {
        return None;
    }
    for (r, with) in subs {
        if r.end <= bytes.len() && r.len() == with.len() {
            bytes[r].copy_from_slice(&with);
        }
    }
    Some(bytes)
}

/// A [`TextJob`]: the text parsed whole by its language's grammar and
/// its runs read, nothing kept for later.
fn highlight_text(parser: &mut Parser, grammars: &mut Grammars, job: &TextJob) -> TextAnswer {
    let text = text_buffer::Buffer::with_text(job.text.as_bytes());
    let runs = match grammars.get(&job.language) {
        Some(g) if parser.set_language(&g.language).is_ok() => {
            let stood_in = stand_in(&g, &text);
            let mut read = |byte: usize, _: Point| match &stood_in {
                Some(b) => &b[byte.min(b.len())..],
                None => text.chunk_at(byte),
            };
            match parser.parse_with_options(&mut read, None, None) {
                Some(tree) => {
                    capture_runs(parser, grammars, &g, tree.root_node(), &text, 0..text.len())
                }
                None => Vec::new(),
            }
        }
        _ => Vec::new(),
    };
    TextAnswer {
        token: job.token,
        runs,
    }
}

/// The outline of a buffer the thread has parsed: every match of the
/// grammar's outline query, one per definition node (the first pattern
/// that took it — a method's before the function's that also matches),
/// in the file's order, each nested in the definitions whose range holds
/// it.
fn outline_of(parsed: &Parsed, grammars: &mut Grammars, job: &OutlineJob) -> OutlineAnswer {
    let answer = |result| OutlineAnswer {
        token: job.token,
        buffer: job.buffer,
        result,
    };
    let Some((language, text, tree)) = parsed.by_buffer.get(&job.buffer) else {
        return answer(Err("the buffer is not parsed".into()));
    };
    let Some(g) = grammars.get(language) else {
        return answer(Err(format!("no grammar for {language}")));
    };
    let Some(o) = &g.outline else {
        return answer(Err(format!("no outline for {language}")));
    };
    // Node id → the pattern that took it, its kind's capture, and the
    // definition, name and detail nodes.
    type Def<'t> = (usize, u32, Node<'t>, Node<'t>, Option<Node<'t>>);
    let mut found: HashMap<usize, Def> = HashMap::new();
    let mut cursor = QueryCursor::new();
    let mut node_text = |n: Node| std::iter::once(text.collect_range(n.byte_range()));
    let mut it = cursor.matches(&o.query, tree.root_node(), &mut node_text);
    while let Some(m) = it.next() {
        let (mut def, mut name, mut detail) = (None, None, None);
        for c in m.captures() {
            if c.index == o.name {
                name = Some(c.node);
            } else if Some(c.index) == o.detail {
                detail = Some(c.node);
            } else if matches!(o.kinds.get(c.index as usize), Some(Some(_))) {
                def = Some((c.index, c.node));
            }
        }
        let (Some((kind, def)), Some(name)) = (def, name) else {
            continue;
        };
        let this = (m.pattern_index, kind, def, name, detail);
        let e = found.entry(def.id()).or_insert(this);
        if m.pattern_index < e.0 {
            *e = this;
        }
    }
    let mut defs: Vec<Def> = found.into_values().collect();
    defs.sort_by_key(|(_, _, d, _, _)| (d.start_byte(), std::cmp::Reverse(d.end_byte())));
    defs.truncate(OUTLINE_MAX);
    let first_line = |n: Node| {
        let bytes = text.collect_range(n.byte_range());
        let s = String::from_utf8_lossy(&bytes);
        s.lines().next().unwrap_or("").trim().to_string()
    };
    let mut open: Vec<usize> = Vec::new();
    let mut out = Vec::with_capacity(defs.len());
    for (_, kind, def, name, detail) in defs {
        while open.last().is_some_and(|&end| def.start_byte() >= end) {
            open.pop();
        }
        let kind = o.kinds[kind as usize].clone().unwrap_or_default();
        let at = name.start_position();
        let line_start = name.start_byte() - at.column;
        let prefix = text.collect_range(line_start..name.start_byte());
        out.push(Outlined {
            name: first_line(name),
            kind,
            detail: detail.map(first_line),
            range: def.byte_range(),
            line: at.row as u32,
            character: String::from_utf8_lossy(&prefix).chars().count() as u32,
            end_line: def.end_position().row as u32,
            depth: open.len() as u32,
        });
        open.push(def.end_byte());
    }
    answer(Ok(out))
}

/// The runs of `span`: the host grammar's captures painted over a byte
/// map, the languages injected into the span painted over them, then
/// coalesced into runs. A capture reaching past the span is cut at it:
/// the layer keeps its own run for the part outside.
fn capture_runs(
    parser: &mut Parser,
    grammars: &mut Grammars,
    g: &Grammar,
    root: Node,
    text: &text_buffer::Buffer,
    span: Range<usize>,
) -> Vec<Run> {
    let base = span.start;
    let mut paint = vec![0u8; span.len()];
    paint_captures(g, root, text, span.clone(), base, &mut paint);
    paint_injections(parser, grammars, g, root, text, span, base, &mut paint, 0);
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

/// `span` widened to the block it is in: up from the node it covers to
/// the child of a container (`Structure::containers`). A span over
/// several blocks — a container itself — is left as it is.
fn enclosing_block(st: &Structure, root: Node, span: Range<usize>) -> Range<usize> {
    if st.containers.is_empty() {
        return span;
    }
    let Some(mut node) = root.descendant_for_byte_range(span.start, span.end) else {
        return span;
    };
    if st.containers.contains(&node.kind()) {
        return span;
    }
    while let Some(parent) = node.parent() {
        if st.containers.contains(&parent.kind()) {
            break;
        }
        node = parent;
    }
    span.start.min(node.start_byte())..span.end.max(node.end_byte())
}

/// `span` widened to the lines it touches, the last one's newline in.
/// What an incremental answer covers for `span`: the parent of the
/// smallest node spanning it — the edited node whole, and its siblings,
/// whose kind and field an edit can change unreported — on the lines
/// `span` touches. The lines alone were once the cover, which on a
/// minified bundle, a megabyte on one line, is the whole file queried
/// again at every keystroke; the parent alone can be a function body
/// many lines long. Each bounds the other, and neither is scanned past.
fn neighbourhood(root: Node, text: &text_buffer::Buffer, span: Range<usize>) -> Range<usize> {
    let len = text.len();
    let span = span.start.min(len)..span.end.min(len);
    let Some(node) = root.descendant_for_byte_range(span.start, span.end) else {
        return span;
    };
    let near = node.parent().unwrap_or(node).byte_range();
    let (lo, hi) = (near.start.min(span.start), near.end.max(span.end).min(len));
    let mut start = span.start;
    while start > lo && text.byte_at(start - 1) != Some(b'\n') {
        start -= 1;
    }
    let mut end = span.end;
    while end < hi && text.byte_at(end) != Some(b'\n') {
        end += 1;
    }
    start..end
}

fn whole_lines(text: &text_buffer::Buffer, span: Range<usize>) -> Range<usize> {
    let len = text.len();
    let mut start = span.start.min(len);
    while start > 0 && text.byte_at(start - 1) != Some(b'\n') {
        start -= 1;
    }
    let mut end = span.end.min(len);
    while end < len && text.byte_at(end) != Some(b'\n') {
        end += 1;
    }
    start..(end + 1).min(len)
}

/// The structure query's captures in `span` as runs, the outer first so
/// an inner one paints over it, each run's style its block kind.
fn structure_runs(
    st: &Structure,
    root: Node,
    text: &text_buffer::Buffer,
    span: Range<usize>,
) -> Vec<Run> {
    let base = span.start;
    let mut paint = vec![0u8; span.len()];
    let mut caps: Vec<(Range<usize>, u8)> = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(span.clone());
    let mut node_text = |n: Node| std::iter::once(text.collect_range(n.byte_range()));
    let mut it = cursor.captures(&st.query, root, &mut node_text);
    while let Some((m, i)) = it.next() {
        let c = m.captures()[*i];
        let Some(Some(kind)) = st.kinds.get(c.index as usize) else {
            continue;
        };
        let r = c.node.byte_range();
        let r = r.start.max(span.start)..r.end.min(span.end);
        if r.start < r.end {
            caps.push((r, *kind as u8));
        }
    }
    caps.sort_by(|a, b| {
        (a.0.start, std::cmp::Reverse(a.0.end)).cmp(&(b.0.start, std::cmp::Reverse(b.0.end)))
    });
    for (r, kind) in caps {
        for p in &mut paint[r.start - base..r.end - base] {
            *p = kind;
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

/// Paints every capture of `g` in `span` over `paint` (whose byte 0 is
/// document byte `base`), outer captures first so an inner one wins
/// (tree-sitter's own precedence).
fn paint_captures(
    g: &Grammar,
    root: Node,
    text: &text_buffer::Buffer,
    span: Range<usize>,
    base: usize,
    paint: &mut [u8],
) {
    let mut caps: Vec<(Range<usize>, Token)> = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(span.clone());
    // A node's text, for the predicates (`#match?` on a builtin's name):
    // asked for that node alone, off the pieces.
    let mut node_text = |n: Node| std::iter::once(text.collect_range(n.byte_range()));
    let mut it = cursor.captures(&g.query, root, &mut node_text);
    while let Some((m, i)) = it.next() {
        let c = m.captures()[*i];
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
    for (r, tok) in caps {
        for p in &mut paint[r.start - base..r.end - base] {
            *p = tok as u8;
        }
    }
}

/// The injection holding byte `at` under `root`, the innermost when
/// they nest: its language as the query names it (the `#set!`, or the
/// `@injection.language` capture's text, its first word — `rust` of
/// `rust,ignore`) and its content's bytes. A `#set! injection.combined`
/// pattern is not followed, as the painter does not. What `gc` asks
/// for the layer's comment token (docs/design/comments.md Decision 4).
pub fn injection_at(
    inj: &kawoosh_languages::Injections,
    root: Node,
    text: &text_buffer::Buffer,
    at: usize,
) -> Option<(String, Range<usize>)> {
    let mut best: Option<(String, Range<usize>)> = None;
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(at..at + 1);
    let mut node_text = |n: Node| std::iter::once(text.collect_range(n.byte_range()));
    let mut it = cursor.matches(&inj.query, root, &mut node_text);
    while let Some(m) = it.next() {
        let settings = inj.query.property_settings(m.pattern_index);
        if settings.iter().any(|p| &*p.key == "injection.combined") {
            continue;
        }
        let name = settings
            .iter()
            .find(|p| &*p.key == "injection.language")
            .and_then(|p| p.value.as_deref().map(str::to_owned))
            .or_else(|| {
                let n = m.nodes_for_capture_index(inj.language?).next()?;
                let word = text.collect_range(n.byte_range());
                Some(String::from_utf8_lossy(&word).into_owned())
            });
        let Some(name) = name else {
            continue;
        };
        let word = name
            .split(|c: char| c == ',' || c.is_whitespace())
            .next()
            .unwrap_or("")
            .to_string();
        if word.is_empty() {
            continue;
        }
        for n in m.nodes_for_capture_index(inj.content) {
            let r = n.byte_range();
            if r.start <= at && at < r.end && best.as_ref().is_none_or(|(_, b)| r.len() < b.len()) {
                best = Some((word.clone(), r));
            }
        }
    }
    best
}

/// Paints the languages inside `span` over the host's paint: each
/// `@injection.content` node of `g`'s injections query that reaches
/// into the span, whose language — a `#set!` on the pattern, or the
/// `@injection.language` node's text, its first word (a fence's
/// `rust,ignore`) — this build has a grammar for, is parsed by that
/// grammar over the node alone and its captures painted on top (JSDoc's
/// tags over the comment's colour), then its own injections under it,
/// [`INJECTION_DEPTH`] deep. A `#set! injection.combined` pattern (a
/// tagged template's pieces as one document) is not followed.
#[allow(clippy::too_many_arguments)]
fn paint_injections(
    parser: &mut Parser,
    grammars: &mut Grammars,
    g: &Grammar,
    root: Node,
    text: &text_buffer::Buffer,
    span: Range<usize>,
    base: usize,
    paint: &mut [u8],
    depth: usize,
) {
    let Some(inj) = &g.injections else {
        return;
    };
    if depth >= INJECTION_DEPTH {
        return;
    }
    let mut found: Vec<(String, Range<usize>)> = Vec::new();
    {
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(span.clone());
        let mut node_text = |n: Node| std::iter::once(text.collect_range(n.byte_range()));
        let mut it = cursor.matches(&inj.query, root, &mut node_text);
        while let Some(m) = it.next() {
            let settings = inj.query.property_settings(m.pattern_index);
            if settings.iter().any(|p| &*p.key == "injection.combined") {
                continue;
            }
            let name = settings
                .iter()
                .find(|p| &*p.key == "injection.language")
                .and_then(|p| p.value.as_deref().map(str::to_owned))
                .or_else(|| {
                    let n = m.nodes_for_capture_index(inj.language?).next()?;
                    let word = text.collect_range(n.byte_range());
                    Some(String::from_utf8_lossy(&word).into_owned())
                });
            let Some(name) = name else {
                continue;
            };
            for n in m.nodes_for_capture_index(inj.content) {
                let r = n.byte_range();
                if r.start < r.end {
                    found.push((name.clone(), r));
                }
            }
        }
    }
    for (name, range) in found {
        let word = name
            .split(|c: char| c == ',' || c.is_whitespace())
            .next()
            .unwrap_or("");
        let Some(ig) = grammars.get(word) else {
            continue;
        };
        let Some(tree) = parse_range(parser, &ig, text, range.clone()) else {
            continue;
        };
        let inner = range.start.max(span.start)..range.end.min(span.end);
        let root = tree.root_node();
        paint_captures(&ig, root, text, inner.clone(), base, paint);
        paint_injections(
            parser,
            grammars,
            &ig,
            root,
            text,
            inner,
            base,
            paint,
            depth + 1,
        );
    }
}

/// Parses `range` of the text alone with `g` — tree-sitter's included
/// ranges, reset after — for an injection. Its nodes' positions are
/// the document's.
pub fn parse_range(
    parser: &mut Parser,
    g: &Grammar,
    text: &text_buffer::Buffer,
    range: Range<usize>,
) -> Option<Tree> {
    parser.set_language(&g.language).ok()?;
    let included = tree_sitter::Range {
        start_byte: range.start,
        end_byte: range.end,
        start_point: point_at(text, range.start),
        end_point: point_at(text, range.end),
    };
    if parser.set_included_ranges(&[included]).is_err() {
        return None;
    }
    let mut read = |byte: usize, _: Point| text.chunk_at(byte);
    let tree = parser.parse_with_options(&mut read, None, None);
    let _ = parser.set_included_ranges(&[]);
    tree
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_doc::Buffer;
    use kawoosh_languages::Block;

    #[test]
    fn rust_gets_keywords_strings_and_comments() {
        let mut g = Grammars::default();
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

    /// An outline of `src` as `language`: each definition as
    /// `depth kind name`, in the file's order.
    fn outline_lines(language: &str, src: &str) -> Vec<String> {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let buf = Buffer::new("t", src);
        let job = Job {
            buffer: BufferId::default(),
            language: language.into(),
            snapshot: buf.snapshot(),
            edits: None,
        };
        highlight(&mut parser, &mut g, &mut parsed, &job);
        let ask = OutlineJob {
            token: 1,
            buffer: BufferId::default(),
        };
        outline_of(&parsed, &mut g, &ask)
            .result
            .unwrap()
            .iter()
            .map(|o| {
                let detail = o
                    .detail
                    .as_deref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default();
                format!("{} {} {}{detail}", o.depth, o.kind, o.name)
            })
            .collect()
    }

    /// The outline nests by range: a method under its `impl`, a field
    /// under its struct, a module's items under it; the method pattern
    /// wins over the function one the same node matches, and a trait's
    /// `impl` names the trait beside the type.
    #[test]
    fn a_rust_outline_is_a_tree() {
        let src = "mod m {\n    pub struct S {\n        a: u8,\n    }\n    impl S {\n        fn new() -> Self { todo!() }\n    }\n    impl Default for S {\n        fn default() -> Self { todo!() }\n    }\n}\nfn main() {\n    let x = 1;\n}\nconst N: u8 = 1;\n";
        assert_eq!(
            outline_lines("rust", src),
            [
                "0 module m",
                "1 struct S",
                "2 field a",
                "1 impl S",
                "2 method new",
                "1 impl S (Default)",
                "2 method default",
                "0 function main",
                "1 variable x",
                "0 constant N",
            ]
        );
        let ask = OutlineJob {
            token: 7,
            buffer: BufferId::default(),
        };
        let a = outline_of(&Parsed::default(), &mut Grammars::default(), &ask);
        assert_eq!(a.token, 7);
        assert!(a.result.is_err(), "nothing parsed, no outline");
    }

    /// Markdown's headings nest by their sections, whatever the level
    /// skipped; the name is the heading's text.
    #[test]
    fn a_markdown_outline_nests_by_section() {
        let src = "# Top\n\ntext\n\n## One\n\n#### Deep\n\n## Two\n\nSet\n---\n";
        assert_eq!(
            outline_lines("markdown", src),
            ["0 h1 Top", "1 h2 One", "2 h4 Deep", "1 h2 Two", "2 h2 Set"]
        );
    }

    /// A test file's blocks are its outline, named by their titles and
    /// nested as they are written (docs/design/breadcrumbs.md): `describe`
    /// and `it` with a modifier or an `.each` table, a template title; a
    /// call that is no test runner's, or has no callback, is none. Go's
    /// subtests under their test.
    #[test]
    fn a_test_files_blocks_are_its_outline() {
        let src = r#"describe("parser", () => {
  const input = 1;
  beforeEach(() => {});
  it("reads a number", () => {
    expect(parse("1")).toBe(1);
  });
  describe.only(`with ${input}`, function () {
    test.skip("skips", async () => {});
    it.each([1, 2])("each %d", (n) => {});
  });
  foo("not a test", () => {});
  it("no callback");
});
"#;
        let want = [
            "0 test parser",
            "1 variable input",
            "1 test reads a number",
            "1 test `with ${input}`",
            "2 test skips",
            "2 test each %d",
        ];
        for language in ["javascript", "typescript", "tsx"] {
            assert_eq!(outline_lines(language, src), want, "{language}");
        }
        let go = "package p\nfunc TestParse(t *testing.T) {\n\tt.Run(\"empty\", func(t *testing.T) {\n\t\tt.Run(\"nested\", func(t *testing.T) {})\n\t})\n}\n";
        assert_eq!(
            outline_lines("go", go),
            [
                "0 package p",
                "0 function TestParse",
                "1 test empty",
                "2 test nested"
            ]
        );
    }

    /// Every language with an outline answers one for a small file.
    #[test]
    fn the_other_outlines() {
        let cases: &[(&str, &str, &[&str])] = &[
            (
                "python",
                "class A:\n    x = 1\n    def f(self): pass\ndef g(): pass\nN = 2\n",
                &[
                    "0 class A",
                    "1 field x",
                    "1 method f",
                    "0 function g",
                    "0 variable N",
                ],
            ),
            (
                "go",
                "package p\ntype S struct { A int }\nfunc (s S) M() {}\nfunc F() {}\n",
                &[
                    "0 package p",
                    "0 struct S",
                    "1 field A",
                    "0 method M ((s S))",
                    "0 function F",
                ],
            ),
            (
                "javascript",
                "class A { m() {} }\nfunction f() {}\nconst g = () => 1;\nconst n = 2;\n",
                &[
                    "0 class A",
                    "1 method m",
                    "0 function f",
                    "0 function g",
                    "0 variable n",
                ],
            ),
            (
                "typescript",
                "interface I { a: number; m(): void }\ntype T = string;\nenum E { X }\n",
                &[
                    "0 interface I",
                    "1 field a",
                    "1 method m",
                    "0 type T",
                    "0 enum E",
                ],
            ),
            (
                "c",
                "struct S { int a; };\nint main(void) { return 0; }\n#define N 1\n",
                &["0 struct S", "1 field a", "0 function main", "0 macro N"],
            ),
            (
                "cpp",
                "namespace n { class C { void m(); }; }\n",
                &["0 namespace n", "1 class C", "2 method m"],
            ),
            (
                "lua",
                "local M = {}\nfunction M.f() end\nlocal function g() end\nreturn M\n",
                &["0 table M", "0 function M.f", "0 function g"],
            ),
            ("bash", "f() { :; }\n", &["0 function f"]),
            (
                "nu",
                "def greet [] { }\nmodule m { }\n",
                &["0 function greet", "0 module m"],
            ),
            (
                "css",
                "a { color: red }\n@media screen { b { } }\n",
                &["0 rule a", "0 media screen", "1 rule b"],
            ),
            (
                "toml",
                "a = 1\n[pkg]\nname = \"x\"\n",
                &["0 key a", "0 table pkg", "1 key name"],
            ),
            ("yaml", "a:\n  b: 1\n", &["0 key a", "1 key b"]),
            ("json", "{\"a\": {\"b\": 1}}\n", &["0 key a", "1 key b"]),
            (
                "sql",
                "CREATE TABLE t (id int);\n",
                &["0 table t", "1 column id"],
            ),
            (
                "scheme",
                "(define-library (my lib)\n  (begin\n    (define (f x) (define y 1) y)\n    (define g (lambda () 1))\n    (define n 2)\n    (define-syntax swap! (syntax-rules () ((_ a b) 1)))\n    (define-record-type point (make-point x) point? (x point-x))))\n",
                &[
                    "0 module (my lib)",
                    "1 function f",
                    "2 variable y",
                    "1 function g",
                    "1 variable n",
                    "1 macro swap!",
                    "1 struct point",
                ],
            ),
        ];
        for (language, src, want) in cases {
            assert_eq!(outline_lines(language, src), *want, "{language}");
        }
    }

    /// A variant is a constructor whatever its case: the query's own rule
    /// is by an uppercase first letter.
    #[test]
    fn rust_enum_variants_are_constructors_by_position() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let src = "enum E {\n    Value,\n    value,\n    Pair(u8),\n}\n";
        let buf = Buffer::new("t", src);
        let job = Job {
            buffer: BufferId::default(),
            language: "rust".into(),
            snapshot: buf.snapshot(),
            edits: None,
        };
        let a = highlight(&mut parser, &mut g, &mut Parsed::default(), &job).update();
        for needle in ["Value", "value", "Pair"] {
            let o = src.find(needle).unwrap();
            let got = a
                .runs
                .iter()
                .find(|r| r.range.contains(&o))
                .map(|r| Token::from_style(r.style));
            assert_eq!(got, Some(Token::Constructor), "{needle}");
        }
    }

    /// Each grammar's query compiles and lands the classes a theme
    /// colours: a keyword, a string, a comment, and one of its own.
    #[test]
    fn the_other_grammars_highlight() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        type Case = (&'static str, &'static str, &'static [(&'static str, Token)]);
        let cases: &[Case] = &[
            (
                "toml",
                "# c\n[pkg]\nname = \"x\"\nn = 1\n",
                &[
                    ("# c", Token::Comment),
                    ("name", Token::Property),
                    ("\"x\"", Token::String),
                    ("1", Token::Number),
                ],
            ),
            (
                "css",
                "/* c */\n.a { color: red; }\n",
                &[
                    ("/* c */", Token::Comment),
                    ("color", Token::Property),
                    ("{", Token::Punctuation),
                ],
            ),
            (
                "javascript",
                "// c\nfunction f(a) { return \"s\" + 1; }\n",
                &[
                    ("// c", Token::Comment),
                    ("function", Token::Keyword),
                    ("f(", Token::Function),
                    ("\"s\"", Token::String),
                    ("1;", Token::Number),
                ],
            ),
            (
                "go",
                "// c\npackage main\nfunc main() { s := \"x\" }\n",
                &[
                    ("// c", Token::Comment),
                    ("func", Token::Keyword),
                    ("main()", Token::Function),
                    ("\"x\"", Token::String),
                ],
            ),
            // lua: its query spells the branch and loop keywords as
            // nvim's `conditional` and `repeat`; a field, a parameter,
            // a method call.
            (
                "lua",
                "-- c\nlocal M = {}\nfunction M.f(a)\n  if a then return \"s\" end\n  while a.n do a:go() end\n  return 1\nend\n",
                &[
                    ("-- c", Token::Comment),
                    ("local", Token::Keyword),
                    ("function", Token::Keyword),
                    ("f(", Token::Function),
                    ("a)", Token::Variable),
                    ("if", Token::Keyword),
                    ("then", Token::Keyword),
                    ("\"s\"", Token::String),
                    ("while", Token::Keyword),
                    ("n do", Token::Property),
                    ("go()", Token::Function),
                    ("1\n", Token::Number),
                ],
            ),
            // typescript: javascript's classes, and its own — a type
            // name, an `interface`, and a capitalised identifier read
            // as a type over javascript's bare variable.
            (
                "typescript",
                "// c\ninterface P { n: number }\nfunction f(a: P): string { return \"s\" + Foo; }\n",
                &[
                    ("// c", Token::Comment),
                    ("interface", Token::Keyword),
                    ("P {", Token::Type),
                    ("number", Token::Type),
                    ("function", Token::Keyword),
                    ("f(", Token::Function),
                    ("\"s\"", Token::String),
                    ("Foo", Token::Type),
                ],
            ),
            // tsx: the same, and JSX's tags and attributes.
            (
                "tsx",
                "const a: number = 1;\nconst e = <div className=\"x\"><Item n={a} /></div>;\n",
                &[
                    ("const", Token::Keyword),
                    ("number", Token::Type),
                    ("div", Token::Tag),
                    ("className", Token::Attribute),
                    ("Item", Token::Type),
                    ("\"x\"", Token::String),
                ],
            ),
            (
                "bash",
                "# c\nif [ -f x ]; then echo \"hi\"; fi\nfoo() { ls; }\n",
                &[
                    ("# c", Token::Comment),
                    ("if", Token::Keyword),
                    ("echo", Token::Function),
                    ("\"hi\"", Token::String),
                    ("foo()", Token::Function),
                ],
            ),
            (
                "nu",
                "# c\ndef greet [name: string] { $\"hi ($name)\" }\nlet x = 1\n",
                &[
                    ("# c", Token::Comment),
                    ("def", Token::Keyword),
                    ("greet", Token::Function),
                    ("let", Token::Keyword),
                    ("1\n", Token::Number),
                ],
            ),
            (
                "c",
                "// c\n#include <stdio.h>\nint main(void) { return 0; }\n",
                &[
                    ("// c", Token::Comment),
                    ("int", Token::Type),
                    ("main", Token::Function),
                    ("return", Token::Keyword),
                    ("0;", Token::Number),
                ],
            ),
            (
                "cpp",
                "// c\nclass A { public: int f() { return 1; } };\n",
                &[
                    ("// c", Token::Comment),
                    ("class", Token::Keyword),
                    ("int", Token::Type),
                    ("f()", Token::Function),
                    ("1;", Token::Number),
                ],
            ),
            (
                "python",
                "# c\ndef f(a):\n    return \"s\" + str(1)\n",
                &[
                    ("# c", Token::Comment),
                    ("def", Token::Keyword),
                    ("f(", Token::Function),
                    ("return", Token::Keyword),
                    ("\"s\"", Token::String),
                    ("1)", Token::Number),
                ],
            ),
            // json: a key is a property, not the `string.special.key`
            // the query says.
            (
                "json",
                "{\"a\": 1, \"b\": [true, null], \"c\": \"s\"}\n",
                &[
                    ("\"a\"", Token::Property),
                    ("1", Token::Number),
                    ("true", Token::Constant),
                    ("\"s\"", Token::String),
                ],
            ),
            (
                "jsonc",
                "// c\n{\"a\": 1}\n",
                &[("// c", Token::Comment), ("\"a\"", Token::Property)],
            ),
            (
                "yaml",
                "# c\nkey: value\nn: 1\nlist:\n  - \"s\"\n",
                &[
                    ("# c", Token::Comment),
                    ("key", Token::Property),
                    ("1\n", Token::Number),
                    ("\"s\"", Token::String),
                ],
            ),
            (
                "sql",
                "-- c\nSELECT name FROM users WHERE id = 1;\n",
                &[
                    ("-- c", Token::Comment),
                    ("SELECT", Token::Keyword),
                    ("FROM", Token::Keyword),
                    ("1;", Token::Number),
                ],
            ),
            (
                "regex",
                "(a|b)+\\d{2}\n",
                &[
                    ("(", Token::Punctuation),
                    ("|", Token::Operator),
                    ("+", Token::Operator),
                    ("2}", Token::Number),
                ],
            ),
            (
                "jsdoc",
                "/** @param {number} x */\n",
                &[("@param", Token::Keyword), ("number", Token::Type)],
            ),
            // diff: the query here names what the crate's calls a
            // string and a keyword.
            (
                "diff",
                "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-old\n+new\n",
                &[
                    ("--- a/x", Token::Removed),
                    ("+++ b/x", Token::Added),
                    ("@@", Token::Attribute),
                    ("-old", Token::Removed),
                    ("+new", Token::Added),
                ],
            ),
            (
                "gitcommit",
                "feat(api): add thing\n\nbody\n\n# Please enter the commit message\n",
                &[
                    ("feat", Token::Keyword),
                    ("api", Token::Variable),
                    ("add thing", Token::Heading),
                    ("# Please", Token::Comment),
                ],
            ),
            (
                "gomod",
                "module example.com/m\n\ngo 1.22\n\nrequire (\n\tx.io/y v1.2.3 // c\n)\n",
                &[
                    ("module", Token::Keyword),
                    ("go 1", Token::Keyword),
                    ("1.22", Token::String),
                    ("require", Token::Keyword),
                    ("v1.2.3", Token::String),
                    ("// c", Token::Comment),
                ],
            ),
            // scheme: a list's head is a call, a form's name a keyword,
            // a quoted datum a constant; a named let's name is a
            // procedure, a record's field a property.
            (
                "scheme",
                "; c\n(define (square n) (* n n))\n(let loop ((i 0)) (display \"s\"))\n(car '(sym) #t 1.5)\n(define-record-type point (make-point px) point? (px point-x))\n",
                &[
                    ("; c", Token::Comment),
                    ("define", Token::Keyword),
                    ("square", Token::Function),
                    ("loop", Token::Function),
                    ("display", Token::Function),
                    ("\"s\"", Token::String),
                    ("sym", Token::Constant),
                    ("#t", Token::Constant),
                    ("1.5", Token::Number),
                    ("define-record-type", Token::Keyword),
                    ("px point-x", Token::Property),
                ],
            ),
            // markdown: the block grammar's own — a heading, a list
            // marker; the paragraph's inside is the inline grammar's,
            // an injection (the next test).
            (
                "markdown",
                "# Title\n\n- item\n\n```\ncode\n```\n",
                &[
                    ("Title", Token::Heading),
                    ("- item", Token::Punctuation),
                    ("```", Token::Punctuation),
                ],
            ),
        ];
        let mut wrong = Vec::new();
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
                if got != Some(*tok) {
                    wrong.push(format!("{lang}: {needle:?} is {got:?}, not {tok:?}"));
                }
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// A name a Scheme form binds — a lambda's formals, a `let`'s or a
    /// `do`'s bindings, `let-values`' — heads a list but is no call: it
    /// is not painted a function, where a call beside it is.
    #[test]
    fn scheme_bindings_are_not_calls() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let src = "(lambda (arg rest) (f arg))\n(let* ((bound 1)) (g bound))\n(let loop ((i 0)) (h i))\n(do ((step 0 (+ step 1))) ((= step 3)))\n(let-values (((q r) (floor/ 7 2))) q)\n(case-lambda ((one) one) ((a b) a))\n";
        let buf = Buffer::new("t", src);
        let job = Job {
            buffer: BufferId::default(),
            language: "scheme".into(),
            snapshot: buf.snapshot(),
            edits: None,
        };
        let a = highlight(&mut parser, &mut g, &mut Parsed::default(), &job).update();
        let at = |needle: &str| {
            let o = src.find(needle).unwrap();
            a.runs
                .iter()
                .find(|r| r.range.contains(&o))
                .map_or(Token::Plain, |r| Token::from_style(r.style))
        };
        for bound in ["arg rest", "bound 1", "i 0", "step 0", "q r", "one)", "a b"] {
            assert_eq!(at(bound), Token::Plain, "{bound:?}");
        }
        for call in ["f arg", "g bound", "h i", "floor/"] {
            assert_eq!(at(call), Token::Function, "{call:?}");
        }
    }

    /// A language inside another: markdown's paragraphs are its inline
    /// grammar's (emphasis, a code span, a link) and a fence's content
    /// its language's, over a plain background between raw fences; a
    /// javascript comment's tags are JSDoc's over the comment's colour,
    /// a regex literal's operators regex's over the string's. A fence
    /// naming no grammar this build has stays plain.
    #[test]
    fn injections_paint_over_the_host() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        type Case = (
            &'static str,
            &'static str,
            &'static [(&'static str, Option<Token>)],
        );
        let cases: &[Case] = &[
            (
                "markdown",
                "# Title\n\nSome *em* and **strong**, `code`, [l](http://x).\n\n```rust\nfn main() { let s = \"x\"; }\n```\n\n```brainfuck\n+++\n```\n",
                &[
                    ("Title", Some(Token::Heading)),
                    ("em*", Some(Token::Emphasis)),
                    ("strong**", Some(Token::Strong)),
                    ("code`", Some(Token::Raw)),
                    ("l](", Some(Token::Link)),
                    ("http://x", Some(Token::Link)),
                    ("Some", None),
                    ("```rust", Some(Token::Punctuation)),
                    ("fn", Some(Token::Keyword)),
                    ("main", Some(Token::Function)),
                    ("let", Some(Token::Keyword)),
                    ("\"x\"", Some(Token::String)),
                    (" s ", None),
                    ("+++", None),
                ],
            ),
            (
                "javascript",
                "/** @param {number} x */\nconst r = /a+b/g;\n",
                &[
                    ("/**", Some(Token::Comment)),
                    ("@param", Some(Token::Keyword)),
                    ("number", Some(Token::Type)),
                    ("x */", Some(Token::Comment)),
                    ("const", Some(Token::Keyword)),
                    ("a+b", Some(Token::String)),
                    ("+b", Some(Token::Operator)),
                ],
            ),
            (
                "typescript",
                "/** @returns {string} y */\nconst r = /[a-z]+/;\n",
                &[
                    ("@returns", Some(Token::Keyword)),
                    ("string", Some(Token::Type)),
                    ("+/", Some(Token::Operator)),
                ],
            ),
        ];
        let mut wrong = Vec::new();
        for (lang, src, want) in cases {
            let buf = Buffer::new("t", src);
            let job = Job {
                buffer: BufferId::default(),
                language: (*lang).into(),
                snapshot: buf.snapshot(),
                edits: None,
            };
            let a = highlight(&mut parser, &mut g, &mut Parsed::default(), &job).update();
            for (needle, tok) in *want {
                let o = src.find(needle).unwrap();
                let got = a
                    .runs
                    .iter()
                    .find(|r| r.range.contains(&o))
                    .map(|r| Token::from_style(r.style));
                if got != *tok {
                    wrong.push(format!("{lang}: {needle:?} is {got:?}, not {tok:?}"));
                }
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// The injected languages survive the incremental path: an edit in
    /// one paragraph answers for a span around it, the inline runs and
    /// a fence's recomputed there, and the layer reads as a whole parse
    /// would.
    #[test]
    fn injections_survive_a_reparse() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let body: String = (0..60)
            .map(|i| {
                format!("Para {i} with *em{i}* and `c{i}`.\n\n```rust\nfn f{i}() {{}}\n```\n\n")
            })
            .collect();
        let mut buf = Buffer::new("t", &body);
        buf.language = "markdown".into();
        let mut sent: Option<kawoosh_doc::Version> = None;
        let mut job = |buf: &Buffer| {
            let edits = sent
                .and_then(|v| buf.journal().edits_since(v).ok())
                .map(|it| it.cloned().collect());
            sent = Some(buf.version());
            Job {
                buffer: BufferId::default(),
                language: "markdown".into(),
                snapshot: buf.snapshot(),
                edits,
            }
        };
        let whole_job = |buf: &Buffer| Job {
            buffer: BufferId::default(),
            language: "markdown".into(),
            snapshot: buf.snapshot(),
            edits: None,
        };
        let first = highlight(&mut parser, &mut g, &mut parsed, &job(&buf)).update();
        buf.apply(first).unwrap();
        let at = buf.text().find("Para 30 ").unwrap() + "Para 30".len();
        buf.replace(at..at, " more");
        let second = highlight(&mut parser, &mut g, &mut parsed, &job(&buf));
        let spanned: usize = second.updates.iter().map(|u| u.span.len()).sum();
        assert!(spanned < buf.len() / 2, "{spanned} of {} bytes", buf.len());
        for u in second.updates {
            buf.apply(u).unwrap();
        }
        let whole = highlight(
            &mut parser,
            &mut g,
            &mut Parsed::default(),
            &whole_job(&buf),
        )
        .update();
        assert_eq!(
            joined(&buf.runs(SYNTAX_LAYER, 0..buf.len())),
            joined(&whole.runs)
        );
        let tok_at = |needle: &str, buf: &Buffer| {
            let o = buf.text().find(needle).unwrap();
            buf.runs(SYNTAX_LAYER, o..o + 1)
                .first()
                .map(|r| Token::from_style(r.style))
        };
        assert_eq!(tok_at("em30", &buf), Some(Token::Emphasis));
        assert_eq!(tok_at("c30`", &buf), Some(Token::Raw));
        assert_eq!(tok_at("fn f30", &buf), Some(Token::Keyword));
    }

    /// A setext underline turned from `=` to `-` changes the kind of
    /// the line above it: the structure repaints the whole block, which
    /// the reparse's changed ranges (the underline's byte) do not reach.
    #[test]
    fn a_setext_underline_typed_on_into_a_list_item_unpaints_its_heading() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let mut buf = Buffer::new("t", "# Top\n\nsome para\nmore para\n\nend\n");
        buf.language = "markdown".into();
        let mut sent: Option<kawoosh_doc::Version> = None;
        let mut run = |buf: &mut Buffer, parsed: &mut Parsed| {
            let edits = sent
                .and_then(|v| buf.journal().edits_since(v).ok())
                .map(|it| it.cloned().collect());
            sent = Some(buf.version());
            let job = Job {
                buffer: BufferId::default(),
                language: "markdown".into(),
                snapshot: buf.snapshot(),
                edits,
            };
            for u in highlight(&mut parser, &mut g, parsed, &job).updates {
                buf.apply(u).unwrap();
            }
        };
        let para = |buf: &Buffer| -> Vec<Block> {
            let at = buf.text().find("some").unwrap();
            buf.runs(STRUCT_LAYER, at..at + 1)
                .iter()
                .filter_map(|r| Block::from_style(r.style))
                .collect()
        };
        run(&mut buf, &mut parsed);
        assert_eq!(para(&buf), []);
        let at = buf.text().find("more para\n").unwrap() + "more para\n".len();
        buf.replace(at..at, "-\n");
        run(&mut buf, &mut parsed);
        assert_eq!(
            para(&buf),
            [Block::H2],
            "a setext heading, as CommonMark says"
        );
        buf.replace(at + 1..at + 1, " [");
        run(&mut buf, &mut parsed);
        let incremental = para(&buf);
        // The same text parsed whole.
        let mut whole = Buffer::new("t", &buf.text());
        whole.language = "markdown".into();
        let mut fresh = Parsed::default();
        let job = Job {
            buffer: BufferId::default(),
            language: "markdown".into(),
            snapshot: whole.snapshot(),
            edits: None,
        };
        for u in highlight(
            &mut Parser::new(),
            &mut Grammars::default(),
            &mut fresh,
            &job,
        )
        .updates
        {
            whole.apply(u).unwrap();
        }
        assert_eq!(para(&whole), [], "parsed whole, a paragraph");
        assert_eq!(incremental, [], "and so when typed");
    }

    #[test]
    fn a_setext_underline_changed_repaints_its_heading() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let mut buf = Buffer::new("t", "Title\n=\n\npara\n");
        buf.language = "markdown".into();
        let mut sent: Option<kawoosh_doc::Version> = None;
        let mut run = |buf: &mut Buffer| {
            let edits = sent
                .and_then(|v| buf.journal().edits_since(v).ok())
                .map(|it| it.cloned().collect());
            sent = Some(buf.version());
            let job = Job {
                buffer: BufferId::default(),
                language: "markdown".into(),
                snapshot: buf.snapshot(),
                edits,
            };
            for u in highlight(&mut parser, &mut g, &mut parsed, &job).updates {
                buf.apply(u).unwrap();
            }
        };
        let title = |buf: &Buffer| -> Vec<Block> {
            buf.runs(STRUCT_LAYER, 0..1)
                .iter()
                .filter_map(|r| Block::from_style(r.style))
                .collect()
        };
        run(&mut buf);
        assert_eq!(title(&buf), [Block::H1]);
        buf.replace(6..7, "-");
        run(&mut buf);
        assert_eq!(title(&buf), [Block::H2]);
    }

    /// A table's row with an empty cell, typed a pipe at a time, leaves
    /// the rest of the document parsed: the heading after it a heading
    /// and not the table's (a lone `|`) or an ERROR's (`|||`), the row
    /// the table's; and the parse after it, back to a plain text, whole.
    #[test]
    fn a_row_of_empty_cells_does_not_swallow_the_document() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let mut buf = Buffer::new(
            "t",
            "| a | b |\n|---|---|\n| c | d |\n\n\n## Next\n\n```\ncode\n```\n",
        );
        buf.language = "markdown".into();
        let mut sent: Option<kawoosh_doc::Version> = None;
        let mut run = |buf: &mut Buffer| {
            let edits = sent
                .and_then(|v| buf.journal().edits_since(v).ok())
                .map(|it| it.cloned().collect());
            sent = Some(buf.version());
            let job = Job {
                buffer: BufferId::default(),
                language: "markdown".into(),
                snapshot: buf.snapshot(),
                edits,
            };
            for u in highlight(&mut parser, &mut g, &mut parsed, &job).updates {
                buf.apply(u).unwrap();
            }
        };
        let blocks = |buf: &Buffer, o: usize| -> Vec<Block> {
            buf.runs(STRUCT_LAYER, o..o + 1)
                .iter()
                .filter_map(|r| Block::from_style(r.style))
                .collect()
        };
        let block_at = |buf: &Buffer, needle: &str| -> Vec<Block> {
            let o = buf.text().find(needle).unwrap();
            buf.runs(STRUCT_LAYER, o..o + 1)
                .iter()
                .filter_map(|r| Block::from_style(r.style))
                .collect()
        };
        run(&mut buf);
        let row = buf.text().find("\n\n\n").unwrap() + 1;
        for typed in ["|", "|", "|"] {
            let at = buf.text()[row..].find('\n').unwrap() + row;
            buf.replace(at..at, typed);
            run(&mut buf);
            let line = &buf.text()[row..buf.text()[row..].find('\n').unwrap() + row];
            assert_eq!(block_at(&buf, "## Next"), [Block::H2], "after {line:?}");
            assert_eq!(block_at(&buf, "code"), [Block::Code], "after {line:?}");
            assert!(
                blocks(&buf, row).contains(&Block::Table),
                "{line:?} is the table's row"
            );
        }
        let at = buf.text()[row..].find('\n').unwrap() + row;
        buf.replace(row..at, "");
        run(&mut buf);
        assert_eq!(block_at(&buf, "## Next"), [Block::H2]);
        assert!(block_at(&buf, "| c").contains(&Block::Table));
    }

    /// A language told to the thread highlights with the grammar it
    /// came with — json's, under a name of its own — and one told
    /// without a grammar answers empty, its kept tree gone.
    #[test]
    fn a_registered_language_highlights_with_its_grammar() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let json = g
            .registry
            .get("json")
            .and_then(|d| d.load().ok().flatten())
            .expect("json's grammar");
        let mut def = LanguageDef::named("jsonx");
        def.extensions = vec!["jsonx".into()];
        g.add(def, Some(Arc::new(json)), &mut parsed);
        let buf = Buffer::new("t", "{\"a\": 1}\n");
        let job = |lang: &str| Job {
            buffer: BufferId::default(),
            language: lang.into(),
            snapshot: buf.snapshot(),
            edits: None,
        };
        let a = highlight(&mut parser, &mut g, &mut parsed, &job("jsonx")).update();
        let tok_at = |a: &Update, needle: &str| {
            let o = buf.text().find(needle).unwrap();
            a.runs
                .iter()
                .find(|r| r.range.contains(&o))
                .map(|r| Token::from_style(r.style))
        };
        assert_eq!(tok_at(&a, "\"a\""), Some(Token::Property));
        assert_eq!(tok_at(&a, "1"), Some(Token::Number));
        assert!(parsed.by_buffer.contains_key(&BufferId::default()));
        g.add(LanguageDef::named("jsonx"), None, &mut parsed);
        assert!(!parsed.by_buffer.contains_key(&BufferId::default()));
        let a = highlight(&mut parser, &mut g, &mut parsed, &job("jsonx"));
        assert!(a.tree.is_none());
        assert!(a.update().runs.is_empty());
    }

    impl Answer {
        /// The one syntax update of a whole-parse answer (the structure
        /// layer's beside it).
        fn update(mut self) -> Update {
            self.updates.retain(|u| u.layer == SYNTAX_LAYER);
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
        let mut g = Grammars::default();
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
        assert!(
            span.start >= at.saturating_sub(64),
            "{span:?} for an edit at {at}"
        );
        assert!(span.end <= at + 64, "{span:?} for an edit at {at}");
        assert!(
            second
                .runs
                .iter()
                .all(|r| span.start <= r.range.start && r.range.end <= span.end)
        );
        buf.apply(second).unwrap();
        let whole = highlight(
            &mut parser,
            &mut g,
            &mut Parsed::default(),
            &whole_job(&buf),
        )
        .update();
        assert_eq!(whole.span, 0..buf.len());
        assert_eq!(
            joined(&buf.runs(SYNTAX_LAYER, 0..buf.len())),
            joined(&whole.runs)
        );
        let renamed = buf.text().find("renamed").unwrap();
        assert_eq!(
            buf.runs(SYNTAX_LAYER, renamed..renamed + 1)
                .first()
                .map(|r| Token::from_style(r.style)),
            Some(Token::Function)
        );
        // A multicursor keystroke: three insertions at
        // once, told to the tree one by one.
        {
            let at = |n: usize, buf: &Buffer| buf.text().find(&format!("fn f{n}(")).unwrap() + 3;
            let (a, b, c) = (at(10, &buf), at(20, &buf), at(30, &buf));
            buf.replace_many(&[(a..a, "x_"), (b..b, "y_"), (c..c, "z_")]);
            let inc = highlight(&mut parser, &mut g, &mut parsed, &job(&buf));
            assert!(
                inc.updates.len() >= 3,
                "a span per cursor: {:?}",
                inc.updates
                    .iter()
                    .map(|u| u.span.clone())
                    .collect::<Vec<_>>()
            );
            assert!(inc.updates.iter().all(|u| u.span.len() < buf.len() / 4));
            for u in inc.updates {
                buf.apply(u).unwrap();
            }
            let whole = highlight(
                &mut parser,
                &mut g,
                &mut Parsed::default(),
                &whole_job(&buf),
            );
            assert_eq!(
                joined(&buf.runs(SYNTAX_LAYER, 0..buf.len())),
                joined(&whole.update().runs)
            );
            let renamed = buf.text().find("fn y_f20(").unwrap() + 3;
            assert_eq!(
                buf.runs(SYNTAX_LAYER, renamed..renamed + 1)
                    .first()
                    .map(|r| Token::from_style(r.style)),
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
            let whole = highlight(
                &mut parser,
                &mut g,
                &mut Parsed::default(),
                &whole_job(&buf),
            );
            assert_eq!(
                joined(&buf.runs(SYNTAX_LAYER, 0..buf.len())),
                joined(&whole.update().runs),
                "step {step}"
            );
            let f60 = buf.text().find("fn f60(").unwrap();
            let tok = buf
                .runs(SYNTAX_LAYER, f60..f60 + 1)
                .first()
                .map(|r| Token::from_style(r.style));
            let want = if step < 2 {
                Token::Comment
            } else {
                Token::Keyword
            };
            assert_eq!(
                tok,
                Some(want),
                "step {step}: f60 in the block comment until it goes"
            );
        }
    }

    /// Typing inside a token — a string, a comment, a name — keeps the
    /// token's colour over the whole of it, not just the typed byte:
    /// the layer carries its run over the edit, and the answer, which
    /// covers the edit and what tree-sitter says changed, leaves the
    /// carried run where it agrees with a whole parse.
    #[test]
    fn typing_inside_a_token_keeps_its_colour() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let src = "// a comment here\nfn main() {\n    let greeting = \"hello world\";\n    println!(\"{greeting}\");\n}\n";
        let mut buf = Buffer::new("t", src);
        buf.language = "rust".into();
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
        let first = highlight(&mut parser, &mut g, &mut parsed, &job(&buf)).update();
        buf.apply(first).unwrap();
        // Where to type: `into` bytes into `needle`, which is the first
        // `len` bytes of a token; what colour every byte of the token
        // should have afterwards.
        let cases: &[(&str, usize, &str, usize, Token)] = &[
            ("hello world", 5, "\\", 11, Token::String),
            ("a comment", 3, "x", 9, Token::Comment),
            ("main()", 2, "_", 4, Token::Function),
            ("hello", 0, "\\n", 5, Token::String),
        ];
        for (needle, into, typed, len, tok) in cases {
            let at = buf.text().find(needle).unwrap() + into;
            buf.replace(at..at, typed);
            let inc = highlight(&mut parser, &mut g, &mut parsed, &job(&buf));
            for u in inc.updates {
                buf.apply(u).unwrap();
            }
            let whole = highlight(
                &mut parser,
                &mut g,
                &mut Parsed::default(),
                &Job {
                    buffer: BufferId::default(),
                    language: "rust".into(),
                    snapshot: buf.snapshot(),
                    edits: None,
                },
            )
            .update();
            assert_eq!(
                joined(&buf.runs(SYNTAX_LAYER, 0..buf.len())),
                joined(&whole.runs),
                "{needle:?} + {typed:?}"
            );
            // The whole token, every byte of it, in its colour.
            let start = at - into;
            let end = start + len + typed.len();
            for o in start..end {
                let got = buf
                    .runs(SYNTAX_LAYER, o..o + 1)
                    .first()
                    .map(|r| Token::from_style(r.style));
                assert_eq!(got, Some(*tok), "{needle:?} + {typed:?} at {o}");
            }
        }
    }

    /// A value typed a letter at a time into settings.lua reads as a
    /// whole parse would at every letter: `tru` + `e` is `true` from its
    /// `t`, and `relativenumber` the field's name once `=` has a value —
    /// neither a change tree-sitter's changed ranges report.
    #[test]
    fn typing_a_token_into_being_repaints_the_whole_of_it() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let src = "-- kawoosh settings\n---@type kawoosh.Settings\nreturn {\n    relativenumber = ,\n    font = { size = 15 },\n}\n";
        let mut buf = Buffer::new("t", src);
        buf.language = "lua".into();
        let mut sent: Option<kawoosh_doc::Version> = None;
        let mut job = |buf: &Buffer| {
            let edits = sent
                .and_then(|v| buf.journal().edits_since(v).ok())
                .map(|it| it.cloned().collect());
            sent = Some(buf.version());
            Job {
                buffer: BufferId::default(),
                language: "lua".into(),
                snapshot: buf.snapshot(),
                edits,
            }
        };
        let first = highlight(&mut parser, &mut g, &mut parsed, &job(&buf)).update();
        buf.apply(first).unwrap();
        let at = buf.text().find(" ,").unwrap() + 1;
        for (i, letter) in ["t", "r", "u", "e"].into_iter().enumerate() {
            buf.replace(at + i..at + i, letter);
            let inc = highlight(&mut parser, &mut g, &mut parsed, &job(&buf));
            for u in inc.updates {
                buf.apply(u).unwrap();
            }
            let whole = highlight(
                &mut parser,
                &mut g,
                &mut Parsed::default(),
                &Job {
                    buffer: BufferId::default(),
                    language: "lua".into(),
                    snapshot: buf.snapshot(),
                    edits: None,
                },
            )
            .update();
            assert_eq!(
                joined(&buf.runs(SYNTAX_LAYER, 0..buf.len())),
                joined(&whole.runs),
                "after {letter:?}"
            );
        }
        let tok = |o: usize| {
            buf.runs(SYNTAX_LAYER, o..o + 1)
                .first()
                .map(|r| Token::from_style(r.style))
        };
        assert!((at..at + 4).all(|o| tok(o) == tok(at + 3)));
        let name = buf.text().find("relativenumber").unwrap();
        assert_eq!(tok(name), Some(Token::Property));
    }

    /// A keystroke in a minified bundle — a megabyte on one line —
    /// answers for the edit's neighbourhood, not the line: the spans
    /// widen to the edited node's parent within its line, so typing
    /// stays as quick as on a short line and reads as a whole parse.
    #[test]
    fn a_keystroke_in_one_long_line_answers_for_its_neighbourhood() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let src: String = (0..20_000)
            .map(|i| format!("function a{i}(b,c){{return b+c*{i}}}var x{i}=\"s{i}\";"))
            .collect();
        let mut buf = Buffer::new("t", &src);
        buf.language = "javascript".into();
        let mut sent: Option<kawoosh_doc::Version> = None;
        let mut job = |buf: &Buffer| {
            let edits = sent
                .and_then(|v| buf.journal().edits_since(v).ok())
                .map(|it| it.cloned().collect());
            sent = Some(buf.version());
            Job {
                buffer: BufferId::default(),
                language: "javascript".into(),
                snapshot: buf.snapshot(),
                edits,
            }
        };
        let first = highlight(&mut parser, &mut g, &mut parsed, &job(&buf)).update();
        buf.apply(first).unwrap();
        let at = buf.text().find("var x10000=").unwrap() + "var x10000=".len();
        // `true||` typed before a value and taken back again, a letter
        // at a time.
        let mut steps: Vec<(Range<usize>, &str)> = ["t", "r", "u", "e", "|", "|"]
            .into_iter()
            .enumerate()
            .map(|(i, l)| (at + i..at + i, l))
            .collect();
        steps.extend((0..6).rev().map(|i| (at + i..at + i + 1, "")));
        for (range, letter) in steps {
            buf.replace(range.clone(), letter);
            let inc = highlight(&mut parser, &mut g, &mut parsed, &job(&buf));
            for u in &inc.updates {
                assert!(
                    u.span.len() < 256,
                    "{range:?} {letter:?}: span {:?}",
                    u.span
                );
            }
            for u in inc.updates {
                buf.apply(u).unwrap();
            }
            let whole = highlight(
                &mut parser,
                &mut g,
                &mut Parsed::default(),
                &Job {
                    buffer: BufferId::default(),
                    language: "javascript".into(),
                    snapshot: buf.snapshot(),
                    edits: None,
                },
            )
            .update();
            let near = at - 200..at + 200;
            let clip = |runs: Vec<(Range<usize>, u32)>| -> Vec<(Range<usize>, u32)> {
                runs.into_iter()
                    .filter(|(r, _)| r.start < near.end && r.end > near.start)
                    .map(|(r, s)| (r.start.max(near.start)..r.end.min(near.end), s))
                    .collect()
            };
            assert_eq!(
                clip(joined(&buf.runs(SYNTAX_LAYER, 0..buf.len()))),
                clip(joined(&whole.runs)),
                "after {range:?} {letter:?}"
            );
        }
    }

    /// A format of a minified bundle — thousands of edits on its one
    /// line — is told the tree as their cover, not one at a time (a
    /// second on a megabyte), and answers as a whole parse does.
    #[test]
    fn a_format_of_one_long_line_answers_as_a_whole_parse() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let mut parsed = Parsed::default();
        let src: String = (0..3_000)
            .map(|i| format!("function a{i}(b,c){{return b+c*{i}}}var x{i}=\"s{i}\";"))
            .collect();
        let mut buf = Buffer::new("t", &src);
        buf.language = "javascript".into();
        let job = |buf: &Buffer, from: Option<kawoosh_doc::Version>| Job {
            buffer: BufferId::default(),
            language: "javascript".into(),
            snapshot: buf.snapshot(),
            edits: from.map(|v| buf.journal().edits_since(v).unwrap().cloned().collect()),
        };
        let first = highlight(&mut parser, &mut g, &mut parsed, &job(&buf, None)).update();
        buf.apply(first).unwrap();
        let v0 = buf.version();
        let edits: Vec<(Range<usize>, &str)> = src
            .match_indices([',', ';', '{', '}'])
            .map(|(i, _)| (i + 1..i + 1, " "))
            .collect();
        assert!(edits.len() > TELL_EACH_MAX);
        buf.replace_many(&edits);
        let inc = highlight(&mut parser, &mut g, &mut parsed, &job(&buf, Some(v0)));
        for u in inc.updates {
            buf.apply(u).unwrap();
        }
        let whole = highlight(
            &mut parser,
            &mut g,
            &mut Parsed::default(),
            &job(&buf, None),
        )
        .update();
        assert_eq!(
            joined(&buf.runs(SYNTAX_LAYER, 0..buf.len())),
            joined(&whole.runs)
        );
    }

    /// Past [`SPANS_MAX`], the smallest gaps close — chosen at once, the
    /// same spans as closing them one at a time, the leftmost of equal
    /// gaps first.
    #[test]
    fn spans_past_the_most_close_their_smallest_gaps() {
        fn one_at_a_time(mut out: Vec<Range<usize>>) -> Vec<Range<usize>> {
            while out.len() > SPANS_MAX {
                let (i, _) = out
                    .windows(2)
                    .enumerate()
                    .map(|(i, w)| (i, w[1].start - w[0].end))
                    .min_by_key(|(_, gap)| *gap)
                    .unwrap();
                out[i].end = out[i + 1].end;
                out.remove(i + 1);
            }
            out
        }
        // Gaps of 1 to 7 bytes, repeating, so many are equal.
        let mut spans = Vec::new();
        let mut at = 0;
        for i in 0..500 {
            spans.push(at..at + 3);
            at += 3 + 1 + (i * 5) % 7;
        }
        let merged = merge_spans(spans.clone());
        assert_eq!(merged.len(), SPANS_MAX);
        assert_eq!(merged, one_at_a_time(spans));
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
        assert_eq!(
            cover(&[e(1, 2, 1), e(3, 1, 2), e(0, 2, 1)]),
            Some((0..5, 4))
        );
        // Typing three chars in a row: old 3..3 became 3..6.
        assert_eq!(
            cover(&[e(3, 0, 1), e(4, 0, 1), e(5, 0, 1)]),
            Some((3..3, 6))
        );
        // Then backspacing four: one char before the typing went too.
        assert_eq!(
            cover(&[
                e(3, 0, 1),
                e(4, 0, 1),
                e(5, 0, 1),
                e(5, 1, 0),
                e(4, 1, 0),
                e(3, 1, 0),
                e(2, 1, 0)
            ]),
            Some((2..3, 2))
        );
    }

    /// A parse answers with its tree, at the snapshot's version; a
    /// language without a grammar with none.
    #[test]
    fn an_answer_carries_its_tree() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let buf = Buffer::new("t", "let x: number = 1;\n");
        let job = |language: &str| Job {
            buffer: BufferId::default(),
            language: language.into(),
            snapshot: buf.snapshot(),
            edits: None,
        };
        let a = highlight(
            &mut parser,
            &mut g,
            &mut Parsed::default(),
            &job("typescript"),
        );
        assert_eq!(a.version, buf.version());
        let tree = a.tree.expect("a tree");
        let root = tree.root_node();
        assert_eq!(root.kind(), "program");
        assert_eq!(root.byte_range(), 0..buf.len());
        assert!(!root.has_error());
        let a = highlight(
            &mut parser,
            &mut g,
            &mut Parsed::default(),
            &job("brainfuck"),
        );
        assert!(a.tree.is_none());
    }

    #[test]
    fn a_text_of_its_own_highlights_whole() {
        let mut g = Grammars::default();
        let mut parser = Parser::new();
        let job = TextJob {
            token: 7,
            language: "rust".into(),
            text: "fn main() { let x = \"s\"; }".into(),
        };
        let a = highlight_text(&mut parser, &mut g, &job);
        assert_eq!(a.token, 7);
        let kw = a
            .runs
            .iter()
            .find(|r| r.range == (0..2))
            .expect("`fn` a run");
        assert_eq!(Token::from_style(kw.style), Token::Keyword);
        assert!(
            a.runs
                .iter()
                .any(|r| Token::from_style(r.style) == Token::String),
            "{:?}",
            a.runs
        );
        let none = highlight_text(
            &mut parser,
            &mut g,
            &TextJob {
                token: 8,
                language: "no-such".into(),
                text: "x".into(),
            },
        );
        assert!(none.runs.is_empty());
    }

    #[test]
    fn unknown_language_answers_empty() {
        let mut g = Grammars::default();
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
