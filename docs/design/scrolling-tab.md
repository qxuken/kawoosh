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
| `<C-w>H` `<C-w>L` | carry the pane past its neighbour | move the column one place left / right |
| `<C-w>J` `<C-w>K` | the same, up / down | the pane up / down inside its column's stack |
| `<A-S-h>` `<A-S-l>` (`<C-w><` `<C-w>>`) | the pane narrower / wider by a twentieth | the column's width to the next preset down / up (niri's `switch-preset-column-width`); a `Ratio` snaps to the nearest first |
| `<A-S-j>` `<A-S-k>` (`<C-w>-` `<C-w>+`) | shorter / taller | the same, inside the column |
| `⌘1`…`⌘9`, `<C-S-1>`…`<C-S-9>` | the Nth pane | the Nth column, the last when there are fewer; both reach one from a terminal pane too |
| `<C-w>e` `<C-w>i` | *(free)* | the pane out of its column's stack into a column of its own after it; the next column's top pane into the stack under it |
| `zs` `ze` `zz` | *(free)* | the focused column against the left edge, the right edge, or centred |
| closing the last pane of a column | — | the column goes, the focus to the column before it |

`:layout scroll` and `:layout tree` convert the current tab (Decision
5), a bare `:layout` (`<leader>tl`) flips it; `layout.default`
(`tree` \| `scroll`) is what `<leader>tn` opens, and what the window's
own first tab is — the strip since 2026-09-22. `<C-w>HJKL` are vim's
"move to the far side", read as one step, and were free until now;
sizing keeps `<A-S-hjkl>`, the pair with no prefix, and gains vim's
`<C-w><>-+` beside it (keys.md).

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

`zs` `ze` `zz` (`strip left` / `right` / `center`) put the focused
column against an edge or in the middle once, for the times the reveal's
"the least that brings it in" is not where the eye wants it; the ribbon
does not scroll past its ends, so the first column cannot be flush
right nor the last flush left.

The gap between columns is `layout.gap` px (the divider's width by
default) and is draggable like a divider: a drag turns the column's
width into `Width::Ratio`.

*Beat:* kawoosh owning the offset every frame (computing it from the
focused column and writing it) — which fights the swipe and rebuilds
what kui already retains, with the `rects` lag on top.

### 4. The strip moves, the tree does not — *shape corrected when built, see "Built"*

This is what Decision 2 wanted kui's animation for, and it is all
declared: a new column has `enter { dx: its own width }` and slides in
from the right; a closing one has `exit { opacity: 0 }` and fades where
it stood (kui replays it frozen and inert) — *dropped when built, see
"Built"*; columns carry `slide`, so
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
  if so; check on day one. *It works; what did bite was a key aimed at
  a column an animation had not finished moving — see "Built".*
- **Twenty columns.** Every column is laid out every frame, and an
  editor pane emits its screenful of rows whether it is on the
  viewport or not. Measure a strip of twenty panes with the perf tab;
  if it costs, a column whose last-frame rect is off the viewport
  emits an empty box of its size instead of its rows — a change local
  to `render_pane`. *It cost (0.09ms a column), and the box was built:
  see "Built".*
- **The swipe and the focus in one frame.** The focus wins (Decision
  3); a swipe that lands a click on another pane focuses it and
  reveals it, which is the expected thing. *A swipe over an editor
  pane's rows never reaches the ribbon, as it happens: the editor owns
  horizontal scrolling there (`on_scroll` on its `lines`). The title
  bars, the gaps and the scrollbar are where a pointer scrolls the
  ribbon; the keyboard has `<C-w>hl`, `<C-N>` and `zs` `ze` `zz`.*

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
  from panes) for its drawn key, so the column keeps its fade and its
  place whatever its panes do — a stack's top pane closing does not
  make a "new" column.
