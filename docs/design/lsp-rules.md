# Rules: a language's server, switched per project

Status: written 2026-09-26 from the user's ask — "toggleable rules for
specific lsp's (load all files in ts for example for a complete
diagnostics and stuff)". The calls below are taken here, each the
user's to overturn. Companion to [lists.md](lists.md), whose
Decision 2 left a TypeScript project's diagnostics at "the files you
opened" and whose "Not built" is where this came from.

## What there was

A language's server was a `ServerDef` — command, args, root markers,
the configuration it reads — from `ServerDef::builtin` or a
`kawoosh.lsp.server(language, t)` in Lua, and nothing about how it was
used could be said per language or per project:

- **A server spoke of what it was sent.** typescript-language-server
  answers for the documents it holds, so `:diagnostics` in a
  TypeScript project listed the errors of the files that happened to
  be open. rust-analyzer's check covers the crate; tsserver has
  nothing like it unasked, and `tsc --noEmit` through `:compile` was
  the only whole answer.
- **A server could not be turned off** but by redefining it to a
  command that is not there.
- **`lsp.inlay_hints` was one switch** for every language: rust's
  hints are worth reading, TypeScript's crowd a line.
- **The settings declared an `lsp` table** — "a language's server:
  `cmd`, `args`, `roots`, `settings`" (roadmap step 34) — that nothing
  read.

## Decisions

### 1. A rule is a setting under its server, named by its language

`lsp.<name>` is a language server, in the settings tree, named by the
language it is first for:

```lua
-- .kawoosh/settings.lua in a TypeScript project
return {
  lsp = {
    typescript = { load_all = true },   -- .ts, .tsx and .js
    rust = { inlay_hints = true },
    python = { enabled = false },
  },
}
```

The tree is layered already (kui.md D10): the user's file says what
holds everywhere, a project's `.kawoosh/settings.lua` what holds there,
and `:set` or `:lsp toggle` what holds for the session — a rule is
switched per project for free, and a `:cd` swaps the project's rules
with its layer. The shell folds the table over the definition of that
name (builtin or Lua's) into the one the pool runs, again on every
settings change.

**One server serves the languages one program reads**
(`ServerDef::languages`): `typescript` is typescript-language-server
for `typescript`, `tsx` and `javascript`, and `c` is clangd for `c` and
`cpp`. They were one process already — the pool keys by root and
command — but three definitions with three sets of rules, so a
TypeScript project needed `load_all` said twice (asked in review:
"typescript and tsx i think can be combined"). A rule is the server's
and reaches all its languages; each loaded file is still sent as its
own language. `lsp.tsx = { … }` without a `cmd` is no server: the
settings say once where its rules go (`lsp.tsx: tsx is served by
lsp.typescript`). With a `cmd` it is a server of its own, and a
language with a server of its own name is served there — `lsp.javascript
= { cmd = … }` takes javascript from typescript's. `languages = { … }`
under a name says the list outright.

Named by **language**, not by command: a buffer carries a language and
a user thinks "TypeScript", not `typescript-language-server`; the name
of the language a server is first for reads as that. `:lsp toggle`
from a `.tsx` buffer flips `lsp.typescript`'s rule.

Beaten: rule fields on `kawoosh.lsp.server` (Lua only, not per
project, not switched at runtime); rules keyed by command
(`lsp["typescript-language-server"]`); a definition per language with
each one's rules (as first built — the same rule twice for one
process); `lsp.tsx` falling back to `lsp.typescript`'s keys (two places
to look for one server's rule).

### 2. The table is the server too

`cmd`, `args`, `roots`, `languages` and `settings` under `lsp.<name>`
replace the definition's, as step 34 declared and nothing did. A new
`cmd`, `args`, `roots` or `languages` restarts the servers on its
command (the path `:lsp restart` takes: the buffers sent again, their
diagnostics cleared until the new server's land); new `settings` reach
a running server as `workspace/didChangeConfiguration`. `enabled =
false` takes the server out of the table the pool reads: it stops, its
languages' buffers are not sent, and `enabled = true` sends them again.
A `lsp.<name>` with a `cmd` and no definition of that name is a new
server: `lsp.zig = { cmd = "zls" }`.

### 3. `load_all`: the server holds every file of its languages

With `load_all` on, a server that starts — or one running when the
rule is switched on — is sent every file of its languages in its
workspace, read from disk: the walk the picker and `:grep` take
(`fs::walk`, `.gitignore` honoured), a language's files by its
extensions and names, at most `load_max` of them (2000), none over
1 MiB (a bundle is not source), and none a secrets rule's `files` names
— a private buffer's text never leaves the process (secrets.md
Decision 1), and a file read behind the buffers' backs is held to the
same rule. The server then speaks of the
project — `:diagnostics` lists every file's errors, as rust-analyzer's
check already does — and a reference or a rename reaches files no
buffer holds.

A loaded file is the **pool's document**, not a buffer's. A buffer
opened on it takes the document over: the text is the disk's, so
nothing is sent unless it differs (a `didChange`, not a second
`didOpen`). A buffer closed hands it back: the disk's text again, a
`didChange` if the buffer had left it different. Switched off, the
loaded files are closed and their diagnostics dropped. More files than
`load_max` load the first by path and say so once.

The documents are counted apart — `:lsp info` shows `loaded N` beside
`docs`, and the title bar's count stays the buffers'.

### 4. `inlay_hints` per server

`lsp.<name>.inlay_hints` decides for its languages' buffers; unset,
`lsp.inlay_hints` does. `<leader>cI` stays the global switch.

### 5. `:lsp toggle RULE [LANGUAGE]`

