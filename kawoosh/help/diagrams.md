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
| `box ID "LABEL" at X,Y size WxH shape S color C` | a box; only its id is needed (`at 0,0`, `size 8x3` otherwise) |
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
- **Double-click** a thing to edit its label.
- **The wheel** pans; {{mac:⌘}}{{pc:Ctrl}} and the wheel zooms where the
  pointer is; the middle button drags the view.
- **Right-click** a thing for what you can do to it — its shape, its
  colour, an arrow's heads, a box added after it — or the empty canvas
  for a box, a text or a paste right there.
- **The toolbar** along the top has the tools and the common changes,
  each with its key; `T` hides it and shows it again (the
  `diagram.toolbar` setting keeps it hidden).

## The keys

Everything can be done from the keyboard. A diagram from nothing: `n`
makes a box and asks its label, `o` adds the next one to the right joined
by an arrow, `O` one below; `hjkl` walk from box to box; `a` on a box
starts an arrow from it, `hjkl` to the box it goes to and `<CR>` joins
them.

| key | what it does |
|---|---|
| `n` | a new box, beside the one selected or in the middle, its label asked |
| `o` · `O` | a new box to the right · below the one selected, an arrow to it |
| `h` `j` `k` `l`, the arrows | select the nearest box or text that way |
| `<C-n>` · `<C-p>` | select the next · previous thing in the file, arrows and groups too |
| `H` `J` `K` `L`, shift and the arrows | move the selection a unit (`5L` five) |
| `<A-h>` `<A-l>` · `<A-k>` `<A-j>` | narrower, wider · shorter, taller |
| `a` | with a box selected, an arrow from it: `hjkl` to the other box, `<CR>` joins, `<Esc>` lets go; otherwise the arrow tool for the mouse |
| `b` · `t` · `v` | the box tool (drag one, or click for an 8×3), the text tool, back to select |
| `<Esc>` | let go of the tool, then the selection |
| `<CR>` `i` | edit the label |
| `x` `<Del>` | delete; a box takes its arrows with it |
| `s` | the next shape |
| `r` · `-` · `R` | an arrow straight or elbowed · dashed or solid · the other way round |
| `c` | the next colour |
| `C` | connect the selected boxes, in the order you selected them |
| `G` | group the selection |
| `y` · `p` · `D` | copy · paste · duplicate |
| `<C-a>` | select everything |
| `u` · `<C-r>` | undo · redo |
| `=` · `0` · `+` `_` · `zz` | fit everything in view · 100% · zoom in, out · centre on the selection |
| `T` | the toolbar |
| `<Tab>` | the text |

`:`, `<leader>` and `<C-w>` work as everywhere else. Every key is a
command you can also type: `:diagram shape cylinder`, `:diagram color
blue`, `:diagram label Payments`, `:diagram heads both`, `:diagram add
down`, `:diagram box 10 4`, `:diagram zoom 2`.

## Sharing it

`:diagram export` writes the diagram as an Excalidraw file beside it
(`services.excalidraw`), or `:diagram export PATH`. Open it at
excalidraw.com or in any app that reads the format: the shapes keep their
labels, the arrows stay attached to their boxes, and it is drawn sharp
in a monospace font rather than hand-drawn.

## Settings

| setting | default | what it does |
|---|---|---|
| `diagram.render` | `true` | open `.kdraw` files as their diagram; `false` opens them as text |
| `diagram.toolbar` | `true` | show the toolbar over the diagram |
