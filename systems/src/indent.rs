//! Indentation read off a syntax tree (docs/design/indent.md): a
//! grammar's indent query in helix's dialect, read relative to the text
//! (Decision 2) — a line is a level in from the line its innermost
//! indenting node starts on, as that line is indented, less a level
//! when it starts with an `@outdent` — so a file's own style stands,
//! and a construct the query misses costs its own lines, not the file's.
//!
//! Pure: a tree, its text, the query, the buffer's unit. The shell's
//! indenter keeps the trees (`kawoosh::indent`).

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use kawoosh_languages::{IndentKind, IndentScope, Indents};
use tree_sitter::{Node, QueryCursor, QueryPredicateArg, StreamingIterator, Tree};

/// One level of indentation as the buffer writes it: `text` (a tab, or
/// `width` spaces), `width` columns of it, a tab `tabstop` wide.
#[derive(Clone, Debug)]
pub struct Unit {
    pub text: String,
    pub width: usize,
    pub tabstop: usize,
}

impl Unit {
    fn tabs(&self) -> bool {
        self.text == "\t"
    }

    /// `cols` columns written as the buffer indents.
    fn write(&self, cols: usize) -> String {
        if self.tabs() {
            let ts = self.tabstop.max(1);
            "\t".repeat(cols / ts) + &" ".repeat(cols % ts)
        } else {
            " ".repeat(cols)
        }
    }

    /// How wide `indent` is.
    fn cols(&self, indent: &str) -> usize {
        let ts = self.tabstop.max(1);
        indent.chars().fold(0, |c, ch| match ch {
            '\t' => c - c % ts + ts,
            _ => c + 1,
        })
    }

    /// `indent` moved `by` levels: a level in appended as the unit, a
    /// level out `width` columns off, written back as the buffer does.
    fn shift(&self, indent: &str, by: isize) -> String {
        if by >= 0 {
            return indent.to_string() + &self.text.repeat(by as usize);
        }
        let off = self.width.max(1) * by.unsigned_abs();
        self.write(self.cols(indent).saturating_sub(off))
    }
}

/// The indent for a line break inserted at byte `at` of `text` — the
/// new line holding what was after `at` — or `None` when the tree
/// cannot say (no node reaches there).
pub fn for_new_line(
    ind: &Indents,
    tree: &Tree,
    text: &text_buffer::Buffer,
    at: usize,
    unit: &Unit,
) -> Option<String> {
    let line = text.line_of_offset(at.min(text.len()));
    let r = Reader::new(
        ind,
        tree,
        Text::new(text, line.saturating_sub(SIBLING_LINES), line),
    );
    let level = r.level(line + 1, at, true)?;
    Some(r.write(level, line + 1, Some(at), &|ln| r.text.indent(ln), unit))
}

/// The indent each of `lines` should have, top down, each against the
/// lines above as they will be (`=`); a blank line's is empty. `None`
/// for a line the tree cannot say, which keeps its own.
pub fn for_lines(
    ind: &Indents,
    tree: &Tree,
    text: &text_buffer::Buffer,
    lines: Range<usize>,
    unit: &Unit,
) -> Vec<Option<String>> {
    let last = lines.end.saturating_sub(1).max(lines.start);
    let r = Reader::new(
        ind,
        tree,
        Text::new(text, lines.start.saturating_sub(SIBLING_LINES), last),
    );
    let t = &r.text;
    let mut new: HashMap<usize, String> = HashMap::new();
    let mut out = Vec::with_capacity(lines.len());
    for ln in lines {
        let Some(first) = t.first_nonblank(ln) else {
            new.insert(ln, String::new());
            out.push(Some(String::new()));
            continue;
        };
        // A line whose start is inside a string or a comment begun
        // above is the text's, not the indent's.
        let got = if r.starts_inside(ln, first) {
            None
        } else {
            r.line_level(ln, first).map(|level| {
                let indent = |b: usize| new.get(&b).cloned().unwrap_or_else(|| t.indent(b));
                r.write(level, ln, None, &indent, unit)
            })
        };
        if let Some(s) = &got {
            new.insert(ln, s.clone());
        }
        out.push(got);
    }
    out
}

/// The text's lines as the reader wants them: a region's read once
/// and indexed here — the piece tree finds a line by scanning its piece
/// for newlines, and `=` over a file asks for every line — the buffer
/// itself asked for one outside it.
struct Text<'a> {
    buf: &'a text_buffer::Buffer,
    /// The region's first line, and the byte it starts at.
    first: usize,
    base: usize,
    bytes: Vec<u8>,
    /// Each line's start in `bytes`.
    starts: Vec<usize>,
}

impl<'a> Text<'a> {
    /// Lines `first..=last` read.
    fn new(buf: &'a text_buffer::Buffer, first: usize, last: usize) -> Self {
        let last = last.min(buf.line_count().saturating_sub(1)).max(first);
        let base = buf.get_line_range(first).map_or(buf.len(), |r| r.start);
        let end = buf.get_line_range(last).map_or(buf.len(), |r| r.end);
        let bytes = buf.collect_range(base..end.max(base));
        let starts = std::iter::once(0)
            .chain(memchr::memchr_iter(b'\n', &bytes).map(|i| i + 1))
            .collect();
        Self {
            buf,
            first,
            base,
            bytes,
            starts,
        }
    }

