-- The picker (roadmap.md step 4): one pane below the keyboard's — a
-- query, the rows that match it, and a preview of the cursor's row —
-- with a source behind it saying what the rows are. Bundled sources:
-- `files` (every file git sees under the working directory, walked on
-- the io thread), `buffers`, `recent` (the files opened before),
-- `smart` (the three together: buffers, then recent, then the walk),
-- `grep` (`rg` run as the query is typed, its locations the rows),
-- `lines` (the buffer's), `symbols` (the buffer's, a server's or its
-- grammar's outline, as a tree) and `workspace_symbols` (a server's),
-- `grs` `grS`, `commands` (the registry, what
-- `:commands` was) and `tools`; `dirs.lua` adds `dirs`. `<leader>f` `<leader><leader>` `<leader>so` `<leader>.`
-- `<leader>g` `<leader>/` `<leader>ic` `<leader>t` open them,
-- `<leader>sr` the last one again where it was left.
--
-- Compositional (mvp.md D8: hackable by design): `kawoosh.picker` is
-- the module, and a plugin takes it whole — `picker.open { items = …,
-- pick = fn }` or `picker.source("mine", def)` and `picker.open("mine")`
-- — or in parts, `picker.rows(ctx, hits, opts)` drawing a list into
-- its own view and `picker.spans(text, positions)` lighting a match.
-- Matching is `kawoosh.matcher` (fzy's scoring, in Rust; a space
-- splits the query into terms matched in any order, `carnet main`
-- finding `roles/carnet/tasks/main.yml`, docs/design/matching.md), the
-- ranking Lua's: `picker.rank(item, hit)` is the score plus the item's
-- `boost`, and a config replaces it; an open buffer and a file
-- attended before are boosted by the memory's rank (memory.md D8,
-- `kawoosh.memory_rank`) and a binary (`picker.binary`, by
-- extension) held back. A row with columns is matched on its name
-- first and on the rest of its text after, and so is a plain row of a
-- `wide` source on its `sub` (the buffers' paths).
--
-- The query is a field (kui.md D12): typing filters, `<Esc>` is normal
-- mode over the line, `<Esc>` again closes; `<C-n>` `<C-p>` `<Down>`
-- `<Up>` `<C-j>` `<C-k>` (and `j` `k` in normal mode) walk the rows,
-- `<PageDown>` `<PageUp>` (and `<C-d>` `<C-u>`) by a page, `J` `K` in
-- normal mode scroll the preview by half of it (a count multiplies), `<CR>`
-- takes the row — a file at its line, a buffer, a command — `<C-v>`
-- `<C-s>` `<C-t>` take it into a split beside, below, a new tab, and
-- `<C-c>` closes from either mode. A click lands the cursor on a row,
-- a second one takes it; the wheel scrolls the list, and the preview.
-- `<A-p>` shows or hides this picker's preview, the `picker.preview`
-- setting left as it is, and `<A-w>` folds a row's text to the list's
-- width or cuts it — the `picker.wrap` setting, flipped for the
-- session (`settings.lua` sets both for good). The preview goes where
-- the picker's laid-out body has room (`picker.place`,
-- docs/design/picker.md Decision 1): beside the list when wide enough
-- for both, below it when tall enough, else none, the head saying
-- `⌥p preview`, `<A-p>` drawing it anyway — a place kept until its
-- room falls a tenth short, and a preview under the list asking less
-- width than one beside it (Decision 9). The pane opens at
-- `picker.share` of the height and the list takes `picker.split` of
-- its width beside the preview (`picker.split_below` of its height
-- above one). Alt is the picker's, from the query too (Decision 6):
-- `<A-j>` `<A-k>` scroll the preview, `<A-l>` `<A-h>` make the list
-- bigger and smaller against it — the divider right and left beside,
-- down and up under it, which drags too — the session's setting;
-- Alt with Shift is the pane's, the editor's `<A-S-h>` `<A-S-j>`
-- `<A-S-k>` `<A-S-l>`, the height they leave kept as `picker.share`.
-- The head gives up its chips' words, the count's long form and the
-- title before the query is squeezed (Decision 7); a source with
-- `cut = "start"` keeps a long row's end, a path's file name
-- (Decision 8); `<A-/>` shows the keys' legend under the list
-- (Decision 10). `<A-g>` in `files` and `grep` lists ignored and
-- hidden files too (Decision 4). A source's own keys ride on the row: `<C-x>` in `buffers`
-- closes the row's buffer, asking first when it has unsaved changes.
-- A source with `columns` draws its rows as a grid, the cells lined up
-- (the commands: name, key, what it does). A `tree` source's rows are
-- indented by their `depth` while the query is empty, a column marked
-- `path` left blank then (the indent says it); a `cursor(items, ctx)`
-- says which row the cursor starts on. A `follow` source moves the
-- caret of the pane it was opened from to the cursor's row as the
-- cursor moves — an item in that buffer, its view scrolled to show it
-- mid-pane, no preview drawn since the pane is one — and closing
-- untaken puts the caret and the view back (docs/design/marks.md
-- Decision 2). The pane is not kept by a session.

local fs = kawoosh.fs

---How a row is taken: into a split beside, below, a new tab; nil is
---the pane the picker came from.
---@alias kawoosh.picker.How "vsplit"|"split"|"tab"

---A picker source (`picker.source(name, def)`, or `picker.open(def)`
---given whole): what its rows are and what taking one does. One of
---`items`, `load` and `search` says the rows.
---@class kawoosh.picker.Source
---@field name? string the name a source given whole is opened under, `custom` without one
---@field title? string the head's label, and its `:picker NAME` command's doc
---@field placeholder? string the query field's while it is empty
---@field empty? string what the list says with no row and no query
---@field items? kawoosh.picker.Item[]|fun(ctx: kawoosh.picker.Ctx): kawoosh.picker.Item[], kawoosh.picker.Loaded? the rows, at once
---@field load? fun(ctx: kawoosh.picker.Ctx, done: fun(items: kawoosh.picker.Item[]?, err?: string, info?: kawoosh.picker.Loaded)) the rows read off the frame, `done` once they are
---@field legend? { [1]: string|string[], [2]: string }[] its own keys (`keys`) as the legend says them: `{ "<C-x>", "close buffer" }`
---@field cut? "start" a row past the list's width loses its start, not its end: `…/src/main.rs` (rows that are paths)
---@field ignored? boolean `<A-g>` flips `ctx.ignored` and loads (or searches) again: files git ignores, and hidden ones, too
---@field search? fun(query: string, job: kawoosh.picker.Job) a dynamic source: asked again as the query is typed, nothing until one is, its rows not filtered again
---@field query? fun(query: string, ctx: kawoosh.picker.Ctx): string? the text matched for what was typed; nil matches it as typed
---@field preview? fun(item: kawoosh.picker.Item, ctx: kawoosh.ViewCtx): kawoosh.picker.Preview the cursor's row beside the list; by default its path's, its buffer's or its `lines`
---@field pick? fun(item: kawoosh.picker.Item, how?: kawoosh.picker.How) what taking a row does; by default its buffer shown, its path opened, its `run` run
---@field answer? fun(item: kawoosh.picker.Item): string? what a pick answers `kawoosh pick` from a shell; by default its path, its buffer's, its text
---@field keys? table<string, fun(item: kawoosh.picker.Item?, state: table)> keys of its own on the cursor's row (`<C-x>`): the item nil on an empty list, `state` the open picker's own
---@field columns? kawoosh.picker.Column[] the rows a grid, the cells lined up (`picker.rows`)
---@field icon? string the item's field naming the kind whose icon starts each row
---@field wide? boolean a row's `sub` matched too, after its text
---@field tree? boolean the rows indented by their `depth` while the query is empty
---@field cursor? fun(items: kawoosh.picker.Item[], ctx: kawoosh.picker.Ctx): integer? the row the cursor starts on, from 1
---@field follow? boolean the pane it was opened from follows the cursor's row, and no preview is drawn
---@field launcher? boolean a module of the launcher too, under its name
---@field launch? fun(item: kawoosh.picker.Item) what its row does in the launcher, filling the bare pane, when `pick` goes elsewhere

---What a `load` (or an `items` function, as a second answer) says
---besides its items.
---@class kawoosh.picker.Loaded
---@field capped? boolean the rows stopped at a cap, more left
---@field rows? kawoosh.picker.Rows rows held by the engine, listed after the items

