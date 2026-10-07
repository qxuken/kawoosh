# Icons and key caps: one set, drawn, reused everywhere

Status: decided 2026-10-02. Asked, after trying the app: "we probably
should crate set of an icons to display keys, close buttons, other
elements drawn by pictures. and reuse it everywhere". Each decision
keeps the alternative it beat.

## What there is

Pictures are drawn in two ways today, each place its own.

**Glyphs**, a character in a text node, wherever the font puts it. A
glyph sits on its face's metrics: `×` on the math axis, below the line's
middle and a different amount per face; `⏎` `⌥` `⇧` `⌘` are in few
monospaced faces, so they come from whatever fallback has them, at its
size and on its baseline. That is how the tab's close button came to
sit low (a255aa1, "glyph icons sit low") and why the markdown task
boxes were swapped for the bundled Nerd Font's (step 48).

**Vectors**, kui's `line` and `polygon` nodes: the tab's close × since
a255aa1 (two `ui.line` strokes about a box's centre) and the VCS log's
graph (`graph.rs`). These sit where they are told.

Every place that draws a picture-like element:

| where | what | how |
|---|---|---|
| `chrome.rs` tab block | close button | `ui.line` × in a square |
| `chrome.rs` tab title | modified `●` | glyph in the label string |
| `panes.rs` pane title | modified `●` | glyph |
| `panes.rs` terminal's "lines below" | `↓ … · ⇧End` | glyphs in one text |
| `panes.rs` strip module | `▯▮▯` columns | glyphs (status line text) |
| `breadcrumbs.rs` | `›` between crumbs, `…` | glyphs, measured |
| `inspector.rs` | `▸` `▾` folds | glyphs |
| `notify.rs` corner | `✓` progress done, `…` running | glyphs |
| `whichkey.rs` | each key: `SPC`, `C-w`, `RET` | words in accent text |
| `memory.rs` foot | `⏎ open · y recall · … · ⇥ view · q close` | one text |
| `memory.rs` jumps | `‹ 2` `› 1` steps; `⏎` for an empty first line | text data |
| `undo.rs` strip | `⏎ restore · u ⌃r g- g+ step · q close` | one text |
| `markdown.rs` | task boxes, bullets `•` | Nerd Font glyphs folded over the source |
| `graph.rs` | the VCS graph's lanes and dots | `ui.line`, `ui.polygon` |
| `theme_check.rs` | `✓` `✗` per pair | report text |
| `search.lua` bar | stage `×`, `›` between stages, `⌥/ keys`, the legend `⏎ search  ⇥ next field …` | glyphs, one paragraph of spans |
| `grammars.lua` foot | `jk walk · / filters · ⏎ installs · …` | one text |
| `fonts.lua`, `themes.lua`, `settings.lua` feet | the same kind of legend | one text each |
| `settings.lua` | `⌥m` on each filter chip; `⏎ keeps · esc drops` | glyph text |
| `launcher.lua` | a row's letter or `⏎` hint at its end | text |
| `du.lua` | a marked entry's `●` | glyph |
| `status.lua` | `● 3` errors in the title bar | segment text |
| `picker.lua`, `vcs.lua`, `lists.lua`, `dir.lua` | `…`, `⋯`, `→`, `w × h` | prose |

The last row is typography, not pictures: an ellipsis, an arrow inside
a sentence (`rename a → b`), a multiplication sign. They stay text.

## What kui offers

From kui alpha.31's `Ui` and kui-lua's prelude:

- `line` / `polyline`: a round-capped stroke through points, joins
  round, `Stroke::curve` for a smooth curve through them
  (Catmull-Rom). It never takes part in layout: it floats in its
  parent's box space, sized to its own bounding box, so a picture is a
  box of a fixed size holding its strokes.
- `polygon`: a filled outline of up to eight points, concave allowed.
- A box's `radius`: a disc is a square box rounded by half its side.
- `image`: a registered RGBA image (no SVG: the host decodes).
- `fragment`: a box a WGSL function paints.

What it does not have, none of it needed for this round:

- **A path node** (arcs, Béziers, an outline stroked with miter
  joins). Circles here are curves through points; an outline is a
  closed polyline, its corners rounded by the caps.
- **The face's metrics to the app.** `measure_text` answers a width
  and a height; there is no ascent, descent, x-height or cap height,
  so an icon cannot be centred on the x-height of the text beside it.
  It is centred on the line box instead (Decision 2). A
  `measure_text` that also returned `x_height` and `baseline` would be
  the kui round that makes it exact.
- **An inline box in a paragraph.** A `rich_text` is spans of text;
  an icon in the middle of a buffer's line (the markdown task box) has
  nowhere to go but a glyph. Not yet (below).

## Decisions

### 1. Icons are drawn, not set in a font

Every picture-like element is a named icon drawn with kui's vectors —
strokes, fills, a centred dot — in a box of its own, in the palette's
colour.

*Beat:* glyphs from the fonts kawoosh ships. The bundled Nerd Font
symbols have most of the shapes, but a glyph still sits on its face's
baseline and the face's line, and a fallback's metrics are not the
text's: the × that sat low did so in a shipped face. *Beat:* SVG files
rasterised to images — kui has no SVG; a rasteriser (resvg) would be a
dependency to draw a dozen strokes, and an image is re-made for every
size, scale and colour a theme gives it. *Beat:* a WGSL fragment per
icon — a shader for a chevron.

### 2. An icon is data: parts in a unit box, sized to its text

An icon is a list of parts in a box of side 1, `y` down:

| part | is | drawn as |
|---|---|---|
| `{ stroke = { {x, y}, … }, width = 0.1, curve = false }` | a line through the points | `line` |
| `{ fill = { {x, y}, … } }` | a filled outline, up to eight points | `polygon` |
| `{ dot = r }` | a disc of radius `r`, centred | a rounded box |
| `{ glyph = "…", scale = 1 }` | a character, centred in the box | `text` |

The box is a square as wide as the text beside it is large (its font
size), centred across the row it is in — on the line's middle, since
kui gives no x-height (above). A stroke's `width` is a share of the
side, 1 px at least; colour is the caller's: the colour of the text it
stands for (a dim key, the accent of a done mark). `glyph` is the way
out for a user who wants a Nerd Font's picture, and the place where a
face's metrics are theirs again.

One function resolves a shape at a size into points; the Rust chrome
draws them with `ui.line`/`ui.polygon`, and the same points go to Lua
as the node tree `kawoosh.icon` returns. So an icon drawn from a
plugin is the one drawn by the chrome, pixel for pixel.

*Beat:* each icon a Rust function drawing itself — what the tab's
close button was. Fine for one, but a Lua view could not have it and a
user could not change it.

### 3. The set, by shape

| name | is | stands for |
|---|---|---|
| `close` | × | close a tab, a stage, a toast |
| `check` | ✓ | done |
| `dot` | ● | modified, marked |
| `chevron-right` `-left` `-up` `-down` | › ‹ ⌃ ⌄ | a step in a path, a direction |
| `folded` `unfolded` | ▸ ▾ | a tree's branch shut and open |
| `arrow-up` `-down` `-left` `-right` | ↑ ↓ ← → | the arrow keys, "more below" |
| `return` | ⏎ | `<CR>` |
| `tab` | ⇥ | `<Tab>` |
| `backspace` `delete` | ⌫ ⌦ | `<BS>` `<Del>` |
| `ctrl` `alt` `shift` `cmd` | ⌃ ⌥ ⇧ ⌘ | the modifiers |
| `missing` | an empty square | a name the set does not have |

Named by shape, not by every use: a user's `dot` is every dot. A name
the set does not have draws `missing` rather than nothing, so a typo
is seen.

### 4. Keys are drawn as caps, from the keymap's own notation

One renderer turns a key notation — the keymap's, `<C-w>j`, `<leader>f`,
`<CR>` — into caps: one outlined, rounded cap a key, its modifiers
inside it before the key. The modifiers and the named keys are icons
(`<C-S-j>` is ⌃⇧j, `<CR>` the `return` icon, `<Up>` `arrow-up`);
`<Esc>` reads `esc`, `<Space>` and the leader `spc`, `<Home>` `home`;
a letter keeps its case, since `j` and `J` are two keys, and a chord's
capital is its shift spelled out (which-key's rule: `<C-H>` is ⌃⇧h).
The cap is as tall as its text's line and pads only sideways, so a
row of text with a cap in it is no taller than without.

A **legend** is data: `{ { "jk", "walk" }, { "/", "filters" }, {
"<CR>", "installs" } }`, each item its keys as caps and its words after,
wrapped between items and never inside one — what the search legend's
no-break spaces did by hand.

The Rust half (`icons::keys`, `icons::legend_items`) draws the Lua
half too: `ctx.keys` and `ctx.legend` are fills the engine draws with
them (lua-boundary.md Decision 9), so a cap is one drawing wherever it
is.

*Beat:* the symbols written into strings, as now (`⌥/`, `⌃⇧J`, `⏎`):
they are glyphs (Decision 1), and a legend in a string is a key map
copied by hand. *Beat:* words for everything (`C-w`, `RET`), the
which-key's: plain, but `RET` on a cap reads worse than the ⏎ the
legends already use.

The modifiers are the keyboard's own (2026-10-04: "We should render
proper keys for windows and linux hosts. probably just build time
feature host"): the Mac's symbols on a Mac, and on a PC, whose caps
print no ⌃ ⌥ ⌘, the words with the key in one text — `ctrl+shift+j`,
`alt+/`, `win+s` on Windows and `super+s` on Linux, the system's key
first as those systems write a chord. Which is the build's
(`icons::Host::HERE`, by `target_os`), not a setting; `caps_on` reads
a notation for any of the three, which is what the tests pin. The
named keys stay icons everywhere (`shift+` and the tab).

### 5. Lua: `kawoosh.icon`, `kawoosh.icons`, and a view's `ctx`

- `kawoosh.icon(name, { size =, color = })` returns the node: a box
  of `size` (13) holding the parts, the strokes in `color` (the
  foreground). Any other key of the options is the box's own (`key`,
  `on_click`, `label`, `hover_bg`…).
- `ctx.keys(notation, { size = })` and `ctx.legend(items, { size = })`
  draw caps and legends in a view's theme: the keys in `muted`, the
  words in `faint`, the caps outlined in `border`.
- `kawoosh.icons.NAME` reads a shape as the parts above;
  `kawoosh.icons.NAME = { … }` replaces it — for the chrome too, since
  the set is one, held in Rust and shared — and `= nil` puts the
  default back. `kawoosh.icon_names()` lists them. An override lasts
  until replaced; `init.lua` read again sets it again.

```lua
-- A heavier ×, and a Nerd Font check mark.
kawoosh.icons.close = {
  { stroke = { { 0.25, 0.25 }, { 0.75, 0.75 } }, width = 0.14 },
  { stroke = { { 0.25, 0.75 }, { 0.75, 0.25 } }, width = 0.14 },
}
kawoosh.icons.check = { { glyph = "\u{F012C}" } }
```

*Beat:* icons as settings (`icons.close = { … }` in `settings.lua`):
a shape is code-shaped data, nested lists of points, which the
settings pane could not edit as a value, and an icon is not a
preference one toggles.

### 6. A legend is compact until asked for, in every pane

Asked the same day: "add key legend compaction like search does on
`<A-/>`". The search bar's legend was the one that folded (search.md
Decision 11: hidden, a dim `⌥/ keys` at the end of its stages' row,
`<A-/>` flipping `search.legend` for the session). Compaction is the
legend's now, not the search's: every legend — `ctx.legend` in a Lua
view (grammars, fonts, themes, settings, search), `legends::legend`
in the chrome (memory, undo) — draws as one `⌥/ keys` until asked,
and whole as its items and a last `⌥/ hide keys`.