    /// The region's bytes.
    fn span(&self) -> Range<usize> {
        self.base..self.base + self.bytes.len()
    }

    fn line_of(&self, at: usize) -> usize {
        if self.span().contains(&at) || at == self.span().end {
            return self.first + self.starts.partition_point(|s| self.base + s <= at) - 1;
        }
        self.buf.line_of_offset(at.min(self.buf.len()))
    }

    fn range(&self, ln: usize) -> Range<usize> {
        match ln
            .checked_sub(self.first)
            .filter(|i| *i < self.starts.len())
        {
            Some(i) => {
                let end = self.starts.get(i + 1).map_or(self.bytes.len(), |n| n - 1);
                self.base + self.starts[i]..self.base + end
            }
            None => self
                .buf
                .get_line_range(ln)
                .unwrap_or(self.buf.len()..self.buf.len()),
        }
    }

    fn line_start(&self, ln: usize) -> usize {
        self.range(ln).start
    }

    /// The bytes of `r`, off the region when it holds them.
    fn slice(&self, r: Range<usize>) -> std::borrow::Cow<'_, [u8]> {
        let span = self.span();
        if span.start <= r.start && r.end <= span.end {
            std::borrow::Cow::Borrowed(&self.bytes[r.start - self.base..r.end - self.base])
        } else {
            std::borrow::Cow::Owned(self.buf.collect_range(r))
        }
    }

    /// The line's leading blanks.
    fn indent(&self, ln: usize) -> String {
        let b = self.slice(self.range(ln));
        let n = b.iter().take_while(|c| **c == b' ' || **c == b'\t').count();
        String::from_utf8_lossy(&b[..n]).into_owned()
    }

    /// The byte of the line's first non-blank, if it has one.
    fn first_nonblank(&self, ln: usize) -> Option<usize> {
        let r = self.range(ln);
        let b = self.slice(r.clone());
        b.iter()
            .position(|c| !c.is_ascii_whitespace())
            .map(|i| r.start + i)
    }
}

/// How far above a line its sibling is looked for.
const SIBLING_LINES: usize = 200;

/// What the tree says of a line.
#[derive(Clone)]
enum Level {
    /// Levels in from the line a node around it starts on.
    From(usize, isize),
    /// Levels in from the margin: nothing around it indents.
    Margin(isize),
    /// Lined up under a column (`@align`): the anchor line's text up to
    /// it, blanked, and levels in from there.
    Align(String, isize),
}

/// The captures that touch nodes, per node.
#[derive(Default)]
struct Caps {
    indent: HashMap<usize, Vec<(IndentKind, IndentScope)>>,
    /// The `@align` node's id → its `@anchor`'s start.
    align: HashMap<usize, usize>,
    extend: HashMap<usize, bool>,
    prevent: HashMap<usize, bool>,
}

struct Reader<'a> {
    tree: &'a Tree,
    text: Text<'a>,
    caps: Caps,
    /// Each line's level as a line of its own, once read: a line is
    /// read again as a sibling of the lines below it.
    lines: std::cell::RefCell<HashMap<usize, Option<Level>>>,
    /// Each broken node's brackets, by id, once read.
    brackets: std::cell::RefCell<HashMap<usize, Rc<Brackets>>>,
}

impl<'a> Reader<'a> {
    /// The captures of every match reaching into the text's region whose
    /// predicates hold.
    fn new(ind: &Indents, tree: &'a Tree, text: Text<'a>) -> Self {
        let mut caps = Caps::default();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(text.span());
        let buf = text.buf;
        let mut node_text = |n: Node| std::iter::once(buf.collect_range(n.byte_range()));
        let mut it = cursor.matches(&ind.query, tree.root_node(), &mut node_text);
        while let Some(m) = it.next() {
            let holds = ind
                .query
                .general_predicates(m.pattern_index)
                .iter()
                .all(|p| {
                    let nodes = |i: usize| -> Vec<Node> {
                        match p.args.get(i) {
                            Some(QueryPredicateArg::Capture(c)) => {
                                m.nodes_for_capture_index(*c).collect()
                            }
                            _ => Vec::new(),
                        }
                    };
                    let word = |i: usize| match p.args.get(i) {
                        Some(QueryPredicateArg::String(s)) => Some(s.as_ref()),
                        _ => None,
                    };
                    let row = |n: &Node| n.start_position().row;
                    let one_line = |n: &Node| n.start_position().row == n.end_position().row;
                    match p.operator.as_ref() {
                        "not-kind-eq?" => nodes(0).iter().all(|n| Some(n.kind()) != word(1)),
                        "same-line?" | "not-same-line?" => {
                            let same = nodes(0)
                                .iter()
                                .all(|a| nodes(1).iter().all(|b| row(a) == row(b)));
                            same == (p.operator.as_ref() == "same-line?")
                        }
                        "one-line?" => nodes(0).iter().all(one_line),
                        "not-one-line?" => nodes(0).iter().all(|n| !one_line(n)),
                        _ => true,
                    }
                });
            if !holds {
                continue;
            }
            let scope = ind.scopes.get(m.pattern_index).copied().flatten();
            let anchor = m
                .captures()
                .iter()
                .find(|c| ind.kinds.get(c.index as usize) == Some(&Some(IndentKind::Anchor)))
                .map(|c| c.node.start_byte());
            for c in m.captures() {
                let Some(Some(kind)) = ind.kinds.get(c.index as usize) else {
                    continue;
                };
                let id = c.node.id();
                match kind {
                    IndentKind::Indent
                    | IndentKind::IndentAlways
                    | IndentKind::Outdent
                    | IndentKind::OutdentAlways => caps
                        .indent
                        .entry(id)
                        .or_default()
                        .push((*kind, scope.unwrap_or(kind.default_scope()))),
                    IndentKind::Align => {
                        if let Some(a) = anchor {
                            caps.align.entry(id).or_insert(a);
                        }
                    }
                    IndentKind::Anchor => {}
                    IndentKind::Extend => {
                        caps.extend.insert(id, true);
                    }
                    IndentKind::ExtendPreventOnce => {
                        caps.prevent.insert(id, true);
                    }
                }
            }
        }
        Self {
            tree,
            text,
            caps,
            lines: Default::default(),
            brackets: Default::default(),
        }
    }

