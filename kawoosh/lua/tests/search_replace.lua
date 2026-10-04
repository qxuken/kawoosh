-- The project search's replace (search.lua, docs/design/search.md
-- Decision 12): the bar's `replace` field, `<A-CR>` replacing every
-- match the results show as one change, `<A-CR>` in the results the one
-- at the caret, and `u` there taking it back from the files.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-replace"
local function write(p, s) kawoosh.fs.write(kawoosh.fs.join(dir, p), s) end
kawoosh.fs.create(kawoosh.fs.join(dir, ".git"), true)
write("a.txt", "old_name = 1\nkeep old_name\nprint(old_name)\n")
write("b.txt", "old_name()\n")
write("c.txt", "nothing here\n")
local sep = kawoosh.fs.join("a", "b"):sub(2, 2)
local function native(p) return (p:gsub("/", sep)) end

kawoosh.cmd("cd " .. dir)
kawoosh.cmd("e c.txt")
kawoosh.frame()

local S = kawoosh.search_ui
local function settled(what)
  kawoosh.wait(function() return not S.state().running end, 400, what)
  kawoosh.frame(2)
end
-- A file's buffer by its name: listed once a replace edited it.
local function file(name)
  for _, b in ipairs(kawoosh.buf.list()) do
    local p = kawoosh.buf.path(b) or ""
    if p:sub(-#name) == native(name) then return b end
  end
end
local function results()
  for _, h in ipairs(kawoosh.buf.list()) do
    if kawoosh.buf.name(h) == "*search*" then return h end
  end
end

-- Nothing searched: nothing to replace, and it says so.
kawoosh.cmd("search project")
kawoosh.frame(2)
kawoosh.cmd("search replace all")
kawoosh.frame()
kawoosh.test.ok(kawoosh.message():find("nothing to replace", 1, true), kawoosh.message())

kawoosh.field_set("search", "find", "old_name")
kawoosh.frame()
kawoosh.cmd("search run")
settled("the search")
kawoosh.test.eq(S.state().stages[1].matches, 4)
kawoosh.test.eq(S.state().replaced, false)

-- `<Tab>` from find is the replace field: it is the search's, not a
-- stage's, so a stage added keeps it.
kawoosh.press("<Tab>")
kawoosh.frame()
kawoosh.test.eq((kawoosh._field("lua:search/replace") or {}).focused, true, "<Tab> to replace")
kawoosh.press("new_name")
kawoosh.frame()
kawoosh.test.eq(kawoosh.field_text("search", "replace"), "new_name")

-- `<A-CR>` in the bar: every match, the results' headers left.
kawoosh.press("<A-CR>")
kawoosh.frame(2)
local a, b = file("a.txt"), file("b.txt")
assert(a and b, "the files a replace edited are listed buffers")
kawoosh.test.eq(kawoosh.buf.text(a):gsub("\r", ""), "new_name = 1\nkeep new_name\nprint(new_name)\n")
kawoosh.test.eq(kawoosh.buf.text(b):gsub("\r", ""), "new_name()\n")
kawoosh.test.ok(kawoosh.message():find("4 matches replaced in 2 files", 1, true), kawoosh.message())
kawoosh.test.eq(S.state().replaced, true, "the count stale")
kawoosh.test.ok(kawoosh.buf.text(results()):find(native("a.txt") .. "  3", 1, true), "the header as it was")
-- Not on disk until `:w`; one `u` in the results takes both back.
local disk = io.open(kawoosh.fs.join(dir, "b.txt")):read("a")
kawoosh.test.eq(disk:gsub("\r", ""), "old_name()\n", "not written")
kawoosh.cmd("search results")
kawoosh.frame()
kawoosh.press("u")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.text(a):gsub("\r", ""), "old_name = 1\nkeep old_name\nprint(old_name)\n")
kawoosh.test.eq(kawoosh.buf.text(b):gsub("\r", ""), "old_name()\n")

-- A regex: the groups in the replacement.
kawoosh.cmd("search query")
kawoosh.frame()
kawoosh.field_set("search", "find", [[(\w+)_name\(\)]])
kawoosh.field_set("search", "replace", "${1}_fn()")
kawoosh.frame()
kawoosh.cmd("search regex")
kawoosh.cmd("search run")
settled("the regex search")
kawoosh.test.eq(S.state().stages[1].matches, 1)
kawoosh.cmd("search replace all")
kawoosh.frame(2)
kawoosh.test.eq(kawoosh.buf.text(b):gsub("\r", ""), "old_fn()\n")
kawoosh.cmd("search results")
kawoosh.frame()
kawoosh.press("u")
kawoosh.frame()
kawoosh.cmd("search regex")

-- A `drop` stage: the line it left out is shown as another's context,
-- painted, and still not replaced — the matches are the answer's.
kawoosh.cmd("search query")
kawoosh.frame()
kawoosh.field_set("search", "find", "old_name")
kawoosh.field_set("search", "replace", "X")
kawoosh.frame()
kawoosh.cmd("search run")
settled("back to the plain search")
kawoosh.cmd("search stage add")
kawoosh.cmd("search stage kind")
kawoosh.cmd("search stage kind")
kawoosh.frame()
kawoosh.field_set("search", "find", "keep")
kawoosh.frame()
kawoosh.cmd("search run")
settled("the drop stage")
kawoosh.test.eq(S.state().stages[2].kind, "drop")
kawoosh.test.ok(kawoosh.buf.text(results()):find("keep old_name", 1, true), "shown as context")
kawoosh.cmd("search replace all")
kawoosh.frame(2)
kawoosh.test.eq(kawoosh.buf.text(a):gsub("\r", ""), "X = 1\nkeep old_name\nprint(X)\n")
kawoosh.cmd("search results")
kawoosh.frame()
kawoosh.press("u")
kawoosh.frame()
kawoosh.search_ui.remove(2)
settled("one stage again")

-- In the results, `<A-CR>` the match at the caret, then on to the next.
kawoosh.cmd("search results")
kawoosh.frame()
kawoosh.press("gg")
kawoosh.press("j")
kawoosh.frame()
kawoosh.press("<A-CR>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.text(a):gsub("\r", ""), "X = 1\nkeep old_name\nprint(old_name)\n", "the first alone")
kawoosh.test.ok(kawoosh.message():find("3 matches left", 1, true), kawoosh.message())
kawoosh.press("<A-CR>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.text(a):gsub("\r", ""), "X = 1\nkeep X\nprint(old_name)\n", "then the next")
-- Each its own change.
kawoosh.press("u")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.text(a):gsub("\r", ""), "X = 1\nkeep old_name\nprint(old_name)\n")
kawoosh.press("u")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.text(a):gsub("\r", ""), "old_name = 1\nkeep old_name\nprint(old_name)\n")
kawoosh.test.eq(S.state().replaced, true, "the count stale")

-- A replace that finds nothing leaves the count as the search said it.
kawoosh.cmd("search run")
settled("searched again")
kawoosh.test.eq(S.state().replaced, false, "a run makes it fresh")
kawoosh.cmd("search results")
kawoosh.frame()
kawoosh.cmd("%s/old_name/zz/g")
kawoosh.frame()
kawoosh.cmd("search replace all")
kawoosh.frame(2)
kawoosh.test.ok(kawoosh.message():find("no match in the results", 1, true), kawoosh.message())
kawoosh.test.eq(S.state().replaced, false, "nothing replaced: not stale")
kawoosh.press("u")
kawoosh.frame()

kawoosh.cmd("search close")
kawoosh.frame()
pcall(kawoosh.fs.remove, dir)
