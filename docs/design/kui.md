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
  panes/tabs/dock with drag dividers, geometric focus moves, a click
  anywhere in a pane focusing it, and a pane dragged by its title bar
  onto another (its middle swaps, an edge puts it beside; `<C-w>x`);
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
  adds others; grammars need a build); a workspace's
  `.kawoosh/settings.lua` is loaded (Decision 10) but its `init.lua`
  (7b's code half) is not — one global `init.lua`, since code from a
  repository wants the trust prompt 7b describes and data does not; no
  `:map` listing; no macros or `.`; the tab strip has no close button.
  (The undo history was linear until 2026-09-18; it is a tree now —
  mvp.md's retained roots kept instead of dropped — with `g-`/`g+` and
  the `:undo_history` pane.)

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
(`=0.1.0-alpha.14`, forgejo registry), `mlua` with **`lua55` + `vendored`**.

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
rule that costs nothing to revisit, and it was revisited once, for
notifications (Decision 9): two floats that never take focus, never
cover the caret's row, and are read rather than worked in.

### 3. The editor pane is rows of runs, not a custom leaf

This is the decision that changes the most code. mvp.md had the editor as
a custom element that pulled `Chunks` for the visible range and laid
glyphs itself, with `geometry(range) → rects` for decorations living in
that leaf. kui's `modal_editor` and `syntax_view` examples show the other
way, and its `highlight` bench measures it: **each visible line is a
`row` holding one `rich_text` of spans** — a span per style run,
selection and search hits as span backgrounds, the block caret an
inverted one-cluster span, the bar caret a 2 px float measured to its
byte — and *the row is the layout*. (Built first as one `text` node per
run with `bg` containers around them; the spans came after alpha.14,
when a review counted the whitespace gaps as nodes and the `highlight`
bench put `rich` at half the cost of runs. Spans break only on grapheme
boundaries, so a flag or a letter with its mark shapes whole under a
caret or a selection edge.)

What this buys, each of which was a milestone-sized piece of work before:

- **No measurement, no glyph geometry.** Selection rects, caret position,
  and decoration extents are all "wrap this run in a container"; the
  solver places them.
- **Virtual text is just another node.** A completion ghost, an inlay
  hint, an end-of-line diagnostic is a `text` node under `role = none`
  emitted beside the line's spans (the ghost splits them, so a click's
  byte and the access tree never count it). mvp.md's governing rule —
  *virtual text may shift real text, never occlude it* — stops being a
  rule the leaf enforces and becomes the only thing the shape can do.
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

**The road to soft wrap** (still not MVP): the line is already one
`rich_text`, so wrapping it is a paragraph wrap — with the caret placed
from `Ui::caret_rect(key, byte)` instead of a measured prefix, which
answers from the last frame and so lags a keystroke; today's measure is
exact for a line that never wraps.
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
`buf`, `store`, `tool`, `compile`, `opt`, `fs`. `init.lua` and every plugin run
in that state; there is no per-plugin sandbox, which is the neovim model
and the honest one for an editor whose config *is* code.

**`kawoosh.fs` is the one path layer, and it is the shell's own.** A
plugin never matches on `/` or reads `$HOME`: `fs.expand` turns a path as
the user wrote it — `~/x`, `../y`, `C:\z` — into the absolute, normalized
one, against the working directory the shell keeps the process in step
with; `join`, `parent`, `basename` do what a pattern on `/` did, on every
platform; `list`, `create`, `rename`, `remove`, `read`, `write`, `exists`,
`is_dir`, `is_file` are the operations, each taking its path as written
and raising with the path in the message. The Rust half is
`kawoosh_systems::fs`, and the shell's `:e`, `:cd`, `:oil` and the status
line's `~` go through the same functions, so what the command line accepts
and what a plugin accepts are one thing (the case that filed this: `:oil
~/projects` refused as "not a directory" because the plugin's `is_dir`
never saw the `~`). The file manager is the acceptance test: no `/` in it.

