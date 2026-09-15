# Kawoosh on kui: frontend attempt #4, and the last one

Status: accepted 2026-09-15; milestones 1–8 implemented the same day, one
commit each (`git log --oneline` from "Rebuild on kui, milestone 1").
Supersedes the platform, UI, input-plumbing and Lua-DSL decisions of
[mvp.md](mvp.md) (Decisions 1, 2, 4b's SDL half, 8's DSL half, the crate
table, the build order). The thesis, the taste constraints, and Decisions
3, 3b, 4, 5, 5b, 5c, 6, 7, 7b stand and are not restated here.

Implementation notes — where reality is thinner than the design, so the
gaps are records rather than surprises:

- **Landed at full design depth**: the kui runner with the bundled face;
  `doc` with the salvaged journal and eagerly shifted layers; the modal
  engine (selection sets, vim operators over motions and text objects,
  visual, multicursor, undo per command, the keymap trie, search, ex);
  the editor pane as rows (selection, block/bar carets, search hits,
  syntax runs, diagnostic underlines + EOL messages, completion ghost);
  panes/tabs/dock with drag dividers and geometric focus moves;
  terminals as `cells` with the pane prefix, scrollback-to-buffer,
  ctrl-click locations, the `$EDITOR --wait` socket and shim; `ts`
  (Rust) and `lsp` (pool by outermost workspace marker — verified: two
  panes, one rust-analyzer); Lua with commands, keymaps, views as slot
  panes, scratch buffers with `on_write`, tools, `kawoosh.store`; oil
  with line identity through the journal; compile mode with `]q`;
  sessions and oldfiles.
