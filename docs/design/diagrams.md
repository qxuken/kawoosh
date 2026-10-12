# Diagrams: a `.kdraw` file, drawn as a canvas

Status: decided and built 2026-10-13. Asked as "something like
excalidraw … with snapping, engineering style draw canvas, backed by
some data file … services, state machines, breakdown some components,
maybe export to excalidraw so i can share it". Two calls were the
user's: the file is a text format of kawoosh's own, a line a shape, and
the first round is the core canvas (Decision 8 says what waits). It is
Rust, not a Lua view: a canvas draws every frame of a drag, and the
policy is that what a profile would move to Rust starts there.

## What there is

- **A buffer drawn another way** is a known shape: markdown is "a way
  of drawing the buffer, not a buffer kind" (markdown.md Decision 1) —
  the same `Buffer`, `:w`, undo tree, title bar dot and session, a
  setting to see the source.
- **Panes without a view's keys** take them in pane mode
  (`listing.rs`): `:`, `<leader>`, `<C-w>` and the shift chords fall
  through to normal mode's maps, everything else is the pane's, gated
  by a fact.
- **kui draws what a canvas needs**: boxes with a border and a radius
  floated anywhere (`FloatConfig::parent().offset`), `line` /
  `polyline` with a `Stroke` (`.dash`), `polygon` (eight points),
  `path` (arcs and curves), text in a floated box; all in the parent's
  box space, all hit by shape. A press on an `on_drag` node is a drag's
  `start` at once, a release its `end`.
- **`Editor::apply_edits`** puts byte-range edits in a buffer as one
  undo node, the views' selections carried through.

## Decisions

### 1. The file is lines, one item each, on a grid

```
# services.kdraw
box api "API gateway" at 2,2 size 8x3 shape round
box db  "Postgres"    at 16,2 size 6x3 shape cylinder color blue
text "edge of the VPC" at 2,9
arrow api -> db "reads" elbow
arrow api -- db dashed
group backend "Backend" { db }
```

- `box ID ["LABEL"] [at X,Y] [size WxH] [shape S] [color C]`; shapes
  `rect` (the default), `round`, `ellipse`, `diamond`, `cylinder`, and
  for state machines `dot` (a start state) and `final` (a ring round a
  dot). Without `size`, 8×3 (a dot 1×1).
- `text "LABEL" at X,Y [color C]`: free text, its top left at the point.
- `arrow A -> B ["LABEL"] [elbow] [dashed] [color C]`: `->` a head at
  B, `<-` at A, `<->` both, `--` neither; `elbow` is an orthogonal
  route, straight by default. `A -> A` is a loop.
- `group ID ["LABEL"] { A B … }`: a frame round its members, a member
  another group too. It has no place of its own: it is where they are.
- Colours: `red orange yellow green teal blue purple gray`.
- `#` starts a comment line; blank lines are kept. A label is quoted,
  `\"`, `\\` and `\n` inside it.

Coordinates and sizes are whole grid units (20 px at zoom 1): the grid
*is* the snap — a box cannot be half a unit off, and nothing in the
file says where a point fell between two. The engineering look follows
from it: straight edges, aligned boxes, arrows that leave a side's
middle.

*Beat:* JSON (noisy to diff and to write by hand), `.excalidraw` as the
file itself (its seeds and version nonces churn every save, and it has
no grid), free pixel coordinates with a snap setting (a hand-written
file would mix the two).

### 2. A buffer drawn as a canvas, not a pane kind

A `.kdraw` buffer in an editor pane is drawn by `canvas.rs` while
`diagram.render` holds (true by default), the way markdown's rows are.
`<Tab>` on the canvas, or `:diagram toggle`, shows the source and back
— for that pane alone, not the setting: the canvas in one pane and its
text in a split beside it is the way to write a diagram by hand and see
it. An edit in either is the other's at the next frame — the canvas
parses the text again when the buffer's version moved.

So `:w`, `:e!`, the title bar's dot, `u` and `<C-r>`, the session and
the file watcher are the buffer's, unchanged.

*Beat:* `Content::Canvas` with its own file handling — a second save
path, a second undo, a second dirty flag.

### 3. The canvas edits lines, and only the lines it changed

A gesture ends in edits on the text: a moved box's line written again,
a deleted one's taken out, a new one appended — `apply_edits`, one undo
node a gesture. A line nothing touched stays byte for byte as it was,
its comments and its alignment with it; a line written again is the
canonical form (`write.rs`). While the pointer is down nothing is
written: the canvas draws the gesture over the last parsed text, and
the release writes it.