**A command declares what its arguments are, and the engine resolves
them.** `kawoosh.command(name, fn, { args = { "path", "text..." } })` —
the kinds are `path`, `buffer`, `command`, `option`, `tool`, `view`,
`text`, the last with `...` for the rest — and the same declaration in
Rust (`Editor::register_with_args`, `Editor::declare` for a command the
shell runs). A `path` reaches the command absolute, whoever registered it
— the engine's `:w`, the shell's `:cd`, a plugin's `:oil` — resolved once
in `Editor::run` against the working directory the shell keeps the engine
told of; and the command line completes each argument from its kind, so
neither the completer nor the plugin keeps a list of which commands take
paths. The declaration is data on the command (`Args`), which is the
extension surface: a plugin can read it, and a kind can be added without
touching a plugin.

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

### 9. Notifications: a level says where it shows

The old shape was one message line under the command line, set from
forty places and overwritten by the next; anything asynchronous — a
server that failed to start, a compile that finished, `$/progress` —
either took the line or was lost. Now a notification has a **level**
(`debug`, `info`, `warn`, `error`) and the level decides where it shows
(`notify.rs`):

- **error, warn → a toast.** A bordered card at the top-right under the
  tab strip, gone after eight seconds — or, when it carries **actions**,
  only when one is taken (a toast with actions is a question, and a
  question does not time out). A plain toast goes on a click too. The
  keyboard reaches them: `<C-w>n` (`:toast`) puts it on the newest —
  `TOAST` in the status strip — `j` `k` walk the toasts, `h` `l` the
  actions, `<CR>` takes one, a digit takes that one, `x` takes a plain
  toast down, `<Esc>` leaves; a focused toast does not time out under
  the user.
