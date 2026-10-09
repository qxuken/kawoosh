# Breadcrumbs

Status: decided and built 2026-09-29 (roadmap step 67), the calls taken
here, each the user's to overturn. Asked: "I miss breadcrumbs
navigating huge test files, let's add toggleable breadcrumbs."
Companion to [marks.md](marks.md), whose outline (Decision 1) is what
names the crumbs.

## What there is

The outline: `Ts::outline` answers a buffer's definitions, nested by
range, from a query per grammar; `grs` lists them as a tree and starts
on the symbol the caret is in. Nothing on screen says where the caret
is while it moves. In a test file there was less still: a `describe`
and an `it` are calls, not definitions, so javascript's and
typescript's outline had nothing for them, and a 3000-line spec was
one flat list of its `const`s.

## Decisions

### 1. On the pane's title bar, after the file's name

`parser.test.ts › parser › with a table › skips`: the symbols the caret
is inside, outermost first, after the name in the editor pane's own
title bar, dim, the innermost in the name's colour. Each pane its own,
so two splits of one file each say where their caret is.

Beaten: **a row of its own under the title** (VS Code's). It costs a
text row in every pane, and every piece of the view that counts rows —
the scroll, the wrapped rows' heights, the click's row — would learn
of it. The title bar is there already and mostly empty.
**Sticky headers** (nvim-treesitter-context): the lines of the
enclosing symbols pinned over the text. It shows more — a signature's
arguments — but takes rows that change height as the caret moves, and
in a nested spec it is four lines of `describe(` for one line of path.

### 2. The grammar's outline, not the server's

The crumbs read `Ts::outline` alone, whatever `symbols.source` says: it
is on the ts thread beside the tree, answers without a round trip, and
every language with a grammar has one — markdown's headings, toml's
tables. The server's `documentSymbol` would be a request per edit to a
process that may be busy indexing. A **local** — a variable inside
another symbol — is a crumb only when something the caret is in is
inside it (a closure assigned to it); a `let` in a test's body says
nothing the test's name does not. A top-level variable is one: a
fixture's big object literal is a place.

### 3. A test file's blocks are symbols

javascript's, typescript's and tsx's outline learn the test runners'
blocks: a call of `describe`, `context`, `suite`, `it`, `test`,
`specify` or `bench` — with `.only`, `.skip`, `.concurrent` after
the name, or `.each(table)` before the call — whose first
argument is a string or a template and which has a function argument,
named by its title (`reads a number`, `` `with ${input}` ``), kind
`test`. go's learns `t.Run("name", func…)`, a subtest under its test.
Rust's `#[test] fn`s in a `mod tests`, python's `class Test…` and
`def test_…` were symbols already. So `grs` lists a spec's blocks too.

Not done: busted's `describe(…, function() … end)` in Lua, and a
`describe.each` over a tagged template, until asked.

### 4. Asked again once the text is still

The outline is kept per buffer with the version it read. After each
frame's highlighting jobs, a buffer a pane with crumbs shows whose
outline is older than the text the thread was sent is asked again —
answered after that job, so it reads that tree — one ask per buffer in
flight. A buffer that has an outline waits until its text has been
still for 200 ms (an alarm wakes the window then): an outline reads
the whole tree, 50 ms for a 22 000-line spec and 12 ms for 11 000
lines of Rust, measured on the release build, and on the ts thread it
would stand before the next keystroke's highlighting. Meanwhile the
crumbs read the outline they have; while typing inside one test, it is
the same test. Moving the caret asks nothing: the path is looked up in
the outline held.

### 5. `editor.breadcrumbs`, and `:breadcrumbs` for one pane

`editor.breadcrumbs` (`true` by default: the title bar has the room)
for every editor pane; `:breadcrumbs` (`<leader>ob`, beside the look's
other toggles) flips the focused pane for the session, as `:wrap`
does. A buffer with no outline shows none. *Amended 2026-10-09:* the
flip is the pane's own `editor.breadcrumbs`
([pane-settings.md](pane-settings.md)).

### 6. A crumb is a place

A click on a crumb puts the caret on its symbol's name, as the symbols
picker's pick does, and is a location in the memory as that is. A
narrow pane keeps the innermost crumbs that fit, a `…` for the ones
left out, the innermost cut with an ellipsis when it alone is too
long.

## Built

2026-09-29, as decided. `languages/src/{javascript,typescript,go}.rs`:
the test blocks in the outline queries (`systems/src/ts.rs`'s
`a_test_files_blocks_are_its_outline`). `kawoosh/src/breadcrumbs.rs`:
the outlines kept and asked (`ask_crumbs`, after `sync_syntax`'s jobs;
the answers routed by token, above the marks'), `path_at`, the crumbs
drawn in `render_pane`'s title bar, the click, `:breadcrumbs`. Tests:
`kawoosh/tests/breadcrumbs.rs` — the path following the caret through
a spec, a local left out, the outline asked again after an edit once
still, a click on a crumb, `:breadcrumbs` and the setting, a narrow
pane's `…`.