### 4. The keys are the canvas's, in pane mode

The canvas's view takes its keys in pane mode while it is drawn, gated
by the `diagram` fact, so a stray `x` never edits text nobody can see,
and `:` `<leader>` `<C-w>` still reach normal mode's maps:

The keys reach everything the mouse does (asked 2026-10-13: "keyboard
navigation (making box without mouse)"): `hjkl` *walk*, not nudge — a
keyboard user moves between things far more than they move things — and
the capitals move.

| key | |
|---|---|
| `n` · `o` · `O` | a box (beside the selected one, else in the middle) · one to its right · below it, joined by an arrow; the label asked each time |
| `h` `j` `k` `l`, arrows | the nearest box or text that way, by its middle, the straight way counting double the sideways |
| `<C-hjkl>` | the same, the arrows among them, each where its label is |
| `<C-n>` `<C-p>` | the next, previous thing in the file — the way to a group too |
| `<C-o>` | out: the group round the selection, then the one round that |
| `V` | extend: those keys add to the selection, `V` or `<Esc>` stop |
| `H` `J` `K` `L`, shift-arrows | move a unit, a count for more |
| `<A-hjkl>` | narrower, taller, shorter, wider |
| `a` | on a box: an arrow from it, `hjkl` to the other, `<CR>` joins, `<Esc>` lets go; else the arrow tool |
| `v` `<Esc>` · `b` · `t` | tools: select, box, text — the box and text tools with a grid cursor, `hjkl` (`HJKL` five) and `<CR>`: a text there, a box's two corners (twice in place, an 8×3) |
| `<CR>` `i` | the selection's label, in the prompt (`:diagram label TEXT`) |
| `x` `<Del>` | delete; a box takes its arrows with it |
| `s` · `r` · `-` · `R` · `c` | next shape · elbowed or straight again, a box's arrows on a box · dashed, an arrow or a box's outline · reversed · next colour |
| `C` | connect the selected boxes in the order selected |
| `G` | group the selection (`g` is a prefix in pane mode: `gt`, `gg`); a group round just those already is selected instead of a second |
| `y` · `p` · `D` | copy · paste · duplicate |
| `<C-a>` | select everything |
| `u` `<C-r>` | the buffer's undo, redo |
| `=` · `0` · `+` `_` · `zz` | fit · zoom 1 · zoom in, out · centre on the selection |
| `<A-/>` | the toolbar: the pane's legend (Decision 8) |
| `<Tab>` | the source |

A box made from the keys lands where it covers nothing: moved down (or
right, for one made below) until it is clear. The view follows the
selection a key moved to, the least that shows it with a unit of air.

The mouse: a press on an item selects it (shift adds), a drag moves the
selection, a corner of one selected box resizes it, a drag on nothing
draws a marquee, a double click edits a label, the wheel pans, the
wheel with ⌘ or Ctrl zooms at the pointer (the window's zoom is the
pane's under the pointer already, pane-settings.md Decision 5), the
middle button drags the view. With the box tool a drag draws the box,
a click puts an 8×3 one; with the arrow tool a drag from a box to a
box connects them, and so does a drag from a box with Alt held.

### 5. Arrows are routed, not drawn

An arrow names its boxes, never points: it leaves A where the line
between the centres crosses A's outline and arrives where it crosses
B's — the outline's own, an ellipse's or a diamond's. `elbow` goes out
of the side facing B, turns once at the middle of the gap and comes in
from the side facing A. Two arrows between the same boxes (a state
machine's `a -> b` and `b -> a`) run side by side, three quarters of a
unit apart. A loop goes up out of the box's top and back down into it,
clear of the sides the other arrows use.
A box moved, its arrows follow, and nothing in the file changes for
them.

### 6. Export: `:diagram export`

`:diagram export [PATH]` opens the diagram as Excalidraw's JSON in a pane
beside — the buffer of `PATH`, else of the file's name with
`.excalidraw`, its text the export, unsaved: `:w` keeps it, `:q` asks
(asked 2026-10-13: written at once, it overwrote a file nobody had looked
at). The JSON: rectangles (rounded
for `round`, `cylinder`), ellipses (a `dot` filled, a `final` a ring
and a dot), diamonds, each label a text bound to its shape, each arrow
an arrow bound at both ends with its label bound to it, a group a
dashed rectangle with its label above. Roughness 0 and the monospace
family, so it opens looking engineered, not sketched. Ids and seeds are
derived from the file's ids, so an export of the same file is the same
JSON.

### 7. The model is a crate of its own

`diagram/` (`kawoosh-diagram`): the parser and writer, the geometry
(rectangles, outlines, routes, label anchors), hit tests, the edits a
gesture makes, and the export — no kui in it, tested in milliseconds.
`kawoosh/src/canvas.rs` is the pane: the view state (pan, zoom,
selection, tool, gesture), the drawing, the events and the commands.

### 8. A right-click menu and a toolbar, both commands

Asked 2026-10-13: "maybe right click to do some actions and small
toolbar, at least toggleable". The canvas claims the secondary button
(`on_button`, as an editor's rows do) so the press says where it was:
what is under it is selected unless it is in the selection already, and
kui's menu opens with the rows for it — a box's label, shape and colour
(submenus, the current one checked), a box after it, below it, an arrow
from it; an arrow's elbow, dash, heads, reverse; connect, group,
ungroup; duplicate, copy, delete — or, over nothing, a box, a text or a
paste *there* (`diagram box X Y`, `diagram text X Y`, `diagram paste X
Y`). Each row runs a command line on the pane (menus.md's thesis), its
key beside it; a capital is spelled `Shift+O`, since a menu shows every
letter as a capital.

The toolbar is a strip over the drawing: the tools and what makes boxes
always, then what changes the selection *for what is selected* — a
box's shape, an arrow's heads and reverse, connect for two boxes,
ungroup for a group (asked 2026-10-13: `elbow` and `dashed` read as
nothing until an arrow was selected) — then the view; each a button
running its command, keyed by it, its key beside it, the tool in hand
lit. A strip under the drawing says the tool, the zoom, what is
selected (`2 boxes · 1 arrow`) and what the keys do now. The changes
reach a box too: `r` on a box elbows its arrows, `-` dashes its own
outline (`box a dashed`, a new word on its line). It is the pane's key legend (icons.md Decision 6, asked 2026-10-13: `use
panels convention <A-/>`): `<A-/>`, or `⌥/ keys` in the title bar,
shows and hides it, `keys.legend` says how it starts — hidden, as every
pane's legend. Every button is always there in its place, the ones that
do not fit the selection dimmed and deaf: a toolbar that grew and shrank
with the selection moved its buttons, and the drawing under them. The
drawing does not move as the strip comes and goes either: its origin is
measured from the pane's inside under the title bar, not from the
canvas's own box below the strip. A drag that starts on it moves nothing.

*Beat:* a palette of shapes to drag from (the keys and the menu reach
every shape already, and a drag-from-palette is a second way to make a
box with its own placement rules).

### 9. Every node keyed; a label inside its box

Reported 2026-10-13: "while i was panning label disappeared … only to
an A labeled box". kui keeps its float layers' stack from frame to frame
*by key*, a key it has not seen going on top
(`emit::stack_floats`). The canvas had drawn its grid lines and its
items under keys given by place, so a pan that drew one grid line more
renumbered everything after it; the box behind `a` came out under a
fresh key, on top — over its label, a float of its own beside it.

Then (the same day, a box over the title bar and a group's wash over
the pane beside): a float escapes its parent's clip unless it says
`clipped`, and the toolbar, a float over the drawing, was under any
box kui saw after it. So the drawing's floats are all `clipped`, and the
toolbar and the status are strips in the flow above and below the
drawing, which no float in it can reach; the drawing's own box is read
from its `on_layout`, not worked out from the pane's.

So: every node the canvas draws has a key of its own — a grid line by
its place on the grid (`gx12`), a thing by its entry (`n3`, `e7`), a
part of it after that (`n3.label`) — and a rectangle's label is a child
of the box's own float, drawn after its fill whatever the stack does. A
path, a line and a polygon open no layer (they paint in their parent's,
in order); a path shape's label is a float over it. A test pans the view
back and forth and fails if any opaque node is stacked over a label
that is not in it.

### 10. Not in this round

Free-floating arrows (an end that is a point, not a box), ports a user
picks, curved arrows, rotation, images, frames that clip, align and
distribute, a grid size setting, auto-layout, import from Excalidraw,
an SVG or PNG export. Each is a line format addition or a command away;
none changes a decision above.
