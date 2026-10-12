# Diagrams

A file ending in `.kdraw` opens as a diagram: boxes, arrows between them,
free text and groups, on a grid. Draw services, state machines, a
component broken into its parts. The file stays plain text, a line for
each thing, so it reads in a diff and you can write it by hand.
`:e services.kdraw` starts a new one.

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

## The keys

| key | what it does |
|---|---|
| `b` · `a` · `t` · `v` | the box tool (drag one, or click for an 8×3), the arrow tool, the text tool, back to select |
| `<Esc>` | let go of the tool, then the selection |
| `<CR>` `i` | edit the label |
| `x` `<Del>` | delete; a box takes its arrows with it |
| `h` `j` `k` `l`, the arrows | nudge a unit; `H` `J` `K` `L` five |
| `s` | the next shape |
| `r` · `-` | an arrow straight or elbowed · dashed or solid |
| `c` | the next colour |
| `C` | connect the selected boxes, in the order you selected them |
| `G` | group the selection |
| `y` · `p` · `D` | copy · paste · duplicate |
| `<C-a>` | select everything |
| `u` · `<C-r>` | undo · redo |
| `=` · `0` · `+` `_` | fit everything in view · 100% · zoom in, out |
| `<Tab>` | the text |

`:`, `<leader>` and `<C-w>` work as everywhere else. Every key is a
command you can also type: `:diagram shape cylinder`, `:diagram color
blue`, `:diagram label Payments`, `:diagram heads` (cycles an arrow's
heads), `:diagram zoom 2`.

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
