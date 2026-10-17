# Completion: when it asks, how it shows, what it offers

Status: designed 2026-10-16/17 with the user, over six rounds of
questions, from the todo's "add autocompletion settings (including
disabling)". Rounds 1–3 of the build (the model and the settings; the
popup; LSP snippets) built 2026-10-17: "Built" at the end says how, and
where each departs. The decisions below are the rounds' answers,
each with the alternative it beat. The build is five rounds (the last
section), one session each. Words are as [GLOSSARY.md](../../GLOSSARY.md)
has them.

## What there is

Built in milestone 6 and roadmap step 7 (`kawoosh/src/lsp.rs`):

- **When:** a completion is asked as a word starts (its first
  identifier character) and on a server's trigger characters (`.` and
  `:` for a server that names none), once per word; typing on narrows
  it by prefix, ignoring case.
- **From:** the language server, else the buffer's own identifiers
  (three characters or longer, nearest first) when no server answers.
- **Shown:** one candidate as a ghost at the caret, no menu (mvp.md
  Decision 5). `<C-n>` `<C-p>` `↓` `↑` cycle, `<Tab>` `<C-y>` `<CR>`
  accept, `<C-e>` dismisses, `<C-x>` lists the candidates in a picker
  (kind, signature, documentation as the preview).
- **Snippets:** none; kawoosh tells servers `snippetSupport: false`.
- **Settings:** none.

What the user found wrong, in their words: no snippets; the ghost
mid-line "way more intrusive than it should"; `<CR>` taking the ghost
"is bad in this key design"; and no way to turn it off or down.

## Decisions

### 1. When it asks: `completion.trigger`

`"typing"` (as today) | `"endline_only"` (as today, but only where the
caret has nothing after it on its line) | `"manual"` (only when asked,
`<C-Space>`) | `"off"` (no requests, no display). Default `typing`;
the shipped language defaults set `manual` for markdown, gitcommit and
plain text.

Every completion setting goes through the layers settings already
have: global, `language.LANG.completion.*`, the project's, and
`:setlocal` for one pane.

*Beat:* a boolean `completion.enabled` — "off" and "manual" are both
wanted, and mid-line is its own case.

### 2. How it shows: two axes, not a style

