# Installs: servers in kawoosh's folder, and qd on both sides

Status: written 2026-10-02 from the user's ask, after a look at how
Zed and mason.nvim install servers — "i don't want to track versions,
i want their versions to be pulled from their package managers", then
"maybe we should make plugin that interface kawoosh, we can add init,
lists statuses, add, init settings sync. and we also can do is to make
a qd plugin for pulling lsps with native package managers. however i
would like the code to be put under our repo so servers could survive
version managers like `uv`/`fnm`", and "Both" when asked which way the
plugin faced. The calls below are taken here, each the user's to
overturn. Follows [lsp-servers.md](lsp-servers.md), whose install
lines were run as the user would, globally.

## What there was

`:lsp install` ran a server's line — `npm i -g yaml-language-server`
— in a terminal. Global, so:

- **A version manager hides it.** Under fnm, `npm i -g` installs into
  the current Node's own prefix; `fnm use 22` and the server is gone
  from the PATH. A `pip install` under uv lands in one Python. The
  user runs fnm and uv (their dotfiles' `javascript` and `python`
  modules).
- **Nothing knew what was installed**, so nothing could update it.
- **Only kawoosh could do it.** A new machine set up by qd, the user's
  dotfiles manager (`~/projects/qd`), had no servers until each file
  type was opened and each line run by hand.

## Decisions

### 1. A server's package goes in kawoosh's folder, by the user's own manager

`kawoosh_systems::servers`: a package — its manager, the packages, any
arguments — is installed under `servers/MANAGER/NAME` beside the state
database (`~/.local/share/kawoosh/servers`, `$KAWOOSH_SERVERS`), with
each manager's own way of keeping out of its global place:

| manager | run as | the program in |
|---|---|---|
| npm | `npm install --prefix DIR NAME@latest` | `node_modules/.bin` |
| pip | `uv tool install --upgrade [--with EXTRA] NAME`, `UV_TOOL_DIR`/`UV_TOOL_BIN_DIR` in DIR; without uv, a venv and its pip | `bin`, `venv/bin` |
| cargo | `cargo install --root DIR NAME` | `bin` |
| go | `GOBIN=DIR/bin go install NAME@latest` | `bin` |
| dotnet | `dotnet tool install` (then `update`) `--tool-path DIR NAME` | DIR |

The server's files are kawoosh's, so switching Node or Python takes
nothing away; it runs on whichever `node` or `python` is current (an
npm package's program is `#!/usr/bin/env node`; uv's tool keeps the
Python it was made with). A server is started from there before the
PATH is looked at (`servers::find`, in `Server::spawn`).

