# The markdown buffer: the source, rendered

Status: decided 2026-09-21 (roadmap step 9), built 2026-09-23 (step 17;
"Built" at the end says where it departed); the "markdown,
the fancy buffer" item of the roadmap's buffers track and the buffer
[kui.md](kui.md) Decision 13 left for its own decision. Its kui half
turned out to be there already: what D13 asked of kui — a per-run size
and weight on `rich_text` — is answered by what a row already is (one
`rich_text` node with its own `TextStyle`, so a heading row has its own
size) and by what a span has carried since kui's C22 (`bold`, `italic`,
`underline`, `strikethrough`, `bg`). What this buffer needs of kui that
the plain pane does not use is soft wrap on a row (`wrap = word`, and
`Ui::caret_rect` for the caret on a wrapped row — kui.md's "road to
soft wrap") and the `image` node. Both exist; the round's first day
verifies the three doors headless (Build order). No kui round comes
first. Each decision keeps the alternative it beat.

## The thesis

"Rendered" here is not a preview: not obsidian's reading view, not
vscode's split, not a browser. It is the buffer itself drawn with its
marks hidden and its structure given weight — typora's and obsidian's
live preview, nvim's conceal — while the text under it stays the
source: the same `Buffer`, the same motions, `:w` writes what `:e`
read, search runs over the bytes, the undo tree is the file's, marksman
speaks LSP to it. The line the caret is on shows its source, so the
marks are editable where they are and the caret never stands in a
hidden byte.

Why not a preview pane: it doubles nothing useful. The eye is on the
edit; a second pane is a second place to look, and the moment it is
scrolled to the same paragraph it is this buffer with a lag.

What the plain pane cannot do, and this one must: draw a row larger
than the others, hide bytes out of a row's columns while a click still
lands on the right byte, and wrap a paragraph — prose is long lines,
and a long line scrolled sideways is what makes a markdown file
unreadable in a code editor today.

## Decisions

### 1. A way of drawing the buffer, not a buffer kind

`markdown.render` is a setting, `true` by default for the markdown
language; `:set -markdown.render` turns it off and `markdown toggle`
(`<leader>cr`, *render*, under the code group) flips it. When it holds
and the buffer's language is markdown, `rows::emit_line` reads a
`Rendered` for the line (a new `kawoosh/src/markdown.rs`) and draws
from it; otherwise the row is what it is today. Nothing else in the
editor knows: the buffer, the file, the histories, the LSP, the picker's
preview (which is the plain pane's rows) are unchanged.

*Beat:* a rendered buffer kind — a `*preview*` scratch re-rendered on
change, in a split or a browser (markdown-preview.nvim's shape): the
pane that is never the one being edited.

### 2. The runs are the grammar's; nothing parses twice

The ts thread already yields the runs (kui.md D13): `Heading`,
`Strong`, `Emphasis`, `Link`, `Raw`, and tree-sitter-md's markers as
`Punctuation` (`@punctuation.special` for `#`, list markers and the
fence's backticks; `@punctuation.delimiter` for `*`, `_`, the link's
brackets and parentheses), the inline grammar injected into every
paragraph and a fence's language into its content. `Rendered` reads
those runs and the line's text and produces three things per row:

- **the spans** — `Strong` → `bold`, `Emphasis` → `italic`, `Link`'s
  label → `underline`, `Raw` (a code span) → a `bg` of the panel
  colour, the injected language's tokens as their colours;
- **the row's size** — a `Heading` row by its level (the count of
  leading `#`; a setext heading by its underline's character), from
  `markdown.heading` (`{ 1.6, 1.35, 1.15, 1.0 }`, h4 and below at the
  body size, bold); everything else the body;
- **the fold table** — byte ranges of the row drawn as something
  else: the marks hidden (`""`), a list's `-` `*` `+` drawn `•`, `[ ]`
  and `[x]` drawn as the shipped Nerd Font's boxes (`U+F0131`, and
  `U+F0C52` in the links' accent — `☐` `☑` until 2026-09-27, which few
  monospaced faces have, so a fallback drew them thin and small), a blockquote's `>` a bar in the margin
  colour, a fence's opening and closing lines a thin rule with the
  info string dim, a link's destination hidden with its parentheses,
  `---` a rule across the row.

`Drawn` (rows.rs) grows from a one-for-one mapping (tabs to spaces,
spaces to NBSP) to a table, so a click's x → byte and a byte → x still
answer through it; the plain pane's table is the identity.

*Beat:* a markdown parser in kawoosh (pulldown-cmark) — a second
opinion of the text beside the grammar's, disagreeing at the edges (a
fence inside a list, an emphasis across a link), without the
injections the grammar already has.

### 3. The caret's line is the source

Every line holding a selection's head — every caret's line, in every
mode — is drawn raw: no fold, the marks visible, the row's size kept
(a heading stays large so the page does not jump), still wrapped
(Decision 4). The rows around it are rendered. This is nvim's
`concealcursor=""` and obsidian's live preview, and it decides three
things at once: the caret is always on a one-for-one row, so today's
column math holds where it matters; `l` never crosses a hidden `**`;
`x` never deletes a byte the user could not see.

The cost is the jump: moving `j` onto a rendered line makes its marks
appear, and the byte column the caret keeps (`goal column`) lands
where it would in the source. Known from nvim, lived with.

*Beat:* the caret moving over rendered text, every motion and text
object reading the fold table to skip hidden bytes — the whole engine
learning about drawing.

*Amended 2026-09-27, from use:* in visual mode every line a selection
covers is raw, not only its head's. With the head's alone, a selection
grown by `j` turned each line it reached raw and the one it left
rendered again, so the text reflowed under the selection at every
step; now a line turns raw once, as the selection reaches it, and
stays so until visual mode ends. The selection is on the source it
takes.

### 4. Rendered rows wrap, and this is where soft wrap enters

A rendered row's text gets `wrap = word` at the pane's text width, and
its box is `Sizing::Fit`: the pane is a column of rows of the height
kui gives them. Scrolling stays by row — `top` is a line — and the
pane emits from `top` as many rows as the viewport holds at the body
height (more than fit once rows wrap; the clip hides the rest), so a
tall row costs nothing new. What changes is that the pane no longer
knows a row's height before the frame: the last visible row, which
`scrolloff` and `<C-d>` need, is read from last frame's rects (the
pane records each row's bottom the way `Layout.rects` records panes);
and the caret on a wrapped row is placed by `Ui::caret_rect(row, byte)`,
which answers from the last frame. So the frame that types draws the
caret where it was, and the pane asks for one more frame, which moves
it — one frame late on a wrapped row, never wrong on a still one.
`j` and `k` stay by line; `gj` `gk` by visual line are not this round.

*Beat:* no wrap — the plain pane's long line scrolled sideways, which
is the thing that makes prose unreadable. And wrap for every language
in the same round: the door is the same, and a `wrap` setting can
follow through it, but a code buffer's wrap changes the long-line
windowing (`LONG_LINE_BYTES`), the caret measure and every column
assumption at once; the markdown buffer is one language, whose rows
already need their own drawing.

### 5. The mono face, at sizes

Headings are the mono face at a ratio, not a proportional face: a
proportional face is still a non-goal, and a mono heading in the same
face reads as the same document. A heading row's cell width is its
size's, so its columns are its own (`Drawn`'s `cell_w` becomes
per-row); its line height is the ratio times `font.line_height`.
Bold and italic come from the face's variants as kui finds them.

