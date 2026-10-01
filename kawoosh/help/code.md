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
F#, Elixir, Erlang, Clojure, Ruby, Perl, R, Julia, Nix, HTML, Svelte,
Astro, SCSS, Makefiles, CMake, Dockerfiles, protobuf, GLSL and WGSL
among them; `:grammars` lists them all. A
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
| `lsp.go` | `gopls` | Go |
| `lsp.c` | `clangd` | C, C++ |

typescript-language-server needs TypeScript 5 installed beside it
(`typescript@5`), in the project or globally; it does not work with
TypeScript 7. A server that refuses to start, as it does without one,
is off until `:lsp restart`, and the corner says what it said and in
which project. One that exits on its own is started again for its
files, the corner saying how it ended; after three exits in three
minutes it is off until `:lsp restart`. Either way the corner names
the project's `.kawoosh/settings.lua`, where `lsp = { NAME = {
enabled = false } }` keeps it off in a project that is no place for it.

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
| `cmd` `args` | the program and its arguments; a change restarts the server |
| `roots` | the marker files that find the project root |
| `languages` | the languages this server serves |
| `settings` | the configuration sent to the server |

### Commands

| command | what |
|---|---|
| `:lsp` | the servers running, on the status line |
| `:lsp info` | the servers in a pane: each one's root, documents and rules |
| `:lsp restart [LANGUAGE]` | restart one server, or all; a missing program, or one that refused to start or kept exiting, is tried again |
| `:lsp logs [LANGUAGE]` | what a server said, its errors included, live; `:lsp logs clear` forgets it |
| `:lsp toggle RULE [LANGUAGE]` | flip `enabled`, `load_all` or `inlay_hints` for the session |

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

TypeScript shortens a long type in a message to `... 4 more ...`.
Kawoosh shows what the server sends, and the server has no
setting for this: it is the compiler option `noErrorTruncation`, so put
`"noErrorTruncation": true` in the project's `tsconfig.json`
`compilerOptions` for the whole type, in the row and in `<C-e>`.

### Completion

Completion is shown in place: the rest of the best candidate appears
as faint "ghost" text after the caret, with no menu. It asks as a word
starts and after the language's trigger characters. With no server, the
buffer's own words are offered.

| keys (insert mode) | what |
|---|---|
| `<C-n>` `<Down>` / `<C-p>` `<Up>` | the next, previous candidate |
| `<Tab>` `<C-y>` `<CR>` | take the candidate |
| `<C-e>` | drop the completion |
| `<C-Space>` | ask for completions now |
| `<C-x>` | the candidates in a picker, with their kind, signature and documentation |

## Formatting

`:format` (`grf`) formats the buffer with its formatter: prettier, biome, stylua, clang-format, ruff, gofmt, taplo, shfmt, rustfmt, the language server, or `indent` — the language's syntax putting each line at its indent and touching nothing else ([editing](editing.md#indentation)). The formatted text goes in as the lines that changed, as one undo step, so the caret stays where it was on lines the formatter did not touch.

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

A project's own copy (`node_modules/.bin/prettier`) is used before one on your `PATH`.

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

A formatter written in Lua is in [lua](lua.md#formatters).

## Compile commands

`:compile CMD` (also `:make`) runs a command into the `*compile*`
buffer. The paths with line numbers in its output are locations:
`<CR>` on one opens it, and `]q` `[q` (`:cnext`, `:cprev`) walk them
from anywhere. `<C-c>` in `*compile*` stops the command and what it
started (`:compile kill`), `r` runs it again where it ran (`:compile
again`), and `q` closes it.

`<leader>cc` is a bare `:compile`. It runs, in order of preference:

1. `compile.default`, if your settings set it;
2. else the last command compiled in this workspace, remembered across
   restarts;
3. else the first command your project's files offer.

`:compile?` says which it would be.

`%` in a command is the current file, written relative to where the
command runs.

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
to finish instead of running. `cwd` sets where it runs. `<Tab>` after
`:compile ` completes the names, then paths.

### Commands your project offers

kawoosh reads the project's files, from the current file up to the
repository root, for what they can run: `Cargo.toml` (cargo check,
build, test, clippy, run), `package.json` scripts (with the package
manager its lockfile says), justfile recipes, `build.nu`, Makefile
targets, `CMakeLists.txt`, `go.mod`, `pyproject.toml` and
`build.zig`. The ones matching the current file's language server come
first, so a Rust file offers `cargo check` and a TypeScript file its
package's scripts. `compile.deduce = false` turns this off.

`<leader>cC` (`:compile pick`) lists everything: `compile.default`,
your named commands, the last lines run here, and every command the
project's files offer, each with where it came from. `<CR>` runs it;
`<C-e>` puts it in the prompt so you can add arguments first. A
command that needs arguments always opens the prompt.

## The syntax tree

`:syntax_tree` (or `:tree`) opens the syntax tree tab in the developer
tools panel (F12 toggles the panel). It shows the focused buffer's
tree-sitter tree, one node per row, indented by depth. The node under
the caret is marked and the tree opens to it as the caret moves.
Clicking a row selects that node's text; its fold hides or shows the
children, and a toggle at the top shows the anonymous nodes too. Run
the command again to close it.