    /// Whether byte `first` of line `ln` is inside a node begun on an
    /// earlier line that has no children — a string's or a comment's
    /// middle, which no indent reaches.
    fn starts_inside(&self, ln: usize, first: usize) -> bool {
        let Some(n) = self
            .tree
            .root_node()
            .descendant_for_byte_range(first, first)
        else {
            return false;
        };
        n.start_byte() < self.text.line_start(ln) && n.child_count() == 0
    }

    /// What the tree says of line `target`: a line break at `at` when
    /// `new_line` (the new line is `target`), else the line itself, its
    /// first non-blank at `at`. The nodes starting the line count on it
    /// (an `@outdent` a level out, an `@indent` scoped `all` a level
    /// in); above them, the innermost node begun on an earlier line
    /// with an `@indent` — with the others begun on its line, counted
    /// once — puts the line a level in from the line it starts on; an
    /// `@align` met first lines it up instead.
    fn level(&self, target: usize, at: usize, new_line: bool) -> Option<Level> {
        let root = self.tree.root_node();
        let nl = new_line.then_some(at);
        let lowest = root.descendant_for_byte_range(at, at)?;
        let lowest = unfinished(lowest, at).unwrap_or(lowest);
        let node = self.extend(lowest, at);
        // The node and its ancestors, root first — walked down once:
        // tree-sitter's `parent` walks down from the root each time,
        // and a file's thousand-line `impl` made that quadratic.
        let mut chain = chain(root, node);
        let start_line = |n: Node| {
            let row = n.start_position().row;
            match nl {
                Some(at) if n.start_byte() >= at => row + 1,
                _ => row,
            }
        };
        let mut this_line = Acc::default();
        let mut from: Option<(usize, Acc)> = None;
        // Whether an ERROR is around the line, or before it under the
        // same node: tree-sitter's recovery from text it cannot fit,
        // the tokens loose.
        let mut broken = false;
        while let Some(node) = chain.pop() {
            // A MISSING token is the parser's guess, not the text: the
            // `}` it closes an unclosed block with, at the caret, starts
            // no line.
            if node.is_missing() {
                continue;
            }
            let line = start_line(node);
            let caps = self.caps.indent.get(&node.id()).map_or(&[][..], |v| &v[..]);
            // A broken tree's brackets are what is left of its structure:
            // an opener left open before the line — the ERROR's around
            // it, or an ERROR's before it, which ends at its last token
            // however far its brackets reach — is a node begun on the
            // opener's line with an `@indent`, inside the node.
            if node.has_error() && line < target {
                let b = self.brackets(node);
                broken |= b.errors_from <= at;
                let top = b.open(at);
                match &mut from {
                    None => {
                        if let Some(top) = top {
                            let mut acc = Acc::default();
                            acc.add(IndentKind::Indent);
                            from = Some((top.row, acc));
                        }
                    }
                    Some((l, acc)) => {
                        if b.opens_on(top, *l) {
                            acc.add(IndentKind::Indent);
                        }
                    }
                }
            }
            if let Some((l, acc)) = &mut from {
                if line != *l {
                    break;
                }
                for (kind, _) in caps {
                    if matches!(kind, IndentKind::Indent | IndentKind::IndentAlways) {
                        acc.add(*kind);
                    }
                }
                continue;
            }
            if line >= target {
                if self.first_on_line(node, nl) {
                    for (kind, scope) in caps {
                        if *scope == IndentScope::All {
                            this_line.add(*kind);
                        }
                    }
                }
                continue;
            }
            // A node the target is a later line of, lined up: the
            // anchor's column, and what starts the line counted from
            // there.
            if let Some(&anchor) = self.caps.align.get(&node.id())
                && self.text.line_of(anchor) == node.start_position().row
            {
                let row = node.start_position().row;
                let upto = self.text.slice(self.text.line_start(row)..anchor);
                let blanked: String = String::from_utf8_lossy(&upto)
                    .chars()
                    .map(|c| if c == '\t' { '\t' } else { ' ' })
                    .collect();
                return Some(Level::Align(blanked, this_line.net()));
            }
            let mut acc = Acc::default();
            for (kind, _) in caps {
                if matches!(kind, IndentKind::Indent | IndentKind::IndentAlways) {
                    acc.add(*kind);
                }
            }
            if acc.net() > 0 {
                from = Some((line, acc));
            }
        }
        match from {
            Some((line, acc)) => Some(Level::From(line, acc.net() + this_line.net())),
            // Nothing indents, but the tree is broken around the line:
            // not the margin's word but the tree's silence — the
            // caller's own rule says (Decision 3's fallback).
            None if broken => None,
            None => Some(Level::Margin(this_line.net())),
        }
    }

