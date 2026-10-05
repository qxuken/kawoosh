-- The picture pane: `:image [PATH]` (the focused buffer's file) opens a
-- column of its own over a picture — a still (PNG, JPEG, WebP, BMP),
-- one that moves (GIF, APNG, animated WebP) or a drawing (SVG) — and a
-- file named as a still or a moving one opens here instead of as bytes
-- (`image.open`). A drawing is text first: it opens as a buffer, and
-- `:image` from it shows it beside its source, read again at each save.
--
-- The picture is fitted to the pane — never drawn larger than it is,
-- but for a drawing, which has no pixels to run out of. `+` and `-`
-- zoom about the pane's middle, `0` is its own size, `f` fits it
-- again, `:image zoom 250` says a percent; `h` `j` `k` `l` move over
-- what does not fit, as the wheel and a drag do. Past twice its size a
-- picture's pixels are drawn as squares; a drawing is drawn again at
-- the size it is shown, sharp at any zoom its pixels have room for.
--
-- One that moves plays: `p` holds it and lets it go, `.` and `,` step
-- a frame on and back. `b` puts a light or a dark ground under
-- the picture, for the one drawn in the colour of the pane; `n` and
-- `N` go to the next picture of the folder and the one before; `r`
-- reads the file again (it is read again by itself when it changes);
-- `t` opens it as what it is under the picture, text or bytes; `q`
-- closes.
--
-- Hackable: `kawoosh.picture` — `open(path)`, `state([pane])`,
-- `panes()`, `is_picture(path)`, `stills`, `drawings` — over
-- `kawoosh.image(path, opts)`, which any view draws a picture by; and
-- the settings `image.open`, `image.backdrop`, `image.max_mb`.

local fs = kawoosh.fs

local VIEW = "image"
local PANE_FACT = "lua:" .. VIEW
local SHARE = 0.5
-- A step of `+` and `-`, and how far zoom goes: a picture's pixel
-- sixty-four across, the whole of it eight.
local STEP = 1.25
local MAX_ZOOM = 64
local MIN_SIDE = 8
-- Past this a picture's pixels are squares, not a blur.
local SQUARE = 2
-- Pixels a drawing is drawn at for each of the pane's.
local DENSITY = 2

kawoosh.setting("image.open", {
  type = "boolean",
  doc = "open a picture file (png, jpg, gif, webp, bmp) in the picture pane, `:image`, rather than as bytes; on by default. An svg opens as text either way",
})
kawoosh.setting("image.backdrop", {
  type = "string",
  doc = "what the picture pane puts under a picture: `none` (the pane's own colour, the default), `light` or `dark`; `b` in the pane changes it for that pane",
})
kawoosh.setting("image.max_mb", {
  type = "integer",
  doc = "a picture file larger than this many megabytes is not read by a pane or a preview; 64 by default",
})

local picture = {}
kawoosh.picture = picture

-- The files that open here by their name, and the ones `:image` draws
-- that open as text.
picture.stills = { png = true, apng = true, jpg = true, jpeg = true, gif = true, webp = true, bmp = true }
picture.drawings = { svg = true, svgz = true }

local function ext_of(path)
  local ext = tostring(path):match("%.([%w]+)$")
  return ext and ext:lower() or nil
end

-- picture.is_picture(path): a still, a moving one or a drawing, by its
-- name — what a preview draws rather than reads.
function picture.is_picture(path)
  local ext = ext_of(path)
  return ext ~= nil and (picture.stills[ext] == true or picture.drawings[ext] == true)
end

-- A pane's state: the file, `zoom` (the pane's pixels for one of the
-- picture's; nil fitted), the point of the picture at the pane's
-- middle (`cx`, `cy`, 0 to 1), `paused`, `want` (a frame asked for and
-- not shown yet), `backdrop`, the file's stamp as last read, and what
-- the last frame drew (`shown`). One a pane, by its id.
local states = {}
local S = nil
local last = nil
-- A file `:image` asked for, taken by the next draw of the pane with
-- the keyboard: the one it opened, or the one of this tab it focused.
local pending = nil
-- Paths `image source` opens as what they are: the opener lets each by
-- once.
local plain = {}

local function pane_of(slot)
  return tonumber(tostring(slot or ""):match("@(%d+)$"))
end

local function stamp_of(path)
  local ok, st = pcall(fs.stat, path)
  if ok and st and not st.is_dir then return tostring(st.size) .. ":" .. tostring(st.modified), st.size end
  return nil
end

-- The files on show are watched: one changed is read again.
local function rewatch()
  local seen, paths = {}, {}
  for _, st in pairs(states) do
    if not seen[st.path] then
      seen[st.path] = true
      paths[#paths + 1] = st.path
    end
  end
  if #paths == 0 then return fs.watch("image", nil) end
  -- The draw that follows sees the stamp moved and asks for it again.
  fs.watch("image", paths, function() end)
end

local function start(pane, path)
  local keep = states[pane]
  if keep and keep.path == path then
    S = keep
    return
  end
  S = { path = path, cx = 0.5, cy = 0.5, backdrop = keep and keep.backdrop or nil }
  states[pane] = S
  rewatch()
end

local function human(n)
  if n < 1024 then return n .. " B" end
  local units = { "KB", "MB", "GB", "TB" }
  local v, i = n / 1024, 1
  while v >= 1024 and i < #units do v, i = v / 1024, i + 1 end
  return string.format(v < 10 and "%.1f %s" or "%.0f %s", v, units[i])
end

local function seconds(ms)
  return string.format(ms < 10000 and "%.1f s" or "%.0f s", ms / 1000)
end

-- The zoom that fits a picture `w` by `h` in `vw` by `vh`: a drawing
-- to the pane, pixels never past their own size.
local function fitted(w, h, vw, vh, vector)
  local z = math.min(vw / w, vh / h)
  if not vector then z = math.min(1, z) end
  return math.max(z, 1e-6)
end

-- A zoom kept where a picture can be seen and found again.
local function bounded(z, w, h)
  local least = math.min(1, MIN_SIDE / math.max(w, h))
  return math.max(least, math.min(MAX_ZOOM, z))
end

local GROUNDS = { light = 0xF4F4F4FF, dark = 0x161616FF }
local function ground()
  local b = S.backdrop or kawoosh.opt("image.backdrop")
  return GROUNDS[b] and b or "none"
end

-- ---------------------------------------------------------------- the view

kawoosh.view(VIEW, function(ctx)
  local env = ctx.env
  local t = env.theme
  local m = ctx.metrics
  if pending and ctx.focused then
    start(ctx.pane, pending)
    pending = nil
  end
  S = states[ctx.pane]
  if ctx.focused then last = ctx.pane end
  if not S then return column { width = "grow", height = "grow", bg = t.bg } end

  -- The file as it is now: one that changed is read again.
  local stamp, size = stamp_of(S.path)
  local reload = S.reload or (S.stamp ~= nil and stamp ~= nil and stamp ~= S.stamp)
  S.reload = nil
  if stamp then S.stamp = stamp end

  -- The room the picture has: what the last frame laid out (a node
  -- that asks for its layout is one `layout_of` knows).
  local key = "image stage " .. ctx.pane
  local box = env.layout_of(key)
  local vw = math.max(1, box and box.w or (ctx.width or 400) - 24)
  local vh = math.max(1, box and box.h or (ctx.height or 300) - 110)

  local img, why = kawoosh.image(S.path)
  local stage, facts, note = nil, nil, nil
  if img then
    local w, h = img.width, img.height
    local fit = fitted(w, h, vw, vh, img.vector)
    local z = S.zoom and bounded(S.zoom, w, h) or fit
    local dw, dh = w * z, h * z
    -- The point at the middle, held where the picture has an edge to
    -- show: one that fits an axis sits in its middle.
    local function place(c, d, v)
      if d <= v then return 0.5, (v - d) / 2 end
      local off = math.max(v - d, math.min(0, v / 2 - c * d))
      return (v / 2 - off) / d, off
    end
    local dx, dy
    S.cx, dx = place(S.cx, dw, vw)
    S.cy, dy = place(S.cy, dh, vh)

    -- The frame, the width and the file asked for, of the one picture
    -- every pane of the file shares.
    local moves = img.frames > 1
    local want = S.want
    if want and want == img.frame then S.want, want = nil, nil end
    kawoosh.image(S.path, {
      play = moves and not S.paused and not want,
      frame = want,
      width = img.vector and math.ceil(dw * DENSITY) or nil,
      reload = reload or nil,
    })

    -- In the flow, in a scroller this view sets the offset of: the
    -- picture's room with air about it where it is the smaller, the
    -- pane's offset over it where it is the larger. (A float would
    -- draw over a dialog.)
    local back = ground()
    env.set_scroll(key, math.floor(math.max(0, -dx) + 0.5), math.floor(math.max(0, -dy) + 0.5))
    stage = column {
      width = math.max(vw, math.floor(dw + 0.5)), height = math.max(vh, math.floor(dh + 0.5)),
      pad = { l = math.floor(math.max(0, dx) + 0.5), t = math.floor(math.max(0, dy) + 0.5) },
      row { key = key .. " picture", on_layout = { kind = "laid" }, bg = GROUNDS[back],
        image { id = img.id, width = math.max(1, math.floor(dw + 0.5)), height = math.max(1, math.floor(dh + 0.5)),
                sampling = (not img.vector and z >= SQUARE) and "nearest" or "linear",
                label = fs.basename(S.path) } },
    }
    local drawn = env.layout_of(key .. " picture")
    S.shown = { drawn_x = drawn and box and drawn.x - box.x, drawn_y = drawn and box and drawn.y - box.y,
                width = w, height = h, format = img.format, frames = img.frames, frame = img.frame,
                vector = img.vector, zoom = z, fit = fit, fitted = S.zoom == nil,
                x = dx, y = dy, w = dw, h = dh, room_w = vw, room_h = vh,
                pixel_width = img.pixel_width, pixel_height = img.pixel_height }

    facts = { w .. " × " .. h, img.format }
    if size then facts[#facts + 1] = human(size) end
    facts[#facts + 1] = string.format("%d%%", math.floor(z * 100 + 0.5)) .. (S.zoom == nil and " fitted" or "")
    if moves then
      facts[#facts + 1] = "frame " .. img.frame .. " of " .. img.frames .. (img.more and "+" or "")
      facts[#facts + 1] = seconds(img.duration)
      if S.paused then facts[#facts + 1] = "held" end
    end
    if not img.vector and img.pixel_width ~= w then
      note = "too large to draw whole: shown from " .. img.pixel_width .. " × " .. img.pixel_height .. " pixels"
    elseif img.more then
      note = "too many frames to keep: the first " .. img.frames .. " play"
    end
  else
    S.shown = nil
    if why and reload then kawoosh.image(S.path, { reload = true }) end
    local said = why and ("not a picture to draw: " .. why) or (stamp and "reading…" or "not there to read")
    stage = column { width = "grow", height = "grow", center = true,
      text(said, { size = m.small, color = why and t.danger or t.muted }) }
    facts = { size and human(size) or "" }
  end

  local head = column { width = "grow", gap = 4, pad = { x = 12, t = 10 },
    row { width = "grow", gap = 8, cross_align = "center",
      text({ { "picture", bold = true } }, { size = m.text, color = t.fg, wrap = "none" }),
      row { width = "grow", min_width = 0,
        text(fs.short and fs.short(S.path) or S.path, { family = "mono", size = m.text, color = t.accent, ellipsis = true }) } },
    text(table.concat(facts, " · "), { size = m.small, color = t.muted, ellipsis = true }) }
  if note then head[#head + 1] = text(note, { size = m.small, color = t.warning, ellipsis = true }) end
  local keys = { { { "+", "-" }, "zoom" }, { "0", "its own size" }, { "f", "fits" },
      { { "h", "j", "k", "l" }, "move" } }
  if img and img.frames > 1 then
    keys[#keys + 1] = { "p", "holds, plays" }
    keys[#keys + 1] = { { ".", "," }, "a frame on, back" }
  end
  for _, k in ipairs { { "b", "ground" }, { { "n", "N" }, "next picture, back" }, { "r", "reads again" },
                       { "t", "as text or bytes" }, { "q", "closes" } } do
    keys[#keys + 1] = k
  end
  head[#head + 1] = ctx.legend(keys, { size = m.note })

  return column { width = "grow", height = "grow", bg = t.bg, gap = 8, clip = true,
    head,
    row { width = "grow", height = "grow", min_height = 0, pad = { x = 12, b = 10 },
      column { key = key, width = "grow", height = "grow", min_width = 0, min_height = 0,
               scroll_x = true, scroll_y = true, scrollbar = "hidden", on_layout = { kind = "laid" },
               cursor = (S.shown and (S.shown.w > vw or S.shown.h > vh)) and "grab" or nil,
               on_drag = { kind = "pan" }, on_scroll = { kind = "wheel" }, stage } } }
end, function(ev)
  if ev.kind == "key" then return false end
  S = states[pane_of(ev.slot)]
  if not S or not S.shown then return end
  local kind = ev.kind
  if (kind == "drag" or kind == "scroll") and type(ev.tag) == "table" then kind = ev.tag.kind end
  local d = S.shown
  if kind == "pan" then
    -- The picture goes with the pointer, from where it was pressed.
    if ev.phase == "start" then S.grab = { S.cx, S.cy } end
    if S.grab then
      S.cx = S.grab[1] - (ev.dx or 0) / d.w
      S.cy = S.grab[2] - (ev.dy or 0) / d.h
    end
    if ev.phase == "end" then S.grab = nil end
  elseif kind == "wheel" then
    S.cx = S.cx - (ev.dx or 0) / d.w
    S.cy = S.cy - (ev.dy or 0) / d.h
  end
end, {
  session = false,
  -- The file's directory: where `:terminal here` starts.
  here = function(pane) return states[pane] and fs.parent(states[pane].path) end,
})

-- ------------------------------------------------------------- the door

-- picture.open(path): the picture pane over `path`, opened or turned
-- to it.
function picture.open(path)
  path = fs.expand(path)
  if fs.is_dir(path) then return kawoosh.echo("a directory: " .. path) end
  if not stamp_of(path) then return kawoosh.echo("no such file: " .. path) end
  pending = path
  kawoosh.view_open(VIEW, { share = SHARE })
end

function picture.panes()
  local out = {}
  for p in pairs(states) do out[#out + 1] = p end
  table.sort(out)
  return out
end

-- picture.state([pane]): what a pane shows — `path`, `paused`,
-- `backdrop` (`"none"`, `"light"`, `"dark"`) and, once the picture is
-- read, `width`, `height`, `format`, `frames`, `frame`, `vector`,
-- `zoom`, `fitted`, where it is drawn in the pane's room (`x`, `y`,
-- `w`, `h`, `room_w`, `room_h`) and the pixels it was read to
-- (`pixel_width`, `pixel_height`) — or nil when it is not open: the
-- pane last drawn with the keyboard when none is named.
function picture.state(pane)
  local st = states[pane or last]
  if not st then return nil end
  local out = { path = st.path, paused = st.paused == true, backdrop = "none" }
  local keep = S
  S = st
  out.backdrop = ground()
  S = keep
  for k, v in pairs(st.shown or {}) do out[k] = v end
  return out
end

kawoosh.command("image", function(ctx)
  local path = ctx.args[1]
  if not path then
    local ok, p = pcall(kawoosh.buf.path)
    path = ok and p or nil
  end
  if not path then return kawoosh.echo("no file here: :image PATH") end
  picture.open(path)
end, {
  args = { "path" },
  doc = "a picture — PATH's, or the focused buffer's file's — in a pane: a still, one that moves, or an svg beside its source",
})

-- A command of the pane with the keyboard: `S` its state.
local function on(name, fn, doc, opts)
  opts = opts or {}
  opts.when, opts.doc = { PANE_FACT }, doc
  kawoosh.command("image " .. name, function(ctx)
    S = states[ctx.pane]
    if S then fn(ctx) end
  end, opts)
end
local function times(ctx) return math.max(1, ctx.count or 1) end

-- The zoom now, fitted or said.
local function zoom_now()
  local d = S.shown
  return d and d.zoom or nil
end
local function zoom_to(z)
  local d = S.shown
  if not d then return end
  S.zoom = bounded(z, d.width, d.height)
end

on("zoom in", function(ctx)
  local z = zoom_now()
  if z then zoom_to(z * STEP ^ times(ctx)) end
end, "the picture a step larger, about the pane's middle")
on("zoom out", function(ctx)
  local z = zoom_now()
  if z then zoom_to(z / STEP ^ times(ctx)) end
end, "the picture a step smaller")
on("actual", function() zoom_to(1) end, "the picture at its own size: a pixel a pixel")
on("fit", function() S.zoom, S.cx, S.cy = nil, 0.5, 0.5 end,
  "the picture fitted to the pane, never larger than it is")
on("zoom", function(ctx)
  local said = table.concat(ctx.args, "")
  if said == "" then return kawoosh.cmdline("image zoom ") end
  if said == "fit" then S.zoom, S.cx, S.cy = nil, 0.5, 0.5 return end
  local n = tonumber((said:gsub("%%$", "")))
  if not n or n <= 0 then return kawoosh.echo("not a zoom: " .. said .. " (250, 50%, fit)") end
  zoom_to(n / 100)
end, "the picture at a percent of its own size (`250`, `50%`), or `fit`", { args = { "text..." } })

-- A move of an eighth of the pane, over what does not fit.
local function move(ax, sign)
  return function(ctx)
    local d = S.shown
    if not d then return end
    local by = sign * times(ctx) / 8
    if ax == "x" then S.cx = S.cx + by * d.room_w / d.w else S.cy = S.cy + by * d.room_h / d.h end
  end
end
on("left", move("x", -1), "what is left of the pane's into it")
on("right", move("x", 1), "what is right of the pane's into it")
on("up", move("y", -1), "what is above the pane's into it")
on("down", move("y", 1), "what is below the pane's into it")
local function half(sign)
  return function(ctx)
    local d = S.shown
    if d then S.cy = S.cy + sign * times(ctx) * d.room_h / 2 / d.h end
  end
end
on("half down", half(1), "half a pane down the picture")
on("half up", half(-1), "half a pane up the picture")

on("play", function() S.paused = not S.paused or nil end, "hold a picture that moves, or let it go")
local function step(by)
  return function(ctx)
    local d = S.shown
    if not d or d.frames < 2 then return end
    S.paused = true
    local from = S.want or d.frame
    S.want = (from - 1 + by * times(ctx)) % d.frames + 1
  end
end
on("next frame", step(1), "hold a picture that moves a frame on")
on("previous frame", step(-1), "hold a picture that moves a frame back")

on("backdrop", function()
  local order = { none = "light", light = "dark", dark = "none" }
  S.backdrop = order[ground()]
end, "the ground under the picture: light, dark, the pane's own")
on("reload", function() S.reload = true end, "read the file again")
on("source", function()
  plain[S.path] = true
  kawoosh.open(S.path)
end, "open the file as what it is under the picture: text, or bytes")

-- The pictures beside this one, by name: the next, the one before,
-- round the folder's ends.
local function sibling(by)
  return function(ctx)
    local dir, name = fs.parent(S.path), fs.basename(S.path)
    local ok, entries = pcall(fs.list, dir)
    if not ok or not entries then return end
    local names = {}
    for _, e in ipairs(entries) do
      if not e.is_dir and picture.is_picture(e.name) then names[#names + 1] = e.name end
    end
    table.sort(names, function(a, b) return a:lower() < b:lower() end)
    local at = nil
    for i, n in ipairs(names) do
      if n == name then at = i break end
    end
    if #names == 0 or (at and #names == 1) then return kawoosh.echo("the only picture here") end
    at = ((at or (by > 0 and 0 or 1)) - 1 + by * times(ctx)) % #names + 1
    start(ctx.pane, fs.join(dir, names[at]))
  end
end
on("next", sibling(1), "the next picture of the folder, by name")
on("previous", sibling(-1), "the picture before in the folder")

on("close", function(ctx)
  states[ctx.pane], S = nil, nil
  rewatch()
  kawoosh.view_close(VIEW)
end, "close the pane")

for k, c in pairs {
  ["+"] = "zoom in", ["="] = "zoom in", ["-"] = "zoom out", ["0"] = "actual", f = "fit",
  h = "left", l = "right", j = "down", k = "up",
  ["<Left>"] = "left", ["<Right>"] = "right", ["<Down>"] = "down", ["<Up>"] = "up",
  ["<C-d>"] = "half down", ["<C-u>"] = "half up", ["<PageDown>"] = "half down", ["<PageUp>"] = "half up",
  p = "play", ["."] = "next frame", [","] = "previous frame",
  b = "backdrop", r = "reload", t = "source", n = "next", N = "previous",
  q = "close",
} do
  kawoosh.map("p", k, "image " .. c, { view = VIEW })
end

-- A still or a moving picture opens here by its name (`image.open`),
-- once `image source` has not asked for it as it is; a drawing is
-- text, and opens as text.
kawoosh.on_open(function(path)
  if plain[path] then
    plain[path] = nil
    return false
  end
  if kawoosh.opt("image.open") == false then return false end
  local ext = ext_of(path)
  if not ext or not picture.stills[ext] or fs.is_dir(path) then return false end
  picture.open(path)
  return true
end)

return picture