---Rows a source hands over held by the engine, not as a Lua table each
---— a hundred thousand of them a moment, not a stall: listed after the
---source's own items, and made an item only when a row is shown or
---ranked (`picker.with_rows`, docs/design/picker.md Decision 5). With
---`root` they are paths relative to it, as `fs.walk`'s `matcher` hands
---them: each path once, in the order `boosted` gives items, an item
---`{ text, path, boost }`.
---@class kawoosh.picker.Rows
---@field matcher kawoosh.Matcher the rows (`kawoosh.matcher(list)`, or a walk's)
---@field root? string the rows are paths relative to it
---@field item? fun(text: string, i: integer): kawoosh.picker.Item row `i`'s item, made when it is first shown; `{ text = text }` (and its `path`) by default

---A row of a picker (and of a launcher's module): its text, and what
---it stands for. A row of a source with `columns` holds their fields.
---@class kawoosh.picker.Item
---@field text string what the row shows and is matched on
---@field sub? string drawn dim after the text
---@field path? string a file: opened at `line` and `col` on a pick
---@field line? integer from 1
---@field col? integer from 1
---@field end_line? integer the last line of its range (a symbol's), from 1
---@field buffer? integer a buffer: shown on a pick
---@field offset? integer a byte in its buffer, from 0
---@field run? string a command line run on a pick
---@field pick? fun(item: kawoosh.picker.Item, how?: kawoosh.picker.How) a pick of its own, when the source has none
---@field boost? number added to its match's score (`picker.rank`); below 0 ranks it last
---@field search? string the text it is matched on after its name; set from its columns (`picker.search`) when not given
---@field lines? string[] lines of its own: the preview's
---@field preview? kawoosh.picker.Preview its preview, as it is
---@field can? true|string true, or why it cannot be taken, drawn in the danger colour
---@field depth? integer its indent in a `tree` source
---@field key? string in the launcher: the letter it wants
---@field hint? string in the launcher: the key drawn on its row
---@field [string] any a column's field, a source's own

---What a source's `items`, `load`, `query` and `cursor` are given:
---where it was opened from. The launcher gives `buffer`, `cwd` and
---`origin`.
---@class kawoosh.picker.Ctx
---@field buffer? integer the buffer in front when it opened
---@field cwd string the working directory
---@field root? string where a walk or a search starts: `here`'s directory, else the working one
---@field ignored? boolean `<A-g>` on, in a source with `ignored`: ignored and hidden files too
---@field terminal? boolean opened from a terminal pane
---@field pane? integer the pane it was opened from
---@field caret? kawoosh.buf.Cursor `kawoosh.buf.cursor`'s, opened from a buffer
---@field origin? kawoosh.ViewOrigin in the launcher: the buffer a bare pane was split from

---A dynamic source's search, as `search(query, job)` is given it.
---@class kawoosh.picker.Job
---@field emit fun(items: kawoosh.picker.Item[]) rows found, appended to the list
---@field done fun(note?: string) the search over; a note shown in the count's place
---@field cancel? fun() set by the source: how a newer query stops this one
---@field stale boolean a newer query took its place: what it emits is dropped

---A row of a list, as `picker.rows` draws it: an item and where the
---query matched it.
---@class kawoosh.picker.Hit
---@field item kawoosh.picker.Item
---@field positions? integer[] the bytes the match lit, from 1, in its text (or its `picker.search` text)

---A column of a grid of rows (`columns`, `picker.rows`): `[1]` the
---item's field it holds.
---@class kawoosh.picker.Column
---@field [1] string the item's field the cell holds
---@field dim? string a field drawn faint after it, matched too
---@field family? string the cell's face: `"mono"`
---@field muted? boolean drawn in the muted colour
---@field min? number px: the narrowest it is
---@field max? number px: the widest `picker.widths` makes it
---@field width? number px: fixed
---@field share? number the most of the list's width it takes, a fraction
---@field grow? boolean it takes the rest of the row
---@field lead? string the item's field drawn before the cell's, not matched: a string, or spans of their own colours
---@field color? string the item's field naming the colour of the cell's own text
---@field path? boolean left blank while a tree is indented

---A preview: lines of text, numbered when `from` says where they
---start, or a picture.
---@class kawoosh.picker.Preview
---@field title? string
---@field lines string[]
---@field from? integer the first line's number, from 1; the lines are numbered when given
---@field at? integer the line lit, by its number
---@field note? string why there are no lines
---@field path? string the file whose language colours the lines
---@field language? string the language the lines are coloured as
---@field image? string a picture's path, drawn instead of the lines
---@field wrap? boolean a line longer than the preview is wide wraps at its words, not cut
---@field code? integer in a markdown preview, how many of the first lines are code, coloured as `code_language` and drawn without a fence
---@field code_language? string the language of the `code` lines

---A run of a text, as `picker.spans` gives them: `[1]` its text.
---@class kawoosh.picker.Span
---@field [1] string
---@field color? kui.Color
---@field bold? boolean
---@field italic? boolean

---The last picker closed, for `picker.resume`.
---@class kawoosh.picker.Last
---@field source string its source's name
---@field query? string
---@field cursor integer from 1

---What `picker.open` takes besides the source.
---@class kawoosh.picker.OpenOpts
---@field query? string the query it opens with
---@field cursor? integer the row the cursor starts on, from 1
---@field root? string the directory a walk or a search starts from; the working one by default
---@field answer? fun(item: kawoosh.picker.Item?) takes the pick instead of the source; nil when the picker closes untaken

---What `picker.rows` draws a list by.
---@class kawoosh.picker.RowsOpts
---@field top? integer the first hit shown, from 1
---@field rows? integer the lines the window has room for
---@field cursor? integer the hit lit
---@field kind? string the click's event kind, `"row"` by default
---@field scroll? string the wheel's tag kind, `"list"` by default
---@field text? fun(item: kawoosh.picker.Item): string a row's text, `item.text` by default
---@field columns? kawoosh.picker.Column[] the rows a grid
---@field widths? table<integer, number> each column's width, px (`picker.widths`)
---@field width? number the list's width, px, for a column's `share`
---@field wrap? boolean a text folds to its width
---@field lines? fun(i: integer): integer how many lines row `i` takes
---@field indent? fun(item: kawoosh.picker.Item): integer? the steps a row is indented
---@field keys? table<integer, string> filled: each drawn row's key, by its index
---@field hover? string the event kind each row posts as the pointer comes and goes
---@field icon? string the item's field naming the kind whose icon starts each row
---@field cut? "start" a text past `width` cut from its start, `…` and its tail kept (a path's file name); else its end is cut

---What the open picker shows (`picker.state()`).
---@class kawoosh.picker.State
---@field source string its source's name
---@field query string
---@field cursor integer the cursor's row, from 1
---@field top integer the first row shown
---@field count integer the rows
---@field text? string the cursor's row's text
---@field item? kawoosh.picker.Item the cursor's row's item
---@field positions? integer[] the bytes its match lit, in its `picker.search` text
---@field loading boolean
---@field preview? kawoosh.picker.Preview the cursor's, as drawn: `runs` once highlighted
---@field rows string[] every hit's text, in order
---@field items kawoosh.picker.Item[] every hit's item, in order
---@field root string where a walk starts
---@field place? "right"|"below" where the preview is drawn; nil for none
---@field ignored boolean `<A-g>`: ignored and hidden files too
---@field capped boolean the rows stopped at a cap, more left

---The picker: its sources by name and the last one closed; its
---functions the parts a plugin's own list is drawn with.
---@class kawoosh.picker
---@field sources table<string, kawoosh.picker.Source> by name: `picker.open("files")`
---@field last? kawoosh.picker.Last the last picker closed, for `<leader>sr`
---@field images table<string, boolean> the extensions the preview draws as a picture
---@field binary table<string, boolean> the extensions that are not text, ranked under the rest
local picker = { sources = {}, last = nil }
kawoosh.picker = picker

local VIEW = "picker"
local FIELD = "q"
-- The keys are on the picker's pane, its field under them or not:
-- what the commands are gated by, so a pane-mode map (the list
-- blurred, `<Esc>` twice) runs them too.
local PANE_FACT = "lua:" .. VIEW
-- Whether the open picker draws a preview beside its list: what the
-- old `list wider` and `list narrower` are gated by (`<A-h>` `<A-l>`
-- size the list whichever way the preview lies).
local PREVIEW_FACT = "picker:preview"
-- The rows' text: the field's size, so the query and its answers line
-- up; a row is the field's height too. The panes' one scale
-- (`kawoosh.metrics`), read each frame (`sizes`), so the picker follows
-- the font as the tabs and the strips do; these are the 13 px defaults.
local SIZE, SMALL = 13, 12
local ROW_H = SIZE + 6
local PREVIEW_SIZE = 12
local PREVIEW_ROW = PREVIEW_SIZE + 4

local function sizes(env)
  local m = kawoosh.metrics(env)
  SIZE, SMALL = m.text, m.small
  ROW_H = SIZE + 6
  PREVIEW_SIZE = m.small
  PREVIEW_ROW = PREVIEW_SIZE + 4
end
-- The most rows a query keeps: a screenful and a few pages after it.
local LIMIT = 200
-- The most lines a search reads before it is stopped.
local GREP_MAX = 2000
local PREVIEW_MAX = 256 * 1024
local PREVIEW_LINES = 200

-- The open picker: its source, items, the hits for the query, the
-- cursor and the window onto them.
local P = nil

local function char_len(s, i)
  local c = s:byte(i)
  if not c then return 0 end
  if c < 0x80 then return 1 elseif c < 0xE0 then return 2 elseif c < 0xF0 then return 3 else return 4 end
end

-- A path for a row: under the working directory, relative to it; under
-- home, with `~`; else whole.
---@param path string
---@return string
local function short_path(path)
  local rel = fs.relative(path)
  if rel and rel ~= "." then return rel end
  return fs.short(path)
end
picker.short_path = short_path

-- ------------------------------------------------------------- ranking

-- The boost an item with a path or a buffer gets: the memory's rank
-- (memory.md D8, `kawoosh.memory_rank.boosts` — a file attended more,
-- and lately, ranks higher, a pinned one above any), and an open
-- buffer's file — what the hands are in — one more.
local function boosts()
  local by = {}
  local mem = kawoosh.memory_rank
  if mem then by = mem.boosts("file", 500) end
  for _, h in ipairs(kawoosh.buf.list()) do
    local ok, p = pcall(kawoosh.buf.path, h)
    -- A pin ranks above any boost, in pin order: an open one keeps its
    -- place.
    if ok and p and (by[p] or 0) < 10 then by[p] = (by[p] or 0) + 1 end
  end
  return by
end

-- picker.rank(item, hit): what a row is ordered by among the hits —
-- the match's score with the item's `boost` added. Replace it from a
-- config to rank by what matters to you.
---@param item kawoosh.picker.Item
---@param hit kawoosh.FuzzyHit the matcher's
---@return number
function picker.rank(item, hit)
  return hit.score + (item.boost or 0)
end

-- The files the preview draws as a picture, by extension: what the
-- engine decodes (`kawoosh.image`).
picker.images = { png = true, apng = true, jpg = true, jpeg = true, gif = true, webp = true, bmp = true,
                  svg = true, svgz = true }

-- A file that is not text, by its extension: fonts, images, sound,
-- archives, what a build left — ranked under the rest, since a query
-- for `font` wants the source that loads it before the `.ttf`.
picker.binary = {
  png = true, jpg = true, jpeg = true, gif = true, webp = true, ico = true, icns = true, bmp = true, tiff = true,
  ttf = true, otf = true, woff = true, woff2 = true,
  mp3 = true, wav = true, ogg = true, flac = true, mp4 = true, mov = true, webm = true,
  zip = true, gz = true, tgz = true, bz2 = true, xz = true, zst = true, ["7z"] = true, tar = true, jar = true,
  pdf = true, o = true, a = true, so = true, dylib = true, dll = true, exe = true, wasm = true, class = true,
  pyc = true, db = true, sqlite = true, bin = true, dat = true,
}
-- Read from the path's end, a few bytes at most: a pattern anchored
-- at the end is tried from every byte. A walk's rows are read by the
-- engine, by the same rule (`m:order`'s `last`, lua/src/lib.rs
-- `picker_extension`).
local function is_binary(path)
  local n = #path
  for i = n, math.max(1, n - 8), -1 do
    local b = path:byte(i)
    if b == 46 then -- `.`
      local ext = path:sub(i + 1)
      if ext == "" or ext:find("%W") then return false end
      local hit = picker.binary[ext]
      if hit == nil and ext:find("%u") then hit = picker.binary[ext:lower()] end
      return hit == true
    end
    if b == 47 or b == 92 then return false end -- `/` `\`
  end
  return false
end

-- The items boosted by `by` (`boosts()`), and the boosted ones first —
-- the best boost first, so a pin leads and the file worked in all week
-- comes before one glanced at — the binaries last: an empty query
-- lists what the hands were in before the rest, and the fonts and
-- images after it. A walk's rows are put in the same order by the
-- engine (`picker.with_rows`).
local function boosted(items, by)
  local first, rest, last = {}, {}, {}
  -- Where each boosted one was, for a sort that keeps their order: only
  -- they are sorted, and an index on every item grew every item's table.
  local at = {}
  for i, it in ipairs(items) do
    if it.boost == nil and it.path and by[it.path] then it.boost = by[it.path] end
    if it.boost == nil and it.path and is_binary(it.path) then it.boost = -0.5 end
    if it.boost and it.boost > 0 then
      first[#first + 1] = it
      at[it] = i
    elseif it.boost and it.boost < 0 then last[#last + 1] = it
    else rest[#rest + 1] = it end
  end
  table.sort(first, function(a, b)
    if a.boost ~= b.boost then return a.boost > b.boost end
    return at[a] < at[b]
  end)
  for _, it in ipairs(rest) do first[#first + 1] = it end
  for _, it in ipairs(last) do first[#first + 1] = it end
  return first
end

-- ------------------------------------------------------------ previews

local file_cache = {}

-- A file's first lines (or all of them, up to `PREVIEW_LINES` past
-- `around`), read once per size and mtime.
-- A buffer's lines `from` to `to` as its pane draws them: what its
-- masks hide drawn as `•` (docs/design/secrets.md Decision 3) — a
-- rule by its files, a rule a plugin asked for (a vault's scratch,
-- which has no path), ranges a plugin gave. The whole buffer without
-- a range, as `kawoosh.buf.lines` has it: the empty line after a final
-- newline is no line.
local function shown_lines(h, from, to)
  local lines = kawoosh.buf.shown_lines(from or 1, to or math.maxinteger, h)
  if not from and lines[#lines] == "" and #lines > 1 then lines[#lines] = nil end
  return lines
end

-- A host's files read for the preview while a picker is open, by path:
-- there a stat is a round trip on the frame, and a preview drawn again
-- each frame would ask each frame (docs/design/domains.md, "Built,
-- speed"). Read once, kept until the next picker opens.
local host_cache = {}

-- Whether `path` is spelled on a domain (`box:/…`, `box:~…`): a name of
-- two characters or more, so a drive's `C:/` is not one.
local function on_host(path)
  return path:match("^[%w_%.%-][%w_%.%-]+:[/~]") ~= nil
end

local function file_lines(path, around)
  if on_host(path) then
    local c = host_cache[path]
    if not c then
      local lines, why = (function()
        local ok, st = pcall(fs.stat, path)
        if not ok then return nil, "not on disk" end
        if st.is_dir then return nil, "a directory" end
        if st.size > PREVIEW_MAX then return nil, "too big to preview" end
        local rok, text = pcall(fs.read, path)
        if not rok then return nil, "not text" end
        if text:find("\0", 1, true) then return nil, "binary" end
        text = kawoosh.secrets.mask_text(text, path)
        local out = {}
        for line in (text .. "\n"):gmatch("(.-)\n") do out[#out + 1] = line end
        if out[#out] == "" then out[#out] = nil end
        return out
      end)()
      c = { lines = lines, why = why }
      host_cache[path] = c
    end
    return c.lines, c.why
  end
  local ok, st = pcall(fs.stat, path)
  if not ok then return nil, "not on disk" end
  if st.is_dir then return nil, "a directory" end
  if st.size > PREVIEW_MAX then return nil, "too big to preview" end
  local key = st.size .. ":" .. tostring(st.modified)
  local c = file_cache[path]
  if not c or c.key ~= key then
    local rok, text = pcall(fs.read, path)
    if not rok then return nil, "not text" end
    if text:find("\0", 1, true) then return nil, "binary" end
    -- What a mask rule hides in the file stays hidden in the preview.
    text = kawoosh.secrets.mask_text(text, path)
    c = { key = key, lines = {} }
    for line in (text .. "\n"):gmatch("(.-)\n") do
      c.lines[#c.lines + 1] = line
    end
    if c.lines[#c.lines] == "" then c.lines[#c.lines] = nil end
    file_cache = { [path] = c }
  end
  return c.lines
end

-- The preview of an item with a path, or a buffer, or lines of its
-- own: `{ title =, lines =, from =, at = }` — `from` the number of the
-- first line shown, `at` the line to light.
local function preview_of(item)
  if item.preview then return item.preview end
  local lines, why, title
  if item.lines then
    lines, title = item.lines, item.text
  elseif item.buffer then
    local ok, name = pcall(kawoosh.buf.name, item.buffer)
    if not ok then return { title = "gone", lines = {} } end
    title = name
    local from = math.max((item.line or 1) - 10, 1)
    local ok2, got = pcall(shown_lines, item.buffer, from, from + PREVIEW_LINES)
    lines = ok2 and got or {}
    local pok, path = pcall(kawoosh.buf.path, item.buffer)
    return { title = title, lines = lines, from = from, at = item.line, path = pok and path or nil }
  elseif item.path then
    title = short_path(item.path)
    -- A picture is drawn, not read as text (`kawoosh.image`).
    local ext = item.path:match("%.([%w]+)$")
    if ext and picker.images[ext:lower()] then
      return { title = title, lines = {}, image = item.path }
    end
    lines, why = file_lines(item.path)
  else
    return { title = item.text, lines = { item.sub or "" } }
  end
  if not lines then return { title = title, lines = {}, note = why } end
  local from = math.max((item.line or 1) - 10, 1)
  local window = {}
  for i = from, math.min(#lines, from + PREVIEW_LINES) do window[#window + 1] = lines[i] end
  return { title = title, lines = window, from = from, at = item.line, path = item.path }
end

-- The preview's lines coloured: `kawoosh.highlight` asked once per
-- preview with the language its path says, the runs kept on it
-- (`pv.runs`) when they come, with where each line starts in the
-- text they cover — the next frame paints them.
local function highlight_preview(pv)
  if pv.runs or pv.asked or not (pv.path or pv.language) or #pv.lines == 0 then return end
  pv.asked = true
  -- A markdown preview's `code` lines fenced for the highlighter alone:
  -- the fences are in the text it colours, never in the lines drawn.
  local code = pv.language == "markdown" and pv.code or 0
  local parts, starts, at = {}, {}, 1
  local function put(l)
    parts[#parts + 1] = l
    at = at + #l + 1
  end
  if code > 0 then put("```" .. (pv.code_language or "")) end
  for i, l in ipairs(pv.lines) do
    if code > 0 and i == code + 1 then put("```") end
    starts[i] = at
    put(l)
  end
  if code > 0 and #pv.lines <= code then put("```") end
  kawoosh.highlight(table.concat(parts, "\n"), { path = pv.path, language = pv.language }, function(runs)
    if not runs then return end
    pv.runs = runs
    pv.starts = starts
  end)
end

-- Line `i` of a preview as spans: the runs that reach into it, each
-- its colour and its style, the rest `color`.
local function preview_spans(pv, i, l, color)
  local runs = pv.runs
  if not runs or #runs == 0 then return { { l, color = color } } end
  local from = pv.starts[i]
  local to = from + #l - 1
  local out, at = {}, 1
  for _, r in ipairs(runs) do
    if r.to >= from and r.from <= to and r.color then
      local a = math.max(r.from, from) - from + 1
      local b = math.min(r.to, to) - from + 1
      if a > at then out[#out + 1] = { l:sub(at, a - 1), color = color } end
      if b >= a then
        out[#out + 1] = { l:sub(a, b), color = r.color, bold = r.bold, italic = r.italic,
                          underline = r.underline, strikethrough = r.strikethrough }
      end
      at = math.max(at, b + 1)
    end
  end
  if at <= #l then out[#out + 1] = { l:sub(at), color = color } end
  if #out == 0 then out[1] = { l, color = color } end
  return out
end

-- ------------------------------------------------------------ the parts

-- picker.spans(text, positions): the text as spans with the matched
-- bytes (from 1) lit in the accent, for a row of your own.
---@param text string
---@param positions? integer[] bytes from 1
---@param theme table<string, kui.Color> the env's theme: its `accent` lights the match
---@return kawoosh.picker.Span[]
function picker.spans(text, positions, theme)
  if not positions or #positions == 0 then return { { text } } end
  local lit = {}
  for _, p in ipairs(positions) do lit[p] = true end
  local out, i = {}, 1
  while i <= #text do
    local n = char_len(text, i)
    local piece = text:sub(i, i + n - 1)
    local on = lit[i] or false
    local last = out[#out]
    if last and last.on == on then
      last[1] = last[1] .. piece
    else
      out[#out + 1] = { piece, on = on, bold = on or nil, color = on and theme.accent or nil }
    end
    i = i + n
  end
  for _, s in ipairs(out) do s.on = nil end
  return out
end

-- The pieces a row with columns shows, in order: each column's field
-- and its `dim` field after it, with where each starts (a byte, from
-- 1) in the row's search text — the pieces joined by two spaces — so
-- a match's positions, found in that text, land on the piece they
-- are in. A plain row's pieces are its text (`main`, else `item.text`)
-- and its `sub`, dim after it.
local function pieces_of(it, columns, main)
  if not columns then
    main = main or it.text
    local out = { { text = main, from = 1 } }
    if it.sub and it.sub ~= "" then out[2] = { text = it.sub, from = #main + 3, dim = true } end
    return out
  end
  local out, at = {}, 1
  for j, c in ipairs(columns) do
    local main = tostring(it[c[1]] or "")
    out[#out + 1] = { text = main, col = j, from = at }
    at = at + #main + 2
    if c.dim then
      local d = tostring(it[c.dim] or "")
      if d ~= "" then
        out[#out + 1] = { text = d, col = j, from = at, dim = true }
        at = at + #d + 2
      end
    end
  end
  return out
end

-- picker.search(item, columns): the text a row with columns is matched
-- on — its columns' fields and their `dim` ones, joined by two spaces
-- — what a source with `columns` has its items matched by (`search`,
-- set when the items load unless the item brought its own). Without
-- columns, the row's text and its `sub`: a `wide` source's.
---@param it kawoosh.picker.Item
---@param columns? kawoosh.picker.Column[]
---@return string
function picker.search(it, columns)
  local parts = {}
  for _, p in ipairs(pieces_of(it, columns)) do parts[#parts + 1] = p.text end
  return table.concat(parts, "  ")
end

-- The width column `j` is fixed at, if it is: its own `width` or
-- `widths[j]`, at least `min`, at most its `share` of `list_w`.
local function fixed_width(c, widths, j, list_w)
  local fixed = c.width or widths[j]
  if not fixed then return nil end
  fixed = math.max(fixed, c.min or 0)
  if c.share and list_w then fixed = math.min(fixed, math.floor(list_w * c.share)) end
  return fixed
end

-- picker.widths(items, columns): a floor for each column that does
-- not grow, from the widest cell among every item — an estimate from
-- the mono glyph's width, since a cell's text is not measured here —
-- so the columns are the same whichever rows the window shows and
-- whatever the query kept. A `max` on the column caps it.
---@param items kawoosh.picker.Item[]
---@param columns kawoosh.picker.Column[]
---@return table<integer, number> px by column, none for a growing one
function picker.widths(items, columns)
  local out = {}
  for j, c in ipairs(columns) do
    if not c.grow then
      -- The main and the dim piece are one cell: their widths add.
      local main, dim = 0, 0
      for _, it in ipairs(items) do
        local m = tostring(it[c[1]] or "")
        main = math.max(main, utf8.len(m) or #m)
        if c.dim then
          local d = tostring(it[c.dim] or "")
          if d ~= "" then dim = math.max(dim, (utf8.len(d) or #d) + 2) end
        end
      end
      local w = math.ceil((main + dim) * SIZE * 0.6) + 4
      if c.max then w = math.min(w, c.max) end
      out[j] = w
    end
  end
  return out
end

-- The positions inside piece `p`, as positions in its own text.
local function lit_in(positions, p)
  local out = {}
  for _, pos in ipairs(positions or {}) do
    if pos >= p.from and pos < p.from + #p.text then out[#out + 1] = pos - p.from + 1 end
  end
  return out
end

-- picker.rows(ctx, hits, opts): the rows of a list — each hit
-- `{ item =, positions = }` — as a column showing the window from
-- `opts.top` down as far as `opts.rows` lines allow, `opts.cursor`
-- lit; a row's click posts `{ kind = opts.kind or "row", i = }` and
-- the wheel over the column `{ kind = "scroll", tag = { kind =
-- opts.scroll or "list" } }`. A row is one text: `opts.text(item)`
-- (default `item.text`) with the match lit, `item.sub` dim after it
-- (lit too where a match on `picker.search`'s text fell in it),
-- `item.can` in the danger colour when it is not true. With
-- `opts.columns` — `{ { FIELD, dim = FIELD, family =, muted =, min =,
-- width =, grow = }, … }` — the rows are a grid, a cell per column
-- holding the item's field (its `dim` field faint after it), the
-- cells lined up: a column is `width` wide, or `opts.widths[j]` wide
-- (`picker.widths`: the widest of every item, at least `min`, so the
-- columns hold still as the window slides — a fixed width, since a
-- floor would still let the window's widest cell push the column
-- out), a column's `share` capping that at the fraction of
-- `opts.width`, the list's, so the growing column keeps room beside
-- a preview, and a cell past its column is cut; without either it
-- sits at its widest cell; or it `grow`s into the rest; the match
-- is lit where it falls (`picker.search`), and `item.can` ends the
-- last cell. A column's `lead = FIELD` names what the cell shows
-- before its field — a string, or a list of spans with colours of
-- their own (a symbol's `pub fn` in the syntax's colours) — which the
-- match is not looked for in; its `color = FIELD` names the colour
-- the field's own text takes where the match does not light it (the
-- name's token). With `opts.wrap` a text folds to its width — every
-- cell's, so the whole of a long path or name shows — and
-- `opts.lines(i)` says how many lines row `i` takes (one, when not
-- given). With `opts.indent(item)` a number, the first cell (or the
-- text) is indented by that many steps, and a column marked `path` is
-- left blank — a tree's rows. With `opts.keys` a table, each drawn
-- row's key is put in it by its index — what `env.is_hovered` asks by.
-- With `opts.hover` a kind, each row hears the pointer as
-- `{ kind = "hover", phase, by, tag = { kind = opts.hover, i = i } }`.
-- With `opts.icon` a field, each row starts with the icon of the kind
-- that field names (`kawoosh.kind_icon`), a cell of the editor's face
-- wide, in `font.icons` (completion.md Decision 9): a candidate's, a
-- symbol's.
---@param ctx kawoosh.ViewCtx the view's
---@param hits kawoosh.picker.Hit[]
---@param opts kawoosh.picker.RowsOpts
---@return kui.Node
function picker.rows(ctx, hits, opts)
  local t = ctx.env.theme
  local top = opts.top or 1
  local budget = opts.rows or #hits
  local lines_of = opts.lines or function() return 1 end
  local columns = opts.columns
  local widths = opts.widths or {}
  local wrap = opts.wrap and "glyph" or "none"
  local mono_w = opts.cut == "start" and opts.width
    and ctx.env.measure_text("M", { family = "mono", size = SIZE, wrap = "none" }).width or nil
  if mono_w and mono_w <= 0 then mono_w = nil end
  -- The window is a column that clips, and inside it the rows as tall
  -- as they are: `min_height = "fit"` is the floor kui compresses a
  -- child toward when its parent overflows, so a folded row keeps its
  -- lines and the last row is cut instead of every row squeezed. A
  -- grid when the rows have columns, so the cells line up.
  local outer = column { width = "grow", height = "grow", clip = true, gap = 0,
    on_scroll = { kind = opts.scroll or "list" } }
  local inner = (columns and grid or column) { width = "grow", height = "fit", min_height = "fit", gap = 0 }
  outer[#outer + 1] = inner
  -- A row is keyed by its text (`row NAME`), the second of one text
  -- numbered, so a row keeps its state as the window slides and a
  -- test finds it by name.
  local seen = {}
  local used = 0
  -- Rows while the budget lasts, and the one that crosses it: the
  -- lines are an estimate and the column clips, so a row too many is
  -- cut where a row too few would leave a gap.
  for i = top, #hits do
    if used >= budget then break end
    used = used + lines_of(i)
    local h = hits[i]
    local it = h.item
    local selected = i == opts.cursor
    local text_of = opts.text and opts.text(it) or it.text
    local key = "row " .. text_of
    if seen[key] then
      seen[key] = seen[key] + 1
      key = key .. " (" .. seen[key] .. ")"
    else
      seen[key] = 1
    end
    if opts.keys then opts.keys[i] = key end
    local r = row {
      key = key,
      width = "grow", min_height = ROW_H,
      pad = { x = 8 }, gap = 8, cross_align = "center",
      bg = selected and (ctx.focused and t.selection or t.sunken) or nil,
      hover_bg = not selected and t.sunken or nil,
      on_click = { kind = opts.kind or "row", i = i },
      on_hover = opts.hover and { kind = opts.hover, i = i } or nil,
    }
    local off = it.can ~= nil and it.can ~= true
    local color = off and t.muted or t.fg
    local indent = opts.indent and opts.indent(it) or nil
    local step = indent and indent > 0 and string.rep("  ", indent) or nil
    local icon = opts.icon and it[opts.icon] and kawoosh.icon(kawoosh.kind_icon(it[opts.icon]),
      { cell = true, size = SIZE, color = selected and t.accent or t.muted }) or nil
    if columns then
      local ps = pieces_of(it, columns)
      for j, c in ipairs(columns) do
        local spans = {}
        -- A row's icon after its indent: the indent a text of its own.
        local lead_step = j == 1 and icon and step or nil
        if j == 1 and step and not lead_step then spans[1] = { step } end
        -- The column's own colour for its field: the item's, when the
        -- column names one and the row is not off.
        local own = c.color and not off and it[c.color] or nil
        for _, p in ipairs(ps) do
          if p.col == j and p.text ~= "" and not (indent and c.path) then
            if #spans > (step and j == 1 and 1 or 0) then spans[#spans + 1] = { "  " } end
            local lead = not p.dim and c.lead and it[c.lead] or nil
            if lead then
              if type(lead) == "string" then
                spans[#spans + 1] = { lead, color = c.muted and t.muted or color }
              else
                for _, sp in ipairs(lead) do
                  spans[#spans + 1] = { sp[1], color = sp.color or (c.muted and t.muted or color),
                                        bold = sp.bold, italic = sp.italic }
                end
              end
              spans[#spans + 1] = { " " }
            end
            for _, sp in ipairs(picker.spans(p.text, lit_in(h.positions, p), t)) do
              if not sp.color then
                sp.color = p.dim and t.faint or (c.muted and t.muted or (not p.dim and own) or color)
              end
              spans[#spans + 1] = sp
            end
          end
        end
        if j == #columns and off then
          spans[#spans + 1] = { (#spans > 0 and "  " or "") .. it.can, color = t.danger }
        end
        local fixed = fixed_width(c, widths, j, opts.width)
        local cell = row { width = c.grow and "grow" or fixed or "fit", min_width = not fixed and c.min or nil,
          clip = true, cross_align = "center", gap = j == 1 and icon and 6 or nil }
        if lead_step then cell[#cell + 1] = text(lead_step, { family = c.family, size = SIZE, wrap = "none" }) end
        if j == 1 and icon then cell[#cell + 1] = icon end
        if #spans > 0 then
          cell[#cell + 1] = text(spans, { family = c.family, size = SIZE, wrap = wrap })
        end
        r[#r + 1] = cell
      end
    else
      local ps = pieces_of(it, nil, text_of)
      local shown, lit = text_of, lit_in(h.positions, ps[1])
      -- Past the list's width, a text cut from its start where the
      -- source says its end is what counts (`cut = "start"`, a path's
      -- file name): `…` and the tail, the match lit where it still
      -- falls. The rows are mono, so the room is in characters
      -- (picker.md Decision 8).
      if mono_w and wrap == "none" then
        local room = math.floor((opts.width - 16 - (icon and SIZE + 8 or 0)) / mono_w)
          - (ps[2] and (utf8.len(ps[2].text) or #ps[2].text) + 2 or 0) - (step and #step or 0)
        local n = utf8.len(text_of)
        if n and room >= 2 and n > room then
          local from = utf8.offset(text_of, n - room + 2)
          shown = "…" .. text_of:sub(from)
          local moved = {}
          for _, pos in ipairs(lit) do
            if pos >= from then moved[#moved + 1] = pos - from + 1 + #"…" end
          end
          lit = moved
        end
      end
      local spans = picker.spans(shown, lit, t)
      for _, sp in ipairs(spans) do if not sp.color then sp.color = color end end
      if step then table.insert(spans, 1, { step }) end
      if ps[2] then
        spans[#spans + 1] = { "  ", color = t.muted }
        for _, sp in ipairs(picker.spans(ps[2].text, lit_in(h.positions, ps[2]), t)) do
          if not sp.color then sp.color = t.muted end
          spans[#spans + 1] = sp
        end
      end
      if off then
        spans[#spans + 1] = { "  " .. it.can, color = t.danger }
      end
      if icon then r[#r + 1] = icon end
      r[#r + 1] = text(spans, { family = "mono", size = SIZE, wrap = wrap })
    end
    inner[#inner + 1] = r
  end
  return outer
end

-- picker.preview(ctx, pv, rows, from, room): a preview `{ title =,
-- lines =, from =, at =, note =, path = }` as a column, `rows` lines of
-- it at most from line `from` (1) of them, the syntax coloured when a
-- `path` says the language (asked of the ts thread once, painted
-- when it answers); the wheel over it posts `{ kind = "scroll", tag =
-- { kind = "preview" } }`. `room` (`{ width =, height = }`, px) is
-- what a picture is fitted to; the view's right half without it.
---@param ctx kawoosh.ViewCtx the view's
---@param pv? kawoosh.picker.Preview nil draws the empty column
---@param rows integer the lines it has room for
---@param from? integer the first of `pv.lines` drawn, from 1
---@param room? { width: number, height: number } the preview's size
---@return kui.Node
function picker.preview(ctx, pv, rows, from, room)
  local t = ctx.env.theme
  local col = column { width = "grow", height = "grow", clip = true, pad = 8, gap = 2, bg = t.sunken,
    on_scroll = { kind = "preview" } }
  if not pv then return col end
  from = from or 1
  col[#col + 1] = text(pv.title or "", { size = PREVIEW_SIZE, color = t.fg, wrap = "none" })
  if pv.note then col[#col + 1] = text(pv.note, { size = PREVIEW_SIZE, color = t.muted }) end
  -- A picture, fitted to the preview's room and never larger than it
  -- is; `reading…` until it is.
  if pv.image then
    local img, why = kawoosh.image(pv.image)
    if not img then
      col[#col + 1] = text(why or "reading…", { size = PREVIEW_SIZE, color = t.muted })
      return col
    end
    col[#col + 1] = text(img.width .. " × " .. img.height, { size = PREVIEW_SIZE, color = t.muted })
    room = room or { width = (ctx.width or 400) * (1 - (tonumber(kawoosh.opt("picker.split")) or 0.5)),
                     height = ctx.height or 300 }
    local room_w = math.max(room.width - 16, 16)
    local room_h = math.max(room.height - 2 * (PREVIEW_SIZE + 8) - 16, 16)
    local scale = math.min(1, room_w / img.width, room_h / img.height)
    col[#col + 1] = image {
      id = img.id, fit = "contain",
      width = math.max(1, math.floor(img.width * scale)),
      height = math.max(1, math.floor(img.height * scale)),
    }
    return col
  end
  local first = pv.from or 1
  highlight_preview(pv)
  for i = from, math.min(#pv.lines, from + rows - 1) do
    local l = pv.lines[i]
    local ln = first + i - 1
    local hit = pv.at and ln == pv.at
    -- A prose line wrapped takes the rows it needs, a code line one.
    local r = row { height = (not pv.wrap) and PREVIEW_ROW or nil, min_height = pv.wrap and PREVIEW_ROW or nil,
      width = "grow", min_width = pv.wrap and 0 or nil, gap = 8, cross_align = "center",
      bg = hit and t.selection or nil }
    -- Numbered when the lines are a file's or a buffer's (`from` says
    -- where they start); a spec's lines are not.
    if pv.from then
      r[#r + 1] = text(string.format("%4d", ln), { family = "mono", size = PREVIEW_SIZE, color = t.faint })
    end
    if l ~= "" then
      -- The syntax's colours over the line, the tabs widened after
      -- the runs are cut so their bytes stay the text's.
      local spans = preview_spans(pv, i, l, hit and t.fg or t.muted)
      for _, sp in ipairs(spans) do sp[1] = sp[1]:gsub("\t", "    ") end
      r[#r + 1] = text(spans, { family = "mono", size = PREVIEW_SIZE, wrap = pv.wrap and "word" or "none" })
    end
    col[#col + 1] = r
  end
  return col
end


-- ---------------------------------------------------------- the picker

-- Whether a row's text wraps (`picker.wrap`), and where the preview
-- goes (`picker.preview`: `auto`, `right`, `below`, `off`; `true` and
-- `false`, its spellings before it was a word, are `auto` and `off`):
-- settings, the first flipped for the session by `<A-w>` (`<A-p>` is
-- the open picker's own, `P.shown`).
local function wrapping() return kawoosh.opt("picker.wrap") == true end
local function preview_mode()
  local v = kawoosh.opt("picker.preview")
  -- `:set picker.preview=false` arrives spelled, the setting a word.
  if v == false or v == "off" or v == "false" then return "off" end
  if v == "right" or v == "below" then return v end
  return "auto"
end
local function previewing() return preview_mode() ~= "off" end

-- A fraction setting, clamped; `picker.share` the pane's height,
-- `picker.split` the list's width beside the preview and
-- `picker.split_below` its height above one.
local function fraction(name, default)
  local v = tonumber(kawoosh.opt(name)) or default
  return math.max(0.1, math.min(0.9, v))
end
local function share() return fraction("picker.share", 0.5) end
local function split() return fraction("picker.split", 0.5) end
local function split_below() return fraction("picker.split_below", 0.5) end

-- The width of the divider between the list and the preview.
local DIVIDER = 4

-- The least each part of a picker is drawn at (docs/design/picker.md
-- Decision 1): the list `LIST_COLS` characters of its text wide and
-- `LIST_ROWS` rows high, the preview `PREVIEW_COLS` characters of its
-- own (smaller) text wide and `PREVIEW_MIN` lines under its title —
-- each with its padding, in the faces' sizes as they are (`sizes`).
-- Under the list the preview takes the list's whole width, so it asks
-- only `PREVIEW_COLS_BELOW` of it (picker.md Decision 9): a narrow,
-- tall pane reads a file's start there rather than nothing.
local LIST_COLS, PREVIEW_COLS, PREVIEW_COLS_BELOW = 30, 40, 24
local LIST_ROWS, PREVIEW_MIN = 4, 4
local function least()
  local cw, pw = SIZE * 0.6, PREVIEW_SIZE * 0.6
  return {
    list_w = LIST_COLS * cw + 16, preview_w = PREVIEW_COLS * pw + 16,
    below_w = PREVIEW_COLS_BELOW * pw + 16,
    list_h = LIST_ROWS * ROW_H, preview_h = 16 + (2 + PREVIEW_MIN) * PREVIEW_ROW,
  }
end
-- A place kept until its room falls this far under what it asks for
-- (picker.md Decision 9): a pane resized across a threshold flips the
-- preview once, not back and forth with every pixel the drag moves.
local KEEP = 0.9

-- picker.place(w, h, mode, forced): where a preview goes in a picker
-- whose body (the pane under its query row, as laid out) is `w` by `h`
-- px — `"right"`, `"below"`, or nil for none. `right` and `below` are
-- where they say, `off` none; `auto` beside the list when the width
-- holds both at their least and the height the preview's least lines,
-- else below it when the height holds both, else none — and `forced`
-- (`<A-p>` with none drawn) the one of the two with the more room,
-- whatever it is. `was`, where it is now, is kept while its room is
-- `KEEP` of what it asks for.
---@param w number
---@param h number
---@param mode "auto"|"right"|"below"|"off"
---@param forced? boolean
---@param was? "right"|"below"
---@return "right"|"below"|nil
function picker.place(w, h, mode, forced, was)
  if mode == "off" then return nil end
  if mode == "right" or mode == "below" then return mode end
  local m = least()
  local across = math.min(w / (m.list_w + DIVIDER + m.preview_w), h / math.max(m.list_h, m.preview_h))
  local down = math.min(h / (m.list_h + DIVIDER + m.preview_h), w / m.below_w)
  if across >= (was == "right" and KEEP or 1) then return "right" end
  if down >= (was == "below" and KEEP or 1) then return "below" end
  if forced then return across >= down and "right" or "below" end
  return nil
end

-- Where the open picker's preview goes in a body `w` by `h`: by the
-- setting, unless `<A-p>` said otherwise in this picker (`P.shown`):
-- false none, true one wherever the more room is.
local function placement(w, h)
  if P.src.follow or P.shown == false then return nil end
  local mode = preview_mode()
  if P.shown and mode == "off" then mode = "auto" end
  return picker.place(w, h, mode, P.shown == true, P.place)
end

-- The list's share of `room` px beside or above a preview: the
-- setting's, held so neither part is drawn under its least while the
-- room holds both.
local function share_of(room, frac, least_list, least_preview)
  local at = math.floor((room - DIVIDER) * frac)
  if room - DIVIDER >= least_list + least_preview then
    at = math.max(least_list, math.min(at, room - DIVIDER - least_preview))
  end
  return math.max(at, 0)
end

-- How many lines row `i` takes: one, or with wrapping what its text
-- folds to at the list's width (`P.cols` characters, an estimate from
-- the mono size — kui lays the real lines out and clips).
local function lines_of(i)
  if not P or not P.wrap then return 1 end
  local it = P.hits[i].item
  local cols = math.max(P.cols or 40, 1)
  local len
  if P.src.columns then
    -- Every cell folds to its column: the row is as tall as the
    -- tallest, a fixed column's width what the rows give it and the
    -- growing one's the rest — in pixels, a proportional face's
    -- glyph narrower than the mono's.
    local rest, lines = (P.list_w or 800) - 16, 1
    local grow = {}
    for j, c in ipairs(P.src.columns) do
      local n = utf8.len(tostring(it[c[1]] or "")) or 0
      if c.dim then
        local d = tostring(it[c.dim] or "")
        if d ~= "" then n = n + 2 + (utf8.len(d) or #d) end
      end
      local glyph = SIZE * (c.family == "mono" and 0.6 or 0.5)
      local w = fixed_width(c, P.widths or {}, j, P.list_w)
      if w then
        lines = math.max(lines, math.ceil(n * glyph / math.max(w, 1)))
        rest = rest - w - 8
      else
        grow[#grow + 1] = n * glyph
      end
    end
    for _, px in ipairs(grow) do
      lines = math.max(lines, math.ceil(px / math.max(rest, 40)))
    end
    return lines
  else
    len = utf8.len(it.text) or #it.text
    if it.sub and it.sub ~= "" then len = len + 2 + (utf8.len(it.sub) or #it.sub) end
  end
  return math.max(1, math.ceil(len / cols))
end

-- The window slid so the cursor is in it: up to the cursor, or down
-- until the rows from `top` to the cursor fill the budget.
local function ensure_visible()
  if not P then return end
  if P.cursor < 1 then P.cursor = 1 end
  if P.cursor > #P.hits then P.cursor = math.max(#P.hits, 1) end
  if P.cursor < P.top then P.top = P.cursor end
  local budget = math.max(P.rows or 1, 1)
  local used = 0
  local top = P.cursor
  while top > P.top do
    used = used + lines_of(top)
    if used + lines_of(top - 1) > budget then break end
    top = top - 1
  end
  if top > P.top then P.top = top end
  if P.top < 1 then P.top = 1 end
end

local function move(by)
  if not P then return end
  P.cursor = P.cursor + by
  ensure_visible()
end

-- A row down or up, round from the last to the first and back:
-- `<C-n>` `<C-p>` walk the list as a menu's keys do, where `j` `k`
-- stop at its ends.
local function cycle(by)
  if not P or #P.hits == 0 then return end
  P.cursor = (P.cursor - 1 + by) % #P.hits + 1
  ensure_visible()
end

-- The preview scrolled by `step` lines, held to its lines.
local function preview_by(step)
  if not P then return end
  local n = P.preview and #P.preview.lines or 0
  P.pv_top = math.max(1, math.min((P.pv_top or 1) + step, math.max(n - (P.prows or 1) + 1, 1)))
end

-- The wheel over the list or the preview: `dy` logical pixels (up is
-- positive), kept in an accumulator so a trackpad's small steps add
-- up to rows.
local function scroll(what, dy)
  if not P then return end
  if what == "preview" then
    P.pv_acc = (P.pv_acc or 0) - dy
    local step = P.pv_acc >= 0 and math.floor(P.pv_acc / PREVIEW_ROW) or -math.floor(-P.pv_acc / PREVIEW_ROW)
    if step ~= 0 then
      P.pv_acc = P.pv_acc - step * PREVIEW_ROW
      preview_by(step)
    end
    return
  end
  P.acc = (P.acc or 0) - dy
  local step = P.acc >= 0 and math.floor(P.acc / ROW_H) or -math.floor(-P.acc / ROW_H)
  if step ~= 0 then
    P.acc = P.acc - step * ROW_H
    P.top = math.max(1, math.min(P.top + step, math.max(#P.hits, 1)))
  end
end


-- The hits for `q` over a static source's items: the matcher's best,
-- ranked by `picker.rank`. With columns, the rows whose name matched
-- come first, ranked among themselves, then the ones the query found
-- elsewhere in (a key, the doc) — so `dir` lists the `dir` commands
-- before every command whose doc mentions a directory. A `wide`
-- source's the same over its `sub`: `main` lists the buffer `main.rs`
-- before `util.rs` in `main/`.
local function ranked(hits, q)
  local out = {}
  for i, h in ipairs(hits) do
    local item = P.items[h.index]
    out[i] = { item = item, positions = h.positions, score = h.score, rank = picker.rank(item, h), i = i }
  end
  if q ~= "" then
    table.sort(out, function(a, b)
      if a.rank ~= b.rank then return a.rank > b.rank end
      return a.i < b.i
    end)
  end
  return out
end

local function filter_static(q)
  if not P.matcher then P.hits = {} return end
  local out = ranked(P.matcher:query(q, LIMIT), q)
  if P.wide and q ~= "" and #out < LIMIT then
    local seen = {}
    for _, h in ipairs(out) do seen[h.item] = true end
    local more = {}
    for _, h in ipairs(P.wide:query(q, LIMIT)) do
      if not seen[P.items[h.index]] then more[#more + 1] = h end
    end
    for _, h in ipairs(ranked(more, q)) do
      if #out >= LIMIT then break end
      out[#out + 1] = h
    end
  end
  P.hits = out
end

-- A dynamic source asked for `q`: the job before it cancelled, this
-- one's answers appended as they come until it is done.
local function search_dynamic(q)
  if P.job then
    P.job.stale = true
    if P.job.cancel then pcall(P.job.cancel) end
    P.job = nil
  end
  P.hits = {}
  P.loading = false
  P.note = nil
  if q == "" then return end
  local job = { stale = false }
  local hits = P.hits
  function job.emit(items)
    if job.stale then return end
    for _, it in ipairs(items) do hits[#hits + 1] = { item = it } end
  end
  function job.done(note)
    if job.stale then return end
    P.loading = false
    P.note = note
    if P.job == job then P.job = nil end
  end
  P.job = job
  P.loading = true
  local ok, err = pcall(P.src.search, q, job)
  if not ok then
    P.loading = false
    P.note = tostring(err)
  end
end

-- The field's line is the query; a source's `query(q, ctx)` says what
-- of it is matched.
local function refilter(q)
  P.query = q
  P.cursor, P.top = 1, 1
  if P.src.query then q = P.src.query(q, P.ctx) or q end
  if P.src.search then search_dynamic(q) else filter_static(q) end
end

---What `picker.with_rows` takes besides the items and the rows.
---@class kawoosh.picker.WithRows
---@field matcher? kawoosh.Matcher the items' own, the rows appended to it; one is made without
---@field wide? kawoosh.Matcher a `wide` source's, the rows appended too
---@field by? table<string, number> the boosts, by path (`boosts()`); `{}` leaves the rows' order
---@field last? table<string, boolean> the extensions put last (`picker.binary`); `{}` none
---@field drop? table<string, any> paths listed elsewhere, left out besides the items' own

-- picker.with_rows(items, rows, opts): rows the engine holds listed
-- after a source's own `items`, each made an item only when the list
-- is indexed there — a row shown or ranked — by `rows.item`, else `{
-- text }` (docs/design/picker.md Decision 5). Rows with a `root` are
-- paths under it: those the items (or `opts.drop`) list already are
-- left out, the rest put in `boosted`'s order by the engine (ordering a
-- hundred thousand in Lua was 100 ms a load), and a row's item has its
-- `path` joined to the root and the boost `boosted` would have given
-- it, unless `rows.item` gave one. Answers the list (`items` itself,
-- `#` counting the rows) and a matcher over all of it; an `ipairs`
-- over the list makes every row. The picker and the launcher read
-- rows so.
---@param items kawoosh.picker.Item[]
---@param rows kawoosh.picker.Rows
---@param opts? kawoosh.picker.WithRows
---@return kawoosh.picker.Item[] list
---@return kawoosh.Matcher matcher
function picker.with_rows(items, rows, opts)
  opts = opts or {}
  local m, root, make = rows.matcher, rows.root, rows.item
  local n = #items
  local by, tail = {}, m:count() + 1
  if root then
    by = opts.by or boosts()
    local function rel(p)
      local r = fs.relative(p, root)
      if r ~= "." then return r end
    end
    local drop, boost = {}, {}
    for _, it in ipairs(items) do
      local r = it.path and rel(it.path)
      if r then drop[r] = true end
    end
    for p in pairs(opts.drop or {}) do
      local r = rel(p)
      if r then drop[r] = true end
    end
    for p, b in pairs(by) do
      local r = rel(p)
      if r then boost[r] = b end
    end
    tail = m:order { drop = drop, boost = boost, last = opts.last or picker.binary }
  end
  local all = m
  if n > 0 then
    all = opts.matcher
    if not all then
      local texts = {}
      for i, it in ipairs(items) do texts[i] = it.text end
      all = kawoosh.matcher(texts)
    end
    all:extend(m)
  end
  if opts.wide then opts.wide:extend(m) end
  local count = n + m:count()
  return setmetatable(items, {
    __index = function(t, k)
      if math.type(k) ~= "integer" or k <= n or k > count then return nil end
      local text, i = all:text(k), k - n
      local it = make and make(text, i) or { text = text }
      if root then
        it.path = it.path or fs.join(root, text)
        if it.boost == nil then it.boost = by[it.path] or (i >= tail and -0.5 or nil) end
      end
      rawset(t, k, it)
      return it
    end,
    __len = function() return count end,
  }), all
end

-- What a static source gave: kept, boosted, and a matcher over the
-- rows' text; rows the engine holds after them (`picker.with_rows`).
local function loaded(items, err, info)
  if not P then return end
  P.loading = false
  P.read = true
  P.capped = info and info.capped or false
  if not items then
    P.note = tostring(err)
    P.items = {}
    P.matcher = nil
    return
  end
  P.note = nil
  local by = boosts()
  items = boosted(items, by)
  local texts, wide = {}, {}
  local widened = P.src.columns or P.src.wide
  for i, it in ipairs(items) do
    if widened and not it.search then it.search = picker.search(it, P.src.columns) end
    texts[i] = it.text
    wide[i] = it.search or it.text
  end
  P.matcher = kawoosh.matcher(texts)
  -- The name is what a match on a row lights and ranks by; the rest
  -- of the row (`search`) is looked in after it.
  P.wide = widened and kawoosh.matcher(wide) or nil
  if info and info.rows then
    items, P.matcher = picker.with_rows(items, info.rows, { matcher = P.matcher, wide = P.wide, by = by })
  end
  P.items = items
  P.widths = P.src.columns and picker.widths(items, P.src.columns) or nil
  -- Where the cursor starts, while nothing is typed: the source's say,
  -- through the refilter the load sets off (`P.keep`).
  if (P.query or "") == "" and P.src.cursor and not P.started then
    P.started = true
    local ok, i = pcall(P.src.cursor, items, P.ctx)
    if ok and i then P.cursor, P.keep = i, i end
  end
  P.dirty = true
end

-- The source's items asked for: `load` on the io thread, `items` at
-- once; a `search` source has none until a query.
local function load_items()
  local src, ctx = P.src, P.ctx
  if src.search then return end
  P.loading = true
  if src.load then
    -- A load asked again (`<A-g>` twice, quickly) answers in any order:
    -- only the last one asked is kept.
    local this = P
    this.load_gen = (this.load_gen or 0) + 1
    local gen = this.load_gen
    src.load(ctx, function(items, err, info)
      if P ~= this or this.load_gen ~= gen then return end
      loaded(items, err, info)
    end)
  elseif type(src.items) == "function" then
    -- A second answer may hand rows the engine holds (`info.rows`).
    local items, info = src.items(ctx)
    loaded(items, nil, info)
  elseif src.items then
    loaded(src.items)
  else
    loaded({})
  end
end

-- What a pick answers a caller that asked for one (`kawoosh pick` from
-- a shell): the source's `answer(item)`, else the item's path, its
-- buffer's path or name, its text.
---@param src kawoosh.picker.Source
---@param item kawoosh.picker.Item
---@return string?
function picker.answer_of(src, item)
  if src.answer then return src.answer(item) end
  if item.path then return item.path end
  if item.buffer then
    local ok, p = pcall(kawoosh.buf.path, item.buffer)
    if ok and p then return p end
    local ok2, n = pcall(kawoosh.buf.name, item.buffer)
    if ok2 then return n end
  end
  return item.text
end

-- Whether `item` is in the buffer the picker was opened from, and at
-- which byte: its `offset`, or its line and column there.
local function in_origin(item)
  local h = P and P.ctx.buffer
  if not h or not item then return nil end
  if item.buffer and item.buffer ~= h then return nil end
  if not item.buffer then
    if not item.path then return nil end
    local ok, path = pcall(kawoosh.buf.path, h)
    if not ok or path ~= item.path then return nil end
  end
  if item.offset then return item.offset end
  if not item.line then return nil end
  local ok, at = pcall(kawoosh.buf.offset, item.line, item.col or 1, h)
  return ok and at or nil
end

-- The wash over what a following picker shows: the item's line, and
-- faintly the rest of its range (a symbol's body).
local FOLLOW_PAINT = "picker follow"
local FOLLOW_LINE, FOLLOW_RANGE = 0.28, 0.08
local LINE_END = 1 << 30

local function unwash()
  if P and P.washed then
    pcall(kawoosh.buf.paint, FOLLOW_PAINT, {}, P.ctx.buffer)
    P.washed = nil
  end
end

-- A `follow` source's cursor row shown in the pane it came from: the
-- caret there, the line mid-pane, washed — and a symbol's whole range
-- faintly; where the caret and the view were before the first one is
-- kept for `unfollow`. An item elsewhere takes the wash away.
local function follow(item)
  local at = in_origin(item)
  if not at then return unwash() end
  local h = P.ctx.buffer
  if not P.back then
    local ok, c = pcall(kawoosh.buf.cursor, h)
    if not ok then return end
    P.back = { offset = c.offset, top = c.top }
  end
  kawoosh.buf.set_cursor(at, h, { center = true })
  local line = item.line
  if not line then
    local ok, c = pcall(kawoosh.buf.cursor, h)
    line = ok and c.line or nil
  end
  if line then
    local from = kawoosh.buf.offset(line, 1, h)
    local spans = {}
    if item.end_line and item.end_line > line then
      spans[1] = { from, kawoosh.buf.offset(item.end_line, LINE_END, h), "accent", bg = FOLLOW_RANGE }
    end
    spans[#spans + 1] = { from, kawoosh.buf.offset(line, LINE_END, h), "accent", bg = FOLLOW_LINE }
    kawoosh.buf.paint(FOLLOW_PAINT, spans, h)
    P.washed = true
  end
end

-- The caret and the view put back as they were before the picker
-- followed its rows, and the wash gone.
local function unfollow()
  unwash()
  if P and P.back then
    pcall(kawoosh.buf.set_cursor, P.back.offset, P.ctx.buffer, { top = P.back.top })
    P.back = nil
  end
end

-- picker.close(): the open picker closed, the keys back where they came
-- from — for a source's key that acts somewhere else.
---@return nil
local function close()
  if P then
    unfollow()
    if P.job and P.job.cancel then pcall(P.job.cancel) end
    -- A caller waiting on this picker gets nothing: it was closed, or
    -- another took its place.
    if P.answer then P.answer(nil) end
    picker.last = { source = P.name, query = P.query, cursor = P.cursor }
    P = nil
  end
  kawoosh.view_close(VIEW)
end

picker.close = close

-- The default pick: a buffer shown, a file opened at its line, a
-- `run` line run, or the source's own `pick`.
local function pick(how)
  if not P then return end
  local hit = P.hits[P.cursor]
  if not hit then return kawoosh.echo("nothing to pick") end
  local item, src = hit.item, P.src
  local answer = P.answer
  P.answer = nil
  P.back = nil
  close()
  if answer then return answer(item) end
  if src.pick then return src.pick(item, how) end
  if item.pick then return item.pick(item, how) end
  if item.buffer then
    kawoosh.buf.show(item.buffer, { split = how })
    -- A pick is a jump however near (docs/design/jumps.md Decision 2).
    local at = item.offset
    if not at and item.line then
      local ok, o = pcall(kawoosh.buf.offset, item.line, item.col or 1, item.buffer)
      at = ok and o or nil
    end
    if at then kawoosh.buf.set_cursor(at, item.buffer, { jump = true }) end
  elseif item.path then
    kawoosh.open(item.path, { line = item.line, col = item.col, split = how })
  elseif item.run then
    kawoosh.run(item.run)
  end
end

-- picker.open(name | def[, opts]): the picker on a registered source by
-- name, or on a definition given whole — `{ title =, items = {…} |
-- load = fn(ctx, done) | search = fn(query, job), pick = fn(item,
-- how), answer = fn(item), preview = fn(item), keys = { ["<C-x>"] =
-- fn(item) }, columns = {…} (as `picker.rows` takes them), icon =
-- FIELD (each row's kind icon, `picker.rows`'s `icon`), wide =
-- true (a row's `sub` matched too, after its text, as a row with
-- columns is on its cells after its name), query = fn(query, ctx)
-- (the text matched for what was typed), launcher = true (a module of
-- the launcher too, launcher.lua), launch = fn(item) (what its row does
-- there, filling the bare pane, when `pick` goes elsewhere) }`. An item
-- is `{ text =, sub =, path =, line =, col =, buffer =, offset =, run
-- =, boost = }`, and a column's field. `opts`: `query`, `cursor`,
-- `root` (the directory a source that walks or searches starts from,
-- the working one by default) and `answer = fn(item)`, which takes the
-- pick instead of the source — `fn(nil)` when the picker closes
-- untaken. A source's `ctx` is `{ buffer =, cwd =, root =, terminal =,
-- caret = }`, `terminal` when it was opened from a terminal pane,
-- `caret` (`kawoosh.buf.cursor`'s) when from a buffer. A picker already
-- open switches to it.
---@param what string|kawoosh.picker.Source a registered source's name, or one given whole
---@param opts? kawoosh.picker.OpenOpts
---@return nil
function picker.open(what, opts)
  opts = opts or {}
  local name, src
  if type(what) == "string" then
    name, src = what, picker.sources[what]
    if not src then return kawoosh.echo("no picker source named " .. what) end
  else
    name, src = what.name or "custom", what
  end
  if P and P.job and P.job.cancel then pcall(P.job.cancel) end
  if P and P.answer then P.answer(nil) end
  unfollow()
  host_cache = {}
  -- `terminal`: opened from a terminal pane, where a pick goes back to;
  -- `pane`, that pane's id, for a source that renders to its size.
  local ctx = { buffer = kawoosh.buf.current(), cwd = fs.cwd(), root = opts.root or fs.cwd(),
                terminal = kawoosh.holds("terminal") == true, pane = kawoosh.pane() }
  if ctx.buffer and not ctx.terminal then
    local ok, c = pcall(kawoosh.buf.cursor, ctx.buffer)
    if ok then ctx.caret = c end
  end
  P = { name = name, src = src, ctx = ctx, items = {}, hits = {}, cursor = opts.cursor or 1, top = 1,
        query = nil, loading = false, rows = 20, answer = opts.answer }
  kawoosh.view_open(VIEW, { below = true, share = share() })
  kawoosh.field_set(VIEW, FIELD, opts.query or "")
  kawoosh.field_focus(VIEW, FIELD)
  load_items()
end

-- `kawoosh pick SOURCE [QUERY]` from a shell (`Request::Pick`): the
-- picker on the source, and the caller answered with the pick —
-- `picker.answer_of` — or with nothing when it closes untaken.
---@param token integer the request's, for `kawoosh._answer`
---@param source string
---@param query string empty for none
---@return nil
function kawoosh._pick_request(token, source, query)
  local src = picker.sources[source]
  if not src then
    kawoosh._answer(token, nil)
    return kawoosh.echo("no picker source named " .. source)
  end
  local answered = false
  picker.open(source, { query = query ~= "" and query or nil, answer = function(item)
    if answered then return end
    answered = true
    kawoosh._answer(token, item and picker.answer_of(src, item) or nil)
  end })
end

-- picker.reload(): the open picker's items read again on the next
-- frame — after a key of the source's own changed what they are, a
-- buffer closed — the query and the cursor kept.
---@return nil
function picker.reload()
  if not P then return end
  P.stale = true
end

-- picker.source(name, def): a source to open by name, and its
-- `:picker NAME [here]` command.
---@param name string
---@param def kawoosh.picker.Source
---@return nil
function picker.source(name, def)
  picker.sources[name] = def
  kawoosh.command("picker " .. name, function(ctx)
    local where = ctx.args[1]
    if where == "here" then return picker.open(name, { root = picker.here() }) end
    if where then return kawoosh.echo("picker " .. name .. ": `here` or nothing, not " .. where) end
    picker.open(name)
  end, {
    args = { "text" },
    doc = "the picker on " .. (def.title or name) .. "; `here`, from the file's directory",
  })
  for key in pairs(def.keys or {}) do
    if not picker._keys[key] then
      picker._keys[key] = true
      kawoosh.map("i", key, "picker key " .. key, { view = VIEW, field = FIELD })
      kawoosh.map("n", key, "picker key " .. key, { view = VIEW, field = FIELD })
    end
  end
end
picker._keys = {}

-- picker.state(): what the open picker shows — `source`, `query`,
-- `cursor` (a row's index from 1), `top`, `count` (the rows), `text`
-- (the cursor's row), `item` (its item), `positions` (the bytes its
-- match lit, in its `picker.search` text), `loading`, `preview` (the
-- cursor's, as drawn: `title`, `lines`, `runs` once highlighted),
-- `rows` (every hit's text, in order) and `root` (where a walk starts)
-- — or nil when none is open; for a status line, a test, a plugin's
-- key.
---@return kawoosh.picker.State?
function picker.state()
  if not P then return nil end
  local hit = P.hits[P.cursor]
  local rows, items = {}, {}
  for i, h in ipairs(P.hits) do rows[i], items[i] = h.item.text, h.item end
  return { source = P.name, query = P.query or "", cursor = P.cursor, top = P.top, count = #P.hits,
           text = hit and hit.item.text or nil, item = hit and hit.item or nil,
           positions = hit and hit.positions or nil, loading = P.loading,
           preview = P.preview, rows = rows, items = items, root = P.ctx.root,
           place = P.place, ignored = P.ctx.ignored == true, capped = P.capped == true }
end

-- picker.resume(): the last picker again, its query and cursor as
-- they were.
---@return nil
function picker.resume()
  local l = picker.last
  if not l or not picker.sources[l.source] then return kawoosh.echo("no picker to resume") end
  picker.open(l.source, { query = l.query, cursor = l.cursor })
end

-- ------------------------------------------------------------ the view

-- The keys' legend (every pane's `<A-/>`, picker.md Decision 10): the
-- source's own first, then the picker's.
local function legend_items()
  local items = {}
  for _, it in ipairs(P.src.legend or {}) do items[#items + 1] = it end
  for _, it in ipairs {
    { "<CR>", "open" }, { "<C-v>", "beside" }, { "<C-s>", "below" }, { "<C-t>", "tab" },
    { { "<C-j>", "<C-k>" }, "rows" }, { { "<A-j>", "<A-k>" }, "scroll preview" },
    { { "<A-h>", "<A-l>" }, "list smaller, bigger" }, { "<A-p>", "preview" }, { "<A-w>", "wrap" },
  } do items[#items + 1] = it end
  if P.src.ignored then items[#items + 1] = { "<A-g>", "ignored files" } end
  items[#items + 1] = { "<Esc>", "normal mode" }
  items[#items + 1] = { "<C-c>", "close" }
  return items
end

-- The head as the pane's width holds it (picker.md Decision 7): the
-- title, `>`, the query's field — never under `FIELD_MIN` characters
-- — the count and the chips. What does not fit goes in this order:
-- the chips' words (their caps stay, the key said), the count to its
-- short form (`12/4031`), the title, the count, the chips. Measured,
-- so a font zoomed asks for more.
local FIELD_MIN = 12
local function head_of(ctx, w, parts)
  local env, t = ctx.env, ctx.env.theme
  local plain, small = { size = SIZE, wrap = "none" }, { size = SMALL, wrap = "none" }
  local function tw(s, style) return env.measure_text(s, style).width end
  -- A chip: its caps (⌥ an icon on a Mac, `alt+` before the key on a
  -- PC, three px a side and the border), its word four px after.
  local function chip_w(c, words)
    local caps = kawoosh.os == "mac" and (SMALL + 1 + tw(c.key, small)) or tw("alt+" .. c.key, small)
    return 12 + caps + 8 + (words and 4 + tw(c.word, small) or 0)
  end
  local GAP = 8
  local function width(level)
    local n, total = 0, 16
    local function add(x) total, n = total + x, n + 1 end
    if level < 3 then add(tw(parts.title, plain)) end
    add(tw(">", plain))
    add(FIELD_MIN * SIZE * 0.6)
    if level < 2 then add(tw(parts.count, small)) elseif level < 4 then add(tw(parts.short, small)) end
    if level < 5 then for _, c in ipairs(parts.chips) do add(chip_w(c, level < 1)) end end
    return total + GAP * (n - 1)
  end
  local level = 0
  while level < 5 and width(level) > w do level = level + 1 end
  local head = row {
    width = "grow", height = ROW_H + 4, pad = { x = 8 }, gap = GAP, cross_align = "center",
    bg = t.surface,
  }
  if level < 3 then head[#head + 1] = text(parts.title, { size = SIZE, color = t.muted, wrap = "none" }) end
  head[#head + 1] = text(">", { family = "mono", size = SIZE, color = t.accent })
  head[#head + 1] = ctx.field { name = FIELD, placeholder = P.src.placeholder or "type to filter", size = SIZE,
                                width = "grow" }
  local count = level < 2 and parts.count or level < 4 and parts.short or nil
  if count then head[#head + 1] = text(count, { size = SMALL, color = t.faint, wrap = "none" }) end
  if level < 5 then
    for _, c in ipairs(parts.chips) do
      local fg = c.on and t.bg or t.faint
      local chip = row { pad = { x = 6 }, height = ROW_H - 2, radius = 4, gap = 4, cross_align = "center",
        bg = c.on and t.accent or nil, hover_bg = not c.on and t.sunken or nil, on_click = { kind = c.kind },
        ctx.keys("<A-" .. c.key .. ">", { size = SMALL, color = fg, border = c.on and fg or nil }) }
      if level < 1 then chip[#chip + 1] = text(c.word, { size = SMALL, color = fg, wrap = "none" }) end
      head[#head + 1] = chip
    end
  end
  return head
end

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  if not P then
    return column { pad = 12, text("no picker open", { color = t.muted }) }
  end
  if P.stale then
    P.stale = nil
    P.keep = P.cursor
    load_items()
  end
  local q = ctx.field_text(FIELD)
  if q ~= P.query or P.dirty then
    P.dirty = nil
    local cursor = (P.query == nil) and P.cursor or P.keep
    P.keep = nil
    refilter(q)
    if cursor then P.cursor = cursor ensure_visible() end
  end
  -- The pane's height as it is — a divider drag's — kept as the
  -- setting, so the picker opens next at the size it was left.
  if ctx.share and math.abs(ctx.share - share()) > 0.005 then
    kawoosh.opt("picker.share", math.max(0.1, math.min(0.9, ctx.share)))
  end
  local h = (ctx.height or 0) > 0 and ctx.height or 400
  local w = (ctx.width or 0) > 0 and ctx.width or 800
  -- The body under the query row, as laid out, says where the preview
  -- goes (`picker.place`) — every frame, so a pane or a window resized
  -- moves it.
  -- The legend while it is whole (`<A-/>`), under the list: its rows
  -- out of the body's, as tall as it was laid out last frame.
  local legend = ctx.legend(legend_items(), { size = SMALL })
  local legend_h = 0
  if legend then
    local box = ctx.env.layout_of("picker legend")
    legend_h = box and box.h or 2 * ROW_H
  end
  local bh = math.max(h - ctx.title_h - ROW_H - 4 - legend_h, 0)
  P.body_w, P.body_h = w, bh
  local place = placement(w, bh)
  P.place = place
  if P.beside ~= (place == "right") then
    P.beside = place == "right"
    kawoosh.fact(PREVIEW_FACT, P.beside)
  end
  P.wrap = wrapping()
  -- The list's width: its share of the pane beside a preview, the
  -- whole of it else; and in characters, for the wrap estimate. Its
  -- height the share above a preview below it, else the body's.
  local m = least()
  local list_h = bh
  P.list_w = w
  if place == "right" then
    P.list_w = share_of(w, split(), m.list_w, m.preview_w)
  elseif place == "below" then
    list_h = share_of(bh, split_below(), m.list_h, m.preview_h)
  end
  P.cols = math.floor((P.list_w - 16) / (SIZE * 0.6))
  local rows = math.max(math.floor(list_h / ROW_H), 1)
  P.rows = rows
  if P.top > math.max(#P.hits, 1) then P.top = math.max(#P.hits, 1) end
  if P.top < 1 then P.top = 1 end
  -- A reload (`<A-g>`, a source read again) keeps the count it had
  -- until its answer lands: one answered by the next frame would blink
  -- `reading…` and back. The `<A-g>` chip, lit at once, says the key
  -- was heard. Only a first read, with nothing to count yet, says so.
  local reading = P.loading and (P.src.search or not P.read)
  -- The count, and its short form for a narrow head.
  local count, short
  if reading then
    count = P.src.search and "searching…" or "reading…"
    short = "…"
  elseif P.note then
    count, short = P.note, "!"
  elseif #P.items > 0 and #P.hits < #P.items then
    count = #P.hits .. " of " .. #P.items
    short = #P.hits .. "/" .. #P.items
  else
    count = tostring(#P.hits)
    short = count
  end
  -- A walk stopped at its cap says so: what was not walked is not here.
  if P.capped and not reading then
    count = count .. " · walk stopped at " .. #P.items
    short = short .. "+"
  end
  -- The head's chips: `<A-g>`'s state, on a source that has it, lit
  -- while on; `<A-p>` where no preview has room (below).
  local chips = {}
  if P.src.ignored then
    chips[1] = { key = "g", word = "ignored", kind = "ignored", on = P.ctx.ignored == true }
  end
  local indent = P.src.tree and (P.query or "") == "" and function(it) return it.depth or 0 end or nil
  -- A row the pointer left is forgotten by the next frame: a row
  -- entered after it is the pointer come back to the list.
  P.left_row = nil
  local list = picker.rows(ctx, P.hits, { top = P.top, cursor = P.cursor, rows = rows, wrap = P.wrap, lines = lines_of,
                                          columns = P.src.columns, widths = P.widths, width = P.list_w,
                                          indent = indent, hover = "row", icon = P.src.icon, cut = P.src.cut })
  if #P.hits == 0 and not reading then
    list[#list + 1] = row { pad = { x = 8, y = 4 }, text((P.query ~= "" and "no matches") or P.src.empty or "nothing here", { size = SIZE, color = t.muted }) }
  end
  local body = (place == "below" and column or row) { width = "grow", height = "grow", gap = 0, list }
  if place == "right" then list.width = P.list_w end
  if place == "below" then list.height = list_h end
  local hit = P.hits[P.cursor]
  if P.src.follow and hit and P.followed ~= hit.item then
    P.followed = hit.item
    follow(hit.item)
  end
  if place then
    local pv
    if hit then
      local key = hit.item
      if P.preview_for ~= key then
        P.preview_for = key
        P.pv_top, P.pv_acc = 1, 0
        local fn = P.src.preview or preview_of
        local ok, got = pcall(fn, hit.item, ctx)
        P.preview = ok and got or { title = "preview failed", lines = { tostring(got) } }
      end
      pv = P.preview
    else
      P.preview_for, P.preview = nil, nil
    end
    -- The preview's room: the rest of the body beside or under the list.
    local room = place == "right" and { width = w - P.list_w - DIVIDER, height = bh }
                 or { width = w, height = bh - list_h - DIVIDER }
    local prows = math.max(math.floor((room.height - 16 - 2 * PREVIEW_ROW) / PREVIEW_ROW), 1)
    P.prows = prows
    -- The divider: dragged, the list's share follows the pointer.
    body[#body + 1] = splitter(ctx.env, { key = "picker divider", thickness = DIVIDER,
      dir = place == "below" and "column" or "row", on_drag = { kind = "divide" } })
    body[#body + 1] = picker.preview(ctx, pv, prows, P.pv_top, room)
  else
    -- None drawn, none kept: `picker.state().preview` is what is drawn.
    P.preview_for, P.preview = nil, nil
  end
  if not place and not P.src.follow and previewing() and P.shown == nil then
    -- No room for one: the way to it anyway, `<A-p>`, at the head's end.
    chips[#chips + 1] = { key = "p", word = "preview", kind = "preview" }
  end
  local title = (P.src.title or P.name) .. (P.ctx.root ~= P.ctx.cwd and (" in " .. short_path(P.ctx.root) .. "/") or "")
  local head = head_of(ctx, w, { title = title, count = count, short = short, chips = chips })

  local out = column { width = "grow", height = "grow", clip = true, gap = 0, head, body }
  if legend then
    out[#out + 1] = row { key = "picker legend", width = "grow", pad = { x = 8, y = 4 }, bg = t.surface,
                          on_layout = { kind = "laid" }, legend }
  end
  return out
end, function(ev)
  if not P then return end
  if ev.kind == "scroll" then
    scroll(ev.tag and ev.tag.kind, ev.dy or 0)
  elseif ev.kind == "drag" then
    -- The divider under the pointer: its x over the body's width is
    -- the list's share, kept for the session.
    local par = ev.parent or {}
    if ev.phase ~= "end" and P.place == "below" and par.h and par.h > 0 then
      local at = ((ev.y or 0) - (par.y or 0)) / par.h
      kawoosh.opt("picker.split_below", math.max(0.1, math.min(0.9, at)))
    elseif ev.phase ~= "end" and par.w and par.w > 0 then
      local at = ((ev.x or 0) - (par.x or 0)) / par.w
      kawoosh.opt("picker.split", math.max(0.1, math.min(0.9, at)))
    end
  elseif ev.kind == "preview" then
    kawoosh.run("picker preview")
  elseif ev.kind == "ignored" then
    kawoosh.run("picker ignored")
  elseif ev.kind == "hover" then
    -- The pointer moving from one row onto another takes the cursor
    -- there — the pane follows it too. Rows scrolled or refiltered under
    -- a pointer at rest are `by = "content"`, not the pointer moving;
    -- nor is the pointer come to the list to wheel it, an enter with no
    -- row left before it.
    if ev.by ~= "pointer" then
      P.left_row = nil
    elseif ev.phase == "leave" then
      P.left_row = true
    elseif P.left_row then
      P.left_row = nil
      P.cursor = ev.tag.i
    end
  elseif ev.kind == "row" then
    kawoosh.field_focus(VIEW, FIELD)
    if ev.i == P.cursor then pick() else P.cursor = ev.i ensure_visible() end
  end
  -- A key with the field blurred is pane mode's: the maps below under
  -- `p`, and every pane's shared keys (`<C-w>…`, `<leader>…`, `:`).
end, { session = false })

-- ------------------------------------------------------- the commands

-- The keys are the picker's own (docs/design/local-maps.md): its
-- query's, and its pane's with the query blurred.
local at = { view = VIEW, field = FIELD }
local on_pane = { view = VIEW }
local function on(name, fn, doc)
  kawoosh.command("picker " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("query", function() kawoosh.field_focus(VIEW, FIELD) end, "the keys to the query field")
on("next", function() move(1) end, "the cursor a row down")
on("prev", function() move(-1) end, "the cursor a row up")
on("cycle next", function() cycle(1) end, "the cursor a row down, from the last to the first")
on("cycle prev", function() cycle(-1) end, "the cursor a row up, from the first to the last")
on("page down", function() move(P and P.rows or 10) end, "the cursor a page down")
on("page up", function() move(-(P and P.rows or 10)) end, "the cursor a page up")
on("first", function() if P then P.cursor = 1 ensure_visible() end end, "the cursor on the first row")
on("last", function() if P then P.cursor = #P.hits ensure_visible() end end, "the cursor on the last row")
on("pick", function() pick() end, "take the cursor's row")
on("pick vsplit", function() pick("vsplit") end, "take the cursor's row into a split beside")
on("pick split", function() pick("split") end, "take the cursor's row into a split below")
on("pick tab", function() pick("tab") end, "take the cursor's row into a new tab")
on("close", function() close() end, "close the picker")
-- The preview by half of its height a step, COUNT steps.
local function half() return math.max(math.floor((P and P.prows or 10) / 2), 1) end
kawoosh.command("picker preview down", function(ctx) preview_by(half() * math.max(ctx.count or 1, 1)) end,
  { when = { PANE_FACT }, doc = "the preview half a screen down, COUNT times" })
kawoosh.command("picker preview up", function(ctx) preview_by(-half() * math.max(ctx.count or 1, 1)) end,
  { when = { PANE_FACT }, doc = "the preview half a screen up, COUNT times" })
-- `<A-p>`: this picker's preview put away when drawn, shown when not —
-- where there is the more room, even where `picker.preview` is `off` or
-- `auto` finds too little for one. The setting is left as it is: the
-- next picker opens by it again.
on("preview", function()
  if not P then return end
  P.shown = not P.place
  -- The fact at once, for a width key before the next frame.
  if P.body_w then
    P.place = placement(P.body_w, P.body_h)
    P.beside = P.place == "right"
    kawoosh.fact(PREVIEW_FACT, P.beside)
  end
end, "show the cursor's row beside or under the list, or not, in this picker")
-- `<A-g>`, in a source that has it (`files`, `grep`): ignored and
-- hidden files too, or not — this picker's, read again at once.
on("ignored", function()
  if not P then return end
  if not P.src.ignored then return kawoosh.echo("picker " .. P.name .. ": lists no ignored files") end
  P.ctx.ignored = not P.ctx.ignored
  if P.src.search then P.dirty = true else P.keep = P.cursor load_items() end
end, "ignored and hidden files too, or not, in a picker that walks or searches (files, grep)")
on("wrap", function() kawoosh.opt("picker.wrap", not wrapping()) end,
  "fold a row's text to the list's width, or cut it (the `picker.wrap` setting, for the session)")
-- The list's share against the preview, stepped (picker.md Decision
-- 6): `<A-l>` bigger, `<A-h>` smaller, along whichever way the preview
-- lies — the divider right and left beside the list, down and up under
-- it — the setting of that way (`picker.split`, `picker.split_below`)
-- for the session. With no preview the list is the whole pane: said,
-- nothing moved. The pane itself is `<A-S-h>` `<A-S-j>` `<A-S-k>`
-- `<A-S-l>`, the editor's, whatever the picker draws.
local function list_by(step)
  if not P then return end
  local name = P.place == "right" and "picker.split" or P.place == "below" and "picker.split_below" or nil
  if not name then return kawoosh.echo("picker: no preview to size the list against (<A-p> draws one)") end
  local now = name == "picker.split" and split() or split_below()
  kawoosh.opt(name, math.max(0.1, math.min(0.9, now + step)))
end
on("list bigger", function() list_by(0.05) end,
  "the list bigger against the preview: the divider right beside it, down under it (for the session)")
on("list smaller", function() list_by(-0.05) end,
  "the list smaller against the preview: the divider left beside it, up under it (for the session)")
-- Their names before Decision 6, kept for a map of one's own: beside
-- only, as they were.
local function beside(name, fn, doc)
  kawoosh.command("picker " .. name, fn, { when = { PANE_FACT, PREVIEW_FACT }, doc = doc })
end
beside("list wider", function() kawoosh.opt("picker.split", math.min(0.9, split() + 0.05)) end,
  "the list wider beside the preview (the `picker.split` setting, for the session)")
beside("list narrower", function() kawoosh.opt("picker.split", math.max(0.1, split() - 0.05)) end,
  "the list narrower beside the preview (the `picker.split` setting, for the session)")
kawoosh.command("picker key", function(ctx)
  if not P then return end
  local key = ctx.args[1]
  local fn = P.src.keys and P.src.keys[key]
  local hit = P.hits[P.cursor]
  -- A key about the list rather than a row (`<C-a>` in buffers) runs
  -- on an empty one too, the item nil.
  if fn then fn(hit and hit.item, P) end
end, { when = { PANE_FACT }, args = { "text" }, doc = "a source's own key on the cursor's row" })

for _, mode in ipairs { "i", "n" } do
  kawoosh.map(mode, "<CR>", "picker pick", at)
  kawoosh.map(mode, "<Down>", "picker next", at)
  kawoosh.map(mode, "<Up>", "picker prev", at)
  kawoosh.map(mode, "<C-n>", "picker cycle next", at)
  kawoosh.map(mode, "<C-p>", "picker cycle prev", at)
  kawoosh.map(mode, "<C-j>", "picker next", at)
  kawoosh.map(mode, "<C-k>", "picker prev", at)
  kawoosh.map(mode, "<PageDown>", "picker page down", at)
  kawoosh.map(mode, "<PageUp>", "picker page up", at)
  kawoosh.map(mode, "<C-v>", "picker pick vsplit", at)
  kawoosh.map(mode, "<C-s>", "picker pick split", at)
  kawoosh.map(mode, "<C-t>", "picker pick tab", at)
  kawoosh.map(mode, "<C-c>", "picker close", at)
end
-- Normal mode over the field, and pane mode with the field blurred
-- (`<Esc>` again): the same list keys.
for _, m in ipairs { { "n", at }, { "p", on_pane } } do
  local mode, where = m[1], m[2]
  kawoosh.map(mode, "<Esc>", "picker close", where)
  kawoosh.map(mode, "j", "picker next", where)
  kawoosh.map(mode, "k", "picker prev", where)
  kawoosh.map(mode, "<C-d>", "picker page down", where)
  kawoosh.map(mode, "<C-u>", "picker page up", where)
  kawoosh.map(mode, "gg", "picker first", where)
  kawoosh.map(mode, "G", "picker last", where)
  kawoosh.map(mode, "J", "picker preview down", where)
  kawoosh.map(mode, "K", "picker preview up", where)
end
kawoosh.map("p", "<CR>", "picker pick", on_pane)
kawoosh.map("p", "<Down>", "picker next", on_pane)
kawoosh.map("p", "<C-n>", "picker cycle next", on_pane)
kawoosh.map("p", "<C-p>", "picker cycle prev", on_pane)
kawoosh.map("p", "<Up>", "picker prev", on_pane)
kawoosh.map("p", "q", "picker close", on_pane)
kawoosh.map("p", "<C-c>", "picker close", on_pane)
kawoosh.map("p", "i", "picker query", on_pane)
kawoosh.map("p", "a", "picker query", on_pane)
kawoosh.map("p", "/", "picker query", on_pane)
for _, mode in ipairs { "i", "n", "p" } do
  local where = mode == "p" and on_pane or at
  kawoosh.map(mode, "<A-p>", "picker preview", where)
  kawoosh.map(mode, "<A-w>", "picker wrap", where)
  kawoosh.map(mode, "<A-g>", "picker ignored", where)
  -- Alt inside the picker, from the query too — where the editor's
  -- would move or indent the query's one line: the preview scrolled,
  -- the list sized against it.
  kawoosh.map(mode, "<A-j>", "picker preview down", where)
  kawoosh.map(mode, "<A-k>", "picker preview up", where)
  kawoosh.map(mode, "<A-h>", "picker list smaller", where)
  kawoosh.map(mode, "<A-l>", "picker list bigger", where)
end
-- The keys' legend from the query too, as pane mode has it.
for _, mode in ipairs { "i", "n" } do kawoosh.map(mode, "<A-/>", "legend", at) end
-- Alt with Shift is the pane, as in the editor's own modes, which
-- pane mode has not.
for k, c in pairs { h = "pane narrower", l = "pane wider", j = "pane shorter", k = "pane taller" } do
  kawoosh.map("p", "<A-S-" .. k .. ">", c, on_pane)
end

-- The completion's candidates (`<C-x>` in insert mode, `lsp
-- candidates`): a row per candidate — its label, its kind, the
-- server's detail (a signature, a type) — the query the word typed so
-- far, the cursor's signature and documentation as the preview,
-- `⏎` taking one into the text (`lsp accept N`). The rows are what
-- `kawoosh.lsp.candidates()` says while the picker is up; what a server
-- left out of its candidates (`completionItem/resolve`) the preview
-- asks for, as the popup's panel does.
local CANDIDATE_COLUMNS = {
  { "text", family = "mono", min = 140, max = 360, share = 0.4 },
  { "kind", muted = true, min = 70, max = 140, share = 0.15 },
  { "detail", family = "mono", muted = true, grow = true },
}
-- A detail's first line, for its row: a server's may be several (a
-- TypeScript class's interface and its constructor).
local function first_line(s) return s:match("^[^\n]*") end

-- A candidate's preview lines: its detail over its documentation, the
-- text it puts when it has neither; and how many of them are the
-- detail, which is code in the buffer's language, not markdown.
local function candidate_lines(item)
  local lines = {}
  local detail = item.signature
  for l in (detail ~= "" and (detail .. "\n") or ""):gmatch("(.-)\n") do lines[#lines + 1] = l end
  local code = #lines
  if item.documentation ~= "" then
    if #lines > 0 then lines[#lines + 1] = "" end
    for l in (item.documentation .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = l end
  end
  if #lines == 0 then lines[1] = item.insert end
  return lines, code
end
picker.source("candidates", {
  title = "candidates", placeholder = "a candidate",
  columns = CANDIDATE_COLUMNS, icon = "kind",
  items = function(ctx)
    local cs = kawoosh.lsp.candidates()
    local items = {}
    if not cs then return items end
    local lok, language = pcall(kawoosh.buf.language, ctx.buffer)
    for _, c in ipairs(cs) do
      items[#items + 1] = {
        text = c.label, kind = c.kind, detail = first_line(c.detail), signature = c.detail, index = c.index,
        insert = c.insert, documentation = c.documentation, language = lok and language or nil,
        boost = c.index == cs.current and 1 or 0,
      }
    end
    return items
  end,
  pick = function(item) kawoosh.run("lsp accept " .. item.index) end,
  -- Kept on the item: the server asked once, its answer landing in
  -- the preview however the cursor walked meanwhile.
  preview = function(item)
    if item.shown then return item.shown end
    local title = item.text .. (item.kind ~= "" and ("  ·  " .. item.kind) or "")
    local pv = { title = title, language = "markdown", code_language = item.language, wrap = true }
    pv.lines, pv.code = candidate_lines(item)
    item.shown = pv
    if item.documentation == "" then
      pv.note = "asking the server…"
      kawoosh.lsp.resolve(item.index, function(c)
        pv.note = nil
        if not c then return end
        if c.detail ~= "" then item.signature, item.detail = c.detail, first_line(c.detail) end
        item.documentation = c.documentation
        pv.lines, pv.code = candidate_lines(item)
        pv.runs, pv.asked = nil, nil
      end)
    end
    return pv
  end,
  empty = "no candidate matches",
})

-- The code actions a server offered (`gra`, `lsp action`): a
-- row per action — its title, its kind — searched by title, the preview
-- what taking it does (its edit as a diff, a command it runs), `⏎`
-- taking one (`lsp action N`). The rows are what `kawoosh.lsp.actions()`
-- says: an offer until one is taken, `picker resume` having it back
-- till then.
picker.source("actions", {
  title = "code actions", placeholder = "an action",
  columns = {
    { "text", grow = true },
    { "kind", muted = true, min = 70, max = 180, share = 0.25 },
  },
  items = function()
    local items = {}
    for _, a in ipairs(kawoosh.lsp.actions() or {}) do
      items[#items + 1] = { text = a.title, kind = a.kind, index = a.index, diff = a.preview }
    end
    return items
  end,
  pick = function(item) kawoosh.run("lsp action " .. item.index) end,
  preview = function(item)
    return { title = item.text, lines = item.diff, language = "diff" }
  end,
  empty = "no action matches",
})

-- What the project can compile (`<leader>cC`, `compile pick`,
-- docs/design/compile.md Decisions 3, 6 and 7): `compile.default`, the
-- named `compile.commands`, the lines run here, and the commands its
-- files offer — the name and command, what said so beside it (`…` when
-- it wants arguments), the preview where it runs,
-- why and how it is declared; `⏎` runs it (`compile pick N`), or puts
-- one wanting arguments in the prompt to finish, as `<C-e>` does any
-- (`compile edit N`). The rows are what `kawoosh.compile_offer()` says,
-- in its order: the tiers of Decision 19 (a query ranks by the match,
-- equal ones in that order).
picker.source("compile", {
  title = "compile", placeholder = "a command · <C-e> adds arguments first",
  legend = { { "<C-e>", "arguments first" } },
  keys = { ["<C-e>"] = function(item)
    if not item then return end
    picker.close()
    kawoosh.run("compile edit " .. item.index)
  end },
  columns = {
    { "text", grow = true },
    { "from", muted = true, min = 90, max = 240, share = 0.3 },
  },
  items = function()
    local items = {}
    for _, o in ipairs(kawoosh.compile_offer() or {}) do
      local text = (o.name and (o.name .. "  ") or "") .. o.cmd .. (o.needs and " …" or "")
      items[#items + 1] = { text = text, from = o.from, index = o.index, cmd = o.cmd,
                            cwd = o.cwd, why = o.why, detail = o.detail }
    end
    return items
  end,
  pick = function(item) kawoosh.run("compile pick " .. item.index) end,
  preview = function(item)
    local lines = { "$ " .. item.cmd, "in " .. item.cwd, "from " .. item.from }
    if item.why ~= "" then
      lines[#lines + 1] = ""
      lines[#lines + 1] = item.why
    end
    if #item.detail > 0 then
      lines[#lines + 1] = ""
      for _, l in ipairs(item.detail) do lines[#lines + 1] = l end
    end
    return { title = item.text, lines = lines }
  end,
  empty = "no command matches",
})

-- `:picker [SOURCE]`: bare, the smart one.
kawoosh.command("picker", function(ctx)
  picker.open(ctx.args[1] or "smart")
end, { args = { "text" }, doc = "the picker on SOURCE (files, buffers, recent, smart, grep, lines, symbols, workspace_symbols, marks, pins, workspaces, dirs, commands, tools, compile, domains)" })
kawoosh.command("picker resume", function() picker.resume() end, { doc = "the last picker again, where it was left" })

-- -------------------------------------------------------- the sources

-- picker.here(): the directory "here" is — a `dir` listing's own, the
-- buffer's file's, else the working one — what `:picker files here`
-- (`<leader>F`) and `:picker grep here` (`<leader>G`) start from.
---@return string
function picker.here()
  local h = kawoosh.buf.current()
  local ok, name = pcall(kawoosh.buf.name, h)
  local listed = ok and name and name:match("^dir: (.+)$")
  if listed then return listed end
  local pok, path = pcall(kawoosh.buf.path, h)
  if pok and path then return fs.parent(path) or fs.cwd() end
  return fs.cwd()
end

-- Every file git sees under the root (the working directory, or where
-- `here` said), relative to it — all of a root the project ignores (a
-- `node_modules`), and with `<A-g>` every file but `.git` — at most
-- the walk's cap, said when it stopped there (picker.md Decision 4).
-- The rows stay the engine's, a matcher of them, made items only as
-- they are shown (Decision 5).
local function walk_items(ctx, done)
  local root = ctx.root or ctx.cwd
  fs.walk(root, function(m, err, _, capped)
    if not m then return done(nil, err) end
    done({}, nil, { capped = capped, rows = { matcher = m, root = root } })
  end, { ignored = ctx.ignored == true, matcher = true })
end

-- A path typed whole — absolute, or from `~` — is matched as the rows
-- spell it, from the root, when it is under the root.
local function from_root(q, ctx)
  if not (q:match("^[/~]") or q:match("^%a:[/\\]")) then return q end
  local rel = fs.relative(q, ctx.root or ctx.cwd)
  if rel and rel ~= "." then return rel end
  return q
end

picker.source("files", {
  title = "files", placeholder = "find a file", cut = "start",
  load = walk_items, ignored = true,
  query = from_root,
  empty = "no files under " .. fs.cwd(),
})

-- The listed buffers, the current one last: `<leader><leader><CR>` is the
-- one before it. The focused tab's (`buffers.scope = "tab"`, roadmap
-- step 30) or every one; `<C-a>` in the picker flips it. A row is the
-- name and the path as `short_path` writes it, both matched (`wide`):
-- a directory typed finds the buffers under it, a name's hit first.
local function buffer_items(ctx)
  local items, current = {}, nil
  local tab = kawoosh.opt("buffers.scope") ~= "all"
  for _, h in ipairs(kawoosh.buf.list { tab = tab }) do
    local ok, name = pcall(kawoosh.buf.name, h)
    if ok and not name:match("^%*lua:") then
      local pok, path = pcall(kawoosh.buf.path, h)
      local mod = pcall(kawoosh.buf.modified, h) and kawoosh.buf.modified(h)
      local it = { text = name, sub = (pok and path) and short_path(path) or "", path = pok and path or nil,
                   buffer = h, boost = 0, modified = mod }
      if mod then it.sub = it.sub .. " [+]" end
      if h == ctx.buffer then current = it else items[#items + 1] = it end
    end
  end
  if current then items[#items + 1] = current end
  return items
end

-- `<C-x>` on a row: its buffer closed as `:bd` closes it, the list
-- read again; one with unsaved changes is asked about first.
local function close_row(item)
  if not item then return end
  local function shut()
    kawoosh.buf.close(item.buffer, { force = true })
    picker.reload()
  end
  if item.modified then
    kawoosh.confirm {
      title = "Close " .. item.text .. "? Its changes are not saved.",
      lines = { "close " .. item.text .. ", the changes dropped" },
      actions = { { "Discard", shut }, { "Keep" } },
    }
  else
    shut()
  end
end

-- `<C-a>`: the tab's buffers or all of them, for the session.
local function flip_scope()
  local all = kawoosh.opt("buffers.scope") == "all"
  kawoosh.opt("buffers.scope", all and "tab" or "all")
  kawoosh.echo(all and "buffers: this tab's" or "buffers: every tab's")
  picker.reload()
end

picker.source("buffers", {
  title = "buffers", placeholder = "find a buffer · <C-x> closes one · <C-a> this tab's or all",
  items = buffer_items, wide = true,
  keys = { ["<C-x>"] = close_row, ["<C-a>"] = flip_scope },
  legend = { { "<C-x>", "close buffer" }, { "<C-a>", "this tab's or all" } },
  empty = "no buffers in this tab · <C-a> for every tab's",
})

-- The files attended before (the memory's `file` rows, the
-- workspace's — memory.md D2), newest first, each at the line it was
-- left; ranked by the memory.
local function recent_items()
  local items = {}
  local by = kawoosh.memory_rank and kawoosh.memory_rank.boosts("file", 500) or {}
  for _, f in ipairs(kawoosh.oldfiles(500)) do
    items[#items + 1] = { text = short_path(f.path), path = f.path, line = f.line, boost = by[f.path] or 0 }
  end
  return items
end

-- The workspaces worked in before (roadmap step 32, workspaces.md
-- Decisions 11 and 13): each root the memory has files under, newest
-- first, the one in front left out, with the file last attended there.
-- A pick opens a new tab on it — its directory the workspace, that file
-- open at its line, the workspace as it was left, or the root listed
-- when it has none — or goes to the tab already on it; `<C-o>` moves
-- the tab in front there instead, as a pick did before. A section of
-- the launcher too (`launcher = true`), where a row fills the bare pane
-- in place, as `<C-o>` does (`launch`).
local function workspace_items()
  local by, order = {}, {}
  local here = fs.cwd()
  for _, r in ipairs(kawoosh.memory { kind = "file", limit = 2000 }) do
    local ws = r.workspace
    -- Not the one in front, nor one holding it.
    if ws and ws ~= "" and not fs.relative(here, ws) then
      local w = by[ws]
      if not w then
        w = { ws = ws, last = -1 }
        by[ws] = w
        order[#order + 1] = w
      end
      if r.last > w.last then
        w.last, w.path, w.line = r.last, r.subject, (r.meta and r.meta.line or 0) + 1
      end
    end
  end
  table.sort(order, function(a, b) return a.last > b.last end)
  local items = {}
  for _, w in ipairs(order) do
    local name = fs.basename(w.ws) or w.ws
    local file = w.path and (fs.relative(w.path, w.ws) or w.path)
    items[#items + 1] = { text = name, sub = short_path(w.ws) .. (file and ("  " .. file) or ""),
                          ws = w.ws, file = w.path, at = w.line }
  end
  return items
end

-- The tab in front moved to the workspace, its file opened in the pane
-- the keys are in (or the root listed): `<C-o>`, and a launcher's row.
local function workspace_here(item)
  if not item then return end
  fs.chdir(item.ws)
  if item.file and fs.exists(item.file) then
    kawoosh.open(item.file, { line = item.at })
  else
    -- The path as it is, not through a command line's words.
    kawoosh.dir.open(item.ws)
  end
end

-- The index of a tab, not the one in front, whose directory is in the
-- workspace: the workspace is open there already.
local function workspace_tab(ws)
  for _, t in ipairs(kawoosh.tabs()) do
    if not t.active and fs.relative(t.cwd, ws) then return t.index end
  end
end

-- `<CR>`: the tab on the workspace — the one there is, else a new one,
-- its directory the workspace and its file open (as `<C-t>` in the
-- directory jumps makes a tab on a directory, dirs.lua).
local function workspace_tab_on(item)
  if not item then return end
  local i = workspace_tab(item.ws)
  if i then return kawoosh.run("tab goto " .. i) end
  if item.file and fs.exists(item.file) then
    kawoosh.open(item.file, { split = "tab", line = item.at })
  else
    kawoosh.open(item.ws, { split = "tab" })
  end
  -- After the open, so it is the new tab's directory that moves.
  fs.chdir(item.ws)
end

picker.source("workspaces", {
  title = "workspaces", placeholder = "a project worked in before · <C-o> moves this tab there", launcher = true,
  legend = { { "<C-o>", "move this tab there" } },
  items = workspace_items,
  pick = workspace_tab_on,
  launch = workspace_here,
  keys = { ["<C-o>"] = function(item)
    if not item then return end
    picker.close()
    workspace_here(item)
  end },
  empty = "no other workspace in the memory",
})

-- The workspace's pinned files, in pin order (`<leader>mp` is the
-- pane).
picker.source("pins", {
  title = "pins", placeholder = "a pinned file",
  items = function()
    local items = {}
    for _, r in ipairs(kawoosh.memory { pinned = true, workspace = true }) do
      if r.kind == "file" then
        items[#items + 1] = {
          text = "#" .. r.pinned .. "  " .. short_path(r.subject), path = r.subject,
          line = (r.meta and r.meta.line or 0) + 1, boost = 100 - r.pinned,
        }
      end
    end
    return items
  end,
  empty = "nothing pinned (<leader>ma pins the buffer's file)",
})

picker.source("recent", {
  title = "recent", placeholder = "find a file opened before", cut = "start",
  items = recent_items,
  empty = "no files opened before",
})

-- Buffers, then the files opened before, then every file under the
-- working directory, each path once.
picker.source("smart", {
  title = "smart", placeholder = "find a buffer, a recent file, a file", cut = "start",
  load = function(ctx, done)
    local items, seen = {}, {}
    -- The buffers ranked by the memory among themselves, the current
    -- one last (`<leader>.<CR>` is the one before it), a pin above all.
    local by = boosts()
    local bufs = buffer_items(ctx)
    for i, it in ipairs(bufs) do
      if it.path then seen[it.path] = true end
      it.text = it.path and short_path(it.path) or it.text
      it.sub = it.modified and "[+]" or ""
      local b = it.path and by[it.path] or 0
      if b >= 10 then it.boost = b
      elseif i == #bufs then it.boost = 1
      else it.boost = 1 + b end
      items[#items + 1] = it
    end
    for _, it in ipairs(recent_items()) do
      if not seen[it.path] then
        seen[it.path] = true
        items[#items + 1] = it
      end
    end
    -- The walk's rows after them, those listed already left out there.
    walk_items(ctx, function(_, _, info)
      done(items, nil, info and { rows = info.rows })
    end)
  end,
})

-- `rg` on the query, as it is typed: `path:line:col: text` a row each,
-- the file at the line on `<CR>`. The rows are what rg found — not
-- fuzzy-filtered again — and a search past `GREP_MAX` lines is
-- stopped where it is. The query goes to rg as one argument, no shell
-- between: `cmd.exe`, Windows' shell when `SHELL` is unset, reads no
-- single quotes, and searched `'fn` in a file `helper'`.
picker.source("grep", {
  title = "grep", placeholder = "a pattern for rg", ignored = true,
  search = function(q, job)
    local root = P and P.ctx.root or fs.cwd()
    local n, odd = 0, {}
    local token
    local argv = { "rg", "--vimgrep", "--color", "never", "--smart-case", "--max-columns", "300" }
    -- `<A-g>`: rg's own spelling of everything but `.git`.
    if P and P.ctx.ignored then
      for _, a in ipairs { "--no-ignore", "--hidden", "--glob", "!.git" } do argv[#argv + 1] = a end
    end
    for _, a in ipairs { "--", q, "." } do argv[#argv + 1] = a end
    token = kawoosh.spawn(
      argv,
      {
        cwd = root,
        on_lines = function(lines)
          local items = {}
          for _, l in ipairs(lines) do
            local path, ln, col, rest = l:match("^(.-):(%d+):(%d+):(.*)$")
            if path then
              path = path:gsub("^%.[/\\]", "")
              local full = fs.join(root, path)
              items[#items + 1] = {
                text = path .. ":" .. ln,
                -- As the file's pane draws the line: whole when it is
                -- inside a value opened above it (a `.env`'s quoted one).
                sub = kawoosh.secrets.mask_text((rest:gsub("^%s+", "")), full, { line = tonumber(ln) }),
                path = full, line = tonumber(ln), col = tonumber(col),
              }
            elseif l ~= "" then
              odd[#odd + 1] = l
            end
          end
          job.emit(items)
          n = n + #lines
          if n >= GREP_MAX and token then
            kawoosh.kill(token)
            token = nil
          end
        end,
        on_exit = function(code)
          local note
          -- No code and not killed here: it never started.
          if code == nil and token then note = "rg did not start (is it installed?)" end
          if code == 2 and odd[1] then note = odd[1] end
          if n >= GREP_MAX then note = "stopped at " .. GREP_MAX .. " lines" end
          job.done(note)
        end,
      })
    job.cancel = function()
      if token then kawoosh.kill(token) token = nil end
    end
  end,
  empty = "no matches",
})

kawoosh.setting("symbols.source", {
  type = { "auto", "lsp", "syntax" },
  doc = "where a buffer's symbols come from: its server's with what the grammar's outline adds — locals, headings — (`auto`), or only one",
})

-- Symbols as rows: the name, its kind (and a short detail — an
-- `impl`'s trait, a method's receiver), where it sits — the symbols it is
-- inside, `a › b`, or its file — `⏎` going there. A buffer's are in
-- the file's order, each with its `depth`, the tree the picker draws.
--
-- With the buffer's `lines`, a row reads as its line does: its `head`
-- is what stands before the name on the name's line — `pub fn`,
-- `impl`, `struct`, `##`, a C field's type — when the name is found
-- there at its column, and the kind is left to it; a bare name (a
-- method with no keyword, a key) keeps the kind faint as its `tag`.
-- `at` is where the head and the name lie (bytes from 1 in the line),
-- for the syntax's colours (`colour_symbols`).
local function symbol_rows(items, root, lines)
  local rows = {}
  local path = {}
  for i, s in ipairs(items or {}) do
    local depth = s.depth or 0
    for d = #path, depth + 1, -1 do path[d] = nil end
    local where = #path > 0 and table.concat(path, " › ") or (s.container or "")
    path[depth + 1] = s.name
    if root and s.path then
      local rel = fs.relative(s.path, root) or s.path
      where = (where ~= "" and (where .. " · ") or "") .. rel .. ":" .. s.line
    end
    -- A short detail rides faint beside the kind: a server's signature,
    -- an `impl`'s trait.
    local short = s.detail and s.detail ~= "" and not s.detail:find("\n") and #s.detail <= 40 and s.detail or nil
    local head, at
    local l = lines and s.line and s.col and lines[s.line]
    if l then
      local b = utf8.offset(l, s.col)
      if b and l:sub(b, b + #s.name - 1) == s.name then
        local from = l:find("%S") or b
        head = l:sub(from, b - 1):match("^(.-)%s*$")
        if head == "" then head = nil end
        at = { line = s.line, from = from, name = b, to = b + #s.name - 1 }
      end
    end
    rows[i] = {
      text = s.name, kind = s.kind, short = short, sub = where, detail = s.detail,
      head = head, at = at, tag = not head and not s.name:find(" ", 1, true) and s.kind or nil,
      path = s.path, line = s.line, col = s.col, depth = depth, end_line = s.end_line,
    }
  end
  return rows
end

-- The most text the syntax's colours are read for: a file past it
-- lists plain.
local SYMBOLS_COLOUR_MAX = 1024 * 1024

-- The rows coloured as the pane paints them: the buffer's text
-- highlighted once (`kawoosh.highlight`, on the ts thread), and each
-- row's `head` turned into spans in the tokens' colours, its name's
-- `color` the token's it starts in. The rows are in the file's order
-- and the runs in the text's, so one walk over the runs serves every
-- row; the next frame paints what came.
local function colour_symbols(rows, lines, path, language)
  local text = table.concat(lines, "\n")
  if #text == 0 or #text > SYMBOLS_COLOUR_MAX then return end
  local starts, off = {}, 1
  for i, l in ipairs(lines) do
    starts[i] = off
    off = off + #l + 1
  end
  kawoosh.highlight(text, { path = path, language = language }, function(runs)
    if not runs then return end
    local targets = {}
    for _, r in ipairs(rows) do
      if r.at then targets[#targets + 1] = r end
    end
    table.sort(targets, function(a, b) return starts[a.at.line] + a.at.from < starts[b.at.line] + b.at.from end)
    local k = 1
    for _, r in ipairs(targets) do
      local base = starts[r.at.line]
      local from, name, to = base + r.at.from - 1, base + r.at.name - 1, base + r.at.to - 1
      while k <= #runs and runs[k].to < from do k = k + 1 end
      -- The head's spans: the runs reaching into it, the rest uncoloured.
      if r.head then
        local spans, at, j = {}, from, k
        while j <= #runs and runs[j].from < name do
          local run = runs[j]
          local a, b = math.max(run.from, at), math.min(run.to, name - 1)
          if b >= a then
            if a > at then spans[#spans + 1] = { text:sub(at, a - 1) } end
            spans[#spans + 1] = { text:sub(a, b), color = run.color, bold = run.bold, italic = run.italic }
            at = b + 1
          end
          j = j + 1
        end
        if at < name then spans[#spans + 1] = { text:sub(at, name - 1) } end
        -- The space before the name is the row's own.
        local last = spans[#spans]
        if last then last[1] = last[1]:match("^(.-)%s*$") if last[1] == "" then spans[#spans] = nil end end
        if #spans > 0 then r.head = spans end
      end
      local j = k
      while j <= #runs and runs[j].to < name do j = j + 1 end
      local run = runs[j]
      if run and run.from <= to and run.to >= name then r.color = run.color end
    end
  end)
end

-- picker.symbol_at(items, line): the innermost row whose symbol holds
-- `line` — the last one starting at or before it whose range reaches
-- it — else the last starting before it.
---@param items { line?: integer, end_line?: integer }[] in the file's order
---@param line integer from 1
---@return integer? index the row's, from 1; nil when none starts at or before `line`
function picker.symbol_at(items, line)
  local inside, before
  for i, it in ipairs(items) do
    if (it.line or 0) > line then break end
    before = i
    if (it.end_line or it.line) >= line then inside = i end
  end
  return inside or before
end

-- picker.merge_symbols(server, outline): a server's symbols with the
-- grammar's the server did not list — a local variable, a heading —
-- one list in the file's order, nested again by the lines each holds:
-- a grammar's symbol is the server's when it starts on the same line
-- and the server's name holds its name (`impl Marks` holds `Marks`).
---@param server? kawoosh.lsp.Symbol[] the server's (`kawoosh.lsp.symbols`)
---@param outline? kawoosh.lsp.Symbol[] the grammar's
---@return kawoosh.lsp.Symbol[] # the same tables, `depth` set again and `order` (their place before the sort) added
function picker.merge_symbols(server, outline)

  local by_line = {}
  local out = {}
  for _, s in ipairs(server or {}) do
    out[#out + 1] = s
    local l = by_line[s.line] or {}
    l[#l + 1] = s.name
    by_line[s.line] = l
  end
  for _, s in ipairs(outline or {}) do
    local dup = false
    for _, name in ipairs(by_line[s.line] or {}) do
      if name:find(s.name, 1, true) then dup = true break end
    end
    if not dup then out[#out + 1] = s end
  end
  for i, s in ipairs(out) do s.order = i end
  table.sort(out, function(x, y)
    if x.line ~= y.line then return x.line < y.line end
    local xe, ye = x.end_line or x.line, y.end_line or y.line
    if xe ~= ye then return xe > ye end
    return x.order < y.order
  end)
  local open = {}
  for _, s in ipairs(out) do
    while #open > 0 and open[#open] < s.line do open[#open] = nil end
    s.depth = #open
    open[#open + 1] = s.end_line or s.line
  end
  return out
end

-- A buffer's symbols as `symbols.source` says: `auto` the server's and
-- the grammar's merged (the grammar's alone without a server), or one.
local function buffer_symbols(buffer, done)
  local source = kawoosh.opt("symbols.source") or "auto"
  if source ~= "auto" then
    return kawoosh.lsp.symbols({ buffer = buffer, source = source }, done)
  end
  local got, answers = 0, {}
  local function back(which)
    return function(items, err)
      answers[which] = { items = items, err = err }
      got = got + 1
      if got < 2 then return end
      local s, o = answers.lsp, answers.syntax
      if not s.items and not o.items then return done(nil, s.err or o.err) end
      done(picker.merge_symbols(s.items, o.items))
    end
  end
  kawoosh.lsp.symbols({ buffer = buffer, source = "lsp" }, back("lsp"))
  kawoosh.lsp.symbols({ buffer = buffer, source = "syntax" }, back("syntax"))
end

-- The buffer's symbols (`grs`): a tree in the file's order, each row
-- reading as its line does in the syntax's colours (`pub fn name`,
-- `impl S`, `## Heading`), the cursor on the one the caret is in, the
-- pane following the cursor.
picker.source("symbols", {
  title = "symbols", placeholder = "a symbol in this buffer",
  tree = true, follow = true, icon = "kind",
  columns = {
    { "text", grow = true, family = "mono", lead = "head", color = "color" },
    { "tag", dim = "short", muted = true, min = 70, max = 200, share = 0.25 },
    { "sub", muted = true, min = 60, max = 260, share = 0.3, path = true },
  },
  load = function(ctx, done)
    local h = ctx.buffer
    -- A private buffer's lines as a list shows them (the heads read
    -- from them, and the colours).
    local lines = h and shown_lines(h) or nil
    buffer_symbols(h, function(items, err)
      if not items then return done(nil, err) end
      local rows = symbol_rows(items, nil, lines)
      done(rows)
      if lines then
        local ok, path = pcall(kawoosh.buf.path, h)
        colour_symbols(rows, lines, ok and path or nil, kawoosh.buf.language(h))
      end
    end)
  end,
  cursor = function(items, ctx)
    return ctx.caret and picker.symbol_at(items, ctx.caret.line)
  end,
  empty = "no symbols",
})

-- The marks (`<leader>'`, `:marks`; docs/design/marks.md): the
-- memory's `mark` rows of the workspace — this file's first, then the
-- capitals, then every other file's — each its letter, its line's
-- text, where it is; an adrift one says so. The pane follows the
-- cursor over this file's; `<C-x>` deletes the row's mark.
local function mark_rows(ctx)
  local here = ctx.buffer and select(2, pcall(kawoosh.buf.path, ctx.buffer)) or nil
  local rows = {}
  for _, r in ipairs(kawoosh.memory { kind = "mark", workspace = true, limit = 10000 }) do
    local m = r.meta or {}
    local name = r.subject:sub(1, 1)
    local local_ = name:match("%l") ~= nil
    local rank = (local_ and m.path == here) and 0 or (local_ and 2 or 1)
    local text = (m.text or ""):gsub("^%s+", "")
    rows[#rows + 1] = {
      text = name .. "  " .. text, name = name, subject = r.subject, rank = rank,
      sub = (m.adrift and "adrift · " or "") .. short_path(m.path or "") .. ":" .. tostring(m.line or 1),
      path = m.path, line = m.line, col = m.col,
    }
  end
  table.sort(rows, function(a, b)
    if a.rank ~= b.rank then return a.rank < b.rank end
    if a.path ~= b.path then return (a.path or "") < (b.path or "") end
    return a.name < b.name
  end)
  return rows
end

picker.source("marks", {
  title = "marks", placeholder = "a mark", follow = true,
  legend = { { "<C-x>", "forget mark" } },
  items = mark_rows,
  keys = { ["<C-x>"] = function(item)
    if not item then return end
    kawoosh.forget("mark", item.subject)
    picker.reload()
  end },
  empty = "no marks — `m` and a letter sets one",
})
kawoosh.command("marks", function() picker.open("marks") end, { doc = "the marks, in the picker" })

-- The workspace's symbols matching the query (`grS`), asked of
-- the server of the buffer the picker was opened from as it is typed.
picker.source("workspace_symbols", {
  title = "workspace symbols", placeholder = "a symbol anywhere",
  follow = true, icon = "kind",
  columns = {
    { "text", grow = true },
    { "kind", muted = true, min = 70, max = 160, share = 0.2 },
    { "sub", muted = true, min = 60, max = 320, share = 0.4 },
  },
  search = function(q, job)
    local root = fs.cwd()
    kawoosh.lsp.symbols({ workspace = true, query = q, buffer = P.ctx and P.ctx.buffer },
      function(items, err)
        job.emit(symbol_rows(items, root))
        job.done(err)
      end)
  end,
  empty = "no symbol matches",
})

-- The buffer's lines, the caret put on the one taken and on the
-- cursor's as it moves — by its number, the buffer finding the byte:
-- `kawoosh.buf.lines` drops a `\r\n` whole, so a count of the lines'
-- lengths would fall a byte short a line in a CRLF file.
picker.source("lines", {
  title = "lines", placeholder = "find a line", follow = true,
  items = function(ctx)
    local h = ctx.buffer
    if not h then return {} end
    local lines = shown_lines(h)
    local items = {}
    local width = #tostring(#lines)
    for i, l in ipairs(lines) do
      items[i] = { text = string.format("%" .. width .. "d  %s", i, l), buffer = h, line = i, lines = lines }
    end
    return items
  end,
  preview = function(item)
    local from = math.max(item.line - 10, 1)
    local window = {}
    for i = from, math.min(#item.lines, from + PREVIEW_LINES) do window[#window + 1] = item.lines[i] end
    local ok, name = pcall(kawoosh.buf.name, item.buffer)
    return { title = ok and name or "", lines = window, from = from, at = item.line }
  end,
  empty = "no lines",
})

-- The registry (kui.md D12), what `:commands` was: every spec a row —
-- its name with its forms marked, the first key bound to it and what
-- it does, or in the danger colour why it cannot run where the
-- keyboard came from — `<CR>` running the cursor's command or opening
-- the command line on it when it takes arguments, and the spec in
-- full as the preview: aliases, the forms and what each means, the
-- conditions and which hold, the keys in every mode, the subcommands.
-- The rows' columns: the name with its first alias faint, the first
-- key bound to it, and what it does — or why it cannot run here.
local COMMAND_COLUMNS = {
  { "text", dim = "alias", family = "mono", min = 160, max = 340, share = 0.36 },
  { "key", family = "mono", muted = true, min = 100, max = 220, share = 0.26 },
  { "doc", grow = true },
}

local function command_items()
  local specs = kawoosh.commands()
  local names = {}
  for _, s in ipairs(specs) do names[#names + 1] = s.name end
  local items = {}
  for _, s in ipairs(specs) do
    local marks = s.name .. (s.bang and "!" or "") .. (s.query and "?" or "")
    local can = kawoosh.can(s.name)
    local it = { text = marks, name = s.name, spec = s, can = can, keys = s.keys,
                 alias = s.aliases[1] and (":" .. s.aliases[1]) or "",
                 key = s.keys[1] or "",
                 doc = can == true and s.doc or "" }
    if #s.args == 0 then it.run = s.name else it.cmdline = s.name .. " " end
    -- The subcommands: the names one word longer.
    local subs = {}
    for _, n in ipairs(names) do
      local rest = n:sub(1, #s.name + 1) == s.name .. " " and n:sub(#s.name + 2) or nil
      if rest and not rest:find(" ", 1, true) then subs[#subs + 1] = rest end
    end
    it.subs = subs
    -- Which conditions hold, asked where the keyboard is now.
    local when = {}
    for _, c in ipairs(s.when) do
      local neg = c:sub(1, 1) == "!"
      local fact = neg and c:sub(2) or c
      local holds = kawoosh.holds(fact) ~= neg
      when[#when + 1] = c .. (holds and " (holds)" or " (does not hold)")
    end
    it.when = when
    items[#items + 1] = it
  end
  return items
end

picker.source("commands", {
  title = "commands", placeholder = "search commands, keys, docs",
  items = command_items,
  columns = COMMAND_COLUMNS,
  pick = function(item)
    if item.cmdline then kawoosh.cmdline(item.cmdline) else kawoosh.run(item.run) end
  end,
  preview = function(item)
    local s = item.spec
    local lines = { "does      " .. s.doc }
    local aliases = {}
    for _, a in ipairs(s.aliases) do aliases[#aliases + 1] = ":" .. a end
    lines[#lines + 1] = "aliases   " .. (#aliases > 0 and table.concat(aliases, " ") or "none")
    lines[#lines + 1] = "takes     " .. (#s.args > 0 and table.concat(s.args, " ") or "nothing")
    if s.bang then lines[#lines + 1] = "with !    " .. s.bang end
    if s.query then lines[#lines + 1] = "with ?    " .. s.query end
    lines[#lines + 1] = "when      " .. (#item.when > 0 and table.concat(item.when, " · ") or "always")
    lines[#lines + 1] = "keys      " .. (#item.keys > 0 and table.concat(item.keys, " · ") or "not bound")
    if #item.subs > 0 then lines[#lines + 1] = "subcommands  " .. table.concat(item.subs, " ") end
    local title = item.can == true and (s.name .. " · can run here") or item.can
    return { title = title, lines = lines }
  end,
  empty = "no command matches",
})

-- `:commands [QUERY]`, `<leader>ic`: the registry as a picker.
kawoosh.command("commands", function(ctx)
  picker.open("commands", { query = ctx.args[1] })
end, {
  aliases = { "cmds" },
  args = { "command" },
  doc = "every command as a picker, searched as you type; QUERY starts the search",
})

-- The tools the config registered (`kawoosh.tool`), run on `<CR>`.
local WHERE = { column = "in a column of its own", under = "under the pane", dock = "in the dock" }
picker.source("tools", {
  title = "tools", placeholder = "find a tool",
  items = function()
    local items = {}
    for _, t in ipairs(kawoosh.tools()) do
      items[#items + 1] = { text = t.name, sub = t.cmd .. (t.cwd and ("  in " .. t.cwd) or ""), run = "tool " .. t.name,
                            preview = { title = t.name, lines = { t.cmd, t.cwd and ("in " .. t.cwd) or "in the file's directory", WHERE[t.place] } } }
    end
    return items
  end,
  empty = "no tools registered (kawoosh.tool in init.lua)",
})

-- The shells on the PATH (`kawoosh.fs.shells`), `<CR>` a terminal whose
-- own process is the one picked (`:terminal shell NAME`); `:terminal
-- shell` with no name opens it. The one a terminal runs by default
-- (`terminal.shell`, else `$SHELL`) is first and says so, a row of its
-- own when it is a path the PATH does not hold.
local function shell_stem(path)
  local name = (fs.basename(path) or path):lower()
  return (name:gsub("%.exe$", ""))
end

picker.source("shells", {
  title = "shells", placeholder = "a shell for a new terminal",
  columns = {
    { "text", grow = true },
    { "is", muted = true, min = 50, max = 80, share = 0.1 },
    { "where", muted = true, min = 120, max = 420, share = 0.6 },
  },
  items = function()
    local default = kawoosh.opt("terminal.shell")
    if default == nil or default == "" then default = os.getenv("SHELL") end
    local stem = default and default ~= "" and shell_stem(default)
    local items, seen = {}, false
    for _, s in ipairs(fs.shells() or {}) do
      local is_default = s.name == stem
      seen = seen or is_default
      local row = {
        text = s.name, is = is_default and "default" or "", where = fs.short(s.path),
        run = "terminal shell " .. s.name, boost = is_default and 1 or 0,
        preview = { title = s.name, lines = { s.path, "a terminal whose own process is " .. s.name } },
      }
      table.insert(items, is_default and 1 or #items + 1, row)
    end
    if stem and not seen then
      table.insert(items, 1, {
        text = stem, is = "default", where = fs.short(default), run = "terminal shell " .. default, boost = 1,
        preview = { title = stem, lines = { default, "a terminal whose own process is " .. stem } },
      })
    end
    return items
  end,
  empty = "no shell found on the PATH",
})

-- The machines within reach (docs/design/domains.md W3): the settings'
-- domains, `~/.ssh/config`'s hosts, WSL's distros, as `domain pick`
-- last published them (`kawoosh.domains()`), each with what it is —
-- `ssh: box`, `wsl: Ubuntu-24.04` — and how it stands in words, what
-- that means in the preview. `<CR>` opens a tab on one, its home listed
-- — connected first when it is down; `<C-o>` connects it and stays;
-- `<C-x>` disconnects it, the row saying so once it has.
local DOMAIN_STATE = {
  up = "connected", down = "not connected", connecting = "connecting…", failed = "failed",
  dropped = "disconnected",
}

picker.source("domains", {
  title = "domains", placeholder = "a machine · <C-o> connects · <C-x> disconnects",
  columns = {
    { "text", grow = true },
    { "what", muted = true, min = 120, max = 320, share = 0.4 },
    { "state", muted = true, min = 80, max = 180, share = 0.2 },
  },
  items = function()
    local items = {}
    for _, d in ipairs(kawoosh.domains() or {}) do
      local state = (DOMAIN_STATE[d.state] or d.state) .. (d.open > 0 and (" · " .. d.open .. " open") or "")
      items[#items + 1] = {
        text = d.name, what = d.label, state = state, name = d.name, dropped = d.state == "dropped",
        boost = d.state == "up" and 0.5 or 0,
        preview = { title = d.name .. " — " .. d.label, lines = {
          d.means, "", "found in " .. d.from, d.name .. ":~ in a new tab on <CR>",
        } },
      }
    end
    return items
  end,
  keys = {
    ["<C-o>"] = function(item)
      if not item then return end
      picker.close()
      kawoosh.run((item.dropped and "domain reconnect " or "domain connect ") .. item.name)
    end,
    -- The picker stays: the row reads `not connected` once it is.
    ["<C-x>"] = function(item)
      if not item then return end
      kawoosh.run("domain disconnect " .. item.name)
    end,
  },
  pick = function(item) if item then kawoosh.run("domain tab " .. item.name) end end,
  empty = "no domain: none in the settings, no host in ~/.ssh/config, no WSL distro",
})

-- The keys keys.md keeps for these (keymap-regroup.md): the daily
-- finders one stroke each, their directory's twins in upper case.
kawoosh.map("n", "<leader>f", "picker files")
kawoosh.map("n", "<leader>g", "picker grep")
-- The same two from the file's directory (a listing's own in `dir`).
kawoosh.map("n", "<leader>F", "picker files here")
kawoosh.map("n", "<leader>G", "picker grep here")
-- And under `<leader>s` beside `sS`, the project search from there.
kawoosh.map("n", "<leader>sF", "picker files here")
kawoosh.map("n", "<leader>sG", "picker grep here")
kawoosh.map("n", "<leader>/", "picker lines")
kawoosh.map("n", "<leader>.", "picker smart")
kawoosh.map("n", "<leader><leader>", "picker buffers")
kawoosh.map("n", "<leader>so", "picker recent")
kawoosh.map("n", "<leader>sr", "picker resume")
kawoosh.map("n", "<leader>ww", "picker workspaces")
kawoosh.map("n", "<leader>wh", "domain pick")
kawoosh.map("n", "<leader>t", "picker tools")
