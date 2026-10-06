# Compile commands, deduced

Status: written 2026-09-26 from the user's ask — "compile commands
deductions from lsp's, maybe some heuristics like cargo project or
package.json files", and then "makefiles/justfiles as well"; round two
the same day, "let's read the memory" and "i also sometimes make
build.nu files with arguments" (Decisions 2 and 6); round three,
from "propose a way to pass arguments. i often use `yarn pw
...some-project-path`" and "we probably need custom compile commands
that i can call", shaped over three exchanges (Decision 7); the keys
in `*compile*` and `:c` 2026-10-02 (Decision 8); a monorepo's
packages in the picker 2026-10-05 (Decision 9); the buffer named
for its command, then a buffer a command and directory, the same day
(Decisions 10 and 11); the colours, the head and how long it took
2026-10-06 (Decision 12); the end said once, a location's pane and
the session the same day (Decision 13); the note's "Not built"
2026-10-07 (Decisions 14–18). The calls
below are taken here, each the user's to overturn. Amends mvp.md
Decision 5c (compile mode), whose `:compile` ran what it was told or
`compile.command`, and said "compile what?" otherwise.

## What there was

- **A bare `:compile` needed `compile.command`.** A project without a
  `.kawoosh/settings.lua` saying it had to have its command typed, every
  time, though its `Cargo.toml` or `package.json` said what it builds
  with.
- **The compile's directory was guessed apart**: the outermost
  `Cargo.toml`, `package.json` or `Makefile` in the repository, the same
  for every command — `make` in a crate under a Makefile'd repository
  ran at the top.
- **Nothing listed what a project can run**: its scripts, its recipes,
  its targets.

## Decisions

### 1. A project's commands are read from its files, ranked by its server

`kawoosh/src/deduce.rs` walks from the caret's file (in `*compile*`,
from where its command ran; with no file, from the working directory)
up to the repository's root — the nearest `.git`; outside one, up to
the home directory, not into it — and reads what it finds, each kind
once. A host's path is not read, each look a round trip there; a
compile on a host is typed or set, as before.

| file | commands | where |
|---|---|---|
| `Cargo.toml` | `cargo check` `build` `test` `clippy`, `run` for a binary | the outermost, the workspace — cargo prints its paths from there |
| `package.json` | `<pm> run SCRIPT` per script; `<pm> exec tsc --noEmit` beside a `tsconfig.json` with no script that runs `tsc` | the nearest; the package manager by its lockfile (`pnpm-lock.yaml`, `yarn.lock`, `bun.lock[b]`) or `packageManager`, else npm |
| `justfile` `Justfile` `.justfile` | `just`, `just RECIPE` per public recipe, its `#` comment as the why | the nearest |
| `build.nu` | `nu build.nu [SUB]` for a script's `main` and `main SUB` (an `alias "main SUB"` too), else `nu -c 'use build.nu; build NAME'` per `export def` of a module; the comment above as the why (Decision 6) | the nearest |
| `Makefile` `makefile` `GNUmakefile` | `make`, `make TARGET` per plain target, a `##` or preceding `#` comment as the why | the nearest |
| `CMakeLists.txt` | `cmake --build build` once `build/CMakeCache.txt` is there, `cmake -B build` | the outermost |
| `go.mod` | `go build ./...` `vet` `test` | the nearest |
| `pyproject.toml` | `pytest`, `mypy .`, `ruff check` as its `[tool.*]` tables configure them, under `uv run` beside a `uv.lock` | the nearest |
| `build.zig` | `zig build`, `zig build test` | the nearest |

**The language server ranks them.** The kinds whose file is among the
caret buffer's server's root markers come first, in the markers' order
— the effective `ServerDef`, `lsp.NAME.roots` included (lsp-rules.md):
a `.rs` file's server roots at `Cargo.toml`, so `cargo check` leads; a
`.ts` file's at `tsconfig.json` / `package.json`, so the package's
scripts do; clangd's at `CMakeLists.txt` and `Makefile`. In a
repository holding a Tauri app, the same `<leader>cc` checks the crate
from a `.rs` buffer and builds the frontend from a `.tsx` one. Then the
task runners (`just`, `make`), which a project keeps as its own front
door; then the rest nearest first. A buffer with no server — or no
file — ranks by nearness alone.

