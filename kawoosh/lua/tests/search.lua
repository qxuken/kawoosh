-- The project search (search.lua, docs/design/search.md) on the
-- harness: the bar, the comma lists, the pipeline, and the results as
-- a live multibuffer — an edit there is in the file, `:w` writes it.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-search"
local function write(p, s) kawoosh.fs.write(dir .. "/" .. p, s) end
kawoosh.fs.create(dir .. "/.git", true)
write("src/a.ts", "const needle = 1\nfoo\nbar\n")
write("src/b.tsx", "needle()\n")
write("src/c.js", "needle\n")
write("src/a__test__.ts", "needle\n")
write("tests/t.ts", "needle and more\n")
write("target/x.ts", "needle\n")
write(".gitignore", "target/\n")

kawoosh.cmd("cd " .. dir)
kawoosh.cmd("e src/c.js")
kawoosh.frame()

local S = kawoosh.search_ui
local function state() return S.state() end
local function settled(what)
  kawoosh.wait(function() return not state().running end, 400, what)
  kawoosh.frame(2)
end
local function contains(text, s, what)
  kawoosh.test.ok(text:find(s, 1, true), (what or "contains") .. ": " .. s .. " not in " .. text)
end
local function results()
  for _, h in ipairs(kawoosh.buf.list()) do
    if kawoosh.buf.name(h) == "*search*" then return h end
  end
end

-- `:search project PATTERN`: the bar, and the search at once.
kawoosh.cmd("search project needle")
settled("the first search")
local st = state()
kawoosh.test.eq(st.stages[1].find, "needle")
kawoosh.test.eq(st.stages[1].files, 5, "every file git sees: " .. tostring(st.stages[1].err))
local h = assert(results(), "the results are a buffer")
kawoosh.test.eq(kawoosh.buf.language(h), "multibuffer")
local text = kawoosh.buf.text(h)
contains(text, "src/b.tsx  1", "a file's header with its count")
contains(text, "needle()", "its matched line")

-- The comma lists: brackets as a list, an exclude with no `/` at any depth.
kawoosh.field_set("search", "include", "src/*.[ts,tsx], tests/*.ts")
kawoosh.field_set("search", "exclude", "*__test__*")
kawoosh.frame()
kawoosh.cmd("search run")
settled("the narrowed search")
st = state()
kawoosh.test.eq(st.stages[1].files, 3, "a.ts, b.tsx, t.ts")
text = kawoosh.buf.text(results())
kawoosh.test.eq(text:find("c.js", 1, true), nil, "no .js")
kawoosh.test.eq(text:find("__test__", 1, true), nil, "no test file")

-- A glob that does not parse is the field's error; nothing runs.
kawoosh.field_set("search", "include", "src/{a")
kawoosh.frame()
kawoosh.cmd("search run")
kawoosh.frame(2)
contains(state().stages[1].err or "", "include", "the include field's error")
kawoosh.field_set("search", "include", "src/*.[ts,tsx], tests/*.ts")
kawoosh.frame()
kawoosh.cmd("search run")
settled("back again")

-- The pipeline: a `drop` stage after it takes the matched lines that
-- also say `more` out.
kawoosh.cmd("search stage add")
kawoosh.frame()
kawoosh.test.eq(state().cur, 2)
kawoosh.test.eq(state().stages[2].kind, "in")
kawoosh.cmd("search stage kind")
kawoosh.cmd("search stage kind")
kawoosh.frame()
kawoosh.test.eq(state().stages[2].kind, "drop")
kawoosh.field_set("search", "find", "more")
kawoosh.frame()
kawoosh.cmd("search run")
settled("the drop stage")
st = state()
kawoosh.test.eq(st.stages[1].files, 3, "the first stage kept its answer")
kawoosh.test.eq(st.stages[2].files, 2, "t.ts dropped")
text = kawoosh.buf.text(results())
kawoosh.test.eq(text:find("tests/t.ts", 1, true), nil, "dropped from the results")
-- `in`: search in what the stage before found.
kawoosh.cmd("search stage kind")
kawoosh.frame()
kawoosh.test.eq(state().stages[2].kind, "in", "round from the last kind to the first")
kawoosh.field_set("search", "find", "const")
kawoosh.frame()
kawoosh.cmd("search run")
settled("the in stage")
kawoosh.test.eq(state().stages[2].files, 1, "only a.ts says const")

-- The results are live: into them, the match's line edited.
kawoosh.cmd("search results")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.current(), results(), "the keyboard on the results")
kawoosh.press(":%s/const/let/g<CR>")
kawoosh.frame()
local a
for _, b in ipairs({ table.unpack(kawoosh.buf.list()) }) do
  local p = kawoosh.buf.path(b) or ""
  if p:sub(-#"src/a.ts") == "src/a.ts" then a = b end
end
-- Borrowed until edited: now listed, its text the edit.
assert(a, "a.ts is a listed buffer once edited")
kawoosh.test.eq(kawoosh.buf.line(1, a), "let needle = 1", "the file's buffer has the edit")
kawoosh.press(":w<CR>")
kawoosh.frame()
local disk = io.open(dir .. "/src/a.ts"):read("a")
kawoosh.test.eq(disk, "let needle = 1\nfoo\nbar\n", "written through the results")
-- `u` in the results: the file steps back.
kawoosh.press("u")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.line(1, a), "const needle = 1", "undone in the file")
-- `<CR>` on an excerpt's line: the file, at that line.
kawoosh.press("gg")
kawoosh.press("j")
kawoosh.press("<CR>")
kawoosh.frame()
local path = kawoosh.buf.path() or ""
kawoosh.test.eq(path:sub(-#"src/a.ts"), "src/a.ts", "the file opened: " .. path)
kawoosh.fs.remove(dir)
