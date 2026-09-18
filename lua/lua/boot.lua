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
-- keymap, the command line, or Rust. A name of two words is a
-- subcommand (`"oil cd"` runs as `:oil cd`, completes under `:oil`).
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
--   when    facts that must hold, `{ "language:oil", "!terminal" }` —
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

-- kawoosh.map(mode, keys, cmd[, opts]): `cmd` is a command name (with
-- args, as the command line would spell it) or a function, which
-- becomes one. A key can be bound more than once: the newest binding
-- whose `opts.when` holds and whose command can run is the one that
-- runs, so `map("n", "<CR>", "goto location", { when = { "!language:oil" } })`
-- and then `map("n", "<CR>", "oil enter")` (a command gated on the
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
                height = params.height, env = env, name = name }
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
