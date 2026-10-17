//! The LSP snippet grammar (docs/design/completion.md Decision 7):
//! parsed into [`Node`]s and expanded into a text and its fields.
//!
//! What it reads, as LSP 3.17 and VS Code write it: tabstops `$1`
//! `${1}`, placeholders `${1:default}` (nested: `${1:a ${2:b}}`),
//! choices `${1|a,b|}`, the final caret `$0`, variables `$NAME`
//! `${NAME}` `${NAME:default}`, transforms `${1/re/format/flags}` and
//! `${NAME/re/format/flags}`, and the escapes `\$` `\}` `\\` (and `\,`
//! `\|` in a choice).
//!
//! Lenient, never failing: a `$` that begins no construct the grammar
//! knows — `$`, `${`, an unclosed `${1:…`, a choice with no `|}` — is
//! the text it is, and the rest is read on from after it, as VS Code
//! reads one. Nesting deeper than [`MAX_DEPTH`] is text too, and an
//! expansion visits at most [`BUDGET`] nodes, so no body, however
//! made, overflows the stack or grows without end.
//!
//! [`expand`] lays the nodes out: each field's places in the text (a
//! field that repeats is several), its choices, the mirrors a transform
//! rewrites when the field is left, and `$0` last — at the text's end
//! when the body names none. Every line after the first is indented as
//! the line it goes into, and a body's leading tabs are the buffer's
//! indent unit ([`Indent`]).

use std::collections::{HashMap, HashSet};
use std::ops::Range;

/// The deepest a construct nests before it is read as text.
pub const MAX_DEPTH: usize = 64;
/// The most nodes one expansion visits: a body whose fields' defaults
/// mirror one another can say a great deal in a few bytes.
pub const BUDGET: usize = 100_000;

/// One piece of a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Text(String),
    /// `$1`, `${1}`, or with a transform `${1/re/format/}`: a place for
    /// field 1, or a mirror of it rewritten when the field is left.
    Tabstop {
        index: u32,
        transform: Option<Transform>,
    },
    /// `${1:…}`: field 1 with a default, which may hold fields itself.
    Placeholder {
        index: u32,
        children: Vec<Node>,
    },
    /// `${1|a,b|}`: field 1, its first option put, the rest offered.
    Choice {
        index: u32,
        options: Vec<String>,
    },
    /// `$NAME`, `${NAME:default}`, `${NAME/re/format/}`.
    Variable {
        name: String,
        default: Option<Vec<Node>>,
        transform: Option<Transform>,
    },
}

/// `/regex/format/options`: what a variable's value or a field's text
/// is rewritten by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transform {
    pub regex: String,
    pub format: Vec<Format>,
    pub flags: String,
}

/// One piece of a transform's format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Text(String),
    /// `$1`, `${1}`: a group as matched.
    Group(u32),
    /// `${1:/upcase}` and the rest.
    Case(u32, Case),
    /// `${1:+if}`: `if` when the group matched.
    If(u32, String),
    /// `${1:?if:else}`.
    IfElse(u32, String, String),
    /// `${1:-else}`, `${1:else}`: the group, or `else` when it did not
    /// match or matched nothing.
    Else(u32, String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    Upcase,
    Downcase,
    Capitalize,
    Camelcase,
    Pascalcase,
}

/// A body parsed: never fails (the module's doc says how).
pub fn parse(src: &str) -> Vec<Node> {
    let mut p = Parser {
        s: src.chars().collect(),
        i: 0,
        depth: 0,
        failed: HashSet::new(),
    };
    p.seq(false)
}

struct Parser {
    s: Vec<char>,
    i: usize,
    depth: usize,
    /// Where a construct was tried and was none: a construct's reading
    /// does not depend on what holds it, so it is never tried twice —
    /// an unclosed `${1:` repeated would otherwise be retried at every
    /// depth it fell back from.
    failed: HashSet<usize>,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn at(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }

