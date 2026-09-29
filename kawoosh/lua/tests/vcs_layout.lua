-- `kawoosh.diff` and the review's layout (docs/design/vcs.md Decision
-- 6): the hunks of two texts at once, and the parts a review is made
-- of — a header, the base's lines taken out as a `deleted` gap before
-- the new lines, context around, runs that meet made one.
local hs = kawoosh.diff("one\ntwo\nthree\ngone\nfour\n", "one\ntwo!\nthree\nfour\nfive\n")
kawoosh.test.eq(#hs, 3)
kawoosh.test.eq(hs[1].kind, "modified")
kawoosh.test.eq(hs[1].line, 2)
kawoosh.test.eq(hs[1].old[1], "two")
kawoosh.test.eq(hs[2].kind, "deleted")
kawoosh.test.eq(hs[2].line, 4)
kawoosh.test.eq(hs[3].kind, "added")
kawoosh.test.eq(hs[3].line, 5)
kawoosh.test.eq(#kawoosh.diff("same\n", "same\n"), 0)

kawoosh.opt("places.context", 1)
kawoosh.frame()
local parts = kawoosh.vcs.layout {
  { rel = "a.txt", path = "/x/a.txt", hunks = hs, lines = 5 },
}
-- The header counts the lines: two added (two!, five), two taken out.
kawoosh.test.eq(parts[1], "a.txt  +2 −2\n")
-- Line 1 for context, `two` taken out, then lines 2–3 (two!, three),
-- `gone` taken out, then 4–5 — one run, since the hunks' context meets.
kawoosh.test.eq(parts[2].path, "/x/a.txt")
kawoosh.test.eq(parts[2].from, 1)
kawoosh.test.eq(parts[2].to, 1)
kawoosh.test.eq(parts[3].text, "two\n")
kawoosh.test.eq(parts[3].color, "deleted")
kawoosh.test.eq(parts[4].from, 2)
kawoosh.test.eq(parts[4].to, 3)
kawoosh.test.eq(parts[5].text, "gone\n")
kawoosh.test.eq(parts[6].from, 4)
kawoosh.test.eq(parts[6].to, 5)
kawoosh.test.eq(#parts, 6)

-- Two hunks apart: a `⋯` between their runs; a scratch source by name.
local far = kawoosh.diff("a\nb\nc\nd\ne\nf\ng\nh\n", "A\nb\nc\nd\ne\nf\ng\nH\n")
local p2 = kawoosh.vcs.layout { { rel = "s", name = "vcs:x:s", hunks = far, lines = 8 } }
kawoosh.test.eq(p2[1], "s  +2 −2\n")
kawoosh.test.eq(p2[2].text, "a\n")
kawoosh.test.eq(p2[3].name, "vcs:x:s")
kawoosh.test.eq(p2[3].from, 1)
kawoosh.test.eq(p2[3].to, 2)
kawoosh.test.eq(p2[4], "⋯\n")
kawoosh.test.eq(p2[5].from, 7)
kawoosh.test.eq(p2[5].to, 7)
kawoosh.test.eq(p2[6].text, "h\n")
kawoosh.test.eq(p2[7].from, 8)
kawoosh.test.eq(p2[7].to, 8)

-- `ago`.
kawoosh.test.eq(kawoosh.vcs.ago(os.time()), "now")
kawoosh.test.eq(kawoosh.vcs.ago(os.time() - 3 * 86400), "3d")
kawoosh.test.eq(kawoosh.vcs.ago(os.time() - 400 * 86400), "1y")
