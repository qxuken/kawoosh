# The syntax tree in Lua

Status: decided and built 2026-09-29, the calls taken here, each the
user's to overturn; where the build moved a call, the section says so. Asked, after `<A-u>` (`node parent`): "design the
Lua node API", with ckolkey/ts-node-action as what it should make
possible — a boolean flipped, an argument list split onto its lines
and joined back.

## What there is

The ts thread keeps a tree per buffer, parsed incrementally from the
journal's edits, and each answer hands the shell a `Tree` — a count on
nodes shared with the thread's copy, not a walk — kept as
`inspector.trees` beside the version it was parsed from. Four things
read it, all Rust: `nodes.rs` (`<A-o>` `<A-i>` `<A-n>` `<A-p>`
`<A-u>`), the inspector (`:syntax_tree`, which shows each node's type
and the field it fills), the outline, and the breadcrumbs. An injected
language's tree (a markdown fence's rust) is parsed for a paint and
dropped; only the buffer's own language's is kept.

Lua has none of it. `kawoosh.buf` reads a buffer from `Published`, the
snapshot the shell publishes each frame — its text, its version, the
selections — and a change (`edits`, `replace`, `set_selections`) is a
message queued and applied after the call returns. So inside one call
the text a plugin reads never moves under it.

ts-node-action splits its work the same way this note does: tree-sitter
says which node is under the cursor, its type, its range and its
children; the action — a word flipped, children joined with `, ` — is
plain text logic in Lua, looked up by the node's type in a table per
language. What it needs from the editor is the tree, read; the edit it
makes is an ordinary replace.

## Decisions

### 1. A node is a table; the tree stays in Rust

`kawoosh.node.at()` returns a plain table:

```lua
{
  type = "call_expression", -- the grammar's kind
  named = true,             -- false for a token such as `(` or `==`
  field = "value",          -- the field it fills in its parent, or nil
  from = 120, to = 164,     -- bytes from 0, `to` exclusive, as kawoosh.buf's
  line = 7, end_line = 9,   -- from 1
  error = false,            -- an ERROR or MISSING node itself
  has_error = false,        -- one somewhere inside it
  language = "rust",        -- the grammar it is from
  buffer = 3, version = 41, id = 94118273,
}
```

Its functions sit on one shared metatable, so `n:parent()` reads as a
method while `pairs(n)`, `kawoosh.test.eq(n.type, …)` and `:lua
=kawoosh.node.at()` see data; each is `kawoosh.node.NAME(n, …)` too.
`tostring(n)` is `call_expression 120..164`. To walk from a table, Rust
finds its node again at the table's version: down through the nodes
over `from..to` to the one with its `id` — as deep as the tree, times
the siblings on the way.

Built: the `id` alone did not hold. The ts thread may answer one
version twice, the second a whole parse whose nodes have new ids, so a
node read before it was not found. The same version is the same text,
so the node of the table's `type` over exactly `from..to` (the
innermost, if several) is the same node, and is taken when the id is
not there; `==` compares buffer, version, range and type, not the id.

Beaten: **a userdata handle**, neovim's `TSNode`. Each step is a pointer
move rather than a re-find, but a handle is not data — it prints as an
address, compares by identity, and a plugin that keeps one keeps a
whole old tree alive. **The whole tree as tables**: a 20 000-key JSON
file is several hundred thousand tables for one question.

### 2. The tree of the text Lua reads, or none

The shell hands the runtime each tree as the ts thread answers it
(`Runtime::set_tree`, beside `inspector.trees`), and `kawoosh.node`
reads it only when its version is the published snapshot's, so the
tree a plugin walks is always the text `kawoosh.buf.slice` reads —
offsets from one are good in the other.
When the tree is behind the text (a frame or two after a keystroke) or
the language has no grammar, `kawoosh.node.at()` returns `nil` and the
reason, in `<A-o>`'s words: `the syntax tree is behind the text: again
in a moment`, `no syntax tree for NAME`. A node kept past its version
and walked later is an error naming both versions — a stale node would
answer in offsets of a text that is gone, and quietly.

Since edits are queued, an action reads every node it needs from one
tree and sends one `kawoosh.buf.edits` batch: one undo step, offsets
all in the text as it was.

Beaten: **parsing on demand** when the tree is behind. The parser and
its incremental state live on the ts thread; a second one on the main
thread doubles a big file's memory and parses it whole, in the
keystroke. A `kawoosh.node.wait(fn)` for scripts that edit and then
read the tree is left until a plugin needs one; a test has
`kawoosh.wait`.

### 3. Three ways in

- `kawoosh.node.at(where, buffer)`: the smallest **named** node over
  `where` — what `<A-o>` selects first.
- `kawoosh.node.leaf(where, buffer)`: the smallest node, a token
  included — `true`, `==`, `"`: where a flip or a cycle starts.
- `kawoosh.node.root(buffer)`.