- **info → a corner line.** A dim line at the bottom-right above the
  strips, gone after four seconds, grouped under its **source** with
  the source's name below the group — fidget's shape. A language
  server's `$/progress` is one of these, live while it runs (`Indexing
  163/336 48%`), `Completed …` once done, the server's name ticked when
  every token of its is done. The same thing said again counts up,
  `(2x)`, instead of showing twice.
- **debug → the log only.** And **trace → nothing**, unless
  `RUST_LOG=trace` asks the log to keep it: a server's stderr
  (rust-analyzer says a line per watched path), a startup fact. The
  `log` crate's max level is set to match, so a trace nobody keeps
  costs its macro one atomic load. The log keeps the last thousand
  entries;
  `:messages` opens it as the read-only `*messages*` buffer — time,
  level, source, text, a repeat's count — live while it is open, and
  `:messages clear` empties it. What the command line shows lands in it
  too, so the old message line keeps its job (a command's immediate
  answer) and stops being the record — and it clears itself after
  `ECHO_TTL` (8 s), or on `<Esc>` at once; before, a message stayed
  until the next one, across buffers, with no way to dismiss it.

A notification is data — `Note { level, source, text, actions, show,
ttl }` — and an **action is a command line**: `{ label, command }`, run
through the engine as `:` would run it, so a toast's buttons come from
the same table a keymap's bindings do, whoever registered the command.
`kawoosh.notify(text, { level=, source=, timeout=, show=, actions = {
{ label=, run = fn | "command" } } })` is the Lua half; a function is
made a command first, as `kawoosh.map` does. `:notify [LEVEL] TEXT` is
the command line's. The shell's own sources are the lsp glue (a server
not found; `window/showMessage` by its type, `window/logMessage` to the
log; `$/progress`, asked for with `window.workDoneProgress`) and compile
mode's exit, and a server's stderr, line by line, a trace under the
server's name; the level is the caller's to pick and `show` overrides
it.

The **`log` crate is a source** (`logger.rs`): `log::warn!` anywhere in
the process is a warn toast, `log::info!` a corner line, `log::debug!`
a line in `:messages` — kawoosh's own crates from debug up, anyone
else's (wgpu, winit) from warn — the source being the module for ours
and the crate for theirs. Its sinks are the notification log and, when
stderr is a terminal or `RUST_LOG` names a level, stderr, where every
entry the notification log takes (a `notify`, an echo, a record) is
written once a frame by the frame. The caller pays nanoseconds: the cut
by level and target comes before the message is looked at, a literal
message is borrowed, a formatted one goes into the record's inline
buffer, the record is one channel send, and only the first of a burst
wakes the loop (an `AtomicBool` the drain clears). `benches/logger.rs`
measures it: ~3 ns for a record nobody wants, ~20 ns for a literal,
~80 ns formatted, with a drain running beside.

A file opening on the io thread is a progress line under `io` —
`Opening NAME 42%`, `Completed Opening NAME` — the same shape as a
server's token. A running token with a percentage is a loader: a bar
under its line, filled that far, whoever owns the token.

Redraw stays event-driven: what times out arms a `systems::Alarm` at the
earliest expiry (`Alarm::spawn_soonest`; the diagnostics debounce keeps
the latest-wins one), so a toast's going brings its own frame and a
quiet editor still draws nothing.

### 10. Settings: data in layers, a file per source, reloaded on save

mvp.md's Decision 8 drew the line — config is code, state is data — and
left settings on the code side: `kawoosh.opt(name, value)` from
`init.lua`, into a flat map of strings the engine read back by parsing.
That shape had no answer for a project: 7b's `.kawoosh/init.lua` would
have been code from a repository, run on open, behind a trust prompt
nobody had built. The answer is that **a setting is data, and the file
that holds it is a Lua table, not a Lua program**:

```lua
-- ~/.config/kawoosh/settings.lua, or <project>/.kawoosh/settings.lua
return {
  tabstop = 2,
  compile = { command = "cargo test" },
  lsp = { rust = { roots = { "Cargo.toml" }, args = { "-v" } } },
}
```

A settings file is evaluated in a sandbox (`Runtime::eval_settings`):
the pure library — `string`, `table`, `math`, `pairs`, `tostring` — and
nothing that reaches out, no `os`, `io`, `require`, no `kawoosh`. So a
file in a repository is read the way a `.editorconfig` is read: on
open, without asking, because it cannot do anything but describe. A
file that reaches for `os` is an error toast naming its line, and the
files beside it still load. (7b's `.kawoosh/init.lua` stays unbuilt:
code from a repository still wants the prompt, and most of what 7b
listed for it — compile commands, LSP settings, tool definitions — is
data, which this file carries.)

**A `Setting` is a tree** — `Bool`, `Int`, `Float`, `Str`, `List`,
`Table` — read by dotted path (`lsp.rust.cmd`), typed at the reader
(`settings.int("tabstop")`), lenient where it costs nothing (a `"2"`
reads as 2). **`Settings` keeps a layer per source**, and the effective
tree is their merge, in this order:

| Layer | Source | Set by |
|---|---|---|
| default | `default` | the engine (`tabstop = 4`, `expandtab`, `scrolloff`, `leader = " "`) |
| user | `~/.config/kawoosh/settings.lua`, then `user` | the file; then `init.lua`'s `kawoosh.opt` |
| project | every `.kawoosh/settings.lua` above the cwd, outermost first | the repository |
| session | `session` | `:set`, and `kawoosh.opt` from a command |

A table over a table merges key by key, so a member crate's file can
add `lsp.rust.args` without repeating the root's `lsp.rust.roots`;
anything else replaces. Each layer is swapped whole: `:cd` replaces the
project layer from the new directory and leaves `:set` alone; `:set
PATH!` takes the session's value out and what was under it shows
again; `:set PATH?` says the value and its origin (`project:
repo/.kawoosh/settings.lua`). `init.lua` runs after the user's file, so
code can read what data said; what it sets lands in the user layer
under the project's files, which is what makes a project file an
*override*. `:compile` with no argument runs `compile.command` — the
setting a project file is there to set.

**The leader is a setting** (`leader = ","`), and a map keeps
`<leader>` as its own token rather than the key it stood for when the
map was made: the keymap resolves it at lookup, so the bundled plugins'
maps — bound before any file loads — and a reloaded file's leader
agree without anything rebound, and `:map` listings say `<leader>cd`.
A pressed key that is the leader's follows both its own branch and the
`<leader>` branch, an exact map on the key winning; a leader map open
past a bound key keeps the sequence waiting rather than firing the
bare key, which is what choosing `,` as leader means in vim too. The
engine applies the leader before a key is looked up whenever the
settings' version moved (`Editor::sync_settings`).

The Lua side is one verb, typed: `kawoosh.opt("tabstop")` is `4` (an
integer, not `"4"`), `kawoosh.opt("lsp")` the subtree as a table,
`kawoosh.opt()` the whole tree; `kawoosh.opt("lsp.rust", { args = {} })`
sets a subtree, `nil` unsets. A function anywhere in the value is
refused where it is (`a function at \`x.f\``) — settings are data, and
the message crossing the boundary (`Msg::Option { path, value }`) is
the same shape a file returns.

