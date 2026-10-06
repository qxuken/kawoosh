# Editing

How text is edited: the modes, where kawoosh differs from vim, multiple
selections, surround and align, node actions, repeating and macros, and
where yanked text goes.

## Modes

| mode | enter with | what |
|---|---|---|
| normal | `<Esc>`, `<C-c>` | keys are commands |
| insert | `i` `a` `I` `A` `o` `O`, `s` `S` `C`, `c` + motion | keys type text |
| visual | `v` | motions extend the selection |
| visual line | `V` | the same, by whole lines |
| operator-pending | `d` `c` `y` `>` `<` `=` `gu` `gU` `g~` `gc` `gb` `gsa` `ga` | waiting for a motion or a text object |

An operator doubled works on lines, as in vim: `dd`, `yy`, `cc`, `>>`,
`==`, `guu`, `gUU`, `g~~`, `gcc`, with a count for more lines. `<Esc>` in
normal mode backs out of whatever is open, one step a press: a pending
operator, then the extra cursors, then the search highlight.

Wherever text is typed — insert mode, the command line, a pane's field
— the keys a Mac edits a line with work as well as vim's:

| keys | what |
|---|---|
| {{mac:`<A-BS>` `<A-Del>` (⌥⌫ ⌥⌦)}}{{pc:`<C-BS>` `<C-Del>`}} | delete the word before the caret (where `<C-w>` stops), the word after it |
{{mac:| `<D-BS>` `<D-Del>` (⌘⌫ ⌘⌦) | delete to the line's start (as `<C-u>`), to its end |
}}| `<A-Left>` `<A-Right>` | the caret a word back, past the end of the word |
| `<D-Left>` `<D-Right>` | the caret to the line's start, its end (as `<Home>` `<End>`) |

On Windows and Linux the word keys are Ctrl's: `<C-BS>` `<C-Del>`
`<C-Left>` `<C-Right>`. None of them reaches past its line.

## Indentation

`<CR>`, `o` and `O` open a line at the indent the language's syntax
says — inside a `{`, after Python's `:`, out again after a `return` —
following the lines around it, so a two-space file stays two. `=` puts
lines where the syntax says: `==` one, `=ip` a paragraph, `gg=G` the
file. A language without indent rules keeps the line's indent, a level
deeper after an opening bracket, and `=` says it has none. A language
added with `kawoosh.language` takes an `indents.scm` beside its other
queries, in helix's dialect.

## What differs from vim

Most of vim's letters mean what they always did. The exceptions:

| keys | here |
|---|---|
| `U` | redo, like `<C-r>` |
| `;` | repeats the last `f` `t` `F` `T` **across lines**, so `f=` then `;;;` walks every `=` in the file |
| `,` | keeps the primary selection and drops the rest (there is no reverse `;`) |
| `S` | changes the line, keeping its indent |
| `s` `x` in `V` mode | change, delete the selected lines |
| `x` on a line's end | the caret can stand past the last character, on the line break (`$` then `l`, `j` onto a shorter line, an empty line), and `x` there deletes it: the next line joins on as it is, no space put in (vim's `gJ`). A count stops at the line's end, so `9x` on the text never joins. A visual selection over the break takes it too |
| `zz` `zs` `ze` | place the focused column of a [strip](panes.md#the-scrolling-strip), not the cursor line |
| `-` `_` | open the file's directory, the working directory, as a listing ([files](files.md)) |
| `<CR>` | on a `path:line` in the text, opens it |
| `m` `'` `` ` `` | marks as vim's, but a capital letter is the workspace's, across files; `]'` `['` walk the marked lines, `<leader>'` lists them |
| `<C-o>` `<C-i>` | the jumps are the tab's, not the window's, and a big move is one whatever made it (below) |

Missing on purpose: named registers (`"a`) — the [memory](memory.md) is
what they were for; only `"_`, the black hole, is there, to delete or
change without keeping the text — and visual block `<C-v>`, whose job `<C-j>` in
visual mode does with carets (below).

## Motions and text objects

Beside vim's `hjkl`, `w` `b` `e` `ge`, `0` `^` `$`, `gg` `G`, `f` `t`,
`%`, `n` `N` `*` and `/` `?`:

