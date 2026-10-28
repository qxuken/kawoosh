# Code

What kawoosh knows about the code you edit: syntax highlighting,
language servers (hover, go to definition, rename, diagnostics,
completion), compile commands, and the syntax tree.

## Syntax highlighting

Highlighting comes from tree-sitter grammars built into kawoosh:
Rust, TOML, CSS, JavaScript, TypeScript, TSX, Go and `go.mod`, Lua,
Bash, Nushell, C, C++, Python, Scheme, JSON, JSONC, YAML, SQL, diffs,
git commit messages and Markdown, with regular expressions and JSDoc
highlighted inside the languages that hold them. A file's language is
chosen by its extension, its name or its shebang line.

`:syntax NAME` (also `:setf`, `:ft`) reads the buffer as another
language, `text` for none; `:syntax` alone says which one it is. A
language of your own is added from Lua with `kawoosh.language`; see
[lua](lua.md).

### More languages

Kawoosh knows more languages than it carries grammars for — some
seventy: C#, PHP, Java, Kotlin, Scala, Dart, Zig, Haskell, OCaml,
F#, Elixir, Erlang, Clojure, Ruby, Perl, R, Julia, Nix, Teal, HTML, XML
(SVG with it), Svelte, Astro, Vue, templ, SCSS, Makefiles, CMake, Dockerfiles,
protobuf, GLSL and WGSL among them; `:grammars` lists them all. A
file of one is recognised, and its language server starts, but it has
no colours until its grammar is installed:

`:grammar install NAME` fetches the grammar, built for this machine,
and paints the language's open buffers; the corner shows how far it
is. It needs `curl` and nothing else — no compiler. An installed
grammar is kept and is there at the next launch, with no network.

An installed grammar colours, outlines and selects by node as a
built-in one does. Thirty-one of them — Ruby, Elixir, Erlang, Haskell,
OCaml, Nix, fish, Julia, Zig, Java, Kotlin, Scala, Dart, PHP and more —
also bring an indent query, so `<CR>`, `o` and `=` follow the syntax
tree ([editing](editing.md)); the others indent by their brackets.

The first time a file of such a language is on show, a notification
says so, with an Install button, and stays up fifteen seconds — longer
while the pointer is over it. The button installs the grammar; so does
`<C-w>n` (the keyboard on the notification) then `⏎`, and `x` puts it
away. `grammars.install` chooses: `"ask"` (that notification, once per
language in a session), `"auto"` (the grammar is installed without
asking) or `"never"`.

