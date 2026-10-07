-- `kawoosh.buf.edits(edits, buffer, { carets = … })`: the edits as one
-- step and the carets after them placed by the engine — into an edit's
-- text, or an offset of the text before as the edits move it (docs/
-- design/lua-boundary.md).
local function text() return table.concat(kawoosh.buf.lines(), "\n") end
local function heads()
  local out = {}
  for _, s in ipairs(kawoosh.buf.selections()) do out[#out + 1] = s.head end
  return table.concat(out, " ")
end
kawoosh.cmd("enew")
kawoosh.frame()

-- A caret into each edit's text; the second the primary.
kawoosh.buf.set_text("a b")
kawoosh.frame()
kawoosh.buf.edits({ { 0, 0, "()" }, { 2, 2, "[]" } }, nil,
  { carets = { { edit = 1, at = 1 }, { edit = 2, at = 1, primary = true } } })
kawoosh.frame()
kawoosh.test.eq(text(), "()a []b", "both inserted")
kawoosh.test.eq(heads(), "1 5", "each caret inside its pair")
kawoosh.test.eq(kawoosh.buf.selections()[2].primary, true, "the primary as marked")

-- An offset: moved by the edits before it, before an insertion at its
-- own byte, kept its distance into a replaced range (its last
-- character at most).
kawoosh.buf.set_text("abcdef")
kawoosh.frame()
kawoosh.buf.edits({ { 0, 1, "XYZ" }, { 3, 3, "+" }, { 4, 6, "Q" } }, nil,
  { carets = { { at = 2 }, { at = 3 }, { at = 5 } } })
kawoosh.frame()
kawoosh.test.eq(text(), "XYZbc+dQ", "the edits")
kawoosh.test.eq(heads(), "4 5 7", "after XYZ; before the +; on Q, the replaced range's last")

-- No edits, carets only: where they are given.
kawoosh.buf.set_text("abc")
kawoosh.frame()
kawoosh.buf.edits({}, nil, { carets = { { at = 2 } } })
kawoosh.frame()
kawoosh.test.eq(heads(), "2", "moved, nothing edited")
kawoosh.test.eq(text(), "abc", "and the text as it was")