**Reload on save.** The user's two files and every candidate
`.kawoosh/settings.lua` above the working directory — whether it exists
yet, so making one counts — are on a watch (`systems::watch`): a
thread stats the set twice a second, posts the paths whose stamp
changed and wakes the loop, and posts nothing while they are still,
so a quiet editor stays parked. Polling rather than the platform's
file events: a handful of paths, a microsecond a stat, no dependency
and no backend per platform. The frame re-layers the file's layer; a
saved `init.lua` runs again with what it set before taken out first,
so a line removed is a setting gone; a corner line under `settings`
says which file (`reloaded .kawoosh/settings.lua`). `:settings reload`
does it on demand.

**The Settings tab** (`:settings`, beside Syntax and Perf in the
devtools) is the state made visible: the layers from the one that wins
down, each source's leaves as `path = value`, a file's row a click from
opening it, and the effective tree with where every value came from. A
layer with no file yet offers one — `~/.config/kawoosh/settings.lua ·
new`, `.kawoosh/settings.lua · new` for the working directory — and the
click opens a buffer at that path with a `return {}` template in it and
nothing on disk: `:w` is the user's (it makes the directory), and the
watch lists the file once it lands. A click in the panel that opens
something hands the keyboard to what opened; a press on a plain row
blurs kui's focus to none, and the focused pane takes it back the
next frame — a modal editor has no state where keys go nowhere. Its header counts the sources and the watched files and says what
was last reloaded and when.

### 11. Histories: a buffer's undo tree and unsaved text survive a restart, and a quit is a hot exit

mvp.md's Decision 7 kept file contents and undo trees out of the store.
That left two things a session did not carry: a scratch buffer came back
empty, and a file left modified came back as the disk had it — so `:q`
had to refuse unsaved changes, vim's way, and closing the window from
outside, or a crash, lost them. Now **a buffer has a history**: a row
in the store (`histories`, keyed `file:<absolute path>` or
`scratch:<n>`) holding its undo tree and, while it is unsaved, its
whole text — a history with unsaved text is a *draft* — and, for a
file, a hash of the disk text it was loaded from. The tree is kept
as each state's edit from its parent, both ways — the text it took out
and the text it put in — so the whole of it rebuilds from the one
text: up from the current state to the root by the edits reversed,
then down every branch (`Editor::history_states` hands the tree out as
data, `set_history_states` takes it back). Typing in progress is one
more state, so a draft written mid-insert can be undone to before it. It is written once the buffer has been still for a
second after an edit, or every ten seconds while it is being typed in,
and dropped the moment the buffer is clean: written, undone back to
what was loaded, discarded. SQLite in WAL mode makes each write whole
or nothing, so a crash mid-write leaves the row as it was.

**A quit is a hot exit.** `:q` and `:qa` go through with unsaved
changes; the session save flushes every draft first, and the next
launch has the text back, modified, with `u` stepping through what was
done to it. `:q!`, `:qa!`, `:bd!` and `:bdo!` are the discard: the
buffer's text goes back to what was loaded and the row goes with it.
`:bd` without `!` still refuses a modified buffer — deleting it is how
its draft would be lost. Without a store (the state db failed to open)
there is nowhere to keep a draft, and `:q` refuses as before; so does a
buffer an `$EDITOR --wait` caller is waiting on, whose caller reads the
disk. The decision the engine used to make in `quit` is the shell's:
`Effect::Quit { force }` carries the `!`, and the shell knows whether
it has a store.

