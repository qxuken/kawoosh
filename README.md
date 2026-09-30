# kawoosh

kawoosh is a modal editor and a terminal multiplexer in one window. Editing is vim's (motions, operators, text objects, macros, `.`), with multicursor editing and a few helix and Zed habits on top. Terminals, editor buffers and plugin panes live side by side in the same splits, tabs and dock. It is written in Rust and draws on kui, a GPU UI toolkit. It is configured and extended in Lua 5.5, and the file manager, the pickers and the project search are Lua plugins written against the same API a user's `init.lua` has.

## Features

- Vim-style modal editing with several selections: `<C-n>` adds the next match, `<C-j>`/`<C-k>` add carets on nearby lines, and Alt moves the selected text or selects syntax nodes.
- Terminals in panes, with a copy mode that turns the scrollback into a buffer, prompt jumps for shells that mark their prompts, and `$EDITOR` that opens files back in the editor.
- Splits, tabs, a dock, and tabs that scroll as a strip of columns.
- A file manager whose listing is a buffer you edit to rename, move, create and delete.
- Pickers for files, buffers, grep, symbols, commands and recent places; a project search whose results are a live, editable multibuffer.
- Language servers (diagnostics, completion, hover, rename, code actions, formatting, inlay hints) and tree-sitter highlighting for two dozen languages, plus grammars of your own.
- Compile mode: build output in a buffer, with `]q` walking the errors.
- A working memory of what you yanked, deleted and visited, kept across restarts; marks, sessions and workspaces.
- Markdown drawn rendered in the buffer that edits it.
- Themes and fonts panes, with a lab that checks a theme's contrast.
- Editing on other machines over your own `ssh`, with nothing to install on the host.
- Masked secrets: `.env` values and similar files are drawn as dots and kept out of history.
- In-app help (`:help`) and a tutorial (`:tutor`).

## Requirements

- A stable Rust toolchain recent enough for edition 2024. The repository pins no version (there is no `rust-toolchain` file and no `rust-version`).
- A C compiler: Lua and the tree-sitter grammars are built from source.
- [kui](https://drydock9.qxuken.dev/qxuken/kui), the UI toolkit. Its crates come from the drydock9 registry named in `.cargo/config.toml`. While `Cargo.toml` takes them by path (it does now, pointing at `../kui/crates/...`), clone kui next to this repository as `../kui`.
- Git LFS, for the shipped fonts under `assets/fonts` (about 250 MB). Run `git lfs install` and `git lfs pull` after cloning. Without them the app looks for Iosevka installed on the system, and otherwise draws in the system's monospaced font.
- [nushell](https://www.nushell.sh) for the scripts under `scripts/`.

## Build and run

```sh
cargo run --release -p kawoosh            # open the last session
cargo run --release -p kawoosh -- PATH    # open a file, or list a folder
```

`kawoosh --help` lists every way in: `kawoosh test` for Lua tests, and the `edit`, `ex`, `theme` and `pick` commands that talk to a running kawoosh from one of its terminals.

As an app:

- macOS: `nu scripts/macos-app.nu` builds `Kawoosh.app` in `target/release` (or in a folder you name as the first argument). `--no-fonts` leaves the fonts out of the bundle. To use the CLI from a shell, link the binary onto your `PATH`: `ln -s /Applications/Kawoosh.app/Contents/MacOS/kawoosh ~/.local/bin/`.
- Windows: `nu scripts/windows-app.nu` builds a `Kawoosh` folder in `target\release` (or a folder you name). `--no-fonts` leaves the fonts out, and `--install` puts it in `%LOCALAPPDATA%\Programs` with a Start menu shortcut.

## Configuration

Your configuration lives in `~/.config/kawoosh/` (or `$XDG_CONFIG_HOME/kawoosh/`):

- `settings.lua` returns a table of settings. It is data, not code, and is read again when you save it.
- `init.lua` is Lua code run at startup, and again when you save it: commands, keys, hooks, panes of your own.
- `fonts/` holds font files of your own, picked up as they are added.

A project can carry `.kawoosh/settings.lua` and `.kawoosh/init.lua`. A project's `init.lua` runs only after you trust it.

State (sessions, the memory, trusted files) is kept in `~/.local/share/kawoosh/state.db` (or under `$XDG_DATA_HOME`).

Environment variables override the paths:

| variable | what it names |
|---|---|
| `KAWOOSH_SETTINGS` | the settings file |
| `KAWOOSH_INIT` | `init.lua` |
| `KAWOOSH_STATE` | the state database |
| `KAWOOSH_FONTS` | the user fonts folder |
| `KAWOOSH_TYPES` | where the Lua type files for lua-language-server are written (default: `types` beside the state database) |

## Help

Inside kawoosh, `:help` opens the help pages and `:help TOPIC` goes to a page, a command or a key; `:tutor` opens a hands-on tutorial to practise on. The pages are the Markdown files in `kawoosh/help/`.

## Repository layout

| path | what |
|---|---|
| `text-buffer/` | the text storage: a persistent piece table, where a snapshot is a cheap clone |
| `doc/` (`kawoosh-doc`) | buffers, the edit journal, and layers of runs (syntax, diagnostics) carried through edits |
| `editor/` (`kawoosh-editor`) | the modal engine: views with selection sets, modes, the command registry, the keymap, settings |
| `term/` (`kawoosh-term`) | terminals: `alacritty_terminal` and a pty behind a grid of cells |
| `systems/` (`kawoosh-systems`) | background threads talking to the main loop over channels: io, files, search, tree-sitter, language servers, the state store, SFTP |
| `languages/` (`kawoosh-languages`) | what kawoosh knows about each language: how a file is recognised, its grammar and highlight queries |
| `lua/` (`kawoosh-lua`) | the `kawoosh` Lua API; `lua/lua/boot.lua` is its Lua half |
| `kawoosh/` | the application: the window, panes, terminals, commands, and the headless test harness |
| `kawoosh/lua/` | the bundled Lua plugins, and their tests in `kawoosh/lua/tests/` |
| `kawoosh/help/` | the in-app help pages |
| `assets/` | the shipped fonts (Git LFS) and the icons |
| `scripts/` | the app builds and the checks |
| `docs/design/` | the design notes |

## Checks

```sh
nu scripts/verify.nu        # cargo fmt --check, clippy with warnings denied, every test
nu scripts/verify.nu --fix  # format the tree instead of failing on it
```

The tests include the Lua plugin tests in `kawoosh/lua/tests/`, which `cargo test` runs through the headless harness.

## Design notes

`docs/design/` records why kawoosh is shaped the way it is, one note per subject:

- [mvp.md](docs/design/mvp.md): the original design and its decisions.
- [kui.md](docs/design/kui.md): the rebuild on kui, which replaces parts of mvp.md.
- [keys.md](docs/design/keys.md): the default keymap and the rules behind it.
- [roadmap.md](docs/design/roadmap.md): what was built, in what order, and what is left.

The rest cover one feature each: search, marks, lists, themes, fonts, domains (remote editing), memory, secrets and more.
