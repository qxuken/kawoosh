//! `kawoosh.node` (docs/design/nodes.md): the syntax tree read from Lua.
//! A node is a plain table — its type, its range, the buffer and the
//! version it is of, and tree-sitter's id for it — with the walking
//! functions on a shared metatable; each call finds the node again in
//! the tree the shell handed over (`Runtime::set_tree`), which is only
//! read when it is of the text Lua reads. A node kept past its version
//! is an error when walked: its offsets are of a text that is gone.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use mlua::{Lua, Table, Value as LV};
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::{BufSnap, Msg, Published};

/// A node's fields and methods for lua-language-server, after the
/// walk's `kawoosh.node` in the meta file: what `n:` completes.
pub(crate) const LUALS_CLASS: &str = r#"---A syntax node (docs/design/nodes.md): data, walked by its methods.
---@class kawoosh.Node
---@field type string the grammar's kind: `call_expression`, `true`
---@field named boolean false for a token such as `(` or `==`
---@field field string? the field it fills in its parent
---@field from integer its first byte, from 0
---@field to integer the byte after its last
---@field line integer its first line, from 1
---@field end_line integer its last line, from 1
---@field error boolean an ERROR or MISSING node itself
---@field has_error boolean one somewhere inside it
---@field language string the grammar it is from
---@field buffer integer
---@field version integer the text's version it was read at
---@field id integer
local Node = {}
---@return kawoosh.Node?
function Node:parent() end
---@param opts? { anonymous?: boolean }
---@return kawoosh.Node[]
function Node:children(opts) end
---@param i integer from 1; -1 the last
---@param opts? { anonymous?: boolean }
---@return kawoosh.Node?
function Node:child(i, opts) end
---@param field string `condition`, `body`, `operator`
---@return kawoosh.Node?
function Node:get(field) end
---@param opts? { anonymous?: boolean }
---@return kawoosh.Node?
function Node:next(opts) end
---@param opts? { anonymous?: boolean }
---@return kawoosh.Node?
function Node:prev(opts) end
---@param types string|string[]
---@return kawoosh.Node?
function Node:closest(types) end
---@return string
function Node:text() end
function Node:select() end
---@param source string a tree-sitter query
---@return kawoosh.NodeMatch[]
function Node:query(source) end

---@class kawoosh.NodeMatch
---@field pattern integer which pattern, from 1
---@field captures table<string, kawoosh.Node> each capture's first node
---@field all table<string, kawoosh.Node[]> each capture's nodes
"#;

/// What `at` and `root` say when the tree is not the text's.
const BEHIND: &str = "the syntax tree is behind the text: again in a moment";

struct Shared {
    pp: Rc<RefCell<Published>>,
    queue: Rc<RefCell<Vec<Msg>>>,
    /// Every node table's metatable: `__index` the methods.
    meta: Table,
    /// Compiled queries, by language and source.
    queries: RefCell<HashMap<(String, String), Rc<Query>>>,
}

/// `buffer`'s tree when it is of the text as published, or why not.
fn tree_of(p: &Published, h: u64) -> Result<(Tree, &BufSnap), String> {
    let b = p.buffers.get(&h).ok_or_else(|| format!("no buffer {h}"))?;
    match p.trees.get(&h) {
        Some((v, t)) if *v == b.snapshot.version => Ok((t.clone(), b)),
        Some(_) => Err(BEHIND.into()),
        None => Err(format!("no syntax tree for {}", b.name)),
    }
}

/// The byte after the character at `o`, read off its first byte.
fn next_char(b: &BufSnap, o: usize) -> usize {
    let len = b.snapshot.len();
    if o >= len {
        return len;
    }
    let first = b.snapshot.text.collect_range(o..o + 1);
    let w = match first.first() {
        Some(c) if *c >= 0xf0 => 4,
        Some(c) if *c >= 0xe0 => 3,
        Some(c) if *c >= 0xc0 => 2,
        _ => 1,
    };
    (o + w).min(len)
}

