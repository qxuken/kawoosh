-- Node actions (docs/design/node-actions.md): `g.` does what the node
-- under the caret means — flip, operator, split and join, quotes,
-- digits — the innermost node an action answers for, up to a body;
-- many carets one step, a node inside another's skipped; `.` again,
-- `u` once; `node action NAME N`; a shipped one off by the settings.
local function text() return kawoosh.buf.text() end
local function tree()
  kawoosh.wait(function() return kawoosh.node.root() ~= nil end, 100, "the tree")
end
local function buf(language, s)
  kawoosh.cmd("enew")
  kawoosh.frame()
  kawoosh.buf.set_text(s)
  kawoosh.cmd("syntax " .. language)
  tree()
end
-- The offset of the first `s` in the text, `skip` bytes into it.
local function at(s, skip)
  return assert(text():find(s, 1, true), s) - 1 + (skip or 0)
end
local function act(s, skip, keys)
  kawoosh.buf.set_cursor(at(s, skip))
  kawoosh.frame()
  tree()
  kawoosh.press(keys or "g.")
  kawoosh.frame()
end

-- flip, in three grammars' spellings.
buf("rust", "fn main() {\n    let a = true;\n}\n")
act("true", 3)
kawoosh.test.eq(text(), "fn main() {\n    let a = false;\n}\n", "rust's true")
kawoosh.test.eq(kawoosh.message(), "flip · true")
kawoosh.test.eq(kawoosh.buf.cursor().offset, at("false", 3), "the caret kept its place")
act("false")
kawoosh.test.ok(text():find("= true;", 1, true), "and back")
buf("python", "x = True\n")
act("True")
kawoosh.test.eq(text(), "x = False\n", "python's")
buf("yaml", "a: true\n")
act("true")
kawoosh.test.eq(text(), "a: false\n", "yaml's")

-- operator: mirrored, lua's own not-equal, and a type's `<` left be.
buf("rust", "fn f() -> bool {\n    let v: Vec<u8> = w;\n    a < b && c == d\n}\n")
act("==")
kawoosh.test.ok(text():find("c != d", 1, true), "== → !=")
act("&&")
kawoosh.test.ok(text():find("a < b || c", 1, true), "&& → ||")
act("< b")
kawoosh.test.ok(text():find("a > b", 1, true), "< → >")
local before = text()
act("<u8")
kawoosh.test.eq(text(), before, "a type's < is no comparison")
buf("lua", "local x = a == b\n")
act("==")
kawoosh.test.eq(text(), "local x = a ~= b\n", "lua's ~=")

-- split and join: rust, from anywhere in the list, the caret on the
-- opener after; the trailing comma; back on one line.
buf("rust", "fn main() {\n    f(a, g(b), c);\n}\n")
act("c)")
kawoosh.test.eq(text(), "fn main() {\n    f(\n        a,\n        g(b),\n        c,\n    );\n}\n")
kawoosh.test.eq(kawoosh.message(), "split · arguments")
kawoosh.test.eq(kawoosh.buf.cursor().offset, at("f(", 1), "on the opener")
act("        a")
kawoosh.test.eq(text(), "fn main() {\n    f(a, g(b), c);\n}\n", "joined")
-- The innermost list answers first; the Nth up by name.
act("b)", 0, ":node action split 2<CR>")
kawoosh.test.ok(text():find("f(\n", 1, true), "the second list up: f's")
kawoosh.press("u")
kawoosh.frame()
act("b)", 0, ":node action split<CR>")
kawoosh.test.ok(text():find("g(\n", 1, true), "the first: g's")
kawoosh.press("u")
kawoosh.frame()
-- A one-item tuple keeps its comma; a struct literal its spaces.
buf("rust", "fn main() {\n    let t = (\n        1,\n    );\n    let s = S {\n        a: 1,\n    };\n}\n")
act("(\n")
kawoosh.test.ok(text():find("let t = (1,);", 1, true), "(1,)")
act("{\n        a")
kawoosh.test.ok(text():find("S { a: 1 };", 1, true), "S { a: 1 }")
-- A comment in the list: no join, and the climb finds nothing more.
buf("rust", "fn main() {\n    f(\n        a, // why\n        b,\n    );\n}\n")
before = text()
act("(\n")
kawoosh.test.eq(text(), before, "a line comment would swallow b")
kawoosh.test.eq(kawoosh.message(), "no node action here")
-- The climb stops at a body: a closure's does not split the call.
buf("rust", "fn main() {\n    f(|| { x }, y);\n}\n")
before = text()
act("x }")
kawoosh.test.eq(text(), before, "not from inside the closure's body")
-- Other languages' rules: go's comma, json's none, lua's call and table.
buf("go", "package p\n\nvar x = []int{1, 2}\n")
local unit = kawoosh.buf.indent().unit
act("1,")
kawoosh.test.eq(text(), "package p\n\nvar x = []int{\n" .. unit .. "1,\n" .. unit .. "2,\n}\n", "go")
buf("json", "[1, 2]\n")
unit = kawoosh.buf.indent().unit
act("1,")
kawoosh.test.eq(text(), "[\n" .. unit .. "1,\n" .. unit .. "2\n]\n", "json: no trailing comma")
buf("lua", "f(a, b)\nlocal t = {\n  1,\n  2,\n}\n")
unit = kawoosh.buf.indent().unit
act("a,")
kawoosh.test.ok(text():find("f(\n" .. unit .. "a,\n" .. unit .. "b\n)", 1, true), "a lua call: no trailing comma")
act("{\n")
kawoosh.test.ok(text():find("local t = { 1, 2 }", 1, true), "a lua table joined, padded")