`where` is an offset, a `{ from, to }` range, or nil for the primary
selection: the caret's character, or in visual mode the selected bytes
— the same bytes `<A-o>` reads. `buffer` defaults to the current one,
as in `kawoosh.buf`. Several carets are the caller's loop over
`kawoosh.buf.selections()`, each finding its own node (mvp.md Decision
4); the API does not guess what a plugin wants from each.

### 4. Walking: named by default, tokens asked for

| function | what |
|---|---|
| `n:parent()` | the node around it, nil at the root |
| `n:children(opts)` | its children, in order |
| `n:child(i, opts)` | the `i`th, from 1; `-1` the last |
| `n:get(name)` | the child filling the field `name` — `condition`, `body`, `operator` |
| `n:next(opts)` `n:prev(opts)` | the sibling after, before |
| `n:closest(types)` | itself or the nearest node around it whose type is one of `types` (a string, or a list) — the DOM's `closest` |
| `n:text()` | its text, read from the snapshot when asked |
| `n:select()` | the selection over it, visual, the head on its last character, as `<A-o>` leaves it |

`opts.anonymous = true` counts tokens too; without it they are skipped,
as `<A-o>` and the inspector's default view skip them. Built: the
note had `n:field(name)`, but a table's `field` is its own field name,
and a key cannot be both the data and the method — `n.field` would be
the function whenever the node fills none — so the method is `get`.
`get` finds a token as well — a binary expression's `operator` is `==` — since a field
names it. `closest` is most of what an action or a textobject asks: the
function around the caret, the list the caret is in. `text()` is a call
rather than a field so that the root of a 5 MB file costs nothing until
someone reads it.

### 5. Queries, over a node's range

`kawoosh.node.query(source, where)` runs a tree-sitter query over a
node (`where` a node), or the whole buffer (`where` a buffer handle or
nil), and returns its matches in the text's order:

```lua
for _, m in ipairs(kawoosh.node.query([[
  (function_item name: (identifier) @name body: (block) @body)
]], kawoosh.node.at():closest("impl_item"))) do
  print(m.captures.name:text(), m.captures.body.from)
end
```

Each match is `{ pattern = i, captures = { NAME = node }, all = {
NAME = { node, … } } }` — `captures` the first node of each capture,
which is every capture but a quantified one (`@arg+`), whose nodes
are all in `all`. (A quantifier repeats over adjacent siblings: an
argument list's `(_)+` is a match per argument, the `,` between them
breaking the run, as tree-sitter has it.) The query compiles against the tree's own grammar
(`Tree::language`), so a grammar added with `kawoosh.language` works
the same; compiled queries are kept by language and source, and one
that does not compile raises its row and column. The crate evaluates
`#eq?`, `#match?` and `#any-of?` against the text. It runs in the call,
bounded to the node's bytes, so a query over a function costs the
function.

Beaten: **walking only**. ts-node-action needs no more than §4, but a
textobject — `af`, `ic`, per language — is a query in every editor
that has them (helix's `textobjects.scm`, nvim-treesitter-textobjects),
and without one each plugin writes a walker per grammar.

### 6. The buffer's own language only, for now

A caret in a markdown fence's rust is in markdown's `code_fence_content`;
`n.language` says which grammar a node is from. Walking into an injected
language needs the ts thread to keep the trees it now drops, and the
published snapshot to carry each with its ranges: a step of its own,
when a plugin wants it.

### 7. The types say it

`kawoosh.lua`, the types file written for lua-language-server, gains
`---@class kawoosh.Node` with its fields and methods, so `n:` completes
in a plugin, and `at`, `leaf`, `root` and `query` say they return one.
Built with it: the types file reads a function's doc from `nodes.rs`
as well as `lib.rs`, from a `//` block too (the Rust half's functions
are set in `let`s, where rustc warns of a `///`), and a `[, buffer]`
in a doc's spelling is an optional parameter rather than the end of
the list — so twenty-odd functions documented that way got their docs
and their trailing parameters. The names a grammar gives — `boolean_literal`, the field
`consequence` — are read off `:syntax_tree`, which shows each node's
type and field under the caret; `help/lua.md` says so.

### 8. A hook for each new tree, once a frame

Asked 2026-10-03, from "Left" below: `kawoosh.on_tree(fn)`, for a
plugin that paints from the tree — a rainbow of brackets, a scope's
wash, a semantic colour the grammar's queries do not give.

`fn(root, changed)` after a frame in which a buffer's tree was parsed
again: `root` is `kawoosh.node.root(buffer)` — its `buffer`,
`language` and `version` on it, walked with §4 and queried with §5 —
and `changed` a list of `{ from, to }` byte ranges whose syntax
changed since the hook last heard of the buffer. They are the ts
thread's own spans, the ones its answer repaints (the edits and
`changed_ranges`, each widened to its neighbourhood), so a plugin
paints again what the highlighter did; the first time, and for a hook
set later, the whole text. It returns a function that takes the hook
off — the first `on_*` to; the others stay until the runtime goes.

It runs on the main thread, in the frame, so its cost is kept to what
it is asked for:

- **Coalesced.** Once a buffer a frame at most, every hook in one call
  into Lua, after the frame's answers are in — two answers in one
  frame are one word, their spans together.