/// The bytes `where` names: an offset's character, a `{ from, to }`
/// range, or nil for the primary selection — the caret's character, or
/// the selected bytes in visual mode, as `<A-o>` reads them.
fn bytes_of(p: &Published, b: &BufSnap, at: &LV) -> mlua::Result<Range<usize>> {
    let len = b.snapshot.len();
    Ok(match at {
        LV::Nil => {
            let (a, h) = b.sels.get(b.primary).copied().unwrap_or((0, 0));
            let (a, h) = (a.min(len), h.min(len));
            if p.mode == "visual" {
                a.min(h)..next_char(b, a.max(h))
            } else {
                h..next_char(b, h)
            }
        }
        LV::Integer(_) | LV::Number(_) => {
            let o = (usize::try_from(at.as_i64().unwrap_or(0)).unwrap_or(0)).min(len);
            o..next_char(b, o)
        }
        LV::Table(t) => {
            let from: usize = t
                .get::<Option<usize>>("from")?
                .map_or_else(|| t.get(1), Ok)?;
            let to: usize = t.get::<Option<usize>>("to")?.map_or_else(|| t.get(2), Ok)?;
            let (from, to) = (from.min(len), to.min(len));
            if to < from {
                return Err(mlua::Error::runtime(format!(
                    "{from}..{to} ends before it starts"
                )));
            }
            from..to
        }
        _ => {
            return Err(mlua::Error::runtime(
                "where: an offset, { from, to } or nil",
            ));
        }
    })
}

/// The field `node` fills in its parent, found by walking its
/// siblings; `children` knows it from its own walk.
fn field_of<'t>(node: Node<'t>) -> Option<&'t str> {
    let parent = node.parent()?;
    let mut c = parent.walk();
    if !c.goto_first_child() {
        return None;
    }
    loop {
        if c.node().id() == node.id() {
            return c.field_name();
        }
        if !c.goto_next_sibling() {
            return None;
        }
    }
}

/// The node a table names in `root`: the one of its `id`, or — the
/// ts thread may answer a version twice, a whole parse with new ids —
/// the innermost of its `kind` over exactly `from..to`, which in the
/// same text is the same node.
fn find<'t>(root: Node<'t>, from: usize, to: usize, kind: &str, id: usize) -> Option<Node<'t>> {
    let mut by_kind = None;
    let mut c = root.walk();
    loop {
        let n = c.node();
        if n.id() == id {
            return Some(n);
        }
        let r = n.byte_range();
        if r == (from..to) && n.kind() == kind {
            by_kind = Some(n);
        }
        if r.start <= from && to <= r.end && c.goto_first_child() {
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return by_kind;
            }
        }
    }
}

/// Whether `opts` asks for tokens too: `{ anonymous = true }`.
fn anonymous(opts: &Option<Table>) -> mlua::Result<bool> {
    Ok(match opts {
        Some(o) => o.get::<Option<bool>>("anonymous")?.unwrap_or(false),
        None => false,
    })
}

impl Shared {
    /// `node` as Lua holds it. `field` is the field it fills when the
    /// caller walked to it by one, `None` to look it up.
    fn table(
        &self,
        lua: &Lua,
        b: &BufSnap,
        h: u64,
        node: Node,
        field: Option<Option<&str>>,
    ) -> mlua::Result<Table> {
        let t = lua.create_table_with_capacity(0, 13)?;
        let (start, end) = (node.start_position(), node.end_position());
        let r = node.byte_range();
        t.set("type", node.kind())?;
        t.set("named", node.is_named())?;
        t.set("field", field.unwrap_or_else(|| field_of(node)))?;
        t.set("from", r.start)?;
        t.set("to", r.end)?;
        t.set("line", start.row + 1)?;
        // A node that ends with its line's newline ends on that line.
        let last = if end.column == 0 && r.end > r.start {
            end.row
        } else {
            end.row + 1
        };
        t.set("end_line", last.max(start.row + 1))?;
        t.set("error", node.is_error() || node.is_missing())?;
        t.set("has_error", node.has_error())?;
        t.set("language", b.language.as_str())?;
        t.set("buffer", h)?;
        t.set("version", b.snapshot.version.get())?;
        t.set("id", node.id())?;
        t.set_metatable(Some(self.meta.clone()))?;
        Ok(t)
    }

    fn opt_table(&self, lua: &Lua, b: &BufSnap, h: u64, node: Option<Node>) -> mlua::Result<LV> {
        match node {
            Some(n) => Ok(LV::Table(self.table(lua, b, h, n, None)?)),
            None => Ok(LV::Nil),
        }
    }

