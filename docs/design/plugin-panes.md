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
| `kawoosh.view_open(name, { focus, below, share })` / `view_close` / `view_toggle` | puts it in a pane — a split of the focused pane, below when asked, at `share` of it — and takes it out |
| `ctx.field { name, placeholder, size }` and `kawoosh.field_focus`, `field_text`, `field_set` | a one-line input drawn through the editor (Decision 12) |
| `kawoosh.map(mode, keys, cmd, { when = { "field:lua:NAME/FIELD" } })` | keys while the field has them |

`:view NAME` opens one from the command line; a view with
`opts.session = false` is not brought back by a session.

## Drawing: `fn(ctx)`

`ctx` carries what the slot knows: `pane` (its id), `focused`, `width`
and `height` in logical px, `share` (its fraction of the split it is in,
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
pane-mode maps are gated on the fact `lua:NAME`, which holds while
the view's pane has the keys, its field under them or not:

```lua
kawoosh.map("p", "j", "finder next", { when = { "lua:finder" } })
kawoosh.map("p", "<CR>", "finder pick", { when = { "lua:finder" } })
```

Prefer a map to handling `ev.kind == "key"`: a map is listed by `:map
list p`, remapped by a user, counted (`3j`) and shown by the which-key,
and a handler is none of those. A click on a field node focuses it
before the handler sees anything.

## Fields: a line of the editor in the pane

`ctx.field { name = "q", placeholder = "find", size = 13 }` returns the
node to put in the tree, and the field is an engine view named
`lua:NAME/q` — one line of a real buffer with the editor's modes, so
`ciw`, `<C-w>`, `.` and a search in it are the editor's. `kawoosh.
field_focus(NAME, "q")` gives it the keys (insert mode; `<Esc>` is
normal mode over the line, `<Esc>` again hands the keys back to the
view — `field blur`); `field_focus(NAME, nil)` takes them back;
`field_text` and `field_set` read and write the line. The status line
shows the field's mode (`INS`, `NOR`) while it has the keys.

Keys the plugin wants on the field are maps gated on the fact
`field:lua:NAME/q`, which holds only while that field has the keys:

```lua
local AT = { when = { "field:lua:finder/q" } }
kawoosh.command("finder submit", function() … end, { when = AT.when })
kawoosh.map("i", "<CR>", "finder submit", AT)
kawoosh.map("n", "<Esc>", "finder close", AT)
```

The map rides over the editor's own binding on the same key while the
fact holds and falls through when it does not — the picker's `<C-n>`
walks its rows in the query and cycles the completion everywhere else.

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

Two things a plugin pane cannot do yet, both kui's: an entrance or
keyframe animation on its nodes (kui-requirements §9), and images
(deferred). And one thing the editor's: a Lua pane has no `session`
record of its own beyond being reopened by name, so a plugin that
wants its state back keeps it in `kawoosh.store`.
