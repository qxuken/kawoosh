# `.editorconfig` and the languages' ways

Status: decided and built 2026-09-29 (roadmap step 69), the calls taken
here, each the user's to overturn. Asked: "Let's implement .editorconfig
support and helpers to init it. also some default profiles per langs."

## What there was

`tabstop` and `expandtab`, two keys of the settings tree, read the same
for every buffer: a Go file indented with four spaces unless the user
flipped `expandtab` by hand, and a TypeScript file after it indented
with tabs. An indent was always `tabstop` wide — no `shiftwidth` — and a
save wrote the text as it stood. Nothing read a repository's
`.editorconfig`, which most projects that care about their whitespace
ship.

## Decisions

### 1. A buffer reads its settings through a scope

A setting that is a buffer's — its indentation, what its save tidies —
is read through the buffer's **scope** (`Settings::scoped`, `Scope`): its
language and its own sources. Tiers, the first that has the key
winning:

1. the session's `language.LANG.KEY`, then its bare `KEY` — what was
   typed (`:set tabstop=2`) is meant now, over any file;
2. the buffer's own sources: what the `.editorconfig` files above it
   say, a source per section (`/repo/.editorconfig [*.ts]`);
3. `language.LANG.KEY` in the project's layer, the user's, the
   default's;
4. the bare `KEY` in the project's, the user's, the default's.

So a language's key beats a bare key whatever layer each is from, and
between two of a kind the higher layer wins: a user's `tabstop = 2`
does not make Go's tabs two wide or undo markdown's kept trailing
spaces — it is their preference where a language has no way of its
own, as a vimrc's `set` is under a filetype plugin's `setlocal`.
`language.go.tabstop = 8` in their file is how they say Go's.

The engine keeps a buffer's own sources in `Editor::locals`, keyed by
the path they were resolved for; a buffer whose path is not that one
(`:w other`) reads without them until the shell looks again. Every
reader asks for the buffer: `tabstop_in`, `expandtab_in`,
`shiftwidth_in`, `indent_unit_in`, `setting_in` — the pane's tab width,
`<Tab>` in insert mode, `>` `<` and `<A-h>` `<A-l>`, a server's format
options, `:set KEY?` (which names the tier: `editorconfig: … [*.ts]`,
`default (language.go)`), and Lua's `kawoosh.buf.indent()`.