Within a kind, the ones that answer "does it build" come first: a
script named `check`, `typecheck`, `build`, `lint`, `test` in that
order, then the rest by name (`dev` and `start` last — they do not
end); a Makefile's and a justfile's default first, then in the file's
order.

Beaten: a language → toolchain table in the shell (the server's markers
already say it, and a user's `lsp.NAME.roots` moves it with them);
asking a server — rust-analyzer's `experimental/runnables` is the one
that answers, and only for Rust, asynchronously; it can join as a kind
later. A plugin API of deducers: the list reaches Lua as data
(`kawoosh.compile_offer()` while the picker is open) and `kawoosh.compile`
runs anything; a deducer registry waits for a second use (Decision 17
since).

### 2. A bare `:compile` runs the project's word, else again, else the first

1. `compile.default`, when the settings say one — the project's word
   (`compile.command` before round three; Decision 7).
2. Else the command last compiled in this workspace — typed, picked or
   deduced: `:compile cargo test`, then `<leader>cc`, tests again;
   emacs's `recompile`. **Read from the memory** (round two): the
   `tool` row every compile already made (memory.md round four, its
   meta the command and its directory), under the memory's workspace —
   what is pending of it, else the store's. So it holds across
   launches, and forgetting the row in `:memory` forgets it here: one
   record, not a second map beside it.
3. Else the first deduced that runs as it is (Decision 1; one wanting
   arguments is the picker's), the message saying what and why:
   `cargo check (Cargo.toml)`.

`:compile?` says which of the three it would be.

### 3. `:compile pick` (`<leader>cC`) lists them

A picker of `compile.default`, the named `compile.commands`, the lines
run here (newest first; Decision 7), and every deduced one, a command
listed once: the command, what said so (`Cargo.toml`,
`web/package.json`) muted beside it, the preview its directory, its
why (the script's body, the recipe's comment) and how it is declared
(a `def`'s signature, a recipe's line). `<CR>` runs it (`:compile pick
N`); `<C-e>` puts it in the prompt to add arguments first (`:compile
edit N`).

### 4. Every command runs where its kind says

A deduced command carries its directory (Decision 1's "where"). A typed
or set one runs where the deduced kind of its program does —
`:compile cargo test` at the workspace, `:compile make foo` beside the
nearest Makefile, `pnpm …` at the package — and anything else where the
compile ran before this: the outermost marker in the repository, else
the repository, else the file's directory. The locations in its output
resolve against the same directory.

### 5. tsc's locations are locations

`src/a.ts(3,5): error TS2322` — how `tsc` prints when it is not on a
terminal — reads as `src/a.ts:3:5` wherever a location is read
(`location_at`): `]q` walks a `tsc --noEmit` the way it walks cargo.

### 6. `build.nu`, and commands that want arguments

A `build.nu` is read two ways, as nushell runs it. With a `main` it is
a script: `def main` is `nu build.nu`, `def "main binary"` is `nu
build.nu binary`, and `export alias "main test" = test` is `nu build.nu
test`; its other defs are helpers. Without one it is a module, as
`use build.nu` takes it: each `export def NAME` is `nu -c 'use build.nu;
build NAME'` (`build` is the module's name, the file's), a plain `def`
a helper, an alias a second name for a row already there. The comment
above a def is its why; its signature, from `def` to the `]`, the
preview's detail — each flag with its type, default and comment, as
the file wrote it.

The signature is read for one thing more: a **positional parameter
with no default** (not `name?`, not `name = …`, not a `--flag`, not
`...rest`). A command with one cannot run as it is, so a bare
`:compile` passes over it and the picker's `<CR>` does not run it: it
opens the prompt on `:compile CMD ` with the caret where the arguments
go — inside the quote of a `nu -c '…'`, since after it they would be
nu's. A justfile's recipe with a parameter without a default is the
same (offered now, where round one hid it). Flags are never required,
so no row is made per flag: `<C-e>` opens any row in the prompt the
same way, and the preview lists what there is to add.

Beaten: running `nu` to ask for the signatures (`scope commands`) — a
process per `:compile`, and a module's top level evaluated to answer;
the file's text says it. A row per flag or per completion of a
parameter (`entry: string@examples`) — too many rows, and the
completer is code.

### 7. Commands by name, `%`, and the lines run

**The settings name commands.** `compile` is a closed table of options
and one open table of the user's commands:

```lua
compile = {
  default = "pw",                   -- a :compile line: a name, or a command
  commands = {
    pw = "yarn pw %",               -- a string is its command
    e2e = { cmd = "yarn pw", args = true, doc = "a path to finish" },
    web = { cmd = "yarn build", cwd = "apps/web" },
  },
  deduce = true,                    -- Decision 1's files read; on unless false
}
```

- **`:compile NAME [ARGS]`** runs `NAME`'s command with the words after
  it appended — `:compile e2e e2e/login.spec.ts` is `yarn pw
  e2e/login.spec.ts`. Names first: a name hides a program of the same
  name, since the user wrote it; `pick`, `edit` and `kill` are the
  command's own. `kawoosh.compile(line)` reads a line the same way.
- **`compile.default`** is a line, not a command: `"pw"` is `:compile
  pw`, `"cargo build --workspace"` itself. It sits beside `commands`,
  not in it — a setting of compile mode, not one of the commands.
- **`args = true`**: called with nothing after it, the command is not
  run but put in the prompt, `:compile e2e ` with the caret at its end.
- **`cwd`**: a relative one is the project's whose `.kawoosh/settings.lua`
  said it, joined in Rust; from the user's file or `:set`, the caret's
  project's (its `.kawoosh`, else its repository). Without one, where its
  program's kind runs (Decision 4).
- **Why a table inside the table**: step 34's declared settings flag a
  misspelt key (`compile = { comand = … }`) and give the language
  server its types. A table of the user's names has to be open — any
  key goes — so commands straight in `compile` would open it all, and
  a command named `deduce` would be an option. `compile` stays closed;
  `commands` is the one open table (`table<string, any>` to LuaLS).
- **`compile.command` is gone**, not aliased: a file still setting it is
  told `compile.command is now compile.default` (the undeclared-key
  toast, worded as a move). This repository's `.kawoosh/settings.lua`,
  the `compile` tool (`tools.lua`, `kawoosh.compile_default()`: the
  default made a command line; none when it has a `%`, which a terminal
  has no file for) and the tests moved with it.

**`%` is the file, from where the command runs.** As `:!` has it
(`%`, `%:h`, `%:t`, `%%` a `%`, each quoted), but a path under the
command's directory is written from there: in `apps/web`, `%` from
`apps/web/e2e/login.spec.ts` is `'e2e/login.spec.ts'` — what a test
runner filters by and what reads in the header. Put in when the line
runs, so `pw = "yarn pw %"` is always *the file I'm in*; a `%` with no
file (in `*compile*`) says so and runs nothing.

**The lines run are kept, exactly.** The memory's `tool` row for the
compile (Decision 2) holds the last ten lines run in the workspace with
their directories, newest first, as they ran — `%` put in, the name's
command spelled out. A bare `:compile` runs the first again: after
`:compile pw` from a spec, `<leader>cc` from the code under test runs
*that spec*, not `yarn pw src/Button.tsx`. The two meanings stay apart
without a switch: the setting is a template, the memory a record. The
picker lists the lines after the named commands, `last run here` then
`recent`; a line that is a named command's is listed once.

**npm's `--`**: `npm run pw path` gives `path` to npm, not the script,
so arguments after an `npm run X` go past a `--` — appended to a name,
and in the prompt `<C-e>` opens; yarn, pnpm and bun pass them on.

**`<Tab>` after `:compile `** completes the names, then paths from the
directory the line will run in — `:compile pw e2e/lo` in `apps/web`
from anywhere.

**A trusted `init.lua` says where in code.** `settings.lua` is data (no
`kawoosh` in its sandbox); a project's `init.lua`, once trusted, has the
API and writes the same project layer with `kawoosh.opt`. It is told
where it lives — `kawoosh.project` is `{ root, dir }` while it runs,
nil after, so it is captured at the top — and `kawoosh.fs.join` takes
any number of parts:

```lua
local root = kawoosh.project.root
kawoosh.opt("compile.commands.web", {
  cmd = "yarn build",
  cwd = kawoosh.fs.join(root, "apps", "web"),
})
```

**And on which system.** A settings file cannot ask, being data; code
can: `kawoosh.os` is `"mac"`, `"linux"` or `"windows"` — the names a
server's install lines are keyed with — so one name runs this system's
line. This repository's `.kawoosh/init.lua` names `install` so
(`scripts/windows-app.nu --install`, `scripts/macos-app.nu
/Applications`):

```lua
local install = { mac = "nu scripts/macos-app.nu /Applications",
                  windows = "nu scripts/windows-app.nu --install" }
kawoosh.opt("compile.commands.install", install[kawoosh.os])
```

Beaten there: an `os` in the settings sandbox (the builtin servers'
table is read in it too, and a file of data that branches is code); a
fact, `kawoosh.holds("windows")` (a `when` is about where the keys are,
and a table keyed by system wants the name).

Beaten: `compile.commands.default` (the default is compile mode's, not
a command's); commands straight in `compile` (above); `@name` to call
one (names first reads better; a hidden program is the user's own
doing); a row per argument set in the settings (the recent lines are
that, unwritten); a second picker for the path (completion and `%`
are fewer keys).

### 8. The keys go to `*compile*`, and `:c` is `:compile`

*Added 2026-10-02, asked:* "compile should focus the panel it's
created" and "compile should have fast alias maybe something like
`:c`". Every compile ends in `compile_in`, so every door — `:compile`,
`<leader>cc`, the picker's `<CR>`, `r`, `kawoosh.compile` — gives the
keys to `*compile*`: to the pane it opens (a column of its own, as
pane-placement.md Decision 2 has it), and, when one shows it already,
to that one, the run there. The second is the call taken: a rerun
from the file with the output beside it is the same act as the first
run, and keys that went to the pane one time and stayed the next
would make the user look before each `<C-c>`. `q` there closes it, the
keys going back to the pane they were in last (pane-placement.md
Decision 5).

Two things follow from the keys being there. **The caret follows the
output only while it is at the end** — the views start there, and one
moved up to read an error, or put on a location by `]q`, stays as more
lines come, where before every line pulled every view down. **`%` in a
line asked from `*compile*` is the file its run was asked from**
(`Compile::file`), not "no file for %": with `compile.default = "pw"`
and `pw = "yarn pw %"`, `<leader>cc` from the spec and `<leader>cc`
again from its output test the same spec. `r` keeps it too. The
project is read from where the run ran, as before (Decision 1).

**`:c`** is the alias, beside `:make`. Nothing had it: kawoosh resolves
a name, an alias or nothing — no prefix matching — and vim's `:c`
(`:change`) has no kawoosh counterpart. vim's quickfix family is here
already as `:cn` `:cp`, so `:c` reads as one of them. `:c pick`, `:c?`
and `<Tab>` after `:c ` work as for `:compile`, the alias resolved
before anything reads the line.

Beaten: focusing only a pane just made (what `show_in_pane` alone
would say for a text pane), for the inconsistency above; and `:cc`,
which is vim's "go to error N" and would be the natural spelling of a
`]q` with a count.

### 9. The picker reads every package of a monorepo

*Added 2026-10-05, asked:* "I want `:compile pick` to look inside all
package.jsons in a workspace or at least relative to open buffers. It
relevant to monorepos". Decision 1 reads one `package.json`, the
nearest above the caret: from `apps/web` the picker had nothing of
`apps/api`, nor the root's scripts.

After the caret's project's rows, the picker lists each other
package's — `deduce::packages`: the `package.json` nearest each open
buffer first (they are what is being worked on, and may sit outside
the caret's repository), then every one in the caret's repository, as
the file picker's walk sees it — nothing git ignores, so no
`node_modules`. Each is read as Decision 1 reads the nearest: its
scripts, its `tsc`, the package manager by the lockfile at or above
it, run in its own directory; what said so (`apps/api/package.json`)
is beside the row and matched as it is typed, so `api build` finds it.

A command is listed once *where it runs* now: `yarn run build` in two
packages is two rows, where the picker went by the command alone.

Only the picker: a bare `:compile` and Decision 4's directory stay the
caret's project's — a build picked for me from a package I am not in
would be a guess. The walk is made as the picker opens, stopped at 500
packages or 200 000 entries seen; outside a repository only the open
buffers' packages are read, and `compile.deduce = false` turns it off
with the rest.

Beaten: the workspace globs (`workspaces`, `pnpm-workspace.yaml`) — a
reader per package manager, and a repository of packages that is no
workspace would list nothing; `<pm> --filter` rows from the root — the
same commands spelled three ways. The other kinds (a crate's, a
directory's Makefile) the same way waits for the ask: cargo's are the
workspace's already.

### 10. The buffer is named for its command

*Added 2026-10-05, asked:* "Let's name compile mode with command that
it points to so i could find it more easily. Something like `*compile:
cargo build*`". The buffer a run is shown in is named `*compile: CMD*`
(`compile::buffer_name`): the command on one line, cut at 60
characters. One buffer then, the next command renaming it — Decision
11 gives each its own.

Its maps and every `when` knew it as `buffer:*compile*`, a user's
`buffer = "*compile*"` too, and a name that changes with each run
would lose them. So a fact `buffer:*KIND*` holds of a buffer named
`*KIND: SUBJECT*` as of one named `*KIND*` (`command::named`): the
name's kind is what a place is, its subject what it shows. Beaten: a
`compile` fact of its own beside the name — every map written against
the name would have to move to it — and a title apart from the name,
which the buffer picker and `:b` would then not match on.

### 11. A run is its command and its directory

*Added 2026-10-05, asked:* "We already give compile different names.
let's separate them by directory and command. Now new command just
replaces previous one". There was one buffer and one process: `:compile
cargo test` over a `cargo build` stopped the build if it still ran and
wrote over its output either way.

`Compile` holds runs now (`compile::Run`), a run its command, the
directory it runs in, its buffer and its process. **The same command in
the same directory is the same run**: it goes into its buffer again,
its last process stopped if it has not ended — `r`, a bare `:compile`
again. **Another command, or the same one somewhere else, is another
run**: a buffer of its own, the others' output and processes as they
were. Two builds run side by side; `yarn build` in `apps/web` and in
`apps/api` are two.

- **The name says where when it has to.** `*compile: CMD*` in the
  working directory, `*compile: CMD in DIR*` elsewhere — `DIR` from the
  working directory, else from home. A buffer is found by its run, not
  its name, so a `:cd` between two runs cannot make two one.
- **The pane.** A run on show already is run there. Else a pane showing
  a run that has *ended* gives its place: one pane of output across
  commands, as before, the older buffers a `:b` away. A run still going
  is left on show and the new one takes a column of its own. The keys
  go to the run started, by every door (Decision 8).
- **`r`, `<C-c>`, `%` and the project are the pane's run's**: in a
  compile buffer they mean the run it shows; from anywhere else `:compile
  again` and `:compile kill` mean the run started last. `compiling` (the
  fact `<C-c>` is bound under) is that run still going.
- **`]q` `[q` walk the run started last**, as they walked the one
  buffer; a location's path is read from its own run's directory.
- **A run ends with its buffer.** `q` closes the pane and keeps the
  buffer, as it did; `:bd` on it stops the command if it runs and
  forgets the run (`Compile::forget`, from `drop_buffer`). Nothing is
  swept: there are as many buffers as commands run, not as times run.
- The corner's `finished` names its command while another still runs;
  the title bar says `compiling…` while any does.

The memory is as it was (Decision 7): the lines run, by workspace — a
record of what was asked, not of what is open.

Beaten: a buffer per *time* run (history piling up for a `r`); the
directory always in the name (most runs are the workspace's, and the
name is what is read in a list); a pane per command (a row of columns
after a morning's commands); closing the buffer on `q` (the output of
a build just read is what `]q` walks next).

### 12. The output in its colours, under where and when, over how long

*Added 2026-10-06, asked:* "I don't like that it doesn't have colors,
error highlighting is convenient. And we probably should print some
info at first line like the directory and time it was run on. At the
end would be cool to see time it took to complete a task."

```
~/projects/kawoosh · 2026-10-06 14:03:22
$ cargo build
   Compiling kawoosh v0.0.1
error[E0308]: mismatched types
…

[exited with 101 in 8.2s]
```

- **The head.** The first line is the directory the command runs in
  (from home) and the local date and time it was started, dim; the
  command's echo is the second, where it was the first. Neither is a
  location (`location_on`).
- **The last line** says how it ended and how long it took, in the
  colour of how: `[finished in 8.2s]` green, `[exited with 1 in
  0.34s]` red, `[killed after 2m 03s]` yellow (`compile::took`: `0.34s`
  under a second, `8.2s` under a minute, `2m 03s`, `1h 02m`). The
  corner's note says the same words.
- **The colours are asked for.** The command still runs on a pipe, so
  its programs would print plain; it is given `FORCE_COLOR=1`,
  `CLICOLOR=1`, `CLICOLOR_FORCE=1` and `CARGO_TERM_COLOR=always`
  (`ProcSpec::env`; exported on a host), each only where the editor's
  environment has no word of its own. `compile.color = false` gives
  none, as does a `NO_COLOR` in the environment.
- **And kept as paints.** Each line goes through vte — the parser the
  terminal reads with, already in the graph under alacritty
  (`kawoosh_term::plain`; a hand-written reader was the first build:
  "maybe we will use some libs we have?"). The buffer's text has no
  escape sequence in it — what `/`, a yank and `]q` read — and the
  foregrounds they set are the buffer's `compile` paint set. One of
  the sixteen is the paint `ansi:N`, resolved as it is drawn, so the
  output follows the theme as a terminal's does; the 256 and true
  colours are `#rrggbb`; dim is `dim`. Bold, backgrounds and underlines
  are read past: a paint is a colour. A carriage return drops what it
  wrote over, so a progress line is its last state. The state runs on
  from line to line.
- **A line printed plain** has its `error` and `warning` painted where
  it says one as compilers do — before a `:` or a code (`error:`,
  `error[E0308]`, `error TS2322`) — in the diagnostics' colours. A line
  its program coloured is left as printed.
- **A frame resolves the paints of the lines it draws.** A compile has
  a paint a word, hundreds of thousands in a long build, and
  `paints_of` resolved every one of a buffer's a frame: 45 ms a frame
  on 200 000 coloured lines. It is two now — `settle_paints` carries
  the sets through edits, `paints_in` resolves what reaches into the
  drawn lines — and the same output is 0.4 ms. An append leaves the
  set at the buffer's version, nothing to carry.

Beaten: a pty for the command — every program would colour unasked,
but a pty also means a width to wrap at, pagers, progress bars
redrawing, and a prompt on `/dev/tty` waiting where today it fails at
once (`run_command`'s own session); `:!` is that door. Colours resolved
to RGB as they arrive — a theme switched after would leave the old
ones. A grammar for compiler output — each tool's format a guess, where
its own colours are exact.

### 13. The end said once, a location in the pane showing its file, no output across a restart

*Added 2026-10-06, asked:* "It should not throw toast about completed
task when it's in focused", "I don't like that it opens buffer in-place
when i have it open in the editor besides", and "i don't like it opens
scratch on restart. Either restore output or close it".

- **How a run ended is said once.** Its last line says it, in its
  colour (Decision 12); while the pane showing the run has the keys,
  that is under the eyes, and the note goes to the log alone
  (`Show::Log`). Away from it — in the file, the keys moved after
  `<leader>cc` — the corner or a toast says it as before. The
  pane with the keys at the *end* is what counts, not where the run
  was asked from: `<leader>cc` gives the keys to the run (Decision 8),
  so a toast comes exactly when the user left.
- **A location opens in the pane showing its file.** `<CR>` and `]q`
  looked first to the pane the list was opened from, then to the first
  other editor pane, and opened the file there — over what that pane
  showed, though the file was on show a pane further (asked from
  `b.rs`, the error in `a.rs` open beside: `b.rs` lost its place).
  Now a visible pane showing the location's file is the first choice
  (`open_location`), the two rules after it as they were. The lists
  (`gr`, `:diagnostics`) go through the same door.
- **A session keeps no compile pane.** It kept the pane as an editor
  on a nameless buffer, and a restart filled it with a blank scratch
  beside the file. Its process is gone, and what it printed was of
  the files as they were; the memory keeps the line run for
  `<leader>cc` to run again (Decision 7). So the pane goes as a `:term
  CMD` pane does (`PaneData::Gone`), a tab holding only it with it;
  `:session restore` from inside leaves the output hidden, as it did.

Beaten: a note quieted whenever the run is *visible* — the user asked
for focus, and a line at the end of a pane beside is easy to miss
while typing; `Show::Corner` instead of the log — the corner is still
a word appearing where the eyes are not. Restoring the output: the
text and its paints in the session (a long build's are megabytes
written at every quit), with a run to make for `r`, or a stub saying
the output was not kept — a scratch by another name.

### 14. `run` is a tool like the rest

*Added 2026-10-07*, from this note's "Not built" ("let's do the compile
ones"). `run.command` was a setting of its own that made a `run` tool
(`tools.lua`), beside a `tools` table that makes tools by name: two
spellings of one thing, as `compile.command` and `compile.commands`
were. `tools.run` is the one now — `run = { cmd = "cargo run", cwd =
"root" }` — and `run.command` is gone, not aliased: a file still setting
it is told `run.command is now tools.run`, as Decision 7 moved
`compile.command`. A string is its `cmd`, as for every tool, so it runs
in the file's directory unless it says `cwd = "root"`, where
`run.command` always ran at the root: the one tool that did not read
like the others. `compile` stays made from `compile.default`: it is
compile mode's line in a terminal, not a tool the user names.

Beaten: `run.command` kept as an alias (two ways to say it, one of them
undocumented); a `run` beside `compile`'s default (`compile.run`) —
running a program is not compile mode's, which reads its output.

### 15. A plain line painted as the compilers paint theirs

*Added 2026-10-07*, from "Not built": "a program that colours only on a
terminal and reads none of the variables (gcc and clang without
`-fdiagnostics-color`, `go`)". Tried first: Apple clang 21 (and the
`gcc` that is it) on a pipe with `CLICOLOR_FORCE`, `FORCE_COLOR` and
`GCC_COLORS` set prints plain — nothing in the environment turns it on,
and go has no colours at all. So the line is painted for them, as they
would have (`plain_paints`), where Decision 12 painted only `error` and
`warning`:

- the **location heading the line** (`b.c:3:22`, `./main.go:3:30`,
  tsc's `src/a.ts(3,5)`) bold — clang's and gcc's locus, and what `]q`
  goes to;
- **`error` and `warning`** in the diagnostics' colours, bold, and the
  **message** after them bold;
- **`note`, `help` and `remark`** before a `:` bold cyan (`ansi:6`, the
  theme's, as clang prints a note);
- the **caret line** under a quoted source line — `^~~~` after a `  3 |
  ` gutter, or alone — bold green, from its first mark to its last; a
  gutter line with no `^` or `~` (a table's `-----`) is not one;
- a **test runner's verdict** heading a line — go's `--- FAIL`, `FAIL`,
  `--- PASS`, `PASS`, `ok`, `--- SKIP` — red, green or yellow; the
  bare words only as go prints them, alone or before a tab, so prose
  (`ok, so…`) is not one.

A line its program coloured is left as printed, as before.

Beaten: a pty for the command (Decision 12's beaten still holds);
`CCC_OVERRIDE_OPTIONS=+-fcolor-diagnostics` for clang — it works, and
prints a `### Adding argument` line into the output each time; adding
`-fdiagnostics-color` to `CFLAGS` — a Makefile that sets its own wins,
and a build that does not read the variable never sees it; a grammar
per tool (Decision 12's beaten).

### 16. The nushell files a project names, and its parameters' completions

*Added 2026-10-07*, from "Not built": "other nushell files than
`build.nu` (a `toolkit.nu`, nushell's own habit), and a parameter's
completer offered in the prompt".

**Which files.** `compile.nushell` lists them — names, or paths from a
directory — each read at its nearest above the caret, as the other
kinds' files are; `{ "build.nu", "toolkit.nu" }` unless it is set. Two
found are two sets of rows, `build.nu`'s and `toolkit.nu`'s, ranked
together as the task runners they are. A path is spelled from the
directory it was found in: `compile.nushell = { "scripts/verify.nu" }`
offers `nu scripts/verify.nu`, run there.

**Script or module, as nushell reads it.** Decision 6 took a file with
any `main` for a script. nushell's own `toolkit.nu` has an `export def
main` beside its other exports, and is used as a module — `use
toolkit.nu`, then `toolkit` and `toolkit fmt` — so a script now is a
file with a `def "main SUB"` (or an `alias "main SUB"`), a `main` that
is not exported, or a `main` and nothing else exported; anything else a
module, its exported `main` the module's own name (`nu -c 'use
toolkit.nu; toolkit'`). A `def` inside another's body is not the
file's: bodies are passed over, brace to brace, outside strings.

**A parameter's completions in the prompt.** The signature already read
for `needs` (Decision 6) is read for each parameter whole now — its
type, its default, and a completion: `string@[debug release]` is those
values; `string@targets` is what `targets` answers. `<Tab>` after a
nushell row's command, at a positional parameter with one, offers them
before the paths, a flag with a type (`--jobs (-j): int`) passing over
its value as it is counted. Answers come in order of cost:

1. **From the file**, when the completer's body is a list of plain
   values (`def targets [] { ["debug", "release"] }`): no process.
2. **Else nu**: `nu --no-config-file -c "source FILE; NAME | to json
   -r"` in the file's directory — `source` defines the file's commands
   and does not run its `main` — read from the last line (the file's
   own top level may print before it), as a list of values, of `{
   value }` records, or a record of `completions`. Kept by file and
   completer while the file's stamp holds, so it is once per change; a
   second at most, a completer that hangs costing that once. The
   prompt's candidates are made on every key, so a process per key
   was not an option.

A value with a space or a shell's character is double-quoted — inside
a `nu -c '…'` too, where a single quote would end the line's — and one
asked before a `nu -c` row's closing quote keeps the quote.

Beaten: every `*.nu` in the project (a deploy script is a `.nu` too,
and a bare `:compile` would run the first that runs as it is); nu's
own `scope commands` to read the signatures (Decision 6's beaten); the
completer asked as the prompt opens, ahead of the key (it is asked
rarely, and the cache makes a second `<Tab>` free); a nushell grammar
for the file (the signature's shape is small and the text says it).

### 17. A plugin's kind of build, found as the builtin ones are

*Added 2026-10-07*, from "Not built": "a deducer registry for plugins
(Decision 1's beaten)". Decision 1 waited for a second use; the ask is
the use.

A kind was an enum with its facts in `match`es — its files, nearest or
outermost, a task runner or not, its programs — and its commands read
from its file in Rust. The facts are data now (`deduce::Spec`), so a
plugin says them as data:

```lua
kawoosh.compile_kind("mix", {
  markers = { "mix.exs" },      -- the files that say a directory is its
  outermost = false,            -- nearest (the default), or outermost
  runner = false,               -- ranked with just and make
  programs = { "mix" },         -- `:compile mix test` runs where mix.exs is
  commands = { "mix compile", { cmd = "mix test", why = "the tests" } },
})
```

`commands` is a list — strings, or `{ cmd, why, needs, detail }` as a
row has them — or a function of `{ file, dir, text }` answering one,
for a kind whose commands are in its file (a `mix.exs`'s aliases).
The walk finds a plugin's kind as it finds a builtin one, and Decision
1's ranking holds for it — the language server's markers, then the
runners, then nearness, then the order the kinds were said in — so a
bare `:compile`, the picker and Decision 4's directory read it with
the rest. A function's error is said in the log and its kind offers
nothing; it is called each time the project is read (a picker opened,
a bare `:compile`, the prompt's `<Tab>`), so it reads `text` rather
than the disk.

**A builtin's name puts it in that kind's place**: `cargo`, `node`,
`just`, `nu`, `make`, `cmake`, `go`, `python`, `zig`.
`kawoosh.compile_kind("make", { markers = { "Makefile" }, commands = {
"make -j8" } })` is the user's make; `kawoosh.compile_kind("make",
false)` no make at all. A `node` of a plugin's also takes the monorepo's
walk (Decision 9) with it: that walk reads `package.json`s the builtin
way.

Beaten: a function per kind that does the finding too (`deduce(dir)`)
— every plugin would walk the directories again and rank itself
outside the ranking; the builtin kinds rewritten in Lua (their readers
are tested Rust, and a reader is not where a user's change is: the
commands are); a kind only adding commands to a builtin one (a
replacement whose function calls nothing back is simpler, and the
builtin's commands are a picker away).

## Not built

- ~~A program that colours only on a terminal and reads none of the
  variables (gcc and clang without `-fdiagnostics-color`, `go`): plain,
  but for `error:` and `warning:`.~~ Decision 15.

- rust-analyzer's runnables as a kind (the test at the caret).
- ~~A deducer registry for plugins (Decision 1's beaten).~~ Decision 17.
- ~~Other nushell files than `build.nu` (a `toolkit.nu`, nushell's own
  habit), and a parameter's completer offered in the prompt.~~ Decision
  16.
- ~~`run.command` as a `tools` entry: the same shape question as
  `compile.command`, left for its own round.~~ Decision 14.
