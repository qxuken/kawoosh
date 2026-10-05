# Comments: `gc` + motion, `gcc` the line

Status: planned 2026-10-06 from the ask "Let's plan comment movements.
I used often `gcc` to comment out a block of code or a line", rounds 1
and 2 built the same day ("allright let's build it", "let's do round
2"). The calls below are
taken here, each the user's to overturn; where the build moved one,
the section says so. Roadmap step 90.
Companion to [editorconfig.md](editorconfig.md) (where a language's
ways live), [nodes.md](nodes.md) (the syntax's door) and
[keys.md](keys.md) (the letters).

## What there is

Operators are a registry kind (`Kind::Operator`, `commands.rs`
`operator`): one waits for a motion or a text object, doubles for the
line (`dd`, `>>`, `guu`), runs at once over a visual selection, and
`apply_operator` gets the ranges, one per selection, each marked
linewise or not. `indent` and `dedent` are the linewise shape to copy:
`op_lines` turns a range into its lines, the edits go through
`edit_each` as one undo step, `to_first_lines` leaves the caret. `.`
replays any operator for free, by the registry. `ga` shows how one
that needs more than the range waits for it.

A language's ways are `language.LANG.KEY` tables read through the
buffer's scope (editorconfig.md Decision 1): the session's `:set`
first, then `.editorconfig`, then the project's, user's and default's
`language` tables. The defaults for the built-in languages are set in
`settings.rs`; an installed grammar's row comes from the manifest
(grammars.md), which carries its files and nothing about its text.

The grammar's text objects already know a comment: `a/` `i/`
(`@comment.around`, `@comment.inside`). Nothing writes one. There is
no key under `gc`; `gb` is free too.

Vim has none built in; vim-commentary gave `gc` + motion, `gcc`, and
`commentstring` (`// %s`). neovim 0.10 built the same in (`gc`, `gcc`,
`gc` as a text object in operator-pending mode), the string from the
tree-sitter layer at the cursor when `commentstring` is set per
language. Comment.nvim adds `gb` for a block comment and puts the
tokens at the lines' least indent so they line up. helix has `<C-c>`
for the line, `<A-c>` for the block, tokens in `languages.toml`.

## Decisions

### 1. `gc` is an operator; `gcc` the line; the keys as neovim's

`gc` + motion or text object comments or uncomments every line the
range touches (`gcip`, `gcj`, `gca/`, `gc3j`); `gcc` the line, COUNT
lines (`3gcc`); `gc` over a visual selection, linewise whatever the
selection's shape, as `>` does. A `v` selection of two words on one
line comments the line. It is a toggle (Decision 2), so one key does
both and `.` after `gcc` undoes what `gcc` did on the next line, or
does it: `gcc` `j.` `j.`.

The command is `comment`, registered `Kind::Operator` beside
`indent`. The caret stays where it was in the line (its byte offset
kept through the edit, as `indent` keeps it); on a motion it goes to
the first line of the range's first non-blank, as `=` and `>` leave
it. The register is untouched.

Beaten: **helix's `<C-c>`** — Ctrl counts, and `<C-c>` is a terminal's
interrupt in the one pane that has a shell. **A text object `gc` in
operator-pending mode** (neovim's `dgc`): `a/` `i/` are the
grammar's, found from the tree rather than by scanning for tokens, and
work in every language with a query; `dgc` would be a second spelling
of `da/`. **`gb` for a block comment** (Comment.nvim): not now — a
block-only language gets its lines wrapped one by one (Decision 2),
which is what `gc` on a CSS rule should do, and a whole-range `/* */`
is a round of its own when use asks (Decision 7).

### 2. A toggle over the range's lines, decided for all of them at once

The lines of the range, blank ones (whitespace only) set aside. If
every line left starts, after its indent, with the line token: every
one is uncommented — the token taken off, and one space after it when
there is one. Else every line left is commented, blank lines skipped,
as vim-commentary and Comment.nvim do: a paragraph with a blank line
in it comments as one, and uncommenting it leaves the blank line as it
was. A range of only blank lines does nothing and says so.

Commenting puts the token at the **least indent** of the lines
commented, not at each line's own, so a block reads as a block and
uncommenting leaves the indentation it found:

```rust
    if a {
        b();
    }
```

```rust
    // if a {
    //     b();
    // }
```

The least indent is the longest common prefix of the lines' leading
whitespace, bytes not columns: a file that mixes tabs and spaces gets
the token where the lines agree, never inside a tab's width. The
token goes in with a space after it (`// `, `# `, `-- `); uncommenting
takes one space back out, and only one, so `//  two` keeps its second.
A commented line is recognised by the token alone — `//x` with no
space is a comment too, and uncommenting it gives `x`.

A language with a block pair and no line token (CSS, HTML, markdown's
`<!-- -->`) wraps each line: `/* ` after the least indent, ` */` at the
line's end; the line counts as commented when, after its indent, it
starts with the opener and ends with the closer. A language with
neither says `no comment token for text`.

Beaten: **each line toggled on its own** (vim-commentary's `gc` on a
mixed range comments the uncommented and uncomments the rest, in
effect): the common case is "comment this out, whatever is in it", and
a range with a commented line in it would then half-uncomment under
`gcc` on a visual block. neovim and Comment.nvim take the all-or-none
rule; so here. **Tokens at each line's indent**: the block then
reads as a staircase and `gc` twice on a mixed-indent block can move
nothing but still looks changed; the least indent is what every
plugin settled on. **A column rather than a byte prefix**: counting
columns means a tabstop, and a token inside a tab run is wrong in
every editor that shows the file.

### 3. The tokens are `language.LANG.comment` and `comment_block`

Two keys in the settings tree, read through the buffer's scope like
`tabstop`:

```lua
language = {
  rust  = { comment = "//", comment_block = { "/*", "*/" } },
  lua   = { comment = "--", comment_block = { "--[[", "]]" } },
  css   = { comment_block = { "/*", "*/" } },
  html  = { comment_block = { "<!--", "-->" } },
  nu    = { comment = "#" },
}
```

`comment` is the line token, a string without its trailing space:
the space after it is the operator's rule, not a key.
`comment_block` is a list of two strings. Both are
declared keys (`DOCS`), so a typo in a user's file is named by
`undeclared` as `tabstp` is, and `:set comment?` says which tier
answered (`default (language.rust)`). A session's `:set comment=#`
changes the buffer's for the session, as `commentstring` does.

The defaults for the twenty-seven built-in languages go in
`settings.rs` beside the `tabstop` profiles: `//` for c, cpp, go,
javascript, typescript, tsx, rust, jsonc, (scss when it comes);
`#` for bash, nu, python, toml, yaml, gitcommit, diff's none; `--`
for lua, sql; `;` for scheme; the block pairs where the language has
them (c-family `/* */`, lua `--[[ ]]`, html and markdown `<!-- -->`,
python none, bash none). json, text, regex, markdown_inline, jsdoc,
gomod (`//`): json has no comment in the standard — jsonc's `//` is
its own profile — so `gc` in a `.json` says so rather than writing
what the parser then rejects.

An installed grammar's tokens come with it: `comment` and
`comment_block` in its `grammar.toml` in kawoosh-grammars, through the
manifest (a `comment` field on each grammar's row, two optional lists)
into the row's language defaults when the install is listed, the same
way its extensions reach `detect`. A grammar whose `grammar.toml` says
nothing has none, and `gc` says so; the first pass through the
seventy fills the common ones (every C-descended `//`, every
shell-shaped `#`, the Lisps' `;`, the MLs' `(* *)`, haskell's `--`,
erlang's `%`, vim's `"`, latex's `%`, matlab's `%`, …) in one
kawoosh-grammars release, `r7`. A user's `kawoosh.language(name, t)`
takes the same two keys in `t`.

Beaten: **vim's `commentstring` with `%s`**: one string that has to be
parsed for the pair, and that cannot say "a line token *and* a block
pair" — Comment.nvim keeps two strings for that reason. **Reading the
tokens off the grammar**: a grammar's node is `line_comment` or
`comment` and its text is whatever was written; the opener could be
taken from the first comment node in the buffer, but an empty file
has none, and a buffer's one comment may be a block. Left as a
thought for a language with no token set, not done. **A table in Lua
only** (`kawoosh.comment.tokens[lang]`): then `.editorconfig`'s tiers,
`:set` and `:set KEY?` all miss it, and a user's override would be a
third mechanism for a per-language value when the settings tree is
the one there is.

### 4. The language is the layer's, where the tree has layers

A `<script>` in an HTML file holds JavaScript, and `gcc` on one of
its lines should write `//`, not `<!-- -->`; a fenced block in
markdown likewise; a Lua string in `kawoosh/lua` holding an SQL query
less obviously but the same. The operator asks the syntax's door for
the language at the range's first non-blank line's first byte — a new
method on `SyntaxObjects`, `language_at(id, buf, at) -> Option<String>`,
answered by the shell from the buffer's kept tree and its grammar's
`Injections` query (the `@injection.content` capture whose range holds
the byte, innermost wins, its language from `#set!` or the
`@injection.language` capture's text), `None` when there is no tree or
no injection there — and reads that language's `comment` through a
scope whose language is the answer's, the buffer's own sources kept.
One language per `gc`: a range that starts in the script and ends
past `</script>` is commented as the script's lines would be, which is
what the first line says it is.

This is round 2 (Decision 7); round 1 takes the buffer's language and
is right in every file of one language.

*Built 2026-10-06, with two calls the build added.* A layer whose
language has no token falls back to the buffer's: a JSDoc comment's
lines in a JavaScript file take `//`, not a refusal for `jsdoc`. The
layer's name is the registry's — an injection query's `js` or `sh` is
read as the language it is an alias of, so `language.javascript.comment`
answers for a `js` fence; a name the registry has no language for is
kept as said and, having no tokens, falls back. The lookup nests only
as deep as the grammars the trees have seen: the first layer's name
needs no grammar, a layer inside it the injected grammar at hand.

Beaten: **the language per line**: a range straddling an injection
would then write two kinds of token, and uncommenting it would have to
decide per line again — neovim's `gc` reads the layer at the cursor,
once. **Asking the draw thread's layers**: the shell's syntax thread
keeps the painted layers, but behind an answer that lags the text; the
indenter's `Trees` are caught up to the buffer before they are read,
and the injections query runs over the same kept tree in microseconds.

### 5. `:comment lines` is the command's spelling

`comment` is the operator (`gc`), and a command run from the command
line cannot be told from one a key ran — `:comment` alone leaves the
operator pending, as `:align` does. The lines' form has a name of its
own, `comment lines`: COUNT lines at the caret, or the selection's in
visual mode, at once. It is what `gcc` runs — `c` in operator-pending
mode is bound to it, and completes a pending `comment`; after any
other operator it does what `c` did there (`cc` changes the line, `dc`
nothing) — and what Lua and the socket have (`kawoosh ex comment
lines`). There is no `!`: the toggle has no force.
`kawoosh.buf.comment_tokens(buf?)` returns `{ line = "//", block = {
"/*", "*/" } }` as the scope reads them for the buffer, each absent
where the language has none. `{ at = }` for a byte's layer is not
built: Lua's `kawoosh.buf` reads a published snapshot, with no door to
the trees — the node API's door would serve, when a plugin asks.

*Built 2026-10-06 as written here; the note had said `:comment`.*

### 6. Each selection its own range, a line never twice

Under a multicursor, every selection's lines are toggled by Decision
2's rule for that selection — two cursors in two functions comment the
two functions, each judged alone, as `>` indents each. A line two
selections share is edited once (the first's say), as `indent` dedups
its lines. Visual mode ends on the operator, as for every operator but
the two that wait for a character.

### 7. Rounds

1. **The operator**: `comment` in `commands.rs` (`apply_operator`'s
   `"comment"` branch with Decision 2's rule, the keys `gc` and `gcc`,
   `:comment`), `comment` and `comment_block` declared and defaulted
   for the built-in languages, `kawoosh.buf.comment_tokens`, the
   help's `editing.md` and keys.md's `g` table, tests in
   `kawoosh/tests` (toggle on and off, least indent with tabs and
   spaces, blank lines inside, block-only language, no token, COUNT,
   visual, multicursor sharing a line, `.`).
2. **Injections**: `language_at` on the door, the shell's answer from
   the kept tree, `gc` in a `<script>` and a fence.
3. **The grammars' tokens**: `comment` in `grammar.toml` and the
   manifest, kawoosh-grammars `r7` with the seventy filled in, the
   built-in manifest copy following, `kawoosh.language` taking the
   keys.
4. **`gb`**, a block comment over the range as one pair, when use asks.

## Built

Round 1, 2026-10-06. `comment` is the operator and `comment lines` the
line form (`editor/src/commands.rs`: `comment_lines` plans the edits,
`apply_operator`'s `"comment"` branch runs them through `edit_keeping`
so a caret rides its text, then puts an extended selection on its
first line's first non-blank); `gc` in normal and visual mode, `c` in
operator-pending mode. `comment` and `comment_block` are declared bare
(`:set comment=#`, `undeclared` names a typo) and defaulted per
language in `settings.rs`; `Editor::comment_tokens_in` reads them
through the scope, an empty string taking a token away.
`kawoosh.buf.comment_tokens()` in `lua/src/lib.rs`. Help: `editing.md`
(a Comments section), `settings.md` (the profiles' table), keys.md.
Tests: `kawoosh/tests/comment.rs` (the toggle both ways and the caret,
a count and a motion over mixed indents with a tab, one space taken
on the way back, a block-only language, no token and only blank lines,
`.` with `cc` and `dc` unchanged, two carets sharing a line, the
command-line spelling, the session's token),
`kawoosh/lua/tests/buf_comment_tokens.lua`.

Round 2, 2026-10-06. `SyntaxObjects::language_at` (default `None`),
answered by `Trees::language_at` in `kawoosh/src/indent.rs` over the
kept tree: `ts::injection_at` (the innermost `@injection.content`
holding the byte, its language from the `#set!` or the capture's
text, `injection.combined` passed over as the painter does), then
`ts::parse_range` into the injected grammar where the trees have it,
`INJECTION_DEPTH` deep. The ts thread now keeps a tree for a grammar
with an injection query too (markdown's, javascript's), not only one
with indents or text objects. `Trees::set_names` holds the registry's
names and aliases, synced at start and on every language added
(`sync_language_names`). `Editor::comment_tokens_at` reads the layer's
`comment` and `comment_block` through a scope with the layer's
language and the buffer's own sources, falling back to the buffer's
(Decision 4's build note); the operator asks at the first non-blank
line's first character. Tests in `kawoosh/tests/comment.rs`: a rust
and a `js` fence in markdown against the prose's pair, a range
starting in the fence, JSDoc's fallback, C in `ffi.cdef`. Rounds 3–4
open.
