# Diagrams

A file ending in `.kdraw` opens as a diagram: boxes, arrows between them,
free text and groups, on a grid. Draw services, state machines, a
component broken into its parts. The file stays plain text, a line for
each thing, so it reads in a diff and you can write it by hand.
`:diagram new` starts one (`diagram.kdraw` here, or `:diagram new
services` for `services.kdraw`); `:e` on any `.kdraw` file does too.

`<Tab>` on the diagram shows its text; `:diagram toggle` goes back. You
can also have the text open in a split beside the diagram: each follows
the other as you edit. `:w` saves, `u` and `<C-r>` undo and redo — it is
the file's own undo, so an edit in the text and a drag on the diagram are
steps of the same history.

## The file

```
# services
box api "API gateway" at 2,2 size 8x3 shape round
box db  "Postgres"    at 16,2 shape cylinder color blue
text "edge of the VPC" at 2,9
arrow api -> db "reads" elbow
arrow api -- db dashed
group backend "Backend" { db }
```

| line | what it is |
|---|---|
| `box ID "LABEL" at X,Y size WxH shape S color C dashed align A` | a box; only its id is needed (`at 0,0`, `size 8x3` otherwise); `dashed` draws its outline dashed; `align left` or `align right` sets its label from the top against that side, for a contract or code (`center` otherwise) |
| `text "LABEL" at X,Y color C` | free text, its top left at the point |
| `arrow A -> B "LABEL" elbow dashed color C` | an arrow between two boxes; `->` a head at B, `<-` at A, `<->` both, `--` none |
| `group ID "LABEL" { A B }` | a frame round boxes (or other groups) |

- **Shapes**: `rect`, `round`, `ellipse`, `diamond`, `cylinder`, and for
  state machines `dot` (a start state) and `final` (an end state).
- **Colours**: `red`, `orange`, `yellow`, `green`, `teal`, `blue`,
  `purple`, `gray`.
- **Positions and sizes** are whole grid units (20 px at 100%), so
  everything snaps.
- **Arrows** follow their boxes: they leave and meet a box at its outline;
  `elbow` routes at right angles. `a -> a` is a loop, and two arrows
  between the same boxes run side by side.
- A line starting `#` is a comment. In a label, `\n` is a line break.

A line that does not read is listed over the diagram (line 4: no box
`ghost`); the rest still draws. Lines the diagram changes are written
again in a standard form; lines it does not touch stay as you wrote them.

## The mouse

- **Click** a thing to select it; **shift-click** adds to the selection.
  **Drag** on an empty spot to select what the rectangle holds.
- **Drag** the selection to move it; a group moves its boxes.
- **Drag a corner** of a selected box to resize it.
- **Alt-drag** from one box to another draws an arrow.
- **Double-click** a thing to type its label where it is; a click
  anywhere else on the diagram keeps what you typed.
- **The wheel** pans; {{mac:⌘}}{{pc:Ctrl}} and the wheel zooms where the
  pointer is; the middle button drags the view.
- **Right-click** a thing for what you can do to it — its shape, its
  colour, an arrow's heads, a box added after it — or the empty canvas
  for a box, a text or a paste right there.
- **The toolbar** above the diagram has the tools, what makes boxes, and
  the changes — lit when they fit what is selected, a box's shape, an
  arrow's heads — each with its key; the line under the diagram says what
  is selected. It is the pane's key legend: {{mac:`⌥/`}}{{pc:`<A-/>`}}, or
  `keys` in the title bar, shows and hides it, and `keys.legend = "full"`
  shows it from the start.

## Labels

`<CR>` or `i` on a box, an arrow, a text or a group — or a double-click —
types its label right where it is drawn, over as many lines as you like:
a contract, a schema, a few lines of code. It is the editor's own text:
insert mode to type, `<CR>` a new line (it starts at the indent of the one
before), `<Esc>` for normal mode and vim's keys over the lines. A box
grows to hold its label as you type; it never shrinks on its own.

| key | what it does |
|---|---|
| `<Esc>` in normal mode (`<Esc><Esc>` from insert), `<CR>` in normal mode | keep the label |
| {{mac:`⌘↩`}}{{pc:`<C-CR>`}} | keep it, from either mode |
| `<C-c>` | let it go: the label as it was |

