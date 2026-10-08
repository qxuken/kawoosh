# kawoosh's docs

Three kinds of file, each with one job:

- **[backlog.md](backlog.md)**: what is wanted and not built. The one
  open list.
- **[parked.md](parked.md)**: what is set aside on purpose, why, and
  what would bring it back.
- **[design/](design/)**: one note per subject, saying why kawoosh is
  shaped the way it is — each decision with the alternative it beat,
  then what the build changed. A note's "Not built" or "Left" was
  true when it was written; the backlog and parked.md are what is
  true now.

[design/roadmap.md](design/roadmap.md) is the history: the steps in the
order they were built, 1 to 92. Code and notes cite it as "roadmap
step N".

How to use it is not here: that is the in-app help, `kawoosh/help/`
(`:help`, `:tutor`).

## Where it stands, 2026-10-07

mvp.md's nine milestones (2026-08-28), kui.md's eight (2026-09-15),
and the roadmap's 92 steps since are built, except folds (step 38,
parked). After step 92 the work has come as rounds on the notes: the
compile decisions 14–18, the language servers' initialisation and
incremental sync, and lua-boundary.md's thirteen rounds. 1,121
commits; 70 integration test files and 31 Lua test scripts.

## The design notes

### Foundations

- [mvp.md](design/mvp.md): the original design: tmux and neovim as one
  process, and its decisions.
- [kui.md](design/kui.md): the rebuild on kui, which replaces parts of
  mvp.md; the crate table.
- [kui-requirements.md](design/kui-requirements.md): what kawoosh asks
  of kui.
- [core.md](design/core.md): buffers, the journal, layers of runs (the
  `doc` crate).
- [domains.md](design/domains.md): where a process spawns and a path
  lives; ssh as a domain.
- [disk.md](design/disk.md): a buffer against its file: the watch, a
  change reloaded or asked about, a growing file followed.

### Keys and editing

- [keys.md](design/keys.md): the default keymap and the clusters behind
  it.
- [keymap-regroup.md](design/keymap-regroup.md): the regroup of
  2026-09-28 (LSP under `gr`, help under `<leader>i`).
- [local-maps.md](design/local-maps.md): maps local to a view, a
  buffer, a language.
- [selections.md](design/selections.md): helix's selections by a
  pattern.
- [pairs.md](design/pairs.md): auto-closing brackets.
- [comments.md](design/comments.md): `gc`, `gcc`, `gb`.
- [indent.md](design/indent.md): indentation from the syntax tree.
- [nodes.md](design/nodes.md): the syntax tree in Lua, text objects,
  `%`.
- [node-actions.md](design/node-actions.md): `g.`, actions on a node.
- [jumps.md](design/jumps.md): `<C-o>` `<C-i>`, the tab's trail.
- [wrap.md](design/wrap.md): soft wrap.
- [terminal-keys.md](design/terminal-keys.md): what a terminal pane's
  keys send.

### Panes and the window

- [launcher.md](design/launcher.md): a new pane asks what it is for.
- [pane-placement.md](design/pane-placement.md): where a pane opens.
- [scrolling-tab.md](design/scrolling-tab.md): a tab as a strip of
  columns.
- [workspaces.md](design/workspaces.md): the directory per tab, the
  dock.
- [status.md](design/status.md): the title bar and tab strip.
- [statusline.md](design/statusline.md): the status line as modules.
- [breadcrumbs.md](design/breadcrumbs.md): the outline in the title
  bar.
- [menus.md](design/menus.md): a right-click's menu and the Mac's bar.
- [icons.md](design/icons.md): one set of icons and key caps.

### Search and places

- [search.md](design/search.md): the project search over live
  multibuffers.
- [lists.md](design/lists.md): diagnostics and references as
  multibuffers.
- [marks.md](design/marks.md): the outline, marks, folds.
- [memory.md](design/memory.md): what was yanked, deleted and visited,
  kept.

### Code

- [grammars.md](design/grammars.md): grammars built ahead, installed on
  demand.
- [lsp-rules.md](design/lsp-rules.md): a language's server, switched
  per project.
- [lsp-servers.md](design/lsp-servers.md): a server for every language.
- [lsp-installs.md](design/lsp-installs.md): installs, and qd.
- [formatters.md](design/formatters.md): formatters, and what they say
  of indentation.
- [editorconfig.md](design/editorconfig.md): `.editorconfig` and the
  languages' ways.
- [compile.md](design/compile.md): compile commands, deduced, and their
  runs.
- [vcs.md](design/vcs.md): hunks, review, blame, history, worktrees.

### Buffers with a shape

- [markdown.md](design/markdown.md): the source, rendered.
- [secrets.md](design/secrets.md): masks, a private buffer, a register
  that forgets.
- [sqlite.md](design/sqlite.md): a database's pane.
- [man.md](design/man.md): `:man` into a buffer.
- [kitty-graphics.md](design/kitty-graphics.md): images in a terminal.
- The byte pane (`:hex`) and the picture pane (`:image`) have no note:
  roadmap.md steps 88 and 89 hold their design.

### Look and settings

- [themes.md](design/themes.md): a registry, a dark and a light, the
  pane.
- [fonts.md](design/fonts.md): the fonts pane and lab.
- [settings.md](design/settings.md): the settings pane.

### Lua

- [plugin-panes.md](design/plugin-panes.md): what a Lua plugin can
  draw; the map of the API.
- [lua-boundary.md](design/lua-boundary.md): where Lua ends and Rust
  begins, the bundled plugins reviewed.
- [native.md](design/native.md): native extensions, a C ABI the kui
  way; rounds one and two built.
