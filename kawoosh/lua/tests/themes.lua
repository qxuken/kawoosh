-- The themes (docs/design/themes.md): `kawoosh.themes` is data, the
-- `:theme` commands set the session's `theme.*`, and the pane walks the
-- cards and puts the cursor's in its half.
local themes = kawoosh.themes
local names = {}
for _, v in ipairs(themes.variants) do names[#names + 1] = v.name end
kawoosh.test.eq(table.concat(names, " "),
  "rose-pine rose-pine-moon ayu-dark ayu-mirage high-contrast-dark rose-pine-dawn ayu-light high-contrast-light")
local mirage = themes.variants[4]
kawoosh.test.eq(mirage.title, "Ayu Mirage")
kawoosh.test.eq(mirage.dark, true)
kawoosh.test.eq(mirage.roles.bg, 0x242936ff, "a role as 0xRRGGBBAA")
kawoosh.test.eq(mirage.syntax.keyword, 0xffad66ff)
kawoosh.test.eq(mirage.syntax.plain, nil, "plain text has no hue")
kawoosh.test.eq(#mirage.ansi, 16)
kawoosh.test.eq(themes.families[1].name, "rose-pine")
-- Styles beside the hues: comments italic everywhere, keywords bold in
-- the high-contrast pair alone.
kawoosh.test.eq(mirage.styles.comment.italic, true)
kawoosh.test.eq(mirage.styles.keyword, nil)
kawoosh.test.eq(themes.variants[5].styles.keyword.bold, true)

-- A highlighted text's runs carry the style, and `tokens.styles` turns
-- it off.
local function highlight(text)
  local got
  kawoosh.highlight(text, { language = "rust" }, function(runs) got = runs end)
  kawoosh.wait(function() return got end, nil, "the highlight")
  for _, r in ipairs(got) do
    if r.token == "comment" then return r end
  end
end
kawoosh.test.eq(highlight("// hi\nfn main() {}\n").italic, true)
kawoosh.opt("tokens.styles", { comment = { italic = false } })
kawoosh.frame()
kawoosh.test.eq(highlight("// hi\nfn main() {}\n").italic, nil)
kawoosh.opt("tokens.styles", nil)
kawoosh.frame()

-- What is on show: the default family, the base the headless OS's.
kawoosh.frame()
local cur = themes.current()
kawoosh.test.eq(cur.family, "rose-pine")
kawoosh.test.eq(cur.dark, "rose-pine")
kawoosh.test.eq(cur.light, "rose-pine-dawn")
kawoosh.test.eq(cur.appearance, "system")
local base = cur.base

-- The halves apart; a light theme is not a dark one.
kawoosh.cmd("theme dark ayu-mirage")
kawoosh.frame()
kawoosh.test.eq(kawoosh.opt("theme.dark"), "ayu-mirage")
kawoosh.test.eq(themes.current().dark, "ayu-mirage")
kawoosh.cmd("theme dark ayu-light")
kawoosh.frame()
kawoosh.test.ok(kawoosh.message():find('no dark theme "ayu-light"', 1, true), kawoosh.message())
kawoosh.test.eq(themes.current().dark, "ayu-mirage")

-- The toggle pins the other base, `system` follows the OS again.
kawoosh.cmd("theme toggle")
kawoosh.frame()
local other = base == "dark" and "light" or "dark"
kawoosh.test.eq(themes.current().base, other)
kawoosh.test.eq(kawoosh.opt("theme.appearance"), other)
kawoosh.cmd("theme system")
kawoosh.frame()
kawoosh.test.eq(themes.current().base, base)

-- A family whole, over what the halves said; `reset` back to the files.
kawoosh.cmd("theme high-contrast")
kawoosh.frame()
cur = themes.current()
kawoosh.test.eq(cur.family, "high-contrast")
kawoosh.test.eq(cur.dark, "high-contrast-dark")
kawoosh.test.eq(cur.light, "high-contrast-light")
kawoosh.cmd("theme")
kawoosh.frame()
kawoosh.test.ok(kawoosh.message():find("light high-contrast-light", 1, true), kawoosh.message())
kawoosh.cmd("theme reset")
kawoosh.frame()
kawoosh.test.eq(themes.current().dark, "rose-pine")

-- The pane: the cursor starts on what is shown, walks the grid, and
-- `<CR>` puts its card in its half.
kawoosh.press("<leader>oo")
kawoosh.frame()
local st = themes.state()
kawoosh.test.ok(st, "the pane is open")
kawoosh.test.eq(st.cursor, base == "dark" and "rose-pine" or "rose-pine-dawn")
kawoosh.test.ok(#st.grid >= 2, "a row of dark cards and one of light")
kawoosh.test.eq(st.grid[1][1], "rose-pine")
-- To the first card, then one on: the moon, a dark one.
kawoosh.press("gg")
for _ = 1, 8 do kawoosh.press("h") end
kawoosh.press("l")
kawoosh.frame()
kawoosh.test.eq(themes.state().cursor, "rose-pine-moon")
kawoosh.press("<CR>")
kawoosh.frame()
kawoosh.test.eq(themes.current().dark, "rose-pine-moon")
kawoosh.test.eq(kawoosh.opt("theme.dark"), "rose-pine-moon")
-- Down to the light row, the column held where it can be.
for _ = 1, 4 do kawoosh.press("j") end
kawoosh.frame()
local c = themes.state().cursor
kawoosh.test.ok(c == "rose-pine-dawn" or c == "ayu-light" or c == "high-contrast-light", c)
kawoosh.press("<CR>")
kawoosh.frame()
kawoosh.test.eq(themes.current().light, c)
-- The walk keeps the cursor's card in view: to the last and back up.
for _ = 1, 8 do kawoosh.press("j") end
kawoosh.frame(3)
kawoosh.test.eq(themes.state().cursor, "high-contrast-light")
kawoosh.test.ok(themes.state().scrolled > 0, "scrolled down to the last card")
for _ = 1, 8 do kawoosh.press("k") end
kawoosh.frame(3)
kawoosh.test.eq(themes.state().cursor, "rose-pine")
kawoosh.test.eq(themes.state().scrolled, 0, "and back to the first")
kawoosh.test.eq(themes.state().keep,
  string.format('theme = { dark = "rose-pine-moon", light = "%s" }', c))
-- The check (themes.md Decision 7): the look on show whole, and a
-- variant as it ships; high contrast clears every floor.
local now = themes.check()
kawoosh.test.ok(now.title:find("as shown", 1, true), now.title)
kawoosh.test.ok(#now.checks > 50, "every pair")
kawoosh.test.ok(now.hit & 0xff < 0xff, "a hit is a wash")
local hc = themes.check("high-contrast-light")
for _, ch in ipairs(hc.checks) do kawoosh.test.ok(ch.ok, ch.what) end
kawoosh.test.eq(hc.syntax.keyword, 0xa3006bff)
kawoosh.test.eq(hc.styles.keyword.bold, true)
kawoosh.test.ok(not pcall(themes.check, "nope"), "a stranger is an error")
-- `y` copies the line; `q` closes.
kawoosh.press("y")
kawoosh.frame()
kawoosh.test.eq(kawoosh.memory()[1].text, themes.state().keep)
kawoosh.press("q")
kawoosh.frame()
kawoosh.test.eq(themes.state(), nil)
kawoosh.cmd("theme reset")
kawoosh.frame()

-- The report: `*theme check*`, a head and the pairs; a variant's
-- shortfall on the line.
kawoosh.cmd("theme check high-contrast-dark")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "*theme check*")
kawoosh.test.ok(kawoosh.buf.line(1):find("theme check · high-contrast-dark", 1, true), kawoosh.buf.line(1))
kawoosh.test.ok(kawoosh.message():find("high-contrast-dark 0", 1, true), kawoosh.message())
kawoosh.cmd("theme check all")
kawoosh.frame()
kawoosh.test.ok(kawoosh.message():find("ayu-light", 1, true), kawoosh.message())

-- The lab: the look on show measured, `f` narrowing to what falls
-- short, following a pick.
kawoosh.press("<leader>ol")
kawoosh.frame(2)
local lab = themes.lab()
kawoosh.test.ok(lab, "the lab is open")
local short = 0
for _, ch in ipairs(themes.check().checks) do if not ch.ok then short = short + 1 end end
kawoosh.test.eq(lab.short, short)
kawoosh.press("f")
kawoosh.frame()
kawoosh.test.eq(themes.lab().only_short, true)
kawoosh.cmd("theme high-contrast")
kawoosh.frame(2)
kawoosh.test.eq(themes.lab().short, 0, "high contrast, measured again")
kawoosh.press("q")
kawoosh.frame()
kawoosh.test.eq(themes.lab(), nil)
kawoosh.cmd("theme reset")
kawoosh.frame()