`:grammars` opens them all in a pane: the ones installed, each at
its revision; the ones there are to install, with their files and
size; the ones built into kawoosh, each tagged `built in` — and where
the releases have a grammar of the same name (C), saying the built-in
one is what is used. `/` filters it: what you type narrows the rows to
the grammars whose name or files match (`zig`, `.rb`, `Dockerfile`),
the best match under the cursor; `⏎` or `<Esc>` goes back to the rows,
and `<Esc>` there clears the filter. `j` `k` walk it, `gg` `G` go to
the first and the last, `<C-d>` `<C-u>` ten rows down and up, `⏎` (or `i`, or
a row's button) installs the grammar under the cursor, `u` updates
every one, `d` removes the cursor's, `q` closes. One that is on its way
shows how far it is, and one that failed says why.

| command | |
|---|---|
| `:grammars` | the pane |
| `:grammar` | which grammars are installed, and how many more there are |
| `:grammar install NAME` | fetch and load NAME's grammar |
| `:grammar update` | fetch the list, and install again every grammar whose release moved |
| `:grammar update NAME` | the same for one |
| `:grammar remove NAME` | take NAME's grammar out; its files are still recognised |
| `:grammar build NAME` | build NAME's grammar here from its source |

`:grammar build NAME` is the other way in: git fetches the grammar's
source and the machine's C compiler — `$CC`, else `cc`, `clang`, `gcc`
or `zig cc` — compiles it. It is for a listed grammar whose releases
have no library for your machine (an install that fails for that says
so), and for a grammar of your own, named in your settings with where
it is and which files are its:

```lua
grammars = {
  sources = {
    mylang = {
      repo = "https://example.com/tree-sitter-mylang",
      rev = "main",              -- a commit, a tag or a branch
      extensions = { "my" },     -- and filenames, shebangs, aliases
      -- path = "grammars/mylang", symbol = "tree_sitter_mylang"
    },
  },
}
```

Its colours are the repository's own `queries/highlights.scm`; where
there is none, write one under the config directory's
`queries/mylang/`. `:grammar update mylang` builds it again when its
source has moved.

A source need not be a repository. `dir` names a directory on this
machine instead, read as it lies — no git, and what is not committed
too:

```lua
grammars = { sources = { mylang = {
  dir = "~/src/tree-sitter-mylang",   -- holds src/parser.c and queries/
  extensions = { "my" },
} } }
```

`:grammar build mylang` compiles it when a file of it has changed —
its parser, its scanner or its queries — and loads it afresh, so
working on a grammar is: edit, `tree-sitter generate`, `:grammar build
mylang`. With nothing changed it says the grammar is up to date. In the pane a grammar that can only be built says
`build`, and `b` builds the one under the cursor.

The grammars are built by the `kawoosh-grammars` repository, which
releases them on two hosts. `grammars.urls` lists where to fetch from,
in order; each is a folder holding `manifest.json` and one archive per
grammar, so a mirror of your own, or a folder of your own archives, is
a URL in your settings:

```lua
grammars = { urls = {
  "file:///Users/me/grammars",   -- mine first
  "https://github.com/qxuken/kawoosh-grammars/releases/latest/download",
  "https://drydock9.qxuken.dev/qxuken/kawoosh-grammars/releases/download/latest",
} }
```

The lists are one: a grammar several of them list comes from the first
that does, and one only a later URL lists is there to install all the
same. When that first one does not answer, or fails, the grammar is
fetched from the next that has it, and `:grammar update` lists what an
unanswering URL listed as it was. `kawoosh.grammars.list()` says which
URL each grammar came from, or would, as its `base`.
A URL said twice counts once. The setting was `grammars.url` until
2026-10-02; a settings file that still says it is told so.

Only your own settings are read for it: a project's
`grammars.urls` and `grammars.sources` are passed over, since a grammar
is code that runs inside kawoosh, and of `grammars.install` a project's
settings may say `"never"` and nothing else.

A query file of your own under the config directory,
`queries/NAME/highlights.scm` (or `injections.scm`, `outline.scm`,
`indents.scm`), is used in place of the installed grammar's.

The same tree drives selections by syntax node, `<A-o>` `<A-i>`
`<A-n>` `<A-p>`, the move up to the node around the caret, `<A-u>`
([editing](editing.md)), the outline in the
symbols picker ([search](search.md#symbols-and-the-outline)), and your
own code through `kawoosh.node` ([lua](lua.md#the-syntax-tree)).

### A language inside a string

In JavaScript, TypeScript and TSX a string after a comment naming a
language is highlighted as that language:

```typescript
const module = device.createShaderModule({
  code: /* wgsl */ `
    @fragment fn fs() -> @location(0) vec4f {
      return vec4f(1.0, 0.0, 0.0, ${alpha});
    }
  `,
});
const q = /* sql */ 'select * from users';
```

The comment is a block comment of one word, right before the string,
and the word is any language with a grammar, built in or installed:
`/* wgsl */`, `/* glsl */`, `/* sql */`, `/* html */`, `/* css */`. A
tagged template works the same way, its tag the language:
`` sql`select 1` ``, `` css`color: red` ``. Inside a template the
`${…}` stay JavaScript, and the text around them reads as one piece of
the other language. `gcc` on a line of it comments the line in that
language's way.

## Language servers

A server starts when you open a file of its language, if its program
is on your `PATH`. Its project root is the nearest directory with one
of its marker files.

| setting | program | languages |
|---|---|---|
| `lsp.rust` | `rust-analyzer` | Rust |
| `lsp.typescript` | `typescript-language-server` | TypeScript, TSX, JavaScript |
| `lsp.lua` | `lua-language-server` | Lua |
| `lsp.python` | `pyright-langserver` | Python |
| `lsp.go` | `gopls` | Go, go.mod |
| `lsp.c` | `clangd` | C, C++, Objective-C |
| `lsp.bash` | `bash-language-server` | Bash |
| `lsp.fish` | `fish-lsp` | fish |
| `lsp.nu` | `nu` | Nushell |
| `lsp.html` | `vscode-html-language-server` | HTML |
| `lsp.css` | `vscode-css-language-server` | CSS, SCSS |
| `lsp.json` | `vscode-json-language-server` | JSON, JSONC |
| `lsp.yaml` | `yaml-language-server` | YAML |
| `lsp.toml` | `taplo` | TOML |
| `lsp.markdown` | `marksman` | Markdown |
| `lsp.dockerfile` | `docker-langserver` | Dockerfile |
| `lsp.svelte` | `svelteserver` | Svelte |
| `lsp.astro` | `astro-ls` | Astro |
| `lsp.vue` | `vue-language-server` | Vue, beside `lsp.typescript` |
| `lsp.templ` | `templ lsp` | templ |
| `lsp.php` | `intelephense` | PHP |
| `lsp.ruby` | `ruby-lsp` | Ruby |
| `lsp.java` | `jdtls` | Java |
| `lsp.kotlin` | `kotlin-language-server` | Kotlin |
| `lsp.scala` | `metals` | Scala |
| `lsp.csharp` | `csharp-ls` | C# |
| `lsp.fsharp` | `fsautocomplete` | F# |
| `lsp.dart` | `dart` | Dart |
| `lsp.zig` | `zls` | Zig |
| `lsp.haskell` | `haskell-language-server-wrapper` | Haskell |
| `lsp.ocaml` | `ocamllsp` | OCaml |
| `lsp.elixir` | `elixir-ls` | Elixir |
| `lsp.erlang` | `erlang_ls` | Erlang |
| `lsp.gleam` | `gleam` | Gleam |
| `lsp.elm` | `elm-language-server` | Elm |
| `lsp.purescript` | `purescript-language-server` | PureScript |
| `lsp.clojure` | `clojure-lsp` | Clojure |
| `lsp.racket` | `racket` | Racket |
| `lsp.nix` | `nil` | Nix |
| `lsp.cmake` | `cmake-language-server` | CMake |
| `lsp.fortran` | `fortls` | Fortran |
| `lsp.r` | `R` | R |
| `lsp.prisma` | `prisma-language-server` | Prisma |
| `lsp.proto` | `protols` | protobuf |
| `lsp.dot` | `dot-language-server` | Graphviz |
| `lsp.awk` | `awk-language-server` | AWK |
| `lsp.wgsl` | `wgsl-analyzer` | WGSL |
| `lsp.glsl` | `glsl_analyzer` | GLSL |
| `lsp.odin` | `ols` | Odin |
| `lsp.luau` | `luau-lsp` | Luau |
| `lsp.teal` | `teal-language-server` | Teal |
| `lsp.eslint` | `vscode-eslint-language-server` | TypeScript, TSX, JavaScript (beside, where an eslint config is) |
| `lsp.biome` | `biome` | TypeScript, TSX, JavaScript, JSON, JSONC, CSS (beside, where `biome.json` is) |
| `lsp.ruff` | `ruff` | Python (beside, where `ruff.toml` is) |

Most of them are not installed with an editor. When a file's server
is not found, the corner says so once, and how to get it: `:lsp
install` (or `:lsp install LANGUAGE`) installs it in a terminal pane
of its own, where you see it run and answer what it asks. When that
ends well the server starts for the files that missed it.

A server published to npm, PyPI, crates.io, Go or NuGet is installed
by kawoosh itself, with your own `npm`, `uv` (or `python3`), `cargo`,
`go` or `dotnet`, into a folder of kawoosh's —
`~/.local/share/kawoosh/servers` — not the package manager's global
one. A server installed with `npm i -g` lives inside one Node of
fnm's, and is gone when you switch to another; one in kawoosh's folder
stays, and runs on whichever `node` or `python` is current. kawoosh
starts a server from there before it looks on your `PATH`. Nothing
pins a version: an install takes the latest the manager has, and
`:lsp update` asks again for every server kawoosh installed (`:lsp
update LANGUAGE` for one). The rest — brew, rustup, gem, opam, ghcup —
are installed by the line their project gives (`brew install
marksman`), as you would.

Versions are locked. An install takes the version you ask for —
`:lsp install yaml@1.15.0` — or the registry's latest, and stays at it:
nothing updates a server until you ask. Once a day kawoosh asks the
registries what is newer and says so in the corner; `:lsp servers`
shows `1.14.0 → 1.24.0`, and `:lsp update` (or `:lsp update yaml`)
moves to the latest. `lsp.check_updates = false` stops the asking.

The servers a machine wants go in your settings, so they travel with
them:

```lua
return {
  lsp = {
    ensure_installed = { "rust", "typescript", "yaml@1.15.0", "toml" },
  },
}
```

kawoosh installs what is missing in the background, a line in the
corner saying so, and starts each server once it is in. A name with a
version is kept at that version; one without is installed at the
latest and then left. A server kawoosh does not install itself (brew's,
rustup's) is named in the corner with how to install it. With
[qd](settings.md#dotfiles-with-qd) keeping your settings, a new machine gets its
servers at the first launch.

The same works with no window: `kawoosh lsp install yaml toml@0.9.3`,
`kawoosh lsp update`, `kawoosh lsp outdated`, `kawoosh lsp remove yaml`
and `kawoosh lsp list`, which says of each server whether kawoosh
installed it (at which version), it is on your `PATH`, or it is
missing, and how it installs. `$KAWOOSH_SERVERS` names another folder.

`:lsp servers` lists every server: running, off, installed by kawoosh,
found on the `PATH` or missing, and how a missing one installs.
`lsp.NAME.install` says another way: a line, or a package —
`install = { npm = "my-language-server" }`, `{ pip = {
"cmake-language-server", "pygls<2" } }` (the first is the program, the
others go in beside it), `{ cargo = "taplo-cli", args = { "--locked",
"--features", "lsp" } }`.

What a server says — its messages, an exit, a program not found — is
a line in the corner, never a toast; `:messages` keeps each at its
level, and `:lsp logs` keeps the server's own words.

typescript-language-server needs TypeScript 5 installed beside it
(`typescript@5`), in the project or globally; it does not work with
TypeScript 7. A server that refuses to start, as it does without one,
is off until `:lsp restart`, and the corner says what it said and in
which project. One that exits on its own is started again for its
files, the corner saying how it ended; after three exits in three
minutes it is off until `:lsp restart`. Either way the corner names
the project's `.kawoosh/settings.lua`, where `lsp = { NAME = {
enabled = false } }` keeps it off in a project that is no place for it.

A server runs per project: two Rust projects open are two
rust-analyzers, each with its own log. One that has held no open file
for five minutes is shut down, said in `:messages`, and started again
when a file of its project opens; `lsp.idle_stop_secs` sets how long
(`0` keeps it running), `lsp.NAME.idle_stop_secs` for one server. A
file of a server's library — the Lua API's types, reached by `gd` from
your config — is opened under the server already running, not a new
one of its own.

### Several servers for a language

A language can have more than one server: TypeScript has its language
server and, where the project lints with eslint, eslint beside it. The
diagnostics of all of them are shown together; code actions are all of
theirs (eslint's fixes beside TypeScript's refactorings); everything
else — hover, completion, rename, go to definition — is answered by the
first server that does it.

A server with `when` files runs only where one of them is in the
file's directory or one above it, up to the repository's root: eslint
needs an `eslint.config.*` or an `.eslintrc*`, biome a `biome.json`,
ruff a `ruff.toml`. Say which servers a language has, in which order,
with `lsp.languages`:

```lua
return {
  lsp = {
    languages = { typescript = { "typescript", "eslint" }, python = { "python", "ruff" } },
    -- A server of your own beside a language's: it runs where `.mylint` is.
    mylint = { cmd = "mylint-lsp", languages = { "typescript" }, when = { ".mylint" } },
  },
}
```

A name is a server's `lsp.NAME` or its program (`"typescript-language-server"`).

### Settings per server

`lsp.NAME` in your [settings](settings.md) changes a server, and a
project's `.kawoosh/settings.lua` changes it for that project only:

```lua
return {
  lsp = {
    typescript = { load_all = true },  -- .ts, .tsx and .js
    rust = { inlay_hints = true },
    python = { enabled = false },
  },
}
```

| key | what |
|---|---|
| `enabled` | `false` stops the server and sends it nothing |
| `load_all` | send the server every file of its languages in the project, so diagnostics, references and renames cover files you have not opened |
| `load_max` | the most files `load_all` sends (2000) |
| `inlay_hints` | inlay hints for this server's languages; unset, `lsp.inlay_hints` decides |
| `diagnostics` | `false` hides the server's diagnostics, the server running on; `lsp.diagnostics = false` hides every server's |
| `idle_stop_secs` | seconds the server may hold no open file before it is shut down; unset, `lsp.idle_stop_secs` (300) decides; `0` keeps it running |
| `cmd` `args` | the program and its arguments, saying the words `init` does (`--tsdk={typescript}`); a change restarts the server |
| `roots` | the marker files that find the project root |
| `languages` | the languages this server serves |
| `when` | files one of which must be at or above a file for the server to run for it |
| `settings` | the configuration sent to the server |
| `answers` | requests of the server's own and the result each is answered with: `{ ["eslint/confirmESLintExecution"] = 4 }` |
| `init` | what the server is started with (its `initializationOptions`); in a string, `{root}` is the server's root, `{typescript}` a TypeScript's `lib` folder — the project's, else the one installed with typescript-language-server — and `{package:NAME}` the folder server NAME's package is in: astro's is `{ typescript = { tsdk = "{typescript}" } }`. An item of a list naming what is not there is left out. A change starts the server again |
| `with` | the servers a file of its languages has beside it, after it, unless `lsp.languages` says: vue's is `{ "typescript" }` |
| `relay` | notifications of the server's own carried to another server as a command, the answer sent back: vue's `tsserver/request` to typescript's `typescript.tsserverRequest`, answered as `tsserver/response` |
| `install` | how `:lsp install` installs the program: a package (`{ npm = "name" }`; `pip`, `cargo`, `go`, `dotnet`) into kawoosh's folder, a shell line, or a line for each platform (`{ mac = "brew install x", windows = "winget install x" }`; `linux` too) |

The servers kawoosh knows are rows of these same keys (`servers.lua` in
kawoosh's source), and `kawoosh.lsp.server(NAME, t)` in
[Lua](lua.md) takes the same table, the rules aside: `lsp.NAME` over
either changes what you say and keeps the rest.

`lsp.enabled = false` turns every server off, whatever `lsp.NAME.enabled`
says: only the grammar's highlighting is left. `:set -lsp.enabled` (or
`:lsp disable`) does it for the session, `:set +lsp.enabled` turns them
back on. To keep one file away from its server while the rest of the
project is served, `:lsp detach` in it.

To keep the servers but not their diagnostics — completion, hover and
`gd` still answer — `lsp.diagnostics = false`, or `:lsp diagnostics`
(`<leader>od`) for the session; `:lsp toggle diagnostics` hides one
server's — eslint's, say, and not typescript's. Shown again, what they
said comes back at once.

A plugin can add rules of its own with `kawoosh.lsp.rule`
([lua](lua.md#language-servers)). They are set the same way, as
`lsp.NAME.RULE` for one server or `lsp.RULE` for all of them, and `:lsp
toggle` and `:lsp info` know them.

### Commands

| command | what |
|---|---|
| `:lsp` | the servers running, on the status line |
| `:lsp info` | the servers in a pane: each one's root, documents and rules |
| `:lsp servers` | every server there is: running, off, found or missing, and how to install a missing one |
| `:lsp install [LANGUAGE]` | install the server in a terminal pane — its package into kawoosh's folder, else its line; it starts once that ends well |
| `:lsp update [LANGUAGE]` | every server kawoosh installed, or one, at its manager's latest; restarted once that ends well |
| `:lsp restart [LANGUAGE]` | restart the language's server, or bare every one running or wanted by an open file; a missing program, or one that refused to start or kept exiting, is tried again |
| `:lsp logs [LANGUAGE]` | what a server said, its errors included, live: bare the one serving the caret's file, `LANGUAGE` its server in every project, `:lsp logs all` every server's; `:lsp logs clear` forgets it |
| `:lsp toggle RULE [LANGUAGE]` | flip `enabled`, `load_all`, `inlay_hints`, `diagnostics` or a plugin's rule for the session |
| `:lsp diagnostics` | every server's diagnostics hidden or shown for the session, the servers running on (`lsp.diagnostics`) |
| `:lsp disable [LANGUAGE]` | every server off for the session (`lsp.enabled`), or the language's (`lsp.NAME.enabled`): stopped and sent nothing; highlighting stays |
| `:lsp enable [LANGUAGE]` | every server on again, or the language's |
| `:lsp detach` | this file taken from its servers — its diagnostics and hints gone, nothing sent or asked — until `:lsp attach`; the language's other files stay served |
| `:lsp attach` | this file sent to its servers again |

### Keys

| keys | what |
|---|---|
| `K` | hover: what the server says about the symbol, in a pane |
| `gd` | go to the definition (`:lsp definition`) |
| `gD` | go to the declaration; several are a list |
| `grr` | references, as a list ([search](search.md#lists)) |
| `grn` | rename: the prompt opens with `lsp rename NAME`; edit the name, `<CR>` |
| `gra` | code actions at the caret or over the selection, in a picker |
| `gri` | go to the implementation; several are a list |
| `grt` | go to the type definition |
| `grf` | format the buffer with its formatter ([formatting](#formatting)); in visual mode, the selection |
| `grs` `grS` | the buffer's symbols, the workspace's, in a picker ([search](search.md#symbols-and-the-outline)) |
| `<leader>oh` | inlay hints on or off for the session (`:lsp hints`) |
| `<leader>od` | the servers' diagnostics hidden or shown for the session (`:lsp diagnostics`) |
| `<leader>ob` | breadcrumbs on or off for the pane (`:breadcrumbs`, [search](search.md#breadcrumbs)) |

The server's own keys sit under `gr`, as in neovim 0.11: press `gr` and
the which-key lists them.

In the hover pane, `gd` goes to a symbol the text names, `K` shows
that symbol's hover, and `q` closes it.

The code actions picker searches by title, and its preview shows what
taking one would do, as a diff. A rename or an action that changes a
file no pane shows leaves it as an unsaved buffer, and the message
names those files.

### Diagnostics

A server's errors and warnings are underlined, with the first line of
the message at the end of the row. A message wider than its pane is cut
with `…` on a wrapped line and scrolls sideways with the text on one
that is not; it never reaches the pane beside.

| keys | what |
|---|---|
| `]d` `[d` | the next, previous diagnostic |
| `<C-e>` | every diagnostic under the caret, in full, with its source and code (`ts(2322)`) |
| `<leader>d` `<leader>D` | the workspace's, the file's diagnostics as a list ([search](search.md#lists)) |

A server may report on files you have not opened. rust-analyzer does
for its whole crate; for TypeScript, turn on `load_all`.

A plugin can report diagnostics too, under a name of its own
(`kawoosh.diagnostics.set`, [lua](lua.md#diagnostics)). They show in
all the same places, beside the servers'.

What another program changes in the project — a checkout, a
generator, a formatter run in a terminal — reaches the servers: a file
`load_all` sent is read again (gone, it is closed; a new one is sent),
and a server that asked to hear of files (rust-analyzer's `Cargo.toml`)
is told. A file open in a buffer is the buffer's: the server has the
buffer's text, reloaded or not. Changes under `.git`, `target`,
`node_modules` and what `.gitignore` names are not passed on. A server
on another machine ([remote](remote.md)) is not told; rust-analyzer and
TypeScript's watch the files there themselves.

TypeScript shortens a long type in a message to `... 4 more ...`.
Kawoosh shows what the server sends, and the server has no
setting for this: it is the compiler option `noErrorTruncation`, so put
`"noErrorTruncation": true` in the project's `tsconfig.json`
`compilerOptions` for the whole type, in the row and in `<C-e>`.

### Completion

Completion offers what the word you are typing could be. Where
nothing follows the caret on its line, the best candidate's rest is
drawn faintly after the caret, a "ghost"; wherever no ghost is drawn —
mid-line, a candidate that matches the word without beginning with it,
the ghost turned off — a list of candidates opens under the caret (over
it near the pane's foot): each row its kind's icon, its name with the
letters you typed lit, and its detail faint at the right. In code it
asks as a word starts, after the language server's trigger characters
(`.`), and after a path's `/`; in Markdown, commit messages and plain
text it asks only when you press `<C-Space>`.

| keys (insert mode) | what |
|---|---|
| `<Tab>` `<C-y>` | take the selected candidate |
| `<C-n>` / `<C-p>` | the next, previous candidate |
| `<C-e>` | close it; typing on in the word asks nothing more |
| `<C-f>` / `<C-b>` | scroll the candidate's documentation |
| `<C-Space>` | ask for completions now |
| `<C-x>` | the candidates in a picker, with their kind, signature and documentation |
| `<C-g>` | the signature of the call the caret is in ([below](#signatures)); again, put away |

Beside the list `<C-n>` walks every row; from a ghost it walks onto a
candidate the ghost cannot show and the list opens; with the list
turned off (`completion.popup = "never"`) it walks what a ghost can
show. A moment after a candidate is selected (`completion.docs`, 200
ms) its documentation shows in one panel beside the list — above the
caret's line when there is no list — asked of the language server for
that candidate alone; `completion docs` shows it at once, and again
puts it away.

The keys but `<C-Space>` and `<C-x>` hold only while a completion is on
show (and `<C-f>` `<C-b>` while its documentation is): otherwise `<Tab>`
is a tab. `<CR>` is always a newline and the arrows always move. They
are ordinary keymap commands — `completion accept`, `completion next`,
`completion prev`, `completion close`, `completion docs down`,
`completion docs up` — bound in places of their own, `completion` and
`completion_docs`, so you rebind them as any key:

```lua
kawoosh.map("i", "<C-l>", "completion accept", { scope = "completion" })
kawoosh.unmap("i", "<Tab>", { scope = "completion" })
```

Candidates come from every source in `completion.sources`, merged:
the language server (`lsp`), the files after `./`, `../`, `~/` or `/`
relative to the buffer's file (`path`; on a remote machine, there),
the functions, types and constants the syntax finds in open buffers
(`buffer_symbols`), the words of every open buffer (`buffer_words`),
and, if you add it, the server's workspace symbols from three typed
characters (`workspace_symbols`). A private buffer's words are never
offered ([secrets](memory.md#secrets)). Candidates match fuzzily, as in a picker; then
the source's place in the list decides, then how near (this buffer and
fewer lines away, then the files you used lately, then other buffers),
then what you accepted before in this project, then what you yanked or
searched for lately.

```lua
return {
  completion = {
    trigger   = "typing",    -- typing | endline_only | manual | off
    ghost     = "endline",   -- always | endline | never
    popup     = "fallback",  -- always | fallback (where no ghost is) | never
    sources   = { "lsp", "path", "snippets", "buffer_symbols", "buffer_words" },
    fuzzy     = true,        -- false: candidates that begin with the word
    docs      = 200,         -- ms before the documentation; false: on `completion docs`
    max_items = 10,          -- the list's rows
    signature = "manual",    -- manual (<C-g>) | auto (also on `(` and `,`) | off
    callable  = "name",      -- name | args: a function's parameters as fields
  },
  language = {
    markdown = { completion = { trigger = "typing" } },
  },
}
```

`:setlocal completion.trigger=off` turns it off in one pane. The
`snippets` source offers your own snippets ([below](#your-own-snippets)). The kinds'
icons — in the list, the `<C-x>` picker and the symbol pickers — are
Nerd Fonts' codicons, drawn in `font.icons` ([look](look.md#icons-and-keys)).

#### Signatures

Inside a call, `<C-g>` asks the language server for its signature and
shows it in the one panel — beside the list while a completion is on
show, else above the caret's line (under it at the pane's top): the
parameter the caret is in lit, its documentation's first paragraph and
the function's under it. It follows the caret: type `,` and the next
parameter is lit; go into a call inside it and that call's signature
shows, and leaving it brings the outer one back. It closes when the
caret leaves the parentheses, on `<Esc>`, on `<C-e>`, or on `<C-g>`
again. With `completion.signature = "auto"` it also opens as `(` or
`,` is typed (whatever characters the server names); `"off"` never
shows it.

| keys (insert mode, while a signature is on show and no completion is) | what |
|---|---|
| `<C-n>` / `<C-p>` | the next, previous overload (`1/3` before it says which) |
| `<C-e>` | put it away |
| `<C-f>` / `<C-b>` | scroll its documentation |

While a completion is on show these keys are the completion's: `<C-e>`
closes the completion first, then the signature. The panel is one, never
two: walking the list with `<C-n>` shows the selected candidate's
documentation in it, and typing on brings the signature back. Its text
is drawn through the masks, as everything is ([secrets](memory.md#secrets)).
The commands are `signature`, `signature next`, `signature prev` and
`signature close`, the last three in the place `signature`.

Taking a function from the list puts its name alone
(`completion.callable = "name"`), whatever brackets or fields the server
sent. With `"args"` it puts its brackets and its parameters as snippet
fields: the server's own snippet where it sends one, else the name and
`()` with the caret inside, the parameters filled in as fields once the
server says the signature — `a` for `a: i32`, the parameter as written
where its label puts no `:` — unless you typed first. A function of no
parameters leaves the caret after the brackets; a `(` already after the
caret takes no more.

#### Snippets

A language server's snippet — a `for` loop, a function with its
parameters — is a candidate like any other, matched by its name. Taken,
it puts its text with its fields in it, and the first field is
selected: type, and what you type replaces it. A field that appears
twice is a caret in each place, so both change as you type. Only a
field's selection is typed over: any other selection in insert mode
(one an undo put back) is typed at its caret. A choice
field (`pub`, `pub(crate)`, `priv`) opens the list of its options at
once: `<C-n>` `<C-p>` walk them, `<Tab>` takes one. The fields you have
not reached yet are faintly underlined.

| keys (insert mode, while a snippet is live) | what |
|---|---|
| `<C-l>` | the next field; from the last, the caret where the snippet ends and the snippet done |
| `<C-k>` | the field before |

`<Tab>` is never a snippet's: it takes a candidate, or is a tab. There
is one snippet at a time, and it is gone when it ends: at its last
field, when you leave insert mode, when the caret leaves its text by
anything but `<C-l>` `<C-k>` (typing on after its last character
included, unless the field you are filling ends there), when something
is edited outside it, when
its pane shows another buffer or another pane takes the keys, on an
undo or a redo, and when another snippet is taken. With none live,
`<C-l>` and `<C-k>` do nothing; nothing brings you back into a snippet
you left. They are the commands `snippet next` and `snippet prev`,
bound in the place `snippet`:

```lua
kawoosh.map("i", "<C-j>", "snippet next", { scope = "snippet" })
```

A snippet's lines are indented as the line it goes into, and its tabs
are your indentation. Its variables are VS Code's — `$TM_FILENAME`,
`$TM_SELECTED_TEXT`, `$CLIPBOARD`, `$CURRENT_YEAR`, `$UUID` and the rest;
one that would put text a mask hides (the clipboard holding a secret,
a line with a key on it) puts nothing ([secrets](memory.md#secrets)). A
snippet that is not written right is put as the text it is.

#### Your own snippets

Your snippets are VS Code's snippet format, written in Lua in `init.lua`:

```lua
kawoosh.snippets.add("rust", {
  ["for loop"] = {
    prefix = "for",                                  -- or a list: { "for", "fl" }
    body = { "for ${1:x} in ${2:iter} {", "\t$0", "}" }, -- a string, or its lines
    description = "a for loop",
  },
})
```

Each is offered by its prefix as you type, matched as any candidate is,
with its description beside it and what it puts in the documentation
panel, and taken it is a snippet as above: `$1` and `${1:default}` its
fields, `${1|a,b|}` a choice, `$0` where it ends, the variables
(`$TM_FILENAME`, `$CURRENT_YEAR`, …). `"all"` as the language offers
them everywhere; a name added again replaces the one before;
`kawoosh.snippets.add` answers how many it took and, when some were
not taken (no prefix, no body, a body over 1 MiB), why each.

Snippet files work as they are, VS Code's or a package such as
friendly-snippets. Name them in your settings:

```lua
return {
  snippets = { paths = { "~/snippets", "~/src/friendly-snippets" } },
}
```

A path is a folder of VS Code snippet files (`rust.json` holds Rust's,
a `*.code-snippets` file every language's, or those its snippets'
`scope` names), a package (its `package.json`'s `contributes.snippets`
names each language's file, inside the package's folder), or one file; `~` or absolute, else under
your config folder. Nothing is read until a buffer of a language asks
for a completion, and then only that language's files and the files
for every language; they are watched, so saving one reads it again.
`kawoosh.snippets.load_json(path[, lang][, fn])` reads any of these at
once, every language, in the background, then calls `fn(count, why)`.
Comments and trailing commas in the files are fine; what goes wrong is
said in the log (`:messages`). No snippets ship with kawoosh.

## Formatting

`:format` (`grf`) formats the buffer with its formatter: prettier, prettierd, biome, stylua, clang-format, ruff, gofmt, taplo, shfmt, rustfmt, the language server, or `indent` — the language's syntax putting each line at its indent and touching nothing else ([editing](editing.md#indentation)). The formatted text goes in as the lines that changed, as one undo step, so the caret stays where it was on lines the formatter did not touch.

Which one formats a buffer is its `formatter` setting. The default, `auto`, is the formatter whose config file is nearest the file (a `.prettierrc`, a `"prettier"` key in `package.json`, `biome.json`, `stylua.toml`, `.clang-format`, `ruff.toml` or `[tool.ruff]` in `pyproject.toml`, `taplo.toml`), then one that always runs for the language (gofmt for Go), then the language server, then `indent` where the language has indent rules — so JSON, YAML or TOML with no formatter configured and no server still get `:format` and a format on save. Rust formats through rust-analyzer, which knows the crate's edition.

```lua
return {
  language = {
    typescript = { formatter = "biome", format_on_save = true },
    go = { format_on_save = true },
    rust = { formatter = { "rustfmt", "lsp" } },   -- tried in order
    yaml = { formatter = "indent" },               -- the syntax's indent alone
  },
}
```

A list goes on past what cannot format: a formatter not installed, a server that is off or did not start, or one that does not format. When none can, `:format?` names each and why. `auto` in a list is the exception: a project whose config names a formatter that is not installed is not formatted in another's style, and `:format` says it is missing.

| command | what |
|---|---|
| `:format` | format the buffer |
| `:format NAME` | format it with that formatter |
| `:format selection` | format the selection (`grf` in visual mode), with a formatter that can: prettier, stylua, clang-format, `indent` |
| `:format?` | which formatter, why, and the indent it uses |
| `:format allow` | let the project's own formatter run on save (below) |
| `:format revoke` | take that back |
| `:format install [NAME]` | install the buffer's formatter, or NAME, in a terminal pane (below) |
| `:format update [NAME]` | move every formatter kawoosh installed, or NAME, to the latest |

A project's own copy (`node_modules/.bin/prettier`) is used first, then one kawoosh installed, then one on your `PATH`.

**Installing a formatter.** Formatters install the way [servers](#language-servers) do: into kawoosh's folder (`~/.local/share/kawoosh/servers`), with your own `npm`, `uv`, `cargo` or `go`, locked at a version until you update them. When a project's config names a formatter that is not found, `:format?` says `:format install prettier`. They are installed as:

| formatter | from |
|---|---|
| prettier | npm `prettier` |
| prettierd | npm `@fsouza/prettierd` |
| biome | npm `@biomejs/biome`, the same install as `lsp.biome` |
| stylua | `cargo install stylua` |
| clang-format | PyPI `clang-format` |
| ruff | PyPI `ruff`, the same install as `lsp.ruff` |
| taplo | `cargo install taplo-cli`, the same install as `lsp.toml` |
| shfmt | `go install mvdan.cc/sh/v3/cmd/shfmt` |
| rustfmt | `rustup component add rustfmt` |
| gofmt | comes with Go |

A program that is both a server and a formatter is installed once. Installing it as the other kind only records it as that kind too. Removing it as one kind keeps it for the other.

The formatters a machine wants go in your settings beside the servers:

```lua
return {
  format = { ensure_installed = { "prettier", "stylua" } },
}
```

kawoosh installs what is missing in the background, and says once a day what `:format update` would bring (`format.check_updates = false` stops the asking). With no window: `kawoosh format install prettier`, `kawoosh format update`, `kawoosh format outdated`, `kawoosh format remove prettier` and `kawoosh format list`.

**prettierd** is prettier kept running: node starts once, and each format after that is a message to it, not a new process. It reads the same configs and uses the project's own prettier when there is one. `auto` picks prettier where both are installed, and prettierd where prettier is not. To use it everywhere, set `formatter = "prettierd"` (or `{ "prettierd", "prettier" }`) for the languages you want.

**On save.** With `format_on_save`, `:w` formats first and writes what the formatter made. If the formatter fails, or takes too long, or you type while it works, the file is written as it is and the message says why. `:w!` writes without formatting. `:wq` and `:wqa` quit once everything is written.

**The project's own formatter.** A formatter in the project's `node_modules`, or one whose config is JavaScript (`prettier.config.js`), is code from the repository. `:format` runs it when you ask. It does not run on save, or to read the indent, until you allow it: the first save asks, and `:format allow` does the same. `:trust?` counts the formatters allowed, and `:trust revoke` forgets them.

**Indentation.** A formatter decides the indentation of the files it formats, so kawoosh asks it: it formats a few lines with one nested block and reads the indent from the answer. That indent applies to the buffer over its `.editorconfig`, so `<Tab>` and `>>` indent as the formatter will. `:set tabstop?` names it (`prettier: …/.prettierrc`); saving the config reads it again.

### Your own formatters

A formatter is data under `format.NAME`. The shipped ones can be changed a key at a time, and new ones added:

```lua
return {
  format = {
    prettier = { args = { "--stdin-filepath", "{path}", "--no-semi" } },
    shfmt = { when = "always" },                 -- format every shell script
    black = {
      cmd = "black", args = { "-q", "--stdin-filename", "{path}", "-" },
      languages = { "python" }, when = { "pyproject.toml:tool.black" },
      probe = { python = "if a:\n  b\n" },
    },
  },
}
```

| key | what |
|---|---|
| `cmd`, `args` | the program, reading the text on stdin and writing it formatted on stdout; `{path}` is the file's path |
| `languages` | the languages it formats |
| `when` | the files that say a project uses it, looked for from the file's folder up (`FILE:KEY` for a key in a JSON file or a TOML table), or `"always"`, or `"never"` (only when named) |
| `node` | look in the project's `node_modules/.bin` first |
| `range` | the arguments that format a range: `{start}` `{end}` `{length}` in bytes, `{start_utf16}` `{end_utf16}` in UTF-16 units |
| `probe` | a snippet per language whose formatting shows the indent |
| `timeout_ms` | how long it may take (5000) |
| `enabled` | `false` to turn it off |
| `install` | how `:format install` installs it, as a server's `install`: a package (`{ npm = "name" }`; `pip`, `cargo`, `go`, `dotnet`), a shell line, or a line for each platform |

A formatter written in Lua is in [lua](lua.md#formatters).

## Compile commands

`:compile CMD` (also `:c` and `:make`) runs a command into a
`*compile*` buffer, and the keys go there. The caret follows the
output while it is at the end; move it up to read a line and it stays
there. The paths with line numbers in the output are locations: `<CR>`
on one opens it in an editor pane beside, and `]q` `[q` (`:cnext`,
`:cprev`) walk those of the command started last from anywhere.
`<C-c>` in the buffer stops its command and what it started (`:compile
kill`), `r` runs it again where it ran (`:compile again`), and `q`
closes the pane, the keys going back to the pane you were in — the
buffer stays; `:bd` closes it, stopping the command if it still runs.
A `%` in a line run from the buffer is the file its run was started
from. A location whose file is on show already opens in that pane.

How a run ended is said in the corner, or as a toast when it failed,
unless the keys are in its buffer as it ends: its last line says the
same there. The log (`:messages`) has it either way. A session keeps
no compile pane: the output is gone with its process at a restart,
and `<leader>cc` runs the last line again.

The buffer opens on a line saying where the command runs and when it
was started, then the command; its last line says how it ended and how
long it took: `[finished in 8.2s]`, `[exited with 1 in 0.34s]`,
`[killed after 2m 03s]`. The output keeps its programs' colours: they
are asked for them (`FORCE_COLOR`, `CLICOLOR_FORCE`,
`CARGO_TERM_COLOR`, each only where your environment does not set it),
since a program that is not on a terminal prints plain otherwise, and
the sixteen ANSI colours are the theme's. A line printed plain is
coloured the way clang and gcc colour theirs on a terminal (neither does
on a pipe, whatever the environment says): the location starting the
line bold, `error:` and `warning:` in their colours with the message
bold, `note:` and `help:` cyan, the `^~~~` under a quoted line green;
and go's test verdicts (`--- FAIL`, `ok`) red or green. `compile.color = false` asks
for none, as does a `NO_COLOR` in the environment.

Each command has a buffer of its own, in each directory it is run in,
named for both: `*compile: cargo build*`, and `*compile: yarn build in
apps/web*` when it runs somewhere other than the working directory.
The same command in the same place runs into its buffer again; another
leaves the first's output, and the first itself if it still runs,
alone. A command that has ended gives its pane to the next one; one
still running keeps it, and the next opens beside it. Run from a pane
that holds nothing — an empty `*scratch*` (`:enew`, the launcher's
`s`) or the launcher itself — the command takes that pane instead,
and the empty scratch goes. A map or a
`when` knows every one of them as `*compile*`
(`buffer = "*compile*"`).

`<leader>cc` is a bare `:compile`. It runs, in order of preference:

1. `compile.default`, if your settings set it;
2. else the last command compiled in this workspace, remembered across
   restarts;
3. else the first command your project's files offer.

`:compile?` says which it would be.

`%` in a command is the current file — in a plugin's pane on a file,
`:hex`'s or `:image`'s, the file it shows — written relative to where
the command runs. As the command itself it is `./NAME` when it sits
right there, so `:compile %` on an executable runs it.

A command runs where its kind does — `cargo` at the workspace, a
`package.json` script at its package, a named command in its `cwd`.
`:compile here CMD` runs it where the pane in front is instead: the
file's directory, the listing's, the terminal's, or in a `*compile*`
buffer where its run ran. `<leader>c.` opens the prompt on `:compile
here ` for you to say which command; paths complete from that
directory, `%` is written from it, and the run's buffer says it:
`*compile: CMD in DIR*`. A command named `here` is still `:compile
commands:here`.

### Named commands

```lua
compile = {
  default = "pw",                    -- a name, or a whole command
  commands = {
    pw = "yarn pw %",
    e2e = { cmd = "yarn pw", args = true, doc = "a path to finish" },
    web = { cmd = "yarn build", cwd = "apps/web" },
  },
}
```

`:compile NAME ARGS` runs a named command with ARGS after it. One with
`args = true`, called with nothing after it, opens the prompt for you
to finish instead of running. `cwd` sets where it runs: a path from
the project, or `"root"` for the project itself. `commands:NAME` says
a name and only a name — no such command is an error, where a bare
word that names nothing runs as a command.

`<Tab>` completes a `:compile` line. Its first word: the names, then
the lines the picker lists (`compile.default`, the lines run here, the
project's commands), in the picker's order, then `commands:NAME` once
you type a letter; a word with a `/`, or one starting `.`, `~` or `%`,
completes as a path. Once you type a picker's line far enough, the line
completes whole (`cargo b` to `cargo build --release`). After the first
word: a nushell command's parameter first — `target: string@targets`
offers what `targets` answers (read from the file when its body is a
list, else asked of `nu` once while the file is unchanged, in the
background — the line completes as it answers; `nu` runs the file to
ask, so only once you trust it with `:trust allow build.nu`, and again
after its text changes),
`string@[debug release]` those two — then the program's own arguments
from [carapace](https://carapace.sh) when it is on your PATH (`git
co` to `checkout`, `commit`; a flag's description beside the
candidates), then paths from where the line runs. `%` completes to
`%`, `%:h` and `%:t`, each with the path it names, and `%:h/` lists the
file's directory. A named command's arguments are not asked of carapace.
In a project you have not trusted, carapace runs where it can read
nothing of it: you get every program's flags and subcommands, and what
reads the project — `git checkout`'s branches, a Makefile's targets —
once you trust the directory with `:trust allow DIR` (or its
`.kawoosh/init.lua` is trusted).
`cmdline.complete_with` names another completer that speaks
carapace's `export`, or `false` turns it off; a project's settings can
only turn it off. `:!`, `:shell` and `:terminal CMD` complete their
program's arguments the same way, from the working directory.

### Chains of steps

A list runs its steps in turn, each only once the one before finished
well — `make && ./a.out` without a shell having to know `&&`:

```lua
compile = {
  default = { "build", "run" },      -- or "commands:build", "commands:run"
  commands = {
    build = { cmd = "cc -std=c23 main.c", cwd = "root" },
    run = { cmd = "./a.out", cwd = "root" },
    check = { "build", "./a.out --selftest" },   -- a named chain
    all = { cmd = { "check", "make docs" }, cwd = "root", doc = "everything" },
  },
}
```

Each step is a `:compile` line: a name — a chain's name puts its steps
there — or a command, `%` the file. A named chain's `cwd` is where its
steps run that do not say their own. A chain takes no arguments, and a
step naming a command that wants them is refused, as is a chain that
names itself, however far round.

The steps run into one buffer, each under its own `$ CMD` and ending
on its own `[finished in …]` line. A step that fails stops the chain:
the steps after it are listed as `not run`. `<C-c>` stops the step
running, and so the chain; `r` runs the chain again from its first
step. The picker lists a chain as one row, its steps joined with `&&`,
each on a line of the preview; the memory keeps it as one line, so a
bare `:compile` with no `compile.default` runs the whole chain again.
A chain has no `compile` tool (`:tool compile`), there being no one
shell line to run.

### Commands your project offers

kawoosh reads the project's files, from the current file up to the
repository root, for what they can run: `Cargo.toml` (cargo check,
build, test, clippy, run, then `build --release` and `run --release`),
`package.json` scripts (with the package
manager its lockfile says), justfile recipes, nushell files
(`build.nu` and `toolkit.nu`, or the names and paths `compile.nushell`
lists), Makefile targets, `CMakeLists.txt`, `go.mod`, `pyproject.toml`
and `build.zig`. A nushell file with a `main` is run as a script (`nu
build.nu`, `nu build.nu SUB` for each `def "main SUB"`); one without,
or whose exported `main` sits beside other exports, as a module (`nu -c
'use toolkit.nu; toolkit fmt'`). The ones matching the current file's language server come
first, so a Rust file offers `cargo check` and a TypeScript file its
package's scripts. `compile.deduce = false` turns this off.

`<leader>cC` (`:compile pick`) lists everything: `compile.default`,
your named commands, the last lines run here, and every command the
project's files offer, each with where it came from. In a monorepo it
also lists the scripts of every other `package.json` in the repository
(the open files' packages first), each run in its own package; type
part of the path to narrow them (`api build`). A language server that
lists what can run — rust-analyzer does — adds its rows as it answers,
after the project's named commands, before its files': the test under
the caret (`cargo test --package app --lib -- tests::adds --exact`), its
module's, and the package's check and test. `<CR>` runs it;
`<C-e>` puts it in the prompt so you can add arguments first. A
command that needs arguments always opens the prompt.

The list is in this order, each command once, where it first appears:

1. commands you ran in this workspace that you typed, or named in your
   own settings — newest first;
2. the project's commands you ran in this workspace (what its files
   offer, what its `.kawoosh` settings name) — newest first;
3. your own named commands and `compile.default`;
4. the rest of the project's: its named commands, the language
   server's, its files';
5. commands you ran in another workspace, marked `recent elsewhere`.

A workspace is the outermost folder with a `.kawoosh` above the current
file, else its repository. Typing ranks by the match, as every picker
does; between equal matches, this order.

## Manual pages

`:man ls` reads the system's manual page into a read-only buffer,
`*man ls(1)*`, in the pane you are in — rendered to that pane's width,
the way `man` would fill a terminal that wide, with its bold and
underline kept: section heads in the keyword colour, options and names
bold, arguments underlined, a reference to another page (`chmod(1)`) in
the link colour. `:man 3 printf` and `:man printf(3)` name a section.
`:man` alone takes the word under the caret; on nothing it opens the
picker over every page your system has (`:man pick`, with their
one-line descriptions, `<CR>` opens one; `<C-v>` `<C-s>` `<C-t>` into a
pane beside, below, a new tab).

`<leader>ik` reads the page of the word under the caret beside your
code instead (`:man beside`, which takes the same words): in the **man
column**, a column of its own made next to the pane you are in, or the
pane already showing a page, so the next `<leader>ik` replaces the
page there rather than open another column. The page is rendered to
the column's width and the keys go to it. So `<leader>ik` on
`strftime` in your code opens `strftime(3)` beside it; on nothing it
opens the picker, whose `<CR>` goes to the man column too.
`<leader>iK` is `:man`: the page in the pane you are in.

| keys | where the page goes |
|---|---|
| `<leader>ik` | the man column (`:man beside`) |
| `<leader>iK` | this pane (`:man`) |

It is a buffer: `/` finds, `y` yanks, `*` searches the word, and a
session brings it back. In it:

| keys | what |
|---|---|
| `K`, `<CR>` | the page the reference under the caret names (`chmod(1)`), or the word there, in this pane: in the man column, it stays there |
| `<C-o>` | back to the page, or the file, you came from — a jump |
| `]]` `[[` | the next, previous section head (`NAME`, `SYNOPSIS`, …), COUNT on, put at the pane's top |
| `q` | closes the page (`:bd`); in a column `<leader>ik` made, the column too, the keys back where they were |

`man.width` renders every page at a width of your own instead of the
pane's; `man.command` names the reader when it is not `man` on your
path (`env MANPATH=/opt/man man`). Windows has no `man`; `wsl man`
reads WSL's.

## The syntax tree

`:syntax_tree` (or `:tree`) opens the syntax tree tab in the developer
tools panel (F12 toggles the panel). It shows the focused buffer's
tree-sitter tree, one node per row, indented by depth. The node under
the caret is marked and the tree opens to it as the caret moves.
Clicking a row selects that node's text; its fold hides or shows the
children, and a toggle at the top shows the anonymous nodes too. Run
the command again to close it.
