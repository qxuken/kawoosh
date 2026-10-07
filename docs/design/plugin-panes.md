# Plugin panes: direct kui access from Lua

Status: written 2026-09-21 for roadmap step 8. The page the roadmap's
"plugin-built panes" item asked for: how a Lua plugin gets a pane of
its own, draws kui's tree into it, takes keys and clicks, and puts a
one-line input in it. Nothing here is new — `picker.lua` is built on
every piece — so this is the map, with the picker as the worked
example. Companion to [kui.md](kui.md) Decisions 6 and 12 and mvp.md
Decision 8.

## The shape

A plugin pane is **a kui slot the editor fills from Lua**. The editor's
window is a kui tree; a pane whose content is `Content::Lua(name)` is a
`slot` node in that tree, and kui asks the Lua extension to fill it
every frame it is on show (kui ADR 0014). The Lua side answers with a
table of nodes in kui-lua's DSL — `row`, `column`, `text`, the
container props of kui's schema — and kui lays it out, paints it,
hit-tests it and routes its events like any node the editor built
itself. There is no second UI toolkit: the picker's rows and the
editor's tab strip are the same kind of thing.

Four verbs, all in `boot.lua` over the Rust half:

| verb | what |
|---|---|
| `kawoosh.view(name, fn, on_event, opts)` | declares the view: `fn(ctx)` returns the tree, `on_event(ev)` takes its clicks and keys |
| `kawoosh.view_open(name, { focus, below, share })` / `view_close` / `view_toggle` | puts it in a pane — a column of its own, or under the focused pane when `below` ([pane-placement.md](pane-placement.md)), at `share` of it — and takes it out |
| `ctx.field { name, placeholder, size, width }` and `kawoosh.field_focus`, `field_text`, `field_set` | a one-line input drawn through the editor (Decision 12) |
| `kawoosh.map(mode, keys, cmd, { when = { "field:lua:NAME/FIELD" } })` | keys while the field has them |

`:view NAME` opens one from the command line; a view with
`opts.session = false` is not brought back by a session.

## Drawing: `fn(ctx)`

`ctx` carries what the slot knows: `pane` (its id), `focused`, `width`
and `height` in logical px (the height counting the pane's title bar,
`title_h` tall at the chrome's font), `share` (its fraction of the split it is in,
`nil` when it is the whole window), `env` (kui's env reading — the
theme under `env.theme`, the tokens, the viewport), and `name`. The
function returns a node:

```lua
kawoosh.view("counter", function(ctx)
  local t = ctx.env.theme
  return column { pad = 8, gap = 4,
    text("count: " .. count, { color = t.fg }),
    row { pad_x = 8, pad_y = 2, bg = t.surface, radius = 4,
          on_click = { kind = "bump" }, text("bump") },
  }
end, function(ev)
  if ev.kind == "bump" then count = count + 1 end
  if ev.kind == "key" and ev.key == "q" then kawoosh.cmd("close") end
end)
```

The DSL is kui-lua's prelude, documented in kui's `props.md` (container
props, text props, events) and conformance-tested against the other
bindings — a `bg = "$keyword"` names a token the editor declared
(`look.rs`), `env.theme.surface` a role. A view that raises draws its
error in the pane instead of taking the frame down; one that returns
something other than a table says so the same way.

Sizes are the frame's: a list that must fit uses `ctx.height` and the
row height it draws at, and slides its window itself (`picker.lua`'s
`ensure_visible`). A divider drag changes `ctx.share` the next frame,
which is how the picker keeps `picker.share` as the setting.

## Sizes: one scale, the chrome's

Asked 2026-10-02: "make it the same font size as the rest. ensure all
panels uses the same token for metrics. if no, use existing or add one
and expose to the settings". Every pane — Rust or Lua — draws its text
at one of three sizes, the chrome's (`look::Chrome`): the size the tab
strip and the title bars are set in, `font.chrome_size`, or the
editor's font up to 16 px when that is `0` — the setting there was.

| step | default | what |
|---|---|---|
| `text` (`$chrome`) | 13 | a pane's text: its rows, its fields, its headings (bold) |
| `small` (`$chrome_small`) | 12 | a step under: secondary text, a chip, a status, a pane's title |
| `note` (`$chrome_note`) | 11 | two under: a note, a count, a tag, a key legend |
| `row` (`$chrome_row`) | 20 | a line of `text` with the chrome's air |

