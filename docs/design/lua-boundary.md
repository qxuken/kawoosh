# Where Lua ends: the bundled plugins reviewed

Status: asked 2026-10-07, "didn't we overreach with lua plugins? should
we rewrite some in rust?", then "start with the cheap correctness batch
then move to rust building blocks". The calls below are taken here,
each the user's to overturn. Companion to
[plugin-panes.md](plugin-panes.md) (what a Lua plugin can do) and
mvp.md Decision 8 (the bundled plugins dogfood the API).

## What there was

About 16k lines of bundled Lua (`kawoosh/lua/*.lua`, `lua/lua/boot.lua`)
over about 170k of Rust. The heavy lifting was already Rust's — search,
the fuzzy matcher, LSP, pictures, sqlite, the sizing walk, grammars —
and the Lua was mostly panes, keys, sources, backends and data. Of the
184 `kawoosh.*` names the bundled plugins call, about 100 are called by
one plugin only: the API grew a door per feature more than a set of
primitives, its Rust half one file of 7.5k lines (`lua/src/lib.rs`).

## Decisions

### 1. No plugin is rewritten in Rust whole

What a user would fork stays Lua: picker sources and `picker.rank`,
vcs backends, search stages, the launcher's modules, the settings,
theme and font panes, servers (data), pairs' rules, node actions, the
statusline's segments. A plugin's *mechanism* moves to Rust as a
primitive the plugin calls — not the plugin.

### 2. What is Rust's: three kinds

- **Mechanism with stakes**: what loses data when it is wrong — a
  listing's write (Decision 3), and which line is which entry.
- **Work that grows with the data on the UI thread**: per frame, per
  key or per item over 10k–100k rows — copied, sorted, parsed or
  measured in Lua each time. Moved after a profile shows it (the
  user's rule: profile before optimising), each as a primitive that
  answers a window or a count rather than everything.
- **Widgets**: drawing a text field, key caps, legends — chrome, the
  same in every pane, written once in `boot.lua` today.

### 3. `kawoosh.fs.apply`: a listing's write in one call

`dir.lua` applied a write op by op from Lua — deletes renamed aside,
copies on threads, renames in two steps with `fs.exists` checks between,
creates, the makes-way deletes put back, the removals — the renames and
checks on the frame, the order (which is what keeps a file from being
lost) spread over five closures. Now `fs::apply(changes, settled)` in
`systems/src/fs.rs` holds the order and runs it on a thread of its own;
`kawoosh.fs.apply(changes, { settled =, done = })` takes `{ kind, from,
to }` / `{ kind, path, dir }` changes and answers every change's
outcome by index, once when only the removals are left (the listings
are read again) and once at the end. The plan — which line is which
entry, what became of it — stays in `dir.lua`; so do the messages.

## Rounds

**Round 1, 2026-10-07: the cheap batch** (d290f9f). `ctx.title_h`
(three plugins had `TITLE_H = 22`, the height before the title followed
the font); `kawoosh.tool`'s `place` read as `Place::parse` reads it
(`below`/`beside`), the launcher's letters in the one registry; zoxide
and ansible-vault from argument lists; `kawoosh.settings.line`
instead of three Lua spellings of a settings line; `open_scratch`'s
doc no longer 206 lines of other functions'. Left: a cache of pairs'
rules (it would go stale under `kawoosh.pairs.languages`, changed
live, to save microseconds), and the grammars pane's install-or-build
(grammars.md Round 6 keeps the verbs apart).

**Round 2, 2026-10-07: `kawoosh.fs.apply`** (Decision 3). Verified by
`fs::tests` (a swap, a copy and a create; a delete making way for a
rename; a delete kept when nothing came, a rename onto a taken name
refused) and the listing tests in `kawoosh/tests/lua.rs`, unchanged.

## Open

- Entries carried by the engine: a listing's `ids` (journal id →
  entry) mirror the engine's tracked lines, and `entries_of` /
  `keep_register` work around the register outliving the listing it
  was yanked in. A payload per tracked line, carried by the register,
  would end both.
- One "edits, then carets" primitive: `pairs.lua` and
  `node_actions.lua` each carry carets across a multi-caret edit by
  hand.
- `field_node`, `keys_node`, `legend_node` out of `boot.lua`.
- To profile, then move or leave: the vcs statusline segment copying
  every hunk's old lines to count them; the picker's walk built as
  100k Lua tables and copied back to the matcher; `du.lua`'s `shown()`
  re-sorting per update; sqlite's width pass over every fetched row,
  `OFFSET` paging, uncapped blobs; `man.lua`'s overstrike parse; hex's
  find on the frame and the store written per byte.