    /// Nodes up to the end, or up to the `}` that closes a placeholder
    /// or a variable's default (`inner`), which is left unread.
    fn seq(&mut self, inner: bool) -> Vec<Node> {
        let mut out = Vec::new();
        let mut text = String::new();
        while let Some(c) = self.peek() {
            match c {
                '\\' => match self.at(1) {
                    Some(n @ ('$' | '}' | '\\')) => {
                        text.push(n);
                        self.i += 2;
                    }
                    _ => {
                        text.push('\\');
                        self.i += 1;
                    }
                },
                '}' if inner => break,
                '$' => {
                    let at = self.i;
                    match self.dollar() {
                        Some(node) => {
                            if !text.is_empty() {
                                out.push(Node::Text(std::mem::take(&mut text)));
                            }
                            out.push(node);
                        }
                        None => {
                            self.failed.insert(at);
                            self.i = at + 1;
                            text.push('$');
                        }
                    }
                }
                _ => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        if !text.is_empty() {
            out.push(Node::Text(text));
        }
        out
    }

    fn int(&mut self) -> Option<u32> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == start {
            return None;
        }
        // An index past u32 is no index: the text it is.
        self.s[start..self.i]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }

    fn var(&mut self) -> Option<String> {
        let first = self.peek()?;
        if !(first == '_' || first.is_ascii_alphabetic()) {
            return None;
        }
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| c == '_' || c.is_ascii_alphanumeric())
        {
            self.i += 1;
        }
        Some(self.s[start..self.i].iter().collect())
    }

    fn eat(&mut self, c: char) -> Option<()> {
        (self.peek() == Some(c)).then(|| self.i += 1)
    }

    /// A construct at the `$` under the cursor, or none — the cursor
    /// then anywhere; the caller puts it back.
    fn dollar(&mut self) -> Option<Node> {
        if self.failed.contains(&self.i) || self.depth >= MAX_DEPTH {
            return None;
        }
        self.i += 1;
        match self.peek()? {
            c if c.is_ascii_digit() => Some(Node::Tabstop {
                index: self.int()?,
                transform: None,
            }),
            '{' => {
                self.i += 1;
                self.depth += 1;
                let node = self.braced();
                self.depth -= 1;
                node
            }
            _ => Some(Node::Variable {
                name: self.var()?,
                default: None,
                transform: None,
            }),
        }
    }

    /// After `${`.
    fn braced(&mut self) -> Option<Node> {
        if self.peek()?.is_ascii_digit() {
            let index = self.int()?;
            match self.peek()? {
                '}' => {
                    self.i += 1;
                    Some(Node::Tabstop {
                        index,
                        transform: None,
                    })
                }
                ':' => {
                    self.i += 1;
                    let children = self.seq(true);
                    self.eat('}')?;
                    Some(Node::Placeholder { index, children })
                }
                '|' => {
                    self.i += 1;
                    let options = self.choices()?;
                    Some(Node::Choice { index, options })
                }
                '/' => {
                    self.i += 1;
                    let transform = self.transform()?;
                    Some(Node::Tabstop {
                        index,
                        transform: Some(transform),
                    })
                }
                _ => None,
            }
        } else {
            let name = self.var()?;
            match self.peek()? {
                '}' => {
                    self.i += 1;
                    Some(Node::Variable {
                        name,
                        default: None,
                        transform: None,
                    })
                }
                ':' => {
                    self.i += 1;
                    let default = self.seq(true);
                    self.eat('}')?;
                    Some(Node::Variable {
                        name,
                        default: Some(default),
                        transform: None,
                    })
                }
                '/' => {
                    self.i += 1;
                    let transform = self.transform()?;
                    Some(Node::Variable {
                        name,
                        default: None,
                        transform: Some(transform),
                    })
                }
                _ => None,
            }
        }
    }