A scratch comes back with the session, in its pane (`PaneData::Editor`
names it by number) or as a buffer without one; a file's draft comes
back whenever the file is opened — with the session or `kawoosh
path` — as one edit over the disk text, so the undo to clean is the
disk. Rows the session does not claim are restored hidden, and rows
come back even without a session (a crash before the first quit), so
nothing waits in the store unseen; the restore's message counts them
(`session restored (2 tab(s), 3 unsaved)`). A file whose disk text is
not the one the draft was taken from gets its draft all the same — the
unsaved work is the user's — and an error toast saying the disk moved:
`:w` writes the draft over it, `:e!` loads the disk's text into the
buffer as one undoable change — clean on it, `u` a step back to the
draft — so both can be looked at before choosing.

**The store is visible and bounded.** `:history` is a pane of its own
(`history_pane.rs`, `Content::History`, kept by a session), beside the
buffer as the undo history is: every row as a table — name and path,
size, when it was last touched, and its state: held by a buffer (on
show, hidden, saved), its file gone, not opened, a saved file's
history, not restored — and
under it the cursor's row inspected: name, language, path, size,
how many states its history holds, whether the disk still has the
text it was taken from, and the unsaved changes themselves as a diff
of the disk's lines against the buffer's, drawn as the undo pane draws
a change. `⏎` or a click opens the cursor's row (the buffer that
holds it shown, a file opened, a scratch restored), `x` drops it,
`q` closes, `<Esc>` hands the keyboard back. The rows are read off
the store when it changed, not once a frame. `:history list` is the
same as text; `:history drop KEY` takes one out, reverting the buffer
that holds it; `:history clear` takes out every row no buffer holds and
`:history clear!` the held ones too, and either `VACUUM`s the db so it
does not sit at its high-water mark.

Every size in both panes is `devtab::Tab`'s — the tokens the devtools
tabs already read off kui's metrics: the strip, the inset, the cell
gap, the small text, the zebra and hover of a table's rows, the diff's
style — plus the one size that is the editor's, a line of buffer text
(`Tab::line_h`), for what a buffer holds. The undo pane's own numbers
(an inset of 8, a gap of 6, text at 11, a strip of 24) were those
tokens' values by hand; now they are the tokens, so the two panes and
the tabs cannot drift, and a density the app sets reaches them all. The setting `history.keep_days`
(90; 0 keeps everything) drops rows untouched that long at the first
frame of a launch, with the hidden buffer holding one — a row is
touched when it is written and when a pane shows its buffer, so a
history looked at every session never ages, and one restored hidden
and never looked at does. A row whose meta cannot be read (another
build's, a corrupted one) still gives its text back, history and name
gone, with a warning: the text is the authoritative part. A row whose
key is not a history's is dropped. A row whose file cannot be opened is
kept, and the listing says so.

**A saved file keeps its history** the way neovim's `undofile` does.
Its row is *clean*: the tree alone, no text — the disk is the text —
and in the text's place a real hash of it (BLAKE3 with the length,
`Base`), because a hash that matches installs edits into a buffer at
their offsets, so a checksum will not do. On open, the disk is hashed:
the same, and the tree goes around it, `u` undoing what was saved;
different — a checkout, an edit elsewhere — and the row is dropped
without a word, the tree having been of another text. A clean row is
not opened by a session restore (nothing is unsaved in it); it waits
for the file. `:w` turns a draft row into a clean one in place; a
discard (`:q!`, `:bd!`, `x` in the pane) clears the buffer's history
along with its changes, so the history does not outlive what it was
of. The rule for a row's existence is therefore: the buffer is
modified or has history; neither, and the row goes.

Bounds, so a row is a few times the buffer at most: a buffer past 8 MB
has no row and a warning says so once; a tree past 200 states or
1 MB of edit text is kept as its trunk — the states `u` walks from the
current one, newest first, as many as fit — each state costing what it
changed (`doc::diff_trees` against its parent). A file that opens on
the io thread is past the cap by definition. And the store as a whole
is capped: `history.max_mb` (64; 0 for none). Past it, after a write
and at the launch sweep, the oldest-touched rows go one by one until
the rest fit — a row nobody holds is dropped, one a buffer holds clean
is its history and the buffer forgets it with the row, one a buffer
holds *modified* is never evicted, since the store is what keeps those
changes; the store may sit over the cap by exactly what is unsaved.
A corner line says how many went.