- **One key, everywhere: `<A-/>`**, the `legend` command — pane mode's
  for every pane that is not an editor's; the search bar's fields map
  it too, as they did. A click on the hint is the same. `?` without its
  shift, as the search had it. Not every field's: the resident pane
  view is a field too, so a `field` place would be every pane's and
  the command line's (`a_panes_places_are_not_its_fields`); a filter's
  field gives `<Esc>` first.
- **Per pane, for the session.** A flip is the pane's
  (`Legends`, keyed by pane, shared with Lua as `kawoosh._legend`):
  opening the grammars' keys leaves the themes pane's as they were,
  and a pane opened later starts as the setting says. A flip is kept
  over a change of the setting — it is what the user asked of that
  pane; a closed pane's is forgotten at the next flip.
- **`keys.legend = "compact" | "full"`** says how every legend starts,
  `compact` by default. `search.legend` is gone into it (the settings'
  moved table names it), since a boolean for one pane would be the
  exception the setting replaces.
- **Compact is the hint alone**, not the first few items: which items
  are first is a guess per pane, and a cut that moves with the pane's
  width reads as a different legend at each width. One hint is the
  search's, which was the reference.
- A legend that is a prompt's own — the settings pane's `⏎ keeps ·
  esc drops` beside a value being edited — is `full = true`: two keys
  that are the only way out are not folded. A view that puts the hint
  elsewhere passes `toggle = false` and draws `ctx.legend_toggle`
  where it wants it: the search bar keeps its hint at the end of the
  stages' row and its legend under the bar only while whole, as before.

