# Formatters: prettier and the rest, and what they say of indentation

Status: decided and built 2026-09-29 (roadmap step 71), in the six
rounds of the build order; "Built" at the end says where the build
departed from the text. The calls are taken here, each the user's to
overturn. Asked, after
[editorconfig.md](editorconfig.md): "there is also tools like prettier
and eslint that can dictate the rules. they probably offer some
protocol to get it?", then "we probably should be able to format using
them". Companion to [lsp-rules.md](lsp-rules.md), whose `lsp.NAME`
shape the formatters' settings follow.

## What there is

- **Formatting is the language server's alone.** `:lsp format` (`grf`)
  sends `textDocument/formatting` with the buffer's `shiftwidth_in` and
  `expandtab_in` and applies the edits if the buffer is still at the
  version sent (`Event::Formatted`). A TypeScript project formatted by
  prettier gets typescript-language-server's formatter instead, which
  is not prettier and disagrees with it.
- **Indentation comes from the settings and `.editorconfig`**
  (editorconfig.md Decision 1). A project whose `.prettierrc` says
  `useTabs: true` and has no `.editorconfig` is indented with spaces
  until the formatter rewrites it.
- **What exists to build on:** `Io::run_process_with` pipes a text to a
  process's stdin off the frame; `Editor::apply_edits` applies edits as
  one undo node and carries every caret through them; a buffer's own
  settings sources (`Editor::locals`, the scope's second tier);
  `trust.rs`'s confirm-and-record for code from a repository. There is
  no line diff: `doc::diff_edit` answers one span from the first
  difference to the last.

## There is no protocol

No tool answers "what are your whitespace rules?" in a common way, and
LSP has no such request — the editor tells the server `tabSize` and
`insertSpaces`, not the other way round. Each tool has its own way:

| tool | configured by | ask it |
|---|---|---|
| prettier | `.prettierrc` (json, yaml, toml), `.prettierrc.{js,cjs,mjs,ts}`, `prettier.config.*`, `"prettier"` in `package.json`; reads `.editorconfig` too | only the Node API (`resolveConfig`); the CLI finds the file (`--find-config-path`) but does not print what it resolved |
| biome | `biome.json`, `biome.jsonc` (default: tabs, 2 wide) | none |
| rustfmt | `rustfmt.toml`, `.rustfmt.toml` | `--print-config current PATH` |
| stylua | `stylua.toml`, `.stylua.toml` (default: tabs, 4 wide) | none |
| clang-format | `.clang-format`, `_clang-format` | `--dump-config --assume-filename=PATH` |
| ruff | `ruff.toml`, `.ruff.toml`, `[tool.ruff]` in `pyproject.toml` | `ruff check --show-settings PATH` (everything, unstructured) |
| eslint | `eslint.config.*`, `.eslintrc*` | `--print-config PATH`: seconds per directory, and its formatting rules are deprecated since 8.53 |

A config written in code (prettier's, eslint's) can only be read by
running it. So the answer is not a parser per format (Decision 6).

## Decisions

### 1. A formatter is data, `format.NAME`, as a server is `lsp.NAME`

```lua
-- the shipped prettier, as the defaults' layer spells it
format = {
  prettier = {
    cmd = "prettier",
    args = { "--stdin-filepath", "{path}" },
    languages = { "javascript", "typescript", "tsx", "json", "jsonc", "css", "yaml", "markdown" },
    when = { ".prettierrc", ".prettierrc.json", ".prettierrc.yaml", ".prettierrc.yml",
             ".prettierrc.toml", ".prettierrc.js", ".prettierrc.cjs", ".prettierrc.mjs",
             ".prettierrc.ts", "prettier.config.js", "prettier.config.cjs",
             "prettier.config.mjs", "prettier.config.ts", "package.json:prettier" },
    node = true,              -- node_modules/.bin/prettier first
    range = { "--range-start", "{start}", "--range-end", "{end}" },
    probe = { javascript = "if (a) {\nb;\n}\n", json = "{\n\"a\": 1\n}\n", css = "a {\nb: c;\n}\n" },
  },
}
```

A formatter reads the buffer's text on stdin and writes the formatted
text on stdout. `{path}` is the buffer's path (the tool finds its
config and its ignore file from it), `{start}` and `{end}` a range's
bytes. It runs in the directory its `when` file was found in, else the
buffer's. `when` is the files that say a project uses it, found from
the buffer's directory up: a name, or `FILE:KEY` for a key in a JSON
file. `node = true` looks for `node_modules/.bin/CMD` from the buffer
up before the `PATH`, so a project's own prettier version formats it.

