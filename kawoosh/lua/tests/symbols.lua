-- The symbols picker (docs/design/marks.md Decisions 1–2): the
-- grammar's outline as a tree in the file's order, the cursor on the
-- symbol the caret is in, the pane following the cursor, and closing
-- untaken putting the caret back; typed into, the matches, and `<CR>`
-- going there.
local eq = kawoosh.test.eq
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-symbols"
kawoosh.fs.create(dir, true)
local file = kawoosh.fs.join(dir, "a.rs")
kawoosh.fs.write(file, table.concat({
  "struct S {",
  "    a: u8,",
  "}",
  "",
  "impl S {",
  "    fn new() -> Self {",
  "        todo!()",
  "    }",
  "",
  "    fn get(&self) -> u8 {",
  "        self.a",
  "    }",
  "}",
  "",
  "fn main() {}",
  "",
}, "\n"))
-- The grammar's, whatever server this machine has.
kawoosh.opt("symbols.source", "syntax")
kawoosh.cmd("e " .. file)
kawoosh.wait(function() return kawoosh.buf.line(1) == "struct S {" end, nil, "the file read")
local h = kawoosh.buf.current()
kawoosh.buf.set_cursor(kawoosh.buf.offset(11, 9))
kawoosh.frame()
eq(kawoosh.buf.cursor(h).line, 11, "the caret inside `get`")

kawoosh.press("grs")
kawoosh.wait(function()
  local st = kawoosh.picker.state()
  return st and st.count > 0
end, nil, "the outline")
local st = kawoosh.picker.state()
eq(st.source, "symbols")
eq(table.concat(st.rows, " "), "S a S new get main", "every definition, in the file's order")
eq(st.text, "get", "the cursor on the symbol the caret is in")
eq(st.item.depth, 1, "a method inside its impl")
eq(st.item.kind, "method")
-- A row reads as its line does: what stands before the name is its
-- head, and once the syntax answers, in the tokens' colours.
local function head_text(it)
  if type(it.head) ~= "table" then return it.head end
  local parts = {}
  for _, sp in ipairs(it.head) do parts[#parts + 1] = sp[1] end
  return table.concat(parts)
end
eq(head_text(st.item), "fn", "`fn get`")
eq(st.item.tag, nil, "the head says the kind")
local heads = {}
for i, it in ipairs(st.items) do heads[i] = head_text(it) or "·" end
eq(table.concat(heads, " "), "struct · impl fn fn fn", "each row's head from its line; a bare field has none")
eq(st.items[2].tag, "field", "and keeps its kind faint")
kawoosh.wait(function()
  local s = kawoosh.picker.state()
  return s and type(s.item.head) == "table"
end, nil, "the syntax's colours")
st = kawoosh.picker.state()
eq(st.item.head[1][1], "fn")
eq(st.item.head[1].color ~= nil, true, "`fn` in the keyword's colour")
eq(st.item.color ~= nil, true, "the name in its token's colour")
kawoosh.frame()
eq(kawoosh.buf.cursor(h).line, 10, "the pane follows: the caret on `get`'s name")

kawoosh.press("<C-p>")
kawoosh.frame(2)
eq(kawoosh.picker.state().text, "new")
eq(kawoosh.buf.cursor(h).line, 6, "and on `new` as the cursor moves")

kawoosh.press("<C-c>")
kawoosh.frame(2)
eq(kawoosh.picker.state(), nil, "closed")
local c = kawoosh.buf.cursor(h)
eq(c.line, 11, "closed untaken: the caret back where it was")
eq(c.col, 9)

kawoosh.press("grs")
kawoosh.wait(function()
  local s = kawoosh.picker.state()
  return s and s.count > 0
end, nil, "the outline again")
kawoosh.press("main")
kawoosh.frame()
st = kawoosh.picker.state()
eq(st.text, "main", "typed into, the match")
kawoosh.press("<CR>")
kawoosh.frame(2)
eq(kawoosh.picker.state(), nil, "gone on a pick")
eq(kawoosh.buf.cursor(h).line, 15, "the pick kept")
-- Only the pick was a jump (docs/design/jumps.md): not the pane's
-- moves under the picker, nor the close untaken.
local left = {}
for _, j in ipairs(kawoosh.memory { jumps = true }) do
  if j.path and j.path:match("a%.rs$") then left[#left + 1] = j.line .. ":" .. j.col end
end
eq(table.concat(left, " "), "11:9", "the pick left `get`'s line, once")

-- `picker.symbol_at`: the innermost holding the line, else the last
-- before it.
local items = {
  { line = 1, end_line = 10 }, { line = 2, end_line = 4 }, { line = 6, end_line = 8 },
}
eq(kawoosh.picker.symbol_at(items, 3), 2)
eq(kawoosh.picker.symbol_at(items, 5), 1)
eq(kawoosh.picker.symbol_at(items, 12), 3)

-- `picker.merge_symbols`: the server's, and the grammar's it did not
-- list, nested by the lines they hold; `impl S` is the grammar's `S`.
local merged = kawoosh.picker.merge_symbols(
  { { name = "impl S", line = 5, end_line = 13 }, { name = "get", line = 10, end_line = 12 } },
  { { name = "S", line = 5, end_line = 13 }, { name = "get", line = 10, end_line = 12 },
    { name = "v", line = 11, end_line = 11 }, { name = "main", line = 15, end_line = 15 } })
local shape = {}
for _, m in ipairs(merged) do shape[#shape + 1] = m.depth .. " " .. m.name end
eq(table.concat(shape, ", "), "0 impl S, 1 get, 2 v, 0 main")
-- `pcall`: rust-analyzer, started on the `.rs`, holds the directory as
-- its cwd, and Windows will not remove it then.
pcall(kawoosh.fs.remove, dir)
