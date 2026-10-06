# kawoosh

**tmux and neovim, collapsed into one process.**

kawoosh is a modal editor and a terminal multiplexer in one GPU-drawn window. Terminals, buffers and plugin panes are peers in the same splits, tabs and dock, and you drive all of them with vim's keys.

## Why collapse them

If you live in tmux with an editor per tab, every editor starts its own `rust-analyzer`, its own `tsserver`, its own everything. In kawoosh there is one process, so there is one language server per project, shared by every pane that looks at it. The same holds for buffers, search, history and sessions.

The terminal stops being a foreign country:

- `$EDITOR` in a kawoosh terminal opens the file in the pane next to it.
- Scrollback becomes a buffer you search and yank from with ordinary motions.
- Build output lands in a buffer, and `]q` walks the errors.

## Everything is text you can edit

- **The file manager is a buffer.** Rename, move, create and delete by editing the listing.
- **Project search results are a live multibuffer.** Edit the matches in place and the files change.
- **Diagnostics and references** open the same way.
- **What you yanked, deleted and visited is kept**, across restarts, in a working memory you can browse.

## Remote without a server

Edit on another machine over the `ssh` you already configured: files, terminals, builds and language servers run there. Nothing is installed on the host.

## Hackable all the way down

The file manager, pickers, project search, version control and settings panes are Lua plugins, written against the same API your `init.lua` gets. Settings are plain data. If you dislike how a built-in works, its source is the example for replacing it.

## Batteries included

- Vim's editing (motions, operators, text objects, macros, `.`) with several selections: `<C-n>` adds the next match, `<C-j>`/`<C-k>` add carets on nearby lines, and Alt moves the selected text or selects syntax nodes.
- Language servers (diagnostics, completion, hover, rename, code actions, formatting, inlay hints) with one-command installs (`:lsp install`), and formatters.
- Tree-sitter highlighting for two dozen languages, more installed on demand (`:grammar install`), plus grammars of your own.
- Pickers for files, buffers, grep, symbols, commands and recent places.
- Version control: hunks in the gutter, review, blame and log.
- Splits, tabs, a dock, and tabs that scroll as a strip of columns; marks, jumps, sessions and workspaces.
- Terminals with prompt jumps for shells that mark their prompts.
- Markdown drawn rendered in the buffer that edits it.
- Masked secrets: `.env` values and similar files are drawn as dots and kept out of history.
- Themes and fonts panes, with a lab that checks a theme's contrast.
- In-app help (`:help`) and a tutorial (`:tutor`).

## Status

Young, single-author, and macOS-first; Windows and Linux builds exist but are less tested. Written in Rust on kui, a GPU UI toolkit built for it, and configured and extended in Lua 5.5.

## How it was built

Most of the code was written by an AI coding agent (Claude Code) under one person's direction. The design is the author's: what the editor is, how it should behave, and which trade-off wins were decided in conversation, one feature at a time, and each decision is recorded in `docs/design/` with the alternative it beat. Commit messages quote the request that started each change and the result of the full check (`nu scripts/verify.nu`: formatting, clippy with warnings denied, every test) that it passed before merging. The author uses kawoosh daily, and most changes began as something that got in the way.

## Requirements

- A stable Rust toolchain recent enough for edition 2024. The repository pins no version (there is no `rust-toolchain` file and no `rust-version`).
- A C compiler: Lua and the tree-sitter grammars are built from source.
- [kui](https://drydock9.qxuken.dev/qxuken/kui), the UI toolkit. Its crates come from crates.io at one exact pre-release. While `Cargo.toml` takes them by path (pointing at `../kui/crates/...`, during a kui round), clone kui next to this repository as `../kui`.
- Git LFS, for the shipped fonts under `assets/fonts` (about 250 MB). Run `git lfs install` and `git lfs pull` after cloning. Without them the app looks for Iosevka installed on the system, and otherwise draws in the system's monospaced font.
- [nushell](https://www.nushell.sh) for the scripts under `scripts/`.

## Build and run

```sh
cargo run --release -p kawoosh            # open the last session
cargo run --release -p kawoosh -- PATH    # open a file, or list a folder
```

`kawoosh --help` lists every way in: `kawoosh test` for Lua tests, and the `edit`, `ex`, `theme` and `pick` commands that talk to a running kawoosh from one of its terminals.

As an app:

- macOS: `nu scripts/macos-app.nu` builds `Kawoosh.app` in `target/release` (or in a folder you name as the first argument). `--no-fonts` leaves the fonts out of the bundle. To use the CLI from a shell, link the binary onto your `PATH`: `ln -s /Applications/Kawoosh.app/Contents/MacOS/kawoosh ~/.local/bin/`. Built over an app a Kawoosh is running from, the new one takes its place and the running Kawoosh offers to relaunch into it. The app is in Finder's Open With for every file and folder — a type for each language `kawoosh --languages` lists, then any file — and opens what you hand it (Open With, a file dropped on the Dock icon, `open -a Kawoosh FILE`) in the running window; it takes no file type from the app that is its default.
- Windows: `nu scripts/windows-app.nu` builds a `Kawoosh` folder in `target\release` (or a folder you name). Microsoft's console host goes in beside it (`conpty.dll`, `OpenConsole.exe`: a terminal's output arrives as the program wrote it, where Windows' own redraws it in pieces), downloaded once into `target\conpty`. `--no-fonts` leaves the fonts out, and `--install` puts it in `%LOCALAPPDATA%\Programs` with a Start menu shortcut. Built while a Kawoosh runs from that folder — the one you build it in — the new one waits beside it (`Kawoosh.new`), and the running Kawoosh offers to relaunch into it: `:relaunch` quits, puts the new folder in place (or puts nothing back out of place when it can't) and starts it on your session. A folder built `--no-fonts` is replaced at once instead, the running Kawoosh's executable left aside until it quits, and the relaunch starts the new one.

`:relaunch` quits as `:qa` does and starts Kawoosh again on your session, on every platform: the Kawoosh installed by then, so after a build or an install it is the new one, and with none it is the same one started afresh. `:relaunch?` says which.

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
| `KAWOOSH_TYPES` | where the Lua type files for lua-language-server are written (default: a folder per kawoosh executable under `types` beside the state database) |

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

## License

MIT; see [LICENSE](LICENSE). That covers kawoosh's own code and its icons. Two things in the tree keep their own licences:

- The fonts under `assets/fonts`: each family's folder carries its licence, the SIL Open Font License for most, MIT for Hack and the Nerd Fonts symbols.
- The indent and text-object queries under `languages/queries` taken from [helix](https://github.com/helix-editor/helix): MPL-2.0, as the first lines of each such file say.
