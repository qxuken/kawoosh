# Where Lua ends: the bundled plugins reviewed

Status: asked 2026-10-07, "didn't we overreach with lua plugins? should
we rewrite some in rust?", then "start with the cheap correctness batch
then move to rust building blocks". The calls below are taken here,
each the user's to overturn. Companion to
[plugin-panes.md](plugin-panes.md) (what a Lua plugin can do) and
mvp.md Decision 8 (the bundled plugins dogfood the API). *Rounds 1–13
built 2026-10-07*; "Open" at the end is what is left.

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
| picker, 100k files | one 171 ms frame as the load lands; keys 2–6 ms | halved (Round 8) |
| du, 16k entries | worst 35 ms frame during the walk; re-sort 10.5 ms | left |
| sqlite, 200k rows | the grid never pages past its first 1000 | a bug (Round 6) |
| `:man bash` | an 80–100 ms frame as the page lands | moved (Round 7) |
| hex find, 50 MB, absent | 11.4 ms | left |

The vcs cost was Rust's, not Lua's: `Base::lines` read the whole base
for its line starts once a hunk, and the statusline asked for every
hunk's old lines to count them.

### 7. A tracked line carries what it is: payloads

A listing tracked its lines through the engine (by id) and kept what
each was in Lua (`ids`: id → `{ dir, name, meta }`). The register said
which tracked line of which buffer each of its lines was — an id, good
only while that buffer held the same fill — so `dir.lua` kept the
register's entries itself before every fill (`keep_register`,
`dir.kept`, three call sites), and a listing closed outright took its
entries with it. A tracked line has a payload now, a string the plugin
gives (`open_scratch { payloads = { [line] = … } }`, `kawoosh.buf.
track(line, buffer, payload)`), and the register answers each line's
(`kawoosh.buf.register().payloads`), keeping them by the take once read
— every call into Lua comes after a publish — so they outlive the fill
and the buffer. `dir.lua` reads the register through the live listing
while it is the same fill (an entry the plan gave a line since is the
line's) and from the payloads otherwise; `keep_register` and `dir.kept`
are gone.

### 8. A Lua view's field is the engine's line

`boot.lua` drew a Lua view's field itself: 140 lines of spans, carets,
a lifted block, sideways scrolling and a `measure_text` a frame — a
second, smaller copy of `Kawoosh::field_line` (one caret, no tabs or
escapes drawn, no extra carets). kui already holds what it takes to
draw the engine's instead (no kui round): a Lua tree may declare a
`fill`, and a guest extension fills it (ADR 0014's amendment of
2026-09-08, "a Lua view wanting a native panel inside it is the day").
`ctx.field` is a `fill { name = "field/…", params = { field, size,
placeholder, focused, width } }`; `fields::FieldDraw`, an extension
under the `field` namespace with a wildcard slot, draws it with
`panes::draw_field` — `field_line`'s drawing, split from what it
gathers (`Kawoosh::field_scene`) — from the scenes the app gathers
before a Lua pane or a header is drawn (`publish_field_scenes`). The
Lua host (`fields::LuaHost`, what `attach_lua` returns) loads it the
first frame it draws (`Ui::add_extension`), so whoever registers `lua`
has the fields. A click is a reply to the view that declared the
field, `{ kind = "field", field = }`, handled before `boot.lua` reads
a slot off it — kui-lua names a reply's slot only for slots the script
fills itself. A fill is a position, so a field's width is an option
(`width = "grow"`), not a prop set on the node after.

### 9. Caps, legends and the way to a legend are the engine's too

`boot.lua` composed a view's key caps, legends and `⌥/ keys` toggle
itself over the Rust half's reading of a notation (`kawoosh._key_caps`)
and its measures (`kawoosh._cap`) — the third drawing of a cap beside
`icons::keys` and `legends::toggle`. They are fills now, as the field
(Decision 8): the extension is `engine` (`fields::EngineDraw`), a
slot's `kind` saying what — `field`, `keys`, `legend`, `toggle` — and
it draws them with `icons::keys`, `icons::legend_items` and
`legends::toggle` (a hover group given only in the title bar now). A
view's fills are named by the view, its pane and a count of what it drew
this frame, since a slot's name is the frame's. Whether a legend is
whole stays the view's to say: `ctx.legend` gives nil while it is
compact, and tells the title bar it drew one. A toggle's click is a
reply carrying its pane. `_key_caps` and `_cap` are gone with their one
user.

### 10. A pane's tree is replayed while nothing its view read has moved

Every frame the window drew ran every visible Lua view and had kui-lua
turn the table it returned into nodes — about 2 µs a table, twice the
view's own time: the settings pane's 1,561 tables cost 1.25 ms in the
view and 3.0 ms in the walk on a frame where a key went to the editor
beside it, and 14–22 ms cold at the caret's 2 Hz with the pane focused
(the perf log of 2026-10-09; kui's layout of the same frame 0.3 ms, the
editor pane 0.25). kui F142 (ADR 0045, which amends its ADR 0016) lets
the host say a slot is unchanged and have the kept tree pushed again
without the view running, kui itself checking the params and every fact
of the frame the fill read. The host's half here is what the view read
*of kawoosh*: at boot every native under `kawoosh` and its tables is
wrapped with a category — `editor` (the published snapshot), `fields`,
`settings`, `commands`, `memory`, `palette`, `pane_settings` (a pane's
own values and its legend's fullness, under the store's count of
changes), `legend`, `clock`, `none`
for one pure of its arguments — or `opaque` for the rest (a write, the
file system, a process, the store) — the shell's own doors, seeded after
the script (`kawoosh.themes`, `.fonts`, `.grammars`, `.settings`,
`.du`, the icons', the legends'), are wrapped by a second walk
(`Runtime::track_reads`, before the bundled plugins take a local) and
read as states the shell hashes while a view reads one — and a view
running notes what it called (`kawoosh._reads[NAME@PANE]`); `ctx.env` is a proxy that notes
`now` and `caret_visible`, the two fields of kui's reading that kui
cannot compare. After the publish each frame the shell reads the
generations (`Runtime::gens`: a hash of the snapshot per category,
`FrameGens` adding the palette's and the legends') and whether any
plugin code ran outside a view since — `timed`'s flag on the Lua side,
and the runtime's `touched` for every call the Rust half makes that
delivers something to a plugin (a hook, a process's lines, a picker's
answer, a command), which `timed` never sees — a reading the shell asks
of a plugin every frame (a status segment, a tab's title) is not one,
or nothing would ever replay: if it did, every view is run once more; else a view that read only tracked categories,
none of which moved since its last run, has its pane declared with
`slot_replay`, the others with `slot_kept`. `render_lua_pane` logs the
answer and, for a view that will not replay, why and which natives it
called, so a pane that stays slow says so in `KAWOOSH_PERF_LOG`. A view
that calls an opaque native is correct and slow, never stale.

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

**Round 6, 2026-10-07: sqlite pages past its first**. A browsed
table asks for a page and the row past it, so the cap cuts a full page
and `truncated` says there is more; before, `LIMIT page` with `cap =
page` was never cut and `G` stopped at row 1000 of 200 000. A page
appended is measured on its own over the widths there, and a row read
again after an edit on its own: a table scrolled through is measured
once a row, not once a row a page.

**Round 7, 2026-10-07: a man page read by the engine**.
`kawoosh.overstrike(raw)` (`systems/src/overstrike.rs`) reads
overstrikes and SGR off into the plain text, its styled runs and its
all-bold lines; `man.render` keeps the heads, references, header and
footer, a line at a time, and looks for references only on a line with
a `(` before a digit. `man.render` of bash(1) from 80 ms to 7.5 ms.

**Round 8, 2026-10-07: the picker's 100k-file load**. `fs.walk`'s
answer has each path joined to the root as `fs.join` joins (a join a
row from Lua was a call into the engine each); `boosted` indexes only
the boosted rows it sorts (an index on every row grew every row's
table); `is_binary` reads the extension from the path's end. The frame
the load lands in from 244 ms to 108 ms, open to rows from 389 to 215.

**Round 9, 2026-10-07: a file's base is its newest ask's**. Staging
writes the index more than once, and `vcs.moved` reads every open
file's base again each time: two `git show`s of one file in flight
answered in either order, the older sometimes last. `fetch_base`
numbers its asks by path and puts only the newest's answer — a number,
not one ask in flight at a time, so a backend that never answers holds
nothing up.

**Round 10, 2026-10-07: sqlite blobs cut for showing**.
`kawoosh.sqlite.query`'s `opts.blob_cap` keeps a blob to that many
bytes; a longer one is `{ blob = its head, size =, cut = true }`
(`Value::Cut`), and binding one is an error in Rust and in the binding,
so a cut blob is never written back short. The pane reads blobs to 4 KiB
and shows the real size; `y`, `Y` and the change `u` puts back read the
row whole again by its key first. A page of rows with a megabyte blob
each was that many megabytes in Lua.

**Round 11, 2026-10-07: payloads** (Decision 7). Verified by
`a_line_yanked_before_its_listing_moved_on_is_still_its_entry` (the
listing buffer refilled three times, the line a copy each time) and
`a_line_yanked_in_a_listing_since_closed_is_still_its_entry` (the
listing closed, the paste in another a copy — `← new` before), both red
without the payloads, and the listing suites.

**Round 12, 2026-10-07: the field drawn by the engine** (Decision 8).
Asked "open the kui round for the field but ensure we don't have this
in kui already" — kui had it. Verified by the field tests as they were,
three of them reading the drawn tree now rather than the Lua node
tables (`n[1].caret` → the caret kui anchors the input method at, the
line one level down under the scroller), the overflow sweep, and a
test window driven over its socket (the picker's query, the search
bar's four fields and their placeholders).

**Round 13, 2026-10-07: caps and legends drawn by the engine**
(Decision 9). Verified by the legend and caps suites as they were
(`icons`, `legends`, the panes with legends), `nu scripts/verify.nu`
1138 of 1138, and a test window over its socket: the search bar's
`⌥/ hide keys` and its legend, caps with their icons, wrapped between
items.

**Round 14, 2026-10-09: the pane's tree replayed** (Decision 10; kui
F142). Asked after the perf log found the walk: "open the kui round for
slot replay". Verified by the perf log over the same panes and runs as
the finding (release build, the editor driven over its socket with the
pane beside it unfocused, forty frames each, medians): the settings
pane 4.6 → 0.30 ms a frame, themes 5.0 → 0.22, the theme lab 3.1 →
0.14, grammars 2.3 → 0.15, the whole frame's work 5.9 / 6.2 / 4.6 /
3.5 → 1.65 / 1.36 / 1.60 / 1.54 ms against 1.16 with no pane; 344
frames replayed, the few filled fresh each with a reason the log names
— the focus moving to the pane, the window's focus in the env reading,
the engine extension loaded on a pane's first frame, plugin code run by
a command. The pane's `lua host view` span is gone from the replayed
frames and `replayed` stands in its place; and the kui suites (nine
tests of the replay, the Lua and C and Node bindings' each).

## Open

- A listing's `ids` still mirror its tracked lines in Lua (the plan
  reassigns an entry to a line, Decision 7): the payloads could be the
  only record once the engine takes a payload changed on a tracked line.
- The picker's 108 ms frame as a 100k-file walk lands (Round 8): what is
  left is the rows as Lua tables and the matcher's copy of their text —
  a list kept in the engine and lent to Lua as rows are shown would end
  it, and change every source.
