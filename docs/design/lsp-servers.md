# Servers: one for every language kawoosh knows, and a quiet word

Status: written and built 2026-10-02 from the user's ask — "Let's
integrate more lsps. Maybe the same way grammar does, but in this case
it's probably not required. And make lsp notifications just info, not
a toast", then "It toasts about missing lsp". The calls below are
taken here, each the user's to overturn. Companion to
[lsp-rules.md](lsp-rules.md), whose `lsp.NAME` table this fills, and
to [grammars.md](grammars.md), whose seventy languages had files and,
once installed, colours, but no server.

## What there was

- **Six servers**: rust-analyzer, typescript-language-server,
  lua-language-server, pyright, gopls, clangd. A Ruby, Zig, YAML or
  Markdown file had none unless the user wrote its `lsp.NAME = { cmd
  = … }` by hand.
- **Nothing said how to get one.** A server not on the PATH was
  "`X` not found; lsp off. Installed since? :lsp restart", which left
  the finding to the user.
- **A server's word was a toast**: `window/showMessage` of type error
  or warning, a server that exited and was started again, and above
  all the "not found" of every file whose server is not installed. The
  notes landed over the text, top right, for eight seconds, about
  something the user had not asked for. help/code.md already said "the
  corner says what it said"; the code toasted.

## Decisions

### 1. A server for every language a known one serves

The builtin servers are a table — since 2026-10-03 Lua data,
`kawoosh/lua/servers.lua` (Decision 5); `BUILTIN` in
`systems/src/lsp.rs` before —
each row a server by the language it is first for, the languages it
serves, its command and arguments, its root markers and its install
line. It covers the builtin languages (bash, CSS, JSON, YAML, TOML,
Markdown, nu, `go.mod` through gopls) and the installable ones whose
server is the one every editor's LSP setup reaches for: HTML, SCSS,
Dockerfile, Svelte, PHP, Ruby, Java, Kotlin, Scala, C#, F#, Dart, Zig,
Haskell, OCaml, Elixir, Erlang, Gleam, Elm, PureScript, Clojure,
Racket, Nix, CMake, Fortran, R, Prisma, protobuf, Graphviz, AWK, WGSL,
GLSL, Odin, Luau, fish, Objective-C through clangd. A test holds every
row's languages to ones the registry or the grammar manifest names.

