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

Left out: a server that needs `initializationOptions` kawoosh did not
send (astro-ls wants its TypeScript's path; Decision 8 since), one with no settled
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

## Tried 2026-10-03 on Windows 11

Asked as one item of a round: "The install commands for the 47
language servers: each one is copied from the server project's docs,
and none has been run." Every server kawoosh installs itself was run
through `kawoosh lsp install NAME` into a scratch folder — HOME,
USERPROFILE, APPDATA, LOCALAPPDATA, TEMP, `KAWOOSH_SERVERS`,
`KAWOOSH_STATE`, the settings, `XDG_*`, `CARGO_HOME`, `GOPATH`,
`GOCACHE`, `NUGET_PACKAGES`, `DOTNET_CLI_HOME` and the npm and uv
caches all pointed into it, rustup's toolchains read where they are —
and each program, found as `servers::find` finds it, was sent
`initialize` and answered or not. Each manager stays in the package's
directory as lsp-installs.md Decision 1 says: npm `--prefix`, uv's
`UV_TOOL_DIR`/`UV_TOOL_BIN_DIR`, `cargo install --root`, `GOBIN`,
`dotnet tool --tool-path`. A line kawoosh does not drive was not run
where it installs globally (rustup, winget); where its manager is not
on this machine it was not fetched, and the package's name was looked
up in its registry instead. uv made its tools on the CPython it had
installed already (found, not written to). The user's own package
folders — `~/.cargo/bin`, `~/go`, `~/.dotnet/tools`, `~/.nuget`, the
global npm, uv's tools and Pythons, `~/.config/kawoosh`,
`~/.local/share/kawoosh` — were listed before and after: no entry was
added or removed. The one rewritten, `state.db`, was another kawoosh's
on the machine; `kawoosh lsp` opens no state, and the scratch
`KAWOOSH_STATE` stayed empty.

| server | how | installed | answers `initialize` |
|---|---|---|---|
| rust | `rustup component add` | not run (a toolchain's component; in already) | rust-analyzer 1.99.0, on the PATH |
| typescript | npm | typescript-language-server 6.0.1, typescript@5 | yes |
| lua | brew; none on Windows | not tried | 3.19.1, on the PATH (scoop) |
| python | npm | pyright 1.1.414 | yes |
| go | go | gopls v0.23.0 | yes |
| c | `winget install LLVM.LLVM` | not run (global); the package is there, 23.1.2 | clangd 23.1.0, on the PATH (scoop) |
| bash | npm | bash-language-server 5.8.1 | yes |
| fish | npm | fish-lsp 1.1.5 | no: it runs `fish`, which Windows has not |
| nu | `winget install nushell` | not run (global); resolves to Nushell.Nushell 0.116.0 | `nu --lsp` 0.115.0, on the PATH |
| html, css, json, eslint | npm | vscode-langservers-extracted 4.10.0 | yes, all four |
| yaml | npm | yaml-language-server 1.24.0 | yes |
| toml | cargo | taplo-cli 0.10.0 (`--locked --features lsp`) | yes |
| dockerfile | npm | dockerfile-language-server-nodejs 0.15.0 | yes |
| svelte | npm | svelte-language-server 0.18.4 | yes |
| php | npm | intelephense 1.18.5 | yes |
| csharp | dotnet | csharp-ls 0.28.0 | yes |
| fsharp | dotnet | fsautocomplete 0.84.0 | yes |
| elm | npm | @elm-tooling/elm-language-server 2.10.0 | yes |
| purescript | npm | purescript-language-server 0.18.5 | yes |
| cmake | pip (uv) | cmake-language-server 0.1.11, pygls 1.3.1 | yes |
| fortran | pip (uv) | fortls 3.2.2 | yes |
| prisma | npm | @prisma/language-server 31.12.10 | yes |
| proto | cargo | protols 0.14.1 | yes |
| dot | npm | dot-language-server 3.2.1 | yes |
| awk | npm | awk-language-server 0.10.6 — failed bare: its tree-sitter-awk has no prebuilt binary for Node 25 and node-gyp found no Python (`python` is the Store's alias); installed with `PYTHON` naming one | yes |
| wgsl | cargo `--git` | wgsl-analyzer at d368db82, no version | yes |
| biome | npm | @biomejs/biome 2.5.15 | yes (`lsp-proxy`) |
| ruff | pip (uv) | ruff 0.16.10 | yes |
| ruby, scala, haskell, ocaml, racket, r | gem, cs, ghcup, opam, raco, R | not tried (none here); ruby-lsp 0.26.11, metals, hls, ocaml-lsp-server, racket-langserver and languageserver are where the lines look | — |
| markdown, java, kotlin, dart, zig, elixir, erlang, gleam, clojure | brew; none on Windows | not tried; each formula is there | — |
| nix | `nix profile install` | not tried (no Nix on Windows) | — |
| glsl, odin, luau | none known | — | — |

Twenty-five packages, every one kawoosh drives: all installed (awk
given a Python), and all but fish-lsp answered.

Found and fixed:

- **No npm package installed on Windows at all**: `npm` is `npm.cmd`
  there, and `std::process::Command` looks for `npm.exe` alone —
  "program not found" before anything ran. `io::command` takes a bare
  name that is no `.exe` on the PATH but a `.cmd` or `.bat` by that
  file's path (`io::shim`), which std runs through `cmd.exe`. It is
  every child's: `npm view`, and a server npm put on the PATH
  (`typescript-language-server.cmd`), which was not found either. One
  kawoosh installed was always started by its path, `.cmd` included.
- **R's line did not survive `cmd /C`**: `R -e 'install.packages(…)'`
  quotes with `'`, which cmd does not read as quoting. Double quotes
  outside, single inside, which sh reads the same.
- **nil's line ran on Windows**, where there is no Nix: mac and Linux
  only now, so `kawoosh lsp install nix` says no way is known.
- **A line whose manager is not here** went to the shell and ended with
  cmd's "not recognized". It is looked for first and said as a
  package's missing manager is: `` `gem install ruby-lsp` needs `gem`,
  which is not on the PATH ``.
- awk and fish keep their rows, each with a comment saying what it
  needs on Windows.

Guards: `io.rs`'s `a_cmd_on_the_path_is_taken_by_its_path`;
`lsp_rules.rs`'s `every_install_line_runs_a_known_manager_and_reads_under_cmd`
(every row's line on every platform starts with a manager some row
runs, and one Windows runs has no `'` outside `"`); and, over real
installs, `kawoosh/tests/lsp.rs`'s ignored `installed_servers_answer_kawoosh`
— kawoosh itself starting yaml, taplo, pyright, gopls, the json and
css servers, bash and typescript from `KAWOOSH_SERVERS` and each
answering, under a second each here.

Not tried: the pip fallback without uv (on Windows `python` is often
the Store's alias, which `on_path` finds and which runs nothing); a
global line run for real.

### 8. What a server is started with: `init`

*Added 2026-10-07*, from "Not built" ("then do the lsp ones"): "a
server's `initializationOptions`, which would bring astro-ls and
Volar". `settings` is what a server reads when it asks
(`workspace/configuration`) and is told again as it changes; some
servers read nothing but what `initialize` carried —
`initializationOptions` — and astro-ls will not start without
`typescript.tsdk` there, the TypeScript it runs.

`init` is that, on a row and in `lsp.NAME`, as data like the rest:

```lua
lsp = { astro = { init = { typescript = { tsdk = "{typescript}" } } } }
```

- **Said once.** It goes with `initialize` and nowhere else, so a change
  to it is a new server: `sync_lsp_rules` restarts one that runs, as it
  does for a new command or arguments.
- **What a row cannot know, said in words.** A path depends on the
  project, so two words in a string are put in as the server starts
  (`init_options`): `{root}`, the server's root, and `{typescript}`, a
  TypeScript's `lib` — the nearest `node_modules/typescript/lib` at or
  above the root (the project's own, what its build type-checks with),
  else one kawoosh installed beside a server
  (typescript-language-server's, installed with `typescript@5`), else
  the word as written, for the server to say what it misses. On a host
  only `{root}` is said, as the host spells it: the host's disk is not
  looked at from here.
- **astro-ls is a row** (`astro`, installed as
  `@astrojs/language-server` with `typescript@5`, so `{typescript}`
  has its own when the project has none). Not tried against astro-ls
  itself: none on the machine it was built on.

Volar is not a row. `@vue/language-server` 3 runs only beside a
TypeScript server carrying its plugin, the two passing `tsserver/request`
between them — a protocol of its own, not an option; 2.x's
`vue.hybridMode = false` would be an `init` away, but there is no Vue
grammar in the manifest for its files to be a language.

Beaten: a Lua function for a row's `init` (a row is data, read before
any other Lua runs, and by `kawoosh lsp` on the command line); `settings`
sent as `initializationOptions` too (a server reading both would read
its configuration twice, and some refuse an unknown key in one);
`{typescript}` asked of `node` (`require.resolve`) — a process per
start where a walk up the directories finds what node would.

## Not built

- ~~**Install lines checked against the package managers.**~~ Tried
  2026-10-03 on Windows 11, above: every package kawoosh installs, and
  the lines' packages looked up. Not on macOS or Linux, and not the
  lines whose managers this machine has not.
- ~~**A server's `initializationOptions`**, which would bring astro-ls
  and Volar.~~ Decision 8: astro-ls; Volar waits for a Vue grammar and
  its TypeScript plugin's protocol.
- ~~**More than one server for a language**~~ (a linter beside the
  language's server, ruff beside pyright). Built 2026-10-03:
  [lsp-installs.md](lsp-installs.md) Decision 7, `when` files.