- `completion.ghost` = `"always"` | `"endline"` (only where it overlays
  nothing: the caret at its line's end) | `"never"`.
- `completion.popup` = `"always"` | `"fallback"` (whenever no ghost is
  drawn: mid-line, a fuzzy-only match, the ghost off) | `"never"`.

Every look is a pair: `always`+`never` is today's ghost, `never`+`always`
a popup alone, `always`+`always` both (the popup with the selected
candidate's ghost), `endline`+`fallback` a ghost at the line's end and
the popup mid-line. Default `endline` + `fallback`: never intrusive
mid-line.

The popup is a list under the caret, flipping above near the pane's
foot, at most `completion.max_items` (10) rows: the kind's icon, the
label with the matched characters lit as a picker lights them, the
detail faint at the right.

*Beat:* `completion.style = "ghost" | "popup"`, the first ask — every
in-between (`midline_popup`) would be another value; two axes compose
them all.

### 3. Keys are the keymap's

The keys are commands bound in local maps that hold only while a
completion or a snippet is live (local-maps.md): `completion accept`
(`<Tab>`, `<C-y>`), `completion next` / `prev` (`<C-n>` / `<C-p>`),
`completion close` (`<C-e>`), `completion docs` and its scroll
(`<C-f>` / `<C-b>`), `snippet next` / `prev` (`<C-l>` / `<C-k>`),
`signature` (`<C-g>`), and in insert mode always `lsp complete`
(`<C-Space>`) and `lsp candidates` (`<C-x>`, the picker, whatever
`popup` says). `<CR>`, `↑` and `↓` are not taken: a newline is a
newline, an arrow moves. Rebinding is `kawoosh.map` / `kawoosh.unmap`;
`:map export` and the which-key show them.

*Beat:* `completion.accept` and `completion.cycle` as key lists in
settings (the first round's answer, revised) — a second place keys
live, which the keymap, its export and its which-key would not know.

### 4. Where candidates come from: `completion.sources`

An ordered list; every source in it is asked, and the answers are
merged (Decision 5). Default
`{ "lsp", "path", "snippets", "buffer_symbols", "buffer_words" }`.

- `lsp` — the server's answer.
- `path` — on `/`, `./`, `../` or `~/` before the caret: the entries
  relative to the buffer's file's directory (the tab's for a scratch),
  directories first with a `/` that asks again; listed on the io
  thread, a host's over its connection, never on the frame.
- `snippets` — the snippets of the buffer's language and `all`
  (Decision 8).
- `buffer_symbols` — the names the grammar's outline finds in open
  buffers (functions, types, constants), with their kinds: no server
  needed.
- `buffer_words` — the identifiers of every open buffer, three
  characters or longer.
- `workspace_symbols` — opt-in: the server's `workspace/symbol`, asked
  from three typed characters, debounced, dropped when late.

**A private buffer is no source** (secrets.md Decision 1): its words
and symbols are never offered anywhere.

*Beat:* `workspace_words` — every word of the project needs an index of
the whole tree; `workspace_symbols` answers most of it from the
server. *Beat:* the buffer's words only when no server answers (as
built) — merged and ranked, a word the server does not know still
shows, below its answers.

### 5. Ranking: the match first, the rest break ties

Candidates match fuzzily, by the matcher every picker uses
(matching.md: terms, smart case, a word's start scoring more). Their
rank is, in order: the match's quality; the source's place in
`completion.sources`; nearness (the same buffer, fewer lines away,
then other buffers); **frecency** — every accepted candidate is
remembered per workspace in the state db, decaying with time; and the
**memory's signals**, per workspace: a candidate in a text yanked or
pasted in the last minutes, a word searched for (`/`, `*`, the project
search), and words and symbols of the files attended and edited lately
weighing over other open buffers. A private buffer's texts are never
in the memory, so never in a signal. `completion.fuzzy = false` matches
by prefix only, as built.

A ghost shows only a candidate that continues what is typed; a
fuzzy-only match is the popup's, or `<C-n>`'s in a ghost-only look.

*Not yet:* pins' symbols and the jump list's change spots as signals —
if the ranking still feels off.

### 6. The data model is cheap by construction

- **A session per anchor** — the buffer, the word's start, the version
  it started at; a key that moves the caret off the word or an edit
  elsewhere ends it.
- **Answers** are each source's, shared and never copied (`Rc<[Item]>`)
  with the source's rank; one request in flight per source; an answer
  for an anchor that is gone is dropped as it lands.
- **The ranking** is `(score, source, index)` triples, made again only
  when the typed word changes; a longer word only drops matches, so it
  rescans the survivors alone; only the top `max_items` are sorted
  fully, the rest counted.
- **The selection** is an index into the ranking.
- **The display** is worked out each frame from the two axes, "the
  caret at its line's end" and "the selection continues the word": a
  small value — none, a ghost (a borrowed suffix), the popup, both —
  with no allocation.
- **The popup's rows** are built for the rows shown only, the match's
  lit ranges kept beside its score.
- **The documentation** (`completionItem/resolve`) is asked for the
  selected candidate only, after `completion.docs` ms, and kept on it.
- **`buffer_words` and `buffer_symbols`** are an index per buffer at its
  version, built off the frame; a completion reads and merges them.

A keystroke costs a rescan of the survivors and a top-K sort; a frame,
a constant.

### 7. Snippets are selections

`snippetSupport` on. A snippet accepted becomes kawoosh's own
selections: the current field selected, so typing replaces it; a field
that repeats (`$1` twice) is several carets edited together; `$0` the
last caret; a choice field (`${1|a,b|}`) offers its choices as a popup.
`<C-l>` goes to the next field, `<C-k>` back; `<Tab>` is left alone.

**One snippet at a time, and gone when it ends** — what the user hated
in LuaSnip was `<Tab>` jumping back into a snippet long left. A snippet
ends at its last field, on leaving insert mode, when the caret leaves
its text by anything but a jump, when an edit lands outside it, on a
buffer switch, and when another is accepted. With none live, `<C-l>`
does nothing; nothing remembers past ones. While one is live, the
fields not yet visited are drawn faintly underlined.

### 8. The user's snippets: Lua in VS Code's shape, JSON beneath

```lua
kawoosh.snippets.add("rust", {
  ["for loop"] = {
    prefix = "for",
    body = { "for ${1:x} in ${2:iter} {", "\t$0", "}" },
    description = "a for loop",
  },
})
```

The table is VS Code's JSON snippet format as Lua, so a JSON file is
the same data. Under it, two general doors: `kawoosh.json.decode` /
`encode`, and `kawoosh.fs.read_json(path)` with its async form
`kawoosh.fs.read_json(path, function(value, err) … end)` — the
callback last, off the frame, as `fs.list`. On them a helper,
`kawoosh.snippets.load_json(path)`: a VS Code JSON file, or a package
laid out as friendly-snippets is (`package.json`'s
`contributes.snippets` naming each language's file). `snippets.paths`
in the settings names folders to load; a language's are read the first
time a buffer of it asks, and the folders are watched. Bodies are the
LSP grammar: `$1`, `${1:default}`, `${1|a,b|}`, `$0`, and the variables
(`$TM_FILENAME` and the rest). No snippets ship.

*Beat:* a snippet loader of its own reading JSON — a JSON door is
wanted beyond snippets; the loader is ten lines over it. *Beat:*
coroutines for the async read — kawoosh's Lua is callbacks everywhere
else; coroutines are a later, separate step.

### 9. One icon set, in its own font

One icon per kind — function, method, field, variable, constant,
class, struct, enum, interface, module, keyword, snippet, file,
folder, word, … — Nerd Fonts' codicons (VS Code's own glyphs), one
table in icons.md / `kawoosh.icon`. LSP's completion kinds and symbol
kinds share the names; the popup, the `<C-x>` candidates picker and
the symbols pickers all draw from it, and so do `path`, `buffer_words`
and `snippets`.

Every icon kawoosh draws itself — these and every glyph the sweep
finds (gutter signs, chevrons, the tab's close, status segments) —
draws in `font.icons` (default the shipped `Symbols Nerd Font Mono`,
fonts.md Decision 6), never the text's family, in a box one cell wide
centred on the line, so a patched font, a size or a wide glyph cannot
move a column. A terminal's glyphs stay its program's.

*Beat:* the fallback as built — a patched Nerd Font as `font.family`
drew its own icons first, at its own widths.

*Amended 2026-10-17, the user, after round 2:* "if something belongs
to kawoosh ui itself, like close icon, i think it belongs to vector
api". The pictures of kawoosh's own interface — the tab's close, the
chevrons, the folds, the done mark, the key caps — stay icons.md's
vectors; `font.icons` is for the icons that are data — a kind's, a
character in a status segment, a task's box.

### 10. One information panel, and signature help called for

At most two panels: the popup, and one information panel beside it —
the selected candidate's documentation while the popup is being moved
through, the signature otherwise; never both stacked (the user's nvim,
seen in a screenshot: three panels at once, "noisy, however useful").
With `popup = "never"` the panel sits above the caret's line.

Signature help: `completion.signature = "manual" | "auto" | "off"`,
default `manual`. `<C-g>` inside a call shows the signature with the
parameter the caret is in lit and its documentation's first paragraph;
it follows the arguments as `,` is typed and closes when the caret
leaves the parentheses, on `<C-e>`, or on `<C-g>` again. `auto` opens
it on `(` and `,`, as lsp_signature does.

`completion.callable = "name" | "args"`, default `name`: accepting a
function puts its name only (no brackets, as the user had blink); with
`args`, its parameters as snippet fields where the server's candidate
has them.

*Beat:* signature help always on — useful and noisy; called for, it is
there when wanted.

### 11. The settings, whole

```lua
completion = {
  trigger   = "typing",    -- typing | endline_only | manual | off
  ghost     = "endline",   -- always | endline | never
  popup     = "fallback",  -- always | fallback | never
  sources   = { "lsp", "path", "snippets", "buffer_symbols", "buffer_words" },
  fuzzy     = true,
  docs      = 200,         -- ms before the information panel; false: on `completion docs` only
  max_items = 10,
  words     = { min_length = 3 },
  signature = "manual",    -- manual | auto | off
  callable  = "name",      -- name | args
},
snippets = { paths = {} },
font = { icons = "Symbols Nerd Font Mono" },
```

No key lists, no brackets setting.

## Build: five rounds

1. **The model and the settings.** Decisions 1–6: `trigger`, the
   two axes with today's ghost as the only display, the keymap
   commands and local maps, fuzzy ranking with frecency and the
   memory's signals, merged sources with `path`, `buffer_words` and
   `buffer_symbols` (and `workspace_symbols` opt-in), the indexes off
   the frame. `<CR>` and the arrows given back.
2. **The popup.** Its rows, the kind icons, `font.icons` and the glyph
   sweep (Decision 9), the information panel with documentation
   (Decision 10's first half).
3. **LSP snippets** (Decision 7).
4. **The user's snippets** (Decision 8): `kawoosh.json`,
   `kawoosh.fs.read_json`, `kawoosh.snippets.add` and `load_json`,
   `snippets.paths`.
5. **Signature help and `callable`** (Decision 10's rest).

One session each, in order, each merged before the next.

## Built

### Round 1, 2026-10-17: the model and the settings

`kawoosh/src/completion.rs` (completion's half; lsp.rs keeps asking
the server and the candidates picker), `lua/src/fuzzy.rs`'s `Single`,
`systems/src/words.rs`, the state db's `completion_accepts`, the
settings in `editor/src/settings.rs`, the keys in
`editor/src/commands.rs` local to the `completion` fact.

As designed: Decision 1's four triggers through every layer (the
pane's `:setlocal` too — `completion.trigger`, `ghost`, `popup`,
`sources`, `fuzzy`, `max_items` and `words.min_length` are pane
settings); Decision 2's two axes, the
ghost the only display, `popup` read and not drawn; Decision 3's
commands and local maps, `<CR>` and the arrows given back; Decision
4's sources but `snippets` (a no-op until round 3); Decision 5's
ranking; Decision 6's model.

Where the build departs, or says more than the decisions did:

- **The tie-breakers bite in buckets.** Compared field by field, a
  float would leave nothing for the next field to decide, so each is
  coarse: the match is fzy's score ×10 rounded, with the letters after
  the last matched one not counted (`Single`) — a word typed finds
  every word it begins alike; nearness is the same buffer by powers of
  two of lines away, then the files attended lately (the ring's last
  forty rows), then other open buffers; frecency is its score ×4
  rounded, halving in a week. Two words on one line are in the text's
  order (the index keeps lines, not columns).
- **The ghost-only look walks what a ghost can show.** `<C-n>` and
  `<C-p>` go round the candidates that continue the word as typed; a
  fuzzy-only match, or one in another case (`Widget` for `wid`), is
  ranked and reachable through `<C-x>`, and is the popup's in round 2.
  Decision 5's "or `<C-n>`'s in a ghost-only look" is left for round 2
  to settle beside the popup. Once `<C-n>` or `<C-p>` picked a
  candidate, a landing that ranks again (an index, an outline, an
  answer) keeps it selected by what it puts, the best only when it is
  gone; until then a landing selects the best afresh, so a server's
  answer can still take over from the buffer's words it followed.
- **The session's end.** A key that moves the caret with no edit ends
  it, even onto the word's own start (`<Home>`); typing or deleting in
  the word follows it; an edit whose length is not the word's is an
  edit elsewhere, and so is any version change no key followed (a
  server's edit, a reload, checked each frame). With several carets
  the session follows the primary one's word, its start carried
  through the edits since the last key; each caret typing what it
  types is no edit elsewhere. The servers forgetting the buffer (a
  restart, their table changed) drops only the request in flight. A session with no
  candidates stays, quiet, for its word rather than asking at every
  key; the server is asked again only when it said its list was
  incomplete (`isIncomplete`, now read). `completion close` keeps the
  session, closed, so typing on in the word asks nothing.
- **What asks.** A word begun (a letter, a digit, `_`), a server's
  trigger character (`.` and `:` for one that names none), or a `/`
  that ends a path. A `/` or `//` alone — division, a comment — is a
  path's only on `<C-Space>`. Words and symbols are offered for a word
  begun, never after a trigger with nothing typed. A path's session
  lists the directory (and asks the server where `/` is its trigger,
  or when asked); it offers no words.
- **The indexes.** `buffer_words`: an io job per buffer whose index is
  older than it (its text, or its masks), the first 2 MiB of its text,
  50,000 distinct words at most, every length — one index a buffer,
  each session reading it at its own `words.min_length` — and the
  stale index read until the fresh one lands. A word
  the same as the one typed is never offered; one typed earlier and
  left half-done is a word like any other. `buffer_symbols`: the
  outlines of the buffers the syntax thread holds a tree of (those on
  show, or shown since), asked at the tree's version.
- **Masks are no source.** A buffer's text is read through its masks
  (secrets.md Decisions 3 and 9: a rule's, a plugin's, a multibuffer's
  carried, the one `zv` shows too): a word reaching into a mask is not
  indexed but kept apart, and no source offers it — a server's answer,
  a symbol, another buffer's word — nor is it a signal or remembered;
  the job's copy of the text is wiped. A buffer made private leaves
  the live session at once and its words are wiped; one private by
  itself (a multibuffer showing a private file) is dropped at the next
  frame.
- **The signals.** The texts are the memory's moments of the last ten
  minutes, none a secret's; the searches are the search prompt's last
  twenty, read again whenever its newest entry moves. `*` and the
  project search do not reach that history yet. Frecency is not kept
  for a private buffer.
- **The state db, off the keys.** The workspace's frecency is read
  from the frame once the workspace is known, the files attended lately
  every few seconds at most, each on an io job over a read-only
  connection of its own; a ranking before they land goes without them
  and is ranked again when they do. An accepted candidate counts at
  once and is written with the memory's next flush, one transaction
  for all of them.
- **`workspace_symbols`** is asked once a session, 150 ms after the
  third character, and narrowed by the rescan as the word grows.

Measured with `kawoosh/tests/completion.rs`'s
`timing_a_keystroke_and_a_frame` (dev profile, a 20,000-line buffer of
words, median / p90): a keystroke 0.63 / 5.65 ms before, 0.62 / 0.68
ms after; a word's start 5.59 / 5.70 ms before (the buffer read on the
frame), 2.26 / 2.34 ms after (forty thousand indexed words ranked;
2.36 / 2.41 ms once the review's fixes read the masks and keep every
length); a frame 0.170 / 0.178 ms before, 0.171 / 0.182 ms after, a
ghost on show.

### Round 2, 2026-10-17: the popup, the icons, the panel

`kawoosh/src/completion.rs` (the display, the rows, the panel's
model), `kawoosh/src/completion_view.rs` (drawn), `rows::LineDraw::hang`
(the mark the popup hangs from), `kawoosh/src/icons.rs` (`KINDS`,
`kind_icon`, `cell_icon`, `text_with_icons`, `font.icons` on the set),
`fuzzy::Single::positions`, `completionItem/resolve` in
`systems/src/lsp.rs`, `picker.lua`'s `icon =`. Tests:
`kawoosh/tests/completion_popup.rs`; the strip's and the counts'
icons in `tests/status.rs`, the task boxes' mark in `markdown.rs`.

As designed: Decision 2's popup and the pairs (each ghost × popup at a
line's end and mid-line is a test), at most `completion.max_items`
rows, flipping above near the pane's foot; Decision 6's display (one
small value a frame), rows built for the rows on show only and the lit
ranges read for them alone, the documentation asked for the selection
only after `completion.docs` ms and kept on the item; Decision 9's one
table of kinds in the popup, the `<C-x>` picker and both symbol
pickers, `font.icons`; Decision 10's first half, one panel, beside the
popup or above the caret's line with no popup, `<C-f>` / `<C-b>`.

Where the build departs, or says more than the decisions did:

- **`<C-n>` and fuzzy-only matches (round 1's open question),
  settled.** Beside the popup `completion next` / `prev` walk every
  row, a fuzzy-only one or one in another case too, the window
  following the selection. From a ghost in a `fallback` look, a walk
  onto a candidate no ghost can show brings the popup — Decision 2's
  "whenever no ghost is drawn" — and the popup then stays for the
  word, rather than giving way to a ghost under the keys as the walk
  comes back onto a continuation. In the ghost-only look (`popup =
  "never"`) they walk what a ghost can show, as round 1 built: a
  fuzzy-only match selected there would show nothing; it is `<C-x>`'s.
  Decision 5's "or `<C-n>`'s in a ghost-only look" is so only where a
  popup may come.
- **The selection.** Where a popup may show (`always`, `fallback`) the
  first candidate is selected, so a fuzzy-only best is the popup's and
  no ghost of a lower one stands in for it; in the ghost-only look the
  best that continues the word, as in round 1. A `fallback` popup is
  in place of the ghost, never beside it; `always` + a ghost axis that
  draws is both (the selected candidate's ghost with the popup).
- **Accepting a fuzzy match** replaces the word typed with the
  candidate (at the primary caret); one that continues the word types
  its rest at every caret, as the ghost did.
- **Where it hangs.** The caret's row leaves an empty mark, a line
  tall, at the word's start; the popup is a float anchored to that
  mark by key and declared outside the row, so none of its text is the
  line's (kui warns of a text deeper than four levels under a `line`
  row, and a query by the row would find it). It hangs left of the mark
  by its border, pad and icon column, so the labels start under the
  word. The flip reads where the mark was last frame (where its line
  falls when it is new) against the pane's rect: over the row when the
  room below is less than the popup and the room above more.
- **Its width** is its widest row's, measured once a build (kui gave a
  growing row in a fitting column no width), within the room to the
  pane's right; a detail is cut at half that room.
- **The panel shows documentation.** A candidate with a detail and no
  documentation has no panel — its row shows the detail already. Its
  text is built once as it lands, its head the detail, a code block's
  fences left out (round 2 draws the text plain, not rendered
  markdown). `completion docs` pins the panel for the word: each
  candidate walked to shows its own at once, until asked away. The
  panel follows the selection by where it came from and what it puts,
  so an answer landing in place of its source's earlier one keeps it.
  `completion docs` has no key (Decision 3 named none); `<C-f>` and
  `<C-b>` are local to a fact of their own, `completion_docs`, holding
  only while the panel is on show, so they are insert mode's own
  otherwise. `completion.docs` is a whole number or `false`
  (`SettingKind::IntOrFalse`, `:set completion.docs=false` read as
  `false`).
- **Masks.** A row's detail and the panel's text are masked — what the
  rules mask in the buffer being completed (`mask_text`) and any word
  inside a mask of an open buffer drawn as `•`; a masked candidate is
  no row (round 1 drops it from the ranking).
- **Signature help's room.** The panel is `completion_view::Panel` —
  a head and a body — which round 5's signature fills as the
  documentation does; nothing else draws a panel.
- **The icons are a cell, fitted.** Decision 9's box one cell wide is
  the editor's cell at the text's size, and a glyph is set so its
  advance fits it — Symbols Nerd Font Mono's glyphs are an em wide, so
  at the cell of a face half an em wide they are drawn at half the
  text's size, as kui's terminal fits a wide glyph to its cell (F120).
  kui needed no round: a span names a family (`Span::family`), and a
  box a cell wide with the glyph centred in it is plain layout; a
  glyph inside a paragraph cannot be centred on its line at a smaller
  size, so every icon kawoosh draws itself is a box of its own beside
  the text (`text_with_icons` for chrome text), but the markdown task
  box (below).
- **The sweep.** Moved onto `font.icons`: the kinds' codicons; a Lua
  icon's `glyph` part (`kawoosh.icon`, which also takes `cell = true`);
  every Private Use Area character in the title bar's and the status
  line's segments — `status.diagnostics` counts after codicons' error
  and warning (were `●` `▲`), the strip's marks Nerd Fonts' squares
  (were `▯▮▯`); the markdown task boxes, a span of their own in
  `font.icons` at the text's size, not a cell: each stands for the
  three cells of `[ ]`, and fitted to one they would be specks.
  Kept, as the user confirmed (Decision 9's amendment): the vector
  icons of icons.md Decision 1 — the tab's close, the chevrons, the
  folds, the done mark, the dots, the key caps — kawoosh's own
  interface, on the vector API; in no family and in boxes of their
  own, nothing a font does moves them. Typography stays
  text (`…`, `→` in a sentence, a grid's `│`, a field's `▏` caret),
  the theme lab's samples show the face, and a terminal's glyphs stay
  its program's.

Measured with `kawoosh/tests/completion.rs`'s
`timing_a_keystroke_and_a_frame` (dev profile, a 20,000-line buffer of
words, median / p90), the ghost alone (`popup = "never"`, round 1's
look) against the popup open beside it (`always`): a keystroke 0.60 /
0.68 ms → 0.63 / 0.73 ms, a word's start 2.42 / 2.65 ms → 2.56 / 2.78
ms, a frame 0.173 / 0.185 ms → 0.177 / 0.190 ms. Round 1 as merged,
on the same machine before any of this: a keystroke 0.63 / 0.67 ms, a
word's start 2.38 / 2.45 ms, a frame 0.171 / 0.182 ms.

**After a review and a load test, the same day.** A resolve's answer
fills the candidate it was asked for (`Session::resolving`), whatever
`<C-n>` selected in the frame it landed. A row's label and detail are
masked both ways, as the panel is, and a candidate whose label quotes
a masked word is no row. The mark the popup hangs from reports its
layout, so the flip and the room are read from where the caret's row
is (soft wraps, tall rows, headers); the popup moves left to stay
inside its pane and its panel goes to whichever side has room. The
window follows the selection down as well as up through a landing.
With neither a ghost nor the popup on show, nothing is asked and no
panel shows. The harness loads the shipped icons' family
(`fonts::load_symbols`), so the icons' tests pass on a machine without
it installed and read nothing of the home.

### Round 3, 2026-10-17: LSP snippets as selections

`systems/src/snippet.rs` (the grammar: parsed, expanded),
`kawoosh/src/snippet.rs` (the live snippet: its fields, its keys, its
ends, its variables), the editor's typing over a selection
for a field (`Editor::erase_selected`, `View::fill`) and its count of
moves through history
(`history_moves`), completion.rs's accept and the choice session,
`rows::Mark::underline_color` for the faint fields. Tests:
`kawoosh/tests/snippets.rs`, the grammar's in `snippet.rs`,
`fake_lsp.py --snippets`.

As designed: Decision 7 whole — `snippetSupport: true` (with
`insertReplaceSupport` kept, `insertTextModeSupport` added), an
accepted snippet kawoosh's own selections, a repeated field several
carets, `$0` the last caret, a choice's options the popup's, `snippet
next` / `snippet prev` on `<C-l>` / `<C-k>` local to a `snippet` fact
that holds only while one is live, `<Tab>` left alone, one at a time,
every end, nothing remembered, the fields not reached faintly
underlined.

Where the build departs, or says more than the decision did:

- **The grammar, read leniently.** LSP 3.17's whole, as VS Code reads
  it: tabstops, placeholders nested to any depth, choices (`\,` `\|`),
  `$0`, variables with a default or a transform, a field's transform
  (`${1/(.*)/${1:/upcase}/g}`, the format's cases and its `+ ? -`
  forms, the flags `g i m s` on Rust's regex — one that does not
  compile leaves the value), the escapes. A `$` that begins no
  construct is the text it is and the rest is read on from after it, so
  a body that is not written right is put as written around what is,
  never refused; nesting past 64 is text, an expansion visits at most
  100,000 nodes (fields whose defaults mirror each other double at each
  step), and nothing panics.
- **A snippet is matched by its label.** Round 1 matched a candidate
  by what it puts; a snippet's body (`#[test]\nfn ${1:name}() …`) is
  not what the user types, its label (`tfn`) is. It is offered when the
  label is typed whole, and dropped only when its plain text — each
  field its default — is what is typed. The plain text is what the
  dedup, the frecency and the ghost read; a ghost shows its first line
  alone.
- **Typing goes over a field's selections, and only those.** In insert
  mode a typed text, `<CR>`, a paste, `<BS>` and `<Del>` erase the
  selections first (the deletes, only that) while they are exactly the
  ones a field made (`View::fill`, dropped as insert mode ends, by an
  undo, and as the snippet ends). Made editor-wide at first, it typed
  over the selection an undo puts back (`vlld u iX` gave `Xlo world`),
  found by the review; a drag's selection is typed at its caret, as
  before.
- **Where it goes.** In place of the word typed: at every caret where
  the candidate continues the word, at the primary one otherwise (the
  first field's selections then replace the other carets). Each caret
  gets a copy; field N is its places in every copy, a caret each, and
  the snippet's text is the copies — a caret, or an edit, outside all
  of them ends it. The lines after the first are indented as the line
  it goes into and a line's leading tabs are the indent unit, unless
  the server sent `insertTextMode` 1 (`asIs`).
- **The fields follow the journal**, edit by edit: the field being
  filled, and the fields it is nested in, grow with what is typed at
  their edges; every other keeps out of it, so typing at a field's end
  never spills into the next. Which field holds which is the body's
  nesting, kept as each field's parent — not guessed from the ranges,
  which an emptied inner field shares with its outer one. A field
  nested in the one being filled whose place there the typing emptied
  is gone (VS Code's rule), and `<C-l>` passes it. Bytes throughout: a
  multi-byte character is one edit like another.
- **An undo or a redo ends it** (the call this round was left to make).
  In insert mode the snippet came in with the typing session an undo
  takes back whole, and what the history puts back is its own text,
  not a snippet's to follow; a redo brings back no snippet, nothing
  remembers one. `Editor::history_moves` counts every move through a
  history, so the snippet tells an undo from typing.
- **The ends, said exactly.** An edit outside it is one not wholly
  inside its text: a `<BS>` reaching before its start ends it too. A
  buffer switch is its view showing another buffer (`show_buffer`
  leaves insert mode as well), and another pane taking the keys ends
  it — the candidates picker over its pane excepted, as the
  completion's session is. An edge of its text is its own only where
  the field being filled touches it: a caret after its last character
  with no field there has left it, and typing on there ends it. Reaching
  `$0` ends it, the carets there (a
  `${0:text}` selected); a body with no field but `$0` puts the carets
  there and leaves nothing live; a body with no `$0` ends at its text's
  end.
- **A choice is the popup's whatever `completion.popup` says**: no
  ghost can offer it, and under `never` it was reachable only through
  `<C-x>`. Its options are a completion session of their own
  (`WordKind::Choice`): every option, in its order, until the field is
  typed in, then narrowed as a word is. Taking one puts it in every
  place of the field, the carets after it; the snippet stays on the
  field.
- **A transform's places are written when its field is left** by
  `<C-l>` or `<C-k>`, as VS Code writes them — not as the field is
  typed in, and not when the snippet ends any other way.
- **The variables** are VS Code's: `TM_SELECTED_TEXT`,
  `TM_CURRENT_LINE`, `TM_CURRENT_WORD`, `TM_LINE_INDEX`,
  `TM_LINE_NUMBER`, `TM_FILENAME`, `TM_FILENAME_BASE`, `TM_DIRECTORY`,
  `TM_FILEPATH`, `RELATIVE_FILEPATH`, `CLIPBOARD`, `WORKSPACE_NAME`,
  `WORKSPACE_FOLDER`, `CURSOR_INDEX`, `CURSOR_NUMBER`, `LINE_COMMENT`,
  `BLOCK_COMMENT_START`, `BLOCK_COMMENT_END`, the `CURRENT_*` dates and
  times (local, `localtime_r`), `RANDOM`, `RANDOM_HEX`, `UUID`. One
  nobody knows is its name in a field after the body's own (VS Code's
  rule). The texts that could carry a mask out are empty when they
  would: the selection, the line and the word in a private buffer or
  under a mask; `CLIPBOARD` — the register's newest, which the system
  clipboard is adopted into — when it is a secret's or holds a word
  inside any open buffer's mask or one a rule masks.
- **Drawn.** A field not reached yet with text in it is underlined in
  the dim colour (`Mark::underline_color`); an empty one (`$2`) has
  nothing to underline. The field being filled is the selections.
- **Left for later.** A candidate's `textEdit` range is still not read
  — the word typed is replaced, as since round 1; `additionalTextEdits`
  (an auto-import) are not applied — applied later, they land outside
  the snippet and must go in before it is live, or be carried.
  `completion.callable`'s `args` (round 5) and the user's snippets
  (round 4) come through the same door, `snippet_accept` (a body and
  the places it goes; `snippet_expand` at the carets).
- **Found on the way, not this round's:** `cc` on a buffer of one empty
  line (`"\n"`) puts its text after the newline. And round 2's sticky
  popup races a landing: in the `fallback` look a frame drawn while the
  ranking selects a candidate no ghost can show (before the server's
  answer lands) sets `Session::popup_up`, and the popup then stays for
  the word in place of the ghost the answer would give —
  `lsp.rs`'s `diagnostics_definition_hover_and_completion` fails so
  about one run in fifty under 48 parallel copies, on main as here.

Measured with `kawoosh/tests/snippets.rs`'s
`timing_a_keystroke_in_a_snippet` (dev profile, a 20,000-line buffer
of words, `gamm` typed after `f(` mid-line in the default look, so the
popup is up either way; median / p90, three runs): with no snippet
0.47–0.49 / 0.53–0.55 ms, inside a live snippet's field 0.48–0.50 /
0.54–0.55 ms — the fields carried through a key's edits cost nothing
measurable. A field in two places, each key typing at two carets, is
0.55–0.57 / 0.62–0.66 ms: what two carets cost.

**After a review and a load test, the same day.** Typing goes over a
field's selections only (`View::fill`): made editor-wide first, an
undo's restored selection was erased by the next key (above). A field
with only a transform in the body has no place and is no field — it
selected nothing, and the next key panicked. An edge of the snippet's
text is its own only where the field being filled touches it, so
typing on past its end ends it and `<C-l>` cannot reach back (the
LuaSnip complaint, which the first build still let through). Copies at
several carets pair their fields by number, not by place in the first
copy (a variable's default can hold a field one copy has and another
not), and each caret replaces its own word, measured there. An
expansion stops past 1 MiB (`snippet::MAX_TEXT`, a mirror's copies and
a variable's value counted) or the node budget and is too big: a
server's candidate is then its text, an accepted body is put as
written, nothing live. A choice field's session asks no source. Round
2's panel test checks "not before the delay" only while its frame ran
before the timer was due.

