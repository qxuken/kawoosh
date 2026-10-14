# Substitute: `:s` previewed, over the selection, with many carets

Status: written 2026-10-14 from three todo items — "replace with change
preview", "replace across selection without multicursor?", "replace
across multicursor does strange things with selection" — "replace"
being `:s`, and the preview neovim's `inccommand`: what the line would
do, shown in the buffer as it is typed, nothing done until `<CR>`. The
calls below are taken here, each the user's to overturn. Companion to
[selections.md](selections.md) (the selection set `:s` works over) and
[search.md](search.md) (the project search's replace, which has its own
preview: the results pane).

## Where kawoosh stood

`:[range]s/pat/rep/[gi]` (`editor/src/commands.rs`, `substitute`) finds
its matches on every core and applies them as one edit, one `u`. Its
range: none for the lines every selection touches, `%`, `N,M`, and
`'<,'>` for the lines the selections cover. The prompt previewed `/`,
`?` and the `select` commands as they were typed, and not `:`.

What was wrong, found with the engine's tests:

- **The carets after.** The primary went to the last changed line's
  start (vim's rule) while every other selection followed its text,
  each end mapped on its own. Three carets on `two` in `one two` ×3,
  `:s/one/1/`: the first two on their `two`, the primary at its line's
  start — the next `ix` typed `1 xtwo`, `1 xtwo`, `x1 two`. From
  visual mode (`<C-n>` thrice, then `:s`) the mode ended but the other
  selections stayed wide in normal mode, and one whose text was
  replaced folded to an empty one at its start.
- **The scope.** From visual mode `:s` (no range) took the lines the
  selections touch: with `<C-n>`'s selections on the second `foo` of
  `foo x foo`, `:s/foo/bar/` replaced the *first* — the one not
  selected. There was no way to say "in the selection", vim's `\%V`.

## Decisions

### 1. The preview is drawn over the text, never applied

While the command line reads as a substitution, the view it is over
shows what `<CR>` will do:

- **The pattern alone** (`:s/foo`): its matches in the range are washed
  as search hits are — only those the line would take, the first per
  line without `g`, inside the range — and the old search's hits are
  not drawn in that view meanwhile.
- **With the replacement begun** (`:s/foo/`, the second delimiter
  typed): each match struck through in the danger colour and what
  replaces it drawn right after it in the success colour — `foo` →
  ~~foo~~`bar` — a diff of the line, an empty replacement a strike
  alone. A newline in the replacement is drawn `⏎`.

Nothing else moves: the carets, the search `n` walks, the undo
history, the modified flag — and the scroll only as Decision 5 takes
it. `<Esc>` closes the prompt and the paint goes with it. A pattern that does
not compile yet (`:s/(a`) previews nothing. Only the lines drawn are
worked out, each frame, as the search's hits are: a `:%s` over a large
file costs what is on screen.

*Beat: neovim's apply-and-undo.* Its `inccommand` runs the substitution
in the buffer on each key and undoes it on the next. Here an edit is an
event many things hear: the journal that carries paints, marks and
diagnostics, the language server's `didChange`, the multibuffers
mirroring the file, the VCS hunks, the moments, the undo tree and the
modified flag. Applying and reverting per key sends a server two
changes a keystroke and churns all of that, and "`<Esc>` leaves no
trace" becomes a reversal to get exactly right rather than a fact.

*Beat: the replaced text alone, neovim's look.* It needs the row to
hide the match and draw other text in its place, and a row's cells are
the buffer's bytes — the carets, the wrap, the sideways scroll measure
them. The diff look needs only what the rows already have (a mark that
strikes, and virtual text as the inlay hints and the completion's ghost
are drawn), and it shows what goes as well as what comes, which is the
point of previewing a regex.

*Not drawn:* in a row that wraps (soft wrap, rendered markdown) virtual
text is not drawn — the inlay hints and the ghost are not either — so
there the matches are struck and the replacement is not shown (kept
so, the user's call of 2026-10-14).

### 2. From visual mode `:` is `:'<,'>`, and `'<,'>` is the selections

`:` in visual mode opens the line with `'<,'>` on it, vim's habit, and
`'<,'>` names the selections as they are: `V`'s their whole lines, `v`'s
their characters, a match taken only when it lies inside one (vim's
`\%V` folded into the range) — each selection its own place for the
first-match rule without `g`. An empty selection (a caret) is its line.
Erasing the `'<,'>` gives the range none means: the lines the
selections touch, as before.

`'<,'>` before any other command is dropped: every command acts on the
selections already (`:'<,'>select within x`, `:'<,'>align =`). Before
a shell line it is refused: `:'<,'>!sort` is vim's filter of the lines
through the command, not built, and run as a bare `:!` it would be a
pane waiting on input nobody gives it.

*Beat: vim's `'<,'>` as lines, with `\%V` in the pattern.* Nobody types
`\%V`, and the regex crate has no such assertion. In vim the selection
is gone once `:` is typed and `'<,'>` remembers its lines; here the
selections are still there while the prompt is open (kawoosh's model is
the selection set, helix's), so the range can be exactly them.
*Beat: no `'<,'>`, with no range meaning the selections.* The rule
would be invisible; on the line, the range says what `:s` will touch,
and the preview shows it.

### 3. Each caret follows its text; a lone one goes where vim's does

After `:s`, one caret lands at the start of the last line changed —
vim's. Several each land at the start of where their text went (the
primary too, so the set keeps its shape), and from visual mode each
selection becomes a caret at the start of what it covered — normal
mode, which `:s` ends in, with no selection left wide.

*Beat: always following the text.* A `:%s` with one caret then leaves
it where it was, which is no vim hand's habit. *Beat: always vim's
line start.* Several carets cannot all go there.

## Built

2026-10-14, the three a commit each. `editor/src/commands.rs`:
`SubstitutePlan` (the scope from `substitute_ranges`, the regex, the
template; `edits` over a window of lines), which `substitute` applies
and the preview reads, and the carets placed after. `editor/src/lib.rs`:
`preview_substitute` keeps the plan while the command line reads as a
`:s` (`parse_substitute_typed` says whether the replacement is begun);
`substitute_preview(view, lines)` is what a pane asks for the lines it
draws; `execute` drops `'<,'>` before other commands. The pane
(`kawoosh/src/panes.rs`) draws the matches as hits or strikes and
`rows::LineDraw::inserted` the replacements; the command line completes
past `'<,'>` (`kawoosh/src/cmdline.rs`). Tests: `editor/tests/modal.rs`'
`substitute_keeps_every_caret_where_its_text_went`,
`substitute_from_visual_mode_is_over_the_selection`,
`substitute_is_previewed_as_it_is_typed`; `kawoosh/tests/cmdline.rs`'
`a_substitution_is_previewed_in_the_pane_as_it_is_typed`. Decision 4:
`substitute` says and leaves visual mode for `substitute_or_say`'s
error (`a_failed_substitute_from_visual_mode_ends_in_normal_mode`).
Decision 5: `SubstitutePlan::tally` (bounded, by windows of lines),
`preview_substitute` scrolling and `unscroll_prompt`, the pane's
follow held by `substitute_previewing`, the count in `command_line`
(`substitute_preview_goes_to_the_first_match_and_counts`,
`a_substitution_out_of_sight_is_shown_and_counted`).

### 4. A failed `:s` from visual mode ends in normal mode too

The user's call, 2026-10-14. A `:s` from visual mode that changes
nothing — no match, a pattern that does not compile, a read-only
buffer — ends in normal mode as vim's does, each selection a caret at
its start as one that changed something leaves several, and the
message says why. Before, it stayed in visual mode with the selection
up, so the next key acted on a selection the user had already sent to
`:`.

### 5. Out of sight, the view goes to the first match; the strip counts

The user's call, 2026-10-14, on what Open asked. While a `:s` is typed
and none of its matches in the range is in sight, the view is taken to
the first one, its line in the middle as a far jump puts it — the feel
of `/` moving to its match as it is typed. It is the view's scroll that
moves, not the caret: the caret is what `.` and a bare range read, so
moving it would change the range being typed. Each key puts the scroll
back first and decides again, so a line that has a match in sight once
more returns the view to where it was, and `<Esc>` and `<CR>` put it
back too (`<CR>` then moves the caret as Decision 3 says, and the view
follows that). Meanwhile the view does not follow its caret: the caret
left behind would pull it back on the next key.

The command line's strip says, at its right end and dim, what `<CR>`
would change: "3 changes on 2 lines", "no match". It is counted once a
key, not a frame, a window of whole lines at a time from the range's
start, and stops at 10,000 changes or 8 ms, whichever comes first,
saying "10000+ changes" (or as many as it reached, with the `+`) — a
`:%s` over a big file is never counted whole on every key. The same
walk finds the first match the view is taken to.

*Beat: the message line.* While the prompt is open the strip is the
prompt's and the message is not drawn; a count put in the message
would be unseen until the prompt closed, and would overwrite what the
message last said. *Beat: the completion's ghost after the caret.* The
ghost is text `<Tab>` takes into the line; a count there would read as
something to type. The strip's end is where the project search's bar
puts its count beside its field, so the eye already looks there.

## Open

- The replacement in a wrapping row (Decision 1's *not drawn*): it
  wants the wrapped row's text to carry virtual spans, which its caret
  and `gj` placement read back by byte.