The whole label is one step: `u` takes it back at once. A new text that
you leave with no words is taken out again. `A` on a box sets its label
from the top left, then the right, then the middle again.

## The keys

Everything can be done from the keyboard. A diagram from nothing: `n`
makes a box and asks its label, `o` adds the next one to the right joined
by an arrow, `O` one below; `hjkl` walk from box to box; `a` on a box
starts an arrow from it, `hjkl` to the box it goes to and `<CR>` joins
them. The box and text tools (`b`, `t`) put out a cursor on the grid:
`hjkl` move it (`HJKL` five), `<CR>` puts a text there, or a box's first
corner and then its second — `<CR>` twice where it stands for an 8×3.
`V` makes `hjkl` and `<C-n>` add to the selection instead of replacing it,
to connect, group or move several things.

| key | what it does |
|---|---|
| `n` | a new box, beside the one selected or in the middle, its label typed in it |
| `o` · `O` | a new box to the right · below the one selected, an arrow to it |
| `h` `j` `k` `l`, the arrows | select the nearest box or text that way |
| `<C-h>` `<C-j>` `<C-k>` `<C-l>` | the same, the arrows among them (by their label) |
| `<C-n>` · `<C-p>` | select the next · previous thing in the file, arrows and groups too |
| `<C-o>` | the group round the selection, then the one round that |
| `V` | extend: the keys above add to the selection; `V` or `<Esc>` stop |
| `H` `J` `K` `L`, shift and the arrows | move the selection a unit (`5L` five) |
| `<A-h>` `<A-l>` · `<A-k>` `<A-j>` | narrower, wider · shorter, taller |
| `a` | with a box selected, an arrow from it: `hjkl` to the other box, `<CR>` joins, `<Esc>` lets go; otherwise the arrow tool for the mouse |
| `b` · `t` · `v` | the box tool (drag one, or `hjkl` the cursor and `<CR>` its two corners), the text tool (click, or the cursor and `<CR>`), back to select |
| `<Esc>` | let go of the tool, then the selection |
| `<CR>` `i` | type the label where it is ([Labels](#labels)) |
| `x` `<Del>` | delete; a box takes its arrows with it |
| `s` | the next shape |
| `A` | a box's label against the left, the right, or in the middle |
| `r` | arrows elbowed or straight again — a box's arrows, on a box |
| `-` | dashed or solid: an arrow, or a box's outline |
| `R` | an arrow the other way round |
| `c` | the next colour |
| `C` | connect the selected boxes, in the order you selected them |
| `G` | group the selection (once: the same boxes again select the group they are in) |
| `y` · `p` · `D` | copy · paste · duplicate |
| `<C-a>` | select everything |
| `u` · `<C-r>` | undo · redo |
| `=` · `0` · `+` `_` · `zz` | fit everything in view · 100% · zoom in, out · centre on the selection |
| {{mac:`⌘=` `⌘-` · `⌘0`}}{{pc:`<C-=>` `<C-->` · `<C-0>`}} | zoom in, out · 100% — in a diagram these zoom the drawing, not the text size they change everywhere else |
| `g h` `g j` `g k` `g l` | pan the view left, down, up, right by a quarter of it (`3gl` three times as far) |
| `<Tab>` | the text |

`:`, `<leader>` and `<C-w>` work as everywhere else. Every key is a
command you can also type: `:diagram shape cylinder`, `:diagram color
blue`, `:diagram label Payments`, `:diagram align left`, `:diagram heads both`, `:diagram add
down`, `:diagram box 10 4`, `:diagram zoom 2`, `:diagram pan right`.

## Sharing it

`:diagram export` opens the diagram as an Excalidraw file in a pane beside
it — `services.excalidraw`, or `:diagram export PATH` — not written yet:
`:w` there saves it. Open it at
excalidraw.com or in any app that reads the format: the shapes keep their
labels, the arrows stay attached to their boxes, and it is drawn sharp
in a monospace font rather than hand-drawn.

## Settings

| setting | default | what it does |
|---|---|---|
| `diagram.render` | `true` | open `.kdraw` files as their diagram; `false` opens them as text |