    /// `node`'s open brackets, read once.
    fn brackets(&self, node: Node) -> Rc<Brackets> {
        self.brackets
            .borrow_mut()
            .entry(node.id())
            .or_insert_with(|| Rc::new(Brackets::of(node)))
            .clone()
    }

    /// The level of line `ln` as it stands, its first non-blank at
    /// `first`.
    fn line_level(&self, ln: usize, first: usize) -> Option<Level> {
        if let Some(l) = self.lines.borrow().get(&ln) {
            return l.clone();
        }
        let l = self.level(ln, first, false);
        self.lines.borrow_mut().insert(ln, l.clone());
        l
    }

    /// Whether nothing but blanks is before `n` on its line — the line
    /// break at `nl`, when `n` is after it, starting a line of its own.
    fn first_on_line(&self, n: Node, nl: Option<usize>) -> bool {
        let start = n.start_byte();
        let mut from = start - n.start_position().column;
        if let Some(at) = nl
            && start >= at
        {
            from = from.max(at);
        }
        self.text
            .slice(from..start)
            .iter()
            .all(|b| b.is_ascii_whitespace())
    }

    /// `@extend`: the node right before `at`, or one of its ancestors
    /// under `node`, reaches over the line `at` is on when that line
    /// is where it ends or is indented past its first — the innermost
    /// such, an `@extend.prevent-once` on the way skipping the next.
    fn extend(&self, node: Node<'a>, at: usize) -> Node<'a> {
        if self.caps.extend.is_empty() {
            return node;
        }
        let Some(d) = deepest_preceding(node, at) else {
            return node;
        };
        let line = self.text.line_of(at);
        let cols = |ln: usize| self.text.indent(ln).len();
        let mut prevent = false;
        // From `d` up to `node`, walked down once from `node`.
        let mut up = chain(node, d);
        while let Some(d) = up.pop() {
            if d.id() == node.id() {
                break;
            }
            if self.caps.prevent.contains_key(&d.id()) {
                prevent = true;
            }
            if self.caps.extend.contains_key(&d.id()) {
                if prevent {
                    prevent = false;
                } else if d.end_position().row == line || cols(line) > cols(d.start_position().row)
                {
                    return d;
                }
            }
        }
        node
    }

    /// A level as the text's indent for line `target` (`indent` says
    /// a line's): from the nearest line above it under the same node —
    /// a sibling, at the indent it has, so a file's own width stands —
    /// else a level in from the node's line; `at` is the line break's
    /// when `target` is the line after it. A line out from its sibling
    /// (a closer, an `elseif`) is read from the node's line: a sibling
    /// less a unit is the unit's width, not the file's.
    fn write(
        &self,
        level: Level,
        target: usize,
        at: Option<usize>,
        indent: &dyn Fn(usize) -> String,
        unit: &Unit,
    ) -> String {
        let (from, n) = match level {
            Level::From(line, n) => (line, n),
            Level::Margin(n) => return unit.shift("", n.max(0)),
            Level::Align(prefix, n) => return unit.shift(&prefix, n.max(0)),
        };
        let mut b = target;
        for _ in 0..SIBLING_LINES {
            b -= 1;
            if b <= from {
                break;
            }
            let Some(first) = self.text.first_nonblank(b) else {
                continue;
            };
            if at.is_some_and(|at| first >= at) || self.starts_inside(b, first) {
                continue;
            }
            if let Some(Level::From(l, m)) = self.line_level(b, first)
                && l == from
            {
                if n < m {
                    break;
                }
                return unit.shift(&indent(b), n - m);
            }
        }
        unit.shift(&indent(from), n)
    }
}

/// The innermost node under `node` left unfinished before `at`: one
/// whose last child there is MISSING — the closer the parser guessed,
/// zero-wide where the text stopped — reaches over what follows it as
/// an unclosed block does, though it ends before `at`.
fn unfinished<'t>(node: Node<'t>, at: usize) -> Option<Node<'t>> {
    let mut n = node;
    loop {
        let c = (0..n.child_count())
            .rev()
            .filter_map(|i| n.child(i))
            .find(|c| c.end_byte() <= at)?;
        if c.is_missing() {
            return Some(n);
        }
        if !c.has_error() {
            return None;
        }
        n = c;
    }
}

/// The deepest node under `node` that ends at or before `at`: its last
/// child that does, and that one's last descendant.
fn deepest_preceding(node: Node, at: usize) -> Option<Node> {
    let mut cursor = node.walk();
    let mut d = node
        .children(&mut cursor)
        .filter(|c| c.end_byte() <= at && c.end_byte() > c.start_byte())
        .last()?;
    while let Some(last) = (0..d.child_count())
        .rev()
        .filter_map(|i| d.child(i))
        .find(|c| c.end_byte() > c.start_byte())
    {
        d = last;
    }
    Some(d)
}