    /// Runs `f` on the node a table names, in the tree it was read from.
    fn with_node<R>(
        &self,
        t: &Table,
        f: impl FnOnce(&Published, &BufSnap, u64, Node) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        let h: u64 = t.get("buffer")?;
        let v: u64 = t.get("version")?;
        let from: usize = t.get("from")?;
        let to: usize = t.get("to")?;
        let id: usize = t.get("id")?;
        let kind: String = t.get("type")?;
        let p = self.pp.borrow();
        let b = p
            .buffers
            .get(&h)
            .ok_or_else(|| mlua::Error::runtime(format!("no buffer {h}")))?;
        let now = b.snapshot.version.get();
        let tree = match p.trees.get(&h) {
            Some((tv, tree)) if v == now && tv.get() == v => tree.clone(),
            _ => {
                return Err(mlua::Error::runtime(format!(
                    "a node of version {v}, and the buffer is at {now}: read it again"
                )));
            }
        };
        let root = tree.root_node();
        let node = find(root, from, to, &kind, id)
            .ok_or_else(|| mlua::Error::runtime("the node is not in its tree"))?;
        f(&p, b, h, node)
    }

    /// The node `pick` takes from the tree over `where`, or nil and why.
    fn entry(
        &self,
        lua: &Lua,
        at: LV,
        h: Option<u64>,
        pick: impl FnOnce(Node, Range<usize>) -> Option<Node>,
    ) -> mlua::Result<(LV, Option<String>)> {
        let p = self.pp.borrow();
        let Some(h) = h.or(p.current) else {
            return Err(mlua::Error::runtime("no current buffer"));
        };
        let (tree, b) = match tree_of(&p, h) {
            Ok(x) => x,
            Err(why) => return Ok((LV::Nil, Some(why))),
        };
        let r = bytes_of(&p, b, &at)?;
        let node = pick(tree.root_node(), r);
        Ok((self.opt_table(lua, b, h, node)?, None))
    }

    /// `kawoosh.node.query(source, where)`'s run: the matches over a
    /// node, in the text's order.
    fn query(
        &self,
        lua: &Lua,
        b: &BufSnap,
        h: u64,
        node: Node,
        source: &str,
    ) -> mlua::Result<Table> {
        let key = (b.language.to_string(), source.to_string());
        let cached = self.queries.borrow().get(&key).cloned();
        let query = match cached {
            Some(q) => q,
            None => {
                let lang = node.language();
                let q = Query::new(&lang, source)
                    .map_err(|e| mlua::Error::runtime(format!("query: {e}")))?;
                let q = Rc::new(q);
                self.queries.borrow_mut().insert(key, q.clone());
                q
            }
        };
        let names = query.capture_names();
        let out = lua.create_table()?;
        let mut cursor = QueryCursor::new();
        let text = &b.snapshot.text;
        let mut node_text = |n: Node| std::iter::once(text.collect_range(n.byte_range()));
        let mut it = cursor.matches(&query, node, &mut node_text);
        let mut i = 0;
        while let Some(m) = it.next() {
            let first = lua.create_table()?;
            let all = lua.create_table()?;
            for c in m.captures() {
                let name = names[c.index as usize];
                // `@_name`: a predicate's, not an answer.
                if name.starts_with('_') {
                    continue;
                }
                let n = self.table(lua, b, h, c.node, None)?;
                let list: Table = match all.get::<Option<Table>>(name)? {
                    Some(l) => l,
                    None => {
                        let l = lua.create_table()?;
                        all.set(name, l.clone())?;
                        first.set(name, n.clone())?;
                        l
                    }
                };
                list.push(n)?;
            }
            let t = lua.create_table()?;
            t.set("pattern", m.pattern_index + 1)?;
            t.set("captures", first)?;
            t.set("all", all)?;
            i += 1;
            out.set(i, t)?;
        }
        Ok(out)
    }
}

