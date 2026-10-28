# Plugins as kui extensions

Status: **built 2026-10-27**, Decisions 1–3 and 5, but the launcher's
slot. Asked with kui alpha.68's migration: "maybe we can utilize slots
for tabbar for example, statuslines etc. maybe more", and, offered three
ways (slots for native extensions only, none for now, or every plugin its
own kui extension), the third: "Redesign plugins as extensions". The
calls below are taken here, each the user's to overturn; the ones marked
*open* are left for them. What the building changed is at the end.

Companion to [lua-boundary.md](lua-boundary.md) (what is Rust's and
what is Lua's), [plugin-panes.md](plugin-panes.md) (what a plugin can
draw), [native.md](native.md) (a C extension's pane), kui ADR 0060
(components), ADR 0067 (slots fold into components; a slot is where
extensions contribute) and ADR 0058 (the recording).

## What there is

- **One Lua state, one kui extension.** `kawoosh_lua::Runtime` holds a
  single `mlua::Lua`; `boot.lua` and the 29 bundled plugins
  (`plugins.rs`, in an order that matters) and the user's `init.lua`
  all run in it. kui sees one extension, `lua`, whose `components["*"]`
  draws every pane (`lua/NAME@PANE`) and whose `on_event` gets every
  click under it; `boot.lua` routes by the view's name.
- **Plugins share tables.** `kawoosh.picker` is picker.lua's, and man,
  search, vcs, node_actions and dirs open pickers through it; du deletes
  through `kawoosh.dir.remove`; fonts and theme_lab share themes.lua's
  sample. A plugin is a file, not a sandbox.
- **The chrome is the shell's, fed data.** A plugin adds to the title
  bar, the tab strip and the status line with `kawoosh.status(name, fn,
  { place })` (status.md, statusline.md Decision 3): `fn` answers parts
  in theme colour words and the shell draws, fits and cuts them.
- **kui alpha.68's slots** (ADR 0067): a host declares `slot(name,
  props)`; every loaded extension that lists a contribution to `name`
  has a component placed there, in load order then `order`, each its own
  instance. Contributions are read once, when the extension is loaded.
- **kui-lua's `reentry`** hands out a second `LuaExtension` over the
  same state, with an env of its own, for a component placed while its
  extension is still drawing; its contributions are empty.

## Decisions (proposed)

### 1. A plugin is an extension handle over the one state

Each plugin file, bundled or the user's, is loaded into kui as an
extension of its own under its name as namespace (`dir`, `picker`,
`vcs`; `init` for `init.lua`), all sharing the one Lua state: a handle
like `reentry`'s, which draws with the plugin's functions and routes the
plugin's events back to `boot.lua`'s dispatch. Separate Lua states are
beaten: they would break every shared table above, and load order,
for nothing a plugin asked for.

What it buys:

- **kui knows whose a node is.** Every pane's fill is its plugin's
  origin: kui's recording times `fill picker`, `fill vcs` (ADR 0058,
  decision 4's amendment) with no profiler of kawoosh's, and the
  devtools' tree names the plugin a node came from.
- **A plugin contributes to the chrome** through kui's slots (Decision
  3), the shell declaring where and the plugin what.
- **Plugins provide components to each other**: picker's list as a
  `picker/list` a plugin places in its own pane, kui keying and
  replaying each instance (ADR 0060).

### 2. A pane is its plugin's component

`kawoosh.view(name, fn, on_event, opts)` registers `name` as a
component of the plugin that called it; the pane is placed as
`PLUGIN/NAME@PANE`, keyed by that name, with the `rev` claim of
lua-boundary.md Decision 10 (amended 2026-10-27). The view's events
reach its plugin's handle and `boot.lua`'s routing as now
(`ev.component` names it). Native panes (native.md Decision 4) are
already this shape.

### 3. The slots the shell declares

Each a contribution point a plugin's component can be placed in, with
the props kawoosh passes it (theme words, the pane, the room):

| slot | where | props |
|---|---|---|
| `kawoosh.statusline` | where the layout's `...` stands | `width` left after the fitting |
| `kawoosh.title` | the title bar's segments, before the window's buttons | — |
| `kawoosh.tabs` | after the last tab, before the new-tab button | — |
| `kawoosh.pane_title` | a pane's title bar, before its close button | `pane`, `kind`, `focused` |
| `kawoosh.launcher` | a launcher module the layout names `slots` | `pane`, `origin` |

A plugin says `kawoosh.contribute(slot, view_fn, { order })` at its
top level; the function is drawn as a component with the slot's props,
its events its own. A contribution's width counts against the status
line's path fitting from the frame before (`layout_of` on the slot's
wrapper), as the Lua modules' texts count now.

### 4. `kawoosh.status` stays

Data in, the shell drawing, is what lets the line fit the path to the
room and cut a module mid-word with `…` (statusline.md Decision 2); a
component's width is known only once it is laid out. Slots are for what
a segment cannot be — a button, a progress bar, a sparkline — not a
replacement. *Open*: whether `kawoosh.status` should become sugar over
a contribution.

### 5. Plugin time

`fill PLUGIN` on kui's recording covers a plugin's panes and
contributions; `prof.rs` keeps timing what is not a fill — a hook, a
command, a job's answer — each a span `plugin NAME` where it ran,
through kui's thread spans (kui F215, alpha.69), nested under the fill,
the handler or the build it ran in.

## What it costs

- **kui**: a handle over a shared `LuaExtension` with contributions of
  its own — `reentry` has none, so kawoosh wraps it (below; kui F217);
  and contributions that can change after load (a plugin
  reloaded, a contribution made from `init.lua` once the plugin is in)
  — today they are read once (kui F218).
- **kawoosh**: `Runtime` loads each plugin under a handle (its name from
  the chunk, `plugins.rs`), `scripting.rs` places `PLUGIN/NAME@PANE`
  where it places `lua/NAME@PANE`, `boot.lua` keeps a view's plugin
  beside its function, five `ui.slot` calls in the chrome, and
  `kawoosh.contribute` with its Lua types. The tests that name
  `lua/NAME@PANE` (view_scroll, idle_replay, launcher, native) follow.

## What the building changed

- **No kui change was needed for Decision 1.** A handle over the shared
  state is kui-lua's `reentry`, which a host can call: `attach_lua` keeps
  one (`Scripting::lua_spare`) and makes each plugin's from it, wrapped
  in `fields::PluginHost`, kawoosh's own `Extension`, which answers the
  plugin's namespace, `["*"]` and its contributions and hands drawing and
  events to the handle. kui F217 asks for the same as a door of its own,
  `reentry` being meant for a spare while an extension draws.
- **Loaded from the host's view** (`Kawoosh::load_plugin_extensions`,
  a lap of the `lua` span): kui's list takes an extension from any fill
  of the frame, the host's included, so a plugin met since the frame
  before is loaded before the panes are drawn. `boot.lua` counts the
  namespaces met and the contributions said (`kawoosh._plugins_gen`), so
  a frame with none new reads one number. The engine's drawing loads the
  same way, from the shell, where `LuaHost` loaded it the first time a
  Lua pane drew — no pane of the runtime's own draws now.
- **A plugin is its file** (`prof.rs`'s `plugin_name`, `.lua` dropped):
  `dir`, `picker`, `init` for `init.lua`, never `lua` or `engine`. A
  view whose plugin has no extension (its load refused, a namespace
  taken by a native one) is drawn by the runtime's own, `lua`, as before.
- **A contribution is drawn fresh** every frame its slot is: what it
  reads is its plugin's, which kui cannot see move, as a `kawoosh.status`
  segment is asked every frame. Its row is keyed by the slot, and the
  width it took the frame before counts where the chrome fits text (the
  status line's path, a pane's title) — a component's width is known
  once it is laid out. No row while no loaded plugin contributes to the
  slot, so an empty one takes no room and no gap; kawoosh knows it from
  the plugins' contributions, as kui has no door to ask whether a slot
  has any before placing it (kui F219) — a C extension's contribution
  to one no plugin contributes to is not drawn.
- **The status line keeps the keyboard with the pane** (`keep_focus`,
  as the title bar and the tab strip had): a click on a module with a
  `run`, and now on a contribution, took it to the clicked node.
- **Tested** in `kawoosh/tests/plugin_extensions.rs`: a plugin's four
  contributions drawn in their slots, its click its own, the editor's
  keys after it; the launcher pane timed as `fill launcher`.

## Open

- Whether `kawoosh.status` becomes a contribution (Decision 4).
- Which of Decision 3's slots to build first, and whether the launcher
  slot replaces `launcher.layout`'s Lua modules.
- Whether a plugin's components (Decision 1, picker's list) are an API
  kawoosh documents, or kui's mechanism only.