Not driven: brew (one global place already, and no version manager in
front of it), rustup (rust-analyzer is a toolchain's component), gem
(a gem's binstub reads `GEM_HOME`, and a native gem is one Ruby's
anyway), opam, ghcup, coursier, raco, nix. They keep their line.

Beaten: our own downloads of release binaries, Zed's way (each
server's assets named its own way, ours to follow); a registry of
pinned versions, mason's (the user: no versions tracked); one shared
`node_modules` for every npm server (one bad package's install breaks
the others').

### 2. No version is kept *(the user's)*

An install asks for the latest — `@latest`, `--upgrade`, cargo's
reinstall of an older one — and an update is the same install again.
What a directory holds is a record, `kawoosh-package.json`: the
package as asked for, no version, so `kawoosh lsp update` knows what
to ask again. A version a server needs is the package's to say
(`typescript@5` for typescript-language-server, `pygls<2` beside
cmake-language-server); nothing else pins.

### 3. `kawoosh lsp`, the CLI, is the one way in

`kawoosh lsp install NAME…`, `update [NAME…]`, `remove NAME…`, `list`
— no window, no socket. NAME is a server's `lsp.NAME` or a language it
serves. `:lsp install` runs `kawoosh lsp install --spec JSON NAME` in
a kept terminal pane (no shell between: the user's is nu); `:lsp
update` runs `kawoosh lsp update`; either ending well restarts the
servers on those commands. `--spec` carries a package the settings say
(`lsp.NAME.install = { npm = … }`), which the CLI does not read.

`lsp.NAME.install` is a line or a package: `{ npm = "x" }`, `{ pip =
{ "x", "dep<2" } }`, `{ cargo = "x", args = { … } }`. A `cmd` that
names another program clears both, unless a new one is said.

### 4. The servers a machine wants are a setting, synced as the settings are *(the user's)*

First built as a qd plugin — a `kawoosh = { lsp = { … } }` key in a
dotfiles module, its `packages.install` running `kawoosh lsp install` —
and taken out the next day: "no plugin in qd is not right. let just add
list of ensure installed lsps inside a kawoosh itself. kawoosh need to
have easy setup for it's sync into qd. and opening as a workspace
dotfiles."

- **`lsp.ensure_installed`**, a list in kawoosh's settings (`{ "rust",
  "yaml@1.15.0", "eslint" }`), is what a machine installs: kawoosh does
  it at launch and when the settings change, in the background, a
  corner line saying so; what is in stays at its version (Decision 6).
- **`:qd setup`** is the one step to keep kawoosh's config folder —
  `settings.lua` with that list in it, `init.lua`, plugins — in the
  dotfiles: module `kawoosh`, `fonts/**` left out, pulled in; one that
  keeps the folder already is pulled again. A new machine's `qd init`
  brings the settings, and kawoosh's first launch the servers. qd needs
  no plugin and knows nothing of servers.
- **`:qd open`** opens the dotfiles repository as a workspace: a tab on
  it, its working directory (workspaces.md Decision 6's gesture).

Beaten: the qd plugin (qd's `packages` runs one manager, and the list
lived apart from the settings it serves).

### 6. Versions are locked, any one can be had, an update is said and not made *(the user's)*

Asked 2026-10-03, over Decision 2's "no version is kept": "however we
do not lock, i would lock installed versions so we can indicate updates
and don't cause breaks when it's unnessesary. basically we should give
ability to install any version."

- **An install is at a version**: the one asked (`kawoosh lsp install
  yaml@1.15.0`, `"yaml@1.15.0"` in `lsp.ensure_installed`) or, with
  none, the registry's latest, asked first — `npm view`, `go list -m`,
  PyPI's, crates.io's and NuGet's JSON with curl — and installed at
  exactly that (`pkg@1.15.0`, `pkg==1.15.0`, `cargo install --version`,
  `@v0.17.1`, `dotnet tool --version`). The record keeps it. A cargo
  `--git` has no version.
- **Nothing moves it unasked**: a name in `lsp.ensure_installed` that is
  in stays at its version; one pinned is installed again when the one
  in is another.
- **Updates are said**: `kawoosh lsp outdated` asks the registries;
  with `lsp.ensure_installed` set, kawoosh asks once a day
  (`lsp.check_updates`, on), keeps the answer in each record and says
  in the corner what `:lsp update` would bring; `:lsp servers` and
  `kawoosh lsp list` show `1.14.0 → 1.24.0`. `update` installs the
  latest only where it is newer.

### 7. Several servers for a language, each where the project says so *(the user's)*

Asked 2026-10-03: "also we should run multiple servers for a language
for example typescript = {"eslint", "typescript-language-server"} but
detect whether to run it. for example eslint if eslint config exists."

- **A buffer has a list of servers.** Every server that serves its
  language runs for it, in the table's order — or, with
  `lsp.languages.LANG = { … }` (a server's `lsp.NAME` or its command),
  those named, in that order and no others.
- **`when`**: a server with `when` files runs for a file only where one
  of them is at or above it, up to the repository's root — eslint where
  an `eslint.config.*` or `.eslintrc*` is, biome where a `biome.json`
  is, ruff where a `ruff.toml` is. Such a server runs *beside* the
  language's own and keeps its languages; one without `when` named for
  another language's files still gives them up to that language's own
  (lsp-rules.md Decision 1). Looked up once a directory, again after
  `:lsp restart` or a settings change.
- **Who answers what**: a request goes to the first of the buffer's
  servers whose capabilities say it answers it (eslint has no hover:
  typescript's answers even with eslint first) — the first of all when
  none says so. Code actions are asked of every server and gathered,
  and a command runs on the server that offers it. Diagnostics are
  every server's together, per document. A language's capabilities, as
  the shell reads them, are its servers' together.
- **One process per command and arguments**: two definitions run alike
  share a server; two run differently (a fake server and the same
  fake as a linter) do not.
- **Counting on a server**: a server beside (`when`) is not counted as
  the language's — a missing eslint is not why TypeScript has no
  server, and a formatter list does not wait on biome where there is
  no `biome.json`.

- **Pulled diagnostics**: a server that gives diagnostics only when
  asked (`diagnosticProvider` — vscode-eslint does nothing else) is
  asked for a document's (`textDocument/diagnostic`) after each sync,
  again for all of its documents on `workspace/diagnostic/refresh`, and
  the answer joins the others as a push would. lists.md's "pull model"
  is built this far; `workspace/diagnostic` is not.
- **A code action's diagnostics go back as they came**: `source` and
  `code` with them (a number as a number) — eslint finds its fixes by
  the rule's code, and offered none without.
- **Who is counted**: a buffer is sent while any of its language's
  servers is up, a linter beside included (`lsp_syncs`); a request is
  made when the language's own server is up or a server the pool says
  holds the buffer is (`Event::Holders`) — eslint's fixes in a
  JavaScript project with no TypeScript.

Builtin beside: eslint (`vscode-eslint-language-server`, from
`vscode-langservers-extracted`, the editor extension's settings with
`workspaceFolder` answered as the root and its
`eslint/confirmESLintExecution` approved), biome (`biome lsp-proxy`),
ruff (`ruff server`).

### 5. kawoosh → qd: a `:qd` pane *(round three)*

A Lua plugin in kawoosh over qd — linked as a library, asked by the
user after the first build over the CLI ("you probably can interface by
building a library … publish qd even on a crates.io as well as drydock9
registry"), and the binary when the two differ:

- **qd is a library**: `qd::Session` (qd's `src/session.rs`) holds the
  state and the repo and does every operation, returning data; the
  binary is a printer over it, behind a default `cli` feature. The
  crate is `qdot` (crates.io's `qd` is a float crate); lib and bin stay
  `qd`. drydock9 from CI on a tag, crates.io by hand once qd has a
  license — it has one since 0.2.0, and kawoosh takes `qdot` from
  crates.io (2026-10-04), naming no registry of its own.
- **Which door**: the library while `qd::VERSION` is the version of the
  `qd` on the PATH, or there is none — they share `state.toml`, the
  journal and the trash, and an older reader of a newer journal is
  where `undo` goes wrong — and the binary otherwise. Flags and `qd
  init` always go to the binary.
- **One at a time**: the library's operations run on one worker thread
  in the order asked (`kawoosh/src/dotfiles.rs`). A test typing `<lt>`
  — four keys, the last `>` — ran a push into a pull in flight and
  removed a file the pull was writing; qd's own CLI never meets that,
  being one process a command.

The first build, over the CLI alone: `:qd` opens the status —
`qd status --json`, by module, what push and pull would do — with
keys to push, pull, add a file to a module, and `qd init` on a new
machine. Shipped with kawoosh like the VCS panes; nothing when `qd` is
not on the PATH.

## Built

Round one, 2026-10-02: Decisions 1–3. `systems/src/servers.rs`
(`Package`, `steps`, `install`, `find`, `installed_in`, `remove`),
`ServerDef::package` and the builtin rows' packages (npm, pip, cargo,
go, dotnet) in `systems/src/lsp.rs`, `kawoosh/src/lsp_cli.rs`,
`:lsp install`/`:lsp update` in `kawoosh/src/lsp.rs` over
`spawn_bang_argv`, `lsp.NAME.install` tables in `lsp_rules.rs`. Tests:
`servers.rs`'s three (each manager's commands into kawoosh's folder,
what is missing named, a program found by its record),
`lsp_cli.rs`'s names, and `kawoosh/tests/lsp.rs`'s
`a_package_is_installed_into_kawooshs_directory_and_run_from_there` (a
fake `npm`, the real `kawoosh lsp install` in the pane, the server
started from the folder). Run for real into a scratch folder:
yaml-language-server by npm; cmake-language-server and fortls by uv —
cmake-language-server broke on pygls 2, hence its `pygls<2`.

Round two, 2026-10-02: the qd plugin, `contrib/qd/kawoosh.lua` —
taken out 2026-10-03 for Decision 4 as it now reads.

Round four, 2026-10-03: Decision 7 in the pool (`systems/src/lsp.rs`):
`homes` a list a buffer, `Cmd::Order`, `ServerDef::when` with
`marked`, `answerer` by `Caps::answers`, code action groups, merged
`published` diagnostics, `Caps::union`; servers keyed by their
arguments too. The shell's `lsp.languages` (`sync_lsp_order`, its
languages' buffers sent again) and `lsp.NAME.when`. The fake server's
`--linter NAME`; test `a_linter_runs_beside_the_language_server_where_its_config_is`.
Tried for real headless: `kawoosh lsp install eslint` into a scratch
folder (vscode-langservers-extracted 4.10.0), a project with eslint 9
and a flat config — eslint started there and TypeScript said why it
did not; eslint's `no-unused-vars` pulled, and its four code actions
(remove, disable for the line, for the file, documentation) offered
once the diagnostics went back with their codes.

Round three, 2026-10-02: Decision 5, over the `qd` CLI.
`kawoosh/lua/qd.lua` (bundled after vcs): `:qd`, `:qd push`, `:qd
pull`, `:qd add [PATH] [NAME]` (bare: kawoosh's config folder as
`kawoosh`, `fonts/**` ignored — a bought font is not the user's to
publish, and the dotfiles mirror to GitHub), `:qd init [URL]`; `>`
`<` `<CR>` `a` `r` in the pane; `kawoosh.qd.status(fn)` and
`kawoosh.qd.state()`. `kawoosh.json.decode`/`encode` came with it.
Test `kawoosh/tests/qd.rs` against a fake `qd`; seen in a window
against the user's own repo (thirteen modules, wezterm's `ui.lua`
differing), read only. Asked while it was built: "you probably can
interface by building a library … publish qd even on a crates.io as
well as drydock9 registry" — the CLI stays the door until that is
decided (the version a linked qd would be, against the one the user
runs on the same state, is the question).