/// `kawoosh.node`, and the metatable its nodes share.
pub(crate) fn install(
    lua: &Lua,
    k: &Table,
    pp: &Rc<RefCell<Published>>,
    queue: &Rc<RefCell<Vec<Msg>>>,
) -> mlua::Result<()> {
    let node = lua.create_table()?;
    let methods = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__index", methods.clone())?;
    let sh = Rc::new(Shared {
        pp: pp.clone(),
        queue: queue.clone(),
        meta: meta.clone(),
        queries: RefCell::new(HashMap::new()),
    });
    meta.set(
        "__tostring",
        lua.create_function(|_, t: Table| {
            Ok(format!(
                "{} {}..{}",
                t.get::<String>("type")?,
                t.get::<usize>("from")?,
                t.get::<usize>("to")?
            ))
        })?,
    )?;
    meta.set(
        "__eq",
        lua.create_function(|_, (a, b): (Table, Table)| {
            // The id is one parse's; the range and the type are the
            // text's, so two parses of one version agree on them.
            let key = |t: &Table| -> mlua::Result<(u64, u64, usize, usize, String)> {
                Ok((
                    t.get("buffer")?,
                    t.get("version")?,
                    t.get("from")?,
                    t.get("to")?,
                    t.get("type")?,
                ))
            };
            Ok(key(&a)? == key(&b)?)
        })?,
    )?;

    // `kawoosh.node.at(where, buffer)`: the smallest named node over
    // `where` — an offset, `{ from, to }`, or nil for the primary
    // selection — or nil and why: no tree, or one behind the text.
    // @return kawoosh.Node?
    // @return string? why
    let s = sh.clone();
    node.set(
        "at",
        lua.create_function(move |lua, (at, h): (LV, Option<u64>)| {
            s.entry(lua, at, h, |root, r| {
                root.named_descendant_for_byte_range(r.start, r.end)
            })
        })?,
    )?;
    // `kawoosh.node.leaf(where, buffer)`: the smallest node over
    // `where`, a token included — `true`, `==`, `"`.
    // @return kawoosh.Node?
    // @return string? why
    let s = sh.clone();
    node.set(
        "leaf",
        lua.create_function(move |lua, (at, h): (LV, Option<u64>)| {
            s.entry(lua, at, h, |root, r| {
                root.descendant_for_byte_range(r.start, r.end)
            })
        })?,
    )?;
    // `kawoosh.node.root(buffer)`: the whole tree's node, or nil and why.
    // @return kawoosh.Node?
    // @return string? why
    let s = sh.clone();
    node.set(
        "root",
        lua.create_function(move |lua, h: Option<u64>| {
            s.entry(lua, LV::Nil, h, |root, _| Some(root))
        })?,
    )?;

    // The walking functions: each `kawoosh.node.NAME(n, …)` and `n:NAME(…)`.
    let both = |name: &str, f: mlua::Function| -> mlua::Result<()> {
        node.set(name, f.clone())?;
        methods.set(name, f)
    };
    // `kawoosh.node.parent(n)`: the node around it, nil at the root.
    let s = sh.clone();
    both(
        "parent",
        lua.create_function(move |lua, t: Table| {
            s.with_node(&t, |_, b, h, n| s.opt_table(lua, b, h, n.parent()))
        })?,
    )?;
    // `kawoosh.node.children(n, { anonymous = })`: its children in
    // order, the named ones unless `anonymous` asks for tokens too.
    let s = sh.clone();
    both(
        "children",
        lua.create_function(move |lua, (t, opts): (Table, Option<Table>)| {
            let anon = anonymous(&opts)?;
            s.with_node(&t, |_, b, h, n| {
                let out = lua.create_table()?;
                let mut c = n.walk();
                if c.goto_first_child() {
                    loop {
                        let child = c.node();
                        if anon || child.is_named() {
                            out.push(s.table(lua, b, h, child, Some(c.field_name()))?)?;
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                Ok(out)
            })
        })?,
    )?;
    // `kawoosh.node.child(n, i, { anonymous = })`: the `i`th child,
    // from 1, `-1` the last; named ones unless `anonymous`.
    let s = sh.clone();
    both(
        "child",
        lua.create_function(move |lua, (t, i, opts): (Table, i64, Option<Table>)| {
            let anon = anonymous(&opts)?;
            s.with_node(&t, |_, b, h, n| {
                let mut kids = Vec::new();
                let mut c = n.walk();
                if c.goto_first_child() {
                    loop {
                        if anon || c.node().is_named() {
                            kids.push((c.node(), c.field_name()));
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                let at = if i < 0 { kids.len() as i64 + i } else { i - 1 };
                match usize::try_from(at).ok().and_then(|at| kids.get(at)) {
                    Some((k, f)) => Ok(LV::Table(s.table(lua, b, h, *k, Some(*f))?)),
                    None => Ok(LV::Nil),
                }
            })
        })?,
    )?;
    // `kawoosh.node.get(n, field)`: the child filling `field` —
    // `condition`, `body`, `operator` (a token, too) — or nil.
    let s = sh.clone();
    both(
        "get",
        lua.create_function(move |lua, (t, field): (Table, String)| {
            s.with_node(&t, |_, b, h, n| match n.child_by_field_name(&field) {
                Some(c) => Ok(LV::Table(s.table(lua, b, h, c, Some(Some(&field)))?)),
                None => Ok(LV::Nil),
            })
        })?,
    )?;
    // `kawoosh.node.next(n, { anonymous = })`: the sibling after it.
    let s = sh.clone();
    both(
        "next",
        lua.create_function(move |lua, (t, opts): (Table, Option<Table>)| {
            let anon = anonymous(&opts)?;
            s.with_node(&t, |_, b, h, n| {
                let next = if anon {
                    n.next_sibling()
                } else {
                    n.next_named_sibling()
                };
                s.opt_table(lua, b, h, next)
            })
        })?,
    )?;
    // `kawoosh.node.prev(n, { anonymous = })`: the sibling before it.
    let s = sh.clone();
    both(
        "prev",
        lua.create_function(move |lua, (t, opts): (Table, Option<Table>)| {
            let anon = anonymous(&opts)?;
            s.with_node(&t, |_, b, h, n| {
                let prev = if anon {
                    n.prev_sibling()
                } else {
                    n.prev_named_sibling()
                };
                s.opt_table(lua, b, h, prev)
            })
        })?,
    )?;
    // `kawoosh.node.closest(n, types)`: the node itself or the nearest
    // around it whose type is `types` — a string, or a list of them.
    let s = sh.clone();
    both(
        "closest",
        lua.create_function(move |lua, (t, types): (Table, LV)| {
            let types: Vec<String> = match types {
                LV::String(s) => vec![s.to_str()?.to_string()],
                LV::Table(l) => l.sequence_values::<String>().collect::<mlua::Result<_>>()?,
                _ => return Err(mlua::Error::runtime("closest: a type or a list of types")),
            };
            s.with_node(&t, |_, b, h, n| {
                let mut at = Some(n);
                while let Some(n) = at {
                    if types.iter().any(|t| t == n.kind()) {
                        return Ok(LV::Table(s.table(lua, b, h, n, None)?));
                    }
                    at = n.parent();
                }
                Ok(LV::Nil)
            })
        })?,
    )?;
    // `kawoosh.node.text(n)`: its text.
    let s = sh.clone();
    both(
        "text",
        lua.create_function(move |_, t: Table| {
            s.with_node(&t, |_, b, _, n| Ok(b.snapshot.slice(n.byte_range())))
        })?,
    )?;
    // `kawoosh.node.select(n)`: the selection over it, in visual mode,
    // the head on its last character — as `<A-o>` leaves one.
    let s = sh.clone();
    both(
        "select",
        lua.create_function(move |_, t: Table| {
            let (h, r) = s.with_node(&t, |_, _, h, n| Ok((h, n.byte_range())))?;
            // The handler floors an offset to its character's start.
            let head = if r.end > r.start { r.end - 1 } else { r.start };
            s.queue.borrow_mut().push(Msg::SetSelections {
                buffer: h,
                sels: vec![(r.start, head)],
                primary: 0,
                visual: true,
            });
            Ok(())
        })?,
    )?;
    // `kawoosh.node.query(source, where)`: a tree-sitter query's
    // matches over a node, or a buffer's whole tree (a handle, or nil
    // for the current buffer) — each `{ pattern, captures = { NAME =
    // node }, all = { NAME = { node, … } } }`; nil and why when there
    // is no tree of the text.
    // @return kawoosh.NodeMatch[]?
    // @return string? why
    let s = sh.clone();
    let query = lua.create_function(move |lua, (source, at): (String, LV)| match at {
        LV::Table(t) => s
            .with_node(&t, |_, b, h, n| s.query(lua, b, h, n, &source))
            .map(|m| (LV::Table(m), None)),
        other => {
            let h = match other {
                LV::Nil => None,
                v => Some(
                    v.as_u64()
                        .ok_or_else(|| mlua::Error::runtime("query: a node or a buffer"))?,
                ),
            };
            let p = s.pp.borrow();
            let Some(h) = h.or(p.current) else {
                return Err(mlua::Error::runtime("no current buffer"));
            };
            match tree_of(&p, h) {
                Ok((tree, b)) => Ok((
                    LV::Table(s.query(lua, b, h, tree.root_node(), &source)?),
                    None,
                )),
                Err(why) => Ok((LV::Nil, Some(why))),
            }
        }
    })?;
    node.set("query", query)?;
    // `n:query(source)`: `kawoosh.node.query(source, n)`.
    let q: mlua::Function = node.get("query")?;
    methods.set(
        "query",
        lua.create_function(move |_, (t, source): (Table, String)| {
            q.call::<mlua::MultiValue>((source, t))
        })?,
    )?;
    k.set("node", node)?;
    Ok(())
}
