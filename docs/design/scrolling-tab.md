# The scrolling tab: a strip of columns beside the tree

Status: decided 2026-09-21 (roadmap step 9); **built 2026-09-22**
(roadmap step 11) as designed, with the departures listed under
"Built" at the end. Roadmap Decision 2 gave it its shape — a per-tab
layout kind beside the splitmux tree, not instead of it — and named a
kui ask, kui-requirements §9's `enter` / `exit` / keyframes. That ask
was built in the meantime (alpha.14 onwards: props.md's `enter`,
`exit`, `keyframes`, `slide`, and `Ui::reveal`), so no kui round came
before this one. Each decision keeps the alternative it beat.
Companion to [roadmap.md](roadmap.md)'s panes track, [keys.md](keys.md)'s
"Panes, tabs, the dock", and `kawoosh/src/layout.rs`, which is the
data this note adds a second kind to.

## The thesis

The tree is i3's and vim's: a tab is a binary tree of splits with a
ratio at every node (`Node::Split { dir, ratio, a, b }`), every new pane
takes its room from the pane it split, and past four panes on a laptop
the splits are slivers at 13 px. The habit that grew around it is
closing panes to make room — which is the wrong thing to be thinking
about while reading a `*references*` list beside the file it came from
beside the test beside the terminal.

niri's and PaperWM's answer is a strip: columns of a chosen width laid
left to right on a ribbon wider than the window, the viewport scrolled
so that the focused column shows. A new column pushes the ribbon, not
its neighbours; a column keeps the width it was given until told
otherwise; what is not in view is not gone, one `<C-w>l` away. Five
panes at a half each is a ribbon two and a half windows wide that reads
like a document scrolled sideways, and that is the kawoosh day above.

The tree stays. It is the right shape for two panes and a dock, for
`:split` under a terminal, and for everyone who has i3 in their hands;
the strip is a second kind a tab can be, chosen per tab, converted both
ways.

## Decisions

### 1. A kind per tab, and a column is a tree

```rust
pub struct Tab { pub layout: Kind, pub focused: PaneId }
pub enum Kind { Tree(Node), Scroll(Strip) }
pub struct Strip { pub columns: Vec<Column> }
pub struct Column { pub node: Node, pub width: Width }
pub enum Width { Third, Half, TwoThirds, Full, Ratio(f32) }
```

A column is a `Node` — one pane, or a stack of `SplitDir::V` splits
with their ratios — so everything the tree knows keeps working inside
a column unchanged: `render_node`, `Node::resize`, `split_beside`, the
divider drag, `panes()`, `contains()`. The strip owns only the
horizontal axis: the order of the columns and each one's width as a
fraction of the viewport (`Width::Ratio` after a drag, the named
presets otherwise; the default preset is `layout.column_width`,
`half`).

*Beat:* a scroll leaf inside the tree (a `Node::Scroll` holding panes
under a split) — the mixed case nobody asked for, which doubles every
path that walks the tree; and replacing the tree with the strip, which
throws away i3's habits, the session files, and twenty tests for a
layout that is worse for two panes.

### 2. The keys are the tree's keys, read on the strip's axis

Nothing new to learn: a key that means *beside* in the tree means it in
the strip.

| key | in a tree | in a strip |
|---|---|---|
| `<C-w>v` `:vsplit` | a split beside | a new column after the focused one, at the default width |
| `<C-w>s` `:split` | a split below | a split below, inside the column (a stack) |
| `<C-w>h` `<C-w>l` | the pane beside, by rect | the column beside, by index; the pane in it at the focused pane's row, else its top |
| `<C-w>j` `<C-w>k` | the pane below / above, by rect | the same, inside the column |
| `<C-w>H` `<C-w>L` | *(free)* | move the column one place left / right |
| `<A-S-h>` `<A-S-l>` | the pane narrower / wider by a twentieth | the column's width to the next preset down / up (niri's `switch-preset-column-width`); a `Ratio` snaps to the nearest first |
| `<A-S-j>` `<A-S-k>` | shorter / taller | the same, inside the column |
| closing the last pane of a column | — | the column goes, the focus to the column before it |

`:layout scroll` and `:layout tree` convert the current tab (Decision
5); `layout.default` (`tree` \| `scroll`) is what `<leader>tn` opens.
`<C-w>H` and `<C-w>L` are vim's "move to the far side", read as one
step; they are free today and stay unbound in a tree.

*Beat:* a key family of its own (`<leader>t…` for the strip) — a second
vocabulary for the same intentions.

### 3. The viewport follows the focus, and only the focus

