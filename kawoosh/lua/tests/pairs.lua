-- Auto-closing brackets (pairs.lua, docs/design/pairs.md): on by
-- default and off when set so; `(` pairs, `)` steps over, `<BS>` deletes both, `<CR>`
-- opens a block, a quote pairs only where one can open, rust's `'` does
-- not, two carets pair each, `.` repeats a pairing, an undo takes it
-- with the insert, and the command line types plain.
local function text() return table.concat(kawoosh.buf.lines(), "\n") end
local function reset(s)
  kawoosh.cmd("enew")
  kawoosh.frame()
  if s and s ~= "" then kawoosh.buf.set_text(s) end
  kawoosh.frame()
end

kawoosh.cmd("set pairs.enabled=false")
kawoosh.frame()
reset()
kawoosh.press("i(<Esc>")
kawoosh.test.eq(text(), "(", "off when set so")

kawoosh.cmd("set pairs.enabled=true")
kawoosh.frame()
reset()
kawoosh.press("i(")
kawoosh.test.eq(text(), "()")
kawoosh.test.eq(kawoosh.buf.cursor().col, 2, "the caret between")
kawoosh.press("x)")
kawoosh.test.eq(text(), "(x)", "a closer steps over its own")
kawoosh.test.eq(kawoosh.buf.cursor().col, 4)
kawoosh.press("<Esc>")

reset()
kawoosh.press("i[<BS>")
kawoosh.test.eq(text(), "", "<BS> between a pair deletes both")
kawoosh.press("ab<BS><Esc>")
kawoosh.test.eq(text(), "a", "and one character elsewhere")

reset()
kawoosh.press("i{<CR>x<Esc>")
kawoosh.test.eq(text(), "{\n    x\n}", "<CR> opens the block")

reset()
kawoosh.press("ia\"<Esc>")
kawoosh.test.eq(text(), "a\"", "no quote pair after a word")
reset()
kawoosh.press("i\"q\"<Esc>")
kawoosh.test.eq(text(), "\"q\"", "a quote pairs, and steps over its own")
reset("foo")
kawoosh.press("i(<Esc>")
kawoosh.test.eq(text(), "(foo", "no pair before a word")

reset()
kawoosh.cmd("syntax rust")
kawoosh.frame()
kawoosh.press("i'a<Esc>")
kawoosh.test.eq(text(), "'a", "rust's lifetimes")

-- Two carets: each pairs.
reset("x\ny")
kawoosh.press("gg<C-j>I(<Esc>")
kawoosh.test.eq(text(), "(x\n(y", "before a word, each caret plain")
reset("\n")
kawoosh.press("gg<C-j>i(<Esc>")
kawoosh.test.eq(text(), "()\n()", "two carets, two pairs")

-- `<BS>` at `(|)|`: the pair and the `)` after it reach into each
-- other, and are one deletion, not two that cut the text.
reset("()ab")
kawoosh.press("i")
kawoosh.buf.set_selections({ { 1, 1 }, { 2, 2 } })
kawoosh.frame()
kawoosh.press("<BS><Esc>")
kawoosh.test.eq(text(), "ab", "the pair and its closer, once")

-- The primary caret stays the primary through a pairing.
reset("\n")
kawoosh.press("i")
kawoosh.buf.set_selections({ { 0, 0 }, { 1, 1, primary = true } })
kawoosh.frame()
kawoosh.press("(")
local sels = kawoosh.buf.selections()
kawoosh.test.eq(text(), "()\n()", "two pairs")
kawoosh.test.eq(sels[2].primary, true, "the second is still the primary")
kawoosh.press("<Esc>")

-- `.` repeats a pairing.
reset("a\nb")
kawoosh.press("A(<Esc>j.")
kawoosh.test.eq(text(), "a()\nb()")

-- An undo takes the pair with the rest of the insert.
reset()
kawoosh.press("ia(b<Esc>u")
kawoosh.test.eq(text(), "", "one undo")

-- The command line types plain.
reset()
kawoosh.press(":echo (<CR>")
kawoosh.test.eq(kawoosh.message(), "(", "the prompt is not paired")

kawoosh.cmd("set pairs.enabled=false")
