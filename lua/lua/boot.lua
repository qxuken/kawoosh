-- kawoosh's Lua bootstrap: the one kui extension the editor loads, and
-- the `kawoosh` API's Lua half. The Rust half seeds `kawoosh` with the
-- functions that cross the boundary (reads from a published snapshot,
-- writes as queued messages — kui.md D6, mvp.md D8); this file adds the
-- verbs built on them and the dispatch kui calls into.

slots = { "*" }

kawoosh = kawoosh or {}
kawoosh._views = {}
kawoosh._handlers = {}
kawoosh._commands = {}
kawoosh._writers = {}
kawoosh._changers = {}
kawoosh._restorers = {}
kawoosh._openers = {}
kawoosh._transient = {}
kawoosh._settings_hooks = {}
kawoosh._watches = {}
kawoosh._tools = {}
kawoosh._nonce = 0

-- kawoosh.command(name, fn[, opts]): a named command, callable from a
-- keymap, the command line, or Rust. A name of two words is a
-- subcommand (`"dir cd"` runs as `:dir cd`, completes under `:dir`).
-- `fn(ctx)` gets { count = n, args = {...}, form = "run" | "bang" |
-- "query", bang = bool, query = bool, pane = the id of the pane the
-- keyboard is in — a view's `ctx.pane` when it is one }. `opts`:
--   args    what the arguments are, one kind per position — "path",
--           "buffer", "command", "option", "tool", "view", "text" — the
--           last with "..." for the rest: a "path" reaches `fn` absolute
--           (`~`, `..`, the working directory resolved), and the command
--           line completes each kind.
--   aliases the ex spellings, `{ "o" }`.
--   bang    a line on what `!` means; without one `:name!` is refused.
--   query   the same for `?`.
--   when    facts that must hold, `{ "language:dir", "!terminal" }` —
--           the engine's `visual`, `modified`, `file`, `readonly`, `buffer:NAME`,
--           `language:NAME`, `field` (a one-line input has the keys:
--           the command line, a pane's query), `field:NAME`, `prompt`
--           (the command line or a search); the shell's `store`,
--           `lsp`, `editor`, `terminal`, `lua`, `dock`; or one a plugin
--           published with `kawoosh.fact`. The command line refuses
--           with the reason.
--   doc     one line on what it does.
-- `kawoosh.commands()` lists every command's spec as such a table;
-- `kawoosh.can(name)` is true, or the reason it cannot run now.
function kawoosh.command(name, fn, opts)
  kawoosh._commands[name] = fn
  kawoosh._register(name, opts)
end

-- kawoosh.language(name[, opts]) (Rust): a language of your own, or
-- a builtin's replaced — its files, and its grammar from a shared
-- library as `tree-sitter build` makes one. `opts`:
--   extensions, filenames, shebangs   what detects it: `{ "zig" }`,
--           `{ "build.zig.zon" }`, `{ "zig" }` for a `#!` line.
--   aliases other spellings — a fence's ` ```zg `.
--   path    the library, a directory holding it (a grammar's checkout
--           after `tree-sitter build`), or its path without the
--           extension; `~` is home. Nothing said: `parsers/<name>.<ext>`
--           under the config directory (`~/.config/kawoosh`), and
--           nothing there is a language of files alone.
--   symbol  when it is not `tree_sitter_<name>`.
--   highlights, injections   the query files. Nothing said:
--           `queries/<name>/*.scm` under the config directory, else the
--           checkout's `queries/`.
-- A path that leads nowhere, a symbol the library lacks, a grammar
-- built for another tree-sitter, a query that does not compile: a
-- warning, and the language goes in without colours. The newest
-- registration wins a file name or a spelling.
--
-- The place `opts` gives a map of its own (docs/design/local-maps.md),
-- as the fact that says where it holds; nil for a global map.
local function scope_of(opts)
  if type(opts) ~= "table" then return nil end
  if opts.scope then return opts.scope end
  if opts.field then
    if opts.view then return "field:lua:" .. opts.view .. "/" .. opts.field end
    return "field:" .. opts.field
  end
  if opts.view then return "lua:" .. opts.view end
  local b = opts.buffer
  if b == true then b = kawoosh.buf.current() end
  if type(b) == "number" then return string.format("buffer#%d", b) end
  if type(b) == "string" then return "buffer:" .. b end
  if opts.language then return "language:" .. opts.language end
  return nil
end

-- kawoosh.map(mode, keys, cmd[, opts]): `cmd` is a command name (with
-- args, as the command line would spell it) or a function, which
-- becomes one. A map belongs to a place when `opts` names one — `view`
-- (a view's pane), `view` and `field` (its field), `buffer` (a handle,
-- `true` for the current one, or a name), `language`, or any fact as
-- `scope` — and is found only there, before the global map, whose
-- binding on the same keys and every longer one under them it shadows
-- there: `map("n", "<CR>", "dir enter", { language = "dir" })` is the
-- listing's `<CR>` and nothing anywhere else. A key can be bound more
-- than once in one place: the newest binding whose `opts.when` holds
-- and whose command can run is the one that runs.
function kawoosh.map(mode, keys, cmd, opts)
  if type(cmd) == "function" then
    kawoosh._nonce = kawoosh._nonce + 1
    local name = "lua." .. mode .. "." .. keys:gsub("[<>%s]", "_") .. "." .. kawoosh._nonce
    kawoosh.command(name, cmd)
    cmd = name
  end
  kawoosh._map(mode, keys, cmd, opts and opts.when or nil, scope_of(opts))
end

-- A list of actions — `{ label = "Retry", run = fn }`, or `run =
-- "command line"` — as the labels and the commands they run, a
-- function made a command under `prefix`.
local function actions_of(list, prefix)
  local labels, commands = {}, {}
  for i, a in ipairs(list or {}) do
    local cmd = a.run or a.command or a[2]
    if type(cmd) == "function" then
      kawoosh._nonce = kawoosh._nonce + 1
      local name = prefix .. kawoosh._nonce
      kawoosh.command(name, cmd)
      cmd = name
    end
    labels[i] = tostring(a.label or a[1] or ("action " .. i))
    commands[i] = tostring(cmd or "")
  end
  return labels, commands
end

-- kawoosh.unmap(mode, keys[, opts]): the key's bindings gone — the
-- global ones, or those of the place `opts` names as `kawoosh.map`'s do
-- — the longer ones beneath it staying. A binding whose `when` does not
-- hold where the key is pressed does not shadow those either.
function kawoosh.unmap(mode, keys, opts)
  kawoosh._unmap(mode, keys, scope_of(opts))
end

-- kawoosh.on_open(fn): `fn(path)` for every path the editor is asked
-- to open — `:e`, the command line's argument, a location, a
-- `kawoosh.open` — before it is read as a file; an opener that returns
-- true has taken it (the file manager takes a directory and lists it),
-- and the rest, and the editor, do not see it. The path arrives
-- resolved. An opener shows what it opens itself (`open_scratch`, a
-- view); one that asked `kawoosh.open` of the same path would be asked
-- again.
function kawoosh.on_open(fn)
  kawoosh._openers[#kawoosh._openers + 1] = fn
end

-- kawoosh.tab_title(fn): the tab strip's labels written by
-- `fn(tab)`, wezterm's `format-tab-title` — `tab` is `{ index, active,
-- title, dir, cwd, kind, name, path, modified, bell, panes }`: `title`
-- the label kawoosh would draw (`N: name`, the directory as
-- `tabs.directory` says, ` ●` when modified), `dir` the last part of
-- `cwd` (a terminal's is where its shell says it is), `kind` its focused
-- pane's (`editor`, `terminal`, `lua`, `undo`, `memory`), `name` that
-- pane's buffer, terminal title or view, `path` its file. It returns
-- the label, or nil for kawoosh's; `kawoosh.tab_title(nil)` takes it
-- off, and so does an error, said once. Called for every tab, every
-- frame the strip is drawn: keep it to strings.
function kawoosh.tab_title(fn)
  kawoosh._tab_title = fn
end

-- kawoosh.status(name, fn[, opts]): a segment on the right of the title
-- bar (`place = "title"`, the default) or of the tab strip (`"tabs"`),
-- or a module of the status line (`"statusline"`, placed by
-- `statusline.left` / `.right`, else where `...` is — statusline.md),
-- in `order` then by name (docs/design/status.md). `fn(ctx)` answers
-- what it shows now — nil for nothing, a string, a part `{ text =,
-- color = }` or a list of parts, `color` one of `fg`, `dim`, `accent`,
-- `ok`, `warning`, `danger` — and is asked each frame the window draws.
-- `opts.every = SECONDS` wakes the window to draw it that often, on the
-- wall clock's beat (a clock) — or a function answering how often now,
-- nil for not at all; `opts.run` is a command line a click
-- runs. The same name replaces it; `kawoosh.status(name, nil)` takes it
-- away.
kawoosh._status = {}
function kawoosh.status(name, fn, opts)
  if fn == nil then
    kawoosh._status[name] = nil
    return
  end
  opts = opts or {}
  kawoosh._status[name] = { fn = fn, place = opts.place or "title", order = opts.order or 0,
                            every = opts.every, run = opts.run }
end

-- kawoosh.on_memory_open(kind, fn): how a moment of a plugin's own
-- kind (`<plugin>.<kind>`, memory.md Decision 9) is opened from the
-- memory pane — `fn(row)`, `row` its `kind`, `subject` and `meta` (a
-- table), the keyboard on the editor pane the memory pane came from.
-- A kind with no opener says it has nothing to open.
kawoosh._memory_openers = {}
function kawoosh.on_memory_open(kind, fn)
  kawoosh._memory_openers[kind] = fn
end

-- kawoosh.on_settings(fn): `fn()` whenever the settings changed — a
-- file reloaded on save, `:set`, `kawoosh.opt` — once a frame, with
-- `kawoosh.opt` reading the new tree; and once at registration, so a
-- plugin reads what is set now the same way it reads what changes.
-- kawoosh.fs.watch(name, paths, fn): the paths — files or directories
-- — polled twice a second, `fn(changed)` with those whose stamp moved
-- (written, made, gone; a directory's when an entry is added, removed
-- or renamed). A plugin's set by `name`, replaced by each call;
-- `kawoosh.fs.watch(name, nil)` stops it.
function kawoosh.fs.watch(name, paths, fn)
  kawoosh._watches[name] = paths and fn or nil
  kawoosh._fs_watch(name, paths or {})
end

function kawoosh._watched(name, paths)
  local fn = kawoosh._watches[name]
  if not fn then return end
  local ok, err = pcall(fn, paths)
  if not ok then kawoosh.echo("fs.watch " .. name .. ": " .. tostring(err)) end
end

function kawoosh.on_settings(fn)
  kawoosh._settings_hooks[#kawoosh._settings_hooks + 1] = fn
  local ok, err = pcall(fn)
  if not ok then kawoosh.echo("on_settings: " .. tostring(err)) end
end

-- Called from Rust when the settings' version moved.
function kawoosh._settings()
  for _, fn in ipairs(kawoosh._settings_hooks) do
    local ok, err = pcall(fn)
    if not ok then kawoosh.echo("on_settings: " .. tostring(err)) end
  end
end

-- kawoosh.on_cwd(fn): `fn(path, how)` whenever the working directory
-- moved, once a frame, after it moved; not for the directory kawoosh
-- started in. The working directory is the focused tab's
-- (docs/design/workspaces.md): `how` is "cd" (`:cd`, a listing's
-- `~` in a listing, `kawoosh.fs.chdir`) or "tab" (the keys went to a tab
-- in another directory).
kawoosh._cwd_hooks = {}
function kawoosh.on_cwd(fn)
  kawoosh._cwd_hooks[#kawoosh._cwd_hooks + 1] = fn
end

function kawoosh._cwd(path, how)
  for _, fn in ipairs(kawoosh._cwd_hooks) do
    local ok, err = pcall(fn, path, how)
    if not ok then kawoosh.echo("on_cwd: " .. tostring(err)) end
  end
end

-- kawoosh.on_diagnostics(fn): `fn()` after a frame in which the
-- diagnostics moved — a server's word, a file's kept ones taken by the
-- buffer that opened it — with `kawoosh.lsp.diagnostics` reading them.
-- kawoosh.on_focus(fn): `fn(buffer)` when the keyboard goes to an
-- editor pane on another buffer (docs/design/lists.md Decision 5).
-- kawoosh.on_places(fn): `fn(title, items)` with a server's list —
-- `references`, `implementations`, `declarations` — each `{ path, line,
-- col, end_line, end_col }` from 1; a hook that returns true made the
-- list, and the engine's plain one is not made.
kawoosh._diagnostics_hooks = {}
function kawoosh.on_diagnostics(fn)
  kawoosh._diagnostics_hooks[#kawoosh._diagnostics_hooks + 1] = fn
end
function kawoosh._diagnostics()
  for _, fn in ipairs(kawoosh._diagnostics_hooks) do
    local ok, err = pcall(fn)
    if not ok then kawoosh.echo("on_diagnostics: " .. tostring(err)) end
  end
end
kawoosh._focus_hooks = {}
function kawoosh.on_focus(fn)
  kawoosh._focus_hooks[#kawoosh._focus_hooks + 1] = fn
end
function kawoosh._focus(buffer)
  for _, fn in ipairs(kawoosh._focus_hooks) do
    local ok, err = pcall(fn, buffer)
    if not ok then kawoosh.echo("on_focus: " .. tostring(err)) end
  end
end
kawoosh._places_hooks = {}
function kawoosh.on_places(fn)
  kawoosh._places_hooks[#kawoosh._places_hooks + 1] = fn
end
function kawoosh._places(title, items)
  local taken = false
  for _, fn in ipairs(kawoosh._places_hooks) do
    local ok, r = pcall(fn, title, items)
    if not ok then kawoosh.echo("on_places: " .. tostring(r))
    elseif r then taken = true end
  end
  return taken
end

-- kawoosh.term.send(text[, { prompt = true }]): `text` typed into the
-- terminal pane with the keys, as the keyboard would (`\r` runs a
-- line); with `prompt`, only while its shell sits at an empty prompt —
-- marked by OSC 133 and nothing typed since — else refused with a
-- message saying why.
--
-- kawoosh.on_restore(fn): `fn(name, buffer)` for every scratch buffer a
-- session brings back — empty, named as it was — so the plugin that
-- made it can fill it again (`open_scratch` by that name, `show =
-- false`).
function kawoosh.on_restore(fn)
  kawoosh._restorers[#kawoosh._restorers + 1] = fn
end

-- kawoosh.notify(text[, opts]): a notification. `opts` is a level name
-- ("debug", "info", "warn", "error"; info when omitted) or a table:
-- `level`, `source` (who says so), `show` ("toast", "corner", "log" —
-- else the level decides: an error or a warning is a toast, an info a
-- dim corner line, a debug the log's alone), `timeout` in ms (0 keeps
-- it until acted on), and `actions`, a list of `{ label = "Retry", run
-- = fn }` (or `run = "command line"`) — a toast with actions stays
-- until one is clicked. Every notification is in `:messages`.
function kawoosh.notify(text, opts)
  if type(opts) == "string" then opts = { level = opts } end
  opts = opts or {}
  local labels, commands = actions_of(opts.actions, "lua.notify.")
  local timeout = opts.timeout
  if timeout == false then timeout = 0 end
  kawoosh._notify(tostring(text), opts.level, opts.source, opts.show, timeout, labels, commands)
end

-- kawoosh.confirm{ title=, lines=, actions=, default= }: a question the
-- user answers before anything happens — one modal float over the
-- window with the keys on it. `title` is the question, `lines` what
-- would happen (one each, shown in mono), `actions` a list as
-- notify's — the first is what `<CR>` and `y` take unless `default`
-- names another (from 1); `h` `l` `<Tab>` move between them, a digit
-- takes that one, and `<Esc>`, `n`, `q` or a press outside answer
-- with none. An action without `run` is a plain "no".
function kawoosh.confirm(opts)
  local labels, commands = actions_of(opts.actions, "lua.confirm.")
  local lines = {}
  for i, l in ipairs(opts.lines or {}) do lines[i] = tostring(l) end
  kawoosh._confirm(tostring(opts.title or "?"), lines, labels, commands, opts.default)
end

-- kawoosh.view(name, fn[, on_event[, opts]]): a pane whose content is
-- what `fn` returns — a kui table tree (row, column, text, edit,
-- button, ...). `fn(ctx)` gets { pane = id, focused = bool, width =,
-- height =, share = (the pane's fraction of the split it is in, nil
-- when it is the whole window), origin = (a new pane's launcher: the
-- buffer it was split from, `{ buffer =, name =, path = }`), env =
-- kui's env }. Events from the tree's on_click / on_key
-- payloads reach `on_event(ev)`. `opts.session = false` keeps the view
-- out of a session: a picker is asked for again, not brought back.
function kawoosh.view(name, fn, on_event, opts)
  kawoosh._views[name] = fn
  kawoosh._handlers[name] = on_event
  kawoosh._transient[name] = (opts and opts.session == false) or nil
end

-- kawoosh.tool(name, { cmd =, cwd =, dock =, restore = }): a launch
-- target for `:tool NAME` and the tools picker — `restore` has a
-- session start it again in the directory it was left in (a shell, a
-- git UI; not a build); `kawoosh.tools()` lists them, by name, each
-- with what was registered.
local register_tool = kawoosh.tool
function kawoosh.tool(name, t)
  kawoosh._tools[name] = { cmd = t.cmd, cwd = t.cwd, dock = t.dock or false, restore = t.restore or false }
  register_tool(name, t)
end

function kawoosh.tools()
  local out = {}
  for name, t in pairs(kawoosh._tools) do
    out[#out + 1] = { name = name, cmd = t.cmd, cwd = t.cwd, dock = t.dock, restore = t.restore }
  end
  table.sort(out, function(a, b) return a.name < b.name end)
  return out
end

-- kawoosh.buf.open_scratch{ name=, text=, on_write=fn, on_change=fn,
-- read_only=bool, language=, reuse=handle, line=n, private=bool,
-- about=path }: a buffer that is not a file; `private` keeps it out of
-- the store, the memory, the session and the clipboard
-- (docs/design/secrets.md); `about` is the file it stands for, which
-- `%` names in a command line (`:!ansible-vault edit %`). `on_write(lines)` handles :w; it returns `false` when
-- the write is not done yet (a `kawoosh.confirm` is up), and the
-- buffer stays modified until it is. `on_change(name)` is told, once
-- a frame, that the text changed — an edit, an undo — so what a
-- plugin draws from it (annotations) can follow. A buffer named `name` already open is
-- refilled; else `reuse`, a scratch buffer's handle, is renamed and
-- refilled instead of a new buffer being made beside it — unless it
-- is shown in another pane too, which keeps it; `line` is where the
-- caret goes (from 1); `show = false` fills the buffer where it is —
-- another pane, the background — without putting it in the focused
-- pane, or makes it in the background.
--
-- Secrets (docs/design/secrets.md), the buffer as `annotate` takes it:
-- kawoosh.buf.set_private([private = true][, buffer]): no history row,
-- no memory, not in a session, not sent to a server, a yank from it a
-- secret in the register (put once, never on the clipboard);
-- kawoosh.buf.private([buffer]) says whether it is.
-- kawoosh.buf.mask_with(rule[, buffer]): a `secrets.masks` rule by
-- name on the buffer whatever its path — a vault decrypted into a
-- scratch. kawoosh.buf.mask(ranges[, buffer]): `{ {a, b}, … }`, byte
-- ranges (0-based, end exclusive) drawn as `•`, replacing the ones
-- given before and carried through edits after; `{}` takes them off.
-- kawoosh.buf.paint(set, spans[, buffer]): a plugin's named set of
-- coloured ranges, `{ {from, to, colour}, … }` (bytes, 0-based, end
-- exclusive), drawn over the syntax's colours, replacing the set's
-- earlier ones and carried through edits after; `{}` clears the set.
-- A colour is a role — `fg` `dim` `faint` `accent` `danger`, `added`
-- `modified` `ignored` `conflict` — or a syntax token's name
-- (`comment`, `keyword`, …). `dir.lua`'s version control marks paint.
-- kawoosh.secrets.private(path): whether a rule names the file;
-- kawoosh.secrets.mask_text(text, path[, language]): the text with
-- what the rules for that file mask drawn as `•` — a list's line.
--
-- kawoosh.buf.annotate(notes[, buffer]): text after a line's end that
-- is not the buffer's — what an entry is, beside its name — `{ [id] =
-- "text" }` by tracked line, drawn dim past the line and never in its
-- bytes; a note goes where its line goes (a line typed above moves it
-- down, its line deleted takes it away) and stays until set again or
-- taken off with `false`; the notes not named are kept. `buffer` is a
-- handle, a name (a scratch just asked for by `open_scratch`, which is
-- not in the snapshot yet — its ids are its line numbers then), or the
-- current one.
-- `kawoosh.buf.tracked_lines([buffer])` is where each line a hooked
-- buffer opened with is now (a line number from 1, or false), beside
-- `tracked()`'s what it became, both by the line's id — its index;
-- `kawoosh.buf.tracked_line(id[, buffer])` is one line's text and line
-- number (nil once deleted); `kawoosh.buf.track(line[, buffer])`
-- follows one more line from now on — a line pasted in — and returns
-- its id, which all of them know at once (nil for a line the buffer
-- does not have). A buffer's lines are tracked again, from 1, whenever
-- `open_scratch` fills it, its notes going with the old ones.
--
-- kawoosh.buf.changes([buffer]): what differs from the buffer's lines
-- as tracked, so a plugin reads what changed and not the buffer —
-- `edited`, by id, the text of each tracked line that reads otherwise
-- than when it was tracked, with `lines` its line number; `gone`, the
-- ids deleted; `untracked`, by line number, the text of each line no
-- tracked line is on (typed or pasted in); and `shared`, by line
-- number, the ids of the tracked lines that landed on one line (`J`).
--
-- kawoosh.buf.show(buffer): the buffer into the focused pane, as `:b`
-- would, its caret where it was left.
--
-- kawoosh.buf.close(buffer[, { force = true }]): the buffer closed as
-- `:bd` closes it, every pane on it moved to another listed buffer (a
-- new scratch when it was the last); one with unsaved changes stays,
-- the message saying so, unless `force` drops them as `:bd!` does.
--
-- kawoosh.buf.retarget(from, to): every buffer open at path `from`, or
-- under it, is at `to` from now on, named after it — a file renamed or
-- moved by the file manager while it was open, so its `:w` goes where
-- the file went.
--
-- kawoosh.memory(): the working memory, newest first — what passed
-- through the hands: each moment's `text`, `linewise`, `took` ("yank",
-- "delete", "change", "clipboard"), `from` (the buffer's name then),
-- `buffer` (its handle, while it is open) and `age` in seconds. The
-- `"` register is the newest moment; `kawoosh.recall(i)` makes moment
-- `i` the newest, so the next `p` — or a plugin reading the register's
-- origin — has it.
--
-- kawoosh.memory { … }: the memory as data (docs/design/memory.md) —
-- the store's rows as of the last flush, a second behind at most. A
-- query names `kind` ("file", "scratch", "text", "command", "search",
-- or a plugin's own "<plugin>.<kind>"), `workspace` (a path, or `true`
-- for the current one; nothing for every workspace), `subject` (one
-- row, or nil), `since` (seconds back), `pinned = true` (the pins in
-- pin order) and `limit` (200); `{ recent = true, limit = }` is the
-- ring instead — the transitions newest first, `{ at =, age =, kind =,
-- subject =, workspace = }` each. A row: `kind`, `subject`, `workspace`,
-- `first`, `last` (unix seconds), `age`, `visits`, `dwell` (seconds),
-- `edits`, `yanks`, `pinned` (0, or the pin's ordinal), `meta` (a
-- table: a file's `line`, a text's `took`), and a text's `text`.
-- kawoosh.remember { kind =, subject =, signals = { visits =, edits =,
-- yanks =, dwell = }, meta = }: signals added to a subject's row — a
-- file's path resolved as `:e` would — and `meta` set; a plugin's own
-- kind is "<plugin>.<kind>", capped at five hundred rows, and never a
-- text (the register is the engine's). kawoosh.forget(kind, subject)
-- takes a row out, its draft with it; kawoosh.pin(kind, subject[,
-- on]) pins or unpins one. kawoosh.now(): unix seconds, for a `rank`.
-- The picker's ranking is `kawoosh.memory_rank` (`memory.lua`):
-- `rank(row, now)` is the score a row gets and yours to replace,
-- `boosts(kind, limit)` the rows as the picker's boosts.
--
-- kawoosh.buf.register(): the `"` register — `text`, `linewise`, and,
-- when one yank or delete filled it, `buffer` (the handle it came
-- from) and `entries`, for each line of the text the tracked line of
-- that buffer it was (an index of `tracked()`, or false): how a line
-- pasted into one listing is known to be an entry of another.
--
-- kawoosh.view_open(name[, { focus = false, below = true, share = 0.5 }])
-- puts a Lua view in a split — beside, or below with `below`, taking
-- `share` of the room — or focuses its pane, resized to `share` when
-- one is given; `focus = false` leaves the keyboard where it is;
-- `height = px`, in place of `share`, makes a new pane that tall below
-- its title — a bar as tall as its rows from the first frame. kawoosh.view_close(name) closes that pane
-- and hands the keyboard back to the pane it came from;
-- kawoosh.view_toggle(name[, opts]) does one or the other.
--
-- kawoosh.open(path[, { line =, col =, split = "vsplit" | "split" |
-- "tab" }]): the path in an editor pane — the focused one, or a new
-- one beside, below, or in a new tab — the caret on the line.
-- `kawoosh.buf.show(buffer[, { split = }])` the same for a buffer.
-- kawoosh.cmdline(text): the command line opened with `text` on it.
-- kawoosh.copy(text): onto the system clipboard and into the register,
-- as a yank puts text — for what no buffer's range holds, a path.
-- kawoosh.pass(): from a command a key ran, the key is not this
-- command's here — the binding under it gets it (the next whose `when`
-- holds), and a key that types, with none left, types; do nothing
-- before passing. Two plugins share a key this way: `timed.lua`'s
-- `<CR>` passes to pairs' outside a timed buffer.
-- kawoosh.run(line): a command line run where the keyboard is, after
-- what was asked before it (a pane closed, a file opened) — where
-- `kawoosh.cmd` runs at once, inside the command that asked.
--
-- kawoosh.highlight(text, { language = | path = }, fn): the text's
-- syntax, read on the ts thread by the language named, or the one a
-- path (and the first line) says; `fn(runs)` when done, each run
-- `{ from =, to =, token =, color = }` — the bytes it covers (from 1,
-- `to` the last), the token's name ("keyword", "string", …) and the
-- colour the theme paints it, `0xRRGGBBAA`, or nil for none, and the
-- style it sets the text in as a span's flags — `bold`, `italic`,
-- `underline`, `strikethrough` — each there when on. For a
-- preview, a pane of a plugin's own: a few hundred lines is a moment.
--
-- kawoosh.fs.walk(root, fn): every file under `root` as git sees it —
-- `.gitignore`d, hidden and `.git` left out — relative to it, read on
-- a thread of its own; `fn(paths)` when done, or `fn(nil, why)`.
-- kawoosh.fs.form(path, form): the path as `path copy` copies it —
-- "relative" (to the working directory, whole when outside it),
-- "absolute", "dir", "dir absolute", "name", "stem".
-- kawoosh.spawn(cmd, { cwd =, stdin =, on_lines = fn(lines), on_exit =
-- fn(code) }) runs `cmd` through the shell — `stdin` written to it
-- and closed, for text that must not be on a command line — and hands
-- its output over in lines as they come, once a frame; it returns a token `kawoosh.kill(token)`
-- stops the process with (its `on_exit` then gets no code).
-- kawoosh.fuzzy(needle, list[, limit]) scores a small list;
-- kawoosh.matcher(list) holds a big one — `m:query(needle, limit)`
-- answers `{ index =, score =, positions = }` best first, positions
-- the matched characters' bytes from 1; `m:count()`. Case is smart.
-- kawoosh.oldfiles([limit]): the files attended before (the memory's
-- `file` rows), newest first, `{ path =, line = }` each. kawoosh.holds(fact): whether a fact holds
-- where the keyboard is. kawoosh.buf.lines_in(from, to[, buffer]): a
-- window of a buffer's lines.
function kawoosh.buf.open_scratch(t)
  if t.on_write then kawoosh._writers[t.name] = t.on_write end
  if t.on_change then kawoosh._changers[t.name] = t.on_change end
  kawoosh._open_scratch(t.name, t.text or "", t.on_write ~= nil, t.read_only or false, t.language,
    t.reuse, t.line, t.show ~= false, t.on_change ~= nil, t.private or false, t.about)
end

-- ---------------------------------------------------------------- fields

-- The byte length of the character at byte `i` (1-based) of `s`.
local function char_len(s, i)
  local c = s:byte(i)
  if not c then return 0 end
  if c < 0x80 then return 1 elseif c < 0xE0 then return 2 elseif c < 0xF0 then return 3 else return 4 end
end

-- A view's field, drawn from the engine's: the text, the selection in
-- visual mode, the caret — a bar on kui's blink in insert mode, a block
-- in normal — and a placeholder while it is empty and off the keys.
-- `full` names the engine field (`lua:<view>/<name>`); one that is not
-- open yet is asked for and drawn empty this frame.
local function field_node(view_name, env, opts)
  local full = "lua:" .. view_name .. "/" .. opts.name
  local st = kawoosh._field(full)
  if not st then
    kawoosh._field_open(full)
    st = { text = "", mode = "normal", caret = 0, anchor = 0, focused = false }
  end
  local t = env.theme
  local size = opts.size or 13
  -- One line, whatever its length: never wrapped, so no second row is
  -- drawn over what is under the field.
  local style = { family = "mono", size = size, wrap = "none" }
  local line = st.text
  local focused = st.focused
  local insert = st.mode == "insert"
  -- Byte ranges (0-based, end exclusive) of the selection and the block
  -- caret's character, in visual and normal mode.
  local sel_lo, sel_hi
  if focused and st.mode == "visual" then
    sel_lo = math.min(st.anchor, st.caret)
    sel_hi = math.max(st.anchor, st.caret)
    sel_hi = sel_hi + math.max(char_len(line, sel_hi + 1), 1)
  end
  local block_lo, block_hi
  if focused and not insert then
    block_lo = st.caret
    block_hi = st.caret + math.max(char_len(line, st.caret + 1), 1)
  end
  -- A rounded selection (`editor.selection_radius`): the spans' own
  -- backgrounds, which kui joins into one shape (its F101), as a pane's.
  local radius = sel_lo and kawoosh._selection_radius() or nil
  -- A block caret inside a rounded selection: its character keeps the
  -- selection's background, so the shape has no hole there, and the
  -- caret is drawn over it on its own.
  local lifted = radius and block_lo and block_lo < #line
    and block_lo >= sel_lo and block_hi <= sel_hi
  -- The text as spans, cut at every edge, each styled by what it is in.
  local cuts = { 0, #line }
  local function cut(b) if b and b > 0 and b < #line then cuts[#cuts + 1] = b end end
  cut(sel_lo) cut(sel_hi) cut(block_lo) cut(block_hi)
  if insert and focused then cut(st.caret) end
  table.sort(cuts)
  local function spans_between(from, to)
    local out = {}
    for i = 1, #cuts - 1 do
      local a, b = cuts[i], cuts[i + 1]
      if a >= from and b <= to and b > a then
        local piece = line:sub(a + 1, b)
        local span = { piece }
        if sel_lo and a >= sel_lo and b <= sel_hi then
          span.bg = t.selection
          span.bg_radius = radius
        end
        if block_lo and not lifted and a >= block_lo and b <= block_hi then
          span.bg = t.accent
          span.color = t.bg
          span.bg_radius = nil
        end
        out[#out + 1] = span
      end
    end
    return out
  end
  local row_h = size + 6
  -- A line wider than the field scrolls sideways under it, as little
  -- as keeps the caret in view — the bar, or the block's character (a
  -- space past the end) — from where the last frame left it.
  local label = "field:" .. full
  local g = env.scroll_geometry(label)
  -- A field's first frame has no geometry to follow the caret by: one
  -- more frame (`animate`, for this one), or a line set before it — a
  -- picker opened on a query — waits for a key to scroll.
  local again = focused and not g
  if focused and g then
    local off = g.offset.x
    local x = env.measure_text(line:sub(1, st.caret), style).width
    local w = 2
    if not insert then
      local glyph = line:sub(st.caret + 1, st.caret + char_len(line, st.caret + 1))
      w = env.measure_text(glyph ~= "" and glyph or " ", style).width
    end
    local want = off
    if x < off then want = x elseif x + w > off + g.w then want = x + w - g.w end
    if want ~= off then env.set_scroll(label, want, 0) end
  end
  local children = {}
  local function push(node) children[#children + 1] = node end
  if insert and focused then
    -- Two texts around a bar that keeps its place on the blink's off
    -- phase, so the line does not shift.
    local before, after = spans_between(0, st.caret), spans_between(st.caret, #line)
    if #before > 0 then push(text(before, style)) end
    push(row { width = 2, height = size + 2, bg = env.caret_visible and t.accent or nil })
    if #after > 0 then push(text(after, style)) end
  else
    local all = spans_between(0, #line)
    if #all > 0 then push(text(all, style)) end
    -- A block caret past the end sits on a space of its own, and so
    -- does a selection that takes the line's end.
    if block_lo and block_lo >= #line then
      push(text({ { " ", bg = t.accent, color = t.bg } }, style))
    elseif sel_hi and sel_hi > #line then
      push(text({ { " ", bg = t.selection, bg_radius = radius } }, style))
    end
    if lifted then
      local glyph = line:sub(block_lo + 1, block_hi)
      local h = env.measure_text(glyph, style).height
      push(row {
        float = { dx = env.measure_text(line:sub(1, block_lo), style).width, dy = (row_h - h) / 2,
                  clip = true },
        text({ { glyph, bg = t.accent, color = t.bg } }, style),
      })
    end
  end
  if #line == 0 and not focused and opts.placeholder then
    push(text(opts.placeholder, { family = "mono", size = size, color = t.muted, wrap = "none" }))
  end
  -- The line at its own width, however wide, in a row that scrolls it
  -- (with no bar: the caret says where it is).
  local line_row = row {
    min_width = "fit",
    height = row_h,
    cross_align = "center",
    role = "line",
    caret = focused and st.caret or nil,
    label = opts.label or opts.name,
  }
  for _, c in ipairs(children) do line_row[#line_row + 1] = c end
  return row {
    key = label,
    height = row_h,
    scroll_x = true,
    scrollbar = "hidden",
    animate = again or nil,
    on_click = { kind = "field", field = full },
    line_row,
  }
end

-- kawoosh.field_text(view, name): a view's field's line, "" before it
-- is opened. `kawoosh.field_set(view, name, text)` puts a line on it.
function kawoosh.field_text(view_name, name)
  local st = kawoosh._field("lua:" .. view_name .. "/" .. name)
  return st and st.text or ""
end

local field_set = kawoosh.field_set
function kawoosh.field_set(view_name, name, text)
  field_set("lua:" .. view_name .. "/" .. name, text)
end

-- kawoosh.field_focus(view, name): the view's keys go to the field —
-- the editor's own keys, insert mode to type, `<Esc>` to normal mode,
-- `<Esc>` again back to the view. `nil` for back to the view.
function kawoosh.field_focus(view_name, name)
  kawoosh._field_focus(view_name, name and ("lua:" .. view_name .. "/" .. name) or nil)
end

-- ---------------------------------------------------------------- kui's doors

-- "lua/counter@2" (an event's full name) or "counter@2" (a view's own):
-- the view's name and the pane it is in.
local function split_slot(name)
  name = name:match("([^/]*)$") or name
  local view, pane = name:match("^(.-)@(%d+)$")
  return view or name, tonumber(pane) or 0
end

function view(env, slot)
  local name, pane = split_slot(slot.name)
  local fn = kawoosh._views[name]
  local t = env.theme
  if not fn then
    return column { pad = 12, text("no such view: " .. name, { color = t.danger }) }
  end
  local params = slot.params or {}
  -- `ctx.field { name = "q", placeholder = , size = , label = }`: a
  -- one-line input drawn through the editor (kui.md Decision 12), the
  -- node to put in the tree; `ctx.field_text("q")` is its line.
  local ctx = { pane = pane, focused = params.focused, width = params.width,
                height = params.height, share = params.share, origin = params.origin, env = env,
                name = name }
  ctx.field = function(opts) return field_node(name, env, opts) end
  ctx.field_text = function(field) return kawoosh.field_text(name, field) end
  local ok, tree = pcall(fn, ctx)
  if not ok then
    return column { pad = 12, gap = 6,
      text("view `" .. name .. "` failed", { color = t.danger }),
      text(tostring(tree), { size = 12, color = t.muted, wrap = "word" }) }
  end
  if type(tree) ~= "table" then
    return column { pad = 12, text("view `" .. name .. "` returned " .. type(tree), { color = t.danger }) }
  end
  return tree
end

function on_event(ev)
  if not ev.slot then return end
  local name = split_slot(ev.slot)
  -- A click on a field: the keys go to it.
  if ev.kind == "field" and type(ev.field) == "string" then
    kawoosh._field_focus(name, ev.field)
    return
  end
  local h = kawoosh._handlers[name]
  if h then
    local ok, err = pcall(h, ev)
    if not ok then kawoosh.echo("view `" .. name .. "`: " .. tostring(err)) end
  end
end

-- Called from Rust for a key in a focused Lua pane whose field does
-- not have it: the view's handler keeps the key by returning true;
-- otherwise it is pane mode's (the view's `kawoosh.map("p", …)`, the
-- list keys, and what every pane shares).
function kawoosh._key(name, ev)
  local h = kawoosh._handlers[name]
  if not h then return false end
  local ok, taken = pcall(h, ev)
  if not ok then
    kawoosh.echo("view `" .. name .. "`: " .. tostring(taken))
    return false
  end
  return taken == true
end

-- Called from Rust when a command registered here runs.
function kawoosh._run(name, ctx)
  local fn = kawoosh._commands[name]
  if not fn then return end
  local ok, err = pcall(fn, ctx)
  if not ok then kawoosh.echo("command `" .. name .. "`: " .. tostring(err)) end
end

-- Called from Rust for each path to open: true when an opener took it.
function kawoosh._open(path)
  for _, fn in ipairs(kawoosh._openers) do
    local ok, taken = pcall(fn, path)
    if not ok then kawoosh.echo("open `" .. path .. "`: " .. tostring(taken)) end
    if ok and taken then return true end
  end
  return false
end

-- Called from Rust for a plugin's moment opened from the memory pane:
-- true when its kind has an opener.
function kawoosh._memory_open(kind, subject, meta)
  local fn = kawoosh._memory_openers[kind]
  if not fn then return false end
  local ok, err = pcall(fn, { kind = kind, subject = subject, meta = meta or {} })
  if not ok then kawoosh.echo("memory `" .. kind .. "`: " .. tostring(err)) end
  return true
end

-- Called from Rust for each scratch buffer a session restored.
function kawoosh._restore(name, h)
  for _, fn in ipairs(kawoosh._restorers) do
    local ok, err = pcall(fn, name, h)
    if not ok then kawoosh.echo("restore `" .. name .. "`: " .. tostring(err)) end
  end
end

-- Called from Rust when a watched scratch buffer's text changed.
function kawoosh._change(name)
  local fn = kawoosh._changers[name]
  if not fn then return end
  local ok, err = pcall(fn, name)
  if not ok then kawoosh.echo("change `" .. name .. "`: " .. tostring(err)) end
end

-- Called from Rust when a scratch buffer with an on_write is written:
-- false when the hook said the write waits.
function kawoosh._write(name, lines)
  local fn = kawoosh._writers[name]
  if not fn then return true end
  local ok, res = pcall(fn, lines)
  if not ok then
    kawoosh.echo("write `" .. name .. "`: " .. tostring(res))
    return true
  end
  return res ~= false
end

-- ---------------------------------------------------------------- tests, eval

-- The test harness (`kawoosh test PATH`, roadmap step 8): a script runs
-- as a coroutine and these yield to the editor, which presses the keys,
-- draws the frames and publishes the state again before resuming — so
-- the line after `kawoosh.press("dd")` reads the buffer as it is then.
-- Outside a test they raise, since there is no harness to yield to.
function kawoosh.press(keys) coroutine.yield({ press = keys }) end
function kawoosh.frame(n) coroutine.yield({ frame = n or 1 }) end
function kawoosh.sleep(ms) coroutine.yield({ sleep = ms or 10 }) end
-- kawoosh.wait(fn[, frames]): frames until `fn()` holds, ten
-- milliseconds apart for a thread to answer; fails past `frames`.
function kawoosh.wait(fn, frames, what)
  frames = frames or 300
  for _ = 1, frames do
    if fn() then return true end
    coroutine.yield({ sleep = 10 })
  end
  error("waited " .. frames .. " frames" .. (what and (" for " .. what) or ""), 2)
end

kawoosh.test = {}
function kawoosh.test.eq(got, want, what)
  if got ~= want then
    error(string.format("%s: expected %s, got %s", what or "eq", kawoosh._show(want), kawoosh._show(got)), 2)
  end
end
function kawoosh.test.ok(cond, what)
  if not cond then error(what or "expected true", 2) end
end
function kawoosh.test.has(list, item, what)
  for _, x in ipairs(list or {}) do if x == item then return end end
  error(string.format("%s: %s not in %s", what or "has", kawoosh._show(item), kawoosh._show(list)), 2)
end

-- A value spelled for a message: a table shallowly, its keys in order.
function kawoosh._show(v, depth)
  depth = depth or 0
  if type(v) ~= "table" then
    if type(v) == "string" then return string.format("%q", v) end
    return tostring(v)
  end
  if depth > 2 then return "{…}" end
  local keys = {}
  for k in pairs(v) do keys[#keys + 1] = k end
  table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
  local parts = {}
  local n = #v
  for _, k in ipairs(keys) do
    local val = kawoosh._show(v[k], depth + 1)
    if type(k) == "number" and k >= 1 and k <= n then
      parts[#parts + 1] = val
    else
      parts[#parts + 1] = tostring(k) .. " = " .. val
    end
  end
  return "{ " .. table.concat(parts, ", ") .. " }"
end