The strip is one kui row with `scroll_x`: kui retains the offset,
draws the bar, and a trackpad's horizontal swipe scrolls it with no
code in kawoosh. On the frame the focus moves to another column, or a
column is inserted, kawoosh calls `Ui::reveal` on the focused column's
key and does nothing else to the offset — so a swipe is never fought,
and a `<C-w>l` always lands the column in view. `layout.scroll.center`
(`never` \| `always`; niri's `center-focused-column`) makes the focus
frame set `scroll_offset` to centre the column instead of revealing it.

The gap between columns is `layout.gap` px (the divider's width by
default) and is draggable like a divider: a drag turns the column's
width into `Width::Ratio`.

*Beat:* kawoosh owning the offset every frame (computing it from the
focused column and writing it) — which fights the swipe and rebuilds
what kui already retains, with the `rects` lag on top.

### 4. The strip moves, the tree does not

This is what Decision 2 wanted kui's animation for, and it is all
declared: a new column has `enter { dx: its own width }` and slides in
from the right; a closing one has `exit { opacity: 0 }` and fades where
it stood (kui replays it frozen and inert); columns carry `slide`, so
`<C-w>H` glides the column past its neighbour and a width change eases;
the `transition` is the one the split ratio already uses. The tree's
panes keep snapping — a split is a cut, not a motion.

*Beat:* keyframes (nothing here cycles), and animating the tree's
splits for symmetry (a ratio drag already eases; a split appearing has
nowhere to come from).

### 5. Conversion both ways, lossless where it can be

`:layout scroll` on a tree tab walks the tree: each arm of the
top-level `H` splits, left to right, becomes a column whose node is
that arm (its `V` splits stay a stack); an `H` split nested inside a
`V` arm cannot be a column's inside, so it is flattened — its leaves
become consecutive columns in reading order. Widths come from the
ratios, snapped to the nearest preset. `:layout tree` folds the columns
back into `H` splits, right-nested, the ratios from the widths; a
column's stack is kept as it is. Round-tripping a tree that had no
nested `H`-in-`V` gives the same tree.

Sessions: the `Tab` data gains `kind` and, for a strip, `columns` (a
`NodeData` and a width each); a file from before reads as a tree. The
dock is the window's, under both kinds, untouched.

### 6. Focus by index on the strip's axis, by rect inside a column

Today `<C-w>hjkl` are directional over `Layout.rects`, where each pane
was drawn last frame. In a strip the column beside may be off the
viewport, so the strip's axis moves by index (the column before or
after), and the row within it by rect as today. `rects` still holds
every drawn pane, off-screen or not, so the mouse and the resize keys
need nothing. Clicking a pane focuses it and reveals its column.

### Deliberately not

- **Workspaces.** niri's vertical stack of workspaces is what tabs are
  here.
