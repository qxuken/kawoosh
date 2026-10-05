# What kawoosh requires of kui

Status: written 2026-09-15 against kui `0.1.0-alpha.12`; K1–K4 shipped the
same day in `0.1.0-alpha.13` (source at `~/projects/kui`; kawoosh builds
against the path until the registry has it). Companion to [kui.md](kui.md),
which decides how kawoosh is built on kui; this file is the list of what
that build needs from the library, checked door by door against alpha.12
so that each line is either a fact with the API named or a gap with the
change named. It is written to be pasted into kui's `docs/BACKLOG.md` as
the **fourth editor-and-mux round** — the first three were assessments of
kui by kui; this one is the consumer's list.

Each requirement carries a state:

- **✓ have** — alpha.12 does it; the door is named.
- **◐ have, awkward** — alpha.12 does it with a workaround kawoosh would
  rather not carry; the better kui shape is named.
- **✗ need** — alpha.12 did not do it; the proposal is a `K` item (all four shipped in alpha.13).
- **○ wish** — not needed for the MVP; recorded so it is not re-derived.

And the kawoosh milestone (kui.md's build order, M1–M8) that first needs it.

## 1. Packaging and versioning

| | Requirement | State | Door / proposal |
|---|---|---|---|
| R1.1 | `kui-native` (`kui` until alpha.19), `kui-core`, `kui-lua` resolvable from the drydock9 registry at one exact pre-release | ✓ | verified 2026-09-15: `=0.1.0-alpha.12`, 23 s cold build, M1; on `=0.1.0-alpha.14` from the registry since the same day; on `=0.1.0-alpha.19` since 2026-09-26, where the runner crate is `kui-native`; by path through alpha.20–22, on `=0.1.0-alpha.23` since 2026-09-28, by path for F108, on `=0.1.0-alpha.24` since the same day, by path for F109 and F110, on `=0.1.0-alpha.25` since 2026-09-29, by path for F111, and on `=0.1.0-alpha.26` since the same day; from crates.io at `=0.1.0-alpha.34` since 2026-10-03 `=0.1.0-alpha.36` since 2026-10-05, by path for F120–F122, and `=0.1.0-alpha.38` since 2026-10-06 (drydock9's copies take their kui-* siblings from crates.io from alpha.34 on, two `kui_core`s in one graph) |
| R1.2 | One Lua for the whole binary, and it is kui-lua's | ✓ | `mlua` `lua55` + `vendored`; kawoosh adopts 5.5 (kui.md D6). Kawoosh will not ask for a LuaJIT feature. |
| R1.3 | The surface kawoosh touches is stable across alphas or its breaks are in the CHANGELOG's "what an app can delete" list | ✓ | the CHANGELOG already does this. The surface: `App`, `Waker`, `Launcher`, `Ui` (open/with/text/cells/slot/take_key_focus/set_clipboard/request_paste/caret_rect/set_scroll/nodes), `NodeSpec`, `TextStyle`, `CellGrid`, `Extensions`, `LuaExtension::{from_file, lua}`, `Core::{frame_with, press, take_warnings, set_inspect, nodes, load_fonts_dir, set_tokens}`. |
| R1.4 | kui-core is a data-contract dependency a non-UI crate may take | ✓ | `term` depends on `kui-core` for `Cell`/`CellGrid` only; kui-core pulls cosmic-text but no window or GPU, which is acceptable. A `kui-types` split is **not** requested. |

## 2. Runner and loop

| | Requirement | State | Door / proposal |
|---|---|---|---|
| R2.1 | The loop parks between events; a frame is drawn only for input, a wake, a transition or `request_frame` | ✓ | `ControlFlow::Wait`; C27 measured the idle cost. M1 |
| R2.2 | A thread-safe handle that wakes the loop, cloneable into any number of threads, coalescing | ✓ | `App::setup(Waker)`, `Waker::wake` (C21). Systems get it as a `Box<dyn Fn() + Send + Sync>`. M4 |
| R2.3 | Bundled fonts loaded from a directory and selectable per style; the mono family resolved upright | ✓ | `Core::load_fonts_dir`, `TextStyle::font(id)`; C32 pinned the generic mono. M1 |
| R2.4 | Window title, minimum size, an opening size from the environment | ✓ | `ui.window_title`, `Launcher::min_size`, `KUI_WINDOW=WxH`. M1 |
| R2.5 | Devtools with no app code | ✓ | `KUI_DEVTOOLS=1`. M1 |
| R2.6 | Window position, declared on open and read back, for session restore | ○ | kui's own wish list (third round). Sessions restore size only until then. M8 |
| R2.7 | Native menu bar with the standard macOS rows kawoosh does not draw | ✓ | ADR 0018 / 0030; kawoosh declares its bar since 2026-10-03 ([menus.md](menus.md)) with a `Window` menu for the platform's rows. A declared bar loses winit's Hide / Hide Others / Services: kui has no application-menu roles (ADR 0018 declined them), a kui round when wanted. |
| R2.8 | Context menus over custom-drawn panes | ✓ | ADR 0017: `on_context_menu`, `Core::open_menu`; an editor claims the secondary button (`on_button`, F105) for the press's `line` and `byte` ([menus.md](menus.md)). |

## 3. The editor pane: rows of runs

kui.md D3: a visible line is a `row` of mono `text` runs inside one
`on_key` sink; selection is a `bg` container around runs, the caret an
inline node, virtual text a run in a `role="none"` wrapper.

| | Requirement | State | Door / proposal |
|---|---|---|---|
| R3.1 | ~8 runs × 55 lines × 2 panes warm in well under a frame; one line retyped or scrolled in costs a fraction of it | ✓ | `highlight` bench: 160 µs / 228 µs / 282 µs; cold file 1.65 ms. Gated by `scripts/bench-check.sh`. M1 |
| R3.2 | Runs of one style and different colors share the shaping cache | ✓ | cache keyed (content, style, scale), color excluded. |
| R3.3 | A press or drag inside the sink says which drawn line and which byte of its text, and the click count | ✓ | `on_drag` payload `line` / `byte` / `clicks` on `Role::Line` rows (C34); `byte` counts across the row's runs (AR30). M2 |
| R3.4 | Runs that are not document text — gutter, ghost completion, inlay hint, EOL diagnostic — are excluded from `byte` and from what a screen reader reads | ✓ | a `role="none"` ancestor takes its subtree out of the line's text (AR30). Kawoosh wraps every virtual run in one. M6 |
| R3.5 | A blinking caret for an app-owned caret, on the runner's clock, parked off when the window loses the keyboard | ✓ | `Role::Line` row `.caret(byte)` arms the clock; `ui.caret_visible()` reads the phase (C35). M2 |
| R3.6 | The OS IME candidate window anchored at that caret; composition delivered to the sink as data, commit as `text` | ✓ | `ime_rect` from the sink's `caret` row; `{kind="preedit", text, cursor}` then `{kind="text"}` (C17). M2 — test with a CJK layout in M2, not later. |
| R3.7 | Clipboard out and paste in for a key sink, without an `edit` widget | ✓ | `ui.set_clipboard`, `ui.request_paste` → `{kind="text"}` (C33). M2 |
| R3.8 | Trailing and repeated spaces measure reliably in a run | ✓ | **K3, alpha.13**: a run's spaces measure at the face's advance — leading, repeated and trailing — so no NBSP mapping and no 2-byte arithmetic. |
| R3.9 | A fixed-stride virtual list with programmatic scroll and a readable visible range, so the pane can own culling *or* hand it to kui | ✓ | `widgets::uniform_list` (`virtual_column` until alpha.19) + `set_scroll` + `scroll_geometry`; or `on_scroll` `{lines}` on the sink with app-owned `top` (ADR 0029). M2 decides which; both are there. |
| R3.10 | A path to soft wrap without changing the data shape | ✓ | one `rich_text` per line (`Span` color + per-span `bg`) wraps as a paragraph; `caret_rect(key, byte)` places the caret. `highlight_2x55_warm_rich` 81 µs. Post-MVP. |
| R3.11 | An underline of its own color and style (a diagnostic's wavy red under keyword-colored text) | ✓ | **K4, alpha.13**: `TextStyle::underline_color` / `underline_style` (`Solid`, `Wavy`, `Dotted`), the same on `Span`. M6 |
| R3.12 | A long line (a minified bundle) costs the screenful it shows | ✓ | chunked shaping (C19): 100k chars, 18 ms first frame, 160 µs to edit. |
| R3.13 | The shaped-text cache is bounded | ✓ | `set_text_cache_budget`, 64 MB default (C16). |
| R3.14 | The pointer is an I-beam over text, an arrow over chrome | ✓ | `cursor = "text"` on the sink. |

## 4. The terminal pane: `cells`

| | Requirement | State | Door / proposal |
|---|---|---|---|
| R4.1 | A screenful of cells with per-cell fg/bg/bold/italic/underline/strike/wide, a cursor of three shapes, cheap when every cell changes | ✓ | `CellGrid`, `Cell::flags`, `CursorShape`; 200×50 streaming ~60 µs. M4 |
| R4.2 | Selection in cells (drag, word, row, block), copy trimming trailing blanks, ends as absolute session lines | ✓ | ADR 0017 + `origin_line`; `cell_selection()`, `request_copy`. M4 |
| R4.3 | The wheel and a drag past the edge arrive as whole lines the app adds to its own `top` | ✓ | `on_scroll` → `{kind="scroll", lines}` (ADR 0029). M4 |
| R4.4 | A click says which cell, for `gf` from terminal output | ✓ | pointer payload `cell: {row, col}` (C20). M4 |
| R4.5 | Underline color and undercurl per cell (SGR 58 / 4:3) | ✓ | **K4, alpha.13**: `Cell::ul`, `cells::flags::WAVY` / `DOTTED`. M4 |
| R4.6 | Hyperlinks per cell (OSC 8) | ✓ | not kui's after all (roadmap step 54, 2026-09-28): `term` keeps each cell's link (alacritty's) and kawoosh finds the run under the pointer's `cell: {row, col}` (R4.4), so the grid needs no link field. |
| R4.7 | The middle button, for the Linux primary-selection paste | ✓ | kui F105 (2026-09-28, roadmap step 55): `on_button` + `buttons` — a node claims the non-primary buttons, captured from press to release, with `cell` on a grid; a claimed secondary press is the node's instead of a context menu. kawoosh's middle button pastes the clipboard (macOS has no primary selection). |
| R4.8 | Blink, dim, inverse, hidden, and the cursor's own color | ✓ | app-side attributes resolved into `fg`/`bg` before the grid; cursor takes a `Color`. |

## 5. Input

| | Requirement | State | Door / proposal |
|---|---|---|---|
| R5.1 | Every press as data with a layout-resolved `code`, a US-QWERTY `physical`, modifiers, `text` and `repeat` | ✓ | `{kind="key", code, physical, shift, ctrl, alt, super, text, repeat}` (ADR 0002 d.11). The hybrid rule is kawoosh's (kui.md D5). M2 |
| R5.2 | A root sink hears every key when nothing is focused, Tab included, and never has to take focus | ✓ | ADR 0022's rule; a modal editor owns Tab. M1 |
| R5.3 | Keys a focused control does not consume bubble to the enclosing sink | ✓ | ADR 0011 — a Lua pane's `edit` keeps typing, `Esc` reaches kawoosh; `ui.blur()` hands it back. M7 |
| R5.4 | Modifier state as an event, so ⌘-drag pane moves and plain clicks coexist without a core concept | ✓ | `{kind="modifiers"}` (splitmux). M3 |
| R5.5 | Presses and releases where a held key matters | ✓ | `key_up` opt-in; kawoosh does not use it. |
| R5.6 | Headless key delivery on both channels in one call | ✓ | `Core::press`. |

## 6. Extensions and Lua

kui.md D6: one `LuaExtension`, one state, kawoosh's API seeded into it;
every Lua-drawn pane is a slot the host declares.

| | Requirement | State | Door / proposal |
|---|---|---|---|
| R6.1 | A host can seed globals into the extension's Lua state and keep a handle to call into it from Rust after the runner owns the extension | ✓ | `LuaExtension::lua()`; `mlua::Lua: Clone` (a handle), so kawoosh keeps a clone for calling `kawoosh.command` handlers from the keymap. M7 |
| R6.2 | A slot the host declares mid-view, with per-frame parameters, filled in place under a key namespace of its own | ✓ | `ui.slot_with(name, &Value)` (ADR 0014). M7 |
| R6.3 | **A slot name not known when the extension loaded** — a view registered from `init.lua` at runtime, one slot per (view, pane) | ✓ | **K1, alpha.13**: `slots = { "*" }` (`kui_core::ANY_SLOT`) matches every name under the namespace, `root` included when declared, no `unknown-slot`. M7 |
| R6.4 | **An event knows which fill it came from**, so the bootstrap routes by pane without stamping every payload | ✓ | **K2, alpha.13**: `UiEvent::slot: Option<Key>`; `ev.slot` in Lua is the full slot name the script was handed. M7 |
| R6.5 | An extension's `view` error is visible and non-fatal | ✓ | `extension-view-error` warning + the red text in the fill. |
| R6.6 | The Lua DSL covers what kawoosh's chrome and plugins need: containers, text with spans, `edit` for plugin fields, `fill`, theme roles, tokens by `$name`, `measure_text` | ✓ | kui-lua prelude + `env`; schema-generated docs (`props.md`). |
| R6.7 | Replies from a fill reach whoever declared the slot | ✓ | ADR 0014 d.6. Kawoosh's bootstrap replies with command invocations as data. |
| R6.8 | Host state reachable from inside a fill (a Lua view reading buffer lines while the host's `view(&mut self)` is on the stack) | n/a | kawoosh's constraint, not kui's: state behind `Rc<RefCell<_>>`, borrowed per call, never across `ui.slot_with`. Recorded so M7 does not rediscover it. |

## 7. Theme and tokens

| | Requirement | State | Door / proposal |
|---|---|---|---|
| R7.1 | Chrome colors as roles that follow the OS appearance and accent | ✓ | `ui.theme()`; `set_theme` / `set_accent` for a user palette (ADR 0019). M1 |
| R7.2 | App-owned names for syntax hues, declared once from Lua, referenced from Lua by `$name` and from Rust by name | ✓ | `tokens` global (per origin), `ui.token_color("keyword")` (ADR 0027). M5 |
| R7.3 | A token's light/dark halves picked by the appearance | ✓ | ADR 0027. |

## 8. Testing headless

| | Requirement | State | Door / proposal |
|---|---|---|---|
| R8.1 | The whole app — host view, extension fills, events — runs without a window | ✓ | `Core::frame_with(viewport, scale, &mut Extensions)`, `Ui::finish`; verified 2026-09-15 with a Lua fill. M1 |
| R8.2 | Keys, pointer, clock, paste driven as data | ✓ | `press`, `cursor`, `mouse`, `set_time`, `InputEvent::Text`. |
| R8.3 | The drawn frame readable as data, not pixels | ✓ | `set_inspect(true)` then `nodes()` (text, role, rect per node); `quads()` for geometry. |
| R8.4 | Misconfiguration is an assertable list | ✓ | `take_warnings()`, every code documented in `props.md`. |
| R8.5 | Frame-cost regressions are caught in kui, not discovered in kawoosh | ✓ | `bench-check.sh` gates seven frame benches at 10%; kawoosh adds a bench of its own rows in M5 against the `highlight` numbers. |

## 9. Not in the MVP, wanted after it

So the MVP does not build on them and nobody removes them from kui on
kawoosh's account — these are on the road, in roughly this order:

- **Audio** — the terminal bell and an editor's beep on a failed motion:
  `click_sound`-style one-shots off the `audio` feature. Cheap; the first
  thing after M8.
- **Images** — file preview in the oil buffer and a markdown/asset pane
  (`image` node, `update_image` for a stream), and the **kitty graphics
  protocol** in terminal panes: `alacritty_terminal` does not parse APC
  graphics, so this is a `term` change (an APC hook feeding registered
  images placed over `cells`) before it is a kui one. kui's side — an
  image placed at a cell rect, clipped to the grid — is `image` + a
  float, which exists. Built 2026-09-28 (roadmap step 56,
  kitty-graphics.md) with nothing new from kui: an image under the text
  is opened before the grid, which is then a float of its own.
- **Multi-window** (`ui.window`) — a detached pane on a second monitor.
- **`enter` / `exit` / keyframes** — beyond the split-ratio transition,
  once the chrome is settled enough to animate. *Built by alpha.16*
  (props.md's `enter`, `exit`, `keyframes`, `slide`, and `Ui::reveal`
  beside them); the first consumer is the scrolling tab
  ([scrolling-tab.md](scrolling-tab.md)).
- **The `edit` widget** for plugin-authored fields inside Lua views only;
  the editor pane never uses it.
- Not on the road: the Node binding, the C runner (`kui_run`). Accessibility
  is not required but comes free from the `Role::Line` rows and is kept.

## 10. The ask

K1–K4 shipped in alpha.13 (2026-09-15). What is left is kui's own wish
list, unchanged in priority: window position (M8's session restore), the
middle button — OSC 8 turned out kawoosh's own (R4.6), and so did §9's
kitty graphics, `image` and a float being enough.

Everything else kawoosh needs, alpha.13 has, and the doors are named
above so that a bump that moves one is a search in this file.