Flips a rule that is on or off (`enabled`, `load_all`, `inlay_hints`)
for the server of the caret buffer's language — or `LANGUAGE`'s — in
the session layer, and says where it now stands. The session layer is what `:set`
writes, so the settings tab shows it as the session's and `:set
lsp.typescript.load_all -` takes it back.

### 6. A plugin's rules, set where the shell's are

Asked 2026-10-03, from "Not built" below: a rule that is not the
shell's — a plugin that organizes imports on save, runs a server's
fix-all, or turns a code lens on, per server and per project.

`kawoosh.lsp.rule(name, { doc =, default = })` declares one. It is set
exactly as the shell's rules are: `lsp.NAME.RULE` in the settings'
layers — the user's file, a project's `.kawoosh/settings.lua`, the
session — for the server `NAME`, and, like `inlay_hints`, `lsp.RULE`
for every server that does not say; then `default` (`false` unless
said; any value a setting holds). On or off, it is a switch: `:lsp
toggle RULE [LANGUAGE]` flips it for the caret's server in the session,
as it flips `load_all`. `:lsp info` lists it where it is set, with its
origin (`organize (project: /repo/.kawoosh/settings.lua)`).

`kawoosh.lsp.rules(where)` reads them for a buffer (a handle, the
current one by default) or a language (a name): `{ server = NAME,
enabled =, load_all =, load_max =, inlay_hints = }` and every rule
plugins declared, each resolved as above. `server` is the `lsp.NAME`
the rules go under — a `.tsx` buffer's are `typescript`'s (Decision
1), which the shell tells Lua whenever its table moves
(`Runtime::set_lsp_names`), so the plugin need not know which server
serves what.

The pool never reads a plugin's rule: the plugin does, when it acts
(on a write, on a key), so a rule switched mid-session holds from its
next read and nothing restarts. A rule's name is lowercase, digits and
`_`, and none of `lsp.NAME`'s own keys or `lsp`'s (`cmd`, `settings`,
`load_all`, `languages` …) — `lsp.RULE` would read as one — and a
table under `lsp.RULE` is not taken for a server. The same name again
replaces the rule.

Beaten: **rules in `kawoosh.lsp.server`'s table**, beside a server's
data: a rule is a plugin's, not a server's, and reaches every server —
a format-on-save plugin's switch is the same for rust-analyzer and
gopls. **`kawoosh.setting` per server** (`lsp.rust.organize` declared
for each name): the names are the table's and move with it; one
declaration under `lsp.*` is what a rule is. **A rule `enabled` by
the shell for the plugin** (the pool skipping a server, say): a rule
the shell acts on is the shell's to add; a plugin's means what the
plugin does with it.

## Beside it

`didOpen`'s `languageId` is the document's own language now, where it
was the language that started the server: a `.tsx` opened on a server
started by a `.ts` was sent as `typescript` and read without JSX. `tsx`
is sent as `typescriptreact`, LSP's name for it.

## Built

2026-09-26, as decided; one server for several languages the same
day, from the first look at it. `:lsp info` lists what each server
serves and its rules. The shell's half is `kawoosh/src/lsp_rules.rs`
(`lsp_table` folds the settings over the definitions, `sync_lsp_rules`
diffs the table and restarts, stops or closes; `:lsp toggle enabled`,
`load_all`, `inlay_hints`); the pool's is `systems/src/lsp.rs` — a
`Document` whose `buffer` is `None` is a loaded file, `reconcile_loads`
walks on a thread (`load`) when a rule is switched on or a server comes
up, `Cmd::Stop` for a command no language uses now, and a server's new
`settings` sent as `workspace/didChangeConfiguration` where a restart
is not needed. `kawoosh/tests/lsp.rs`'s
`rules_load_all_hints_and_enabled`, `the_settings_table_is_the_server` and `one_server_serves_typescript_tsx_and_javascript`
run it against the fake server, and a throwaway probe against
typescript-language-server 6.0.1 on TypeScript 5.9.3 (configured by
`lsp.typescript` alone): a `.ts` and a `.tsx` never opened had their
errors listed within seconds of `load_all`. npm's `typescript` is 7
now, the native port, with no `tsserver.js`; typescript-language-server
on it answers nothing, so it wants `typescript@5` beside it.

Found on the way: `:lsp restart` said each buffer the server held is
sent again, and one no pane showed was not; a reset now keeps them in
`also_sync`.

`:lsp logs [LANGUAGE]` came with it, asked while configuring clangd:
what a server said — stderr included, which the notification log drops
as a trace — kept per server and shown live (`kawoosh/src/lsp_logs.rs`).

## Not built

- **A loaded file changed on disk by another program** is not read
  again; the server holds what the walk read until a buffer opens it,
  the rule is switched, or the server restarts. A
  `workspace/didChangeWatchedFiles` registration is the way.
- **The pull model, workspace-wide** (`workspace/diagnostic`) —
  lists.md's; with it a server that answers would need no `load_all`.
  A document's own pull is built ([lsp-installs.md](lsp-installs.md)
  Decision 7).
- ~~**Rules a plugin defines**: the table is open, but only the shell
  reads its rules.~~ Built 2026-10-03 (Decision 6):
  `kawoosh.lsp.rule` and `kawoosh.lsp.rules` (`lua/src/lib.rs`),
  `add_lsp_rule` and `tell_lsp_names` with the plugin's rules in
  `:lsp info` and `:lsp toggle` (`kawoosh/src/lsp_rules.rs`). Test:
  `kawoosh/tests/lsp.rs`'s
  `a_plugins_rule_is_set_and_flipped_as_the_shells_are` (toggled from a
  default either way, the session's word, a project's, `lsp.RULE` for
  every server, `.tsx` read under `typescript`).