- **A tabbed column** (niri's, one pane shown of several). A tab is a
  tab; a column shows its stack.
- **Floating panes.** Floats are kui's chrome (confirms, which-key, the
  picker's preview); a pane is never one.
- **The vertical ribbon** — rows of panes scrolling down. It is this
  design with the axis flipped, and nobody asked for it; if someone
  does, `Strip` gets an axis, not a second type.
- **A pane that is not rendered while off the viewport.** Measured
  first (Risks), then decided.

## Build order

One round, one commit:

1. `layout.rs`: `Kind`, `Strip`, `Column`, `Width`; insert, close,
   move, the width presets, both conversions; unit tests for each,
   including the round trip and the flattened nested split.
2. `panes.rs`: the strip rendered as a `scroll_x` row of keyed columns
   with `enter`, `exit`, `slide` and the gap; the reveal on the focus
   frame; the centre setting.
3. The commands read on the axis (`split`, `vsplit`, the moves, the
   resizes, close), `<C-w>H` `<C-w>L`, `:layout`, `layout.default`,
   `layout.column_width`, `layout.gap`, `layout.scroll.center`.
4. `session.rs`: the kind and the columns, an old file read as a tree.
5. keys.md's table above; the roadmap's item struck.

Tests, headless (`kawoosh/tests/layout.rs`): `:layout scroll`, three
`<C-w>v`, the fourth column focused and its rect inside the viewport
after a settled frame; `<C-w>h` three times revealing the first;
`<C-w>L` moving a column and the order in `Strip`; `<A-S-l>` stepping
the preset; closing a column's last pane; `:layout tree` and back; a
session saved and restored as a strip.

## Risks

- **`reveal` on a node declared this frame.** kui's howto says `reveal`
  finds nothing for a row nobody declared; a column inserted and
  revealed in the same frame may need the reveal on the frame after.
  The pane's follow-up frame (one more frame requested) is the answer
  if so; check on day one.
- **Twenty columns.** Every column is laid out every frame, and an
  editor pane emits its screenful of rows whether it is on the
  viewport or not. Measure a strip of twenty panes with the perf tab;
  if it costs, a column whose last-frame rect is off the viewport
  emits an empty box of its size instead of its rows — a change local
  to `render_pane`.
- **The swipe and the focus in one frame.** The focus wins (Decision
  3); a swipe that lands a click on another pane focuses it and
  reveals it, which is the expected thing.

## Built (2026-09-22)

`layout.rs`'s `Kind`, `Strip`, `Column`, `Width` and `Tab`'s methods
over both kinds (`node_of`, `split_of` in a `"2/ab"` grammar for a
strip, `ratio_mut`, `share_of`, `resize`, `to_scroll`, `to_tree`);
`panes.rs`'s `render_strip`; `:layout`, `:layout scroll`, `:layout
tree`, `column left` / `right` on `<C-w>H` / `<C-w>L`; the four
settings; the session's `kind` and `columns`; `kawoosh/tests/layout.rs`
(three tests) and seven unit tests in `layout.rs`. Where the build
departed from the text above, and what day one found:

- **A bare `:layout` flips the tab**, and `<leader>tl` is bound to it —
  one key to try the other kind and come back. The message after says
  what the tab is now and, for a strip, the three keys that matter.
- **A column carries a number of its own** (`Column::id`, counted apart
  from panes) for its drawn key, so the column keeps its slide, enter
  and exit whatever its panes do — a stack's top pane closing does not
  make a "new" column.
- **The entrance is a third of the width, with a fade**, not the whole
  width: kui hears no key for a sink outside the viewport (its hit
  regions are the clipped ones), so a column that started wholly off it
  would drop the keystroke typed during its 200ms slide. A third keeps
  it partly in view from the first frame. The strip's first frame — a
  conversion, a restore, a tab switched to — snaps: those columns are
  not arriving (`Kawoosh::strip_known`).
- **The reveal is asked again for sixteen frames** after the focus
  moved (`strip_settling`), not once: a width still easing (`<A-S-l>`
  then `<C-w>v` inside 200ms) lays the ribbon out shorter on the focus
  frame than it ends up, and a column revealed against that frame
  drifts out of view as it grows. A reveal of a column in view is a
  no-op, so a swipe is fought only inside that quarter second.
  `Ui::reveal` on a node declared the same frame works (the first risk
  above did not materialise), and leaves a 4px margin, so the column
  beside shows as a sliver when the ribbon has room — a hint that
  there is more.
- **The glide is free**: `slide` on a column eases its drawn position,
  and a scroll offset change is a position change, so `<C-w>l` glides
  the ribbon over 200ms with nothing written for it. A key typed inside
  the glide at a column that is still wholly off the viewport is
  dropped by kui (the same limitation as the entrance); a hand's own
  pace is slower than that.
- **A width step that would not be seen is not a step**: a `Ratio`
  snaps to the nearest preset first, and the snap counts as the step
  only when it moves the column by more than 4% of the viewport in the
  asked direction (`Width::SEEN`); else the key steps on past it.
  `<A-S-l>` / `<A-S-h>` say where they landed (`column two-thirds`,
  `column full already`) and take a COUNT.
- **The status line shows the columns** as `▯▮▯`, the focused one
  filled, before the caret's line and column — a column off the
  viewport is not out of mind.
- **The session writes both**: `root` holds the strip folded to a tree
  (`Tab::to_tree`), so a file written by this build reads as a tree in
  the build before it, and `kind: "scroll"` with `columns` brings the
  strip back here.
- **Twenty columns measured** (the second risk): a headless release
  frame with twenty columns of a 200-line file costs 1.6ms, the same as
  twenty tree panes on screen, against 0.09ms for one pane — under the
  budget, so a column off the viewport still emits its rows; the empty
  box stays an option for a wider ribbon than that.
- **Closing a pane inside a stack keeps the keyboard in the column**
  (its first pane), where the tree's rule sends it to the tab's first
  pane; closing a column's last pane goes to the column before, else
  the one that took its place.
- **A tab switch cross-fades**: the strip node is keyed once for every
  tab, so the columns of the tab left declare `exit` and fade over the
  tab arrived at for 200ms, and the same on `:layout tree`. Kept: it
  reads as a transition, and a column whose subtree is past kui's
  exit budget (512 nodes — a screenful of rows) snaps as before.