- **The ribbon is what moves** — Decision 4, rebuilt three times over
  2026-09-22 against what use showed, and twice against kui. The first
  shape gave every column `slide` and a `Fixed` width: the width easing
  retargeted the neighbours' slides every frame on a fresh 200ms leg
  (the rubber under a resize and a gap drag), and an arriving column's
  `enter` offset travelled a different distance than the ribbon's
  reveal (the speeds). Making a column a wrapper of `Fit` width around
  a box of `Fixed` width answered both: no width slot on the moving
  node, so widths snap and only positions ease. Then the scrollbar: a
  `slide` eases a node's **viewport** position, which a scroll offset
  changes too, so the columns tweened behind the thumb and could not
  tell the hand's scrolling from a key's. Then the keystroke: kui
  handed a key to the region its focused sink had last frame, and a
  node outside the scroller's clip has none, so `<C-w>l` to a column
  off the viewport and a `q` inside the next 200ms went nowhere
  (`kawoosh/tests/lua.rs`). Two asks to kui, both built the same day —
  **F79**, a key goes to the sink that holds focus wherever the frame
  drew it, and **F80**, a `transition` on a scroll container eases the
  offset a `reveal` takes it to while the wheel, the thumb and an edge
  drag land whole — and the shape that came out of them:
  - **the ribbon eases** (the row's own `transition`, `RIBBON_MS` =
    160ms) to the column a key reveals, which is the motion the eye
    follows;
  - **a column's width and place snap**, so a preset step and `<C-w>H`
    are as fast as the key — and no `slide` rides on a column, which is
    what keeps the thumb linear;
  - **a column arriving** in a strip already on show comes in from a
    third of its width away and fades up over the same 160ms, landing
    as the ribbon does; the strip's first frame (a conversion, a
    restore, a tab switched to) does neither, those columns not being
    arrivals (`Kawoosh::strip_known`);
  - **a closing column goes at once**: an `exit` fade replays on a tab
    switch too, kui playing a ghost whether or not its ancestors
    survived.
  Tests: `the_ribbon_glides_to_a_key_and_a_width_lands_at_once`,
  `a_scroll_the_pointer_makes_is_followed_one_to_one` and
  `a_key_reaches_a_column_the_animation_has_not_finished_moving`, which
  types into a column the slide has not finished moving — the case F79
  was asked for.
- **The reveal answers the strip's shape, not only the focus**
  (`StripShape`: the tab, the focus and its column, the columns' order
  and widths), asked on the frame it changed — once, since kui lays a
  reveal out in the same frame and a second ask against a ribbon
  already gliding measures from where the content has got to and stops
  the leg short — so a
  column widened at the viewport's right edge comes wholly into view
  rather than growing past it, and a tab moved along the tab strip (a
  new key) is revealed again. Not under a gap drag: the offset moving
  under the pointer would feed the width it measures. `Ui::reveal` on
  a node declared the same frame works (the first risk above did not
  materialise), and leaves a 4px margin, so the column beside shows as
  a sliver when the ribbon has room — a hint that there is more. `zs`
  `ze` `zz` ask for an edge or the middle instead, once.
- **A column far off the ribbon draws its chrome and no rows**
  (`Kawoosh::culled`, the second Risk below, measured and taken). A
  pane costs a screenful of shaped, highlighted rows; a column outside
  half a viewport of the viewport — the focused one never — emits an
  empty box in their place, keeping its rect, its title and its hit
  region, so the moves, the mouse and the sessions see no difference.
  Which columns those are is read from the model (the widths and the
  retained offset), not from last frame's drawn rects, so a column
  swiped into view has its rows on the frame it arrives — the *drawn*
  offset, which during a glide is not the one the ribbon is heading
  for (`Ui::scroll_geometry`, kui F80). A frame with
  a 400-line file in a 1600×1000 window (`kawoosh/tests/perf.rs`'s
  `ribbon_frame_cost`): 1 column 110µs, 20 335µs, 60 480µs, 120 703µs,
  250 1.3ms, **500 3.0ms** — about 6µs a column past the handful on
  screen. Without it the cost was 0.09ms a column: 15ms, a whole frame
  at 60Hz, by 120 columns. So the layout is not what limits a ribbon
  any more; a strip of hundreds is a strip nobody can find anything
  in, which is the real limit.
