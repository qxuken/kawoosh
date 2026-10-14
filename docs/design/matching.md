# Fuzzy matching: terms split at a space

Status: decided and built 2026-10-14. Asked as "file search should
fuzzy find with spaces like (carnet main should match
roles/carnet/tasks/main.yml)". The matcher is `lua/src/fuzzy.rs`
(`kawoosh.matcher`, `kawoosh.fuzzy`): fzy's scoring in Rust since
roadmap step 4, the list crossing the Lua boundary once and a query
answering the best two hundred with positions. Each decision keeps the
alternative it beat.

## What there was

A needle was one run of characters to find in order, its spaces among
them: `carnet main` asked for a space between `t` and `m`, which no path
has, so it found nothing. The user typed a space the way fzf, telescope
and zed have taught: as "and".

## Decisions

### 1. A space splits the needle into terms, everywhere the matcher is used

`Matcher::query` splits the needle at whitespace (a run of it once,
leading and trailing ignored). A row matches when every term matches it
as fzy matches a needle — its characters in order — each anywhere in the
row and in any order, so `main carnet` is `carnet main`. Terms may land
on the same characters (`car carnet`), as fzf allows; requiring them
apart would cost a search over placements for a case nobody types.

One rule for every list the matcher answers: the pickers (files,
buffers, recent, smart, commands, tools, symbols, lines, the plugins'),
the launcher's modules and the `:memory` filter. None has a reason to
read a space as a character: a command's name with a space in it
(`dir here`) is still found, both its words matching. `:grammars`
matches a grammar's name and its files word by word (`kawoosh.fuzzy`
over single words), so a two-term query there finds a grammar only when
one word holds both — as before, when it found none.

*Beat:* splitting in the picker's Lua (a matcher per term, the hits
intersected): two thousand-row crossings and a join on every key, and
the launcher and the memory filter would each want the same code.

### 2. `\ ` is a space to match

A backslash before a space keeps the space in its term (`my\ notes`
finds `my notes.md`, not `my/notes.md`); any other backslash is itself,
since a Windows path is spelled with them. fzf's escape.

### 3. Smart case is each term's

A term with an upper-case letter is matched as written, one without in
either case: `cargo LOCK` asks for `LOCK` and either `cargo`. fzf
decides over the whole query; per term lets a capital narrow only the
word it is in.

### 4. The score is the terms' summed, positions are every term's

Each term is scored by fzy's dynamic programme over the whole row, and
the row's score is the sum; a term equal to the whole row is still
unbeatable. Every term's matched characters are the hit's positions,
sorted and once each, so `picker.spans` lights both `carnet` and `main`.
Summing counts a row's length against it once a term, which favours the
shorter of two rows that match equally — the direction fzy already
leans.

*Beat:* the best term's score alone (a second term would not rank at
all), a product (fzy's scores go negative).

### 5. A term that ends a name earns a bonus

fzy rewards where a match *starts* — after a `/`, a `-`, on a capital —
but not where it stops, so `main` put `src/mainframe.rs` before
`src/main/a_longer_name.rs`, the shorter row winning on its gaps. A term
is most often a name typed whole, so a term's last character earns
`MATCH_END` (0.5) when what follows it is not a letter or a digit, or is
a capital after a lower-case letter, or is the row's end: `carnet main`
puts `roles/carnet/tasks/main.yml` before `roles/carnetx/mainframe.yml`.
Half a start bonus, so it decides between matches alike and never beats
a run of consecutive characters (1.0 each).

### 6. The capital bonus reads the row as written

Found on the way: the bonuses were read off the row the needle is
matched against, which is lower-cased for a needle without a capital,
so the capital bonus never fired for a query typed in lower case (`fb`
found `foobar.rs` as good as `fooBar.rs`). They are read off the row as
written now, and only where a character matches.

## Cost

Measured over 45,000 paths of a monorepo's shape
(`timing_over_forty_five_thousand_paths`, the 200 best, the dev
profile), the matcher before and after run side by side, the best of
fifteen each: `main` 11.7 ms before, 8.2 ms after; `carnetmain` 7.7 →
7.6 ms; `mod12fi` 0.8 → 0.8 ms; `carnet main` 7.7 ms and `roles carnet
main yml` 4.8 ms (it found nothing before). Two terms cost what one
term as long costs: a row a term misses is ruled out by the cheap
in-order scan before any table is filled. The tables are kept from row
to row instead of allocated and filled per row, and the bonuses are
worked out only where a character matches — what paid for the end bonus
and the second term.
