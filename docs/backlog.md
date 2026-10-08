# Backlog: what is open

The one list of work that is wanted and not built. Everything set aside
on purpose — waiting for use to ask, declined, out of scope, or
blocked on kui, a platform or an upstream — is in
[parked.md](parked.md) instead, with the reason. What was built, and in
what order, is the history in [design/roadmap.md](design/roadmap.md).

Gathered 2026-10-07 from the roadmap's tracks and steps, every design
note's "Not built", "Left" and "Open", and the personal todo, each
item checked against the code. A design note's "Not built" was true
when it was written; this file is what is true now. When an item is
built, it leaves this file and the commit says so; when one is
parked, it moves to parked.md with its reason.

Each item names where its design is. "Small" marks what is under an
afternoon.

## Editing and keys

- **`'a` and `` `a `` as an operator's motion** (`d'a`, `y`a`). The
  marks are normal-mode bindings only. Small. ([marks.md](design/marks.md),
  "Built", round two)
- **`%` on more blocks**: `repeat … until`, grammars whose keywords
  are named nodes, an injected language's blocks; and `%` finding the
  next bracket on the line when the caret is on none, as vim does.
  ([nodes.md](design/nodes.md) Decision 10)
- **`]F` `[F`** to a function's end, and walks over the other text
  objects; only `]f` `[f` exist. ([nodes.md](design/nodes.md) Decision 9)
- **Text objects for Lua**: `kawoosh.node.objects("function")`.
  ([nodes.md](design/nodes.md) Decision 9)
- **`]]` `[[` outside a man page**: nothing binds them elsewhere yet
  (a markdown heading, a function). Small. ([keys.md](design/keys.md),
  "]x / [x")
- **A node action: conditional ↔ ternary**, per language — the second
  round, if missed. ([node-actions.md](design/node-actions.md),
  Decision 7)
- **Lua's unclosed keyword blocks indent**: `function f()` inside an
  ERROR still gets the fallback; a `(ERROR "function" …)` pattern in
  lua's `indents.scm` is the fix. Small. ([indent.md](design/indent.md)
  Decision 7)

## Panes and pickers