- **Thinner than designed, still open**: mouse reporting covers the
  primary button, drags and the wheel (kui routes the other buttons
  nowhere, and hover motion without a button is not sent); document sync to LSP is whole-text per change, not
  incremental from the journal; the journal is never pruned (bounded by
  the buffer's life, not by memory); only Rust has a tree-sitter grammar
  and only rust-analyzer a builtin server definition (`kawoosh.lsp.server`
  adds others; grammars need a build); workspace `.kawoosh/init.lua`
  (7b) is not loaded — one global `init.lua`; no `:map` listing; no
  macros or `.`; the tab strip has no close button; the undo history is
  linear.

## What changed

The previous design spent two of its nine milestones building a shell
(SDL3 + glyph atlas + cosmic-text painter) and a layout crate (a ~2k-line
clay-alike), then a third of the Lua milestone building a table-to-element
DSL on top of that crate. All three exist now as [kui](https://drydock9.qxuken.dev/qxuken/kui)
— a clay-style flat-tree immediate-mode core with the text stack inside it,
a wgpu backend, a winit runner, a Lua binding that converts tables to nodes,
and a C ABI — built to the same principles this project was going to build
them to (UI as data, events as data, headless by construction), and tested
to a depth the MVP would never have reached (a conformance corpus across
three bindings, benches with regression gates, an accessibility audit).

So: **everything above `text-buffer` is scrapped and rebuilt on kui.** Not
ported — rebuilt, because the old layers were shaped by having to own the
pixels, and kui changes what the editor pane even *is*: a row of text
runs instead of a custom leaf laying glyphs. The one thing carried over
unchanged is the piece-tree text buffer, which never knew what a pixel was.

The scrapped crates are salvage, not architecture. Where the rebuild lands
on the same shape (a versioned edit journal; a selection-set editing
engine; an LSP JSON-RPC thread), lifting a file is allowed and noted per
milestone; nothing is kept for having existed.

## System shape

```
┌────────────────────────── kawoosh (kui::App) ──────────────────────────┐
│  view(): pane tree → kui nodes            on_event(): UiEvent → commands │
│  editor pane = rows of runs  · term pane = ui.cells  · lua pane = slot   │
└──────▲──────────────────────────────▲──────────────────────────▲────────┘
       │ reads                        │ reads                    │ fills
┌──────┴──────┐                ┌──────┴──────┐            ┌──────┴────────┐
│ doc + editor│                │    term     │            │ lua (kui-lua) │
│ buffers,    │                │ alacritty → │            │ kawoosh.* API │
│ layers,     │                │ CellGrid    │            │ one Lua state │
│ selections  │                └──────▲──────┘            └──────▲────────┘
└──────▲──────┘                       │                          │
       │ Update{version,span,runs}    │ bytes                    │ data
┌──────┴───────────────────────────────┴──────────────────────────┴────────┐
│  systems: io (ptys, fs, socket) · ts · lsp · store — threads + channels  │
│  each holds a kui::Waker clone; a message + wake() is how the loop draws │
└──────────────────────────────────────────────────────────────────────────┘
                              kui runner (winit + wgpu)
             parks between events · Waker wakes · KUI_DEVTOOLS=1 for free
```

One process, one writer, unchanged. What moved: the shell is now a
`kui::App` impl; layout, text, painting, input decoding, focus, theme,
devtools and the headless harness are kui's.

## Crate layout

| Crate | Role | Depends on | Fate of the old one |
|---|---|---|---|
| `text-buffer` | piece-tree text, `Arc` snapshots | — | **kept as is** |
| `doc` | buffers, layers (style runs, selections, constraints), versioned journal, provider `Update`s | `text-buffer` | rewrite; `core/src/version.rs` is salvage |
| `editor` | modal engine: selection sets, commands, keymap tree, undo, macros — no UI types | `doc` | rewrite; `kawoosh/src/editor.rs` is salvage |
| `term` | `alacritty_terminal` + `portable-pty` behind a facade that yields a `kui_core::CellGrid` | `kui-core` | rewrite against `CellGrid` |
| `systems` | `io`, `ts`, `lsp`, `store` threads; message enums; a `Wake` callback | `doc`, `term` | rewrite; `systems/src/lsp.rs` JSON-RPC plumbing is salvage |
| `lua` | `kawoosh.*` API seeded into a `kui_lua::LuaExtension`; the bootstrap script | `kui-lua`, `doc`, `editor` | rewrite |
| `kawoosh` | the `kui::App`: pane tree, tabs, docks, the three pane kinds, status/command strips, wiring, CLI shim | everything, `kui` | rewrite |
| ~~`ui`~~ | — | — | **deleted** (kui-core) |
| ~~`core`~~ | — | — | **deleted** (becomes `doc`) |

Dependency rule kept from mvp.md, restated for kui: `doc`, `editor` and
`systems` never see a kui type. `term` sees `kui-core` only for the `Cell`
/ `CellGrid` data contract — kui-core is the bindable data layer, not the
runner, so this is the same kind of dependency as `serde`. Only `kawoosh`
and `lua` depend on the runner and the binding.

Workspace dependencies that go: `sdl3`, `cosmic-text` (kui owns shaping).
That come: `kui`, `kui-core`, `kui-lua` at one pinned pre-release
(`=0.1.0-alpha.12`, forgejo registry), `mlua` with **`lua55` + `vendored`**.

## Decisions

### 1. Platform: the kui runner. SDL3 is gone.

`kui::app("kawoosh").run(app)` — winit + wgpu, one instanced draw call per
frame, subpixel text where the GPU can blend it, fonts from
`Core::load_fonts_dir("assets/fonts")`. The loop parks between events;
systems threads hold a `kui::Waker` clone and call `wake()` after posting
a message, which is exactly the "SDL user event wakes the loop" design
with the plumbing already written. Event-driven redraw stays the rule:
kawoosh never asks for a frame it has no dirty state for.

mvp.md's Decision 1 argued for cosmic-text over SDL3_ttf so that
measurement would not drag SDL into layout. kui made that cut for us and
went further: kawoosh does **no text measurement at all**. The editor pane
never asks how wide a glyph is (see Decision 3).

### 2. Layout and paint: kui-core. The `ui` crate is gone.

The clay-alike was to stay under 2k lines "or the sizing model is wrong".
kui-core's solver is that model, plus scroll regions, floats, clipping,
transitions, hover-as-data, focus-as-data, an access tree, diagnostics as
warnings and a devtools panel. Kawoosh uses a deliberately small slice of
it:

- `row` / `column` with `Grow` / `Fixed` / `Fit`, `pad`, `gap`, `clip` —
  the pane tree, strips and title bars (the splitmux example is the
  reference implementation of exactly this pane tree, ⌘-drag included).
- `text` with a mono `TextStyle` — every run of editor text.
- `cells` — every terminal pane.
- `on_key` sinks, `on_drag`, `on_click`, `on_scroll` — all input.
- `slot` — every Lua-drawn pane.
- `transition` on split ratios only; no other motion in the MVP.

Not used, on purpose: `edit` (kui's own editor widget — kawoosh owns its
text model), floats for anything but the drag ghost, `window` kinds other
than the main window, audio, fragments.

**On "no popups".** mvp.md banned floating UI partly on taste and partly
because it *paid*: no z-order, no occlusion, no focus stealing in a
homegrown layout crate. kui solves all three, so the engineering half of
the justification is gone. The taste half stays: transient UI goes in
panes and strips, completion is in-place, hover opens a pane. It is now a
rule that costs nothing to revisit, and it is not revisited here.

### 3. The editor pane is rows of runs, not a custom leaf

This is the decision that changes the most code. mvp.md had the editor as
a custom element that pulled `Chunks` for the visible range and laid
glyphs itself, with `geometry(range) → rects` for decorations living in
that leaf. kui's `modal_editor` and `syntax_view` examples show the other
way, and its `highlight` bench measures it: **each visible line is a
`row` of `text` runs** — one node per style run, a `bg`-colored container
around selected runs, the caret an inline 2px node (bar) or an inverted
one-char container (block) — and *the row is the layout*.

What this buys, each of which was a milestone-sized piece of work before:

- **No measurement, no glyph geometry.** Selection rects, caret position,
  and decoration extents are all "wrap this run in a container"; the
  solver places them.
- **Virtual text is just another run.** A completion ghost, an inlay hint,
  an end-of-line diagnostic is a `text` node emitted between two real
  runs with a ghost style. mvp.md's governing rule — *virtual text may
  shift real text, never occlude it* — stops being a rule the leaf
  enforces and becomes the only thing the shape can do.
- **Multicursor is free.** A selection set is N carets and N selected
  ranges on the visible lines; emitting them is the same loop.
- **The mouse comes back as data.** A sink declaring `on_drag` over
  `Role::Line` rows gets `{line, byte, clicks}` per press and move
  (kui's C34): click-to-caret, drag-select and double-click-word are
  arithmetic in `on_event`.
- **A screen reader reads the buffer** through the same rows (`Role::Line`
  + `caret` / `selection_anchor` byte offsets), for nothing.

Cost, measured by kui's `highlight` bench (two 55-line panes, ~8 runs a
line): well inside a frame. The text cache keys on (content, style,
scale) with color excluded, so token runs dedupe across lines. The pane
emits only `view.top .. view.top + rows`; a 100k-line buffer costs the
screenful it shows.

Trailing spaces in a run are unreliable across fonts — the examples map
` ` to NBSP before emitting, and so does kawoosh (one char for one, so
byte arithmetic on the drawn text still maps back).

**The road to soft wrap** (still not MVP): a line as one `rich_text`
node — spans carrying color and a per-span `bg` for selection — with the
caret placed from `Ui::caret_rect(key, byte)`, wraps as a paragraph.
Same data in, one node instead of eight; the switch is local to the
line-emit function. This replaces mvp.md's "ghost-text shifting is the
same machinery inlay hints need later": now both are trivially runs, and
wrap is the only remaining layout question in the editor.

### 4. Terminal panes are `cells`

`ui.cells(&CellGrid, spec)`: a `Cell { ch, fg, bg, flags }` per cell,
row-major, a cursor with a shape, and an `origin_line` that places the
screenful in the session's history. kui's `cells` bench: 200×50 with
every character new each frame is ~60 µs. The same grid as text nodes is
~2.2 ms, and a text-node pane whose lines all change is 25–64 ms — the
`stream` bench — so **a terminal is never drawn as text rows**; that is
the perf cliff this decision exists to name.

`term`'s facade is `alacritty_terminal` in, `CellGrid` out: `feed(bytes)`,
`resize(cols, rows)`, `grid(top) -> CellGrid`, `scrollback_text(range)`.
Selection is kui's, in cells (ADR 0017): drag, word, row, Alt for a block,
copy trims trailing blanks. Scroll arrives as `{kind="scroll", lines}` and
the app adds it to its own `top` (ADR 0029). Materialize-scrollback-into-a-
buffer (mvp.md Decision 3) reads `scrollback_text` into a `doc` buffer,
unchanged in spirit.

Alternate screen, mouse reporting and truecolor are `alacritty_terminal`'s
as before. Key bytes to the pty come from the same key events every other
pane gets (Decision 5), encoded by `term`.

### 5. Input: one key sink, the hybrid rule against kui's payload

kui delivers every press as `{kind="key", code, physical, mods, text,
repeat}`, where `code` is the layout-resolved character or a named key and
`physical` is the US-QWERTY position (W3C `code` names: `"KeyJ"`). That
is the pair mvp.md's Decision 4b needed SDL keycodes and scancodes for.
The hybrid rule is unchanged and now lives in one function in `editor`:

- match by `code` when the active layout yields a mappable (ASCII)
  symbol — Dvorak's `j` is where Dvorak puts it;
- fall back to `physical` when it does not — Cyrillic normal mode keeps
  working;
- text insertion **never** comes from `code`: insert mode, the command
  line and pty input consume `text` (kui sends IME commits and the
  clipboard's paste as `{kind="text"}` too), so composed and CJK input
  flow through untouched.

**One sink, kawoosh's keymap does the routing.** The root declares
`on_key`; kui's rule that a root sink hears every key when nothing is
focused means kawoosh never takes focus to have somewhere for chords to
land, and kui's Tab ring never engages (a modal editor owns Tab). The
keymap dispatches by (focused pane kind, mode): editor panes to the modal
engine, terminal panes to the pty (with a prefix chord for pane commands,
as tmux), Lua panes to whatever `kawoosh.map` bound for that view.
Modifier state arrives as `{kind="modifiers"}` and is kept in the model,
which is how ⌘-drag pane moves and plain clicks coexist (splitmux).

`kui::Core::press` drives the same path headless — kawoosh's key-sequence
tests are kui `Core` tests, not a harness kawoosh maintains.

### 6. Lua: kui-lua's binding, one state, `kawoosh.*` seeded into it

**Lua 5.5, not LuaJIT.** `kui-lua` pins `mlua` to `lua55`, and mlua allows
one Lua per binary. mvp.md chose LuaJIT for per-frame table building and
for 5.1-dialect plugin culture; the first is answered by kui-lua's
converter being the hot path (measured, and views run only on dirty
frames), the second was never going to hold — neovim plugins would not
have run against a different API in any dialect. Accepted.

**One `LuaExtension`, one Lua state, all of kawoosh's Lua in it.** kui-lua's
unit is a script that defines `view(env, slot)` and `on_event(ev)`. Kawoosh
loads exactly one, a bundled bootstrap, and seeds its state
(`LuaExtension::lua()`) with the `kawoosh` table: `command`, `map`, `view`,
`buf`, `store`, `tool`, `compile`, `opt`. `init.lua` and every plugin run
in that state; there is no per-plugin sandbox, which is the neovim model
and the honest one for an editor whose config *is* code.

The bootstrap's `view(env, slot)` is a dispatcher: the host declares
`ui.slot_with("lua/<view>@<pane>", params)` inside every pane whose content
is a Lua view, and the bootstrap calls the function `kawoosh.view(name,
fn)` registered, returning its table. The slot key namespaces every node
under it, so the same Lua view in two panes has two sets of scroll offsets
and edit state. Params are the pane's facts this frame (id, focused,
size, the buffer it is attached to if any) — declared every frame, never
retained, per ADR 0014. The bootstrap's `on_event` routes back to that
view's handler.

Two things kui does not do today, both small and both kawoosh's to add to
kui rather than work around here (the full door-by-door list is
[kui-requirements.md](kui-requirements.md)):

- **K1 — dynamic slots.** `Extension::slots` is read once at load and a
  slot name is filled only if listed. A view registered from `init.lua`
  is not known at load. Proposed: a `"*"` entry in `slots` matches any
  name in the extension's namespace. One branch in `Extensions::fill`.
- **K2 — an event knows its fill.** `UiEvent` carries origin, window, key
  and payload but not the slot it was filled under, so the bootstrap must
  stamp `_view`/`_pane` onto every payload table it returns (a table walk
  on dirty frames; cheap, but a convention). Proposed: `UiEvent::slot`
  (the fill's key, or its name) so routing by pane is a field read.

Until K1 lands the fallback is to bypass slot matching from the host:
`Ui::fill(origin, &Slot{..}, |ui| ext.view(&slot, ui))` is public, and
events under an origin the runner does not know are handed to the host,
which forwards them to `ext.on_event`. It works and it is a workaround;
K1 is a better kui.

The rest of mvp.md's Decision 8 holds: Lua is the only configuration
language; config is code, state is data; the four verbs; the file manager,
compile mode and the buffer switcher are bundled plugins on the public API
and are its acceptance test. The UI DSL is no longer kawoosh's to design —
it is kui-lua's prelude (`row`, `column`, `text`, `edit`, `fill`, theme
roles on `env.theme`, tokens with `$name` references), which is documented,
schema-generated and conformance-tested against the other bindings.

The native boundary mvp.md deferred is now half-built by someone else:
kui-ffi's `CExtension` is a shared library filling a slot, and a Lua
script can load one (`env.add_extension`). Kawoosh does nothing with this
in the MVP and loses nothing by it being there.

### 7. Theme and syntax colors

Chrome colors are kui theme roles (`bg`, `surface`, `sunken`, `fg`,
`muted`, `faint`, `selection`, `focus_ring`, the three statuses) read off
`ui.theme()`, so the window follows the OS light/dark and accent with no
kawoosh code — `modal_editor` maps its whole palette from roles and so
does kawoosh's chrome. Syntax hues are the app's own and go in **tokens**
(ADR 0027): `tokens = { colors = { keyword = { light, dark }, ... } }`
from `init.lua`, referenced as `$keyword` from Lua and resolved by name in
Rust when `ts` runs assign captures to colors. A user theme is a token
table plus, optionally, `set_theme` for a palette that follows nothing.

### 8. Testing: kui's headless core is the harness

mvp.md planned a headless harness in milestone 4. kui *is* one: a `Core`
with a viewport, `press`, `cursor`, `mouse`, `set_time`, and back out the
quads, the events, the warnings and the access tree. So:

- `text-buffer`, `doc`, `editor`: unit and property tests as before, no
  kui in sight.
- **The app**: `Core::press("j")` through the real `App::view` /
  `on_event`, then assert on buffer text, selections, mode — *and* on the
  drawn rows (`nodes()`), so "the caret is on line 3" is checked where it
  is drawn, not only in the model. `take_warnings()` is asserted empty in
  every test: a `Grow` with nothing to split, a duplicate key, an
  unnamed control is a failing test, not a devtools discovery.
- `term`: byte streams in, `CellGrid` out, as before; plus `cells`
  selection driven by kui's cursor.
- systems: message in, message out, as before; the `Wake` callback is a
  counter in tests.
- Lua: API tests are scripts run through the bootstrap in a headless
  `Core`; the bundled plugins are the integration suite.

The thing not covered headless is smaller than before: it was the
painter, and the painter is now kui's, with its own coverage.

### Deliberately not in the MVP

Unchanged from mvp.md: detachable daemon, soft wrap, proportional fonts,
images, ligatures, plugin manager, treesitter injections, DAP, multiple
windows. kui makes several of these cheaper (multi-window is a `ui.window`
declaration; soft wrap is Decision 3's `rich_text` path; images are an
`image` node) and none of them is pulled forward for it.

## Build order

Each milestone runnable and dogfoodable; the order front-loads the one
new risk (kui as a dependency: version pin, Lua version, the two kui
changes) and keeps the old ones (terminal embedding, off-thread
providers) where they were.

1. **Skeleton on kui.** Workspace pruned to `text-buffer` + `kawoosh`;
   kui pinned from forgejo; fonts loaded; one read-only buffer drawn as
   rows with `j`/`k` scrolling through a root `on_key` sink; one headless
   test asserting rows and no warnings. *Proves the dependency, the Lua
   feature set, and Decision 3's shape.* The dependency half is already
   proven: a scratch crate on 2026-09-15 pulled `kui`, `kui-core` and
   `kui-lua` `0.1.0-alpha.12` from the forgejo registry with `mlua`
   `lua55`+`vendored`, seeded a `kawoosh` global into a `LuaExtension`'s
   state, filled a host-declared slot from that script through
   `Core::frame_with(.., &mut Extensions)`, and asserted the fill's text
   in `nodes()` with `take_warnings()` empty — 23 s cold build, 0.2 s
   test.
2. **`doc` + `editor`.** Buffers, layers, journal; selection sets over
   `text-buffer`; normal/insert/visual/command; motions, edits,
   checkpoints, undo; command registry; keymap tree with the hybrid
   `code`/`physical` rule, tested with synthesized non-latin presses.
   Multicursor from the first commit (`&[Selection]`, mvp.md Decision 4).
3. **Panes, tabs, docks, view list.** The splitmux tree with editor
   panes in it; title bar per pane; statusline; command line as a buffer
   pane; hidden views; ⌘-drag moves. Sessions' data shape fixed here.
4. **Terminal panes.** `term` facade → `cells`; `io` thread + Waker;
   scrollback-to-buffer; alt screen / mouse / truecolor verified against
   lazygit; the `kawoosh` CLI shim, socket and `$EDITOR --wait` handoff;
   the locations pattern table and `gf` from a terminal.
5. **`ts` system.** tree-sitter off-thread through `doc`'s `Update`
   path; captures → token colors → runs on the rows.
6. **`lsp` system.** Shared pool, journal-driven sync, diagnostics as
   runs plus EOL virtual text, goto-definition, in-place completion. *The
   thesis demonstrable: two tabs, five panes, one rust-analyzer.*
7. **Lua.** kui changes K1/K2 landed and released; the bootstrap and
   `kawoosh.*`; `init.lua`; oil-shaped file manager and compile mode as
   bundled plugins.
8. **Sessions.** SQLite via `rusqlite`; restore.

## Risks

- **kui is alpha and kawoosh is its first real consumer.** Expect API
  churn and expect to file kui backlog items from here (K1 and K2 are the
  first two). Mitigation: pin `=0.1.0-alpha.N`, bump deliberately, and
  keep the kui-touching surface in `kawoosh` and `lua` only, so a bump is
  two crates' worth of diff.
- **Lua 5.5** closes the door on LuaJIT-only libraries (ffi, in
  particular). Nothing in the MVP wants it.
- **Per-line row building** is O(visible runs) per dirty frame and runs
  on the main thread. Fine at two panes; a six-pane tab with dense
  highlighting is the case to bench early (milestone 5) against kui's
  `highlight` numbers, before deciding whether a row cache keyed by
  (line version, layers version) is needed.
- **Text-as-rows for the wrong pane.** Anything that streams — compile
  output, a log — is a buffer, so it is rows, and rows whose text changes
  every frame is the `stream` cliff. A compile buffer changes a few lines
  per wake, not fifty; if a stream ever does, it is a `cells` pane with a
  buffer behind it, not a fix in the row emitter.
- **The Waker is per window.** Multiple windows stay a non-goal; if they
  arrive, systems need a waker per window or a wake-all.