- **A pane leaves its stack by `<C-w>e` and joins one by `<C-w>i`**
  (`Layout::expel` and `Layout::consume`, niri's expel and consume).
  `e` sends the focused pane into a column of its own after the one it
  left, at `layout.column_width`, the keyboard going with it — the
  keyboard's spelling of what a title bar dragged onto a pane's left
  or right edge already did (Decision 1 keeps the tree's drag rules
  inside a column, and `move_pane` makes a column of a pane dropped on
  a side). `i` reads the other way: the *next* column's top pane comes
  into this column's stack under the focused one, and the column it
  emptied goes; pressed twice it takes the two panes that column
  showed, in that order. The keyboard does not follow what it
  consumed — the pane you were in is the pane you are in. A pane that
  is a whole column has nothing to expel, and a last column nothing
  after it to take, and each says so. `e` out and `i` in are `<A-o>`
  and `<A-i>` read for a column, which is where the letters come
  from.
- **The digits are ⌘'s and ctrl-shift's, not Ctrl's** (2026-09-22): a
  terminal pane hears only the chords `Kawoosh::pane_chord` forwards,
  which were the ctrl-shift and alt-shift ones, so `<C-3>` from a
  shell was the pty's and the columns were out of reach there. ⌘ is
  forwarded now — a pty has no use for it at all — and `⌘1`…`⌘9` and
  `<C-S-1>`…`<C-S-9>` are the bindings, the plain `<C-N>` left to the
  shell. Two engine fixes under it: the count read any lone digit,
  and while Ctrl and Alt were excluded ⌘ was not, so ⌘2 did nothing
  *and* left a count for the next key; and a chord's digit lost its
  Shift, `<C-S-1>` normalising to `<C-1>` while the press arrived as
  `<C-!>` (the symbol the layout prints over the digit) — a digit has
  no case to carry Shift the way a letter does, so `notation` and
  `normalize_chord` keep the `S-` and read the symbol back to its
  digit (`keymap.rs`'s `digit_of`). `editor/tests/modal.rs`'s
  `a_digit_under_a_chord_is_not_a_count` and `keymap.rs`'s
  `a_chords_digit_keeps_its_shift`.
- **A share is a column's width** (`Layout::set_share`). The undo
  pane, the memory pane and a Lua view's `view_open { share = … }`
  asked for a fraction of the split they opened in; in a strip a pane
  that is a whole column takes that fraction of the *viewport*, and
  the column it opened beside gives up what it must for the two to be
  on screen together — which is what asking for a share of the width
  means. A pane opened *below* (the picker) is in its column's stack
  and takes its share of that split, as in a tree. An ordinary
  `<C-w>v` asks for no share and pushes the ribbon, as Decision 3
  says.
- **A width step that would not be seen is not a step**: a `Ratio`
  snaps to the nearest preset first, and the snap counts as the step
  only when it moves the column by more than 4% of the viewport in the
  asked direction (`Width::SEEN`); else the key steps on past it.
  `<A-S-l>` / `<A-S-h>` say where they landed (`column two-thirds`,
  `column full already`) and take a COUNT.
- **The status line shows the columns** as `▯▮▯`, the focused one
  filled, before the caret's line and column — a column off the
  viewport is not out of mind.
- **`layout.default` decides the window's own tab too**
  (`Layout::apply_default_kind`, once, on the first frame after any
  session came back): a tab that is still a lone pane in a tree becomes
  a strip of one column, so `layout.default = scroll` — the default
  since 2026-09-22 — is what the window opens as and not only what
  `:tabnew` makes. A tab a session brought back with splits keeps the
  kind the file gave it.
- **The session writes both**: `root` holds the strip folded to a tree
  (`Tab::to_tree`), so a file written by this build reads as a tree in
  the build before it, and `kind: "scroll"` with `columns` brings the
  strip back here.
- **Closing a pane inside a stack keeps the keyboard in the column**
  (its first pane), where the tree's rule sends it to the tab's first
  pane; closing a column's last pane goes to the column before, else
  the one that took its place.
- **No `exit` on a column, and the strip keyed per tab.** With
  `exit { opacity: 0 }` on the columns, a tab switch cross-faded: the
  columns of the tab left replayed as ghosts over the tab arrived at
  for 200ms, and the same on `:layout tree` — and a key per tab does
  not stop it, since kui plays a ghost whether or not its ancestors
  survived (`depart.rs`). Decided 2026-09-22: the close fade goes, a
  closed column vanishes and its neighbours glide into the room. The
  strip is keyed `strip{tab}` all the same, so each tab keeps the
  offset kui retains for it across switches, and `strip_seen` carries
  the tab index so a tab moved along the tab strip (`]T`, a new key)
  reveals its column again. To get the fade back: a kui prop that
  scopes `exit` to a surviving parent — an ask, not yet asked.
