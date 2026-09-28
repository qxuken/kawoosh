# Lua

How to extend kawoosh in Lua: where your code goes, the `kawoosh` table and its main parts, panes of your own, and testing a plugin. The bundled plugins (the file manager, the picker, the project search, the themes pane and more) are written against this same API, so anything they do, your code can do too.

## Where your code goes

- `~/.config/kawoosh/init.lua` is yours. It runs at startup, after your `settings.lua` has been read, so it can read your settings. Saving it runs it again, and anything it set with `kawoosh.opt` last time is removed first. `$XDG_CONFIG_HOME` moves the folder; `KAWOOSH_INIT` names another file.
- `.kawoosh/init.lua` in a project (in the working directory or any folder above it) is the project's. It is code from a repository, so kawoosh asks before running it: the first time it sees the file it shows you the file's lines and offers to trust and run it. A trusted file runs without asking again until its text changes. `:trust` trusts and runs the working directory's files, `:trust?` lists them and where each one stands, and `:trust revoke` forgets what you trusted. While a project's `init.lua` runs, `kawoosh.project` holds `{ root = ..., dir = ... }`: the project folder and its `.kawoosh` folder.
- `settings.lua` (yours, and a project's `.kawoosh/settings.lua`) is not code. It returns a table and runs with no access to files, processes or `kawoosh`, so a project's settings never need trust. See [settings](settings.md).

Try things without a file: `:lua CODE` runs a line of Lua, and `<leader>x` runs the line under the caret (the selection, in visual mode) and shows the result.

## Commands

Everything a key does is a named command. `kawoosh.command(name, fn, opts)` adds one, and it can be run from the command line (`:name`), a key, or other code.

- A name of two words is a subcommand: `"notes open"` runs as `:notes open` and completes under `:notes`.
- `fn(ctx)` gets `ctx.args` (the words after the name), `ctx.count`, and `ctx.bang` / `ctx.query` for `:name!` and `:name?`.
- `opts.args` says what the arguments are, one kind per position: `"path"`, `"buffer"`, `"command"`, `"option"`, `"tool"`, `"view"` or `"text"`, the last one written with `"..."` for the rest. The command line completes each kind, and a `"path"` reaches `fn` already made absolute.
- `opts.when` lists facts that must hold for the command to run, such as `"editor"` (an editor pane has the keys), `"!readonly"`, `"visual"`, `"language:rust"` or `"terminal"`. The command line refuses with the reason when one does not hold. `kawoosh.fact(name, on)` publishes a fact of your own.
- `opts.doc` is one line on what it does, shown in the command palette (`<leader>ic`); `opts.aliases` gives other spellings, and `opts.bang` / `opts.query` describe what `!` and `?` mean.

`kawoosh.cmd(line)` runs a command line right away; `kawoosh.run(line)` runs it after what was already asked for. `kawoosh.commands()` lists every command, and `kawoosh.can(name)` says whether one can run now, or why not.

## Keys

`kawoosh.map(mode, keys, cmd, opts)` binds keys. `mode` is `"n"`, `"i"`, `"v"`, `"o"` (operator pending) or `"p"` (pane mode: panes that are not editors, such as your own views). `keys` is written in vim notation (`<leader>`, `<C-w>`, `<A-S-l>`). A chord's upper-case letter means Shift, so `<A-L>` is `<A-S-l>` and not `<A-l>`, which is how `:map list` shows it. `cmd` is a command line, as you would type it after `:`, or a function.

A key can be bound more than once: the newest binding whose `opts.when` holds, and whose command can run, is the one that runs. That is how one key does the right thing in different places. `kawoosh.unmap(mode, keys)` removes a key's bindings. Inside a command a key ran, `kawoosh.pass()` hands the key on to the binding under it.

`:map list` shows every binding in a pane (`:map list n`, or `:map list <leader>c` for the keys under a prefix), and `:map group <leader>i insert` names a group for the which-key card. `:map export PATH` writes the keymap and every command as JSON — each binding with the command it runs, the groups' names, the leader (bare, it opens in a pane).

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

- Reading: `current()`, `list()`, `name()`, `path()`, `language()`, `modified()`, `text()`, `lines()`, `line(n)`, `line_count()`, `lines_in(from, to)`, `cursor()` (the caret's `offset`, `line`, `col`), `selections()`.
- Changing: `insert(offset, text)`, `replace(from, to, text)`, `set_text(text)`, `edits({ { from, to, text }, ... })` (several edits as one undo step), `type(text)` (typed at every caret), `set_cursor(offset)`, `set_selections(...)`.
- Showing: `show(buffer)` puts a buffer in the focused pane, `close(buffer)` closes it as `:bd` does.
- `open_scratch { name = ..., text = ..., on_write = fn, read_only = ..., language = ... }` makes a buffer that is not a file. With `on_write(lines)`, `:w` hands you its lines.
- `annotate(notes)` draws dim text after lines, and `paint(set, spans)` colours ranges over the syntax colours.

`kawoosh.open(path, { line = ..., col = ..., split = "vsplit" | "split" | "tab" })` opens a file in an editor pane.

## Files and processes

`kawoosh.fs` works on paths as you would write them (`~/x`, `../y`), relative to the working directory, and on a remote host's paths too (see [remote](remote.md)). A failed operation raises an error naming the path.

- `read(path)`, `write(path, text)`, `exists`, `is_file`, `is_dir`, `stat`, `create(path, is_dir)`, `rename(a, b)`, `copy(a, b)`, `remove(path)`.
- `list(path)` lists a folder now; `list(path, fn)` reads it in the background and calls `fn(entries)`. `copy(a, b, fn)` and `remove(path, fn)` do the same for a change: made in the background, then `fn(true)`, or `fn(nil, why)`. `walk(root, fn)` lists every file under a folder as git sees it, in the background.
- Path helpers: `join`, `parent`, `basename`, `relative`, `expand`, `short`, `home`, `cwd`, `chdir`.
- `watch(name, paths, fn)` calls `fn(changed)` when the files or folders change; `watch(name, nil)` stops it.

`kawoosh.status(name, fn, { place = "title" | "tabs", order = n, every = seconds, run = "command" })` adds a segment to the right of the title bar or the tab strip: `fn(ctx)` returns what it shows now — `nil` to hide, a string, `{ text = ..., color = "dim" }`, or a list of those — and is asked each time the window draws; `every` redraws it on the clock (a clock of your own); `run` is what a click runs. `kawoosh.status(name, nil)` removes it. `kawoosh.lsp.counts()` gives the diagnostics' `errors`, `warnings`, `infos` and `hints` cheaply, for such a segment.

`kawoosh.size(spec, room)` works out a [size](settings.md#sizes) — `720`, `"80%"`, `"clamp(400px, 80%, 1000px)"` — in pixels of `room`, or gives `nil` and why for a spelling that is not one.

`kawoosh.launcher.module(name, def)` adds a module to [the launcher](panes.md#its-layout), the same name replacing one: rows — `items = fn(ctx)` returning `{ text =, sub =, run = "command" | pick = fn(item) | path = | buffer = }`, `load = fn(ctx, done)`, or `source = "<picker source>"` — or a block, `draw = fn(ctx)` returning a node (`text(...)`, `row { ... }`), shown while the query is empty. `ctx` has `origin` (the buffer split from), `cwd`, `query`, `theme`, `size` and `width`. Fields: `title`, `limit`, `show`, `style`, `keys`, as in the layout. A module you add appears where the layout says `"..."` until you place it. `kawoosh.launcher.entry { text =, run =, module = "here", key = "x" }` adds one row to a module (`plugins` by default).

`kawoosh.spawn(cmd, { cwd = ..., on_lines = fn, on_exit = fn })` runs a shell command and hands you its output as it comes; `kawoosh.kill(token)` stops it. `kawoosh.store(name)` is a small store kept between runs: `get(key)`, `set(key, value)`, `del(key)`, `keys()`. `kawoosh.tool(name, { cmd = ... })` adds a launch target for `:tool` and the tools picker (`<leader>t`). `:tool NAME` goes to the tool's terminal in this tab, or starts one here: each tab runs its own. With `dock = true` the tool lives in the dock, one for every tab, and `:tool NAME` shows or hides it.

## Talking to the user

- `kawoosh.echo(text)` puts a line on the message line.
- `kawoosh.notify(text, opts)` shows a notification. `opts` is a level (`"debug"`, `"info"`, `"warn"`, `"error"`) or a table with `level`, `source`, `timeout` and `actions`, a list of `{ label = "Retry", run = fn }`. Every notification is kept in `:messages`.
- `kawoosh.confirm { title = ..., lines = { ... }, actions = { ... } }` asks a question before anything happens. The first action is what `<CR>` and `y` take; `<Esc>`, `n` and `q` answer no.
- `kawoosh.cmdline(text)` opens the command line with text on it, and `kawoosh.copy(text)` puts text on the clipboard.

## Hooks

- `kawoosh.on_open(fn)`: `fn(path)` for every path about to be opened. Return `true` to take it; the file manager takes folders this way.
- `kawoosh.on_settings(fn)`: when the settings change, as above.
- `kawoosh.on_cwd(fn)`: `fn(path, how)` when the working directory changes.
- `kawoosh.on_focus(fn)`: `fn(buffer)` when the keys go to an editor pane on another buffer.
- `kawoosh.on_diagnostics(fn)`: after the diagnostics change; read them with `kawoosh.lsp.diagnostics`.
- `kawoosh.on_restore(fn)`: `fn(name, buffer)` for each scratch buffer a session brings back, so you can fill it again.
- `kawoosh.tab_title(fn)`: `fn(tab)` returns the label for each tab in the tab strip, or `nil` for kawoosh's own. It runs every frame, so keep it cheap.

## Panes of your own

A plugin pane is a function from your state to a tree of UI nodes, drawn every frame it is on screen.

- `kawoosh.view(name, fn, on_event, opts)` declares a view. `fn(ctx)` returns the tree: `column`, `row`, `text` and the rest of kui's nodes. `ctx` has `width`, `height`, `focused`, and `env.theme` for the theme's colours. `opts.session = false` keeps the view out of saved sessions.
- Clicks come back to `on_event(ev)`: the table you gave as a node's `on_click`, as `ev`. A key pressed in the pane arrives as `{ kind = "key", key = "j" }`; return `true` to keep it. Keys you do not keep work as in any other pane: `<C-w>` moves, `:`, `<leader>`.
- `kawoosh.view_open(name, { below = true, share = 0.3, focus = false })` shows the view in a split, `kawoosh.view_close(name)` closes it, and `kawoosh.view_toggle(name, opts)` does one or the other. `:view NAME` opens one from the command line.
- Pane-mode maps (`"p"`) with `when = { "lua:NAME" }` apply only while your view has the keys. Prefer them to handling keys in `on_event`: they show up in `:map list`, take counts, and can be remapped.
- `ctx.field { name = "q", placeholder = "find" }` puts a one-line input in the tree. It is a line of the editor, with its modes and motions. `kawoosh.field_focus(view, name)` gives it the keys, and `kawoosh.field_text` / `field_set` read and write it.

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

kawoosh.map("p", "+", function() count = count + 1 end, { when = { "lua:counter" } })

kawoosh.command("counter", function()
  kawoosh.view_toggle("counter", { below = true, share = 0.3 })
end, { doc = "show or hide the counter" })
```

`:counter` opens it below; click `bump` or press `+`; `q` closes it.

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

- Your Lua language server knows the whole API. At startup kawoosh writes `kawoosh.lua` (the `kawoosh` table), `kui.lua` (the view nodes) and `settings.lua` (every declared setting) to a `types` folder beside its state database (`~/.local/share/kawoosh/types`, or `KAWOOSH_TYPES`), and adds that folder to lua-language-server's library, so `kawoosh.` and `row {` complete in `init.lua`, in plugins and in settings files.
- In the source, every function is described in a comment above it in `lua/lua/boot.lua` and `lua/src/lib.rs`, and the bundled plugins in `kawoosh/lua/` are working examples.

See also: [settings](settings.md), [commands](commands.md), [keys](keys.md).