### 12. A command is a spec and a body

mvp.md's Decision 4 made the command registry the hackability primitive:
everything the editor does has a name Lua can call and bind. What a
command *was* stayed spread out — its name and closure in the engine, its
ex spelling in one table, its arguments in another, its `!` smuggled into
the arguments for each body to scan for, its subcommands string-matched
inside the parent, and whether it could run at all an early `return` with
whatever message that body chose, or none. Now **a command is two things:
a `Spec`, and a body** (`kawoosh_editor::command`).

The spec is data, and it is the whole of what is not behaviour: the
name; the aliases (`bd`, `bdelete` — the old alias table folded into the
specs); the arguments as before; what `!` means to it and what `?` means
to it, each a line of doc whose presence is the permission (`:pwd!` is
refused with "pwd takes no !" before any body runs, `:q!` discards, `:e!`
reloads, `:cd?` says where — the meaning is the command's, the check is
the engine's); the conditions under which it runs; and a line on what it
does. The body is the `Command<H>` trait, implemented on the host it acts
on — `Editor` for the engine's, the shell for the shell's — so that
everything about `:history drop` is in `history.rs` and nothing about it
is in a dispatcher. Most bodies are `FnCommand`, a spec and a closure.
The engine's registry holds every spec — its own, the shell's declared,
Lua's — and the shell keeps its bodies by name; running a command without
a body here is `Effect::Shell` with the context ready, as before, only
now the form, the subcommand and the paths are resolved in it.

**A subcommand is a command whose name is two words.** `history drop`
is registered as one, and the engine walks a line's words as far as they
name subcommands, carrying a `!` or `?` from any of them (`:history
clear!` and `:history! clear` alike), then hands the rest as arguments.
The same walk serves a keymap's binding (`kawoosh.map("n", "<leader>cd",
"oil cd")`) and the command line's completion: under `:history ` the
words are `clear`, `drop`, `list`, and after one the subcommand's own
arguments. Not a second trait — a subcommand needs nothing a command
does not have.

**`when` is a list of facts, not a closure.** A closure could not cross
to Lua, could not be shown, and could not be evaluated inside the engine
when what it asks about is the shell's — a store, an LSP server, which
kind of pane has the keyboard. So the shell *publishes* facts into the
engine as they change (`store`, `lsp`, `editor`, `terminal`, `lua`,
`dock`), a plugin publishes its own (`kawoosh.fact("plug:ready")`), the
engine answers the ones it can from the view (`visual`, `modified`,
`file`, `buffer:NAME`, `language:NAME`), and a spec names them, `!` for
must-not (`when = { "language:oil", "!terminal" }`). The engine refuses
with the reason in one voice — "scrollback needs terminal", "oil cd needs
language:oil" — whether the command came from the line, a key, a toast's
action or Lua, and `kawoosh.can(name)` is that reason or `true`. This is
VS Code's `when` clause without the expression language; a list with
negation is enough, and if a day comes when it is not, that is the day to
add one. Facts are strings, not an enum, so a plugin can invent one
(hackable-by-design over typed): the names the shell and engine answer
are documented in `boot.lua`, and a typo is a command that never runs,
which `kawoosh.can` shows.

Lua never sees the trait. `kawoosh.command(name, fn, opts)` takes the
spec's fields as the table (`args`, `aliases`, `bang`, `query`, `when`,
`doc`), `ctx.form` / `ctx.bang` / `ctx.query` reach the function, and
`kawoosh.commands()` hands every spec back as such a table — the same
data the completion reads, for a palette or a help pane to build from.
The bundled oil is the acceptance test: `oil cd` is a subcommand gated on
`language:oil`, `:oil?` says what is listed, and `<leader>cd` off a
listing runs nothing and says why.

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
