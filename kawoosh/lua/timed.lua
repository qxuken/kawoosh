-- Timed rows (roadmap step 21): a notes buffer whose every new line
-- begins with the time it was started. `:timed` turns it on for the
-- buffer — `:timed clock` (the default: `14:02 `), `:timed relative`
-- (`T+01:41 `, from the buffer's first stamp), `:timed off` — and the
-- line the caret is on gets a stamp at once if it is empty.
--
-- The stamp is the line's text, not an annotation: it is saved with the
-- file and edited like the rest. A relative stamp counts from the
-- first stamp in the buffer, read back out of the text — the first
-- line of a relative buffer is stamped with the date and the time
-- (`2026-09-23 14:02 `), so a file reopened tomorrow counts on from
-- where it began. `timed.clock` is the clock stamp's `os.date` format.
--
-- `<CR>` in insert mode and `o` `O` in normal mode stamp the line they
-- open; in a buffer that is not timed they pass the key on
-- (`kawoosh.pass()`) to the binding under them — pairs' `<CR>`, the
-- engine's `o` — so the two plugins share the key.

local M = {}
kawoosh.timed = M

-- The buffers that are timed, and how: `"clock"` or `"relative"`.
M.on = {}

kawoosh.setting("timed.clock", { type = "string", doc = "the stamp's `os.date` format, `%H:%M` by default" })

local function clock_format()
  local f = kawoosh.opt("timed.clock")
  return (type(f) == "string" and f ~= "") and f or "%H:%M"
end

-- The first stamp's time in the buffer: a `YYYY-MM-DD HH:MM` at the
-- start of a line, the earliest line that has one.
local function start_of(lines)
  for _, l in ipairs(lines) do
    local y, mo, d, h, mi = l:match("^(%d%d%d%d)%-(%d%d)%-(%d%d) (%d%d):(%d%d)")
    if y then
      return os.time { year = tonumber(y), month = tonumber(mo), day = tonumber(d),
                       hour = tonumber(h), min = tonumber(mi), sec = 0 }
    end
  end
end

-- The stamp a new line in buffer `h` starts with, now.
function M.stamp(h, now)
  now = now or os.time()
  local how = M.on[h]
  if how == "relative" then
    local start = start_of(kawoosh.buf.lines(h))
    if not start then return os.date("%Y-%m-%d %H:%M ", now) end
    local mins = math.max(0, math.floor((now - start) / 60))
    return string.format("T+%02d:%02d ", mins // 60, mins % 60)
  end
  return os.date(clock_format(), now) .. " "
end

local function current() return kawoosh.buf.current() end

kawoosh.command("timed", function(ctx)
  local h = current()
  local how = ctx.args[1] or "clock"
  if how == "off" then
    M.on[h] = nil
    return kawoosh.echo("timed rows off")
  end
  if how ~= "clock" and how ~= "relative" then
    return kawoosh.echo("timed: clock, relative or off")
  end
  M.on[h] = how
  -- An empty line under the caret is stamped at once, the caret left
  -- after the stamp (`A` goes on writing).
  if (kawoosh.buf.line(kawoosh.buf.cursor().line) or "") == "" then
    local at = kawoosh.buf.selections()[1].head
    local stamp = M.stamp(h)
    kawoosh.buf.edits({ { at, at, stamp } })
    kawoosh.buf.set_selections({ { at + #stamp - 1, at + #stamp - 1, primary = true } })
  end
  kawoosh.echo("timed rows: " .. how)
end, { args = { "text" }, doc = "each new line of this buffer starts with the time: clock, relative, or off (timed.lua)" })

-- The keys: each stamps the line it opens in a timed buffer and passes
-- the key on in any other.
kawoosh.command("timed newline", function()
  local h = current()
  if not M.on[h] then return kawoosh.pass() end
  kawoosh.cmd("insert newline")
  kawoosh.buf.type(M.stamp(h))
end, { when = { "!prompt", "!field" }, doc = "a new line, stamped, in a timed buffer (timed.lua)" })

for key, open in pairs { o = "open below", O = "open above" } do
  kawoosh.command("timed " .. open, function()
    local h = current()
    if not M.on[h] then return kawoosh.pass() end
    kawoosh.cmd(open)
    kawoosh.buf.type(M.stamp(h))
  end, { doc = "`" .. key .. "`, the line stamped in a timed buffer (timed.lua)" })
  kawoosh.map("n", key, "timed " .. open)
end
kawoosh.map("i", "<CR>", "timed newline", { when = { "!prompt", "!field" } })

return M
