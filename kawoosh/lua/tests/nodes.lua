-- The syntax tree in Lua (docs/design/nodes.md): `kawoosh.node.at`,
-- `leaf`, `root`, the walking functions, a query's captures, `select`,
-- a stale node's error, and `<A-u>` again in Lua, landing where the
-- Rust one does.
local src = table.concat({
  "// one",
  "// two",
  "fn main() {",
  "    let ok = a == b;",
  "    let x = f(1, true);",
  "}",
  "",
}, "\n")
kawoosh.cmd("enew")
kawoosh.frame()
kawoosh.buf.set_text(src)
kawoosh.cmd("syntax rust")
kawoosh.wait(function() return kawoosh.node.root() ~= nil end, 100, "the tree")

-- The byte offset of the first `s` in the text, from 0.
local function at(s) return assert(src:find(s, 1, true), s) - 1 end

local root = kawoosh.node.root()
kawoosh.test.eq(root.type, "source_file")
kawoosh.test.eq(root.line, 1)
kawoosh.test.eq(root.end_line, 6, "the last newline ends the last line")
kawoosh.test.eq(root.language, "rust")

local f = kawoosh.node.at(at("f("))
kawoosh.test.eq(f.type, "identifier")
kawoosh.test.eq(f:text(), "f")
kawoosh.test.eq(f.field, "function", "the field it fills")
kawoosh.test.eq(tostring(f), "identifier " .. at("f(") .. ".." .. at("f(") + 1)
kawoosh.test.ok(f == kawoosh.node.at(at("f(")), "one node, two tables, equal")

local call = f:parent()
kawoosh.test.eq(call.type, "call_expression")
kawoosh.test.eq(call:text(), "f(1, true)")
kawoosh.test.eq(call:get("arguments"):text(), "(1, true)")
kawoosh.test.eq(call.line, 5)

-- A token is a leaf's, not at's.
kawoosh.test.eq(kawoosh.node.at(at("true")).type, "boolean_literal")
local t = kawoosh.node.leaf(at("true"))
kawoosh.test.eq(t.type, "true")
kawoosh.test.eq(t.named, false)

local args = call:get("arguments")
kawoosh.test.eq(#args:children(), 2, "the named children")
kawoosh.test.eq(#args:children { anonymous = true }, 5, "and the tokens")
kawoosh.test.eq(args:child(1):text(), "1")
kawoosh.test.eq(args:child(-1):text(), "true")
kawoosh.test.eq(args:child(1, { anonymous = true }):text(), "(")
kawoosh.test.eq(args:child(3), nil)
kawoosh.test.eq(args:child(1):next():text(), "true")
kawoosh.test.eq(args:child(1):next { anonymous = true }:text(), ",")
kawoosh.test.eq(args:child(-1):prev():text(), "1")
kawoosh.test.eq(args:child(-1):next(), nil)

local bin = kawoosh.node.at(at("a ==")):parent()
kawoosh.test.eq(bin.type, "binary_expression")
local op = bin:get("operator")
kawoosh.test.eq(op:text(), "==", "a field names a token too")
kawoosh.test.eq(op.named, false)

local one = kawoosh.node.at(at("1,"))
kawoosh.test.eq(one:closest("let_declaration"):text(), "let x = f(1, true);")
kawoosh.test.eq(one:closest { "block", "function_item" }.type, "block")
kawoosh.test.eq(one:closest("integer_literal"), one, "itself first")
kawoosh.test.eq(one:closest("impl_item"), nil)
kawoosh.test.eq(root:parent(), nil)

-- Queries: over the buffer, over a node, a quantified capture's all.
local names = {}
for _, m in ipairs(kawoosh.node.query("(let_declaration pattern: (identifier) @name)")) do
  names[#names + 1] = m.captures.name:text()
end
kawoosh.test.eq(table.concat(names, " "), "ok x")
local ms = call:query("(arguments (_) @arg)")
kawoosh.test.eq(#ms, 2, "a match per argument, inside the call only")
local q = kawoosh.node.query("(source_file . (line_comment)+ @doc)", root)
kawoosh.test.eq(#q, 1)
kawoosh.test.eq(#q[1].all.doc, 2, "a quantified capture's nodes")
kawoosh.test.eq(q[1].captures.doc:text(), "// one", "its first")
kawoosh.test.eq(q[1].pattern, 1)
local ok, err = pcall(kawoosh.node.query, "(let_declaration", root)
kawoosh.test.ok(not ok and tostring(err):find("query"), "a query that does not compile")

-- select: visual over the node, the head on its last character.
call:select()
kawoosh.frame()
kawoosh.test.eq(kawoosh.mode(), "visual")
local s = kawoosh.buf.selections()[1]
kawoosh.test.eq(s.anchor, call.from)
kawoosh.test.eq(s.head, call.to - 1)
kawoosh.press("<Esc>")

-- `<A-u>` in Lua: a caret on a token asks its parent, a caret between
-- a node's children is inside that node, and up past any node that
-- starts where the caret is.
local function up()
  local c = kawoosh.buf.cursor().offset
  local n = kawoosh.node.leaf(c)
  if #n:children { anonymous = true } == 0 then n = n:parent() end
  while n and n.from >= c do n = n:parent() end
  if n then kawoosh.buf.set_cursor(n.from) end
end
for _, start in ipairs({ at("true"), at("b;"), at("x ="), at("\n}") }) do
  local rust, lua = {}, {}
  kawoosh.buf.set_cursor(start)
  kawoosh.frame()
  for i = 1, 4 do
    kawoosh.press("<A-u>")
    rust[i] = kawoosh.buf.cursor().offset
  end
  kawoosh.buf.set_cursor(start)
  kawoosh.frame()
  for i = 1, 4 do
    up()
    kawoosh.frame()
    lua[i] = kawoosh.buf.cursor().offset
  end
  kawoosh.test.eq(table.concat(lua, " "), table.concat(rust, " "), "up from " .. start)
end

-- A node kept past its text's version is an error when walked; the
-- tree read again is the new text's.
kawoosh.buf.insert(0, "// c\n")
kawoosh.frame()
local fresh, why = kawoosh.node.root()
kawoosh.test.ok(fresh == nil and why:find("behind"), "behind the text right after an edit")
kawoosh.wait(function() return kawoosh.node.root() ~= nil end, 100, "the tree again")
local ok2, err2 = pcall(function() return call:text() end)
kawoosh.test.ok(not ok2 and tostring(err2):find("read it again"), "a stale node")
kawoosh.test.eq(kawoosh.node.at(at("f(") + 5):parent():text(), "f(1, true)")

-- No grammar, no tree: nil and why.
kawoosh.cmd("syntax text")
kawoosh.frame()
local none, why2 = kawoosh.node.at(0)
kawoosh.test.eq(none, nil)
kawoosh.test.ok(why2 ~= nil, "a reason")