/// The brackets a node's ERRORs leave open, read once for every line
/// under it — a walk over the node's children up to each line was
/// quadratic down a long flat array.
struct Brackets {
    /// From which byte the node is or holds an ERROR before a line: 0
    /// for an ERROR, else its first ERROR child's end, else never.
    errors_from: usize,
    /// Each loose bracket in the text's order: the byte from which it
    /// counts (its end, or its ERROR child's) and the top of the stack
    /// of brackets open after it.
    steps: Vec<(usize, Option<usize>)>,
    /// That stack, shared by the steps.
    frames: Vec<Frame>,
}

/// An open bracket: its row, the one below it, and per kind the nearest
/// at or below it — the one a closer of that kind closes.
struct Frame {
    row: usize,
    below: Option<usize>,
    nearest: [Option<usize>; 3],
}

impl Brackets {
    /// Under `node`: an ERROR's loose `(` `[` `{` less the closers after
    /// them — an ERROR inside one read as part of it — and any other
    /// node whole, its brackets its own.
    fn of(node: Node) -> Self {
        let mut cursor = node.walk();
        let errors_from = if node.is_error() {
            0
        } else {
            node.children(&mut cursor)
                .find(|c| c.is_error())
                .map_or(usize::MAX, |c| c.end_byte())
        };
        let mut b = Brackets {
            errors_from,
            steps: Vec::new(),
            frames: Vec::new(),
        };
        b.scan(node, node.is_error(), None, &mut None);
        b
    }

    /// `n`'s children onto the stack from `top`; `whole`, inside a
    /// child ERROR, the byte all of it counts from.
    fn scan(&mut self, n: Node, loose: bool, whole: Option<usize>, top: &mut Option<usize>) {
        let mut cursor = n.walk();
        for c in n.children(&mut cursor) {
            let from = whole.unwrap_or(c.end_byte());
            if c.is_error() {
                self.scan(c, true, Some(from), top);
                continue;
            }
            if !loose || c.is_named() || c.is_missing() {
                continue;
            }
            // Each bracket's kind, and whether it opens.
            let (kind, opens) = match c.kind() {
                "(" => (0, true),
                "[" => (1, true),
                "{" => (2, true),
                ")" => (0, false),
                "]" => (1, false),
                "}" => (2, false),
                _ => continue,
            };
            if opens {
                let mut nearest = top.map_or([None; 3], |t| self.frames[t].nearest);
                nearest[kind] = Some(self.frames.len());
                self.frames.push(Frame {
                    row: c.start_position().row,
                    below: *top,
                    nearest,
                });
                *top = Some(self.frames.len() - 1);
            } else if let Some(f) = top.and_then(|t| self.frames[t].nearest[kind]) {
                *top = self.frames[f].below;
            } else {
                continue;
            }
            self.steps.push((from, *top));
        }
    }

    /// The innermost bracket left open before `at`.
    fn open(&self, at: usize) -> Option<&Frame> {
        let i = self.steps.partition_point(|(from, _)| *from <= at);
        let top = i.checked_sub(1).and_then(|i| self.steps[i].1)?;
        Some(&self.frames[top])
    }

    /// Whether `top` or a bracket open below it is on `row`: the rows
    /// only fall going down.
    fn opens_on(&self, top: Option<&Frame>, row: usize) -> bool {
        let mut f = top;
        while let Some(fr) = f {
            if fr.row <= row {
                return fr.row == row;
            }
            f = fr.below.map(|b| &self.frames[b]);
        }
        false
    }
}

/// `target` and the nodes above it up to `top`, `top` first: from
/// `top` down, each step the child that holds `target`'s range, found
/// by the cursor's skip to the byte.
fn chain<'t>(top: Node<'t>, target: Node<'t>) -> Vec<Node<'t>> {
    let (from, to) = (target.start_byte(), target.end_byte());
    let holds = |c: Node| {
        c.id() == target.id()
            || (c.start_byte() <= from && c.end_byte() >= to && c.end_byte() > c.start_byte())
    };
    let mut out = vec![top];
    let mut cursor = top.walk();
    while cursor.node().id() != target.id() {
        if cursor.goto_first_child_for_byte(from).is_none() {
            break;
        }
        while !holds(cursor.node()) {
            if !cursor.goto_next_sibling() {
                return out;
            }
        }
        out.push(cursor.node());
    }
    out
}

/// What the captures on one line add up to: an `@indent` and an
/// `@outdent` count once however many share the line, the `.always`
/// kinds each time.
#[derive(Clone, Copy, Default)]
struct Acc {
    indent: isize,
    indent_always: isize,
    outdent: isize,
    outdent_always: isize,
}

impl Acc {
    fn add(&mut self, kind: IndentKind) {
        match kind {
            IndentKind::Indent if self.indent_always == 0 => self.indent = 1,
            IndentKind::IndentAlways => {
                self.indent = 0;
                self.indent_always += 1;
            }
            IndentKind::Outdent if self.outdent_always == 0 => self.outdent = 1,
            IndentKind::OutdentAlways => {
                self.outdent = 0;
                self.outdent_always += 1;
            }
            _ => {}
        }
    }

