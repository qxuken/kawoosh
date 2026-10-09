-- Manual pages (docs/design/man.md): `:man PAGE`, `:man SECTION PAGE`
-- or `:man PAGE(SECTION)` reads the system's `man` into a read-only
-- buffer — `*man ls(1)*` — in the pane the keys are in, rendered to
-- that pane's width (`man.width` for one of your own), its bold and
-- underline kept as the buffer's paints: section heads in the keyword
-- colour, a reference to another page (`chmod(1)`) in the link colour.
-- Windows has no `man`; `man.command = "wsl man"` reads WSL's.
-- `:man` alone takes the word under the caret, and with none the
-- picker over every page `man -k` knows (`:man pick`, `picker man`).
--
-- `:man beside` (`<leader>ik`; `<leader>iK` is `:man`) takes the same
-- words and reads the page into the man column instead: the pane of
-- the tab in front that shows a page already, else a column of its own
-- made beside (man.md Decision 6), the page rendered to that column's
-- width once it is laid out, the keys going to it.
--
-- In a page: `K` or `<CR>` follows the reference under the caret (the
-- word there when it is none) into the same pane — the man column's
-- page stays in the man column — `<C-o>` is the way back, `]]` `[[` go
-- to the next and previous section head, `q` closes the page (`:bd`),
-- and a column `:man beside` made with it. The buffer is a buffer: `/`
-- finds, `y` yanks, `:w` is refused.
--
-- Hackable: `kawoosh.man` — `open(page, section, opts)`, `render(raw)`
-- (the text with its overstrikes read off, and the spans they styled),
-- `parse(words)`, `reference_at(line, col)`, `pages()` (what is open),
-- `index(fn)` (every page, from `man -k .`, cached for the session),
-- `column(from)` (the man column's pane) — and the settings
-- `man.command`, `man.width`.

local fs = kawoosh.fs
local picker = kawoosh.picker

local LANG = "man"
local SET = "man"
-- Narrower than this and a page is unreadable: the floor on the width.
local MIN_WIDTH = 40
local DEFAULT_WIDTH = 80

kawoosh.setting("man.command", {
  type = "string",
  doc = "the manual reader, `man` by default; a line, split on spaces (`env MANPATH=/opt/man man`)",
})
kawoosh.setting("man.width", {
  type = "integer",
  doc = "columns a page is rendered to; `0`, the default, is the width of the pane it opens in",
})

-- A language of its own, so the page's keys are local to it
-- (docs/design/local-maps.md) and `:set` says what it is; no grammar,
-- no files of its own.
kawoosh.language(LANG, {})

local man = {}
kawoosh.man = man

-- What is open, by buffer name: `page`, `section`, `width`, and
-- `sections` — `{ line =, offset = }` of each section head, for `]]`.
local open = {}
-- `man -k .`, parsed, once a session (`man.index`).
local index = nil
-- The buffer a column `:man beside` makes shows while its first page is
-- read: the column is made at once, so it is laid out — and its width
-- known — by the time the page is.
local PENDING = "*man*"
-- The panes `:man beside` made, this session: `q` in a page there
-- closes the column with the page, which was made for it.
local columns = {}
-- How often a page for the man column is read again when the column
-- turns out another width than it was read at.
local REFITS = 3

-- --------------------------------------------------------------- reading

-- The command as argv: `man.command` split on spaces, `man` by default.
local function command()
  local line = kawoosh.opt("man.command")
  if type(line) ~= "string" or line:match("^%s*$") then line = "man" end
  local argv = {}
  for w in line:gmatch("%S+") do argv[#argv + 1] = w end
  return argv
end

-- The environment a page is rendered under: no pager, the overstrikes
-- kept (groff's `MAN_KEEP_FORMATTING`, and no SGR in their place), the
-- width asked for. `WSLENV` names them all, or `wsl man` hears none of
-- them and prints 80 columns with no bold.
local function env_for(width)
  return {
    MANPAGER = "cat", PAGER = "cat", MAN_KEEP_FORMATTING = "1", GROFF_NO_SGR = "1",
    MANWIDTH = tostring(width), COLUMNS = tostring(width),
    WSLENV = "MANPAGER:PAGER:MAN_KEEP_FORMATTING:GROFF_NO_SGR:MANWIDTH:COLUMNS",
  }
end

-- man.render(raw): the page as `man` printed it, read into the text a
-- buffer holds — no backspace, no escape in it — and what was styled:
-- `spans`, `{ from, to, style }` each (bytes from 0, end exclusive;
-- `style` "bold", "underline" or "bold underline"), `heads`, the
-- section heads (`{ line =, offset = }`, lines from 1: a line all bold
-- from its first column), `subheads` the same indented, `refs`, the
-- references to other pages (`{ from, to, page, section }`), and
-- `header`/`footer`, the byte ranges of the page's first and last
-- lines when they name it (`LS(1) … LS(1)`).
function man.render(raw)
  -- The overstrikes and SGR read off by the engine (`kawoosh.overstrike`):
  -- the text, its styled runs, and its all-bold lines.
  local text, spans, bold = kawoosh.overstrike(raw)
  local heads, subheads, refs = {}, {}, {}
  local plain_lines = {}
  local offset = 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    local ln = #plain_lines + 1
    plain_lines[ln] = line
    -- A line all bold is a head: from its first column a section's, an
    -- indented one a subsection's.
    if bold[ln] then
      local head = { line = ln, offset = offset }
      if line:match("^%S") then heads[#heads + 1] = head else subheads[#subheads + 1] = head end
    end
    -- References: `name(1)`, `name(3p)`, `name(n)` — a word and a
    -- section in brackets, right after it; looked for on a line that has
    -- a `(` before a digit, the pattern's backtracking spared the rest.
    local pos = line:find("%(%d") and 1 or #line + 1
    while true do
      local a, b, page, section = line:find("([%w_%-%.:+]+)%((%d[%w]*)%)", pos)
      if not a then break end
      -- `foo(...)` is a call, not a page: sections are short.
      if #section <= 4 then
        refs[#refs + 1] = { offset + a - 1, offset + b, page = page, section = section }
      end
      pos = b + 1
    end
    offset = offset + #line + 1
  end
  if text == "" then plain_lines = {} end
  local header, footer
  if plain_lines[1] and plain_lines[1]:match("^%S+%(%S+%)") then
    header = { 0, #plain_lines[1] }
    -- The first line's name is no reference.
    local kept = {}
    for _, r in ipairs(refs) do if r[1] >= header[2] then kept[#kept + 1] = r end end
    refs = kept
  end
  local last = #plain_lines
  while last > 1 and plain_lines[last] == "" do last = last - 1 end
  if last > 1 and plain_lines[last]:match("%S+%(%S+%)%s*$") then
    local from = 0
    for i = 1, last - 1 do from = from + #plain_lines[i] + 1 end
    footer = { from, from + #plain_lines[last] }
    local kept = {}
    for _, r in ipairs(refs) do if r[2] <= footer[1] then kept[#kept + 1] = r end end
    refs = kept
  end
  return text, { spans = spans, heads = heads, subheads = subheads, refs = refs, header = header, footer = footer }
end

-- The paints a rendered page gets: references in the link colour first
-- (the first paint over a span is its colour; the markdown buffer's
-- links are drawn in it), the heads bold in the keyword colour, the
-- styled spans as they were, the header and footer dim.
local function paints_of(text, info)
  local p = {}
  for _, r in ipairs(info.refs) do p[#p + 1] = { r[1], r[2], "link" } end
  local function line_end(from)
    local nl = text:find("\n", from + 1, true)
    return nl and nl - 1 or #text
  end
  for _, h in ipairs(info.heads) do p[#p + 1] = { h.offset, line_end(h.offset), "bold keyword" } end
  for _, h in ipairs(info.subheads) do p[#p + 1] = { h.offset, line_end(h.offset), "bold keyword" } end
  for _, s in ipairs(info.spans) do p[#p + 1] = { s[1], s[2], s[3] } end
  if info.header then p[#p + 1] = { info.header[1], info.header[2], "dim" } end
  if info.footer then p[#p + 1] = { info.footer[1], info.footer[2], "dim" } end
  return p
end

-- --------------------------------------------------------------- naming

-- man.parse(words): what `:man`'s arguments name — `{ "ls" }`,
-- `{ "1", "ls" }`, `{ "ls(1)" }`, `{ "ls.1" }` — as the page and its
-- section (nil for none); nil for nothing.
function man.parse(words)
  if type(words) == "string" then
    local w = {}
    for x in words:gmatch("%S+") do w[#w + 1] = x end
    words = w
  end
  if #words == 0 then return nil end
  if #words >= 2 and words[1]:match("^%d[%w]*$") then
    return words[2], words[1]
  end
  local page = words[1]
  local name, section = page:match("^(.-)%((%w+)%)$")
  if name and name ~= "" then return name, section end
  name, section = page:match("^(.-)%.(%d[%w]*)$")
  if name and name ~= "" then return name, section end
  return page, nil
end

local function buffer_name(page, section)
  return "*man " .. page .. (section and ("(" .. section .. ")") or "") .. "*"
end

-- The page a buffer name says, or nil.
local function named(name)
  local page, section = name:match("^%*man (.-)%((%w+)%)%*$")
  if page then return page, section end
  page = name:match("^%*man (.-)%*$")
  return page, nil
end

-- man.reference_at(line, col): the reference `name(N)` the character
-- at `col` (from 1, in characters) is on, as `page, section`; else the
-- word there, with no section; nil on a space.
function man.reference_at(line, col)
  local byte = utf8.offset(line, col) or (#line + 1)
  local pos = 1
  while true do
    local a, b, page, section = line:find("([%w_%-%.:+]+)%((%d[%w]*)%)", pos)
    if not a then break end
    if byte >= a and byte <= b and #section <= 4 then return page, section end
    pos = b + 1
  end
  -- The word: letters, digits, `_`, `-`, `.`, `:`, trimmed of the dots
  -- that end a sentence.
  local function is_word(i)
    local c = line:sub(i, i)
    return c ~= "" and c:match("[%w_%-%.:+]") ~= nil
  end
  if not is_word(byte) then return nil end
  local a, b = byte, byte
  while a > 1 and is_word(a - 1) do a = a - 1 end
  while b < #line and is_word(b + 1) do b = b + 1 end
  local word = line:sub(a, b):gsub("[%.:]+$", ""):gsub("^[%.:]+", "")
  if word == "" then return nil end
  -- A trailing `(` after the word: a call's name, as `K` in C reads it.
  return word, nil
end

-- The reference or word under the caret of the current buffer.
local function under_caret()
  local h = kawoosh.buf.current()
  if not h then return nil end
  local cur = kawoosh.buf.cursor(h)
  local line = kawoosh.buf.line(cur.line, h)
  if not line then return nil end
  return man.reference_at(line, cur.col)
end

local function in_page()
  local h = kawoosh.buf.current()
  return h ~= nil and open[kawoosh.buf.name(h)] ~= nil
end

-- --------------------------------------------------------------- opening

-- `man.width` when one is set, else nil.
local function set_width()
  local set = tonumber(kawoosh.opt("man.width")) or 0
  if set > 0 then return math.max(MIN_WIDTH, math.floor(set)) end
end

-- The width a page fits `pane` at — its columns less one — or nil while
-- the pane is not laid out (or is not an editor pane).
local function fitted(pane)
  local g = pane and kawoosh.pane_size(pane)
  if g and g.cols and g.cols > 0 then
    -- A line as wide as the pane lands on its last cell's edge; one
    -- less and it is whole.
    return math.max(MIN_WIDTH, g.cols - 1)
  end
end

-- The width a page opened from `pane` is rendered to: `man.width`, else
-- the pane's columns, else the classic eighty.
local function width_for(pane)
  return set_width() or fitted(pane) or DEFAULT_WIDTH
end

-- The buffer named `name`, or nil.
local function buffer_named(name)
  for _, h in ipairs(kawoosh.buf.list()) do
    local ok, n = pcall(kawoosh.buf.name, h)
    if ok and n == name then return h end
  end
end

local function is_page(h)
  if not h then return false end
  local ok, lang = pcall(kawoosh.buf.language, h)
  return ok and lang == LANG
end

-- man.column(from): the man column — the pane of the tab in front a
-- page is read into by `:man beside` — or nil for none: the column
-- being made (it shows `*man*`), else `from` (the pane the keys are
-- in) when it shows a page, else a column `:man beside` made that shows
-- one, else the first pane that does.
function man.column(from)
  local panes = kawoosh.panes()
  local mine, any
  for _, p in ipairs(panes) do
    if p.buffer then
      local ok, name = pcall(kawoosh.buf.name, p.buffer)
      if ok and name == PENDING then return p.pane end
    end
  end
  for _, p in ipairs(panes) do
    if is_page(p.buffer) then
      if p.pane == from then return p.pane end
      if columns[p.pane] and not mine then mine = p.pane end
      if not any then any = p.pane end
    end
  end
  return mine or any
end

-- The page's name and section as its header line says them (`LS(1)`),
-- the name in the case it was asked by.
local function header_of(text, page, section)
  local name, sec = text:match("^(%S+)%((%S+)%)")
  if not name then return page, section end
  if name:lower() == page:lower() then name = page end
  return name, sec
end

-- The rendered page into its buffer: shown in the focused pane unless
-- `opts.show == false` (a session's restore fills where it is), or in
-- pane `into` (`open_scratch`'s `pane`: a number, or `"column"`) — the
-- man column, whose `*man*` becomes the page.
local function fill(raw, page, section, width, opts, into)
  local text, info = man.render(raw)
  page, section = header_of(text, page, section)
  local name = buffer_name(page, section)
  -- The stand-in of the column the page goes to, when it is there.
  local pending = nil
  if into then
    local h = buffer_named(PENDING)
    for _, p in ipairs(kawoosh.panes()) do
      if h and p.pane == into and p.buffer == h then pending = h end
    end
  end
  -- A page open already is shown as it is refilled, and the column's
  -- stand-in is not needed: it goes once the page is in its place.
  local drop = pending and buffer_named(name) ~= nil
  kawoosh.buf.open_scratch { name = name, text = text, read_only = true, language = LANG, show = opts.show ~= false,
                             restore = true, pane = into, reuse = pending }
  if drop then kawoosh.buf.close(pending, { force = true }) end
  kawoosh.buf.paint(SET, paints_of(text, info), name)
  open[name] = { page = page, section = section, width = width, heads = info.heads }
  return name
end

-- man.open(page[, section][, opts]): the page read and shown in the
-- focused pane. `opts.pane` is the pane the keys are in (a command's
-- `ctx.pane`), whose width the page is rendered to; `opts.width` one
-- of your own; `opts.show = false` fills the buffer without showing
-- it; `opts.beside = true` reads it into the man column instead
-- (`man.column`, a column of its own made beside when there is none);
-- `opts.done(name)` is told the buffer's name, or `done(nil, why)`.
function man.open(page, section, opts)
  opts = opts or {}
  if type(page) ~= "string" or page == "" then return kawoosh.echo("man: which page?") end
  local fixed = opts.width or set_width()
  local target, made = nil, false
  if opts.beside then
    target = man.column(opts.pane)
    if not target then
      -- The column now, so it is laid out by the time the page is read
      -- and the page can be rendered to its width.
      made = true
      kawoosh.buf.open_scratch { name = PENDING, read_only = true, language = LANG, pane = "column",
                                 text = "reading " .. page .. (section and ("(" .. section .. ")") or "") .. " …" }
    end
  end
  local tries = 0
  local function read(width)
    tries = tries + 1
    local argv = command()
    if section then argv[#argv + 1] = section end
    argv[#argv + 1] = page
    local said = {}
    kawoosh.spawn(argv, {
      env = env_for(width),
      on_stderr = function(ls)
        for _, l in ipairs(ls) do if l ~= "" then said[#said + 1] = l end end
      end,
      on_done = function(text, code)
        if code ~= 0 or text == nil or text:match("^%s*$") then
          local why = said[1] or ("no manual entry for " .. page .. (section and (" in section " .. section) or ""))
          if code == nil then why = argv[1] .. ": not found" end
          kawoosh.echo(why)
          -- A column made for this page, and no page: the column goes
          -- again when the keys are still in it, and its stand-in.
          local h = made and buffer_named(PENDING)
          if h then
            for _, p in ipairs(kawoosh.panes()) do
              if p.buffer == h and p.pane == kawoosh.pane() then kawoosh.run("close") end
            end
            kawoosh.buf.close(h, { force = true })
          end
          if opts.done then opts.done(nil, why) end
          return
        end
        local into = nil
        if opts.beside then
          into = man.column(opts.pane)
          -- The column laid out at another width than the page was
          -- read at — one just made, whose width was not known when the
          -- page was asked for — reads it again at its own.
          local want = into and fitted(into)
          if not fixed and into and want ~= width and tries < REFITS then
            return read(want or width)
          end
          if into and made then columns[into] = true end
        end
        local name = fill(text, page, section, width, opts, opts.beside and (into or "column") or nil)
        if opts.done then opts.done(name) end
      end,
    })
  end
  -- The width of the pane it goes to: the man column's, the pane the
  -- keys are in; a column being made is not laid out yet (`man.width`,
  -- else eighty, and read again once it is).
  local into = opts.pane
  if opts.beside then into = target end
  read(fixed or width_for(into))
end

-- man.pages(): the pages open, by buffer name — `{ page =, section =,
-- width = }` each.
function man.pages()
  local there = {}
  for _, h in ipairs(kawoosh.buf.list()) do
    local ok, name = pcall(kawoosh.buf.name, h)
    if ok then there[name] = true end
  end
  local out = {}
  for name, p in pairs(open) do
    if there[name] then
      out[name] = { page = p.page, section = p.section, width = p.width }
    else
      open[name] = nil
    end
  end
  return out
end

-- A session brings a page back: rendered again, as wide as its lines
-- were (its pane is not known yet), filled where it is.
kawoosh.on_restore(function(name, h)
  local page, section = named(name)
  if not page then return end
  local width = 0
  for _, l in ipairs(kawoosh.buf.lines(h)) do
    local n = utf8.len(l) or #l
    if n > width then width = n end
  end
  if width < MIN_WIDTH then width = DEFAULT_WIDTH end
  man.open(page, section, { width = width, show = false })
end)

-- ----------------------------------------------------------------- index

-- man.index(fn): every page `man -k .` lists — `{ page =, section =,
-- doc = }` each, by name — handed to `fn(list)`, or `fn(nil, why)`;
-- read once a session unless `again`.
function man.index(fn, again)
  if index and not again then return fn(index) end
  local argv = command()
  argv[#argv + 1] = "-k"
  argv[#argv + 1] = "."
  kawoosh.spawn(argv, {
    env = env_for(DEFAULT_WIDTH),
    on_done = function(text, code)
      if code ~= 0 and (text == nil or text == "") then
        return fn(nil, argv[1] .. " -k: nothing listed")
      end
      local list, seen = {}, {}
      for line in text:gmatch("[^\n]+") do
        -- `ls(1), dir(1) - list directory contents`: every name on the
        -- left a page; the description after ` - `.
        local left, doc = line:match("^(.-)%s+%-%s+(.*)$")
        if left then
          for page, section in left:gmatch("([^%s,()]+)%(([^)]+)%)") do
            local key = page .. "(" .. section .. ")"
            if not seen[key] then
              seen[key] = true
              list[#list + 1] = { page = page, section = section, doc = doc }
            end
          end
        end
      end
      table.sort(list, function(a, b)
        local la, lb = a.page:lower(), b.page:lower()
        if la ~= lb then return la < lb end
        return a.section < b.section
      end)
      index = list
      fn(list)
    end,
  })
end

local COLUMNS = {
  { "text", family = "mono", min = 120, max = 280, share = 0.3 },
  { "doc", grow = true },
}

-- The pane the picker was opened from: where a pick is shown, whose
-- width it is rendered to — or, opened by `:man beside` on nothing,
-- the pane the man column is looked for from (`pick_beside`).
local from_pane = nil
local pick_beside = false

picker.source("man", {
  title = "manual pages", placeholder = "find a manual page",
  load = function(ctx, done)
    from_pane = ctx.pane
    man.index(function(list, why)
      if not list then return done(nil, why) end
      local items = {}
      for _, e in ipairs(list) do
        items[#items + 1] = { text = e.page .. "(" .. e.section .. ")", doc = e.doc, page = e.page, section = e.section }
      end
      done(items)
    end)
  end,
  columns = COLUMNS,
  preview = function(item)
    return { title = item.text, lines = { item.doc } }
  end,
  pick = function(item, how)
    -- Into a pane beside, below, a new tab: the pane made first, the
    -- page shown in it (`run`, so it lands before the page does); its
    -- width is not known yet, so `man.width` or the classic eighty.
    local split = ({ vsplit = "vsplit", split = "split", tab = "tab new" })[how]
    if split then
      kawoosh.run(split)
      return man.open(item.page, item.section, {})
    end
    -- `<CR>`: where the key that opened the picker would have put the
    -- page — the man column for `<leader>ik`, the pane for `:man`.
    man.open(item.page, item.section, { pane = from_pane, beside = pick_beside })
  end,
  empty = "no page matches",
})

-- -------------------------------------------------------------- commands

-- `:man [PAGE | SECTION PAGE | PAGE(SECTION)]`: the page; alone, the
-- word under the caret, else the picker. `beside`: into the man column.
local function man_command(ctx, beside)
  local page, section = man.parse(ctx.args)
  if not page then
    page, section = under_caret()
    if not page then
      pick_beside = beside
      return picker.open("man")
    end
  end
  man.open(page, section, { pane = ctx.pane, beside = beside })
end

kawoosh.command("man", function(ctx) man_command(ctx, false) end, {
  args = { "text..." },
  doc = "the manual page in this pane: `:man ls`, `:man 3 printf`, `:man printf(3)`; alone, the word under the caret, else the picker over every page",
})

-- `:man beside [PAGE…]` (`<leader>ik`): the same, into the man column —
-- the pane of the tab that shows a page, else a column of its own.
kawoosh.command("man beside", function(ctx) man_command(ctx, true) end, {
  args = { "text..." },
  doc = "the manual page in a column of its own, the one a page is in already if there is one: `:man beside ls`; alone, the word under the caret, else the picker",
})

kawoosh.command("man pick", function(ctx)
  if ctx.bang then index = nil end
  pick_beside = false
  picker.open("man")
end, {
  bang = "read the list of pages again",
  doc = "every manual page in a picker, its description beside; `<CR>` opens one",
})

-- `K`, `<CR>` in a page: the reference under the caret, into the pane —
-- the man column's page stays in the man column.
kawoosh.command("man here", function(ctx)
  local page, section = under_caret()
  if not page then return kawoosh.echo("no page under the caret") end
  man.open(page, section, { pane = ctx.pane })
end, { when = { "language:" .. LANG }, doc = "the page the reference under the caret names, in this pane" })

-- `]]` `[[`: the next, previous section head, COUNT on.
local function section_step(ctx, dir)
  local h = kawoosh.buf.current()
  local p = h and open[kawoosh.buf.name(h)]
  if not p or #p.heads == 0 then return kawoosh.echo("no sections") end
  local cur = kawoosh.buf.cursor(h).line
  local count = math.max(ctx.count or 1, 1)
  local target
  if dir > 0 then
    local n = 0
    for _, head in ipairs(p.heads) do
      if head.line > cur then
        n = n + 1
        target = head
        if n == count then break end
      end
    end
  else
    local n = 0
    for i = #p.heads, 1, -1 do
      local head = p.heads[i]
      if head.line < cur then
        n = n + 1
        target = head
        if n == count then break end
      end
    end
  end
  if not target then return kawoosh.echo(dir > 0 and "no section after" or "no section before") end
  kawoosh.buf.set_cursor(target.offset, h, { top = target.line })
end
kawoosh.command("man section next", function(ctx) section_step(ctx, 1) end,
  { when = { "language:" .. LANG }, doc = "the next section head of the page, COUNT on", jump = true })
kawoosh.command("man section prev", function(ctx) section_step(ctx, -1) end,
  { when = { "language:" .. LANG }, doc = "the previous section head of the page, COUNT back", jump = true })

-- `q`: the page closed, as `:bd`; in a column `:man beside` made, the
-- column too — it was made for the page, where a pane the page was read
-- into was yours before it — the keys back where they were last.
kawoosh.command("man close", function(ctx)
  local h = kawoosh.buf.current()
  if columns[ctx.pane] then
    columns[ctx.pane] = nil
    kawoosh.run("close")
  end
  if h then kawoosh.buf.close(h, { force = true }) end
end, { when = { "language:" .. LANG }, doc = "the page closed, as `:bd`, and the column `:man beside` made for it" })

local AT = { language = LANG }
kawoosh.map("n", "K", "man here", AT)
kawoosh.map("n", "<CR>", "man here", AT)
kawoosh.map("n", "]]", "man section next", AT)
kawoosh.map("n", "[[", "man section prev", AT)
kawoosh.map("n", "q", "man close", AT)
-- The manual's key everywhere: a column of its own, the man column
-- (man.md Decision 6); the shifted one in the pane, as `:man`.
kawoosh.map("n", "<leader>ik", "man beside")
kawoosh.map("n", "<leader>iK", "man")
