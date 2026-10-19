# Selections, helix's

Status: decided 2026-09-27 (roadmap step 51), the keys at the user's
word. Asked in the todo as "how to multiselect all? it seems like we
need to review and take helix multiselect movements and commands".
*Built 2026-09-27.*

## Where kawoosh stands

The engine is a selection set (mvp.md Decision 4) and keys.md's
selection cluster already covers much of helix's multiselect: a caret
on the line below and above (`<C-j>` `<C-k>`, helix's `C` `<A-C>`), a
caret on every line of a selection (visual `<C-j>`), the next match
and every match of the word or selection (`<C-n>` `<C-S-n>`, helix's
`*` then `n`), keep the primary (`,`), rotate it (`(` `)`), select all
(`<D-a>`, helix's `%`), and the syntax node's `<A-o>` `<A-i>` `<A-n>`
`<A-p>`.

What helix has and kawoosh does not is the part that works *inside*
selections by a pattern: `s` (the matches inside every selection
become the selections — `%s` is "every match in the file", `vips` "in
this paragraph"), `S` (split every selection on a pattern), `K` and
`<A-K>` (keep, or drop, the selections that match), `<A-s>` (every
line of a selection its own selection), and `<A-,>` (drop the primary).
`<C-S-n>` is every match in the buffer and never within the selection.

## Decisions

### 1. Five commands, each a prompt previewed as it is typed

| command | what | helix |
|---|---|---|
| `select within PATTERN` | the matches inside every selection become the selections | `s` |
| `select split PATTERN` | every selection split on the pattern: the pieces between matches | `S` |
| `select keep PATTERN` | the selections whose text matches stay; `!PATTERN` keeps those that do not | `K`, `<A-K>` |
| `select lines` | every line of every selection its own selection, its newline off | `<A-s>` |
| `select drop primary` | the primary selection gone, the one before it primary | `<A-,>` |

Bare, the first three open a prompt (`select `, `split `, `keep `) over
the view; as the pattern is typed the view shows what `<CR>` will make,
the way `/` previews its match, and `<Esc>` puts the selections back.
A pattern is read as `/` reads one (a Rust regex, case ignored; `(?-i)` to match it). The prompt
walks the search's history, and the search itself is left alone: `n`
goes on with what it had. With an argument (`:select within foo`) the
command runs at once. A pattern that matches nothing says so and
changes nothing — helix's rule, since an empty selection set does not
exist.

Keep and drop are one command, `!` saying which, where helix has two
keys: one prompt, and the vim hand knows `!` as "not" from `:g!`.
helix's `C` is not built: it is `<C-j>`.

### 2. The keys: a `<leader>v` group from visual mode

| keys | command |
|---|---|
| `<leader>vs` | `select within` |
| `<leader>vS` | `select split` |
| `<leader>vk` | `select keep` (`!pat` drops) |
| `<leader>vl` | `select lines` |
| `<leader>ve` | `select every` (Decision 4), from normal mode too |
| `<leader>v,` | `select drop primary`, from normal mode too (the carets `<C-j>` makes) |

`<D-a><leader>vs` is helix's `%s`; `vip<leader>vs` its `vips`.

*Beat: helix's letters in visual mode.* `s`, `S` and `K` are vim's
there (`s` `S` the same as `c` `C`), and the user's hand uses `Vs`
(roadmap step 45 fixed it); keys.md's rule is that no vim letter is
shadowed. *Beat: Alt chords.* `<A-K>` is the pane's resize
(`<A-S-k>`), the Alt cluster is "this one selection's shape and place"
while these change how many there are, and the letters left over would
be helix's only in part. The leader group costs a key and shows in the
which-key (`v`: selections), and nothing a vim hand does changes. The
pick was the user's, 2026-09-27.

### 3. What the new set is

Characters, in visual mode (`V` is left: a match is no line), each
selection over its match as a visual one lies — the head on its last
character — so an operator takes exactly the match. The primary is the
first of the new selections inside the old primary, else the first.
Empty matches (`^`, `a*` on nothing) are no selections; `select lines`
skips an empty line.

### 4. Every Nth selection: `select every N`

Asked 2026-10-19: a pasted list of title and value lines, the titles
to go and the values to be joined — "how can i multiselect all lines and
reduce selection to each second". `select keep` does it only when the
titles share a pattern; `:g` and `:norm` do not exist, so a macro was
the only way.

`select every N` (`<leader>ve`) keeps the primary and every Nth
selection counted from it, before it and after, and drops the rest; the
primary stays primary. N is the argument (`:select every 3`), else the
count (`3<leader>ve`), else 2. There is no offset argument and no
negated form: the primary is the offset, and `(` `)` already move it, so
`ggVG<leader>vl<leader>ve` keeps the titles (`select lines` leaves the
first primary) and `)` before `<leader>ve` the values. Empty lines are
no selections (Decision 3), so a blank line between pairs leaves the
alternation alone. Then `Vd` takes the kept lines whole and `ggVGJ`
joins what is left.

*Beat: helix, Kakoune, VS Code.* None of them has it built in;
vim-visual-multi has no such command either, as far as the round knows.
The leader group had `e` free and keeps it off vim's letters.

## Built

`editor/src/commands.rs` (`select_within`, `select_split`,
`select_keep`, `select_lines`, `select_every`, `select_drop_primary`, and
`sel_ranges`, what a visual selection covers), the prompt as
`Prompt::Select` in `editor/src/lib.rs` (previewed by
`preview_select`), the keys in `default_keymap`. Tests:
`kawoosh/tests/normal_mode.rs`' `selections_within_split_keep_lines_drop`
and `select_every_keeps_every_nth_from_the_primary`.