| keys | what |
|---|---|
| `gh` `gl` | the line's first non-blank, its end (helix) |
| `W` `B` `E` `gE` | WORDs: runs that only whitespace ends, like `a.b(c)` or a path |
| `}` `{` | the blank line after, before the paragraph — also after an operator (`d}`) |
| `%` | off a bracket, on a keyword of a block that a word closes (Lua's and Ruby's `end`, a shell's `fi` `done` `esac`): the block's next keyword — `if` to `elseif` to `else` to `end` and round, by the syntax; `d%` takes both keywords whole |
| `]f` `[f` | the start of the next, previous function, by the syntax — also after an operator (`d]f`) |
| `H` `M` `L` | the pane's top, middle, bottom line |
| `<C-d>` `<C-u>` `<C-f>` `<C-b>` | half a screen, a screen |
| `]<Space>` `[<Space>` | add an empty line below, above, the caret staying |
| `<C-a>` `<C-x>` | add to, subtract from the number under or after the caret, per selection |

Text objects follow `i` (inside) or `a` (around): `w` word, `W` WORD,
`p` paragraph, `(` `)` or `b`, `[` `]`, `{` `}` or `B`, `<` `>`, and the
quotes `"` `'` `` ` ``. So `ciw`, `da(`, `yi"`, `vip`. When the object is
not there, the operator does nothing — nothing is yanked over the
register.

A quote object is read on the caret's line: the string the caret is in,
else the next one on the line. Where the line leaves a string open — a
terminal's output wrapped in the middle of one, a string written over
several lines — its other end is found up to a hundred lines away.

The syntax adds its own, read from the language's grammar: `f` a
function, `c` a class (a struct, an enum, an impl, an interface — the
type), `a` an argument, `/` a comment, `T` a test, `e` an entry (an
array's element, a table's pair). `if` is a function's body wherever in
the function the caret is, `af` the whole of it; `ie` is the key or
the value of a pair, whichever the caret is on. `daf` takes its lines
whole when it has them to itself, and `daa` takes the argument with its
comma — the last of a list across lines the comma before it. A count
is the one further out (`2daf` from a closure is the function around
it), `vaf` again grows to the next one out, and every caret takes its
own. With the caret before any, the first after it on the line is
taken (`cia` on the `(`); none on the line, nothing is. `]f` `[f` go to the functions'
starts. Rust, JavaScript, TypeScript, Go, Lua, Bash, Nushell, C, C++
and Python have them; JSON, TOML and YAML their entries (and comments),
SQL its comments. A grammar installed with `:grammar install` has them
when it brings a `textobjects.scm`.

`:s/PAT/REP/g` replaces on the lines every selection touches (the
caret's line, with one caret), `:%s` in the whole file, `:N,Ms` on lines N
to M. Searching is covered in [search](search.md).

## Jumps

`<C-o>` goes back to where you were before a jump, `<C-i>` forward
again; a count goes that many places. A jump is any move that takes the
caret into another buffer or a screen or more away — a `50j`, a list's
`<CR>`, a definition, a plugin's move — and `gg` `G` `:N` `n` `N` `*`
`%`, `<A-u>`, a search, a mark, a pick from the picker are jumps however
near. Paging (`<C-d>` `<C-f>`), an edit that carries the caret (an undo,
a paste), and moving the keys to another pane are not.

The list is the tab's: a jump made in one pane and a jump in another
are one trail, and going back to a place goes into the pane it was left
in — switched back to its file if it shows another now — while that
pane is still in the tab; a file closed and deleted since is stepped
over. Going back and then jumping somewhere new
drops the places that were ahead, as a browser does. `<leader>mj`
(`:jumps`) lists them in the [memory](memory.md) pane, how many
`<C-o>` away each is; a session keeps each tab's list.

## Case

| keys | what |
|---|---|
| `gu` `gU` `g~` + motion | lower-case, upper-case, flip the case of what the motion covers |
| `guu` `gUU` `g~~` | the same for the line |
| `~` | flip the case of COUNT characters and step past them |
| `u` `U` `~` in visual mode | lower, upper, flip the selection (`u` is not undo there) |

## Multiple selections

The editor always holds a set of selections; one cursor is a set of one.
Every command acts on each selection, so inserting, deleting, operators,
`<C-a>` and surround all work at every caret at once. The rule of thumb:
**Ctrl counts them, Alt moves one.**

| keys | what |
|---|---|
| `<C-j>` `<C-k>` | add a caret on the line below, above (also `<C-Down>` `<C-Up>`) |
| `<C-j>` `<C-k>` in visual mode | a caret on every selected line, at the same column |
| `<C-n>` | select the word under the caret; again, the next match too{{mac: (`⌘d`)}} |
| `<C-S-n>` | select every match at once{{mac: (`⌘⇧L`)}} |
| `,` | keep only the primary selection |
| `(` `)` | make the previous, next selection the primary one |
| `<Esc>` | in normal mode, drop the extra carets |
| `<A-j>` `<A-k>` | move each selection's lines down, up — in insert mode too |
| `<A-h>` `<A-l>` | on lines: dedent, indent, keeping the selection; in `v` mode: drag the text a column left, right |
| `<A-o>` `<A-i>` | select the syntax node under the caret, then the one around it; back in |
| `<A-n>` `<A-p>` | the next, previous sibling node |
| `<A-u>` | the caret up to the start of the node around it, one more each press; in visual mode the head goes |
| {{mac:`⌘a`, }}`<C-S-a>` | select the whole buffer (`:select all`) |
| `o` in visual mode | swap the selection's ends |

The primary selection is drawn solid, the others washed. `<C-n>` also
sets the search, so `n` carries on from wherever the caret is.
`V<A-l><A-l><A-j>` indents a block twice and moves it a line down in one
gesture.

### Selecting inside selections

Helix's pattern tools live under `<leader>v` in visual mode. Each opens a
prompt that previews the result as you type; `<Esc>` puts the selections
back.

| keys | what |
|---|---|
| `<leader>vs` | the matches of a pattern inside every selection become the selections (`:select within`) |
| `<leader>vS` | split every selection on a pattern (`:select split`) |
| `<leader>vk` | keep the selections that match; `!pattern` keeps those that do not (`:select keep`) |
| `<leader>vl` | every line of every selection its own selection (`:select lines`) |
| `<A-,>` | drop the primary selection, from normal mode too (`:select drop primary`) |

{{mac:`⌘a<leader>vs`}}{{pc:`<C-S-a><leader>vs`}} then a pattern selects every match in the file;
`vip<leader>vs` every match in the paragraph.

## Putting over a selection

In visual mode `p` replaces the selection with the register and puts
what it replaced in the register, so a second `p` elsewhere swaps them
back. `P` keeps the register, to paste one text over many places.

## Surround and align

| keys | what |
|---|---|
| `gsa` + motion + char | wrap what the motion covers: `gsaiw)` wraps a word in `()` |
| `gsa` + char in visual | wrap the selection: `viwgsa"` |
| `gsd` + char | take the pair off from around the caret: `gsd"` |
| `gsr` + char + char | swap one pair for another: `gsr)]` |
| `ga` + motion + char | line the lines up on their first char: `gaip=`, `Vjga:` — on each line only inside what is covered, so a selection on each line (`<C-j>`, then `vi{`) lines up what is inside the braces |
| `ga` + motion + `*` + char | every one of the char, each a column in turn: `ga*=` on lines of `a = { b = 1 }` |
| `ga` + motion + count + char | the Nth alone: `ga2=` |
| `ga` + motion + `<CR>` | a pattern instead, asked for: `gaip<CR>or_else<CR>`; `:align PATTERN`, `:align * PATTERN`, `:align 2 PATTERN` over the selection |

Either bracket of a pair names it (`(` or `)`), `b` and `B` are round and
curly, and any other character wraps with itself on both sides.

## Comments

| keys | what |
|---|---|
| `gc` + motion | comment the lines it covers out, or back in when every one is a comment: `gcip`, `gc3j`, `gca/` |
| `gcc` | the line; `3gcc` three |
| `gc` in visual | the selection's lines, whole |
| `gb` + motion, `gbc`, `gb` in visual | the same with the block pair as one: `/* … */` around the whole range, taken off again when it is one; `gbiw` a word |

One rule for the whole range: if every line (blank ones aside) already
starts with the token, all are uncommented; otherwise all are commented,
blank lines skipped. The token goes at the lines' least indent with a
space after it, so a block reads as a block and comes back as it was:

```rust
    if a {          // gcc on each, or gc2j once:
        b();        //     // if a {
    }               //     //     b();
                    //     // }
```

Uncommenting takes the token and one space. The token is the language's
`comment` setting (`//`, `#`, `--`), with `comment_block` the pair where
there is one; a language with only the pair (CSS, HTML, markdown) wraps
each line: `/* color: red; */`, and `gb` needs the pair (`gb` in Python
says so). `:set comment=#` changes a buffer's for
the session, `language.NAME.comment` in your settings for good, and
`kawoosh.buf.comment_tokens()` reads them from Lua. Where the syntax
says a line is another language's — a `<script>` in HTML, a fenced
block in markdown — the token is that language's. `:comment lines` is
`gcc` by name. `.` repeats, so `gcc` `j.` `j.` walks down a file.

## Node actions

`g.` does what the syntax node under the caret means:

| on | `g.` |
|---|---|
| `true`, `false` | flips it |
| `==`, `&&`, `<`, `and`, … | its counterpart: `!=`, `\|\|`, `>`, `or` (lua's `~=`) |
| anywhere in a list — arguments, parameters, an array, an object, a table | one item a line if it is on one line, back on one line if not, with the trailing comma the language's formatter writes |
| a string | its quotes: `"` → `'` → `` ` `` in javascript and typescript, `"` ↔ `'` in python and lua |
| a number of five digits or more | `1000000` ↔ `1_000_000` |

The innermost node one of them changes is the one changed, so on `true`
inside a call's arguments it flips, and on any other argument it splits
the list; it does not reach past the body of the function, loop or
closure the caret is in. It works at every caret as one change, `u`
undoes it at once and `.` does it again. `:node actions` lists every
one there is from the caret up in a picker, the outer lists too;
`:node action split 2` runs the second list up's split. Turn one off
with `node_actions = { quotes = false }` in [the settings](settings.md),
or add your own from Lua ([lua](lua.md#the-syntax-tree)).

## Auto-pairs

Typing an opening bracket or quote types its closer with the caret
between; typing the closer steps over it; `<BS>` between a pair deletes
both; `<CR>` between brackets opens the block. Quotes pair only where a
quote can start. It works at every caret. Set `pairs.enabled = false` to
turn it off, or change the pairs per language with `pairs.rules` — see
[the settings](settings.md).

## Repeat and macros

| keys | what |
|---|---|
| `.` | the last change again, on the selections as they are now; a count replaces its count |
| `q` + letter … `q` | record a macro into that register; a capital letter appends to it |
| `@` + letter | play it, COUNT times |
| `@@` | the macro played last |
| `@:` | the last command line again |

While recording, the status line says `REC @a`. A macro records commands,
not raw keys, so it keeps working after you remap a key, and it runs to
the end rather than stopping on a failed motion: `100@a` is how to run
one over a whole file. Search and `:` lines are recorded too.

## Yanks, puts and the memory

`y`, `d` and `c` put the text in the register and on the system clipboard,
and text copied in another program becomes the register when you come
back to the window — so `p` pastes it. In insert mode {{mac:`⌘v` (`<C-S-v>`)}}{{pc:`<C-S-v>`}}
pastes the clipboard; `<C-S-u>` deletes the whole line into the register
without leaving insert mode. As in vim, insert's `<BS>`, `<Del>`, `<C-w>`
and `<C-u>` — and the {{mac:⌥ and ⌘}}{{pc:Ctrl}} deletes above — leave the register and
the clipboard alone; `x` and `X` are
deletes like `dl` and `dh` and fill them.

Every yank and delete is also kept in the **memory**, newest first:

| keys | what |
|---|---|
| `[p` `]p` | right after a put: swap it for the older, newer text in the memory |
| `<leader>mm` | the memory pane: every text, to put again (`:memory`) |

Texts survive a restart for a few days. See [memory](memory.md).