It is data in the settings tree, so a project's `.kawoosh/settings.lua`
can add args or turn one off (`format.prettier.enabled = false`), and
`:settings` lists them. `kawoosh.formatter(name, def)` adds one from
Lua, and a def may be code: `run = fn(ctx, text, done)` for a tool that
is not stdin to stdout — eslint's fixes, for instance, come back as
JSON (`eslint --fix-dry-run --stdin --stdin-filename PATH --format
json`), which a few lines of Lua unwrap. Eslint is not shipped
(Decision 7).

Shipped:

| name | languages | runs | used unnamed when |
|---|---|---|---|
| `prettier` | javascript, typescript, tsx, json, jsonc, css, yaml, markdown | `prettier --stdin-filepath {path}` | a prettier config above the buffer |
| `biome` | javascript, typescript, tsx, json, jsonc, css | `biome format --stdin-file-path={path}` | `biome.json` or `biome.jsonc` |
| `stylua` | lua | `stylua --stdin-filepath {path} -` | `stylua.toml` or `.stylua.toml` |
| `clang-format` | c, cpp | `clang-format --assume-filename={path}` | `.clang-format` or `_clang-format` |
| `ruff` | python | `ruff format --stdin-filename {path} -` | `ruff.toml`, `.ruff.toml`, `pyproject.toml:tool.ruff` |
| `gofmt` | go | `gofmt` | always |
| `taplo` | toml | `taplo fmt -` | `.taplo.toml` or `taplo.toml` |
| `shfmt` | bash | `shfmt --filename {path}` | never; only when named |
| `rustfmt` | rust | `rustfmt --edition 2021` | never; only when named |

Rust formats through its server by default: rust-analyzer runs rustfmt
with the crate's edition, which rustfmt on stdin does not know. stylua
and biome run only with their config, since without one they indent
with tabs and would fight the language's shipped way.

