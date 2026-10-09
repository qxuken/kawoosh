-- `words.c` in Lua, written as a Lua plugin would be: the same commands
-- under `lwords*`, the same output, the same pane's tree, so
-- `tests/lua_costs.rs` (`native_vs_lua`) can time the two at one job and
-- assert they agree. Each command echoes its own clock, phase by phase.
local clock = kawoosh._clock or os.clock
local TOP, ROWS = 100, 200
local VIEW = "lwords"

local ranked, most, selected = {}, 1, 1

local function ms(a, b) return (b - a) * 1e3 end

kawoosh.command("lwords", function()
  local t0 = clock()
  local text = kawoosh.buf.text()
  local t1 = clock()
  local counts, words = {}, 0
  for w in text:gmatch("[%a_][%w_]*") do
    counts[w] = (counts[w] or 0) + 1
    words = words + 1
  end
  local t2 = clock()
  local all = {}
  for w in pairs(counts) do all[#all + 1] = w end
  table.sort(all, function(a, b)
    local ca, cb = counts[a], counts[b]
    if ca ~= cb then return ca > cb end
    return a < b
  end)
  local t3 = clock()
  ranked = {}
  for i = 1, math.min(ROWS, #all) do ranked[i] = { word = all[i], count = counts[all[i]] } end
  most, selected = ranked[1] and ranked[1].count or 1, 1
  local out = {}
  for i = 1, math.min(TOP, #all) do out[i] = counts[all[i]] .. " " .. all[i] end
  kawoosh.buf.open_scratch { name = "lwords", text = table.concat(out, "\n"), read_only = true }
  local t4 = clock()
  kawoosh.echo(string.format("read %.3f count %.3f sort %.3f out %.3f total %.3f ms; %d words, %d distinct",
    ms(t0, t1), ms(t1, t2), ms(t2, t3), ms(t3, t4), ms(t0, t4), words, #all))
end)

kawoosh.command("lwords_rename", function(ctx)
  local old, new = ctx.args[1], ctx.args[2]
  local t0 = clock()
  local text = kawoosh.buf.text()
  local t1 = clock()
  local edits, pat, at = {}, "%f[%w_]" .. old .. "%f[^%w_]", 1
  while true do
    local s, e = text:find(pat, at)
    if not s then break end
    edits[#edits + 1] = { s - 1, e, new }
    at = e + 1
  end
  local t2 = clock()
  kawoosh.buf.edits(edits)
  local t3 = clock()
  kawoosh.echo(string.format("read %.3f find %.3f queue %.3f total %.3f ms; %d edits",
    ms(t0, t1), ms(t1, t2), ms(t2, t3), ms(t0, t3), #edits))
end, { args = { "text", "text" } })

-- The rename as a Lua author tuning it would write it: a plain find (no
-- pattern), the word's edges checked by byte.
local function word_byte(b)
  return b and (b == 95 or (b >= 48 and b <= 57) or (b >= 65 and b <= 90) or (b >= 97 and b <= 122))
end
kawoosh.command("lwords_rename_fast", function(ctx)
  local old, new = ctx.args[1], ctx.args[2]
  local t0 = clock()
  local text = kawoosh.buf.text()
  local t1 = clock()
  local edits, n, at = {}, #old, 1
  local byte, find = string.byte, string.find
  while true do
    local s = find(text, old, at, true)
    if not s then break end
    local e = s + n - 1
    if not word_byte(byte(text, s - 1)) and not word_byte(byte(text, e + 1)) then
      edits[#edits + 1] = { s - 1, e, new }
      at = e + 1
    else
      at = s + 1
    end
  end
  local t2 = clock()
  kawoosh.buf.edits(edits)
  local t3 = clock()
  kawoosh.echo(string.format("read %.3f find %.3f queue %.3f total %.3f ms; %d edits",
    ms(t0, t1), ms(t1, t2), ms(t2, t3), ms(t0, t3), #edits))
end, { args = { "text", "text" } })

kawoosh.command("lwords_pane", function() kawoosh.view_open(VIEW) end)
kawoosh.command("lwords_next", function()
  if #ranked > 0 then selected = selected % #ranked + 1 end
end)

kawoosh.view(VIEW, function(ctx)
  local t = ctx.env.theme
  local mono = { size = 13, color = t.fg, family = "mono", wrap = "none" }
  local faint = { size = 13, color = t.faint, family = "mono", wrap = "none" }
  local body = column { key = "body", width = "grow", height = "grow", pad = 12, gap = 2, bg = t.bg, scroll_y = true,
    text(string.format("%d words ranked, row %d", #ranked, selected), { size = 12, color = t.muted }) }
  for i, e in ipairs(ranked) do
    body[#body + 1] = row { width = "grow", pad = { x = 8, y = 2 }, gap = 8, cross_align = "center", radius = 3,
      bg = i == selected and t.selection or nil,
      row { width = 64, text(tostring(e.count), faint) },
      row { width = "grow", text(e.word, mono) },
      row { width = 160, height = 8, bg = t.sunken, radius = 2,
        row { width = 160 * e.count / most, height = "grow", bg = t.accent, radius = 2 } } }
  end
  return body
end)