*Beat:* `font.prose`, a second `Face` for rendered rows — a second face
through every mono path for a difference nobody asked for.

### 6. An image line is an image row

A line that is only `![alt](path)` with a local path is drawn as kui's
`image` node: the bytes read by the io thread, `add_image` once per
path, at most the pane's width and by aspect, capped in size
(`markdown.image_max_mb`); the source line when the caret is on it. A
URL or a path that does not read shows the alt, dim. Nothing inline —
kui shapes a text as one paragraph and an image is not a glyph.

### 7. Tables align, links open

A table's rows are padded through the fold table so the pipes line up
— `Rendered` for a table's lines reads the whole table's node from the
tree, since the widths need every row. `gx` on a link (free in keys.md;
nvim's) opens it: a relative path through `kawoosh.open` (the openers,
so a `.md` beside opens here), a URL through the OS. Both after the
rest if the round runs long.

### Deliberately not

- **A preview pane**, live or otherwise.
- **Math, diagrams, HTML.** Drawn as their source.
- **A proportional face.** Decision 5.
- **Wrap for other buffers, in this round.** Decision 4.
- **`gj` `gk`.** With wrap, later, if missed.
- **Editing through the rendering** (typing `**` around a word and
  seeing bold as you type). You see it when the caret leaves the line.

## Build order

One round, one commit, in this order:

1. **The doors, headless, on day one.** A wrapped `rich_text` row with
   `bold` and `italic` spans and a `Fit` height in a clipped column;
   `Ui::caret_rect` on it after a settled frame, and whether it answers
   on the frame the row was declared; an `image` in a row. Each in a
   test against kui's headless core before any of kawoosh's code moves.
