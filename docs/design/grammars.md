# Grammars: built ahead, installed on demand

Status: decided and built 2026-10-01 (roadmap step 77), in the six
rounds of the build order and one after them; "Built" at the end says
where the build departed from the text.
Asked: "How
can we implement Tree-Sitter grammar auto installation? Like neovim
does, or at least `nvim-treesitter/nvim-treesitter` plugins do. We
don't ship all grammars since it will bloat the installation, but we
can pull at least from somewhere?", and then: "windows one of my
primary platform, so we should resolve this problem." The user's calls
are marked *(the user's)*; the rest are taken here and theirs to
overturn. Companion to [kui.md](kui.md) Decision 13, whose pluggable
grammar this fills.

## What there is

- **Twenty-two grammars linked in**, one cargo feature each, behind
  twenty-five languages (kui.md Decision 13). Built as shared
  libraries they are 15.9 MiB; three hundred at that average would be
  about 200 MB, so shipping every grammar is out.
- **Loading is done.** `kawoosh.language(name, opts)` adds a language
  whose grammar is a shared library: `Library::find` looks for
  `parsers/NAME.EXT` and `queries/NAME/*.scm` under the config
  directory, `Library::load` opens it, checks its ABI against what
  this tree-sitter reads (13 to 15 under tree-sitter 0.27) and
  compiles its queries, and `Kawoosh::add_language` tells the ts
  thread and sends the language's buffers again. Nothing fetches a
  grammar and nothing builds one.
- **Linked already:** `rusqlite` with its own sqlite (the state db),
  `flate2` (the terminal's graphics), `blake3` (trust records),
  `serde_json`. **Not linked:** `tar`, `sha2`, `zstd`, any HTTP client.
- **A progress line**: `notes.progress`, the corner line with a bar a
  language server's `$/progress` draws. A process off the frame:
  `Io::run_process_with`. A data directory: where `state.db` is
  (`$XDG_DATA_HOME/kawoosh`, else `~/.local/share/kawoosh`).
- **Queries do not inherit.** A `; inherits: javascript` comment is not
  read; `typescript` and `cpp` are written as additions in their
  modules.

## What others do

| | the list | a parser arrives by | on the machine |
|---|---|---|---|
| nvim-treesitter (archived 2026-04-03) | a revision per parser | the source at that revision, built with the `tree-sitter` CLI | the CLI and a C compiler |
| helix | `languages.toml`, a git revision each | `git fetch`, then the C compiler | git and a C compiler, or a package that built them all |
| emacs | a URL per language | `git clone`, then the C compiler | git and a C compiler |
| zed | extensions | a `.wasm` per grammar | nothing; a wasmtime in the editor |
| tree-sitter-language-pack | 371 definitions, repository and revision | one bundle per platform, 21–26 MB, all of them | nothing |

Every editor that builds on the machine has the same hole: a Windows
without a compiler gets no grammars.

## Measured

On macOS arm64 with zig 0.16, over the twenty-two grammars already in
the cargo registry:

- `cc -shared -fPIC -O2` builds each in 0.1–2 s. None needs the
  `tree-sitter` CLI or a C++ compiler.
- `zig cc -target T -shared -O2` built all twenty-two for six targets
  (x86_64 and aarch64, each of Windows, Linux at glibc 2.17, macOS)
  with no failure, 14–31 s a target, 15–16 MiB a target. The Windows
  libraries import `KERNEL32` and the UCRT alone, and export
  `tree_sitter_NAME`. **Not loaded on a Windows yet.**
- A grammar built `-O0` parses 10–50% slower than one built `-O2`
  (Rust, 121 KiB: 8.4 ms against 5.7 ms): the time is in tree-sitter's
  runtime, which is kawoosh's. A compiler that does not optimise would
  do.
- As sqlite archives, zlib inside: one target's library is 40–340 KiB
  a grammar (2.6 MiB the twenty-two); all six targets' in one archive
  60 KiB–1.9 MiB (11.9 MiB the twenty-two). The stock `sqlite3 -A`
  lists and extracts them, byte for byte.

## Decisions

### 1. Built ahead, not on the user's machine *(the user's)*

A grammar is fetched as a library already built for the machine.
Nothing is compiled where kawoosh runs, so a Windows with no compiler
installs a grammar as a Mac with Xcode does.

Beaten: **building on the machine first**, as helix and nvim-treesitter
do — no grammars on a Windows without a toolchain. **A TinyCC shipped
beside kawoosh**: about a megabyte and fast enough (the `-O0` figure
above), but its last release is 0.9.27 of 2017, Windows on arm64 is on
its unreleased branch alone, it is LGPL, and whether it builds the
scanners real grammars have is untested; it would also be a second way
to make every library, to keep honest against the first.
**tree-sitter-language-pack's bundles**: they work today, but it is
all 371 parsers or none (21 MB for one language), the bundle holds
libraries without queries or licences, its Windows bundles are two
months old, it has one maintainer, and it patches grammars and
regenerates 161 of them — a patch of its own shipped a segfault in
typst in 1.15.0. Its `language_definitions.json` (MIT) is a good seed
for the list. **A C compiler written in Rust**: none builds a Windows
library. **wasm**: declined in kui.md Decision 13, and a wasmtime
weighs more than the grammars it would let out of the binary.

### 2. A public repository of its own, on two hosts *(the user's)*

`kawoosh-grammars`: the list, the query files, the builder. It holds
nothing of kawoosh's or kui's — revisions of public repositories,
their licences, queries — so it is public while they are not. The
libraries are loaded into the editor, so what they were built from and
how is there to read. A grammar moves without a kawoosh release.

It lives on drydock9 and is push-mirrored to GitHub; CI builds on
both, and each host publishes its own release. Two builds are not the
same bytes, so **each host serves its own manifest, and kawoosh takes
a manifest and the archives it names from the same base**:

```lua
grammars = {
  url = {   -- tried in order; a base is a folder of `manifest.json` and `NAME.sqlar`
    "https://github.com/qxuken/kawoosh-grammars/releases/latest/download",
    "https://drydock9.qxuken.dev/qxuken/kawoosh-grammars/releases/download/latest",
  },
}
```

Beaten: **one build published to both**, the same hashes everywhere —
the hosts would no longer stand alone, the one waiting on the other's
upload. **drydock9 alone**: everyone's colours would hang on one
server. **Inside kawoosh's repository**: it is private, and an
endpoint that takes a token is no endpoint.

### 3. A grammar is a directory: a `grammar.toml`, a sample, the queries it changes

```
grammars/
  zig/
    grammar.toml
    sample.zig        # parsed in CI: no ERROR node
    queries/          # only what replaces or adds to the grammar's own
      indents.scm
  tsx/
    grammar.toml
```

```toml
# grammars/zig/grammar.toml
extensions = ["zig", "zon"]
filenames = []
shebangs = []
aliases = []

[source]
repo = "https://github.com/tree-sitter-grammars/tree-sitter-zig"
rev = "…40 hex…"
path = "."            # where src/parser.c is: "tsx" in typescript's repository
license = "MIT"       # SPDX; the text is copied from the checkout

[queries]
inherits = []         # ["javascript"]: those grammars' queries first
```

The directory's name is the language's, and the symbol is
`tree_sitter_NAME` unless `symbol` says otherwise. The names are
`LanguageDef`'s, so the same words are a `grammar.toml`'s, the
manifest's and `kawoosh.language`'s.

TOML *(the user's)*, since kawoosh never reads this file: the builder
does, in CI, and kawoosh reads the manifest. Beaten: **Lua**, which is
easier only where kawoosh reads it; here it would put an interpreter
in the builder and make every pull request to a public repository code
that runs in order to be read.

The builder is a small Rust program in that repository, on the crates
kawoosh uses for the other end (`rusqlite`, `flate2`, `blake3`), and a
pinned zig. For each grammar it:

1. fetches the source at `rev`; no `src/parser.c` there, or a scanner
   in C++, is refused — the grammar waits for its upstream;
2. builds the six libraries;
3. takes the checkout's `queries/` — `highlights.scm`,
   `injections.scm`, and `tags.scm` as the outline — lays the
   directory's `queries/` over them file by file, and writes each with
   its `inherits` in front, so the archive's queries are whole and
   kawoosh needs no `; inherits:`;
4. checks, with the host's library: every query compiles against the
   grammar, and the sample parses with no ERROR node. A revision moved
   past its queries fails here, before it is a release;
5. writes the archive (Decision 4) and its row of the manifest
   (Decision 5).

`indents.scm` comes from the directory alone, never from the checkout:
kawoosh reads helix's dialect ([indent.md](indent.md) Decision 1), and
what a grammar's repository carries is nvim's.

### 4. One sqlite archive a grammar, every target in it *(sqlite the user's)*

`NAME.sqlar`, sqlite's own archive table
(`sqlar(name, mode, mtime, sz, data)`, a blob zlib-deflated when that
is smaller):

```
lib/x86_64-windows.dll    lib/aarch64-windows.dll
lib/x86_64-linux.so       lib/aarch64-linux.so
lib/x86_64-macos.dylib    lib/aarch64-macos.dylib
queries/highlights.scm    queries/injections.scm   …
LICENSE
```

and a `meta(key, value)` table: `name`, `repo`, `rev`, `abi`, the zig
that built it. Kawoosh reads its own target's row and the queries.
Nothing new is linked for it; `sqlite3 NAME.sqlar -At` lists one from
a shell.

All six targets in one, since a grammar is then one file with one
hash, its queries and licence are stored once, and three hundred
grammars are three hundred files a release — under GitHub's thousand.
It costs about four and a half times the bytes of one target's, which
is 60 KiB–1.9 MiB a grammar.

Beaten: **`.tar.gz`** — `tar` is a new crate, or the machine's `tar`
is trusted to be there. **An archive per target**: 1,800 files, a
release a target on GitHub, six hashes a grammar. **One bundle of
everything**: the language pack's 21 MB again.

### 5. The manifest is what kawoosh reads

`manifest.json` beside the archives: for each grammar its name, its
`extensions`, `filenames`, `shebangs` and `aliases`, its `repo`, `rev`
and `path`, its `abi`, and the archive's `size` and `blake3`.

Kawoosh is built with a copy of it, so a file's language is known with
no network, and a kawoosh never updated still names archives by hash.
A fetched manifest replaces the copy — on `:grammar update`, and when
an install finds the copy's archive gone — and is kept in the data
directory.

`repo`, `rev` and `path` are there so a grammar can be built from the
manifest alone (Decision 9).

### 6. A listed language is a language of files until its grammar is in

Every grammar of the manifest is in the `Registry` from launch as
kui.md's "language of files alone": detected, nameable by
`language:zig` in a keymap, served by its language server — without
colours. Installed, it is the same entry with `Source::Library`.

Installed grammars live in the data directory, `grammars/NAME/` — the
library, `queries/`, `LICENSE`, and the `rev` it is at — not the
config directory, which is what the user wrote and may keep in git.
The order, first wins: what `kawoosh.language` said (and the config
directory's `parsers/` and `queries/`, as now), an installed grammar,
a linked-in one. A query file under the config directory's
`queries/NAME/` still replaces an installed grammar's.

Kawoosh passes over a manifest's name it links in: the repository does
not know what kawoosh links, and is free to carry `rust` for someone
else. Moving a linked-in grammar out of the binary is not this note's.

### 7. Installing: asked for, or on the first file

`:grammar install NAME` (and `update [NAME]`, `remove NAME`), and
`:grammars`, a pane of the list — linked in, installed and at which
revision, there to install, failed and why.

`grammars.install` says what opening a file of a listed, uninstalled
language does:

- `"ask"`, the default: one corner line a language a session, `zig has
  a grammar: :grammar install zig`;
- `"auto"`: it installs;
- `"never"`: nothing.

An install is one progress line under `grammar`, off the frame:

1. **fetch**: `curl -fL` *(the user's)* to a file in the data
   directory; the bar is the file's size against the manifest's;
2. **verify**: blake3 against the manifest's; a mismatch fetches the
   manifest again and tries once more, then fails with both hashes;
3. **extract**: this target's library and the queries, written beside
   and renamed in;
4. **load**: the library opened and its queries compiled on the
   install's thread, then `add_language` with the grammar ready, as
   `kawoosh.language` adds one — the language's open buffers colour
   without a restart.

A failure is a line under `grammar` and the language stays a language
of files. No `curl` on the `PATH` says so, once.

The installer is the shell's (Rust): it reads the archive, checks the
hash and owns the progress line. The pane is a bundled Lua plugin over
`kawoosh.grammars()`, as the themes' and fonts' are.

Beaten: **an HTTP client in the binary** (`ureq` and a TLS stack): a
megabyte or two for a fetch that `curl` — on Windows since 10, on
macOS, on nearly every Linux — already does. **Installing unasked by
default**: it is native code arriving over the network; one line to
say yes is cheap, and `"auto"` is there.

### 8. Who may say where grammars come from

A grammar is native code in the editor's process. `grammars.url`, and
any source of a grammar, is the user's layer's alone: a project's
`.kawoosh/settings.lua` that names one is refused with a line, as code
from a repository is until trusted ([formatters.md](formatters.md)
Decision 5). `grammars.install` a project may set to `"never"` and
nothing else.

The hash guards the fetch, not the build: what a revision holds is
vouched for by the pull request that moved the pin, as in helix's and
nvim-treesitter's lists.

### 9. Building on the machine is the fallback, and later

For a grammar outside the list, or a machine the releases have no
library for: `:grammar build NAME`, from the manifest's or the user's
`repo`, `rev` and `path`, with what is on the machine — git, and
`$CC`, `cc`, `clang`, `gcc` or `zig cc`. The result goes where an
install's does. Someone with a grammar of their own has a compiler.

TinyCC is not shipped for it. If Windows users without a compiler turn
out to want grammars off the list, it is reopened with a sweep first:
every listed grammar built with it and its trees compared with
clang's.

## Build order

1. **The repository**: the builder, `grammar.toml`, the checks, CI on
   both hosts, a first release of a handful (zig, html, java, ruby,
   kotlin, dockerfile).
2. **`:grammar install NAME`**: the manifest's copy in the build, the
   listed languages in the `Registry`, fetch, verify, extract, load,
   the progress line. Tested against a `file://` base and an archive
   made in the test.
3. **On the first file**: `grammars.install`, the corner line;
   `update` and `remove`; the refused project setting.
4. **`:grammars`**, the pane, and `kawoosh.grammars()`.
5. **The list, wide**: seeded from the language pack's definitions, a
   grammar in only with a sample that parses and queries that compile.
6. **`:grammar build`** (Decision 9).

## Risks

- **No Windows has loaded one of these libraries.** They are built by
  zig's mingw target and loaded by an MSVC kawoosh: the boundary is C
  functions, a scanner frees what it allocated itself, and both sides
  use the UCRT. The first thing round 1 proves.
- **A grammar's own queries are uneven**, and written for the
  `tree-sitter` CLI's capture names. `Token::from_capture` reads nvim's
  spellings; a capture it does not know is plain. The directory's
  `queries/` is where a language is made good, one at a time.
- **Grammars that cannot be built so**: no committed `parser.c`, a C++
  scanner. Refused by the builder, listed as waiting.
- **zig moves.** Pinned in CI; the archive's `meta` says which built
  it.
- **A host is down.** The next base is tried; installed grammars are
  files on disk and need no network.
- **A signed Kawoosh.** The macOS bundle is signed ad hoc today and
  loads any library. Under a Developer ID with the hardened runtime, a
  library signed by no one is refused without the
  `disable-library-validation` entitlement — `kawoosh.language`'s
  libraries too.
- **`latest` is spelled two ways.** GitHub's alias is
  `releases/latest/download/FILE`, Forgejo's
  `releases/download/latest/FILE` (checked against Codeberg's Forgejo
  16, the version drydock9 runs). A base is a whole URL, so each is
  said as it is.

## Deliberately not

- **Moving the linked-in grammars out.** The binary would shrink by
  what they weigh, and a fresh kawoosh with no network would colour
  nothing.
- **Queries fetched apart from their parser.** They move together, in
  one archive, at one revision; that is what nvim-treesitter's
  lockfile was for.
- **Signatures over the manifest.** The hash is checked against a
  manifest fetched over TLS from a host the user named.

## Built

**Round 1, 2026-10-01**: `~/projects/kawoosh-grammars`, public on
drydock9 and push-mirrored to GitHub (the user's doing), CI green on
both, two releases out: `r1` (277ef5e) with dockerfile, html, java,
kotlin, ruby and zig, and `r2` (2cd997c) with scala and odin, asked for
the same day. The builder is `kawoosh-grammars build [NAME…]`; an
archive is 97 KiB to 2.7 MiB (scala's six libraries are over 2 MiB each
before zlib). The bases answer as Decision 2 spells them:

- `https://drydock9.qxuken.dev/qxuken/kawoosh-grammars/releases/download/latest/`
- `https://github.com/qxuken/kawoosh-grammars/releases/latest/download/`

Where it departed from the text:

- **The licence is every `LICENSE…`, `LICENCE…` or `COPYING…` file**
  of the checkout under its own name, not one `LICENSE`.
- **The manifest has a `format`** (1), its `targets`, and per grammar
  its `symbol` and the `queries` its archive holds.
- **A `; inherits: X` comment in a query file is an error** until
  `grammar.toml` names X: the comment alone does nothing, and a query
  that leans on it would ship half.
- **The archive is read back** after it is written: this machine's
  library out of it must be the bytes that were checked.
- **A query must have the capture its kind is read by**:
  `@injection.content` in `injections.scm`, `@name` in `tags.scm` and
  `outline.scm`. odin's own `injections.scm` is `(comment) @comment`,
  nvim's old spelling, which compiles and which `Injections::new`
  refuses; the archive passed the check and failed in kawoosh's
  loader. The check asks what the loader asks, and odin has an
  `injections.scm` in the repository.
- **zig is pinned in `.zig-version`**; the builder refuses another
  without `--any-zig`. In CI it is the `ziglang` pip wheel
  (`ZIG="python3 -m ziglang"`), as kui's cross builds have it.

Verified on macOS arm64: a broken sample, a query naming a node the
grammar lacks and a revision that is not there each fail with the
place; a second build is the same bytes; `sqlite3 -A` lists and
extracts an archive; and all eight, extracted to `parsers/NAME.dylib`
and `queries/NAME/`, load through `Library::find` and `Library::load`
as they stand — of their captures `Token::from_capture` leaves five
unread (`@include`, `@exception`, `@embedded`, `@error`, `@spell`).

Verified by the runs: a Linux runner builds all six targets, macOS's
too, with the `ziglang` wheel and no SDK; drydock9's run publishes its
release with the run's own token; and the two hosts' manifests are the
same bytes, `r1` and `r2` both — the same wheel on the same kind of
runner. This Mac's differ from theirs in the Linux libraries alone, by
the `.comment` section, where Homebrew's zig names its own clang. So
Decision 2's rule stands as the safe one, and today either host's
manifest would do for the other's archives.

Not verified: no Windows has loaded a library.

**Round 2, 2026-10-01**: `:grammar install NAME`.
`systems/src/grammars.rs` is the install, on a thread
(`Io::stream`): the manifest, the fetch, the check, the files written
out, each step an `IoMsg::Grammar`. `kawoosh/src/grammars.rs` is the
shell's half: the listed languages at launch, the command, the
progress line, the load. `kawoosh/grammars/manifest.json` is the copy
in the build, `r2`'s. `kawoosh/tests/grammars.rs` drives it against a
`file://` base and an archive made in the test.

Where it departed from the text:

- **An install asks its base for the manifest every time** (4 KB), and
  checks the archive against that one; it is kept as
  `grammars/manifest.json`. The copy in the build says which languages
  there are and what their files are called, nothing more: a base's
  `latest` moves, and a hash built into an older kawoosh would refuse
  every archive released since. Decision 5's "a kawoosh never updated
  still names archives by hash" is gone with it.
- **An install is a directory named by its archive's hash**,
  `grammars/NAME/HASH12/`, and `grammars/NAME/current` names the one in
  use. Another archive is another directory: no library a process has
  open is written over (Windows would refuse, and a second `dlopen` of
  one path answers the first), and the old directory is pruned at the
  next launch.
- **The row an install came from is kept beside it** (`grammar.json`),
  so a launch needs no manifest to know an installed language's files
  and symbol — one added to the releases after this kawoosh was built
  included.
- **At launch an installed grammar is registered, not loaded**: the ts
  thread loads it at its first buffer, as it does a linked one. A
  fresh install loads on the spot, so one that does not load is a
  warning then — on the install's thread since 2026-10-02 (below).
- **A listed language does not take a file from one with colours**: it
  is the newest in the table, and would win the file to show it plain.
- **Decision 8 came forward**: `grammars.url` is read from the session,
  the user's files and the defaults, never a project's, which is said
  once. With the setting there at all, a later round was too late.
- **`$KAWOOSH_GRAMMARS`** names the directory, as `$KAWOOSH_TYPES`
  does the types'.
- **The fetch's progress is paced** at 125 ms and said only when it
  moved, as a language server's is.

Verified: `nu scripts/verify.nu`, 843 of 843, seven of them new — an
install fetched, checked and written out; a base that is not there
passed over; an archive that is not its manifest's refused with both
sizes and nothing half in; names out of an archive that would leave
its directory dropped; an open file claimed and painted by the
installed grammar's query; the next launch finding it with no base;
a project's `grammars.url` passed over. And over the network
(`KAWOOSH_GRAMMARS_LIVE=1`): zig installed from each of the two
shipped bases alone, about two seconds each, painting keywords and
strings.

And in a window: a build launched on a zig file with an `init.lua`
of `kawoosh.run("grammar install zig")` and a state of its own shows
the file plain with `Installing zig fetching 0%` in the corner, then
painted, the message `grammar: zig installed (6479aa13f32f)`.

Not verified: Windows.

**Round 3, 2026-10-01**: the first file, `update`, `remove`.

- **`grammars.install`** (`ask`, `auto`, `never`): the frame's syntax
  sync, which already passes over a pane whose language has no
  grammar, notes the ones that are listed and not installed; each is
  met once a session. `ask` is an info line under `grammar` — `zig has
  a grammar: :grammar install zig` — in the corner and the log, not a
  toast. A project's value counts only when it is `never`.
- **`:grammar update NAME`** is the install again: the base's manifest
  fetched, and the archive only if it is another than the one in —
  else `NAME is up to date (REV)` and nothing reloaded. **Bare**, the
  list is fetched alone (`grammars::refresh`, `IoMsg::Grammars`), what
  it adds is listed, and each installed grammar whose archive's hash
  is not the list's is installed again, each under its own progress
  line: `updated (OLD → NEW)`.
- **`:grammar remove NAME`** takes `current` out first, so the grammar
  is not installed whatever a Windows with the library loaded lets go
  of; the language is one of files again, its buffers' syntax layer
  cleared, and it is not asked about again that session. The
  directory left behind goes at the next launch's prune.
- **`:grammar`**, bare, says what is installed and how many more there
  are — the line until round 4's pane.
- **An install's manifest is listed as it lands**: a language released
  since this kawoosh was built is there to install, and its files are
  its, after any install or update, without a restart.

Where it departed from the text: nothing of Decision 7's but the
wording of the line. Decision 8's "refused with a line" is "passed
over, and said once" for `grammars.url`, and silent for
`grammars.install`, where a project's word is only ever weaker than
the user's.

- **Found in the window, not by the tests**: listing the languages
  through `add_language` put `language NAME: no grammar` in the corner
  for every one of them at launch — its info line, which the tests'
  app has no logger to hear. A grammar's languages go in through
  `put_language(def, false)`, the line at debug; an install and a
  removal have their own word on the message line. The test that
  holds it hooks the `log` macros as the app does.

Verified: `nu scripts/verify.nu`, 847 of 847, the tests in
`kawoosh/tests/grammars.rs` and `systems/src/grammars.rs` — a listed
language asked about once and not twice, a project's `never` heeded
and its `auto` not (the test fails with the layers read as one),
`auto` installing at the first file, an update that fetches nothing
when nothing moved, one that repaints with the new release's query
when it did, a removal that clears the colours and is not undone by
`auto`, and the next launch without the grammar. And in a window, a
build of its own state opened on a zig file: the corner says `zig has
a grammar: :grammar install zig` and nothing else.

**Round 4, 2026-10-01**: `:grammars`, the pane, and its door.

- **`kawoosh.grammars.list()`**: every grammar there is, each `{ name,
  state, installed, rev, latest, repo, license, extensions, filenames,
  size, step, percent, why }`, `state` one of `built in`, `installed`,
  `available`, `installing`, `failed` — the linked-in ones first, the
  rest by name. The shell publishes it whenever anything of it moves
  (`Kawoosh::publish_grammars`), so the door reads a list already
  made. `latest` is the revision an update would bring, when the
  list's archive is another than the one in; `why` is the last
  install's failure, kept until the name is tried again.
- **`lua/grammars.lua`**, a bundled plugin as the themes' and fonts'
  are: a column of its own with three parts — installed, at their
  revisions; to install, with their files and their archives' sizes;
  built in, as names. `j` `k` and the arrows walk the first two, a
  click moves the cursor; `<CR>`, `i` or a row's button installs the
  cursor's (again, when it is in: nothing is fetched unless its release
  moved); `u` runs `:grammar update`; `d` removes the cursor's; `q`
  and `<Esc>` close. A grammar on its way shows its step and a bar, a
  failed one `failed`, why under it, and `again`. Its keys are
  commands (`grammars take`, `update`, `remove`, `up`, `down`,
  `close`) local to the view, and it changes nothing but through
  `:grammar …`.

Where it departed from the text: the door is `kawoosh.grammars.list()`
— a table with a function, as `kawoosh.fonts` and `kawoosh.themes`
are, the pane's own `take` and `state` beside it — not a bare
`kawoosh.grammars()`. No key opens the pane: `<leader>o` is the look,
and this is not it.

Verified: `nu scripts/verify.nu`; `the_pane_lists_walks_installs_and_
removes` (the rows as drawn and in order, `jjj⏎` installing the fourth,
the row moved to the installed with its revision and no button, `d`
taking it out, a failed install's row saying so, its texts inside their
boxes at 760 px); the overflow sweep with `grammars` in it; and in a
window, with zig in: the three parts, zig's revision, seven rows with
their sizes and buttons.

**Round 5, 2026-10-01**: the list, wide — sixty-nine grammars, release
`r3` (kawoosh-grammars 3583533) on both hosts, and the copy in the
build with it.

- **Seeded from tree-sitter-language-pack's definitions**: each one's
  repository and pinned revision; the files, aliases and interpreters
  are the repository's own. Of ninety candidates surveyed, sixty-five
  had what the builder needs — a committed `src/parser.c`, a scanner
  in C, a `highlights.scm`, a licence file — and sixty-one went in:
  astro, awk, bibtex, bicep, c, clojure, cmake, csharp, csv, d, dart,
  devicetree, dot, dotenv, elisp, elixir, elm, erb, erlang, fish,
  fortran, fsharp, git_config, git_rebase, gitattributes, gleam, glsl,
  haskell, http, ini, json5, julia, kconfig, kdl, llvm, luau, make,
  meson, nginx, ninja, nix, objc, ocaml, pascal, pem, perl, php,
  powershell, prisma, properties, proto, purescript, r, racket,
  requirements, ron, scss, ssh_config, starlark, svelte, wgsl. Each
  has a sample written for it, not copied from its repository.
- **`c` is in the repository** for glsl's and objc's queries, which
  are additions over c's (`inherits`); kawoosh links c and passes the
  name over, as the note has it.
- **Waiting**, each for a reason the builder gives: no highlights in
  the grammar's repository (hcl, terraform, xml, vue, vim, typst, just,
  gitignore, tcl, rst, jq, asm, groovy, fennel, gdscript, matlab,
  crystal, editorconfig), no committed parser (swift, latex), a C++
  scanner (nim), a highlights query that does not compile against its
  own grammar (solidity, vhdl, ocaml's interfaces). A highlights query
  written in the repository's `queries/` is what brings the first lot
  in.
- **The builder grew two things for it**: `kawoosh-grammars check
  [NAME…]`, the checks with this machine's library alone and nothing
  written, sixty-nine in under a minute; and `[queries] skip`, a query
  file of the checkout left out — an `injections.scm` in nvim's old
  spelling, a `tags.scm` with no `@name`.
- **A grammar's limit is said where it is pinned**: wgsl's grammar
  reads no module-scope `const`, scss's no map and no `%placeholder`;
  each parses the rest and is painted around the ERROR. A comment at
  the top of its `grammar.toml`.
- **drydock9's release is a draft until its last file is on**
  (kawoosh-grammars 3c68ad5). At `r3` the workflow published the
  release and then attached seventy files, and for those minutes
  `releases/download/latest/manifest.json` was a release with none: an
  install from that base in that window fails over to the next. GitHub's
  `gh release create` uploads before it publishes. `r4`, the same
  grammars again, was cut to prove it: polled every five seconds while
  it built and published, drydock9's latest manifest was whole each
  time.
- **`@include` and `@exception` are keywords** to
  `Token::from_capture`: nvim's older names, which fourteen of the
  sixty-nine grammars' own queries say, were painted plain.
- **The pane follows its cursor** when the list reshapes under it — a
  grammar installed is among the first, removed back among the rest —
  and a file named as its extension is (`.env`) is said once.
- **The update test raced**: it read the buffer's paint after
  `wait_for_syntax`, which sees nothing pending when the text has not
  moved; under load the old paint was still there. It waits for the
  new paint.

Verified: every one of the sixty-nine built for six targets (1 min 58 s
here) and, extracted, loaded through `Library::find` and `load` with
its symbol, painting its sample; `r3`'s manifests on the two hosts the
same bytes; `nu scripts/verify.nu`, 864 of 864, with the wider list
built in, on main as it had become (below); the live test against both
hosts; the pane in a window with sixty-seven rows to install.

- **main moved under the round**: twenty-four commits, among them the
  rule that a child process is started through `kawoosh_systems::spawn`
  (its `clippy.toml` reaches a worktree from the checkout above it, so
  the sweep stopped at clippy while the branch had no such module). The
  branch is on that main now — merged first, then rebased — and the
  fetch's `curl` and the tests' `cc` and `curl` start through the rule
  from the commits that brought them.

Not verified: Windows.

**After the rounds, 2026-10-01**: queries on top, and indentation.
Asked: "will indent work with this grammars? Can we add our queries on
top of repos?" — it did not, and only by replacing a file whole.

- **`; extends`** (kawoosh-grammars 75e4f93): a file of the
  repository's `queries/` whose opening comments say so goes after the
  checkout's of its name instead of in its place. A node two patterns
  match is the later one's (`ts.rs`), so a few patterns added change a
  few captures. Resolved at build, as `inherits` is: the archive's
  query is whole. *Not done:* the same word in a user's own
  `queries/NAME/` under the config directory, which still replaces.
- **The builder reads an `indents.scm` as `Indents::new` does**: a
  predicate past helix's five or a `scope` that is not `all` or `tail`
  is refused there. Here it fails `Library::load`, so the grammar would
  have installed with no colours at all for a misspelt indent query.
- **Thirty-one grammars bring an indent query**, release `r5`: c,
  cmake, d, dart, elixir, erlang, fish, fortran, glsl, haskell, java,
  julia, kdl, kotlin, llvm, luau, make, meson, nix, objc (c's, which
  its queries are additions over), ocaml, odin, perl, php, proto, ron,
  ruby, scala, starlark, svelte, zig. Helix 25.07.1's own where it has
  one, changed where the grammar pinned names a node otherwise or the
  language's usual style asked (a `rescue`, an `else`, a `catch`, an
  `end`, a label a level out; nix's `in` and body not indented; zig's
  struct members; fish's `case`), each file saying what and carrying
  the MPL's notice; written in the repository for erlang, haskell and
  meson. The other thirty-eight indent by the bracket rule, as before.
- **The bar is this indenter**: a query goes in when `for_lines`,
  given it, leaves every line of the grammar's sample where it is.
  `the_repository_s_samples_reindent_as_they_are`
  (`KAWOOSH_GRAMMARS_REPO=PATH`) installs every grammar from the
  repository's `dist` as a base, loads it as an install is, and
  reindents its sample: thirty-one of thirty-one. The repository cannot
  try it itself, the indenter being kawoosh's.
- **`default` lines up with its `case`s in C and C++**
  (`languages/queries/c/indents.scm`): helix's query outdents `case`
  and not `default`, which sat a level in from its siblings. Found
  writing the repository's c; `a_default_lines_up_with_its_cases`.
- **Seen and left**: `#set! "scope" "all"` on a node of several lines
  counts twice here — its first line a level in, and its later lines a
  level in from that line, the reader being relative to the text
  (indent.md Decision 2). nix's `let` was written around it (the `let`
  indented, its `in` and body outdented). Helix's own c uses `all` for
  a braceless body, which is one line as a rule; a statement there
  that runs over two would be a level too far.

Verified: the live test installs ruby from the shipped bases and a
line opened under a `def` is a level in, where no bracket says
anything; `nu scripts/verify.nu`.

**Round 6, 2026-10-01**: `:grammar build NAME` (Decision 9), on main
as it then was (the branch rebased onto it first, at the user's word).

- **`grammars::build`** (`systems/src/grammars.rs`): one commit of the
  repository fetched with git — `rev` a hash, a tag or a branch, the
  commit it resolves to kept — `src/parser.c` and `src/scanner.c`
  compiled with `$CC`, else the first of `cc`, `clang`, `gcc` and
  `zig cc` that answers, and the checkout's `highlights.scm`,
  `injections.scm`, `tags.scm` and licence beside the library, in
  `grammars/NAME/src-REV12/`. The same commit built before is answered
  as it is. A C++ scanner or no committed parser is refused with the
  reason, as the releases' builder refuses them. `Row` has `path` and
  `built` for it; its archive's fields are empty for a grammar built.
- **The source** is the user's, `grammars.sources.NAME = { repo, rev,
  path, symbol, extensions, filenames, shebangs, aliases }`, else the
  list's row. The user's are read from the layers a project does not
  write (a project's is passed over, said once), again whenever the
  settings move, and each is a language of files until it is built.
- **`:grammar update`** takes a grammar as it came: one built here is
  built again when its source's commit is another (`is up to date`
  otherwise), bare or by name.
- **An install that fails for want of a library says the way**: when
  the base "builds no libraries for" the machine and the list names
  the grammar's source, the error ends `` `:grammar build NAME` builds
  it here``. A failed install's manifest is listed too, as a finished
  one's is.
- **The pane**: a grammar no release has an archive of says `build`
  where the others say `install`, one built here says so beside its
  revision, `b` builds the cursor's, and `<CR>` on one that is in
  updates it as it came. `kawoosh.grammars.list()` has `built` and
  `prebuilt`.

Where it departed from the text: a grammar with no highlights of its
own is built all the same, and the loader says where it looked — the
config directory's `queries/NAME/` is where the user writes one.
`cl` is not tried: its flags are another compiler's.

Verified: `a_grammar_is_built_from_its_source` (a local repository
with json's parser: by branch, the steps, the files, the same commit
not compiled twice, each refusal's words);
`a_grammar_of_the_user_s_own_is_built_from_its_source` (a project's
source passed over, the user's a language of files, built under
`Building NAME`, painted, up to date when asked again, there at the
next launch); the hint in `an_install_that_fails_says_why`; the
pane's `build` in `a_grammar_that_can_only_be_built_says_so_in_the_pane`;
over the network, kdl built from its repository at the list's revision
with this machine's `cc` (`KAWOOSH_GRAMMARS_LIVE=1`); and `nu
scripts/verify.nu`, 869 of 869.

Not verified: a build on Windows, where `-shared` is mingw's or
clang's to honour and `$CC` may be needed; and any library loaded
there at all.

**A directory as a source, 2026-10-01.** Asked: "but how grammars not
from git repo can be injected?", then "ok, let's add dir". Three ways
there were: a library the user built, through `kawoosh.language` and
its `path` (kui.md Decision 13, unchanged); a `repo` that is a path to
a local checkout, which builds what is committed there; and
`grammars.url` naming a `file://` folder of the user's own archives.
None took a source that is in no repository, or a checkout's
uncommitted work.

- **`grammars.sources.NAME.dir`** in the repository's place (`~` the
  home; with both said, the directory): `grammars::build` reads it as
  it lies — no git, no scratch copy — and the install is
  `grammars/NAME/dir-HASH12/`, the hash of `src/parser.c`,
  `src/scanner.c` and the three query files, which is also the
  revision it shows.
- **Built again only when a file of it moved**: the same hash is the
  install there already (`is up to date`), another is another
  directory, loaded afresh — a library a process has open is never
  written over, here as for an install. So a grammar is worked on by
  editing, `tree-sitter generate`, `:grammar build NAME`; a query
  edited alone is enough to be painted by it.
- The answer given before the build said "rebuild every time it is
  asked"; the hash is the better rule, and what was built.

Verified: `a_grammar_is_built_from_its_source` (the directory's part:
no source step, named by its hash, nothing compiled when nothing
moved, another install for a query edited, each refusal's words);
`a_directory_is_built_as_it_lies` (a directory that is no repository,
built, painted, up to date, repainted by an edited query, and one that
is not there); `a_source_is_a_repository_or_a_directory`.

Not done: `kawoosh.language`'s own libraries are not in the `:grammars`
pane; a grammar there is one the list, an install or
`grammars.sources` names.

**The todo's three, 2026-10-01.** Asked: "search in grammars",
"indicate builtins in grammars", "add install button to a notification
and make it stay for something like 15 sec".

- **`ask` is a toast with an Install button**, up fifteen seconds
  (`ASK_TTL`) — an info shown as a toast, the text as before, the
  button `:grammar install NAME` (`Build` and `:grammar build NAME` for
  a listed grammar no release has an archive of). Decision 7's "one
  corner line" is that toast now. Notifications had actions and a time
  of their own already (kui.md Decision 9); what they lacked was the
  two together: a toast with actions was a question, not put away by
  `x` or a click until answered, which an offer that goes by itself in
  fifteen seconds is not. `Shown::waits` says which a toast is —
  actions and no time — and only a question refuses `x` and a click on
  its card. The keyboard's way to the button is the toasts' own,
  `<C-w>n` then `⏎`.
- **The built-in grammars are rows**, in their own part after the rest,
  each with its files and a `built in` tag where the others have their
  button; the cursor walks them, and `⏎` `d` `b` on one say it is built
  in. `kawoosh.grammars.list()` has `released`: a release lists the
  name. The names a manifest lists that this build links (only `c`
  today, there for glsl's and objc's queries) are kept as they are
  passed over (`Grammars::passed`), so under c's row a line says
  "used over the release's c" — under it, where a failure's reason
  goes: beside the tag it squeezed the row's files to nothing at the
  pane's width.
- **`/` filters the pane**: a field over the rows (`ctx.field`, as the
  settings pane's search is), each grammar matched by its name or one
  of its files, word by word, with `kawoosh.fuzzy` — the picker's fzy
  scoring — at its best word, so `zig`, `.rb` and `Dockerfile` each find
  theirs. Matched as one line, a query's letters were gathered from
  across a long list of extensions: `qqqq` found sql. Every part is
  filtered and keeps its place, its rows best first and the matched
  letters lit; the cursor goes to the best match of all whenever the
  filter moves, stays on its grammar when the filter is emptied, and
  acts on nothing while its row is hidden. `⏎` or `<Esc>` in the field
  gives the keys back to the rows (the arrows, `<C-n>` `<C-p>` `<C-j>`
  `<C-k>` walk them from the field); `<Esc>` on the rows empties the
  filter, and with none closes the pane. The head and the field stay
  put over the scrolling rows.

Calls taken: the toast keeps the corner line's text, command and all,
since the log keeps it after the toast has gone; the filter keeps the
parts rather than one ranked list, the cursor on the best match
showing where it is; `i`, which installs, is not the filter's key.

Verified: `an_offer_goes_in_its_time_and_can_be_put_away`
(`kawoosh/tests/notify.rs`); `the_ask_is_a_toast_whose_button_installs`
and `the_pane_filters_by_name_and_file_and_tags_the_built_in`
(`kawoosh/tests/grammars.rs`), and the pane test's rows with the
built-in ones after the rest.

**An install's end froze the window, 2026-10-02.** Reported: "grammar
panel lags at the moment of finishing installation. if i install
something and scroll it freezes for a 1-2s". Measured: the frame an
install ended in opened the new library and compiled its queries.
macOS checks a library at its first open — a fresh 1 MB grammar took
220 ms and a 3.5 MB one 250 ms on a busy machine, a later open of the
same file half a millisecond — and php's queries compile in 35–50 ms;
the rest of the end (the stored manifest read again, the list
published, the pane's rows) is under a millisecond. Now the install's
thread does the load (`grammars::Loaded`: `Installed::library`, then
`Library::load`) and sends the grammar ready with `Step::Done`; the
frame takes it in with `put_loaded`, a failure still the warning it
was. Verified: `an_install_s_grammar_is_loaded_off_the_frame`, the
frame's CPU time against the load's of a heavy query (1–2 ms against
200).