-- quotes: javascript's three, python's prefix, a decline.
buf("javascript", "const s = \"a\";\nconst t = \"it's\";\n")
act("\"a\"", 1)
kawoosh.test.ok(text():find("const s = 'a';", 1, true), "\" → '")
act("'a'", 1)
kawoosh.test.ok(text():find("const s = `a`;", 1, true), "' → `")
act("`a`", 1)
kawoosh.test.ok(text():find("const s = \"a\";", 1, true), "` → \"")
before = text()
act("it's")
kawoosh.test.eq(text(), before, "it's cannot be '…'")
buf("python", "x = f\"a\"\n")
act("a\"")
kawoosh.test.eq(text(), "x = f'a'\n", "the prefix kept")

-- digits.
buf("rust", "fn main() {\n    let n = 1000000u64 + 1000;\n}\n")
act("1000000")
kawoosh.test.ok(text():find("1_000_000u64", 1, true), "grouped, the suffix kept")
act("1_000")
kawoosh.test.ok(text():find("1000000u64", 1, true), "and back")
before = text()
act("1000;")
kawoosh.test.eq(text(), before, "four digits are left")

-- Many carets: one step, one undo; a node inside another's skipped.
buf("rust", "fn main() {\n    let a = true;\n    let b = false;\n}\n")
tree()
kawoosh.buf.set_selections({ { at("true"), at("true") }, { at("false"), at("false"), primary = true } })
kawoosh.frame()
kawoosh.press("g.")
kawoosh.frame()
kawoosh.test.eq(text(), "fn main() {\n    let a = false;\n    let b = true;\n}\n", "both")
kawoosh.test.eq(#kawoosh.buf.selections(), 2, "both carets kept")
kawoosh.press("u")
kawoosh.frame()
kawoosh.test.eq(text(), "fn main() {\n    let a = true;\n    let b = false;\n}\n", "one undo")
buf("rust", "fn main() {\n    f(true, x);\n}\n")
kawoosh.buf.set_selections({ { at("true"), at("true") }, { at("x)"), at("x)"), primary = true } })
kawoosh.frame()
kawoosh.press("g.")
kawoosh.frame()
kawoosh.test.ok(text():find("f(\n        true,\n        x,\n    )", 1, true), "the list split, the flip inside it not")
kawoosh.test.eq(kawoosh.message(), "split · arguments · 1 inside another skipped")

-- `.` does it again elsewhere.
buf("rust", "fn main() {\n    let a = true;\n    let b = true;\n}\n")
act("true")
kawoosh.buf.set_cursor(at("true"))
kawoosh.frame()
tree()
kawoosh.press(".")
kawoosh.frame()
kawoosh.test.eq(text(), "fn main() {\n    let a = false;\n    let b = false;\n}\n", ". flips the next")

-- From visual mode: the selection picks the node, normal mode after.
buf("rust", "fn main() {\n    let a = true;\n}\n")
kawoosh.buf.set_cursor(at("true"))
kawoosh.frame()
kawoosh.press("vl")
kawoosh.press("g.")
kawoosh.frame()
kawoosh.test.eq(text(), "fn main() {\n    let a = false;\n}\n", "visual")
kawoosh.test.eq(kawoosh.mode(), "normal")

-- Off by the settings; a plugin's own action, newest first.
kawoosh.opt("node_actions", { flip = false })
act("false")
kawoosh.test.eq(text(), "fn main() {\n    let a = false;\n}\n", "flip turned off")
kawoosh.opt("node_actions", {})
kawoosh.node.action("shout", {
  languages = { "rust" },
  types = { "false" },
  run = function(n) return n:text():upper() end,
})
act("false")
kawoosh.test.ok(text():find("= FALSE;", 1, true), "a plugin's action before the shipped one")
kawoosh.node.action("shout", nil)

-- `:node actions`: every answer from the caret up, innermost first; the
-- pick acts on the buffer and caret it was opened from.
buf("rust", "fn main() {\n    f(true, x);\n}\n")
kawoosh.buf.set_cursor(at("true"))
kawoosh.frame()
tree()
kawoosh.cmd("node actions")
kawoosh.frame()
kawoosh.test.eq(table.concat(kawoosh.picker.state().rows, " ; "), "flip · true ; split · arguments")
kawoosh.press("<C-n><CR>")
kawoosh.frame()
kawoosh.test.eq(text(), "fn main() {\n    f(\n        true,\n        x,\n    );\n}\n", "the second picked")

-- A list a user adds to `split`'s table is split.
kawoosh.node_actions.lists.rust.token_tree = { trailing = false }
buf("rust", "fn main() {\n    m!(a, b);\n}\n")
act("a,")
kawoosh.test.eq(text(), "fn main() {\n    m!(\n        a,\n        b\n    );\n}\n", "a macro's tokens, added")
kawoosh.node_actions.lists.rust.token_tree = nil