- **Only a tree of the text as it is.** An answer behind the typing
  (the tree §2 would refuse) is not told; its spans wait, carried
  through the journal (`clamp_range`) to the text of the answer that
  catches up, which is told with them. Past 64 waiting spans, or a
  journal that no longer reaches back, it is the whole text.
- **Nothing while no hook is set.** The spans are gathered only while
  one is (`Runtime::tree_hooks`, a look at a Lua table a frame), and
  the snapshot is published for the call only when there is a tree
  to tell of.
- **Only what is parsed**: the buffers on show and a multibuffer's
  visible files — the ts thread's jobs. A buffer no pane shows has no
  new tree to tell of.

A hook new since the last frame (`kawoosh._tree_gen` moved) hears every
tree there is, whole, the next frame — so does every other hook,
which a painter takes as a repaint; a hook that fails is said on the
message line and the others still run.

Beaten: **the tree's diff as nodes** (neovim's `on_changedtree` gives
ranges too): a node table per changed node is a walk the plugin may not
want. **A call per answer**: during fast typing the thread answers
versions the text has already left, and each such call would read a
tree §2 refuses. **The hook told per buffer it asked for**
(`on_tree(buffer, fn)`): `root.buffer` and `root.language` filter as
cheaply in the hook, and a painter wants every buffer of its language.

## What it is for

ts-node-action's two examples, against this API — a command each, every
caret its own node, one batch of edits:

```lua
local flip = { ["true"] = "false", ["false"] = "true",
               ["True"] = "False", ["False"] = "True" }

kawoosh.command("node flip", function()
  local edits = {}
  for _, s in ipairs(kawoosh.buf.selections()) do
    local n = kawoosh.node.leaf(s.head)
    local to = n and flip[n:text()]
    if to then edits[#edits + 1] = { n.from, n.to, to } end
  end
  kawoosh.buf.edits(edits)
end, { when = { "editor", "!readonly" } })

local lists = { "arguments", "parameters", "array", "array_expression",
                "table_constructor", "object", "tuple_expression" }

kawoosh.command("node split", function()
  local n, why = kawoosh.node.at()
  local list = n and n:closest(lists)
  if not list then return kawoosh.echo(why or "no list here") end
  local items = list:children()
  if #items == 0 then return end
  local open = kawoosh.buf.slice(list.from, items[1].from)
  local close = kawoosh.buf.slice(items[#items].to, list.to)
  local texts = {}
  for i, it in ipairs(items) do texts[i] = it:text() end
  local text
  if list.line == list.end_line then
    local ind = kawoosh.buf.indent().unit
    text = open:gsub("%s+$", "") .. "\n" .. ind
      .. table.concat(texts, ",\n" .. ind) .. "\n" .. close:gsub("^[%s,]+", "")
  else
    text = open:gsub("%s+$", "") .. table.concat(texts, ", ")
      .. close:gsub("^[%s,]+", "")
  end
  kawoosh.buf.edits({ { list.from, list.to, text } })
end, { when = { "editor", "!readonly" } })
```

(The split's indent is the list's line's plus one unit in the real one;
the sketch keeps the arithmetic out of the way.)

Node actions proper — one key that does what the node under the caret
means, a table of actions by language and type that plugins add to — is
a bundled plugin on this, and a note of its own: which key, which
actions ship for which languages, where the caret lands after one.

## Build

1. `kawoosh-lua` depends on `tree-sitter` itself; `Published` gains
   the trees, which the shell sets as they come (`Runtime::set_tree`)
   and `publish` prunes to the open buffers.
2. `lua/src/nodes.rs`: the table ↔ node round trip (§1), `at`, `leaf`,
   `root`, the walking functions, the metatable, the version check.
3. `kawoosh.node.query` with its cache.
4. `meta.rs`: `kawoosh.Node`; `help/lua.md`: a "Syntax tree" section.
5. Tests: a Lua test (`kawoosh/lua/tests`) over a rust buffer — `at`,
   `leaf`, `closest`, `field("operator")`, a query's captures, `nil`
   and the reason while the tree is behind, a stale node's error; and
   `<A-u>` again in six lines of Lua, landing where the Rust one does,
   as the proof the surface is enough for what the shell does itself.

## Left

- Injected languages' trees (§6).
- `kawoosh.node.wait`. ~~A hook when a buffer's tree changes
  (`kawoosh.on_tree`), for a plugin that paints from the tree.~~ Built
  2026-10-03 (Decision 8): `kawoosh.on_tree` in `lua/lua/boot.lua`,
  `Runtime::tree_hooks` and `tree_hook` (`lua/src/lib.rs`), the spans
  gathered by `TreeNews` and told by `Kawoosh::tell_trees`
  (`kawoosh/src/nodes.rs`) from `sync_syntax`. Test:
  `kawoosh/lua/tests/on_tree.lua` (the whole text first, a span around
  an edit after, a late hook told whole, the handle, a failing hook).
- Queries a grammar ships by name — `textobjects.scm` beside
  `highlights.scm` — and `af` / `if` / `ac` on them.
- Node actions (above).
