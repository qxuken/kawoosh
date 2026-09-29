-- The settings pane (docs/design/settings.md): every setting a row in
-- a section, the search keeping the rows whose path, doc, section or
-- value hold every word, `@` a filter, the keys walking rows and
-- sections; a change written into the project's file, the user's
-- refused where no config was loaded, as in this harness.
local door = kawoosh.settings
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-settings"
kawoosh.fs.create(dir, true)
kawoosh.fs.chdir(dir)
kawoosh.frame()

kawoosh.cmd("settings")
kawoosh.frame(2)
local st = assert(door.state(), "the pane is open")
kawoosh.test.eq(#st.shown, st.total, "no search: every row")
kawoosh.test.eq(st.sections[1], "Editing")
kawoosh.test.eq(st.shown[1], "tabstop", "the section's own order")
kawoosh.test.eq(st.scope, "user")
-- A plugin's setting is in a section the list names.
local has = {}
for _, s in ipairs(st.sections) do has[s] = true end
kawoosh.test.ok(has["Files & tools"], "dir.hidden's section")

-- Typing searches: every word, the path's matches first.
kawoosh.press("wrap")
kawoosh.frame()
st = door.state()
kawoosh.test.eq(st.query, "wrap")
kawoosh.test.has(st.shown, "editor.wrap")
kawoosh.test.has(st.shown, "picker.wrap")
for _, p in ipairs(st.shown) do kawoosh.test.ok(p ~= "tabstop", "tabstop has no wrap") end
-- By its letters in order, and by its doc.
kawoosh.press("<C-u>fsz")
kawoosh.frame()
kawoosh.test.eq(door.state().shown[1], "font.size")
kawoosh.press("<C-u>quotes as you type")
kawoosh.frame()
kawoosh.test.eq(door.state().shown[1], "pairs.enabled")
-- A filter by kind.
kawoosh.press("<C-u>@bool editor")
kawoosh.frame()
for _, p in ipairs(door.state().shown) do
  kawoosh.test.ok(p == "editor.bell" or p == "editor.breadcrumbs" or p == "editorconfig.enabled", p)
end

-- The rows: the cursor kept on its row as the search empties; `gg`
-- the first, `j` `k` walk, `]]` the next section.
kawoosh.press("<C-u><Esc>")
kawoosh.frame()
kawoosh.test.eq(door.state().cursor, "editor.bell", "kept")
kawoosh.press("gg")
kawoosh.frame()
kawoosh.test.eq(door.state().cursor, "tabstop")
kawoosh.press("j")
kawoosh.test.eq(door.state().cursor, "expandtab")
kawoosh.press("]]")
kawoosh.frame()
st = door.state()
local look
for i, p in ipairs(st.shown) do if p == st.cursor then look = i end end
kawoosh.test.ok(look and look > 2, "past the editing rows")
kawoosh.test.eq(st.cursor:sub(1, 5), "font.", "Look begins with the font")
kawoosh.press("j")
kawoosh.press("[[")
kawoosh.test.eq(door.state().cursor, "font.family", "this section's first row")
kawoosh.press("[[")
kawoosh.test.eq(door.state().cursor, "tabstop", "the one before")

-- The user's file: none loaded here, so the change is refused.
kawoosh.press("l")
kawoosh.frame(2)
kawoosh.test.ok(kawoosh.message():find("no config dir", 1, true), kawoosh.message())
-- The project's: made in the working directory.
kawoosh.press("p")
kawoosh.press("l")
kawoosh.frame(2)
local file = kawoosh.fs.join(dir, ".kawoosh", "settings.lua")
kawoosh.test.ok(kawoosh.fs.read(file):find("tabstop = 5", 1, true), kawoosh.fs.read(file))
kawoosh.test.eq(kawoosh.opt("tabstop"), 5)
-- `y` copies the line that sets it.
kawoosh.press("y")
kawoosh.test.eq(kawoosh.message(), "copied tabstop = 5")
-- `<Tab>` shows its layers.
kawoosh.press("<Tab>")
kawoosh.frame()
kawoosh.test.ok(door.state().open.tabstop, "open")
local ls = door.layers("tabstop")
kawoosh.test.eq(ls[1].layer, "project")
kawoosh.test.eq(ls[1].line, 4, "the template's fourth line")
kawoosh.press("r")
kawoosh.frame(2)
kawoosh.test.eq(kawoosh.opt("tabstop"), 4, "the default again")

kawoosh.press("q")
kawoosh.frame()
kawoosh.test.eq(door.state(), nil, "closed")
kawoosh.fs.remove(dir)
