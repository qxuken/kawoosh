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
packages in the picker 2026-10-05 (Decision 9). The calls
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
runs anything; a deducer registry waits for a second use.

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

## Not built

- rust-analyzer's runnables as a kind (the test at the caret).
- A deducer registry for plugins (Decision 1's beaten).
- Other nushell files than `build.nu` (a `toolkit.nu`, nushell's own
  habit), and a parameter's completer offered in the prompt.
- `run.command` as a `tools` entry: the same shape question as
  `compile.command`, left for its own round.