Left out: a server that needs `initializationOptions` kawoosh does not
send (astro-ls wants its TypeScript's path), one with no settled
stdio command (SQL, Julia, Perl, PowerShell, Bicep), and languages
whose files are data (CSV, INI, `.env`).

A server is still started only for a file of its language and only
if its program is found; a builtin costs nothing until then.

### 2. Each server knows its install line; kawoosh runs it when asked *(not the grammars' way)*

Grammars are built ahead and fetched because no one else ships them
(grammars.md Decision 1). Servers are shipped by their projects
through package managers the user already has — npm, cargo, go, gem,
dotnet, opam, ghcup, pipx, brew — and a fetched server would be ours
to update forever. So a server carries one line, `install`, the one
its project documents: `npm i -g yaml-language-server`,
`gem install ruby-lsp`, `rustup component add rust-analyzer`. Where
the lines differ by platform the row holds one for each (brew on macOS
and Linux, none known on Windows where no package manager is sure);
`""` for none known.

- **`:lsp install [LANGUAGE]`** runs the line of the caret language's
  server, or LANGUAGE's, as a `:!` would: in a terminal pane of its own
  (`terminal.place`), in the working directory, kept when it ends, `r`
  running it again. It is the user's command, seen run, with its
  prompts answered in the pane — never a download behind their back.
  Ended with 0, the server's command is looked for again and started
  for the buffers that missed it (`lsp_restart_commands`), and the
  corner says so.
- **`:lsp servers`** lists every server there is to run: its
  `lsp.NAME`, whether it is running, off, found or missing on the PATH,
  its command and languages, and a missing one's install line.
- **`lsp.NAME.install`** in the settings (and `install` in
  `kawoosh.lsp.server`) says the line; a `cmd` that changes the program
  clears the old program's line unless a new one is said.

Beaten: the grammars' prebuilt archives (Decision 1's reasons do not
hold for servers); installing on the first file (a package manager run
unasked); an install toast with a button (Decision 4); a list pane
with keys, as `:grammars` has (one command does what its `⏎` would; a
text pane is `:lsp info`'s way, and enough).

### 3. "Not found" names the way in

`` `yaml-language-server` not found; lsp off. :lsp install yaml `` when
its line is known; the old "Installed since? :lsp restart" when not.
It is said once a session per command, as before.

### 4. A server's word is a corner line *(the user's)*

Everything a language server says, or kawoosh says of one, is a corner
line (`Show::Corner`): `window/showMessage` of every type, a server
exited and started again, one that is not found, did not start or kept
exiting. Each is still logged at its own level — `:messages` shows an
error as an error — and `:lsp logs` keeps the server's own words. A
corner line is one line cut at the corner's width, so each note leads
with what matters; the rest is in the log.

Toasts stay for what is kawoosh's own to say and the user's to answer
— a grammar's offer, a settings mistake. A server has none of those.

### 5. The table is Lua data, in the shape `lsp.NAME` has *(the user's)*

Asked 2026-10-03: "maybe we should extract data from code about lsps?
put the lsp configuration in a separate repo like we did with
grammars?", then, on the answer below, "let's do that in lua".

`kawoosh/lua/servers.lua` returns the rows, a list in the asking order
(a language's servers, when `lsp.languages` does not say, are asked
first to last — typescript's before eslint's and biome's). A row is
what `lsp.NAME` says, with its `name`: `cmd`, `args`, `roots`,
`languages`, `when`, `install`, `settings`, `answers`. It is read as a settings
file is — the pure library, nothing that reaches out, no `kawoosh` —
in a Lua of its own (`kawoosh_lua::eval_data`), once, so `kawoosh lsp`
on the command line, which has no runtime, reads it too. Local
helpers are allowed (`brew(formula)`, eslint's configs and settings);
what it returns is data.

One reading for the three places a server is said
(`lsp_rules::fold`): a row of the table, `kawoosh.lsp.server(name, t)`
and `lsp.NAME` in the settings. So `kawoosh.lsp.server` takes `when`
and a package `install`, which it did not, and every place takes an
install line by platform, `{ mac = …, linux = …, windows = … }` (this
machine's taken, `""` where it has none) — what `Install::Os` was in
Rust, now a user's too: a settings file kept by qd across a Mac and a
Windows machine says both. The rules (`enabled`, `load_all`,
`load_max`, `inlay_hints`) stay the settings' alone, as lsp-rules.md
decided.

Two values Lua data cannot hold changed: eslint's `workspaceFolder`,
`null` before and answered with the server's root, is the string
`"root"` (a `{ uri, name }` object is what the server reads, so no
real value is that string); its `rulesCustomizations = []` is left
out, which the server reads as none (an empty Lua table would go as
`{}`). Every other field of every row is what the Rust table gave,
compared row by row when it moved.

Beaten: **a repository of its own**, as the grammars have. The
grammars' repository is there to build libraries for six targets and
publish them where anyone can fetch; a server table is a few
kilobytes of text with nothing to build. Fetched, it would want a
cache, a manifest version between two repositories and a copy built
in for a first launch offline — the grammars' machinery for none of
their reasons — and what a server needs beyond data (`language_id`'s
spellings, how a manager installs) would stay in kawoosh's code. Once
the table is data the move is small, if there is ever a reason: rows
added without a release, other people's servers, another tool reading
the list. **TOML**: a second format beside the settings' Lua, and no
helpers. **The rows run in the user's runtime** (`kawoosh.lsp.server`
calls in a bundled plugin): the command line has no runtime, and the
table would be code, not data.

`answers` (asked the same day, "it would help future packages to
declare lsp integrations"): a server's own requests and the result
each is answered with, looked up before the pool's own answers. It
took the last server's name out of the pool — vscode-eslint's
`eslint/confirmESLintExecution`, answered 4 ("approved") by a match
arm, is the eslint row's `answers` now — and lets a row, a plugin's
`kawoosh.lsp.server` or a user's `lsp.NAME` answer a server's
extension without code. A fixed result only: an answer that must read
the request or the editor is the pool's.

`language_id` (`tsx` → `typescriptreact`) stays in
`systems/src/lsp.rs`: it is a language's name in LSP, not a server's,
and every server of the language reads it.

## Built

2026-10-02, as decided. `BUILTIN`, `Install` and `ServerDef::install`
in `systems/src/lsp.rs` (with `language_id` for `shellscript`,
`objective-c`, `go.mod`, `nushell`); `lsp_install`, `lsp_installed`,
`lsp_servers_text` and the corner notes in `kawoosh/src/lsp.rs`; the
install terminal's end in `terminals.rs` `term_closed`. Tests:
`kawoosh/tests/lsp.rs`'s `a_missing_server_is_installed_by_its_line`
(the corner line, `:lsp servers`, `:lsp install` ending in a started
server) and `progress_and_messages_land_in_the_corner` (a warning a
corner line, logged as a warning); `grammars.rs`'s
`every_builtin_server_serves_a_known_language`.

Decision 5, 2026-10-03: `kawoosh/lua/servers.lua`;
`lsp_rules::builtin` (read once) and `lsp_rules::fold` (the one
reading, with `platform_line`) in `kawoosh/src/lsp_rules.rs`;
`kawoosh_lua::eval_data`; `Msg::LspServer` carries the table as a
`Setting`; the pool starts with no servers until the shell sends its
table; `ServerDef::answers`, looked up first among a server's
requests. `BUILTIN`, `Install`, `ESLINT_CONFIGS` and `eslint_settings`
left `systems/src/lsp.rs`. Tests: `lsp_rules.rs`'s
`the_builtin_servers_are_read_from_lua` and
`an_install_line_by_platform`, `kawoosh/tests/lua.rs`'s
`a_lua_server_is_an_lsp_table`, `kawoosh/tests/lsp.rs`'s
`a_servers_own_request_is_answered_from_its_row` (the fake server's
`--ask METHOD`).

## Not built

- **Install lines checked against the package managers.** Each line is
  the server project's documented one as of this writing; none was run
  here but the fake server's.
- **A server's `initializationOptions`**, which would bring astro-ls
  and Volar.
- ~~**More than one server for a language**~~ (a linter beside the
  language's server, ruff beside pyright). Built 2026-10-03:
  [lsp-installs.md](lsp-installs.md) Decision 7, `when` files.
