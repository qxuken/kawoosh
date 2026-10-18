-- The project search over ignored paths (docs/design/search.md
-- Decision 14): `<A-g>` searches every file but `.git`; an include that
-- names an ignored path from the root searches it with the toggle off,
-- however the `.gitignore` spells it; an exclude still wins; the walk's
-- cap is said in the bar's count.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-search-ignored"
local function write(p, s) kawoosh.fs.write(kawoosh.fs.join(dir, p), s) end
kawoosh.fs.create(kawoosh.fs.join(dir, ".git"), true)
write("src/a.ts", "needle\n")
write("src/x.log", "needle\n")
write("node_modules/foo/index.js", "needle\n")
write("node_modules/foo/lib/deep.js", "needle\n")
write("node_modules/bar/index.js", "needle\n")
write("target/debug/out.txt", "needle\n")
write(".gitignore", "node_modules/**\ntarget/\n*.log\n")
local sep = kawoosh.fs.join("a", "b"):sub(2, 2)
local function native(p) return (p:gsub("/", sep)) end

kawoosh.cmd("cd " .. dir)
kawoosh.cmd("e src/a.ts")
kawoosh.frame()

local S = kawoosh.search_ui
local function state() return S.state() end
local function settled(what)
  kawoosh.wait(function() return not state().running end, 400, what)
  kawoosh.frame(2)
end
local function results()
  for _, h in ipairs(kawoosh.buf.list()) do
    if kawoosh.buf.name(h) == "*search*" then return kawoosh.buf.text(h) end
  end
  return ""
end
local function run(include, exclude, what)
  kawoosh.field_set("search", "include", include)
  kawoosh.field_set("search", "exclude", exclude)
  kawoosh.frame()
  kawoosh.cmd("search run")
  settled(what)
  return state().stages[1]
end

-- As git sees it: the source only.
kawoosh.cmd("search project needle")
settled("the first search")
kawoosh.test.eq(state().stages[1].files, 1, "src/a.ts alone: " .. results())

-- An include naming an ignored package: it, and nothing else.
local st = run("node_modules/foo/**", "", "a named package")
kawoosh.test.eq(st.files, 2, results())
kawoosh.test.ok(results():find(native("node_modules/foo/lib/deep.js"), 1, true), results())
kawoosh.test.ok(not results():find("bar", 1, true), results())

-- A bare name and a path beside it: the packages, and src as git sees it.
st = run("node_modules, src", "", "a bare name")
kawoosh.test.eq(st.files, 4, results())
kawoosh.test.ok(not results():find("x.log", 1, true), "src's ignored log stays out: " .. results())

-- An exclude wins over the include that named it.
st = run("node_modules", "node_modules/bar", "an exclude")
kawoosh.test.eq(st.files, 2, results())

-- A glob that starts with one names nothing.
st = run("**/node_modules/**", "", "no named path")
kawoosh.test.eq(st.files, 0, results())

-- `<A-g>`: everything but `.git`, the include fields empty.
st = run("", "", "back as git sees it")
kawoosh.test.eq(st.files, 1)
kawoosh.cmd("search ignored")
kawoosh.cmd("search run")
settled("ignored too")
st = state().stages[1]
kawoosh.test.eq(st.ignored, true)
kawoosh.test.eq(st.files, 6, results())
kawoosh.test.ok(not state().status:find("stopped", 1, true), state().status)

-- The walk's cap, said in the count.
S.max_walk = 3
kawoosh.cmd("search run")
settled("capped")
st = state().stages[1]
kawoosh.test.eq(st.capped, true)
kawoosh.test.ok(st.files <= 3, tostring(st.files))
kawoosh.test.ok(state().status:find("(stopped: the walk's first 3 files)", 1, true), state().status)
S.max_walk = nil
kawoosh.cmd("search ignored")
kawoosh.cmd("search close")
kawoosh.frame()
pcall(kawoosh.fs.remove, dir)
