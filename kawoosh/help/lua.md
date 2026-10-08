# Lua

How to extend kawoosh in Lua: where your code goes, the `kawoosh` table and its main parts, panes of your own, and testing a plugin. The bundled plugins (the file manager, the picker, the project search, the themes pane and more) are written against this same API, so anything they do, your code can do too.

## Where your code goes

- `~/.config/kawoosh/init.lua` is yours. It runs at startup, after your `settings.lua` has been read, so it can read your settings. Saving it runs it again, and anything it set with `kawoosh.opt` last time is removed first. `$XDG_CONFIG_HOME` moves the folder; `KAWOOSH_INIT` names another file.
- `.kawoosh/init.lua` in a project (in the working directory or any folder above it) is the project's. It is code from a repository, so kawoosh asks before running it: the first time it sees the file it shows you the file's lines and offers to trust and run it. A trusted file runs without asking again until its text changes. `:trust` trusts and runs the working directory's files, `:trust?` lists them and where each one stands, and `:trust revoke` forgets what you trusted. While a project's `init.lua` runs, `kawoosh.project` holds `{ root = ..., dir = ... }`: the project folder and its `.kawoosh` folder. `kawoosh.os` is the system kawoosh runs on, `"mac"`, `"linux"` or `"windows"`, for a command that differs by system: `kawoosh.opt("compile.commands.install", ({ mac = "make install", windows = "nu install.nu" })[kawoosh.os])`.
- `settings.lua` (yours, and a project's `.kawoosh/settings.lua`) is not code. It returns a table and runs with no access to files, processes or `kawoosh`, so a project's settings never need trust. See [settings](settings.md).

Try things without a file: `:lua CODE` runs a line of Lua, and `<leader>x` runs the line under the caret (the selection, in visual mode) and shows the result.

## Commands

Everything a key does is a named command. `kawoosh.command(name, fn, opts)` adds one, and it can be run from the command line (`:name`), a key, or other code.

- A name of two words is a subcommand: `"notes open"` runs as `:notes open` and completes under `:notes`.
- `fn(ctx)` gets `ctx.args` (the words after the name), `ctx.count`, and `ctx.bang` / `ctx.query` for `:name!` and `:name?`.
- `opts.args` says what the arguments are, one kind per position: `"path"`, `"buffer"`, `"command"`, `"option"`, `"tool"`, `"view"` or `"text"`, the last one written with `"..."` for the rest. The command line completes each kind, and a `"path"` reaches `fn` already made absolute.
- `opts.when` lists facts that must hold for the command to run, such as `"editor"` (an editor pane has the keys), `"!readonly"`, `"visual"`, `"language:rust"` or `"terminal"`. The command line refuses with the reason when one does not hold. `kawoosh.fact(name, on)` publishes a fact of your own.
- `opts.doc` is one line on what it does, shown in the command palette (`<leader>ic`); `opts.aliases` gives other spellings, and `opts.bang` / `opts.query` describe what `!` and `?` mean.
- `opts.jump = true` makes the move it makes a [jump](editing.md#jumps) however near, so `<C-o>` comes back from it; a move of a screen or more, or into another buffer, is one anyway. `kawoosh.memory { jumps = true }` reads the tab's list, newest first: `path`, `line`, `col`, `buffer` while open, and `current` on the place the list is at.

`kawoosh.cmd(line)` runs a command line right away; `kawoosh.run(line)` runs it after what was already asked for. `kawoosh.commands()` lists every command, and `kawoosh.can(name)` says whether one can run now, or why not.

## Keys

`kawoosh.map(mode, keys, cmd, opts)` binds keys. `mode` is `"n"`, `"i"`, `"v"`, `"o"` (operator pending) or `"p"` (pane mode: panes that are not editors, such as your own views). `keys` is written in vim notation (`<leader>`, `<C-w>`, `<A-S-l>`). A chord's upper-case letter means Shift, so `<A-L>` is `<A-S-l>` and not `<A-l>`, which is how `:map list` shows it. `cmd` is a command line, as you would type it after `:`, or a function.

A map can belong to a place, and then it is found only there, before the global map: `opts.view = "NAME"` (your view's pane), `view` with `field = "q"` (its field), `buffer` (a buffer handle, `true` for the current buffer, or a buffer's name such as `"*compile*"`), `language = "dir"`, or any fact as `scope = "terminal"`. There it shadows the global binding of the same keys and every longer one under them, and nowhere else does it exist, so two plugins' panes never collide on a key: `kawoosh.map("n", "<CR>", "dir enter", { language = "dir" })` is the file manager's `<CR>`, and `<CR>` stays `goto location` in every other buffer. `:map <buffer> n KEYS CMD` does the same for the buffer you are in, and a buffer's own maps go when it closes.

A key can be bound more than once in one place: the newest binding whose `opts.when` holds, and whose command can run, is the one that runs. `kawoosh.unmap(mode, keys, opts)` removes a key's bindings, the global ones or those of the place `opts` names. Inside a command a key ran, `kawoosh.pass()` hands the key on to the binding under it: the global one, under a place's.

`:map list` shows every binding in a pane (`:map list n`, or `:map list <leader>c` for the keys under a prefix), a place's own saying `in` the place (a buffer's by its name); `:map list here` shows only what applies where your keys are, the innermost place's bindings of a key first, and `:map group <leader>i insert` names a group for the which-key card. `:map export PATH` writes the keymap and every command as JSON — each binding with the command it runs, the groups' names, the leader (bare, it opens in a pane).

## Example: a command with a key

```lua
-- ~/.config/kawoosh/init.lua
kawoosh.command("today", function()
  kawoosh.buf.type(os.date("%Y-%m-%d"))
end, { when = { "editor", "!readonly" }, doc = "type today's date at every caret" })

kawoosh.map("n", "<leader>id", "today")
kawoosh.cmd("map group <leader>i insert")
```

Save the file and `<leader>id` types the date; `:today` does the same.

## Settings

A plugin reads settings the same way the editor does.

- `kawoosh.setting(path, { type = ..., doc = ... })` declares a setting your code reads. The type is `"string"`, `"boolean"`, `"integer"`, `"number"`, `"list"`, `"table"` (a table whose keys are the user's), or a list of allowed words such as `{ "on", "off" }`. A key in a settings file that nobody declared gets a warning, so declare what you read.
- `kawoosh.opt(path)` reads the value in effect (a table for a part of the tree, the whole tree with no path). `kawoosh.opt(path, value)` sets it; `nil` unsets it. What `init.lua` sets this way belongs with your own settings.
- `kawoosh.settings` is what the settings pane reads and writes: every setting with its doc and layers, and a change written into a settings file ([settings](settings.md#the-settings-pane)).
- `kawoosh.on_settings(fn)` calls `fn()` once when you register it and again whenever the settings change (a settings file saved, `:set`, `kawoosh.opt`). Read your settings there, and the same code handles startup and later changes.

## Example: reading a setting

```lua
-- init.lua
kawoosh.setting("greet.name", { type = "string", doc = "who :greet greets" })

local name = "world"
kawoosh.on_settings(function()
  name = kawoosh.opt("greet.name") or "world"
end)

kawoosh.command("greet", function(ctx)
  kawoosh.notify("hello, " .. (ctx.args[1] or name))
end, { args = { "text" }, doc = "say hello" })
```

```lua
-- settings.lua
return {
  greet = { name = "Ada" },
}
```

## Buffers

`kawoosh.buf` reads and changes buffers. A buffer is named by a handle; leave it out to mean the current buffer. Lines count from 1; offsets are bytes from 0, with the end not included.

- Reading: `current()`, `list()`, `name()`, `path()`, `language()`, `indent()` (`tabstop`, `shiftwidth`, `expandtab` and `unit`, one indent's text, as the buffer's language and `.editorconfig` say), `comment_tokens()` (`line`, the token `gc` writes, and `block`, the pair, each absent where the language has none), `modified()`, `text()`, `lines()`, `line(n)`, `line_count()`, `lines_in(from, to)`, `cursor()` (the caret's `offset`, `line`, `col`), `selections()`.
- Changing: `insert(offset, text)`, `replace(from, to, text)`, `set_text(text)`, `edits({ { from, to, text }, ... }[, buffer][, { carets = { ... } }])` (several edits as one undo step; `carets` places the selections after them — `{ edit = i, at = k }` `k` bytes into edit `i`'s text, `{ at = o }` an offset of the text before, moved by the edits, one of them `primary = true`), `type(text)` (typed at every caret), `set_cursor(offset, h, { top =, center =, jump = })` (`jump = true`: the move goes on the tab's [jumps](editing.md#jumps) however near), `set_selections(...)`.
- Showing: `show(buffer)` puts a buffer in the focused pane, `close(buffer)` closes it as `:bd` does. `header({ view =, height =, field = }[, buffer])` draws your view over the buffer's text in every pane that shows it, one pane with the view's fields on top — the project search's bar over its results: the keys are on the view's field while one is focused, on the text otherwise, `<C-S-j>` and `<C-S-k>` moving them down and up (to `field` the first time); `nil` takes it off.
- `open_scratch { name = ..., text = ..., on_write = fn, read_only = ..., language = ... }` makes a buffer that is not a file. With `on_write(lines)`, `:w` hands you its lines. A session brings it back by its name for `kawoosh.on_restore` to fill — with `on_write`, or `restore = true` for a page that writes nothing.
- `annotate(notes)` draws dim text after lines, and `paint(set, spans)` colours ranges over the syntax colours — a span's colour is a role (`accent`), a token (`keyword`, `link`), `ansi:N` or `#rrggbb`, with `bold`, `italic`, `underline` or `strike` before it (`"bold keyword"`) or alone (`"underline"`) to set the text's style too. `annotate(notes, buffer, { align = true })` makes the buffer's notes a column: each starts past the buffer's widest line, as a listing's sizes do.

`kawoosh.open(path, { line = ..., col = ..., split = "vsplit" | "split" | "tab" })` opens a file in an editor pane. `kawoosh.pane()` is the pane the keyboard was in when the command at hand ran (a command's `ctx.pane`), and `kawoosh.pane_size(pane)` an editor pane's text column as last drawn — `width`, `height` in px, `cols`, `rows` in cells — for what renders to fit it.

## The syntax tree

`kawoosh.node` reads the buffer's syntax tree, the one the colours come from. A node is a plain table: `type` (the grammar's name for it, such as `call_expression`), `named` (false for a token such as `(` or `==`), `field` (the field it fills in its parent, such as `condition`), `from` and `to` (bytes, as in `kawoosh.buf`), `line` and `end_line`, `error`, `has_error` and `language`. `:syntax_tree` shows the names a language uses, for the node under the caret.

- Getting one: `at(where)` is the smallest named node, `leaf(where)` the smallest node, a token included, and `root()` the whole tree. `where` is an offset, `{ from, to }`, or nothing for the selection; a buffer handle can follow. When there is no tree, or it has not caught up with the last keystroke yet, they return `nil` and the reason.
- Walking: `n:parent()`, `n:children()`, `n:child(i)` (from 1, `-1` the last), `n:get(field)`, `n:next()`, `n:prev()`, and `n:closest(types)`, which is the node itself or the nearest one around it of a type (a name, or a list of names). Tokens are skipped unless you pass `{ anonymous = true }`. `n:text()` reads its text, and `n:select()` selects it in visual mode.
- Queries: `kawoosh.node.query(source, where)` runs a tree-sitter query over a node or a whole buffer (`n:query(source)` over `n`) and returns the matches, each with `captures.NAME` (the first node) and `all.NAME` (every node).

A node belongs to the text it was read from. After an edit, read it again: using an old one is an error. Changes are made once your code returns, so make every edit from the same tree in one `kawoosh.buf.edits` call:

```lua
local flip = { ["true"] = "false", ["false"] = "true" }
kawoosh.command("flip", function()
  local edits = {}
  for _, s in ipairs(kawoosh.buf.selections()) do
    local n = kawoosh.node.leaf(s.head)
    if n and flip[n:text()] then edits[#edits + 1] = { n.from, n.to, flip[n:text()] } end
  end
  kawoosh.buf.edits(edits)
end, { when = { "editor", "!readonly" }, doc = "flip the boolean under every caret" })
```

`g.` does this for you, from actions you can add to. `kawoosh.node.action(name, { types = { … }, languages = "*" | { … }, run = fn(n, ctx) })` adds one: `types` are the node types it takes (a token such as `==` is a type too), or a function of a type saying whether it takes it, and `run` answers the node's new text, `{ text = …, cursor = i }` to put the caret `i` bytes into it, or `nil` when this node is not one it changes, and then the next action, or the next node up, is asked. `run` only reads; `g.` makes the edits, at every caret. `ctx` has `language`, `caret`, `indent` (the node's line's leading whitespace) and `unit` (one indent). The same name again replaces an action — the shipped ones are `flip`, `operator`, `split`, `quotes` and `digits` — and `nil` removes it. The newest action is asked first. `kawoosh.node_actions.lists` holds `split`'s lists by language, to add to.

`kawoosh.on_tree(fn)` calls `fn(root, changed)` when a buffer's tree is parsed again, for a plugin that colours from the tree: `root` is the buffer's root node (`root.buffer`, `root.language`), and `changed` is a list of `{ from, to }` byte ranges whose syntax changed since your hook last heard of that buffer. The first time, and whenever a new hook is added, that is the whole text. It runs at most once per buffer per frame, only for a tree that has caught up with the text, and only for buffers on screen. It runs while the frame is drawn, so look at the ranges that changed rather than walking the whole tree. It returns a function that removes the hook:

```lua
local off = kawoosh.on_tree(function(root, changed)
  if root.language ~= "rust" then return end
  for _, r in ipairs(changed) do
    -- the node over what changed, and the macros in it
    local n = kawoosh.node.at({ r.from, r.to }, root.buffer)
    for _, m in ipairs(n and n:query("(macro_invocation) @m") or {}) do
      -- paint m.captures.m.from .. m.captures.m.to
    end
  end
end)
-- later: off()
```

## Diagnostics

A plugin can report problems the way a language server does: a linter you run, a spell checker, your project's own check. `kawoosh.diagnostics.set(buffer, name, list)` sets what `name` says about a buffer (a handle, or `0` for the current one) or about a file (a path). It replaces what `name` said there before, and leaves the servers' and other plugins' alone. An empty list takes yours back, and `kawoosh.diagnostics.clear(name)` takes back everything `name` said, everywhere.

```lua
kawoosh.diagnostics.set(0, "todo", {
  { line = 3, col = 5, end_col = 9, severity = "info", message = "a TODO left" },
})
```

Each item has `line` and `col` (from 1, the column in characters), optionally `end_line` and `end_col` (one character unless you say), `severity` (`"error"`, `"warning"`, `"info"`, `"hint"` or 1–4; an error unless you say), `message`, `source` (your `name` unless you say) and `code`. They are underlined, shown at the end of the row, walked by `]d`, shown by `<C-e>`, listed by `:diagnostics` and counted by `kawoosh.lsp.counts()`, beside the servers'. They move with the text as you edit, until you set them again. A file's that no buffer has open yet appear in the list, and the buffer that opens it takes them; their columns are characters there too. A line or column is a whole number from 1; anything else is an error. A line the text does not have (it moved since your linter read it) is not shown.

Setting the same list again changes nothing and wakes no `on_diagnostics`, so a linter can publish on every keystroke or reparse. While you are typing in a buffer, a new list for it waits, as a server's does, until the typing pauses or you leave insert mode; `get` reads what has landed.

`kawoosh.diagnostics.get { buffer =, root =, severity =, from = }` reads every diagnostic, or a buffer's, those under a folder, those at least as bad as a severity, or one publisher's: each row has the fields above, plus `path`, `buffer`, `level` and `from` (`"lsp"` for the servers, otherwise the plugin's name). A row read back can be set again as it is. `buffer = 0` with no buffer open reads none. A server's row for a file no buffer holds has the column as the server counted it (UTF-16 units, which differ from characters only past the Basic Multilingual Plane). `kawoosh.lsp.diagnostics` is the same function.

## Language servers

`kawoosh.lsp.server(name, t)` adds or changes a server, with the keys of `lsp.NAME` ([code](code.md#settings-per-server)).

`kawoosh.lsp.rule(name, { doc = ..., default = false })` adds a rule your plugin reads, set like kawoosh's own: `lsp.NAME.RULE` for one server, in your settings, a project's or the session's, or `lsp.RULE` for every server. A rule that is on or off can be flipped with `:lsp toggle RULE`, and `:lsp info` shows where it is set. `kawoosh.lsp.rules(buffer)` (or a language's name) reads a server's rules as they stand now: `server` (the `lsp.NAME` they are set under; a `.tsx` file's is `typescript`), `enabled`, `load_all`, `load_max`, `inlay_hints` and every plugin's rule. A rule can't take a server's name (`rust`, one from `kawoosh.lsp.server`, or an `lsp.NAME` with a `cmd`), and a server can't take a rule's.

```lua
kawoosh.lsp.rule("organize_on_save", { doc = "organize imports when a file is saved" })
kawoosh.on_write(function(path, buffer)
  if kawoosh.lsp.rules(buffer).organize_on_save then
    -- ask the server for its "source.organizeImports" action
  end
end)
```

```lua
-- .kawoosh/settings.lua
return { lsp = { typescript = { organize_on_save = true } } }
```

## Formatters

`kawoosh.formatter(name, def)` adds a formatter ([code](code.md#formatting)): `def` has the keys of `format.NAME` (`cmd`, `args`, `languages`, `when`, …), and your settings file still overrides them. For a tool that does not read stdin and write stdout, or one that is slow, give `run` instead of `cmd`:

```lua
kawoosh.formatter("pg_format", {
  languages = { "sql" },
  timeout_ms = 20000,
  run = function(ctx, text, done)
    local out = {}
    kawoosh.spawn("pg_format -", {
      cwd = ctx.cwd,
      stdin = text,
      on_lines = function(lines) for _, l in ipairs(lines) do out[#out + 1] = l end end,
      on_exit = function(code)
        if code == 0 then done(table.concat(out, "\n") .. "\n") else done(nil, out[1] or "failed") end
      end,
    })
  end,
})
```

`run(ctx, text, done)` gets the buffer's text. It calls `done(text)` with the formatted text, or `done(nil, why)`, whenever it is ready. A quick one can `return` the text instead. `ctx` has `path`, `language`, `buffer`, `cwd`, and `from` and `to` (bytes, from 0) when formatting a selection. The formatter's `timeout_ms` (5 seconds unless it says) applies either way.

`kawoosh.format(buffer, { with = "name" })` formats a buffer (the current one when `buffer` is nil) with its formatter or the one named.

## Compile commands

`kawoosh.compile_kind(name, def)` teaches the compile picker and a bare `:compile` ([code](code.md#compile-commands)) a kind of project, as `Cargo.toml` and `package.json` are taught already. `markers` are the files that say a directory is that kind's, found at the nearest one above the current file (`outermost = true` for the outermost in the repository); `runner = true` ranks it with `just` and `make`, ahead of the rest; a command typed with one of its `programs` runs where its file is. `commands` is a list — strings, or `{ cmd, why, needs, detail }` — or a function of `{ file, dir, text }` that answers one, called each time the project is read:

```lua
kawoosh.compile_kind("mix", {
  markers = { "mix.exs" },
  programs = { "mix" },
  commands = function(ctx)
    local rows = { "mix compile", { cmd = "mix test", why = "the tests" } }
    if ctx.text:find(":phoenix") then rows[#rows + 1] = "mix phx.server" end
    return rows
  end,
})
```

A builtin kind's name — `cargo`, `node`, `just`, `nu`, `make`, `cmake`, `go`, `python`, `zig` — puts yours in its place, and `kawoosh.compile_kind("make", false)` turns that kind off.

## Version control

A buffer's hunks ([vcs](vcs.md)) are the difference between its text and a *base* — the editor diffs them; a backend only says what the base is. `kawoosh.buf.base(text, label[, buffer[, { head = }]])` gives a buffer its base (`kawoosh.buf.base(nil)` takes it away; `head`, the text the base is itself read against — HEAD's under the index — makes what is staged), `kawoosh.buf.hunks([buffer])` reads the hunks back once diffed — each `{ kind = "added" | "modified" | "deleted", line, end_line, old_line, old_end, old = { … } }`, lines from 1, ends exclusive — and `kawoosh.diff(old, new)` diffs two texts at once, the same shape. `kawoosh.buf.blame(rows[, buffer])` puts the blame column on from rows `{ line, count, label, rev, summary }`; `kawoosh.buf.blame_at(line)` reads one back.

Another version control system is a table of functions, and what it lacks it does not have:

```lua
kawoosh.vcs.register("jj", {
  probe = function(dir, done) … end,            -- done(root) or done(nil)
  head = function(root, done) … end,            -- done{ branch =, rev = }
  base = function(root, path, rev, done) … end, -- done(text) or done(nil, why); rev nil = the index
  status = function(root, done) … end,          -- done{ { path =, state = }, … }
  blame = function(root, path, text, done) … end,
  log = function(root, path, done) … end,
  show = function(root, rev, done) … end,
  stage = function(root, path, patch, done) … end, -- done(true) or done(false, why)
  -- changed, merge_base, refs, worktrees, worktree_add, watch: the rest
})
```

`probe` and `base` alone colour the gutter. `stage` takes the patch `hunk stage` and `hunk unstage` made against the index — the `@@` sections alone, three lines of context, the file named by `path` — and applies it to the index (git's is `git apply --cached`); with it, `vcs.lua` fetches HEAD's text with the base, and the lines staged show. The bundled `vcs.lua` has git whole and fossil in part, as the worked examples. `kawoosh.vcs.of(dir)` says which backend owns a directory; `kawoosh.on_write(fn(path, buffer))` runs after a file is written, where a backend reads it again; `kawoosh.on_stage(fn(path, patch, opts))` is where the editor hands a patch to stage (`vcs.lua` sets it).

## Files and processes

`kawoosh.fs` works on paths as you would write them (`~/x`, `../y`), relative to the working directory, and on a remote host's paths too (see [remote](remote.md)). A failed operation raises an error naming the path.

- `read(path)`, `write(path, text)`, `exists`, `is_file`, `is_dir`, `stat`, `create(path, is_dir)`, `rename(a, b)`, `copy(a, b)`, `remove(path)`.
- `list(path)` lists a folder now; `list(path, fn)` reads it in the background and calls `fn(entries)`. `copy(a, b, fn)` and `remove(path, fn)` do the same for a change: made in the background, then `fn(true)`, or `fn(nil, why)`. `walk(root, fn)` lists every file under a folder as git sees it, in the background.
- Path helpers: `join`, `parent`, `basename`, `relative`, `expand`, `short`, `home`, `cwd`, `chdir`.
- `watch(name, paths, fn)` calls `fn(changed)` when the files or folders change; `watch(name, nil)` stops it.

`kawoosh.sqlite` reads and writes a SQLite file, in the background, for [the database pane](files.md#a-database) and for a plugin of your own. `query(path, sql[, params][, { cap = n, blob_cap = n }], fn)` runs the SQL — several statements in turn — and calls `fn(result)` with `{ columns = {…}, rows = { {…}, … }, truncated =, changes =, ms = }`, the rows the last statement's with columns, up to `cap` of them (1000); or `fn(nil, why)` with SQLite's words. A value comes as what it is: an integer, a float, a string, `kawoosh.sqlite.null` for NULL, `{ blob = bytes }` for a blob (past `blob_cap` bytes, `{ blob = its head, size =, cut = true }`, which cannot be bound back); `params` bind by position (`?1`, `?2`) in the same shapes. `schema(path, fn)` calls `fn({ tables = { { name =, kind = "table" | "view", rows =, without_rowid =, columns = { { name =, type =, notnull =, pk =, default = } } } }, bytes = })`. `is(path)` says whether the file's head is SQLite's; `quote(name)` quotes an identifier.

`kawoosh.status(name, fn, { place = "title" | "tabs" | "statusline", order = n, every = seconds, run = "command" })` adds a segment to the right of the title bar or the tab strip, or a module to [the status line](#the-status-line): `fn(ctx)` returns what it shows now — `nil` to hide, a string, `{ text = ..., color = "dim" }`, or a list of those — and is asked each time the window draws; `every` redraws it on the clock (a clock of your own); `run` is what a click runs. `kawoosh.status(name, nil)` removes it. `kawoosh.lsp.counts()` gives the diagnostics' `errors`, `warnings`, `infos` and `hints` cheaply, for such a segment.

## The status line

The line under the panes is modules, placed left to right by one list:

```lua
statusline = {
  layout = { "mode", "recording", "path", "keys", "gap",
             "...", "strip", "selections", "position", "percent" },
}
```

That is the default. kawoosh's modules are `mode`, `recording` (`REC @a`), `path` (the file, as `statusline.path` says, `[+]` when modified), `keys` (typed so far), `strip` (a scrolling tab's columns), `selections`, `position` (`line:col`) and `percent`. `"gap"` is a spring that pushes what follows it right; several share the room evenly, so `{ "mode", "gap", "...", "gap", "percent" }` centres the middle. A `kawoosh.status(name, fn, { place = "statusline" })` segment is a module too: named in the layout it is drawn there, otherwise where `"..."` is — leave `"..."` out to draw only what the layout names. Quote it: a bare `...` is Lua's varargs and disappears. A segment named like a built-in module replaces it. The path is cut from the left, a directory at a time, until it fits what the other modules leave.

`kawoosh.launcher.module(name, def)` adds a module to [the launcher](panes.md#its-layout), the same name replacing one: rows — `items = fn(ctx)` returning `{ text =, sub =, run = "command" | pick = fn(item) | path = | buffer = }`, `load = fn(ctx, done)`, or `source = "<picker source>"` — or a block, `draw = fn(ctx)` returning a node (`text(...)`, `row { ... }`), shown while the query is empty. `ctx` has `origin` (the buffer split from), `cwd`, `query`, `theme` and `size`; a block sizes itself with kui sizes (`width = "50%"`). Fields: `title`, `limit`, `show`, `style`, `keys`, as in the layout. A module you add appears where the layout says `"..."` until you place it. `kawoosh.launcher.entry { text =, run =, module = "here", key = "x" }` adds one row to a module (`plugins` by default).

`kawoosh.spawn(cmd, { cwd = ..., env = { NAME = "value" }, on_lines = fn, on_exit = fn })` runs a shell command — `env` the variables it has over the inherited ones — and hands you its output as it comes; `kawoosh.kill(token)` stops it. `cmd` as a list — `{ "git", "status" }` — runs the program with those arguments and no shell between, so nothing needs quoting; `on_done = fn(text, code)` gets the output whole when it ends, and `on_stderr = fn(lines)` takes stderr apart. A plugin's processes run 32 at a time; the rest wait their turn in order, and one killed before it started ends with no code. `kawoosh.store(name)` is a small store kept between runs: `get(key)`, `set(key, value)`, `del(key)`, `keys()`. `kawoosh.tool(name, { cmd = ... })` adds a launch target for `:tool` and the tools picker (`<leader>t`). `:tool NAME` goes to the tool's terminal in this tab, or starts one here: each tab runs its own. With `dock = true` the tool lives in the dock, one for every tab, and `:tool NAME` shows or hides it.

## Talking to the user

- `kawoosh.echo(text)` puts a line on the message line.
- `kawoosh.notify(text, opts)` shows a notification. `opts` is a level (`"debug"`, `"info"`, `"warn"`, `"error"`) or a table with `level`, `source`, `show` (`"toast"`, `"corner"`, `"log"`), `timeout` (in milliseconds) and `actions`, a list of `{ label = "Retry", run = fn }`. A toast with actions stays until one is taken; given a `timeout` too, it is an offer that goes when its time is up, and a click or `x` puts it away. Every notification is kept in `:messages`.
- `kawoosh.confirm { title = ..., lines = { ... }, actions = { ... } }` asks a question before anything happens. The first action is what `<CR>` and `y` take; `<Esc>`, `n` and `q` answer no.
- `kawoosh.cmdline(text)` opens the command line with text on it, and `kawoosh.copy(text)` puts text on the clipboard.

## Hooks

- `kawoosh.on_open(fn)`: `fn(path)` for every path about to be opened. Return `true` to take it; the file manager takes folders this way.
- `kawoosh.on_settings(fn)`: when the settings change, as above.
- `kawoosh.on_cwd(fn)`: `fn(path, how)` when the working directory changes.
- `kawoosh.on_focus(fn)`: `fn(buffer)` when the keys go to an editor pane on another buffer.
- `kawoosh.on_diagnostics(fn)`: after the diagnostics change; read them with `kawoosh.diagnostics.get`.
- `kawoosh.on_tree(fn)`: `fn(root, changed)` when a buffer's syntax tree is parsed again ([above](#the-syntax-tree)). It returns a function that removes the hook.
- `kawoosh.on_restore(fn)`: `fn(name, buffer)` for each scratch buffer a session brings back, so you can fill it again.
- `kawoosh.tab_title(fn)`: `fn(tab)` returns the label for each tab in the tab strip, or `nil` for kawoosh's own. It runs every frame, so keep it cheap.

## Panes of your own

A plugin pane is a function from your state to a tree of UI nodes, drawn every frame it is on screen.

- `kawoosh.view(name, fn, on_event, opts)` declares a view. `fn(ctx)` returns the tree: `column`, `row`, `text` and the rest of kui's nodes. `ctx` has `width`, `height`, `focused`, and `env.theme` for the theme's colours. `opts.session = false` keeps the view out of saved sessions. `opts.here(pane)` returns the directory the pane shows, if any: `<C-w>.` (`:terminal here`) starts a terminal there, as `:du` does.
- Clicks come back to `on_event(ev)`: the table you gave as a node's `on_click`, as `ev`. A key pressed in the pane arrives as `{ kind = "key", key = "j" }`; return `true` to keep it. Keys you do not keep work as in any other pane: `<C-w>` moves, `:`, `<leader>`.
- `kawoosh.view_open(name, { below = true, share = 0.3, focus = false })` shows the view in a column of its own, or under the focused pane with `below` ([panes](panes.md#where-a-pane-opens)), `kawoosh.view_close(name)` closes it, and `kawoosh.view_toggle(name, opts)` does one or the other. `:view NAME` opens one from the command line.
- Pane-mode maps (`"p"`) with `view = "NAME"` apply only while your view has the keys, and a field's with `view` and `field`. Prefer them to handling keys in `on_event`: they show up in `:map list` (with `in lua:NAME`), take counts, and can be remapped.
- `ctx.field { name = "q", placeholder = "find" }` puts a one-line input in the tree. It is a line of the editor, with its modes and motions. `kawoosh.field_focus(view, name)` gives it the keys, and `kawoosh.field_text` / `field_set` read and write it.
- `ctx.metrics` is the panes' one scale, the sizes every pane's text is drawn at, so your pane matches the others and follows `font.chrome_size`: `text` (a pane's text: its rows, its fields), `small` (a step under: secondary text, a chip), `note` (two under: a note, a count, a key legend), `row` (a line of `text` with the chrome's air) and `font` (the editor's font size). `kawoosh.metrics(env)` is the same from anywhere you have kui's `env`. The fields, icons, caps and legends below are drawn at these sizes unless you give one.
- `ctx.icon("close", { size = 13, color = t.muted })` puts an icon in the tree, drawn with strokes in a box as wide as the text beside it is large, so it sits in the line's middle where a glyph like `×` would sit low. `kawoosh.icon(name, opts)` is the same outside a view; any other key of `opts` (`key`, `on_click`, `hover_bg`) is the box's. The set: `close` `check` `dot` `chevron-right` `chevron-left` `chevron-up` `chevron-down` `folded` `unfolded` `arrow-up` `arrow-down` `arrow-left` `arrow-right` `return` `tab` `backspace` `delete` `ctrl` `alt` `shift` `cmd` (`kawoosh.icon_names()` lists them).
- `ctx.keys("<C-w>j", { size = 12 })` draws keys as caps, one outlined cap a key, written in map notation: `<CR>` is the return arrow, `<A-/>` {{mac:the option sign and a `/`}}{{pc:reads `alt+/`}}. `ctx.legend({ { "<CR>", "opens" }, { { "j", "k" }, "walk" } }, { size = 12 })` is a key legend: each item its keys and its words, wrapped between items at the pane's width. It is compact or whole as the pane's legend is: compact it draws nothing in your view (it returns `nil`) and the pane's title bar ends in `⌥/ keys`; `<A-/>`, or a click on that, opens it, and your view draws its items while the title bar says `⌥/ hide keys`. `full = true` keeps one whole always (an edit's two keys), with no hint; `toggle = false` keeps the hint out of the title bar, for a view that draws it itself with `ctx.legend_toggle({ size = 12 })` (the search bar's, at the end of its stages). `ctx.legend_full()` says which it is.

## Example: a tiny pane

```lua
local count = 0

kawoosh.view("counter", function(ctx)
  local t = ctx.env.theme
  return column { pad = 12, gap = 8,
    text("count: " .. count, { color = t.fg }),
    row { pad_x = 8, pad_y = 2, bg = t.surface, radius = 4,
          on_click = { kind = "bump" },
          text("bump", { color = t.accent }) },
  }
end, function(ev)
  if ev.kind == "bump" then count = count + 1 end
end)

kawoosh.map("p", "+", function() count = count + 1 end, { view = "counter" })

kawoosh.command("counter", function()
  kawoosh.view_toggle("counter", { below = true, share = 0.3 })
end, { doc = "show or hide the counter" })
```

`:counter` opens it below; click `bump` or press `+`; `q` closes it.

## Icons of your own

`kawoosh.icons.NAME` is an icon's shape, a list of parts in a box of side 1 (`y` down), and setting it changes that icon everywhere — the tab's close button too. `= nil` puts kawoosh's back.

```lua
kawoosh.icons.close = {                       -- a heavier ×
  { stroke = { { 0.25, 0.25 }, { 0.75, 0.75 } }, width = 0.14 },
  { stroke = { { 0.25, 0.75 }, { 0.75, 0.25 } }, width = 0.14 },
}
kawoosh.icons.check = { { glyph = "\u{F012C}" } }   -- a Nerd Font's
```

A part is `{ stroke = points, width = 0.1, curve = false }` (a line through the points, `width` a share of the box), `{ fill = points }` (a filled outline of three to eight points), `{ dot = r }` (a disc in the middle) or `{ glyph = "…", scale = 1 }` (a character, where its font puts it).

## Native extensions

A shared library in C, or anything that speaks C, can do what a Lua plugin does — commands, keys, hooks, panes — by the same names, with its own computation, its own threads and its own drawing.

```lua
kawoosh.extension("dupes")   -- in init.lua
```

loads `ext/dupes.dylib` (macOS), `ext/dupes.so` (Linux, or anywhere) or `ext/dupes.dll` (Windows) from your config folder (`kawoosh.fs.config()` says where). A second argument names the library instead: the file, a folder holding it, or its path without the extension. `:extensions` lists what is loaded. A library built for another ABI is refused, with both numbers, and nothing of it runs.

- In C, `kawoosh.buf.lines(...)` is `kw_call(ctx, KUI_STR("buf.lines"), args)`, the arguments a list, and the header's shorthand makes a call one line: `kw_do(ctx, "echo", kw_str("hi"))`. Where Lua takes a function, pass `kw_fn(ctx, fn, user)`. A thread of yours comes back with `kw_wake(fn, user)`. `kawoosh.h` documents every function, and who frees what.
- What to build against is the extension pack for your platform: the headers (`kawoosh.h`, and the `kui.h` of the kui this Kawoosh is built with), an example, and on Windows `kawoosh.lib`. `nu scripts/pack.nu` in a kawoosh checkout writes one for each platform into `target/pack/`; the Windows `Kawoosh` folder carries the same, in `include\` and `kawoosh.lib`. From the pack's folder:
  - macOS: `cc -O2 -shared -undefined dynamic_lookup -I include example/dupes.c -o dupes.dylib`
  - Linux: `cc -O2 -shared -fPIC -I include example/dupes.c -o dupes.so`
  - Windows: `clang -O2 -shared -I include example\dupes.c lib\kawoosh.lib -o dupes.dll` (an MSVC-ABI compiler; a UCRT MinGW `gcc` works too). A DLL linked so loads into `kawoosh.exe` and no other program.
- A library is never unloaded: build it again and `:relaunch`. Windows will not write over a DLL in use, so move the old one aside first.
- A project's `.kawoosh/init.lua` loads one only once you trust it, as with the rest of its code. A crash in the library is a crash of Kawoosh.

## Where a frame's time goes

The developer tools panel (F12) has tabs that measure kawoosh as it runs. `:perf` opens the Perf tab: what a frame spends where — by phase, by system and by plugin, a plugin's Lua time its own row — what the systems report of their threads, and what the process holds. `:frames` opens the Frames tab: why each frame was drawn — a key, a thread's wake, a frame asked for by the one before — and the *burns*, frames drawn one after another with no input between them, summed up when they end. Each measures only while it is on show; closed, nothing is timed. `KAWOOSH_FRAME_LOG=PATH` in kawoosh's environment appends every burn to PATH, frame by frame, with no tab open. `:syntax_tree` is the third tab ([code](code.md#the-syntax-tree)); each command again closes the panel.

## Testing a plugin

`kawoosh test FILE.lua ...` runs Lua scripts against a headless editor, with the bundled plugins loaded but not your `init.lua`. The script presses keys and reads the state back, and it exits with status 0 when every script passes.

```lua
-- today_test.lua
dofile("today.lua")  -- relative to where you run `kawoosh test`
kawoosh.press("<leader>id")
kawoosh.test.eq(kawoosh.buf.line(1), os.date("%Y-%m-%d"), "the date")
```

- `kawoosh.press(keys)` presses keys in map notation; `kawoosh.frame(n)` draws frames; `kawoosh.wait(fn)` draws frames until `fn()` is true (for background work).
- `kawoosh.test.eq(got, want, what)`, `kawoosh.test.ok(cond, what)` and `kawoosh.test.has(list, item, what)` fail the script with its line. A plain `assert` or `error` works too.
- `kawoosh.buf.*`, `kawoosh.mode()` and `kawoosh.message()` read what the editor did.

## The full reference

This page covers the main parts. For everything else:

- Your Lua language server knows the whole API. At startup kawoosh writes `kawoosh.lua` (the `kawoosh` table), `kui.lua` (the view nodes) and `settings.lua` (every declared setting) to a folder of its own under `types` beside its state database (`~/.local/share/kawoosh/types/kawoosh-…`, one per kawoosh executable; `KAWOOSH_TYPES` names one instead), and adds that folder to lua-language-server's library, so `kawoosh.` and `row {` complete in `init.lua`, in plugins and in settings files.
- In the source, every function is described in a comment above it in `lua/lua/boot.lua` and `lua/src/lib.rs`, and the bundled plugins in `kawoosh/lua/` are working examples.

See also: [settings](settings.md), [commands](commands.md), [keys](keys.md).
