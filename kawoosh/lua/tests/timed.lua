-- Timed rows (timed.lua): `:timed` stamps the empty line and every line
-- `<CR>`, `o` and `O` open, the timed buffer's own keys; a buffer that is
-- not timed never finds them — pairs' `<CR>` still opens a block there;
-- `:timed relative` starts from a dated stamp and counts from it.
local function lines() return kawoosh.buf.lines() end
local CLOCK = "^%d%d:%d%d "

kawoosh.cmd("enew")
kawoosh.frame()
kawoosh.cmd("timed")
kawoosh.frame()
kawoosh.test.ok(lines()[1]:match(CLOCK .. "$"), "the empty line stamped: " .. lines()[1])
kawoosh.press("Afirst<CR>second<Esc>")
kawoosh.test.ok(lines()[1]:match(CLOCK .. "first$"), lines()[1])
kawoosh.test.ok(lines()[2]:match(CLOCK .. "second$"), "<CR> stamps: " .. lines()[2])
kawoosh.press("othird<Esc>")
kawoosh.test.ok(lines()[3]:match(CLOCK .. "third$"), "o stamps: " .. tostring(lines()[3]))
kawoosh.press("ggOzero<Esc>")
kawoosh.test.ok(lines()[1]:match(CLOCK .. "zero$"), "O stamps: " .. lines()[1])

-- Another buffer: the keys are not there — pairs' block, a plain `o`.
kawoosh.cmd("enew")
kawoosh.frame()
kawoosh.press("iif {<CR>x<Esc>")
local block = table.concat(lines(), "\n")
kawoosh.test.ok(block:match("^if {\n%s+x\n}$"), "pairs' <CR> under timed's: " .. block)
kawoosh.press("Goplain<Esc>")
kawoosh.test.eq(lines()[4], "plain", "a plain o")

-- Relative: a dated first stamp, the rest counted from it.
kawoosh.cmd("enew")
kawoosh.frame()
kawoosh.cmd("timed relative")
kawoosh.frame()
kawoosh.test.ok(lines()[1]:match("^%d%d%d%d%-%d%d%-%d%d %d%d:%d%d $"), "dated: " .. lines()[1])
local h = kawoosh.buf.current()
kawoosh.test.eq(kawoosh.timed.stamp(h, os.time() + 101 * 60), "T+01:41 ")
kawoosh.press("Astart<CR>next<Esc>")
kawoosh.test.ok(lines()[2]:match("^T%+00:0%d next$"), "counted: " .. lines()[2])
kawoosh.cmd("timed off")
kawoosh.frame()
kawoosh.press("oafter<Esc>")
kawoosh.test.eq(lines()[3], "after", "off")
