# Completion: when it asks, how it shows, what it offers

Status: designed 2026-10-16/17 with the user, over six rounds of
questions, from the todo's "add autocompletion settings (including
disabling)". Not built. The decisions below are the rounds' answers,
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
