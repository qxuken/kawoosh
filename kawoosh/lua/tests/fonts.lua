-- The fonts (docs/design/fonts.md): `kawoosh.fonts` is data, `:font
-- NAME` takes a family for the session, the pane lists the families
-- each drawn in itself and takes the cursor's, and the lab sets its
-- samples in the face on show.
local fonts = kawoosh.fonts
local themes = kawoosh.themes
kawoosh.frame()

-- The families: none of the OS's `.` ones, each saying what it is.
local fams = fonts.families()
local mono = {}
for _, f in ipairs(fams) do
  kawoosh.test.ok(f.name:sub(1, 1) ~= ".", "a hidden face: " .. f.name)
  kawoosh.test.eq(type(f.mono), "boolean", f.name)
  kawoosh.test.eq(type(f.italic), "boolean", f.name)
  for i = 2, #f.weights do kawoosh.test.ok(f.weights[i - 1] < f.weights[i], "weights sorted: " .. f.name) end
  if f.mono then mono[#mono + 1] = f end
end

-- The face on show: headless, no face shipped, kui's mono at 13 px.
local cur = fonts.current()
kawoosh.test.eq(cur.family, "")
kawoosh.test.eq(cur.size, 13)
kawoosh.test.eq(cur.row, 20, "13 × 1.5")
kawoosh.test.ok(cur.cell > 0, "a cell measured")

-- A machine with fewer than two monospaced families has nothing to pick
-- between: the rest needs them.
if #mono < 2 then return end
local a, b = mono[1], mono[2]

-- `:font NAME`, the rest of the line; a stranger says so.
kawoosh.cmd("font " .. a.name)
kawoosh.frame(2)
kawoosh.test.eq(kawoosh.opt("font.family"), a.name)
cur = fonts.current()
kawoosh.test.eq(cur.name, a.name)
kawoosh.test.ok(cur.font, "a handle for a view")
kawoosh.test.eq(cur.mono, true)
kawoosh.cmd("font")
kawoosh.frame()
kawoosh.test.ok(kawoosh.message():find("font " .. a.name, 1, true), kawoosh.message())
kawoosh.cmd("font No Such Face 9000")
kawoosh.frame()
kawoosh.test.ok(kawoosh.message():find('no family "No Such Face 9000"', 1, true), kawoosh.message())
kawoosh.test.eq(kawoosh.opt("font.family"), a.name, "the face stays")

-- A family's handle: asked, registered at the next frame.
kawoosh.test.eq(fonts.face(b.name), nil, "not yet")
kawoosh.frame()
kawoosh.test.eq(type(fonts.face(b.name)), "number", "the frame registered it")
kawoosh.test.eq(fonts.face("No Such Face 9000"), nil)

-- The pane: the monospaced families, the cursor on the face on show.
kawoosh.press("<leader>of")
kawoosh.frame(2)
local st = assert(fonts.state(), "the pane is open")
kawoosh.test.eq(st.cursor, a.name)
kawoosh.test.eq(st.all, false)
kawoosh.test.eq(#st.list, #mono, "mono only")
-- To the first and one on, which `⏎` takes.
kawoosh.press("gg")
kawoosh.press("j")
kawoosh.frame()
kawoosh.test.eq(fonts.state().cursor, st.list[2])
kawoosh.press("<CR>")
kawoosh.frame(2)
kawoosh.test.eq(kawoosh.opt("font.family"), st.list[2])
kawoosh.test.eq(fonts.current().name, st.list[2])
-- Walking far keeps the cursor's card in view.
kawoosh.press("G")
kawoosh.frame(3)
kawoosh.test.eq(fonts.state().cursor, st.list[#st.list])
if #st.list > 8 then kawoosh.test.ok(fonts.state().scrolled > 0, "scrolled to the last card") end
kawoosh.press("gg")
kawoosh.frame(3)
kawoosh.test.eq(fonts.state().scrolled, 0, "and back to the first")
-- `m`: every family; again, the monospaced.
kawoosh.press("m")
kawoosh.frame()
kawoosh.test.eq(fonts.state().all, true)
kawoosh.test.eq(#fonts.state().list, #fams)
kawoosh.press("m")
kawoosh.frame()
-- The filter: any part of the name, case aside.
local part = a.name:sub(2, 4):upper()
kawoosh.press("/")
kawoosh.press(part)
kawoosh.frame(2)
st = fonts.state()
kawoosh.test.eq(st.query, part)
kawoosh.test.ok(#st.list >= 1, "the part matches its own")
for _, n in ipairs(st.list) do kawoosh.test.ok(n:lower():find(part:lower(), 1, true), n) end
-- Normal mode over the filter: `j` `k` walk what it left.
kawoosh.press("<Esc>")
kawoosh.frame()
if #st.list > 1 then
  local was = fonts.state().cursor
  kawoosh.press("gg")
  kawoosh.press("j")
  kawoosh.frame()
  kawoosh.test.eq(fonts.state().cursor, st.list[2], "j walks the filtered list")
  kawoosh.test.eq(fonts.state().query, part, "and leaves the filter's text")
  kawoosh.press("k")
  kawoosh.frame()
  kawoosh.test.eq(fonts.state().cursor, st.list[1])
end
kawoosh.press("<Esc>")
kawoosh.frame()
-- `+`: the size; `y` copies the line that keeps the pick.
kawoosh.press("+")
kawoosh.frame(2)
kawoosh.test.eq(fonts.current().size, 14)
kawoosh.press("y")
kawoosh.frame()
kawoosh.test.eq(kawoosh.memory()[1].text, fonts.state().keep)
kawoosh.test.ok(fonts.state().keep:find("size = 14", 1, true), fonts.state().keep)
kawoosh.press("q")
kawoosh.frame()
kawoosh.test.eq(fonts.state(), nil)

-- The lab sets its samples in the face on show, and follows a pick.
kawoosh.cmd("font lab")
kawoosh.frame(2)
local lab = assert(themes.lab(), "the lab is open")
kawoosh.test.eq(lab.face, fonts.current().name)
kawoosh.test.eq(lab.size, 14)
kawoosh.cmd("font " .. b.name)
kawoosh.frame(2)
kawoosh.test.eq(themes.lab().face, b.name)
kawoosh.press("q")
kawoosh.frame()

kawoosh.cmd("font reset")
kawoosh.cmd("set font.family!")
kawoosh.frame()
kawoosh.test.eq(fonts.current().family, "")
