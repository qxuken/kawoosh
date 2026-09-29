# Node actions

Status: decided and built 2026-09-29; the calls taken here, each the
user's to overturn, and three settled with the user (below). Where the
build moved a call, the section says so, and "Built" at the end. Asked: ckolkey/ts-node-action's node actions — one key that
does what the node under the caret means, "fliping a boolean or change
block indentation" — on [nodes.md](nodes.md)'s `kawoosh.node`.

## What there is

`kawoosh.node` (nodes.md, built): the tree as plain tables, `at`, `leaf`,
`closest`, `get`, `children`, `text`, read only while it is the text's.
`gra` lists the language server's code actions in the picker
(`lsp action`); a server's refactors are there, when it has any.
`kawoosh.buf.edits` is one undo step. `.` replays the engine's commands
and Lua's, but not the shell's (`Editor::step_end`: "what the shell
runs is not one"). The picker takes a source from Lua
(`picker.open(def)`).

ts-node-action: a table per filetype, from a node's type to a function
or a list of them. The function gets the node under the cursor and
returns its replacement — a string or lines — and options: where the
cursor goes, a callback, whether to reindent. A list is offered in a
`vim.ui.select`. Its helpers are the actions: `toggle_boolean`,
`toggle_operator`, `toggle_multiline` (the children joined on one line,
or one a line), `cycle_quotes`, `cycle_case`, `toggle_int_readability`,
and a few per language (ruby's blocks, a conditional to a ternary).

## Decisions

### 1. An action is a name, where it applies, and a function

```lua
kawoosh.node.action("flip", {
  doc = "true ↔ false",
  languages = "*",                   -- or { "rust", "python", … }
  types = { "true", "false", "boolean_scalar", "boolean" },
  run = function(n, ctx)
    return ({ ["true"] = "false", ["false"] = "true",
              True = "False", False = "True" })[n:text()]
  end,
})
```

`types` are node types, tokens as well as named nodes — `==` is a type.
`run(n, ctx)` answers one of:

- a string: the node's new text;
- `{ text = …, cursor = i }`: that, the caret `i` bytes into it;
- `nil`: not this one — the dispatch goes on as if the action did not
  take the type (a flip on a word that is neither, a join over a
  comment).

`ctx` is `{ buffer, language, caret, indent, unit }`: the caret's offset,
the leading whitespace of the node's first line, and one indent's text
(`kawoosh.buf.indent().unit`). `kawoosh.node.action(name, nil)` removes
one; the same name again replaces it, so `init.lua` redefines a shipped
action by its name. The registry and the dispatch are a bundled plugin,
`kawoosh/lua/node_actions.lua`, on the public API alone — the dogfood
nodes.md asked for.

Beaten: **a list of edits** as the answer. Every action ts-node-action
ships replaces its node, and an action that means to change more than
its node is an action on a larger node; the edits answer waits for one
that cannot be put so.

### 2. The innermost node an action takes, up to a body

The dispatch starts at the caret's leaf — a token included — and climbs:
the first node, innermost first, that an action takes and does not
decline is acted on. On `x` in `f(x, true)` that is the argument list
(split); on `true` it is the boolean (flip), the list around it next.
The climb stops at a node that fills a `body` field — a function's, a
loop's, a closure's: the field most grammars name the same (rust has
sixteen such kinds, javascript twenty-one, python thirteen). So a caret
in a closure passed to a call does not split the call it is in, however
far out the call's parenthesis is.

Beaten: **the node under the caret only**, ts-node-action's. A list is
split from its parenthesis and nowhere else, and the caret is always on
an item. **The outermost**: the file's first list.

### 3. What the caret does

It keeps its distance from the node's start, held to the new text's last
character: a flip leaves it where it was. `cursor` in the answer says
otherwise — `split` answers `cursor = 0`, the opener, since a kept
distance lands in the new indentation. From visual mode the
selection's bytes pick the node (as `<A-o>` reads them) and the action
leaves normal mode.

### 4. Several carets

Each caret finds its own node in the one tree, and the edits go as one
`kawoosh.buf.edits` — one undo step. Two carets on one node act once.
When one caret's node lies inside another's (a flip on `true`, a split
of the list around it), the outer one is done and the inner skipped,
and the message says how many were: an inner edit would be written
into text the outer one replaced.

### 5. The key runs the first; `:node actions` lists them all

`g.` runs the first action that takes the caret's node (§2), among the
actions for a node the newest defined first — a plugin's or yours
before a shipped one, as the newest binding of a key wins. The message
names it: `split · arguments`. `:node actions` opens the picker on every
action that would take a node from the caret up to the body, innermost
first (`flip · true`, `split · arguments`, `split · array_expression`),
and runs the one picked; `:node action NAME` runs one by name, and
`:node action NAME N` on the Nth node up that it takes.

Built: the picker lists from the primary caret and acts there alone,
on the buffer and the selections it was opened from — a pick runs
while the picker's own field still has the keys, so it cannot read
"the current buffer".

`g.` because keymap-regroup.md's rule 3 wants a key without a leader
where one is free and vim does not own it, and vim has no `g.`; the file
manager's `g.` (hidden files) is local to a listing, which has no nodes.

### 6. `.` repeats it

Flip one boolean, `.` on the next. Built: this needed nothing. The
note had it that `.` skips a Lua command, reading `step_end`'s "what
the shell runs is not one"; but a Lua command is registered with the
engine with a body (`register_spec`) and its edits are applied inside
that body, so it is recorded and replayed like any engine command —
only the shell's own Rust commands are not. The `repeat = true` flag
the note proposed was not built.

### 7. What ships

Five actions, in `node_actions.lua`, each a table a user or plugin can
change:

- **flip**: `true` ↔ `false`, `True` ↔ `False`, on the tokens and
  named booleans of every grammar that has them (yaml's
  `boolean_scalar`, toml's `boolean`).
- **operator**: `==` ↔ `!=`, `===` ↔ `!==`, `&&` ↔ `||`, `and` ↔ `or`,
  `<` ↔ `>`, `<=` ↔ `>=` — on a token that fills an `operator` field
  (python's `operators`), so rust's `Vec<T>` is no comparison.
- **split** (and join): a list on one line becomes one item a line,
  each at the node's line's indent and one unit more, the closer on a
  line of its own; a list on several lines is joined. Per language, the
  lists and how each is written, as the language's formatter writes it:

  | language | lists | a trailing comma when split | inner spaces when joined |
  |---|---|---|---|
  | rust | `arguments` `parameters` `array_expression` `tuple_expression` `use_list` `field_initializer_list` | yes | `field_initializer_list` |
  | javascript, typescript, tsx | `arguments` `formal_parameters` `array` `object` `named_imports` `object_pattern` `array_pattern` | yes | `object` `named_imports` `object_pattern` |
  | python | `argument_list` `parameters` `list` `tuple` `dictionary` `set` | yes | — |
  | go | `argument_list` `parameter_list` `literal_value` | yes (go requires it) | — |
  | lua | `arguments` `parameters` `table_constructor` | the table's only (a call's is an error) | `table_constructor` |
  | json, jsonc | `array` `object` | no | `object` |
  | c, cpp | `argument_list` `parameter_list` `initializer_list` | the initializer's only | — |
  | toml | `array` | yes | — |

  A join declines over a comment among the items: a line comment would
  swallow what followed it.
- **quotes**: `"…"` → `'…'` → `` `…` `` (javascript's family) → `"…"`,
  in javascript, typescript, tsx, python and lua; a prefix (python's
  `f`, `r`, `b`) kept. It declines when the text holds the quote it
  would change to, holds an escape, is python's triple-quoted, or is a
  template with a `${`.
- **digits**: `1000000` ↔ `1_000_000`, a decimal literal of five digits
  or more, in rust, python, javascript, typescript and go — a suffix
  (`1000000u64`) kept, a hex, octal or binary literal declined.

`node_actions.NAME = false` in the settings turns a shipped one off.
`kawoosh.node_actions.lists` is `split`'s table, and a list added to it
is split — `split`'s `types` is a function reading it (`types` may be
a list or a function).

Not shipped: **cycling an identifier's case** (ts-node-action's
`cycle_case`). It changes one occurrence of a name and leaves the others,
which is a rename done wrong; `grn` asks the server to do it right. **A
conditional to a ternary and back**: per language, with its own
questions (an `else if`, a statement against an expression); a second
round, if missed.

## Decided with the user

2026-09-29: `g.` for the key; `<` ↔ `>` mirrored, as ts-node-action
does, not negated; node actions kept apart from `gra` — the server's
menu stays the server's.

## Built

`kawoosh/lua/node_actions.lua`, loaded after the picker: the registry
(`kawoosh.node.action`, `kawoosh.node_actions.list`), the dispatch
(`M.answers` from a place, `M.run` over the carets), `g.` in normal and
visual mode, `:node action [NAME [N]]`, `:node actions`, the
`node_actions` setting, and the five actions with their tables. No
engine change (§6). `help/editing.md` "Node actions", `help/lua.md`,
keys.md. `kawoosh/lua/tests/node_actions.lua`: each action in the
languages above, a decline falling through, the climb stopping at a
closure's body, the type's `<`, a join refused over a comment, a
one-item tuple and a padded struct literal, two carets and a node
inside another's, `.`, one `u`, visual mode, the setting, a plugin's
action first, the picker's second pick, a list added to the table.

## Left

- Edits beyond the node (§1), when an action needs them.
- A conditional ↔ ternary, per language.
- Actions inside an injected language (a markdown fence's rust), with
  nodes.md's injected trees.