Beaten: **Lua plugins for every formatter**, the engine only running a
command. Formatting sits on the save path and `:wq` has to wait for
it; that is the engine's to get right once, not each plugin's. The
defs stay data a user edits, and `run` is the door for the odd one.
**Formatters as a language's attribute** (`kawoosh.language { format =
… }`): one tool formats many languages, and which one a project uses is
the project's word, not the language's.

### 2. Which one formats a buffer: the setting, else what the project uses

`formatter`, read through the buffer's scope (editorconfig.md
Decision 1), so it can be said per language and per project:

```lua
return {
  language = { typescript = { formatter = "biome" } },  -- or a list, tried in order
}
```

Its default, `"auto"`, is the first of:

1. a formatter for the language whose `when` file is above the buffer,
   the nearest file winning (a monorepo member with its own
   `biome.json` under a root `.prettierrc` is biome's);
2. one whose `when` is `always` (gofmt);
3. `lsp`, the buffer's server, when it formats;
4. `indent`, the syntax's indentation alone (indent.md), when the
   language has indent rules — added 2026-09-30, asked: "can it be
   used as a last resort for formats? … we don't have any lsps for
   json, tomls, yamls";
5. none, and `:format` says so.

`lsp` is a formatter name like the others, so
`language.typescript.formatter = "lsp"` keeps today's behaviour, and
`{ "prettier", "lsp" }` falls back to the server where there is no
prettier. `:format?` says which one and why (`prettier:
web/.prettierrc`).

### 3. `:format`, and the result applied as a line diff

`:format` formats the buffer with its formatter, `:format NAME` with
that one; `:format selection` (`grf` in visual mode) the selection,
where the formatter has a `range` (prettier, stylua, clang-format).
`grf` becomes `:format`; `:lsp format` stays.

The text is sent with its version, off the frame, and the answer
applies only while the buffer is still at that version, as LSP's does.
The formatted text is not put in whole: a **line diff** (`imara-diff`'s
histogram, which helix uses for the same job) turns it into the edits
that change only the lines that changed, applied through `apply_edits`
as one undo node. A caret on a line the formatter did not touch stays
where it was, and a mark stays on its line.

A failure — a syntax error, a missing binary, a tool past
its `timeout_ms` (default 5000) — is a message with the tool's first
line of stderr, the whole of it in the log, and the buffer left as
it was. A private buffer (secrets.md) is never sent to a formatter, as
it is never sent to a server. A host's buffer (domains.md) formats
through its server only: a local tool would resolve its config against
a path that is not here.

Beaten: **the formatted text put in whole** (`restore`, as `:e!` does):
every caret would land at the end of one span, and the undo node would
be the whole file. **One span from `diff_edit`**: the same, from the
first changed line to the last.

### 4. Format on save: the write waits for it

`format_on_save`, a boolean read through the buffer's scope, off by
default: `language.go.format_on_save = true` in the user's file, or
`format_on_save = true` in a project's. The save:

1. `:w` starts the format and says `formatting…`;
2. its answer, at the version sent, is applied, then the save's tidy
   (editorconfig.md Decision 4), then the write;
3. an answer at another version (the user typed meanwhile), a failure
   or the timeout writes the text as it is and says why — a formatter
   never stops a file being saved.

`:wa`, `:wq` and `:wqa` wait for every format they started, bounded by
the timeout, before they write and quit. `:w!` writes at once without
formatting: the way to save a file the formatter chokes on.

Beaten: **write, then format and write again**: two writes, and a
watcher sees a file that was never meant. **Formatting on the frame**:
prettier is 150–400 ms of node starting.

### 5. Code from a repository runs once allowed

A project's `node_modules/.bin/prettier` is the repository's code, and
so is a `prettier.config.js`, which any prettier loads. `:format` asked
for is the user running it, as `:compile` is. What runs **unasked** —
a format on save, the probe of Decision 6 — runs a tool that is the
project's own, or reads a config that is code, only once allowed. The
first time, a confirm names the tool and the files (`web/ runs its own
prettier: node_modules/.bin/prettier, prettier.config.mjs`); *allow*
records the project's directory and the tool in the store's `trust`
namespace, beside the `init.lua` records, by path — a reinstall changes
the binary and should not ask again. `:trust?` lists them, `:trust
revoke` forgets them. A tool on the `PATH` with a static config needs
no allowing.

Noted, not changed here: typescript-language-server already loads a
workspace's TypeScript from its `node_modules` unasked.

### 6. What a formatter says of indentation: ask it by formatting

The question the round began with. A formatter's config is the truth
for the files it formats — it rewrites them to it on every save — so
its word is a source of the buffer's own, after its `.editorconfig`'s
and so over it: `tabstop = 2 (prettier: web/.prettierrc)` on
`:set tabstop?`.

It is read by **formatting a probe**: a few lines per language with one
nested block, unindented (`if (a) {\nb;\n}\n`), sent with the buffer's
path. The indented line in the answer is the indent — a tab, or so
many spaces. That is the one protocol every formatter speaks: it reads
a config in code as well as one in JSON, it includes whatever the tool
merges (prettier's `.editorconfig`, a monorepo's overrides), and it
needs no parser for TOML, YAML or JavaScript. It gives `expandtab` and
`shiftwidth`; `tabstop` stays the lower tiers' when the indent is a
tab.

It runs once per formatter and config file, off the frame, and is kept
until the config file changes (the file is on the config watch, as
`.editorconfig` is) — for the formatter the buffer would format with,
and only when allowed (Decision 5). Until the answer, the buffer reads
as before. A formatter without a probe for the language says nothing:
markdown's list indent is the marker's width, not a setting.

Beaten: **a reader per config format** — JSON for biome, TOML for
stylua and rustfmt, YAML for clang-format, and prettier's six
spellings, three of them JavaScript. More code than the probe, a TOML
and a YAML parser the engine does not have, still blind to a JS
config, and it drifts each time a tool renames a key.
**`--dump-config` where there is one**: two tools of seven.

### 7. Not eslint

Eslint deprecated its formatting rules in 8.53 (`indent`,
`no-trailing-spaces`, `eol-last`, `linebreak-style`; they live on in
`@stylistic`), and a project that uses eslint nearly always formats
with prettier, which the probe reads. `--print-config` loads the whole
plugin tree per directory. `eslint --fix` is a fixer, not a formatter,
and a def with `run` can do it (Decision 1).

## Build order

1. **The line diff**: `imara-diff` in `doc`, and
   `Editor::replace_diffed(id, text, version)` — a new text as the
   edits that change only what changed. Tested alone; `:e!` could use
   it after.
2. **Formatters and `:format`**: `format.NAME` in the defaults,
   declared; the choice (Decision 2), the binary found, the run and
   its timeout; `:format`, `:format NAME`, `:format?`, `grf`; `lsp` as
   a formatter. Tested with a fake formatter, a shell one-liner over
   stdin.
3. **Format on save**: the write that waits, `:wq` and `:wa` waiting,
   `:w!` past it.
4. **Allowing** a project's own tool (Decision 5).
5. **The probe**, and the buffer's source from it (Decision 6).
6. **Lua**: `kawoosh.formatter(name, def)` with `run`, and
   `kawoosh.format()`.

## Risks

- **A tool's CLI moves.** The args are data a user can fix in their
  own file the day it happens, without a release.
- **node's start-up.** A format is 150–400 ms for prettier; on save it
  is felt, not blocking. A daemon (`prettierd`) is a def with another
  `cmd`.
- **A probe the tool rejects** (a snippet it cannot parse): the probe
  fails and the buffer keeps the lower tiers — said on `:format?`,
  never a toast.
- **Windows**: `node_modules/.bin/prettier.cmd`; the lookup tries the
  `.cmd` there.

## Built

Six commits, one a round, on `claude/formatters-note`. Where the build
departed from the text above, and what it found:

- **The timeout is a def's**, `format.NAME.timeout_ms`: a bare
  `format.timeout_ms` would have been read as a formatter named
  `timeout_ms`.
- **A range is `:format selection`**, bound to `grf` in visual mode,
  not `:format` from visual mode — whether a selection survives the
  `:` prompt is not a thing to lean on. The selection is taken as a
  visual operator takes it (`Editor::selection_ranges`). A server's
  `rangeFormatting` is not asked: `lsp` formats the whole buffer and
  says so for a selection. prettier counts its range in UTF-16 units,
  so the placeholders are `{start}` `{end}` `{length}` in bytes and
  `{start_utf16}` `{end_utf16}`.
- **The line diff pairs lines.** imara-diff gives adjacent changed
  lines as one hunk; a hunk with as many lines each side (a reindent)
  is an edit per line, each narrowed to the bytes that differ, so an
  indent put in is an insert before the line's text and every caret on
  those lines keeps its character.
- **A Lua `run` is asynchronous**, asked mid-build: "some formats are
  slow, so add async version or make run be asyncable".
  `run(ctx, text, done)` answers through `done` whenever it has — from
  a `kawoosh.spawn`'s `on_exit` — or returns the text at once; the
  def's timeout holds for both, and an error before an answer is the
  answer. A Lua formatter's data goes into the engine's layer, so a
  user's `format.NAME` overrides it key by key; `run = "lua"` marks it
  in the tree.
- **A save's quit waits in the shell.** The engine hands the buffers
  that format on save to the shell as one
  `Effect::FormatThenWrite { buffers, after }` — after the
  changed-on-disk check, so a conflict is asked before anything runs —
  and the shell writes each through `Editor::write_now` and quits once
  every write of that save has landed well. A server's format has no
  timeout of its own, so its save waits for its answer, a failed
  `formatting`, or a deadline checked each frame. `:w!` writes at once,
  unformatted — the bang already meant "write it as it is".
- **Local sources name their kind.** A buffer's own sources are named
  `editorconfig: /repo/.editorconfig [*.ts]` and `prettier:
  /repo/.prettierrc`, so `:set KEY?` says which. With
  `editorconfig.enabled` off a buffer still has its own (empty)
  sources, so a formatter's word applies.
- **A probe is asked again** when its config is saved (on the config
  watch) and when the `format` table changes — new args may indent
  otherwise.

Tests: `kawoosh/tests/format.rs` (the nearest config's formatter, a
caret on an untouched line kept, one undo, a failure's line, named, off
and `lsp`, gofmt always, a range; a save formatted, `:w!`, a failing
formatter's file still written, `:wqa` formatting two buffers then
quitting; a project's own binary allowed; the probe over an
`.editorconfig`, a saved config read again, a probe with no indent),
`kawoosh/lua/tests/formatter.lua` (at once, later, a failure, a Lua
formatter's probe), the line diff's and `replace_diffed`'s, and
`systems/src/filter.rs`'s run, failure, timeout and missing program.
The formatters in the tests are `/bin/sh` one-liners defined as a user
would, and the real clang-format where it is installed.

Left:

- A config file made after a buffer was opened is seen by `:format`
  and by a save, which choose afresh, but the buffer's indent is not
  probed again until its path or the settings move.
- A server's `rangeFormatting`.
- Of the shipped defs, clang-format runs for real in a test where it is
  installed (`the_shipped_clang_format_runs_for_real`: chosen by its
  config, its `IndentWidth` probed, the file formatted), and gofmt's
  and rustfmt's command lines were run by hand. Its first run showed
  the C probe `int f() { return 0; }` folded onto one line by LLVM's
  style, so the C and C++ probes have two statements. prettier, biome,
  stylua, ruff, taplo and shfmt are checked against their
  documentation only: the first real run on a project is the check.
