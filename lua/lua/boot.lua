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
kawoosh._nonce = 0

-- kawoosh.command(name, fn[, opts]): a named command, callable from a
-- keymap, the command line, or Rust. `fn(ctx)` gets { count = n, args =
-- {...} }. `opts.args` says what the arguments are, one kind per
-- position — "path", "buffer", "command", "option", "tool", "view",
-- "text" — the last with "..." for the rest: a "path" reaches `fn`
-- absolute (`~`, `..`, the working directory resolved), and the command
-- line completes each kind. `kawoosh.command("oil", fn, { args = {
-- "path" } })`.
function kawoosh.command(name, fn, opts)
  kawoosh._commands[name] = fn
  kawoosh._register(name, opts and opts.args or nil)
end

-- kawoosh.map(mode, keys, cmd): `cmd` is a command name (with args, as
-- the command line would spell it) or a function, which becomes one.
function kawoosh.map(mode, keys, cmd)
  if type(cmd) == "function" then
    kawoosh._nonce = kawoosh._nonce + 1
    local name = "lua." .. mode .. "." .. keys:gsub("[<>%s]", "_") .. "." .. kawoosh._nonce
    kawoosh.command(name, cmd)
    cmd = name
  end
  kawoosh._map(mode, keys, cmd)
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
  local labels, commands = {}, {}
  for i, a in ipairs(opts.actions or {}) do
    local cmd = a.run or a.command or a[2]
    if type(cmd) == "function" then
      kawoosh._nonce = kawoosh._nonce + 1
      local name = "lua.notify." .. kawoosh._nonce
      kawoosh.command(name, cmd)
      cmd = name
    end
    labels[i] = tostring(a.label or a[1] or ("action " .. i))
    commands[i] = tostring(cmd or "")
  end
  local timeout = opts.timeout
  if timeout == false then timeout = 0 end
  kawoosh._notify(tostring(text), opts.level, opts.source, opts.show, timeout, labels, commands)
end

-- kawoosh.view(name, fn[, on_event]): a pane whose content is what `fn`
-- returns — a kui table tree (row, column, text, edit, button, ...).
-- `fn(ctx)` gets { pane = id, focused = bool, env = kui's env }. Events
-- from the tree's on_click / on_key payloads reach `on_event(ev)`.
function kawoosh.view(name, fn, on_event)
  kawoosh._views[name] = fn
  kawoosh._handlers[name] = on_event
end

-- kawoosh.buf.open_scratch{ name=, text=, on_write=fn, read_only=bool,
-- language=, reuse=handle, line=n }: a buffer that is not a file.
-- `on_write(lines)` handles :w. A buffer named `name` already open is
-- refilled; else `reuse`, a scratch buffer's handle, is renamed and
-- refilled instead of a new buffer being made beside it; `line` is
-- where the caret goes (from 1).
function kawoosh.buf.open_scratch(t)
  if t.on_write then kawoosh._writers[t.name] = t.on_write end
  kawoosh._open_scratch(t.name, t.text or "", t.on_write ~= nil, t.read_only or false, t.language,
    t.reuse, t.line)
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
  local ok, tree = pcall(fn, { pane = pane, focused = params.focused, width = params.width,
                              height = params.height, env = env, name = name })
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
  local h = kawoosh._handlers[name]
  if h then
    local ok, err = pcall(h, ev)
    if not ok then kawoosh.echo("view `" .. name .. "`: " .. tostring(err)) end
  end
end

-- Called from Rust for a key in a focused Lua pane.
function kawoosh._key(name, ev)
  local h = kawoosh._handlers[name]
  if h then
    local ok, err = pcall(h, ev)
    if not ok then kawoosh.echo("view `" .. name .. "`: " .. tostring(err)) end
  end
end

-- Called from Rust when a command registered here runs.
function kawoosh._run(name, ctx)
  local fn = kawoosh._commands[name]
  if not fn then return end
  local ok, err = pcall(fn, ctx)
  if not ok then kawoosh.echo("command `" .. name .. "`: " .. tostring(err)) end
end

-- Called from Rust when a scratch buffer with an on_write is written.
function kawoosh._write(name, lines)
  local fn = kawoosh._writers[name]
  if not fn then return end
  local ok, err = pcall(fn, lines)
  if not ok then kawoosh.echo("write `" .. name .. "`: " .. tostring(err)) end
end