- **`:picker files|grep here` from a terminal** uses the working
  directory, not the terminal's: `here_dir()` knows it in Rust, but
  `kawoosh.term` gives Lua only `send`. Small. (roadmap.md, "Panes
  and pickers")
- **`<C-w>x` across tab and dock**: `pane swap` trades panes on one
  side only. Small. (roadmap.md, "Panes and pickers")
- **⌘-hover in an editor pane**: the terminal's underline and hand
  cursor over a link, in an editor pane too. (roadmap.md step 49)
- **The dock's panes kept by a session**: only `dock_open` and
  `dock_ratio` are saved. ([workspaces.md](design/workspaces.md),
  roadmap.md "Workspaces")
- **A dock task ending or failing in another workspace says so** in
  the title bar. In the recommended shape of step 32, never carried
  into the note. (roadmap.md, "Workspaces")
- **`layout.column_width` as a kui size** (a clamp, a percentage); it is
  still the `Width` enum. ([launcher.md](design/launcher.md) Decision 8)
- **The picker's frame when a 100k-file walk lands** (108 ms): a row
  list kept in the engine and lent to Lua.
  ([lua-boundary.md](design/lua-boundary.md), "Open")

## Buffers with a shape

- **Hex**: the pane restored by a session (`session = false` in
  `hex.lua`); search off the frame's thread (`fs.find` runs on it);
  encodings other than ASCII in the text half. (roadmap.md step 88)
- **Pictures**: the pane restored by a session; a checkered ground
  under a transparent image; EXIF orientation; an animated picture in
  a markdown buffer (its first frame is drawn); tiles for an SVG past
  the 4096 px cap; a change stamp finer than a second. (roadmap.md
  step 89)
- **Manual pages**: a page rendered again when its pane is resized
  (or opened beside at the new width in one step); a page's section
  heads in `grs`. ([man.md](design/man.md), "Not built")

## Search, lists and marks

- **Results that follow their query** after edits, searched again
  live; today `⏎` searches again. ([search.md](design/search.md)
  Decision 10)
- **A mark in a multibuffer**, on the source's line: `m` there still
  says the buffer is no file. ([marks.md](design/marks.md), "Beside
  the search")

## Code: grammars, servers, formatters

- **`textobjects.scm` shipped by kawoosh-grammars**: kawoosh reads
  them, the builder ships none (builder, compile check, samples).
  ([grammars.md](design/grammars.md), "Text objects, 2026-10-03")
- **`; extends` in a user's own `queries/NAME/`**: a user file still
  replaces the builtin one. ([grammars.md](design/grammars.md), "Built")
- **Libraries added through `kawoosh.language` in `:grammars`**: they
  are missing from the pane. Small. ([grammars.md](design/grammars.md),
  "Built")
- **A server's `rangeFormatting`**, for a selection.
  ([formatters.md](design/formatters.md), "Left")
- **A formatter's config made after a buffer opened**: the indent is
  probed again only when the path or the settings change.
  ([formatters.md](design/formatters.md), "Left")
- **Formatters run on a real project**: prettier, biome, stylua, ruff,
  taplo and shfmt are checked against their docs only. Verification.
  ([formatters.md](design/formatters.md), "Left")
- **`.editorconfig` for a file on an ssh host** is not resolved.
  ([editorconfig.md](design/editorconfig.md) Decision 3)

## Version control

- **Blame asked again after a second of stillness**, as Decision 7
  says; today on save only. `kawoosh.on_tree` is the hook it lacked.
  Small. ([vcs.md](design/vcs.md) Decision 7, "Built")
- **Conflict washes in a multibuffer**: a review's excerpts show a
  conflict's lines unwashed. ([vcs.md](design/vcs.md), "Not built")

## Terminal

- **Hover motion reported** (mode 1003, motion with no button held).
  ([kui.md](design/kui.md), implementation notes)

## Memory

- **The edit journal pruned**: `Journal::prune` runs only in a test,
  so a buffer's journal grows for its life.
  ([kui.md](design/kui.md), implementation notes;
  [core.md](design/core.md))
- **Co-occurrence**: the files that were open beside this one, from
  the session layouts. Wants a note of its own first.
  ([memory.md](design/memory.md), build order item 5)

## Lua and plugins

- **A listing's ids as payloads only**: `ids` still mirror the tracked
  lines in Lua. ([lua-boundary.md](design/lua-boundary.md), "Open")
- **Types for LuaLS**: parameters are `any` unless the name says,
  every one after the first optional, an undocumented Rust function
  `(...)`. (roadmap.md, "Lua and plugins"; `lua/src/meta.rs`)
- **Native extensions, round five**: a `kawoosh-ext` crate for Rust
  authors, when the first asks. Rounds one (the loader, `kw_call`,
  `kw_fn`, `:extensions`), two (native panes, `kw_wake`), three (the
  parity test, the typed buffer doors) and four (Windows: the exports,
  `kawoosh.lib` shipped, a run there) are built.
  ([native.md](design/native.md))

## Look

- **`icon =` on a `kawoosh.status` segment.** Small.
  ([icons.md](design/icons.md), "Not yet")
- **A spinner for "running"**, and the strings still standing in for
  icons (`‹ 2`, `⏎`, the theme check's report, the message line) drawn
  as icons. ([icons.md](design/icons.md), "Not yet")

## Remote

- **A reconnect that reopens the master's pane by itself.**
  ([domains.md](design/domains.md), "Built")

## The app

- **The window's size kept by a session** (its position waits on kui,
  in parked.md). ([kui-requirements.md](design/kui-requirements.md)
  §10)
- **A notices file in the built apps** for the Rust dependencies and
  the Windows folder's conpty files. Small. (commit a1c8f7e, the
  licence)
- **A beep on a failed motion**; the terminal's bell is built.
  Small. ([kui-requirements.md](design/kui-requirements.md) §9)
- **UTF-8 boundaries in the buffer**: whether an edit off a char
  boundary is refused or rounded, decided deliberately.
  ([core.md](design/core.md), "Deliberately not here")
