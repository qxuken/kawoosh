-- A buffer's base and its hunks through the doors (docs/design/vcs.md
-- Decisions 1, 3 and 10): `kawoosh.buf.base` given, `kawoosh.buf.hunks`
-- read back once the diff answered, `]h` walking them, a reset putting
-- the base's lines back, the base taken away.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-hunks"
kawoosh.fs.create(dir, true)
local file = kawoosh.fs.join(dir, "a.txt")
kawoosh.fs.write(file, "one\ntwo!\nthree\nfour\nfive\n")
kawoosh.cmd("e " .. file)
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.hunks(), nil, "no base yet")
kawoosh.test.eq(kawoosh.buf.base_label(), nil)

kawoosh.buf.base("one\ntwo\nthree\ngone\nfour\n", "index")
kawoosh.frame(2)
kawoosh.test.eq(kawoosh.buf.base_label(), "index")
local hs = kawoosh.buf.hunks()
kawoosh.test.eq(#hs, 3, "three hunks")
kawoosh.test.eq(hs[1].kind, "modified")
kawoosh.test.eq(hs[1].line, 2)
kawoosh.test.eq(hs[1].end_line, 3)
kawoosh.test.eq(hs[1].old[1], "two")
kawoosh.test.eq(hs[2].kind, "deleted")
kawoosh.test.eq(hs[2].line, 4, "the line after what was taken out")
kawoosh.test.eq(hs[2].old[1], "gone")
kawoosh.test.eq(hs[3].kind, "added")
kawoosh.test.eq(hs[3].line, 5)
kawoosh.test.eq(#hs[3].old, 0)

-- `]h` `[h` walk the hunks' lines.
kawoosh.press("gg")
kawoosh.press("]h")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.cursor().line, 2)
kawoosh.press("2]h")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.cursor().line, 5)
kawoosh.press("[h")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.cursor().line, 4)

-- A reset puts the base's line back, undoable; the diff follows.
kawoosh.cmd("hunk reset")
kawoosh.frame(2)
kawoosh.test.eq(kawoosh.buf.text(), "one\ntwo!\nthree\ngone\nfour\nfive\n")
kawoosh.test.eq(#kawoosh.buf.hunks(), 2, "the deletion is gone")
kawoosh.press("u")
kawoosh.frame(2)
kawoosh.test.eq(kawoosh.buf.text(), "one\ntwo!\nthree\nfour\nfive\n")
kawoosh.test.eq(#kawoosh.buf.hunks(), 3)

-- `hunk reset!` is the whole buffer.
kawoosh.cmd("hunk reset!")
kawoosh.frame(2)
kawoosh.test.eq(kawoosh.buf.text(), "one\ntwo\nthree\ngone\nfour\n")
kawoosh.test.eq(#kawoosh.buf.hunks(), 0)

-- An edit is diffed again once made.
kawoosh.press("Gonew line<Esc>")
kawoosh.frame(2)
kawoosh.test.eq(#kawoosh.buf.hunks(), 1)
kawoosh.test.eq(kawoosh.buf.hunks()[1].kind, "added")

-- The base taken away: nothing to walk.
kawoosh.buf.base(nil)
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.hunks(), nil)
kawoosh.press("gg]h")
kawoosh.frame()
kawoosh.test.eq(kawoosh.message(), "no base to diff against")