    /// After `${1|`: the options up to `|}`.
    fn choices(&mut self) -> Option<Vec<String>> {
        let mut out = Vec::new();
        let mut cur = String::new();
        loop {
            match self.peek()? {
                '\\' => match self.at(1) {
                    Some(n @ ('$' | '}' | '\\' | ',' | '|')) => {
                        cur.push(n);
                        self.i += 2;
                    }
                    _ => {
                        cur.push('\\');
                        self.i += 1;
                    }
                },
                ',' => {
                    out.push(std::mem::take(&mut cur));
                    self.i += 1;
                }
                '|' if self.at(1) == Some('}') => {
                    out.push(cur);
                    self.i += 2;
                    return Some(out);
                }
                c => {
                    cur.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// Up to an unescaped `/`, read past it; `\/` is a `/`, and with
    /// `keep` every other escape is kept as written (a regex's).
    fn until_slash(&mut self, keep: bool) -> Option<String> {
        let mut out = String::new();
        loop {
            match self.peek()? {
                '/' => {
                    self.i += 1;
                    return Some(out);
                }
                '\\' if self.at(1) == Some('/') => {
                    out.push('/');
                    self.i += 2;
                }
                '\\' if keep => {
                    out.push('\\');
                    out.push(self.at(1)?);
                    self.i += 2;
                }
                c => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// After the `/` that opens one: `regex/format/flags}`.
    fn transform(&mut self) -> Option<Transform> {
        let regex = self.until_slash(true)?;
        let format = self.format()?;
        let mut flags = String::new();
        while let Some(c) = self.peek() {
            if c == '}' {
                self.i += 1;
                return Some(Transform {
                    regex,
                    format,
                    flags,
                });
            }
            if !c.is_ascii_alphabetic() {
                return None;
            }
            flags.push(c);
            self.i += 1;
        }
        None
    }

    /// A transform's format, up to and past its closing `/`.
    fn format(&mut self) -> Option<Vec<Format>> {
        let mut out = Vec::new();
        let mut text = String::new();
        loop {
            match self.peek()? {
                '/' => {
                    self.i += 1;
                    if !text.is_empty() {
                        out.push(Format::Text(text));
                    }
                    return Some(out);
                }
                '\\' => {
                    match self.at(1)? {
                        'n' => text.push('\n'),
                        't' => text.push('\t'),
                        c => text.push(c),
                    }
                    self.i += 2;
                }
                '$' => {
                    let at = self.i;
                    match self.format_item() {
                        Some(f) => {
                            if !text.is_empty() {
                                out.push(Format::Text(std::mem::take(&mut text)));
                            }
                            out.push(f);
                        }
                        None => {
                            self.i = at + 1;
                            text.push('$');
                        }
                    }
                }
                c => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// `$1`, `${1}`, `${1:/upcase}`, `${1:+if}`, `${1:?if:else}`,
    /// `${1:-else}`, `${1:else}`.
    fn format_item(&mut self) -> Option<Format> {
        self.i += 1;
        if self.peek()?.is_ascii_digit() {
            return Some(Format::Group(self.int()?));
        }
        self.eat('{')?;
        let n = self.int()?;
        match self.peek()? {
            '}' => {
                self.i += 1;
                Some(Format::Group(n))
            }
            ':' => {
                self.i += 1;
                match self.peek()? {
                    '/' => {
                        self.i += 1;
                        let name = self.var()?;
                        self.eat('}')?;
                        let case = match name.as_str() {
                            "upcase" => Case::Upcase,
                            "downcase" => Case::Downcase,
                            "capitalize" => Case::Capitalize,
                            "camelcase" => Case::Camelcase,
                            "pascalcase" => Case::Pascalcase,
                            _ => return None,
                        };
                        Some(Format::Case(n, case))
                    }
                    '+' => {
                        self.i += 1;
                        Some(Format::If(n, self.format_text(&['}'])?.0))
                    }
                    '?' => {
                        self.i += 1;
                        let (yes, end) = self.format_text(&[':', '}'])?;
                        if end == '}' {
                            return Some(Format::IfElse(n, yes, String::new()));
                        }
                        Some(Format::IfElse(n, yes, self.format_text(&['}'])?.0))
                    }
                    '-' => {
                        self.i += 1;
                        Some(Format::Else(n, self.format_text(&['}'])?.0))
                    }
                    _ => Some(Format::Else(n, self.format_text(&['}'])?.0)),
                }
            }
            _ => None,
        }
    }

    /// Text up to one of `ends`, read past it, and which it was.
    fn format_text(&mut self, ends: &[char]) -> Option<(String, char)> {
        let mut out = String::new();
        loop {
            let c = self.peek()?;
            if ends.contains(&c) {
                self.i += 1;
                return Some((out, c));
            }
            if c == '\\' {
                out.push(self.at(1)?);
                self.i += 2;
            } else {
                out.push(c);
                self.i += 1;
            }
        }
    }
}

impl Transform {
    /// `text` rewritten: the first match, or every one with `g`, put
    /// back by the format; the text as it is when the regex does not
    /// compile.
    pub fn apply(&self, text: &str) -> String {
        let mut pre = String::new();
        for f in self.flags.chars() {
            match f {
                'i' => pre.push('i'),
                'm' => pre.push('m'),
                's' => pre.push('s'),
                _ => {}
            }
        }
        let pattern = if pre.is_empty() {
            self.regex.clone()
        } else {
            format!("(?{pre}){}", self.regex)
        };
        let Ok(re) = regex::RegexBuilder::new(&pattern)
            .size_limit(1 << 20)
            .build()
        else {
            return text.to_string();
        };
        let all = self.flags.contains('g');
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for caps in re.captures_iter(text) {
            let m = caps.get(0).expect("a match has its whole");
            out.push_str(&text[last..m.start()]);
            self.put(&caps, &mut out);
            last = m.end();
            if !all {
                break;
            }
        }
        out.push_str(&text[last..]);
        out
    }

    fn put(&self, caps: &regex::Captures, out: &mut String) {
        let group = |n: u32| caps.get(n as usize).map(|m| m.as_str()).unwrap_or("");
        for f in &self.format {
            match f {
                Format::Text(t) => out.push_str(t),
                Format::Group(n) => out.push_str(group(*n)),
                Format::Case(n, c) => out.push_str(&case(group(*n), *c)),
                Format::If(n, t) => {
                    if !group(*n).is_empty() {
                        out.push_str(t);
                    }
                }
                Format::IfElse(n, t, e) => {
                    out.push_str(if group(*n).is_empty() { e } else { t });
                }
                Format::Else(n, e) => {
                    let g = group(*n);
                    out.push_str(if g.is_empty() { e } else { g });
                }
            }
        }
    }
}

fn case(s: &str, c: Case) -> String {
    match c {
        Case::Upcase => s.to_uppercase(),
        Case::Downcase => s.to_lowercase(),
        Case::Capitalize => {
            let mut ch = s.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().chain(ch).collect(),
                None => String::new(),
            }
        }
        Case::Camelcase | Case::Pascalcase => {
            let mut out = String::new();
            for (i, w) in s
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .enumerate()
            {
                let mut ch = w.chars();
                let f = ch.next().expect("a word is not empty");
                if i == 0 && c == Case::Camelcase {
                    out.extend(f.to_lowercase());
                } else {
                    out.extend(f.to_uppercase());
                }
                out.extend(ch);
            }
            out
        }
    }
}

/// How the body's lines meet the text they go into.
#[derive(Clone, Debug, Default)]
pub struct Indent {
    /// What each line after the first starts with: the leading
    /// whitespace of the line the snippet goes into.
    pub base: String,
    /// What each tab a line of the body starts with becomes (the
    /// buffer's indent unit); empty leaves the tabs.
    pub unit: String,
    /// The line break: the buffer's (`\n`, `\r\n`).
    pub eol: String,
}

/// One field of an expanded snippet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Its number in the body; `0` is the final caret. A variable
    /// nobody knows (VS Code's rule) is a field numbered after the
    /// body's own.
    pub index: u32,
    /// Its places in the text, in the text's order: several when it
    /// repeats, each a caret edited with the others.
    pub ranges: Vec<Range<usize>>,
    /// A choice field's options.
    pub choices: Option<Vec<String>>,
    /// The places a transform of it fills, rewritten from its text when
    /// it is left.
    pub mirrors: Vec<(Range<usize>, Transform)>,
    /// The field whose placeholder it is laid out in (`${1:a ${2:b}}`:
    /// 2's is 1), which grows as it is typed in.
    pub parent: Option<u32>,
}

/// A body laid out: its text, and its fields in the order they are
/// visited — `1`, `2`, …, the unknown variables', then `0`, always
/// there and always last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    pub fields: Vec<Field>,
}

/// A body's plain text: every field its default, every variable its
/// default or nothing — what a candidate shows and is matched by
/// before it is expanded where it goes.
pub fn plain(src: &str) -> String {
    expand(
        &parse(src),
        &mut |_| Some(String::new()),
        &Indent::default(),
    )
    .text
}

/// `nodes` laid out, `vars` answering each variable's value — `None`
/// for a variable it does not know, which is put as its name in a field
/// of its own.
pub fn expand(
    nodes: &[Node],
    vars: &mut dyn FnMut(&str) -> Option<String>,
    indent: &Indent,
) -> Expanded {
    let mut defaults: HashMap<u32, Default<'_>> = HashMap::new();
    find_defaults(nodes, &mut defaults, 0);
    let mut x = Expander {
        defaults,
        vars,
        indent,
        out: String::new(),
        line_start: false,
        fields: HashMap::new(),
        unknown: Vec::new(),
        primary: HashSet::new(),
        within: Vec::new(),
        budget: BUDGET,
    };
    x.main(nodes);
    let mut numbered: Vec<Field> = x.fields.into_values().collect();
    numbered.sort_by_key(|f| f.index);
    let after = numbered.last().map_or(1, |f| f.index.saturating_add(1));
    let zero = numbered
        .iter()
        .position(|f| f.index == 0)
        .map(|i| numbered.remove(i));
    let mut fields = numbered;
    for f in &mut fields {
        f.ranges.sort_by_key(|r| (r.start, r.end));
        f.mirrors.sort_by_key(|(r, _)| (r.start, r.end));
    }
    for (k, (_, r)) in x.unknown.into_iter().enumerate() {
        fields.push(Field {
            index: after.saturating_add(k as u32),
            ranges: vec![r],
            choices: None,
            mirrors: Vec::new(),
            parent: None,
        });
    }
    let end = x.out.len();
    fields.push(zero.unwrap_or(Field {
        index: 0,
        ranges: std::iter::once(end..end).collect(),
        choices: None,
        mirrors: Vec::new(),
        parent: None,
    }));
    // `$0` only mirrored by a transform has no place of its own: the end.
    if let Some(z) = fields.last_mut()
        && z.ranges.is_empty()
    {
        z.ranges.push(end..end);
    }
    Expanded {
        text: x.out,
        fields,
    }
}

/// What a field's places put when it is not the one that says: its
/// first placeholder's children, or its first choice's first option.
#[derive(Clone, Copy)]
enum Default<'a> {
    Nodes(&'a [Node]),
    Text(&'a str),
}

fn find_defaults<'a>(nodes: &'a [Node], out: &mut HashMap<u32, Default<'a>>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for n in nodes {
        match n {
            Node::Placeholder { index, children } => {
                out.entry(*index).or_insert(Default::Nodes(children));
                find_defaults(children, out, depth + 1);
            }
            Node::Choice { index, options } => {
                out.entry(*index)
                    .or_insert(Default::Text(options.first().map_or("", |s| s.as_str())));
            }
            Node::Variable {
                default: Some(d), ..
            } => find_defaults(d, out, depth + 1),
            _ => {}
        }
    }
}

struct Expander<'a, 'v> {
    defaults: HashMap<u32, Default<'a>>,
    vars: &'v mut dyn FnMut(&str) -> Option<String>,
    indent: &'v Indent,
    out: String,
    /// The last thing put was a line break: a tab now is the indent
    /// unit's.
    line_start: bool,
    fields: HashMap<u32, Field>,
    /// The variables nobody knows, as fields: their names' places.
    unknown: Vec<(String, Range<usize>)>,
    /// The fields whose placeholder (or choice) has been laid out: a
    /// later one of the same number is a mirror.
    primary: HashSet<u32>,
    /// The placeholders being laid out, innermost last.
    within: Vec<u32>,
    budget: usize,
}

impl Expander<'_, '_> {
    fn emit(&mut self, s: &str) {
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' if chars.peek() == Some(&'\n') => {}
                '\n' => {
                    match self.indent.eol.as_str() {
                        "" => self.out.push('\n'),
                        eol => self.out.push_str(eol),
                    }
                    self.out.push_str(&self.indent.base);
                    self.line_start = true;
                }
                '\t' if self.line_start && !self.indent.unit.is_empty() => {
                    self.out.push_str(&self.indent.unit);
                }
                c => {
                    self.out.push(c);
                    self.line_start = false;
                }
            }
        }
    }

    fn spend(&mut self) -> bool {
        if self.budget == 0 {
            return false;
        }
        self.budget -= 1;
        true
    }

    fn field(&mut self, index: u32) -> &mut Field {
        let parent = self.within.last().copied().filter(|p| *p != index);
        self.fields.entry(index).or_insert_with(|| Field {
            index,
            ranges: Vec::new(),
            choices: None,
            mirrors: Vec::new(),
            parent,
        })
    }

    /// The body itself: its first placeholder of each field laid out
    /// with the fields inside it, every other place of a field its
    /// default as text.
    fn main(&mut self, nodes: &[Node]) {
        for n in nodes {
            if !self.spend() {
                return;
            }
            match n {
                Node::Text(t) => self.emit(t),
                Node::Placeholder { index, children } if self.primary.insert(*index) => {
                    let start = self.out.len();
                    // Made before its children, so it is theirs.
                    self.field(*index);
                    self.within.push(*index);
                    self.main(children);
                    self.within.pop();
                    let r = start..self.out.len();
                    self.field(*index).ranges.push(r);
                }
                Node::Choice { index, options } if self.primary.insert(*index) => {
                    let start = self.out.len();
                    self.emit(options.first().map_or("", |s| s.as_str()));
                    let r = start..self.out.len();
                    let f = self.field(*index);
                    f.ranges.push(r);
                    f.choices = Some(options.clone());
                }
                Node::Tabstop {
                    index,
                    transform: Some(t),
                } => {
                    let start = self.out.len();
                    let mut text = String::new();
                    self.plain_of(*index, &mut vec![], &mut text);
                    self.emit(&t.apply(&text));
                    let r = start..self.out.len();
                    self.field(*index).mirrors.push((r, t.clone()));
                }
                Node::Tabstop { index, .. }
                | Node::Placeholder { index, .. }
                | Node::Choice { index, .. } => {
                    let start = self.out.len();
                    let mut text = String::new();
                    self.plain_of(*index, &mut vec![], &mut text);
                    self.emit(&text);
                    let r = start..self.out.len();
                    self.field(*index).ranges.push(r);
                }
                Node::Variable {
                    name,
                    default,
                    transform,
                } => match (self.vars)(name) {
                    Some(v) if !v.is_empty() || transform.is_some() => {
                        let v = match transform {
                            Some(t) => t.apply(&v),
                            None => v,
                        };
                        self.emit(&v);
                    }
                    known => match default {
                        Some(d) => self.main(d),
                        None if known.is_none() => {
                            let start = self.out.len();
                            self.emit(name);
                            let r = start..self.out.len();
                            self.unknown.push((name.clone(), r));
                        }
                        None => {}
                    },
                },
            }
        }
    }

    /// Field `index`'s default as text, into `out`: what a mirror of it
    /// puts. `stack` holds the fields being rendered so far down: one
    /// inside its own default is nothing there.
    fn plain_of(&mut self, index: u32, stack: &mut Vec<u32>, out: &mut String) {
        if stack.contains(&index) || stack.len() > MAX_DEPTH {
            return;
        }
        match self.defaults.get(&index).copied() {
            Some(Default::Text(t)) => out.push_str(t),
            Some(Default::Nodes(nodes)) => {
                stack.push(index);
                self.plain(nodes, stack, out);
                stack.pop();
            }
            None => {}
        }
    }

    fn plain(&mut self, nodes: &[Node], stack: &mut Vec<u32>, out: &mut String) {
        for n in nodes {
            if !self.spend() {
                return;
            }
            match n {
                Node::Text(t) => out.push_str(t),
                Node::Tabstop {
                    index,
                    transform: Some(t),
                } => {
                    let mut text = String::new();
                    self.plain_of(*index, stack, &mut text);
                    out.push_str(&t.apply(&text));
                }
                Node::Tabstop { index, .. }
                | Node::Placeholder { index, .. }
                | Node::Choice { index, .. } => self.plain_of(*index, stack, out),
                Node::Variable {
                    name,
                    default,
                    transform,
                } => match (self.vars)(name) {
                    Some(v) if !v.is_empty() || transform.is_some() => match transform {
                        Some(t) => out.push_str(&t.apply(&v)),
                        None => out.push_str(&v),
                    },
                    known => match default {
                        Some(d) => self.plain(d, stack, out),
                        None if known.is_none() => out.push_str(name),
                        None => {}
                    },
                },
            }
        }
    }
}

#[cfg(test)]
// A field of one place is what these mean.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn x(src: &str) -> Expanded {
        expand(&parse(src), &mut |_| None, &Indent::default())
    }

    fn places(e: &Expanded) -> Vec<(u32, Vec<&str>)> {
        e.fields
            .iter()
            .map(|f| {
                (
                    f.index,
                    f.ranges.iter().map(|r| &e.text[r.clone()]).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn tabstops_placeholders_and_the_final_caret() {
        let e = x("for ${1:x} in ${2:iter} {\n\t$0\n}");
        assert_eq!(e.text, "for x in iter {\n\t\n}");
        assert_eq!(
            places(&e),
            [(1, vec!["x"]), (2, vec!["iter"]), (0, vec![""])]
        );
        assert_eq!(e.fields[2].ranges[0], 17..17);
    }

    #[test]
    fn no_final_caret_is_one_at_the_end() {
        let e = x("push(${1})");
        assert_eq!(e.text, "push()");
        assert_eq!(e.fields[0].ranges, [5..5]);
        assert_eq!(e.fields[1].index, 0);
        assert_eq!(e.fields[1].ranges, [6..6]);
    }

    #[test]
    fn nested_placeholders_hold_their_fields() {
        let e = x("${1:a ${2:b ${3:c}}}!");
        assert_eq!(e.text, "a b c!");
        assert_eq!(
            places(&e),
            [
                (1, vec!["a b c"]),
                (2, vec!["b c"]),
                (3, vec!["c"]),
                (0, vec![""])
            ]
        );
        let parents: Vec<Option<u32>> = e.fields.iter().map(|f| f.parent).collect();
        assert_eq!(parents, [None, Some(1), Some(2), None]);
    }

    #[test]
    fn a_repeated_field_is_every_place_with_one_default() {
        let e = x("$1 = ${1:name}; use($1)");
        assert_eq!(e.text, "name = name; use(name)");
        assert_eq!(places(&e)[0], (1, vec!["name", "name", "name"]));
    }

    #[test]
    fn choices_put_the_first() {
        let e = x("let ${1|mut,ref mut,const|} = ${1};");
        assert_eq!(e.text, "let mut = mut;");
        assert_eq!(
            e.fields[0].choices.as_deref(),
            Some(&["mut".to_string(), "ref mut".into(), "const".into()][..])
        );
        assert_eq!(places(&e)[0], (1, vec!["mut", "mut"]));
        let e = x(r"${1|a\,b,c\|d|}");
        assert_eq!(e.text, "a,b");
        assert_eq!(e.fields[0].choices.as_ref().unwrap()[1], "c|d");
    }

    #[test]
    fn escapes_and_stray_dollars_are_text() {
        assert_eq!(x(r"\$1 \} \\ $ ${ $} a$").text, r"$1 } \ $ ${ $} a$");
        assert_eq!(x("}{").text, "}{");
        assert_eq!(x(r"${1:a\}b}").text, "a}b");
        // A lone backslash before anything else is itself.
        assert_eq!(x(r"\n\q").text, r"\n\q");
    }

    #[test]
    fn malformed_constructs_are_their_text() {
        for (src, want) in [
            ("${1:unclosed", "${1:unclosed"),
            ("${1|a,b", "${1|a,b"),
            ("${1|a,b|", "${1|a,b|"),
            ("${1:a ${2:b}", "${1:a b"),
            ("${x", "${x"),
            ("${1/(a/b", "${1/(a/b"),
            ("${99999999999}", "${99999999999}"),
            ("${-1}", "${-1}"),
            ("${1:${2:${3:", "${1:${2:${3:"),
        ] {
            assert_eq!(x(src).text, want, "{src}");
        }
        // Inside what is left: a field still a field.
        let e = x("${1:a ${2:b}");
        assert_eq!(places(&e)[0], (2, vec!["b"]));
    }

    #[test]
    fn deep_or_mirrored_bodies_end() {
        let deep = "${1:".repeat(5000);
        assert_eq!(x(&deep).text, deep);
        let closed = format!("{}x{}", "${1:".repeat(200), "}".repeat(200));
        assert!(!x(&closed).text.is_empty());
        // Each default mirrors the next twice: 2^30 if laid out whole.
        let mut body = String::new();
        for i in 1..30 {
            body.push_str(&format!("${{{i}:${}${}}}", i + 1, i + 1));
        }
        let e = x(&body);
        assert!(e.text.len() < 1 << 22, "{}", e.text.len());
        // A field inside its own default is nothing there.
        assert_eq!(x("${1:a$1}").text, "aa");
    }

    #[test]
    fn variables_known_empty_and_unknown() {
        let mut vars = |n: &str| match n {
            "TM_FILENAME" => Some("main.rs".to_string()),
            "TM_SELECTED_TEXT" => Some(String::new()),
            _ => None,
        };
        let e = expand(
            &parse(
                "$TM_FILENAME ${TM_SELECTED_TEXT:none} ${TM_SELECTED_TEXT} $NOPE ${NADA:dflt $1}",
            ),
            &mut vars,
            &Indent::default(),
        );
        assert_eq!(e.text, "main.rs none  NOPE dflt ");
        // The unknown variable is a field after the body's own, before 0.
        assert_eq!(
            places(&e),
            [(1, vec![""]), (2, vec!["NOPE"]), (0, vec![""])]
        );
    }

    #[test]
    fn transforms_rewrite_variables_and_mirrors() {
        let mut vars = |n: &str| (n == "TM_FILENAME").then(|| "my_file.rs".to_string());
        let e = expand(
            &parse(
                r"${TM_FILENAME/(.*)\..+$/${1:/pascalcase}/} ${1:abc} ${1/(b)/[${1:/upcase}]/g}",
            ),
            &mut vars,
            &Indent::default(),
        );
        assert_eq!(e.text, "MyFile abc a[B]c");
        let f = &e.fields[0];
        assert_eq!(f.mirrors.len(), 1);
        assert_eq!(&e.text[f.mirrors[0].0.clone()], "a[B]c");
        assert_eq!(f.mirrors[0].1.apply("bob"), "[B]o[B]");
        let t = |src: &str, v: &str| {
            let n = parse(src);
            let Node::Variable {
                transform: Some(t), ..
            } = &n[0]
            else {
                panic!("{n:?}")
            };
            t.apply(v)
        };
        assert_eq!(t("${X/(a)?b/${1:+yes}${1:?Y:N}${1:-no}/}", "b"), "Nno");
        assert_eq!(t("${X/(a)?b/${1:+yes}${1:?Y:N}${1:no}/}", "ab"), "yesYa");
        assert_eq!(t("${X/A/z/gi}", "aAa"), "zzz");
        assert_eq!(t("${X/(.*)/${1:/camelcase}/}", "foo-bar baz"), "fooBarBaz");
        assert_eq!(t("${X/(.*)/${1:/capitalize}/}", "élan"), "Élan");
        // A regex that does not compile leaves the value.
        assert_eq!(t("${X/(/z/}", "value"), "value");
    }

    #[test]
    fn lines_are_indented_as_the_one_they_go_into() {
        let indent = Indent {
            base: "    ".into(),
            unit: "  ".into(),
            eol: "\r\n".into(),
        };
        let e = expand(&parse("if ${1:c} {\n\t\t$0\n}"), &mut |_| None, &indent);
        assert_eq!(e.text, "if c {\r\n        \r\n    }");
        assert_eq!(&e.text[e.fields[1].ranges[0].clone()], "");
        assert_eq!(e.fields[1].ranges[0].start, 16);
    }

    #[test]
    fn multi_byte_text_keeps_byte_ranges() {
        let e = x("«${1:ключ}» → ${2:値}$1");
        assert_eq!(e.text, "«ключ» → 値ключ");
        assert_eq!(places(&e)[0], (1, vec!["ключ", "ключ"]));
        assert_eq!(places(&e)[1], (2, vec!["値"]));
    }

    #[test]
    fn plain_is_the_defaults() {
        assert_eq!(
            plain("fn ${1:name}(${2}) -> ${3:T} {\n\t$0\n}"),
            "fn name() -> T {\n\t\n}"
        );
        assert_eq!(plain("$TM_FILENAME${CLIPBOARD:x}"), "x");
    }
}