2. **`markdown.rs`.** `Rendered` from the runs: the spans, the size,
   the fold table; `Drawn` on a table; headings, emphasis, strong,
   links, code spans, fences, lists, checkboxes, quotes, rules; the
   caret-line rule; `markdown.render`, `markdown.heading`, the toggle
   and its key.
3. **Wrap.** `wrap = word` and `Fit` on rendered rows, the rows' rects,
   `scrolloff` and the half-page moves from them, `caret_rect` with the
   follow-up frame.
4. **Images, tables, `gx`.**
5. keys.md (`<leader>cr`, `gx`), kui.md D13's paragraph pointed here,
   the roadmap's item struck.

Tests (`kawoosh/tests/markdown.rs`, a fixture with every construct):
the heading row's size and the body's; the `**` folded on a rendered
row and present on the caret's; a click on a rendered row landing on
the right byte through the table; `caret_rect` on a wrapped row after
the follow-up frame; the toggle; the image row's height; `:w`
writing the source unchanged. The picker's preview of the same file
unchanged.

## Risks

- **The marker captures.** `Token::from_capture` maps
  `punctuation.special` and `punctuation.delimiter` to `Punctuation`,
  the same token a code buffer's `;` has. Inside markdown that is what
  the fold reads (with the run's text saying which mark), but inside
  an injected fence the tokens are the fence language's and must not
  fold — the injection's range says which is which; if it is not
  enough, a `Marker` token for the two markdown captures is the small
  fix.
- **`caret_rect`'s lag under a held key.** A frame behind at the key
  repeat rate; the follow-up frame is at once, so the caret trails by
  a frame while a key is held and catches up when it is released.
  Watch it; if it reads as wrong, the caret's row can be measured the
  plain way while it does not wrap (most rows do not).
- **Fit rows and the frame's scroll.** Scrolling past a tall image row
  jumps by its height — by design (scroll by row), and what nvim does.
- **The grammar under a half-typed mark.** A lone `**` makes the rest
  of the paragraph strong until its pair is typed — on the caret's
  line it is raw, so the flicker is on the lines below, which is what
  every live preview does.

## Built

2026-09-23, one round, as decided but for these:

- **A structure layer beside the syntax's (Decision 2, and Risk
  one).** The runs alone cannot say which line is a fence's: tree-sitter-md
  paints `code_fence_content` `@none`, so a fence's lines are either
  unpainted or the injected language's, and a heading's `#` is
  `Punctuation` like any other. So a grammar may carry a second query,
  a *structure* query (`Grammar::with_structure`, captures
  `@block.NAME`, a `Block` each: a code block, a fence and its info, a
  table with its header and delimiter row, a quote's `>`, a bullet, an
  ordered marker, a task's box, a heading by level, a setext underline,
  a rule, verbatim HTML), painted by the ts thread from the same tree
  into the `structure` layer in the same answer — one parse, no second
  parser. The inline marks are still read off the syntax's runs, byte
  by byte.
