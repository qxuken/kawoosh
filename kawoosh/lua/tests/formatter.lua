-- `kawoosh.formatter(name, def)` with a `run` (docs/design/formatters.md):
-- a formatter written in Lua, answering at once (the text returned) or
-- later (`done`, from a slow job's end); a failure said; its probe read
-- for the indent like any formatter's; `kawoosh.format()`.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-formatter"
kawoosh.fs.create(dir, true)
local file = kawoosh.fs.join(dir, "a.ts")
kawoosh.fs.write(file, "if (a) {\nb;\n}\n")
kawoosh.fs.write(kawoosh.fs.join(dir, ".upperrc"), "")

-- At once: the text returned.
kawoosh.formatter("upper", {
  languages = { "typescript" },
  when = { ".upperrc" },
  run = function(ctx, text)
    assert(ctx.path == file, ctx.path)
    assert(ctx.language == "typescript")
    return text:upper()
  end,
})
kawoosh.cmd("e " .. file)
kawoosh.frame()
kawoosh.cmd("format")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.text(), "IF (A) {\nB;\n}\n", "returned at once")
kawoosh.test.eq(kawoosh.message(), "formatted with upper (2 edits)")

-- Later: `done` from a process's end.
kawoosh.formatter("later", {
  languages = { "typescript" },
  run = function(_, text, done)
    kawoosh.spawn("true", {
      on_exit = function() done(text .. "// later\n") end,
    })
  end,
})
kawoosh.format(nil, { with = "later" })
kawoosh.wait(function() return kawoosh.buf.text():find("// later", 1, true) ~= nil end,
  nil, "the slow formatter's answer")

-- A failure, said; the text as it was.
kawoosh.formatter("bad", {
  languages = { "typescript" },
  run = function(_, _, done) done(nil, "cannot parse") end,
})
local before = kawoosh.buf.text()
kawoosh.cmd("format bad")
kawoosh.frame()
kawoosh.test.eq(kawoosh.message(), "not formatted: bad: cannot parse")
kawoosh.test.eq(kawoosh.buf.text(), before)

-- Its probe is read like any formatter's: three spaces here.
kawoosh.formatter("upper", {
  languages = { "typescript" },
  when = { ".upperrc" },
  probe = { typescript = "if (a) {\nb;\n}\n" },
  run = function(_, text) return (text:gsub("\nb;", "\n   b;")) end,
})
kawoosh.frame()
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.indent().shiftwidth, 3, "the Lua formatter's indent")

kawoosh.fs.remove(dir)
