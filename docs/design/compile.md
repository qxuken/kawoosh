# Compile commands, deduced

Status: written 2026-09-26 from the user's ask — "compile commands
deductions from lsp's, maybe some heuristics like cargo project or
package.json files", and then "makefiles/justfiles as well". The calls
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
2. Else the command last compiled in this workspace this session
   (`workspace_of`: its `.kawoosh`, else its repository) — typed, picked
   or deduced: `:compile cargo test`, then `<leader>cc`, tests again;
   emacs's `recompile`.
3. Else the first deduced (Decision 1), the message saying what and
   why: `cargo check (Cargo.toml)`.

`:compile?` says which of the three it would be. Session-only on
purpose: what was compiled last is not a setting, and the next launch
starts from the project's word or its files.

### 3. `:compile pick` (`<leader>cC`) lists them

A picker of the setting's command, the last one run here, and every
deduced one, deduplicated: the command, what said so (`Cargo.toml`,
`web/package.json`) muted beside it, the preview its directory and its
why (the script's body, the recipe's comment). `<CR>` runs it
(`:compile pick N`).

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

## Not built

- rust-analyzer's runnables as a kind (the test at the caret).
- A deducer registry for plugins (Decision 1's beaten).
- Remembering the last compile across launches (the memory keeps its
  `tool` row with the command; bare `:compile` does not read it).