- **Row heights from kui's layout, a frame behind (Decision 4).** Each
  rendered row is keyed and declares `on_layout`; its height is read
  back with `layout_of` the next frame (`Kawoosh::md_heights`), the
  pane follows the caret by those heights (`md_follow`, a row not seen
  yet at the body's), and a row that measured otherwise than the
  scroll assumed asks for a frame more. The first headless check found
  a real defect of the plan: an overflowing column compresses its
  children toward their floors, so a wrapped row was squeezed back to
  its first lines until it said `min_height = fit`.
- **The line number is in the row.** A gutter column of fixed rows
  cannot sit beside rows of their own heights; each rendered row
  carries its number in a `Role::None` cell, which kui's line bytes
  do not count.
- **No horizontal scroll, and code wraps by glyph.** A rendered pane's
  rows are the pane's width; prose wraps by word, a code block's lines
  by glyph (cut, a line would be lost off the edge), and a table not at
  all (its rows are clipped: the alignment is the point).
- **The bar caret by `caret_rect`, as planned; no ghost.** The
  completion's ghost is a node beside the text, and a wrapped row's
  text is one paragraph; a rendered row draws none (`<C-x>` still lists
  the candidates).
- **Images on the io thread with the `image` crate**, already in the
  tree through kui's clipboard (PNG, JPEG, GIF), capped by
  `markdown.image_max_mb`, registered with kui on the next frame; a
  URL, a path that does not read, or one still being read shows its
  alt, dim.
- **Tables and `gx` made the round**, as did `markdown.heading`.

`kawoosh/src/markdown.rs`, the structure query in
`languages/src/markdown.rs`, `Drawn::folded` and `RowForm` in
`rows.rs`, `kawoosh/tests/markdown.rs` (the fixture
`tests/fixtures/rendered.md`), and the renderer's unit tests.

After a day's use (2026-09-23), five more:

- **The structure is repainted over whole lines.** A heading typed a
  `#` at a time kept its first byte an h1 while the line became an h2:
  the reparse's changed span was the typed byte, and the layer's runs
  outside it stood. The ts thread widens every structure span to the
  lines it touches.
- **A table scrolls on its own.** A table's rows are one block, keyed
  by its first line, that scrolls sideways — the wheel's `dx` over it,
  and the caret, which slides it to show itself — its line numbers in
  a column beside it that does not scroll; the rest of the pane does
  not move. Its rows are as wide as their text (`RowForm::fit`).
- **A line of images is a row of images**, side by side, each at most
  its share of the width. A `data:` URI's base64 is decoded in place.
- **A table is a kui table** (kui's ADR 0033, `NodeSpec::table()`): its
  rows are the table's rows and their cells its cells — a 1px rule, a
  cell, a rule, …, a rule — so the layout lines the columns up whatever
  is in them, a text or an image, and a rule is as tall as its row and
  meets the next row's. The delimiter row is a rule across each cell;
  the edges above the first row and below the last are rows of rules,
  1px, as wide as the columns. A row's drawn text is its cells' texts
  one after another — the pipes, the pads and an image's source folded
  away, a cell's inline marks as prose's — and kui reads a line's text
  from its text nodes in order, so a click through a cell lands on its
  byte. The caret's row is its source, a child of the table and not a
  row of it — with its cells as they are drawn away from the caret
  beside it, 0px tall (`rows::table_ghost`), so the columns keep their
  widths: without them `j` and `k` through a table moved every column
  whose widest cell was on the caret's row. A cell is inline like a
  paragraph: the grammar injects `markdown_inline` into
  `pipe_table_cell` too, which tree-sitter-md's own query does not, and
  a cell's `**bold**` had kept its stars. An image is a cell, at most its column's share of the pane,
  so a README's light and dark screenshots sit under their headers.
  This replaced cells padded to monospace widths between box-drawing
  characters: `│` is a glyph shorter than its line, so the sides never
  met, and a column as wide as the source's `![…](…)` had nothing to do
  with the image drawn under it.
- **A row of empty cells does not swallow the document.** tree-sitter-md
  loses its place on a table's row with an empty cell — a lone `|` took
  the blank line and the heading after it into the table, `|||` made
  the rest of the document one ERROR, every heading and fence in it
  gone — which is every row on its way to being typed. The grammar
  carries stand-ins (`Grammar::stand_ins`, markdown's `stand_ins`):
  such a row is read by the parser as `|   |` of its own length (`|a`,
  `a` for the shortest), still a row; the text keeps its bytes and the
  cells are read off the text. A document with one is parsed whole, its
  tree not kept, since an edit anywhere can change which rows they are.
- **A key takes only a ghost it can see.** A rendered row that wraps
  draws no completion ghost (Decision 4's "no ghost"), but `<Tab>` and
  `<CR>` still took the completion: `Setex` then Enter put `Setext` in
  the paragraph and no newline. `emit_line` answers whether it drew the
  ghost, the pane keeps it (`Kawoosh::ghost_shown`), and the keys accept
  only then — in a paragraph `<CR>` is a newline; on a table's source
  row, where the ghost is drawn, it completes as before. A code block's
  rows wrap by glyph and draw none either, so a fence in the rendered
  buffer does not complete in place (`<C-x>` lists the candidates).
- **Prose wraps at the width of this frame.** A wrapped row's text was a
  box fixed at the pane's width as last frame's layout recorded it, and
  a pane drawn for the first time — a restored tab shown, a split made —
  had none: it wrapped at 40px for a frame, a word or a syllable a
  line. The text now grows to its row and kui wraps it there, in the
  frame it is drawn; an image's share of the width, which still needs a
  number, takes the window's when the pane has no rect yet.
- **`gx` on an anchor goes to its heading**: `#seed-data`, or
  `file.md#top` after opening the file, by GitHub's slug (lower-cased,
  punctuation dropped, spaces as `-`, a repeat numbered); it had opened
  the directory.
