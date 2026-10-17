# The Lua API's types

Status: asked 2026-10-17 — the generated types are weak:
`function kawoosh.map(mode, keys, cmd, opts) end` is all `any`, so
completion works badly. The calls below are taken here, each the
user's to overturn. Companion to [lua-boundary.md](lua-boundary.md)
(what is Lua's and what is Rust's) and roadmap step 12 (the meta files
themselves, `kawoosh/src/types.rs`).

## What there was

At launch kawoosh writes `kawoosh.lua`, a lua-language-server
`---@meta` file, from the live runtime (`lua/src/meta.rs`): every name
under `kawoosh`, a Lua function's parameters off the line it is
defined on and its doc off the comment above, a Rust one's off a
backticked spelling in its doc comment (`` `kawoosh.buf.close(buffer,
{ force = })` ``). A parameter's type was guessed from its name — `opts`
and `t` a `table`, `fn`, `f`, `cb`, `callback` and `on_*` a `function`,
anything else `any` — every parameter past the first was marked
optional, and every answer was `any` unless the doc said `@return`. In
the user's file 437 of 517 `@param` lines were `any`, 421 answers too,
and no option table had a field: `opts` was a bare `table`, so neither
its keys completed nor a wrong one was caught. No Lua source carried a
single LuaLS annotation.

## Decisions

### 1. A type is said where the function is, never guessed

The name-based guess is gone: a parameter is typed by what its source
says, or it is untyped. An untyped parameter is still declared — `any`
and optional, so a call is never flagged for it — and counted
(Decision 5). The same for answers: an answer nobody said is `any`,
never "nothing", which would make every use of it a field of `nil`.

Alternative beaten: better guesses (a table of known names, types read
off the Rust closures' `(usize, Option<u64>)`). A guess cannot say
which keys an option table takes nor what a callback is given, which
is what completion is for; the closure types say `Table` for every
table and nothing about answers, and a typed doc is a line beside them.

### 2. A Lua function: LuaLS annotations in its comment

A function written in Lua (`boot.lua`, `kawoosh/lua/*.lua`, a user's
plugin) is typed the way any Lua is for lua-language-server: `---@param`
and `---@return` lines in the comment above it. The walk passes them
through and writes no `@param` of its own for a parameter the doc
types; a parameter the doc leaves out is untyped.

```lua
-- kawoosh.map(mode, keys, cmd[, opts]): …
---@param mode kawoosh.Mode|kawoosh.Mode[]
---@param keys string
---@param cmd string|fun(ctx: kawoosh.Ctx)
---@param opts? kawoosh.MapOpts
---@return nil
function kawoosh.map(mode, keys, cmd, opts)
```

The file itself is then checked as it is edited, not only its callers.

### 3. A Rust function: the typed signature its doc starts with

A function seeded from Rust has no Lua line to annotate; its doc
comment's first line has spelled it since step 12. That spelling is
now typed, LuaLS's own words in TypeScript's order:

```rust
/// `kawoosh.buf.close(buffer?: integer, opts?: kawoosh.CloseOpts): nil`:
/// the buffer closed …
```

- `name: type` a parameter a call must pass, `name?: type` one it may
  leave off, `...: type` the rest. Optionality is said, not "every one
  past the first".
- The answers after the parentheses, comma-separated:
  `` `kawoosh.node.at(where: integer, buffer?: integer): kawoosh.Node?, string?` ``.
- The spelling may run over lines; it ends at the closing backtick.
- `@param` and `@return` lines in the doc, for a parameter that wants
  a description, win over the signature's.
- Where two docs spell one name (a `Msg` variant's notes and the
  binding's), the typed one is read.

Alternative beaten: `@param` lines in every Rust doc. Two hundred
functions would each grow three to six lines restating what one line
already says, and the one line is what a reader of the Rust looks at.

### 4. Option and data tables are classes, declared once beside their user

A `---@class` / `---@alias` block anywhere in the sources the walk
reads — a Rust doc comment (`/// @class kawoosh.CloseOpts`, `/// @field
force? boolean …`), a Lua comment — is lifted out of whatever doc it
sits in and declared once, after the walk. It is written where the
function that takes it is: `kawoosh.MapOpts` beside `kawoosh.map`,
`kawoosh.PickerSource` at the head of `picker.lua`. A block is a
`@class` line and the `@field` lines right under it (or an `@alias`
line and its `---|` lines), one line each — LuaLS reads a field off
one line.

A class named as a table the walk reaches (`kawoosh.picker`) is that
table's header, and the fields it declares are left to it: the walk
does not go into `kawoosh.picker.sources` and write each bundled
source's functions with their own untyped parameters, the class says
`---@field sources table<string, kawoosh.PickerSource>` — which is what
a user adding a source is checked against.

A name declared twice is dropped the second time and fails the tests:
one declaration, one place to change it.

Alternative beaten: one shipped `types.lua` of every class. It would
drift from the functions it describes; a class beside its function is
changed with it.

### 5. The ratchet: the untyped may only get fewer

`Meta::untyped` lists every untyped parameter and answer
(`kawoosh.map(mode)`, `kawoosh.map: return`). Two tests hold it to a
number: `lua/src/meta.rs`'s `CORE_UNTYPED` over the core — the boot
script and the `lua` crate's Rust half — and `kawoosh/src/types.rs`'s
`UNTYPED` over the whole API as a launch writes it, the shell's doors
(`DOORS`) and the bundled plugins too. More than the number fails with
the list (a new function nobody typed); fewer fails asking for the
number to be lowered, so it never drifts back up. The path is to zero:
the core is held there now.

### 6. Checking with lua-language-server

`nu scripts/lua-types.nu` writes the meta (the ratchet test, with
`KAWOOSH_META_OUT`), puts it in a temporary workspace beside a sample
`init.lua` of typed calls and one deliberate type error, and runs
`lua-language-server --check` over it: a malformed annotation, a class
used and never declared, or a sample call flagged is a failure; the
deliberate error not being flagged is one too.

## Built

<!-- numbers filled in when the round lands -->
