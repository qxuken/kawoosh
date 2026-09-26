# Compile commands, deduced

Status: written 2026-09-26 from the user's ask — "compile commands
deductions from lsp's, maybe some heuristics like cargo project or
package.json files", and then "makefiles/justfiles as well"; round two
the same day, "let's read the memory" and "i also sometimes make
build.nu files with arguments" (Decisions 2 and 6). The calls
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

1. `compile.command`, when the settings say one — the project's word.
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

A picker of the setting's command, the last one run here, and every
deduced one, deduplicated: the command, what said so (`Cargo.toml`,
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

## Not built

- rust-analyzer's runnables as a kind (the test at the caret).
- A deducer registry for plugins (Decision 1's beaten).
- Other nushell files than `build.nu` (a `toolkit.nu`, nushell's own
  habit), and a parameter's completer offered in the prompt.
