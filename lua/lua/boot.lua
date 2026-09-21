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
kawoosh._tools = {}
kawoosh._nonce = 0

-- kawoosh.command(name, fn[, opts]): a named command, callable from a
-- keymap, the command line, or Rust. A name of two words is a
-- subcommand (`"dir cd"` runs as `:dir cd`, completes under `:dir`).
-- `fn(ctx)` gets { count = n, args = {...}, form = "run" | "bang" |
-- "query", bang = bool, query = bool }. `opts`:
--   args    what the arguments are, one kind per position — "path",
--           "buffer", "command", "option", "tool", "view", "text" — the
--           last with "..." for the rest: a "path" reaches `fn` absolute
--           (`~`, `..`, the working directory resolved), and the command
--           line completes each kind.
--   aliases the ex spellings, `{ "o" }`.
--   bang    a line on what `!` means; without one `:name!` is refused.
--   query   the same for `?`.
--   when    facts that must hold, `{ "language:dir", "!terminal" }` —
--           the engine's `visual`, `modified`, `file`, `buffer:NAME`,
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
-- kawoosh.map(mode, keys, cmd[, opts]): `cmd` is a command name (with
-- args, as the command line would spell it) or a function, which
-- becomes one. A key can be bound more than once: the newest binding
-- whose `opts.when` holds and whose command can run is the one that
-- runs, so `map("n", "<CR>", "goto location", { when = { "!language:dir" } })`
-- and then `map("n", "<CR>", "dir enter")` (a command gated on the
-- listing) make one key do the right thing in each place. A binding
-- with no `when` on a command with none shadows the older ones.
function kawoosh.map(mode, keys, cmd, opts)
  if type(cmd) == "function" then
    kawoosh._nonce = kawoosh._nonce + 1
    local name = "lua." .. mode .. "." .. keys:gsub("[<>%s]", "_") .. "." .. kawoosh._nonce
    kawoosh.command(name, cmd)
    cmd = name
  end
  kawoosh._map(mode, keys, cmd, opts and opts.when or nil)
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

-- kawoosh.unmap(mode, keys): the key's bindings gone — the longer ones
-- beneath it stay. A binding whose `when` does not hold where the key
-- is pressed does not shadow those either, so a plugin that unmaps a
-- key and maps it back `when` elsewhere has it as a prefix of its own
-- where it needs one.

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

-- kawoosh.on_settings(fn): `fn()` whenever the settings changed — a
-- file reloaded on save, `:set`, `kawoosh.opt` — once a frame, with
-- `kawoosh.opt` reading the new tree; and once at registration, so a
-- plugin reads what is set now the same way it reads what changes.
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
-- when it is the whole window), env = kui's env }. Events from the tree's on_click / on_key
-- payloads reach `on_event(ev)`. `opts.session = false` keeps the view
-- out of a session: a picker is asked for again, not brought back.
function kawoosh.view(name, fn, on_event, opts)
  kawoosh._views[name] = fn
  kawoosh._handlers[name] = on_event
  kawoosh._transient[name] = (opts and opts.session == false) or nil
end

-- kawoosh.tool(name, { cmd =, cwd =, dock = }): a launch target for
-- `:tool NAME` and the tools picker; `kawoosh.tools()` lists them, by
-- name, each with what was registered.
local register_tool = kawoosh.tool
function kawoosh.tool(name, t)
  kawoosh._tools[name] = { cmd = t.cmd, cwd = t.cwd, dock = t.dock or false }
  register_tool(name, t)
end

function kawoosh.tools()
  local out = {}
  for name, t in pairs(kawoosh._tools) do
    out[#out + 1] = { name = name, cmd = t.cmd, cwd = t.cwd, dock = t.dock }
  end
  table.sort(out, function(a, b) return a.name < b.name end)
  return out
end

-- kawoosh.buf.open_scratch{ name=, text=, on_write=fn, on_change=fn,
-- read_only=bool, language=, reuse=handle, line=n }: a buffer that is
-- not a file. `on_write(lines)` handles :w; it returns `false` when
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
-- kawoosh.buf.annotate(notes[, buffer]): text after a line's end that
-- is not the buffer's — what an entry is, beside its name — `{ [id] =
-- "text" }` by tracked line, drawn dim past the line and never in its
-- bytes; a note goes where its line goes (a line typed above moves it
-- down, its line deleted takes it away) and stays until set again or
-- taken off with `false`; the notes not named are kept. `buffer` is a
-- handle, a name (a scratch just asked for by `open_scratch`, which is
-- not in the snapshot yet — its ids are its line numbers then), or the
-- current one. Spaces in it are `\u{A0}`, which every font keeps.
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
-- one is given; `focus = false` leaves the keyboard where it is. kawoosh.view_close(name) closes that pane
-- and hands the keyboard back to the pane it came from;
-- kawoosh.view_toggle(name[, opts]) does one or the other.
--
-- kawoosh.open(path[, { line =, col =, split = "vsplit" | "split" |
-- "tab" }]): the path in an editor pane — the focused one, or a new
-- one beside, below, or in a new tab — the caret on the line.
-- `kawoosh.buf.show(buffer[, { split = }])` the same for a buffer.
-- kawoosh.cmdline(text): the command line opened with `text` on it.
-- kawoosh.run(line): a command line run where the keyboard is, after
-- what was asked before it (a pane closed, a file opened) — where
-- `kawoosh.cmd` runs at once, inside the command that asked.
--
-- kawoosh.highlight(text, { language = | path = }, fn): the text's
-- syntax, read on the ts thread by the language named, or the one a
-- path (and the first line) says; `fn(runs)` when done, each run
-- `{ from =, to =, token =, color = }` — the bytes it covers (from 1,
-- `to` the last), the token's name ("keyword", "string", …) and the
-- colour the theme paints it, `0xRRGGBBAA`, or nil for none. For a
-- preview, a pane of a plugin's own: a few hundred lines is a moment.
--
-- kawoosh.fs.walk(root, fn): every file under `root` as git sees it —
-- `.gitignore`d, hidden and `.git` left out — relative to it, read on
-- a thread of its own; `fn(paths)` when done, or `fn(nil, why)`.
-- kawoosh.spawn(cmd, { cwd =, on_lines = fn(lines), on_exit = fn(code)
-- }) runs `cmd` through the shell and hands its output over in lines
-- as they come, once a frame; it returns a token `kawoosh.kill(token)`
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
    t.reuse, t.line, t.show ~= false, t.on_change ~= nil)
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
  local style = { family = "mono", size = size }
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
        if sel_lo and a >= sel_lo and b <= sel_hi then span.bg = t.selection end
        if block_lo and a >= block_lo and b <= block_hi then span.bg = t.accent; span.color = t.bg end
        out[#out + 1] = span
      end
    end
    return out
  end
  local row_h = size + 6
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
    -- A block caret past the end sits on a space of its own.
    if block_lo and block_lo >= #line then
      push(text({ { " ", bg = t.accent, color = t.bg } }, style))
    end
  end
  if #line == 0 and not focused and opts.placeholder then
    push(text(opts.placeholder, { family = "mono", size = size, color = t.muted }))
  end
  local node = row {
    key = "field:" .. full,
    height = row_h,
    cross_align = "center",
    on_click = { kind = "field", field = full },
    role = "line",
    caret = focused and st.caret or nil,
    label = opts.label or opts.name,
  }
  for _, c in ipairs(children) do node[#node + 1] = c end
  return node
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
                height = params.height, share = params.share, env = env, name = name }
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
