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

### 4. Carets placed by the engine: `kawoosh.buf.edits(…, { carets })`

`pairs.lua` and `node_actions.lua` each computed where every caret goes
after a multi-caret edit — a running shift per edit, a rule for an
offset inside a replaced range — and set the selections after the
edits. Now `kawoosh.buf.edits(edits, buffer, { carets = … })` takes the
carets with the edits: `{ edit = i, at = k }` `k` bytes into edit `i`'s
text, `{ at = o }` an offset of the text before, moved by the edits —
before an insertion at its own byte, kept its distance into a replaced
range (its last character at most), one rule for both plugins.
`Editor::apply_edits_at` answers where each edit's text starts; an
edit that changes nothing is none, and edits with carets but no text
change move the carets only.

### 5. A plugin's processes run 32 at a time

`vcs.lua`'s working-tree review asks for a `git show` per changed file,
and a review of two revisions two per file, all at once: two thousand
changed files were two thousand children and their pipes, past the
descriptors macOS allows a process (256). The bound is the engine's,
for every plugin: `kawoosh.spawn` runs at most 32 at once and the rest
wait their turn in order, each a job until it starts (`wait_for_jobs`
counts it); a waiting one killed ends with no code, as a running one
does. A plugin keeps writing the fan-out it means.

### 6. The profile decides: `kawoosh/tests/lua_costs.rs`

The suspected hot spots measured before any moved (an ignored harness,
`cargo test -p kawoosh --release --test lua_costs -- --ignored
--nocapture --test-threads=1`; Apple Silicon, release, 2026-10-07):

| case | measured | verdict |
|---|---|---|
| vcs statusline, 10k lines, 5000 hunks | 306 ms every idle frame | moved (Round 5) |
| picker, 100k files | one 171 ms frame as the load lands; keys 2–6 ms | open |
| du, 16k entries | worst 35 ms frame during the walk; re-sort 10.5 ms | left |
| sqlite, 200k rows | the grid never pages past its first 1000 | a bug (Round 6) |
| `:man bash` | an 80–100 ms frame as the page lands | open |
| hex find, 50 MB, absent | 11.4 ms | left |

The vcs cost was Rust's, not Lua's: `Base::lines` read the whole base
for its line starts once a hunk, and the statusline asked for every
hunk's old lines to count them.

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

**Round 3, 2026-10-07: carets placed by the engine** (Decision 4).
Verified by `kawoosh/lua/tests/buf_edits_carets.lua` (into edits,
offsets before an insertion and inside a replaced range, carets with no
edits) and the pairs and node-action scripts unchanged. The listing
test that asserted `big/` aside on the frame of the confirm waits for
it now: the aside is `fs::apply`'s, on its thread.

**Round 4, 2026-10-07: spawns bounded** (Decision 5). Verified by
`a_plugins_processes_wait_their_turn` (forty processes: 32 at most
running, the rest waiting, the last killed before it started, the 39
others run) and the suites that spawn (lua, vcs, git, fossil, dirs, qd,
secrets).

**Round 5, 2026-10-07: the vcs statusline**. `Base` keeps its line
starts, read once a text; `kawoosh.buf.hunk_counts` counts the hunks
by kind, which the statusline segment reads instead of the hunks. 306
ms an idle frame with 5000 hunks is 0.43 ms, `j` 0.5 ms.

## Open

- Entries carried by the engine: a listing's `ids` (journal id →
  entry) mirror the engine's tracked lines, and `entries_of` /
  `keep_register` work around the register outliving the listing it
  was yanked in. A payload per tracked line, carried by the register,
  would end both.
- `field_node`, `keys_node`, `legend_node` out of `boot.lua`.
- The picker's 171 ms frame as a 100k-file walk lands, and `man.lua`'s
  80–100 ms frame as a page lands (Decision 6).
- sqlite's blobs come back whole (a copy of a cell wants the whole), and
  its width pass runs over every fetched row once paging works.