Beaten: **a buffer-local layer written into at open** (vim's
`setlocal`, neovim's editorconfig plugin). Copies go stale: a user's
`language.go` edited after the open, a `:set` meant for every buffer,
a `.editorconfig` saved — each would have to find and rewrite the
copies. A read through the scope is a few map lookups and always the
tree's word. **Language tables that lose to a higher layer's bare key**
(VS Code's order): then a user's one `tabstop = 2` breaks Go.

### 2. Profiles are the defaults' `language` tables

What a language's community writes — gofmt's tabs, prettier's two
spaces — ships as `language.NAME` tables in the engine's layer, where
`:settings` lists them and a user's file overrides a key at a time:

| language | ships |
|---|---|
| javascript, typescript, tsx, json, jsonc, css, yaml, markdown, lua | `tabstop = 2` |
| go, gomod, odin | `expandtab = false`, `tabstop = 4` |
| markdown, diff, gitcommit | `trim_trailing_whitespace = false` |

Every other language takes the bare keys (four spaces). `language` is
declared an open table, and a key under a language is checked as the
bare key it names, so `language.go.tabstp` is still a warning.

Beaten: **a table per language in the language registry**
(`kawoosh.language { indent = … }`). Indentation is a preference, not
a fact about the grammar; in the settings it layers, shows in the tab,
and a project's `.kawoosh/settings.lua` can say it.

### 3. `.editorconfig` above the project, below the session

The files are read from the buffer's directory up, a nearer file's
word over a farther one's, a later section's over an earlier one's,
and a `root = true` file the last read — editorconfig.org's rules, the
globs whole (`*`, `**`, `?`, `[…]`, `[!…]`, nested `{a,b}`, `{1..9}`,
`\`). What a property becomes:

| property | setting |
|---|---|
| `indent_style` | `expandtab` |
| `indent_size` | `shiftwidth` (`tab` → 0, the tab's width) |
| `tab_width` | `tabstop`; unsaid, a number `indent_size` |
| `end_of_line` | `end_of_line` |
| `trim_trailing_whitespace`, `insert_final_newline` | the same |

`charset` is UTF-8's or nothing: kawoosh reads and writes UTF-8, and
another charset is named by `:editorconfig` as not applied, with
`max_line_length` and every other tool's property. `unset` takes a key
back out.

Its tier is the buffer's own, over the project's layer: the file is
the project's statement to every editor, per glob, where a
`.kawoosh/settings.lua` is general. `editorconfig.enabled = false`
turns it off. A host's file (domains.md) is not resolved: the settings
files stay local.

Beaten: **a layer of its own in `Settings`**. A layer is one tree for
every buffer; an `.editorconfig` says something different per file.

### 4. A save tidies as the settings say

`Editor::save` makes one undoable edit before it writes, from the
buffer's settings: `trim_trailing_whitespace` takes spaces and tabs off
every line's end, `end_of_line` makes every line end the one way, and
`insert_final_newline` ends the text with one (the file's own kind when
`end_of_line` says none). All three default to leaving the text as it
is, so a file is changed on save only where an `.editorconfig` or the
user said so. `false` for `insert_final_newline` leaves a file's last
line as it is rather than taking its newline off — the property's
letter says "ensure it doesn't", and no one wants a save to do that
silently. A buffer past 16 MB is written untidied.

A newline typed is still `\n`; the save makes the lines agree.

### 5. `:editorconfig init` writes one from what is there

`:editorconfig init` opens `.editorconfig` in the working directory as
a buffer — a template, unsaved, `:w` keeps it, as the settings tab's
*new* does — written from:

- `[*]`: `charset = utf-8`, `end_of_line` (the setting's, else `lf`),
  `insert_final_newline = true`, `trim_trailing_whitespace = true`, and
  the bare indentation — the conventions a new project wants, written
  out to be read and changed;
- a section for each way of the languages the project has files of (a
  walk of up to 20 000, as git sees them), where it differs from `[*]`;
  languages with the same way share one (`[*.{tsx,ts,mts,cts}]`), the
  globs from the registry's extensions and names;
- `[{Makefile,makefile,GNUmakefile,*.mk}]` tabs, when there is one — a
  make needs tabs and kawoosh has no language for it.

An `.editorconfig` already there is opened as it is. `:editorconfig`
says what applies to the focused buffer: each property, the files, and
what is not applied.

## Build

- `editor/src/editorconfig.rs`: the parse, the globs (to `regex`, the
  ranges checked after), `resolve`, the properties → settings, the
  template's writer. Unit tests for the globs, the precedence, a
  written file read back.
- `editor/src/settings.rs`: `Scope`, `scoped`, `scoped_origin`, each
  layer's merge kept; the new keys and the `language` tables.
- `editor/src/lib.rs`, `disk.rs`: `locals`, the `_in` reads, `tidy`
  (`tidy_edits` tested alone).
- `kawoosh/src/editorconfig.rs`: the files read and cached, a buffer
  resolved each frame its path is new, the places looked at on the
  config watch (a file made later counts), the commands.
- `kawoosh/tests/editorconfig.rs`, `kawoosh/lua/tests/buf_indent.lua`.

## Left

- A formatter's word (prettier's, biome's, …) as a source over the
  `.editorconfig`, and formatting through them: step 71,
  [formatters.md](formatters.md).
- `:setlocal`: a value for one buffer, typed. The scope has room for it
  (a source in the buffer's tier), nothing asks yet.
- `max_line_length` as a ruler, when there is a ruler.
- Guessing a file's indentation from its text when nothing says
  (vim-sleuth). The profiles and `.editorconfig` cover the projects
  kawoosh is used on; a guess is a heuristic to add only if missed.
