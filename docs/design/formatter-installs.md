# Formatter installs: prettier as eslint is, a category beside the servers

Status: decided and built 2026-10-12. Asked: "let's make prettier to be
a lsp server like eslint, maybe we should add a separate category of
formatters (like mason)". Asked back which: prettier as a server
running beside, or formatters installed as the servers are — the user
took the second ("Formatter installs"), with a CLI of its own
("Separate `kawoosh format …`"). Follows
[formatters.md](formatters.md) (a formatter is `format.NAME`, run on
stdin) and [lsp-installs.md](lsp-installs.md) (a server's package in
kawoosh's folder, locked, `lsp.ensure_installed`).

## What there was

eslint was easy and prettier was not, and the difference was the
install, not the protocol. `kawoosh lsp install eslint`,
`lsp.ensure_installed = { "eslint" }` and `:lsp install` put
vscode-eslint-language-server in kawoosh's folder, where no version
manager can take it away. A formatter had no `install`. A project with a
`.prettierrc` and no prettier on the `PATH` said "prettier is not
found", and getting one was `npm i -g prettier`, inside one fnm Node
again.

## Decisions

### 1. Not a prettier server

Prettier has no language server of its own. What npm has is small and
third-party: `prettier-language-server` (0.0.5) and
`@daliusd/prettier-lsp` (0.4.0, one commit, the project's prettier
first). Run beside as eslint is, it would need the formatter choice to
name a server (`lsp:prettier`, not the first that formats, which is
typescript's), and it would lose what the stdin formatter already has:
the project's own prettier and its allowing (formatters.md Decision 5),
the probe (Decision 6), a timeout. What a server would bring, a warm
node, `prettierd` brings as a stdin formatter (Decision 5 here).

Beaten: **`lsp.prettier` on a third-party server**, above.
**efm-langserver** wrapping the command: a Go program to install to run
a command kawoosh runs already.

### 2. A formatter's `install`, as a server's

`format.NAME.install` reads as `lsp.NAME.install` does
(`lsp_rules::package_setting`, `platform_line`): a package kawoosh
installs (`{ npm = "prettier" }`, `pip`, `cargo`, `go`, `dotnet`, with
`args`), a line, or a line per platform. The shipped ones:

| formatter | install |
|---|---|
| prettier | `{ npm = "prettier" }` |
| prettierd | `{ npm = "@fsouza/prettierd" }` |
| biome | `{ npm = "@biomejs/biome" }`, `lsp.biome`'s |
| stylua | `{ cargo = "stylua", args = { "--locked" } }` |
| clang-format | `{ pip = "clang-format" }` (LLVM's wheels) |
| ruff | `{ pip = "ruff" }`, `lsp.ruff`'s |
| taplo | `{ cargo = "taplo-cli", args = { "--locked", "--features", "lsp" } }`, `lsp.toml`'s to the argument |
| shfmt | `{ go = "mvdan.cc/sh/v3/cmd/shfmt" }` |
| rustfmt | `rustup component add rustfmt` |
| gofmt | none: Go's |

A program is found **the project's own first** (`node`), **then
kawoosh's** (`servers::find`), **then the `PATH`**. A "not found" says
how to get it: `prettier is not found, and .prettierrc says the project
formats with it; :format install prettier`.

stylua by cargo, the crate its author publishes, rather than npm's
`@johnnymorganz/stylua-bin`, a `run.js` wrapper whose way of getting
the binary was not looked into. A build of a minute or two, once; a
user's `format.stylua.install` can say npm instead.

### 3. One folder, the record says the kind *(mason's categories)*

Formatters go in the servers' folder (`servers/MANAGER/NAME`,
`$KAWOOSH_SERVERS`), not one of their own: biome, ruff and taplo are
each one program that is a server and a formatter, and two folders
would install each twice, at two versions. The record
(`kawoosh-package.json`) says what the package was installed as,
`"kinds": ["server", "formatter"]`; a record from before kinds is a
server's.

- Installing as the other kind what is in already (`kawoosh format
  install biome` with `lsp.biome` in) runs nothing: the record is marked
  (`servers::mark`), and the version stays where the server had it.
- Removing as one kind keeps the folder for the other; removing a kind
  it is not installed as is refused.
- `kawoosh lsp update` and `outdated` with no names are the servers'
  packages, `kawoosh format …` the formatters'.

Beaten: **a `formatters/` folder**: the double install above. **No
kind**, a formatter's package found by matching the defs: a package a
user's settings named (`--spec`) is in no def the CLI reads, and bare
`update` would lose it.

### 4. `kawoosh format …`, `:format install`, `format.ensure_installed`

As the servers' (lsp-installs.md Decisions 3, 4 and 6), a verb apart:

- `kawoosh format install NAME[@VERSION]…`, `update [NAME…]`,
  `outdated`, `remove NAME…`, `list`; `--spec JSON` for a package the
  settings say. NAME is the formatter's `format.NAME`.
- `:format install [NAME]` in a terminal pane — the buffer's formatter
  when none is named: the one its `formatter` setting names first, or
  `auto`'s nearest config, found or not. `:format update [NAME]`. One
  that ends well has the buffers' formatters found afresh and the
  probes asked again.
- `format.ensure_installed = { "prettier", "stylua@2.0.2" }` and
  `format.check_updates`, beside `lsp.ensure_installed` on the same
  worker (one install at a time, whatever the kind). The day's check
  says each kind's updates on its own line: `1 formatter update:
  prettier 3.6.2 → 3.7.0 (:format update)`.

`ensure_installed` and `check_updates` are keys of `format`, as of
`lsp`: `format.NAME` with no `cmd` is no formatter, so neither reads
as one, and `kawoosh.formatter` refuses the two names.

### 5. prettierd, shipped

`prettierd {path}` reads stdin and writes stdout like prettier, to a
daemon it starts once: no node start per format (prettier's 150–400 ms,
formatters.md's Risks). It finds the project's own prettier itself, and
reads the same configs. Same `when` as prettier, after it by name: so
`auto`, at a tie of configs, takes the first that is **found** —
prettier where both are, prettierd where prettier is not. The tie rule
is new: before, a tie went to the first by name whether found or not.
`formatter = "prettierd"` uses it everywhere. No `range`: its CLI takes
none.

## Built

`systems/src/servers.rs`: `Kind`, `Record::kinds` and `is`,
`install(…, kind, …)`, `mark`, `remove(…, kind)`. `kawoosh/src/lsp_cli.rs`'s
verbs shared (`Verbs`, `take_spec`, `each`, `install_line`,
`install_package`); `kawoosh/src/format_cli.rs`; `format::Install`,
`Formatter::install`, `pick`'s kawoosh lookup, `:format install`,
`:format update`; `lsp_installs.rs` taking both lists; the defaults'
`install`s and `prettierd` in `editor/src/settings.rs`.

Tests: `servers::tests::a_record_says_its_kinds`,
`a_package_is_one_directory_for_both_kinds`;
`format_cli::tests::the_shipped_formatters_say_how_they_install` (and
that biome's, ruff's and taplo's packages are their servers');
`kawoosh/tests/format.rs`
`a_formatter_is_installed_into_kawooshs_folder_and_runs_from_there` and
`format_ensure_installed_installs_and_says_updates`, over a fake `npm`.

Left: none of the shipped installs was run against its registry here;
the package names were checked on npm, PyPI and the Go proxy
(2026-10-12). `:lsp servers` has no formatters' twin in the window
(`kawoosh format list` is the list).