    fn net(&self) -> isize {
        self.indent + self.indent_always - self.outdent - self.outdent_always
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lang: &str, src: &str) -> (kawoosh_languages::Grammar, Tree, text_buffer::Buffer) {
        let l = kawoosh_languages::LANGUAGES
            .iter()
            .find(|l| l.name == lang)
            .unwrap_or_else(|| panic!("no {lang}"));
        let g = (l.grammar.expect("a grammar"))().unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&g.language).unwrap();
        let tree = parser.parse(src, None).unwrap();
        (g, tree, text_buffer::Buffer::with_text(src.as_bytes()))
    }

    fn spaces() -> Unit {
        Unit {
            text: "    ".into(),
            width: 4,
            tabstop: 4,
        }
    }

    /// The indent of a line break at `|`.
    fn new_line(lang: &str, marked: &str) -> String {
        new_line_in(lang, marked, &spaces())
    }

    fn new_line_in(lang: &str, marked: &str, unit: &Unit) -> String {
        let at = marked.find('|').expect("a |");
        let src = marked.replacen('|', "", 1);
        let (g, tree, text) = parse(lang, &src);
        for_new_line(g.indents.as_ref().unwrap(), &tree, &text, at, unit).expect("an answer")
    }

    /// Every line reindented.
    fn reindent(lang: &str, src: &str) -> String {
        let (g, tree, text) = parse(lang, src);
        let n = text.line_count();
        let got = for_lines(g.indents.as_ref().unwrap(), &tree, &text, 0..n, &spaces());
        src.split('\n')
            .zip(got)
            .map(|(l, i)| match i {
                Some(i) if !l.trim().is_empty() => i + l.trim_start(),
                Some(_) => String::new(),
                None => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn rust_blocks() {
        assert_eq!(new_line("rust", "fn f() {|\n}\n"), "    ", "o after {{");
        assert_eq!(
            new_line("rust", "fn f() {\n    x|\n}\n"),
            "    ",
            "O before }}"
        );
        assert_eq!(new_line("rust", "fn f() {\n    x\n}|\n"), "", "after }}");
        assert_eq!(
            new_line("rust", "fn f() {|}\n"),
            "",
            "between: the closer's line"
        );
        assert_eq!(
            new_line(
                "rust",
                "fn f() {\n    if a {\n        b\n    } else {|\n    }\n}\n"
            ),
            "        ",
            "}} else {{"
        );
        assert_eq!(
            new_line("rust", "fn f() {\n    match x {|\n    }\n}\n"),
            "        "
        );
        assert_eq!(
            new_line(
                "rust",
                "fn f() {\n    match x {\n        A => {|\n        }\n    }\n}\n"
            ),
            "            "
        );
        assert_eq!(
            new_line("rust", "fn f() {\n    g(\n        a,|\n    );\n}\n"),
            "        ",
            "in an argument list"
        );
    }

    /// An unclosed item is an ERROR of loose tokens, no `block` for the
    /// query to match: its brackets are what is left of the structure.
    #[test]
    fn unclosed_brackets_in_a_broken_tree() {
        assert_eq!(
            new_line("rust", "fn f() {|"),
            "    ",
            "a new file's first {{"
        );
        // Where the grammar closes the block with a MISSING `}` instead,
        // the guessed `}` starts no line.
        assert_eq!(new_line("rust", "struct A {|"), "    ");
        assert_eq!(new_line("javascript", "function f() {|"), "    ");
        assert_eq!(new_line("c", "int f() {|"), "    ");
        assert_eq!(new_line("css", "a {|"), "    ");
        assert_eq!(new_line("nu", "def f [] {|"), "    ");
        assert_eq!(new_line("rust", "fn f() {|\n"), "    ");
        assert_eq!(
            new_line("rust", "impl A {\n    fn f() {|\n"),
            "        ",
            "the innermost unclosed"
        );
        assert_eq!(
            new_line("rust", "fn f() {\n  let x = 1;|\n"),
            "  ",
            "a sibling's indent"
        );
        assert_eq!(
            new_line(
                "rust",
                "fn f() {\n}\nfn g() {\n    if a {\n        b();\n    }|\n"
            ),
            "    ",
            "a closed pair in the broken part"
        );
        assert_eq!(
            reindent("rust", "impl A {\nfn f() {\nx\n}\n"),
            "impl A {\n    fn f() {\n        x\n    }\n"
        );
        // Nothing open and nothing indenting in a broken tree: the tree
        // cannot say (the caller's own rule runs), not the margin.
        let src = "fn f() {}\n  x y z\n";
        let (g, tree, text) = parse("rust", src);
        assert!(
            tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
        let at = src.len() - 1;
        let ind = g.indents.as_ref().unwrap();
        assert_eq!(for_new_line(ind, &tree, &text, at, &spaces()), None);
        // The line the ERROR starts on is read from what is around it,
        // sound: the top level.
        assert_eq!(
            for_lines(ind, &tree, &text, 1..2, &spaces()),
            vec![Some(String::new())]
        );
    }

    #[test]
    fn relative_to_the_line_above() {
        assert_eq!(
            new_line("rust", "fn f() {\n  x|\n}\n"),
            "  ",
            "a two-space file stays two"
        );
        assert_eq!(
            new_line("rust", "fn f() {\n  if a {|\n  }\n}\n"),
            "      ",
            "and a level is the unit"
        );
        assert_eq!(
            new_line("rust", "fn f() {\n\tx|\n}\n"),
            "\t",
            "a tab stays a tab"
        );
        let tabs = Unit {
            text: "\t".into(),
            width: 4,
            tabstop: 4,
        };
        assert_eq!(new_line_in("go", "func f() {|\n}\n", &tabs), "\t");
        assert_eq!(
            new_line_in("go", "func f() {\n\tif a {\n\t\tb|\n\t}\n}\n", &tabs),
            "\t\t"
        );
    }

    /// A closer lines up with the line its block opened on, not a
    /// sibling's indent less a unit the file does not use.
    #[test]
    fn a_closer_lines_up_with_its_opener() {
        let two = Unit {
            text: "  ".into(),
            width: 2,
            tabstop: 4,
        };
        let line = |lang: &str, src: &str, ln: usize, unit: &Unit| {
            let (g, tree, text) = parse(lang, src);
            for_lines(g.indents.as_ref().unwrap(), &tree, &text, ln..ln + 1, unit)
                .remove(0)
                .expect("an answer")
        };
        let src = "function f() {\n    if (a) {\n        b();\n    }\n}\n";
        assert_eq!(
            line("javascript", src, 3, &two),
            "    ",
            "`==` in a four-space file under a unit of two"
        );
        assert_eq!(
            new_line_in(
                "javascript",
                "function f() {\n    if (a) {\n        b();|}\n}\n",
                &two
            ),
            "    ",
            "`<CR>` before the closer"
        );
        let src = "fn f() {\n  if a {\n    b;\n  }\n}\n";
        assert_eq!(
            line("rust", src, 3, &spaces()),
            "  ",
            "a two-space file under a unit of four"
        );
        let src = "if a then\n  b\nelseif c then\n  d\nend\n";
        assert_eq!(line("lua", src, 2, &spaces()), "", "Lua's `elseif`");
    }

    #[test]
    fn python_blocks_end_by_indentation() {
        assert_eq!(new_line("python", "def f():|\n    pass\n"), "    ");
        assert_eq!(
            new_line("python", "def f():\n    x = 1|\n"),
            "    ",
            "still in the body"
        );
        assert_eq!(
            new_line("python", "def f():\n    return 1|\n"),
            "",
            "out after a return"
        );
        assert_eq!(
            new_line("python", "class A:\n    def f(self):\n        return 1|\n"),
            "    ",
            "out of the method, in the class"
        );
        assert_eq!(
            new_line("python", "if a:\n    b = 1\nelse:|\n    c\n"),
            "    "
        );
        assert_eq!(new_line("python", "x = [|\n]\n"), "    ");
        assert_eq!(
            new_line("python", "foo(a,|\n    b)\n"),
            "    ",
            "lined up under the first argument"
        );
        assert_eq!(
            new_line("python", "xs = foo(a,|\n         b)\n"),
            "         "
        );
    }

    #[test]
    fn other_languages() {
        assert_eq!(new_line("lua", "function f()|\nend\n"), "    ");
        assert_eq!(new_line("lua", "if a then|\nend\n"), "    ");
        assert_eq!(new_line("javascript", "function f() {|\n}\n"), "    ");
        assert_eq!(new_line("javascript", "const a = {|\n};\n"), "    ");
        assert_eq!(new_line("javascript", "foo(() => {|\n});\n"), "    ");
        assert_eq!(new_line("typescript", "interface A {|\n}\n"), "    ");
        assert_eq!(
            new_line("tsx", "const a = (\n    <div>|\n    </div>\n);\n"),
            "        "
        );
        assert_eq!(new_line("json", "{|\n}\n"), "    ");
        assert_eq!(new_line("json", "{\n    \"a\": [|\n    ]\n}\n"), "        ");
        assert_eq!(new_line("c", "int f() {|\n}\n"), "    ");
        assert_eq!(new_line("cpp", "class A {|\n};\n"), "    ");
        assert_eq!(new_line("bash", "if a; then|\nfi\n"), "    ");
        assert_eq!(new_line("toml", "a = [|\n]\n"), "    ");
        assert_eq!(new_line("css", "a {|\n}\n"), "    ");
        assert_eq!(new_line("yaml", "a:\n  b: 1|\n"), "  ");
        assert_eq!(new_line("nu", "def f [] {|\n}\n"), "    ");
        // scheme's query is written for two-space levels.
        let two = Unit {
            text: "  ".into(),
            width: 2,
            tabstop: 4,
        };
        assert_eq!(new_line_in("scheme", "(define (f x)|\n  x)\n", &two), "  ");
        assert_eq!(
            new_line_in("scheme", "(foo a|\n     b)\n", &two),
            "     ",
            "under the second"
        );
    }

    #[test]
    fn a_range_reindented() {
        let src = "fn f() {\nx;\n        if a {\ny;\n    }\n\n  }\n";
        assert_eq!(
            reindent("rust", src),
            "fn f() {\n    x;\n    if a {\n        y;\n    }\n\n}\n"
        );
        // An `if` hugging a call's parens: its `else` branch a level in
        // from the `} else {` line, not two from the margin.
        let src = "fn f() {\nSome(if a {\nb\n} else {\nc\n})\n}\n";
        assert_eq!(
            reindent("rust", src),
            "fn f() {\n    Some(if a {\n        b\n    } else {\n        c\n    })\n}\n"
        );
        // Lua's `elseif` and `else` at their `if`'s level.
        let src = "if a then\nb\nelseif c then\nd\nelse\ne\nend\n";
        assert_eq!(
            reindent("lua", src),
            "if a then\n    b\nelseif c then\n    d\nelse\n    e\nend\n"
        );
        // A raw string's lines are the string's.
        let src = "fn f() {\nlet s = r\"a\n  b\";\n}\n";
        assert_eq!(
            reindent("rust", src),
            "fn f() {\n    let s = r\"a\n  b\";\n}\n"
        );
    }

    /// This repository's own sources, formatted by hand or by rustfmt,
    /// reindented whole: the lines `=` would move, past those inside a
    /// macro, a string or a comment (which the tree cannot read), stay
    /// a sliver — rustfmt's Rust 0.14% and the hand-kept Lua 1% when
    /// written.
    /// A `switch`'s `default` sits where its `case`s do: helix's query
    /// outdents the one and not the other, and `default:` was a level
    /// in from its siblings.
    #[test]
    fn a_default_lines_up_with_its_cases() {
        let src = "void f(int n) {\n    switch (n) {\n    case 1:\n        g();\n        break;\n    default:\n        h();\n    }\n}\n";
        assert_eq!(reindent("c", src), src);
        assert_eq!(reindent("cpp", src), src);
    }

    #[test]
    fn this_repository_reindents_as_it_is() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let moved = |lang: &str, ext: &str, dirs: &[&str], unit: &Unit| {
            let (mut lines, mut moved) = (0, 0);
            for d in dirs {
                for e in std::fs::read_dir(format!("{root}/{d}")).unwrap() {
                    let p = e.unwrap().path();
                    if p.extension().is_none_or(|x| x != ext) {
                        continue;
                    }
                    let src = std::fs::read_to_string(&p).unwrap();
                    let (g, tree, text) = parse(lang, &src);
                    let n = text.line_count();
                    let got = for_lines(g.indents.as_ref().unwrap(), &tree, &text, 0..n, unit);
                    let mut at = 0;
                    for (line, want) in src.split('\n').zip(got) {
                        let start = at;
                        at += line.len() + 1;
                        if line.trim().is_empty() {
                            continue;
                        }
                        let own = line.len() - line.trim_start_matches([' ', '\t']).len();
                        let first = start + own;
                        let opaque = std::iter::successors(
                            tree.root_node().descendant_for_byte_range(first, first),
                            |n| n.parent(),
                        )
                        .any(|n| {
                            matches!(
                                n.kind(),
                                "token_tree"
                                    | "string_literal"
                                    | "raw_string_literal"
                                    | "line_comment"
                                    | "block_comment"
                                    | "comment"
                                    | "string"
                            )
                        });
                        if opaque {
                            continue;
                        }
                        lines += 1;
                        if want.is_some_and(|w| w != line[..own]) {
                            moved += 1;
                        }
                    }
                }
            }
            (moved, lines)
        };
        let (m, n) = moved(
            "rust",
            "rs",
            &[
                "editor/src",
                "kawoosh/src",
                "systems/src",
                "languages/src",
                "doc/src",
                "text-buffer/src",
            ],
            &spaces(),
        );
        assert!(m * 1000 < n * 5, "rust: {m} of {n} lines moved");
        let two = Unit {
            text: "  ".into(),
            width: 2,
            tabstop: 2,
        };
        let (m, n) = moved("lua", "lua", &["kawoosh/lua"], &two);
        assert!(m * 100 < n * 2, "lua: {m} of {n} lines moved");
    }

    /// A long flat node under an error — a big JSON array with a
    /// trailing comma (its MISSING element), a Rust `impl` left open (an
    /// ERROR of loose tokens) — reindented whole in time linear in its
    /// lines: its open brackets read once, not walked up to each line
    /// (20,000 elements took 41 s, now milliseconds). The bound is generous.
    #[test]
    fn a_broken_flat_node_reindents_in_linear_time() {
        let mut json = String::from("[\n");
        for i in 0..20_000 {
            json += &format!("{i},\n");
        }
        json += "]\n";
        let mut rust = String::from("impl A {\n");
        for i in 0..5_000 {
            rust += &format!("fn f{i}() {{\nx;\n}}\n");
        }
        for (lang, src, at) in [("json", &json, 1), ("rust", &rust, 2)] {
            let (g, tree, text) = parse(lang, src);
            assert!(tree.root_node().has_error(), "{lang}");
            let n = text.line_count();
            let t = std::time::Instant::now();
            let got = for_lines(g.indents.as_ref().unwrap(), &tree, &text, 0..n, &spaces());
            let took = t.elapsed();
            assert_eq!(
                got[n - 3].as_deref(),
                Some(" ".repeat(4 * at).as_str()),
                "{lang}"
            );
            assert!(took.as_secs() < 2, "{lang}: {n} lines in {took:?}");
        }
    }
}
