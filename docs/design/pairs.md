# Auto-closing brackets: a plugin, on by default

Status: decided 2026-09-21 (roadmap step 9), built 2026-09-23 (step
16; "Built" at the end says where it departed); on by default since the
same evening, at the user's word after a day's use — Decision 1's "off
by default" is the one reversed, `pairs.enabled = false` the way out. The shape the
roadmap's engine track recommended, with the three engine doors it
needs named, and the first plugin whose acceptance test is a `kawoosh
test` script (roadmap step 8). Each decision keeps the alternative it
beat.

## The thesis

Auto-pairing is contested: helix has it on, vim has none, nvim has it
by plugin, and with several carets it is the feature most often wrong.
So it is not the engine's — the engine holds no opinion about what a
`(` means — but a bundled plugin, gated on `pairs.enabled` (on; it was
decided off, and reversed after a day's use — Status), in
a hundred lines of Lua the user can read and rewrite; exactly the
behaviour "hackable by design" ([mvp.md](mvp.md)'s principle) says a user
should be able to switch and reshape. `gsa` (surround) already covers
the after-the-fact case, and wrapping a selection is `gsa(`.

What it is: `(` types `()` with the caret between; `)` before a `)`
steps over it; `<BS>` between a pair deletes both; `<CR>` between `{`
and `}` opens the block; a quote pairs only where a quote can open.

## Decisions

### 1. `pairs.lua`, bundled, gated on a setting

`kawoosh/lua/pairs.lua` is loaded with the other bundled plugins and
does nothing while `pairs.enabled` is `false` (it is `true` by default
since the reversal in Status; it was decided `false`). Its rules
are a settings table merged over a default, per language:

```lua
pairs = {
  enabled = true,
  rules = {
    default  = { { "(", ")" }, { "[", "]" }, { "{", "}" },
                 { '"', '"' }, { "'", "'" }, { "`", "`" } },
    rust     = { ["'"] = false },          -- lifetimes
    markdown = { { "*", "*" }, { "_", "_" } },
  },
}
```

A language's table adds pairs and removes with `false`. The plugin
binds, in insert mode, every distinct opener and closer, `<BS>` and
`<CR>`, with `when = { "!prompt", "!field" }` so a prompt and a Lua
pane's field keep typing plain (the conditions the picker's field
round added). It reads `pairs.enabled` on each key.

*Beat:* an engine feature with a setting — the roadmap's argument: the
behaviour is opinion, and the engine holds none about text.

### 2. The plugin does the default itself

An insert-mode key is looked up before it is typed (`Editor::key`'s
insert arm: an exact binding runs, else the stroke's text is inserted),
and a Lua command cannot decline to the binding under it. So the
plugin's function always runs and, when its rule does not apply, does
the plain thing itself: `kawoosh.buf.type("(")`, or the engine's
`delete char back` and `insert newline` through `kawoosh.cmd`.

*Beat:* a decline protocol (`return false` falls through to the older
binding) — worth having in general, not needed here, its own round.

### 3. Every caret, in one step

The rule is read per selection: `kawoosh.buf.selections()` gives every
anchor and head, `kawoosh.buf.line` the character before and after
each head. When every caret agrees, the action is one engine call for
all of them — `kawoosh.buf.type("()")` then `kawoosh.cmd("move left")`,
since the engine's `text` and its motions already run per selection.
When they disagree — one caret before a `)` and one not — the plugin
builds one edit per caret and applies them in one
`kawoosh.buf.edits({ { from, to, text }, … })`, which is
`Editor::apply_edits` (roadmap step 7): one undo node, every
selection carried; then it places the carets with
`kawoosh.buf.set_selections({ { anchor, head }, … })`.

Undo needs nothing: insert-mode edits coalesce into the insert's node
as they do. `.` needs nothing: the binding is recorded as a
`Step::Command`, and replaying it runs the function against the buffer
as it then is.

The three engine doors, in `lua/src/lib.rs`, each a few lines over
what exists:

- `kawoosh.buf.type(text)` — `Editor::text`, what a keystroke does, at
  every caret;
- `kawoosh.buf.edits(list)` — `Editor::apply_edits`;
- `kawoosh.buf.set_selections(list)` — the plural of `set_cursor`.

*Beat:* a loop of `kawoosh.buf.insert` per caret — the offsets shift
under the loop, which is the "cheap to get wrong".

### 4. The rules are the standard ones

- An opener pairs when the character after the caret is not a word
  character (`foo|bar` typing `(` gives `foo(|bar`, not `foo(|)bar`).
- A closer typed before the same closer steps over it — always, not
  only over one the plugin typed (helix's rule; tracking which `)` was
  typed by whom is the fragile part of nvim-autopairs).
- A quote pairs when neither neighbour is a word character and the
  one before is not a backslash; before the same quote it steps over.
- `<BS>` with the caret between an opener and its closer deletes both.
- `<CR>` with a closer right after the caret and its opener right
  before opens the block: newline, newline, up, one indent — spelt with
  the engine's `insert newline` (which keeps the indentation) and
  `insert tab`; the test says `{\n    |\n}` (the tab the buffer's
  indentation, four spaces by default).
- Nothing on `<Esc>`: a pair typed and left empty stays (helix's
  choice; deleting it surprises more than it helps).

### 5. The acceptance test is a script

`kawoosh/lua/tests/pairs.lua`, on the harness: off when set so (`i(`
gives `(`); `:set +pairs.enabled`; `i(` gives `()` with the caret
between; `)` steps over; `<BS>` deletes both; `<CR>` opens a block;
`"` after a word does not pair; `'a` in a rust buffer does not pair;
two carets (`<C-j>` then `i(`) both paired; `.` after `a(` repeats it.
The three doors get unit tests in `lua/src` beside `insert`'s.

### Deliberately not

- **A pair-aware `x`** in normal mode.
- **Rainbow brackets** — a highlight, the ts thread's, another item.
- **Wrapping a selection by typing `(`** — that is `gsa(`.
- **A decline protocol for Lua bindings** — Decision 2.

## Build order

One commit: the three doors with their unit tests; `pairs.lua`; the
defaults `pairs.enabled` and `pairs.rules` in the engine's layer; the
script; keys.md's insert-mode line (with `pairs.enabled`, an opener
pairs, a closer steps over, `<BS>` and `<CR>` know the pair); the
roadmap's item struck.

## Risks

- **A binding on `(` shadows a user's own.** The newest binding wins,
  which is the keymap's rule; a user who maps `(` in insert mode after
  the plugin loads is on top, as they expect.
- **The field conditions.** The bindings must not fire in the command
  line or a picker's field; `!prompt` and `!field` are the spellings
  the keymap has today — check they hold in a Lua pane's field on day
  one, and add the fact if one is missing.
- **`type` from inside a binding.** It runs where `paste clipboard`'s
  insert-mode path runs the engine's `text` from a command already;
  the same path, nothing new.

## Built

2026-09-23, as decided but for these:

- **The keys are gated on a fact, not read on each key.** While
  `pairs.enabled` is off the plugin's bindings do not hold (`when = {
  "pairs", "!prompt", "!field" }`, the `pairs` fact set from
  `on_settings`), and a gated-off binding falls through: `<BS>` and
  `<CR>` to the engine's own under them, and a typing key to typing —
  which the engine did not do (a key whose every binding was gated off
  was refused, its character lost), fixed in `Editor::key`'s insert
  arm on the way. So Risk two's worry held, and the fix is the
  engine's, not the plugin's. Decision 2's "does the default itself"
  stays for when the plugin is on and a rule does not apply.
- **One path for every caret.** Decision 3's two — one engine call
  when the carets agree, `edits` when they do not — are one: each
  caret's action is an edit, all of them one `kawoosh.buf.edits`, the
  carets put back with `set_selections`; a step over is a caret moved
  with no edit. `<CR>` is the exception: the block opens only when every
  caret is between a pair, through the engine's `insert newline`,
  `move up`, `line end insert`, `insert tab`; else a plain newline.
- **A fourth door**, `kawoosh.buf.slice(from, to)`: the character on
  either side of a caret without the whole text, held to characters.
- **The rules' defaults are the plugin's**, not the engine's layer: a
  table keyed by `'` is not a settings path. `pairs.enabled` is the
  engine's default; `pairs.rules` is read when set.
- **No markdown `*` `_` by default**: a `*` at a line's start is a
  list's bullet more often than emphasis, and a pair there is in the
  way. A `markdown` table in `pairs.rules` adds them.
- **Edits are disjoint, and the primary stays** (from the branch's
  review): two carets at `(|)|` and `<BS>` made a pair's deletion and
  the `)`'s overlap, which the engine applied one after the other in a
  text the first had moved — a character too many gone, or one cut in
  half. `kawoosh.buf.edits` refuses a backwards or overlapping range
  with an error, `Editor::apply_edits` clamps one that reaches the next
  whatever sent it, and the plugin starts a caret's deletion where the
  one before ended. `set_selections` made the first selection the
  primary, so every pairing moved `<C-j>`'s primary to the top: the
  selections `kawoosh.buf.selections()` gives mark the primary
  (`primary = true`), `set_selections` takes the mark back, and the
  plugin keeps it. `set_selections` also snaps an offset inside a
  character to its start.

`kawoosh/lua/pairs.lua`, `kawoosh/lua/tests/pairs.lua`, and
`type_edits_selections_and_slice` in `lua/src/lib.rs`.
