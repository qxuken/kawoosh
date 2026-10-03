-- A plugin's diagnostics (docs/design/lists.md Decision 7):
-- `kawoosh.diagnostics.set` under a name of its own, read back by
-- `get` (and `kawoosh.lsp.diagnostics`, every publisher's), counted,
-- walked by `]d`, carried through an edit, replaced by the same name's
-- next word and taken back by `clear`; a path no buffer holds kept for
-- the buffer that opens it; `on_diagnostics` told.
kawoosh.cmd("enew")
kawoosh.frame()
kawoosh.buf.set_text("let a = 1\nlet b = 2\nlet c = 3\n")
kawoosh.frame()
local me = kawoosh.buf.current()

local heard = 0
kawoosh.on_diagnostics(function() heard = heard + 1 end)

kawoosh.diagnostics.set(0, "lint", {
  { line = 2, col = 5, message = "b is a poor name", code = "N1" },
  { line = 3, col = 5, end_col = 6, severity = "hint", message = "c too" },
})
kawoosh.frame()
local rows = kawoosh.diagnostics.get { buffer = 0 }
kawoosh.test.eq(#rows, 2, "both said")
kawoosh.test.eq(rows[1].line, 2)
kawoosh.test.eq(rows[1].col, 5)
kawoosh.test.eq(rows[1].end_col, 6, "one character unless said")
kawoosh.test.eq(rows[1].severity, 1, "an error unless said")
kawoosh.test.eq(rows[1].from, "lint")
kawoosh.test.eq(rows[1].source, "lint", "the name its source unless said")
kawoosh.test.eq(rows[1].code, "N1")
kawoosh.test.eq(rows[1].buffer, me)
kawoosh.test.eq(rows[2].level, "hint")
kawoosh.test.eq(#kawoosh.lsp.diagnostics { buffer = 0 }, 2, "every publisher's")
kawoosh.test.eq(#kawoosh.diagnostics.get { from = "lsp" }, 0, "none of the servers'")
kawoosh.test.eq(#kawoosh.diagnostics.get { from = "lint", severity = "error" }, 1)
kawoosh.test.eq(kawoosh.lsp.counts().errors, 1)
kawoosh.test.eq(kawoosh.lsp.counts().hints, 1)
kawoosh.test.ok(heard > 0, "on_diagnostics told")

-- `]d` walks them as it walks a server's.
kawoosh.press("gg0]d")
kawoosh.test.eq(kawoosh.buf.cursor().line, 2)
kawoosh.test.eq(kawoosh.buf.cursor().col, 5)
kawoosh.test.eq(kawoosh.message(), "b is a poor name")

-- An edit above carries them, as the layer carries a server's.
kawoosh.press("ggO// x<Esc>")
kawoosh.frame()
rows = kawoosh.diagnostics.get { buffer = 0 }
kawoosh.test.eq(rows[1].line, 3, "moved down with the text")
kawoosh.test.eq(rows[2].line, 4)

-- The same name again replaces its own; another name's stays.
kawoosh.diagnostics.set(0, "spell", { { line = 1, col = 4, end_col = 5, message = "x?" } })
kawoosh.diagnostics.set(0, "lint", { { line = 4, col = 1, end_col = 4, message = "let" } })
kawoosh.frame()
rows = kawoosh.diagnostics.get { buffer = 0 }
kawoosh.test.eq(#rows, 2)
kawoosh.test.eq(rows[1].from, "spell")
kawoosh.test.eq(rows[2].message, "let")
-- A row read back is an item again.
kawoosh.diagnostics.set(0, "lint", kawoosh.diagnostics.get { from = "lint" })
kawoosh.frame()
local again = kawoosh.diagnostics.get { from = "lint" }
kawoosh.test.eq(#again, 1)
kawoosh.test.eq(again[1].line, 4)
kawoosh.test.eq(again[1].end_col, 4)

-- An empty list takes one name's back; `clear` every buffer's.
kawoosh.diagnostics.set(0, "lint", {})
kawoosh.frame()
kawoosh.test.eq(#kawoosh.diagnostics.get { buffer = 0 }, 1, "spell's left")
kawoosh.diagnostics.clear("spell")
kawoosh.frame()
kawoosh.test.eq(#kawoosh.diagnostics.get { buffer = 0 }, 0, "all gone")

-- The servers' name is not a plugin's.
local ok, err = pcall(kawoosh.diagnostics.set, 0, "lsp", {})
kawoosh.test.ok(not ok and tostring(err):find("not a name", 1, true), tostring(err))
ok, err = pcall(kawoosh.diagnostics.set, 0, "lint", { { message = "where?" } })
kawoosh.test.ok(not ok and tostring(err):find("a `line`", 1, true), tostring(err))

-- A file no buffer holds: kept by path, taken by the buffer that opens it.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-plugin-diagnostics"
kawoosh.fs.create(dir, true)
local file = kawoosh.fs.join(dir, "later.txt")
kawoosh.fs.write(file, "one\ntwo\n")
kawoosh.diagnostics.set(file, "lint", { { line = 2, col = 1, end_col = 4, message = "two!" } })
kawoosh.frame()
rows = kawoosh.diagnostics.get { from = "lint" }
kawoosh.test.eq(#rows, 1)
kawoosh.test.eq(rows[1].buffer, nil, "no buffer yet")
kawoosh.test.eq(rows[1].path, file)
kawoosh.cmd("e " .. file)
kawoosh.wait(function()
  local r = kawoosh.diagnostics.get { buffer = 0 }
  return #r == 1 and r[1].buffer ~= nil
end, 100, "the opened file's buffer to take them")
rows = kawoosh.diagnostics.get { buffer = 0 }
kawoosh.test.eq(rows[1].line, 2)
kawoosh.test.eq(rows[1].end_col, 4)
kawoosh.test.eq(rows[1].from, "lint")
kawoosh.diagnostics.clear("lint")
kawoosh.frame()
kawoosh.test.eq(#kawoosh.diagnostics.get {}, 0)

local removed = false
kawoosh.fs.remove(dir, function() removed = true end)
kawoosh.wait(function() return removed end, 200, "the scratch folder removed")