*Beat:* one global flip (what `search.legend` was, a session value):
`<A-/>` in one pane would re-lay every pane that has a legend, the
search bar growing behind the pane one asked in. *Beat:* a setting per
pane (`search.legend`, `grammars.legend`…): a setting for each plugin
to declare for one behaviour.

### 7. The way to a legend is the pane's title bar

Asked the same day, of the grammars, fonts, themes, memory and
settings panes: the hint "kinda sticks too much. In search it is
integrated well. I think it either should be scrolled by or be placed
somewhere else." A compact legend was a row of its own, `⌥/ keys`
alone, in the head those panes keep over their list — a fixed row for
two words, under every scroll. The search's sits at the end of a row
it has anyway.

- **Placed somewhere else: the title bar's end.** Every pane has one,
  its right end is empty but for an editor's crumbs, and it is where
  the pane already says what it is. `⌥/ keys` there, `⌥/ hide keys`
  while the legend is whole; a click flips it, as `<A-/>` does. The
  pane's rows carry the legend only while it is whole — where the
  view puts it, since which row suits is the view's — and nothing
  while it is compact.
- **A view says it has one; the chrome draws the way.** `ctx.legend`
  tells the pane's title bar (`kawoosh._legend_drawn`), and returns
  `nil` while compact; the memory and undo panes' `legends::legend`
  the same. The title bar is drawn before the pane's rows, so it reads
  the panes that drew a legend this frame or the last
  (`Legends::roll`, `declare`, `has`), and a frame in which one began
  or stopped asks for one more (`settled`, "legend hint"). A pane with
  no legend has no hint; `full = true` (a prompt's two keys) and
  `toggle = false` (a view drawing its own way, the search bar's) say
  nothing to the title bar.
- **The search bar keeps its own**, at the end of its stages' row: the
  integration that was the reference, in a header that is the bar.

*Beat:* scrolled by, with the list. A pane's list is a virtual one
(the fonts', a `uniform_list`) or a scroller of the view's own, and the
head is outside it; the hint would have to move into each list as its
first row, and a hint scrolled away is a hint lost — the key works
wherever, but the way to learn it would be at the top of a list one
had scrolled past. *Beat:* the status line, which is the window's and
says nothing of the pane under it.

## What moves now

The first round: the tab's close (`close`), the search stages'
close and `›` (`chevron-right`), the inspector's folds (`folded`
`unfolded`), the breadcrumbs' `›`, the corner's done mark (`check`),
the modified dot of the pane title, of the tab label kawoosh writes
(a `kawoosh.tab_title` hook still gets it as text in `title`, since
its label is a string) and `:du`'s marks (`dot`), the terminal's
"lines below" (`arrow-down` and a `<S-End>` cap), the which-key's
keys, and the legends and inline key hints — search, grammars,
fonts, themes, settings (its filter chips' `<A-m>` too), the
launcher's letters (a launcher item's `hint` is a key notation now:
`"<CR>"`), memory, undo.

Not icons: the VCS log's graph (`graph.rs`) draws data — lanes and
commits — with the same vectors, at a geometry of its own.

## Not yet

- **The markdown task boxes and bullets.** They are folded over the
  source in a buffer's line, which is a paragraph of spans; an icon
  there needs kui to hold a box inline in a paragraph.
- **The x-height.** Centred on the line box until kui's
  `measure_text` gives a face's x-height and baseline.
- **The status line and the title bar's segments** (`● 3`, `▯▮▯`):
  a `kawoosh.status` segment is text; an `icon =` field on a segment
  would carry one.
- **Text data that is shown elsewhere too**: the memory's jump steps
  (`‹ 2`), its `⏎` for an empty line, `theme_check`'s report, the
  message line (`⏎ opens this one`) — strings, not drawings.
- **Words on Linux and Windows** for the modifiers (`Ctrl`, `Alt`).
- **A spinner** for "running" (the corner's `…`, `searching…`).

## Built

2026-10-02, as decided. `kawoosh/src/icons.rs`: the shapes
(`default_shape`, `NAMES`), the set the user's shapes go over
(`Icons`, shared as `Kawoosh::icons`, its `fg` the frame's), `resolve`
into what is drawn, `icon` / `draw` / `icon_box` for the chrome;
`caps` reading a notation through the keymap's `parse_notation`,
`keys`, `keys_width` and `legend_items`, `CAP` the measures; and
`lua_door`: `kawoosh.icon`, the `kawoosh.icons` table (a metatable
over the set), `kawoosh.icon_names`. boot.lua's `ctx.icon`, and
`ctx.keys`, `ctx.legend` as fills `fields.rs` draws (lua-boundary.md
Decision 9).
The which-key keeps its words in the title and folds a numbered run
as the first key's caps and `…9`. Tests: `icons.rs`'s unit tests
(every shape centred in its box, notations as caps, an override and
its clearing); `kawoosh/tests/icons.rs` — the tab's ×
centred in its button and the button in its tab (red with the ×
moved down a tenth: 1.3 px off), a user's shape in the chrome until
cleared and a bad one refused, a Lua icon stroke for stroke the
chrome's, caps no taller than their line with their icons in their
middle and a legend wrapping between items, the panes' legends as
caps, the which-key's and the undo panel's keys as caps.

Decision 6, 2026-10-02: `kawoosh/src/legends.rs` (`Legends`, the
`legend` command, `legend` / `toggle` for the chrome,
`kawoosh._legend`), boot.lua's `ctx.legend` (`full =`, `toggle =`),
`ctx.legend_toggle`, `ctx.legend_full`, the click caught in
`on_event`; `keys.legend`; `<A-/>` in pane mode and the search bar's fields.
Tests: `kawoosh/tests/legends.rs` — a pane's legend one hint until
`<A-/>` opens it and again closes it (a click too, another pane's its
own), the undo pane's the same, `keys.legend = "full"` starting them
whole; the search panel's legend test on the shared state;
`search.legend` named as moved.

Decision 7, 2026-10-02: `Legends::roll` / `declare` / `has` /
`settled`, `legends::legend` declaring and drawing items alone,
`legends::toggle` at the title bar's end (`render_pane`, the name's
room less the hint's), `kawoosh._legend_drawn`, boot.lua's
`ctx.legend` returning `nil` while compact. Tests:
`the_way_to_a_legend_is_in_the_panes_title_bar` (the six panes: the
hint in the title bar and none in the rows, none on the editor, a click
on it opening and closing), `a_title_bar_reads_this_frames_legends_and_the_last`.

