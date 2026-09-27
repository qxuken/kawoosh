# Code

What kawoosh knows about the code you edit: syntax highlighting,
language servers (hover, go to definition, rename, diagnostics,
completion), compile commands, and the syntax tree.

## Syntax highlighting

Highlighting comes from tree-sitter grammars built into kawoosh:
Rust, TOML, CSS, JavaScript, TypeScript, TSX, Go and `go.mod`, Lua,
Bash, Nushell, C, C++, Python, JSON, JSONC, YAML, SQL, diffs, git
commit messages and Markdown, with regular expressions and JSDoc
highlighted inside the languages that hold them. A file's language is
chosen by its extension, its name or its shebang line.

`:syntax NAME` (also `:setf`, `:ft`) reads the buffer as another
language, `text` for none; `:syntax` alone says which one it is. A
language of your own is added from Lua with `kawoosh.language`; see
[lua](lua.md).

The same tree drives selections by syntax node, `<A-o>` `<A-i>`
`<A-n>` `<A-p>` ([editing](editing.md)), and the outline in the
symbols picker ([search](search.md#symbols-and-the-outline)).

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
(`typescript@5`); it does not work with TypeScript 7.

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
| `:lsp restart [LANGUAGE]` | restart one server, or all; a missing program is looked for again |
| `:lsp logs [LANGUAGE]` | what a server said, its errors included, live; `:lsp logs clear` forgets it |
| `:lsp toggle RULE [LANGUAGE]` | flip `enabled`, `load_all` or `inlay_hints` for the session |

### Keys

| keys | what |
|---|---|
| `K` | hover: what the server says about the symbol, in a pane |
| `gd` | go to the definition (`:lsp definition`) |
| `gD` | go to the declaration; several are a list |
| `gI` | go to the implementation; several are a list |
| `<leader>D` | go to the type definition |
| `gr` | references, as a list ([search](search.md#lists)) |
| `<leader>r` | rename: the prompt opens with `lsp rename NAME`; edit the name, `<CR>` |
| `<leader>ca` | code actions at the caret or over the selection, in a picker |
| `<leader>cF` | format the buffer (`:lsp format`) |
| `<leader>cI` | inlay hints on or off for the session (`:lsp hints`) |

In the hover pane, `gd` goes to a symbol the text names, `K` shows
that symbol's hover, and `q` closes it.

The code actions picker searches by title, and its preview shows what
taking one would do, as a diff. A rename or an action that changes a
file no pane shows leaves it as an unsaved buffer, and the message
names those files.

### Diagnostics

A server's errors and warnings are underlined, with the first line of
the message at the end of the row.

| keys | what |
|---|---|
| `]d` `[d` | the next, previous diagnostic |
| `<C-e>` | every diagnostic under the caret, in full, with its source and code (`ts(2322)`) |
| `<leader>ce` `<leader>cE` | the workspace's, the file's diagnostics as a list ([search](search.md#lists)) |

A server may report on files you have not opened. rust-analyzer does
for its whole crate; for TypeScript, turn on `load_all`.

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

## Compile commands

`:compile CMD` (also `:make`) runs a command into the `*compile*`
buffer. The paths with line numbers in its output are locations:
`<CR>` on one opens it, and `]q` `[q` (`:cnext`, `:cprev`) walk them
from anywhere. `<C-c>` in `*compile*` stops the command and what it
started (`:compile kill`).

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