A Lua view reads them as `ctx.metrics` (`kawoosh.metrics(env)` from
anywhere with kui's env), and `ctx.field`, `ctx.icon`, `ctx.keys` and
`ctx.legend` draw at them unless given a size; a Rust pane through
`devtab::Tab`, whose text sizes are the scale's and whose insets and
gaps kui's `Metrics`. A line of buffer text in a pane stays the
editor's (`font.size`, `Tab::line_h`), and a specimen — the fonts
pane's family names in their own face, the font lab — is drawn at the
size it shows.

What the panes read, before and after:

| pane | before | after |
|---|---|---|
| tabs, title bars, strips, message line, confirm, toasts, breadcrumbs | `Chrome` | `Chrome` |
| which-key | `Chrome`, its hints `small - 1` | `Chrome`, its hints `note` |
| memory, undo history | `devtab::Tab` off kui's `Metrics::hint_text` (12, 11): deaf to `font.chrome_size` | `Tab` off the scale (`small`, `note`) |
| Perf, Frames (devtools) | `Tab` off kui's hint size | `Tab` off the scale |
| Syntax (devtools) | the buffer's `font.size - 1`, rows 18 px | `Tab` |
| search's bar | `SIZE = 13` of its own, never the token; labels 12, keys 10 | `ctx.metrics` |
| picker | `$chrome`, `$chrome_small` | `kawoosh.metrics` |
| grammars, fonts, themes, theme lab, launcher | `$chrome` and sums: `- 1`, `- 2`, `- 3` (themes' badge) | `text`, `small`, `note` |
| settings | `$chrome` and sums; headings `+ 3` and `+ 1`, filter caps `- 3` | the scale; headings bold at `text`, as every pane's |
| du | `$chrome` and sums; rows 22 px of their own | the scale; rows `row + 2` |
| dir | `$chrome_small`, read raw | `ctx.metrics.small` |
| boot.lua's field, caps, legend | 13, 12, 12 | `text`, `note`, `note` |
| lists, the memory's Lua half, vcs, status | no text of their own (multibuffers, Rust panes) | — |

`kawoosh/tests/metrics.rs` opens the panes with the scale set to 22 px
and finds no text under its `note`: a size kept as a number stays
behind and is found smaller.

Not yet: a row's height is still each pane's sum over `text` (a field
`+ 6`, a chip or the launcher's row `+ 8`, a grammar's `+ 16`), and the
Lua panes' paddings are numbers of their own (8, 14) where the Rust
ones' are kui's `Metrics` — they follow nothing a user sets, and
nothing asked for it yet. `kawoosh.icon` outside a view draws 13 px
unless given a size.

## Events: `on_event(ev)`

Every `on_click = { … }` payload in the tree comes back as `ev` with
its fields, plus `ev.slot` (which pane). A key pressed while the pane
has the keyboard — and no field has it — arrives as `{ kind = "key",
key = "j", … }` in the keymap's notation; the handler keeps it by
returning `true`, and any other key is **pane mode's** (keys.md
"Panes without a view"): the view's own `kawoosh.map("p", …)` bindings,
the list keys, and what every pane shares — `<C-w>…`, `<leader>…`,
`:`, the shift chords — which is how a plugin's pane gets the
command line and the pane cluster without handling a key. A view's
pane-mode maps are its own, local to the place `lua:NAME`, which holds
while the view's pane has the keys, its field under them or not
([local-maps.md](local-maps.md), 2026-09-29; they were gated on the
fact by a `when` until then, and found by every other pane's lookup):

```lua
kawoosh.map("p", "j", "finder next", { view = "finder" })
kawoosh.map("p", "<CR>", "finder pick", { view = "finder" })
```

Prefer a map to handling `ev.kind == "key"`: a map is listed by `:map
list p`, remapped by a user, counted (`3j`) and shown by the which-key,
and a handler is none of those. A click on a field node focuses it
before the handler sees anything.

A node's `on_drag` and `on_scroll` come back under kui's own kinds —
`ev.kind == "drag"` (with `phase`, `x`, `y`, `parent`) and `"scroll"`
(with `dx`, `dy`) — the payload the node declared under `ev.tag`,
where a click's payload is the event itself. On a `cells` grid each
carries what the grid knows: a click and a drag their `cell = { row,
col }`, the wheel its whole `lines` (positive toward later rows).
`hex.lua` is the example: one grid, a cursor put by `cell`, rows
scrolled by `lines`.

## Fields: a line of the editor in the pane

`ctx.field { name = "q", placeholder = "find", size = 13 }` returns the
node to put in the tree — a `fill` the engine draws, the line the app's
own fields are (`fields.rs`, lua-boundary.md Decision 8): tabs and
escapes, every selection, a caret per selection — as wide as its line
unless `width` says `"grow"` (the room its row has) or pixels; a fill
is a position, so its width is an option, not a prop set on what came
back. The field is an engine view named
`lua:NAME/q` — one line of a real buffer with the editor's modes, so
`ciw`, `<C-w>`, `.` and a search in it are the editor's. `kawoosh.
field_focus(NAME, "q")` gives it the keys (insert mode; `<Esc>` is
normal mode over the line, `<Esc>` again hands the keys back to the
view — `field blur`); `field_focus(NAME, nil)` takes them back;
`field_text` and `field_set` read and write the line. The status line
shows the field's mode (`INS`, `NOR`) while it has the keys.

Keys the plugin wants on the field are the field's own maps, local to
`field:lua:NAME/q`, which holds only while that field has the keys:

```lua
local AT = { view = "finder", field = "q" }
kawoosh.command("finder submit", function() … end, { when = { "field:lua:finder/q" } })
kawoosh.map("i", "<CR>", "finder submit", AT)
kawoosh.map("n", "<Esc>", "finder close", AT)
```

The map rides over the editor's own binding on the same key in the
field and is not there anywhere else — the picker's `<C-n>` walks its
rows in the query and cycles the completion everywhere else.

## The worked example: `picker.lua`

One view (`picker`), one field (`q`), one fact
(`field:lua:picker/q`). `picker.open` calls `kawoosh.view_open(VIEW, {
below = true, share = share() })`, `field_set(VIEW, FIELD, query)` and
`field_focus(VIEW, FIELD)`; the view function reads `ctx.field_text(
FIELD)`, refilters when it moved, and draws the rows with
`picker.rows(ctx, hits, opts)` — a function a plugin can call into a
view of its own — and the preview beside them; every key is a
`picker …` command mapped under the fact; the pane's height comes back
through `ctx.share`. `kawoosh.picker.state()` is what a test reads.

## Testing a pane: `kawoosh test`

A plugin's test is a Lua script run against a headless editor
(`harness.rs`): `kawoosh test PATH…` from the CLI, or `cargo test`
for the bundled ones under `kawoosh/lua/tests`. The script presses keys
and reads the state:

```lua
kawoosh.press("<leader>f")            -- keys in map notation
kawoosh.frame(2)                      -- frames to draw
local st = kawoosh.picker.state()
kawoosh.test.eq(st.source, "files")
kawoosh.wait(function() return kawoosh.picker.state().count > 0 end)
kawoosh.press("<CR>")
kawoosh.test.eq(kawoosh.buf.line(1), "pub fn lib() {}")
```

The script is a coroutine: `press`, `frame`, `sleep` and `wait` yield
to the editor, which does the thing and publishes the state again
before resuming, so the next line reads the editor as it is then.
`kawoosh.mode()`, `kawoosh.buf.*`, `kawoosh.message()` and a plugin's
own readers (`picker.state()`) are the assertions' material;
`kawoosh.test.eq`, `ok` and `has`, or a plain `assert`, fail the run
with the script's line and a traceback. A kui warning raised during
the run fails it too.

## What is not here

One thing the editor's: a Lua pane has no `session` record of its own
beyond being reopened by name, so a plugin that wants its state back
keeps it in `kawoosh.store`. (The two that were kui's are here: a view's
nodes take `enter`, `exit` and `keyframes` as any binding's do, and an
image is `kawoosh.image(path)`'s handle in kui's `image { id = }` —
`image.lua` is the example: a picture zoomed and moved over, one that
moves played by `{ play = true }`, a drawing drawn at `{ width = }`.)
